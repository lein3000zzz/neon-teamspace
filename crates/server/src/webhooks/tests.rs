//! The transport against a receiver on loopback, and against resolvers and dialers that say
//! what they were asked.

use std::sync::Mutex;
use std::time::Instant;

use neon_domain::webhooks::Blocked;
use tokio::net::TcpListener;

use super::receiver::{Receiver, Reply, ScriptedResolver};
use super::*;

/// Connects as the system does, after noting each address it is asked for; or refuses
/// every one, when told to.
#[derive(Default)]
struct RecordingDialer {
    dialed: Mutex<Vec<SocketAddr>>,
    refuse: bool,
}

impl RecordingDialer {
    fn refusing() -> Arc<Self> {
        Arc::new(Self {
            dialed: Mutex::new(Vec::new()),
            refuse: true,
        })
    }

    fn dialed(&self) -> Vec<SocketAddr> {
        self.dialed.lock().expect("lock").clone()
    }
}

#[async_trait]
impl Dial for RecordingDialer {
    async fn dial(&self, address: SocketAddr) -> io::Result<TcpStream> {
        self.dialed.lock().expect("lock").push(address);
        if self.refuse {
            Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"))
        } else {
            TcpStream::connect(address).await
        }
    }
}

fn request(url: &str) -> Outgoing {
    Outgoing {
        url: Url::parse(url).expect("a URL"),
        headers: vec![
            ("Content-Type", "application/json".to_owned()),
            ("Neon-Webhook-Event", "webhook.test".to_owned()),
        ],
        body: Bytes::from_static(br#"{"version":1}"#),
    }
}

fn public(port: u16) -> SocketAddr {
    SocketAddr::from(([93, 184, 215, 14], port))
}

#[tokio::test]
async fn private_addresses_are_refused_unless_allowed() {
    let receiver = Receiver::start().await;
    let transport = HttpTransport::default();

    let refused = transport.send(request(&receiver.url("/hook")), false).await;
    assert_eq!(refused, Err(SendError::Refused(Blocked::Loopback)));
    assert!(receiver.received().is_empty(), "nothing was sent");

    let answer = transport
        .send(request(&receiver.url("/hook?x=1")), true)
        .await
        .expect("delivered");
    assert_eq!(answer.status, 200);
    let received = receiver.wait_for(1).await;
    let got = &received[0];
    assert_eq!(got.method, Method::POST);
    assert_eq!(got.path, "/hook");
    assert_eq!(got.header("user-agent"), Some(USER_AGENT));
    assert_eq!(got.header("content-type"), Some("application/json"));
    assert_eq!(got.header("neon-webhook-event"), Some("webhook.test"));
    assert_eq!(
        got.header("host"),
        Some(receiver.addr().to_string().as_str())
    );
    assert_eq!(got.body.as_ref(), br#"{"version":1}"#);
}

#[tokio::test]
async fn literal_addresses_in_every_form_are_checked() {
    let receiver = Receiver::start().await;
    let port = receiver.addr().port();
    let transport = HttpTransport::default();

    for host in [
        "127.0.0.1",
        "2130706433",
        "0x7f.1",
        "[::ffff:127.0.0.1]",
        "[::1]",
        "0.0.0.0",
        "169.254.169.254",
        "[fd00:ec2::254]",
    ] {
        let url = format!("http://{host}:{port}/");
        let refused = transport.send(request(&url), false).await;
        assert!(
            matches!(refused, Err(SendError::Refused(_))),
            "{host}: {refused:?}"
        );
    }
    // Even allowed private addresses never open the metadata service or 0.0.0.0.
    for host in ["0.0.0.0", "169.254.169.254"] {
        let url = format!("http://{host}:{port}/");
        let refused = transport.send(request(&url), true).await;
        assert!(
            matches!(refused, Err(SendError::Refused(_))),
            "{host}: {refused:?}"
        );
    }
    assert!(receiver.received().is_empty());
}

#[tokio::test]
async fn a_name_is_resolved_once_and_only_the_address_checked_is_dialed() {
    let receiver = Receiver::start().await;
    let port = receiver.addr().port();
    // A name that answers a public address first, then the receiver's: the rebinding.
    let resolver = ScriptedResolver::new(vec![vec![public(port)], vec![receiver.addr()]]);
    let dialer = RecordingDialer::refusing();
    let transport = HttpTransport::new(resolver.clone(), dialer.clone());
    let url = format!("http://hooks.example.com:{port}/in");

    let sent = transport.send(request(&url), false).await;

    assert_eq!(sent, Err(SendError::Connect));
    assert_eq!(
        resolver.asked(),
        [("hooks.example.com".to_owned(), port)],
        "one lookup"
    );
    assert_eq!(dialer.dialed(), [public(port)], "the checked address alone");
    assert!(receiver.received().is_empty());

    // The next delivery looks again, gets the private address, and refuses it before any
    // connection.
    let again = transport.send(request(&url), false).await;
    assert_eq!(again, Err(SendError::Refused(Blocked::Loopback)));
    assert_eq!(dialer.dialed(), [public(port)]);
    assert!(receiver.received().is_empty());
}

#[tokio::test]
async fn a_private_address_among_those_resolved_refuses_them_all() {
    let resolver = ScriptedResolver::new(vec![vec![
        public(443),
        SocketAddr::from(([10, 0, 0, 7], 443)),
    ]]);
    let dialer = RecordingDialer::refusing();
    let transport = HttpTransport::new(resolver.clone(), dialer.clone());

    for url in [
        "https://mixed.example.com/",
        "http://mixed.example.com:443/",
    ] {
        assert_eq!(
            transport
                .check(&Url::parse(url).expect("a URL"), false)
                .await,
            Err(SendError::Refused(Blocked::Private)),
            "{url}"
        );
        assert_eq!(
            transport.send(request(url), false).await,
            Err(SendError::Refused(Blocked::Private))
        );
    }
    assert!(dialer.dialed().is_empty());

    let nowhere = HttpTransport::new(ScriptedResolver::new(Vec::new()), dialer.clone());
    assert_eq!(
        nowhere
            .check(
                &Url::parse("https://missing.example.com/").expect("a URL"),
                false
            )
            .await,
        Err(SendError::Unresolvable)
    );
}

#[tokio::test]
async fn a_name_resolving_to_the_receiver_reaches_it_once_private_addresses_are_allowed() {
    let receiver = Receiver::start().await;
    let resolver = ScriptedResolver::new(vec![vec![receiver.addr()]]);
    let dialer = Arc::new(RecordingDialer::default());
    let transport = HttpTransport::new(resolver, dialer.clone());
    let url = format!("http://receiver.test:{}/hook", receiver.addr().port());

    let answer = transport
        .send(request(&url), true)
        .await
        .expect("delivered");

    assert_eq!(answer.status, 200);
    assert_eq!(dialer.dialed(), [receiver.addr()]);
    let received = receiver.wait_for(1).await;
    assert_eq!(
        received[0].header("host"),
        Some(format!("receiver.test:{}", receiver.addr().port()).as_str())
    );
}

#[tokio::test]
async fn redirects_are_answers_and_never_followed() {
    let receiver = Receiver::start().await;
    receiver.always(Reply {
        status: 302,
        headers: vec![("location", receiver.url("/elsewhere"))],
        body: b"moved".to_vec(),
    });
    let transport = HttpTransport::default();

    let answer = transport
        .send(request(&receiver.url("/hook")), true)
        .await
        .expect("answered");

    assert_eq!(answer.status, 302);
    assert_eq!(answer.excerpt.as_deref(), Some("moved"));
    let received = receiver.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/hook");
}

#[tokio::test]
async fn only_the_start_of_a_large_answer_is_read() {
    let receiver = Receiver::start().await;
    receiver.always(Reply {
        status: 500,
        headers: Vec::new(),
        body: vec![b'x'; 8 * MAX_ANSWER],
    });
    let transport = HttpTransport::default();

    let answer = transport
        .send(request(&receiver.url("/hook")), true)
        .await
        .expect("answered");

    assert_eq!(answer.status, 500);
    let excerpt = answer.excerpt.expect("an excerpt");
    assert_eq!(excerpt.len(), neon_app::webhooks::transport::MAX_EXCERPT);
    assert!(excerpt.chars().all(|c| c == 'x'));
}

#[tokio::test]
async fn a_receiver_that_never_answers_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let addr = listener.local_addr().expect("bound");
    // Takes the connection and says nothing.
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    let transport =
        HttpTransport::default().timeouts(Duration::from_millis(500), Duration::from_millis(500));
    let started = Instant::now();

    let sent = transport
        .send(request(&format!("http://{addr}/hook")), true)
        .await;

    assert_eq!(sent, Err(SendError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    silent.abort();
}

#[tokio::test]
async fn a_send_that_times_out_stops_driving_its_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let addr = listener.local_addr().expect("bound");
    // Takes the connection and reads nothing, so writing a large body stalls.
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    let metrics = tokio::runtime::Handle::current().metrics();
    let tasks = metrics.num_alive_tasks();
    let transport =
        HttpTransport::default().timeouts(Duration::from_millis(500), Duration::from_millis(500));
    let large = Outgoing {
        body: Bytes::from(vec![b' '; 64 * 1024 * 1024]),
        ..request(&format!("http://{addr}/hook"))
    };

    let sent = transport.send(large, true).await;

    assert_eq!(sent, Err(SendError::Timeout));
    let stopped = tokio::time::timeout(Duration::from_secs(5), async {
        while metrics.num_alive_tasks() > tasks {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(stopped.is_ok(), "the connection's task outlived the send");
    silent.abort();
}

/// Never answers, as a resolver whose name server is a tarpit.
struct StalledResolver;

#[async_trait]
impl Resolve for StalledResolver {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn a_name_that_never_resolves_is_given_up_on_in_time() {
    let transport = HttpTransport::new(Arc::new(StalledResolver), Arc::new(SystemDialer))
        .timeouts(Duration::from_millis(200), Duration::from_secs(60));
    let url = Url::parse("https://hooks.example.com/hook").expect("a URL");

    let checked = tokio::time::timeout(Duration::from_secs(30), transport.check(&url, false))
        .await
        .expect("the check ends");

    assert_eq!(checked, Err(SendError::Unresolvable));
}

#[tokio::test]
async fn a_closed_port_fails_to_connect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let addr = listener.local_addr().expect("bound");
    drop(listener);
    let transport = HttpTransport::default();

    let sent = transport
        .send(request(&format!("http://{addr}/hook")), true)
        .await;

    assert_eq!(sent, Err(SendError::Connect));
}

#[tokio::test]
async fn https_needs_a_certificate_the_web_trusts() {
    let receiver = Receiver::start().await;
    let transport = HttpTransport::default();
    // The receiver speaks plain HTTP, which no TLS handshake gets past.
    let url = format!("https://{}/hook", receiver.addr());

    let sent = transport.send(request(&url), true).await;

    assert_eq!(sent, Err(SendError::Tls));
}

#[test]
fn certificates_are_checked_against_the_urls_host() {
    let name = |url: &str| server_name(&Url::parse(url).expect("a URL"));

    assert_eq!(
        name("https://hooks.example.com./x"),
        ServerName::try_from("hooks.example.com").ok()
    );
    assert!(matches!(
        name("https://203.0.113.9/"),
        Some(ServerName::IpAddress(_))
    ));
    assert!(matches!(
        name("https://[2001:db8::1]/"),
        Some(ServerName::IpAddress(_))
    ));
    assert_eq!(
        host_header(&Url::parse("https://example.com:8443/x").expect("a URL")),
        Some("example.com:8443".to_owned())
    );
    assert_eq!(
        host_header(&Url::parse("https://example.com:443/x").expect("a URL")),
        Some("example.com".to_owned())
    );
}
