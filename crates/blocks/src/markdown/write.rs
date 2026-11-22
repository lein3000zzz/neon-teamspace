//! Document to markdown.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};

use super::parse::options;
use crate::inline::{Marks, Piece, to_pieces};
use crate::model::{
    Block, BlockKind, DEFAULT_LANGUAGE, Document, Inline, TableContent, TextAlignment,
};

/// The document as markdown, ending with a line break unless it is empty.
pub fn to_markdown(document: &Document) -> String {
    let lines = blocks(&document.blocks);
    if lines.is_empty() {
        return String::new();
    }
    let mut markdown = lines.join("\n");
    markdown.push('\n');
    markdown
}

/// Which list a block's lines continue: items of the same list are written without a blank
/// line between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    Bullet,
    Numbered(char),
}

/// The lines of a sequence of sibling blocks.
fn blocks(blocks: &[Block]) -> Vec<String> {
    let mut flat = Vec::new();
    flatten(blocks, &mut flat);
    let mut lines: Vec<String> = Vec::new();
    let mut previous: Option<List> = None;
    let mut number: u64 = 0;
    let mut delimiter = '.';
    for block in flat {
        let (rendered, list) = match &block.kind {
            BlockKind::BulletListItem { content, .. } => (
                list_item("- ", "", content, &block.children),
                Some(List::Bullet),
            ),
            BlockKind::CheckListItem { props, content } => {
                let check = if props.checked { "[x] " } else { "[ ] " };
                (
                    list_item("- ", check, content, &block.children),
                    Some(List::Bullet),
                )
            }
            BlockKind::NumberedListItem { props, content } => {
                match (previous, props.start) {
                    (Some(List::Numbered(_)), None) => number += 1,
                    (Some(List::Numbered(_)), Some(start)) => {
                        delimiter = if delimiter == '.' { ')' } else { '.' };
                        number = u64::from(start);
                    }
                    (_, start) => {
                        delimiter = '.';
                        number = u64::from(start.unwrap_or(1));
                    }
                }
                let marker = format!("{number}{delimiter} ");
                (
                    list_item(&marker, "", content, &block.children),
                    Some(List::Numbered(delimiter)),
                )
            }
            _ => (block_lines(block), None),
        };
        if !lines.is_empty() && (list.is_none() || list != previous) {
            lines.push(String::new());
        }
        lines.extend(rendered);
        previous = list;
    }
    lines
}

/// The blocks to write at one level: children of blocks that cannot hold them in markdown
/// come after their parent, and empty paragraphs are left out.
fn flatten<'a>(blocks: &'a [Block], flat: &mut Vec<&'a Block>) {
    for block in blocks {
        if holds_children(&block.kind) {
            flat.push(block);
            continue;
        }
        let empty_paragraph =
            matches!(&block.kind, BlockKind::Paragraph { content, .. } if content.is_empty());
        if !empty_paragraph {
            flat.push(block);
        }
        flatten(&block.children, flat);
    }
}

fn holds_children(kind: &BlockKind) -> bool {
    match kind {
        BlockKind::BulletListItem { .. }
        | BlockKind::NumberedListItem { .. }
        | BlockKind::CheckListItem { .. }
        | BlockKind::ToggleListItem { .. } => true,
        BlockKind::Heading { props, .. } => props.is_toggleable,
        _ => false,
    }
}

fn list_item(marker: &str, check: &str, content: &[Inline], children: &[Block]) -> Vec<String> {
    let indent = " ".repeat(marker.len());
    let text = inline(content, Context::lines());
    let mut lines = Vec::new();
    for (index, line) in text.iter().enumerate() {
        if index == 0 {
            lines.push(format!("{marker}{check}{line}").trim_end().to_owned());
        } else {
            lines.push(indented(&indent, line));
        }
    }
    if lines.is_empty() {
        lines.push(format!("{marker}{check}").trim_end().to_owned());
    }
    let nested = blocks(children);
    let starts_with_list = children.first().is_some_and(|child| {
        matches!(
            child.kind,
            BlockKind::BulletListItem { .. }
                | BlockKind::NumberedListItem { .. }
                | BlockKind::CheckListItem { .. }
        )
    });
    if !nested.is_empty() && !starts_with_list && !(content.is_empty() && check.is_empty()) {
        lines.push(String::new());
    }
    lines.extend(nested.iter().map(|line| indented(&indent, line)));
    lines
}

fn indented(indent: &str, line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else {
        format!("{indent}{line}")
    }
}

/// The lines of one block that is not a list item.
fn block_lines(block: &Block) -> Vec<String> {
    match &block.kind {
        BlockKind::Paragraph { content, .. } => inline(content, Context::lines()),
        BlockKind::Heading { props, content } => {
            let hashes = "#".repeat(usize::from(props.level.clamp(1, 6)));
            let title = heading_line(&hashes, content);
            if props.is_toggleable {
                toggle(&title, &block.children)
            } else {
                vec![title]
            }
        }
        BlockKind::ToggleListItem { content, .. } => {
            let title = single_line(content, Context::title());
            toggle(&title, &block.children)
        }
        BlockKind::Quote { content, .. } => {
            let lines = inline(content, Context::lines());
            if lines.is_empty() {
                vec![">".to_owned()]
            } else {
                lines.iter().map(|line| format!("> {line}")).collect()
            }
        }
        BlockKind::CodeBlock { props, code } => code_block(&props.language, code),
        BlockKind::Divider { .. } => vec!["---".to_owned()],
        BlockKind::Table { content, .. } => table(content),
        BlockKind::Image { props } => vec![format!(
            "![{}]({})",
            escape_text(&props.name, Edges::default(), Context::inline()),
            target(&props.url, &props.caption)
        )],
        BlockKind::Video { props } => vec![media("video", &props.name, &props.url, &props.caption)],
        BlockKind::Audio { props } => vec![media("audio", &props.name, &props.url, &props.caption)],
        BlockKind::File { props } => vec![media("file", &props.name, &props.url, &props.caption)],
        BlockKind::BulletListItem { .. }
        | BlockKind::NumberedListItem { .. }
        | BlockKind::CheckListItem { .. } => Vec::new(),
    }
}

fn heading_line(hashes: &str, content: &[Inline]) -> String {
    let text = single_line(content, Context::heading());
    if text.is_empty() {
        hashes.to_owned()
    } else {
        format!("{hashes} {text}")
    }
}

fn toggle(title: &str, children: &[Block]) -> Vec<String> {
    let mut lines = vec![if title.is_empty() {
        ":::toggle".to_owned()
    } else {
        format!(":::toggle {title}")
    }];
    let nested = blocks(children);
    if !nested.is_empty() {
        lines.push(String::new());
        lines.extend(nested);
        lines.push(String::new());
    }
    lines.push(":::".to_owned());
    lines
}

fn media(name: &str, file: &str, url: &str, caption: &str) -> String {
    format!(
        ":::{name} [{}]({})",
        escape_text(file, Edges::default(), Context::inline()),
        target(url, caption)
    )
}

/// A link's or image's destination, with its title when there is one.
fn target(url: &str, title: &str) -> String {
    let needs_brackets = url.chars().any(|c| c.is_whitespace() || c.is_control());
    let mut out = String::new();
    if needs_brackets {
        out.push('<');
        for c in url.chars() {
            if matches!(c, '<' | '>' | '\\') {
                out.push('\\');
            }
            out.push(if c.is_control() { ' ' } else { c });
        }
        out.push('>');
    } else {
        for c in url.chars() {
            if matches!(c, '(' | ')' | '<' | '>' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
    }
    if !title.is_empty() {
        out.push_str(" \"");
        for c in title.chars() {
            match c {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' | '\r' => out.push(' '),
                _ => out.push(c),
            }
        }
        out.push('"');
    }
    out
}

fn code_block(language: &str, code: &str) -> Vec<String> {
    let longest = |fence: char| {
        code.lines()
            .map(|line| {
                let trimmed = line.trim_start();
                trimmed.chars().take_while(|c| *c == fence).count()
            })
            .max()
            .unwrap_or(0)
    };
    let info = if language == DEFAULT_LANGUAGE {
        ""
    } else {
        language
    };
    let fence_char = if info.contains('`') { '~' } else { '`' };
    let fence = fence_char
        .to_string()
        .repeat(longest(fence_char).max(2) + 1);
    let mut lines = vec![format!("{fence}{}", info.replace(['\n', '\r'], " "))];
    if !code.is_empty() {
        lines.extend(code.split('\n').map(str::to_owned));
    }
    lines.push(fence);
    lines
}

fn table(content: &TableContent) -> Vec<String> {
    let columns = content
        .rows
        .iter()
        .map(|row| row.cells.len())
        .max()
        .unwrap_or(0)
        .max(1);
    let alignments: Vec<TextAlignment> = (0..columns)
        .map(|column| {
            let mut cells = content.rows.iter().filter_map(|row| row.cells.get(column));
            let first = cells
                .next()
                .map_or(TextAlignment::Left, |cell| cell.props.text_alignment);
            if cells.all(|cell| cell.props.text_alignment == first) {
                first
            } else {
                TextAlignment::Left
            }
        })
        .collect();
    let row_line = |cells: Vec<String>| format!("| {} |", cells.join(" | "));
    let rendered_row = |index: usize| -> Vec<String> {
        (0..columns)
            .map(|column| {
                content
                    .rows
                    .get(index)
                    .and_then(|row| row.cells.get(column))
                    .map(|cell| single_line(&cell.content, Context::cell()))
                    .unwrap_or_default()
            })
            .collect()
    };
    let mut lines = vec![row_line(rendered_row(0))];
    lines.push(row_line(
        alignments
            .iter()
            .map(|alignment| match alignment {
                TextAlignment::Center => ":---:".to_owned(),
                TextAlignment::Right => "---:".to_owned(),
                TextAlignment::Left | TextAlignment::Justify => "---".to_owned(),
            })
            .collect(),
    ));
    for index in 1..content.rows.len() {
        lines.push(row_line(rendered_row(index)));
    }
    lines
}

// Inline content.

/// Where inline content is written, which decides what must be escaped and how line breaks
/// are written.
#[derive(Debug, Clone, Copy)]
struct Context {
    /// Line breaks as `<br>` on one line, rather than lines ending in `\`.
    one_line: bool,
    /// Whether each line starts a block, so a leading `#`, `-` or `1.` must be escaped.
    starts_lines: bool,
    /// In a heading, where a `#` after a space could close it.
    heading: bool,
    /// In a table cell, where `|` ends the cell.
    cell: bool,
}

impl Context {
    fn lines() -> Self {
        Self {
            one_line: false,
            starts_lines: true,
            heading: false,
            cell: false,
        }
    }

    fn title() -> Self {
        Self {
            one_line: true,
            ..Self::lines()
        }
    }

    fn heading() -> Self {
        Self {
            one_line: true,
            starts_lines: false,
            heading: true,
            cell: false,
        }
    }

    fn cell() -> Self {
        Self {
            one_line: true,
            starts_lines: false,
            heading: false,
            cell: true,
        }
    }

    fn inline() -> Self {
        Self {
            one_line: true,
            starts_lines: false,
            heading: false,
            cell: false,
        }
    }
}

fn single_line(content: &[Inline], context: Context) -> String {
    inline(content, context).join("")
}

/// The lines of a block's inline content. A line break becomes `\` at the end of the line,
/// or `<br>` where a backslash would not make one: at the start or end of the content, and
/// where the context is one line.
fn inline(content: &[Inline], context: Context) -> Vec<String> {
    let mut lines: Vec<Vec<(String, Marks)>> = vec![Vec::new()];
    for piece in to_pieces(content) {
        match piece {
            Piece::Text { text, marks } => {
                if let Some(line) = lines.last_mut() {
                    line.push((text, marks));
                }
            }
            Piece::Break => lines.push(Vec::new()),
        }
    }
    if lines.len() == 1 && lines[0].is_empty() {
        return Vec::new();
    }
    let rendered: Vec<String> = lines
        .iter()
        .map(|line| render_line(line, context))
        .collect();
    let last = rendered.len() - 1;
    let mut out: Vec<String> = Vec::new();
    let mut current = rendered[0].clone();
    for (index, line) in rendered.iter().enumerate().skip(1) {
        let edge = (index == last && line.is_empty()) || (index == 1 && rendered[0].is_empty());
        if context.one_line || edge {
            current.push_str("<br>");
            current.push_str(line);
        } else {
            current.push('\\');
            out.push(std::mem::take(&mut current));
            current.clone_from(line);
        }
    }
    out.push(current);
    out
}

/// A style written with delimiters or tags around its text.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Link(String),
    Underline,
    Bold,
    Italic,
    Strike,
}

impl Kind {
    /// The styles of `marks`, in the order they open when they start together.
    fn of(marks: &Marks) -> Vec<Kind> {
        let mut kinds = Vec::new();
        if let Some(href) = &marks.link {
            kinds.push(Kind::Link(href.clone()));
        }
        if marks.styles.underline {
            kinds.push(Kind::Underline);
        }
        if marks.styles.bold {
            kinds.push(Kind::Bold);
        }
        if marks.styles.italic {
            kinds.push(Kind::Italic);
        }
        if marks.styles.strike {
            kinds.push(Kind::Strike);
        }
        kinds
    }

    fn delimiter(&self) -> Option<&'static str> {
        match self {
            Kind::Bold => Some("**"),
            Kind::Italic => Some("*"),
            Kind::Strike => Some("~~"),
            Kind::Link(_) | Kind::Underline => None,
        }
    }

    fn open(&self, html: bool) -> String {
        match (self, html) {
            (Kind::Link(_), _) => "[".to_owned(),
            (Kind::Underline, _) => "<u>".to_owned(),
            (Kind::Bold, true) => "<strong>".to_owned(),
            (Kind::Italic, true) => "<em>".to_owned(),
            (Kind::Strike, true) => "<del>".to_owned(),
            (kind, false) => kind.delimiter().unwrap_or_default().to_owned(),
        }
    }

    fn close(&self, html: bool) -> String {
        match (self, html) {
            (Kind::Link(href), _) => format!("]({})", target(href, "")),
            (Kind::Underline, _) => "</u>".to_owned(),
            (Kind::Bold, true) => "</strong>".to_owned(),
            (Kind::Italic, true) => "</em>".to_owned(),
            (Kind::Strike, true) => "</del>".to_owned(),
            (kind, false) => kind.delimiter().unwrap_or_default().to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
enum Token {
    Text(String),
    Code(String),
    /// A style opening, with the index of the token that closes it.
    Open(Kind, usize),
    Close(Kind),
}

fn render_line(runs: &[(String, Marks)], context: Context) -> String {
    let runs = expel_whitespace(runs);
    let tokens = tokenize(&runs);
    // Styles start as delimiters. Where CommonMark would read a style's delimiters otherwise,
    // as the characters around them can make it, that style is written as HTML tags.
    let mut html = vec![false; tokens.len()];
    loop {
        let (line, delimiters) = assemble(&tokens, &html, context);
        let misread = misread(&line, &tokens, &delimiters);
        if misread.is_empty() {
            return line;
        }
        for token in misread {
            html[token] = true;
        }
    }
}

/// Moves whitespace at the edges of bold, italic and strikethrough text outside the style,
/// as delimiters next to whitespace do not parse.
fn expel_whitespace(runs: &[(String, Marks)]) -> Vec<(String, Marks)> {
    let mut chars: Vec<(char, Marks)> = runs
        .iter()
        .flat_map(|(text, marks)| text.chars().map(move |c| (c, marks.clone())))
        .collect();
    type Flag = fn(&mut Marks) -> &mut bool;
    let flags: [Flag; 3] = [
        |m| &mut m.styles.bold,
        |m| &mut m.styles.italic,
        |m| &mut m.styles.strike,
    ];
    for flag in flags {
        let mut index = 0;
        while index < chars.len() {
            if !*flag(&mut chars[index].1) {
                index += 1;
                continue;
            }
            let start = index;
            while index < chars.len() && *flag(&mut chars[index].1) {
                index += 1;
            }
            let end = index;
            let mut from = start;
            while from < end && chars[from].0.is_whitespace() {
                *flag(&mut chars[from].1) = false;
                from += 1;
            }
            let mut to = end;
            while to > from && chars[to - 1].0.is_whitespace() {
                *flag(&mut chars[to - 1].1) = false;
                to -= 1;
            }
        }
    }
    let mut merged: Vec<(String, Marks)> = Vec::new();
    for (c, marks) in chars {
        match merged.last_mut() {
            Some((text, last)) if *last == marks => text.push(c),
            _ => merged.push((c.to_string(), marks)),
        }
    }
    merged
}

/// The runs as tokens, styles opening and closing as a stack: a style that ends while one
/// opened after it goes on closes that one too, which then opens again.
fn tokenize(runs: &[(String, Marks)]) -> Vec<Token> {
    let kinds: Vec<Vec<Kind>> = runs.iter().map(|(_, marks)| Kind::of(marks)).collect();
    let mut tokens = Vec::new();
    let mut stack: Vec<(Kind, usize)> = Vec::new();
    for (index, (text, marks)) in runs.iter().enumerate() {
        let wanted = &kinds[index];
        let keep = stack
            .iter()
            .take_while(|(kind, _)| wanted.contains(kind))
            .count();
        while stack.len() > keep {
            close(&mut tokens, &mut stack);
        }
        let mut opening: Vec<&Kind> = wanted
            .iter()
            .filter(|kind| !stack.iter().any(|(open, _)| open == *kind))
            .collect();
        // The style that goes on longest opens first, so it closes last.
        let reach = |kind: &Kind| {
            kinds[index..]
                .iter()
                .take_while(|later| later.contains(kind))
                .count()
        };
        opening.sort_by_key(|kind| std::cmp::Reverse(reach(kind)));
        for kind in opening {
            stack.push((kind.clone(), tokens.len()));
            tokens.push(Token::Open(kind.clone(), 0));
        }
        if marks.styles.code {
            tokens.push(Token::Code(text.clone()));
        } else {
            tokens.push(Token::Text(text.clone()));
        }
    }
    while !stack.is_empty() {
        close(&mut tokens, &mut stack);
    }
    tokens
}

fn close(tokens: &mut Vec<Token>, stack: &mut Vec<(Kind, usize)>) {
    if let Some((kind, open)) = stack.pop() {
        let at = tokens.len();
        if let Some(Token::Open(_, closing)) = tokens.get_mut(open) {
            *closing = at;
        }
        tokens.push(Token::Close(kind));
    }
}

/// A delimiter written for a style, where it is in the line.
#[derive(Debug)]
struct Delimiter {
    /// The opening token of its style.
    open_token: usize,
    opens: bool,
    range: Range<usize>,
    symbol: char,
}

/// The styles, by their opening token, whose delimiters CommonMark does not read as the
/// style they were written for. The line is parsed as markdown, and each style it finds is
/// compared with the ones written. CommonMark takes delimiters from the inner end of a run
/// of them, so a style's extent is taken to the edges of the delimiter runs it starts and
/// ends in: `***x***` is bold and italic over `x` whichever of the two it reads as outer.
fn misread(line: &str, tokens: &[Token], delimiters: &[Delimiter]) -> Vec<usize> {
    if delimiters.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<&Delimiter> = delimiters.iter().collect();
    sorted.sort_by_key(|delimiter| delimiter.range.start);
    let mut runs: Vec<(Range<usize>, char)> = Vec::new();
    for delimiter in sorted {
        match runs.last_mut() {
            Some((run, symbol))
                if run.end == delimiter.range.start && *symbol == delimiter.symbol =>
            {
                run.end = delimiter.range.end;
            }
            _ => runs.push((delimiter.range.clone(), delimiter.symbol)),
        }
    }
    let widen = |range: Range<usize>| {
        let start = runs
            .iter()
            .find(|(run, _)| run.start <= range.start && range.start < run.end)
            .map_or(range.start, |(run, _)| run.start);
        let end = runs
            .iter()
            .find(|(run, _)| run.start < range.end && range.end <= run.end)
            .map_or(range.end, |(run, _)| run.end);
        start..end
    };
    let mut read: Vec<(&'static str, Range<usize>)> = Parser::new_ext(line, options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::Strong) => Some(("bold", widen(range))),
            Event::Start(Tag::Emphasis) => Some(("italic", widen(range))),
            Event::Start(Tag::Strikethrough) => Some(("strike", widen(range))),
            _ => None,
        })
        .collect();
    let mut misread = Vec::new();
    for opener in delimiters.iter().filter(|delimiter| delimiter.opens) {
        let Some(closer) = delimiters
            .iter()
            .find(|delimiter| !delimiter.opens && delimiter.open_token == opener.open_token)
        else {
            continue;
        };
        let style = match &tokens[opener.open_token] {
            Token::Open(Kind::Bold, _) => "bold",
            Token::Open(Kind::Italic, _) => "italic",
            _ => "strike",
        };
        let written = (style, widen(opener.range.start..closer.range.end));
        match read.iter().position(|found| *found == written) {
            Some(found) => {
                read.swap_remove(found);
            }
            None => misread.push(opener.open_token),
        }
    }
    misread
}

/// The line's markdown, and where each style's delimiters are in it.
fn assemble(tokens: &[Token], html: &[bool], context: Context) -> (String, Vec<Delimiter>) {
    let mut out = String::new();
    let mut delimiters = Vec::new();
    let mut open_of: Vec<usize> = vec![0; tokens.len()];
    for (index, token) in tokens.iter().enumerate() {
        if let Token::Open(_, closing) = token
            && let Some(slot) = open_of.get_mut(*closing)
        {
            *slot = index;
        }
    }
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Text(text) => {
                let at_start = out.is_empty();
                let edges = Edges {
                    line_start: context.starts_lines && at_start,
                    first: at_start,
                    last: index + 1 == tokens.len(),
                };
                out.push_str(&escape_text(text, edges, context));
            }
            Token::Code(code) => out.push_str(&code_span(code, context.cell)),
            Token::Open(kind, _) => {
                let as_html = html[index];
                if matches!(kind, Kind::Link(_)) && out.ends_with('!') {
                    // `![` would open an image.
                    out.pop();
                    out.push_str("\\!");
                }
                let start = out.len();
                out.push_str(&kind.open(as_html));
                if let (Some(symbol), false) = (kind.delimiter(), as_html) {
                    delimiters.push(Delimiter {
                        open_token: index,
                        opens: true,
                        range: start..out.len(),
                        symbol: symbol.chars().next().unwrap_or('*'),
                    });
                }
            }
            Token::Close(kind) => {
                let open = open_of[index];
                let as_html = html[open];
                let start = out.len();
                out.push_str(&kind.close(as_html));
                if let (Some(symbol), false) = (kind.delimiter(), as_html) {
                    delimiters.push(Delimiter {
                        open_token: open,
                        opens: false,
                        range: start..out.len(),
                        symbol: symbol.chars().next().unwrap_or('*'),
                    });
                }
            }
        }
    }
    (out, delimiters)
}

fn code_span(code: &str, cell: bool) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = code.starts_with('`')
        || code.ends_with('`')
        || (code.starts_with(' ') && code.ends_with(' ') && !code.chars().all(|c| c == ' '));
    let code = if cell {
        code.replace('|', "\\|")
    } else {
        code.to_owned()
    };
    if pad {
        format!("{fence} {code} {fence}")
    } else {
        format!("{fence}{code}{fence}")
    }
}

/// Where a text is in its line.
#[derive(Debug, Clone, Copy, Default)]
struct Edges {
    /// It starts the line, and the line starts a block.
    line_start: bool,
    /// It starts the line.
    first: bool,
    /// It ends the line.
    last: bool,
}

/// Text with every character that markdown would read as syntax escaped. Whitespace that
/// starts or ends the line, which markdown would strip, becomes a character reference.
fn escape_text(text: &str, edges: Edges, context: Context) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, &c) in chars.iter().enumerate() {
        let before = index.checked_sub(1).map(|i| chars[i]);
        let after = chars.get(index + 1).copied();
        let at_edge = (edges.first && index == 0) || (edges.last && index + 1 == chars.len());
        if at_edge && (c == ' ' || c == '\t') {
            out.push_str(&format!("&#{};", u32::from(c)));
            continue;
        }
        let escape = match c {
            '\\' | '*' | '`' | '[' | ']' | '<' | '~' => true,
            '_' => {
                !(before.is_some_and(char::is_alphanumeric)
                    && after.is_some_and(char::is_alphanumeric))
            }
            '&' => after.is_some_and(|a| a.is_alphanumeric() || a == '#'),
            '|' => context.cell,
            '#' => context.heading && before.is_none_or(char::is_whitespace),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    if edges.line_start {
        escape_line_start(&out)
    } else {
        out
    }
}

/// Escapes what would start a block at the start of a line: a heading, a quote, a list item,
/// a thematic break, a setext underline or a directive.
fn escape_line_start(text: &str) -> String {
    let first = text.chars().next();
    match first {
        Some('#' | '>' | '-' | '+' | '=') => return format!("\\{text}"),
        Some(':') if text.starts_with(":::") => return format!("\\{text}"),
        _ => {}
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && let Some(punctuation @ ('.' | ')')) = text[digits..].chars().next()
    {
        return format!("{}\\{punctuation}{}", &text[..digits], &text[digits + 1..]);
    }
    text.to_owned()
}
