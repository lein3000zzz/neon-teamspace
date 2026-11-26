//! Writing an export's zip, in memory: each page's markdown file named after its title, and
//! the pages under a page, with the files its blocks show, in a folder of the same name
//! beside its file, as an import reads them back.

use std::collections::HashMap;
use std::io::{Cursor, Write as _};

use neon_domain::attachments::{File, MediaType};
use neon_domain::pages::files::{Names, file_name};
use time::OffsetDateTime;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use super::{TransferError, size};
use crate::pages::OutlinePage;

/// An export's zip as it is written, parents before the pages under them.
pub(super) struct ZipExport {
    writer: ZipWriter<Cursor<Vec<u8>>>,
    /// The folder of each page written, where the pages under it go, with the names given
    /// there so far.
    folders: HashMap<Uuid, (String, Names)>,
    /// The names given at the top of the zip.
    top: Names,
    /// The markdown written so far, and the most there may be.
    bytes: usize,
    max_bytes: usize,
}

/// Where a page's markdown file goes in the zip.
pub(super) struct Placed {
    /// The file's path.
    file: String,
    /// The folders it is in, which its links start from.
    pub(super) folder: Vec<String>,
    /// When the page's content last changed, which dates its file.
    changed: OffsetDateTime,
}

impl ZipExport {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            writer: ZipWriter::new(Cursor::new(Vec::new())),
            folders: HashMap::new(),
            top: Names::default(),
            bytes: 0,
            max_bytes,
        }
    }

    /// Adds a page's file, in the folder of the page it is under when that was added, else
    /// at the top.
    #[cfg(test)]
    pub(super) fn add(&mut self, page: &OutlinePage, markdown: &str) -> Result<(), TransferError> {
        let placed = self.place(page);
        self.write(&placed, markdown)
    }

    /// Names a page's file, in the folder of the page it is under when that was placed, else
    /// at the top, and gives it a folder of its own for the pages under it and its files.
    pub(super) fn place(&mut self, page: &OutlinePage) -> Placed {
        let (folder, names) = match page.parent_id.and_then(|id| self.folders.get_mut(&id)) {
            Some((folder, names)) => (folder.clone(), names),
            None => (String::new(), &mut self.top),
        };
        let name = names.claim(&file_name(&page.title));
        self.folders
            .insert(page.id, (format!("{folder}{name}/"), Names::default()));
        Placed {
            file: format!("{folder}{name}.md"),
            folder: folder
                .split('/')
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect(),
            changed: page.content_updated_at,
        }
    }

    /// Adds a file a page shows to the page's folder, named after it, and gives its path in
    /// the zip. Place the page first.
    pub(super) fn attach(
        &mut self,
        page_id: Uuid,
        file: &File,
        bytes: &[u8],
    ) -> Result<String, TransferError> {
        let (folder, names) = self
            .folders
            .get_mut(&page_id)
            .ok_or_else(|| TransferError::Internal("a file of a page not placed".to_owned()))?;
        let path = format!("{folder}{}", names.claim_file(&file.name));
        let options = options(file.created_at).compression_method(if compressed(file.media) {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        });
        self.writer
            .start_file(path.as_str(), options)
            .and_then(|()| Ok(self.writer.write_all(bytes)?))
            .map_err(|error| TransferError::Internal(format!("writing the zip: {error}")))?;
        Ok(path)
    }

    /// Writes a placed page's markdown file.
    pub(super) fn write(&mut self, placed: &Placed, markdown: &str) -> Result<(), TransferError> {
        self.bytes += markdown.len();
        if self.bytes > self.max_bytes {
            return Err(TransferError::too_large(format!(
                "the pages hold more than {} of markdown, the most one export holds: export \
                 fewer at once",
                size(self.max_bytes)
            )));
        }
        self.writer
            .start_file(placed.file.as_str(), options(placed.changed))
            .and_then(|()| Ok(self.writer.write_all(markdown.as_bytes())?))
            .map_err(|error| TransferError::Internal(format!("writing the zip: {error}")))
    }

    /// The zip's bytes.
    pub(super) fn finish(self) -> Result<Vec<u8>, TransferError> {
        self.writer
            .finish()
            .map(Cursor::into_inner)
            .map_err(|error| TransferError::Internal(format!("writing the zip: {error}")))
    }
}

/// Whether a file's bytes are compressed already, as media and archives are, so deflating
/// them again gains nothing.
fn compressed(media: MediaType) -> bool {
    use MediaType as M;
    !matches!(
        media,
        M::Svg | M::Html | M::Xml | M::Text | M::Unknown | M::Wav
    )
}

/// A file deflated, readable by everyone, dated when its page's content last changed, as
/// near as a zip's two-second clock goes. Pages older than a zip's dates, 1980, are dated
/// 1980.
fn options(changed: OffsetDateTime) -> SimpleFileOptions {
    let at = changed.to_offset(time::UtcOffset::UTC);
    let date = u16::try_from(at.year())
        .ok()
        .and_then(|year| {
            DateTime::from_date_and_time(
                year,
                u8::from(at.month()),
                at.day(),
                at.hour(),
                at.minute(),
                at.second(),
            )
            .ok()
        })
        .unwrap_or_default();
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(date)
        .unix_permissions(0o644)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use zip::ZipArchive;

    use super::*;

    fn page(id: u128, parent: Option<u128>, title: &str) -> OutlinePage {
        OutlinePage {
            id: Uuid::from_u128(id),
            kind: neon_domain::pages::PageKind::Page,
            parent_id: parent.map(Uuid::from_u128),
            title: title.to_owned(),
            content_updated_at: OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a time"),
        }
    }

    fn files(bytes: Vec<u8>) -> Vec<(String, String)> {
        let mut zip = ZipArchive::new(Cursor::new(bytes)).expect("a zip");
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).expect("an entry");
                let mut text = String::new();
                file.read_to_string(&mut text).expect("text");
                (file.name().expect("a name").into_owned(), text)
            })
            .collect()
    }

    #[test]
    fn pages_go_in_their_parents_folders() {
        let mut export = ZipExport::new(1 << 20);
        for (page, text) in [
            (page(1, None, "Roadmap"), "# Roadmap\n"),
            (page(2, Some(1), "Q1: Goals"), "# Q1: Goals\n"),
            (page(3, Some(2), "Hiring"), "# Hiring\n"),
            (page(4, Some(1), "q1 goals"), "# q1 goals\n"),
            (page(5, Some(9), "Shared"), "# Shared\n"),
            (page(6, None, ""), ""),
            (page(7, None, "Roadmap.md"), "# Roadmap.md\n"),
        ] {
            export.add(&page, text).expect("added");
        }
        let written = files(export.finish().expect("finished"));
        let names: Vec<&str> = written.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Roadmap.md",
                "Roadmap/Q1 Goals.md",
                "Roadmap/Q1 Goals/Hiring.md",
                "Roadmap/q1 goals (2).md",
                "Shared.md",
                "Untitled.md",
                "Roadmap.md (2).md",
            ]
        );
        assert_eq!(written[1].1, "# Q1: Goals\n");
    }

    #[test]
    fn files_go_in_their_page_s_folder_beside_its_pages() {
        let mut export = ZipExport::new(1 << 20);
        let roadmap = page(1, None, "Roadmap");
        let placed = export.place(&roadmap);
        assert!(placed.folder.is_empty());
        let file = |name: &str, media| File {
            id: Uuid::from_u128(9),
            workspace_id: Uuid::from_u128(8),
            page_id: roadmap.id,
            uploaded_by: None,
            kind: neon_domain::attachments::FileKind::File,
            key: "k".to_owned(),
            name: name.to_owned(),
            media,
            size: 3,
            sha256: [0; 32],
            dimensions: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
        };
        assert_eq!(
            export
                .attach(roadmap.id, &file("diagram.png", MediaType::Png), b"png")
                .expect("attached"),
            "Roadmap/diagram.png"
        );
        assert_eq!(
            export
                .attach(roadmap.id, &file("notes.md", MediaType::Text), b"txt")
                .expect("attached"),
            "Roadmap/notes.md.txt"
        );
        export.write(&placed, "# Roadmap\n").expect("written");
        let child = export.place(&page(2, Some(1), "diagram.png"));
        assert_eq!(child.folder, ["Roadmap"]);
        export.write(&child, "# child\n").expect("written");
        assert!(
            export
                .attach(Uuid::from_u128(7), &file("x.png", MediaType::Png), b"x")
                .is_err(),
            "a page not placed has no folder"
        );
        let names: Vec<String> = files(export.finish().expect("finished"))
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [
                "Roadmap/diagram.png",
                "Roadmap/notes.md.txt",
                "Roadmap.md",
                "Roadmap/diagram.png (2).md"
            ]
        );
    }

    #[test]
    fn exports_stop_at_their_size() {
        let mut export = ZipExport::new(10);
        export.add(&page(1, None, "a"), "12345").expect("fits");
        let error = export
            .add(&page(2, None, "b"), "123456")
            .expect_err("too large");
        assert_eq!(error.code(), "too_large");
    }

    #[test]
    fn files_are_dated_by_their_content() {
        let mut export = ZipExport::new(1 << 20);
        export.add(&page(1, None, "a"), "a").expect("added");
        let mut old = page(2, None, "b");
        old.content_updated_at = OffsetDateTime::UNIX_EPOCH;
        export.add(&old, "b").expect("added");
        let mut zip =
            ZipArchive::new(Cursor::new(export.finish().expect("finished"))).expect("a zip");
        let dated = zip
            .by_index(0)
            .expect("an entry")
            .last_modified()
            .expect("a date");
        assert_eq!(
            (dated.year(), dated.month(), dated.day(), dated.hour()),
            (2026, 11, 22, 12)
        );
        let old = zip
            .by_index(1)
            .expect("an entry")
            .last_modified()
            .expect("a date");
        assert_eq!(old.year(), 1980);
    }
}
