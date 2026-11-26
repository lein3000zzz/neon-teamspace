//! Reading an upload, one markdown file or a zip of them, into the files and folders an
//! import makes pages of, within the [`Limits`]. A zip is read in memory, as uploaded:
//! nothing of it is written to disk.
//!
//! In a zip, each markdown file is a page and each folder holding one is too. A folder and
//! a markdown file of the same name beside it, `Notes/` and `Notes.md`, are one page: the
//! file's content with the folder's pages under it, which is how an export writes them.
//! Siblings keep the order of the zip, by the first entry of each. Other files are kept
//! aside, unread, for the pages that show them, and reported as left out until one does;
//! hidden ones and links are left out and reported; the folders and files archivers add for
//! their own use are left out without a word.

use std::collections::HashMap;
use std::io::{Cursor, Read as _};

use bytes::Bytes;
use neon_domain::pages::files::entry_path;
use zip::{CompressionMethod, ZipArchive};

use super::{Deadline, Limits, SkipReason, Skipped, TransferError, Upload, size};

/// Names archivers give their own files: macOS's resource forks and folder settings, and
/// Windows' thumbnails and folder settings.
const SYSTEM_FOLDER: &str = "__MACOSX";
const SYSTEM_FILES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];
const RESOURCE_FORK: &str = "._";

/// The extensions of markdown files.
const MARKDOWN: &[&str] = &["md", "markdown"];

/// A markdown file or a folder of an upload, as a page to be.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Entry {
    /// The file's name without its extension, or the folder's.
    pub(super) name: String,
    /// Where it is in the upload, for messages: the file's path, or the folder's.
    pub(super) path: String,
    /// The file's text; `None` for a folder without a file of its name.
    pub(super) markdown: Option<String>,
    /// The folders the file is in, from the top of the zip, which its links to other files
    /// start from.
    pub(super) folder: Vec<String>,
    pub(super) children: Vec<Entry>,
    /// Each child's index among them, by name.
    pub(super) index: HashMap<String, usize>,
}

impl Entry {
    /// The child of this name, made if there is none: `made` counts the entries made.
    fn child(&mut self, name: &str, path: &str, made: &mut usize) -> &mut Entry {
        let at = match self.index.get(name) {
            Some(at) => *at,
            None => {
                *made += 1;
                self.children.push(Entry {
                    name: name.to_owned(),
                    path: path.to_owned(),
                    ..Entry::default()
                });
                self.index.insert(name.to_owned(), self.children.len() - 1);
                self.children.len() - 1
            }
        };
        &mut self.children[at]
    }
}

/// What an upload holds for an import.
#[derive(Debug, Default)]
pub(super) struct Contents {
    /// The pages to be at the top, in order, each with the pages to be under it.
    pub(super) entries: Vec<Entry>,
    /// What is left out, in the zip's order: the files kept aside among them, until a page
    /// shows one.
    pub(super) skipped: Vec<Skipped>,
    /// How many pages the entries make.
    pub(super) pages: usize,
    /// The other files a page may show, by their path in the zip, with their index in it.
    pub(super) files: HashMap<String, usize>,
}

/// Reads an upload: a markdown file, by its extension `.md` or `.markdown`, or a zip of
/// them, `.zip`. Past a limit, or past `deadline`, it stops.
pub(super) fn read(
    upload: &Upload,
    limits: &Limits,
    deadline: Deadline,
) -> Result<Contents, TransferError> {
    if upload.bytes.len() > limits.max_upload {
        return Err(TransferError::too_large(format!(
            "the file is larger than {}",
            size(limits.max_upload)
        )));
    }
    // Some clients send the path the file was picked from.
    let name = upload.name.rsplit(['/', '\\']).next().unwrap_or_default();
    if let Some(stem) = markdown_stem(name) {
        if upload.bytes.len() > limits.max_file {
            return Err(TransferError::too_large(format!(
                "a markdown file is at most {}",
                size(limits.max_file)
            )));
        }
        let markdown = String::from_utf8(upload.bytes.to_vec())
            .map_err(|_| TransferError::invalid(format!("{} is not text in UTF-8", shown(name))))?;
        let entry = Entry {
            name: stem.to_owned(),
            path: shown(name),
            markdown: Some(markdown),
            ..Entry::default()
        };
        return Ok(Contents {
            entries: vec![entry],
            pages: 1,
            ..Contents::default()
        });
    }
    if extension(name).is_some_and(|ext| ext.eq_ignore_ascii_case("zip")) {
        return unzip(upload.bytes.clone(), limits, deadline);
    }
    Err(TransferError::invalid(
        "import a markdown file, named .md or .markdown, or a zip of them, named .zip",
    ))
}

/// Reads a zip's markdown files into the pages they make; see the module docs.
fn unzip(bytes: Bytes, limits: &Limits, deadline: Deadline) -> Result<Contents, TransferError> {
    let unreadable = |error: zip::result::ZipError| {
        TransferError::invalid(format!("the zip cannot be read: {error}"))
    };
    let too_many = |listed: u64| {
        TransferError::too_large(format!(
            "the zip lists {listed} files and folders; an import takes at most {}",
            limits.max_entries
        ))
    };
    let max_entries = u64::try_from(limits.max_entries).unwrap_or(u64::MAX);
    // Parsing the directory takes memory for each entry it lists, so count them first.
    let listed = listed_entries(&bytes);
    if listed > max_entries {
        return Err(too_many(listed));
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    if zip.len() > limits.max_entries {
        return Err(too_many(u64::try_from(zip.len()).unwrap_or(u64::MAX)));
    }
    let mut root = Entry::default();
    let mut contents = Contents::default();
    let mut unpacked = 0;
    for i in 0..zip.len() {
        deadline.check()?;
        let entry = zip.by_index_data(i).map_err(unreadable)?;
        let raw = entry.name().map_err(unreadable)?;
        let parts = entry_path(&raw).map_err(|why| {
            TransferError::invalid(format!("{} leads outside the zip: {why}", shown(&raw)))
        })?;
        let Some((file, folders)) = parts.split_last() else {
            continue;
        };
        if is_system(&parts) || entry.is_dir() {
            continue;
        }
        let path = shown(&parts.join("/"));
        if folders.len() > limits.max_depth {
            return Err(TransferError::too_large(format!(
                "{path} is {} folders deep; an import takes at most {}",
                folders.len(),
                limits.max_depth
            )));
        }
        let skip = if parts.iter().any(|part| part.starts_with('.')) {
            Some(SkipReason::Hidden)
        } else if entry.is_symlink() {
            Some(SkipReason::Link)
        } else if entry.encrypted() {
            Some(SkipReason::Encrypted)
        } else if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            Some(SkipReason::Compression)
        } else {
            None
        };
        if let Some(reason) = skip {
            contents.skipped.push(Skipped { path, reason });
            continue;
        }
        if markdown_stem(file).is_none() {
            contents.files.insert(parts.join("/"), i);
            contents.skipped.push(Skipped {
                path,
                reason: SkipReason::Attachment,
            });
            continue;
        }
        let declared = usize::try_from(entry.size()).unwrap_or(usize::MAX);
        let too_large = |path: &str| {
            TransferError::too_large(format!(
                "{path} unpacks to more than {}, the most a markdown file may be",
                size(limits.max_file)
            ))
        };
        let too_much = || {
            TransferError::too_large(format!(
                "the zip's markdown files unpack to more than {}",
                size(limits.max_unpacked)
            ))
        };
        if declared > limits.max_file {
            return Err(too_large(&path));
        }
        if unpacked + declared > limits.max_unpacked {
            return Err(too_much());
        }
        let folders: Vec<String> = folders.iter().map(|part| (*part).to_owned()).collect();
        let stem = markdown_stem(file).unwrap_or(file).to_owned();
        // The size the zip states may be false: what is read is bounded by the limit.
        let mut data = Vec::with_capacity(declared);
        let reader = zip.by_index(i).map_err(unreadable)?;
        reader
            .take(u64::try_from(limits.max_file).unwrap_or(u64::MAX) + 1)
            .read_to_end(&mut data)
            .map_err(|error| {
                TransferError::invalid(format!("{path} cannot be unpacked: {error}"))
            })?;
        if data.len() > limits.max_file {
            return Err(too_large(&path));
        }
        unpacked += data.len();
        if unpacked > limits.max_unpacked {
            return Err(too_much());
        }
        let Ok(markdown) = String::from_utf8(data) else {
            contents.skipped.push(Skipped {
                path,
                reason: SkipReason::NotText,
            });
            continue;
        };
        let mut at = &mut root;
        let mut folder_path = String::new();
        for folder in &folders {
            folder_path.push_str(folder);
            at = at.child(folder, &shown(&folder_path), &mut contents.pages);
            folder_path.push('/');
        }
        let page = at.child(&stem, &path, &mut contents.pages);
        if page.markdown.is_some() {
            contents.skipped.push(Skipped {
                path,
                reason: SkipReason::Duplicate,
            });
            continue;
        }
        page.markdown = Some(markdown);
        page.path = path;
        page.folder = folders;
        if contents.pages > limits.max_pages {
            return Err(TransferError::too_large(format!(
                "the zip makes more than {} pages, the most one import makes",
                limits.max_pages
            )));
        }
    }
    if root.children.is_empty() {
        return Err(TransferError::invalid(
            "the zip holds no markdown file to import",
        ));
    }
    contents.entries = root.children;
    Ok(contents)
}

/// The bytes of a zip's entry kept aside for the pages that show it, at most `max` of them:
/// `None` when it unpacks to more, whatever size the zip states.
pub(super) fn unpack(bytes: Bytes, index: usize, max: u64) -> Result<Option<Bytes>, TransferError> {
    let unreadable = |error: zip::result::ZipError| {
        TransferError::invalid(format!("the zip cannot be read: {error}"))
    };
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    let entry = zip.by_index(index).map_err(unreadable)?;
    if entry.size() > max {
        return Ok(None);
    }
    let path = shown(&entry.name().map_err(unreadable)?);
    let mut data = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    entry
        .take(max.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|error| TransferError::invalid(format!("{path} cannot be unpacked: {error}")))?;
    if u64::try_from(data.len()).unwrap_or(u64::MAX) > max {
        return Ok(None);
    }
    Ok(Some(Bytes::from(data)))
}

/// The most entries any end-of-directory record of a zip says it lists, before its
/// directory is parsed. Readers look for the record in the last 64 KiB and a little, and
/// may try each one they find there, so every one counts. A record of a zip64 archive
/// counts what its zip64 record says, and one whose zip64 record cannot be found counts
/// as listing everything.
fn listed_entries(bytes: &[u8]) -> u64 {
    const END: &[u8] = b"PK\x05\x06";
    const END_LEN: usize = 22;
    const LOCATOR: &[u8] = b"PK\x06\x07";
    const LOCATOR_LEN: usize = 20;
    const END64: &[u8] = b"PK\x06\x06";
    const END64_LEN: usize = 56;
    const WINDOW: usize = END_LEN + (1 << 16);

    let u16_at = |at: usize| u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    let u64_at = |at: usize| {
        bytes
            .get(at..at + 8)
            .and_then(|b| b.try_into().ok())
            .map_or(u64::MAX, u64::from_le_bytes)
    };
    let zip64 = |end: usize| {
        let locator = end.checked_sub(LOCATOR_LEN)?;
        if bytes.get(locator..locator + 4) != Some(LOCATOR) {
            return None;
        }
        let record = usize::try_from(u64_at(locator + 8)).ok()?;
        if record.checked_add(END64_LEN)? > bytes.len() || &bytes[record..record + 4] != END64 {
            return None;
        }
        Some(u64_at(record + 24).max(u64_at(record + 32)))
    };
    let Some(last) = bytes.len().checked_sub(END_LEN) else {
        return 0;
    };
    (last.saturating_sub(WINDOW)..=last)
        .filter(|at| &bytes[*at..*at + 4] == END)
        .map(|at| {
            let listed = u16_at(at + 8).max(u16_at(at + 10));
            if listed == u64::from(u16::MAX) {
                zip64(at).unwrap_or(u64::MAX)
            } else {
                listed
            }
        })
        .max()
        .unwrap_or(0)
}

/// The extension of a file's name, without its dot.
fn extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then_some(ext)
}

/// A markdown file's name without its extension; `None` for any other file.
fn markdown_stem(name: &str) -> Option<&str> {
    let ext = extension(name)?;
    MARKDOWN
        .iter()
        .any(|markdown| ext.eq_ignore_ascii_case(markdown))
        .then(|| &name[..name.len() - ext.len() - 1])
}

/// Whether an entry is an archiver's own.
fn is_system(parts: &[&str]) -> bool {
    parts.contains(&SYSTEM_FOLDER)
        || parts
            .last()
            .is_some_and(|name| SYSTEM_FILES.contains(name) || name.starts_with(RESOURCE_FORK))
}

/// A name from an upload as messages show it: control characters as U+FFFD.
pub(super) fn shown(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    const LATER: Duration = Duration::from_secs(60);

    fn upload(name: &str, bytes: Vec<u8>) -> Upload {
        Upload {
            name: name.to_owned(),
            bytes: Bytes::from(bytes),
        }
    }

    /// A zip of `files`, each a name and its content, deflated.
    fn zipped(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in files {
            if name.ends_with('/') {
                writer
                    .add_directory(*name, SimpleFileOptions::default())
                    .expect("a folder");
            } else {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .expect("a file");
                writer.write_all(content.as_bytes()).expect("written");
            }
        }
        writer.finish().expect("finished").into_inner()
    }

    fn read_zip(files: &[(&str, &str)], limits: &Limits) -> Result<Contents, TransferError> {
        read(
            &upload("notes.zip", zipped(files)),
            limits,
            Deadline::after(LATER),
        )
    }

    /// Each entry as its name, whether it has markdown, and its children, in order.
    fn shape(entries: &[Entry]) -> Vec<(String, bool, usize)> {
        let mut out = Vec::new();
        fn walk(entries: &[Entry], depth: usize, out: &mut Vec<(String, bool, usize)>) {
            for entry in entries {
                out.push((entry.name.clone(), entry.markdown.is_some(), depth));
                walk(&entry.children, depth + 1, out);
            }
        }
        walk(entries, 0, &mut out);
        out
    }

    fn reasons(contents: &Contents) -> Vec<(&str, SkipReason)> {
        contents
            .skipped
            .iter()
            .map(|skipped| (skipped.path.as_str(), skipped.reason))
            .collect()
    }

    #[test]
    fn a_markdown_file_is_one_page() {
        let limits = Limits::default();
        let deadline = Deadline::after(LATER);
        let read = read(
            &upload("Meeting notes.MD", b"# Notes\n".to_vec()),
            &limits,
            deadline,
        )
        .expect("read");
        assert_eq!(
            shape(&read.entries),
            [("Meeting notes".to_owned(), true, 0)]
        );
        assert_eq!(read.pages, 1);
        let picked = super::read(
            &upload("C:\\fakepath\\plan.markdown", b"text".to_vec()),
            &limits,
            deadline,
        )
        .expect("read");
        assert_eq!(picked.entries[0].name, "plan");

        for refused in ["notes.txt", "notes", ".md", "notes.zip.gz"] {
            let error =
                super::read(&upload(refused, b"x".to_vec()), &limits, deadline).expect_err(refused);
            assert_eq!(error.code(), "invalid_input", "{refused}");
        }
        let binary = super::read(&upload("a.md", vec![0xff, 0xfe, 0x00]), &limits, deadline)
            .expect_err("not text");
        assert_eq!(binary.to_string(), "a.md is not text in UTF-8");
        let small = Limits {
            max_file: 3,
            ..Limits::default()
        };
        let large = super::read(&upload("a.md", b"four".to_vec()), &small, deadline)
            .expect_err("too large");
        assert_eq!(large.code(), "too_large");
    }

    #[test]
    fn folders_nest_and_pair_with_their_files() {
        let read = read_zip(
            &[
                ("Roadmap.md", "# Roadmap"),
                ("Roadmap/", ""),
                ("Roadmap/Q1/Goals.md", "goals"),
                ("Roadmap/Q1.md", "q1"),
                ("Roadmap/Risks.markdown", "risks"),
                ("Archive/Old/Plan.md", "plan"),
                ("Welcome.md", "hello"),
            ],
            &Limits::default(),
        )
        .expect("read");
        assert_eq!(
            shape(&read.entries),
            [
                ("Roadmap".to_owned(), true, 0),
                ("Q1".to_owned(), true, 1),
                ("Goals".to_owned(), true, 2),
                ("Risks".to_owned(), true, 1),
                ("Archive".to_owned(), false, 0),
                ("Old".to_owned(), false, 1),
                ("Plan".to_owned(), true, 2),
                ("Welcome".to_owned(), true, 0),
            ]
        );
        assert_eq!(read.pages, 8);
        assert_eq!(read.entries[0].children[0].path, "Roadmap/Q1.md");
        assert_eq!(read.entries[1].path, "Archive");
        assert!(read.skipped.is_empty());
    }

    #[test]
    fn other_files_are_left_out_and_reported() {
        let read = read_zip(
            &[
                ("Notes.md", "notes"),
                ("Notes/diagram.png", "png"),
                ("Notes/.obsidian/app.json", "{}"),
                (".hidden.md", "hidden"),
                ("Assets/logo.svg", "svg"),
                ("__MACOSX/._Notes.md", "fork"),
                ("Notes/.DS_Store", "store"),
                ("._Notes.md", "fork"),
                ("Notes.markdown", "again"),
            ],
            &Limits::default(),
        )
        .expect("read");
        assert_eq!(shape(&read.entries), [("Notes".to_owned(), true, 0)]);
        assert_eq!(
            reasons(&read),
            [
                ("Notes/diagram.png", SkipReason::Attachment),
                ("Notes/.obsidian/app.json", SkipReason::Hidden),
                (".hidden.md", SkipReason::Hidden),
                ("Assets/logo.svg", SkipReason::Attachment),
                ("Notes.markdown", SkipReason::Duplicate),
            ]
        );
    }

    #[test]
    fn entries_that_are_not_text_are_reported() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("latin.md", SimpleFileOptions::default())
            .expect("a file");
        writer.write_all(&[b'c', 0xe9, b'!']).expect("written");
        writer
            .start_file("fine.md", SimpleFileOptions::default())
            .expect("a file");
        writer.write_all(b"fine").expect("written");
        let bytes = writer.finish().expect("finished").into_inner();
        let read = read(
            &upload("a.zip", bytes),
            &Limits::default(),
            Deadline::after(LATER),
        )
        .expect("read");
        assert_eq!(reasons(&read), [("latin.md", SkipReason::NotText)]);
        assert_eq!(read.pages, 1);
    }

    #[test]
    fn paths_out_of_the_zip_refuse_it() {
        for name in [
            "../evil.md",
            "/etc/notes.md",
            "C:\\notes.md",
            "a/../../b.md",
        ] {
            let error =
                read_zip(&[("ok.md", "ok"), (name, "x")], &Limits::default()).expect_err(name);
            assert_eq!(error.code(), "invalid_input", "{name}");
            assert!(
                error.to_string().contains("leads outside the zip"),
                "{error}"
            );
        }
    }

    #[test]
    fn limits_hold() {
        let limits = Limits {
            max_file: 8,
            max_unpacked: 12,
            max_entries: 3,
            max_depth: 2,
            max_pages: 3,
            ..Limits::default()
        };
        let code = |files: &[(&str, &str)]| {
            read_zip(files, &limits)
                .map(|read| read.pages)
                .map_err(|error| error.code())
        };
        assert_eq!(code(&[("a.md", "12345678")]), Ok(1));
        assert_eq!(code(&[("a.md", "123456789")]), Err("too_large"));
        assert_eq!(
            code(&[("a.md", "12345678"), ("b.md", "12345")]),
            Err("too_large")
        );
        assert_eq!(
            code(&[("a.md", ""), ("b.md", ""), ("c.md", ""), ("d.md", "")]),
            Err("too_large")
        );
        assert_eq!(code(&[("a/b/c.md", "")]), Ok(3));
        assert_eq!(code(&[("a/b/c/d.md", "")]), Err("too_large"));
        assert_eq!(code(&[("a/b.md", ""), ("c/d.md", "")]), Err("too_large"));
        assert_eq!(
            code(&[("a.png", "123456789123456789")]),
            Err("invalid_input")
        );

        let many = Limits {
            max_file: 1024,
            max_unpacked: 2048,
            ..Limits::default()
        };
        let big = "x".repeat(1025);
        let error = read_zip(&[("big.md", &big)], &many).expect_err("too large");
        assert_eq!(
            error.to_string(),
            "big.md unpacks to more than 1 KiB, the most a markdown file may be"
        );
    }

    #[test]
    fn a_zip_that_lies_about_sizes_is_still_bounded() {
        let mut bytes = zipped(&[("a.md", &"x".repeat(4096))]);
        // The central directory's uncompressed size of the one entry, 4 bytes at offset 24
        // of its header, says 1 byte.
        let header = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x01\x02")
            .expect("a central header");
        bytes[header + 24..header + 28].copy_from_slice(&1u32.to_le_bytes());
        let limits = Limits {
            max_file: 1024,
            ..Limits::default()
        };
        let error =
            read(&upload("a.zip", bytes), &limits, Deadline::after(LATER)).expect_err("bounded");
        assert!(
            matches!(error.code(), "too_large" | "invalid_input"),
            "{error}"
        );
    }

    #[test]
    fn entries_are_counted_before_the_directory_is_parsed() {
        let three = zipped(&[("a.md", "a"), ("b.md", "b"), ("c.md", "c")]);
        assert_eq!(listed_entries(&three), 3);
        assert_eq!(listed_entries(b"PK"), 0);
        assert_eq!(listed_entries(&[]), 0);

        // A record that claims many more entries than the zip holds refuses it at once.
        let mut claiming = three.clone();
        let end = claiming.len() - 22;
        claiming[end + 8..end + 12].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        assert_eq!(listed_entries(&claiming), 10_000);
        let limits = Limits {
            max_entries: 5,
            ..Limits::default()
        };
        let error = read(&upload("a.zip", claiming), &limits, Deadline::after(LATER))
            .expect_err("too many");
        assert_eq!(
            error.to_string(),
            "the zip lists 10000 files and folders; an import takes at most 5"
        );

        // So does a zip64 marker without the zip64 record it points to.
        let mut zip64 = three.clone();
        zip64[end + 8..end + 12].copy_from_slice(&[0xff; 4]);
        assert_eq!(listed_entries(&zip64), u64::MAX);

        // And a second whole record, in the comment, counts as much as the last.
        let mut record = b"PK\x05\x06\0\0\0\0\xff\x7f\xff\x7f".to_vec();
        record.resize(22, 0);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .set_raw_comment(record.into_boxed_slice())
            .expect("a comment");
        writer
            .start_file("a.md", SimpleFileOptions::default())
            .expect("a file");
        let commented = writer.finish().expect("finished").into_inner();
        assert_eq!(listed_entries(&commented), 0x7fff);
    }

    #[test]
    fn what_is_not_a_zip_or_holds_no_markdown_is_refused() {
        let deadline = Deadline::after(LATER);
        let limits = Limits::default();
        let error = read(&upload("a.zip", b"not a zip".to_vec()), &limits, deadline)
            .expect_err("not a zip");
        assert!(
            error.to_string().starts_with("the zip cannot be read"),
            "{error}"
        );
        let empty = read_zip(&[("photo.jpg", "jpg"), ("folder/", "")], &limits).expect_err("empty");
        assert_eq!(
            empty.to_string(),
            "the zip holds no markdown file to import"
        );
    }

    #[test]
    fn reading_stops_at_the_deadline() {
        let error = read(
            &upload("a.zip", zipped(&[("a.md", "a")])),
            &Limits::default(),
            Deadline {
                at: Instant::now() - Duration::from_millis(1),
                limit: Duration::from_secs(20),
            },
        )
        .expect_err("too late");
        assert_eq!(error.code(), "timeout");
    }
}
