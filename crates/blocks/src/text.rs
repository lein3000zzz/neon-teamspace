//! The plain text of a document, for search, and the mentions it holds.

use crate::inline::plain_text;
use crate::model::{BlockKind, Document, Inline, Mention};

/// The mentions of inline content, in order.
pub fn mentions_in(content: &[Inline]) -> impl Iterator<Item = &Mention> {
    content.iter().filter_map(|run| match run {
        Inline::Mention(mention) => Some(mention),
        Inline::Text(_) | Inline::Link(_) => None,
    })
}

impl Document {
    /// The document's words without markup, for full-text search: each block's text on lines
    /// of its own, children after their parent, table cells separated by tabs, and the names
    /// and captions of images and files. Link targets, colours and other props are left out,
    /// and so are database blocks, which only name a database.
    pub fn plain_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for block in self.walk() {
            match &block.kind {
                BlockKind::CodeBlock { code, .. } => lines.push(code.clone()),
                BlockKind::Table { content, .. } => {
                    for row in &content.rows {
                        let cells: Vec<String> = row
                            .cells
                            .iter()
                            .map(|cell| plain_text(&cell.content))
                            .collect();
                        lines.push(cells.join("\t"));
                    }
                }
                BlockKind::Image { props } | BlockKind::Video { props } => {
                    lines.extend([props.name.clone(), props.caption.clone()]);
                }
                BlockKind::Audio { props } => {
                    lines.extend([props.name.clone(), props.caption.clone()]);
                }
                BlockKind::File { props } => {
                    lines.extend([props.name.clone(), props.caption.clone()]);
                }
                BlockKind::Divider { .. } | BlockKind::Database { .. } => {}
                kind => lines.extend(kind.inline_content().map(plain_text)),
            }
        }
        lines.retain(|line| !line.trim().is_empty());
        lines.join("\n")
    }

    /// Every mention in the document's text, table cells included, in document order, each
    /// with the id of the block it is in.
    pub fn mentions(&self) -> Vec<(Option<&str>, &Mention)> {
        let mut found = Vec::new();
        for block in self.walk() {
            let id = block.id.as_deref();
            match &block.kind {
                BlockKind::Table { content, .. } => {
                    for cell in content.rows.iter().flat_map(|row| &row.cells) {
                        found.extend(mentions_in(&cell.content).map(|mention| (id, mention)));
                    }
                }
                kind => {
                    let content = kind.inline_content().unwrap_or_default();
                    found.extend(mentions_in(content).map(|mention| (id, mention)));
                }
            }
        }
        found
    }
}

impl Inline {
    /// The text of inline content, without styles or link targets; a mention is `@` and
    /// its label.
    pub fn plain_text(content: &[Inline]) -> String {
        plain_text(content)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Block, BlockKind, DatabaseProps, Document, ImageProps};

    #[test]
    fn blocks_are_lines_and_children_follow_parents() {
        let document = Document::new(vec![
            Block::paragraph("first").with_children(vec![Block::paragraph("nested")]),
            Block::new(BlockKind::Image {
                props: ImageProps {
                    name: "diagram.png".into(),
                    caption: "The parts".into(),
                    ..ImageProps::default()
                },
            }),
            Block::paragraph(""),
            Block::paragraph("last"),
        ]);
        assert_eq!(
            document.plain_text(),
            "first\nnested\ndiagram.png\nThe parts\nlast"
        );
    }

    #[test]
    fn databases_are_left_out_but_their_children_are_not() {
        let database = Block::new(BlockKind::Database {
            props: DatabaseProps {
                database_id: "0193a5b2-0000-7000-8000-0000000000d1".into(),
                view_id: Some("0193a5b2-0000-7000-8000-0000000000e1".into()),
            },
        });
        let document = Document::new(vec![
            Block::paragraph("before"),
            database.with_children(vec![Block::paragraph("under it")]),
        ]);
        assert_eq!(document.plain_text(), "before\nunder it");
    }
}
