//! Pages over HTTP: the routes, their answers and error codes, sharing with a guest, the
//! scopes they need and the origin check.

use axum::Router;
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::{Method, StatusCode};
use neon_app::crypto::Envelope;
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;

use super::auth_tests::{Answer, call, sign_in, user};
use super::tests::{state, state_with};
use super::workspaces_tests::{Browser, id, team};
use super::*;

fn titles(answer: &Answer) -> Vec<&str> {
    answer.body["pages"]
        .as_array()
        .unwrap_or_else(|| panic!("no pages: {}", answer.body))
        .iter()
        .map(|page| page["title"].as_str().expect("a title"))
        .collect()
}

/// The default teamspace of a workspace, General.
pub(super) async fn general(by: &Browser<'_>, workspace: &str) -> String {
    let listed = by
        .get(&format!("/api/v1/workspaces/{workspace}/teamspaces"))
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    listed.body["teamspaces"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// Makes a page; `place` names where, as the body does.
pub(super) async fn create(by: &Browser<'_>, workspace: &str, place: Value, title: &str) -> String {
    let mut body = place;
    body["title"] = json!(title);
    let created = by
        .post(&format!("/api/v1/workspaces/{workspace}/pages"), body)
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    id(&created)
}

/// Brings `email`, an account already, into the workspace as a guest.
pub(super) async fn guest<'a>(
    app: &'a Router,
    by: &Browser<'_>,
    workspace: &str,
    email: &str,
) -> Browser<'a> {
    let guest = Browser::sign_in(app, email).await;
    let sent = by
        .post(
            &format!("/api/v1/workspaces/{workspace}/invitations"),
            json!({"email": email, "role": "guest"}),
        )
        .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let accepted = guest
        .post(
            &format!("/api/v1/account/invitations/{}/accept", id(&sent)),
            json!({}),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    guest
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn pages_are_made_nested_changed_moved_trashed_and_restored(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;

    let made = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({
                "teamspace_id": general,
                "title": "Roadmap",
                "icon": "🗺",
                "cover": "accent-gradient",
                "style": {"font": "serif"},
            }),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["teamspace_id"], general.as_str());
    assert_eq!(made.body["owner_id"], Value::Null);
    assert_eq!(made.body["parent_id"], Value::Null);
    assert_eq!(made.body["position"], "a0");
    assert_eq!(made.body["access"], "edit");
    assert_eq!(made.body["cover"], "accent-gradient");
    assert_eq!(
        made.body["style"],
        json!({"font": "serif", "small_text": false, "full_width": false})
    );
    assert_eq!(made.body["trashed_at"], Value::Null);
    let root = id(&made);
    let child = create(&t.ada, ws, json!({"parent_id": root}), "Q1").await;
    let grandchild = create(&t.ada, ws, json!({"parent_id": child}), "Goals").await;
    let first = create(
        &t.ada,
        ws,
        json!({"teamspace_id": general, "placement": "first"}),
        "Welcome",
    )
    .await;
    create(
        &t.ada,
        ws,
        json!({"teamspace_id": general, "placement": {"after": first}}),
        "Second",
    )
    .await;
    let private = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"private": true, "title": "Notes"}),
        )
        .await;
    assert_eq!(private.status, StatusCode::CREATED, "{}", private.body);
    assert_eq!(private.body["teamspace_id"], Value::Null);
    assert!(private.body["owner_id"].is_string());

    for no_place in [
        json!({"title": "Lost"}),
        json!({"private": true, "parent_id": root}),
    ] {
        let refused = t
            .ada
            .post(&format!("/api/v1/workspaces/{ws}/pages"), no_place)
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
        assert_eq!(refused.code(), "invalid_input");
    }

    let tree = t
        .dee
        .get(&format!("/api/v1/teamspaces/{general}/pages"))
        .await;
    assert_eq!(tree.status, StatusCode::OK, "{}", tree.body);
    assert_eq!(
        titles(&tree),
        ["Welcome", "Second", "Roadmap", "Q1", "Goals"]
    );
    assert_eq!(tree.body["pages"][2]["child_count"], 1);
    assert_eq!(tree.body["pages"][3]["parent_id"], root.as_str());
    assert_eq!(tree.body["next"], Value::Null);
    let part = t
        .dee
        .get(&format!("/api/v1/teamspaces/{general}/pages?limit=2"))
        .await;
    assert_eq!(titles(&part), ["Welcome", "Second"]);
    let cursor = part.body["next"].as_str().expect("more").to_owned();
    let rest = t
        .dee
        .get(&format!(
            "/api/v1/teamspaces/{general}/pages?limit=2&after={cursor}"
        ))
        .await;
    assert_eq!(titles(&rest), ["Roadmap", "Q1"]);
    let mine = t
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/private-pages"))
        .await;
    assert_eq!(titles(&mine), ["Notes"]);
    let theirs = t
        .dee
        .get(&format!("/api/v1/workspaces/{ws}/private-pages"))
        .await;
    assert_eq!(titles(&theirs), Vec::<&str>::new());
    let hidden = t.dee.get(&format!("/api/v1/pages/{}", id(&private))).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.code(), "not_found");

    let changed = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{root}"),
            Some(json!({"title": "Plan", "icon": null, "cover": null, "style": {"full_width": true}})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["title"], "Plan");
    assert_eq!(changed.body["icon"], Value::Null);
    assert_eq!(changed.body["cover"], Value::Null);
    assert_eq!(changed.body["style"]["font"], "serif");
    assert_eq!(changed.body["style"]["full_width"], true);
    let unknown = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{root}"),
            Some(json!({"cover": "https://example.com/cover.png"})),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown.code(), "invalid_input");

    // Between teamspaces, with what is under it.
    let open = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Launch", "access": "open"}),
        )
        .await;
    assert_eq!(open.status, StatusCode::CREATED, "{}", open.body);
    let open = id(&open);
    let moved = t
        .ada
        .post(
            &format!("/api/v1/pages/{child}/move"),
            json!({"teamspace_id": open, "placement": "first"}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.body["teamspace_id"], open.as_str());
    assert_eq!(moved.body["parent_id"], Value::Null);
    let below = t.ada.get(&format!("/api/v1/pages/{grandchild}")).await;
    assert_eq!(below.body["teamspace_id"], open.as_str());
    let crumbs = t
        .ada
        .get(&format!("/api/v1/pages/{grandchild}/breadcrumbs"))
        .await;
    assert_eq!(crumbs.status, StatusCode::OK, "{}", crumbs.body);
    assert_eq!(titles(&crumbs), ["Q1", "Goals"]);
    let cycle = t
        .ada
        .post(
            &format!("/api/v1/pages/{child}/move"),
            json!({"parent_id": grandchild}),
        )
        .await;
    assert_eq!(cycle.status, StatusCode::CONFLICT, "{}", cycle.body);
    assert_eq!(cycle.code(), "cycle");
    let viewer = t
        .dee
        .post(
            &format!("/api/v1/pages/{child}/move"),
            json!({"teamspace_id": general}),
        )
        .await;
    assert_eq!(viewer.status, StatusCode::FORBIDDEN, "{}", viewer.body);

    // The trash and back.
    let trashed = t
        .ada
        .post(&format!("/api/v1/pages/{child}/trash"), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    assert!(trashed.body["trashed_at"].is_string());
    assert_eq!(trashed.body["trashed_with"], child.as_str());
    let trash = t.ada.get(&format!("/api/v1/workspaces/{ws}/trash")).await;
    assert_eq!(trash.status, StatusCode::OK, "{}", trash.body);
    assert_eq!(titles(&trash), ["Q1"]);
    let gone = t.ada.get(&format!("/api/v1/teamspaces/{open}/pages")).await;
    assert_eq!(titles(&gone), Vec::<&str>::new());
    let live = t
        .ada
        .send(Method::DELETE, &format!("/api/v1/pages/{root}"), None)
        .await;
    assert_eq!(
        (live.status, live.code()),
        (StatusCode::BAD_REQUEST, "invalid_input"),
        "{}",
        live.body
    );
    let restored = t
        .ada
        .post(&format!("/api/v1/pages/{child}/restore"), json!({}))
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.body);
    assert_eq!(restored.body["trashed_at"], Value::Null);
    let back = t.ada.get(&format!("/api/v1/teamspaces/{open}/pages")).await;
    assert_eq!(titles(&back), ["Q1", "Goals"]);

    t.ada
        .post(&format!("/api/v1/pages/{child}/trash"), json!({}))
        .await;
    let deleted = t
        .ada
        .send(Method::DELETE, &format!("/api/v1/pages/{child}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let after = t.ada.get(&format!("/api/v1/pages/{grandchild}")).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_guest_sees_only_the_pages_shared_with_them(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let me = gus.get("/api/v1/account/me").await;
    let gus_id = me.body["id"].as_str().expect("an id").to_owned();
    let a = create(&t.ada, ws, json!({"teamspace_id": general}), "A").await;
    let b = create(&t.ada, ws, json!({"parent_id": a}), "B").await;
    let c = create(&t.ada, ws, json!({"parent_id": b}), "C").await;
    let d = create(&t.ada, ws, json!({"teamspace_id": general}), "D").await;

    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{b}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    assert_eq!(shared.body["user"]["email"], "gus@example.com");
    assert_eq!(shared.body["access"], "can_view");
    assert_eq!(shared.body["inherited"], false);
    assert_eq!(shared.body["page_id"], b.as_str());

    let seen = gus.get(&format!("/api/v1/pages/{b}")).await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.body);
    assert_eq!(seen.body["access"], "view");
    // Neither the page above, which is not shared with them, nor who made the page, as a
    // guest knows no other member.
    for hidden in ["parent_id", "created_by", "last_edited_by", "owner_id"] {
        assert_eq!(seen.body[hidden], Value::Null, "{hidden}");
    }
    let by_member = t.cy.get(&format!("/api/v1/pages/{b}")).await;
    assert_eq!(by_member.body["parent_id"], a.as_str());
    assert!(by_member.body["created_by"].is_string());
    let below = gus.get(&format!("/api/v1/pages/{c}")).await;
    assert_eq!(below.status, StatusCode::OK);
    assert_eq!(below.body["parent_id"], b.as_str());
    for unseen in [&a, &d] {
        let answer = gus.get(&format!("/api/v1/pages/{unseen}")).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    }
    let tree = gus
        .get(&format!("/api/v1/workspaces/{ws}/shared-pages"))
        .await;
    assert_eq!(tree.status, StatusCode::OK, "{}", tree.body);
    assert_eq!(titles(&tree), ["B", "C"]);
    assert_eq!(tree.body["pages"][0]["parent_id"], Value::Null);
    let teamspace = gus
        .get(&format!("/api/v1/teamspaces/{general}/pages"))
        .await;
    assert_eq!(teamspace.status, StatusCode::NOT_FOUND);
    let private = gus
        .get(&format!("/api/v1/workspaces/{ws}/private-pages"))
        .await;
    assert_eq!(private.status, StatusCode::FORBIDDEN);
    let crumbs = gus.get(&format!("/api/v1/pages/{c}/breadcrumbs")).await;
    assert_eq!(titles(&crumbs), ["B", "C"]);
    let edit = gus
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{c}"),
            Some(json!({"title": "Mine now"})),
        )
        .await;
    assert_eq!(edit.status, StatusCode::FORBIDDEN, "{}", edit.body);
    assert_eq!(edit.code(), "forbidden");
    let listing = gus.get(&format!("/api/v1/pages/{b}/shares")).await;
    assert_eq!(listing.status, StatusCode::FORBIDDEN);

    let inherited = t.cy.get(&format!("/api/v1/pages/{c}/shares")).await;
    assert_eq!(inherited.status, StatusCode::OK, "{}", inherited.body);
    assert_eq!(inherited.body["shares"][0]["inherited"], true);
    assert_eq!(inherited.body["shares"][0]["page_id"], b.as_str());

    let upgraded = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{b}/shares/{gus_id}"),
            Some(json!({"access": "can_edit"})),
        )
        .await;
    assert_eq!(upgraded.status, StatusCode::OK, "{}", upgraded.body);
    assert_eq!(upgraded.body["access"], "can_edit");
    let edited = gus
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{c}"),
            Some(json!({"title": "Edited"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    let again = t
        .ada
        .post(
            &format!("/api/v1/pages/{b}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.code(), "already_shared");

    // Favorites, for the guest too.
    let added = gus
        .post("/api/v1/account/favorites", json!({"page_id": c}))
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    gus.post("/api/v1/account/favorites", json!({"page_id": b}))
        .await;
    let reordered = gus
        .post(
            &format!("/api/v1/account/favorites/{b}/move"),
            json!({"placement": "first"}),
        )
        .await;
    assert_eq!(
        reordered.status,
        StatusCode::NO_CONTENT,
        "{}",
        reordered.body
    );
    let favorites = gus
        .get(&format!("/api/v1/account/favorites?workspace_id={ws}"))
        .await;
    assert_eq!(favorites.status, StatusCode::OK, "{}", favorites.body);
    assert_eq!(titles(&favorites), ["B", "Edited"]);
    assert_eq!(favorites.body["pages"][0]["parent_id"], Value::Null);
    assert_eq!(favorites.body["pages"][1]["parent_id"], b.as_str());
    let unseen = gus
        .post("/api/v1/account/favorites", json!({"page_id": a}))
        .await;
    assert_eq!(unseen.status, StatusCode::NOT_FOUND);

    let removed = t
        .ada
        .send(
            Method::DELETE,
            &format!("/api/v1/pages/{b}/shares/{gus_id}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(
        gus.get(&format!("/api/v1/pages/{b}")).await.status,
        StatusCode::NOT_FOUND
    );
    let none = gus.get("/api/v1/account/favorites").await;
    assert_eq!(titles(&none), Vec::<&str>::new());
    let unfavored = gus
        .send(
            Method::DELETE,
            &format!("/api/v1/account/favorites/{b}"),
            None,
        )
        .await;
    assert_eq!(unfavored.status, StatusCode::NO_CONTENT);
    let twice = gus
        .send(
            Method::DELETE,
            &format!("/api/v1/account/favorites/{b}"),
            None,
        )
        .await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn changes_need_the_write_scope_and_the_server_s_own_origin(pool: PgPool) {
    let public = Url::parse("https://neon.example.com").expect("a URL");
    let state = state_with(
        pool,
        Envelope::default(),
        WebConfig::new(Some(&public), Vec::new()),
    )
    .await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let cookie = sign_in(&app, "ada@example.com").await;
    let same_origin = [
        (COOKIE.as_str(), cookie.as_str()),
        (ORIGIN.as_str(), "https://neon.example.com"),
    ];
    let created = call(
        &app,
        Method::POST,
        "/api/v1/workspaces",
        &same_origin,
        Some(json!({"name": "Design"})),
    )
    .await;
    let ws = id(&created);
    let page = call(
        &app,
        Method::POST,
        &format!("/api/v1/workspaces/{ws}/pages"),
        &same_origin,
        Some(json!({"private": true, "title": "Notes"})),
    )
    .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.body);
    let page = id(&page);

    let cross = call(
        &app,
        Method::POST,
        &format!("/api/v1/pages/{page}/trash"),
        &[
            (COOKIE.as_str(), cookie.as_str()),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);

    let made = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &same_origin,
        Some(json!({"name": "reader", "scopes": ["read"]})),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let bearer = format!("Bearer {}", made.body["token"].as_str().expect("a token"));
    let token = [(AUTHORIZATION.as_str(), bearer.as_str())];
    let read = call(
        &app,
        Method::GET,
        &format!("/api/v1/pages/{page}"),
        &token,
        None,
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    let write = call(
        &app,
        Method::POST,
        &format!("/api/v1/pages/{page}/trash"),
        &token,
        Some(json!({})),
    )
    .await;
    assert_eq!(write.status, StatusCode::FORBIDDEN);
    assert_eq!(write.code(), "insufficient_scope");
    let unauthenticated = call(
        &app,
        Method::GET,
        &format!("/api/v1/pages/{page}"),
        &[],
        None,
    )
    .await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_share_neither_moves_a_page_away_nor_tells_of_pages_above_it(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let mut ids = Vec::new();
    for browser in [&t.bob, &t.cy] {
        let me = browser.get("/api/v1/account/me").await;
        ids.push(me.body["id"].as_str().expect("an id").to_owned());
    }
    let (bob_id, cy_id) = (&ids[0], &ids[1]);
    // Ada's private Diary, with Entry under it: Bob may view all of it, Cy edit Entry.
    let diary = create(&t.ada, ws, json!({"private": true}), "Diary").await;
    let entry = create(&t.ada, ws, json!({"parent_id": diary}), "Entry").await;
    for (page, user, access) in [(&diary, bob_id, "can_view"), (&entry, cy_id, "can_edit")] {
        let shared = t
            .ada
            .post(
                &format!("/api/v1/pages/{page}/shares"),
                json!({"user_id": user, "access": access}),
            )
            .await;
        assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    }

    // Cy edits Entry, and may not take it out of Ada's private pages.
    let move_to = |place: Value| {
        let cy = &t.cy;
        let entry = entry.clone();
        async move { cy.post(&format!("/api/v1/pages/{entry}/move"), place).await }
    };
    for place in [json!({"private": true}), json!({"teamspace_id": general})] {
        let moved = move_to(place.clone()).await;
        assert_eq!(
            moved.status,
            StatusCode::FORBIDDEN,
            "{place}: {}",
            moved.body
        );
        assert_eq!(moved.code(), "forbidden");
    }
    let kept = t.ada.get(&format!("/api/v1/pages/{entry}")).await;
    assert_eq!(kept.status, StatusCode::OK);
    assert_eq!(kept.body["parent_id"], diary.as_str());
    assert!(kept.body["owner_id"].is_string());

    // Cy sees Entry, not Diary: Bob's share from above shows, not where it is set.
    let seen = t.cy.get(&format!("/api/v1/pages/{entry}")).await;
    assert_eq!(seen.body["parent_id"], Value::Null);
    let listed = t.cy.get(&format!("/api/v1/pages/{entry}/shares")).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let shares = listed.body["shares"].as_array().expect("a list");
    assert_eq!(shares.len(), 2, "{}", listed.body);
    assert_eq!(shares[0]["page_id"], entry.as_str());
    assert_eq!(shares[0]["inherited"], false);
    assert_eq!(shares[1]["user"]["email"], "bob@example.com");
    assert_eq!(shares[1]["inherited"], true);
    assert_eq!(shares[1]["page_id"], Value::Null);
    // Bob, who sees Diary, is told where it is set.
    let listed = t.bob.get(&format!("/api/v1/pages/{entry}/shares")).await;
    let shares = listed.body["shares"].as_array().expect("a list");
    assert!(shares.iter().all(|share| share["page_id"].is_string()));
}
