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
//! | database | `:::database <databaseId>`, or `:::database <databaseId> view=<viewId>` |
//! | bold, italic, strikethrough | `**bold**`, `*italic*`, `~~strike~~`; `<strong>`, `<em>` and `<del>` where the delimiters would not parse |
//! | underline | `<u>underline</u>` |
//! | inline code | `` `code` `` |
//! | link | `[text](url)` |
//! | mention | `@[Name](user:<id>)` for a person, agent or integration, `@[Title](page:<id>)` for a page |
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
//!
//! :::database 0193a5b2-0000-7000-8000-0000000000d1 view=0193a5b2-0000-7000-8000-0000000000e1
//! ```
//!
//! `:::toggle` takes the rest of its line as the title, read as markdown: a heading there
//! makes a toggleable heading of that level, anything else a toggle list item. Its children
//! follow, until a line of colons alone closes the toggle; the end of the list item, quote
//! or document it is in closes it too. `:::video`, `:::audio` and `:::file` take a link on
//! the rest of their line: its text is the file's name, its target the file's URL and its
//! title the caption. `:::database` takes the id of the database the block shows, then
//! `view=` and the id of the view it shows it in, each left out when the block has none; an
//! id is percent-encoded where it holds anything but ASCII letters, digits, `-`, `_`, `.` and
//! `~`, so a UUID is written as it is, and any other word on the line makes it text. A line
//! of colons that closes nothing is text, as is a line that starts with an escaped colon,
//! `\:::`.
//!
//! # Mentions
//!
//! A mention is an `@` right before a link whose target is `user:` or `page:` and the id of
//! what it names, its text the name or title it had: `@[Ada](user:0193a5b2-...)`. The
//! writer escapes an `@` before any other link to such a target, `\@[text](user:...)`, so
//! that it reads back as the link it is.
//!
//! [`inline_from_markdown`] reads markdown as a comment holds it: inline content alone,
//! each line of the text a line of it, nothing on a line starting a block.
//!
//! # Block ids
//!
//! [`to_markdown_with_ids`] writes the same markdown with a line naming each block before the
//! block's own lines, at its level: indented under a list item, quoted in a quote.
//!
//! ```markdown
//! <!-- block 0193a5b2-0000-7000-8000-0000000000b1 -->
//! # Plan
//!
//! <!-- block 0193a5b2-0000-7000-8000-0000000000b2 -->
//! - First
//!   <!-- block 0193a5b2-0000-7000-8000-0000000000b3 -->
//!   - Under it
//! <!-- block 0193a5b2-0000-7000-8000-0000000000b4 -->
//! - Second
//! ```
//!
//! The line is `<!-- block `, the id percent-encoded as a database's is, and ` -->`. Every
//! block with an id has one, empty paragraphs included, which have no other line; blocks whose
//! children markdown cannot nest are followed by their children's lines, each child named.
//! Such a line is an HTML comment, which renderers leave unshown. [`from_markdown`] leaves out
//! every line that names a block, wherever it stands but in code, so markdown written with ids
//! reads as the same blocks as without them: a client may send back what it read, edited, and
//! the lines it kept change nothing. A line that names a block with no lines of its own, one
//! followed by a blank line, another such line, the end of the list item, quote or toggle it
//! is in, or the end of the text, reads as the empty paragraph it names, so those come back
//! too.
//!
//! # What markdown loses
//!
//! - Block ids. Read blocks have none until they are written into a document.
//! - Text and background colours, of blocks and of text, and text alignment, except a table
//!   column's.
//! - Empty paragraphs, unless written with their ids.
//! - The nesting of children under blocks other than list items and toggles.
//! - Tables' column widths, header columns, spans and cell colours, and a table without a
//!   header row, which comes back with its first row as the header.
//! - Images' and videos' preview widths and whether files show a preview.
//! - A numbered list item that restarts at 1 right after another numbered list.
//! - Whitespace at the edges of bold, italic and strikethrough text, which is written
//!   outside the style.
//! - Nesting deeper than [`MAX_DEPTH`] levels of blocks. Reading, a quote or list item that
//!   would be deeper is left out, its content read at its parent's level; a toggle, quote or
//!   list item at the last level holds no children, which follow it there instead; and a line
//!   that starts with more than `MAX_DEPTH + 1` quote or list markers reads the rest as text.
//!   [`try_from_markdown`] refuses such markdown.
//!
//! Reading markdown, a soft line break is a space, as CommonMark has it; raw HTML other than
//! the tags above is kept as text; and an image inside a paragraph splits it, as images are
//! blocks.

mod parse;
mod write;

pub use parse::{MAX_DEPTH, TooDeep, from_markdown, inline_from_markdown, try_from_markdown};
pub(crate) use write::{encode_id, holds_children};
pub use write::{to_markdown, to_markdown_with_ids};

#[cfg(test)]
mod tests;
