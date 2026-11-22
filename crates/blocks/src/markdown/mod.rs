//! A [`Document`](crate::Document) as GitHub-flavoured markdown, and back.
//!
//! [`to_markdown`] writes the blocks markdown has a syntax for in that syntax, and the rest in
//! a small extension of it; [`from_markdown`] reads both. Reading what was written gives the
//! same blocks back, but for what markdown cannot hold, listed below, so writing a parsed
//! document again gives the same markdown: the conversion is stable after one pass.
//!
//! | Block or style | Markdown |
//! |---|---|
//! | paragraph | its text; a line break is `\` at the end of a line |
//! | heading, levels 1 to 6 | `#` to `######` |
//! | bullet list item | `- item` |
//! | numbered list item | `1. item`, counting up from its list's `start` |
//! | check list item | `- [ ] item`, `- [x] item` |
//! | toggle list item | `:::toggle Title`, its children, then `:::` |
//! | toggleable heading | `:::toggle ## Title`, its children, then `:::` |
//! | quote | `> text` |
//! | code block | a fenced block with the language as its info string |
//! | divider | `---` |
//! | table | a pipe table; its first row is the header, and a column whose cells share a centre or right alignment gets it |
//! | image | `![name](url "caption")` on a line of its own |
//! | video, audio, file | `:::video [name](url "caption")`, `:::audio ...`, `:::file ...` |
//! | bold, italic, strikethrough | `**bold**`, `*italic*`, `~~strike~~`; `<strong>`, `<em>` and `<del>` where the delimiters would not parse |
//! | underline | `<u>underline</u>` |
//! | inline code | `` `code` `` |
//! | link | `[text](url)` |
//!
//! Children nest under list items, by indentation, and under toggles. The children of other
//! blocks are written after their parent, at its level.
//!
//! # The extension
//!
//! A line of three colons and a name opens a block markdown has no syntax for:
//!
//! ```markdown
//! :::toggle A toggle's title, with **styles**
//!
//! Its children: any markdown, other toggles included.
//!
//! :::
//!
//! :::toggle ## A toggleable heading
//! :::
//!
//! :::video [demo.mp4](https://example.com/demo.mp4 "A caption")
//! ```
//!
//! `:::toggle` takes the rest of its line as the title, read as markdown: a heading there
//! makes a toggleable heading of that level, anything else a toggle list item. Its children
//! follow, until a line of colons alone closes the toggle; the end of the list item, quote
//! or document it is in closes it too. `:::video`, `:::audio` and `:::file` take a link on
//! the rest of their line: its text is the file's name, its target the file's URL and its
//! title the caption. A line of colons that closes nothing is text, as is a line that
//! starts with an escaped colon, `\:::`.
//!
//! # What markdown loses
//!
//! - Block ids. Read blocks have none until they are written into a document.
//! - Text and background colours, of blocks and of text, and text alignment, except a table
//!   column's.
//! - Empty paragraphs.
//! - The nesting of children under blocks other than list items and toggles.
//! - Tables' column widths, header columns, spans and cell colours, and a table without a
//!   header row, which comes back with its first row as the header.
//! - Images' and videos' preview widths and whether files show a preview.
//! - A numbered list item that restarts at 1 right after another numbered list.
//! - Whitespace at the edges of bold, italic and strikethrough text, which is written
//!   outside the style.
//!
//! Reading markdown, a soft line break is a space, as CommonMark has it; raw HTML other than
//! the tags above is kept as text; and an image inside a paragraph splits it, as images are
//! blocks.

mod parse;
mod write;

pub use parse::from_markdown;
pub use write::to_markdown;

#[cfg(test)]
mod tests;
