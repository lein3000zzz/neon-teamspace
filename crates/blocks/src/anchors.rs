//! Anchors of inline threads of comments: ranges of a page's text between two Yjs relative
//! positions, yrs' sticky indexes, as Yjs encodes them.
//!
//! The editor makes an anchor's positions from a selection with y-prosemirror, each in the
//! text, or the element, it falls in. A position keeps to the character after it, so the
//! range grows and shrinks with what is typed and deleted inside it and stays put as the
//! text around it changes, however the edits of several people merge. Once every character
//! of the range is deleted, both ends fall on the same place: the anchor is detached, and
//! its thread is kept, as the comments on the text that was there.
//!
//! Positions are compared in document order, counted as ProseMirror counts its positions:
//! text by its length, an element by its content and one at each side. Only what is in the
//! document counts, so a range whose text, and mentions, are all deleted is empty, however
//! many texts it spanned.

use std::collections::HashMap;

use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Assoc, BranchID, IndexedSequence, Offset, Out, ReadTxn, StickyIndex, Text, Xml,
    XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};

use crate::ydoc::{BLOCK_CONTAINER, BLOCK_GROUP};

/// Where an anchor is in the document now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchored {
    /// The block whose text the range starts in, while any of it is left.
    pub block_id: Option<String>,
    /// Every character of the range was deleted.
    pub detached: bool,
}

impl Anchored {
    const DETACHED: Self = Self {
        block_id: None,
        detached: true,
    };
}

/// An anchor over the whole text of a block, as a selection of it makes one: the encoded
/// positions of its start and end, and the text it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAnchor {
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub quote: String,
}

/// What the position count needs of each node of the fragment: where it starts, and
/// whether it is text, an element, or the fragment itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Fragment,
    Element,
    Text,
}

/// The fragment's nodes by branch, each with the position it starts at and its size.
struct Layout {
    nodes: HashMap<BranchID, (Node, u32)>,
    sizes: HashMap<BranchID, u32>,
}

impl Layout {
    fn new<T: ReadTxn>(txn: &T, fragment: &XmlFragmentRef) -> Self {
        let mut layout = Self {
            nodes: HashMap::new(),
            sizes: HashMap::new(),
        };
        layout
            .nodes
            .insert(branch_id(fragment), (Node::Fragment, 0));
        let mut at = 0;
        for child in fragment.children(txn) {
            at += layout.place(txn, &child, at);
        }
        layout.sizes.insert(branch_id(fragment), at);
        layout
    }

    /// Notes a node starting at `start` and everything in it, and says its size.
    fn place<T: ReadTxn>(&mut self, txn: &T, node: &XmlOut, start: u32) -> u32 {
        let size = match node {
            XmlOut::Text(text) => {
                self.nodes.insert(branch_id(text), (Node::Text, start));
                text.len(txn)
            }
            XmlOut::Element(element) => {
                self.nodes
                    .insert(branch_id(element), (Node::Element, start));
                let mut at = start + 1;
                for child in element.children(txn) {
                    at += self.place(txn, &child, at);
                }
                at + 1 - start
            }
            XmlOut::Fragment(_) => 0,
        };
        self.sizes.insert(branch_id(node), size);
        size
    }

    /// Where an offset falls, or `None` when what it is in is no longer in the document.
    fn position<T: ReadTxn>(&self, txn: &T, offset: &Offset) -> Option<u32> {
        let id = offset.branch.id();
        let (node, start) = *self.nodes.get(&id)?;
        if node == Node::Text {
            return Some(start + offset.index);
        }
        let element = XmlOut::try_from(offset.branch).ok()?;
        let children: Vec<XmlOut> = match &element {
            XmlOut::Element(element) => element.children(txn).collect(),
            XmlOut::Fragment(fragment) => fragment.children(txn).collect(),
            XmlOut::Text(_) => Vec::new(),
        };
        let before: u32 = children
            .iter()
            .take(usize::try_from(offset.index).unwrap_or(usize::MAX))
            .map(|child| self.sizes.get(&branch_id(child)).copied().unwrap_or(0))
            .sum();
        Some(start + u32::from(node == Node::Element) + before)
    }
}

fn branch_id(node: &impl AsRef<yrs::branch::Branch>) -> BranchID {
    node.as_ref().id()
}

/// Where each anchor, its start and end positions as Yjs encodes them, is in the
/// fragment now. A position that does not decode, or that names what the document does not
/// hold, leaves its anchor detached.
pub fn resolve<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    anchors: &[(&[u8], &[u8])],
) -> Vec<Anchored> {
    if anchors.is_empty() {
        return Vec::new();
    }
    let layout = Layout::new(txn, fragment);
    anchors
        .iter()
        .map(|(start, end)| {
            let offset = |bytes: &[u8]| StickyIndex::decode_v1(bytes).ok()?.get_offset(txn);
            let (Some(from), Some(to)) = (offset(start), offset(end)) else {
                return Anchored::DETACHED;
            };
            match (layout.position(txn, &from), layout.position(txn, &to)) {
                (Some(start), Some(end)) if start < end => Anchored {
                    block_id: block_of(txn, &from),
                    detached: false,
                },
                _ => Anchored::DETACHED,
            }
        })
        .collect()
}

/// The id of the block an offset is in.
fn block_of<T: ReadTxn>(txn: &T, offset: &Offset) -> Option<String> {
    let mut node = XmlOut::try_from(offset.branch).ok()?;
    loop {
        let parent = match &node {
            XmlOut::Element(element) if element.tag().as_ref() == BLOCK_CONTAINER => {
                return match element.get_attribute(txn, "id") {
                    Some(Out::Any(Any::String(id))) => Some(id.to_string()),
                    _ => None,
                };
            }
            XmlOut::Element(element) => element.parent(),
            XmlOut::Text(text) => text.parent(),
            XmlOut::Fragment(_) => return None,
        };
        node = parent?;
    }
}

/// An anchor over the whole text of the block with `id`, its table cells' included but not
/// its children's: from the start of its first text to the end of its last. `None` when no
/// such block holds any text.
pub fn block_anchor<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    id: &str,
) -> Option<BlockAnchor> {
    let container = find_container(txn, fragment.children(txn), id)?;
    let XmlOut::Element(content) = container.children(txn).next()? else {
        return None;
    };
    let mut texts = Vec::new();
    texts_in(txn, &content, &mut texts);
    let (first, last) = (texts.first()?, texts.last()?);
    let start = first.sticky_index(txn, 0, Assoc::After)?;
    let end = last.sticky_index(txn, last.len(txn), Assoc::After)?;
    let quote: String = texts
        .iter()
        .flat_map(|text| text.diff(txn, YChange::identity))
        .filter_map(|chunk| match chunk.insert {
            Out::Any(Any::String(text)) => Some(text.to_string()),
            _ => None,
        })
        .collect();
    Some(BlockAnchor {
        start: start.encode_v1(),
        end: end.encode_v1(),
        quote,
    })
}

fn find_container<T: ReadTxn>(
    txn: &T,
    children: impl Iterator<Item = XmlOut>,
    id: &str,
) -> Option<XmlElementRef> {
    for child in children {
        let XmlOut::Element(element) = child else {
            continue;
        };
        if element.tag().as_ref() == BLOCK_CONTAINER
            && matches!(element.get_attribute(txn, "id"), Some(Out::Any(Any::String(found))) if found.as_ref() == id)
        {
            return Some(element);
        }
        if let Some(found) = find_container(txn, element.children(txn), id) {
            return Some(found);
        }
    }
    None
}

/// The texts in an element, in order, but those of the block groups it holds.
fn texts_in<T: ReadTxn>(txn: &T, element: &XmlElementRef, texts: &mut Vec<XmlTextRef>) {
    for child in element.children(txn) {
        match child {
            XmlOut::Text(text) if text.len(txn) > 0 => texts.push(text),
            XmlOut::Element(inner) if inner.tag().as_ref() != BLOCK_GROUP => {
                texts_in(txn, &inner, texts);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use yrs::{Doc, GetString as _, Transact as _, Update};

    use super::*;
    use crate::ids::SequentialIds;
    use crate::markdown::from_markdown;
    use crate::model::{Block, Document, Inline, Mention, MentionKind, StyledText, Styles};
    use crate::ydoc::{fragment, replace};

    /// A page of `markdown`, its blocks `block-1` and on.
    fn page(client: u64, markdown: &str) -> Doc {
        let doc = Doc::with_client_id(client);
        {
            let fragment = fragment(&doc);
            let mut txn = doc.transact_mut();
            replace(
                &mut txn,
                &fragment,
                &from_markdown(markdown),
                &mut SequentialIds::new("block"),
            );
        }
        doc
    }

    /// A copy of `doc` as another client, as a second editor loads it.
    fn copy(doc: &Doc, client: u64) -> Doc {
        let other = Doc::with_client_id(client);
        sync(doc, &other);
        other
    }

    /// Sends `from` what `to` lacks.
    fn sync(from: &Doc, to: &Doc) {
        let missing = from
            .transact()
            .encode_state_as_update_v1(&to.transact().state_vector());
        to.transact_mut()
            .apply_update(Update::decode_v1(&missing).expect("decodes"))
            .expect("applies");
    }

    /// The first text of the block at `index` of the top level.
    fn text_of(doc: &Doc, index: u32) -> XmlTextRef {
        let fragment = fragment(doc);
        let txn = doc.transact();
        let Some(XmlOut::Element(group)) = fragment.first_child() else {
            panic!("a block group")
        };
        let Some(XmlOut::Element(container)) = group.get(&txn, index) else {
            panic!("a block")
        };
        let Some(XmlOut::Element(content)) = container.first_child() else {
            panic!("its content")
        };
        content
            .children(&txn)
            .find_map(|child| match child {
                XmlOut::Text(text) => Some(text),
                _ => None,
            })
            .expect("a text")
    }

    /// An anchor over `range` of a text, as y-prosemirror makes one of a selection.
    fn anchor(doc: &Doc, text: &XmlTextRef, range: std::ops::Range<u32>) -> (Vec<u8>, Vec<u8>) {
        let txn = doc.transact();
        let start = text
            .sticky_index(&txn, range.start, Assoc::After)
            .expect("a start");
        let end = text
            .sticky_index(&txn, range.end, Assoc::After)
            .expect("an end");
        (start.encode_v1(), end.encode_v1())
    }

    fn resolve_one(doc: &Doc, anchor: &(Vec<u8>, Vec<u8>)) -> Anchored {
        let fragment = fragment(doc);
        let txn = doc.transact();
        resolve(&txn, &fragment, &[(&anchor.0, &anchor.1)]).remove(0)
    }

    /// The text an anchor within one text holds now.
    fn quoted(doc: &Doc, anchor: &(Vec<u8>, Vec<u8>)) -> String {
        let txn = doc.transact();
        let offset = |bytes: &[u8]| {
            StickyIndex::decode_v1(bytes)
                .expect("decodes")
                .get_offset(&txn)
                .expect("resolves")
        };
        let (start, end) = (offset(&anchor.0), offset(&anchor.1));
        let XmlOut::Text(text) = XmlOut::try_from(start.branch).expect("a node") else {
            panic!("in a text")
        };
        let all = text.get_string(&txn);
        all[start.index as usize..end.index as usize].to_owned()
    }

    #[test]
    fn an_anchor_keeps_to_its_text_through_concurrent_edits() {
        let ada = page(1, "The quick brown fox jumps.\n\nSecond paragraph.");
        let bea = copy(&ada, 2);
        let text = text_of(&ada, 0);
        let fox = anchor(&ada, &text, 10..19);
        assert_eq!(quoted(&ada, &fox), "brown fox");

        // At once: Ada types before the range and inside it, Bea deletes a word before it
        // and types after it.
        {
            let mut txn = ada.transact_mut();
            text.insert(&mut txn, 0, "Look: ");
            text.insert(&mut txn, 6 + 16, "red ");
        }
        {
            let theirs = text_of(&bea, 0);
            let mut txn = bea.transact_mut();
            theirs.remove_range(&mut txn, 4, 6);
            theirs.insert(&mut txn, 19, " high");
        }
        sync(&ada, &bea);
        sync(&bea, &ada);

        for doc in [&ada, &bea] {
            assert_eq!(quoted(doc, &fox), "brown red fox");
            assert_eq!(
                resolve_one(doc, &fox),
                Anchored {
                    block_id: Some("block-1".to_owned()),
                    detached: false
                }
            );
        }
        let merged = text_of(&bea, 0);
        assert_eq!(
            merged.get_string(&bea.transact()),
            "Look: The brown red fox jumps high."
        );
    }

    #[test]
    fn an_anchor_whose_text_is_deleted_is_detached() {
        let ada = page(1, "Keep this, drop that.\n\nAnother block.");
        let bea = copy(&ada, 2);
        let text = text_of(&ada, 0);
        let that = anchor(&ada, &text, 16..20);
        let this = anchor(&ada, &text, 5..9);
        {
            let theirs = text_of(&bea, 0);
            let mut txn = bea.transact_mut();
            theirs.remove_range(&mut txn, 11, 9);
        }
        sync(&bea, &ada);
        assert_eq!(text.get_string(&ada.transact()), "Keep this, .");
        assert_eq!(resolve_one(&ada, &that), Anchored::DETACHED);
        assert!(!resolve_one(&ada, &this).detached);

        // A whole block deleted takes its anchors with it.
        let other = anchor(&ada, &text_of(&ada, 1), 0..7);
        {
            let fragment = fragment(&ada);
            let mut txn = ada.transact_mut();
            let Some(XmlOut::Element(group)) = fragment.first_child() else {
                panic!("a block group")
            };
            group.remove_range(&mut txn, 1, 1);
        }
        assert_eq!(resolve_one(&ada, &other), Anchored::DETACHED);
    }

    #[test]
    fn a_range_over_a_mention_is_empty_once_all_of_it_is_deleted() {
        let doc = Doc::with_client_id(1);
        let blocks = vec![Block::new(crate::model::BlockKind::Paragraph {
            props: crate::model::TextProps::default(),
            content: vec![
                Inline::Text(StyledText {
                    text: "Ask ".into(),
                    styles: Styles::default(),
                }),
                Inline::Mention(Mention::new(MentionKind::User, "u1", "Ada")),
                Inline::Text(StyledText {
                    text: " today".into(),
                    styles: Styles::default(),
                }),
            ],
        })];
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
        let (start, end) = {
            let fragment = fragment(&doc);
            let txn = doc.transact();
            let Some(XmlOut::Element(group)) = fragment.first_child() else {
                panic!("a group")
            };
            let Some(XmlOut::Element(container)) = group.first_child() else {
                panic!("a block")
            };
            let Some(XmlOut::Element(paragraph)) = container.first_child() else {
                panic!("a paragraph")
            };
            let texts: Vec<XmlTextRef> = paragraph
                .children(&txn)
                .filter_map(|child| match child {
                    XmlOut::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            (
                texts[0]
                    .sticky_index(&txn, 2, Assoc::After)
                    .expect("a start"),
                texts[1]
                    .sticky_index(&txn, 3, Assoc::After)
                    .expect("an end"),
            )
        };
        let anchor = (start.encode_v1(), end.encode_v1());
        assert!(!resolve_one(&doc, &anchor).detached, "the mention is in it");
        {
            let fragment = fragment(&doc);
            let mut txn = doc.transact_mut();
            let Some(XmlOut::Element(group)) = fragment.first_child() else {
                panic!("a group")
            };
            let Some(XmlOut::Element(container)) = group.first_child() else {
                panic!("a block")
            };
            let Some(XmlOut::Element(paragraph)) = container.first_child() else {
                panic!("a paragraph")
            };
            let Some(XmlOut::Text(first)) = paragraph.get(&txn, 0) else {
                panic!("a text")
            };
            let Some(XmlOut::Text(last)) = paragraph.get(&txn, 2) else {
                panic!("a text")
            };
            first.remove_range(&mut txn, 2, 2);
            last.remove_range(&mut txn, 0, 3);
            paragraph.remove_range(&mut txn, 1, 1);
        }
        assert_eq!(resolve_one(&doc, &anchor), Anchored::DETACHED);
    }

    #[test]
    fn positions_that_do_not_decode_or_resolve_leave_an_anchor_detached() {
        let doc = page(1, "Text.");
        let fragment = fragment(&doc);
        let txn = doc.transact();
        let found = resolve(&txn, &fragment, &[(&[0xff][..], &[0xff][..])]);
        assert_eq!(found, [Anchored::DETACHED]);
        // An id the document has never seen.
        let unknown =
            StickyIndex::from_id(yrs::ID::new(yrs::block::ClientID::new(99), 5), Assoc::After)
                .encode_v1();
        let found = resolve(&txn, &fragment, &[(&unknown[..], &unknown[..])]);
        assert_eq!(found, [Anchored::DETACHED]);
        assert!(resolve(&txn, &fragment, &[]).is_empty());
    }

    #[test]
    fn a_block_anchor_covers_its_text_and_follows_it() {
        let doc = page(
            1,
            "# Title\n\n| a | b |\n| --- | --- |\n| c | d |\n\n- item\n  - child",
        );
        let fragment = fragment(&doc);
        let txn = doc.transact();
        let title = block_anchor(&txn, &fragment, "block-1").expect("an anchor");
        assert_eq!(title.quote, "Title");
        let table = block_anchor(&txn, &fragment, "block-2").expect("an anchor");
        assert_eq!(table.quote, "abcd");
        let item = block_anchor(&txn, &fragment, "block-3").expect("an anchor");
        assert_eq!(item.quote, "item", "its children's text is not its own");
        assert_eq!(block_anchor(&txn, &fragment, "block-9"), None);
        let found = resolve(
            &txn,
            &fragment,
            &[
                (&title.start, &title.end),
                (&table.start, &table.end),
                (&item.start, &item.end),
            ],
        );
        let blocks: Vec<Option<String>> = found
            .into_iter()
            .map(|anchored| anchored.block_id)
            .collect();
        assert_eq!(
            blocks,
            [
                Some("block-1".into()),
                Some("block-2".into()),
                Some("block-3".into())
            ]
        );
        drop(txn);

        // Text typed at either end of the block is in its anchor.
        let text = text_of(&doc, 0);
        {
            let mut txn = doc.transact_mut();
            let len = text.len(&txn);
            text.insert(&mut txn, len, "!");
        }
        assert_eq!(
            quoted(&doc, &(title.start.clone(), title.end.clone())),
            "Title!"
        );
    }

    #[test]
    fn a_block_without_text_has_no_anchor() {
        let doc = page(1, "---\n\nText");
        let fragment = fragment(&doc);
        let txn = doc.transact();
        assert_eq!(block_anchor(&txn, &fragment, "block-1"), None);
        assert!(block_anchor(&txn, &fragment, "block-2").is_some());
    }
}
