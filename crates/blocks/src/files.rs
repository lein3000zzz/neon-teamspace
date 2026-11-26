//! The files a document shows: its image, video, audio and file blocks, each by the URL of
//! its file.
//!
//! The editor keeps a file's URL in its block's `url` prop, and markdown writes it as the
//! link of `![name](url)` or of the extension's `:::video`, `:::audio` and `:::file`. The
//! crate does not know what a URL points to: the server finds its own files among them,
//! and an export rewrites them as paths inside its zip.

use crate::model::{Block, BlockKind, Document};

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

#[cfg(test)]
mod tests {
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
}
