//! Agents over HTTP: their routes and answers, their tokens acting as them within their
//! owner's reach, how people and pages name them, and tokens bound to one workspace.

use axum::Router;
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::auth_tests::{Answer, admin, call};
use super::pages_tests::{create, general};
use super::tests::state;
use super::workspaces_tests::{Browser, id, team};
use super::*;

/// A client calling with a bearer token.
pub(super) struct Bearer<'a> {
    app: &'a Router,
    header: String,
}

impl<'a> Bearer<'a> {
    pub(super) fn new(app: &'a Router, header: String) -> Self {
        Self { app, header }
    }

    pub(super) async fn send(&self, method: Method, uri: &str, body: Option<Value>) -> Answer {
        call(
            self.app,
            method,
            uri,
            &[(AUTHORIZATION.as_str(), &self.header)],
            body,
        )
        .await
    }

    pub(super) async fn get(&self, uri: &str) -> Answer {
        self.send(Method::GET, uri, None).await
    }

    pub(super) async fn post(&self, uri: &str, body: Value) -> Answer {
        self.send(Method::POST, uri, Some(body)).await
    }

    /// The `Authorization` header value it sends.
    pub(super) fn header(&self) -> &str {
        &self.header
    }
}

/// An agent `by` makes in `workspace`: its id.
pub(super) async fn agent(by: &Browser<'_>, workspace: &str, name: &str) -> String {
    let made = by
        .post(
            &format!("/api/v1/workspaces/{workspace}/agents"),
            json!({"name": name}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    id(&made)
}

/// A token its owner makes for an agent, as a bearer header.
pub(super) async fn agent_token(
    by: &Browser<'_>,
    workspace: &str,
    agent: &str,
    scopes: &[&str],
) -> String {
    let made = by
        .post(
            &format!("/api/v1/workspaces/{workspace}/agents/{agent}/tokens"),
            json!({"name": "worker", "scopes": scopes}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    format!("Bearer {}", made.body["token"].as_str().expect("a token"))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn agents_are_made_listed_renamed_handed_over_and_deleted(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let agents = format!("/api/v1/workspaces/{ws}/agents");

    let made = team
        .cy
        .post(&agents, json!({"name": "Scout", "hue_override": 160}))
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["kind"], "agent");
    assert_eq!(made.body["status"], "active");
    assert_eq!(made.body["hue"], 160);
    assert_eq!(made.body["owner"]["display_name"], "cy");
    assert_eq!(made.body["deleted_at"], Value::Null);
    let scout = id(&made);
    let one = format!("{agents}/{scout}");

    let refused = team.cy.post(&agents, json!({"name": ""})).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    let listed = team.dee.get(&agents).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["agents"][0]["name"], "Scout");
    assert_eq!(team.dee.get(&one).await.body["id"], scout.as_str());

    // The workspace counts it apart from its people, and lists it among its members.
    let workspace = team.ada.get(&format!("/api/v1/workspaces/{ws}")).await;
    assert_eq!(workspace.body["member_count"], 4);
    assert_eq!(
        workspace.body["agents"],
        json!({"available": true, "enabled": true, "creation": "members", "count": 1})
    );
    let members = team
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/members"))
        .await;
    let member = members.body["members"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|member| member["user"]["id"] == scout.as_str())
        .expect("listed")
        .clone();
    assert_eq!(member["role"], "member");
    assert_eq!(member["suspended"], false);
    assert_eq!(member["user"]["kind"], "agent");
    assert_eq!(member["user"]["email"], Value::Null);
    assert_eq!(member["user"]["owner"]["display_name"], "cy");

    // Its owner renames it; an admin hands it over; then deletes it.
    let renamed = team
        .cy
        .send(
            Method::PATCH,
            &one,
            Some(json!({"name": "Ranger", "hue_override": null})),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["name"], "Ranger");
    assert_eq!(renamed.body["hue_override"], Value::Null);
    let not_theirs = team
        .ada
        .send(Method::PATCH, &one, Some(json!({"name": "Mine"})))
        .await;
    assert_eq!(not_theirs.status, StatusCode::FORBIDDEN);
    let dee_id = id(&team.dee.get("/api/v1/account/me").await);
    let by_member = team
        .dee
        .post(&format!("{one}/transfer"), json!({"owner_id": dee_id}))
        .await;
    assert_eq!(
        by_member.status,
        StatusCode::FORBIDDEN,
        "{}",
        by_member.body
    );
    let handed = team
        .bob
        .post(&format!("{one}/transfer"), json!({"owner_id": dee_id}))
        .await;
    assert_eq!(handed.status, StatusCode::OK, "{}", handed.body);
    assert_eq!(handed.body["owner"]["id"], dee_id.as_str());
    let deleted = team.dee.send(Method::DELETE, &one, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let gone = team.ada.get(&one).await;
    assert_eq!(gone.body["status"], "deleted");
    assert!(gone.body["deleted_at"].is_string());

    // Agents off in the workspace: none is made.
    let off = team
        .bob
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}"),
            Some(json!({"agents_enabled": false, "agent_creation": "admins"})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(off.body["agents"]["enabled"], false);
    assert_eq!(off.body["agents"]["creation"], "admins");
    let refused = team.ada.post(&agents, json!({"name": "Late"})).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
}

/// The names of the teamspaces an agent's teamspaces list, in its order, each with whether
/// the caller may take the agent out of it.
fn teamspaces_of(answer: &Answer) -> Vec<(String, bool)> {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    answer.body["teamspaces"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|teamspace| {
            (
                teamspace["name"].as_str().expect("a name").to_owned(),
                teamspace["removable"].as_bool().expect("a flag"),
            )
        })
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_s_teamspaces_show_as_far_as_each_caller_sees_them(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.cy, ws, "Scout").await;
    let teamspaces = format!("/api/v1/workspaces/{ws}/teamspaces");
    let of_scout = format!("/api/v1/workspaces/{ws}/agents/{scout}/teamspaces");
    let mut made = Vec::new();
    for (name, access) in [("Ops", "closed"), ("Vault", "private")] {
        let teamspace = team
            .ada
            .post(&teamspaces, json!({"name": name, "access": access}))
            .await;
        assert_eq!(teamspace.status, StatusCode::CREATED, "{}", teamspace.body);
        let added = team
            .ada
            .post(
                &format!("{teamspaces}/{}/members", id(&teamspace)),
                json!({"user_id": scout, "role": "member"}),
            )
            .await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
        made.push(id(&teamspace));
    }
    let named = |list: &[(&str, bool)]| -> Vec<(String, bool)> {
        list.iter()
            .map(|(name, removable)| ((*name).to_owned(), *removable))
            .collect()
    };

    // Ada, in every one, owns those she made; nobody leaves the default one.
    assert_eq!(
        teamspaces_of(&team.ada.get(&of_scout).await),
        named(&[("General", false), ("Ops", true), ("Vault", true)])
    );
    // Bob, an admin outside the private one, and Cy, its owner, a member, never see it.
    assert_eq!(
        teamspaces_of(&team.bob.get(&of_scout).await),
        named(&[("General", false), ("Ops", true)])
    );
    assert_eq!(
        teamspaces_of(&team.cy.get(&of_scout).await),
        named(&[("General", false), ("Ops", false)])
    );
    // Nor does Scout itself, while its owner is outside it.
    let worker = Bearer::new(&app, agent_token(&team.cy, ws, &scout, &["read"]).await);
    assert_eq!(
        teamspaces_of(&worker.get(&of_scout).await),
        named(&[("General", false), ("Ops", false)])
    );

    // Taken out of one, it lists the rest.
    let removed = team
        .bob
        .send(
            Method::DELETE,
            &format!("{teamspaces}/{}/members/{scout}", made[0]),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(
        teamspaces_of(&team.ada.get(&of_scout).await),
        named(&[("General", false), ("Vault", true)])
    );

    // An agent nobody has answers as one that does not exist.
    let unknown = team
        .ada
        .get(&format!(
            "/api/v1/workspaces/{ws}/agents/{}/teamspaces",
            uuid::Uuid::now_v7()
        ))
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_token_acts_as_the_agent_in_its_workspace_alone(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.cy, ws, "Scout").await;
    let tokens = format!("/api/v1/workspaces/{ws}/agents/{scout}/tokens");

    // Made from a browser session by its owner, read and write only, shown once.
    let admin_scope = team
        .cy
        .post(&tokens, json!({"name": "worker", "scopes": ["admin"]}))
        .await;
    assert_eq!(
        admin_scope.status,
        StatusCode::FORBIDDEN,
        "{}",
        admin_scope.body
    );
    let by_other = team
        .ada
        .post(&tokens, json!({"name": "worker", "scopes": ["read"]}))
        .await;
    assert_eq!(by_other.status, StatusCode::FORBIDDEN, "{}", by_other.body);
    let worker = Bearer::new(
        &app,
        agent_token(&team.cy, ws, &scout, &["read", "write"]).await,
    );
    let own = team.ada.get(&tokens).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    let token = &own.body["tokens"][0];
    assert_eq!(token["workspace_id"], ws.as_str());
    assert_eq!(token["scopes"], json!(["read", "write"]));
    assert!(token.get("token").is_none(), "no secret: {token}");
    let personal = Bearer::new(&app, team.cy.token(&["write"]).await);
    let by_token = personal
        .post(&tokens, json!({"name": "more", "scopes": ["read"]}))
        .await;
    assert_eq!(by_token.status, StatusCode::FORBIDDEN, "{}", by_token.body);

    // It lists its one workspace, and makes a page where it and Cy both are.
    let listed = worker.get("/api/v1/workspaces").await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["workspaces"].as_array().map(Vec::len), Some(1));
    let general = general(&team.ada, ws).await;
    let made = worker
        .post(
            &format!("/api/v1/workspaces/{ws}/pages"),
            json!({"teamspace_id": general, "title": "From Scout"}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["created_by"], scout.as_str());
    let seen = team.ada.get(&format!("/api/v1/pages/{}", id(&made))).await;
    assert_eq!(seen.body["last_edited_by"], scout.as_str());

    // Nothing of the account, the instance or another workspace.
    for uri in [
        "/api/v1/account/me",
        "/api/v1/account/tokens",
        "/api/v1/admin/users",
    ] {
        let refused = worker.get(uri).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{uri}: {}",
            refused.body
        );
        assert_eq!(refused.code(), "token_bound", "{uri}");
    }
    let refused = worker
        .post("/api/v1/workspaces", json!({"name": "Mine"}))
        .await;
    assert_eq!(refused.code(), "token_bound");
    let other = team
        .ada
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await;
    let refused = worker
        .get(&format!("/api/v1/workspaces/{}", id(&other)))
        .await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    // Nor does it manage: no agents, members or teamspaces.
    let refused = worker
        .post(
            &format!("/api/v1/workspaces/{ws}/agents"),
            json!({"name": "Child"}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let refused = worker
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Bots", "access": "open"}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);

    // Revoked by its owner, it is refused at once.
    let token_id = own.body["tokens"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let revoked = team
        .cy
        .send(Method::DELETE, &format!("{tokens}/{token_id}"), None)
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT, "{}", revoked.body);
    assert_eq!(
        worker.get("/api/v1/workspaces").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_s_token_is_refused_once_its_owner_leaves(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.dee, ws, "Scout").await;
    let worker = Bearer::new(&app, agent_token(&team.dee, ws, &scout, &["read"]).await);
    assert_eq!(
        worker.get("/api/v1/workspaces").await.status,
        StatusCode::OK
    );

    let left = team
        .dee
        .post(&format!("/api/v1/workspaces/{ws}/leave"), json!({}))
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    // Refused saying why and how it resumes, apart from a token unknown or revoked.
    let refused = worker.get("/api/v1/workspaces").await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "agent_suspended"),
        "{}",
        refused.body
    );
    assert!(
        refused.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("hand it to someone else")),
        "{}",
        refused.body
    );
    let listed = team
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/agents"))
        .await;
    assert_eq!(listed.body["agents"][0]["status"], "suspended");
    let members = team
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/members"))
        .await;
    assert!(
        members.body["members"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|member| member["user"]["id"] == scout.as_str() && member["suspended"] == true)
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_s_token_says_agents_are_off_while_they_are(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.cy, ws, "Scout").await;
    let worker = Bearer::new(&app, agent_token(&team.cy, ws, &scout, &["read"]).await);
    let switch = |on: bool| {
        let uri = format!("/api/v1/workspaces/{ws}");
        let ada = &team.ada;
        async move {
            let changed = ada
                .send(Method::PATCH, &uri, Some(json!({"agents_enabled": on})))
                .await;
            assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        }
    };

    switch(false).await;
    let refused = worker.get("/api/v1/whoami").await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "agents_off"),
        "{}",
        refused.body
    );
    switch(true).await;
    assert_eq!(worker.get("/api/v1/whoami").await.status, StatusCode::OK);

    // A token unknown is unauthenticated, whatever agents do.
    let unknown = Bearer::new(&app, "Bearer nw_unknown".to_owned());
    assert_eq!(
        unknown.get("/api/v1/whoami").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn person_properties_and_shares_name_agents_as_they_do_people(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.cy, ws, "Scout").await;
    let general = general(&team.ada, ws).await;
    let page = create(&team.ada, ws, json!({"teamspace_id": general}), "Brief").await;

    let shared = team
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": scout, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    assert_eq!(shared.body["user"]["kind"], "agent");
    assert_eq!(shared.body["user"]["owner"]["display_name"], "cy");
    let open = team
        .cy
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Ops", "access": "closed"}),
        )
        .await;
    let added = team
        .cy
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces/{}/members", id(&open)),
            json!({"user_id": scout, "role": "member"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    assert_eq!(added.body["user"]["kind"], "agent");
    let as_owner = team
        .cy
        .send(
            Method::PATCH,
            &format!(
                "/api/v1/workspaces/{ws}/teamspaces/{}/members/{scout}",
                id(&open)
            ),
            Some(json!({"role": "owner"})),
        )
        .await;
    assert_eq!(
        as_owner.status,
        StatusCode::BAD_REQUEST,
        "{}",
        as_owner.body
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_person_s_bound_token_works_in_its_workspace_alone(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let bound = |scopes: Value, workspace: &str| json!({"name": "bound", "scopes": scopes, "workspace_id": workspace});

    let made = team
        .dee
        .post("/api/v1/account/tokens", bound(json!(["admin"]), ws))
        .await;
    assert_eq!(made.status, StatusCode::FORBIDDEN, "{}", made.body);
    let made = team
        .ada
        .post("/api/v1/account/tokens", bound(json!(["write"]), ws))
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["api_token"]["workspace_id"], ws.as_str());
    let writer = Bearer::new(
        &app,
        format!("Bearer {}", made.body["token"].as_str().expect("a token")),
    );
    let listed = writer.get("/api/v1/workspaces").await;
    assert_eq!(listed.body["workspaces"].as_array().map(Vec::len), Some(1));
    let other = team
        .ada
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await;
    let elsewhere = writer
        .get(&format!("/api/v1/workspaces/{}", id(&other)))
        .await;
    assert_eq!(elsewhere.status, StatusCode::NOT_FOUND);
    assert_eq!(writer.get("/api/v1/account/me").await.code(), "token_bound");
    // Without the admin scope, the workspace's owner manages nothing through it.
    let rename = writer
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}"),
            Some(json!({"name": "Renamed"})),
        )
        .await;
    assert_eq!(rename.status, StatusCode::FORBIDDEN, "{}", rename.body);
    let made = team
        .ada
        .post(
            "/api/v1/account/tokens",
            bound(json!(["write", "admin"]), ws),
        )
        .await;
    let manager = Bearer::new(
        &app,
        format!("Bearer {}", made.body["token"].as_str().expect("a token")),
    );
    let rename = manager
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}"),
            Some(json!({"name": "Renamed"})),
        )
        .await;
    assert_eq!(rename.status, StatusCode::OK, "{}", rename.body);

    // An instance admin's bound admin token is no instance admin.
    let root = Browser::sign_in(&app, "admin@example.com").await;
    let created = root
        .post("/api/v1/workspaces", json!({"name": "Root's"}))
        .await;
    let made = root
        .post(
            "/api/v1/account/tokens",
            bound(json!(["admin"]), &id(&created)),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let bound_admin = Bearer::new(
        &app,
        format!("Bearer {}", made.body["token"].as_str().expect("a token")),
    );
    let refused = bound_admin.get("/api/v1/admin/users").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "token_bound");
    // The instance's people are people alone.
    let scout = agent(&team.cy, ws, "Scout").await;
    let listed = root.get("/api/v1/admin/users").await;
    let users = listed.body["users"].as_array().expect("a list");
    assert_eq!(users.len(), 5, "{}", listed.body);
    assert!(
        users
            .iter()
            .all(|user| user["email"].is_string() && user["id"] != scout.as_str())
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_bound_token_keeps_to_its_workspace_s_favorites(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let favorites = "/api/v1/account/favorites";
    let other = id(&team
        .ada
        .post("/api/v1/workspaces", json!({"name": "Elsewhere"}))
        .await);
    let far = general(&team.ada, &other).await;
    let [far_a, far_b] = [
        create(&team.ada, &other, json!({"teamspace_id": far}), "A").await,
        create(&team.ada, &other, json!({"teamspace_id": far}), "B").await,
    ];
    let near = general(&team.ada, ws).await;
    let [here, there] = [
        create(&team.ada, ws, json!({"teamspace_id": near}), "Here").await,
        create(&team.ada, ws, json!({"teamspace_id": near}), "There").await,
    ];
    for page in [&far_a, &far_b, &here] {
        let added = team.ada.post(favorites, json!({"page_id": page})).await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    }
    let made = team
        .ada
        .post(
            "/api/v1/account/tokens",
            json!({"name": "bound", "scopes": ["read", "write"], "workspace_id": ws}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let bound = Bearer::new(
        &app,
        format!("Bearer {}", made.body["token"].as_str().expect("a token")),
    );
    let ids = |answer: &Answer| -> Vec<String> {
        answer.body["pages"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|page| page["id"].as_str().expect("an id").to_owned())
            .collect()
    };

    // It lists its workspace's favorites alone.
    assert_eq!(
        ids(&bound.get(favorites).await),
        std::slice::from_ref(&here)
    );
    let elsewhere = bound
        .get(&format!("{favorites}?workspace_id={other}"))
        .await;
    assert_eq!(elsewhere.status, StatusCode::OK, "{}", elsewhere.body);
    assert_eq!(ids(&elsewhere), Vec::<String>::new());

    // The others are not there for it: to place beside, move or remove, they answer as a
    // page that is no favorite at all.
    let unknown = "00000000-0000-4000-8000-000000000001".to_owned();
    for beside in [&far_a, &unknown] {
        let placed = bound
            .post(
                favorites,
                json!({"page_id": there, "placement": {"after": beside}}),
            )
            .await;
        assert_eq!(placed.status, StatusCode::BAD_REQUEST, "{}", placed.body);
    }
    for page in [&far_b, &unknown] {
        let moved = bound
            .post(
                &format!("{favorites}/{page}/move"),
                json!({"placement": "first"}),
            )
            .await;
        assert_eq!(moved.status, StatusCode::NOT_FOUND, "{}", moved.body);
        let removed = bound
            .send(Method::DELETE, &format!("{favorites}/{page}"), None)
            .await;
        assert_eq!(removed.status, StatusCode::NOT_FOUND, "{}", removed.body);
    }
    let session = team
        .ada
        .get(&format!("{favorites}?workspace_id={other}"))
        .await;
    assert_eq!(ids(&session), [far_a.clone(), far_b.clone()]);

    // In its own workspace it adds, moves and removes them.
    let added = bound
        .post(
            favorites,
            json!({"page_id": there, "placement": {"before": here}}),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    assert_eq!(
        ids(&bound.get(favorites).await),
        [there.clone(), here.clone()]
    );
    let moved = bound
        .post(
            &format!("{favorites}/{there}/move"),
            json!({"placement": "last"}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::NO_CONTENT, "{}", moved.body);
    assert_eq!(
        ids(&bound.get(favorites).await),
        [here.clone(), there.clone()]
    );
    let removed = bound
        .send(Method::DELETE, &format!("{favorites}/{here}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    let session = team.ada.get(favorites).await;
    assert_eq!(ids(&session), [far_a, far_b, there]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_is_suspended_while_its_owner_is_a_guest(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let team = team(&state, &app).await;
    let ws = &team.workspace;
    let scout = agent(&team.cy, ws, "Scout").await;
    let worker = Bearer::new(
        &app,
        agent_token(&team.cy, ws, &scout, &["read", "write"]).await,
    );
    let one = format!("/api/v1/workspaces/{ws}/agents/{scout}");
    let cy_id = id(&team.cy.get("/api/v1/account/me").await);
    let role = |role: &'static str| {
        let uri = format!("/api/v1/workspaces/{ws}/members/{cy_id}");
        let ada = &team.ada;
        async move {
            let changed = ada
                .send(Method::PATCH, &uri, Some(json!({"role": role})))
                .await;
            assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        }
    };
    assert_eq!(
        worker.get("/api/v1/workspaces").await.status,
        StatusCode::OK
    );

    // A guest owns no agent: Cy's is suspended, and the admins see it so.
    role("guest").await;
    let refused = worker.get("/api/v1/workspaces").await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "agent_suspended"),
        "{}",
        refused.body
    );
    assert_eq!(team.ada.get(&one).await.body["status"], "suspended");

    // A member again, it works again; an admin may also hand it to someone else.
    role("member").await;
    assert_eq!(
        worker.get("/api/v1/workspaces").await.status,
        StatusCode::OK
    );
    assert_eq!(team.ada.get(&one).await.body["status"], "active");
    role("guest").await;
    let bob_id = id(&team.bob.get("/api/v1/account/me").await);
    let handed = team
        .bob
        .post(&format!("{one}/transfer"), json!({"owner_id": bob_id}))
        .await;
    assert_eq!(handed.status, StatusCode::OK, "{}", handed.body);
    assert_eq!(handed.body["status"], "active");
}
