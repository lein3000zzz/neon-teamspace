//! What changed between two versions of a document, block by block: each block of either,
//! in the order of the later one, with the earlier one's blocks that went at the places
//! they had, and whether it stayed the same, changed, was added or was removed.
//!
//! Blocks are matched level by level, by their ids, as the editor keeps a block's id however
//! its content changes; blocks without an id match a block that holds the same. A block
//! both versions have is the same when its type, props and content are, whatever its
//! children, which are compared a level down; else it changed. A block moved under another
//! is removed where it was and added where it is.

use std::iter;

use crate::align::{Key, pairs};
use crate::model::{Block, Document};

/// What happened to a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Same,
    Changed,
    Added,
    Removed,
}

/// One block of the comparison, without its children, which follow it a level deeper.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub change: Change,
    /// How deep it is: 0 at the top of the page.
    pub depth: usize,
    /// The block as the earlier version has it; `None` when it was added.
    pub before: Option<Block>,
    /// The block as the later version has it; `None` when it was removed.
    pub after: Option<Block>,
}

/// How many blocks were added, changed and removed, at any depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

impl Summary {
    pub fn of(lines: &[Line]) -> Self {
        let count = |change: Change| lines.iter().filter(|line| line.change == change).count();
        Self {
            added: count(Change::Added),
            changed: count(Change::Changed),
            removed: count(Change::Removed),
        }
    }

    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The blocks of `before` and `after` compared: see the module docs.
pub fn diff(before: &Document, after: &Document) -> Vec<Line> {
    let mut lines = Vec::new();
    level(&before.blocks, &after.blocks, 0, &mut lines);
    lines
}

fn level(before: &[Block], after: &[Block], depth: usize, lines: &mut Vec<Line>) {
    let old: Vec<Option<Key<'_>>> = before
        .iter()
        .map(|block| Some(Key::of_block(block)))
        .collect();
    let new: Vec<Key<'_>> = after.iter().map(Key::of_block).collect();
    let (mut i, mut j) = (0, 0);
    for (paired_i, paired_j) in pairs(&old, &new)
        .into_iter()
        .chain(iter::once((before.len(), after.len())))
    {
        for block in &before[i..paired_i] {
            subtree(block, Change::Removed, depth, lines);
        }
        for block in &after[j..paired_j] {
            subtree(block, Change::Added, depth, lines);
        }
        if let (Some(old), Some(new)) = (before.get(paired_i), after.get(paired_j)) {
            let change = if old.kind == new.kind {
                Change::Same
            } else {
                Change::Changed
            };
            lines.push(Line {
                change,
                depth,
                before: Some(alone(old)),
                after: Some(alone(new)),
            });
            level(&old.children, &new.children, depth + 1, lines);
        }
        (i, j) = (paired_i + 1, paired_j + 1);
    }
}

/// A block added or removed, with every block under it.
fn subtree(block: &Block, change: Change, depth: usize, lines: &mut Vec<Line>) {
    let (before, after) = match change {
        Change::Removed => (Some(alone(block)), None),
        _ => (None, Some(alone(block))),
    };
    lines.push(Line {
        change,
        depth,
        before,
        after,
    });
    for child in &block.children {
        subtree(child, change, depth + 1, lines);
    }
}

/// A block without its children.
fn alone(block: &Block) -> Block {
    Block {
        id: block.id.clone(),
        kind: block.kind.clone(),
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::from_markdown;

    fn named(id: &str, text: &str) -> Block {
        Block::paragraph(text).with_id(id)
    }

    fn changes(before: &[Block], after: &[Block]) -> Vec<(Change, usize, String)> {
        diff(
            &Document::new(before.to_vec()),
            &Document::new(after.to_vec()),
        )
        .into_iter()
        .map(|line| {
            let block = line.after.or(line.before).expect("a block");
            let text = Document::new(vec![block]).plain_text();
            (line.change, line.depth, text)
        })
        .collect()
    }

    #[test]
    fn the_same_document_has_no_changes() {
        let blocks = vec![named("a", "One").with_children(vec![named("b", "Two")])];
        let lines = diff(&Document::new(blocks.clone()), &Document::new(blocks));
        assert!(lines.iter().all(|line| line.change == Change::Same));
        assert_eq!(lines.len(), 2);
        assert!(Summary::of(&lines).is_empty());
    }

    #[test]
    fn a_block_keeps_its_id_when_its_text_changes() {
        assert_eq!(
            changes(
                &[named("a", "Plan"), named("b", "Old")],
                &[named("a", "Plan"), named("b", "New")]
            ),
            [
                (Change::Same, 0, "Plan".to_owned()),
                (Change::Changed, 0, "New".to_owned())
            ]
        );
    }

    #[test]
    fn removed_blocks_stay_where_they_were_and_added_ones_come_where_they_are() {
        let lines = changes(
            &[
                named("a", "Kept"),
                named("x", "Gone").with_children(vec![named("y", "Under it")]),
                named("c", "End"),
            ],
            &[
                named("a", "Kept"),
                named("n", "New"),
                named("c", "End").with_children(vec![named("m", "Nested new")]),
            ],
        );
        assert_eq!(
            lines,
            [
                (Change::Same, 0, "Kept".to_owned()),
                (Change::Removed, 0, "Gone".to_owned()),
                (Change::Removed, 1, "Under it".to_owned()),
                (Change::Added, 0, "New".to_owned()),
                (Change::Same, 0, "End".to_owned()),
                (Change::Added, 1, "Nested new".to_owned()),
            ]
        );
        let summary = Summary::of(&diff(
            &Document::new(vec![named("x", "Gone"), named("c", "End")]),
            &Document::new(vec![named("c", "End, edited"), named("n", "New")]),
        ));
        assert_eq!(
            summary,
            Summary {
                added: 1,
                changed: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn a_block_moved_under_another_is_removed_and_added() {
        assert_eq!(
            changes(
                &[named("a", "Parent"), named("b", "Moved")],
                &[named("a", "Parent").with_children(vec![named("b", "Moved")])]
            ),
            [
                (Change::Same, 0, "Parent".to_owned()),
                (Change::Added, 1, "Moved".to_owned()),
                (Change::Removed, 0, "Moved".to_owned()),
            ]
        );
    }

    #[test]
    fn blocks_without_ids_match_what_holds_the_same() {
        let before = from_markdown("Kept\n\nOld\n");
        let after = from_markdown("Kept\n\nNew\n");
        let lines: Vec<Change> = diff(&before, &after)
            .into_iter()
            .map(|line| line.change)
            .collect();
        assert_eq!(lines, [Change::Same, Change::Removed, Change::Added]);
    }
}
