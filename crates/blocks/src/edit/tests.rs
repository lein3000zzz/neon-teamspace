use yrs::block::ClientID;
use yrs::updates::decoder::Decode as _;
use yrs::{Doc, OffsetKind, Options, StateVector, Transact as _, Update};

use super::*;
use crate::ids::SequentialIds;
use crate::markdown::{from_markdown, to_markdown};
use crate::model::{
    BlockKind, CodeBlockProps, Inline, StyledText, Styles, TableCell, TableContent, TableProps,
    TableRow, TextProps,
};
use crate::ydoc::{fragment, read_doc, replace};

/// A document of `blocks`, its blocks given ids `block-1`, `block-2` and so on, in document
/// order.
fn page(blocks: Vec<Block>) -> Doc {
    page_with(Doc::with_client_id(1), blocks)
}

fn page_with(doc: Doc, blocks: Vec<Block>) -> Doc {
    {
        let fragment = fragment(&doc);
        let mut txn = doc.transact_mut();
        replace(
            &mut txn,
            &fragment,
            &Document::new(blocks),
            &mut SequentialIds::new("block"),
        );
    }
    doc
}

fn edit(doc: &Doc, edit: &Edit) -> Result<Applied, EditError> {
    let fragment = fragment(doc);
    let mut txn = doc.transact_mut();
    apply(&mut txn, &fragment, edit, &mut SequentialIds::new("new"))
}

fn read(doc: &Doc) -> Document {
    read_doc(doc).expect("the document reads")
}

fn md(doc: &Doc) -> String {
    to_markdown(&read(doc))
}

fn ids(doc: &Doc) -> Vec<String> {
    read(doc)
        .walk()
        .map(|block| block.id.clone().unwrap_or_default())
        .collect()
}

fn blocks(markdown: &str) -> Vec<Block> {
    from_markdown(markdown).blocks
}

fn bullet(text: &str) -> Block {
    Block::new(BlockKind::BulletListItem {
        props: TextProps::default(),
        content: Inline::plain(text),
    })
}

fn styled(text: &str, styles: Styles) -> Inline {
    Inline::Text(StyledText {
        text: text.to_owned(),
        styles,
    })
}

fn bold() -> Styles {
    Styles {
        bold: true,
        ..Styles::default()
    }
}

/// The state of `doc` as one update, to see it is unchanged.
fn state(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

fn find(find: &str, replace: &str, all: bool) -> Edit {
    Edit::FindReplace {
        find: find.to_owned(),
        replace: replace.to_owned(),
        all,
    }
}

#[test]
fn appending_goes_before_the_empty_paragraph_the_editor_keeps_last() {
    let doc = page(vec![Block::paragraph("One"), Block::paragraph("")]);
    let applied = edit(
        &doc,
        &Edit::Append {
            blocks: blocks("Two\n\n- three"),
        },
    )
    .expect("appended");

    assert_eq!(applied.added, ["new-1", "new-2"]);
    assert_eq!(ids(&doc), ["block-1", "new-1", "new-2", "block-2"]);
    assert_eq!(md(&doc), "One\n\nTwo\n\n- three\n");

    // Without an empty paragraph last, at the very end.
    let doc = page(vec![Block::paragraph("One")]);
    edit(
        &doc,
        &Edit::Append {
            blocks: blocks("Two"),
        },
    )
    .expect("appended");
    assert_eq!(ids(&doc), ["block-1", "new-1"]);
}

#[test]
fn appending_to_a_page_never_written_makes_its_document() {
    let doc = Doc::with_client_id(1);
    let applied = edit(
        &doc,
        &Edit::Append {
            blocks: blocks("# Hello"),
        },
    )
    .expect("appended");

    assert_eq!(applied.added, ["new-1"]);
    assert_eq!(md(&doc), "# Hello\n");
}

#[test]
fn nothing_to_add_is_refused() {
    let doc = page(vec![Block::paragraph("One")]);
    let before = state(&doc);
    for refused in [
        Edit::Append { blocks: vec![] },
        Edit::InsertAfter {
            block_id: "block-1".into(),
            blocks: vec![],
        },
    ] {
        assert_eq!(edit(&doc, &refused), Err(EditError::NoBlocks));
    }
    assert_eq!(state(&doc), before);
}

#[test]
fn blocks_go_after_a_nested_block_among_its_siblings() {
    let doc = page(vec![
        bullet("parent").with_children(vec![bullet("a"), bullet("b")]),
        Block::paragraph("after"),
    ]);
    let applied = edit(
        &doc,
        &Edit::InsertAfter {
            block_id: "block-2".into(),
            blocks: blocks("- inserted"),
        },
    )
    .expect("inserted");

    assert_eq!(applied.added, ["new-1"]);
    let read = read(&doc);
    let children: Vec<_> = read.blocks[0]
        .children
        .iter()
        .map(|child| child.id.clone().unwrap_or_default())
        .collect();
    assert_eq!(children, ["block-2", "new-1", "block-3"]);
    assert_eq!(md(&doc), "- parent\n  - a\n  - inserted\n  - b\n\nafter\n");
}

#[test]
fn a_block_the_page_lacks_is_named_and_changes_nothing() {
    let doc = page(vec![Block::paragraph("One")]);
    let before = state(&doc);
    let missing = Edit::InsertAfter {
        block_id: "nope".into(),
        blocks: blocks("x"),
    };

    assert_eq!(edit(&doc, &missing), Err(EditError::NoBlock("nope".into())));
    assert_eq!(
        edit(
            &doc,
            &Edit::ReplaceRange {
                from: "block-1".into(),
                to: "nope".into(),
                blocks: vec![]
            }
        ),
        Err(EditError::NoBlock("nope".into()))
    );
    assert_eq!(state(&doc), before);
    // An empty page has no block at all.
    assert_eq!(
        edit(&Doc::new(), &missing),
        Err(EditError::NoBlock("nope".into()))
    );
}

#[test]
fn a_range_of_siblings_is_replaced_both_ends_included() {
    let doc = page(
        ["a", "b", "c", "d"]
            .into_iter()
            .map(Block::paragraph)
            .collect(),
    );
    let applied = edit(
        &doc,
        &Edit::ReplaceRange {
            from: "block-2".into(),
            to: "block-3".into(),
            blocks: blocks("X\n\nY\n\nZ"),
        },
    )
    .expect("replaced");

    assert_eq!(applied.added, ["new-1", "new-2", "new-3"]);
    assert_eq!(applied.removed, 2);
    assert_eq!(ids(&doc), ["block-1", "new-1", "new-2", "new-3", "block-4"]);
    assert_eq!(md(&doc), "a\n\nX\n\nY\n\nZ\n\nd\n");

    // A range of one block.
    edit(
        &doc,
        &Edit::ReplaceRange {
            from: "block-4".into(),
            to: "block-4".into(),
            blocks: blocks("D"),
        },
    )
    .expect("replaced");
    assert_eq!(md(&doc), "a\n\nX\n\nY\n\nZ\n\nD\n");
}

#[test]
fn a_range_runs_forwards_between_siblings() {
    let doc = page(vec![
        bullet("parent").with_children(vec![bullet("child")]),
        Block::paragraph("a"),
        Block::paragraph("b"),
    ]);
    let before = state(&doc);
    let range = |from: &str, to: &str| Edit::ReplaceRange {
        from: from.into(),
        to: to.into(),
        blocks: blocks("x"),
    };

    assert_eq!(
        edit(&doc, &range("block-4", "block-3")),
        Err(EditError::Backwards {
            from: "block-4".into(),
            to: "block-3".into()
        })
    );
    assert_eq!(
        edit(&doc, &range("block-2", "block-3")),
        Err(EditError::NotSiblings {
            from: "block-2".into(),
            to: "block-3".into()
        })
    );
    assert_eq!(state(&doc), before);
}

#[test]
fn removing_the_last_child_drops_its_group_and_the_last_block_leaves_a_paragraph() {
    let doc = page(vec![
        bullet("parent").with_children(vec![bullet("only")]),
        Block::paragraph("other"),
    ]);
    let applied = edit(
        &doc,
        &Edit::ReplaceRange {
            from: "block-2".into(),
            to: "block-2".into(),
            blocks: vec![],
        },
    )
    .expect("removed");

    assert_eq!(applied.removed, 1);
    let read = read(&doc);
    assert!(read.blocks[0].children.is_empty());
    assert_eq!(md(&doc), "- parent\n\nother\n");

    let applied = edit(
        &doc,
        &Edit::ReplaceRange {
            from: "block-1".into(),
            to: "block-3".into(),
            blocks: vec![],
        },
    )
    .expect("removed");
    assert_eq!(applied.removed, 2);
    let read = self::read(&doc);
    assert_eq!(read.blocks.len(), 1);
    assert_eq!(read.blocks[0].kind, Block::paragraph("").kind);
}

#[test]
fn found_text_is_replaced_where_it_occurs_once() {
    let doc = page(vec![
        Block::paragraph("The quick fox"),
        Block::paragraph("jumps"),
    ]);
    let applied = edit(&doc, &find("quick", "slow", false)).expect("replaced");

    assert_eq!(applied.replaced, 1);
    assert_eq!(md(&doc), "The slow fox\n\njumps\n");
    // The blocks keep their ids.
    assert_eq!(ids(&doc), ["block-1", "block-2"]);
}

#[test]
fn text_found_more_than_once_wants_more_of_it_or_every_occurrence() {
    let doc = page(vec![
        Block::paragraph("one cat, two cats"),
        Block::paragraph("cat"),
    ]);
    let before = state(&doc);

    assert_eq!(
        edit(&doc, &find("cat", "dog", false)),
        Err(EditError::Ambiguous(3))
    );
    assert_eq!(state(&doc), before);
    let applied = edit(&doc, &find("cat", "dog", true)).expect("replaced");
    assert_eq!(applied.replaced, 3);
    assert_eq!(md(&doc), "one dog, two dogs\n\ndog\n");
}

#[test]
fn text_to_find_is_one_line_and_somewhere() {
    let doc = page(vec![Block::paragraph("hello")]);
    let before = state(&doc);

    assert_eq!(edit(&doc, &find("", "x", true)), Err(EditError::EmptyFind));
    assert_eq!(
        edit(&doc, &find("hel\nlo", "x", true)),
        Err(EditError::LineBreak)
    );
    assert_eq!(
        edit(&doc, &find("hello", "two\r\nlines", true)),
        Err(EditError::LineBreak)
    );
    assert_eq!(
        edit(&doc, &find("absent", "x", true)),
        Err(EditError::NotFound)
    );
    // Case counts.
    assert_eq!(
        edit(&doc, &find("Hello", "x", true)),
        Err(EditError::NotFound)
    );
    assert_eq!(state(&doc), before);
    assert_eq!(
        edit(&Doc::new(), &find("x", "y", true)),
        Err(EditError::NotFound)
    );
}

#[test]
fn a_replacement_takes_the_styles_of_the_first_character_it_replaces() {
    let doc = page(vec![Block::new(BlockKind::Paragraph {
        props: TextProps::default(),
        content: vec![
            styled("plain ", Styles::default()),
            styled("bold", bold()),
            styled(" tail", Styles::default()),
        ],
    })]);
    edit(&doc, &find("bold", "strong", false)).expect("replaced");
    assert_eq!(md(&doc), "plain **strong** tail\n");

    // Across runs: plain, as its first character.
    edit(&doc, &find("n **", "x", false)).expect_err("markdown is no text of the page");
    edit(&doc, &find("n str", "N-ST", false)).expect("replaced");
    assert_eq!(md(&doc), "plaiN-ST**ong** tail\n");

    // Emptied, the text goes, and the styles around stay.
    edit(&doc, &find("N-ST", "", false)).expect("removed");
    assert_eq!(md(&doc), "plai**ong** tail\n");
}

#[test]
fn links_carry_over_to_their_replacement() {
    let doc = page(blocks("Read [the guide](https://example.com/guide) first"));
    edit(&doc, &find("the guide", "this guide", false)).expect("replaced");
    assert_eq!(
        md(&doc),
        "Read [this guide](https://example.com/guide) first\n"
    );
}

#[test]
fn text_is_found_in_nested_blocks_table_cells_and_code() {
    let table = Block::new(BlockKind::Table {
        props: TableProps::default(),
        content: TableContent {
            rows: vec![
                TableRow {
                    cells: vec![TableCell {
                        content: Inline::plain("Owner"),
                        ..TableCell::default()
                    }],
                },
                TableRow {
                    cells: vec![TableCell {
                        content: Inline::plain("TODO name"),
                        ..TableCell::default()
                    }],
                },
            ],
            column_widths: vec![None],
            header_rows: None,
            header_cols: None,
        },
    });
    let code = Block::new(BlockKind::CodeBlock {
        props: CodeBlockProps {
            language: "rust".into(),
        },
        code: "fn main() {\n    // TODO name\n}".into(),
    });
    let doc = page(vec![
        bullet("list").with_children(vec![bullet("TODO name")]),
        table,
        code,
    ]);
    let applied = edit(&doc, &find("TODO name", "Ada", true)).expect("replaced");

    assert_eq!(applied.replaced, 3);
    let markdown = md(&doc);
    assert!(markdown.contains("  - Ada"), "{markdown}");
    assert!(markdown.contains("| Ada |"), "{markdown}");
    assert!(markdown.contains("    // Ada"), "{markdown}");
    assert!(!markdown.contains("TODO"), "{markdown}");
}

#[test]
fn unicode_is_matched_by_characters_whatever_the_offsets_count() {
    for kind in [OffsetKind::Bytes, OffsetKind::Utf16] {
        let doc = page_with(
            Doc::with_options(Options {
                offset_kind: kind,
                ..Options::with_client_id(ClientID::new(1))
            }),
            vec![
                Block::paragraph("Grüße 🦀 aus 東京, café"),
                Block::paragraph("e\u{301} apart"),
            ],
        );
        edit(&doc, &find("🦀", "🐙🐙", false)).expect("replaced");
        edit(&doc, &find("東京", "大阪", false)).expect("replaced");
        edit(&doc, &find("café", "cafe", false)).expect("replaced");
        edit(&doc, &find("aus", "von", false)).expect("replaced");
        // A combining accent is a character of its own, and only matches as one.
        assert_eq!(
            edit(&doc, &find("é apart", "x", false)),
            Err(EditError::NotFound),
            "{kind:?}"
        );
        edit(&doc, &find("e\u{301}", "é", false)).expect("replaced");
        assert_eq!(
            md(&doc),
            "Grüße 🐙🐙 von 大阪, cafe\n\né apart\n",
            "{kind:?}"
        );
    }
}

#[test]
fn replacing_the_body_keeps_the_blocks_that_stay_the_same() {
    let doc = page(blocks("# Plan\n\nFirst\n\nSecond\n\nThird"));
    let applied = edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("# Plan\n\nFirst, changed\n\nSecond\n\nAdded\n\nThird"),
        },
    )
    .expect("replaced");

    assert_eq!(applied.added, ["new-1", "new-2"]);
    assert_eq!(applied.removed, 1);
    assert_eq!(
        ids(&doc),
        ["block-1", "new-1", "block-3", "new-2", "block-4"]
    );
    assert_eq!(
        md(&doc),
        "# Plan\n\nFirst, changed\n\nSecond\n\nAdded\n\nThird\n"
    );

    // Reordered: the longest run that stays in order stays.
    let applied = edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("Third\n\n# Plan\n\nSecond"),
        },
    )
    .expect("replaced");
    assert_eq!(applied.removed, 3);
    assert_eq!(applied.added.len(), 1);
    assert_eq!(md(&doc), "Third\n\n# Plan\n\nSecond\n");
}

#[test]
fn a_block_s_colour_stays_when_its_markdown_does() {
    let mut coloured = Block::paragraph("Red text");
    if let BlockKind::Paragraph { props, .. } = &mut coloured.kind {
        props.text_color = "red".into();
    }
    let doc = page(vec![coloured.clone(), Block::paragraph("Plain")]);
    edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("Red text\n\nPlain, changed"),
        },
    )
    .expect("replaced");

    assert_eq!(read(&doc).blocks[0], coloured.with_id("block-1"));
}

#[test]
fn an_empty_body_leaves_an_empty_paragraph_and_an_empty_page_takes_the_blocks() {
    let doc = page(blocks("One\n\nTwo"));
    let applied = edit(&doc, &Edit::ReplaceBody { blocks: vec![] }).expect("replaced");
    assert_eq!(applied.removed, 2);
    let read = read(&doc);
    assert_eq!(read.blocks.len(), 1);
    assert_eq!(read.blocks[0].kind, Block::paragraph("").kind);

    let doc = Doc::with_client_id(1);
    let applied = edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("A\n\nB"),
        },
    )
    .expect("replaced");
    assert_eq!(applied.added, ["new-1", "new-2"]);
    assert_eq!(md(&doc), "A\n\nB\n");
    assert_eq!(
        edit(&Doc::new(), &Edit::ReplaceBody { blocks: vec![] }),
        Ok(Applied::default())
    );
}

#[test]
fn the_common_blocks_are_a_longest_sequence_in_order() {
    let old = |items: &[&str]| -> Vec<Option<String>> {
        items.iter().map(|item| Some((*item).to_owned())).collect()
    };
    let new =
        |items: &[&str]| -> Vec<String> { items.iter().map(|item| (*item).to_owned()).collect() };

    assert_eq!(
        common(&old(&["a", "b", "c"]), &new(&["a", "b", "c"])),
        [(0, 0), (1, 1), (2, 2)]
    );
    assert_eq!(
        common(&old(&["a", "b", "c"]), &new(&["a", "x", "c"])),
        [(0, 0), (2, 2)]
    );
    assert_eq!(
        common(&old(&["a", "b", "c", "d"]), &new(&["b", "d", "e"])),
        [(1, 0), (3, 1)]
    );
    assert_eq!(common(&old(&[]), &new(&["a"])), []);
    assert_eq!(common(&old(&["a"]), &new(&[])), []);
    // Blocks that did not read never stay.
    assert_eq!(common(&[None, Some("a".into())], &new(&["a"])), [(1, 0)]);
}

/// A client's document, synced from the server's, and the update a change to it makes.
fn typed(server: &Doc, client_id: u64, type_into: &str, text: &str) -> (Doc, Vec<u8>) {
    let client = Doc::with_client_id(client_id);
    let snapshot = server
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    client
        .transact_mut()
        .apply_update(Update::decode_v1(&snapshot).expect("decodes"))
        .expect("applies");
    let before = client.transact().state_vector();
    {
        let fragment = fragment(&client);
        let mut txn = client.transact_mut();
        let place = find_block(&txn, &fragment, type_into).expect("the block is there");
        let Some(XmlOut::Element(container)) = place.group.get(&txn, place.index) else {
            panic!("a block");
        };
        let Some(XmlOut::Element(content)) = container.children(&txn).next() else {
            panic!("its content");
        };
        let Some(XmlOut::Text(run)) = content.children(&txn).next() else {
            panic!("its text");
        };
        let end = run.len(&txn);
        run.insert(&mut txn, end, text);
    }
    let update = client.transact().encode_state_as_update_v1(&before);
    (client, update)
}

fn apply_update(doc: &Doc, update: &[u8]) {
    doc.transact_mut()
        .apply_update(Update::decode_v1(update).expect("decodes"))
        .expect("applies");
}

#[test]
fn edits_merge_with_typing_that_happened_meanwhile() {
    let edits = [
        Edit::Append {
            blocks: blocks("appended"),
        },
        Edit::InsertAfter {
            block_id: "block-1".into(),
            blocks: blocks("inserted"),
        },
        Edit::ReplaceRange {
            from: "block-3".into(),
            to: "block-3".into(),
            blocks: blocks("replaced"),
        },
        find("other", "changed", false),
        Edit::ReplaceBody {
            blocks: blocks("typed in\n\nother, new\n\nthird"),
        },
    ];
    for change in edits {
        let server = page(blocks("typed in\n\nother\n\nthird"));
        let (client, typing) = typed(&server, 7, "block-1", " and more");
        let before = server.transact().state_vector();
        edit(&server, &change).expect("edited");
        let edited = server.transact().encode_state_as_update_v1(&before);

        // Each hears of the other's change after making its own.
        apply_update(&server, &typing);
        apply_update(&client, &edited);

        let (server, client) = (md(&server), md(&client));
        assert_eq!(server, client, "{}", change.name());
        assert!(
            server.contains("typed in and more"),
            "{}: {server}",
            change.name()
        );
    }
}

#[test]
fn a_replacement_merges_with_typing_in_the_same_block() {
    let server = page(blocks("alpha beta"));
    let (client, typing) = typed(&server, 7, "block-1", " gamma");
    let before = server.transact().state_vector();
    edit(&server, &find("alpha", "ALPHA", false)).expect("replaced");
    let edited = server.transact().encode_state_as_update_v1(&before);
    apply_update(&server, &typing);
    apply_update(&client, &edited);

    assert_eq!(md(&server), "ALPHA beta gamma\n");
    assert_eq!(md(&client), md(&server));
}
