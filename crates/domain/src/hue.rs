//! Hashed hues: the colour an entity is drawn in, worked out from its id, so the server and
//! every client agree on it without storing it.
//!
//! This is the web app's `hueFor` in `web/src/lib/hue.ts`, byte for byte:
//!
//! 1. A canonical UUID, 8-4-4-4-12 hex digits in any case, is hashed as its 16 raw bytes.
//!    The whole id is hashed: UUIDv7 ids made in the same millisecond share their leading
//!    bytes. Any other text is hashed as UTF-8, so every id has a hue.
//! 2. The hash is 32-bit FNV-1a, offset basis `0x811c9dc5`, prime `0x01000193`.
//! 3. The hue is the hash modulo 360.
//!
//! An entity may also pin a hue of its own, which then wins over the hashed one.

use uuid::Uuid;

/// The largest hue, in degrees; 360 would be 0 again.
pub const MAX_HUE: u16 = 359;

const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;
const HUE_RANGE: u32 = 360;

/// 32-bit FNV-1a: each byte is XORed into the hash, which is then multiplied by the prime,
/// modulo 2^32.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

/// The hue of an id given as text: a canonical UUID's raw bytes, or else the text's UTF-8.
pub fn hue_for(id: &str) -> u16 {
    match canonical_uuid(id) {
        Some(uuid) => hue_of_bytes(uuid.as_bytes()),
        None => hue_of_bytes(id.as_bytes()),
    }
}

/// The hue of a UUID, the same as [`hue_for`] of its text.
pub fn hue_for_uuid(id: Uuid) -> u16 {
    hue_of_bytes(id.as_bytes())
}

/// The hue an entity is drawn in: the one it pinned, or else its id's.
pub fn hue_or_hashed(pinned: Option<u16>, id: Uuid) -> u16 {
    pinned.unwrap_or_else(|| hue_for_uuid(id))
}

/// A pinned hue past [`MAX_HUE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a hue is a whole number of degrees from 0 to {MAX_HUE}")]
pub struct InvalidHue;

/// Checks a hue to pin: 0 to [`MAX_HUE`].
pub fn check_hue(hue: i64) -> Result<u16, InvalidHue> {
    u16::try_from(hue)
        .ok()
        .filter(|hue| *hue <= MAX_HUE)
        .ok_or(InvalidHue)
}

fn hue_of_bytes(bytes: &[u8]) -> u16 {
    // Below 360, so it fits.
    u16::try_from(fnv1a32(bytes) % HUE_RANGE).unwrap_or_default()
}

/// The UUID of text in exactly the canonical form, the only form the web app reads as one.
/// `Uuid::parse_str` also takes the simple, braced and URN forms, which hash as text there.
fn canonical_uuid(text: &str) -> Option<Uuid> {
    let bytes = text.as_bytes();
    let canonical = bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, byte)| match i {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    canonical.then(|| Uuid::parse_str(text).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use serde_json::Value as Json;

    use super::*;

    /// The vectors every implementation of the hash shares, kept with the web app's.
    const VECTORS: &str = include_str!("../../../web/src/lib/hue-vectors.json");

    #[test]
    fn every_shared_vector_hashes_as_the_web_app_does() {
        let vectors: Json = serde_json::from_str(VECTORS).expect("the vectors are JSON");
        let vectors = vectors["vectors"].as_array().expect("a list of vectors");
        assert!(vectors.len() >= 10, "too few vectors: {}", vectors.len());

        for vector in vectors {
            let id = vector["id"].as_str().expect("an id");
            let kind = vector["kind"].as_str().expect("a kind");
            let hash = vector["fnv1a32"].as_u64().expect("a hash");
            let hue = vector["hue"].as_u64().expect("a hue");

            let bytes = match kind {
                "uuid" => {
                    let uuid = canonical_uuid(id).expect("a canonical UUID");
                    assert_eq!(u64::from(hue_for_uuid(uuid)), hue, "{id}");
                    uuid.as_bytes().to_vec()
                }
                "text" => {
                    assert_eq!(canonical_uuid(id), None, "{id}");
                    id.as_bytes().to_vec()
                }
                other => panic!("unknown vector kind {other:?}"),
            };
            assert_eq!(u64::from(fnv1a32(&bytes)), hash, "{id}");
            assert_eq!(u64::from(hue_for(id)), hue, "{id}");
        }
    }

    #[test]
    fn only_the_canonical_form_reads_as_a_uuid() {
        let id = "018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f";
        let uuid = Uuid::parse_str(id).expect("valid");

        assert_eq!(canonical_uuid(id), Some(uuid));
        assert_eq!(canonical_uuid(&id.to_uppercase()), Some(uuid));
        for other in [
            "018f3c1e7b2a7c3d9e4f5a6b7c8d9e0f",
            "{018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f}",
            "urn:uuid:018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f",
            "018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0g",
            "018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f ",
            "018f3c1e+7b2a-7c3d-9e4f-5a6b7c8d9e0f",
        ] {
            assert_eq!(canonical_uuid(other), None, "{other}");
            assert_eq!(hue_for(other), hue_of_bytes(other.as_bytes()), "{other}");
        }
    }

    #[test]
    fn a_pinned_hue_wins_over_the_hashed_one() {
        let id = Uuid::parse_str("00000000-0000-0000-0000-000000000000").expect("valid");

        assert_eq!(hue_or_hashed(None, id), 45);
        assert_eq!(hue_or_hashed(Some(200), id), 200);
        assert_eq!(hue_or_hashed(Some(0), id), 0);
    }

    #[test]
    fn pinned_hues_stop_at_359() {
        assert_eq!(check_hue(0), Ok(0));
        assert_eq!(check_hue(359), Ok(359));
        assert_eq!(check_hue(360), Err(InvalidHue));
        assert_eq!(check_hue(-1), Err(InvalidHue));
        assert_eq!(check_hue(i64::from(u16::MAX) + 1), Err(InvalidHue));
    }

    #[test]
    fn hues_stay_below_360() {
        for n in 0..2000u32 {
            assert!(hue_for(&n.to_string()) <= MAX_HUE);
        }
    }
}
