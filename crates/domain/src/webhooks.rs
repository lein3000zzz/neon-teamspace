//! Webhooks: what outgoing ones tell of and where they may go, and the names and secrets of
//! both kinds.
//!
//! - **Outgoing webhooks** belong to one identity of a workspace, a person, an agent or an
//!   integration, and are told of the [`EventType`]s they name about the rows that
//!   identity is assigned to, the mentions of it and the replies to its comments. They go
//!   to an `http` or `https` URL whose host is checked by [`address`] before every
//!   delivery, so none reaches the server's own network unless the instance allows it.
//! - **Incoming webhooks** each act as an integration of their workspace, on one target: a
//!   database, whose rows they make and change, or a page, whose content they add to.
//!
//! Who registers and manages them is for the permission rules, see
//! [`authorize`](crate::workspaces::authorize).

pub mod address;

use url::{Host, Url};

pub use address::{Blocked, check_ip};

text_enum! {
    /// What an outgoing webhook may be told of. A receiver must ignore types it does not
    /// know: more come later.
    EventType {
        /// The identity was added to a person property of a row.
        Assigned => "row.assigned",
        /// The status property of a row changed, while the identity is in one of its person
        /// properties.
        StatusChanged => "row.status_changed",
        /// A page's text, or a comment on it, mentions the identity.
        Mentioned => "page.mentioned",
        /// Someone replied in a thread of comments the identity wrote in.
        Replied => "comment.replied",
    }
}

/// The type of the event that "send test event" delivers, which no webhook subscribes to.
pub const TEST_EVENT: &str = "webhook.test";

/// The version of the payloads outgoing webhooks send, and incoming ones take.
pub const PAYLOAD_VERSION: u32 = 1;

/// The longest webhook URL, in bytes.
pub const MAX_URL: usize = 2048;

/// The longest name of an incoming webhook, in characters: an integration's.
pub const MAX_NAME: usize = crate::agents::MAX_AGENT_NAME;

/// What an outgoing webhook's URL may not be.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidUrl {
    #[error("a webhook URL is an absolute http or https URL")]
    NotAUrl,
    #[error("a webhook URL uses http or https")]
    Scheme,
    #[error("a webhook URL is at most {MAX_URL} bytes")]
    TooLong,
    #[error("a webhook URL names a host")]
    NoHost,
    #[error("a webhook URL carries no user name or password: put a secret in its path instead")]
    Credentials,
}

/// A webhook URL as kept: parsed, `http` or `https`, with a host, no credentials and no
/// fragment, at most [`MAX_URL`] bytes. Where the host points is checked by
/// [`address`] once it is resolved, at every delivery.
pub fn check_url(raw: &str) -> Result<Url, InvalidUrl> {
    let raw = raw.trim();
    if raw.len() > MAX_URL {
        return Err(InvalidUrl::TooLong);
    }
    let mut url = Url::parse(raw).map_err(|_| InvalidUrl::NotAUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(InvalidUrl::Scheme);
    }
    if url.host().is_none() {
        return Err(InvalidUrl::NoHost);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(InvalidUrl::Credentials);
    }
    url.set_fragment(None);
    if url.as_str().len() > MAX_URL {
        return Err(InvalidUrl::TooLong);
    }
    Ok(url)
}

/// The address a URL's host is already, when it is an IP literal rather than a name.
pub fn literal_ip(url: &Url) -> Option<std::net::IpAddr> {
    match url.host()? {
        Host::Ipv4(ip) => Some(ip.into()),
        Host::Ipv6(ip) => Some(ip.into()),
        Host::Domain(_) => None,
    }
}

/// An incoming webhook's name that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a webhook's name has 1 to {MAX_NAME} characters, and no control characters")]
pub struct InvalidName;

/// An incoming webhook's name, trimmed: its integration's name.
pub fn check_name(raw: &str) -> Result<String, InvalidName> {
    crate::agents::check_agent_name(raw).map_err(|_| InvalidName)
}

/// The events an outgoing webhook asks for, each once, in [`EventType::ALL`] order: at
/// least one.
pub fn check_events(asked: &[EventType]) -> Result<Vec<EventType>, String> {
    if asked.is_empty() {
        return Err("a webhook asks for at least one event".to_owned());
    }
    let mut events = asked.to_vec();
    events.sort();
    events.dedup();
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_http_with_a_host_and_no_credentials() {
        let kept = check_url(" https://hooks.example.com/in/a1?x=1#part ").expect("valid");
        assert_eq!(kept.as_str(), "https://hooks.example.com/in/a1?x=1");
        assert!(check_url("http://203.0.113.9:8080/").is_ok());

        assert_eq!(check_url("ftp://example.com/"), Err(InvalidUrl::Scheme));
        assert_eq!(check_url("file:///etc/passwd"), Err(InvalidUrl::Scheme));
        assert_eq!(check_url("javascript:alert(1)"), Err(InvalidUrl::Scheme));
        assert_eq!(check_url("/relative"), Err(InvalidUrl::NotAUrl));
        assert_eq!(check_url("not a url"), Err(InvalidUrl::NotAUrl));
        assert_eq!(
            check_url("https://user:pass@example.com/"),
            Err(InvalidUrl::Credentials)
        );
        assert_eq!(
            check_url("https://user@example.com/"),
            Err(InvalidUrl::Credentials)
        );
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert_eq!(check_url(&long), Err(InvalidUrl::TooLong));
    }

    #[test]
    fn numeric_hosts_in_odd_forms_read_as_the_address_they_are() {
        for raw in [
            "http://2130706433/",
            "http://0x7f.0.0.1/",
            "http://0177.0.0.1/",
            "http://127.1/",
            "http://127.0.0.1./",
        ] {
            let url = check_url(raw).expect("a URL");
            assert_eq!(
                literal_ip(&url),
                Some("127.0.0.1".parse().expect("an address")),
                "{raw}"
            );
        }
        let v6 = check_url("http://[::ffff:7f00:1]/").expect("a URL");
        assert_eq!(literal_ip(&v6), Some("::ffff:127.0.0.1".parse().unwrap()));
        assert_eq!(
            literal_ip(&check_url("https://example.com/").expect("a URL")),
            None
        );
    }

    #[test]
    fn events_are_each_kept_once() {
        assert_eq!(
            check_events(&[
                EventType::StatusChanged,
                EventType::Assigned,
                EventType::Assigned
            ]),
            Ok(vec![EventType::Assigned, EventType::StatusChanged])
        );
        assert!(check_events(&[]).is_err());
        for event in EventType::ALL {
            assert_eq!(EventType::parse(event.as_str()), Some(*event));
        }
        assert_eq!(EventType::parse(TEST_EVENT), None);
    }

    #[test]
    fn names_are_an_integrations() {
        assert_eq!(check_name("  CI  "), Ok("CI".to_owned()));
        assert_eq!(check_name(""), Err(InvalidName));
        assert_eq!(check_name("a\u{7}"), Err(InvalidName));
    }
}
