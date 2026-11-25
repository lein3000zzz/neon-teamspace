//! Writing an export's zip, in memory: each page's markdown file named after its title, and
//! the pages under a page in a folder of the same name beside its file, as an import reads
//! them back.

use std::collections::HashMap;
use std::io::{Cursor, Write as _};

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
    pub(super) fn add(&mut self, page: &OutlinePage, markdown: &str) -> Result<(), TransferError> {
        self.bytes += markdown.len();
        if self.bytes > self.max_bytes {
            return Err(TransferError::too_large(format!(
                "the pages hold more than {} of markdown, the most one export holds: export \
                 fewer at once",
                size(self.max_bytes)
            )));
        }
        let (folder, names) = match page.parent_id.and_then(|id| self.folders.get_mut(&id)) {
            Some((folder, names)) => (folder.as_str(), names),
            None => ("", &mut self.top),
        };
        let name = names.claim(&file_name(&page.title));
        let file = format!("{folder}{name}.md");
        let own = format!("{folder}{name}/");
        self.folders.insert(page.id, (own, Names::default()));
        self.writer
            .start_file(file, options(page.content_updated_at))
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
