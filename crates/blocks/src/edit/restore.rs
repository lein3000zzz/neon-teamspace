//! Restoring a version of a page: [`Edit::Restore`](super::Edit::Restore).
//!
//! The page comes to hold exactly the version's blocks, with their ids, props and nesting.
//! Each level of the page is matched with the same level of the version by the blocks' ids
//! ([`Key`]): a block both have stays in the page, where the version has it, and only its
//! content element is written anew, when it differs; its children are restored the same way,
//! a level down. The page's other blocks go, with what is under them, and the version's
//! others are written in, with their ids, at their places. So text typed meanwhile in a block
//! the version holds as the page does stays, as nothing of that block changes.

use std::cmp::Reverse;
use std::collections::HashMap;

use yrs::branch::BranchID;
use yrs::{TransactionMut, XmlElementPrelim, XmlElementRef, XmlFragment as _, XmlOut};

use super::{
    Applied, EditError, branch, children_of, containers, group_or_new, id_of, make_placed, tidy,
    top_group,
};
use crate::align::{Key, pairs};
use crate::ids::IdSource;
use crate::model::{Block, BlockKind};
use crate::ydoc::{BLOCK_GROUP, read_kind, write_blocks_at, write_kind};

pub(super) fn restore(
    txn: &mut TransactionMut,
    fragment: &yrs::XmlFragmentRef,
    blocks: &[Block],
    ids: &mut dyn IdSource,
) -> Result<Applied, EditError> {
    let Some(group) = top_group(fragment)? else {
        if blocks.is_empty() {
            return Ok(Applied::default());
        }
        let group = group_or_new(txn, fragment)?;
        return Ok(Applied {
            added: write_blocks_at(txn, &group, 0, blocks, ids),
            ..Applied::default()
        });
    };
    let mut applied = Applied::default();
    restore_group(txn, &group, blocks, ids, &mut applied);
    // A version without blocks leaves the empty paragraph a page keeps.
    tidy(txn, &group, ids);
    Ok(applied)
}

/// A block of the page at one level: its container, its id, and its type with its props and
/// content when they read.
struct Present {
    container: XmlElementRef,
    id: Option<String>,
    kind: Option<BlockKind>,
}

/// Makes `group` hold `blocks`, a level of the version, and the levels under them.
fn restore_group(
    txn: &mut TransactionMut,
    group: &XmlElementRef,
    blocks: &[Block],
    ids: &mut dyn IdSource,
    applied: &mut Applied,
) {
    let present: Vec<Present> = containers(txn, group)
        .into_iter()
        .map(|(_, container)| {
            let id = id_of(txn, &container);
            let kind = match container.first_child() {
                Some(XmlOut::Element(content)) => read_kind(txn, &content).ok(),
                _ => None,
            };
            Present {
                container,
                id,
                kind,
            }
        })
        .collect();
    let kept = {
        let old: Vec<Option<Key<'_>>> = present
            .iter()
            .map(|block| {
                block
                    .kind
                    .as_ref()
                    .map(|kind| Key::of(block.id.as_deref(), kind))
            })
            .collect();
        let new: Vec<Key<'_>> = blocks.iter().map(Key::of_block).collect();
        pairs(&old, &new)
    };

    // The blocks that stay keep their place in this group, so the levels under them change
    // first, and the group's indexes still hold after.
    for &(i, j) in &kept {
        let block = &blocks[j];
        if present[i].kind.as_ref() != Some(&block.kind) {
            rewrite(txn, &present[i].container, &block.kind);
        }
        restore_children(txn, &present[i].container, &block.children, ids, applied);
    }

    let index: HashMap<BranchID, u32> = containers(txn, group)
        .into_iter()
        .map(|(at, container)| (branch(&container), at))
        .collect();
    let at = |container: &XmlElementRef| index.get(&branch(container)).copied().unwrap_or(0);
    // Each run of the version's blocks the page lacks goes before the block that stays after
    // it, or after the last that stays, or at the start when none does: (place, 1, run).
    // Each block of the page that goes is (place, 0, its index).
    let mut runs: Vec<&[Block]> = Vec::new();
    let mut placed: Vec<(u32, u8, usize)> = Vec::new();
    let mut from = 0;
    let mut last: Option<usize> = None;
    for end in kept.iter().map(Some).chain([None]) {
        let to = end.map_or(blocks.len(), |&(_, j)| j);
        if to > from {
            let place = match (end, last) {
                (Some(&(i, _)), _) => at(&present[i].container),
                (None, Some(i)) => at(&present[i].container) + 1,
                (None, None) => 0,
            };
            placed.push((place, 1, runs.len()));
            runs.push(&blocks[from..to]);
        }
        if let Some(&(i, j)) = end {
            from = j + 1;
            last = Some(i);
        }
    }
    let mut stays = vec![false; present.len()];
    for &(i, _) in &kept {
        stays[i] = true;
    }
    for (i, block) in present.iter().enumerate() {
        if !stays[i] {
            placed.push((at(&block.container), 0, i));
            applied.removed += 1;
            applied.removed_ids.extend(block.id.clone());
        }
    }
    // From the last place back, so the places before stay; at one place, what goes before
    // what comes.
    placed.sort_by_key(|&(place, kind, n)| (Reverse(place), kind, Reverse(n)));
    make_placed(txn, group, &placed, |txn, place, n| {
        applied
            .added
            .extend(write_blocks_at(txn, group, place, runs[n], ids));
    });
}

/// Writes a block's content element anew, for a block that stays with other content.
fn rewrite(txn: &mut TransactionMut, container: &XmlElementRef, kind: &BlockKind) {
    container.remove_range(txn, 0, 1);
    let content = container.insert(txn, 0, XmlElementPrelim::empty(kind.name()));
    write_kind(txn, &content, kind);
}

/// Makes the children of a block that stays the version's.
fn restore_children(
    txn: &mut TransactionMut,
    container: &XmlElementRef,
    children: &[Block],
    ids: &mut dyn IdSource,
    applied: &mut Applied,
) {
    match children_of(txn, container) {
        // The schema has no empty group of children: it goes whole.
        Some(nested) if children.is_empty() => {
            for (_, child) in containers(txn, &nested) {
                applied.removed += 1;
                applied.removed_ids.extend(id_of(txn, &child));
            }
            container.remove_range(txn, 1, 1);
        }
        Some(nested) => restore_group(txn, &nested, children, ids, applied),
        None if children.is_empty() => {}
        None => {
            let nested = container.push_back(txn, XmlElementPrelim::empty(BLOCK_GROUP));
            applied
                .added
                .extend(write_blocks_at(txn, &nested, 0, children, ids));
        }
    }
}

#[cfg(test)]
mod tests {
    use yrs::updates::decoder::Decode as _;
    use yrs::{
        Doc, ReadTxn as _, StateVector, Text as _, Transact as _, Update, XmlFragment as _, XmlOut,
    };

    use super::super::{Edit, apply};
    use crate::ids::SequentialIds;
    use crate::markdown::from_markdown;
    use crate::model::{Block, BlockKind, Document, HeadingProps, Inline, TextProps};
    use crate::ydoc::{fragment, read_doc, replace};

    /// A document holding `blocks` as given, ids included, those without one numbered
    /// `block-1` and on.
    fn page(client: u64, blocks: &[Block]) -> Doc {
        let doc = Doc::with_client_id(client);
        let fragment = fragment(&doc);
        replace(
            &mut doc.transact_mut(),
            &fragment,
            &Document::new(blocks.to_vec()),
            &mut SequentialIds::new("block"),
        );
        doc
    }

    fn restore(doc: &Doc, version: &[Block]) -> super::Applied {
        let fragment = fragment(doc);
        let mut txn = doc.transact_mut();
        apply(
            &mut txn,
            &fragment,
            &Edit::Restore {
                blocks: version.to_vec(),
            },
            &mut SequentialIds::new("new"),
            usize::MAX,
        )
        .expect("restored")
    }

    fn named(id: &str, text: &str) -> Block {
        Block::paragraph(text).with_id(id)
    }

    fn heading(id: &str, text: &str) -> Block {
        Block::new(BlockKind::Heading {
            props: HeadingProps {
                level: 2,
                ..HeadingProps::default()
            },
            content: Inline::plain(text),
        })
        .with_id(id)
    }

    /// The page restored to `version` holds exactly it, read back with its ids.
    fn restores_to(now: &[Block], version: &[Block]) -> super::Applied {
        let doc = page(1, now);
        let applied = restore(&doc, version);
        assert_eq!(read_doc(&doc).expect("blocks").blocks, version);
        applied
    }

    #[test]
    fn the_page_comes_to_hold_exactly_the_version() {
        let version = vec![
            heading("a", "Plan"),
            named("b", "First").with_children(vec![named("c", "Nested")]),
            named("d", "Last"),
        ];
        // Text changed, blocks added, removed, reordered, retyped and renested.
        restores_to(
            &[
                named("a", "Plan, edited"),
                named("x", "Added since"),
                named("d", "Last"),
                named("b", "First"),
            ],
            &version,
        );
        restores_to(&[named("a", "Plan")], &version);
        restores_to(&[], &version);
        restores_to(
            &[named("b", "First").with_children(vec![named("y", "Other"), named("c", "N")])],
            &version,
        );
        let colored = Block::new(BlockKind::Paragraph {
            props: TextProps {
                text_color: "red".into(),
                ..TextProps::default()
            },
            content: Inline::plain("Last"),
        })
        .with_id("d");
        restores_to(&[colored], &version);
        // Children a block no longer has go with their group.
        restores_to(
            &[named("d", "Last").with_children(vec![named("z", "Under")])],
            &[named("d", "Last")],
        );
    }

    #[test]
    fn what_stays_the_same_is_not_touched() {
        let blocks = vec![named("a", "Same"), named("b", "Same too")];
        let doc = page(1, &blocks);
        let before = doc.transact().state_vector();
        let applied = restore(&doc, &blocks);
        assert_eq!(applied, super::Applied::default());
        let update = doc.transact().encode_state_as_update_v1(&before);
        assert_eq!(update, Update::EMPTY_V1, "nothing changed");
    }

    #[test]
    fn it_says_what_it_added_and_removed() {
        let applied = restores_to(
            &[
                named("a", "Kept"),
                named("x", "Gone").with_children(vec![named("y", "Under")]),
            ],
            &[named("a", "Kept"), named("n", "New")],
        );
        assert_eq!(applied.added, ["n"]);
        assert_eq!(
            (applied.removed, applied.removed_ids),
            (1, vec!["x".to_owned()])
        );
    }

    #[test]
    fn an_empty_version_leaves_an_empty_paragraph_and_an_empty_page_takes_any() {
        let doc = page(1, &[named("a", "Text")]);
        restore(&doc, &[]);
        let left = read_doc(&doc).expect("blocks").blocks;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].kind, Block::paragraph("").kind);

        let empty = Doc::with_client_id(1);
        restore(&empty, &[named("a", "Text")]);
        assert_eq!(
            read_doc(&empty).expect("blocks").blocks,
            [named("a", "Text")]
        );
    }

    #[test]
    fn blocks_without_ids_match_by_what_they_hold() {
        let version = from_markdown("# Plan\n\nKept\n\nNew\n").blocks;
        let doc = page(1, &from_markdown("Kept\n\nOld\n").blocks);
        restore(&doc, &version);
        let read = read_doc(&doc).expect("blocks");
        let texts: Vec<String> = read
            .blocks
            .iter()
            .map(|block| Document::new(vec![block.clone()]).plain_text())
            .collect();
        assert_eq!(texts, ["Plan", "Kept", "New"]);
        // The kept paragraph is the page's own: its id is the one it had.
        assert_eq!(read.blocks[1].id.as_deref(), Some("block-1"));
    }

    /// Text typed at the end of the first text of the block `id`, as the update that types it.
    fn type_into(doc: &Doc, id: &str, text: &str) -> Vec<u8> {
        let fragment = fragment(doc);
        let mut txn = doc.transact_mut();
        let Some(XmlOut::Element(group)) = fragment.first_child() else {
            panic!("a document");
        };
        let (_, container) = super::containers(&txn, &group)
            .into_iter()
            .find(|(_, container)| super::id_of(&txn, container).as_deref() == Some(id))
            .expect("the block");
        let Some(XmlOut::Element(content)) = container.first_child() else {
            panic!("its content");
        };
        let Some(XmlOut::Text(run)) = content.first_child() else {
            panic!("its text");
        };
        let end = run.len(&txn);
        run.insert(&mut txn, end, text);
        txn.encode_update_v1()
    }

    #[test]
    fn typing_meanwhile_in_a_block_the_version_has_too_stays() {
        let now = [named("a", "Kept"), named("b", "Changed since")];
        let version = [named("a", "Kept"), named("b", "Before")];
        let server = page(1, &now);
        let client = Doc::with_client_id(2);
        let state = server
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        client
            .transact_mut()
            .apply_update(Update::decode_v1(&state).expect("decodes"))
            .expect("applies");

        // The client types in both blocks while the server restores; each then gets the
        // other's change.
        let typed = type_into(&client, "a", " and typed");
        let before = server.transact().state_vector();
        restore(&server, &version);
        let restored = server.transact().encode_state_as_update_v1(&before);
        server
            .transact_mut()
            .apply_update(Update::decode_v1(&typed).expect("decodes"))
            .expect("applies");
        client
            .transact_mut()
            .apply_update(Update::decode_v1(&restored).expect("decodes"))
            .expect("applies");

        let expected = [named("a", "Kept and typed"), named("b", "Before")];
        assert_eq!(read_doc(&server).expect("blocks").blocks, expected);
        assert_eq!(read_doc(&client).expect("blocks").blocks, expected);
    }
}
