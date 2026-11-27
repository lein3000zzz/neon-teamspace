use yrs::block::ClientID;
use yrs::updates::decoder::Decode as _;
use yrs::{Doc, OffsetKind, Options, StateVector, Transact as _, Update};

use super::*;
use crate::ids::SequentialIds;
use crate::markdown::{from_markdown, to_markdown, to_markdown_with_ids};
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
    edit_in(doc, edit, usize::MAX)
}

/// An edit given `room` bytes of text to add by replacing.
fn edit_in(doc: &Doc, edit: &Edit, room: usize) -> Result<Applied, EditError> {
    let fragment = fragment(doc);
    let mut txn = doc.transact_mut();
    apply(
        &mut txn,
        &fragment,
        edit,
        &mut SequentialIds::new("new"),
        room,
        Deadline::NONE,
    )
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
    assert_eq!(applied.removed_ids, ["block-2", "block-3"]);
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
fn every_replacement_takes_the_styles_of_what_it_replaces() {
    let doc = page(vec![Block::new(BlockKind::Paragraph {
        props: TextProps::default(),
        content: vec![
            styled("xa ", Styles::default()),
            styled("xb xc", bold()),
            styled(" xd", Styles::default()),
        ],
    })]);
    let applied = edit(&doc, &find("x", "yy", true)).expect("replaced");

    assert_eq!(applied.replaced, 4);
    assert_eq!(md(&doc), "yya **yyb yyc** yyd\n");
}

#[test]
fn find_and_replace_replaces_at_most_its_limit() {
    let text = |times: usize| vec!["ox"; times].join(" ");
    let doc = page(vec![Block::paragraph(text(MAX_REPLACEMENTS + 1))]);
    let before = state(&doc);

    assert_eq!(
        edit(&doc, &find("x", "y", true)),
        Err(EditError::TooManyMatches(MAX_REPLACEMENTS + 1))
    );
    assert_eq!(state(&doc), before);
    let doc = page(vec![Block::paragraph(text(MAX_REPLACEMENTS))]);
    let applied = edit(&doc, &find("x", "y", true)).expect("replaced");
    assert_eq!(applied.replaced, MAX_REPLACEMENTS);
    assert_eq!(
        md(&doc),
        format!("{}\n", vec!["oy"; MAX_REPLACEMENTS].join(" "))
    );
}

#[test]
fn replacements_that_add_more_than_the_room_are_refused() {
    let doc = page(vec![Block::paragraph("a or a")]);
    let before = state(&doc);
    let longer = find("a", "four", true);

    assert_eq!(edit_in(&doc, &longer, 7), Err(EditError::TooLarge));
    assert_eq!(state(&doc), before);
    let applied = edit_in(&doc, &longer, 8).expect("room enough");
    assert_eq!(longer.replaced_bytes(&applied), 8);
    assert_eq!(md(&doc), "four or four\n");
    // Removing adds nothing, whatever the room.
    edit_in(&doc, &find("four ", "", true), 0).expect("removed");
    assert_eq!(md(&doc), "or four\n");
}

#[test]
fn text_that_grows_by_replacing_it_again_and_again_stops_at_the_limit() {
    // Each pass makes ten of every x: the work it takes grows with the matches, not with
    // their square, so the passes up to the limit are quick and the next is refused.
    let doc = page(vec![Block::paragraph("x")]);
    let tenfold = find("x", "xxxxxxxxxx", true);
    let started = std::time::Instant::now();
    for times in [1, 10, 100, 1000, 10_000] {
        assert_eq!(edit(&doc, &tenfold).expect("replaced").replaced, times);
    }
    let before = state(&doc);
    assert_eq!(
        edit(&doc, &tenfold),
        Err(EditError::TooManyMatches(100_000))
    );
    assert_eq!(state(&doc), before);
    assert_eq!(md(&doc).trim_end().len(), 100_000);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
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
                        content: Inline::plain("Somebody"),
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
        code: "fn main() {\n    // Somebody\n}".into(),
    });
    let doc = page(vec![
        bullet("list").with_children(vec![bullet("Somebody")]),
        table,
        code,
    ]);
    let applied = edit(&doc, &find("Somebody", "Ada", true)).expect("replaced");

    assert_eq!(applied.replaced, 3);
    let markdown = md(&doc);
    assert!(markdown.contains("  - Ada"), "{markdown}");
    assert!(markdown.contains("| Ada |"), "{markdown}");
    assert!(markdown.contains("    // Ada"), "{markdown}");
    assert!(!markdown.contains("Somebody"), "{markdown}");
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
    assert_eq!(applied.removed_ids, ["block-2"]);
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
        match content.children(&txn).next() {
            Some(XmlOut::Text(run)) => {
                let end = run.len(&txn);
                run.insert(&mut txn, end, text);
            }
            // An empty paragraph gets its text node as typing starts.
            None => {
                content.push_back(&mut txn, yrs::XmlTextPrelim::new(text));
            }
            Some(_) => panic!("its text"),
        }
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

/// The page's markdown as reading it with ids gives it, read back as blocks: what an agent
/// sends back unchanged.
fn as_read(doc: &Doc) -> Vec<Block> {
    blocks(&to_markdown_with_ids(&read(doc)))
}

/// A paragraph with blocks nested under it, as the editor nests them on Tab.
fn nesting(text: &str, children: Vec<Block>) -> Block {
    Block::paragraph(text).with_children(children)
}

#[test]
fn the_body_as_read_sent_back_changes_nothing() {
    for blocks in [
        vec![Block::paragraph("B"), Block::paragraph("")],
        vec![nesting("Parent", vec![Block::paragraph("Child")])],
        vec![
            Block::paragraph(""),
            nesting(
                "Parent",
                vec![
                    nesting("Child", vec![Block::paragraph("Grandchild")]),
                    bullet("listed").with_children(vec![bullet("under it")]),
                    Block::paragraph(""),
                ],
            ),
            bullet("after").with_children(vec![Block::paragraph("")]),
            Block::paragraph(""),
        ],
    ] {
        let doc = page(blocks);
        let before = state(&doc);
        let applied = edit(
            &doc,
            &Edit::ReplaceBody {
                blocks: as_read(&doc),
            },
        )
        .expect("replaced");

        assert_eq!(applied, Applied::default(), "{}", md(&doc));
        assert_eq!(state(&doc), before, "{}", md(&doc));
    }
}

#[test]
fn a_changed_child_is_replaced_under_its_parent_and_its_siblings_stay() {
    let doc = page(vec![nesting(
        "Parent",
        vec![Block::paragraph("One"), Block::paragraph("Two")],
    )]);
    let applied = edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("Parent\n\nOne, changed\n\nTwo\n\nAfter"),
        },
    )
    .expect("replaced");

    assert_eq!(applied.added, ["new-1", "new-2"]);
    assert_eq!(applied.removed, 1);
    let read = read(&doc);
    // The new child goes where the old one was, under the parent; what comes after the
    // last block goes after the top block it is under.
    assert_eq!(read.blocks.len(), 2);
    let children: Vec<_> = read.blocks[0]
        .children
        .iter()
        .map(|child| child.id.clone().unwrap_or_default())
        .collect();
    assert_eq!(children, ["new-1", "block-3"]);
    assert_eq!(read.blocks[0].id.as_deref(), Some("block-1"));
    assert_eq!(read.blocks[1].id.as_deref(), Some("new-2"));
    assert_eq!(md(&doc), "Parent\n\nOne, changed\n\nTwo\n\nAfter\n");
}

#[test]
fn a_changed_parent_is_replaced_with_the_blocks_under_it() {
    let doc = page(vec![
        nesting("Parent", vec![Block::paragraph("Child")]),
        Block::paragraph("Other"),
    ]);
    let applied = edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: blocks("Parent, changed\n\nChild\n\nOther"),
        },
    )
    .expect("replaced");

    assert_eq!(applied.removed, 1);
    assert_eq!(applied.added, ["new-1", "new-2"]);
    assert_eq!(ids(&doc), ["new-1", "new-2", "block-3"]);
    assert_eq!(md(&doc), "Parent, changed\n\nChild\n\nOther\n");
}

#[test]
fn typing_in_blocks_the_body_keeps_stays() {
    // Into the empty paragraph the editor keeps last, and on in a nested child.
    for (into, text, shows) in [
        ("block-2", "Typed", "Intro\n\nTyped\n"),
        ("block-4", " typed", "Child typed\n"),
    ] {
        let server = page(vec![
            Block::paragraph("Intro"),
            Block::paragraph(""),
            nesting("Parent", vec![Block::paragraph("Child")]),
        ]);
        let sent = as_read(&server);
        let (client, typing) = typed(&server, 7, into, text);
        let before = server.transact().state_vector();
        edit(&server, &Edit::ReplaceBody { blocks: sent }).expect("replaced");
        let edited = server.transact().encode_state_as_update_v1(&before);
        apply_update(&server, &typing);
        apply_update(&client, &edited);

        let markdown = md(&server);
        assert!(markdown.contains(shows), "{into}: {markdown}");
        assert_eq!(markdown, md(&client));
    }
}

#[test]
fn a_block_is_named_by_its_id_as_markdown_writes_it_or_as_it_is() {
    let odd = |doc: &Doc| {
        let fragment = fragment(doc);
        let mut txn = doc.transact_mut();
        replace(
            &mut txn,
            &fragment,
            &Document::new(vec![
                Block::paragraph("one").with_id("odd id/1"),
                Block::paragraph("two").with_id("50%"),
            ]),
            &mut SequentialIds::new("block"),
        );
    };
    let doc = Doc::with_client_id(1);
    odd(&doc);
    let shown = to_markdown_with_ids(&read(&doc));
    assert!(shown.contains("<!-- block odd%20id%2F1 -->"), "{shown}");
    assert!(shown.contains("<!-- block 50%25 -->"), "{shown}");

    for (name, text) in [
        ("odd%20id%2F1", "a"),
        ("odd id/1", "b"),
        ("50%25", "c"),
        ("50%", "d"),
    ] {
        edit(
            &doc,
            &Edit::InsertAfter {
                block_id: name.into(),
                blocks: blocks(text),
            },
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
    assert_eq!(md(&doc), "one\n\nb\n\na\n\ntwo\n\nd\n\nc\n");
    edit(
        &doc,
        &Edit::ReplaceRange {
            from: "odd%20id%2F1".into(),
            to: "50%25".into(),
            blocks: blocks("all"),
        },
    )
    .expect("replaced");
    assert_eq!(md(&doc), "all\n\nd\n\nc\n");
}

/// `n` paragraphs, `p1` to `pn`.
fn paragraphs(n: usize) -> Vec<Block> {
    (1..=n).map(|i| Block::paragraph(format!("p{i}"))).collect()
}

#[test]
fn writes_that_would_walk_too_far_into_a_long_page_are_refused_whole() {
    let doc = page(paragraphs(2001));
    let before = md(&doc);
    // 2000 blocks after 2001 others walk past more than MAX_WALK items.
    let many = paragraphs(2000);
    assert_eq!(
        edit(
            &doc,
            &Edit::Append {
                blocks: many.clone()
            }
        ),
        Err(EditError::TooFar(TooFar))
    );
    assert_eq!(
        edit(
            &doc,
            &Edit::InsertAfter {
                block_id: "block-2001".into(),
                blocks: many.clone(),
            }
        ),
        Err(EditError::TooFar(TooFar))
    );
    assert_eq!(
        edit(
            &doc,
            &Edit::ReplaceRange {
                from: "block-2001".into(),
                to: "block-2001".into(),
                blocks: many.clone(),
            }
        ),
        Err(EditError::TooFar(TooFar))
    );
    {
        let fragment = fragment(&doc);
        let mut txn = doc.transact_mut();
        assert_eq!(
            crate::ydoc::append(
                &mut txn,
                &fragment,
                &many,
                &mut SequentialIds::new("new"),
                Deadline::NONE
            ),
            Err(crate::ydoc::AppendRefused::TooFar(TooFar))
        );
    }
    assert_eq!(md(&doc), before);
    // Near the top of the page, the same blocks go in.
    edit(
        &doc,
        &Edit::InsertAfter {
            block_id: "block-1".into(),
            blocks: many,
        },
    )
    .expect("inserted near the top");
    assert_eq!(read(&doc).blocks.len(), 4001);
}

/// An edit given a deadline that has passed already.
fn edit_late(doc: &Doc, edit: &Edit) -> Result<Applied, EditError> {
    let fragment = fragment(doc);
    let mut txn = doc.transact_mut();
    apply(
        &mut txn,
        &fragment,
        edit,
        &mut SequentialIds::new("new"),
        usize::MAX,
        Deadline::at(std::time::Instant::now()),
    )
}

#[test]
fn past_its_deadline_an_edit_writes_and_removes_no_block_past_a_groups_first_place() {
    // Blocks removed from a group stay in it, unseen, and every write past its first place
    // walks them: a deadline stops what no count of the blocks there bounds.
    let doc = page(paragraphs(3));
    let late = |edit: Edit| edit_late(&doc, &edit);
    let one = || vec![Block::paragraph("new")];
    assert_eq!(
        late(Edit::Append { blocks: one() }),
        Err(EditError::TooFar(TooFar))
    );
    assert_eq!(
        late(Edit::InsertAfter {
            block_id: "block-1".into(),
            blocks: one(),
        }),
        Err(EditError::TooFar(TooFar))
    );
    assert_eq!(
        late(Edit::ReplaceRange {
            from: "block-2".into(),
            to: "block-3".into(),
            blocks: one(),
        }),
        Err(EditError::TooFar(TooFar))
    );
    // Keeping the first block, writing one after it walks.
    assert_eq!(
        late(Edit::ReplaceBody {
            blocks: vec![Block::paragraph("p1"), Block::paragraph("new")],
        }),
        Err(EditError::TooFar(TooFar))
    );
    // Removing a block past the first walks too.
    assert_eq!(
        late(Edit::Restore {
            blocks: vec![read(&doc).blocks[0].clone(), read(&doc).blocks[1].clone()],
        }),
        Err(EditError::TooFar(TooFar))
    );
    {
        let fragment = fragment(&doc);
        let mut txn = doc.transact_mut();
        assert_eq!(
            crate::ydoc::append(
                &mut txn,
                &fragment,
                &one(),
                &mut SequentialIds::new("new"),
                Deadline::at(std::time::Instant::now()),
            ),
            Err(crate::ydoc::AppendRefused::TooFar(TooFar))
        );
    }
    assert_eq!(md(&doc), "p1\n\np2\n\np3\n");
    // What goes at a group's first place walks nothing, and goes in whatever the time.
    late(Edit::ReplaceBody {
        blocks: vec![Block::paragraph("first"), Block::paragraph("second")],
    })
    .expect("written at the start");
    assert_eq!(md(&doc), "first\n\nsecond\n");
    late(Edit::FindReplace {
        find: "second".into(),
        replace: "last".into(),
        all: false,
    })
    .expect("replaced in place");
    assert_eq!(md(&doc), "first\n\nlast\n");
}

#[test]
fn replacing_every_block_of_a_long_page_removes_them_at_once() {
    // One removal of 3000 neighbours walks to the first; one at a time, they would walk
    // past 4.5 million items.
    let doc = page(paragraphs(3000));
    let new: Vec<Block> = (1..=3000)
        .map(|i| Block::paragraph(format!("q{i}")))
        .collect();
    edit(
        &doc,
        &Edit::ReplaceBody {
            blocks: new.clone(),
        },
    )
    .expect("replaced");
    assert_eq!(to_markdown(&read(&doc)), to_markdown(&Document::new(new)));
}

#[test]
fn a_body_that_keeps_a_long_page_and_adds_many_blocks_after_it_is_weighed_so() {
    // The 2000 new blocks go in after the 2100 that stay, as appending them would.
    let doc = page(paragraphs(2100));
    let mut new = paragraphs(2100);
    new.extend((1..=2000).map(|i| Block::paragraph(format!("q{i}"))));
    let before = md(&doc);
    assert_eq!(
        edit(&doc, &Edit::ReplaceBody { blocks: new }),
        Err(EditError::TooFar(TooFar))
    );
    assert_eq!(md(&doc), before);
}
