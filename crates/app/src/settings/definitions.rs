use std::time::Duration;

use neon_domain::agents::IdentityMode;
use neon_domain::auth::invite::MAX_LIFETIME;
use neon_domain::auth::password::{DEFAULT_MIN_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH_BOUNDS};
use neon_domain::auth::{Email, RegistrationMode, session};
use neon_domain::duration;
use neon_domain::mail::{self as rules, Security};
use neon_domain::oauth::{
    ACCESS_TTL_BOUNDS, DEFAULT_ACCESS_TTL, DEFAULT_REFRESH_TTL, REFRESH_TTL_BOUNDS,
};
use neon_domain::settings::{Definition, Key, Registry, Snapshot, checks};
use neon_domain::workspaces::WorkspaceCreation;

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

/// Workspaces: who creates them, and the limits each takes unless an instance admin sets
/// its own.
pub mod workspaces {
    use neon_domain::settings::Key;

    /// Who may create a workspace: `anyone` or `admins`.
    pub const CREATION: Key<str> = Key::new("workspaces.creation");
    /// The most people a workspace holds, guests included; 0 is no limit.
    pub const LIMIT_MEMBERS: Key<i64> = Key::new("workspaces.limits.members");
    /// The most bytes a workspace's attachments take; 0 is no limit.
    pub const LIMIT_STORAGE_BYTES: Key<i64> = Key::new("workspaces.limits.storage_bytes");
    /// The most agents and integrations a workspace has; 0 is no limit.
    pub const LIMIT_AGENTS: Key<i64> = Key::new("workspaces.limits.agents");
}

/// MCP: whom clients act as, and so whether agents exist, and the bounds of their
/// sessions.
pub mod mcp {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// Whom MCP clients may act as: the person who connects them, `user`, one of their
    /// agents, `agent`, or both.
    pub const IDENTITIES: Key<[String]> = Key::new("mcp.identities");
    /// How many MCP sessions this server holds at once, how many one identity may, and how
    /// many one person may with their agents.
    pub const MAX_SESSIONS: Key<i64> = Key::new("mcp.max_sessions");
    pub const MAX_SESSIONS_PER_IDENTITY: Key<i64> = Key::new("mcp.max_sessions_per_identity");
    pub const MAX_SESSIONS_PER_PERSON: Key<i64> = Key::new("mcp.max_sessions_per_person");
    /// How long a session without requests or open streams lasts.
    pub const SESSION_IDLE_TIMEOUT: Key<Duration> = Key::new("mcp.session_idle_timeout");
    /// How often, at most, a session hears that one resource it subscribes to changed.
    pub const NOTIFICATION_INTERVAL: Key<Duration> = Key::new("mcp.notification_interval");
}

/// The OAuth server MCP clients and other programs connect through.
pub mod oauth {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// Clients may register themselves (RFC 7591).
    pub const REGISTRATION: Key<bool> = Key::new("oauth.registration");
    /// How long an access token works.
    pub const ACCESS_TOKEN_TTL: Key<Duration> = Key::new("oauth.access_token_ttl");
    /// How long a refresh token works, if not exchanged first.
    pub const REFRESH_TOKEN_TTL: Key<Duration> = Key::new("oauth.refresh_token_ttl");
    /// The most clients without a grant there may be at once.
    pub const MAX_UNUSED_CLIENTS: Key<i64> = Key::new("oauth.max_unused_clients");
    /// The most clients without a grant that registered from one source there may be.
    pub const MAX_UNUSED_CLIENTS_PER_SOURCE: Key<i64> =
        Key::new("oauth.max_unused_clients_per_source");
    /// How long a client without a grant is kept after it registered or was last used.
    pub const UNUSED_CLIENT_TTL: Key<Duration> = Key::new("oauth.unused_client_ttl");
}

/// Outgoing webhooks: where they may go.
pub mod webhooks {
    use neon_domain::settings::Key;

    /// Webhooks may reach the addresses of a private network: loopback, private, shared,
    /// link-local and unique-local ones.
    pub const ALLOW_PRIVATE: Key<bool> = Key::new("webhooks.allow_private");
}

/// Attachments: where their bytes are kept, how large one may be, and how long one that no
/// page shows any more is kept.
pub mod attachments {
    use std::time::Duration;

    use neon_domain::settings::{Key, Secret};

    /// Where files are kept: `local`, a directory under the server's data directory, or
    /// `s3`, a bucket of an S3-compatible store.
    pub const STORAGE: Key<str> = Key::new("attachments.storage");
    /// The S3-compatible store's URL; empty for Amazon S3.
    pub const S3_ENDPOINT: Key<str> = Key::new("attachments.s3.endpoint");
    pub const S3_BUCKET: Key<str> = Key::new("attachments.s3.bucket");
    pub const S3_REGION: Key<str> = Key::new("attachments.s3.region");
    pub const S3_ACCESS_KEY_ID: Key<str> = Key::new("attachments.s3.access_key_id");
    pub const S3_SECRET_ACCESS_KEY: Key<Secret> = Key::new("attachments.s3.secret_access_key");
    /// Buckets in the URL's path rather than its host.
    pub const S3_PATH_STYLE: Key<bool> = Key::new("attachments.s3.path_style");
    /// The largest file one upload takes, in bytes.
    pub const MAX_FILE_BYTES: Key<i64> = Key::new("attachments.max_file_bytes");
    /// How long a file no page's document, cover or kept version shows is kept before it is
    /// deleted.
    pub const ORPHAN_GRACE: Key<Duration> = Key::new("attachments.orphan_grace");
}

/// What `attachments.storage` may be.
pub const STORAGE_OPTIONS: &[&str] = &["local", "s3"];

/// The bounds of the largest file an upload takes: 1 KiB to 5 GiB.
const MAX_FILE_BYTES_BOUNDS: (i64, i64) = (1024, 5 * 1024 * 1024 * 1024);

/// The shortest and longest a file no page shows is kept.
const MIN_ORPHAN_GRACE: Duration = Duration::from_secs(3600);
const MAX_ORPHAN_GRACE: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// Pages: how long the trash keeps them.
pub mod pages {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// How long a page stays in the trash before the hourly cleanup deletes it for good.
    pub const TRASH_RETENTION: Key<Duration> = Key::new("pages.trash_retention");
}

/// Page history: which versions of each page the hourly cleanup keeps.
pub mod history {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// How long every version of a page is kept.
    pub const KEEP_ALL: Key<Duration> = Key::new("history.keep_all");
    /// Until how old the latest version of each day is kept; older, that of each week.
    pub const KEEP_DAILY: Key<Duration> = Key::new("history.keep_daily");
    /// The most versions a page keeps, the latest.
    pub const MAX_VERSIONS: Key<i64> = Key::new("history.max_versions");
}

/// The inbox's mail: whether people get it, how soon after a notification, and how often a
/// digest.
pub mod notifications {
    use std::time::Duration;

    use neon_domain::settings::Key;

    /// People get mail of their inbox, each as their preference says.
    pub const MAIL: Key<bool> = Key::new("notifications.mail");
    /// How long a notification waits unread before its own mail goes.
    pub const MAIL_DELAY: Key<Duration> = Key::new("notifications.mail_delay");
    /// How often, at most, someone who wants a digest gets one.
    pub const DIGEST_INTERVAL: Key<Duration> = Key::new("notifications.digest_interval");
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

/// The bounds of how long an unused OAuth client is kept, and of how many there may be.
const UNUSED_CLIENT_TTL_BOUNDS: (Duration, Duration) = (
    Duration::from_secs(3600),
    Duration::from_secs(90 * 24 * 3600),
);
const MAX_UNUSED_CLIENTS_BOUNDS: (i64, i64) = (1, 100_000);

/// Checks a bound on unused OAuth clients against [`MAX_UNUSED_CLIENTS_BOUNDS`].
fn within_unused_clients_bounds(n: &i64) -> Result<(), String> {
    let (least, most) = MAX_UNUSED_CLIENTS_BOUNDS;
    if (least..=most).contains(n) {
        Ok(())
    } else {
        Err(format!("must be between {least} and {most}"))
    }
}

/// The shortest and longest the trash keeps pages.
const MIN_TRASH_RETENTION: Duration = Duration::from_secs(3600);
const MAX_TRASH_RETENTION: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// The shortest and longest a page's versions are kept whole, and one a day.
const HISTORY_BOUNDS: (Duration, Duration) = (
    Duration::from_secs(3600),
    Duration::from_secs(10 * 365 * 24 * 3600),
);

/// Every runtime setting the server reads.
///
/// Verifying sign-ups and requests to join need mail set up, by rule: on without it, they
/// would turn people away unseen. Password resets and mailed invites are on by default and
/// work while mail is set up, so a rule would refuse the defaults; the flows check at run
/// time instead. Requests to join also need registration by invite, which they check at
/// run time too: a rule would refuse closing registration while they are on. Every flow a
/// rule guards checks again at run time, as saved values that break a rule only warn when
/// the server starts.
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
                "Neon Teamspace",
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
        .define(Definition::enumeration(
            workspaces::CREATION,
            WorkspaceCreation::OPTIONS,
            WorkspaceCreation::DEFAULT.as_str(),
            "Who may create a workspace: every account (anyone), or instance admins only \
             (admins). Workspaces made before a change stay.",
        ))
        .define(
            Definition::integer(
                workspaces::LIMIT_MEMBERS,
                0,
                "The most people a workspace holds, guests included, unless an instance admin \
                 sets its own limit; 0 is no limit. A workspace at its limit refuses \
                 invitations, accepting them and joining by link; nobody in it is removed.",
            )
            .check(not_negative),
        )
        .define(
            Definition::integer(
                workspaces::LIMIT_STORAGE_BYTES,
                0,
                "The most bytes a workspace's attachments take together, unless an instance \
                 admin sets its own limit; 0 is no limit. A workspace at its limit refuses \
                 uploads, and imports that bring files; nothing in it is removed, and its \
                 pages stay editable.",
            )
            .check(not_negative),
        )
        .define(
            Definition::integer(
                workspaces::LIMIT_AGENTS,
                0,
                "The most agents and integrations a workspace has together, unless an \
                 instance admin sets its own limit; 0 is no limit. A workspace at its limit \
                 makes no more; none it has is removed. People count against \
                 workspaces.limits.members instead.",
            )
            .check(not_negative),
        )
        .define(Definition::set(
            mcp::IDENTITIES,
            IdentityMode::OPTIONS,
            IdentityMode::DEFAULT,
            "Whom MCP clients may act as: the person who connects them (user), one of that \
             person's agents (agent), or both. Without user, /mcp refuses people's tokens, \
             OAuth's consent offers no acting as oneself, and the OAuth connections acting as \
             people stop. Without agent, nobody makes agents and every agent's API tokens and \
             OAuth connections are refused. What either leaves out works again once it is \
             back.",
        ))
        .define(Definition::bool(
            oauth::REGISTRATION,
            true,
            "MCP clients and other programs may register themselves with the OAuth server \
             (RFC 7591 dynamic client registration), as they do before a person first \
             connects them. Off, none registers; those registered keep working.",
        ))
        .define(
            Definition::duration(
                oauth::ACCESS_TOKEN_TTL,
                DEFAULT_ACCESS_TTL,
                "How long an OAuth access token works. A client gets a new one with its \
                 refresh token. Applies to tokens issued from now.",
            )
            .check(between(ACCESS_TTL_BOUNDS.0, ACCESS_TTL_BOUNDS.1)),
        )
        .define(
            Definition::duration(
                oauth::REFRESH_TOKEN_TTL,
                DEFAULT_REFRESH_TTL,
                "How long an OAuth refresh token works. Each works once, and is replaced by \
                 a new one as the client uses it; a client unused this long is connected \
                 again. Longer than oauth.access_token_ttl. Applies to tokens issued from \
                 now.",
            )
            .check(between(REFRESH_TTL_BOUNDS.0, REFRESH_TTL_BOUNDS.1)),
        )
        .define(
            Definition::integer(
                oauth::MAX_UNUSED_CLIENTS,
                1000,
                "The most OAuth clients that nobody has connected there may be at once, as \
                 anyone may register one. Past it, registering waits until the hourly \
                 cleanup removes some; clients people connected never count.",
            )
            .check(within_unused_clients_bounds),
        )
        .define(
            Definition::integer(
                oauth::MAX_UNUSED_CLIENTS_PER_SOURCE,
                20,
                "The most OAuth clients that nobody has connected there may be from one \
                 place at once: one IPv4 address, or one /64 network of IPv6 addresses. Past \
                 it, that place registers no more until some are connected or the hourly \
                 cleanup removes them, so that no one place takes all of \
                 oauth.max_unused_clients.",
            )
            .check(within_unused_clients_bounds),
        )
        .define(
            Definition::duration(
                oauth::UNUSED_CLIENT_TTL,
                Duration::from_secs(24 * 3600),
                "How long an OAuth client that nobody connected, or whose connections all \
                 ended, is kept after it registered or was last used, before the hourly \
                 cleanup removes it.",
            )
            .check(between(
                UNUSED_CLIENT_TTL_BOUNDS.0,
                UNUSED_CLIENT_TTL_BOUNDS.1,
            )),
        )
        .define(
            Definition::integer(
                mcp::MAX_SESSIONS,
                1000,
                "The most MCP sessions this server holds at once, of every identity together. \
                 One more is refused until one ends: a client closes it, it idles out, or what \
                 it acts as stops working.",
            )
            .check(between_counts(1, 100_000)),
        )
        .define(
            Definition::integer(
                mcp::MAX_SESSIONS_PER_IDENTITY,
                10,
                "The most MCP sessions one person or agent holds at once on this server. One \
                 more ends their session used least recently that has no request or stream \
                 open, or is refused while each of theirs is in use.",
            )
            .check(between_counts(1, 1000)),
        )
        .define(
            Definition::integer(
                mcp::MAX_SESSIONS_PER_PERSON,
                20,
                "The most MCP sessions one person holds at once on this server, with those of \
                 their agents together, so nobody takes every session through agents of \
                 theirs. One more is refused until one of theirs ends.",
            )
            .check(between_counts(1, 10_000)),
        )
        .define(
            Definition::duration(
                mcp::SESSION_IDLE_TIMEOUT,
                Duration::from_secs(30 * 60),
                "How long an MCP session lasts without a request and without an open stream, \
                 then ends with its subscriptions. A client starts a new one when it comes \
                 back. A change applies to every session at once.",
            )
            .check(between(
                Duration::from_secs(1),
                Duration::from_secs(24 * 3600),
            )),
        )
        .define(
            Definition::duration(
                mcp::NOTIFICATION_INTERVAL,
                Duration::from_secs(2),
                "How often, at most, an MCP session is told that one resource it subscribes to \
                 changed. Changes meanwhile are told together, once the interval is over.",
            )
            .check(between(
                Duration::from_millis(100),
                Duration::from_secs(3600),
            )),
        )
        .define(
            Definition::bool(
                webhooks::ALLOW_PRIVATE,
                false,
                "Lets outgoing webhooks reach the addresses of a private network: loopback \
                 (127.0.0.0/8, ::1), private (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), \
                 shared (100.64.0.0/10), link-local (169.254.0.0/16, fe80::/10) and \
                 unique-local (fc00::/7) ones, such as a service beside the server. Off, a \
                 webhook whose host resolves to one is refused when it is registered and at \
                 every delivery. Cloud metadata, multicast, broadcast, reserved and \
                 unspecified addresses stay refused either way.",
            )
            .browser_only(),
        )
        .define(
            Definition::duration(
                pages::TRASH_RETENTION,
                Duration::from_secs(30 * 24 * 3600),
                "How long a page stays in the trash, with the pages trashed along with it, \
                 before the hourly cleanup deletes it for good. Until then, those who could \
                 edit it restore it. Applies to the whole trash at the next cleanup.",
            )
            .check(between(MIN_TRASH_RETENTION, MAX_TRASH_RETENTION)),
        )
        .define(
            Definition::duration(
                history::KEEP_ALL,
                Duration::from_secs(7 * 24 * 3600),
                "How long every version of a page is kept. A version is the page's content \
                 each time the server saves it changed. Older versions thin out to one a day, \
                 then one a week; the hourly cleanup applies it to every page.",
            )
            .check(between(HISTORY_BOUNDS.0, HISTORY_BOUNDS.1)),
        )
        .define(
            Definition::duration(
                history::KEEP_DAILY,
                Duration::from_secs(90 * 24 * 3600),
                "Until how old a page keeps the latest version of each day, once older than \
                 history.keep_all; older still, it keeps that of each week, Monday to Sunday, \
                 UTC's days and weeks. At least history.keep_all.",
            )
            .check(between(HISTORY_BOUNDS.0, HISTORY_BOUNDS.1)),
        )
        .define(
            Definition::integer(
                history::MAX_VERSIONS,
                500,
                "The most versions a page keeps, the latest, however young; the latest one \
                 always stays. A page that keeps one more is thinned at once, as the cleanup \
                 would thin it.",
            )
            .check(between_counts(1, 100_000)),
        )
        .define(Definition::bool(
            notifications::MAIL,
            false,
            "Mails people of their inbox: of each notification still unread a while after it \
             came, those that come close together in one mail, or a digest every few hours, as \
             each person's preference says, and never what they may no longer see. Every mail \
             has a link that stops it without signing in. Needs mail set up.",
        ))
        .define(
            Definition::duration(
                notifications::MAIL_DELAY,
                Duration::from_secs(5 * 60),
                "How long a notification waits unread before the mail of it goes, to those \
                 who want a mail for each: one read meanwhile, in the app or over MCP, is not \
                 mailed. Those that come while a mail waits go in it once they too have \
                 waited, or in the next, which goes 15 minutes after the last at the soonest.",
            )
            .check(between(Duration::ZERO, Duration::from_secs(24 * 3600))),
        )
        .define(
            Definition::duration(
                notifications::DIGEST_INTERVAL,
                Duration::from_secs(3 * 3600),
                "How often, at most, those who want a digest get one, of what came to their \
                 inbox since the last and is still unread.",
            )
            .check(between(
                Duration::from_secs(3600),
                Duration::from_secs(7 * 24 * 3600),
            )),
        )
        .define(
            Definition::enumeration(
                attachments::STORAGE,
                STORAGE_OPTIONS,
                "local",
                "Where uploaded files are kept: local, in the attachments directory under \
                 server.data_dir, or s3, in the bucket attachments.s3.bucket of an \
                 S3-compatible store such as SeaweedFS. Applies to files uploaded from now; \
                 the files kept before stay where they were, so move them with it.",
            )
            .browser_only(),
        )
        .define(
            Definition::string(
                attachments::S3_ENDPOINT,
                "",
                "The S3-compatible store's URL, such as http://seaweedfs:8333; empty for \
                 Amazon S3, whose endpoint comes from attachments.s3.region. http is for a \
                 store on a network you trust.",
            )
            .check(check_endpoint)
            .browser_only(),
        )
        .define(
            Definition::string(
                attachments::S3_BUCKET,
                "",
                "The bucket files are kept in, which must exist. Needed while \
                 attachments.storage is s3.",
            )
            .check(checks::max_chars(63))
            .check(no_control)
            .browser_only(),
        )
        .define(
            Definition::string(
                attachments::S3_REGION,
                "us-east-1",
                "The bucket's region. Stores other than Amazon's take any, usually \
                 us-east-1.",
            )
            .check(checks::non_blank)
            .check(checks::max_chars(64))
            .browser_only(),
        )
        .define(
            Definition::string(
                attachments::S3_ACCESS_KEY_ID,
                "",
                "The access key that signs requests to the store, with \
                 attachments.s3.secret_access_key.",
            )
            .check(no_control)
            .browser_only(),
        )
        .define(Definition::secret(
            attachments::S3_SECRET_ACCESS_KEY,
            "The secret of that access key. Saved here, it is kept encrypted, which needs \
             secrets.keys.",
        ))
        .define(
            Definition::bool(
                attachments::S3_PATH_STYLE,
                true,
                "Names the bucket in the URL's path, http://store/bucket/key, as SeaweedFS, \
                 MinIO and most other stores want. Off, it names it in the host, \
                 http://bucket.store/key, as Amazon S3 prefers.",
            )
            .browser_only(),
        )
        .define(
            Definition::integer(
                attachments::MAX_FILE_BYTES,
                50 * 1024 * 1024,
                "The largest file one upload takes, in bytes. A larger one is refused as \
                 it arrives; files uploaded before stay.",
            )
            .check(between_counts(
                MAX_FILE_BYTES_BOUNDS.0,
                MAX_FILE_BYTES_BOUNDS.1,
            )),
        )
        .define(
            Definition::duration(
                attachments::ORPHAN_GRACE,
                Duration::from_secs(30 * 24 * 3600),
                "How long a file that no page's content, no cover and no version the page \
                 history keeps shows any more is kept, from when the hourly cleanup first \
                 finds it so, before it is deleted for good. Undoing the removal within it \
                 keeps the file; restoring a kept version always does.",
            )
            .check(between(MIN_ORPHAN_GRACE, MAX_ORPHAN_GRACE)),
        )
        // An S3 store needs a bucket.
        .rule(attachments::S3_BUCKET.name(), |settings| {
            if settings.get(attachments::STORAGE) == "s3"
                && settings.get(attachments::S3_BUCKET).is_empty()
            {
                Err("must name a bucket while attachments.storage is s3".to_owned())
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
        .rule(notifications::MAIL.name(), |settings| {
            needs_mail(settings, *settings.get(notifications::MAIL))
        })
        // A refresh token that ends first would leave nothing to renew the access token with.
        .rule(oauth::REFRESH_TOKEN_TTL.name(), |settings| {
            let access = *settings.get(oauth::ACCESS_TOKEN_TTL);
            if *settings.get(oauth::REFRESH_TOKEN_TTL) > access {
                Ok(())
            } else {
                Err(format!(
                    "must be longer than oauth.access_token_ttl, {}",
                    duration::format(access)
                ))
            }
        })
        // A version is kept whole before it is thinned to one a day.
        .rule(history::KEEP_DAILY.name(), |settings| {
            let all = *settings.get(history::KEEP_ALL);
            if *settings.get(history::KEEP_DAILY) >= all {
                Ok(())
            } else {
                Err(format!(
                    "must be at least history.keep_all, {}",
                    duration::format(all)
                ))
            }
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

/// Refuses an S3 endpoint that is not an http or https URL without a path, query or
/// credentials; empty is Amazon's.
fn check_endpoint(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    let refused =
        || "want an http or https URL of the store alone, such as http://seaweedfs:8333".to_owned();
    let url = url::Url::parse(value).map_err(|_| refused())?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/");
    if plain { Ok(()) } else { Err(refused()) }
}

/// Refuses limits below 0, which is no limit.
fn not_negative(value: &i64) -> Result<(), String> {
    if *value >= 0 {
        Ok(())
    } else {
        Err("must be 0, for no limit, or more".to_owned())
    }
}

/// Refuses counts outside `least..=most`.
fn between_counts(least: i64, most: i64) -> impl Fn(&i64) -> Result<(), String> + Send + Sync {
    move |value| {
        if (least..=most).contains(value) {
            Ok(())
        } else {
            Err(format!("must be between {least} and {most}"))
        }
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

        assert_eq!(snapshot.get(INSTANCE_NAME), "Neon Teamspace");
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
        assert_eq!(snapshot.get(workspaces::CREATION), "anyone");
        assert_eq!(*snapshot.get(workspaces::LIMIT_MEMBERS), 0);
        assert_eq!(*snapshot.get(workspaces::LIMIT_STORAGE_BYTES), 0);
        assert_eq!(*snapshot.get(workspaces::LIMIT_AGENTS), 0);
        assert_eq!(snapshot.get(mcp::IDENTITIES), ["user", "agent"]);
        assert_eq!(*snapshot.get(mcp::MAX_SESSIONS), 1000);
        assert_eq!(*snapshot.get(mcp::MAX_SESSIONS_PER_IDENTITY), 10);
        assert_eq!(*snapshot.get(mcp::MAX_SESSIONS_PER_PERSON), 20);
        assert_eq!(
            *snapshot.get(mcp::SESSION_IDLE_TIMEOUT),
            Duration::from_secs(1800)
        );
        assert_eq!(
            *snapshot.get(mcp::NOTIFICATION_INTERVAL),
            Duration::from_secs(2)
        );
        assert!(*snapshot.get(oauth::REGISTRATION));
        assert_eq!(
            *snapshot.get(oauth::ACCESS_TOKEN_TTL),
            Duration::from_secs(3600)
        );
        assert_eq!(
            *snapshot.get(oauth::REFRESH_TOKEN_TTL),
            Duration::from_secs(30 * 24 * 3600)
        );
        assert_eq!(*snapshot.get(oauth::MAX_UNUSED_CLIENTS), 1000);
        assert_eq!(*snapshot.get(oauth::MAX_UNUSED_CLIENTS_PER_SOURCE), 20);
        assert_eq!(
            *snapshot.get(oauth::UNUSED_CLIENT_TTL),
            Duration::from_secs(24 * 3600)
        );
        assert!(!*snapshot.get(webhooks::ALLOW_PRIVATE));
        assert_eq!(snapshot.get(attachments::STORAGE), "local");
        assert_eq!(snapshot.get(attachments::S3_ENDPOINT), "");
        assert_eq!(snapshot.get(attachments::S3_BUCKET), "");
        assert_eq!(snapshot.get(attachments::S3_REGION), "us-east-1");
        assert_eq!(snapshot.get(attachments::S3_ACCESS_KEY_ID), "");
        assert_eq!(snapshot.get(attachments::S3_SECRET_ACCESS_KEY).expose(), "");
        assert!(*snapshot.get(attachments::S3_PATH_STYLE));
        assert_eq!(*snapshot.get(attachments::MAX_FILE_BYTES), 50 * 1024 * 1024);
        assert_eq!(
            *snapshot.get(attachments::ORPHAN_GRACE),
            Duration::from_secs(30 * 24 * 3600)
        );
        assert_eq!(
            *snapshot.get(pages::TRASH_RETENTION),
            Duration::from_secs(30 * 24 * 3600)
        );
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
        assert_eq!(
            refuse("workspaces.creation", "members"),
            "want one of anyone, admins"
        );
        for limit in [
            "workspaces.limits.members",
            "workspaces.limits.storage_bytes",
            "workspaces.limits.agents",
        ] {
            assert_eq!(
                refuse(limit, "-1"),
                "must be 0, for no limit, or more",
                "{limit}"
            );
        }
        assert_eq!(
            refuse("pages.trash_retention", "30m"),
            "must be between 1h and 3650d"
        );
        assert_eq!(
            refuse("oauth.access_token_ttl", "4m"),
            "must be between 5m and 1d"
        );
        assert_eq!(
            refuse("oauth.access_token_ttl", "25h"),
            "must be between 5m and 1d"
        );
        assert_eq!(
            refuse("oauth.refresh_token_ttl", "59m"),
            "must be between 1h and 90d"
        );
        assert_eq!(
            refuse("oauth.refresh_token_ttl", "91d"),
            "must be between 1h and 90d"
        );
        assert_eq!(
            refuse("oauth.max_unused_clients", "0"),
            "must be between 1 and 100000"
        );
        assert_eq!(
            refuse("oauth.max_unused_clients_per_source", "0"),
            "must be between 1 and 100000"
        );
        assert_eq!(
            refuse("oauth.unused_client_ttl", "30m"),
            "must be between 1h and 90d"
        );
        assert_eq!(refuse("mail.tls", "ssl"), "want one of starttls, tls, none");
        assert_eq!(
            refuse("attachments.storage", "disk"),
            "want one of local, s3"
        );
        assert_eq!(
            refuse("attachments.max_file_bytes", "1023"),
            "must be between 1024 and 5368709120"
        );
        assert_eq!(
            refuse("attachments.orphan_grace", "30m"),
            "must be between 1h and 3650d"
        );
        for endpoint in [
            "seaweedfs:8333",
            "ftp://store.example.com",
            "https://key:secret@store.example.com",
            "https://store.example.com/bucket",
            "https://store.example.com/?x=1",
        ] {
            assert!(
                refuse("attachments.s3.endpoint", endpoint).starts_with("want an http"),
                "{endpoint}"
            );
        }
        for endpoint in ["http://seaweedfs:8333", "https://s3.example.com/", ""] {
            assert!(
                registry
                    .get("attachments.s3.endpoint")
                    .expect("defined")
                    .parse_text(endpoint)
                    .is_ok(),
                "{endpoint}"
            );
        }
        assert_eq!(
            refuse("mcp.identities", "user,people"),
            "want a list of some of user, agent"
        );
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
                "attachments.s3.access_key_id",
                "attachments.s3.bucket",
                "attachments.s3.endpoint",
                "attachments.s3.path_style",
                "attachments.s3.region",
                "attachments.s3.secret_access_key",
                "attachments.storage",
                "mail.host",
                "mail.password",
                "mail.port",
                "mail.tls",
                "mail.username",
                "webhooks.allow_private"
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
    fn a_refresh_token_outlives_its_access_token() {
        assert_eq!(
            check(&[
                (
                    "oauth.access_token_ttl",
                    Value::Duration(Duration::from_secs(2 * 3600))
                ),
                (
                    "oauth.refresh_token_ttl",
                    Value::Duration(Duration::from_secs(2 * 3600))
                ),
            ]),
            Err(vec![Problem::new(
                "oauth.refresh_token_ttl",
                "must be longer than oauth.access_token_ttl, 2h"
            )])
        );
        assert_eq!(
            check(&[
                (
                    "oauth.access_token_ttl",
                    Value::Duration(Duration::from_secs(2 * 3600))
                ),
                (
                    "oauth.refresh_token_ttl",
                    Value::Duration(Duration::from_secs(3 * 3600))
                ),
            ]),
            Ok(())
        );
    }

    #[test]
    fn an_s3_store_needs_a_bucket() {
        let s3 = ("attachments.storage", Value::String("s3".into()));
        assert_eq!(
            check(std::slice::from_ref(&s3)),
            Err(vec![Problem::new(
                "attachments.s3.bucket",
                "must name a bucket while attachments.storage is s3"
            )])
        );
        assert_eq!(
            check(&[s3, ("attachments.s3.bucket", Value::String("neon".into()))]),
            Ok(())
        );
        assert_eq!(check(&[]), Ok(()), "local disk needs none");
    }

    #[test]
    fn requests_to_join_leave_the_registration_mode_free() {
        for mode in ["open", "invite", "closed"] {
            let mut stored = mail_on();
            stored.push(("auth.requests", Value::Bool(true)));
            stored.push(("auth.registration", Value::String(mode.to_owned())));
            assert_eq!(check(&stored), Ok(()), "{mode}");
        }
    }
}
