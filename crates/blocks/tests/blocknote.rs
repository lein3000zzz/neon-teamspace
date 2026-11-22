//! The crate against BlockNote itself, through the fixtures in `tests/fixtures`:
//!
//! - `blocknote/` holds the Yjs updates BlockNote wrote for the corpus in `markdown/` and
//!   `blocks/`, with the blocks it reads back from them. These tests read the updates with
//!   yrs and expect the same blocks, and parse the markdown corpus and expect what
//!   BlockNote's own parser made of it.
//! - `rust-out/` holds what this crate writes for the same blocks: a Yjs update, the blocks
//!   as JSON and the markdown. These tests check that it is current;
//!   `tools/blocknote-fixtures` checks that BlockNote loads each update as those blocks.
//!
//! `make blocks-fixtures` rewrites both directories, the second by running these tests with
//! `BLOCKS_WRITE_FIXTURES=1`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use neon_blocks::markdown::{from_markdown, to_markdown};
use neon_blocks::{
    Block, BlockKind, Document, FileProps, ImageProps, Inline, Link, SequentialIds, StyledText,
    TableCell, TableCellProps, TextAlignment, fragment, read_doc, replace,
};
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The names of the files in a fixture directory with an extension, sorted.
fn names(directory: &str, extension: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(directory))
        .unwrap_or_else(|e| panic!("{directory}: {e}"))
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(extension).map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The blocks BlockNote read back from its update for a fixture.
fn expected(name: &str) -> Document {
    let json = read(&fixtures().join(format!("blocknote/{name}.json")));
    serde_json::from_slice(&json).unwrap_or_else(|e| panic!("blocknote/{name}.json: {e}"))
}

fn every_fixture() -> Vec<(String, Document)> {
    let all = names("blocknote", ".json");
    assert!(
        !all.is_empty(),
        "no BlockNote fixtures: run make blocks-fixtures"
    );
    all.into_iter()
        .map(|name| {
            let document = expected(&name);
            (name, document)
        })
        .collect()
}

#[test]
fn blocknote_updates_read_as_blocknote_reads_them() {
    for (name, document) in every_fixture() {
        let update = read(&fixtures().join(format!("blocknote/{name}.yjs")));
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).expect("an update"))
            .expect("applies");
        assert_eq!(read_doc(&doc).expect("reads"), document, "{name}");
    }
}

#[test]
fn markdown_parses_as_blocknote_parses_it() {
    for name in names("markdown", ".md") {
        let markdown = String::from_utf8(read(&fixtures().join(format!("markdown/{name}.md"))))
            .expect("UTF-8");
        let mut parsed = from_markdown(&markdown);
        parsed.fill_ids(&mut SequentialIds::new("block"));
        assert_eq!(parsed, expected(&name), "{name}");
    }
}

#[test]
fn documents_come_back_from_yrs_exactly() {
    for (name, document) in every_fixture() {
        let doc = write(&document);
        assert_eq!(read_doc(&doc).expect("reads"), document, "{name}");
    }
}

#[test]
fn markdown_keeps_what_it_can_and_is_stable_after_one_pass() {
    for (name, document) in every_fixture() {
        let markdown = to_markdown(&document);
        let parsed = from_markdown(&markdown);
        assert_eq!(
            without_ids(parsed.clone()),
            markdown_view(&document),
            "{name}"
        );
        assert_eq!(
            to_markdown(&parsed),
            markdown,
            "{name}: stable after one pass"
        );
    }
}

#[test]
fn rust_out_is_current() {
    let directory = fixtures().join("rust-out");
    let write_them = std::env::var_os("BLOCKS_WRITE_FIXTURES").is_some_and(|v| v == "1");
    let mut stale = Vec::new();
    let mut wanted = BTreeSet::new();
    for (name, document) in every_fixture() {
        let doc = write(&document);
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let json = serde_json::to_string_pretty(&document).expect("serializes") + "\n";
        let files = [
            (format!("{name}.yjs"), update),
            (format!("{name}.json"), json.into_bytes()),
            (format!("{name}.md"), to_markdown(&document).into_bytes()),
        ];
        for (file, contents) in files {
            let path = directory.join(&file);
            wanted.insert(file.clone());
            if write_them {
                fs::write(&path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            } else if fs::read(&path).ok().as_deref() != Some(contents.as_slice()) {
                stale.push(file);
            }
        }
    }
    for entry in fs::read_dir(&directory).expect("rust-out") {
        let name = entry
            .expect("an entry")
            .file_name()
            .into_string()
            .expect("UTF-8");
        if !wanted.contains(&name) {
            if write_them {
                fs::remove_file(directory.join(&name)).expect("removes");
            } else {
                stale.push(name);
            }
        }
    }
    assert!(
        stale.is_empty(),
        "rust-out is stale, run make blocks-fixtures: {stale:?}"
    );
}

#[test]
fn rust_out_is_deterministic() {
    for (name, document) in every_fixture() {
        let encode = || {
            write(&document)
                .transact()
                .encode_state_as_update_v1(&StateVector::default())
        };
        assert_eq!(encode(), encode(), "{name}");
    }
}

#[test]
fn the_fixtures_cover_every_block_type_and_style() {
    let mut types = BTreeSet::new();
    let mut styles = BTreeSet::new();
    for (_, document) in every_fixture() {
        for block in document.walk() {
            types.insert(block.kind.name());
            let mut runs: Vec<&StyledText> = Vec::new();
            let mut inline: Vec<&Inline> = block
                .kind
                .inline_content()
                .unwrap_or_default()
                .iter()
                .collect();
            if let BlockKind::Table { content, .. } = &block.kind {
                for row in &content.rows {
                    for cell in &row.cells {
                        inline.extend(&cell.content);
                    }
                }
            }
            for run in inline {
                match run {
                    Inline::Text(text) => runs.push(text),
                    Inline::Link(link) => {
                        styles.insert("link");
                        runs.extend(&link.content);
                    }
                }
            }
            for run in runs {
                let s = &run.styles;
                for (on, style) in [
                    (s.bold, "bold"),
                    (s.italic, "italic"),
                    (s.underline, "underline"),
                    (s.strike, "strike"),
                    (s.code, "code"),
                    (s.text_color.is_some(), "textColor"),
                    (s.background_color.is_some(), "backgroundColor"),
                ] {
                    if on {
                        styles.insert(style);
                    }
                }
            }
        }
    }
    let all_types = BTreeSet::from([
        "paragraph",
        "heading",
        "bulletListItem",
        "numberedListItem",
        "checkListItem",
        "toggleListItem",
        "quote",
        "codeBlock",
        "divider",
        "table",
        "image",
        "video",
        "audio",
        "file",
    ]);
    let all_styles = BTreeSet::from([
        "bold",
        "italic",
        "underline",
        "strike",
        "code",
        "textColor",
        "backgroundColor",
        "link",
    ]);
    assert_eq!(types, all_types);
    assert_eq!(styles, all_styles);
}

/// A Yjs document holding `document`, written as the server writes one: block ids kept, and
/// a fixed client id so the update is the same on every run.
fn write(document: &Document) -> Doc {
    let doc = Doc::with_client_id(1);
    let fragment = fragment(&doc);
    let mut txn = doc.transact_mut();
    replace(
        &mut txn,
        &fragment,
        document,
        &mut SequentialIds::new("new"),
    );
    drop(txn);
    doc
}

fn without_ids(mut document: Document) -> Document {
    fn strip(blocks: &mut [Block]) {
        for block in blocks {
            block.id = None;
            strip(&mut block.children);
        }
    }
    strip(&mut document.blocks);
    document
}

/// What markdown keeps of a document, as the crate's documentation lists it.
fn markdown_view(document: &Document) -> Document {
    Document::new(view(&document.blocks))
}

fn view(blocks: &[Block]) -> Vec<Block> {
    let mut out = Vec::new();
    for block in blocks {
        let nests = matches!(
            block.kind,
            BlockKind::BulletListItem { .. }
                | BlockKind::NumberedListItem { .. }
                | BlockKind::CheckListItem { .. }
                | BlockKind::ToggleListItem { .. }
        ) || matches!(&block.kind, BlockKind::Heading { props, .. } if props.is_toggleable);
        let kind = view_kind(&block.kind);
        let empty = matches!(&kind, BlockKind::Paragraph { content, .. } if content.is_empty());
        if nests {
            out.push(Block::new(kind).with_children(view(&block.children)));
        } else {
            if !empty {
                out.push(Block::new(kind));
            }
            out.extend(view(&block.children));
        }
    }
    out
}

/// Runs without colours, merged where they differed only in colour.
fn view_inline(content: &[Inline]) -> Vec<Inline> {
    let strip = |text: &StyledText| {
        let mut text = text.clone();
        text.styles.text_color = None;
        text.styles.background_color = None;
        text
    };
    let colourless: Vec<Inline> = content
        .iter()
        .map(|run| match run {
            Inline::Text(text) => Inline::Text(strip(text)),
            Inline::Link(link) => Inline::Link(Link {
                href: link.href.clone(),
                content: link.content.iter().map(strip).collect(),
            }),
        })
        .collect();
    Inline::normalize(&colourless)
}

fn view_kind(kind: &BlockKind) -> BlockKind {
    let mut kind = kind.clone();
    match &mut kind {
        BlockKind::Paragraph { props, content }
        | BlockKind::BulletListItem { props, content }
        | BlockKind::ToggleListItem { props, content } => {
            *props = Default::default();
            *content = view_inline(content);
        }
        BlockKind::Heading { props, content } => {
            props.text_color = neon_blocks::DEFAULT_COLOR.into();
            props.background_color = neon_blocks::DEFAULT_COLOR.into();
            props.text_alignment = TextAlignment::Left;
            *content = view_inline(content);
        }
        BlockKind::NumberedListItem { props, content } => {
            props.text_color = neon_blocks::DEFAULT_COLOR.into();
            props.background_color = neon_blocks::DEFAULT_COLOR.into();
            props.text_alignment = TextAlignment::Left;
            props.start = props.start.filter(|start| *start != 1);
            *content = view_inline(content);
        }
        BlockKind::CheckListItem { props, content } => {
            props.text_color = neon_blocks::DEFAULT_COLOR.into();
            props.background_color = neon_blocks::DEFAULT_COLOR.into();
            props.text_alignment = TextAlignment::Left;
            *content = view_inline(content);
        }
        BlockKind::Quote { props, content } => {
            *props = Default::default();
            *content = view_inline(content);
        }
        BlockKind::Table { props, content } => {
            *props = Default::default();
            let columns = content
                .rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0)
                .max(1);
            let alignments: Vec<TextAlignment> = (0..columns)
                .map(|column| {
                    let mut cells = content.rows.iter().filter_map(|row| row.cells.get(column));
                    let first = cells
                        .next()
                        .map_or(TextAlignment::Left, |c| c.props.text_alignment);
                    match first {
                        TextAlignment::Center | TextAlignment::Right
                            if cells.all(|c| c.props.text_alignment == first) =>
                        {
                            first
                        }
                        _ => TextAlignment::Left,
                    }
                })
                .collect();
            content.column_widths = vec![None; columns];
            content.header_rows = Some(1);
            content.header_cols = None;
            for row in &mut content.rows {
                row.cells.resize_with(columns, TableCell::default);
                for (column, cell) in row.cells.iter_mut().enumerate() {
                    cell.content = view_inline(&cell.content);
                    cell.props = TableCellProps {
                        text_alignment: alignments[column],
                        ..TableCellProps::default()
                    };
                }
            }
        }
        BlockKind::Image { props } | BlockKind::Video { props } => {
            *props = ImageProps {
                name: props.name.clone(),
                url: props.url.clone(),
                caption: props.caption.clone(),
                ..ImageProps::default()
            };
        }
        BlockKind::Audio { props } => {
            props.background_color = neon_blocks::DEFAULT_COLOR.into();
            props.show_preview = true;
        }
        BlockKind::File { props } => {
            *props = FileProps {
                name: props.name.clone(),
                url: props.url.clone(),
                caption: props.caption.clone(),
                ..FileProps::default()
            };
        }
        BlockKind::CodeBlock { .. } | BlockKind::Divider { .. } => {}
    }
    kind
}
