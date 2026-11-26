//! The typed block model. Its JSON form, through serde, is BlockNote's: a block is
//! `{"id", "type", "props", "content", "children"}`, and every type, prop and style has
//! BlockNote's name, so blocks the editor exports deserialize into it and the other way round.

use serde::{Deserialize, Serialize};

use crate::ids::IdSource;

/// The colour value BlockNote uses for "no colour".
pub const DEFAULT_COLOR: &str = "default";

/// The language of a code block that names none.
pub const DEFAULT_LANGUAGE: &str = "text";

/// A page's content: its top-level blocks, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Gives every block without an id one from `ids`, parents before their children, in
    /// document order: the order in which writing the document to a Yjs document would ask.
    pub fn fill_ids(&mut self, ids: &mut dyn IdSource) {
        fn fill(blocks: &mut [Block], ids: &mut dyn IdSource) {
            for block in blocks {
                if block.id.is_none() {
                    block.id = Some(ids.next_id());
                }
                fill(&mut block.children, ids);
            }
        }
        fill(&mut self.blocks, ids);
    }

    /// Every block, parents before their children, in document order.
    pub fn walk(&self) -> impl Iterator<Item = &Block> {
        let mut stack: Vec<&Block> = self.blocks.iter().rev().collect();
        std::iter::from_fn(move || {
            let block = stack.pop()?;
            stack.extend(block.children.iter().rev());
            Some(block)
        })
    }
}

/// One block: its type with its props and content, and the blocks nested under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// BlockNote's id of the block. `None` for a block not stored yet: writing it into a
    /// Yjs document gives it one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub kind: BlockKind,
    #[serde(default)]
    pub children: Vec<Block>,
}

impl Block {
    /// A block without an id or children.
    pub fn new(kind: BlockKind) -> Self {
        Self {
            id: None,
            kind,
            children: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_children(mut self, children: Vec<Block>) -> Self {
        self.children = children;
        self
    }

    /// A paragraph of plain text.
    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::new(BlockKind::Paragraph {
            props: TextProps::default(),
            content: Inline::plain(text),
        })
    }
}

/// A block's type, with the props and the content of that type. The variants are
/// BlockNote's default block types, and the app's own: a database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlockKind {
    Paragraph {
        #[serde(default)]
        props: TextProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    Heading {
        #[serde(default)]
        props: HeadingProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    BulletListItem {
        #[serde(default)]
        props: TextProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    NumberedListItem {
        #[serde(default)]
        props: NumberedListItemProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    CheckListItem {
        #[serde(default)]
        props: CheckListItemProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    /// A list item whose children show only when it is open.
    ToggleListItem {
        #[serde(default)]
        props: TextProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    Quote {
        #[serde(default)]
        props: QuoteProps,
        #[serde(default)]
        content: Vec<Inline>,
    },
    CodeBlock {
        #[serde(default)]
        props: CodeBlockProps,
        /// The code, unstyled; line breaks are `\n`. In JSON, BlockNote's single text.
        #[serde(rename = "content", default, with = "plain_content")]
        code: String,
    },
    Divider {
        #[serde(default)]
        props: NoProps,
    },
    Table {
        #[serde(default)]
        props: TableProps,
        content: TableContent,
    },
    Image {
        #[serde(default)]
        props: ImageProps,
    },
    Video {
        #[serde(default)]
        props: ImageProps,
    },
    Audio {
        #[serde(default)]
        props: AudioProps,
    },
    File {
        #[serde(default)]
        props: FileProps,
    },
    /// A database shown in the page. Not one of BlockNote's: the web app's editor adds it.
    Database {
        #[serde(default)]
        props: DatabaseProps,
    },
}

impl BlockKind {
    /// BlockNote's name of the type, as in JSON and in the Yjs document.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Paragraph { .. } => "paragraph",
            Self::Heading { .. } => "heading",
            Self::BulletListItem { .. } => "bulletListItem",
            Self::NumberedListItem { .. } => "numberedListItem",
            Self::CheckListItem { .. } => "checkListItem",
            Self::ToggleListItem { .. } => "toggleListItem",
            Self::Quote { .. } => "quote",
            Self::CodeBlock { .. } => "codeBlock",
            Self::Divider { .. } => "divider",
            Self::Table { .. } => "table",
            Self::Image { .. } => "image",
            Self::Video { .. } => "video",
            Self::Audio { .. } => "audio",
            Self::File { .. } => "file",
            Self::Database { .. } => "database",
        }
    }

    /// The styled text of a block whose content is styled text.
    pub fn inline_content(&self) -> Option<&[Inline]> {
        match self {
            Self::Paragraph { content, .. }
            | Self::Heading { content, .. }
            | Self::BulletListItem { content, .. }
            | Self::NumberedListItem { content, .. }
            | Self::CheckListItem { content, .. }
            | Self::ToggleListItem { content, .. }
            | Self::Quote { content, .. } => Some(content),
            _ => None,
        }
    }
}

/// How a block's text lines up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl TextAlignment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            "justify" => Some(Self::Justify),
            _ => None,
        }
    }
}

fn default_color() -> String {
    DEFAULT_COLOR.to_owned()
}

/// The props of a paragraph, a bullet list item and a toggle list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextProps {
    pub background_color: String,
    pub text_color: String,
    pub text_alignment: TextAlignment,
}

impl Default for TextProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            text_color: default_color(),
            text_alignment: TextAlignment::Left,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeadingProps {
    pub background_color: String,
    pub text_color: String,
    pub text_alignment: TextAlignment,
    /// 1 to 6.
    pub level: u8,
    /// Whether its children fold away under it, like a toggle list item's.
    pub is_toggleable: bool,
}

impl Default for HeadingProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            text_color: default_color(),
            text_alignment: TextAlignment::Left,
            level: 1,
            is_toggleable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NumberedListItemProps {
    pub background_color: String,
    pub text_color: String,
    pub text_alignment: TextAlignment,
    /// The number the list starts at, on its first item, when it is not 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
}

impl Default for NumberedListItemProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            text_color: default_color(),
            text_alignment: TextAlignment::Left,
            start: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckListItemProps {
    pub background_color: String,
    pub text_color: String,
    pub text_alignment: TextAlignment,
    pub checked: bool,
}

impl Default for CheckListItemProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            text_color: default_color(),
            text_alignment: TextAlignment::Left,
            checked: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuoteProps {
    pub background_color: String,
    pub text_color: String,
}

impl Default for QuoteProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            text_color: default_color(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeBlockProps {
    pub language: String,
}

impl Default for CodeBlockProps {
    fn default() -> Self {
        Self {
            language: DEFAULT_LANGUAGE.to_owned(),
        }
    }
}

/// The props of a block that has none, a divider: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoProps {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TableProps {
    pub text_color: String,
}

impl Default for TableProps {
    fn default() -> Self {
        Self {
            text_color: default_color(),
        }
    }
}

/// The props of an image and of a video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageProps {
    pub text_alignment: TextAlignment,
    pub background_color: String,
    /// The file's name, or an image's alternative text.
    pub name: String,
    pub url: String,
    pub caption: String,
    pub show_preview: bool,
    /// The width it is shown at, in pixels, once someone resized it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_width: Option<f64>,
}

impl Default for ImageProps {
    fn default() -> Self {
        Self {
            text_alignment: TextAlignment::Left,
            background_color: default_color(),
            name: String::new(),
            url: String::new(),
            caption: String::new(),
            show_preview: true,
            preview_width: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioProps {
    pub background_color: String,
    pub name: String,
    pub url: String,
    pub caption: String,
    pub show_preview: bool,
}

impl Default for AudioProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            name: String::new(),
            url: String::new(),
            caption: String::new(),
            show_preview: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileProps {
    pub background_color: String,
    pub name: String,
    pub url: String,
    pub caption: String,
}

impl Default for FileProps {
    fn default() -> Self {
        Self {
            background_color: default_color(),
            name: String::new(),
            url: String::new(),
            caption: String::new(),
        }
    }
}

/// The props of a database block: the database it shows, and the view it shows it in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DatabaseProps {
    /// The database's id; empty for a block not given one yet.
    pub database_id: String,
    /// The id of one of the database's views, when the block names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
}

/// A run of inline content: styled text, a link around styled text, or a mention.
///
/// Line breaks inside a block are `\n` in the text, as in BlockNote's JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Inline {
    Text(StyledText),
    Link(Link),
    Mention(Mention),
}

impl Inline {
    /// The runs in the normal form BlockNote reads content in: neighbouring runs with the
    /// same styles, or links with the same target, merged; no empty runs; code without other
    /// styles or a link; an empty colour as no colour; and each line break `\n` at the end
    /// of the run before it. Documents read from Yjs or markdown are in it already, and
    /// writing a document gives the same result as writing it in normal form.
    pub fn normalize(content: &[Inline]) -> Vec<Inline> {
        crate::inline::to_inline(crate::inline::to_pieces(content))
    }

    /// Unstyled text, or nothing for an empty string.
    pub fn plain(text: impl Into<String>) -> Vec<Inline> {
        let text = text.into();
        if text.is_empty() {
            Vec::new()
        } else {
            vec![Inline::Text(StyledText {
                text,
                styles: Styles::default(),
            })]
        }
    }
}

/// A mention of someone or of a page, the editor's own inline content: an atom, without
/// styles or text of its own, that names what it mentions by id and keeps the name or title
/// it had when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub props: MentionProps,
}

impl Mention {
    pub fn new(kind: MentionKind, target: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            props: MentionProps {
                kind,
                target: target.into(),
                label: label.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MentionProps {
    pub kind: MentionKind,
    /// The id of the account or page.
    pub target: String,
    /// Its name or title when the mention was written.
    pub label: String,
}

/// What a mention names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MentionKind {
    /// A person, an agent or an integration, by their account's id.
    #[default]
    User,
    /// A page, by its id.
    Page,
}

impl MentionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Page => "page",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(Self::User),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyledText {
    pub text: String,
    #[serde(default)]
    pub styles: Styles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub href: String,
    #[serde(with = "link_content")]
    pub content: Vec<StyledText>,
}

/// The styles of a run of text. BlockNote's code style excludes every other style and
/// links: content read from a document never has code together with anything else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Styles {
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub strike: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub code: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A table's rows, which header rows and columns it has, and its columns' widths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "tableContent", rename_all = "camelCase")]
pub struct TableContent {
    /// Each column's width in pixels, once someone resized it.
    #[serde(default)]
    pub column_widths: Vec<Option<f64>>,
    /// How many rows are header rows, counted as BlockNote does: the rows whose cells are
    /// all header cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_rows: Option<u32>,
    /// How many columns are header columns, counted the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_cols: Option<u32>,
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "tableCell")]
pub struct TableCell {
    #[serde(default)]
    pub content: Vec<Inline>,
    #[serde(default)]
    pub props: TableCellProps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TableCellProps {
    pub colspan: u32,
    pub rowspan: u32,
    pub background_color: String,
    pub text_color: String,
    pub text_alignment: TextAlignment,
}

impl Default for TableCellProps {
    fn default() -> Self {
        Self {
            colspan: 1,
            rowspan: 1,
            background_color: default_color(),
            text_color: default_color(),
            text_alignment: TextAlignment::Left,
        }
    }
}

/// A code block's text in BlockNote's JSON: one unstyled text run, or none when empty.
mod plain_content {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Inline, StyledText};

    pub fn serialize<S: Serializer>(code: &str, serializer: S) -> Result<S::Ok, S::Error> {
        Inline::plain(code).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        let runs = Vec::<Inline>::deserialize(deserializer)?;
        let mut code = String::new();
        for run in runs {
            match run {
                Inline::Text(StyledText { text, .. }) => code.push_str(&text),
                Inline::Link(link) => {
                    for text in link.content {
                        code.push_str(&text.text);
                    }
                }
                Inline::Mention(mention) => code.push_str(&mention.props.label),
            }
        }
        Ok(code)
    }
}

/// A link's text in BlockNote's JSON, each run with `"type": "text"`.
mod link_content {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::StyledText;

    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "camelCase")]
    enum Tagged<T> {
        Text(T),
    }

    pub fn serialize<S: Serializer>(runs: &[StyledText], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(runs.iter().map(Tagged::Text))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<StyledText>, D::Error> {
        let runs = Vec::<Tagged<StyledText>>::deserialize(deserializer)?;
        Ok(runs.into_iter().map(|Tagged::Text(run)| run).collect())
    }
}
