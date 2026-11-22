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
use neon_domain::pages::files::clean_title;
use uuid::Uuid;
use yrs::updates::encoder::Encode as _;
use yrs::{Doc, ReadTxn as _, StateVector, Transact as _};

use super::archive::Entry;
use super::{Deadline, TransferError, size};
use crate::databases::{ExportRow, ExportTable};
use crate::pages::{ImportedContent, ImportedPage};

/// The pages to import from an upload's entries, in order, each with the pages under it.
/// A document that would be larger than `max_document` bytes refuses the import. Past
/// `deadline`, it stops.
pub(super) fn pages(
    entries: Vec<Entry>,
    max_document: usize,
    deadline: Deadline,
) -> Result<Vec<ImportedPage>, TransferError> {
    entries
        .into_iter()
        .map(|entry| page(entry, max_document, deadline))
        .collect()
}

fn page(
    entry: Entry,
    max_document: usize,
    deadline: Deadline,
) -> Result<ImportedPage, TransferError> {
    deadline.check()?;
    let (title, content) = match &entry.markdown {
        Some(markdown) => {
            let mut document = from_markdown(body(markdown));
            let title = take_title(&mut document).unwrap_or_else(|| clean_title(&entry.name));
            (title, encode(&document, max_document, &entry.path)?)
        }
        None => (clean_title(&entry.name), None),
    };
    let children = entry
        .children
        .into_iter()
        .map(|child| page(child, max_document, deadline))
        .collect::<Result<_, _>>()?;
    Ok(ImportedPage {
        title,
        content,
        children,
    })
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
    if document.is_empty() {
        return Ok(None);
    }
    let doc = Doc::new();
    let fragment = neon_blocks::fragment(&doc);
    let mut txn = doc.transact_mut();
    let mut ids = || Uuid::now_v7().to_string();
    neon_blocks::replace(&mut txn, &fragment, document, &mut ids);
    let state = txn.encode_state_as_update_v1(&StateVector::default());
    if state.len() > max_document {
        return Err(TransferError::too_large(format!(
            "{path} makes a document larger than {}, the most a page holds",
            size(max_document)
        )));
    }
    Ok(Some(ImportedContent {
        state,
        state_vector: txn.state_vector().encode_v1(),
        text: document.plain_text(),
    }))
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
    use yrs::Update;
    use yrs::updates::decoder::Decode as _;

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
