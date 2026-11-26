use super::parse::bound_markers;
use super::{MAX_DEPTH, TooDeep, from_markdown, to_markdown, try_from_markdown};
use crate::inline::{Marks, Piece, to_inline, to_pieces};
use crate::model::{
    Block, BlockKind, CheckListItemProps, CodeBlockProps, DatabaseProps, Document, FileProps,
    HeadingProps, ImageProps, Inline, Link, NumberedListItemProps, StyledText, Styles, TableCell,
    TableCellProps, TableContent, TableProps, TableRow, TextAlignment, TextProps,
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
