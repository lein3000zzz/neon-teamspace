//! A page's content: the block document BlockNote edits, read from and written to the page's
//! Yjs document, and converted to and from markdown.
//!
//! Each page is one Yjs document. The browser edits it with BlockNote, which keeps it in the
//! XML structure y-prosemirror makes of its ProseMirror document; the server holds it with
//! yrs. This crate reads that structure into a typed [`Document`], whose JSON form is
//! BlockNote's own block format, and writes a `Document` back into it in exactly the shape
//! the editor writes, so the server can export, import and index pages, and apply edits to a
//! live document. [`markdown`] converts a `Document` to and from GitHub-flavoured markdown,
//! with a small extension for blocks markdown has no syntax for.
//!
//! Like `neon-domain`, it does no I/O and has no async code.

pub mod edit;
mod files;
mod ids;
mod inline;
pub mod markdown;
mod model;
mod text;
mod ydoc;

pub use ids::{IdSource, SequentialIds};
pub use model::{
    AudioProps, Block, BlockKind, CheckListItemProps, CodeBlockProps, DEFAULT_COLOR,
    DEFAULT_LANGUAGE, DatabaseProps, Document, FileProps, HeadingProps, ImageProps, Inline, Link,
    NoProps, NumberedListItemProps, QuoteProps, StyledText, Styles, TableCell, TableCellProps,
    TableContent, TableProps, TableRow, TextAlignment, TextProps,
};
pub use ydoc::{
    FRAGMENT, Lenient, NotADocument, ReadError, append, fragment, read, read_doc, read_lenient,
    replace,
};
