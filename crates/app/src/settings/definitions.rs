use std::time::Duration;

use neon_domain::auth::invite::MAX_LIFETIME;
use neon_domain::auth::password::{DEFAULT_MIN_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH_BOUNDS};
use neon_domain::auth::{Email, RegistrationMode, session};
use neon_domain::duration;
use neon_domain::mail::{self as rules, Security};
use neon_domain::settings::{Definition, Key, Registry, Snapshot, checks};

/// The instance's name, shown in the interface, in mail and by `GET /api/v1/meta`.
pub const INSTANCE_NAME: Key<str> = Key::new("instance.name");

/// Accounts, sign-ups and sessions.
pub mod auth {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// Who may create an account: `open`, `invite` or `closed`.
    pub const REGISTRATION: Key<str> = Key::new("auth.registration");
    /// Self sign-ups verify their address by a mailed link before they sign in.
    pub const VERIFY_EMAIL: Key<bool> = Key::new("auth.verify_email");
    /// Forgotten passwords can be reset with a mailed link.
    pub const PASSWORD_RESET: Key<bool> = Key::new("auth.password_reset");
    /// How long a new invite works unless its admin says otherwise.
    pub const INVITE_TTL: Key<Duration> = Key::new("auth.invite_ttl");
    /// Admins may mail invites.
    pub const INVITE_EMAIL: Key<bool> = Key::new("auth.invite_email");
    /// People without an invite may ask for one.
    pub const REQUESTS: Key<bool> = Key::new("auth.requests");
    /// Denied requests to join are told so by mail.
    pub const REQUEST_DENIED_MAIL: Key<bool> = Key::new("auth.request_denied_mail");
    /// The fewest characters a new password has.
    pub const MIN_PASSWORD_LENGTH: Key<i64> = Key::new("auth.min_password_length");
    /// A session unused this long signs out.
    pub const SESSION_IDLE_TTL: Key<Duration> = Key::new("auth.session_idle_ttl");
    /// A session signs out after this long however active it is.
    pub const SESSION_ABSOLUTE_TTL: Key<Duration> = Key::new("auth.session_absolute_ttl");
}

/// Outgoing mail: the SMTP server it goes through, and who it comes from.
pub mod mail {
    use neon_domain::settings::{Key, Secret};

    /// Mail goes out, once a host and a From address are set too.
    pub const ENABLED: Key<bool> = Key::new("mail.enabled");
    /// The SMTP server's host name or address.
    pub const HOST: Key<str> = Key::new("mail.host");
    pub const PORT: Key<i64> = Key::new("mail.port");
    /// How the connection is secured: `starttls`, `tls` or `none`.
    pub const TLS: Key<str> = Key::new("mail.tls");
    /// The account to sign in with; empty signs in as nobody.
    pub const USERNAME: Key<str> = Key::new("mail.username");
    pub const PASSWORD: Key<Secret> = Key::new("mail.password");
    /// The address mail comes from.
    pub const FROM: Key<str> = Key::new("mail.from");
    /// The name beside it; empty uses the instance's name.
    pub const FROM_NAME: Key<str> = Key::new("mail.from_name");
    /// Mail about activity, such as a request to join waiting for the admins.
    pub const NOTIFICATIONS: Key<bool> = Key::new("mail.notifications");
}

/// Whether the settings name a mail server to use: mail switched on, with a host and a
/// From address. Mailing links needs more, which the mail queue checks: a secret key to
/// seal them while they wait, and the public URL they point to.
pub fn mail_set_up(settings: &Snapshot) -> bool {
    *settings.get(mail::ENABLED)
        && !settings.get(mail::HOST).is_empty()
        && !settings.get(mail::FROM).is_empty()
}

/// What the rules say about a flow that cannot work without mail.
const NEEDS_MAIL: &str = "needs mail set up first: mail.enabled on, with mail.host and mail.from";

/// The longest a session may last, either way.
const MAX_SESSION_TTL: Duration = Duration::from_secs(365 * 24 * 3600);

/// Every runtime setting the server reads.
///
/// Verifying sign-ups and requests to join need mail set up, by rule: on without it, they
/// would turn people away unseen. Password resets and mailed invites are on by default and
/// work while mail is set up, so a rule would refuse the defaults; the flows check at run
/// time instead.
///
/// # Panics
///
/// When a definition is invalid. That is a programming error, and a test builds this
/// registry, so it cannot reach a release.
pub fn registry() -> Registry {
    Registry::builder()
        .define(
            Definition::string(
                INSTANCE_NAME,
                "Neon Workspace",
                "The instance's name, shown in the interface and in mail.",
            )
            .check(checks::non_blank)
            .check(checks::max_chars(100)),
        )
        .define(Definition::enumeration(
            auth::REGISTRATION,
            RegistrationMode::OPTIONS,
            RegistrationMode::DEFAULT.as_str(),
            "Who may create an account: anyone (open), people with an invite an admin made \
             (invite), or nobody but admins, who add accounts themselves (closed).",
        ))
        .define(Definition::bool(
            auth::VERIFY_EMAIL,
            false,
            "People who sign up themselves get a link by mail and can sign in once they \
             open it; an account still unverified after 7 days is removed. Those who sign \
             up with an invite mailed to them, and accounts admins make, count as verified. \
             Needs mail set up first: see mail.enabled.",
        ))
        .define(Definition::bool(
            auth::PASSWORD_RESET,
            true,
            "Forgotten passwords can be reset with a link mailed to the account, working \
             once for an hour. Setting the new password signs the account out everywhere. \
             Works while mail is set up.",
        ))
        .define(
            Definition::duration(
                auth::INVITE_TTL,
                Duration::from_secs(7 * 24 * 3600),
                "How long a new invite works, unless the admin making it sets its end. \
                 Applies to invites made from now.",
            )
            .check(between(Duration::from_secs(3600), MAX_LIFETIME)),
        )
        .define(Definition::bool(
            auth::INVITE_EMAIL,
            true,
            "Admins can mail an invite to an address, naming themselves; only that address \
             signs up with it, once, and counts as verified. Works while mail is set up.",
        ))
        .define(Definition::bool(
            auth::REQUESTS,
            false,
            "While registration is by invite, people without one can ask for it with a \
             note. They confirm their address by a mailed link first; admins then approve, \
             which mails them an invite, or deny. Needs mail set up first: see \
             mail.enabled.",
        ))
        .define(Definition::bool(
            auth::REQUEST_DENIED_MAIL,
            true,
            "Mail people whose request to join an admin denied, saying they may ask again \
             in 30 days. Off, a denied request just closes.",
        ))
        .define(
            Definition::integer(
                auth::MIN_PASSWORD_LENGTH,
                DEFAULT_MIN_PASSWORD_LENGTH,
                "The fewest characters a new password has. Passwords set before keep \
                 working.",
            )
            .check(|n: &i64| {
                let (least, most) = MIN_PASSWORD_LENGTH_BOUNDS;
                if (least..=most).contains(n) {
                    Ok(())
                } else {
                    Err(format!("must be between {least} and {most}"))
                }
            }),
        )
        .define(
            Definition::duration(
                auth::SESSION_IDLE_TTL,
                session::DEFAULT_IDLE,
                "A browser session unused this long signs out. Applies to sessions in use \
                 from now.",
            )
            .check(between(session::MIN_TTL, MAX_SESSION_TTL)),
        )
        .define(
            Definition::duration(
                auth::SESSION_ABSOLUTE_TTL,
                session::DEFAULT_ABSOLUTE,
                "A browser session signs out this long after it started, however active it \
                 is. Applies to sessions started from now.",
            )
            .check(between(session::MIN_TTL, MAX_SESSION_TTL)),
        )
        .define(Definition::bool(
            mail::ENABLED,
            false,
            "Sends mail through the SMTP server mail.host names. Mail is set up once this is \
             on and mail.host and mail.from are set. Mail also needs server.public_url, \
             which its links point to, and secrets.keys, which seals it while it waits in \
             the queue.",
        ))
        .define(
            Definition::string(
                mail::HOST,
                "",
                "The SMTP server mail goes through, such as smtp.example.com: a host name or \
                 an IP address, without a scheme or port.",
            )
            .check(rules::check_host)
            .browser_only(),
        )
        .define(
            Definition::integer(
                mail::PORT,
                587,
                "The SMTP server's port: 587 for starttls, 465 for tls, 25 or a relay's own \
                 for none.",
            )
            .check(|port: &i64| {
                if (1..=65535).contains(port) {
                    Ok(())
                } else {
                    Err("must be between 1 and 65535".to_owned())
                }
            })
            .browser_only(),
        )
        .define(
            Definition::enumeration(
                mail::TLS,
                Security::OPTIONS,
                Security::DEFAULT.as_str(),
                "How the connection is secured. starttls upgrades it with STARTTLS and \
                 refuses a server that does not offer it; tls speaks TLS from the start; none \
                 stays plain, for a relay on this host or a network you trust. TLS is 1.2 or \
                 later, and the server's certificate is always checked. A password is never \
                 sent over a plain connection, except to this host.",
            )
            .browser_only(),
        )
        .define(
            Definition::string(
                mail::USERNAME,
                "",
                "The account to sign in to the SMTP server with. Empty for a server that \
                 takes mail without signing in.",
            )
            .browser_only(),
        )
        .define(Definition::secret(
            mail::PASSWORD,
            "The password of that account. Saved here, it is kept encrypted, which needs \
             secrets.keys.",
        ))
        .define(
            Definition::string(
                mail::FROM,
                "",
                "The address mail comes from, such as neon@example.com. The SMTP server must \
                 let the account send as it, and its domain needs SPF, DKIM and DMARC records \
                 for mail to arrive.",
            )
            .check(|from: &str| {
                if from.is_empty() || Email::parse(from).is_ok_and(|email| email.as_str() == from) {
                    Ok(())
                } else {
                    Err("want an address alone, such as neon@example.com".to_owned())
                }
            }),
        )
        .define(
            Definition::string(
                mail::FROM_NAME,
                "",
                "The name shown beside the From address. Empty uses instance.name.",
            )
            .check(checks::max_chars(100))
            .check(no_control),
        )
        .define(Definition::bool(
            mail::NOTIFICATIONS,
            true,
            "Mails people about activity that concerns them, while mail is set up: for now, \
             admins hear of each confirmed request to join. Off, they see requests on the \
             admin page only.",
        ))
        .rule(auth::REQUESTS.name(), |settings| {
            let mode = settings.get(auth::REGISTRATION);
            if *settings.get(auth::REQUESTS) && mode != RegistrationMode::Invite.as_str() {
                Err(format!(
                    "needs auth.registration set to invite, not {mode}: people ask for an invite"
                ))
            } else {
                Ok(())
            }
        })
        // The confirmation and the invite of a request to join go by mail.
        .rule(auth::REQUESTS.name(), |settings| {
            needs_mail(settings, *settings.get(auth::REQUESTS))
        })
        .rule(auth::VERIFY_EMAIL.name(), |settings| {
            needs_mail(settings, *settings.get(auth::VERIFY_EMAIL))
        })
        .rule(mail::TLS.name(), |settings| {
            let security = Security::parse(settings.get(mail::TLS)).unwrap_or(Security::DEFAULT);
            let host = settings.get(mail::HOST);
            if settings.get(mail::USERNAME).is_empty() || rules::may_sign_in(security, host) {
                Ok(())
            } else {
                Err(format!(
                    "is none, so the password would cross the network in the clear to {host}: \
                     use starttls or tls, or a mail server on this host"
                ))
            }
        })
        .build()
        .expect("the server's settings are valid")
}

fn needs_mail(settings: &Snapshot, on: bool) -> Result<(), String> {
    if on && !mail_set_up(settings) {
        Err(NEEDS_MAIL.to_owned())
    } else {
        Ok(())
    }
}

/// Refuses line breaks and other control characters, which have no place in a header.
fn no_control(value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        Err("must not hold line breaks or other control characters".to_owned())
    } else {
        Ok(())
    }
}

/// Refuses durations outside `least..=most`.
fn between(
    least: Duration,
    most: Duration,
) -> impl Fn(&Duration) -> Result<(), String> + Send + Sync {
    move |value| {
        if (least..=most).contains(value) {
            Ok(())
        } else {
            Err(format!(
                "must be between {} and {}",
                duration::format(least),
                duration::format(most)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use neon_domain::settings::{Pinned, Problem, Source, Stored, Value};

    use super::*;

    #[test]
    fn every_setting_resolves_to_its_default() {
        let registry = registry();
        let snapshot = registry.resolve(&Pinned::default(), &Stored::new());

        assert_eq!(snapshot.get(INSTANCE_NAME), "Neon Workspace");
        assert_eq!(snapshot.get(auth::REGISTRATION), "invite");
        assert!(!*snapshot.get(auth::VERIFY_EMAIL));
        assert!(*snapshot.get(auth::PASSWORD_RESET));
        assert!(*snapshot.get(auth::INVITE_EMAIL));
        assert!(!*snapshot.get(auth::REQUESTS));
        assert!(*snapshot.get(auth::REQUEST_DENIED_MAIL));
        assert_eq!(*snapshot.get(auth::MIN_PASSWORD_LENGTH), 12);
        assert_eq!(
            *snapshot.get(auth::INVITE_TTL),
            Duration::from_secs(7 * 24 * 3600)
        );
        assert_eq!(*snapshot.get(auth::SESSION_IDLE_TTL), session::DEFAULT_IDLE);
        assert_eq!(
            *snapshot.get(auth::SESSION_ABSOLUTE_TTL),
            session::DEFAULT_ABSOLUTE
        );
        assert!(!*snapshot.get(mail::ENABLED));
        assert_eq!(snapshot.get(mail::HOST), "");
        assert_eq!(*snapshot.get(mail::PORT), 587);
        assert_eq!(snapshot.get(mail::TLS), "starttls");
        assert_eq!(snapshot.get(mail::PASSWORD).expose(), "");
        assert!(*snapshot.get(mail::NOTIFICATIONS));
        assert!(!mail_set_up(&snapshot));
        assert!(snapshot.iter().all(|(_, r)| r.source == Source::Default));
        assert_eq!(registry.check_rules(&snapshot), Ok(()));
    }

    #[test]
    fn values_out_of_bounds_are_refused() {
        let registry = registry();
        let refuse = |key: &str, raw: &str| {
            registry
                .get(key)
                .expect("defined")
                .parse_text(raw)
                .expect_err("out of bounds")
        };

        assert_eq!(
            refuse("auth.min_password_length", "7"),
            "must be between 8 and 128"
        );
        assert_eq!(
            refuse("auth.session_idle_ttl", "4m"),
            "must be between 5m and 365d"
        );
        assert_eq!(
            refuse("auth.session_absolute_ttl", "400d"),
            "must be between 5m and 365d"
        );
        assert_eq!(
            refuse("auth.invite_ttl", "30m"),
            "must be between 1h and 365d"
        );
        assert_eq!(
            refuse("auth.registration", "anyone"),
            "want one of open, invite, closed"
        );
        assert_eq!(refuse("mail.port", "0"), "must be between 1 and 65535");
        assert_eq!(refuse("mail.tls", "ssl"), "want one of starttls, tls, none");
        assert!(refuse("mail.host", "smtp.example.com:587").starts_with("want a host alone"));
        assert_eq!(
            refuse("mail.from", "Neon <neon@example.com>"),
            "want an address alone, such as neon@example.com"
        );
        assert_eq!(
            refuse("mail.from_name", "Neon\r\nBcc: everyone@example.com"),
            "must not hold line breaks or other control characters"
        );
    }

    #[test]
    fn what_says_where_a_secret_goes_changes_only_from_a_browser() {
        let registry = registry();
        let only: Vec<&str> = registry
            .definitions()
            .filter(|def| def.is_browser_only())
            .map(|def| def.key())
            .collect();

        assert_eq!(
            only,
            [
                "mail.host",
                "mail.password",
                "mail.port",
                "mail.tls",
                "mail.username"
            ]
        );
    }

    fn check(stored: &[(&'static str, Value)]) -> Result<(), Vec<Problem>> {
        let registry = registry();
        let stored = Stored::from_iter(stored.iter().cloned());
        registry.check_rules(&registry.resolve(&Pinned::default(), &stored))
    }

    fn mail_on() -> Vec<(&'static str, Value)> {
        vec![
            ("mail.enabled", Value::Bool(true)),
            ("mail.host", Value::String("smtp.example.com".into())),
            ("mail.from", Value::String("neon@example.com".into())),
        ]
    }

    #[test]
    fn verifying_sign_ups_and_requests_to_join_need_mail() {
        for key in ["auth.verify_email", "auth.requests"] {
            assert_eq!(
                check(&[(key, Value::Bool(true))]),
                Err(vec![Problem::new(key, NEEDS_MAIL)]),
                "{key}"
            );
            let mut with_mail = mail_on();
            with_mail.push((key, Value::Bool(true)));
            assert_eq!(check(&with_mail), Ok(()), "{key}");
        }

        let mut no_host = mail_on();
        no_host.retain(|(key, _)| *key != "mail.host");
        no_host.push(("auth.verify_email", Value::Bool(true)));
        assert!(check(&no_host).is_err());
        let mut switched_off = mail_on();
        switched_off[0] = ("mail.enabled", Value::Bool(false));
        switched_off.push(("auth.verify_email", Value::Bool(true)));
        assert!(check(&switched_off).is_err());
    }

    #[test]
    fn passwords_cross_the_network_only_encrypted() {
        let plain_to = |host: &str| {
            check(&[
                ("mail.host", Value::String(host.to_owned())),
                ("mail.tls", Value::String("none".into())),
                ("mail.username", Value::String("neon".into())),
            ])
        };

        assert_eq!(plain_to("localhost"), Ok(()));
        assert_eq!(plain_to("127.0.0.1"), Ok(()));
        let refused = plain_to("smtp.example.com").expect_err("refused");
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].key, "mail.tls");
        assert!(
            refused[0]
                .message
                .contains("in the clear to smtp.example.com"),
            "{}",
            refused[0].message
        );
        assert_eq!(
            check(&[
                ("mail.host", Value::String("smtp.example.com".into())),
                ("mail.tls", Value::String("none".into())),
            ]),
            Ok(()),
            "no password, nothing to protect"
        );
    }

    #[test]
    fn requests_need_registration_by_invite() {
        let with = |mode: &str| {
            let mut stored = mail_on();
            stored.push(("auth.requests", Value::Bool(true)));
            stored.push(("auth.registration", Value::String(mode.to_owned())));
            check(&stored)
        };

        assert_eq!(with("invite"), Ok(()));
        assert_eq!(
            with("open"),
            Err(vec![Problem::new(
                "auth.requests",
                "needs auth.registration set to invite, not open: people ask for an invite"
            )])
        );
        assert!(with("closed").is_err());
    }
}
