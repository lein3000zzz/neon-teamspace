//! Telling which blocks of two documents are the same block, level by level, for comparing
//! versions of a page and restoring one: a block is matched by its id, which it keeps however
//! its content changes, or, when it has none, by what it holds.

use crate::edit::common;
use crate::model::{Block, BlockKind};

/// What a block is matched by: two blocks with ids are the same block when their ids are
/// the same, and otherwise when they hold the same.
#[derive(Debug)]
pub(crate) struct Key<'a> {
    id: Option<&'a str>,
    kind: &'a BlockKind,
}

impl<'a> Key<'a> {
    pub(crate) fn of(id: Option<&'a str>, kind: &'a BlockKind) -> Self {
        Self { id, kind }
    }

    pub(crate) fn of_block(block: &'a Block) -> Self {
        Self::of(block.id.as_deref(), &block.kind)
    }
}

impl PartialEq for Key<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.id, other.id) {
            (Some(mine), Some(theirs)) => mine == theirs,
            _ => self.kind == other.kind,
        }
    }
}

/// The pairs of indexes of `old` and `new` that are the same block, in order: a longest
/// common sequence of their keys. A block keyed `None`, one that does not read, pairs with
/// none.
pub(crate) fn pairs(old: &[Option<Key<'_>>], new: &[Key<'_>]) -> Vec<(usize, usize)> {
    common(old, new)
}
