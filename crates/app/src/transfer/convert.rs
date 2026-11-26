//! Markdown in and out: pages from an upload's files, each with its document encoded, and a
//! page's markdown file from its title and blocks.
//!
//! A file's title is its first block, when that is a heading, which then leaves the
//! content; otherwise the file's name. An export writes the title as that heading, so
//! exporting a page and importing it gives the same title back. A block of YAML front
//! matter at the very top of a file, as static site generators and note apps write, is
//! left out.

use neon_blocks::markdown::{from_markdown, to_markdown};
use neon_blocks::{
    Block, BlockKind, Document, HeadingProps, Inline, TableCell, TableContent, TableProps,
    TableRow, TextProps,
};
use std::collections::HashMap;

use neon_domain::attachments::{File, FileKind};
use neon_domain::pages::files::clean_title;

use super::archive::Entry;
use super::{Deadline, TransferError, size};
use crate::databases::{ExportRow, ExportTable};
use crate::pages::{ImportedContent, ImportedPage};

/// A page of an upload, its markdown read as blocks, before the files it shows are found.
#[derive(Debug)]
pub(super) struct Parsed {
    title: String,
    /// Where it is in the upload, for messages.
    path: String,
    /// The folders its markdown file is in, which its links start from.
    folder: Vec<String>,
    document: Option<Document>,
    children: Vec<Parsed>,
}

/// The pages of an upload's entries, in order, each with the pages under it, their
/// markdown read as blocks. Past `deadline`, it stops.
pub(super) fn parse(entries: Vec<Entry>, deadline: Deadline) -> Result<Vec<Parsed>, TransferError> {
    entries
        .into_iter()
        .map(|entry| parse_entry(entry, deadline))
        .collect()
}

fn parse_entry(entry: Entry, deadline: Deadline) -> Result<Parsed, TransferError> {
    deadline.check()?;
    let (title, document) = match &entry.markdown {
        Some(markdown) => {
            let mut document = from_markdown(body(markdown));
            let title = take_title(&mut document).unwrap_or_else(|| clean_title(&entry.name));
            (title, Some(document))
        }
        None => (clean_title(&entry.name), None),
    };
    Ok(Parsed {
        title,
        document,
        children: entry
            .children
            .into_iter()
            .map(|child| parse_entry(child, deadline))
            .collect::<Result<_, _>>()?,
        path: entry.path,
        folder: entry.folder,
    })
}

/// A file of an upload a page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Link {
    /// Its path in the zip.
    pub(super) path: String,
    /// The kind of the first block that shows it.
    pub(super) kind: FileKind,
    /// The name a file, video or audio block gives it, or else the file's own: an image's
    /// is its alternative text.
    pub(super) name: String,
    /// The page that shows it first, which it belongs to: by its place among the pages,
    /// parents before their children.
    pub(super) owner: usize,
}

/// The files of `files`, paths in the zip, that the pages' blocks link to by a path
/// relative to their markdown file, once each, in the order the pages show them.
pub(super) fn links(pages: &[Parsed], files: &HashMap<String, usize>) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    let mut index = 0;
    preorder(pages, &mut |page| {
        if let Some(document) = &page.document {
            for (block, url) in document.files() {
                let Some(path) = resolve(&page.folder, url).filter(|path| files.contains_key(path))
                else {
                    continue;
                };
                if links.iter().any(|link| link.path == path) {
                    continue;
                }
                let own = path.rsplit('/').next().unwrap_or_default().to_owned();
                let named = block_name(&block.kind).filter(|name| !name.trim().is_empty());
                links.push(Link {
                    name: named.map_or(own, str::to_owned),
                    kind: kind_of(&block.kind),
                    owner: index,
                    path,
                });
            }
        }
        index += 1;
    });
    links
}

fn preorder(pages: &[Parsed], visit: &mut dyn FnMut(&Parsed)) {
    for page in pages {
        visit(page);
        preorder(&page.children, visit);
    }
}

/// The kind of file a block shows.
fn kind_of(kind: &BlockKind) -> FileKind {
    match kind {
        BlockKind::Image { .. } => FileKind::Image,
        BlockKind::Video { .. } => FileKind::Video,
        BlockKind::Audio { .. } => FileKind::Audio,
        _ => FileKind::File,
    }
}

/// The file's name a block gives the file it shows: not an image's, whose name is its
/// alternative text.
fn block_name(kind: &BlockKind) -> Option<&str> {
    match kind {
        BlockKind::Video { props } => Some(&props.name),
        BlockKind::Audio { props } => Some(&props.name),
        BlockKind::File { props } => Some(&props.name),
        _ => None,
    }
}

/// The pages to import, each with its document encoded and the pages under it: each link
/// to a file of `made`, by its path in the zip, becomes the path of the file made of it,
/// which goes with the page that showed it first, by its place among the pages. A document
/// that would be larger than `max_document` bytes refuses the import. Past `deadline`, it
/// stops.
pub(super) fn finish(
    pages: Vec<Parsed>,
    made: &HashMap<String, (usize, File)>,
    max_document: usize,
    deadline: Deadline,
) -> Result<Vec<ImportedPage>, TransferError> {
    let mut index = 0;
    pages
        .into_iter()
        .map(|page| finish_page(page, made, max_document, deadline, &mut index))
        .collect()
}

fn finish_page(
    page: Parsed,
    made: &HashMap<String, (usize, File)>,
    max_document: usize,
    deadline: Deadline,
    index: &mut usize,
) -> Result<ImportedPage, TransferError> {
    deadline.check()?;
    let at = *index;
    *index += 1;
    let content = match page.document {
        Some(mut document) => {
            document.map_files(|_, url| {
                resolve(&page.folder, url)
                    .and_then(|path| made.get(&path))
                    .map(|(_, file)| file.path())
            });
            encode(&document, max_document, &page.path)?
        }
        None => None,
    };
    let files = made
        .values()
        .filter(|(owner, _)| *owner == at)
        .map(|(_, file)| file.clone())
        .collect();
    let children = page
        .children
        .into_iter()
        .map(|child| finish_page(child, made, max_document, deadline, index))
        .collect::<Result<_, _>>()?;
    Ok(ImportedPage {
        title: page.title,
        content,
        files,
        children,
    })
}

/// The path in the zip a link of a markdown file in `folder` leads to: relative, with `.`
/// and `..` taken as folders do and percent-encoding undone, its query and fragment left
/// out. `None` for a link that is not relative or climbs out of the zip.
pub(super) fn resolve(folder: &[String], url: &str) -> Option<String> {
    if url.is_empty() || url.starts_with(['/', '\\', '#', '?']) || url.contains(':') {
        return None;
    }
    let url = url.split(['?', '#']).next()?;
    let decoded = percent_decode(url)?;
    let mut parts: Vec<&str> = folder.iter().map(String::as_str).collect();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| char::from(*b).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high * 16 + low).ok()?);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// A path in the zip as a link from a markdown file in `folder`: up out of the folders the
/// two do not share, then down, each part percent-encoded but for ASCII letters, digits,
/// `-`, `.`, `_` and `~`, so any renderer reads it as one link.
pub(super) fn relative(folder: &[&str], path: &str) -> String {
    let target: Vec<&str> = path.split('/').collect();
    let (dirs, _) = target.split_at(target.len().saturating_sub(1));
    let shared = folder.iter().zip(dirs).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_owned(); folder.len() - shared];
    parts.extend(target[shared..].iter().map(|part| encode(part)));
    return parts.join("/");

    fn encode(part: &str) -> String {
        let mut out = String::with_capacity(part.len());
        for byte in part.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
        out
    }
}

/// A file's markdown without a byte order mark and front matter in front of it.
fn body(markdown: &str) -> &str {
    without_front_matter(markdown.strip_prefix('\u{feff}').unwrap_or(markdown))
}

/// The text after a block of YAML front matter at its very top: a line `---`, a first line
/// of the form `key: value`, and the lines up to a line `---` or `...`. A thematic break at
/// the top of a file is followed by a blank line or text, so it stays.
fn without_front_matter(text: &str) -> &str {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return text;
    };
    let first = rest.lines().next().unwrap_or_default();
    let key = first.split_once(':').is_some_and(|(key, value)| {
        !key.is_empty()
            && !key.starts_with(char::is_whitespace)
            && !key.contains(char::is_whitespace)
            && (value.is_empty() || value.starts_with(char::is_whitespace))
    });
    if !key {
        return text;
    }
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        if line == "---" || line == "..." {
            return &rest[offset..];
        }
    }
    text
}

/// Takes the document's first block out as the title, when it is a heading with text that
/// neither folds nor has blocks under it.
fn take_title(document: &mut Document) -> Option<String> {
    let first = document.blocks.first()?;
    let BlockKind::Heading { props, .. } = &first.kind else {
        return None;
    };
    if props.is_toggleable || !first.children.is_empty() {
        return None;
    }
    let title = clean_title(&Document::new(vec![first.clone()]).plain_text());
    if title.is_empty() {
        return None;
    }
    document.blocks.remove(0);
    Some(title)
}

/// A document as the first snapshot of a page's: written into a Yjs document as the editor
/// would, each block given a fresh id. `None` for an empty document, which a page without a
/// snapshot has already.
fn encode(
    document: &Document,
    max_document: usize,
    path: &str,
) -> Result<Option<ImportedContent>, TransferError> {
    let Some(content) = ImportedContent::of(document) else {
        return Ok(None);
    };
    if content.state.len() > max_document {
        return Err(TransferError::too_large(format!(
            "{path} makes a document larger than {}, the most a page holds",
            size(max_document)
        )));
    }
    Ok(Some(content))
}

/// What a page's file holds besides its title and blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Extra {
    None,
    /// A database's rows, after its blocks, as a table whose first row names the columns.
    Table(ExportTable),
    /// A row's properties, under its title, as a list of `name: value`.
    Properties(ExportRow),
}

/// A page's markdown file: its title as a heading, when it has one, a row's properties,
/// its blocks, then a database's table.
pub(super) fn markdown(title: &str, mut document: Document, extra: &Extra) -> String {
    let mut head = Vec::new();
    if !title.is_empty() {
        head.push(Block::new(BlockKind::Heading {
            props: HeadingProps::default(),
            content: Inline::plain(title),
        }));
    }
    match extra {
        Extra::None => {}
        Extra::Properties(row) => {
            head.extend(row.properties.iter().map(|(name, value)| {
                Block::new(BlockKind::BulletListItem {
                    props: TextProps::default(),
                    content: Inline::plain(format!("{name}: {value}")),
                })
            }));
        }
        Extra::Table(table) => document.blocks.push(table_block(table)),
    }
    document.blocks.splice(0..0, head);
    let mut markdown = to_markdown(&document);
    if !markdown.is_empty() && !markdown.ends_with('\n') {
        markdown.push('\n');
    }
    markdown
}

/// A table block of a database's rows, its first row the columns' names.
fn table_block(table: &ExportTable) -> Block {
    let row = |cells: &[String]| TableRow {
        cells: cells
            .iter()
            .map(|text| TableCell {
                content: Inline::plain(text.as_str()),
                ..TableCell::default()
            })
            .collect(),
    };
    let mut rows = vec![row(&table.columns)];
    rows.extend(table.rows.iter().map(|cells| row(cells)));
    Block::new(BlockKind::Table {
        props: TableProps::default(),
        content: TableContent {
            column_widths: vec![None; table.columns.len()],
            header_rows: Some(1),
            header_cols: None,
            rows,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use neon_blocks::HeadingProps;
    use uuid::Uuid;
    use yrs::updates::decoder::Decode as _;
    use yrs::updates::encoder::Encode as _;
    use yrs::{Doc, ReadTxn as _, Transact as _, Update};

    use super::*;

    fn entry(name: &str, markdown: Option<&str>, children: Vec<Entry>) -> Entry {
        Entry {
            name: name.to_owned(),
            path: format!("{name}.md"),
            markdown: markdown.map(str::to_owned),
            children,
            ..Entry::default()
        }
    }

    fn pages(
        entries: Vec<Entry>,
        max_document: usize,
        deadline: Deadline,
    ) -> Result<Vec<ImportedPage>, TransferError> {
        finish(
            parse(entries, deadline)?,
            &HashMap::new(),
            max_document,
            deadline,
        )
    }

    fn imported(entries: Vec<Entry>) -> Vec<ImportedPage> {
        pages(entries, 1 << 24, Deadline::after(Duration::from_secs(60))).expect("converted")
    }

    /// The blocks a page's snapshot holds.
    fn blocks(content: &ImportedContent) -> Document {
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&content.state).expect("decodes"))
            .expect("applies");
        neon_blocks::read_doc(&doc).expect("reads as blocks")
    }

    fn without_ids(mut document: Document) -> Document {
        fn clear(blocks: &mut [Block]) {
            for block in blocks {
                block.id = None;
                clear(&mut block.children);
            }
        }
        clear(&mut document.blocks);
        document
    }

    #[test]
    fn a_leading_heading_is_the_title() {
        let pages = imported(vec![
            entry(
                "file name",
                Some("# Road *map*\n\nFirst steps.\n"),
                Vec::new(),
            ),
            entry("Plain", Some("Just text.\n"), Vec::new()),
            entry("Second", Some("## Level two\n\ntext\n"), Vec::new()),
            entry(
                "Folded",
                Some(":::toggle # Folded heading\n\ninside\n\n:::\n"),
                Vec::new(),
            ),
            entry("Empty", Some("#\n\ntext\n"), Vec::new()),
            entry("Folder", None, vec![entry("Child", Some(""), Vec::new())]),
        ]);
        let titles: Vec<&str> = pages.iter().map(|page| page.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Road map",
                "Plain",
                "Level two",
                "Folded",
                "Empty",
                "Folder"
            ]
        );
        let first = pages[0].content.as_ref().expect("content");
        assert_eq!(
            without_ids(blocks(first)),
            Document::new(vec![Block::paragraph("First steps.")])
        );
        assert_eq!(first.text, "First steps.");
        assert!(pages[3].content.is_some());
        assert!(pages[5].content.is_none());
        assert_eq!(pages[5].children[0].title, "Child");
        assert!(pages[5].children[0].content.is_none());
    }

    #[test]
    fn front_matter_is_left_out() {
        assert_eq!(
            body("---\ntitle: Notes\ntags: [a]\n---\n# Notes\n"),
            "# Notes\n"
        );
        assert_eq!(body("\u{feff}---\r\nkey:\r\n...\r\nrest"), "rest");
        for kept in [
            "---\n\ntext\n\n---\n",
            "---\nnot: closed\n",
            "---\n key: indented\n---\n",
            "---\nhttps://example.com\n---\n",
            "text\n---\nkey: value\n---\n",
        ] {
            assert_eq!(body(kept), kept, "{kept:?}");
        }
    }

    #[test]
    fn pages_get_their_blocks_and_fresh_ids() {
        let pages = imported(vec![entry(
            "Doc",
            Some("# Doc\n\n- one\n  - nested\n\n```rust\nfn main() {}\n```\n"),
            Vec::new(),
        )]);
        let content = pages[0].content.as_ref().expect("content");
        let document = blocks(content);
        let ids: Vec<&str> = document
            .walk()
            .map(|block| block.id.as_deref().expect("an id"))
            .collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().all(|id| Uuid::try_parse(id).is_ok()), "{ids:?}");
        assert_eq!(content.text, "one\nnested\nfn main() {}");
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&content.state).expect("decodes"))
            .expect("applies");
        assert_eq!(
            doc.transact().state_vector().encode_v1(),
            content.state_vector
        );
    }

    #[test]
    fn documents_past_the_limit_refuse_the_import() {
        let error = pages(
            vec![entry("Big", Some(&"word ".repeat(100)), Vec::new())],
            64,
            Deadline::after(Duration::from_secs(60)),
        )
        .expect_err("too large");
        assert_eq!(
            error.to_string(),
            "Big.md makes a document larger than 64 B, the most a page holds"
        );
    }

    #[test]
    fn links_are_paths_from_their_markdown_file() {
        let folder = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>()
        };
        let roadmap = folder(&["Roadmap"]);
        assert_eq!(
            resolve(&roadmap, "diagram.png").as_deref(),
            Some("Roadmap/diagram.png")
        );
        assert_eq!(
            resolve(&roadmap, "Q1/My%20chart%20%28v2%29.png").as_deref(),
            Some("Roadmap/Q1/My chart (v2).png")
        );
        assert_eq!(
            resolve(&roadmap, "./a/../b.pdf").as_deref(),
            Some("Roadmap/b.pdf")
        );
        assert_eq!(resolve(&roadmap, "../top.png").as_deref(), Some("top.png"));
        assert_eq!(
            resolve(&roadmap, "x.png?v=2#frag").as_deref(),
            Some("Roadmap/x.png")
        );
        assert_eq!(resolve(&[], "plan/y.png").as_deref(), Some("plan/y.png"));
        assert_eq!(resolve(&[], "%E6%97%A5.png").as_deref(), Some("日.png"));
        for away in [
            "../../out.png",
            "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1",
            "https://example.com/a.png",
            "data:image/png;base64,AAAA",
            "C:/disk.png",
            "#anchor",
            "",
            "..",
            "%FF.png",
        ] {
            assert_eq!(resolve(&roadmap, away), None, "{away}");
        }
    }

    #[test]
    fn exported_links_lead_from_the_file_to_the_attachment() {
        assert_eq!(relative(&[], "Roadmap/diagram.png"), "Roadmap/diagram.png");
        assert_eq!(
            relative(&["Roadmap"], "Roadmap/Q1/My chart (v2).png"),
            "Q1/My%20chart%20%28v2%29.png"
        );
        assert_eq!(
            relative(&["Roadmap", "Q1"], "Roadmap/top.png"),
            "../top.png"
        );
        assert_eq!(relative(&["A", "B"], "C/D/x.pdf"), "../../C/D/x.pdf");
        assert_eq!(relative(&[], "日.png"), "%E6%97%A5.png");
        for (folder, path) in [
            (vec!["Roadmap"], "Roadmap/Q1/My chart (v2).png"),
            (vec!["A", "B"], "C/D/x.pdf"),
            (vec![], "日.png"),
        ] {
            let owned: Vec<String> = folder.iter().map(|part| (*part).to_owned()).collect();
            assert_eq!(
                resolve(&owned, &relative(&folder, path)).as_deref(),
                Some(path),
                "{path}"
            );
        }
    }

    #[test]
    fn files_go_with_the_page_that_shows_them_first() {
        let shows = |links: &str| format!("# Page\n\n{links}\n");
        let mut child = entry(
            "Child",
            Some(&shows("![a](../a.png)\n\n:::file [Named](b.pdf)")),
            Vec::new(),
        );
        child.folder = vec!["Top".to_owned()];
        let top = entry(
            "Top",
            Some(&shows("![first](a.png)\n\n![](https://example.com/x.png)")),
            vec![child],
        );
        let files: HashMap<String, usize> = [("a.png", 1), ("Top/b.pdf", 2), ("unused.txt", 3)]
            .into_iter()
            .map(|(path, index)| (path.to_owned(), index))
            .collect();
        let parsed = parse(vec![top], Deadline::after(Duration::from_secs(60))).expect("parsed");
        assert_eq!(
            links(&parsed, &files),
            [
                Link {
                    path: "a.png".to_owned(),
                    kind: FileKind::Image,
                    name: "a.png".to_owned(),
                    owner: 0,
                },
                Link {
                    path: "Top/b.pdf".to_owned(),
                    kind: FileKind::File,
                    name: "Named".to_owned(),
                    owner: 1,
                },
            ]
        );
    }

    #[test]
    fn exports_write_the_title_as_a_heading() {
        let document = Document::new(vec![Block::paragraph("Body *text*")]);
        assert_eq!(
            markdown("Plan: *v2*", document.clone(), &Extra::None),
            "# Plan: \\*v2\\*\n\nBody \\*text\\*\n"
        );
        assert_eq!(markdown("", document, &Extra::None), "Body \\*text\\*\n");
        assert_eq!(markdown("", Document::default(), &Extra::None), "");
        let heading = Block::new(BlockKind::Heading {
            props: HeadingProps {
                level: 2,
                ..HeadingProps::default()
            },
            content: Inline::plain("Part"),
        });
        let paragraph = Block::new(BlockKind::Paragraph {
            props: TextProps::default(),
            content: Inline::plain("x"),
        });
        let again = markdown(
            "Title",
            Document::new(vec![heading, paragraph]),
            &Extra::None,
        );
        let mut read = from_markdown(&again);
        assert_eq!(take_title(&mut read).as_deref(), Some("Title"));
        assert_eq!(to_markdown(&read), "## Part\n\nx\n");
    }

    #[test]
    fn a_database_ends_with_its_table_and_a_row_lists_its_properties() {
        let table = ExportTable {
            columns: vec!["Name".into(), "Status".into()],
            rows: vec![
                vec!["Fix | login".into(), "Done".into()],
                vec!["".into(), "".into()],
            ],
        };
        let document = Document::new(vec![Block::paragraph("About the tasks")]);
        assert_eq!(
            markdown("Tasks", document, &Extra::Table(table)),
            "# Tasks\n\nAbout the tasks\n\n| Name | Status |\n| --- | --- |\n| Fix \\| login | Done |\n|  |  |\n"
        );
        let row = ExportRow {
            properties: vec![
                ("Status".into(), "Done".into()),
                ("Owner".into(), "Ada".into()),
            ],
        };
        let document = Document::new(vec![Block::paragraph("Steps")]);
        assert_eq!(
            markdown("Fix login", document, &Extra::Properties(row)),
            "# Fix login\n\n- Status: Done\n- Owner: Ada\n\nSteps\n"
        );
    }
}
