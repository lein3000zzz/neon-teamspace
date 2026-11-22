use super::{from_markdown, to_markdown};
use crate::inline::{Marks, Piece, to_inline, to_pieces};
use crate::model::{
    Block, BlockKind, CheckListItemProps, CodeBlockProps, Document, FileProps, HeadingProps,
    ImageProps, Inline, Link, NumberedListItemProps, StyledText, Styles, TableCell, TableCellProps,
    TableContent, TableProps, TableRow, TextAlignment, TextProps,
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
    assert_eq!(to_markdown(&document), "> one\\\n> two\n\n- a list in it\n");
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
