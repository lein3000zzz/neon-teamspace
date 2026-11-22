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
}
