//! The OAuth use cases beside the bearer authentication: the principal an access token
//! stands for, read again as live connections do, and the access changes a revoked grant
//! tells.

mod support;

use std::sync::Arc;

use neon_app::auth::AuthError;
use neon_app::crypto::pkce_challenge;
use neon_app::oauth::{Approval, OAuthError, OAuthService, TokenRequest};
use neon_app::pages::AccessChange;
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
                client_name: Some("Claude Code".into()),
                redirect_uris: Some(vec![REDIRECT.into()]),
                ..ClientMetadata::default()
            },
            IP,
        )
        .await
        .expect("registers");
    let request = AuthorizationRequest {
        response_type: Some("code".into()),
        client_id: Some(client.id.to_string()),
        redirect_uri: Some(REDIRECT.into()),
        code_challenge: Some(pkce_challenge(&verifier())),
        code_challenge_method: Some("S256".into()),
        scope: None,
        state: Some("opaque".into()),
        resource: None,
    };
    (client.id, request)
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
    assert_eq!(name, "Claude Code");
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
