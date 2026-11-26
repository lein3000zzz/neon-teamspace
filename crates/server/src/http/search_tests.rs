//! Searching a workspace over HTTP: its results, with their marked runs, the filters as
//! query parameters, parts of the results, and the refusals.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::agents_tests::{Bearer, agent, agent_token};
use super::pages_tests::{general, guest};
use super::tests::state;
use super::workspaces_tests::{id, team};
use super::*;

fn titles(answer: &Value) -> Vec<&str> {
    answer["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["page"]["title"].as_str().expect("a title"))
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_workspace_is_searched_with_its_filters(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    for (title, markdown) in [
        ("Quarterly roadmap", "The harbour bridge ships in spring"),
        ("Harbour", ""),
    ] {
        let created = t
            .ada
            .post(
                &format!("/api/v1/workspaces/{ws}/pages"),
                json!({"teamspace_id": general, "title": title, "markdown": markdown}),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    }
    let search = format!("/api/v1/workspaces/{ws}/search");

    let found = t.dee.get(&format!("{search}?query=harbour")).await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.body);
    assert_eq!(titles(&found.body), ["Harbour", "Quarterly roadmap"]);
    let first = &found.body["results"][0];
    assert_eq!(
        first["title"],
        json!([{"text": "Harbour", "highlight": true}])
    );
    assert_eq!(first["page"]["access"], "edit");
    let snippet = &found.body["results"][1]["snippet"];
    assert_eq!(
        snippet[1],
        json!({"text": "harbour", "highlight": true}),
        "{snippet}"
    );
    assert!(found.body["results"][1]["rank"].as_f64() < first["rank"].as_f64());

    // Misspelled, a title still matches; a part at a time, the next names where it goes on.
    let loose = t.dee.get(&format!("{search}?query=quaterly")).await;
    assert_eq!(titles(&loose.body), ["Quarterly roadmap"]);
    let part = t.dee.get(&format!("{search}?query=harbour&limit=1")).await;
    assert_eq!(titles(&part.body), ["Harbour"]);
    let next = part.body["next"].as_str().expect("a next part");
    let rest = t
        .dee
        .get(&format!("{search}?query=harbour&limit=1&after={next}"))
        .await;
    assert_eq!(titles(&rest.body), ["Quarterly roadmap"]);
    assert_eq!(rest.body["next"], Value::Null);

    // Filters, without words too.
    let ada = t.ada.get("/api/v1/account/me").await.body["id"].clone();
    let rows = t.dee.get(&format!("{search}?query=harbour&kind=row")).await;
    assert_eq!(rows.body["results"], json!([]));
    let recent = t
        .dee
        .get(&format!(
            "{search}?edited_within=7d&created_by={}&teamspace_id={general}",
            ada.as_str().expect("an id")
        ))
        .await;
    assert_eq!(recent.status, StatusCode::OK, "{}", recent.body);
    assert_eq!(titles(&recent.body).len(), 2);

    for (query, status, code) in [
        (
            "edited_within=soon",
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        ("query=-draft", StatusCode::BAD_REQUEST, "invalid_search"),
        ("limit=0", StatusCode::BAD_REQUEST, "invalid_input"),
        ("kind=folder", StatusCode::BAD_REQUEST, "invalid_query"),
    ] {
        let refused = t.dee.get(&format!("{search}?{query}")).await;
        assert_eq!(refused.status, status, "{query}: {}", refused.body);
        assert_eq!(refused.body["error"]["code"], code, "{query}");
    }
    state.sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn search_keeps_to_who_may_see_what(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let created = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"teamspace_id": general, "title": "Budget", "markdown": "numbers"}),
        )
        .await;
    let page = id(&created);
    let search = format!("/api/v1/workspaces/{ws}/search?query=budget");

    super::auth_tests::user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let nothing = gus.get(&search).await;
    assert_eq!(nothing.status, StatusCode::OK, "{}", nothing.body);
    assert_eq!(nothing.body["results"], json!([]));
    let gus_id = gus.get("/api/v1/account/me").await.body["id"].clone();
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let found = gus.get(&search).await;
    assert_eq!(titles(&found.body), ["Budget"]);
    assert_eq!(found.body["results"][0]["page"]["created_by"], Value::Null);
    let filtered = gus
        .get(&format!(
            "{search}&created_by={}",
            gus_id.as_str().expect("an id")
        ))
        .await;
    assert_eq!(filtered.status, StatusCode::FORBIDDEN, "{}", filtered.body);

    // A token that reads searches; one bound to the workspace is all an agent has.
    let scout = agent(&t.ada, ws, "Scout").await;
    let reader = Bearer::new(&app, agent_token(&t.ada, ws, &scout, &["read"]).await);
    let found = reader.send(Method::GET, &search, None).await;
    assert_eq!(titles(&found.body), ["Budget"]);

    // Outside the workspace, it is not there.
    super::auth_tests::user(&state, "eve@example.com").await;
    let eve = super::workspaces_tests::Browser::sign_in(&app, "eve@example.com").await;
    let outside = eve.get(&search).await;
    assert_eq!(outside.status, StatusCode::NOT_FOUND, "{}", outside.body);
    state.sync.shutdown().await;
}

/// A limiter that refuses every search.
struct RefusingSearches;

#[async_trait::async_trait]
impl neon_app::ratelimit::RateLimiter for RefusingSearches {
    async fn check(
        &self,
        bucket: neon_app::ratelimit::Bucket,
        _key: &str,
    ) -> Result<(), neon_app::ratelimit::Limited> {
        match bucket {
            neon_app::ratelimit::Bucket::Search => Err(neon_app::ratelimit::Limited {
                retry_after: std::time::Duration::from_secs(7),
            }),
            _ => Ok(()),
        }
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn searches_past_their_rate_answer_429_and_queries_with_control_characters_400(pool: PgPool) {
    let state = super::tests::state_with_mcp(
        pool,
        WebConfig::new(None, Vec::new()),
        neon_app::sync::Limits::default(),
        std::sync::Arc::new(RefusingSearches),
    )
    .await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let search = format!("/api/v1/workspaces/{}/search", t.workspace);

    let refused = t.dee.get(&format!("{search}?query=a%00b")).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(refused.code(), "invalid_search");

    let limited = t.dee.get(&format!("{search}?query=plan")).await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    assert_eq!(limited.code(), "rate_limited");
    assert_eq!(limited.headers["retry-after"], "7");
}
