use super::parse::bound_markers;
use super::{
    MAX_DEPTH, TooDeep, from_markdown, inline_from_markdown, to_markdown, to_markdown_with_ids,
    try_from_markdown,
};
use crate::inline::{Marks, Piece, to_inline, to_pieces};
use crate::model::{
    Block, BlockKind, CheckListItemProps, CodeBlockProps, DatabaseProps, Document, FileProps,
    HeadingProps, ImageProps, Inline, Link, Mention, MentionKind, NumberedListItemProps,
    QuoteProps, StyledText, Styles, TableCell, TableCellProps, TableContent, TableProps, TableRow,
    TextAlignment, TextProps,
};

fn text(text: &str) -> Vec<Inline> {
    Inline::plain(text)
}

fn styled(text: &str, styles: Styles) -> Inline {
    Inline::Text(StyledText {
        text: text.to_owned(),
        styles,
    })
}

fn bold() -> Styles {
    Styles {
        bold: true,
        ..Styles::default()
    }
}

fn italic() -> Styles {
    Styles {
        italic: true,
        ..Styles::default()
    }
}

fn paragraph(content: Vec<Inline>) -> Block {
    Block::new(BlockKind::Paragraph {
        props: TextProps::default(),
        content,
    })
}

fn bullet(content: &str) -> Block {
    Block::new(BlockKind::BulletListItem {
        props: TextProps::default(),
        content: text(content),
    })
}

fn numbered(content: &str, start: Option<u32>) -> Block {
    Block::new(BlockKind::NumberedListItem {
        props: NumberedListItemProps {
            start,
            ..NumberedListItemProps::default()
        },
        content: text(content),
    })
}

fn heading(level: u8, content: &str, is_toggleable: bool) -> Block {
    Block::new(BlockKind::Heading {
        props: HeadingProps {
            level,
            is_toggleable,
            ..HeadingProps::default()
        },
        content: text(content),
    })
}

fn toggle(content: Vec<Inline>) -> Block {
    Block::new(BlockKind::ToggleListItem {
        props: TextProps::default(),
        content,
    })
}

fn doc(blocks: Vec<Block>) -> Document {
    Document::new(blocks)
}

/// Writes the document, checks the markdown, and reads it back.
#[track_caller]
fn both_ways(document: &Document, markdown: &str) {
    assert_eq!(to_markdown(document), markdown, "writing");
    assert_eq!(&from_markdown(markdown), document, "reading back");
}

#[test]
fn paragraphs_and_breaks() {
    both_ways(
        &doc(vec![
            paragraph(text("One line\nand the next")),
            paragraph(text("\nstarts with a break")),
            paragraph(text("ends with a break\n")),
        ]),
        "One line\\\nand the next\n\n<br>starts with a break\n\nends with a break<br>\n",
    );
}

#[test]
fn soft_breaks_are_spaces() {
    assert_eq!(
        from_markdown("one\ntwo"),
        doc(vec![paragraph(text("one two"))])
    );
}

#[test]
fn headings() {
    both_ways(
        &doc(vec![
            heading(1, "One", false),
            heading(3, "C# and # signs #", false),
            heading(6, "Six", false),
        ]),
        "# One\n\n### C# and \\# signs \\#\n\n###### Six\n",
    );
}

#[test]
fn styles() {
    let content = vec![
        styled("bold", bold()),
        styled(" ", Styles::default()),
        styled("italic", italic()),
        styled(" ", Styles::default()),
        styled(
            "both",
            Styles {
                bold: true,
                italic: true,
                ..Styles::default()
            },
        ),
        styled(" ", Styles::default()),
        styled(
            "struck",
            Styles {
                strike: true,
                ..Styles::default()
            },
        ),
        styled(" ", Styles::default()),
        styled(
            "under",
            Styles {
                underline: true,
                ..Styles::default()
            },
        ),
        styled(" ", Styles::default()),
        styled(
            "code `with` ticks",
            Styles {
                code: true,
                ..Styles::default()
            },
        ),
        styled(" ", Styles::default()),
        Inline::Link(Link {
            href: "https://example.com/a (b)".into(),
            content: vec![
                StyledText {
                    text: "a ".into(),
                    styles: Styles::default(),
                },
                StyledText {
                    text: "link".into(),
                    styles: bold(),
                },
            ],
        }),
    ];
    both_ways(
        &doc(vec![paragraph(content)]),
        "**bold** *italic* ***both*** ~~struck~~ <u>under</u> ``code `with` ticks`` \
         [a **link**](<https://example.com/a (b)>)\n",
    );
}

#[test]
fn delimiters_that_would_not_parse_become_tags() {
    // `**` between a letter and punctuation neither opens nor closes.
    let document = doc(vec![paragraph(vec![
        styled("a", Styles::default()),
        styled("(b)", bold()),
        styled("c", Styles::default()),
    ])]);
    both_ways(&document, "a<strong>(b)</strong>c\n");
}

#[test]
fn whitespace_moves_out_of_styles() {
    let document = doc(vec![paragraph(vec![
        styled("bold ", bold()),
        styled("text", Styles::default()),
    ])]);
    assert_eq!(to_markdown(&document), "**bold** text\n");
}

#[test]
fn escapes() {
    let document = doc(vec![
        paragraph(text("# not a heading")),
        paragraph(text("- not a list")),
        paragraph(text("2024. a year")),
        paragraph(text(
            "*stars* _under_ snake_case [brackets] <tag> a\\b ~tilde~ AT&T",
        )),
        paragraph(text(":::toggle not a toggle")),
        paragraph(text(":::")),
        paragraph(text("  leading and trailing  ")),
    ]);
    both_ways(
        &document,
        "\\# not a heading\n\n\\- not a list\n\n2024\\. a year\n\n\
         \\*stars\\* \\_under\\_ snake_case \\[brackets\\] \\<tag> a\\\\b \\~tilde\\~ AT\\&T\n\n\
         \\:::toggle not a toggle\n\n\\:::\n\n&#32; leading and trailing &#32;\n",
    );
}

#[test]
fn lists() {
    let document = doc(vec![
        bullet("one").with_children(vec![bullet("nested"), numbered("first", None)]),
        Block::new(BlockKind::CheckListItem {
            props: CheckListItemProps {
                checked: true,
                ..CheckListItemProps::default()
            },
            content: text("done"),
        }),
        Block::new(BlockKind::CheckListItem {
            props: CheckListItemProps::default(),
            content: Vec::new(),
        }),
        paragraph(text("between")),
        numbered("three", Some(3)),
        numbered("four", None),
        numbered("again", Some(7)),
        bullet("with a paragraph").with_children(vec![paragraph(text("under it"))]),
    ]);
    both_ways(
        &document,
        "- one\n  - nested\n\n  1. first\n- [x] done\n- [ ]\n\nbetween\n\n3. three\n4. four\n\n\
         7) again\n\n- with a paragraph\n\n  under it\n",
    );
}

#[test]
fn quotes_join_their_paragraphs() {
    let document = doc(vec![
        Block::new(BlockKind::Quote {
            props: Default::default(),
            content: text("one\ntwo"),
        })
        .with_children(vec![bullet("a list in it")]),
    ]);
    assert_eq!(
        from_markdown("> one\n>\n> two\n>\n> - a list in it\n"),
        document
    );
    both_ways(&document, "> one\\\n> two\n>\n> - a list in it\n");
}

#[test]
fn quotes_hold_the_blocks_markdown_puts_in_them() {
    let quote = |content: &str| {
        Block::new(BlockKind::Quote {
            props: Default::default(),
            content: text(content),
        })
    };
    // A quote of a list alone is a quote with no text of its own, holding the list.
    let lists = doc(vec![
        Block::new(BlockKind::Quote {
            props: Default::default(),
            content: Vec::new(),
        })
        .with_children(vec![bullet("a"), bullet("b")]),
    ]);
    assert_eq!(from_markdown("> - a\n> - b\n"), lists);
    both_ways(&lists, "> - a\n> - b\n");
    let nested = doc(vec![quote("outer").with_children(vec![
        quote("inner"),
        paragraph(text("after the inner quote")),
    ])]);
    both_ways(
        &nested,
        "> outer\n>\n> > inner\n>\n> after the inner quote\n",
    );
    // A paragraph under a quote would read as more of its text: it follows the quote.
    let followed = doc(vec![
        quote("said").with_children(vec![paragraph(text("then this")), bullet("and a list")]),
    ]);
    assert_eq!(
        to_markdown(&followed),
        "> said\n\nthen this\n\n- and a list\n"
    );
}

#[test]
fn what_markdown_bounds_is_written_within_its_bounds() {
    // A carriage return in text stays one, rather than ending the line.
    both_ways(&doc(vec![paragraph(text("a\rb"))]), "a&#13;b\n");
    // CommonMark reads nine digits at most as a list item's number.
    let big = doc(vec![
        numbered("a", Some(4_000_000_000)),
        numbered("b", None),
    ]);
    assert_eq!(to_markdown(&big), "999999999. a\n999999999. b\n");
    assert_eq!(
        from_markdown(&to_markdown(&big)),
        doc(vec![numbered("a", Some(999_999_999)), numbered("b", None)])
    );
}

#[test]
fn code_blocks() {
    let code = |language: &str, code: &str| {
        Block::new(BlockKind::CodeBlock {
            props: CodeBlockProps {
                language: language.into(),
            },
            code: code.into(),
        })
    };
    both_ways(
        &doc(vec![
            code("rust", "fn main() {\n    println!(\"```\");\n}"),
            code("markdown", "````\nfenced\n````"),
            code("text", ""),
            code("text", "trailing line\n"),
        ]),
        "```rust\nfn main() {\n    println!(\"```\");\n}\n```\n\n`````markdown\n````\nfenced\n````\n`````\n\n```\n```\n\n```\ntrailing line\n\n```\n",
    );
    assert_eq!(
        from_markdown("    indented\n"),
        doc(vec![code("text", "indented")])
    );
}

#[test]
fn tables() {
    let cell = |content: Vec<Inline>, alignment| TableCell {
        content,
        props: TableCellProps {
            text_alignment: alignment,
            ..TableCellProps::default()
        },
    };
    let document = doc(vec![Block::new(BlockKind::Table {
        props: TableProps::default(),
        content: TableContent {
            column_widths: vec![None, None],
            header_rows: Some(1),
            header_cols: None,
            rows: vec![
                TableRow {
                    cells: vec![
                        cell(text("name"), TextAlignment::Left),
                        cell(text("a | b"), TextAlignment::Right),
                    ],
                },
                TableRow {
                    cells: vec![
                        cell(vec![styled("x", bold())], TextAlignment::Left),
                        cell(text("two\nlines"), TextAlignment::Right),
                    ],
                },
            ],
        },
    })]);
    both_ways(
        &document,
        "| name | a \\| b |\n| --- | ---: |\n| **x** | two<br>lines |\n",
    );
}

#[test]
fn toggles() {
    let document = doc(vec![
        toggle(vec![styled("Title", bold())])
            .with_children(vec![paragraph(text("inside")), toggle(text("nested"))]),
        heading(2, "Section", true).with_children(vec![bullet("item")]),
        toggle(Vec::new()),
    ]);
    both_ways(
        &document,
        ":::toggle **Title**\n\ninside\n\n:::toggle nested\n:::\n\n:::\n\n\
         :::toggle ## Section\n\n- item\n\n:::\n\n:::toggle\n:::\n",
    );
}

#[test]
fn toggles_close_with_their_container_and_tolerate_tight_lines() {
    assert_eq!(
        from_markdown(":::toggle a\nchild\n:::\nafter\n\n- item\n  :::toggle b\n  inside\n- next"),
        doc(vec![
            toggle(text("a")).with_children(vec![paragraph(text("child"))]),
            paragraph(text("after")),
            bullet("item").with_children(vec![
                toggle(text("b")).with_children(vec![paragraph(text("inside"))])
            ]),
            bullet("next"),
        ])
    );
}

#[test]
fn images_and_files() {
    let document = doc(vec![
        Block::new(BlockKind::Image {
            props: ImageProps {
                name: "a diagram".into(),
                url: "https://example.com/d.png".into(),
                caption: "The \"parts\"".into(),
                ..ImageProps::default()
            },
        }),
        Block::new(BlockKind::Video {
            props: ImageProps {
                name: "demo.mp4".into(),
                url: "https://example.com/demo.mp4".into(),
                ..ImageProps::default()
            },
        }),
        Block::new(BlockKind::File {
            props: FileProps {
                name: "report [final].pdf".into(),
                url: "https://example.com/r.pdf".into(),
                caption: "Q3".into(),
                ..FileProps::default()
            },
        }),
    ]);
    both_ways(
        &document,
        "![a diagram](https://example.com/d.png \"The \\\"parts\\\"\")\n\n\
         :::video [demo.mp4](https://example.com/demo.mp4)\n\n\
         :::file [report \\[final\\].pdf](https://example.com/r.pdf \"Q3\")\n",
    );
}

fn database(database_id: &str, view_id: Option<&str>) -> Block {
    Block::new(BlockKind::Database {
        props: DatabaseProps {
            database_id: database_id.to_owned(),
            view_id: view_id.map(str::to_owned),
        },
    })
}

#[test]
fn databases() {
    const DATABASE: &str = "0193a5b2-0000-7000-8000-0000000000d1";
    const VIEW: &str = "0193a5b2-0000-7000-8000-0000000000e1";
    let document = doc(vec![
        database(DATABASE, Some(VIEW)),
        database(DATABASE, None),
        database("", None),
        database("", Some(VIEW)),
        database("", Some("")),
        bullet("in a list").with_children(vec![database(DATABASE, None)]),
    ]);
    both_ways(
        &document,
        ":::database 0193a5b2-0000-7000-8000-0000000000d1 view=0193a5b2-0000-7000-8000-0000000000e1\n\n\
         :::database 0193a5b2-0000-7000-8000-0000000000d1\n\n\
         :::database\n\n\
         :::database view=0193a5b2-0000-7000-8000-0000000000e1\n\n\
         :::database view=\n\n\
         - in a list\n\n  :::database 0193a5b2-0000-7000-8000-0000000000d1\n",
    );
}

#[test]
fn database_ids_are_percent_encoded_where_markdown_would_read_them_otherwise() {
    let document = doc(vec![database("view=a b\\%\n*é", Some("<one> two\\"))]);
    both_ways(
        &document,
        ":::database view%3Da%20b%5C%25%0A%2A%C3%A9 view=%3Cone%3E%20two%5C\n",
    );
}

#[test]
fn database_lines_that_do_not_parse_are_text() {
    assert_eq!(
        from_markdown(":::database one two\n\n:::database one view=a extra\n\n:::databases x"),
        doc(vec![
            paragraph(text(":::database one two")),
            paragraph(text(":::database one view=a extra")),
            paragraph(text(":::databases x")),
        ])
    );
    // A `%` that starts no escape is itself, and bytes that are not UTF-8 are replaced.
    assert_eq!(
        from_markdown(":::database 50%25%off view=%FF%zz"),
        doc(vec![database("50%%off", Some("\u{FFFD}%zz"))])
    );
    assert_eq!(
        to_markdown(&doc(vec![paragraph(text(":::database not a block"))])),
        "\\:::database not a block\n"
    );
}

#[test]
fn images_split_paragraphs() {
    assert_eq!(
        from_markdown("before ![x](https://example.com/x.png) after"),
        doc(vec![
            paragraph(text("before")),
            Block::new(BlockKind::Image {
                props: ImageProps {
                    name: "x".into(),
                    url: "https://example.com/x.png".into(),
                    ..ImageProps::default()
                },
            }),
            paragraph(text("after")),
        ])
    );
}

#[test]
fn children_of_other_blocks_follow_them() {
    let document = doc(vec![
        paragraph(text("parent")).with_children(vec![paragraph(text("child"))]),
        paragraph(Vec::new()),
        Block::new(BlockKind::Divider {
            props: Default::default(),
        }),
    ]);
    assert_eq!(to_markdown(&document), "parent\n\nchild\n\n---\n");
}

#[test]
fn html_is_text() {
    assert_eq!(
        from_markdown("<div>block</div>\n\ninline <span>tag</span>"),
        doc(vec![
            paragraph(text("<div>block</div>")),
            paragraph(text("inline <span>tag</span>")),
        ])
    );
}

#[test]
fn an_empty_document_is_empty_markdown() {
    assert_eq!(to_markdown(&Document::default()), "");
    assert_eq!(from_markdown(""), Document::default());
}

/// A small deterministic generator, so the generated cases are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

/// Each character's marks, but for whitespace, which the writer moves out of some styles.
fn char_marks(content: &[Inline]) -> Vec<(char, Marks)> {
    to_pieces(content)
        .into_iter()
        .flat_map(|piece| match piece {
            Piece::Text { text, marks } => {
                text.chars().map(|c| (c, marks.clone())).collect::<Vec<_>>()
            }
            Piece::Break => vec![('\n', Marks::default())],
            Piece::Mention(mention) => mention
                .props
                .label
                .chars()
                .map(|c| (c, Marks::default()))
                .collect(),
        })
        .map(|(c, mut marks)| {
            if c.is_whitespace() {
                marks.styles.bold = false;
                marks.styles.italic = false;
                marks.styles.strike = false;
            }
            (c, marks)
        })
        .collect()
}

#[test]
fn generated_styled_text_survives() {
    const WORDS: [&str; 22] = [
        "word", " ", "(x)", "a_b", "*", "!", ", ", "#", "1.", "-", "`", "é", "~", "_", "[", "]",
        "<", "&x", "**", ":::", "  ", "\\",
    ];
    let mut rng = Lcg(7);
    for case in 0..3000 {
        let runs = 1 + rng.next(6);
        let mut pieces = Vec::new();
        for _ in 0..runs {
            let mut marks = Marks::default();
            marks.styles.bold = rng.next(2) == 0;
            marks.styles.italic = rng.next(2) == 0;
            marks.styles.strike = rng.next(4) == 0;
            marks.styles.underline = rng.next(4) == 0;
            marks.styles.code = rng.next(8) == 0;
            if rng.next(4) == 0 {
                marks.link = Some(format!("https://example.com/{}", rng.next(2)));
            }
            let mut text = String::new();
            for _ in 0..1 + rng.next(3) {
                text.push_str(WORDS[rng.next(WORDS.len() as u64) as usize]);
            }
            pieces.push(Piece::Text { text, marks });
            if rng.next(10) == 0 {
                pieces.push(Piece::Break);
            }
        }
        let content = to_inline(pieces);
        let document = doc(vec![paragraph(content.clone())]);
        let markdown = to_markdown(&document);
        let read = from_markdown(&markdown);
        let read_content = match read.blocks.as_slice() {
            [
                Block {
                    kind: BlockKind::Paragraph { content, .. },
                    ..
                },
            ] => content.clone(),
            [] => Vec::new(),
            other => panic!("case {case}: {markdown:?} read as {other:?}"),
        };
        let expected: Vec<_> = char_marks(&content);
        let actual: Vec<_> = char_marks(&read_content);
        // A paragraph's trailing break and leading spaces are kept; nothing else may change.
        assert_eq!(actual, expected, "case {case}: {markdown:?}");
        assert_eq!(
            to_markdown(&read),
            markdown,
            "case {case}: stable after one pass"
        );
    }
}

#[test]
fn html_style_tags_are_styles() {
    assert_eq!(
        from_markdown("<b>bold</b> <I>italic</I> <s>struck</s> <ins>under</ins>"),
        doc(vec![paragraph(vec![
            styled("bold", bold()),
            styled(" ", Styles::default()),
            styled("italic", italic()),
            styled(" ", Styles::default()),
            styled(
                "struck",
                Styles {
                    strike: true,
                    ..Styles::default()
                }
            ),
            styled(" ", Styles::default()),
            styled(
                "under",
                Styles {
                    underline: true,
                    ..Styles::default()
                }
            ),
        ])])
    );
}

#[test]
fn a_toggle_title_that_is_another_block_is_text() {
    assert_eq!(
        from_markdown(":::toggle - not a list\n:::"),
        doc(vec![toggle(text("- not a list"))])
    );
}

/// How many levels a document's blocks nest.
fn depth(document: &Document) -> usize {
    let mut deepest = 0;
    let mut stack: Vec<(&Block, usize)> = document.blocks.iter().map(|block| (block, 1)).collect();
    while let Some((block, level)) = stack.pop() {
        deepest = deepest.max(level);
        stack.extend(block.children.iter().map(|child| (child, level + 1)));
    }
    deepest
}

/// The texts of a document's blocks, parents before their children.
fn texts(document: &Document) -> Vec<String> {
    document
        .walk()
        .map(|block| match &block.kind {
            BlockKind::Paragraph { content, .. }
            | BlockKind::Quote { content, .. }
            | BlockKind::BulletListItem { content, .. }
            | BlockKind::ToggleListItem { content, .. } => content
                .iter()
                .map(|inline| match inline {
                    Inline::Text(text) => text.text.clone(),
                    Inline::Link(link) => format!("{link:?}"),
                    Inline::Mention(mention) => format!("{mention:?}"),
                })
                .collect(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// Markdown nesting `levels` blocks of one shape, the innermost one saying `inner`.
fn nested(shape: &str, levels: usize) -> String {
    match shape {
        "quote" => format!("{}inner", "> ".repeat(levels)),
        "list" => (0..levels)
            .map(|level| format!("{}- item {level}\n", "  ".repeat(level)))
            .collect::<String>()
            .replace(&format!("item {}", levels - 1), "inner"),
        "toggle" => format!(
            "{}inner\n\n{}",
            ":::toggle t\n\n".repeat(levels - 1),
            ":::\n\n".repeat(levels - 1)
        ),
        // Toggles holding quotes: half of each.
        _ => format!(
            "{}{}inner\n\n{}",
            ":::toggle t\n\n".repeat(levels / 2),
            "> ".repeat(levels - levels / 2),
            ":::\n\n".repeat(levels / 2)
        ),
    }
}

#[test]
fn blocks_nest_up_to_the_limit() {
    for shape in ["quote", "list", "toggle", "toggled quotes"] {
        let markdown = nested(shape, MAX_DEPTH);
        let read = try_from_markdown(&markdown).unwrap_or_else(|_| panic!("{shape}: refused"));
        assert_eq!(depth(&read), MAX_DEPTH, "{shape}");
        assert_eq!(from_markdown(&markdown), read, "{shape}");
        assert!(texts(&read).iter().any(|text| text == "inner"), "{shape}");
    }
}

#[test]
fn deeper_blocks_are_read_at_the_last_level_or_refused() {
    for shape in ["quote", "list", "toggle", "toggled quotes"] {
        let markdown = nested(shape, MAX_DEPTH + 1);
        assert_eq!(try_from_markdown(&markdown), Err(TooDeep), "{shape}");
        let read = from_markdown(&markdown);
        assert_eq!(depth(&read), MAX_DEPTH, "{shape}");
        assert!(
            texts(&read).iter().any(|text| text.contains("inner")),
            "{shape}: {:?}",
            texts(&read)
        );
    }
    assert_eq!(
        TooDeep.to_string(),
        format!("blocks nest at most {MAX_DEPTH} levels deep")
    );
}

#[test]
fn a_childless_toggle_at_the_last_level_is_not_too_deep() {
    let markdown = format!(
        "{}:::toggle last\n\n:::\n\n{}",
        ":::toggle t\n\n".repeat(MAX_DEPTH - 1),
        ":::\n\n".repeat(MAX_DEPTH - 1)
    );
    let read = try_from_markdown(&markdown).expect("within the limit");
    assert_eq!(depth(&read), MAX_DEPTH);
}

#[test]
fn markdown_nested_thousands_deep_reads_on_a_workers_stack() {
    let inputs = [
        "> ".repeat(5000),
        "- ".repeat(5000) + "x",
        "1. ".repeat(5000) + "x",
        "> - ".repeat(2500) + "x",
        ":::toggle t\n\n".repeat(20_000),
        ":::toggle t\n\n".repeat(20) + &"> ".repeat(5000),
    ];
    // Two MiB, as each of an async runtime's worker threads has.
    let read = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            inputs
                .iter()
                .map(|markdown| {
                    let read = from_markdown(markdown);
                    (depth(&read), try_from_markdown(markdown).is_err())
                })
                .collect::<Vec<_>>()
        })
        .expect("a thread")
        .join()
        .expect("read without overflowing the stack");
    for (index, (depth, refused)) in read.into_iter().enumerate() {
        assert_eq!(depth, MAX_DEPTH, "input {index}");
        assert!(refused, "input {index}");
    }
}

#[test]
fn a_line_opens_a_bounded_number_of_containers() {
    let markers = MAX_DEPTH + 1;
    for marker in ["> ", ">", "- ", "* ", "+\t", "1. ", "12) "] {
        let line = format!("{}text\n", marker.repeat(markers + 5));
        let (bounded, escaped) = bound_markers(&line);
        assert!(escaped, "{marker:?}");
        let cut = marker.len() * markers;
        let at = if marker.starts_with(char::is_numeric) {
            cut + marker.find(['.', ')']).expect("a delimiter")
        } else {
            cut
        };
        assert_eq!(&bounded[..at], &line[..at], "{marker:?}");
        assert_eq!(&bounded[at..at + 1], "\\", "{marker:?}");
        assert_eq!(&bounded[at + 1..], &line[at..], "{marker:?}");
    }

    let within = format!("{}text\n", "> ".repeat(markers));
    assert_eq!(bound_markers(&within), (within.as_str().into(), false));
    // A long thematic break, text that only looks like markers, and every other line, stay.
    for kept in [
        "- ".repeat(100),
        "* ".repeat(100),
        format!("{}x", "-".repeat(100)),
        format!("{}x", "1.".repeat(100)),
        "plain text".to_owned(),
    ] {
        assert_eq!(
            bound_markers(&kept),
            (kept.as_str().into(), false),
            "{kept}"
        );
    }

    // Each line is bounded on its own, and the lines that continue one are kept.
    let lines = format!("{}a\nb\n{}c\n", "> ".repeat(100), "- ".repeat(100));
    let (bounded, escaped) = bound_markers(&lines);
    assert!(escaped);
    assert_eq!(bounded.matches('\\').count(), 2);
    assert!(bounded.contains("\nb\n"));
    let read = from_markdown(&lines);
    assert_eq!(depth(&read), MAX_DEPTH);
    assert!(texts(&read).iter().any(|text| text.ends_with("> > a b")));
}

#[test]
fn ids_are_written_before_their_blocks_at_their_level() {
    let document = doc(vec![
        heading(1, "Plan", false).with_id("h"),
        paragraph(vec![]).with_id("empty"),
        bullet("First")
            .with_id("b1")
            .with_children(vec![bullet("Under it").with_id("b2")]),
        bullet("Second").with_id("b3"),
        Block::new(BlockKind::Quote {
            props: QuoteProps::default(),
            content: text("Said"),
        })
        .with_id("q")
        .with_children(vec![bullet("Inside").with_id("qb")]),
        toggle(text("Fold"))
            .with_id("t")
            .with_children(vec![paragraph(text("Hidden")).with_id("tp")]),
        Block::new(BlockKind::CodeBlock {
            props: CodeBlockProps::default(),
            code: "<!-- block not-one -->".into(),
        })
        .with_id("c"),
        paragraph(text("an id: of / course")).with_id("odd id/1"),
    ]);
    let named = to_markdown_with_ids(&document);
    assert_eq!(
        named,
        "<!-- block h -->\n# Plan\n\n<!-- block empty -->\n\n<!-- block b1 -->\n- First\n  \
         <!-- block b2 -->\n  - Under it\n<!-- block b3 -->\n- Second\n\n<!-- block q -->\n\
         > Said\n>\n> <!-- block qb -->\n> - Inside\n\n<!-- block t -->\n:::toggle Fold\n\n\
         <!-- block tp -->\nHidden\n\n:::\n\n<!-- block c -->\n```\n<!-- block not-one -->\n\
         ```\n\n<!-- block odd%20id%2F1 -->\nan id: of / course\n"
    );
    // Read back, the lines naming blocks are gone, but for what code holds, and the empty
    // paragraph a line names alone stays.
    let mut read = from_markdown(&named);
    assert_eq!(read.blocks.remove(1), paragraph(vec![]));
    assert_eq!(read, from_markdown(&to_markdown(&document)));
    assert!(read.blocks.iter().any(|block| matches!(
        &block.kind,
        BlockKind::CodeBlock { code, .. } if code == "<!-- block not-one -->"
    )));
}

#[test]
fn only_lines_that_name_a_block_alone_are_left_out() {
    assert_eq!(
        from_markdown(
            "<!-- block a -->\nKept\n\n<!-- block two words -->\n\ntext <!-- block b -->"
        ),
        doc(vec![
            paragraph(text("Kept")),
            paragraph(text("<!-- block two words -->")),
            paragraph(text("text <!-- block b -->")),
        ])
    );
    // Numbered lists named item by item read as one list.
    assert_eq!(
        from_markdown("<!-- block a -->\n1. one\n<!-- block b -->\n2. two\n"),
        from_markdown("1. one\n2. two\n")
    );
    assert_eq!(
        from_markdown("> <!-- block a -->\n> quoted"),
        from_markdown("> quoted")
    );
}

#[test]
fn a_line_naming_a_block_with_no_lines_of_its_own_is_an_empty_paragraph() {
    let empty = || paragraph(vec![]);
    // At the end, before a blank line, and before another such line.
    assert_eq!(
        from_markdown("<!-- block a -->\nA\n\n<!-- block e -->\n"),
        doc(vec![paragraph(text("A")), empty()])
    );
    assert_eq!(
        from_markdown("<!-- block e -->\n\n<!-- block a -->\nA\n"),
        doc(vec![empty(), paragraph(text("A"))])
    );
    assert_eq!(
        from_markdown("<!-- block e -->\n<!-- block f -->\n<!-- block a -->\nA"),
        doc(vec![empty(), empty(), paragraph(text("A"))])
    );
    // At the end of a list item, a toggle and a quote.
    assert_eq!(
        from_markdown("- a\n\n  <!-- block e -->\n- b\n"),
        doc(vec![bullet("a").with_children(vec![empty()]), bullet("b")])
    );
    assert_eq!(
        from_markdown("-\n  <!-- block e -->\n"),
        doc(vec![bullet("").with_children(vec![empty()])])
    );
    assert_eq!(
        from_markdown(":::toggle T\n\n<!-- block e -->\n\n:::\n"),
        doc(vec![toggle(text("T")).with_children(vec![empty()])])
    );
    assert_eq!(
        from_markdown("> said\n>\n> - a\n>\n> <!-- block e -->\n"),
        doc(vec![
            Block::new(BlockKind::Quote {
                props: QuoteProps::default(),
                content: text("said"),
            })
            .with_children(vec![bullet("a"), empty()])
        ])
    );
    // A line right before a block's lines names that block.
    assert_eq!(
        from_markdown("<!-- block a -->\n- a\n  <!-- block b -->\n  - b\n"),
        from_markdown("- a\n  - b\n")
    );
}

#[test]
fn empty_paragraphs_written_with_their_ids_read_back_in_place() {
    let document = doc(vec![
        paragraph(text("One")).with_id("a"),
        paragraph(vec![]).with_id("b"),
        bullet("Two").with_id("c").with_children(vec![
            paragraph(vec![]).with_id("d"),
            bullet("x").with_id("e"),
        ]),
        paragraph(vec![]).with_id("f"),
    ]);
    let read = from_markdown(&to_markdown_with_ids(&document));
    assert_eq!(read.blocks, unnamed(&document.blocks));
}

/// The blocks without their ids.
fn unnamed(blocks: &[Block]) -> Vec<Block> {
    blocks
        .iter()
        .map(|block| Block {
            id: None,
            kind: block.kind.clone(),
            children: unnamed(&block.children),
        })
        .collect()
}

fn mention(kind: MentionKind, target: &str, label: &str) -> Inline {
    Inline::Mention(Mention::new(kind, target, label))
}

fn link(href: &str, label: &str) -> Inline {
    Inline::Link(Link {
        href: href.to_owned(),
        content: vec![StyledText {
            text: label.to_owned(),
            styles: Styles::default(),
        }],
    })
}

/// The inline content of a document of one paragraph.
fn paragraph_content(document: &Document) -> Vec<Inline> {
    match document.blocks.as_slice() {
        [block] => block.kind.inline_content().unwrap_or_default().to_vec(),
        other => panic!("one paragraph: {other:?}"),
    }
}

#[test]
fn mentions_are_an_at_sign_before_a_link_to_a_user_or_page() {
    let read = |markdown: &str| paragraph_content(&from_markdown(markdown));
    assert_eq!(
        read("Ask @[Ada](user:a1) about @[Plan **draft**](page:b1)."),
        vec![
            styled("Ask ", Styles::default()),
            mention(MentionKind::User, "a1", "Ada"),
            styled(" about ", Styles::default()),
            mention(MentionKind::Page, "b1", "Plan draft"),
            styled(".", Styles::default()),
        ]
    );
    // An escaped at sign, one in code, a link without one, and other targets stay links.
    assert_eq!(
        read("\\@[x](user:a1) [y](user:a1) @[z](https://example.com) `@`[w](page:b1)"),
        vec![
            styled("@", Styles::default()),
            link("user:a1", "x"),
            styled(" ", Styles::default()),
            link("user:a1", "y"),
            styled(" @", Styles::default()),
            link("https://example.com", "z"),
            styled(" ", Styles::default()),
            styled(
                "@",
                Styles {
                    code: true,
                    ..Styles::default()
                }
            ),
            link("page:b1", "w"),
        ]
    );
    // A backslash escaped before it leaves the at sign a mention's.
    assert_eq!(
        read("\\\\@[Ada](user:a1)"),
        vec![
            styled("\\", Styles::default()),
            mention(MentionKind::User, "a1", "Ada")
        ]
    );
    assert_eq!(
        read("@[](user:a1) @[Bea](team:c1)"),
        vec![
            mention(MentionKind::User, "a1", ""),
            styled(" @", Styles::default()),
            link("team:c1", "Bea"),
        ]
    );
}

#[test]
fn mentions_and_links_like_them_come_back_from_markdown() {
    let content = vec![
        mention(MentionKind::User, "a1", "Ada [the] *first*"),
        styled("@", Styles::default()),
        link("user:a1", "a link"),
        mention(MentionKind::Page, "b1", "Plan"),
        styled(" ", Styles::default()),
        styled("done", bold()),
    ];
    let document = Document::new(vec![paragraph(content.clone())]);
    let markdown = to_markdown(&document);
    assert_eq!(
        markdown,
        "@[Ada \\[the\\] \\*first\\*](user:a1)\\@[a link](user:a1)@[Plan](page:b1) **done**\n"
    );
    assert_eq!(
        paragraph_content(&from_markdown(&markdown)),
        Inline::normalize(&content)
    );
}

#[test]
fn a_document_lists_its_mentions_with_their_blocks() {
    let mut first = paragraph(vec![
        mention(MentionKind::User, "a1", "Ada"),
        styled(" and ", Styles::default()),
        mention(MentionKind::Page, "b1", "Plan"),
    ]);
    first.id = Some("block-1".into());
    let document = Document::new(vec![first, paragraph(text("none"))]);
    let found: Vec<(Option<&str>, &str)> = document
        .mentions()
        .into_iter()
        .map(|(block, mention)| (block, mention.props.target.as_str()))
        .collect();
    assert_eq!(found, [(Some("block-1"), "a1"), (Some("block-1"), "b1")]);
    assert_eq!(document.plain_text(), "@Ada and @Plan\nnone");
}

#[test]
fn comments_read_as_inline_content_alone() {
    let plain = |text: &str| styled(text, Styles::default());
    assert_eq!(
        inline_from_markdown("**Done**, see @[Ada](user:a1)\n\n# not a heading\n- nor a list"),
        vec![
            styled("Done", bold()),
            plain(", see "),
            mention(MentionKind::User, "a1", "Ada"),
            plain("\n\n# not a heading\n- nor a list"),
        ]
    );
    for line in [
        "> not a quote",
        "1. not a list",
        "2) nor this",
        "* nor this",
        "+ nor this",
        "---",
        "***",
        "___",
        "```rust",
        "~~~",
        ":::toggle no toggle",
        "[ref]: https://example.com",
        "<!-- block x -->",
        "=",
    ] {
        assert_eq!(inline_from_markdown(line), text(line), "{line}");
    }
    assert_eq!(
        inline_from_markdown("    indented, *italic* and `code`"),
        vec![
            plain("indented, "),
            styled("italic", italic()),
            plain(" and "),
            styled(
                "code",
                Styles {
                    code: true,
                    ..Styles::default()
                }
            ),
        ]
    );
    assert_eq!(
        inline_from_markdown("an ![image](https://example.com/a.png) is a link"),
        vec![
            plain("an !"),
            link("https://example.com/a.png", "image"),
            plain(" is a link"),
        ]
    );
    assert_eq!(
        inline_from_markdown("[a link](https://example.com) first"),
        vec![link("https://example.com", "a link"), plain(" first")]
    );
    assert!(inline_from_markdown("").is_empty());
}
