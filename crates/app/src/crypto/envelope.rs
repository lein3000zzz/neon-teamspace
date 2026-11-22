use std::fmt;

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use zeroize::Zeroizing;

/// Bytes in a master key and in each data key: AES-256.
pub const KEY_BYTES: usize = 32;

/// The longest key id.
const MAX_KEY_ID: usize = 64;

const FORMAT_VERSION: u8 = 1;
const NONCE_BYTES: usize = 12;

/// A `secrets.keys` entry that cannot be used. The messages name entries by position and
/// keys by id, never by their material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error(
        "entry {0}: want id:base64key, the id 1 to 64 letters, digits, dots, dashes or \
         underscores"
    )]
    Format(usize),
    #[error(
        "key {0:?}: want {KEY_BYTES} bytes in standard base64, such as `openssl rand -base64 32` \
         prints"
    )]
    Material(String),
    #[error("key id {0:?} is listed twice")]
    Duplicate(String),
}

/// The master keys, from `secrets.keys`. The first one seals new secrets; every one opens
/// the secrets it sealed, so keys rotate by putting a new one first and keeping the old
/// ones until nothing sealed with them is left. `Debug` shows only the ids.
#[derive(Clone)]
pub struct Keyring {
    /// The current key first.
    keys: Vec<(String, Zeroizing<[u8; KEY_BYTES]>)>,
}

impl Keyring {
    /// Reads entries written `id:base64key`, the first being the current key. `None` when
    /// there are none.
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Option<Self>, KeyError> {
        let mut keys: Vec<(String, Zeroizing<[u8; KEY_BYTES]>)> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let position = index + 1;
            let (id, encoded) = entry
                .as_ref()
                .trim()
                .split_once(':')
                .ok_or(KeyError::Format(position))?;
            if !valid_id(id) {
                return Err(KeyError::Format(position));
            }
            let decoded = Zeroizing::new(
                STANDARD
                    .decode(encoded.trim())
                    .map_err(|_| KeyError::Material(id.to_owned()))?,
            );
            let mut key = Zeroizing::new([0u8; KEY_BYTES]);
            if decoded.len() != KEY_BYTES {
                return Err(KeyError::Material(id.to_owned()));
            }
            key.copy_from_slice(&decoded);
            if keys.iter().any(|(known, _)| known == id) {
                return Err(KeyError::Duplicate(id.to_owned()));
            }
            keys.push((id.to_owned(), key));
        }
        Ok((!keys.is_empty()).then_some(Self { keys }))
    }

    /// The id of the key that seals new secrets.
    pub fn current_id(&self) -> &str {
        &self.keys[0].0
    }

    /// Every id, the current one first.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().map(|(id, _)| id.as_str())
    }

    fn current(&self) -> (&str, &[u8; KEY_BYTES]) {
        let (id, key) = &self.keys[0];
        (id, key)
    }

    fn get(&self, id: &str) -> Option<&[u8; KEY_BYTES]> {
        self.keys
            .iter()
            .find_map(|(known, key)| (known == id).then_some(&**key))
    }
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field("ids", &self.ids().collect::<Vec<_>>())
            .finish()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_KEY_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    #[error("no secret key is configured: set secrets.keys (NEON_SECRETS__KEYS) first")]
    NoKey,
    #[error("encrypting failed")]
    Encrypt,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    #[error("no secret key is configured: set secrets.keys (NEON_SECRETS__KEYS)")]
    NoKey,
    #[error("it was sealed with key {0:?}, which secrets.keys no longer lists")]
    UnknownKey(String),
    #[error("it is not a sealed secret")]
    Malformed,
    #[error("it does not decrypt: it was changed, or belongs elsewhere")]
    Decrypt,
}

/// Envelope encryption of secrets kept in the database.
///
/// Each secret gets its own random AES-256-GCM data key, which the current master key
/// wraps. The sealed form carries the master key's id, so it opens after the keys rotate:
///
/// ```text
/// version(1) | id length(1) | key id | wrapped length(2) | wrapped key | nonce(12) | ciphertext
/// ```
///
/// The associated data binds a value to where it belongs, such as `invite:<id>` or
/// `setting:<key>`: a sealed value copied to another row does not open there.
#[derive(Debug, Clone, Default)]
pub struct Envelope {
    keys: Option<Keyring>,
}

impl Envelope {
    /// Without keys, nothing can be sealed or opened.
    pub fn new(keys: Option<Keyring>) -> Self {
        Self { keys }
    }

    pub fn has_key(&self) -> bool {
        self.keys.is_some()
    }

    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, SealError> {
        let keys = self.keys.as_ref().ok_or(SealError::NoKey)?;
        let (id, master) = keys.current();

        let mut data_key = Zeroizing::new([0u8; KEY_BYTES]);
        rand::fill(&mut data_key[..]);
        let wrapped = gcm_seal(master, &data_key[..], id.as_bytes())?;
        let sealed = gcm_seal(&data_key, plaintext, aad)?;

        let wrapped_len = u16::try_from(wrapped.len()).map_err(|_| SealError::Encrypt)?;
        let id_len = u8::try_from(id.len()).map_err(|_| SealError::Encrypt)?;
        let mut out = Vec::with_capacity(4 + id.len() + wrapped.len() + sealed.len());
        out.push(FORMAT_VERSION);
        out.push(id_len);
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&wrapped_len.to_be_bytes());
        out.extend_from_slice(&wrapped);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>, OpenError> {
        let keys = self.keys.as_ref().ok_or(OpenError::NoKey)?;
        let parts = Parts::read(sealed).ok_or(OpenError::Malformed)?;
        let master = keys
            .get(parts.key_id)
            .ok_or_else(|| OpenError::UnknownKey(parts.key_id.to_owned()))?;

        let data_key = Zeroizing::new(gcm_open(master, parts.wrapped, parts.key_id.as_bytes())?);
        let data_key: &[u8; KEY_BYTES] = data_key
            .as_slice()
            .try_into()
            .map_err(|_| OpenError::Malformed)?;
        gcm_open(data_key, parts.sealed, aad).map(Zeroizing::new)
    }
}

/// The fields of a sealed value.
struct Parts<'a> {
    key_id: &'a str,
    wrapped: &'a [u8],
    /// The nonce, then the ciphertext with its tag.
    sealed: &'a [u8],
}

impl<'a> Parts<'a> {
    fn read(mut bytes: &'a [u8]) -> Option<Self> {
        let mut take = |n: usize| -> Option<&'a [u8]> {
            let (head, rest) = bytes.split_at_checked(n)?;
            bytes = rest;
            Some(head)
        };
        let head = take(2)?;
        if head[0] != FORMAT_VERSION {
            return None;
        }
        let key_id = std::str::from_utf8(take(usize::from(head[1]))?).ok()?;
        let wrapped_len = take(2)?;
        let wrapped = take(usize::from(u16::from_be_bytes([
            wrapped_len[0],
            wrapped_len[1],
        ])))?;
        // A nonce and at least a tag.
        (bytes.len() > NONCE_BYTES).then_some(Self {
            key_id,
            wrapped,
            sealed: bytes,
        })
    }
}

/// A random nonce, then the ciphertext.
fn gcm_seal(key: &[u8; KEY_BYTES], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, SealError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| SealError::Encrypt)?;
    let mut nonce = [0u8; NONCE_BYTES];
    rand::fill(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| SealError::Encrypt)?;
    let mut out = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn gcm_open(key: &[u8; KEY_BYTES], sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, OpenError> {
    let (nonce, ciphertext) = sealed
        .split_at_checked(NONCE_BYTES)
        .ok_or(OpenError::Malformed)?;
    let nonce: [u8; NONCE_BYTES] = nonce.try_into().map_err(|_| OpenError::Malformed)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| OpenError::Decrypt)?;
    cipher
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| OpenError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> String {
        STANDARD.encode([byte; KEY_BYTES])
    }

    fn keyring(entries: &[String]) -> Keyring {
        Keyring::parse(entries)
            .expect("valid keys")
            .expect("at least one key")
    }

    fn envelope(entries: &[String]) -> Envelope {
        Envelope::new(Some(keyring(entries)))
    }

    #[test]
    fn sealed_values_open_with_the_same_associated_data() {
        let envelope = envelope(&[format!("k1:{}", key(1))]);

        let sealed = envelope.seal(b"inv_code", b"invite:1").expect("seals");

        assert!(!sealed.windows(8).any(|w| w == b"inv_code"));
        assert_eq!(
            envelope
                .open(&sealed, b"invite:1")
                .expect("opens")
                .as_slice(),
            b"inv_code"
        );
        assert_eq!(envelope.open(&sealed, b"invite:2"), Err(OpenError::Decrypt));
        assert_ne!(
            sealed,
            envelope.seal(b"inv_code", b"invite:1").expect("seals")
        );
    }

    #[test]
    fn old_keys_open_what_they_sealed_after_a_rotation() {
        let old = envelope(&[format!("2025:{}", key(1))]);
        let rotated = envelope(&[format!("2026:{}", key(2)), format!("2025:{}", key(1))]);
        let forgotten = envelope(&[format!("2026:{}", key(2))]);

        let sealed = old
            .seal(b"secret", b"setting:mail.password")
            .expect("seals");

        assert_eq!(
            rotated
                .open(&sealed, b"setting:mail.password")
                .expect("opens")
                .as_slice(),
            b"secret"
        );
        assert_eq!(
            forgotten.open(&sealed, b"setting:mail.password"),
            Err(OpenError::UnknownKey("2025".into()))
        );
        let resealed = rotated.seal(b"secret", b"a").expect("seals");
        assert_eq!(&resealed[2..6], b"2026");
    }

    #[test]
    fn a_key_with_the_same_id_but_other_material_does_not_open() {
        let sealed = envelope(&[format!("k1:{}", key(1))])
            .seal(b"secret", b"aad")
            .expect("seals");

        assert_eq!(
            envelope(&[format!("k1:{}", key(9))]).open(&sealed, b"aad"),
            Err(OpenError::Decrypt)
        );
    }

    #[test]
    fn tampered_and_truncated_values_do_not_open() {
        let envelope = envelope(&[format!("k1:{}", key(1))]);
        let sealed = envelope.seal(b"secret", b"aad").expect("seals");

        let mut flipped = sealed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert_eq!(envelope.open(&flipped, b"aad"), Err(OpenError::Decrypt));
        for len in [0, 1, 4, 10, sealed.len() - 17] {
            assert!(envelope.open(&sealed[..len], b"aad").is_err(), "{len}");
        }
        let mut version = sealed;
        version[0] = 2;
        assert_eq!(envelope.open(&version, b"aad"), Err(OpenError::Malformed));
    }

    #[test]
    fn without_keys_nothing_seals_or_opens() {
        let envelope = Envelope::default();

        assert!(!envelope.has_key());
        assert_eq!(envelope.seal(b"x", b"aad"), Err(SealError::NoKey));
        assert_eq!(envelope.open(b"x", b"aad"), Err(OpenError::NoKey));
        assert!(
            Keyring::parse::<&str>(&[])
                .expect("empty is fine")
                .is_none()
        );
    }

    #[test]
    fn malformed_entries_are_named_without_their_material() {
        let short = STANDARD.encode([7u8; 16]);
        let cases = [
            (vec!["no-colon".to_owned()], KeyError::Format(1)),
            (
                vec![format!("k1:{}", key(1)), format!(":{}", key(2))],
                KeyError::Format(2),
            ),
            (vec![format!("bad id:{}", key(1))], KeyError::Format(1)),
            (vec![format!("k1:{short}")], KeyError::Material("k1".into())),
            (
                vec!["k1:not base64!".to_owned()],
                KeyError::Material("k1".into()),
            ),
            (
                vec![format!("k1:{}", key(1)), format!("k1:{}", key(2))],
                KeyError::Duplicate("k1".into()),
            ),
        ];

        for (entries, want) in cases {
            let error = Keyring::parse(&entries).expect_err("malformed");
            assert_eq!(error, want);
            assert!(!error.to_string().contains(&short), "{error}");
        }
    }

    #[test]
    fn debug_output_shows_only_the_ids() {
        let material = key(3);
        let keyring = keyring(&[format!(" k1:{material} "), format!("k0:{}", key(4))]);

        let shown = format!("{keyring:?}");

        assert_eq!(shown, r#"Keyring { ids: ["k1", "k0"] }"#);
        assert_eq!(keyring.current_id(), "k1");
    }
}
