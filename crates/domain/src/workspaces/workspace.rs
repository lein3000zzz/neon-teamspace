//! Workspaces and teamspaces as stored, and the rules for their names, slugs and icons.

use time::OffsetDateTime;
use uuid::Uuid;

use super::roles::TeamspaceAccess;
use crate::hue::hue_or_hashed;

/// The longest workspace or teamspace name, in characters.
pub const MAX_NAME: usize = 100;

/// The longest icon, in characters: an emoji, which may take several, or a short text.
pub const MAX_ICON: usize = 16;

/// The longest slug, in characters.
pub const MAX_SLUG: usize = 60;

/// The longest part of a slug taken from a name, leaving room for a suffix.
const SLUG_BASE: usize = 40;

/// The slug of a name with nothing in it to keep, such as one in another script.
const FALLBACK_SLUG: &str = "workspace";

/// Slugs the web app's own top-level paths use, which a workspace's URL must not shadow.
pub const RESERVED_SLUGS: &[&str] = &[
    "account",
    "admin",
    "api",
    "assets",
    "auth",
    "docs",
    "hooks",
    "invite",
    "invites",
    "join",
    "login",
    "logout",
    "mcp",
    "new",
    "register",
    "reset",
    "settings",
    "signup",
    "styleguide",
    "verify",
    "well-known",
    "workspaces",
    "ws",
];

/// A team's space. Accounts belong to the instance; a person can be in many workspaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    /// Lowercase letters, digits and single dashes, unique on the instance, for URLs.
    pub slug: String,
    /// An emoji or a short text.
    pub icon: Option<String>,
    /// The hue it pinned, if any; see [`hue`](Self::hue).
    pub hue_override: Option<u16>,
    /// Whether members, besides owners and admins, may create teamspaces.
    pub members_can_create_teamspaces: bool,
    /// Who created it; unset once their account is gone.
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Workspace {
    /// The hue it is drawn in: the one it pinned, or else its id's.
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }
}

/// A group of a workspace's members and, later, their pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Teamspace {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub icon: Option<String>,
    pub hue_override: Option<u16>,
    pub access: TeamspaceAccess,
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Teamspace {
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }

    pub fn is_default(&self) -> bool {
        self.access == TeamspaceAccess::Default
    }
}

/// A name that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a name has 1 to {MAX_NAME} characters, and no line breaks or other control characters")]
pub struct InvalidName;

/// The name to keep, trimmed: 1 to [`MAX_NAME`] characters, none of them control
/// characters.
pub fn check_name(raw: &str) -> Result<String, InvalidName> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        Err(InvalidName)
    } else {
        Ok(name.to_owned())
    }
}

/// An icon that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "an icon is an emoji or a short text: at most {MAX_ICON} characters, and no control characters"
)]
pub struct InvalidIcon;

/// The icon to keep, trimmed; blank is no icon.
pub fn check_icon(raw: &str) -> Result<Option<String>, InvalidIcon> {
    let icon = raw.trim();
    if icon.is_empty() {
        Ok(None)
    } else if icon.chars().count() > MAX_ICON || icon.chars().any(char::is_control) {
        Err(InvalidIcon)
    } else {
        Ok(Some(icon.to_owned()))
    }
}

/// A slug that cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidSlug {
    #[error(
        "a slug has 1 to {MAX_SLUG} lowercase letters, digits and dashes, starting and ending \
         with a letter or digit, without two dashes in a row"
    )]
    Form,
    #[error("this slug is one of the app's own paths")]
    Reserved,
    #[error("a slug cannot have the form of an id")]
    LooksLikeId,
}

/// Checks a slug someone typed, lowercased first.
pub fn check_slug(raw: &str) -> Result<String, InvalidSlug> {
    let slug = raw.trim().to_ascii_lowercase();
    if !well_formed(&slug) {
        return Err(InvalidSlug::Form);
    }
    if is_reserved(&slug) {
        return Err(InvalidSlug::Reserved);
    }
    if Uuid::try_parse(&slug).is_ok() {
        return Err(InvalidSlug::LooksLikeId);
    }
    Ok(slug)
}

fn well_formed(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_reserved(slug: &str) -> bool {
    RESERVED_SLUGS.contains(&slug)
}

/// The slug a name gives: its ASCII letters and digits in lowercase, every run of anything
/// else a single dash, at most 40 characters. A name with none gives `workspace`.
pub fn slug_base(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if slug.len() == SLUG_BASE {
                break;
            }
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            if slug.len() + 1 >= SLUG_BASE {
                break;
            }
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        FALLBACK_SLUG.to_owned()
    } else {
        slug.to_owned()
    }
}

/// The slug to try after `base` was taken: `base`, a dash and `suffix`.
pub fn suffixed_slug(base: &str, suffix: &str) -> String {
    format!("{base}-{suffix}")
}

/// Whether a slug from [`slug_base`] can be tried as it is, before any suffix: neither one
/// of the app's paths nor in the form of an id.
pub fn usable_as_is(base: &str) -> bool {
    check_slug(base).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(check_name("  Design team "), Ok("Design team".to_owned()));
        assert_eq!(check_name(&"é".repeat(MAX_NAME)), Ok("é".repeat(MAX_NAME)));
        assert_eq!(check_name("   "), Err(InvalidName));
        assert_eq!(check_name(&"x".repeat(MAX_NAME + 1)), Err(InvalidName));
        assert_eq!(check_name("two\nlines"), Err(InvalidName));
    }

    #[test]
    fn icons_are_an_emoji_or_a_short_text() {
        assert_eq!(check_icon(" 🚀 "), Ok(Some("🚀".to_owned())));
        assert_eq!(check_icon("NW"), Ok(Some("NW".to_owned())));
        // A family emoji is seven characters joined into one picture.
        assert_eq!(check_icon("👨‍👩‍👧‍👦"), Ok(Some("👨‍👩‍👧‍👦".to_owned())));
        assert_eq!(check_icon(""), Ok(None));
        assert_eq!(check_icon("   "), Ok(None));
        assert_eq!(check_icon(&"x".repeat(MAX_ICON + 1)), Err(InvalidIcon));
        assert_eq!(check_icon("a\tb"), Err(InvalidIcon));
    }

    #[test]
    fn slugs_come_from_the_name() {
        assert_eq!(slug_base("Design Team"), "design-team");
        assert_eq!(
            slug_base("  Neon -- Workspace!! 2026 "),
            "neon-workspace-2026"
        );
        assert_eq!(slug_base("Équipe café"), "quipe-caf");
        assert_eq!(slug_base("日本語"), "workspace");
        assert_eq!(slug_base("---"), "workspace");
        let long = slug_base(&"abc ".repeat(30));
        assert!(long.len() <= 40, "{long}");
        assert!(!long.ends_with('-'), "{long}");
        assert_eq!(check_slug(&long), Ok(long.clone()));
        assert_eq!(slug_base(&"a".repeat(50)), "a".repeat(40));
    }

    #[test]
    fn a_taken_slug_gets_a_suffix() {
        assert_eq!(suffixed_slug("design", "k3x9qa"), "design-k3x9qa");
        assert!(check_slug(&suffixed_slug(&slug_base(&"a".repeat(80)), "k3x9qa")).is_ok());
    }

    #[test]
    fn typed_slugs_are_checked() {
        assert_eq!(check_slug(" Design-Team "), Ok("design-team".to_owned()));
        assert_eq!(check_slug("a"), Ok("a".to_owned()));
        assert_eq!(check_slug(&"a".repeat(MAX_SLUG)), Ok("a".repeat(MAX_SLUG)));
        for bad in [
            "",
            "-a",
            "a-",
            "a--b",
            "a_b",
            "a b",
            "ü",
            &"a".repeat(MAX_SLUG + 1),
        ] {
            assert_eq!(check_slug(bad), Err(InvalidSlug::Form), "{bad:?}");
        }
        assert_eq!(check_slug("Admin"), Err(InvalidSlug::Reserved));
        assert_eq!(
            check_slug("018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f"),
            Err(InvalidSlug::LooksLikeId)
        );
    }

    #[test]
    fn reserved_and_id_like_bases_need_a_suffix() {
        assert!(usable_as_is("design"));
        assert!(!usable_as_is("settings"));
        assert!(!usable_as_is("00000000-0000-0000-0000-000000000000"));
        let sorted = {
            let mut sorted = RESERVED_SLUGS.to_vec();
            sorted.sort_unstable();
            sorted
        };
        assert_eq!(
            sorted, RESERVED_SLUGS,
            "kept in order, so a duplicate shows"
        );
        assert!(RESERVED_SLUGS.iter().all(|slug| well_formed(slug)));
    }

    #[test]
    fn hues_are_pinned_or_hashed() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let id = Uuid::parse_str("f47ac10b-58cc-4372-a567-0e02b2c3d479").expect("valid");
        let mut teamspace = Teamspace {
            id,
            workspace_id: Uuid::nil(),
            name: "Design".into(),
            icon: None,
            hue_override: None,
            access: TeamspaceAccess::Open,
            created_by: None,
            created_at: now,
            updated_at: now,
        };

        assert_eq!(teamspace.hue(), 98);
        assert!(!teamspace.is_default());
        teamspace.hue_override = Some(12);
        assert_eq!(teamspace.hue(), 12);

        let workspace = Workspace {
            id,
            name: "Neon".into(),
            slug: "neon".into(),
            icon: None,
            hue_override: None,
            members_can_create_teamspaces: true,
            created_by: None,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(workspace.hue(), 98);
    }
}
