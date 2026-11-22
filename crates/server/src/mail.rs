//! The [`Mailer`] port over SMTP, with lettre on rustls.
//!
//! The mail settings are read again for every mail, so a change on the admin page applies
//! to the next one. TLS is 1.2 or later, and the server's certificate is always checked
//! against the bundled Mozilla roots: `starttls` refuses a server that does not offer
//! STARTTLS rather than going on in the clear, and `tls` speaks TLS from the first byte. A
//! password never goes over a plain connection to another host: a settings rule refuses
//! saving that, and each send checks it again, as saved values that break a rule only warn
//! when the server starts.
//!
//! A mail server's permanent refusal of the recipient or the mail, a 55x answer, is
//! [`SendError::Rejected`], which ends the mail's job; everything else is retried. Errors
//! carry what the server answered, on one line, never the password.

use std::error::Error;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lettre::message::header::{Header, HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters, TlsVersion};
use lettre::transport::smtp::extension::ClientId;
use lettre::transport::smtp::response::{Category, Severity};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport as _, Tokio1Executor};
use neon_app::mail::compose::one_line;
use neon_app::mail::{Mailer, Outgoing, SendError};
use neon_app::settings::{INSTANCE_NAME, SettingsService, mail as keys};
use neon_domain::mail::{Security, may_sign_in};
use neon_domain::settings::Secret;
use tracing::debug;
use url::{Host, Url};
use uuid::Uuid;

/// How long one SMTP command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// The most of a server's answer an error keeps.
const MAX_ANSWER: usize = 300;

/// Sends mail through the SMTP server the settings name.
pub struct SmtpMailer {
    settings: Arc<SettingsService>,
    /// The name the server is greeted with: the public URL's host.
    hello: ClientId,
    timeout: Duration,
    /// Certificates trusted beside the bundled roots. Only tests add any.
    roots: Vec<Certificate>,
}

impl SmtpMailer {
    pub fn new(settings: Arc<SettingsService>, public_url: Option<&Url>) -> Self {
        let hello = match public_url.and_then(Url::host) {
            Some(Host::Domain(name)) => ClientId::Domain(name.to_owned()),
            Some(Host::Ipv4(address)) => ClientId::Ipv4(address),
            Some(Host::Ipv6(address)) => ClientId::Ipv6(address),
            None => ClientId::default(),
        };
        Self {
            settings,
            hello,
            timeout: COMMAND_TIMEOUT,
            roots: Vec::new(),
        }
    }

    fn config(&self) -> Result<Config, SendError> {
        let settings = self.settings.current();
        let not_set = |what: &str| Err(SendError::NotConfigured(what.to_owned()));
        if !*settings.get(keys::ENABLED) {
            return not_set("mail.enabled is off");
        }
        let host = settings.get(keys::HOST);
        if host.is_empty() {
            return not_set("mail.host is not set");
        }
        let from = settings.get(keys::FROM);
        if from.is_empty() {
            return not_set("mail.from is not set");
        }
        let from = Address::from_str(from).map_err(|_| {
            SendError::Failed("mail.from is not an address mail servers take".to_owned())
        })?;
        let from_name = match settings.get(keys::FROM_NAME) {
            "" => settings.get(INSTANCE_NAME),
            name => name,
        };
        let security = Security::parse(settings.get(keys::TLS)).unwrap_or(Security::DEFAULT);
        let username = settings.get(keys::USERNAME);
        if !username.is_empty() && !may_sign_in(security, host) {
            return not_set(
                "mail.tls is none, so the password would cross the network in the clear: use \
                 starttls or tls",
            );
        }
        Ok(Config {
            host: host.to_owned(),
            port: u16::try_from(*settings.get(keys::PORT))
                .map_err(|_| SendError::Failed("mail.port is not a port".to_owned()))?,
            security,
            username: username.to_owned(),
            password: settings.get(keys::PASSWORD).clone(),
            from: Mailbox::new(Some(one_line(from_name)), from),
        })
    }

    fn transport(&self, config: &Config) -> Result<AsyncSmtpTransport<Tokio1Executor>, SendError> {
        let tls = match config.security {
            Security::None => Tls::None,
            Security::StartTls => Tls::Required(self.tls(&config.host)?),
            Security::Tls => Tls::Wrapper(self.tls(&config.host)?),
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .tls(tls)
            .hello_name(self.hello.clone())
            .timeout(Some(self.timeout));
        if !config.username.is_empty() {
            builder = builder.credentials(Credentials::new(
                config.username.clone(),
                config.password.expose().to_owned(),
            ));
        }
        Ok(builder.build())
    }

    fn tls(&self, host: &str) -> Result<TlsParameters, SendError> {
        self.roots
            .iter()
            .fold(TlsParameters::builder(host.to_owned()), |builder, root| {
                builder.add_root_certificate(root.clone())
            })
            .set_min_tls_version(TlsVersion::Tlsv12)
            .build_rustls()
            .map_err(|error| SendError::Failed(answer(&error)))
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: &Outgoing) -> Result<(), SendError> {
        let config = self.config()?;
        let message = message(&config, mail)?;
        let transport = self.transport(&config)?;
        let started = Instant::now();
        match transport.send(message).await {
            Ok(_) => {
                debug!(
                    host = %config.host,
                    took_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "the mail server took the mail"
                );
                Ok(())
            }
            Err(error) => Err(refusal(&error)),
        }
    }
}

/// The mail settings of one send.
struct Config {
    host: String,
    port: u16,
    security: Security,
    username: String,
    password: Secret,
    from: Mailbox,
}

/// The mail as SMTP carries it: plain text with an HTML alternative, a Message-ID on the
/// From address's domain, which names no machine, and marked as sent by a program, so
/// auto-responders leave it be (RFC 3834).
fn message(config: &Config, mail: &Outgoing) -> Result<lettre::Message, SendError> {
    let to = Address::from_str(&mail.to.address).map_err(|_| {
        SendError::Rejected("the recipient is not an address mail servers take".to_owned())
    })?;
    let id = format!(
        "<{}@{}>",
        Uuid::now_v7().simple(),
        config.from.email.domain()
    );
    lettre::Message::builder()
        .from(config.from.clone())
        .to(Mailbox::new(mail.to.name.clone(), to))
        .subject(mail.subject.as_str())
        .date_now()
        .message_id(Some(id))
        .header(AutoSubmitted)
        .multipart(MultiPart::alternative_plain_html(
            mail.text.clone(),
            mail.html.clone(),
        ))
        .map_err(|error| SendError::Failed(format!("the mail could not be written: {error}")))
}

/// What a failed send means for the mail: a 55x answer refuses it for good, anything else
/// may pass.
fn refusal(error: &lettre::transport::smtp::Error) -> SendError {
    let refused = error.status().is_some_and(|code| {
        code.severity == Severity::PermanentNegativeCompletion
            && code.category == Category::MailSystem
    });
    if refused {
        SendError::Rejected(answer(error))
    } else {
        SendError::Failed(answer(error))
    }
}

/// An error with its causes, on one line and at most [`MAX_ANSWER`] characters, for logs
/// and the admin page.
fn answer(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let said = cause.to_string();
        // lettre's errors print their cause already.
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = cause.source();
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(MAX_ANSWER) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

/// `Auto-Submitted: auto-generated`.
#[derive(Clone)]
struct AutoSubmitted;

impl Header for AutoSubmitted {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("Auto-Submitted")
    }

    fn parse(_: &str) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self)
    }

    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), "auto-generated".to_owned())
    }
}

#[cfg(test)]
mod tests;
