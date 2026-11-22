//! A user's look of the web app, kept on the server so it follows them between browsers.
//! The fields and their options are the web app's theme preferences.

use serde_json::{Map, Value as Json, json};

/// The largest accent hue, in degrees; 360 would be 0 again.
pub const HUE_MAX: u16 = 359;

macro_rules! options {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            pub fn parse(text: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|option| option.as_str() == text)
            }
        }
    };
}

options! {
    /// The colour set.
    Palette { Nothing => "nothing", Soft => "soft", Warm => "warm" }
}

options! {
    /// Light or dark, or whichever the system prefers.
    Mode { System => "system", Light => "light", Dark => "dark" }
}

options! {
    /// The interface's typeface.
    UiFont { Default => "default", Mono => "mono" }
}

/// An accent hue past [`HUE_MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the accent hue is a whole number of degrees from 0 to {HUE_MAX}")]
pub struct InvalidHue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferences {
    pub palette: Palette,
    pub mode: Mode,
    accent_hue: u16,
    pub ui_font: UiFont,
}

impl Default for Preferences {
    /// The web app's defaults: the Nothing palette, the system's mode, a blue accent.
    fn default() -> Self {
        Self {
            palette: Palette::Nothing,
            mode: Mode::System,
            accent_hue: 250,
            ui_font: UiFont::Default,
        }
    }
}

impl Preferences {
    pub fn new(
        palette: Palette,
        mode: Mode,
        accent_hue: u16,
        ui_font: UiFont,
    ) -> Result<Self, InvalidHue> {
        if accent_hue > HUE_MAX {
            return Err(InvalidHue);
        }
        Ok(Self {
            palette,
            mode,
            accent_hue,
            ui_font,
        })
    }

    pub fn accent_hue(&self) -> u16 {
        self.accent_hue
    }

    /// Reads preferences as stored. Each field is checked on its own, so one that is
    /// missing or no longer valid falls back to its default without losing the others.
    pub fn from_stored(stored: &Json) -> Self {
        let defaults = Self::default();
        let empty = Map::new();
        let fields = stored.as_object().unwrap_or(&empty);
        let text = |key: &str| fields.get(key).and_then(Json::as_str);
        Self {
            palette: text("palette")
                .and_then(Palette::parse)
                .unwrap_or(defaults.palette),
            mode: text("mode").and_then(Mode::parse).unwrap_or(defaults.mode),
            accent_hue: fields
                .get("accent_hue")
                .and_then(Json::as_u64)
                .and_then(|hue| u16::try_from(hue).ok())
                .filter(|hue| *hue <= HUE_MAX)
                .unwrap_or(defaults.accent_hue),
            ui_font: text("ui_font")
                .and_then(UiFont::parse)
                .unwrap_or(defaults.ui_font),
        }
    }

    /// The form [`from_stored`](Self::from_stored) reads.
    pub fn to_stored(&self) -> Json {
        json!({
            "palette": self.palette.as_str(),
            "mode": self.mode.as_str(),
            "accent_hue": self.accent_hue,
            "ui_font": self.ui_font.as_str(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_preferences_round_trip() {
        let preferences =
            Preferences::new(Palette::Warm, Mode::Dark, 25, UiFont::Mono).expect("valid");

        assert_eq!(
            Preferences::from_stored(&preferences.to_stored()),
            preferences
        );
    }

    #[test]
    fn each_bad_field_falls_back_on_its_own() {
        let stored = json!({
            "palette": "soft",
            "mode": "sepia",
            "accent_hue": 360,
            "ui_font": 3,
        });

        let preferences = Preferences::from_stored(&stored);

        assert_eq!(preferences.palette, Palette::Soft);
        assert_eq!(preferences.mode, Mode::System);
        assert_eq!(preferences.accent_hue(), 250);
        assert_eq!(preferences.ui_font, UiFont::Default);
        assert_eq!(
            Preferences::from_stored(&json!([1, 2])),
            Preferences::default()
        );
        assert_eq!(
            Preferences::from_stored(&json!({"accent_hue": -1})).accent_hue(),
            250
        );
        assert_eq!(
            Preferences::from_stored(&json!({"accent_hue": 12.5})).accent_hue(),
            250
        );
    }

    #[test]
    fn hues_stop_at_359() {
        assert!(Preferences::new(Palette::Nothing, Mode::Light, HUE_MAX, UiFont::Default).is_ok());
        assert_eq!(
            Preferences::new(Palette::Nothing, Mode::Light, 360, UiFont::Default),
            Err(InvalidHue)
        );
    }

    #[test]
    fn options_parse_their_text() {
        for palette in Palette::ALL {
            assert_eq!(Palette::parse(palette.as_str()), Some(*palette));
        }
        assert_eq!(Mode::parse("Dark"), None);
        assert_eq!(UiFont::parse("mono"), Some(UiFont::Mono));
    }
}
