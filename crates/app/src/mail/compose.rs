//! Writing mail: what each [`Message`] says, as a [`Letter`] of a subject, paragraphs and
//! a link, and the letter as plain text and as HTML in the app's look, black on white with
//! the accent on the button. Everything a letter holds is escaped in the HTML.

use std::fmt::Write as _;
use std::time::Duration;

use neon_domain::auth::email_token::UNVERIFIED_TTL;
use neon_domain::auth::join_request::DENIED_WAIT;
use neon_domain::auth::preferences::DEFAULT_ACCENT_HUE;
use time::OffsetDateTime;
use url::form_urlencoded;

use super::Message;
use super::color;

/// The app's page that verifies an address, given `?token=`.
pub const VERIFY_PAGE: &str = "/verify";
/// The app's page that sets a new password, given `?token=`. Without one, it asks for an
/// address to mail a link to.
pub const RESET_PAGE: &str = "/reset";
/// The app's sign-up page, given `?invite=` with an invite code.
pub const REGISTER_PAGE: &str = "/register";
/// The app's page that confirms a request to join, given `?token=`.
pub const JOIN_CONFIRM_PAGE: &str = "/join/confirm";
/// The admin page listing requests to join.
pub const JOIN_REQUESTS_PAGE: &str = "/admin/join-requests";

/// What a mail says: its subject, its paragraphs of plain text, and a link to act on.
#[derive(Clone, PartialEq, Eq)]
pub struct Letter {
    pub subject: String,
    pub paragraphs: Vec<String>,
    pub link: Option<Link>,
}

/// A button in the HTML, and a line in the text.
#[derive(Clone, PartialEq, Eq)]
pub struct Link {
    pub text: String,
    pub url: String,
}

/// Who mail comes from, as letters name them.
#[derive(Debug, Clone, Copy)]
pub struct Sender<'a> {
    /// The instance's name.
    pub name: &'a str,
    /// The origin of the public URL, which links start with, such as
    /// `https://neon.example.com`.
    pub origin: &'a str,
}

impl Sender<'_> {
    fn page(&self, path: &str, query: Option<(&str, &str)>) -> String {
        let mut url = format!("{}{path}", self.origin);
        if let Some((name, value)) = query {
            let value: String = form_urlencoded::byte_serialize(value.as_bytes()).collect();
            let _ = write!(url, "?{name}={value}");
        }
        url
    }

    fn footer(&self) -> String {
        format!("{}, {}", self.name, self.origin)
    }
}

/// What `message` says, from `sender`.
pub fn write(message: &Message, sender: &Sender<'_>) -> Letter {
    let name = sender.name;
    let link = |text: &str, url: String| {
        Some(Link {
            text: text.to_owned(),
            url,
        })
    };
    let (subject, paragraphs, link) = match message {
        Message::Verification { token, lasts } => (
            format!("Verify your email for {name}"),
            vec![
                format!(
                    "Someone, hopefully you, made a {name} account with this address. Open \
                     the link to verify it, then sign in."
                ),
                format!(
                    "It works for {}. Did not sign up? Ignore this mail: the account is \
                     removed within {}.",
                    for_people(*lasts),
                    for_people(UNVERIFIED_TTL)
                ),
            ],
            link(
                "Verify my address",
                sender.page(VERIFY_PAGE, Some(("token", token.expose()))),
            ),
        ),
        Message::PasswordReset { token, lasts } => (
            format!("Set a new {name} password"),
            vec![
                format!(
                    "Someone, hopefully you, asked to set a new password for the {name} \
                     account of this address."
                ),
                format!(
                    "The link works once, for {}. Setting the password signs the account \
                     out everywhere. Did not ask? Ignore this mail: your password stays as \
                     it is.",
                    for_people(*lasts)
                ),
            ],
            link(
                "Set a new password",
                sender.page(RESET_PAGE, Some(("token", token.expose()))),
            ),
        ),
        Message::PasswordChanged { at } => (
            format!("Your {name} password was changed"),
            vec![
                format!(
                    "The password of your {name} account was changed on {}.",
                    when(*at)
                ),
                "If that was you, there is nothing to do. If not, set a new one now and \
                 tell an admin: someone may know your old password."
                    .to_owned(),
            ],
            link("Set a new password", sender.page(RESET_PAGE, None)),
        ),
        Message::Invite {
            inviter_name,
            inviter_email,
            code,
            expires_at,
        } => (
            format!("{inviter_name} invited you to {name}"),
            vec![
                format!(
                    "{inviter_name} ({inviter_email}) invited you to {name}, at {}.",
                    sender.origin
                ),
                format!(
                    "Open the link to make your account with this address. It works once, \
                     until {}. Not expecting it? Ignore this mail.",
                    when(*expires_at)
                ),
            ],
            link(
                "Make my account",
                sender.page(REGISTER_PAGE, Some(("invite", code.expose()))),
            ),
        ),
        Message::JoinConfirmation { token, lasts } => (
            format!("Confirm your request to join {name}"),
            vec![
                format!(
                    "Someone, hopefully you, asked to join {name} with this address. Open the \
                     link to confirm it: the admins then see your request, and mail you an \
                     invite here if they approve it."
                ),
                format!(
                    "It works for {}. Did not ask? Ignore this mail: nothing is sent to the \
                     admins.",
                    for_people(*lasts)
                ),
            ],
            link(
                "Confirm my request",
                sender.page(JOIN_CONFIRM_PAGE, Some(("token", token.expose()))),
            ),
        ),
        Message::JoinDenied { ask_again_after } => (
            format!("Your request to join {name}"),
            vec![format!(
                "The admins of {name} did not approve your request to join this time. You \
                 may ask again in {}.",
                for_people(*ask_again_after)
            )],
            None,
        ),
        Message::JoinRequested { requester, note } => {
            let mut paragraphs = vec![format!(
                "{requester} asks to join {name}, and confirmed the address. Approving the \
                 request mails them an invite; denying it lets them ask again in {}.",
                for_people(DENIED_WAIT)
            )];
            if !note.trim().is_empty() {
                paragraphs.push(format!("They say: “{}”", note.trim()));
            }
            (
                format!("{requester} asks to join {name}"),
                paragraphs,
                link("Review the request", sender.page(JOIN_REQUESTS_PAGE, None)),
            )
        }
        Message::Test { sent_by } => (
            format!("{name} test email"),
            vec![
                format!(
                    "This is a test email from {name}, sent from the admin settings by \
                     {sent_by}."
                ),
                format!("It came through, so {name} can send mail through this server."),
            ],
            None,
        ),
    };
    Letter {
        subject: one_line(&subject),
        paragraphs,
        link,
    }
}

impl Letter {
    /// The plain text part: the paragraphs, the link on a line of its own, and a signature.
    pub fn text(&self, sender: &Sender<'_>) -> String {
        let mut text = String::new();
        for paragraph in &self.paragraphs {
            let _ = write!(text, "{paragraph}\n\n");
        }
        if let Some(link) = &self.link {
            let _ = write!(text, "{}: {}\n\n", link.text, link.url);
        }
        let _ = writeln!(text, "-- \n{}", sender.footer());
        text
    }

    /// The HTML part: the app's Nothing palette in light mode, with inline styles, as mail
    /// clients drop style sheets, and the accent on the button.
    pub fn html(&self, sender: &Sender<'_>) -> String {
        let accent = color::accent(DEFAULT_ACCENT_HUE);
        let mut body = String::new();
        for paragraph in &self.paragraphs {
            let _ = writeln!(
                body,
                r#"<p style="margin:0 0 16px;white-space:pre-wrap">{}</p>"#,
                escape(paragraph)
            );
        }
        if let Some(link) = &self.link {
            let url = escape(&link.url);
            let _ = writeln!(
                body,
                r#"<p style="margin:24px 0"><a href="{url}" style="display:inline-block;padding:10px 20px;border:1px solid {accent};border-radius:9999px;background:{accent};color:#ffffff;font-weight:600;text-decoration:none">{}</a></p>"#,
                escape(&link.text)
            );
            let _ = writeln!(
                body,
                r#"<p style="margin:0 0 16px;font-size:13px;color:#6e6e6e;word-break:break-all">Or open {url}</p>"#
            );
        }

        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light">
<title>{subject}</title>
</head>
<body style="margin:0;padding:0;background:#ffffff">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="border-collapse:collapse;background:#ffffff">
<tr><td style="padding:32px 16px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="border-collapse:collapse;max-width:560px;margin:0 auto;font-family:ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;font-size:15px;line-height:1.6;color:#000000">
<tr><td style="padding:0 0 16px;border-bottom:1px solid #d8d8d8;font-size:13px;font-weight:600;letter-spacing:0.04em">{name}</td></tr>
<tr><td style="padding:24px 0 8px">
{body}</td></tr>
<tr><td style="padding:16px 0 0;border-top:1px solid #d8d8d8;font-size:13px;color:#6e6e6e">{footer}</td></tr>
</table>
</td></tr>
</table>
</body>
</html>
"#,
            subject = escape(&self.subject),
            name = escape(sender.name),
            footer = escape(&sender.footer()),
        )
    }
}

/// How long `duration` is, as people say it: 7 days, 24 hours, 1 hour, 30 minutes.
fn for_people(duration: Duration) -> String {
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    let secs = duration.as_secs();
    let (count, one, many) = if secs >= 2 * DAY && secs.is_multiple_of(DAY) {
        (secs / DAY, "day", "days")
    } else if secs >= HOUR && secs.is_multiple_of(HOUR) {
        (secs / HOUR, "hour", "hours")
    } else {
        ((secs + 30) / 60, "minute", "minutes")
    };
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// A moment as people read it, in UTC: 2 January 2026 at 15:04 UTC.
fn when(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{} {} {} at {:02}:{:02} UTC",
        at.day(),
        at.month(),
        at.year(),
        at.hour(),
        at.minute()
    )
}

/// Text on one line, for a header: control characters become spaces, and runs of spaces
/// one. Settings such as the instance's name may hold line breaks, which a header must not.
pub fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escapes text for HTML, in elements and in quoted attributes alike.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use neon_domain::settings::Secret;

    use super::*;

    const SENDER: Sender<'static> = Sender {
        name: "Neon Teamspace",
        origin: "https://neon.example.com",
    };

    fn hours(n: u64) -> Duration {
        Duration::from_secs(n * 3600)
    }

    fn url_of(message: &Message) -> String {
        write(message, &SENDER).link.expect("a link").url
    }

    #[test]
    fn links_open_the_app_pages_with_their_token() {
        let token = || Secret::new("Ab-c_9");

        assert_eq!(
            url_of(&Message::Verification {
                token: token(),
                lasts: hours(24)
            }),
            "https://neon.example.com/verify?token=Ab-c_9"
        );
        assert_eq!(
            url_of(&Message::PasswordReset {
                token: token(),
                lasts: hours(1)
            }),
            "https://neon.example.com/reset?token=Ab-c_9"
        );
        assert_eq!(
            url_of(&Message::JoinConfirmation {
                token: token(),
                lasts: hours(24)
            }),
            "https://neon.example.com/join/confirm?token=Ab-c_9"
        );
        assert_eq!(
            url_of(&Message::Invite {
                inviter_name: "Grace".into(),
                inviter_email: "grace@example.com".into(),
                code: Secret::new("Team Spring&2026"),
                expires_at: OffsetDateTime::UNIX_EPOCH,
            }),
            "https://neon.example.com/register?invite=Team+Spring%262026"
        );
        assert_eq!(
            url_of(&Message::PasswordChanged {
                at: OffsetDateTime::UNIX_EPOCH
            }),
            "https://neon.example.com/reset"
        );
        assert_eq!(
            url_of(&Message::JoinRequested {
                requester: "ada@example.com".into(),
                note: String::new()
            }),
            "https://neon.example.com/admin/join-requests"
        );
    }

    #[test]
    fn letters_say_how_long_links_work_and_when() {
        let verification = write(
            &Message::Verification {
                token: Secret::new("t"),
                lasts: hours(24),
            },
            &SENDER,
        );
        assert_eq!(verification.subject, "Verify your email for Neon Teamspace");
        assert!(verification.paragraphs[1].starts_with("It works for 24 hours."));
        assert!(verification.paragraphs[1].contains("removed within 7 days"));

        let at = OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a time");
        let changed = write(&Message::PasswordChanged { at }, &SENDER);
        assert!(
            changed.paragraphs[0].ends_with("changed on 22 November 2026 at 12:00 UTC."),
            "{}",
            changed.paragraphs[0]
        );

        let denied = write(
            &Message::JoinDenied {
                ask_again_after: DENIED_WAIT,
            },
            &SENDER,
        );
        assert!(denied.paragraphs[0].ends_with("You may ask again in 30 days."));
        assert!(denied.link.is_none());
    }

    #[test]
    fn durations_read_as_people_say_them() {
        assert_eq!(for_people(hours(1)), "1 hour");
        assert_eq!(for_people(hours(24)), "24 hours");
        assert_eq!(for_people(hours(48)), "2 days");
        assert_eq!(for_people(Duration::from_secs(30 * 60)), "30 minutes");
        assert_eq!(for_people(Duration::from_secs(90)), "2 minutes");
        assert_eq!(for_people(Duration::from_secs(60)), "1 minute");
    }

    #[test]
    fn invites_name_the_admin_and_where() {
        let invite = write(
            &Message::Invite {
                inviter_name: "Grace".into(),
                inviter_email: "grace@example.com".into(),
                code: Secret::new("inv_x"),
                expires_at: OffsetDateTime::UNIX_EPOCH,
            },
            &SENDER,
        );

        assert_eq!(invite.subject, "Grace invited you to Neon Teamspace");
        assert_eq!(
            invite.paragraphs[0],
            "Grace (grace@example.com) invited you to Neon Teamspace, at https://neon.example.com."
        );
        assert!(invite.paragraphs[1].contains("until 1 January 1970 at 00:00 UTC"));
    }

    #[test]
    fn the_text_part_carries_the_link_and_a_signature() {
        let letter = Letter {
            subject: "Hello".into(),
            paragraphs: vec!["One.".into(), "Two.".into()],
            link: Some(Link {
                text: "Go".into(),
                url: "https://neon.example.com/go".into(),
            }),
        };

        assert_eq!(
            letter.text(&SENDER),
            "One.\n\nTwo.\n\nGo: https://neon.example.com/go\n\n-- \nNeon Teamspace, https://neon.example.com\n"
        );
    }

    #[test]
    fn the_html_part_escapes_everything_and_carries_the_accent() {
        let sender = Sender {
            name: "<b>Neon</b>",
            origin: "https://neon.example.com",
        };
        let letter = write(
            &Message::JoinRequested {
                requester: "ada@example.com".into(),
                note: "<script>alert(\"hi\")</script> & 'more'".into(),
            },
            &sender,
        );

        let html = letter.html(&sender);

        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(
            html.contains(
                "&lt;script&gt;alert(&quot;hi&quot;)&lt;/script&gt; &amp; &#39;more&#39;"
            )
        );
        assert!(html.contains("&lt;b&gt;Neon&lt;/b&gt;, https://neon.example.com"));
        assert!(html.contains(r#"href="https://neon.example.com/admin/join-requests""#));
        assert!(html.contains(&format!("background:{}", color::accent(250))));
        assert!(html.starts_with("<!doctype html>"));
    }

    #[test]
    fn subjects_stay_on_one_line() {
        let letter = write(
            &Message::JoinRequested {
                requester: "ada@example.com".into(),
                note: String::new(),
            },
            &Sender {
                name: "Neon\r\nBcc: everyone@example.com",
                origin: "https://neon.example.com",
            },
        );

        assert_eq!(
            letter.subject,
            "ada@example.com asks to join Neon Bcc: everyone@example.com"
        );
        assert_eq!(one_line("  a \t b\u{7}c  "), "a b c");
    }

    #[test]
    fn a_note_left_blank_is_not_quoted() {
        let letter = write(
            &Message::JoinRequested {
                requester: "ada@example.com".into(),
                note: "  ".into(),
            },
            &SENDER,
        );

        assert_eq!(letter.paragraphs.len(), 1);
    }
}
