use std::time::Duration;

use neon_domain::auth::invite::MAX_LIFETIME;
use neon_domain::auth::password::{DEFAULT_MIN_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH_BOUNDS};
use neon_domain::auth::{RegistrationMode, session};
use neon_domain::duration;
use neon_domain::settings::{Definition, Key, Registry, checks};

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

/// The longest a session may last, either way.
const MAX_SESSION_TTL: Duration = Duration::from_secs(365 * 24 * 3600);

/// Every runtime setting the server reads.
///
/// Rules that need mail set up, verifying sign-ups and requests to join among them, come
/// with the mail settings. Until then those flows check at run time that a mail transport
/// is configured, and stay off without one.
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
             Works while mail is set up.",
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
             which mails them an invite, or deny. Works while mail is set up.",
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
        .build()
        .expect("the server's settings are valid")
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
    }

    #[test]
    fn requests_need_registration_by_invite() {
        let registry = registry();
        let with = |mode: &str| {
            let stored = Stored::from([
                ("auth.requests", Value::Bool(true)),
                ("auth.registration", Value::String(mode.to_owned())),
            ]);
            registry.check_rules(&registry.resolve(&Pinned::default(), &stored))
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
