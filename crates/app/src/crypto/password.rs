use std::fmt;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{Error as PhcError, PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};

/// The cost of a password hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// Memory in KiB.
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    /// Bytes of output, and of salt.
    pub output_len: usize,
    pub salt_len: usize,
}

impl Argon2Params {
    /// OWASP's minimum for argon2id: 19 MiB, 2 passes, 1 lane. It keeps a login cheap
    /// enough for a public instance.
    pub const DEFAULT: Self = Self {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
        output_len: 32,
        salt_len: 16,
    };
}

/// Why a hash could not be made or checked. The messages never hold the password.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HashError {
    #[error("invalid argon2 parameters: {0}")]
    Params(String),
    #[error("malformed password hash: {0}")]
    Malformed(String),
    #[error("password hashing failed: {0}")]
    Hashing(String),
}

/// What checking a password against a hash found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    pub matches: bool,
    /// The password matched a hash made with other parameters than the hasher's, so it
    /// should be hashed again and saved.
    pub needs_rehash: bool,
}

/// Hashes passwords with argon2id into PHC strings, like
/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`.
///
/// Both operations take tens of milliseconds of CPU on purpose; call them where blocking
/// is allowed, such as `tokio::task::spawn_blocking`.
#[derive(Clone)]
pub struct PasswordHasher {
    params: Argon2Params,
    argon2: Params,
}

impl PasswordHasher {
    pub fn new(params: Argon2Params) -> Result<Self, HashError> {
        let argon2 = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            Some(params.output_len),
        )
        .map_err(|error| HashError::Params(error.to_string()))?;
        Ok(Self { params, argon2 })
    }

    /// A PHC string of `password` with a new random salt.
    pub fn hash(&self, password: &str) -> Result<String, HashError> {
        let mut salt = vec![0u8; self.params.salt_len];
        rand::fill(salt.as_mut_slice());
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.argon2.clone())
            .hash_password_with_salt(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| HashError::Hashing(error.to_string()))
    }

    /// Checks `password` against a PHC string, with the parameters the string names. The
    /// output is compared in constant time. Only argon2id version 19 hashes are taken.
    pub fn verify(&self, password: &str, phc: &str) -> Result<Verified, HashError> {
        let hash =
            PasswordHash::new(phc).map_err(|error| HashError::Malformed(error.to_string()))?;
        if hash.algorithm.as_str() != Algorithm::Argon2id.ident().as_str() {
            return Err(HashError::Malformed(format!(
                "algorithm {} is not argon2id",
                hash.algorithm
            )));
        }
        if hash.version != Some(Version::V0x13.into()) {
            return Err(HashError::Malformed("not argon2 version 19".to_owned()));
        }
        if hash.salt.is_none() || hash.hash.is_none() {
            return Err(HashError::Malformed("no salt or no output".to_owned()));
        }
        let params =
            Params::try_from(&hash).map_err(|error| HashError::Malformed(error.to_string()))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone());

        match argon2.verify_password(password.as_bytes(), &hash) {
            Ok(()) => Ok(Verified {
                matches: true,
                needs_rehash: !self.made_with_own_params(&params, &hash),
            }),
            Err(PhcError::PasswordInvalid) => Ok(Verified {
                matches: false,
                needs_rehash: false,
            }),
            Err(error) => Err(HashError::Hashing(error.to_string())),
        }
    }

    fn made_with_own_params(&self, params: &Params, hash: &PasswordHash) -> bool {
        params.m_cost() == self.params.memory_kib
            && params.t_cost() == self.params.iterations
            && params.p_cost() == self.params.parallelism
            && params.output_len() == Some(self.params.output_len)
            && hash
                .salt
                .as_ref()
                .is_some_and(|salt| salt.len() == self.params.salt_len)
    }
}

impl fmt::Debug for PasswordHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordHasher")
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters, so the tests run fast.
    const CHEAP: Argon2Params = Argon2Params {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
        output_len: 32,
        salt_len: 16,
    };

    fn hasher(params: Argon2Params) -> PasswordHasher {
        PasswordHasher::new(params).expect("valid parameters")
    }

    #[test]
    fn hashes_are_argon2id_phc_strings_with_fresh_salts() {
        let hasher = hasher(Argon2Params::DEFAULT);

        let hash = hasher.hash("correct horse battery").expect("hashes");

        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{hash}"
        );
        assert!(!hash.contains("correct horse"));
        assert_ne!(hash, hasher.hash("correct horse battery").expect("hashes"));
    }

    #[test]
    fn the_right_password_matches_and_others_do_not() {
        let hasher = hasher(CHEAP);
        let hash = hasher.hash("correct horse battery").expect("hashes");

        assert_eq!(
            hasher.verify("correct horse battery", &hash),
            Ok(Verified {
                matches: true,
                needs_rehash: false
            })
        );
        assert_eq!(
            hasher.verify("correct horse batterY", &hash),
            Ok(Verified {
                matches: false,
                needs_rehash: false
            })
        );
    }

    #[test]
    fn hashes_made_with_other_parameters_want_a_rehash() {
        let old = hasher(CHEAP).hash("correct horse battery").expect("hashes");
        let current = hasher(Argon2Params {
            iterations: 2,
            ..CHEAP
        });

        let verified = current
            .verify("correct horse battery", &old)
            .expect("checks");

        assert_eq!(
            verified,
            Verified {
                matches: true,
                needs_rehash: true
            }
        );
        let wrong = current.verify("wrong password", &old).expect("checks");
        assert!(!wrong.needs_rehash);
    }

    #[test]
    fn other_algorithms_and_garbage_are_malformed() {
        let hasher = hasher(CHEAP);
        let argon2i = hasher
            .hash("pw")
            .expect("hashes")
            .replacen("$argon2id$", "$argon2i$", 1);

        for bad in [
            "",
            "plain text",
            "$argon2id$v=19$m=64,t=1,p=1$c2FsdHNhbHQ",
            "$2b$12$abcdefghijklmnopqrstuuvwxyzABCDEFGHIJKLMNOPQRSTUVW",
            argon2i.as_str(),
        ] {
            assert!(
                matches!(hasher.verify("pw", bad), Err(HashError::Malformed(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn errors_and_debug_output_never_hold_the_password() {
        let hasher = hasher(CHEAP);
        let error = hasher
            .verify("hunter2hunter2", "hunter2hunter2")
            .expect_err("malformed");

        assert!(!error.to_string().contains("hunter2"), "{error}");
        assert!(!format!("{hasher:?}").contains("hunter2"));
    }
}
