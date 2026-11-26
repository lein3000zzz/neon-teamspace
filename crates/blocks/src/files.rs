//! The files a document shows: its image, video, audio and file blocks, each by the URL of
//! its file; and the links of its text, which may lead to a file too.
//!
//! The editor keeps a file's URL in its block's `url` prop, and markdown writes it as the
//! link of `![name](url)` or of the extension's `:::video`, `:::audio` and `:::file`. A link
//! in a paragraph, a heading, a list item, a quote or a table's cell keeps its target in its
//! `href`, and markdown writes it as `[text](url)`. The crate does not know what a URL points
//! to: the server finds its own files among them, and an export rewrites them as paths
//! inside its zip.

use crate::model::{Block, BlockKind, Document, Inline};

impl BlockKind {
    /// The URL of the file an image, video, audio or file block shows, empty until one is
    /// given; `None` for the other blocks.
    pub fn file_url(&self) -> Option<&str> {
        match self {
            Self::Image { props } | Self::Video { props } => Some(&props.url),
            Self::Audio { props } => Some(&props.url),
            Self::File { props } => Some(&props.url),
            _ => None,
        }
    }

    fn file_url_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Image { props } | Self::Video { props } => Some(&mut props.url),
            Self::Audio { props } => Some(&mut props.url),
            Self::File { props } => Some(&mut props.url),
            _ => None,
        }
    }

    /// Every run of styled text the block holds: its content, or each of its table's cells'.
    fn runs(&self) -> Vec<&[Inline]> {
        match self {
            Self::Table { content, .. } => content
                .rows
                .iter()
                .flat_map(|row| row.cells.iter().map(|cell| cell.content.as_slice()))
                .collect(),
            other => other.inline_content().into_iter().collect(),
        }
    }

    fn runs_mut(&mut self) -> Vec<&mut Vec<Inline>> {
        match self {
            Self::Paragraph { content, .. }
            | Self::Heading { content, .. }
            | Self::BulletListItem { content, .. }
            | Self::NumberedListItem { content, .. }
            | Self::CheckListItem { content, .. }
            | Self::ToggleListItem { content, .. }
            | Self::Quote { content, .. } => vec![content],
            Self::Table { content, .. } => content
                .rows
                .iter_mut()
                .flat_map(|row| row.cells.iter_mut().map(|cell| &mut cell.content))
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl Document {
    /// Each block that shows a file, with the file's URL, in document order: parents before
    /// their children. Blocks given no URL yet are left out.
    pub fn files(&self) -> impl Iterator<Item = (&Block, &str)> {
        self.walk().filter_map(|block| {
            block
                .kind
                .file_url()
                .filter(|url| !url.is_empty())
                .map(|url| (block, url))
        })
    }

    /// Gives each block that shows a file the URL `map` returns for it, given the block and
    /// its URL now; a block for which it returns `None`, and one with no URL, keeps its
    /// own. Says how many changed.
    pub fn map_files(&mut self, mut map: impl FnMut(&BlockKind, &str) -> Option<String>) -> usize {
        fn visit(
            blocks: &mut [Block],
            map: &mut dyn FnMut(&BlockKind, &str) -> Option<String>,
            changed: &mut usize,
        ) {
            for block in blocks {
                let url = block
                    .kind
                    .file_url()
                    .filter(|url| !url.is_empty())
                    .and_then(|url| map(&block.kind, url));
                if let (Some(url), Some(slot)) = (url, block.kind.file_url_mut())
                    && *slot != url
                {
                    *slot = url;
                    *changed += 1;
                }
                visit(&mut block.children, map, changed);
            }
        }
        let mut changed = 0;
        visit(&mut self.blocks, &mut map, &mut changed);
        changed
    }
}

impl Document {
    /// The target of each link in the document's text, its blocks' and its tables' cells',
    /// in document order: parents before their children.
    pub fn links(&self) -> impl Iterator<Item = &str> {
        self.walk().flat_map(|block| {
            block
                .kind
                .runs()
                .into_iter()
                .flatten()
                .filter_map(|run| match run {
                    Inline::Link(link) => Some(link.href.as_str()),
                    _ => None,
                })
        })
    }

    /// Gives each link in the document's text the target `map` returns for its target now;
    /// a link for which it returns `None` keeps its own. Says how many changed.
    pub fn map_links(&mut self, mut map: impl FnMut(&str) -> Option<String>) -> usize {
        fn visit(
            blocks: &mut [Block],
            map: &mut dyn FnMut(&str) -> Option<String>,
            changed: &mut usize,
        ) {
            for block in blocks {
                for runs in block.kind.runs_mut() {
                    for run in runs.iter_mut() {
                        if let Inline::Link(link) = run
                            && let Some(href) = map(&link.href)
                            && href != link.href
                        {
                            link.href = href;
                            *changed += 1;
                        }
                    }
                }
                visit(&mut block.children, map, changed);
            }
        }
        let mut changed = 0;
        visit(&mut self.blocks, &mut map, &mut changed);
        changed
    }
}

#[cfg(test)]
mod tests {
    use crate::markdown::from_markdown;
    use crate::model::{AudioProps, Block, BlockKind, Document, FileProps, ImageProps};

    fn image(url: &str) -> Block {
        Block::new(BlockKind::Image {
            props: ImageProps {
                url: url.into(),
                ..ImageProps::default()
            },
        })
    }

    fn document() -> Document {
        Document::new(vec![
            Block::paragraph("text").with_children(vec![image("/api/v1/files/a")]),
            image(""),
            Block::new(BlockKind::Video {
                props: ImageProps {
                    url: "https://example.com/clip.mp4".into(),
                    ..ImageProps::default()
                },
            }),
            Block::new(BlockKind::Audio {
                props: AudioProps {
                    url: "/api/v1/files/b".into(),
                    ..AudioProps::default()
                },
            })
            .with_children(vec![Block::new(BlockKind::File {
                props: FileProps {
                    url: "report.pdf".into(),
                    ..FileProps::default()
                },
            })]),
        ])
    }

    #[test]
    fn files_are_listed_in_document_order() {
        let document = document();
        let listed: Vec<(&str, &str)> = document
            .files()
            .map(|(block, url)| (block.kind.name(), url))
            .collect();
        assert_eq!(
            listed,
            [
                ("image", "/api/v1/files/a"),
                ("video", "https://example.com/clip.mp4"),
                ("audio", "/api/v1/files/b"),
                ("file", "report.pdf"),
            ]
        );
        assert_eq!(Block::paragraph("x").kind.file_url(), None);
    }

    #[test]
    fn urls_are_rewritten_where_the_map_says() {
        let mut document = document();
        let mut seen = Vec::new();
        let changed = document.map_files(|kind, url| {
            seen.push(kind.name());
            url.strip_prefix("/api/v1/files/")
                .map(|id| format!("files/{id}"))
        });
        assert_eq!(changed, 2);
        assert_eq!(
            seen,
            ["image", "video", "audio", "file"],
            "empty URLs are skipped"
        );
        let urls: Vec<&str> = document.files().map(|(_, url)| url).collect();
        assert_eq!(
            urls,
            [
                "files/a",
                "https://example.com/clip.mp4",
                "files/b",
                "report.pdf"
            ]
        );
        assert_eq!(
            document.map_files(|_, url| Some(url.to_owned())),
            0,
            "the same URL is no change"
        );
    }

    #[test]
    fn links_are_listed_from_text_and_tables_in_document_order() {
        let document = from_markdown(
            "A [report](/api/v1/files/a) and [a site](https://example.com).\n\n\
             - [notes](files/b)\n  - [nested](/api/v1/files/c)\n\n\
             | Name | File |\n|---|---|\n| one | [sheet](/api/v1/files/d) |\n\n\
             ![image](/api/v1/files/e)\n\n\
             ```\n[not a link](/api/v1/files/f)\n```\n",
        );
        let links: Vec<&str> = document.links().collect();
        assert_eq!(
            links,
            [
                "/api/v1/files/a",
                "https://example.com",
                "files/b",
                "/api/v1/files/c",
                "/api/v1/files/d",
            ],
            "an image's URL is a block's, and code holds no links"
        );
        assert_eq!(Document::default().links().count(), 0);
    }

    #[test]
    fn links_are_rewritten_where_the_map_says() {
        let mut document = from_markdown(
            "A [report](/api/v1/files/a) and [a site](https://example.com).\n\n\
             | File |\n|---|\n| [sheet](/api/v1/files/d) |\n",
        );
        let changed = document.map_links(|href| {
            href.strip_prefix("/api/v1/files/")
                .map(|id| format!("files/{id}"))
        });
        assert_eq!(changed, 2);
        let links: Vec<&str> = document.links().collect();
        assert_eq!(links, ["files/a", "https://example.com", "files/d"]);
        assert_eq!(
            document.map_links(|href| Some(href.to_owned())),
            0,
            "the same target is no change"
        );
    }
}
