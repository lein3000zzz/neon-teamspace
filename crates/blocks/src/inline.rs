//! Inline content as ProseMirror holds it, text nodes with marks and hard breaks, and the
//! conversions between that and the model's runs. They follow BlockNote's own, so the Yjs
//! reader, the markdown parser and both writers agree on one normal form:
//!
//! - a line break is `\n` in the text it ends, whatever that text's styles;
//! - neighbouring runs with the same styles, or links with the same target, are one run;
//! - no run is empty;
//! - code excludes every other style and the link, as BlockNote's code mark does;
//! - an empty colour is no colour.

use crate::model::{Inline, Link, StyledText, Styles};

/// One piece of a block's inline content.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Piece {
    Text { text: String, marks: Marks },
    Break,
}

/// The marks of a text node: its styles and the link around it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Marks {
    pub styles: Styles,
    pub link: Option<String>,
}

impl Marks {
    /// The marks ProseMirror keeps of these.
    pub fn normalized(mut self) -> Self {
        if self.styles.code {
            return Self {
                styles: Styles {
                    code: true,
                    ..Styles::default()
                },
                link: None,
            };
        }
        if self.styles.text_color.as_deref() == Some("") {
            self.styles.text_color = None;
        }
        if self.styles.background_color.as_deref() == Some("") {
            self.styles.background_color = None;
        }
        self
    }
}

/// The model's runs for a sequence of pieces, as BlockNote reads a block's content.
pub(crate) fn to_inline(pieces: impl IntoIterator<Item = Piece>) -> Vec<Inline> {
    let mut runs = Vec::new();
    let mut current: Option<Inline> = None;
    for piece in pieces {
        match piece {
            Piece::Break => match &mut current {
                Some(Inline::Text(run)) => run.text.push('\n'),
                Some(Inline::Link(link)) => match link.content.last_mut() {
                    Some(run) => run.text.push('\n'),
                    None => link.content.push(plain("\n")),
                },
                None => current = Some(Inline::Text(plain("\n"))),
            },
            Piece::Text { text, marks } => {
                if text.is_empty() {
                    continue;
                }
                let Marks { styles, link } = marks.normalized();
                let run = StyledText { text, styles };
                current = Some(match (current.take(), link) {
                    (None, None) => Inline::Text(run),
                    (None, Some(href)) => new_link(href, run),
                    (Some(Inline::Text(mut last)), None) if last.styles == run.styles => {
                        last.text.push_str(&run.text);
                        Inline::Text(last)
                    }
                    (Some(Inline::Link(mut last)), Some(href)) if last.href == href => {
                        match last.content.last_mut() {
                            Some(tail) if tail.styles == run.styles => {
                                tail.text.push_str(&run.text)
                            }
                            _ => last.content.push(run),
                        }
                        Inline::Link(last)
                    }
                    (Some(previous), None) => {
                        runs.push(previous);
                        Inline::Text(run)
                    }
                    (Some(previous), Some(href)) => {
                        runs.push(previous);
                        new_link(href, run)
                    }
                });
            }
        }
    }
    runs.extend(current);
    runs
}

fn plain(text: &str) -> StyledText {
    StyledText {
        text: text.to_owned(),
        styles: Styles::default(),
    }
}

fn new_link(href: String, run: StyledText) -> Inline {
    Inline::Link(Link {
        href,
        content: vec![run],
    })
}

/// The pieces of the model's runs, as BlockNote writes a block's content: each `\n` a
/// hard break of its own, and no empty text.
pub(crate) fn to_pieces(content: &[Inline]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for run in content {
        match run {
            Inline::Text(text) => split(&mut pieces, text, None),
            Inline::Link(link) => {
                for text in &link.content {
                    split(&mut pieces, text, Some(&link.href));
                }
            }
        }
    }
    pieces
}

fn split(pieces: &mut Vec<Piece>, run: &StyledText, link: Option<&str>) {
    let marks = Marks {
        styles: run.styles.clone(),
        link: link.map(str::to_owned),
    }
    .normalized();
    for (index, line) in run.text.split('\n').enumerate() {
        if index > 0 {
            pieces.push(Piece::Break);
        }
        if !line.is_empty() {
            pieces.push(Piece::Text {
                text: line.to_owned(),
                marks: marks.clone(),
            });
        }
    }
}

/// The text of the runs, without styles or link targets.
pub(crate) fn plain_text(content: &[Inline]) -> String {
    let mut text = String::new();
    for run in content {
        match run {
            Inline::Text(run) => text.push_str(&run.text),
            Inline::Link(link) => {
                for run in &link.content {
                    text.push_str(&run.text);
                }
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str, styles: Styles) -> Piece {
        Piece::Text {
            text: text.to_owned(),
            marks: Marks { styles, link: None },
        }
    }

    fn bold() -> Styles {
        Styles {
            bold: true,
            ..Styles::default()
        }
    }

    #[test]
    fn a_break_ends_the_run_before_it_whatever_its_styles() {
        let runs = to_inline([
            text("a", bold()),
            Piece::Break,
            text("b", Styles::default()),
        ]);
        assert_eq!(
            runs,
            vec![
                Inline::Text(StyledText {
                    text: "a\n".into(),
                    styles: bold()
                }),
                Inline::Text(StyledText {
                    text: "b".into(),
                    styles: Styles::default()
                }),
            ]
        );
        assert_eq!(
            to_pieces(&runs),
            vec![
                text("a", bold()),
                Piece::Break,
                text("b", Styles::default())
            ]
        );
    }

    #[test]
    fn a_leading_break_is_unstyled_text() {
        let runs = to_inline([Piece::Break, text("b", Styles::default())]);
        assert_eq!(runs, Inline::plain("\nb"));
    }

    #[test]
    fn runs_with_the_same_link_merge() {
        let linked = |t: &str, styles: Styles| Piece::Text {
            text: t.to_owned(),
            marks: Marks {
                styles,
                link: Some("https://example.com".into()),
            },
        };
        let runs = to_inline([
            linked("a", Styles::default()),
            linked("b", Styles::default()),
            linked("c", bold()),
        ]);
        assert_eq!(
            runs,
            vec![Inline::Link(Link {
                href: "https://example.com".into(),
                content: vec![
                    StyledText {
                        text: "ab".into(),
                        styles: Styles::default()
                    },
                    StyledText {
                        text: "c".into(),
                        styles: bold()
                    },
                ],
            })]
        );
    }

    #[test]
    fn code_excludes_other_styles_and_links() {
        let marks = Marks {
            styles: Styles {
                bold: true,
                code: true,
                text_color: Some("red".into()),
                ..Styles::default()
            },
            link: Some("https://example.com".into()),
        };
        assert_eq!(
            marks.normalized(),
            Marks {
                styles: Styles {
                    code: true,
                    ..Styles::default()
                },
                link: None
            }
        );
    }
}
