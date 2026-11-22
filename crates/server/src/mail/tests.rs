//! The SMTP adapter against a fake SMTP server: plain, STARTTLS and TLS from the start,
//! with and without signing in, and refusing whom it is told to.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_app::crypto::{Envelope, Keyring};
use neon_app::mail::Recipient;
use neon_app::settings::{Change, registry};
use neon_domain::settings::Pinned;
use serde_json::{Value as Json, json};
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::*;

const PASSWORD: &str = "s3cret-smtp-password";

/// What the fake server saw.
#[derive(Debug, Default, Clone)]
struct Seen {
    greetings: Vec<String>,
    /// `user:password` of each sign-in.
    sign_ins: Vec<String>,
    signed_in_without_tls: bool,
    upgraded: bool,
    messages: Vec<String>,
}

/// An SMTP server for tests: PLAIN sign-in, STARTTLS when it has a certificate, TLS from
/// the start when `implicit`, and a 550 for the recipient it refuses.
struct Fake {
    tls: Option<TlsAcceptor>,
    implicit: bool,
    refuse: Option<&'static str>,
    seen: Mutex<Seen>,
}

impl Fake {
    async fn start(self) -> (u16, Arc<Self>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let fake = Arc::new(self);
        let serving = Arc::clone(&fake);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let fake = Arc::clone(&serving);
                tokio::spawn(async move { fake.connection(stream).await });
            }
        });
        (port, fake)
    }

    async fn connection(&self, stream: tokio::net::TcpStream) {
        let acceptor = self.tls.clone();
        if self.implicit {
            let Some(acceptor) = acceptor else { return };
            if let Ok(tls) = acceptor.accept(stream).await {
                self.session(tls, true, true).await;
            }
            return;
        }
        let Some(plain) = self.session(stream, false, true).await else {
            return;
        };
        let Some(acceptor) = acceptor else { return };
        if let Ok(tls) = acceptor.accept(plain).await {
            self.seen().upgraded = true;
            self.session(tls, true, false).await;
        }
    }

    /// Serves commands until the client quits, or hands the stream back for STARTTLS.
    async fn session<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        stream: S,
        secure: bool,
        greet: bool,
    ) -> Option<S> {
        let mut stream = BufReader::new(stream);
        if greet {
            say(&mut stream, "220 fake ESMTP").await;
        }
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.ok()? == 0 {
                return None;
            }
            let command = line.trim_end().to_owned();
            let upper = command.to_ascii_uppercase();
            if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                self.seen().greetings.push(command.clone());
                say(&mut stream, "250-fake").await;
                if self.tls.is_some() && !secure {
                    say(&mut stream, "250-STARTTLS").await;
                }
                say(&mut stream, "250 AUTH PLAIN").await;
            } else if upper == "STARTTLS" {
                say(&mut stream, "220 go ahead").await;
                return Some(stream.into_inner());
            } else if let Some(encoded) = upper
                .starts_with("AUTH PLAIN ")
                .then(|| command["AUTH PLAIN ".len()..].trim())
            {
                let decoded = STANDARD.decode(encoded).unwrap_or_default();
                let parts: Vec<String> = String::from_utf8_lossy(&decoded)
                    .split('\0')
                    .map(str::to_owned)
                    .collect();
                {
                    let mut seen = self.seen();
                    seen.sign_ins.push(parts[1..].join(":"));
                    seen.signed_in_without_tls |= !secure;
                }
                if parts.len() == 3 && parts[1] == "neon" && parts[2] == PASSWORD {
                    say(&mut stream, "235 2.7.0 Authentication successful").await;
                } else {
                    say(&mut stream, "535 5.7.8 Authentication credentials invalid").await;
                }
            } else if upper.starts_with("MAIL FROM") {
                say(&mut stream, "250 2.1.0 Ok").await;
            } else if upper.starts_with("RCPT TO") {
                if let Some(refused) = self.refuse.filter(|refused| command.contains(refused)) {
                    let refusal = format!("550 5.1.1 <{refused}>: No such user here");
                    say(&mut stream, &refusal).await;
                } else {
                    say(&mut stream, "250 2.1.5 Ok").await;
                }
            } else if upper == "DATA" {
                say(&mut stream, "354 End data with <CR><LF>.<CR><LF>").await;
                let mut data = String::new();
                loop {
                    let mut line = String::new();
                    if stream.read_line(&mut line).await.ok()? == 0 {
                        return None;
                    }
                    if line == ".\r\n" {
                        break;
                    }
                    data.push_str(&line);
                }
                self.seen().messages.push(data);
                say(&mut stream, "250 2.0.0 Ok: queued").await;
            } else if upper == "QUIT" {
                say(&mut stream, "221 2.0.0 Bye").await;
                return None;
            } else {
                say(&mut stream, "250 2.0.0 Ok").await;
            }
        }
    }

    fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().expect("seen lock")
    }
}

async fn say<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut BufReader<S>, line: &str) {
    let writer = stream.get_mut();
    let _ = writer.write_all(format!("{line}\r\n").as_bytes()).await;
    let _ = writer.flush().await;
}

/// A certificate for 127.0.0.1, an acceptor serving it, and the certificate to trust it.
fn self_signed() -> (TlsAcceptor, Certificate) {
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("a cert");
    let der = CertificateDer::from(made.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(made.signing_key.serialize_der()));
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key)
        .expect("a server config");
    let trusted = Certificate::from_der(der.to_vec()).expect("a certificate");
    (TlsAcceptor::from(Arc::new(config)), trusted)
}

fn plain() -> Fake {
    Fake {
        tls: None,
        implicit: false,
        refuse: None,
        seen: Mutex::default(),
    }
}

fn envelope() -> Envelope {
    let key = format!("k1:{}", STANDARD.encode([5u8; 32]));
    Envelope::new(Keyring::parse(&[key]).expect("a key"))
}

/// A mailer whose settings point at the fake on `port`, secured with `tls`.
async fn mailer(pool: PgPool, port: u16, tls: &str, sign_in: Option<&str>) -> SmtpMailer {
    let (settings, _) = SettingsService::load(pool, registry(), Pinned::default(), envelope())
        .await
        .expect("settings load");
    let mut changes: Vec<(&str, Json)> = vec![
        ("mail.enabled", json!(true)),
        ("mail.host", json!("127.0.0.1")),
        ("mail.port", json!(port)),
        ("mail.tls", json!(tls)),
        ("mail.from", json!("neon@example.com")),
    ];
    if let Some(password) = sign_in {
        changes.push(("mail.username", json!("neon")));
        changes.push(("mail.password", json!(password)));
    }
    settings
        .update(
            changes
                .into_iter()
                .map(|(key, value)| (key.to_owned(), Change::Set(value))),
            None,
        )
        .await
        .expect("valid settings");
    let public_url = Url::parse("https://neon.example.com").expect("a URL");
    let mut mailer = SmtpMailer::new(Arc::new(settings), Some(&public_url));
    mailer.timeout = Duration::from_secs(5);
    mailer
}

fn hello() -> Outgoing {
    Outgoing {
        to: Recipient::new("ada@example.com", Some("Ada".into())),
        subject: "Hello".into(),
        text: "Plain words.".into(),
        html: "<p>HTML words.</p>".into(),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_plain_send_writes_both_parts_and_the_headers(pool: PgPool) {
    let (port, fake) = plain().start().await;

    mailer(pool, port, "none", None)
        .await
        .send(&hello())
        .await
        .expect("sent");

    let seen = fake.seen().clone();
    assert_eq!(seen.greetings, ["EHLO neon.example.com"]);
    assert_eq!(seen.messages.len(), 1);
    let data = &seen.messages[0];
    let header = |name: &str| {
        data.lines()
            .find(|line| line.starts_with(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("no {name} header in {data}"))
            .to_owned()
    };
    assert!(header("From").contains("Neon Workspace"), "{data}");
    assert!(header("From").contains("<neon@example.com>"), "{data}");
    assert_eq!(header("To"), "To: Ada <ada@example.com>");
    assert_eq!(header("Subject"), "Subject: Hello");
    assert_eq!(header("Auto-Submitted"), "Auto-Submitted: auto-generated");
    let id = header("Message-ID");
    assert!(
        id.starts_with("Message-ID: <") && id.ends_with("@example.com>"),
        "{id}"
    );
    assert!(data.contains("multipart/alternative"), "{data}");
    assert!(data.contains("text/plain") && data.contains("Plain words."));
    assert!(data.contains("text/html") && data.contains("<p>HTML words.</p>"));
    assert!(header("Date").len() > "Date: ".len());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_from_name_stays_on_its_line(pool: PgPool) {
    let (port, fake) = plain().start().await;
    let mailer = mailer(pool, port, "none", None).await;
    mailer
        .settings
        .update(
            [(
                "instance.name".to_owned(),
                Change::Set(json!("Neon\r\nBcc: everyone@example.com")),
            )],
            None,
        )
        .await
        .expect("saved");

    mailer.send(&hello()).await.expect("sent");

    let data = fake.seen().messages[0].clone();
    assert!(!data.lines().any(|line| line.starts_with("Bcc:")), "{data}");
    // The name has specials, so it arrives as an RFC 2047 encoded word.
    let from = data
        .lines()
        .find(|line| line.starts_with("From: "))
        .expect("a From header");
    let encoded = from
        .split("=?utf-8?b?")
        .nth(1)
        .and_then(|rest| rest.split("?=").next())
        .unwrap_or_else(|| panic!("not an encoded word: {from}"));
    let name = STANDARD.decode(encoded).expect("base64");
    assert_eq!(
        String::from_utf8_lossy(&name),
        "Neon Bcc: everyone@example.com"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn it_signs_in_and_never_repeats_the_password(pool: PgPool) {
    let (port, fake) = plain().start().await;

    // To this host, a plain connection may carry the password.
    mailer(pool.clone(), port, "none", Some(PASSWORD))
        .await
        .send(&hello())
        .await
        .expect("sent");
    assert_eq!(fake.seen().sign_ins, [format!("neon:{PASSWORD}")]);

    let error = mailer(pool, port, "none", Some("a-wrong-password"))
        .await
        .send(&hello())
        .await
        .expect_err("refused");
    let SendError::Failed(said) = &error else {
        panic!("a sign-in refused is for an admin to fix, and retried: {error:?}");
    };
    assert!(said.contains("535"), "{said}");
    assert!(!said.contains("a-wrong-password"), "{said}");
    assert_eq!(fake.seen().messages.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_refused_recipient_is_rejected_for_good(pool: PgPool) {
    let (port, fake) = Fake {
        refuse: Some("nobody@example.com"),
        ..plain()
    }
    .start()
    .await;
    let mail = Outgoing {
        to: Recipient::new("nobody@example.com", None),
        ..hello()
    };

    let error = mailer(pool, port, "none", None)
        .await
        .send(&mail)
        .await
        .expect_err("refused");

    let SendError::Rejected(said) = &error else {
        panic!("a 550 is permanent: {error:?}");
    };
    assert!(
        said.contains("550") && said.contains("5.1.1 <[address]>: No such user here"),
        "{said}"
    );
    assert!(!said.contains("nobody@"), "{said}");
    assert!(fake.seen().messages.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_address_mail_servers_do_not_take_is_rejected(pool: PgPool) {
    let (port, fake) = plain().start().await;
    let mail = Outgoing {
        to: Recipient::new("not an address", None),
        ..hello()
    };

    let error = mailer(pool, port, "none", None)
        .await
        .send(&mail)
        .await
        .expect_err("refused");

    assert!(matches!(error, SendError::Rejected(_)), "{error:?}");
    assert!(fake.seen().greetings.is_empty(), "never connected");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn starttls_refuses_a_server_that_does_not_offer_it(pool: PgPool) {
    let (port, fake) = plain().start().await;

    let error = mailer(pool, port, "starttls", Some(PASSWORD))
        .await
        .send(&hello())
        .await
        .expect_err("no STARTTLS");

    let SendError::Failed(said) = &error else {
        panic!("{error:?}");
    };
    assert!(said.contains("STARTTLS"), "{said}");
    let seen = fake.seen().clone();
    assert!(seen.sign_ins.is_empty(), "no password in the clear");
    assert!(seen.messages.is_empty(), "no mail in the clear");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn starttls_checks_the_certificate_then_signs_in_over_tls(pool: PgPool) {
    let (acceptor, trusted) = self_signed();
    let (port, fake) = Fake {
        tls: Some(acceptor),
        ..plain()
    }
    .start()
    .await;

    let untrusted = mailer(pool.clone(), port, "starttls", Some(PASSWORD))
        .await
        .send(&hello())
        .await
        .expect_err("a certificate nobody vouches for");
    let SendError::Failed(said) = &untrusted else {
        panic!("{untrusted:?}");
    };
    assert!(said.contains("certificate"), "{said}");
    assert!(fake.seen().sign_ins.is_empty());

    let mut mailer = mailer(pool, port, "starttls", Some(PASSWORD)).await;
    mailer.roots.push(trusted);
    mailer.send(&hello()).await.expect("sent");

    let seen = fake.seen().clone();
    assert!(seen.upgraded);
    assert_eq!(seen.sign_ins.len(), 1);
    assert!(!seen.signed_in_without_tls);
    assert_eq!(seen.messages.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn tls_from_the_start_checks_the_certificate_too(pool: PgPool) {
    let (acceptor, trusted) = self_signed();
    let (port, fake) = Fake {
        tls: Some(acceptor),
        implicit: true,
        ..plain()
    }
    .start()
    .await;

    let untrusted = mailer(pool.clone(), port, "tls", None)
        .await
        .send(&hello())
        .await
        .expect_err("untrusted");
    let SendError::Failed(said) = &untrusted else {
        panic!("{untrusted:?}");
    };
    assert!(said.contains("certificate"), "{said}");

    let mut mailer = mailer(pool, port, "tls", None).await;
    mailer.roots.push(trusted);
    mailer.send(&hello()).await.expect("sent");
    assert_eq!(fake.seen().messages.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn settings_are_read_again_for_every_mail(pool: PgPool) {
    let (first_port, first) = plain().start().await;
    let (second_port, second) = plain().start().await;
    let mailer = mailer(pool, first_port, "none", None).await;
    mailer.send(&hello()).await.expect("sent");

    mailer
        .settings
        .update(
            [("mail.port".to_owned(), Change::Set(json!(second_port)))],
            None,
        )
        .await
        .expect("saved");
    mailer.send(&hello()).await.expect("sent");
    mailer
        .settings
        .update(
            [("mail.enabled".to_owned(), Change::Set(json!(false)))],
            None,
        )
        .await
        .expect("saved");
    let off = mailer.send(&hello()).await.expect_err("off");

    assert_eq!(first.seen().messages.len(), 1);
    assert_eq!(second.seen().messages.len(), 1);
    assert_eq!(off, SendError::NotConfigured("mail.enabled is off".into()));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_unreachable_server_is_tried_again_later(pool: PgPool) {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        listener.local_addr().expect("an address").port()
    };

    let error = mailer(pool, port, "none", None)
        .await
        .send(&hello())
        .await
        .expect_err("nobody listens");

    assert!(matches!(error, SendError::Failed(_)), "{error:?}");
}

/// Saved values the rule refuses, as a rule made stricter, or a value the environment
/// pins, can leave them: the server starts with a warning, and sends no password.
#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_password_never_crosses_the_network_in_the_clear(pool: PgPool) {
    for (key, value) in [
        ("mail.enabled", json!(true)),
        ("mail.host", json!("smtp.example.com")),
        ("mail.tls", json!("none")),
        ("mail.username", json!("neon")),
        ("mail.from", json!("neon@example.com")),
    ] {
        neon_store::settings::upsert(&pool, key, &value, None)
            .await
            .expect("saved");
    }
    let (settings, warnings) =
        SettingsService::load(pool, registry(), Pinned::default(), envelope())
            .await
            .expect("the server starts");
    assert_eq!(
        warnings.iter().map(|w| w.key.as_str()).collect::<Vec<_>>(),
        ["mail.tls"]
    );

    let error = SmtpMailer::new(Arc::new(settings), None)
        .send(&hello())
        .await
        .expect_err("refused before connecting");

    let SendError::NotConfigured(said) = &error else {
        panic!("not sent: {error:?}");
    };
    assert!(said.contains("in the clear"), "{said}");
}

#[test]
fn answers_name_no_address() {
    let refusal = std::io::Error::other(
        "550 5.1.1 <bob@example.com>: Recipient address rejected; (from neon@example.com) a@b",
    );

    assert_eq!(
        answer(&refusal),
        "550 5.1.1 <[address]>: Recipient address rejected; (from [address]) [address]"
    );
    let plain = std::io::Error::other("421 4.7.0 try again @ 10:00");
    assert_eq!(answer(&plain), "421 4.7.0 try again @ 10:00");
}

#[test]
fn answers_fit_on_one_short_line() {
    let long = std::io::Error::other(format!("first\nsecond {}", "x".repeat(400)));

    let said = answer(&long);

    assert!(said.starts_with("first second x"), "{said}");
    assert!(!said.contains('\n'));
    assert_eq!(said.chars().count(), MAX_ANSWER + 1);
}
