use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

/// Random bytes in every secret the server makes: 256 bits.
pub const SECRET_BYTES: usize = 32;

/// A new random secret: `prefix` and [`SECRET_BYTES`] random bytes, base64url without
/// padding, so it fits in URLs and headers as it is. Used for session tokens, API tokens,
/// invite codes and mailed links.
///
/// The bytes come from the thread's CSPRNG, which the operating system seeds.
pub fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::fill(&mut bytes);
    let mut secret = String::with_capacity(prefix.len() + 43);
    secret.push_str(prefix);
    URL_SAFE_NO_PAD.encode_string(bytes, &mut secret);
    secret
}

/// What the database keeps of a secret, and finds it by: its SHA-256. The secrets are
/// random 256-bit values, so a fast hash is enough, with no salt or stretching.
pub fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// The PKCE challenge of a code verifier by the S256 method (RFC 7636, section 4.2): its
/// SHA-256, in base64url without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether a code verifier answers an S256 challenge, compared in constant time so that
/// how long it takes says nothing of how much matched.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let expected = pkce_challenge(verifier);
    expected.len() == challenge.len()
        && expected
            .bytes()
            .zip(challenge.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_carry_their_prefix_and_256_random_bits() {
        let token = new_secret("nw_");

        assert!(token.starts_with("nw_"));
        assert_eq!(token.len(), 3 + 43);
        assert!(
            token[3..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(new_secret(""), new_secret(""));
    }

    #[test]
    fn hashes_are_sha256() {
        assert_eq!(hash_secret("abc").len(), 32);
        assert_eq!(hash_secret("abc"), hash_secret("abc"));
        assert_ne!(hash_secret("abc"), hash_secret("abd"));
        // The SHA-256 of "abc", from FIPS 180-2.
        assert_eq!(hash_secret("abc")[..4], [0xba, 0x78, 0x16, 0xbf]);
    }

    #[test]
    fn pkce_answers_the_rfc_7636_example() {
        // RFC 7636, appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        assert_eq!(pkce_challenge(verifier), challenge);
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(verifier, &challenge[..42]));
        assert!(!pkce_matches(&verifier[1..], challenge));
        assert!(!pkce_matches(verifier, ""));
    }
}
