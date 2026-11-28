//! The OAuth use cases beside the bearer authentication: the principal an access token
//! stands for, read again as live connections do, the access changes a revoked grant
//! tells, and using a grant while it ends, which neither side fails over.

use crate::support;

use std::sync::Arc;

use neon_app::auth::AuthError;
use neon_app::crypto::pkce_challenge;
use neon_app::oauth::{Approval, OAuthError, OAuthService, TokenRequest};
use neon_app::pages::AccessChange;
use neon_app::ratelimit::Bucket;
use neon_domain::auth::{Credential, Principal, Scope};
use neon_domain::oauth::{AuthorizationRequest, ClientMetadata, Endpoints, Identity};
use sqlx::PgPool;
use support::{Harness, IP};
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "http://127.0.0.1:43110/callback";

fn endpoints() -> Endpoints {
    Endpoints::new("https://neon.example.com")
}

/// The OAuth service of a harness, on its pool, settings, clock, limiter and listener.
fn oauth(h: &Harness) -> OAuthService {
    OAuthService::builder(h.pool.clone(), Arc::clone(&h.settings))
        .clock(h.time.clock())
        .limiter(h.limiter.clone())
        .access_changes(h.changes.clone())
        .build()
}

/// A PKCE verifier, low in entropy: tests never hold secrets that look real.
fn verifier() -> String {
    "q".repeat(50)
}

/// A client registered, and a request of it, with the challenge of [`verifier`].
async fn client(oauth: &OAuthService) -> (Uuid, AuthorizationRequest) {
    let client = oauth
        .register(
            &ClientMetadata {
                client_name: Some("Example CLI".into()),
                redirect_uris: Some(vec![REDIRECT.into()]),
                ..ClientMetadata::default()
            },
            IP,
        )
        .await
        .expect("registers");
    (client.id, request_of(client.id))
}

/// An authorization request of a client, with the challenge of [`verifier`].
fn request_of(client: Uuid) -> AuthorizationRequest {
    AuthorizationRequest {
        response_type: Some("code".into()),
        client_id: Some(client.to_string()),
        redirect_uri: Some(REDIRECT.into()),
        code_challenge: Some(pkce_challenge(&verifier())),
        code_challenge_method: Some("S256".into()),
        scope: None,
        state: Some("opaque".into()),
        resource: None,
    }
}

/// The code an approval sends the browser back with.
async fn approve(
    oauth: &OAuthService,
    person: &Principal,
    request: &AuthorizationRequest,
    workspace_id: Uuid,
    identity: Identity,
) -> String {
    let back = oauth
        .approve(
            person,
            request,
            &Approval {
                workspace_id,
                identity,
                scopes: vec!["read".into(), "write".into()],
            },
            &endpoints(),
        )
        .await
        .expect("approved");
    let url = Url::parse(&back).expect("an address");
    url.query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, code)| code.into_owned())
        .expect("a code")
}

fn exchange(client: Uuid, code: &str) -> TokenRequest {
    TokenRequest {
        grant_type: Some("authorization_code".into()),
        client_id: Some(client.to_string()),
        code: Some(code.to_owned()),
        redirect_uri: Some(REDIRECT.into()),
        code_verifier: Some(verifier()),
        ..TokenRequest::default()
    }
}

fn refresh(client: Uuid, token: &str) -> TokenRequest {
    TokenRequest {
        grant_type: Some("refresh_token".into()),
        client_id: Some(client.to_string()),
        refresh_token: Some(token.to_owned()),
        ..TokenRequest::default()
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_access_token_stands_for_the_agent_in_its_workspace_until_its_grant_ends(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let scout = h.agent(&ada, design, "Scout").await;
    let (client, request) = client(&oauth).await;
    let code = approve(&oauth, &ada, &request, design, Identity::Agent(scout.id)).await;
    let tokens = oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect("issued");

    let principal = h
        .auth
        .authenticate_token(tokens.access_token.expose())
        .await
        .expect("works");
    assert_eq!(principal.user.id, scout.id);
    assert_eq!(principal.user.owner_id, Some(ada.user.id));
    assert_eq!(principal.workspace(), Some(design));
    assert!(!principal.via_session() && !principal.is_admin());
    let Credential::Token { name, scopes, .. } = &principal.credential else {
        panic!("a token credential: {principal:?}");
    };
    assert_eq!(name, "Example CLI");
    assert_eq!(scopes, &[Scope::Read, Scope::Write]);
    // Read again, as a live connection does.
    assert_eq!(h.auth.refresh(&principal).await.expect("holds"), principal);

    // Its client revokes it: the token stops, and its open connections hear of it.
    h.changes.take();
    oauth
        .revoke(
            Some(tokens.refresh_token.expose()),
            Some(&client.to_string()),
        )
        .await
        .expect("revoked");
    assert_eq!(
        h.changes.take(),
        [AccessChange::Account { user_id: scout.id }]
    );
    assert!(matches!(
        h.auth.refresh(&principal).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        h.auth
            .authenticate_token(tokens.access_token.expose())
            .await,
        Err(AuthError::Unauthenticated)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_reused_refresh_token_revokes_the_grant_and_tells_its_connections(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
    let first = oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect("issued");
    let second = oauth
        .token(&refresh(client, first.refresh_token.expose()), &endpoints())
        .await
        .expect("rotated");
    let principal = h
        .auth
        .authenticate_token(second.access_token.expose())
        .await
        .expect("works");
    assert_eq!(principal.user.id, ada.user.id);

    h.changes.take();
    let reused = oauth
        .token(&refresh(client, first.refresh_token.expose()), &endpoints())
        .await;
    assert!(
        matches!(&reused, Err(OAuthError::Protocol(problem)) if problem.code.as_str() == "invalid_grant"),
        "{reused:?}"
    );
    assert_eq!(
        h.changes.take(),
        [AccessChange::Account {
            user_id: ada.user.id
        }]
    );
    assert!(h.auth.refresh(&principal).await.is_err());
}

/// How many times each race runs: often enough that both orders, and the locks of each
/// side meeting, all come up.
const RACES: usize = 20;

/// Whether a use case failed on the server's side, as a deadlock does.
fn failed<T>(result: &Result<T, OAuthError>) -> bool {
    matches!(
        result,
        Err(OAuthError::Database(_) | OAuthError::Internal(_))
    )
}

fn invalid_grant<T: std::fmt::Debug>(result: &Result<T, OAuthError>) -> bool {
    matches!(result, Err(OAuthError::Protocol(problem)) if problem.code.as_str() == "invalid_grant")
}

async fn grants(h: &Harness) -> i64 {
    h.scalar("SELECT count(*) FROM oauth_grants").await
}

/// The id of the person's one grant.
async fn grant_id(oauth: &OAuthService, person: &Principal) -> Uuid {
    let apps = oauth.connected_apps(person).await.expect("listed");
    assert_eq!(apps.len(), 1, "one grant");
    apps[0].grant.id
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_refresh_racing_a_replay_of_its_predecessor_ends_the_grant_without_failing(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let endpoints = endpoints();

    for _ in 0..RACES {
        let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
        let first = oauth
            .token(&exchange(client, &code), &endpoints)
            .await
            .expect("issued");
        let second = oauth
            .token(&refresh(client, first.refresh_token.expose()), &endpoints)
            .await
            .expect("rotated");
        let (legit, replay) = (
            refresh(client, second.refresh_token.expose()),
            refresh(client, first.refresh_token.expose()),
        );
        let (legit, replay) = tokio::join!(
            oauth.token(&legit, &endpoints),
            oauth.token(&replay, &endpoints),
        );

        assert!(!failed(&legit), "{legit:?}");
        assert!(invalid_grant(&replay), "{replay:?}");
        assert!(legit.is_ok() || invalid_grant(&legit), "{legit:?}");
        assert_eq!(grants(&h).await, 0, "the replay ended the grant");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_refresh_racing_a_revocation_ends_the_grant_without_failing(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let client_id = client.to_string();
    let endpoints = endpoints();

    for round in 0..RACES {
        let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
        let tokens = oauth
            .token(&exchange(client, &code), &endpoints)
            .await
            .expect("issued");
        let refreshed = refresh(client, tokens.refresh_token.expose());
        let refreshing = oauth.token(&refreshed, &endpoints);
        // By its client, with the access token, and by the person, by turns.
        let (refreshed, revoked) = if round % 2 == 0 {
            tokio::join!(
                refreshing,
                oauth.revoke(Some(tokens.access_token.expose()), Some(&client_id)),
            )
        } else {
            let grant = grant_id(&oauth, &ada).await;
            tokio::join!(refreshing, oauth.disconnect_app(&ada, grant))
        };

        assert!(!failed(&refreshed), "{refreshed:?}");
        assert!(
            refreshed.is_ok() || invalid_grant(&refreshed),
            "{refreshed:?}"
        );
        assert!(revoked.is_ok(), "{revoked:?}");
        assert_eq!(grants(&h).await, 0, "the revocation ended the grant");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_exchange_racing_a_disconnection_or_a_second_exchange_ends_the_grant_without_failing(
    pool: PgPool,
) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let endpoints = endpoints();

    for _ in 0..RACES {
        let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
        let grant = grant_id(&oauth, &ada).await;
        let exchanged = exchange(client, &code);
        let (exchanged, disconnected) = tokio::join!(
            oauth.token(&exchanged, &endpoints),
            oauth.disconnect_app(&ada, grant),
        );
        assert!(!failed(&exchanged), "{exchanged:?}");
        assert!(
            exchanged.is_ok() || invalid_grant(&exchanged),
            "{exchanged:?}"
        );
        assert!(disconnected.is_ok(), "{disconnected:?}");
        assert_eq!(grants(&h).await, 0, "the disconnection ended the grant");

        let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
        let twice = exchange(client, &code);
        let (one, other) = tokio::join!(
            oauth.token(&twice, &endpoints),
            oauth.token(&twice, &endpoints),
        );
        assert!(!failed(&one) && !failed(&other), "{one:?} {other:?}");
        assert!(
            one.is_ok() && invalid_grant(&other) || invalid_grant(&one) && other.is_ok(),
            "one exchange works: {one:?} {other:?}"
        );
        assert_eq!(grants(&h).await, 0, "the second exchange ended the grant");
    }
}

fn registration(name: &str) -> ClientMetadata {
    ClientMetadata {
        client_name: Some(name.into()),
        redirect_uris: Some(vec![REDIRECT.into()]),
        ..ClientMetadata::default()
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn one_network_cannot_take_the_room_every_other_has_to_register_in(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("oauth.max_unused_clients", serde_json::json!(50))
        .await;
    h.set("oauth.max_unused_clients_per_source", serde_json::json!(5))
        .await;
    let oauth = oauth(&h);

    // Addresses of one IPv6 /64, a new one each time, register five clients and no more.
    let mut outcomes = Vec::new();
    for host in 1..=60 {
        let ip = format!("2001:db8:1:2::{host:x}");
        outcomes.push(oauth.register(&registration("Spam"), &ip).await);
    }
    assert!(outcomes[..5].iter().all(Result::is_ok), "{outcomes:?}");
    assert!(
        outcomes[5..]
            .iter()
            .all(|refused| matches!(refused, Err(OAuthError::SourceLimit))),
        "{outcomes:?}"
    );
    // Its rate is counted by the /64 too.
    assert!(
        h.limiter
            .seen()
            .iter()
            .all(|(bucket, key)| *bucket == Bucket::OAuthRegister && key == "2001:db8:1:2::/64")
    );

    // Elsewhere, registering works.
    oauth
        .register(&registration("Mine"), "198.51.100.7")
        .await
        .expect("another address registers");
    oauth
        .register(&registration("Mine"), "2001:db8:1:3::1")
        .await
        .expect("another /64 registers");
    // Once a person connects one of the network's clients, it waits no more, and the
    // network registers one more.
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let spam = outcomes[0].as_ref().expect("registered").id;
    approve(&oauth, &ada, &request_of(spam), design, Identity::Person).await;
    oauth
        .register(&registration("Spam"), "2001:db8:1:2::ffff")
        .await
        .expect("room again");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn exchanges_are_rate_limited_by_grant_but_a_second_use_never_is(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
    let first = oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect("issued");
    let second = oauth
        .token(&refresh(client, first.refresh_token.expose()), &endpoints())
        .await
        .expect("rotated");
    let grant = grant_id(&oauth, &ada).await;
    assert!(
        h.limiter
            .seen()
            .contains(&(Bucket::OAuthToken, grant.to_string()))
    );

    h.limiter.refuse(Bucket::OAuthToken);
    let limited = oauth
        .token(
            &refresh(client, second.refresh_token.expose()),
            &endpoints(),
        )
        .await;
    assert!(
        matches!(limited, Err(OAuthError::RateLimited { .. })),
        "{limited:?}"
    );
    let unspent: i64 = h
        .scalar("SELECT count(*) FROM oauth_tokens WHERE kind = 'refresh' AND used_at IS NULL")
        .await;
    assert_eq!(unspent, 1, "a refresh refused leaves its token unspent");
    // A spent refresh token used again still ends the grant at once.
    let reused = oauth
        .token(&refresh(client, first.refresh_token.expose()), &endpoints())
        .await;
    assert!(invalid_grant(&reused), "{reused:?}");
    assert_eq!(grants(&h).await, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn consenting_again_tells_the_connections_of_whom_the_grant_acts_as(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let scout = h.agent(&ada, design, "Scout").await;
    let (_, request) = client(&oauth).await;

    h.changes.take();
    approve(&oauth, &ada, &request, design, Identity::Agent(scout.id)).await;
    assert_eq!(h.changes.take(), [], "a new grant has no connections yet");
    approve(&oauth, &ada, &request, design, Identity::Agent(scout.id)).await;
    assert_eq!(
        h.changes.take(),
        [AccessChange::Account { user_id: scout.id }]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_password_reset_revokes_the_accounts_tokens_and_disconnects_its_apps(pool: PgPool) {
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let scout = h.agent(&ada, design, "Scout").await;
    let (token, _) = h
        .auth
        .create_api_token(&ada, "laptop", None, &["read", "write"], None)
        .await
        .expect("made");
    let (agent_token, _) = h.agent_token(&ada, &scout, &["read"]).await;
    let (client, request) = client(&oauth).await;
    let mut apps = Vec::new();
    for identity in [Identity::Person, Identity::Agent(scout.id)] {
        let code = approve(&oauth, &ada, &request, design, identity).await;
        let tokens = oauth
            .token(&exchange(client, &code), &endpoints())
            .await
            .expect("issued");
        apps.push(tokens.access_token);
    }

    h.auth
        .request_password_reset("ada@example.com", IP)
        .await
        .expect("answers");
    let link = h.token_of("password_reset").await;
    h.changes.take();
    h.auth
        .reset_password(&link, &support::secret("a brand new passphrase"), IP)
        .await
        .expect("reset");

    // Whatever was made with the old password stops working.
    for gone in std::iter::once(&token).chain(&apps) {
        assert!(matches!(
            h.auth.authenticate_token(gone.expose()).await,
            Err(AuthError::Unauthenticated)
        ));
    }
    assert_eq!(grants(&h).await, 0);
    // The connections of the account and of the agent an app acted as look again.
    let mut told = h.changes.take();
    told.sort_by_key(|change| format!("{change:?}"));
    let mut want = vec![
        AccessChange::Account {
            user_id: ada.user.id,
        },
        AccessChange::Account { user_id: scout.id },
    ];
    want.sort_by_key(|change| format!("{change:?}"));
    assert_eq!(told, want);
    // The agent's own token works for its workspace still.
    h.auth
        .authenticate_token(agent_token.expose())
        .await
        .expect("an agent's token stays");
}
