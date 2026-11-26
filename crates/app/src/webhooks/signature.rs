//! How webhooks are signed, both ways: HMAC-SHA256 over the timestamp and the body.
//!
//! A signed request carries two headers:
//!
//! - `Neon-Webhook-Timestamp`: when it was signed, in seconds since 1970.
//! - `Neon-Webhook-Signature`: `v1=` and the hex HMAC-SHA256, keyed with the secret's text as
//!   it is shown, of the timestamp, a dot, and the body's bytes as sent. Several, separated
//!   by commas, while a rotated secret still holds: one per secret.
//!
//! A receiver checks that the timestamp is within [`REPLAY_WINDOW`] of its clock, computes
//! the signature with its secret, and compares it, in constant time, with each `v1=` of the
//! header: one that matches is enough. A request replayed later fails the timestamp; one
//! whose timestamp was changed fails the signature.

use std::time::Duration;

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use time::OffsetDateTime;

/// The header with the signatures.
pub const SIGNATURE_HEADER: &str = "Neon-Webhook-Signature";
/// The header with the moment of signing, in seconds since 1970.
pub const TIMESTAMP_HEADER: &str = "Neon-Webhook-Timestamp";
/// The version of the signing scheme, before each signature.
pub const SCHEME: &str = "v1";
/// How far a signed timestamp may be from the receiver's clock, either way.
pub const REPLAY_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Starts every signing secret the server makes.
pub const SECRET_PREFIX: &str = "whsec_";
/// The most signatures a header may carry that is checked.
const MAX_SIGNATURES: usize = 8;

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &[u8], timestamp: i64, body: &[u8]) -> HmacSha256 {
    // HMAC takes a key of any length.
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC keys of any length are valid"));
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac
}

/// The signature of `body` signed at `timestamp`: `v1=` and 64 hex digits.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let tag = mac(secret, timestamp, body).finalize().into_bytes();
    let mut out = String::with_capacity(SCHEME.len() + 1 + tag.len() * 2);
    out.push_str(SCHEME);
    out.push('=');
    for byte in tag {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    out
}

/// The `Neon-Webhook-Signature` value for `body` with every one of `secrets`: the current
/// one first, then a rotated one that still holds.
pub fn header<S: AsRef<[u8]>>(secrets: &[S], timestamp: i64, body: &[u8]) -> String {
    secrets
        .iter()
        .map(|secret| sign(secret.as_ref(), timestamp, body))
        .collect::<Vec<_>>()
        .join(", ")
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Why a signed request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error(
        "the request is not signed: it needs Neon-Webhook-Timestamp and Neon-Webhook-Signature"
    )]
    Missing,
    #[error("Neon-Webhook-Timestamp is whole seconds since 1970")]
    BadTimestamp,
    #[error("the signature's timestamp is more than 5 minutes from the server's clock")]
    Stale,
    #[error(
        "the signature does not match the body: sign the timestamp, a dot and the body as sent"
    )]
    Mismatch,
}

/// Checks a request's signature headers against its body and the secrets that hold. The
/// timestamp must be within [`REPLAY_WINDOW`] of `now`.
pub fn verify<S: AsRef<[u8]>>(
    secrets: &[S],
    signature: Option<&str>,
    timestamp: Option<&str>,
    body: &[u8],
    now: OffsetDateTime,
) -> Result<(), SignatureError> {
    let (Some(signature), Some(timestamp)) = (signature, timestamp) else {
        return Err(SignatureError::Missing);
    };
    let timestamp: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| SignatureError::BadTimestamp)?;
    if now.unix_timestamp().abs_diff(timestamp) > REPLAY_WINDOW.as_secs() {
        return Err(SignatureError::Stale);
    }
    let tags: Vec<Vec<u8>> = signature
        .split(',')
        .take(MAX_SIGNATURES)
        .filter_map(|part| part.trim().strip_prefix("v1="))
        .filter_map(decode_hex)
        .collect();
    let matches = secrets.iter().any(|secret| {
        tags.iter().any(|tag| {
            mac(secret.as_ref(), timestamp, body)
                .verify_slice(tag)
                .is_ok()
        })
    });
    if matches {
        Ok(())
    } else {
        Err(SignatureError::Mismatch)
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 128 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"whsec_test-secret-for-signing";
    const BODY: &[u8] = br#"{"version":1,"type":"webhook.test"}"#;
    const AT: i64 = 1_795_348_800;

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).expect("a time")
    }

    #[test]
    fn signatures_are_hmac_sha256_of_the_timestamp_a_dot_and_the_body() {
        // RFC 4231, test case 2: the primitive is HMAC-SHA256.
        let mut mac = HmacSha256::new_from_slice(b"Jefe").expect("a key");
        mac.update(b"what do ya want for nothing?");
        let tag: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            tag,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        let signed = sign(SECRET, AT, BODY);
        assert!(signed.starts_with("v1="));
        assert_eq!(signed.len(), 3 + 64);
        let mut by_hand = HmacSha256::new_from_slice(SECRET).expect("a key");
        by_hand.update(format!("{AT}.").as_bytes());
        by_hand.update(BODY);
        let expected: String = by_hand
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(signed, format!("v1={expected}"));
        assert_ne!(sign(SECRET, AT + 1, BODY), signed);
        assert_ne!(sign(b"another", AT, BODY), signed);
    }

    #[test]
    fn a_signature_verifies_within_the_window_alone() {
        let header = header(&[SECRET], AT, BODY);
        let stamp = AT.to_string();
        let check = |now: i64| verify(&[SECRET], Some(&header), Some(&stamp), BODY, at(now));

        assert_eq!(check(AT), Ok(()));
        assert_eq!(check(AT + 300), Ok(()));
        assert_eq!(check(AT - 300), Ok(()));
        assert_eq!(check(AT + 301), Err(SignatureError::Stale));
        assert_eq!(check(AT - 301), Err(SignatureError::Stale));

        // Timestamps as far as they go either way are stale, not an overflow.
        for far in [i64::MIN, i64::MIN + 1, i64::MAX] {
            let far = far.to_string();
            assert_eq!(
                verify(&[SECRET], Some(&header), Some(&far), BODY, at(AT)),
                Err(SignatureError::Stale),
                "{far}"
            );
        }
    }

    #[test]
    fn a_changed_body_timestamp_or_secret_fails() {
        let header = header(&[SECRET], AT, BODY);
        let stamp = AT.to_string();
        let now = at(AT);

        assert_eq!(
            verify(&[SECRET], Some(&header), Some(&stamp), b"{}", now),
            Err(SignatureError::Mismatch)
        );
        let later = (AT + 1).to_string();
        assert_eq!(
            verify(&[SECRET], Some(&header), Some(&later), BODY, now),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify(
                &[b"another".as_slice()],
                Some(&header),
                Some(&stamp),
                BODY,
                now
            ),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify(&[SECRET], None, Some(&stamp), BODY, now),
            Err(SignatureError::Missing)
        );
        assert_eq!(
            verify(&[SECRET], Some(&header), None, BODY, now),
            Err(SignatureError::Missing)
        );
        assert_eq!(
            verify(&[SECRET], Some(&header), Some("soon"), BODY, now),
            Err(SignatureError::BadTimestamp)
        );
        for garbled in ["v1=zz", "v2=00", "", "v1=", "sha256=abc"] {
            assert_eq!(
                verify(&[SECRET], Some(garbled), Some(&stamp), BODY, now),
                Err(SignatureError::Mismatch),
                "{garbled:?}"
            );
        }
    }

    #[test]
    fn while_a_rotated_secret_holds_either_one_verifies() {
        let old: &[u8] = b"whsec_old";
        let new: &[u8] = b"whsec_new";
        let both = header(&[new, old], AT, BODY);
        let stamp = AT.to_string();
        let now = at(AT);

        assert_eq!(both.matches("v1=").count(), 2);
        assert_eq!(verify(&[old], Some(&both), Some(&stamp), BODY, now), Ok(()));
        assert_eq!(verify(&[new], Some(&both), Some(&stamp), BODY, now), Ok(()));
        let only_old = header(&[old], AT, BODY);
        assert_eq!(
            verify(&[new, old], Some(&only_old), Some(&stamp), BODY, now),
            Ok(())
        );
        assert_eq!(
            verify(&[new], Some(&only_old), Some(&stamp), BODY, now),
            Err(SignatureError::Mismatch)
        );
        let upper = sign(new, AT, BODY).to_uppercase().replacen("V1=", "v1=", 1);
        assert_eq!(
            verify(&[new], Some(&upper), Some(&stamp), BODY, now),
            Ok(())
        );
    }
}
