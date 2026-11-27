//! Checking that an address is a Neon Teamspace server, before its pages are loaded.
//!
//! The app asks for `GET /api/v1/meta`, which needs no sign-in, with a time limit and a cap on
//! the answer's size, and follows no redirect: the app loads the origin it checked, and a
//! server that sends its visitors elsewhere is named at the address it sends them to. The
//! answer must be `200`, JSON, at most [`MAX_ANSWER_BYTES`], and an object whose `product` is
//! `neon-teamspace`, with the instance's `name` and the server's semantic `version`. Anything
//! else is refused with an [`CheckError`] that says what was wrong.

use std::sync::Once;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::{Client, StatusCode, redirect};
use serde::Serialize;
use serde_json::Value;

use crate::address::ServerAddress;

/// What `product` says on every Neon Teamspace server.
pub const PRODUCT: &str = "neon-teamspace";

/// The largest answer read. The real one is under a hundred bytes.
pub const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// How long the whole check may take.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// How long connecting, TLS included, may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest instance name taken, in characters.
const MAX_NAME_CHARS: usize = 200;

/// The longest version taken, in bytes.
const MAX_VERSION_LEN: usize = 64;

/// What the check learnt of a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerInfo {
    /// The instance's name, its `instance.name` setting.
    pub name: String,
    /// The server's version, such as `0.1.0`.
    pub version: String,
}

/// Why an address is not a Neon Teamspace server the app can use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error("Could not reach {origin}: {reason}.{hint}")]
    Unreachable {
        origin: String,
        reason: String,
        hint: String,
    },
    #[error("{origin} did not answer within {seconds} seconds.")]
    TimedOut { origin: String, seconds: u64 },
    #[error("{origin} sends its visitors to {to}. Enter that address instead.")]
    Redirected { origin: String, to: String },
    #[error(
        "{origin} has no Neon Teamspace API: /api/v1/meta answered {status}. Check the address \
         is the one you open in a browser."
    )]
    Status { origin: String, status: u16 },
    #[error("{origin} answered with something other than JSON: it is not a Neon Teamspace server.")]
    NotJson { origin: String },
    #[error(
        "{origin} answered with more than {} KiB: it is not a Neon Teamspace server.",
        MAX_ANSWER_BYTES / 1024
    )]
    TooLarge { origin: String },
    #[error("{origin} answered with JSON that is not a Neon Teamspace server's: {what}.")]
    NotNeon { origin: String, what: String },
}

impl CheckError {
    /// A stable name for the picker and the tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable { .. } => "unreachable",
            Self::TimedOut { .. } => "timed_out",
            Self::Redirected { .. } => "redirected",
            Self::Status { .. } => "status",
            Self::NotJson { .. } => "not_json",
            Self::TooLarge { .. } => "too_large",
            Self::NotNeon { .. } => "not_neon",
        }
    }
}

/// How a check is made. Tests shorten the time limit.
#[derive(Debug, Clone, Copy)]
pub struct CheckOptions {
    pub timeout: Duration,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self { timeout: TIMEOUT }
    }
}

/// Asks the server at `address` for its metadata and checks it.
pub async fn check(
    address: &ServerAddress,
    options: CheckOptions,
) -> Result<ServerInfo, CheckError> {
    let origin = address.origin();
    let client = client(options).map_err(|error| unreachable(address, &error))?;
    let mut response = client
        .get(address.meta_url())
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| request_error(address, options, &error))?;

    let status = response.status();
    if status.is_redirection() {
        let to = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| address.meta_url().join(location).ok())
            .map_or_else(
                || "another address".to_owned(),
                |url| url.origin().ascii_serialization(),
            );
        return Err(CheckError::Redirected { origin, to });
    }
    if status != StatusCode::OK {
        return Err(CheckError::Status {
            origin,
            status: status.as_u16(),
        });
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if !is_json(content_type.as_deref()) {
        return Err(CheckError::NotJson { origin });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ANSWER_BYTES as u64)
    {
        return Err(CheckError::TooLarge { origin });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| request_error(address, options, &error))?
    {
        if body.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(CheckError::TooLarge { origin });
        }
        body.extend_from_slice(&chunk);
    }
    read_meta(&origin, &body)
}

/// Checks the body of a `200` JSON answer.
pub fn read_meta(origin: &str, body: &[u8]) -> Result<ServerInfo, CheckError> {
    let not_neon = |what: &str| CheckError::NotNeon {
        origin: origin.to_owned(),
        what: what.to_owned(),
    };
    let value: Value =
        serde_json::from_slice(body).map_err(|_| not_neon("the answer is not valid JSON"))?;
    let Value::Object(fields) = value else {
        return Err(not_neon("the answer is not an object"));
    };
    match fields.get("product") {
        Some(Value::String(product)) if product == PRODUCT => {}
        Some(Value::String(product)) => {
            return Err(not_neon(&format!("it says it is {}", shorten(product))));
        }
        Some(_) => return Err(not_neon("its product is not a string")),
        None => {
            return Err(not_neon(
                "it names no product, as no Neon Teamspace server this app can use does",
            ));
        }
    }
    let name = match fields.get("name") {
        Some(Value::String(name)) => name.trim(),
        _ => return Err(not_neon("it has no instance name")),
    };
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(not_neon(
            "its instance name is empty, too long or unprintable",
        ));
    }
    let version = match fields.get("version") {
        Some(Value::String(version)) if is_semver(version) => version,
        Some(_) => return Err(not_neon("its version is not a version number")),
        None => return Err(not_neon("it has no version")),
    };
    Ok(ServerInfo {
        name: name.to_owned(),
        version: version.clone(),
    })
}

fn client(options: CheckOptions) -> reqwest::Result<Client> {
    install_crypto_provider();
    let builder = Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(options.timeout)
        .connect_timeout(CONNECT_TIMEOUT.min(options.timeout))
        .user_agent(concat!("NeonTeamspaceApp/", env!("CARGO_PKG_VERSION")));
    // The platform's own certificate verifier, which trusts what the system trusts, as the
    // webview does; Android's needs a Java component of its own, so there the Mozilla roots,
    // which Android's system store follows, are used instead.
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der).ok()),
    );
    builder.build()
}

/// ring, as the server uses: aws-lc, rustls' default, needs more tooling to cross-compile.
fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // An error means one is installed already, which is as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn request_error(
    address: &ServerAddress,
    options: CheckOptions,
    error: &reqwest::Error,
) -> CheckError {
    if error.is_timeout() {
        CheckError::TimedOut {
            origin: address.origin(),
            seconds: options.timeout.as_secs_f64().ceil() as u64,
        }
    } else {
        unreachable(address, error)
    }
}

fn unreachable(address: &ServerAddress, error: &reqwest::Error) -> CheckError {
    // The innermost cause says the most, such as "connection refused" or the certificate's
    // problem, where the outer ones only say a request failed.
    let mut cause: &dyn std::error::Error = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    let reason = cause.to_string();
    let lower = reason.to_ascii_lowercase();
    let hint = if address.is_encrypted() && address.is_private() {
        " If the server has no TLS certificate, enter its address with http:// in front."
    } else if lower.contains("certificate") {
        " Its TLS certificate is not one this device trusts."
    } else {
        ""
    };
    CheckError::Unreachable {
        origin: address.origin(),
        reason,
        hint: hint.to_owned(),
    }
}

fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
}

/// `MAJOR.MINOR.PATCH`, then maybe `-pre-release` and `+build` of ASCII letters, digits, dots
/// and hyphens.
fn is_semver(version: &str) -> bool {
    if version.len() > MAX_VERSION_LEN {
        return false;
    }
    let (core, extra) = match version.find(['-', '+']) {
        Some(at) => version.split_at(at),
        None => (version, ""),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    let core_ok = numbers.len() == 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    let extra_ok = extra.len() != 1
        && extra
            .bytes()
            .skip(1)
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    core_ok && extra_ok
}

/// A value from the server, cut short for a message.
fn shorten(text: &str) -> String {
    const MAX: usize = 40;
    let mut short: String = text.chars().filter(|c| !c.is_control()).take(MAX).collect();
    if text.chars().count() > MAX {
        short.push('…');
    }
    format!("\"{short}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://neon.example.com";

    fn read(body: &str) -> Result<ServerInfo, CheckError> {
        read_meta(ORIGIN, body.as_bytes())
    }

    fn refusal(body: &str) -> String {
        match read(body) {
            Ok(info) => panic!("{body} was taken as {info:?}"),
            Err(CheckError::NotNeon { what, .. }) => what,
            Err(other) => panic!("{body} was refused as {other:?}"),
        }
    }

    #[test]
    fn a_neon_teamspace_answer_gives_the_name_and_version() {
        assert_eq!(
            read(r#"{"product":"neon-teamspace","name":"Acme","version":"0.1.0"}"#),
            Ok(ServerInfo {
                name: "Acme".to_owned(),
                version: "0.1.0".to_owned()
            })
        );
    }

    #[test]
    fn fields_the_app_does_not_know_are_ignored_and_the_name_trimmed() {
        let info = read(
            r#"{"product":"neon-teamspace","name":"  Acme  ","version":"1.2.3-rc.1+abc","x":1}"#,
        )
        .unwrap();
        assert_eq!(info.name, "Acme");
        assert_eq!(info.version, "1.2.3-rc.1+abc");
    }

    #[test]
    fn json_that_is_not_an_object_is_refused() {
        assert_eq!(refusal("not json"), "the answer is not valid JSON");
        assert_eq!(refusal(""), "the answer is not valid JSON");
        assert_eq!(refusal("[]"), "the answer is not an object");
        assert_eq!(
            refusal(r#""neon-teamspace""#),
            "the answer is not an object"
        );
    }

    #[test]
    fn another_product_is_refused() {
        assert_eq!(
            refusal(r#"{"product":"gitea","name":"Acme","version":"1.0.0"}"#),
            r#"it says it is "gitea""#
        );
        assert_eq!(
            refusal(r#"{"product":7,"name":"Acme","version":"1.0.0"}"#),
            "its product is not a string"
        );
        assert!(refusal(r#"{"name":"Acme","version":"1.0.0"}"#).starts_with("it names no product"));
        let long = format!(
            r#"{{"product":"{}","name":"A","version":"1.0.0"}}"#,
            "x".repeat(100)
        );
        assert_eq!(
            refusal(&long),
            format!("it says it is \"{}…\"", "x".repeat(40))
        );
    }

    #[test]
    fn a_missing_or_unusable_name_is_refused() {
        for body in [
            r#"{"product":"neon-teamspace","version":"1.0.0"}"#,
            r#"{"product":"neon-teamspace","name":5,"version":"1.0.0"}"#,
        ] {
            assert_eq!(refusal(body), "it has no instance name", "{body}");
        }
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        for name in ["", "   ", "bell\\u0007", long.as_str()] {
            let body =
                format!(r#"{{"product":"neon-teamspace","name":"{name}","version":"1.0.0"}}"#);
            assert_eq!(
                refusal(&body),
                "its instance name is empty, too long or unprintable"
            );
        }
        let longest = "n".repeat(MAX_NAME_CHARS);
        let body =
            format!(r#"{{"product":"neon-teamspace","name":"{longest}","version":"1.0.0"}}"#);
        assert_eq!(read(&body).unwrap().name, longest);
    }

    #[test]
    fn a_missing_or_unusable_version_is_refused() {
        assert_eq!(
            refusal(r#"{"product":"neon-teamspace","name":"Acme"}"#),
            "it has no version"
        );
        for version in [
            r#""""#,
            r#""1""#,
            r#""1.0""#,
            r#""1.0.0.0""#,
            r#""v1.0.0""#,
            r#""1.x.0""#,
            r#""1.0.0-""#,
            r#""1.0.0 beta""#,
            r#""1..0""#,
            "1",
            "null",
        ] {
            let body =
                format!(r#"{{"product":"neon-teamspace","name":"Acme","version":{version}}}"#);
            assert_eq!(
                refusal(&body),
                "its version is not a version number",
                "{version}"
            );
        }
        let long = format!(r#""1.0.0-{}""#, "a".repeat(MAX_VERSION_LEN));
        let body = format!(r#"{{"product":"neon-teamspace","name":"Acme","version":{long}}}"#);
        assert_eq!(refusal(&body), "its version is not a version number");
    }

    #[test]
    fn semantic_versions_are_recognised() {
        for version in [
            "0.1.0",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-rc.1",
            "1.0.0+build.5",
            "1.0.0-a+b",
        ] {
            assert!(is_semver(version), "{version}");
        }
    }

    #[test]
    fn json_content_types_are_recognised() {
        assert!(is_json(Some("application/json")));
        assert!(is_json(Some("application/json; charset=utf-8")));
        assert!(is_json(Some("Application/JSON")));
        assert!(!is_json(Some("text/html")));
        assert!(!is_json(Some("application/jsonp")));
        assert!(!is_json(Some("text/json")));
        assert!(!is_json(None));
    }

    #[test]
    fn every_refusal_has_a_code_and_a_message() {
        let origin = ORIGIN.to_owned();
        let errors = [
            CheckError::Unreachable {
                origin: origin.clone(),
                reason: "connection refused".to_owned(),
                hint: String::new(),
            },
            CheckError::TimedOut {
                origin: origin.clone(),
                seconds: 10,
            },
            CheckError::Redirected {
                origin: origin.clone(),
                to: "https://www.example.com".to_owned(),
            },
            CheckError::Status {
                origin: origin.clone(),
                status: 404,
            },
            CheckError::NotJson {
                origin: origin.clone(),
            },
            CheckError::TooLarge {
                origin: origin.clone(),
            },
            CheckError::NotNeon {
                origin,
                what: "it has no version".to_owned(),
            },
        ];
        let codes: Vec<&str> = errors.iter().map(CheckError::code).collect();
        assert_eq!(
            codes,
            [
                "unreachable",
                "timed_out",
                "redirected",
                "status",
                "not_json",
                "too_large",
                "not_neon"
            ]
        );
        for error in errors {
            assert!(error.to_string().contains(ORIGIN), "{error}");
        }
    }
}
