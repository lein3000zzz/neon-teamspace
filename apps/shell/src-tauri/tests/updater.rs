//! Tauri's updater as the desktop apps configure it, on the mock runtime, against a server on
//! loopback that serves a `latest.json` and an update signed with a key each test makes and
//! throws away: a good signature is taken, and a bad one refused before anything is installed.
//! Then the configuration the releases rely on.

#![cfg(desktop)]

mod support;

use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign::KeyPair;
use serde_json::{Value, json};
use support::{Answer, serve_paths};
use tauri::App;
use tauri::test::{MockRuntime, mock_builder};
use tauri_plugin_updater::{Error, Update, UpdaterExt as _};

/// The feed the releases publish, as the apps' configuration must name it.
const FEED: &str =
    "https://github.com/lein3000zzz/neon-teamspace/releases/latest/download/latest.json";

/// The public key whose private half signs the releases' updates.
const RELEASE_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDM3QUZDNkYzNTFCODA4QTcKUldTbkNMaFI4OGF2TjA4YmZ3ajAzZDZ6L2o4eWhGdGMwRDFDRkhFUzVTQWdvKzErSXB0T3hoVVYK";

/// The version the tests' releases have, newer than any the app has.
const NEWER: &str = "9.9.9";

/// What the tests' update holds.
const UPDATE: &[u8] = b"the bytes of an update, as a test serves them";

fn app() -> App<MockRuntime> {
    neon_shell::shell(mock_builder())
        .build(tauri::generate_context!())
        .expect("the app builds")
}

/// A throwaway key pair, and its public key as Tauri's configuration takes it: the minisign
/// public key file, in base64.
fn keys() -> (KeyPair, String) {
    let keys = KeyPair::generate_unencrypted_keypair().expect("a key pair");
    let public = keys
        .pk
        .to_box()
        .expect("the public key's file")
        .into_string();
    (keys, STANDARD.encode(public))
}

/// The signature of `bytes` by `keys`, as Tauri's signer writes it for release `version`: the
/// minisign signature file, in base64, whose trusted comment names the version, if any.
fn sign(keys: &KeyPair, bytes: &[u8], version: Option<&str>) -> String {
    let mut comment = "timestamp:1764000000\tfile:Neon Teamspace.app.tar.gz".to_owned();
    if let Some(version) = version {
        comment.push_str("\tversion:");
        comment.push_str(version);
    }
    let signature = minisign::sign(
        None,
        &keys.sk,
        Cursor::new(bytes),
        Some(&comment),
        Some("signature from tauri secret key"),
    )
    .expect("a signature");
    STANDARD.encode(signature.to_string())
}

/// This platform's key in `latest.json`, as the updater looks it up.
fn platform() -> String {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    format!("{os}-{arch}")
}

/// `latest.json` of release `version`, whose every platform names `url` and `signature`.
fn manifest(version: &str, url: &str, signature: &str) -> String {
    let entry = json!({"signature": signature, "url": url});
    let platforms: serde_json::Map<String, Value> = [
        "darwin-aarch64".to_owned(),
        "darwin-x86_64".to_owned(),
        "windows-x86_64".to_owned(),
        "linux-x86_64".to_owned(),
        platform(),
    ]
    .into_iter()
    .map(|key| (key, entry.clone()))
    .collect();
    json!({
        "version": version,
        "notes": "A release for the tests.",
        "pub_date": "2026-11-28T12:00:00Z",
        "platforms": platforms,
    })
    .to_string()
}

/// A server with the update, and one with release `version`'s `latest.json`, which names the
/// update with `signature`: the origin of the second.
async fn release(version: &str, signature: &str) -> String {
    let files = serve_paths(vec![(
        "/update".to_owned(),
        Answer::http(
            "200 OK",
            &[("content-type", "application/octet-stream")],
            UPDATE,
        ),
    )])
    .await;
    let manifest = manifest(version, &format!("{}/update", files.origin), signature);
    serve_paths(vec![("/latest.json".to_owned(), Answer::json(&manifest))])
        .await
        .origin
}

/// Checks the release whose feed is at `origin` with the app's updater, trusting `key`.
async fn check(app: &App<MockRuntime>, origin: &str, key: &str) -> Result<Option<Update>, Error> {
    app.updater_builder()
        .endpoints(vec![
            format!("{origin}/latest.json").parse().expect("a URL"),
        ])?
        .pubkey(key)
        .build()?
        .check()
        .await
}

/// The update the release whose feed is at `origin` announces.
async fn announced(app: &App<MockRuntime>, origin: &str, key: &str) -> Update {
    check(app, origin, key)
        .await
        .expect("the check succeeds")
        .expect("a newer release")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_signed_with_the_trusted_key_is_downloaded_and_verified() {
    let app = app();
    let (keys, key) = keys();
    let origin = release(NEWER, &sign(&keys, UPDATE, Some(NEWER))).await;
    let update = announced(&app, &origin, &key).await;
    assert_eq!(update.version, NEWER);
    assert_eq!(update.current_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(update.body.as_deref(), Some("A release for the tests."));
    // The download verifies the signature, and gives the bytes only when it holds.
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .expect("a verified download");
    assert_eq!(bytes, UPDATE);
    update.verify(&bytes).expect("the signature holds");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_whose_signature_is_of_other_bytes_is_refused() {
    let app = app();
    let (keys, key) = keys();
    let origin = release(NEWER, &sign(&keys, b"another update", Some(NEWER))).await;
    let update = announced(&app, &origin, &key).await;
    let refused = update.download(|_, _| {}, || {}).await;
    assert!(matches!(refused, Err(Error::Minisign(_))), "{refused:?}");
    assert!(matches!(update.verify(UPDATE), Err(Error::Minisign(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_signed_with_another_key_is_refused() {
    let app = app();
    let (_, key) = keys();
    let (other, _) = keys();
    let origin = release(NEWER, &sign(&other, UPDATE, Some(NEWER))).await;
    let update = announced(&app, &origin, &key).await;
    let refused = update.download(|_, _| {}, || {}).await;
    assert!(matches!(refused, Err(Error::Minisign(_))), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_release_signed_with_the_key_cannot_pass_for_a_newer_one() {
    let app = app();
    let (keys, key) = keys();
    // A good signature of an older release, which a tampered latest.json announces as newer.
    let origin = release(NEWER, &sign(&keys, UPDATE, Some("0.0.1"))).await;
    let update = announced(&app, &origin, &key).await;
    let refused = update.download(|_, _| {}, || {}).await;
    assert!(
        matches!(refused, Err(Error::SignedVersionMismatch { .. })),
        "{refused:?}"
    );
    // One signed for no version at all, as older signers did, is refused too: the app's
    // configuration asks for the version.
    let origin = release(NEWER, &sign(&keys, UPDATE, None)).await;
    let update = announced(&app, &origin, &key).await;
    let refused = update.download(|_, _| {}, || {}).await;
    assert!(
        matches!(refused, Err(Error::MissingSignedVersion)),
        "{refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_no_newer_than_the_app_is_no_update() {
    let app = app();
    let (keys, key) = keys();
    // The app's own version, an older one, and a release candidate of the app's version, which
    // comes before it: whatever version the app has.
    let current = env!("CARGO_PKG_VERSION");
    let candidate = format!("{current}-rc.1");
    for version in [current, "0.0.9", candidate.as_str()] {
        let origin = release(version, &sign(&keys, UPDATE, Some(version))).await;
        let found = check(&app, &origin, &key)
            .await
            .expect("the check succeeds");
        assert!(found.is_none(), "{version} was taken for an update");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_feed_that_is_not_there_is_an_error_and_no_update() {
    let app = app();
    let (_, key) = keys();
    let origin = serve_paths(Vec::new()).await.origin;
    let found = check(&app, &origin, &key)
        .await
        .map(|update| update.is_some());
    assert!(matches!(found, Err(Error::ReleaseNotFound)), "{found:?}");
}

/// The updater's settings in tauri.conf.json.
fn updater_config() -> Value {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).expect("JSON");
    config["plugins"]["updater"].clone()
}

/// The keys of the object `value`, sorted, whatever order the map keeps them in.
fn fields(value: &Value) -> Vec<&str> {
    let mut fields: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    fields
}

#[test]
fn the_apps_read_the_releases_feed_and_trust_the_releases_key() {
    let config = updater_config();
    // These keys and no others: one more, such as allowDowngrades or a dangerous* flag, in
    // either spelling the plugin reads, would loosen what the apps accept, so it must come as
    // a deliberate change to this test.
    assert_eq!(
        fields(&config),
        ["endpoints", "pubkey", "requireSignedVersion", "windows"]
    );
    assert_eq!(fields(&config["windows"]), ["installMode"]);
    assert_eq!(config["endpoints"], json!([FEED]));
    assert_eq!(config["pubkey"], RELEASE_KEY);
    // The signature binds the version too, so that no older release passes for a newer one.
    assert_eq!(config["requireSignedVersion"], true);
    assert_eq!(config["windows"]["installMode"], "passive");
    let key = String::from_utf8(STANDARD.decode(RELEASE_KEY).expect("base64")).expect("text");
    assert!(
        key.starts_with("untrusted comment: minisign public key: "),
        "{key}"
    );
    assert!(
        minisign::PublicKeyBox::from_string(&key)
            .and_then(|key| key.into_public_key())
            .is_ok()
    );
}

#[test]
fn a_build_without_the_signing_key_makes_no_updater_files() {
    // The release workflow turns them on when it has the key; any other build needs none.
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).expect("JSON");
    assert_eq!(config["bundle"].get("createUpdaterArtifacts"), None);
}
