//! The check of `GET /api/v1/meta` against servers on loopback that answer as the tests need.

mod support;

use std::time::Duration;

use neon_shell::address::ServerAddress;
use neon_shell::meta::{self, CheckError, CheckOptions, MAX_ANSWER_BYTES, ServerInfo};
use support::{Answer, closed_port, serve};

fn options() -> CheckOptions {
    CheckOptions {
        timeout: Duration::from_secs(5),
    }
}

async fn check(origin: &str) -> Result<ServerInfo, CheckError> {
    meta::check(&ServerAddress::parse(origin).unwrap(), options()).await
}

async fn check_answer(answer: Answer) -> Result<ServerInfo, CheckError> {
    let server = serve(answer).await;
    check(&server.origin).await
}

#[tokio::test]
async fn a_neon_teamspace_server_passes_and_is_asked_for_its_metadata() {
    let mut server = serve(Answer::neon("Acme")).await;
    let info = check(&server.origin).await.unwrap();
    assert_eq!(
        info,
        ServerInfo {
            name: "Acme".to_owned(),
            version: "0.1.0".to_owned(),
        }
    );
    assert_eq!(server.next_request().await, "GET /api/v1/meta HTTP/1.1");
}

#[tokio::test]
async fn a_path_in_the_address_is_not_where_the_check_asks() {
    let mut server = serve(Answer::neon("Acme")).await;
    let address = format!("{}/w/acme/p/1?x=1", server.origin);
    meta::check(&ServerAddress::parse(&address).unwrap(), options())
        .await
        .unwrap();
    assert_eq!(server.next_request().await, "GET /api/v1/meta HTTP/1.1");
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_names_where_it_goes() {
    let error = check_answer(Answer::http(
        "301 Moved Permanently",
        &[("location", "https://www.example.com/api/v1/meta")],
        b"",
    ))
    .await
    .unwrap_err();
    assert_eq!(error.code(), "redirected");
    let CheckError::Redirected { to, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(to, "https://www.example.com");
    assert!(
        error.to_string().contains("Enter that address instead"),
        "{error}"
    );
}

#[tokio::test]
async fn a_relative_redirect_names_the_same_server() {
    let server = serve(Answer::http(
        "302 Found",
        &[("location", "/login?next=/api/v1/meta")],
        b"",
    ))
    .await;
    let error = check(&server.origin).await.unwrap_err();
    assert_eq!(
        error,
        CheckError::Redirected {
            origin: server.origin.clone(),
            to: server.origin.clone(),
        }
    );
}

#[tokio::test]
async fn another_status_is_refused() {
    for status in [
        "404 Not Found",
        "500 Internal Server Error",
        "204 No Content",
    ] {
        let error = check_answer(Answer::http(status, &[], b""))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "status", "{status}");
    }
}

#[tokio::test]
async fn an_answer_that_is_not_json_is_refused() {
    let page = Answer::http(
        "200 OK",
        &[("content-type", "text/html")],
        b"<!doctype html><title>Someone else</title>",
    );
    assert_eq!(check_answer(page).await.unwrap_err().code(), "not_json");
    let untyped = Answer::http("200 OK", &[], br#"{"product":"neon-teamspace"}"#);
    assert_eq!(check_answer(untyped).await.unwrap_err().code(), "not_json");
}

#[tokio::test]
async fn an_answer_over_the_cap_is_refused_by_its_length() {
    let body = vec![b' '; MAX_ANSWER_BYTES + 1];
    let answer = Answer::http("200 OK", &[("content-type", "application/json")], &body);
    assert_eq!(check_answer(answer).await.unwrap_err().code(), "too_large");
}

#[tokio::test]
async fn an_answer_over_the_cap_is_refused_as_it_streams() {
    // Chunked, with no length to read beforehand: four chunks of 20 KiB.
    let mut body = Vec::new();
    for _ in 0..4 {
        body.extend_from_slice(format!("{:x}\r\n", 20 * 1024).as_bytes());
        body.extend_from_slice(&[b' '; 20 * 1024]);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let mut bytes =
        b"HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n"
            .to_vec();
    bytes.extend_from_slice(&body);
    assert_eq!(
        check_answer(Answer::Bytes(bytes)).await.unwrap_err().code(),
        "too_large"
    );
}

#[tokio::test]
async fn an_answer_at_the_cap_is_read() {
    let json = r#"{"product":"neon-teamspace","name":"Acme","version":"0.1.0"}"#;
    let mut body = json.as_bytes().to_vec();
    body.resize(MAX_ANSWER_BYTES, b' ');
    let answer = Answer::http("200 OK", &[("content-type", "application/json")], &body);
    assert_eq!(check_answer(answer).await.unwrap().name, "Acme");
}

#[tokio::test]
async fn json_of_another_product_is_refused() {
    let answer = Answer::json(r#"{"name":"Gitea","version":"1.22.0"}"#);
    assert_eq!(check_answer(answer).await.unwrap_err().code(), "not_neon");
}

#[tokio::test]
async fn a_server_that_never_answers_times_out() {
    let server = serve(Answer::Silence).await;
    let address = ServerAddress::parse(&server.origin).unwrap();
    let options = CheckOptions {
        timeout: Duration::from_millis(300),
    };
    let error = meta::check(&address, options).await.unwrap_err();
    assert_eq!(
        error,
        CheckError::TimedOut {
            origin: server.origin.clone(),
            seconds: 1,
        }
    );
}

#[tokio::test]
async fn nothing_listening_is_unreachable() {
    let origin = format!("http://127.0.0.1:{}", closed_port().await);
    let error = check(&origin).await.unwrap_err();
    assert_eq!(error.code(), "unreachable");
    assert!(
        error
            .to_string()
            .starts_with(&format!("Could not reach {origin}: ")),
        "{error}"
    );
}

#[tokio::test]
async fn https_to_a_plain_http_server_on_a_private_address_suggests_http() {
    let server = serve(Answer::neon("Acme")).await;
    let origin = server.origin.replace("http://", "https://");
    let error = check(&origin).await.unwrap_err();
    assert_eq!(error.code(), "unreachable", "{error}");
    assert!(
        error
            .to_string()
            .contains("enter its address with http:// in front"),
        "{error}"
    );
}

#[tokio::test]
async fn garbage_instead_of_http_is_unreachable() {
    let error = check_answer(Answer::Bytes(b"SSH-2.0-OpenSSH_9.9\r\n".to_vec()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unreachable", "{error}");
}
