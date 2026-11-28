//! The app on Tauri's mock runtime, with its real configuration and capabilities: which
//! origins may call its commands, and the picker's flow through them.

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use neon_shell::lock::Navigation;
use neon_shell::meta::CheckOptions;
use neon_shell::{DataDir, Limits, MAIN_WINDOW};
use serde_json::{Value, json};
use support::{Answer, serve};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, AppHandle, Manager, Url, WebviewWindow};

/// Where the app's own pages are, as Tauri serves them on this platform.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

fn app(dir: &Path) -> App<MockRuntime> {
    app_with(dir, Limits::default())
}

fn app_with(dir: &Path, limits: Limits) -> App<MockRuntime> {
    let mut app = neon_shell::shell(mock_builder())
        .build(tauri::generate_context!())
        .expect("the app builds");
    neon_shell::setup(&mut app, &DataDir::At(dir.to_path_buf()), limits).expect("the app sets up");
    app
}

fn window<M: Manager<MockRuntime>>(app: &M) -> WebviewWindow<MockRuntime> {
    app.get_webview_window(MAIN_WINDOW)
        .expect("the main window")
}

/// Calls `command` with `args` as a page at `from` would.
fn call<M: Manager<MockRuntime>>(
    app: &M,
    from: &str,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let request = InvokeRequest {
        cmd: command.to_owned(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: from.parse().expect("a URL"),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_owned(),
    };
    get_ipc_response(&window(app), request).map(|body| body.deserialize().expect("JSON"))
}

fn from_picker<M: Manager<MockRuntime>>(
    app: &M,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    call(app, &format!("{APP_ORIGIN}/index.html"), command, args)
}

/// Where the picker is: Tauri's own origin in a release build, the development server's in
/// the debug builds tests run.
fn picker(app: &App<MockRuntime>) -> String {
    let url = window(app).url().unwrap();
    let origin = url.origin().ascii_serialization();
    assert!(
        [APP_ORIGIN, "http://localhost:1420"].contains(&origin.as_str()),
        "the window starts on {url}"
    );
    url.to_string()
}

/// Whether the answer is the access control's refusal, rather than the command's own error.
fn denied(answer: &Result<Value, Value>) -> bool {
    answer
        .as_ref()
        .err()
        .and_then(Value::as_str)
        .is_some_and(|message| message.contains("not allowed"))
}

#[test]
fn the_window_starts_on_the_picker() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    picker(&app);
}

#[test]
fn no_other_origin_may_call_a_command() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let commands = [
        ("startup", json!({})),
        (
            "connect",
            json!({"address": "https://neon.example.com", "acceptInsecure": false}),
        ),
        ("open_server", json!({"origin": "https://neon.example.com"})),
        (
            "rename_server",
            json!({"origin": "https://neon.example.com", "label": "x"}),
        ),
        (
            "remove_server",
            json!({"origin": "https://neon.example.com"}),
        ),
        ("cancel_opening", json!({})),
        ("check_for_updates", json!({})),
        (
            "plugin:opener|open_url",
            json!({"url": "https://example.com"}),
        ),
        ("plugin:log|log", json!({"level": 3, "message": "x"})),
        ("plugin:updater|check", json!({})),
        ("plugin:updater|download_and_install", json!({"rid": 0})),
        ("plugin:dialog|message", json!({"message": "x"})),
        ("plugin:process|restart", json!({})),
    ];
    for origin in [
        "https://neon.example.com/",
        "http://192.168.1.20:3000/",
        "https://tauri.localhost.example.com/",
        "https://localhost/",
    ] {
        for (command, args) in &commands {
            let refused = call(&app, origin, command, args.clone());
            assert!(denied(&refused), "{origin} called {command}: {refused:?}");
        }
    }
}

#[test]
fn the_picker_may_not_call_plugins_either() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    for (command, args) in [
        (
            "plugin:opener|open_url",
            json!({"url": "https://example.com"}),
        ),
        ("plugin:opener|open_path", json!({"path": "/"})),
        ("plugin:log|log", json!({"level": 3, "message": "x"})),
        ("plugin:webview|create_webview_window", json!({})),
        (
            "plugin:event|listen",
            json!({"event": "x", "target": {"kind": "Any"}, "handler": 0}),
        ),
        // The app's Rust side alone checks for updates, installs them and asks about them.
        ("plugin:updater|check", json!({})),
        ("plugin:updater|download", json!({"rid": 0})),
        ("plugin:updater|install", json!({"rid": 0})),
        ("plugin:updater|download_and_install", json!({"rid": 0})),
        ("plugin:dialog|message", json!({"message": "x"})),
        ("plugin:dialog|open", json!({})),
        ("plugin:process|restart", json!({})),
        ("plugin:process|exit", json!({"code": 0})),
    ] {
        let refused = from_picker(&app, command, args);
        assert!(denied(&refused), "the picker called {command}: {refused:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_commands_run_only_while_the_window_shows_the_apps_page() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let server = serve(Answer::neon("Acme")).await;
    from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": true}),
    )
    .unwrap();
    // The server's page has committed. A message that names the picker's URL, as WebKitGTK's
    // do for the page the window is loading, while that page is still there, gets nothing.
    let lock = neon_shell::lock(app.handle());
    let now = Instant::now();
    lock.started(&server.origin.parse().unwrap(), now);
    for (command, args) in [
        ("startup", json!({})),
        (
            "connect",
            json!({"address": server.origin, "acceptInsecure": true}),
        ),
        ("open_server", json!({"origin": server.origin})),
        (
            "rename_server",
            json!({"origin": server.origin, "label": "taken"}),
        ),
        ("remove_server", json!({"origin": server.origin})),
        ("cancel_opening", json!({})),
        ("check_for_updates", json!({})),
    ] {
        let error = from_picker(&app, command, args).unwrap_err();
        assert_eq!(error["code"], "not_the_picker", "{command}: {error}");
    }
    let file = std::fs::read_to_string(dir.path().join("servers.json")).unwrap();
    assert!(!file.contains("taken"), "{file}");

    // Back on the picker, they run again.
    lock.started(&lock.picker().clone(), now);
    let list = from_picker(&app, "startup", json!({})).unwrap();
    assert_eq!(list["servers"][0]["origin"], server.origin);
}

#[test]
fn the_picker_starts_with_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    assert_eq!(
        from_picker(&app, "startup", json!({})),
        Ok(json!({
            "servers": [],
            "last": null,
            "open": null,
            "interrupted": null,
            // The desktop apps check for updates from their menu.
            "updateCheck": false,
        }))
    );
}

#[test]
fn an_address_that_is_not_one_is_refused_before_any_check() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    for (address, code) in [
        ("", "empty"),
        ("ftp://neon.example.com", "scheme"),
        ("https://ada:secret@neon.example.com", "credentials"),
        ("http://neon.example.com", "insecure_public"),
    ] {
        let error = from_picker(
            &app,
            "connect",
            json!({"address": address, "acceptInsecure": true}),
        )
        .unwrap_err();
        assert_eq!(error["code"], code, "{address}: {error}");
        assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    assert!(!dir.path().join("servers.json").exists());
}

/// What the window does with a navigation to `url` now.
fn navigation(app: &App<MockRuntime>, url: &str) -> Navigation {
    neon_shell::navigation(app.handle(), &url.parse::<Url>().expect("a URL"))
}

/// Whether the navigation stays out of the window, opening outside the app.
fn outside(navigation: &Navigation) -> bool {
    matches!(navigation, Navigation::OpenOutside(_) | Navigation::TooSoon)
}

fn download_allowed(app: &App<MockRuntime>, url: &str) -> bool {
    neon_shell::download_allowed(app.handle(), &url.parse::<Url>().expect("a URL"))
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_needs_the_warning_accepted_first() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let mut server = serve(Answer::neon("Acme")).await;
    let before = picker(&app);
    let error = from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": false}),
    )
    .unwrap_err();
    assert_eq!(error["code"], "insecure");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|m| m.contains("your password included")),
        "{error}"
    );
    // Nothing was checked, saved or opened: the server has had no request at all, as the
    // command answered only once any check it made was done.
    assert_eq!(server.requests_so_far(), Vec::<String>::new());
    assert_eq!(window(&app).url().unwrap().as_str(), before);
    assert!(!dir.path().join("servers.json").exists());
    assert!(outside(&navigation(&app, &server.origin)));
}

#[tokio::test(flavor = "multi_thread")]
async fn connecting_checks_saves_and_opens_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let server = serve(Answer::neon("Acme")).await;
    let origin = server.origin.clone();

    let list = from_picker(
        &app,
        "connect",
        json!({"address": format!("{origin}/w/acme"), "acceptInsecure": true}),
    )
    .unwrap();
    let saved = json!({"origin": origin, "name": "Acme", "version": "0.1.0", "encrypted": false});
    assert_eq!(list, json!({"servers": [saved], "last": origin}));
    assert_eq!(window(&app).url().unwrap().as_str(), format!("{origin}/"));
    let file: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("servers.json")).unwrap()).unwrap();
    assert_eq!(file["servers"][0]["origin"], origin);
    assert_eq!(file["last"], origin);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_fails_its_check_is_neither_saved_nor_opened() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let server = serve(Answer::http("404 Not Found", &[], b"")).await;
    let before = picker(&app);
    let error = from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": true}),
    )
    .unwrap_err();
    assert_eq!(error["code"], "status");
    assert_eq!(window(&app).url().unwrap().as_str(), before);
    assert!(!dir.path().join("servers.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_saved_servers_are_renamed_switched_between_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let first = serve(Answer::neon("Acme")).await;
    let second = serve(Answer::neon("Globex")).await;
    for server in [&first, &second] {
        from_picker(
            &app,
            "connect",
            json!({"address": server.origin, "acceptInsecure": true}),
        )
        .unwrap();
    }

    let list = from_picker(
        &app,
        "rename_server",
        json!({"origin": first.origin, "label": "  Work  "}),
    )
    .unwrap();
    assert_eq!(list["servers"][0]["label"], "Work");
    let error = from_picker(
        &app,
        "rename_server",
        json!({"origin": "https://gone.example", "label": "x"}),
    )
    .unwrap_err();
    assert_eq!(error["code"], "unknown_server");

    let list = from_picker(&app, "open_server", json!({"origin": first.origin})).unwrap();
    assert_eq!(list["last"], first.origin);
    assert_eq!(
        window(&app).url().unwrap().as_str(),
        format!("{}/", first.origin)
    );
    let error = from_picker(
        &app,
        "open_server",
        json!({"origin": "https://gone.example"}),
    )
    .unwrap_err();
    assert_eq!(error["code"], "unknown_server");

    let list = from_picker(&app, "remove_server", json!({"origin": first.origin})).unwrap();
    assert_eq!(list["servers"].as_array().map(Vec::len), Some(1));
    assert_eq!(list["servers"][0]["origin"], second.origin);
    assert_eq!(list["last"], Value::Null);
    let error = from_picker(&app, "remove_server", json!({"origin": first.origin})).unwrap_err();
    assert_eq!(error["code"], "unknown_server");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_server_is_offered_once_as_the_app_starts() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(Answer::neon("Acme")).await;
    {
        let app = app(dir.path());
        from_picker(
            &app,
            "connect",
            json!({"address": server.origin, "acceptInsecure": true}),
        )
        .unwrap();
    }
    let app = app(dir.path());
    let startup = from_picker(&app, "startup", json!({})).unwrap();
    assert_eq!(startup["open"], server.origin);
    assert_eq!(startup["last"], server.origin);
    // Back on the picker later, the list shows and nothing opens by itself.
    let again = from_picker(&app, "startup", json!({})).unwrap();
    assert_eq!(again["open"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lock_opens_to_each_server_opened_and_closes_as_it_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let first = serve(Answer::neon("Acme")).await;
    let second = serve(Answer::neon("Globex")).await;
    let page = |server: &support::Server| format!("{}/w/acme/p/1?x=1", server.origin);
    let file = |server: &support::Server| format!("{}/api/v1/files/abc", server.origin);

    // The picker's own pages load; no server's does yet, nor do its downloads.
    assert_eq!(navigation(&app, &picker(&app)), Navigation::Load);
    assert!(outside(&navigation(&app, &page(&first))));
    assert!(!download_allowed(&app, &file(&first)));

    from_picker(
        &app,
        "connect",
        json!({"address": first.origin, "acceptInsecure": true}),
    )
    .unwrap();
    assert_eq!(navigation(&app, &page(&first)), Navigation::Load);
    assert!(download_allowed(&app, &file(&first)));
    for elsewhere in [
        "https://example.com/",
        &first.origin.replace("127.0.0.1", "localhost"),
        &second.origin,
    ] {
        assert!(outside(&navigation(&app, elsewhere)), "{elsewhere}");
        assert!(!download_allowed(&app, elsewhere), "{elsewhere}");
    }
    assert_eq!(navigation(&app, "file:///etc/passwd"), Navigation::Refuse);

    // Opening a saved server opens the lock to it, as connecting does.
    from_picker(
        &app,
        "connect",
        json!({"address": second.origin, "acceptInsecure": true}),
    )
    .unwrap();
    from_picker(&app, "open_server", json!({"origin": first.origin})).unwrap();
    for server in [&first, &second] {
        assert_eq!(navigation(&app, &page(server)), Navigation::Load);
    }

    from_picker(&app, "remove_server", json!({"origin": first.origin})).unwrap();
    assert!(outside(&navigation(&app, &page(&first))));
    assert!(!download_allowed(&app, &file(&first)));
    assert_eq!(navigation(&app, &page(&second)), Navigation::Load);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_fails_its_check_stays_out_of_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let server = serve(Answer::http("404 Not Found", &[], b"")).await;
    from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": true}),
    )
    .unwrap_err();
    assert!(outside(&navigation(&app, &server.origin)));
}

/// Whether the window is on its way back to the picker, or there: the blank page first, where
/// the webview needs one.
fn back_to_the_picker(app: &App<MockRuntime>, picker: &str) -> bool {
    let url = window(app).url().unwrap();
    url.as_str() == "about:blank" || url.as_str() == picker
}

/// Waits, for at most ten seconds, until `condition` holds.
async fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "the condition did not hold in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_never_starts_loading_takes_the_window_back_to_the_picker() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        page: Duration::from_millis(200),
        ..Limits::default()
    };
    let app = app_with(dir.path(), limits);
    let server = serve(Answer::neon("Acme")).await;
    let picker = picker(&app);
    from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": true}),
    )
    .unwrap();
    assert_eq!(
        window(&app).url().unwrap().as_str(),
        format!("{}/", server.origin)
    );
    // The mock runtime starts no page: the server's never begins to arrive.
    wait_until(|| back_to_the_picker(&app, &picker)).await;
    let startup = from_picker(&app, "startup", json!({})).unwrap();
    let interrupted = &startup["interrupted"];
    assert_eq!(interrupted["code"], "page_timed_out", "{startup}");
    assert_eq!(interrupted["origin"], server.origin);
    assert!(
        interrupted["message"]
            .as_str()
            .is_some_and(|message| message.contains("its page did not start to arrive")),
        "{interrupted}"
    );
    // The server stays saved, and the picker hears of it once.
    assert_eq!(startup["servers"][0]["origin"], server.origin);
    let again = from_picker(&app, "startup", json!({})).unwrap();
    assert_eq!(again["interrupted"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_stops_the_page_loading_and_goes_back_to_the_picker() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let server = serve(Answer::neon("Acme")).await;
    let picker = picker(&app);
    from_picker(
        &app,
        "connect",
        json!({"address": server.origin, "acceptInsecure": true}),
    )
    .unwrap();
    assert_eq!(
        from_picker(&app, "cancel_opening", json!({})),
        Ok(json!(true))
    );
    assert!(back_to_the_picker(&app, &picker));
    let startup = from_picker(&app, "startup", json!({})).unwrap();
    assert_eq!(
        startup["interrupted"],
        json!({
            "origin": server.origin,
            "code": "cancelled",
            "message": format!("Stopped opening {}.", server.origin),
        })
    );
    // Nothing is left to cancel.
    assert_eq!(
        from_picker(&app, "cancel_opening", json!({})),
        Ok(json!(false))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_the_check_opens_and_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        check: CheckOptions {
            timeout: Duration::from_secs(3),
        },
        ..Limits::default()
    };
    let app = app_with(dir.path(), limits);
    let mut server = serve(Answer::Silence).await;
    let picker = picker(&app);
    let handle: AppHandle<MockRuntime> = app.handle().clone();
    let address = server.origin.clone();
    let connecting = tokio::task::spawn_blocking(move || {
        from_picker(
            &handle,
            "connect",
            json!({"address": address, "acceptInsecure": true}),
        )
    });
    // The check has reached the server, which keeps it waiting.
    server.next_request().await;
    assert_eq!(
        from_picker(&app, "cancel_opening", json!({})),
        Ok(json!(false))
    );
    let error = connecting.await.unwrap().unwrap_err();
    assert_eq!(error["code"], "cancelled", "{error}");
    assert_eq!(window(&app).url().unwrap().as_str(), picker);
    assert!(!dir.path().join("servers.json").exists());
    assert!(outside(&navigation(&app, &server.origin)));
}
