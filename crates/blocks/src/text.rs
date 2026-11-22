//! The plain text of a document, for search.

use crate::inline::plain_text;
use crate::model::{BlockKind, Document};

impl Document {
    /// The document's words without markup, for full-text search: each block's text on lines
    /// of its own, children after their parent, table cells separated by tabs, and the names
    /// and captions of images and files. Link targets, colours and other props are left out.
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
                BlockKind::Divider { .. } => {}
                kind => lines.extend(kind.inline_content().map(plain_text)),
            }
        }
        lines.retain(|line| !line.trim().is_empty());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Block, BlockKind, Document, ImageProps};

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
}
