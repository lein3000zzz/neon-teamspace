//! Markdown to document. pulldown-cmark parses the markdown into events, which become a tree
//! of markdown's own blocks first; the extension's lines are then found among the
//! paragraphs, and the tree becomes the document's blocks.

use std::borrow::Cow;
use std::iter::Peekable;
use std::ops::Range;

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, LinkType, OffsetIter, Options, Parser, Tag, TagEnd,
};

use super::write::{ID_CLOSE, ID_OPEN};
use crate::inline::{Marks, Piece, to_inline, to_pieces};
use crate::model::{
    AudioProps, Block, BlockKind, CheckListItemProps, CodeBlockProps, DEFAULT_LANGUAGE,
    DatabaseProps, Document, FileProps, HeadingProps, ImageProps, Inline, NoProps,
    NumberedListItemProps, QuoteProps, TableCell, TableCellProps, TableContent, TableProps,
    TableRow, TextAlignment, TextProps,
};

/// The blocks of a markdown text. Any text is markdown, so this cannot fail; the blocks have
/// no ids. The lines naming blocks that [`to_markdown_with_ids`](super::to_markdown_with_ids)
/// writes are left out, so its markdown reads as [`to_markdown`](super::to_markdown)'s does,
/// but for a line naming a block that has no lines of its own: an empty paragraph, which it
/// stays.
pub fn from_markdown(markdown: &str) -> Document {
    let markdown = without_ids(markdown);
    Document::new(convert(parse(&markdown)))
}

/// The markdown without the lines that name a block: each an HTML block of its own, so none
/// in code is taken, and gone with its line, so blocks around it read as if it never was.
/// One that names an empty paragraph stays, to be read as one.
fn without_ids(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains(ID_OPEN) {
        return Cow::Borrowed(markdown);
    }
    let events: Vec<(Event<'_>, Range<usize>)> = Parser::new_ext(markdown, options())
        .into_offset_iter()
        .collect();
    let mut cuts: Vec<Range<usize>> = Vec::new();
    for (index, (event, range)) in events.iter().enumerate() {
        let Event::Html(html) = event else {
            continue;
        };
        if !names_block(html) {
            continue;
        }
        let start = markdown[..range.start].rfind('\n').map_or(0, |at| at + 1);
        // What comes before it on its line: indentation, or the markers of a quote.
        if !markdown[start..range.start]
            .chars()
            .all(|c| c == ' ' || c == '\t' || c == '>')
        {
            continue;
        }
        let end = if markdown[..range.end].ends_with('\n') {
            range.end
        } else {
            markdown[range.end..]
                .find('\n')
                .map_or(markdown.len(), |at| range.end + at + 1)
        };
        if names_empty_paragraph(markdown, end, &events[index + 1..]) {
            continue;
        }
        cuts.push(start..end);
    }
    if cuts.is_empty() {
        return Cow::Borrowed(markdown);
    }
    let mut kept = String::with_capacity(markdown.len());
    let mut from = 0;
    for cut in cuts {
        if cut.start >= from {
            kept.push_str(&markdown[from..cut.start]);
            from = cut.end;
        }
    }
    kept.push_str(&markdown[from..]);
    Cow::Owned(kept)
}

/// Whether a line naming a block, ending at `end`, names one with no lines of its own: an
/// empty paragraph. The lines of any other block follow right after it, so it is one when
/// what follows is a blank line, another line naming a block, the end of what holds it, or
/// nothing. `after` are the events after the line's.
fn names_empty_paragraph(markdown: &str, end: usize, after: &[(Event<'_>, Range<usize>)]) -> bool {
    let mut after = after
        .iter()
        .filter(|(event, _)| !matches!(event, Event::End(TagEnd::HtmlBlock)));
    let Some((next, range)) = after.next() else {
        return true;
    };
    let blank_between = markdown[end..range.start.max(end)]
        .split_inclusive('\n')
        .any(|line| line.ends_with('\n') && line.chars().all(|c| c.is_whitespace() || c == '>'));
    match next {
        Event::End(_) => true,
        Event::Start(Tag::HtmlBlock) => {
            blank_between
                || matches!(after.next(), Some((Event::Html(html), _)) if names_block(html))
        }
        _ => blank_between,
    }
}

/// Whether an HTML block's line is one naming a block: `<!-- block <id> -->`, the id one word.
fn names_block(html: &str) -> bool {
    html.trim()
        .strip_prefix(ID_OPEN)
        .and_then(|rest| rest.strip_suffix(ID_CLOSE))
        .map(str::trim_end)
        .is_some_and(|id| {
            !id.is_empty()
                && id.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'%')
                })
        })
}

/// The parser's options: CommonMark with GitHub's tables, strikethrough and task lists.
pub(super) fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

fn parse(markdown: &str) -> Vec<Md<'_>> {
    let mut events = Parser::new_ext(markdown, options())
        .into_offset_iter()
        .peekable();
    parse_blocks(&mut events, markdown)
}

type Events<'a> = Peekable<OffsetIter<'a, pulldown_cmark::DefaultBrokenLinkCallback>>;

/// A markdown block, before it becomes the document's.
#[derive(Debug)]
enum Md<'a> {
    Paragraph(Vec<Line<'a>>),
    Heading(u8, Vec<Line<'a>>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Md<'a>>>,
    },
    Quote(Vec<Md<'a>>),
    Code {
        language: String,
        code: String,
    },
    Rule,
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<Vec<Vec<Line<'a>>>>,
    },
    Html(String),
}

/// One line of a paragraph: its inline content, and its source, in which the extension's
/// lines are found.
#[derive(Debug)]
struct Line<'a> {
    parts: Vec<Part>,
    source: &'a str,
    /// How it is joined to the line before it: a soft break, with the marks around it, or a
    /// hard break.
    joint: Joint,
    /// The task list marker the line starts with, in a list item's first line.
    task: Option<bool>,
}

#[derive(Debug, Clone)]
enum Joint {
    First,
    Soft(Marks),
    Hard,
}

#[derive(Debug, Clone)]
enum Part {
    Piece(Piece),
    Image {
        url: String,
        alt: String,
        title: String,
    },
}

fn parse_blocks<'a>(events: &mut Events<'a>, source: &'a str) -> Vec<Md<'a>> {
    let mut blocks = Vec::new();
    while let Some((event, _)) = events.peek() {
        if is_inline(event) {
            blocks.push(Md::Paragraph(parse_lines(events, source)));
            continue;
        }
        let Some((event, _)) = events.next() else {
            break;
        };
        match event {
            Event::End(_) => break,
            Event::Start(Tag::Paragraph) => {
                let lines = parse_lines(events, source);
                events.next();
                blocks.push(Md::Paragraph(lines));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let lines = parse_lines(events, source);
                events.next();
                blocks.push(Md::Heading(level as u8, lines));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                blocks.push(Md::Quote(parse_blocks(events, source)))
            }
            Event::Start(Tag::List(start)) => {
                let mut items = Vec::new();
                while let Some((event, _)) = events.next() {
                    match event {
                        Event::Start(Tag::Item) => items.push(parse_blocks(events, source)),
                        _ => break,
                    }
                }
                blocks.push(Md::List { start, items });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .unwrap_or(DEFAULT_LANGUAGE)
                        .to_owned(),
                    CodeBlockKind::Indented => DEFAULT_LANGUAGE.to_owned(),
                };
                let mut code = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) => code.push_str(&text),
                        _ => break,
                    }
                }
                if code.ends_with('\n') {
                    code.pop();
                }
                blocks.push(Md::Code { language, code });
            }
            Event::Start(Tag::HtmlBlock) => {
                let mut html = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Html(text) | Event::Text(text) => html.push_str(&text),
                        _ => break,
                    }
                }
                blocks.push(Md::Html(html));
            }
            Event::Start(Tag::Table(alignments)) => {
                blocks.push(parse_table(events, source, alignments));
            }
            Event::Rule => blocks.push(Md::Rule),
            Event::Html(text) => blocks.push(Md::Html(text.to_string())),
            // A block of a kind the options leave off: its content, as blocks.
            Event::Start(_) => blocks.extend(parse_blocks(events, source)),
            _ => {}
        }
    }
    blocks
}

fn parse_table<'a>(events: &mut Events<'a>, source: &'a str, alignments: Vec<Alignment>) -> Md<'a> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    while let Some((event, _)) = events.next() {
        match event {
            Event::Start(Tag::TableHead | Tag::TableRow) => row = Vec::new(),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
            Event::Start(Tag::TableCell) => {
                let lines = parse_lines(events, source);
                events.next();
                row.push(lines);
            }
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    Md::Table { alignments, rows }
}

/// Whether an event belongs to inline content, as in a tight list item's text.
fn is_inline(event: &Event<'_>) -> bool {
    match event {
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineHtml(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::TaskListMarker(_)
        | Event::FootnoteReference(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_) => true,
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        _ => false,
    }
}

/// The marks open at a point of inline content: how many of each style's tags or delimiters
/// are open around it, and the link it is in.
#[derive(Debug, Default)]
struct Open {
    bold: u32,
    italic: u32,
    underline: u32,
    strike: u32,
    links: Vec<String>,
}

impl Open {
    fn marks(&self) -> Marks {
        let mut marks = Marks::default();
        marks.styles.bold = self.bold > 0;
        marks.styles.italic = self.italic > 0;
        marks.styles.underline = self.underline > 0;
        marks.styles.strike = self.strike > 0;
        marks.link = self.links.last().cloned();
        marks
    }

    /// Opens or closes a style for an inline HTML tag of one, and tells whether the tag was
    /// one: the writer's `<u>`, `<strong>`, `<em>` and `<del>`, and their synonyms.
    fn html(&mut self, tag: &str) -> bool {
        let tag = tag.trim().to_ascii_lowercase();
        let (closing, name) = match tag.strip_prefix("</") {
            Some(rest) => (true, rest),
            None => (false, tag.strip_prefix('<').unwrap_or_default()),
        };
        let Some(name) = name.strip_suffix('>') else {
            return false;
        };
        let counter = match name.trim() {
            "strong" | "b" => &mut self.bold,
            "em" | "i" => &mut self.italic,
            "u" | "ins" => &mut self.underline,
            "del" | "s" | "strike" => &mut self.strike,
            _ => return false,
        };
        if closing {
            *counter = counter.saturating_sub(1);
        } else {
            *counter += 1;
        }
        true
    }
}

fn is_break_tag(html: &str) -> bool {
    let tag: String = html
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    matches!(tag.as_str(), "<br>" | "<br/>")
}

/// The lines of inline content, up to the end of the block it is in, which it leaves to the
/// caller.
fn parse_lines<'a>(events: &mut Events<'a>, source: &'a str) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let mut parts = Vec::new();
    let mut span: Option<Range<usize>> = None;
    let mut joint = Joint::First;
    let mut task = None;
    let mut open = Open::default();
    let finish = |parts: &mut Vec<Part>,
                  span: &mut Option<Range<usize>>,
                  joint: Joint,
                  task: Option<bool>,
                  lines: &mut Vec<Line<'a>>| {
        let source = span.take().map_or("", |span| &source[span]);
        lines.push(Line {
            parts: std::mem::take(parts),
            source,
            joint,
            task,
        });
    };
    while let Some((event, _)) = events.peek() {
        if !is_inline(event) && !matches!(event, Event::End(tag) if is_inline_end(*tag)) {
            break;
        }
        let Some((event, range)) = events.next() else {
            break;
        };
        let marks = open.marks();
        match event {
            Event::SoftBreak | Event::HardBreak => {
                let next = if matches!(event, Event::HardBreak) {
                    Joint::Hard
                } else {
                    Joint::Soft(marks)
                };
                finish(&mut parts, &mut span, joint, task.take(), &mut lines);
                joint = next;
                continue;
            }
            Event::TaskListMarker(checked) => {
                if lines.is_empty() && parts.is_empty() {
                    task = Some(checked);
                }
                continue;
            }
            _ => {}
        }
        span = Some(match span.take() {
            Some(span) => span.start..range.end.max(span.end),
            // The text of an escaped character starts after its backslash, which belongs
            // to the line's source too: `\:::` is text, not the extension.
            None if source[..range.start].ends_with('\\') => range.start - 1..range.end,
            None => range.clone(),
        });
        match event {
            Event::Text(text) => parts.push(text_part(&text, marks)),
            Event::Code(code) | Event::InlineMath(code) | Event::DisplayMath(code) => {
                let mut marks = Marks::default();
                marks.styles.code = true;
                parts.push(text_part(&code, marks));
            }
            Event::InlineHtml(html) => {
                if is_break_tag(&html) {
                    parts.push(Part::Piece(Piece::Break));
                } else if !open.html(&html) {
                    parts.push(text_part(&html, marks));
                }
            }
            Event::FootnoteReference(label) => parts.push(text_part(&format!("[^{label}]"), marks)),
            Event::Start(Tag::Strong) => open.bold += 1,
            Event::End(TagEnd::Strong) => open.bold = open.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => open.italic += 1,
            Event::End(TagEnd::Emphasis) => open.italic = open.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => open.strike += 1,
            Event::End(TagEnd::Strikethrough) => open.strike = open.strike.saturating_sub(1),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let href = if link_type == LinkType::Email && !dest_url.starts_with("mailto:") {
                    format!("mailto:{dest_url}")
                } else {
                    dest_url.to_string()
                };
                open.links.push(href);
            }
            Event::End(TagEnd::Link) => {
                open.links.pop();
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let mut alt = String::new();
                let mut depth = 0;
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        Event::Start(_) => depth += 1,
                        Event::End(TagEnd::Image) if depth == 0 => break,
                        Event::End(_) => depth -= 1,
                        _ => {}
                    }
                }
                parts.push(Part::Image {
                    url: dest_url.to_string(),
                    alt,
                    title: title.to_string(),
                });
            }
            _ => {}
        }
    }
    if !parts.is_empty() || !lines.is_empty() || task.is_some() {
        finish(&mut parts, &mut span, joint, task, &mut lines);
    }
    lines
}

fn is_inline_end(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

fn text_part(text: &str, marks: Marks) -> Part {
    Part::Piece(Piece::Text {
        text: text.to_owned(),
        marks,
    })
}

// From markdown's blocks to the document's.

/// What a line of a paragraph is: text, or one of the extension's lines.
enum Segment<'a> {
    Text(Vec<Line<'a>>),
    Open(&'a str),
    Close(&'a str),
    /// A line that is a whole block: a file's or a database's.
    Block(Block),
}

/// The blocks being built at one level, with the toggles opened in it and not closed yet.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    toggles: Vec<(Block, Vec<Block>)>,
}

impl Builder {
    fn push(&mut self, block: Block) {
        match self.toggles.last_mut() {
            Some((_, children)) => children.push(block),
            None => self.blocks.push(block),
        }
    }

    fn close(&mut self) -> bool {
        match self.toggles.pop() {
            Some((toggle, children)) => {
                self.push(toggle.with_children(children));
                true
            }
            None => false,
        }
    }

    fn finish(mut self) -> Vec<Block> {
        while self.close() {}
        self.blocks
    }
}

fn convert(blocks: Vec<Md<'_>>) -> Vec<Block> {
    let mut builder = Builder::default();
    for block in blocks {
        match block {
            Md::Paragraph(lines) => {
                for segment in segments(lines) {
                    match segment {
                        Segment::Text(lines) => {
                            for block in paragraph(join(lines)) {
                                builder.push(block);
                            }
                        }
                        Segment::Open(title) => builder.toggles.push((toggle(title), Vec::new())),
                        Segment::Close(source) => {
                            if !builder.close() {
                                builder.push(Block::paragraph(source));
                            }
                        }
                        Segment::Block(block) => builder.push(block),
                    }
                }
            }
            Md::Heading(level, lines) => {
                let (text, images) = split_images(join(lines));
                builder.push(Block::new(BlockKind::Heading {
                    props: HeadingProps {
                        level: level.clamp(1, 6),
                        ..HeadingProps::default()
                    },
                    content: to_inline(text),
                }));
                for image in images {
                    builder.push(image);
                }
            }
            Md::List { start, items } => {
                for (index, item) in items.into_iter().enumerate() {
                    builder.push(list_item(start, index, item));
                }
            }
            Md::Quote(inner) => builder.push(quote(convert(inner))),
            Md::Code { language, code } => builder.push(Block::new(BlockKind::CodeBlock {
                props: CodeBlockProps { language },
                code,
            })),
            Md::Rule => builder.push(Block::new(BlockKind::Divider { props: NoProps {} })),
            Md::Table { alignments, rows } => builder.push(table(&alignments, rows)),
            // A line naming a block that has no other: an empty paragraph.
            Md::Html(html) if names_block(&html) => builder.push(Block::paragraph("")),
            Md::Html(html) => {
                let text = if is_break_tag(&html) {
                    "\n".to_owned()
                } else {
                    html.trim_end_matches('\n').to_owned()
                };
                builder.push(Block::paragraph(text));
            }
        }
    }
    builder.finish()
}

/// A paragraph's lines, split where the extension's lines are.
fn segments(lines: Vec<Line<'_>>) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut text: Vec<Line<'_>> = Vec::new();
    for line in lines {
        let source = line.source.trim();
        let extension =
            if !source.is_empty() && source.chars().all(|c| c == ':') && source.len() >= 3 {
                Some(Segment::Close(source))
            } else if let Some(rest) = directive(source, "toggle") {
                Some(Segment::Open(rest))
            } else {
                media(source)
                    .or_else(|| database(source))
                    .map(Segment::Block)
            };
        match extension {
            Some(segment) => {
                if !text.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut text)));
                }
                segments.push(segment);
            }
            None => {
                let mut line = line;
                if text.is_empty() {
                    line.joint = Joint::First;
                }
                text.push(line);
            }
        }
    }
    if !text.is_empty() {
        segments.push(Segment::Text(text));
    }
    segments
}

/// The rest of a line that opens the directive `name`: three or more colons, the name, and
/// either nothing or whitespace before the rest.
fn directive<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let colons = source.chars().take_while(|c| *c == ':').count();
    if colons < 3 {
        return None;
    }
    let rest = source[colons..].strip_prefix(name)?;
    if rest.is_empty() {
        Some("")
    } else if rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

/// A file block from a `:::video`, `:::audio` or `:::file` line whose rest is one link.
fn media(source: &str) -> Option<Block> {
    let (kind, rest) = ["video", "audio", "file"]
        .into_iter()
        .find_map(|kind| directive(source, kind).map(|rest| (kind, rest)))?;
    let (name, url, caption) = lone_link(rest)?;
    Some(Block::new(match kind {
        "video" => BlockKind::Video {
            props: ImageProps {
                name,
                url,
                caption,
                ..ImageProps::default()
            },
        },
        "audio" => BlockKind::Audio {
            props: AudioProps {
                name,
                url,
                caption,
                ..AudioProps::default()
            },
        },
        _ => BlockKind::File {
            props: FileProps {
                name,
                url,
                caption,
                ..FileProps::default()
            },
        },
    }))
}

/// A database block from a `:::database` line: the database's id, then `view=` and the view's
/// id, each optional, in that order. Anything else on the line makes it text.
fn database(source: &str) -> Option<Block> {
    let rest = directive(source, "database")?;
    let mut words = rest.split_whitespace().peekable();
    let database_id = words
        .next_if(|word| !word.starts_with("view="))
        .map(decode_id)
        .unwrap_or_default();
    let view_id = match words.next() {
        Some(word) => Some(decode_id(word.strip_prefix("view=")?)),
        None => None,
    };
    if words.next().is_some() {
        return None;
    }
    Some(Block::new(BlockKind::Database {
        props: DatabaseProps {
            database_id,
            view_id,
        },
    }))
}

/// An id as the writer encodes it: each `%` and two hex digits stand for a byte, and anything
/// else is itself.
fn decode_id(word: &str) -> String {
    let hex = |digit: u8| {
        char::from(digit)
            .to_digit(16)
            .and_then(|value| u8::try_from(value).ok())
    };
    let bytes = word.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escaped = match bytes.get(index..index + 3) {
            Some([b'%', high, low]) => hex(*high).zip(hex(*low)).map(|(h, l)| h << 4 | l),
            _ => None,
        };
        match escaped {
            Some(escaped) => {
                decoded.push(escaped);
                index += 3;
            }
            None => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The text, target and title of a link that is all of `markdown`.
fn lone_link(markdown: &str) -> Option<(String, String, String)> {
    let mut events = Parser::new_ext(markdown, options());
    if !matches!(events.next(), Some(Event::Start(Tag::Paragraph))) {
        return None;
    }
    let Some(Event::Start(Tag::Link {
        dest_url, title, ..
    })) = events.next()
    else {
        return None;
    };
    let mut text = String::new();
    for event in events.by_ref() {
        match event {
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::End(TagEnd::Link) => break,
            Event::Start(_) | Event::End(_) => {}
            _ => return None,
        }
    }
    match (events.next(), events.next()) {
        (Some(Event::End(TagEnd::Paragraph)), None) => {
            Some((text, dest_url.to_string(), title.to_string()))
        }
        _ => None,
    }
}

/// The block a `:::toggle` line opens: a toggleable heading when its title is a heading, else
/// a toggle list item. A title that is another kind of block is taken as plain text.
fn toggle(title: &str) -> Block {
    let mut parsed = parse(title);
    let (level, content) = match (parsed.len(), parsed.pop()) {
        (1, Some(Md::Heading(level, lines))) => {
            (Some(level), to_inline(split_images(join(lines)).0))
        }
        (1, Some(Md::Paragraph(lines))) => (None, to_inline(split_images(join(lines)).0)),
        _ => (None, Inline::plain(title)),
    };
    Block::new(match level {
        Some(level) => BlockKind::Heading {
            props: HeadingProps {
                level: level.clamp(1, 6),
                is_toggleable: true,
                ..HeadingProps::default()
            },
            content,
        },
        None => BlockKind::ToggleListItem {
            props: TextProps::default(),
            content,
        },
    })
}

/// The parts of a paragraph's lines, with the line breaks between them: a soft break is a
/// space, with the styles around it.
fn join(lines: Vec<Line<'_>>) -> Vec<Part> {
    let mut parts = Vec::new();
    for line in lines {
        match line.joint {
            Joint::First => {}
            Joint::Soft(marks) => parts.push(text_part(" ", marks)),
            Joint::Hard => parts.push(Part::Piece(Piece::Break)),
        }
        parts.extend(line.parts);
    }
    parts
}

/// A paragraph's text, split where its images are: the text between them, and the images as
/// blocks, in order.
fn paragraph(parts: Vec<Part>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text: Vec<Piece> = Vec::new();
    let has_images = parts.iter().any(|part| matches!(part, Part::Image { .. }));
    let flush = |text: &mut Vec<Piece>, blocks: &mut Vec<Block>| {
        let mut pieces = std::mem::take(text);
        if has_images {
            trim(&mut pieces);
        }
        let content = to_inline(pieces);
        if !content.is_empty() {
            blocks.push(Block::new(BlockKind::Paragraph {
                props: TextProps::default(),
                content,
            }));
        }
    };
    for part in parts {
        match part {
            Part::Piece(piece) => text.push(piece),
            Part::Image { url, alt, title } => {
                flush(&mut text, &mut blocks);
                blocks.push(image(url, alt, title));
            }
        }
    }
    flush(&mut text, &mut blocks);
    blocks
}

fn image(url: String, name: String, caption: String) -> Block {
    Block::new(BlockKind::Image {
        props: ImageProps {
            name,
            url,
            caption,
            ..ImageProps::default()
        },
    })
}

/// Drops the whitespace at the edges of text an image was cut out of.
fn trim(pieces: &mut Vec<Piece>) {
    if let Some(Piece::Text { text, .. }) = pieces.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(Piece::Text { text, .. }) = pieces.last_mut() {
        *text = text.trim_end().to_owned();
    }
    pieces.retain(|piece| !matches!(piece, Piece::Text { text, .. } if text.is_empty()));
}

/// The text of the parts without their images, and the images as blocks.
fn split_images(parts: Vec<Part>) -> (Vec<Piece>, Vec<Block>) {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for part in parts {
        match part {
            Part::Piece(piece) => text.push(piece),
            Part::Image { url, alt, title } => images.push(image(url, alt, title)),
        }
    }
    (text, images)
}

/// A list item: its first paragraph is its text, and what follows its children. `list_start`
/// is its list's first number, for a numbered list, and a task marker makes it a check list
/// item whatever its list.
fn list_item(list_start: Option<u64>, index: usize, blocks: Vec<Md<'_>>) -> Block {
    let task = match blocks.first() {
        Some(Md::Paragraph(lines)) => lines.first().and_then(|line| line.task),
        _ => None,
    };
    // An empty paragraph a line names first is the item's child, not its text.
    let named_first = matches!(blocks.first(), Some(Md::Html(html)) if names_block(html));
    let mut converted = convert(blocks).into_iter().peekable();
    let content = match converted.next_if(|block| {
        !named_first
            && matches!(block.kind, BlockKind::Paragraph { .. })
            && block.children.is_empty()
    }) {
        Some(Block {
            kind: BlockKind::Paragraph { content, .. },
            ..
        }) => content,
        _ => Vec::new(),
    };
    let kind = match (task, list_start) {
        (Some(checked), _) => BlockKind::CheckListItem {
            props: CheckListItemProps {
                checked,
                ..CheckListItemProps::default()
            },
            content,
        },
        (None, Some(start)) => BlockKind::NumberedListItem {
            props: NumberedListItemProps {
                start: (index == 0 && start != 1).then(|| u32::try_from(start).unwrap_or(u32::MAX)),
                ..NumberedListItemProps::default()
            },
            content,
        },
        (None, None) => BlockKind::BulletListItem {
            props: TextProps::default(),
            content,
        },
    };
    Block::new(kind).with_children(converted.collect())
}

/// A quote: its leading paragraphs, joined by line breaks, are its text, and what follows
/// them its children.
fn quote(blocks: Vec<Block>) -> Block {
    let mut pieces = Vec::new();
    let mut rest = blocks.into_iter().peekable();
    let mut first = true;
    while let Some(Block {
        kind: BlockKind::Paragraph { content, .. },
        ..
    }) = rest.next_if(|block| matches!(block.kind, BlockKind::Paragraph { .. }))
    {
        if !first {
            pieces.push(Piece::Break);
        }
        first = false;
        pieces.extend(to_pieces(&content));
    }
    Block::new(BlockKind::Quote {
        props: QuoteProps::default(),
        content: to_inline(pieces),
    })
    .with_children(rest.collect())
}

fn table(alignments: &[Alignment], rows: Vec<Vec<Vec<Line<'_>>>>) -> Block {
    let columns = alignments.len();
    let rows: Vec<TableRow> = rows
        .into_iter()
        .map(|cells| TableRow {
            cells: cells
                .into_iter()
                .enumerate()
                .map(|(column, lines)| {
                    let pieces = join(lines).into_iter().map(|part| match part {
                        Part::Piece(piece) => piece,
                        Part::Image { alt, .. } => Piece::Text {
                            text: alt,
                            marks: Marks::default(),
                        },
                    });
                    TableCell {
                        content: to_inline(pieces),
                        props: TableCellProps {
                            text_alignment: match alignments.get(column) {
                                Some(Alignment::Center) => TextAlignment::Center,
                                Some(Alignment::Right) => TextAlignment::Right,
                                _ => TextAlignment::Left,
                            },
                            ..TableCellProps::default()
                        },
                    }
                })
                .collect(),
        })
        .collect();
    Block::new(BlockKind::Table {
        props: TableProps::default(),
        content: TableContent {
            column_widths: vec![None; columns],
            header_rows: Some(1),
            header_cols: None,
            rows,
        },
    })
}
