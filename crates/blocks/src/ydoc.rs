//! The model in a Yjs document, in the shape BlockNote's editor gives it through
//! y-prosemirror.
//!
//! The editor binds one `XmlFragment`, named [`FRAGMENT`], to its ProseMirror document, and
//! y-prosemirror mirrors each ProseMirror node as an `XmlElement` named after the node's type,
//! with the node's attributes as the element's attributes:
//!
//! ```text
//! XmlFragment "document-store"
//!   blockGroup
//!     blockContainer id="..."
//!       paragraph backgroundColor="default" textColor="default" textAlignment="left"
//!         XmlText "Some bold text" (bold: {} over "bold")
//!         hardBreak
//!         XmlText "after the break, for "
//!         mention kind="user" target="..." label="Ada"
//!       blockGroup                      (the block's children, if it has any)
//!         blockContainer id="..."
//!           ...
//! ```
//!
//! Each run of text nodes is one `XmlText`, whose formatting attributes are the marks: the
//! mark's name maps to its attributes, `{}` for `bold`, `italic`, `underline`, `strike` and
//! `code`, `{"stringValue": colour}` for `textColor` and `backgroundColor`, and
//! `{"href": url}` for `link`. A table is `table > tableRow > (tableHeader | tableCell) >
//! tableParagraph`, the cells carrying their spans, colours, alignment and `colwidth`.
//!
//! y-prosemirror writes every attribute that is not `null`, defaults included; an optional
//! prop with no value, such as an image's `previewWidth` or a database's `viewId`, is written
//! as `undefined`.
//! The writer here does the same. y-prosemirror drops an element or text whose node or mark
//! the editor's schema does not know, so the writer never makes one.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use yrs::types::Delta;
use yrs::types::text::YChange;
use yrs::{
    Any, Doc, Number, OffsetKind, Out, ReadTxn, Text, Transact, TransactionMut, Xml,
    XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim,
    XmlTextRef,
};

use crate::ids::IdSource;
use crate::inline::{Marks, Piece, to_inline, to_pieces};
use crate::model::{
    AudioProps, Block, BlockKind, CheckListItemProps, CodeBlockProps, DEFAULT_COLOR, DatabaseProps,
    Document, FileProps, HeadingProps, ImageProps, Inline, Mention, MentionKind, NoProps,
    NumberedListItemProps, QuoteProps, TableCell, TableCellProps, TableContent, TableProps,
    TableRow, TextAlignment, TextProps,
};

/// The name of the `XmlFragment` that holds a page's blocks, which the web app passes to
/// BlockNote's collaboration options.
pub const FRAGMENT: &str = "document-store";

pub(crate) const BLOCK_GROUP: &str = "blockGroup";
pub(crate) const BLOCK_CONTAINER: &str = "blockContainer";
const HARD_BREAK: &str = "hardBreak";
pub(crate) const MENTION: &str = "mention";
pub(crate) const TABLE: &str = "table";
pub(crate) const TABLE_ROW: &str = "tableRow";
pub(crate) const TABLE_CELL: &str = "tableCell";
pub(crate) const TABLE_HEADER: &str = "tableHeader";
pub(crate) const TABLE_PARAGRAPH: &str = "tableParagraph";

/// The marks of the editor's schema, in its order.
const BOLD: &str = "bold";
const ITALIC: &str = "italic";
const UNDERLINE: &str = "underline";
const STRIKE: &str = "strike";
const CODE: &str = "code";
const TEXT_COLOR: &str = "textColor";
const BACKGROUND_COLOR: &str = "backgroundColor";
const LINK: &str = "link";

/// A document whose structure is not the editor's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("expected {expected}, found {found}")]
    Unexpected {
        expected: &'static str,
        found: String,
    },
    #[error("unknown block type {0:?}")]
    UnknownBlock(String),
    #[error("unknown inline node {0:?}")]
    UnknownInline(String),
    #[error("the {attribute:?} attribute of a {node} is not a valid value")]
    InvalidAttribute {
        node: &'static str,
        attribute: &'static str,
    },
}

/// A fragment that holds something other than a block group, which blocks cannot be added to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the fragment does not hold a block group")]
pub struct NotADocument;

/// The most items the writes of one edit may walk past. yrs finds where a block goes by
/// walking the items of its group that come before that place, once for each block put
/// there, and finds a block to remove the same way, so `n` blocks written after `m` others
/// walk `n × m` items: a fraction of a second at most, on the longest pages a document's
/// size allows. The walk passes the blocks removed from the group too, which a document
/// keeps as long as it lasts and no count of its blocks shows: a [`Deadline`] bounds those.
pub const MAX_WALK: usize = 4_000_000;

/// Writes that would walk past more than [`MAX_WALK`] items, refused before any is made, or
/// that ran past their [`Deadline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "the edit writes or removes too many blocks this far into a long page at once: make it \
     in smaller edits"
)]
pub struct TooFar;

/// When writes into a group that holds blocks already stop, refused with [`TooFar`]. Each
/// block written past a group's first place, and each run of blocks removed there, walks
/// every item before its place, those of the blocks removed before included, so a page whose
/// blocks were removed by the thousand makes writes slow that [`MAX_WALK`] lets through. An
/// edit past its deadline is left part made: give one only to an edit made on a copy of a
/// document, to try it first, and make it on the document itself without one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Deadline(Option<Instant>);

impl Deadline {
    /// No deadline: the writes go on however long they take.
    pub const NONE: Self = Self(None);

    /// The writes stop once `at` has passed.
    pub fn at(at: Instant) -> Self {
        Self(Some(at))
    }

    /// Refuses once the deadline has passed.
    pub(crate) fn check(self) -> Result<(), TooFar> {
        match self.0 {
            Some(at) if Instant::now() >= at => Err(TooFar),
            _ => Ok(()),
        }
    }
}

/// Why blocks were not added to a fragment.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppendRefused {
    #[error(transparent)]
    NotADocument(#[from] NotADocument),
    #[error(transparent)]
    TooFar(#[from] TooFar),
}

/// The items writing `blocks` blocks at index `at` of a group walks past.
pub(crate) fn walk(blocks: usize, at: u32) -> usize {
    blocks.saturating_mul(at as usize)
}

/// Refuses writes that would walk past more than [`MAX_WALK`] items.
pub(crate) fn within(walked: usize) -> Result<(), TooFar> {
    if walked > MAX_WALK {
        Err(TooFar)
    } else {
        Ok(())
    }
}

/// The fragment of `doc` that holds its blocks.
pub fn fragment(doc: &Doc) -> XmlFragmentRef {
    doc.get_or_insert_xml_fragment(FRAGMENT)
}

/// Reads the blocks of `doc`'s [`FRAGMENT`]. It takes a transaction of its own, so `doc` must
/// have none open.
pub fn read_doc(doc: &Doc) -> Result<Document, ReadError> {
    let fragment = fragment(doc);
    let txn = doc.transact();
    read(&txn, &fragment)
}

/// Reads the blocks a fragment holds. An empty fragment, a page nobody has written to yet,
/// is an empty document.
pub fn read<T: ReadTxn>(txn: &T, fragment: &XmlFragmentRef) -> Result<Document, ReadError> {
    match fragment.first_child() {
        None => Ok(Document::default()),
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => {
            Ok(Document::new(read_group(txn, &group, None)?))
        }
        Some(other) => Err(unexpected("a blockGroup", &other)),
    }
}

/// What [`read_lenient`] read, and how much it left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Lenient {
    pub document: Document,
    /// How many blocks did not read, each counted once with the blocks under it.
    pub dropped: usize,
}

/// Reads what of a fragment's blocks reads, as the editor shows a document: a block that
/// does not read, such as one of a type the editor's schema does not know, is left out with
/// the blocks under it, and a fragment that holds no block group reads as empty. Use it to
/// show or export what there is; [`read`] tells what is wrong.
pub fn read_lenient<T: ReadTxn>(txn: &T, fragment: &XmlFragmentRef) -> Lenient {
    let mut dropped = 0;
    let blocks = match fragment.first_child() {
        None => Vec::new(),
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => {
            read_group(txn, &group, Some(&mut dropped)).unwrap_or_default()
        }
        Some(_) => {
            dropped += 1;
            Vec::new()
        }
    };
    Lenient {
        document: Document::new(blocks),
        dropped,
    }
}

/// A mention in a document's text, as its Yjs document holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionItem {
    /// The block it is in.
    pub block_id: Option<String>,
    pub mention: Mention,
    /// The Yjs id of its element: the client that made it and where in that client's
    /// clock, which tells whose change wrote it.
    pub id: Option<yrs::ID>,
}

/// Every mention of a fragment's text, table cells included, in document order, as
/// [`Document::mentions`] lists them, each with the Yjs id of its element. A mention whose
/// attributes do not read fails as [`read`] does.
pub fn mention_items<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
) -> Result<Vec<MentionItem>, ReadError> {
    fn walk<T: ReadTxn>(
        txn: &T,
        children: impl Iterator<Item = XmlOut>,
        block_id: Option<&str>,
        found: &mut Vec<MentionItem>,
    ) -> Result<(), ReadError> {
        for child in children {
            let XmlOut::Element(element) = child else {
                continue;
            };
            match element.tag().as_ref() {
                MENTION => {
                    let branch: &yrs::branch::Branch = element.as_ref();
                    found.push(MentionItem {
                        block_id: block_id.map(str::to_owned),
                        mention: read_mention(txn, &element)?,
                        id: match branch.id() {
                            yrs::BranchID::Nested(id) => Some(id),
                            yrs::BranchID::Root(_) => None,
                        },
                    });
                }
                BLOCK_CONTAINER => {
                    let id = match element.get_attribute(txn, "id") {
                        Some(Out::Any(Any::String(id))) => Some(id.to_string()),
                        _ => None,
                    };
                    walk(txn, element.children(txn), id.as_deref(), found)?;
                }
                _ => walk(txn, element.children(txn), block_id, found)?,
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(txn, fragment.children(txn), None, &mut found)?;
    Ok(found)
}

/// Replaces everything in the fragment with `document`. Blocks without an id get one from
/// `ids`, parents before their children, in document order. The top-level block group is
/// kept when there is one, so the editor's binding to it stays.
pub fn replace(
    txn: &mut TransactionMut<'_>,
    fragment: &XmlFragmentRef,
    document: &Document,
    ids: &mut dyn IdSource,
) {
    let kept = match fragment.first_child() {
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => Some(group),
        _ => None,
    };
    let len = fragment.len(txn);
    match &kept {
        Some(group) => {
            if len > 1 {
                fragment.remove_range(txn, 1, len - 1);
            }
            let children = group.len(txn);
            if children > 0 {
                group.remove_range(txn, 0, children);
            }
        }
        None if len > 0 => fragment.remove_range(txn, 0, len),
        None => {}
    }
    if document.blocks.is_empty() {
        // An empty block group is not a valid document; an empty fragment is a new page.
        if kept.is_some() {
            fragment.remove_range(txn, 0, 1);
        }
        return;
    }
    let group = match kept {
        Some(group) => group,
        None => fragment.push_back(txn, XmlElementPrelim::empty(BLOCK_GROUP)),
    };
    // The group holds no block now: its first place walks nothing.
    write_blocks_first(txn, &group, &document.blocks, ids);
}

/// Adds `blocks` after the fragment's last top-level block. Blocks without an id get one
/// from `ids`, as [`replace`] gives them. Refused, before anything changes, when the
/// fragment holds no block group, or when writing the blocks after those there would walk
/// past more than [`MAX_WALK`] items; and once past `deadline`, which leaves some of the
/// blocks written.
pub fn append(
    txn: &mut TransactionMut<'_>,
    fragment: &XmlFragmentRef,
    blocks: &[Block],
    ids: &mut dyn IdSource,
    deadline: Deadline,
) -> Result<(), AppendRefused> {
    if blocks.is_empty() {
        return Ok(());
    }
    let group = match fragment.first_child() {
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => group,
        None => fragment.push_back(txn, XmlElementPrelim::empty(BLOCK_GROUP)),
        Some(_) => return Err(NotADocument.into()),
    };
    let end = group.len(txn);
    within(walk(blocks.len(), end))?;
    write_blocks_at(txn, &group, end, blocks, ids, deadline)?;
    Ok(())
}

// Reading.

fn unexpected(expected: &'static str, found: &XmlOut) -> ReadError {
    let found = match found {
        XmlOut::Element(element) => format!("a {} element", element.tag()),
        XmlOut::Fragment(_) => "a fragment".to_owned(),
        XmlOut::Text(_) => "text".to_owned(),
    };
    ReadError::Unexpected { expected, found }
}

/// The blocks of a group. Strict, with `dropped` `None`, the first that does not read is
/// the error; lenient, it is left out and counted.
fn read_group<T: ReadTxn>(
    txn: &T,
    group: &XmlElementRef,
    mut dropped: Option<&mut usize>,
) -> Result<Vec<Block>, ReadError> {
    let mut blocks = Vec::new();
    for child in group.children(txn) {
        let block = match child {
            XmlOut::Element(container) if container.tag().as_ref() == BLOCK_CONTAINER => {
                read_container(txn, &container, dropped.as_deref_mut())
            }
            other => Err(unexpected("a blockContainer", &other)),
        };
        match (block, dropped.as_deref_mut()) {
            (Ok(block), _) => blocks.push(block),
            (Err(_), Some(dropped)) => *dropped += 1,
            (Err(error), None) => return Err(error),
        }
    }
    Ok(blocks)
}

pub(crate) fn read_container<T: ReadTxn>(
    txn: &T,
    container: &XmlElementRef,
    dropped: Option<&mut usize>,
) -> Result<Block, ReadError> {
    let id = match container.get_attribute(txn, "id") {
        Some(Out::Any(Any::String(id))) => Some(id.to_string()),
        None | Some(Out::Any(Any::Null | Any::Undefined)) => None,
        Some(_) => {
            return Err(ReadError::InvalidAttribute {
                node: BLOCK_CONTAINER,
                attribute: "id",
            });
        }
    };
    let mut children = container.children(txn);
    let content = match children.next() {
        Some(XmlOut::Element(content)) => content,
        Some(other) => return Err(unexpected("a block's content element", &other)),
        None => {
            return Err(ReadError::Unexpected {
                expected: "a block's content element",
                found: "an empty blockContainer".to_owned(),
            });
        }
    };
    let kind = read_kind(txn, &content)?;
    let nested = match children.next() {
        None => Vec::new(),
        Some(XmlOut::Element(group)) if group.tag().as_ref() == BLOCK_GROUP => {
            read_group(txn, &group, dropped)?
        }
        Some(other) => return Err(unexpected("a blockGroup of children", &other)),
    };
    if let Some(other) = children.next() {
        return Err(unexpected("the end of a blockContainer", &other));
    }
    Ok(Block {
        id,
        kind,
        children: nested,
    })
}

/// The attributes of one element, read with the schema's defaults.
struct Attrs<'a, T> {
    txn: &'a T,
    element: &'a XmlElementRef,
    node: &'static str,
}

impl<T: ReadTxn> Attrs<'_, T> {
    fn get(&self, name: &str) -> Option<Any> {
        match self.element.get_attribute(self.txn, name)? {
            Out::Any(Any::Null | Any::Undefined) => None,
            Out::Any(value) => Some(value),
            _ => Some(Any::Undefined),
        }
    }

    fn invalid(&self, attribute: &'static str) -> ReadError {
        ReadError::InvalidAttribute {
            node: self.node,
            attribute,
        }
    }

    fn string(&self, name: &'static str, default: &str) -> Result<String, ReadError> {
        match self.get(name) {
            None => Ok(default.to_owned()),
            Some(Any::String(value)) => Ok(value.to_string()),
            Some(_) => Err(self.invalid(name)),
        }
    }

    /// A string that has no default: `None` when it is unset or `undefined`.
    fn optional_string(&self, name: &'static str) -> Result<Option<String>, ReadError> {
        match self.get(name) {
            None => Ok(None),
            Some(Any::String(value)) => Ok(Some(value.to_string())),
            Some(_) => Err(self.invalid(name)),
        }
    }

    fn color(&self, name: &'static str) -> Result<String, ReadError> {
        self.string(name, DEFAULT_COLOR)
    }

    fn bool(&self, name: &'static str, default: bool) -> Result<bool, ReadError> {
        match self.get(name) {
            None => Ok(default),
            Some(Any::Bool(value)) => Ok(value),
            Some(_) => Err(self.invalid(name)),
        }
    }

    fn number(&self, name: &'static str) -> Result<Option<f64>, ReadError> {
        match self.get(name) {
            None => Ok(None),
            Some(Any::Number(number)) => match number.as_f64() {
                Some(value) if value.is_finite() => Ok(Some(value)),
                _ => Err(self.invalid(name)),
            },
            Some(_) => Err(self.invalid(name)),
        }
    }

    fn whole(&self, name: &'static str) -> Result<Option<u32>, ReadError> {
        match self.number(name)? {
            None => Ok(None),
            Some(value) if value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value) => {
                // In range and whole, so the cast is exact.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(Some(value as u32))
            }
            Some(_) => Err(self.invalid(name)),
        }
    }

    fn alignment(&self) -> Result<TextAlignment, ReadError> {
        match self.get("textAlignment") {
            None => Ok(TextAlignment::Left),
            Some(Any::String(value)) => {
                TextAlignment::parse(&value).ok_or_else(|| self.invalid("textAlignment"))
            }
            Some(_) => Err(self.invalid("textAlignment")),
        }
    }

    fn text_props(&self) -> Result<TextProps, ReadError> {
        Ok(TextProps {
            background_color: self.color("backgroundColor")?,
            text_color: self.color("textColor")?,
            text_alignment: self.alignment()?,
        })
    }
}

/// A block's type with its props and content, read from its content element: the first
/// child of its `blockContainer`.
pub(crate) fn read_kind<T: ReadTxn>(
    txn: &T,
    element: &XmlElementRef,
) -> Result<BlockKind, ReadError> {
    let name = element.tag().as_ref();
    let node = static_name(name).ok_or_else(|| ReadError::UnknownBlock(name.to_owned()))?;
    let attrs = Attrs { txn, element, node };
    let inline = || read_inline(txn, element);
    Ok(match node {
        "paragraph" => BlockKind::Paragraph {
            props: attrs.text_props()?,
            content: inline()?,
        },
        "heading" => {
            let level = match attrs.whole("level")? {
                None => 1,
                Some(level @ 1..=6) => level as u8,
                Some(_) => return Err(attrs.invalid("level")),
            };
            BlockKind::Heading {
                props: HeadingProps {
                    background_color: attrs.color("backgroundColor")?,
                    text_color: attrs.color("textColor")?,
                    text_alignment: attrs.alignment()?,
                    level,
                    is_toggleable: attrs.bool("isToggleable", false)?,
                },
                content: inline()?,
            }
        }
        "bulletListItem" => BlockKind::BulletListItem {
            props: attrs.text_props()?,
            content: inline()?,
        },
        "numberedListItem" => BlockKind::NumberedListItem {
            props: NumberedListItemProps {
                background_color: attrs.color("backgroundColor")?,
                text_color: attrs.color("textColor")?,
                text_alignment: attrs.alignment()?,
                start: attrs.whole("start")?,
            },
            content: inline()?,
        },
        "checkListItem" => BlockKind::CheckListItem {
            props: CheckListItemProps {
                background_color: attrs.color("backgroundColor")?,
                text_color: attrs.color("textColor")?,
                text_alignment: attrs.alignment()?,
                checked: attrs.bool("checked", false)?,
            },
            content: inline()?,
        },
        "toggleListItem" => BlockKind::ToggleListItem {
            props: attrs.text_props()?,
            content: inline()?,
        },
        "quote" => BlockKind::Quote {
            props: QuoteProps {
                background_color: attrs.color("backgroundColor")?,
                text_color: attrs.color("textColor")?,
            },
            content: inline()?,
        },
        "codeBlock" => BlockKind::CodeBlock {
            props: CodeBlockProps {
                language: attrs.string("language", crate::model::DEFAULT_LANGUAGE)?,
            },
            code: read_plain(txn, element)?,
        },
        "divider" => BlockKind::Divider { props: NoProps {} },
        "table" => BlockKind::Table {
            props: TableProps {
                text_color: attrs.color("textColor")?,
            },
            content: read_table(txn, element)?,
        },
        "image" | "video" => {
            let props = ImageProps {
                text_alignment: attrs.alignment()?,
                background_color: attrs.color("backgroundColor")?,
                name: attrs.string("name", "")?,
                url: attrs.string("url", "")?,
                caption: attrs.string("caption", "")?,
                show_preview: attrs.bool("showPreview", true)?,
                preview_width: attrs.number("previewWidth")?,
            };
            if node == "image" {
                BlockKind::Image { props }
            } else {
                BlockKind::Video { props }
            }
        }
        "audio" => BlockKind::Audio {
            props: AudioProps {
                background_color: attrs.color("backgroundColor")?,
                name: attrs.string("name", "")?,
                url: attrs.string("url", "")?,
                caption: attrs.string("caption", "")?,
                show_preview: attrs.bool("showPreview", true)?,
            },
        },
        "file" => BlockKind::File {
            props: FileProps {
                background_color: attrs.color("backgroundColor")?,
                name: attrs.string("name", "")?,
                url: attrs.string("url", "")?,
                caption: attrs.string("caption", "")?,
            },
        },
        "database" => BlockKind::Database {
            props: DatabaseProps {
                database_id: attrs.string("databaseId", "")?,
                view_id: attrs.optional_string("viewId")?,
            },
        },
        _ => return Err(ReadError::UnknownBlock(name.to_owned())),
    })
}

/// The block type's name, if it is one of the schema's.
fn static_name(name: &str) -> Option<&'static str> {
    const NAMES: [&str; 15] = [
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
        "database",
    ];
    NAMES.into_iter().find(|known| *known == name)
}

/// The inline content of an element: its texts' runs, its hard breaks and its mentions.
fn read_inline<T: ReadTxn>(txn: &T, element: &XmlElementRef) -> Result<Vec<Inline>, ReadError> {
    let mut pieces = Vec::new();
    for child in element.children(txn) {
        match child {
            XmlOut::Text(text) => read_text(txn, &text, &mut pieces)?,
            XmlOut::Element(node) if node.tag().as_ref() == HARD_BREAK => {
                pieces.push(Piece::Break);
            }
            XmlOut::Element(node) if node.tag().as_ref() == MENTION => {
                pieces.push(Piece::Mention(read_mention(txn, &node)?));
            }
            XmlOut::Element(node) => return Err(ReadError::UnknownInline(node.tag().to_string())),
            XmlOut::Fragment(_) => return Err(ReadError::UnknownInline("fragment".to_owned())),
        }
    }
    Ok(to_inline(pieces))
}

/// A mention's props, with the schema's defaults.
fn read_mention<T: ReadTxn>(txn: &T, node: &XmlElementRef) -> Result<Mention, ReadError> {
    let attrs = Attrs {
        txn,
        element: node,
        node: MENTION,
    };
    let kind = MentionKind::parse(&attrs.string("kind", MentionKind::User.as_str())?)
        .ok_or_else(|| attrs.invalid("kind"))?;
    Ok(Mention::new(
        kind,
        attrs.string("target", "")?,
        attrs.string("label", "")?,
    ))
}

fn read_text<T: ReadTxn>(
    txn: &T,
    text: &XmlTextRef,
    pieces: &mut Vec<Piece>,
) -> Result<(), ReadError> {
    for chunk in text.diff(txn, YChange::identity) {
        let Out::Any(Any::String(value)) = chunk.insert else {
            return Err(ReadError::UnknownInline("an embed in text".to_owned()));
        };
        let marks = chunk
            .attributes
            .map(|attributes| marks_of(&attributes))
            .unwrap_or_default();
        pieces.push(Piece::Text {
            text: value.to_string(),
            marks,
        });
    }
    Ok(())
}

/// The marks a text's formatting attributes stand for. Attributes of marks the schema does
/// not have, such as comments, are left out, as BlockNote leaves out marks it ignores.
fn marks_of(attributes: &HashMap<Arc<str>, Any>) -> Marks {
    let mut marks = Marks::default();
    for (key, value) in attributes {
        if matches!(value, Any::Null | Any::Undefined) {
            continue;
        }
        let styles = &mut marks.styles;
        match key.as_ref() {
            BOLD => styles.bold = true,
            ITALIC => styles.italic = true,
            UNDERLINE => styles.underline = true,
            STRIKE => styles.strike = true,
            CODE => styles.code = true,
            TEXT_COLOR => styles.text_color = Some(map_string(value, "stringValue")),
            BACKGROUND_COLOR => styles.background_color = Some(map_string(value, "stringValue")),
            LINK => marks.link = Some(map_string(value, "href")),
            _ => {}
        }
    }
    marks
}

fn map_string(value: &Any, key: &str) -> String {
    match value {
        Any::Map(map) => match map.get(key) {
            Some(Any::String(value)) => value.to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// The text of a code block, whose content is unstyled text.
fn read_plain<T: ReadTxn>(txn: &T, element: &XmlElementRef) -> Result<String, ReadError> {
    let mut code = String::new();
    for child in element.children(txn) {
        match child {
            XmlOut::Text(text) => {
                for chunk in text.diff(txn, YChange::identity) {
                    if let Out::Any(Any::String(value)) = chunk.insert {
                        code.push_str(&value);
                    }
                }
            }
            XmlOut::Element(node) if node.tag().as_ref() == HARD_BREAK => code.push('\n'),
            other => return Err(unexpected("text in a codeBlock", &other)),
        }
    }
    Ok(code)
}

fn read_table<T: ReadTxn>(txn: &T, table: &XmlElementRef) -> Result<TableContent, ReadError> {
    let mut column_widths = Vec::new();
    let mut rows = Vec::new();
    let mut header_map: Vec<Vec<bool>> = Vec::new();
    for (row_index, row) in table.children(txn).enumerate() {
        let row = match row {
            XmlOut::Element(row) if row.tag().as_ref() == TABLE_ROW => row,
            other => return Err(unexpected("a tableRow", &other)),
        };
        let mut cells = Vec::new();
        let mut headers = Vec::new();
        for cell in row.children(txn) {
            let (cell, node) = match cell {
                XmlOut::Element(cell) if cell.tag().as_ref() == TABLE_CELL => (cell, TABLE_CELL),
                XmlOut::Element(cell) if cell.tag().as_ref() == TABLE_HEADER => {
                    (cell, TABLE_HEADER)
                }
                other => return Err(unexpected("a tableCell or tableHeader", &other)),
            };
            headers.push(node == TABLE_HEADER);
            let attrs = Attrs {
                txn,
                element: &cell,
                node,
            };
            let props = TableCellProps {
                colspan: attrs.whole("colspan")?.unwrap_or(1),
                rowspan: attrs.whole("rowspan")?.unwrap_or(1),
                background_color: attrs.color("backgroundColor")?,
                text_color: attrs.color("textColor")?,
                text_alignment: attrs.alignment()?,
            };
            if row_index == 0 {
                match cell.get_attribute(txn, "colwidth") {
                    Some(Out::Any(Any::Array(widths))) => {
                        column_widths.extend(widths.iter().map(|width| match width {
                            Any::Number(number) => number.as_f64(),
                            _ => None,
                        }));
                    }
                    _ => column_widths.extend((0..props.colspan).map(|_| None)),
                }
            }
            cells.push(TableCell {
                content: read_cell(txn, &cell)?,
                props,
            });
        }
        header_map.push(headers);
        rows.push(TableRow { cells });
    }
    // As BlockNote counts them: every row whose cells are all headers, and every column whose
    // cells in every row are.
    let header_rows = count(
        header_map
            .iter()
            .filter(|row| row.iter().all(|h| *h))
            .count(),
    );
    let width = header_map.first().map_or(0, Vec::len);
    let header_cols = count(
        (0..width)
            .filter(|&column| {
                header_map
                    .iter()
                    .all(|row| row.get(column).copied().unwrap_or(false))
            })
            .count(),
    );
    Ok(TableContent {
        column_widths,
        header_rows,
        header_cols,
        rows,
    })
}

fn count(n: usize) -> Option<u32> {
    if n == 0 {
        None
    } else {
        Some(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

/// A cell's content: its paragraphs' runs, joined as BlockNote joins them, with a line break
/// where the runs on both sides have the same styles.
fn read_cell<T: ReadTxn>(txn: &T, cell: &XmlElementRef) -> Result<Vec<Inline>, ReadError> {
    let mut content: Vec<Inline> = Vec::new();
    for paragraph in cell.children(txn) {
        let paragraph = match paragraph {
            XmlOut::Element(paragraph) if paragraph.tag().as_ref() == TABLE_PARAGRAPH => paragraph,
            other => return Err(unexpected("a tableParagraph", &other)),
        };
        let runs = read_inline(txn, &paragraph)?;
        let mut runs = runs.into_iter();
        match (content.last_mut(), runs.next()) {
            (Some(Inline::Text(last)), Some(Inline::Text(first)))
                if last.styles == first.styles =>
            {
                last.text.push('\n');
                last.text.push_str(&first.text);
            }
            (_, Some(first)) => content.push(first),
            (_, None) => {}
        }
        content.extend(runs);
    }
    Ok(content)
}

// Writing.
//
// yrs finds an index of a sequence by walking its items from the first, so siblings written
// one after another, each at the end, would walk n²/2 items. Each sequence is written at one
// index instead, its last item first: into a new element at index 0, which walks nothing,
// and into a group that holds blocks already at the index the new ones go to, which walks
// only the blocks before it. A text's runs go in as one string, and each mark is then applied
// to every range it covers in one pass. Writing takes time in proportion to what is written.

/// Writes `blocks` into a group from `index` on, and returns the ids of the blocks written
/// there, those under them left out. Blocks without an id get one from `ids`, parents
/// before their children, in document order. Past a place other than the first, each block
/// is written only while `deadline` has not passed; once it has, the rest are not, and the
/// write is refused.
pub(crate) fn write_blocks_at(
    txn: &mut TransactionMut<'_>,
    group: &XmlElementRef,
    index: u32,
    blocks: &[Block],
    ids: &mut dyn IdSource,
    deadline: Deadline,
) -> Result<Vec<String>, TooFar> {
    let named = named(blocks, ids);
    for block in named.iter().rev() {
        if index > 0 {
            deadline.check()?;
        }
        write_named(txn, group, index, block);
    }
    Ok(named.into_iter().map(|block| block.id).collect())
}

/// Writes `blocks` at the start of a group, as [`write_blocks_at`] does, which walks no item
/// and so has no deadline.
pub(crate) fn write_blocks_first(
    txn: &mut TransactionMut<'_>,
    group: &XmlElementRef,
    blocks: &[Block],
    ids: &mut dyn IdSource,
) -> Vec<String> {
    let named = named(blocks, ids);
    for block in named.iter().rev() {
        write_named(txn, group, 0, block);
    }
    named.into_iter().map(|block| block.id).collect()
}

/// A block, the id it is written with, and its children so.
struct Named<'a> {
    block: &'a Block,
    id: String,
    children: Vec<Named<'a>>,
}

/// The blocks with their ids: their own, or the next of `ids`, parents before their
/// children, in document order.
fn named<'a>(blocks: &'a [Block], ids: &mut dyn IdSource) -> Vec<Named<'a>> {
    blocks
        .iter()
        .map(|block| {
            let id = block.id.clone().unwrap_or_else(|| ids.next_id());
            Named {
                block,
                id,
                children: named(&block.children, ids),
            }
        })
        .collect()
}

/// Writes a block into a group at `index`, with the blocks under it. Its siblings go in at
/// the same index, the last first, so each goes in where the walk to `index` ends and none
/// walks past those written before it; its children so at the start of their new group.
fn write_named(txn: &mut TransactionMut<'_>, group: &XmlElementRef, index: u32, named: &Named<'_>) {
    let container = group.insert(txn, index, XmlElementPrelim::empty(BLOCK_CONTAINER));
    container.insert_attribute(txn, "id", named.id.as_str());
    let content = container.push_back(txn, XmlElementPrelim::empty(named.block.kind.name()));
    write_kind(txn, &content, &named.block.kind);
    if !named.children.is_empty() {
        let nested = container.push_back(txn, XmlElementPrelim::empty(BLOCK_GROUP));
        for child in named.children.iter().rev() {
            write_named(txn, &nested, 0, child);
        }
    }
}

/// The attributes of one element, written in the schema's order.
struct Writer<'a, 'doc> {
    txn: &'a mut TransactionMut<'doc>,
    element: &'a XmlElementRef,
}

impl Writer<'_, '_> {
    fn set(&mut self, name: &str, value: impl Into<Any>) {
        self.element.insert_attribute(self.txn, name, value.into());
    }

    fn alignment(&mut self, alignment: TextAlignment) {
        self.set("textAlignment", alignment.as_str());
    }

    fn number(&mut self, name: &str, value: Option<f64>) {
        self.set(name, value.map_or(Any::Undefined, number));
    }

    fn optional_string(&mut self, name: &str, value: Option<&str>) {
        self.set(name, value.map_or(Any::Undefined, Any::from));
    }

    fn text(&mut self, props: &TextProps) {
        self.set("backgroundColor", props.background_color.as_str());
        self.set("textColor", props.text_color.as_str());
        self.alignment(props.text_alignment);
    }
}

/// A number as Yjs encodes a JavaScript number: whole numbers in range as integers.
fn number(value: f64) -> Any {
    const LIMIT: f64 = 2_147_483_647.0;
    if value.fract() == 0.0 && (-LIMIT..=LIMIT).contains(&value) {
        // Whole and in range, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        Any::Number(Number::Int(value as i64))
    } else {
        Any::Number(Number::Float(value))
    }
}

/// Writes a block's props and content into its content element, made empty and named after
/// its type.
pub(crate) fn write_kind(txn: &mut TransactionMut<'_>, element: &XmlElementRef, kind: &BlockKind) {
    let mut attrs = Writer {
        txn: &mut *txn,
        element,
    };
    match kind {
        BlockKind::Paragraph { props, content }
        | BlockKind::BulletListItem { props, content }
        | BlockKind::ToggleListItem { props, content } => {
            attrs.text(props);
            write_inline(txn, element, content);
        }
        BlockKind::Heading { props, content } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("textColor", props.text_color.as_str());
            attrs.alignment(props.text_alignment);
            attrs.set("level", i64::from(props.level));
            attrs.set("isToggleable", props.is_toggleable);
            write_inline(txn, element, content);
        }
        BlockKind::NumberedListItem { props, content } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("textColor", props.text_color.as_str());
            attrs.alignment(props.text_alignment);
            attrs.number("start", props.start.map(f64::from));
            write_inline(txn, element, content);
        }
        BlockKind::CheckListItem { props, content } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("textColor", props.text_color.as_str());
            attrs.alignment(props.text_alignment);
            attrs.set("checked", props.checked);
            write_inline(txn, element, content);
        }
        BlockKind::Quote { props, content } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("textColor", props.text_color.as_str());
            write_inline(txn, element, content);
        }
        BlockKind::CodeBlock { props, code } => {
            attrs.set("language", props.language.as_str());
            if !code.is_empty() {
                element.push_back(txn, XmlTextPrelim::new(code.as_str()));
            }
        }
        BlockKind::Divider { .. } => {}
        BlockKind::Table { props, content } => {
            attrs.set("textColor", props.text_color.as_str());
            write_table(txn, element, content);
        }
        BlockKind::Image { props } | BlockKind::Video { props } => {
            attrs.alignment(props.text_alignment);
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("name", props.name.as_str());
            attrs.set("url", props.url.as_str());
            attrs.set("caption", props.caption.as_str());
            attrs.set("showPreview", props.show_preview);
            attrs.number("previewWidth", props.preview_width);
        }
        BlockKind::Audio { props } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("name", props.name.as_str());
            attrs.set("url", props.url.as_str());
            attrs.set("caption", props.caption.as_str());
            attrs.set("showPreview", props.show_preview);
        }
        BlockKind::File { props } => {
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.set("name", props.name.as_str());
            attrs.set("url", props.url.as_str());
            attrs.set("caption", props.caption.as_str());
        }
        BlockKind::Database { props } => {
            attrs.set("databaseId", props.database_id.as_str());
            attrs.optional_string("viewId", props.view_id.as_deref());
        }
    }
}

/// Writes inline content as y-prosemirror does: each run of text nodes one `XmlText`, each
/// hard break and each mention an element between them, a mention's attributes in its
/// props' order. The element is new and empty: its nodes go in at index 0, the last first.
fn write_inline(txn: &mut TransactionMut<'_>, element: &XmlElementRef, content: &[Inline]) {
    let mut nodes: Vec<Node> = Vec::new();
    let mut texts: Vec<(String, Marks)> = Vec::new();
    for piece in to_pieces(content) {
        match piece {
            Piece::Text { text, marks } => texts.push((text, marks)),
            Piece::Break => {
                close_text(&mut nodes, &mut texts);
                nodes.push(Node::Break);
            }
            Piece::Mention(mention) => {
                close_text(&mut nodes, &mut texts);
                nodes.push(Node::Mention(mention));
            }
        }
    }
    close_text(&mut nodes, &mut texts);
    for node in nodes.iter().rev() {
        match node {
            Node::Text(runs) => write_text(txn, element, runs),
            Node::Break => {
                element.insert(txn, 0, XmlElementPrelim::empty(HARD_BREAK));
            }
            Node::Mention(mention) => {
                let node = element.insert(txn, 0, XmlElementPrelim::empty(MENTION));
                let props = &mention.props;
                let mut attrs = Writer {
                    txn: &mut *txn,
                    element: &node,
                };
                attrs.set("kind", props.kind.as_str());
                attrs.set("target", props.target.as_str());
                attrs.set("label", props.label.as_str());
            }
        }
    }
}

/// One node of an element's inline content.
enum Node {
    /// A run of text nodes, written as one `XmlText`.
    Text(Vec<(String, Marks)>),
    Break,
    Mention(Mention),
}

/// Ends the run of text nodes so far, if there is one.
fn close_text(nodes: &mut Vec<Node>, texts: &mut Vec<(String, Marks)>) {
    if !texts.is_empty() {
        nodes.push(Node::Text(std::mem::take(texts)));
    }
}

/// The formatting value a mark has on a run, if the run has the mark.
type MarkValue = fn(&Marks) -> Option<Any>;

/// One `XmlText` of the runs, at index 0 of the element. The text goes in first, a run at a
/// time in one pass, so every range a mark covers starts and ends between two of the text's
/// items and none needs splitting; each mark is then applied over the ranges it covers, one
/// attribute at a time and in the schema's order: an insert with several attributes at once
/// would add them in a hash map's order, which differs from run to run, and so would the
/// update's bytes.
fn write_text(txn: &mut TransactionMut<'_>, element: &XmlElementRef, runs: &[(String, Marks)]) {
    let text = element.insert(txn, 0, XmlTextPrelim::new(""));
    text.apply_delta(
        txn,
        runs.iter()
            .map(|(chunk, _)| Delta::Inserted(Any::from(chunk.as_str()), None)),
    );
    let kind = txn.doc().offset_kind();
    let mut spans = Vec::with_capacity(runs.len());
    let mut end = 0;
    for (chunk, _) in runs {
        let start = end;
        end = start + units(chunk, kind);
        spans.push((start, end));
    }
    let marks: [(&str, MarkValue); 8] = [
        (BOLD, |m| flag(m.styles.bold)),
        (ITALIC, |m| flag(m.styles.italic)),
        (UNDERLINE, |m| flag(m.styles.underline)),
        (STRIKE, |m| flag(m.styles.strike)),
        (CODE, |m| flag(m.styles.code)),
        (TEXT_COLOR, |m| {
            m.styles.text_color.as_deref().map(string_value)
        }),
        (BACKGROUND_COLOR, |m| {
            m.styles.background_color.as_deref().map(string_value)
        }),
        (LINK, |m| m.link.as_deref().map(|href| single("href", href))),
    ];
    for (name, value_of) in marks {
        let mut ranges: Vec<(u32, u32, Any)> = Vec::new();
        let mut open: Option<(u32, u32, Any)> = None;
        for ((_, marks), &(start, end)) in runs.iter().zip(&spans) {
            let value = value_of(marks);
            open = match (open.take(), value) {
                (Some((from, _, current)), Some(value)) if current == value => {
                    Some((from, end, current))
                }
                (previous, value) => {
                    ranges.extend(previous);
                    value.map(|value| (start, end, value))
                }
            };
        }
        ranges.extend(open);
        format(txn, &text, name, ranges);
    }
}

/// Applies one mark over ranges of a text, in order and apart, in one pass over it.
fn format(
    txn: &mut TransactionMut<'_>,
    text: &XmlTextRef,
    name: &str,
    ranges: Vec<(u32, u32, Any)>,
) {
    if ranges.is_empty() {
        return;
    }
    let mut delta: Vec<Delta<Any>> = Vec::with_capacity(ranges.len() * 2);
    let mut at = 0;
    for (from, to, value) in ranges {
        if from > at {
            delta.push(Delta::Retain(from - at, None));
        }
        let attributes = HashMap::from([(Arc::<str>::from(name), value)]);
        delta.push(Delta::Retain(to - from, Some(Box::new(attributes))));
        at = to;
    }
    text.apply_delta(txn, delta);
}

/// The length of `text` in the units the document counts offsets in.
pub(crate) fn units(text: &str, kind: OffsetKind) -> u32 {
    let len = match kind {
        OffsetKind::Bytes => text.len(),
        OffsetKind::Utf16 => text.encode_utf16().count(),
    };
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn flag(on: bool) -> Option<Any> {
    on.then(|| Any::Map(Arc::new(HashMap::new())))
}

fn string_value(color: &str) -> Any {
    single("stringValue", color)
}

fn single(key: &str, value: &str) -> Any {
    Any::Map(Arc::new(HashMap::from([(
        key.to_owned(),
        Any::String(value.into()),
    )])))
}

fn write_table(txn: &mut TransactionMut<'_>, table: &XmlElementRef, content: &TableContent) {
    let header_rows = content.header_rows.unwrap_or(0) as usize;
    let header_cols = content.header_cols.unwrap_or(0) as usize;
    let columns = column_positions(&content.rows);
    // Rows, and each row's cells, at index 0, the last first, as the table is new.
    for (row_index, row) in content.rows.iter().enumerate().rev() {
        let row_element = table.insert(txn, 0, XmlElementPrelim::empty(TABLE_ROW));
        for (cell_index, cell) in row.cells.iter().enumerate().rev() {
            let header = row_index < header_rows || cell_index < header_cols;
            let name = if header { TABLE_HEADER } else { TABLE_CELL };
            let element = row_element.insert(txn, 0, XmlElementPrelim::empty(name));
            let props = &cell.props;
            let column = columns
                .get(row_index)
                .and_then(|row| row.get(cell_index))
                .copied()
                .unwrap_or(cell_index);
            let mut attrs = Writer {
                txn: &mut *txn,
                element: &element,
            };
            attrs.set("textColor", props.text_color.as_str());
            attrs.set("backgroundColor", props.background_color.as_str());
            attrs.alignment(props.text_alignment);
            attrs.set("colspan", i64::from(props.colspan));
            attrs.set("rowspan", i64::from(props.rowspan));
            if let Some(widths) = cell_widths(&content.column_widths, column, props.colspan) {
                attrs.set("colwidth", widths);
            }
            let paragraph = element.push_back(txn, XmlElementPrelim::empty(TABLE_PARAGRAPH));
            write_inline(txn, &paragraph, &cell.content);
        }
    }
}

/// A cell's `colwidth`, as BlockNote sets it: the widths of the columns it spans, or none
/// for a one-column cell whose column has no width.
fn cell_widths(widths: &[Option<f64>], column: usize, colspan: u32) -> Option<Any> {
    let width = |index: usize| widths.get(index).copied().flatten();
    if colspan > 1 {
        let spanned: Vec<Any> = (0..colspan as usize)
            .map(|offset| width(column + offset).map_or(Any::Undefined, number))
            .collect();
        Some(Any::Array(spanned.into()))
    } else {
        match width(column) {
            Some(value) if value != 0.0 => Some(Any::Array(vec![number(value)].into())),
            _ => None,
        }
    }
}

/// The grid column of each cell, rows by cells, with spans taken into account as
/// BlockNote's occupancy grid takes them.
fn column_positions(rows: &[TableRow]) -> Vec<Vec<usize>> {
    let height = rows.len();
    let width = rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|c| c.props.colspan.max(1) as usize)
                .sum()
        })
        .max()
        .unwrap_or(0);
    let mut occupied = vec![vec![false; width]; height];
    let mut positions = Vec::with_capacity(height);
    for (row_index, row) in rows.iter().enumerate() {
        let mut row_positions = Vec::with_capacity(row.cells.len());
        for (cell_index, cell) in row.cells.iter().enumerate() {
            let free = (row_index..height)
                .flat_map(|r| (cell_index..width).map(move |c| (r, c)))
                .find(|&(r, c)| !occupied[r][c]);
            let Some((top, left)) = free else {
                row_positions.push(cell_index);
                continue;
            };
            let rowspan = cell.props.rowspan.max(1) as usize;
            let colspan = cell.props.colspan.max(1) as usize;
            for row in occupied.iter_mut().skip(top).take(rowspan) {
                for cell in row.iter_mut().skip(left).take(colspan) {
                    *cell = true;
                }
            }
            row_positions.push(left);
        }
        positions.push(row_positions);
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SequentialIds;
    use crate::model::{StyledText, Styles};

    fn round_trip(document: &Document) -> Document {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            replace(
                &mut txn,
                &fragment,
                document,
                &mut SequentialIds::new("block"),
            );
        }
        read_doc(&doc).expect("reads back")
    }

    #[test]
    fn an_empty_fragment_is_an_empty_document() {
        let doc = Doc::new();
        assert_eq!(read_doc(&doc), Ok(Document::default()));
    }

    #[test]
    fn mention_items_name_each_mentions_block_and_the_client_that_wrote_it() {
        use yrs::updates::decoder::Decode as _;

        let written = crate::markdown::from_markdown(
            "Ask @[Bea](user:b1)\n\n- and @[Cy](user:c1)\n  - under @[Plan](page:p1)",
        );
        let first = Doc::with_client_id(1);
        let fragment_one = fragment(&first);
        {
            let mut txn = first.transact_mut();
            replace(
                &mut txn,
                &fragment_one,
                &written,
                &mut SequentialIds::new("one"),
            );
        }
        // A second client adds a block of its own.
        let second = Doc::with_client_id(2);
        let fragment_two = fragment(&second);
        let update = {
            let txn = first.transact();
            txn.encode_state_as_update_v1(&yrs::StateVector::default())
        };
        let added = {
            let mut txn = second.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(&update).expect("an update"))
                .expect("applies");
            append(
                &mut txn,
                &fragment_two,
                &crate::markdown::from_markdown("Also @[Dee](user:d1)").blocks,
                &mut SequentialIds::new("two"),
                Deadline::NONE,
            )
            .expect("appended");
            txn.encode_update_v1()
        };
        first
            .transact_mut()
            .apply_update(yrs::Update::decode_v1(&added).expect("an update"))
            .expect("applies");

        let txn = first.transact();
        let items = mention_items(&txn, &fragment_one).expect("read");
        let seen: Vec<(Option<&str>, &str, Option<u64>)> = items
            .iter()
            .map(|item| {
                (
                    item.block_id.as_deref(),
                    item.mention.props.target.as_str(),
                    item.id.map(|id| id.client.get()),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (Some("one-1"), "b1", Some(1)),
                (Some("one-2"), "c1", Some(1)),
                (Some("one-3"), "p1", Some(1)),
                (Some("two-1"), "d1", Some(2)),
            ]
        );
        // The same mentions, in the same order, as the typed document lists them.
        let document = read(&txn, &fragment_one).expect("blocks");
        let typed: Vec<(Option<&str>, &Mention)> = document.mentions();
        let walked: Vec<(Option<&str>, &Mention)> = items
            .iter()
            .map(|item| (item.block_id.as_deref(), &item.mention))
            .collect();
        assert_eq!(walked, typed);
    }

    #[test]
    fn writing_fills_missing_ids_in_document_order() {
        let document = Document::new(vec![
            Block::paragraph("a").with_children(vec![Block::paragraph("b")]),
            Block::paragraph("c").with_id("kept"),
        ]);
        let read = round_trip(&document);
        let ids: Vec<_> = read
            .walk()
            .map(|b| b.id.clone().unwrap_or_default())
            .collect();
        assert_eq!(ids, ["block-1", "block-2", "kept"]);
        let mut filled = document;
        filled.fill_ids(&mut SequentialIds::new("block"));
        assert_eq!(read, filled);
    }

    #[test]
    fn styles_and_breaks_survive() {
        let styles = Styles {
            bold: true,
            italic: true,
            underline: true,
            strike: true,
            text_color: Some("red".into()),
            background_color: Some("blue".into()),
            code: false,
        };
        let document = Document::new(vec![
            Block::new(BlockKind::Paragraph {
                props: TextProps::default(),
                content: vec![
                    Inline::Text(StyledText {
                        text: "all\n".into(),
                        styles,
                    }),
                    Inline::Text(StyledText {
                        text: "plain".into(),
                        styles: Styles::default(),
                    }),
                ],
            })
            .with_id("p"),
        ]);
        assert_eq!(round_trip(&document), document);
    }

    #[test]
    fn replacing_keeps_the_block_group_and_drops_the_rest() {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        let mut ids = SequentialIds::new("block");
        {
            let mut txn = doc.transact_mut();
            replace(
                &mut txn,
                &fragment,
                &Document::new(vec![Block::paragraph("old")]),
                &mut ids,
            );
        }
        let group = {
            let txn = doc.transact();
            fragment.get(&txn, 0).and_then(XmlOut::into_xml_element)
        };
        {
            let mut txn = doc.transact_mut();
            let replacement = Document::new(vec![Block::paragraph("new")]);
            replace(&mut txn, &fragment, &replacement, &mut ids);
            append(
                &mut txn,
                &fragment,
                &[Block::paragraph("more")],
                &mut ids,
                Deadline::NONE,
            )
            .expect("a document");
        }
        let txn = doc.transact();
        assert!(group.is_some());
        assert_eq!(
            fragment.get(&txn, 0).and_then(XmlOut::into_xml_element),
            group
        );
        let read = read(&txn, &fragment).expect("reads");
        let texts: Vec<_> = read
            .blocks
            .iter()
            .map(|b| crate::inline::plain_text(b.kind.inline_content().unwrap_or_default()))
            .collect();
        assert_eq!(texts, ["new", "more"]);
    }

    #[test]
    fn replacing_with_nothing_empties_the_fragment() {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        let mut txn = doc.transact_mut();
        let mut ids = SequentialIds::new("block");
        replace(
            &mut txn,
            &fragment,
            &Document::new(vec![Block::paragraph("x")]),
            &mut ids,
        );
        replace(&mut txn, &fragment, &Document::default(), &mut ids);
        assert_eq!(fragment.len(&txn), 0);
    }

    #[test]
    fn unknown_blocks_are_errors() {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            let group = fragment.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_GROUP));
            let container = group.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_CONTAINER));
            container.push_back(&mut txn, XmlElementPrelim::empty("callout"));
        }
        assert_eq!(
            read_doc(&doc),
            Err(ReadError::UnknownBlock("callout".into()))
        );
    }

    #[test]
    fn a_database_block_holds_its_ids_in_attributes() {
        let database = |view_id: Option<&str>| {
            Block::new(BlockKind::Database {
                props: DatabaseProps {
                    database_id: "0193a5b2-0000-7000-8000-0000000000d1".into(),
                    view_id: view_id.map(str::to_owned),
                },
            })
            .with_id("d")
        };
        for document in [
            Document::new(vec![database(Some("0193a5b2-0000-7000-8000-0000000000e1"))]),
            Document::new(vec![database(None)]),
        ] {
            assert_eq!(round_trip(&document), document);
        }

        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            let group = fragment.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_GROUP));
            let container = group.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_CONTAINER));
            let element = container.push_back(&mut txn, XmlElementPrelim::empty("database"));
            element.insert_attribute(&mut txn, "viewId", Any::Number(Number::Int(7)));
        }
        assert_eq!(
            read_doc(&doc),
            Err(ReadError::InvalidAttribute {
                node: "database",
                attribute: "viewId",
            })
        );
    }

    #[test]
    fn a_lenient_read_leaves_out_what_does_not_read() {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            let document = Document::new(vec![
                Block::paragraph("first")
                    .with_id("a")
                    .with_children(vec![Block::paragraph("child").with_id("b")]),
                Block::paragraph("last").with_id("c"),
            ]);
            replace(
                &mut txn,
                &fragment,
                &document,
                &mut SequentialIds::new("block"),
            );
            let group = fragment
                .get(&txn, 0)
                .and_then(XmlOut::into_xml_element)
                .expect("a group");
            // An unknown block between the two, and one under the first.
            let unknown = group.insert(&mut txn, 1, XmlElementPrelim::empty(BLOCK_CONTAINER));
            let callout = unknown.push_back(&mut txn, XmlElementPrelim::empty("callout"));
            let nested = callout.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_GROUP));
            nested.push_back(&mut txn, XmlElementPrelim::empty(BLOCK_CONTAINER));
            let first = group
                .get(&txn, 0)
                .and_then(XmlOut::into_xml_element)
                .expect("a block");
            let children = first
                .get(&txn, 1)
                .and_then(XmlOut::into_xml_element)
                .expect("its children");
            children.push_back(&mut txn, XmlTextPrelim::new("stray"));
        }
        let txn = doc.transact();
        assert!(read(&txn, &fragment).is_err());
        let lenient = read_lenient(&txn, &fragment);
        assert_eq!(lenient.dropped, 2);
        assert_eq!(
            lenient.document,
            Document::new(vec![
                Block::paragraph("first")
                    .with_id("a")
                    .with_children(vec![Block::paragraph("child").with_id("b")]),
                Block::paragraph("last").with_id("c"),
            ])
        );
    }

    #[test]
    fn a_lenient_read_of_a_fragment_without_blocks_is_empty() {
        let doc = Doc::with_client_id(1);
        let fragment = fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        }
        let txn = doc.transact();
        assert_eq!(
            read_lenient(&txn, &fragment),
            Lenient {
                document: Document::default(),
                dropped: 1,
            }
        );
        let empty = Doc::new();
        let fragment = super::fragment(&empty);
        assert_eq!(
            read_lenient(&empty.transact(), &fragment),
            Lenient {
                document: Document::default(),
                dropped: 0,
            }
        );
    }

    #[test]
    fn nodes_count_blocks_rows_cells_and_inline_nodes() {
        let document = crate::markdown::from_markdown(
            "One **bold** and plain\\\nline\n\n| a | b |\n| - | - |\n| c | d |\n\n- top\n  - under\n",
        );
        // The paragraph: itself, three runs, a break and the run after it. The table:
        // itself, two rows of two cells, each with its run. The list: two items, each with its run.
        assert_eq!(document.nodes(), 6 + 11 + 4);
        assert_eq!(crate::nodes(&[]), 0);
    }

    #[test]
    fn a_long_document_writes_in_time_in_proportion_to_its_size() {
        // Each paragraph went in at the end of the group, and every one walked past those
        // before it: 50000 took minutes in a debug build. Each now goes in at the start.
        let document = Document::new(
            (0..50_000)
                .map(|i| Block::paragraph(format!("paragraph {i}")))
                .collect(),
        );
        let doc = Doc::new();
        let fragment = fragment(&doc);
        let started = std::time::Instant::now();
        replace(
            &mut doc.transact_mut(),
            &fragment,
            &document,
            &mut SequentialIds::new("b"),
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(read_doc(&doc).expect("reads").blocks.len(), 50_000);
    }

    #[test]
    fn column_positions_follow_spans() {
        let cell = |colspan, rowspan| TableCell {
            content: Vec::new(),
            props: TableCellProps {
                colspan,
                rowspan,
                ..TableCellProps::default()
            },
        };
        let rows = vec![
            TableRow {
                cells: vec![cell(2, 1), cell(1, 2)],
            },
            TableRow {
                cells: vec![cell(1, 1), cell(1, 1)],
            },
        ];
        assert_eq!(column_positions(&rows), vec![vec![0, 2], vec![0, 1]]);
    }
}
