//! Pages' documents in the database: the log in order with its authors, snapshots taking
//! the log in, what a snapshot records on its page, and writes that find the page gone.

use std::time::Duration;

use neon_domain::auth::{Email, Role, User, UserKind};
use neon_domain::pages::{Page, PageKind, PageStyle, Space};
use neon_domain::workspaces::{Teamspace, TeamspaceAccess, Workspace, WorkspaceRole};
use neon_store::documents::{self, Edited, NewSnapshot, NewUpdate};
use neon_store::{pages, teamspaces, users, workspaces};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a valid time")
}

async fn user(pool: &PgPool, email: &str) -> Uuid {
    let address = Email::parse(email).expect("valid");
    let user = User {
        id: Uuid::now_v7(),
        kind: UserKind::Human,
        email: Some(address.clone()),
        display_name: "Someone".into(),
        role: Role::User,
        hue_override: None,
        email_verified_at: Some(now()),
        disabled_at: None,
        created_at: now(),
        updated_at: now(),
        workspace_id: None,
        owner_id: None,
    };
    users::insert(pool, &user, &address, "hash")
        .await
        .expect("saved");
    user.id
}

/// Ada's page in a teamspace of her workspace, and Ada.
async fn page(pool: &PgPool) -> (Uuid, Uuid) {
    let ada = user(pool, "ada@example.com").await;
    let workspace = Workspace {
        id: Uuid::now_v7(),
        name: "Design".into(),
        slug: "design".into(),
        icon: None,
        hue_override: None,
        members_can_create_teamspaces: true,
        agents_enabled: true,
        agent_creation: neon_domain::agents::AgentCreation::Members,
        created_by: None,
        created_at: now(),
        updated_at: now(),
    };
    assert!(workspaces::insert(pool, &workspace).await.expect("saved"));
    workspaces::add_member(pool, workspace.id, ada, WorkspaceRole::Owner, now())
        .await
        .expect("added");
    let teamspace = Teamspace {
        id: Uuid::now_v7(),
        workspace_id: workspace.id,
        name: "Team".into(),
        icon: None,
        hue_override: None,
        access: TeamspaceAccess::Default,
        created_by: None,
        created_at: now(),
        updated_at: now(),
    };
    teamspaces::insert(pool, &teamspace).await.expect("saved");
    let page = Page {
        id: Uuid::now_v7(),
        workspace_id: workspace.id,
        kind: PageKind::Page,
        space: Space::Teamspace(teamspace.id),
        parent_id: None,
        position: "a0".into(),
        title: String::new(),
        icon: None,
        cover: None,
        cover_image: None,
        style: PageStyle::default(),
        created_by: Some(ada),
        last_edited_by: Some(ada),
        created_at: now(),
        updated_at: now(),
        content_updated_at: now(),
        trashed_at: None,
        trashed_by: None,
        trashed_with: None,
    };
    pages::insert(pool, &page).await.expect("saved");
    (page.id, ada)
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_log_keeps_its_order_and_a_snapshot_takes_it_in(pool: PgPool) {
    let (page, ada) = page(&pool).await;
    let at = now();
    let bodies: [&[u8]; 3] = [b"one", b"two", b"three"];
    let batch: Vec<NewUpdate<'_>> = bodies
        .iter()
        .map(|body| NewUpdate {
            update: body,
            author_id: ada,
            created_at: at,
        })
        .collect();

    assert_eq!(
        documents::append(&pool, page, &[]).await.expect("ran"),
        None
    );
    let last = documents::append(&pool, page, &batch)
        .await
        .expect("logged")
        .expect("a number");
    let logged = documents::updates_after(&pool, page, 0)
        .await
        .expect("read");
    let read: Vec<&[u8]> = logged.iter().map(|row| row.update.as_slice()).collect();
    assert_eq!(read, bodies);
    assert_eq!(logged.last().map(|row| row.seq), Some(last));
    assert!(logged.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    assert!(logged.iter().all(|row| row.author_id == Some(ada)));
    assert_eq!(documents::snapshot(&pool, page).await.expect("read"), None);

    let first = logged[0].seq;
    for (state, through) in [(b"state one".as_slice(), first), (b"state all", last)] {
        documents::save_snapshot(
            &pool,
            page,
            NewSnapshot {
                state,
                state_vector: b"vector",
                last_seq: through,
                at,
            },
        )
        .await
        .expect("saved");
        documents::delete_updates_through(&pool, page, through)
            .await
            .expect("deleted");
    }
    let snapshot = documents::snapshot(&pool, page)
        .await
        .expect("read")
        .expect("a snapshot");
    assert_eq!(snapshot.state, b"state all");
    assert_eq!(snapshot.last_seq, last);
    assert!(
        documents::updates_after(&pool, page, 0)
            .await
            .expect("read")
            .is_empty()
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_snapshot_records_the_search_text_and_the_latest_change(pool: PgPool) {
    let (page, ada) = page(&pool).await;
    let dee = user(&pool, "dee@example.com").await;
    let read = || async {
        sqlx::query_as::<_, (Option<String>, OffsetDateTime, Option<Uuid>)>(
            "SELECT search_text, content_updated_at, last_edited_by FROM pages WHERE id = $1",
        )
        .bind(page)
        .fetch_one(&pool)
        .await
        .expect("read")
    };
    let later = now() + Duration::from_secs(60);

    let edited = Edited {
        at: later,
        by: Some(dee),
    };
    assert!(
        documents::set_content(&pool, page, Some("Hello"), Some(edited))
            .await
            .expect("ran")
    );
    assert_eq!(read().await, (Some("Hello".to_owned()), later, Some(dee)));

    // An older change moves nothing back, and no text keeps the text.
    let older = Edited {
        at: now(),
        by: Some(ada),
    };
    documents::set_content(&pool, page, None, Some(older))
        .await
        .expect("ran");
    assert_eq!(read().await, (Some("Hello".to_owned()), later, Some(dee)));

    // A change of the page itself after the document's keeps its editor.
    sqlx::query("UPDATE pages SET updated_at = $2, last_edited_by = $3 WHERE id = $1")
        .bind(page)
        .bind(later + Duration::from_secs(60))
        .bind(ada)
        .execute(&pool)
        .await
        .expect("updated");
    let between = Edited {
        at: later + Duration::from_secs(30),
        by: Some(dee),
    };
    documents::set_content(&pool, page, Some(""), Some(between))
        .await
        .expect("ran");
    assert_eq!(read().await, (Some(String::new()), between.at, Some(ada)));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn writes_for_a_deleted_page_say_it_is_gone(pool: PgPool) {
    let (page, ada) = page(&pool).await;
    pages::delete(&pool, page).await.expect("deleted");

    let logged = documents::append(
        &pool,
        page,
        &[NewUpdate {
            update: b"late",
            author_id: ada,
            created_at: now(),
        }],
    )
    .await
    .expect_err("refused");
    assert!(documents::is_page_gone(&logged), "{logged}");
    let saved = documents::save_snapshot(
        &pool,
        page,
        NewSnapshot {
            state: b"state",
            state_vector: b"vector",
            last_seq: 0,
            at: now(),
        },
    )
    .await
    .expect_err("refused");
    assert!(documents::is_page_gone(&saved), "{saved}");
    assert!(
        !documents::set_content(&pool, page, Some("text"), None)
            .await
            .expect("ran")
    );
    assert!(!documents::is_page_gone(&sqlx::Error::PoolTimedOut));
}
