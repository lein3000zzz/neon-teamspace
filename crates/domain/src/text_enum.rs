//! Enums kept and sent as fixed texts, such as roles and a page's font.

/// An enum whose values each have a text, as the database keeps them and the API sends
/// them, with the lists of its values and of their texts, each value's text, parsing,
/// `Display`, and serde as that text. Values compare in the order they are listed.
macro_rules! text_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$vdoc])* $variant),+
        }

        impl $name {
            /// Every value, as stored and sent.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Every value's text, in the order of [`ALL`](Self::ALL).
            pub const TEXTS: &'static [&'static str] = &[$($text),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            pub fn parse(text: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|value| value.as_str() == text)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
                Self::parse(&text)
                    .ok_or_else(|| serde::de::Error::unknown_variant(&text, Self::TEXTS))
            }
        }
    };
}
