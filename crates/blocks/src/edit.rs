//! Edits of a page's Yjs document in place: blocks added, replaced or removed, text found and
//! replaced, the whole body replaced, each written in the shape the editor writes.
//!
//! An edit changes only what it names, inside one transaction, so it merges with what others
//! change at the same time: text typed elsewhere in the document, in a block an edit keeps,
//! or elsewhere in a block whose text is replaced, stays. Blocks are named by their ids, which
//! [`to_markdown_with_ids`](crate::markdown::to_markdown_with_ids) shows.
//!
//! - **Append** adds blocks at the end of the page, before the empty paragraph the editor
//!   keeps last when there is one, so typing on after them stays possible.
//! - **Insert after** adds blocks right after a block, among its siblings, under the block it
//!   is under.
//! - **Replace a range** replaces the blocks from one to another, siblings in document order,
//!   both included, with the blocks given, or with none, which removes them.
//! - **Find and replace** replaces text within the inline content of every block, table
//!   cells and code included. The text is matched exactly, character for character, within
//!   one line: it holds no line break, nor does its replacement. The replacement takes the
//!   styles and link of the first character it replaces. Unless every occurrence is to be
//!   replaced, the text must occur once in the page; it is replaced at most
//!   [`MAX_REPLACEMENTS`] times, and never so that the replacements add more text than the
//!   room the edit is given. Each text node is rewritten in one pass, however many times its
//!   text occurs in it.
//! - **Replace the body** replaces every block, but those that write the same markdown as
//!   one of the blocks given, in the same order: they stay as they are, with their ids, their
//!   colours and whatever is typed in them meanwhile.
//!
//! A page keeps at least one block: an edit that removes every block leaves an empty
//! paragraph, as the editor does. Each edit checks what it needs before it changes
//! anything, so one that fails changes nothing.

use yrs::types::text::YChange;
use yrs::types::{Attrs, Delta};
use yrs::{
    Any, OffsetKind, Out, ReadTxn, Text, TransactionMut, Xml, XmlElementPrelim, XmlElementRef,
    XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};

use crate::ids::IdSource;
use crate::markdown::to_markdown;
use crate::model::{Block, Document};
use crate::ydoc::{
    BLOCK_CONTAINER, BLOCK_GROUP, NotADocument, TABLE, TABLE_CELL, TABLE_HEADER, TABLE_PARAGRAPH,
    TABLE_ROW, read_container, write_blocks_at,
};

/// How many pairs of blocks replacing the body compares, at most, to find the ones that
/// stay; past that, the blocks between those the old and new bodies start and end with are
/// all replaced.
const MAX_COMPARISONS: usize = 4_000_000;

/// The most times one find and replace replaces its text.
pub const MAX_REPLACEMENTS: usize = 10_000;

/// A change to a page's blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Adds blocks at the end of the page.
    Append { blocks: Vec<Block> },
    /// Adds blocks right after the block of `block_id`.
    InsertAfter {
        block_id: String,
        blocks: Vec<Block>,
    },
    /// Replaces the blocks from `from` to `to`, siblings, both included; with no blocks,
    /// removes them.
    ReplaceRange {
        from: String,
        to: String,
        blocks: Vec<Block>,
    },
    /// Replaces `find` with `replace`: everywhere when `all`, else where it occurs once.
    FindReplace {
        find: String,
        replace: String,
        all: bool,
    },
    /// Replaces every block, keeping those that stay the same.
    ReplaceBody { blocks: Vec<Block> },
}

impl Edit {
    /// A short name, for logs and metric labels.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Append { .. } => "append",
            Self::InsertAfter { .. } => "insert_after",
            Self::ReplaceRange { .. } => "replace_range",
            Self::FindReplace { .. } => "find_replace",
            Self::ReplaceBody { .. } => "replace_body",
        }
    }

    /// How many bytes of text this edit added by replacing, having done `applied`: what
    /// counts against the room of the edits after it.
    pub fn replaced_bytes(&self, applied: &Applied) -> usize {
        match self {
            Self::FindReplace { replace, .. } => applied.replaced.saturating_mul(replace.len()),
            _ => 0,
        }
    }
}

/// What an edit did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The ids of the blocks it added, in document order, those under them left out.
    pub added: Vec<String>,
    /// How many blocks it removed, those under them left out.
    pub removed: usize,
    /// How many times it replaced the text it was to find.
    pub replaced: usize,
}

/// Why an edit cannot be made. The messages name blocks by id and never quote the page.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the page has no block {0}")]
    NoBlock(String),
    #[error("nothing to add: the markdown makes no blocks")]
    NoBlocks,
    #[error(
        "blocks {from} and {to} are not siblings: a range runs between blocks under the same \
         block, or both at the top"
    )]
    NotSiblings { from: String, to: String },
    #[error("block {to} comes before block {from}: a range runs from its first block to its last")]
    Backwards { from: String, to: String },
    #[error("the text to find is empty")]
    EmptyFind,
    #[error(
        "the text to find and its replacement are within one line: to change more, replace \
         the blocks"
    )]
    LineBreak,
    #[error("the text to find is not in the page")]
    NotFound,
    #[error(
        "the text to find is in the page {0} times: give more of it so it is there once, or \
         replace every occurrence"
    )]
    Ambiguous(usize),
    #[error(
        "the text to find is in the page {0} times, and find and replace replaces it at most \
         {max} times: give more of it, or replace the blocks",
        max = MAX_REPLACEMENTS
    )]
    TooManyMatches(usize),
    #[error("the replacements would add more text than the page has room for")]
    TooLarge,
    #[error(transparent)]
    NotADocument(#[from] NotADocument),
}

/// Applies an edit to the blocks of a fragment. New blocks get their ids from `ids`, parents
/// before their children. `room` is how many bytes of text find and replace may add: one
/// that would add more is refused before it changes anything. Nothing changes when it fails.
pub fn apply(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    edit: &Edit,
    ids: &mut dyn IdSource,
    room: usize,
) -> Result<Applied, EditError> {
    match edit {
        Edit::Append { blocks } => append(txn, fragment, blocks, ids),
        Edit::InsertAfter { block_id, blocks } => {
            insert_after(txn, fragment, block_id, blocks, ids)
        }
        Edit::ReplaceRange { from, to, blocks } => {
            replace_range(txn, fragment, from, to, blocks, ids)
        }
        Edit::FindReplace { find, replace, all } => {
            find_replace(txn, fragment, find, replace, *all, room)
        }
        Edit::ReplaceBody { blocks } => replace_body(txn, fragment, blocks, ids),
    }
}

/// The fragment's top-level block group, if it has one.
fn top_group(fragment: &XmlFragmentRef) -> Result<Option<XmlElementRef>, EditError> {
    match fragment.first_child() {
        None => Ok(None),
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => Ok(Some(group)),
        Some(_) => Err(NotADocument.into()),
    }
}

/// The top-level block group, made when the fragment is empty, as for a page never written.
fn group_or_new(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
) -> Result<XmlElementRef, EditError> {
    match top_group(fragment)? {
        Some(group) => Ok(group),
        None => Ok(fragment.push_back(txn, XmlElementPrelim::empty(BLOCK_GROUP))),
    }
}

/// A block's place: the group holding it, and its index there.
struct Place {
    group: XmlElementRef,
    index: u32,
}

/// The blocks of a group, with their indexes.
fn containers<T: ReadTxn>(txn: &T, group: &XmlElementRef) -> Vec<(u32, XmlElementRef)> {
    (0u32..)
        .zip(group.children(txn))
        .filter_map(|(index, child)| match child {
            XmlOut::Element(container) if container.tag().as_ref() == BLOCK_CONTAINER => {
                Some((index, container))
            }
            _ => None,
        })
        .collect()
}

fn id_of<T: ReadTxn>(txn: &T, container: &XmlElementRef) -> Option<String> {
    match container.get_attribute(txn, "id") {
        Some(Out::Any(Any::String(id))) => Some(id.to_string()),
        _ => None,
    }
}

/// The group of a block's children, if it has any.
fn children_of<T: ReadTxn>(txn: &T, container: &XmlElementRef) -> Option<XmlElementRef> {
    container
        .children(txn)
        .nth(1)
        .and_then(|child| match child {
            XmlOut::Element(group) if group.tag().as_ref() == BLOCK_GROUP => Some(group),
            _ => None,
        })
}

/// Where the block of `id` is, at any depth.
fn locate<T: ReadTxn>(txn: &T, group: &XmlElementRef, id: &str) -> Option<Place> {
    for (index, container) in containers(txn, group) {
        if id_of(txn, &container).as_deref() == Some(id) {
            return Some(Place {
                group: group.clone(),
                index,
            });
        }
        if let Some(found) =
            children_of(txn, &container).and_then(|nested| locate(txn, &nested, id))
        {
            return Some(found);
        }
    }
    None
}

fn find_block<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    id: &str,
) -> Result<Place, EditError> {
    top_group(fragment)?
        .and_then(|group| locate(txn, &group, id))
        .ok_or_else(|| EditError::NoBlock(id.to_owned()))
}

/// Whether a block is an empty paragraph without children: what the editor keeps last.
fn is_empty_paragraph<T: ReadTxn>(txn: &T, container: &XmlElementRef) -> bool {
    let mut children = container.children(txn);
    let first = children.next();
    children.next().is_none()
        && matches!(
            first,
            Some(XmlOut::Element(content))
                if content.tag().as_ref() == "paragraph" && content.len(txn) == 0
        )
}

/// After blocks were removed from a group: a group of children left empty goes, as the
/// editor's schema has no empty ones, and a page left without blocks gets an empty
/// paragraph.
fn tidy(txn: &mut TransactionMut, group: &XmlElementRef, ids: &mut dyn IdSource) {
    if group.len(txn) > 0 {
        return;
    }
    match group.parent() {
        Some(XmlOut::Element(container)) if container.tag().as_ref() == BLOCK_CONTAINER => {
            let index = (0u32..)
                .zip(container.children(txn))
                .find_map(|(at, child)| {
                    matches!(&child, XmlOut::Element(element) if element == group).then_some(at)
                });
            if let Some(index) = index {
                container.remove_range(txn, index, 1);
            }
        }
        _ => {
            write_blocks_at(txn, group, 0, &[Block::paragraph("")], ids);
        }
    }
}

fn append(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    blocks: &[Block],
    ids: &mut dyn IdSource,
) -> Result<Applied, EditError> {
    if blocks.is_empty() {
        return Err(EditError::NoBlocks);
    }
    let group = group_or_new(txn, fragment)?;
    let at = match containers(txn, &group).last() {
        Some((index, last)) if is_empty_paragraph(txn, last) => *index,
        _ => group.len(txn),
    };
    Ok(Applied {
        added: write_blocks_at(txn, &group, at, blocks, ids),
        ..Applied::default()
    })
}

fn insert_after(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    block_id: &str,
    blocks: &[Block],
    ids: &mut dyn IdSource,
) -> Result<Applied, EditError> {
    if blocks.is_empty() {
        return Err(EditError::NoBlocks);
    }
    let place = find_block(txn, fragment, block_id)?;
    Ok(Applied {
        added: write_blocks_at(txn, &place.group, place.index + 1, blocks, ids),
        ..Applied::default()
    })
}

fn replace_range(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    from: &str,
    to: &str,
    blocks: &[Block],
    ids: &mut dyn IdSource,
) -> Result<Applied, EditError> {
    let start = find_block(txn, fragment, from)?;
    let end = containers(txn, &start.group)
        .into_iter()
        .find(|(_, container)| id_of(txn, container).as_deref() == Some(to))
        .map(|(index, _)| index);
    let Some(end) = end else {
        find_block(txn, fragment, to)?;
        return Err(EditError::NotSiblings {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    };
    if end < start.index {
        return Err(EditError::Backwards {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }
    // The new blocks go in first, so the group is never left empty between the two.
    let added = write_blocks_at(txn, &start.group, end + 1, blocks, ids);
    let removed = end - start.index + 1;
    start.group.remove_range(txn, start.index, removed);
    tidy(txn, &start.group, ids);
    Ok(Applied {
        added,
        removed: removed as usize,
        replaced: 0,
    })
}

/// A text node of a block's inline content, with its text and the styles of its runs.
struct Run {
    text: XmlTextRef,
    content: String,
    /// Where each run of the same styles starts in `content`, in bytes, and its styles.
    styles: Vec<(usize, Attrs)>,
}

impl Run {
    /// The styles of the characters at `at`, byte offsets of `content` in ascending order, in
    /// one pass over the runs of styles.
    fn styles_at(&self, at: &[usize]) -> Vec<Attrs> {
        let mut style = 0;
        at.iter()
            .map(|&byte| {
                while self
                    .styles
                    .get(style + 1)
                    .is_some_and(|(start, _)| *start <= byte)
                {
                    style += 1;
                }
                self.styles
                    .get(style)
                    .filter(|(start, _)| *start <= byte)
                    .map(|(_, attrs)| attrs.clone())
                    .unwrap_or_default()
            })
            .collect()
    }
}

/// The text nodes of the inline content of every block under `group`, in document order.
fn runs<T: ReadTxn>(txn: &T, group: &XmlElementRef, found: &mut Vec<Run>) {
    for (_, container) in containers(txn, group) {
        if let Some(XmlOut::Element(content)) = container.children(txn).next() {
            if content.tag().as_ref() == TABLE {
                for paragraph in table_paragraphs(txn, &content) {
                    texts_of(txn, &paragraph, found);
                }
            } else {
                texts_of(txn, &content, found);
            }
        }
        if let Some(nested) = children_of(txn, &container) {
            runs(txn, &nested, found);
        }
    }
}

/// The paragraphs of a table's cells, row by row.
fn table_paragraphs<T: ReadTxn>(txn: &T, table: &XmlElementRef) -> Vec<XmlElementRef> {
    let elements = |parent: &XmlElementRef, tags: &[&str]| -> Vec<XmlElementRef> {
        parent
            .children(txn)
            .filter_map(|child| match child {
                XmlOut::Element(element) if tags.contains(&element.tag().as_ref()) => Some(element),
                _ => None,
            })
            .collect()
    };
    elements(table, &[TABLE_ROW])
        .iter()
        .flat_map(|row| elements(row, &[TABLE_CELL, TABLE_HEADER]))
        .flat_map(|cell| elements(&cell, &[TABLE_PARAGRAPH]))
        .collect()
}

/// The text nodes directly in an element. One holding anything but text, which the editor
/// never writes, is passed over.
fn texts_of<T: ReadTxn>(txn: &T, element: &XmlElementRef, found: &mut Vec<Run>) {
    for child in element.children(txn) {
        let XmlOut::Text(text) = child else {
            continue;
        };
        let mut content = String::new();
        let mut styles = Vec::new();
        let mut plain = true;
        for chunk in text.diff(txn, YChange::identity) {
            let Out::Any(Any::String(value)) = chunk.insert else {
                plain = false;
                break;
            };
            styles.push((
                content.len(),
                chunk.attributes.map(|attrs| *attrs).unwrap_or_default(),
            ));
            content.push_str(&value);
        }
        if plain && !content.is_empty() {
            found.push(Run {
                text,
                content,
                styles,
            });
        }
    }
}

/// The length of `text` in the units the document counts offsets in.
fn units(text: &str, kind: OffsetKind) -> u32 {
    let len = match kind {
        OffsetKind::Bytes => text.len(),
        OffsetKind::Utf16 => text.encode_utf16().count(),
    };
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn find_replace(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    find: &str,
    replace: &str,
    all: bool,
    room: usize,
) -> Result<Applied, EditError> {
    if find.is_empty() {
        return Err(EditError::EmptyFind);
    }
    if find.contains(['\n', '\r']) || replace.contains(['\n', '\r']) {
        return Err(EditError::LineBreak);
    }
    let mut found = Vec::new();
    if let Some(group) = top_group(fragment)? {
        runs(txn, &group, &mut found);
    }
    let matches: Vec<(&Run, Vec<usize>)> = found
        .iter()
        .map(|run| {
            let at: Vec<usize> = run.content.match_indices(find).map(|(at, _)| at).collect();
            (run, at)
        })
        .filter(|(_, at)| !at.is_empty())
        .collect();
    let count: usize = matches.iter().map(|(_, at)| at.len()).sum();
    match count {
        0 => return Err(EditError::NotFound),
        1 => {}
        _ if all => {}
        _ => return Err(EditError::Ambiguous(count)),
    }
    if count > MAX_REPLACEMENTS {
        return Err(EditError::TooManyMatches(count));
    }
    if count.saturating_mul(replace.len()) > room {
        return Err(EditError::TooLarge);
    }
    let kind = txn.doc().offset_kind();
    let find_len = units(find, kind);
    for (run, at) in matches {
        // One pass over the node: what lies between the matches is kept, and each match
        // makes way for its replacement, which goes in before it, as typing would.
        let mut delta: Vec<Delta<Any>> = Vec::with_capacity(at.len() * 3);
        let mut kept_from = 0;
        for (&byte, styles) in at.iter().zip(run.styles_at(&at)) {
            delta.push(Delta::Retain(
                units(&run.content[kept_from..byte], kind),
                None,
            ));
            if !replace.is_empty() {
                delta.push(Delta::Inserted(Any::from(replace), Some(Box::new(styles))));
            }
            delta.push(Delta::Deleted(find_len));
            kept_from = byte + find.len();
        }
        run.text.apply_delta(txn, delta);
    }
    Ok(Applied {
        replaced: count,
        ..Applied::default()
    })
}

/// A block as markdown writes it alone, by which the blocks that stay are told.
fn written(block: Block) -> String {
    to_markdown(&Document::new(vec![block]))
}

fn replace_body(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
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
    let old: Vec<Option<String>> = group
        .children(txn)
        .map(|child| match child {
            XmlOut::Element(container) if container.tag().as_ref() == BLOCK_CONTAINER => {
                read_container(txn, &container, None).ok().map(written)
            }
            _ => None,
        })
        .collect();
    let new: Vec<String> = blocks.iter().cloned().map(written).collect();
    let kept = common(&old, &new);
    // Between the blocks that stay, before the first and after the last, the old blocks go
    // and the new ones come, in document order: the new ones first, so the group is never
    // empty between the two.
    let starts = std::iter::once((0, 0)).chain(kept.iter().map(|&(i, j)| (i + 1, j + 1)));
    let ends = kept.iter().copied().chain([(old.len(), new.len())]);
    let mut applied = Applied::default();
    for ((old_start, new_start), (old_end, new_end)) in starts.zip(ends) {
        // Where the old block at `old_start` is now.
        let at = index(old_start + applied.added.len() - applied.removed);
        let added = write_blocks_at(txn, &group, at, &blocks[new_start..new_end], ids);
        let gone = old_end - old_start;
        if gone > 0 {
            group.remove_range(txn, at + index(added.len()), index(gone));
        }
        applied.removed += gone;
        applied.added.extend(added);
    }
    tidy(txn, &group, ids);
    Ok(applied)
}

fn index(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// The pairs of indexes of `old` and `new` whose blocks stay: a longest common sequence of
/// them, in order. Blocks that do not read never stay.
fn common(old: &[Option<String>], new: &[String]) -> Vec<(usize, usize)> {
    let same = |i: usize, j: usize| old[i].as_deref() == Some(new[j].as_str());
    let mut head = 0;
    while head < old.len() && head < new.len() && same(head, head) {
        head += 1;
    }
    let mut tail = 0;
    while tail < old.len() - head
        && tail < new.len() - head
        && same(old.len() - 1 - tail, new.len() - 1 - tail)
    {
        tail += 1;
    }
    let (first, n, m) = (head, old.len() - tail - head, new.len() - tail - head);
    let mut pairs: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    if n > 0 && m > 0 && n.saturating_mul(m) <= MAX_COMPARISONS {
        // lengths[cell(i, j)]: the longest common sequence of the middles from i and j on.
        let mut lengths = vec![0u32; (n + 1) * (m + 1)];
        let cell = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lengths[cell(i, j)] = if same(first + i, first + j) {
                    lengths[cell(i + 1, j + 1)] + 1
                } else {
                    lengths[cell(i + 1, j)].max(lengths[cell(i, j + 1)])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if same(first + i, first + j) {
                pairs.push((first + i, first + j));
                i += 1;
                j += 1;
            } else if lengths[cell(i + 1, j)] >= lengths[cell(i, j + 1)] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    pairs.extend(
        (0..tail)
            .rev()
            .map(|k| (old.len() - 1 - k, new.len() - 1 - k)),
    );
    pairs
}

#[cfg(test)]
mod tests;
