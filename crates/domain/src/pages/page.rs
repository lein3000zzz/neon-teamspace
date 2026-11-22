//! A page as stored, its style and cover, where it is, and the rules for its title.

use time::OffsetDateTime;
use uuid::Uuid;

/// The longest title, in characters.
pub const MAX_TITLE: usize = 500;

/// How deep pages nest: a page at the top of its space is at depth 1, its subpages at 2.
/// It keeps the walks up and down a tree short.
pub const MAX_DEPTH: usize = 100;

text_enum! {
    /// The font a page is set in.
    Font {
        /// The system's sans-serif, as the rest of the app.
        Default => "default",
        /// The system's serif.
        Serif => "serif",
        /// A monospaced font, bundled with the app.
        Mono => "mono",
    }
}

text_enum! {
    /// A cover drawn from the theme's tokens: a hue, or a gradient between two, at the
    /// lightness and chroma the reader's palette and mode set, like the accent. Image covers
    /// are not built yet.
    Cover {
        /// The reader's accent.
        Accent => "accent",
        /// From the reader's accent to the hue opposite it.
        AccentGradient => "accent-gradient",
        /// Hue 25.
        Red => "red",
        /// Hue 55.
        Orange => "orange",
        /// Hue 90.
        Yellow => "yellow",
        /// Hue 145.
        Green => "green",
        /// Hue 185.
        Teal => "teal",
        /// Hue 250.
        Blue => "blue",
        /// Hue 295.
        Violet => "violet",
        /// Hue 345.
        Pink => "pink",
        /// From hue 55 to hue 345.
        Sunrise => "sunrise",
        /// From hue 185 to hue 250.
        Lagoon => "lagoon",
        /// From hue 145 to hue 295.
        Aurora => "aurora",
        /// From hue 250 to hue 345.
        Dusk => "dusk",
    }
}

text_enum! {
    /// What someone may do with a page: what a share gives, and what the rules work out
    /// for a person. Edit includes view.
    PageAccess {
        /// Read it, its subpages and its place in the tree.
        View => "view",
        /// Also change it, add subpages, move it and trash it.
        Edit => "edit",
    }
}

/// How a page is set: its font, and whether it uses small text or the full width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageStyle {
    pub font: Font,
    pub small_text: bool,
    pub full_width: bool,
}

impl Default for PageStyle {
    fn default() -> Self {
        Self {
            font: Font::Default,
            small_text: false,
            full_width: false,
        }
    }
}

/// Where a page is: a teamspace, or the private pages of one member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Space {
    Teamspace(Uuid),
    Private { owner: Uuid },
}

impl Space {
    /// The id that names the space: the teamspace's, or the owner's.
    pub fn id(self) -> Uuid {
        match self {
            Self::Teamspace(id) | Self::Private { owner: id } => id,
        }
    }

    pub fn teamspace_id(self) -> Option<Uuid> {
        match self {
            Self::Teamspace(id) => Some(id),
            Self::Private { .. } => None,
        }
    }

    pub fn owner_id(self) -> Option<Uuid> {
        match self {
            Self::Private { owner } => Some(owner),
            Self::Teamspace(_) => None,
        }
    }
}

/// Where a page goes among its new siblings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Placement {
    First,
    /// After every sibling, as a new page goes unless told otherwise.
    #[default]
    Last,
    /// Right after the sibling with this id.
    After(Uuid),
    /// Right before the sibling with this id.
    Before(Uuid),
}

/// A page of a workspace, without its content, which is a document of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The teamspace it is in, or the member it is private to.
    pub space: Space,
    /// The page it is under, in the same space; unset at the top of its space.
    pub parent_id: Option<Uuid>,
    /// Its key among its siblings; see [`position`](super::position).
    pub position: String,
    /// Up to [`MAX_TITLE`] characters. Empty, it shows as Untitled.
    pub title: String,
    pub icon: Option<String>,
    pub cover: Option<Cover>,
    pub style: PageStyle,
    pub created_by: Option<Uuid>,
    /// Who changed it last, its content included.
    pub last_edited_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    /// When its title, icon, cover, style or place last changed.
    pub updated_at: OffsetDateTime,
    /// When its content last changed.
    pub content_updated_at: OffsetDateTime,
    /// Set while it is in the trash.
    pub trashed_at: Option<OffsetDateTime>,
    pub trashed_by: Option<Uuid>,
    /// While it is in the trash, the page that was trashed and took it along: itself, or
    /// the page above it that was trashed with its subpages. Restoring that page brings
    /// this one back with it.
    pub trashed_with: Option<Uuid>,
}

impl Page {
    pub fn is_trashed(&self) -> bool {
        self.trashed_at.is_some()
    }
}

/// A page shared with a member or guest of its workspace, for it and every page under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub page_id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub access: PageAccess,
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A title that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "a title has at most {MAX_TITLE} characters, and no line breaks or other control characters"
)]
pub struct InvalidTitle;

/// The title to keep, trimmed: up to [`MAX_TITLE`] characters, none of them control
/// characters. Empty is allowed.
pub fn check_title(raw: &str) -> Result<String, InvalidTitle> {
    let title = raw.trim();
    if title.chars().count() > MAX_TITLE || title.chars().any(char::is_control) {
        Err(InvalidTitle)
    } else {
        Ok(title.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_trimmed_bounded_and_may_be_empty() {
        assert_eq!(check_title("  Roadmap "), Ok("Roadmap".to_owned()));
        assert_eq!(check_title(""), Ok(String::new()));
        assert_eq!(check_title("   "), Ok(String::new()));
        assert_eq!(
            check_title(&"é".repeat(MAX_TITLE)),
            Ok("é".repeat(MAX_TITLE))
        );
        assert_eq!(check_title(&"x".repeat(MAX_TITLE + 1)), Err(InvalidTitle));
        assert_eq!(check_title("two\nlines"), Err(InvalidTitle));
        assert_eq!(check_title("a\tb"), Err(InvalidTitle));
    }

    #[test]
    fn every_value_round_trips_through_its_text() {
        for font in Font::ALL {
            assert_eq!(Font::parse(font.as_str()), Some(*font));
        }
        for cover in Cover::ALL {
            assert_eq!(Cover::parse(&cover.to_string()), Some(*cover));
        }
        for access in PageAccess::ALL {
            assert_eq!(PageAccess::parse(access.as_str()), Some(*access));
        }
        assert_eq!(Cover::ALL.len(), 14);
        assert_eq!(Font::parse("Serif"), None);
        assert_eq!(Cover::parse("https://example.com/cover.png"), None);
        assert_eq!(PageAccess::parse("full"), None);
    }

    #[test]
    fn edit_includes_view() {
        assert!(PageAccess::Edit > PageAccess::View);
        assert_eq!(
            Some(PageAccess::View).max(None),
            Some(PageAccess::View),
            "any access beats none"
        );
        assert_eq!(
            PageStyle::default(),
            PageStyle {
                font: Font::Default,
                small_text: false,
                full_width: false
            }
        );
    }

    #[test]
    fn a_page_is_in_a_teamspace_or_private() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let teamspace = Uuid::from_u128(1);
        let owner = Uuid::from_u128(2);
        let mut page = Page {
            id: Uuid::from_u128(3),
            workspace_id: Uuid::from_u128(4),
            space: Space::Teamspace(teamspace),
            parent_id: None,
            position: "a0".to_owned(),
            title: String::new(),
            icon: None,
            cover: None,
            style: PageStyle::default(),
            created_by: None,
            last_edited_by: None,
            created_at: now,
            updated_at: now,
            content_updated_at: now,
            trashed_at: None,
            trashed_by: None,
            trashed_with: None,
        };

        assert_eq!(page.space.id(), teamspace);
        assert_eq!(page.space.teamspace_id(), Some(teamspace));
        assert_eq!(page.space.owner_id(), None);
        assert!(!page.is_trashed());
        page.space = Space::Private { owner };
        page.trashed_at = Some(now);
        assert_eq!(page.space.id(), owner);
        assert_eq!(page.space.teamspace_id(), None);
        assert_eq!(page.space.owner_id(), Some(owner));
        assert!(page.is_trashed());
        assert_eq!(Placement::default(), Placement::Last);
    }
}
