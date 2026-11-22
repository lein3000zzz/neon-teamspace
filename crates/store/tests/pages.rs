//! What the schema itself holds pages to: a parent in the same workspace and space,
//! positions sorted byte by byte, documents, shares and favorites that go with their page,
//! and everything gone with its teamspace or workspace.

use neon_domain::auth::{Email, Role, User};
use neon_domain::pages::{Page, PageStyle, Space};
use neon_domain::workspaces::{Teamspace, TeamspaceAccess, Workspace, WorkspaceRole};
use neon_store::pages::{self, Siblings};
use neon_store::{favorites, teamspaces, users, workspaces};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a valid time")
}

async fn user(pool: &PgPool, email: &str) -> Uuid {
    let user = User {
        id: Uuid::now_v7(),
        email: Email::parse(email).expect("valid"),
        display_name: "Someone".into(),
        role: Role::User,
        hue_override: None,
        email_verified_at: Some(now()),
        disabled_at: None,
        created_at: now(),
        updated_at: now(),
    };
    users::insert(pool, &user, "hash").await.expect("saved");
    user.id
}

/// A workspace with Ada in it, and two teamspaces of it.
struct Fixture {
    workspace: Uuid,
    ada: Uuid,
    one: Uuid,
    two: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let ada = user(pool, "ada@example.com").await;
    let workspace = Workspace {
        id: Uuid::now_v7(),
        name: "Design".into(),
        slug: "design".into(),
        icon: None,
        hue_override: None,
        members_can_create_teamspaces: true,
        created_by: None,
        created_at: now(),
        updated_at: now(),
    };
    assert!(workspaces::insert(pool, &workspace).await.expect("saved"));
    workspaces::add_member(pool, workspace.id, ada, WorkspaceRole::Owner, now())
        .await
        .expect("added");
    let mut ids = Vec::new();
    for access in [TeamspaceAccess::Default, TeamspaceAccess::Open] {
        let teamspace = Teamspace {
            id: Uuid::now_v7(),
            workspace_id: workspace.id,
            name: "Team".into(),
            icon: None,
            hue_override: None,
            access,
            created_by: None,
            created_at: now(),
            updated_at: now(),
        };
        teamspaces::insert(pool, &teamspace).await.expect("saved");
        ids.push(teamspace.id);
    }
    Fixture {
        workspace: workspace.id,
        ada,
        one: ids[0],
        two: ids[1],
    }
}

fn page(workspace_id: Uuid, space: Space, parent_id: Option<Uuid>, position: &str) -> Page {
    Page {
        id: Uuid::now_v7(),
        workspace_id,
        space,
        parent_id,
        position: position.into(),
        title: String::new(),
        icon: None,
        cover: None,
        style: PageStyle::default(),
        created_by: None,
        last_edited_by: None,
        created_at: now(),
        updated_at: now(),
        content_updated_at: now(),
        trashed_at: None,
        trashed_by: None,
        trashed_with: None,
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_parent_is_in_the_same_space(pool: PgPool) {
    let f = fixture(&pool).await;
    let top = page(f.workspace, Space::Teamspace(f.one), None, "a0");
    pages::insert(&pool, &top).await.expect("saved");
    let mine = page(f.workspace, Space::Private { owner: f.ada }, None, "a0");
    pages::insert(&pool, &mine).await.expect("saved");

    for stray in [
        page(f.workspace, Space::Teamspace(f.two), Some(top.id), "a0"),
        page(
            f.workspace,
            Space::Private { owner: f.ada },
            Some(top.id),
            "a1",
        ),
        page(f.workspace, Space::Teamspace(f.one), Some(mine.id), "a1"),
    ] {
        let refused = pages::insert(&pool, &stray).await.expect_err("refused");
        assert!(
            refused
                .as_database_error()
                .is_some_and(|db| db.is_foreign_key_violation()),
            "{refused}"
        );
    }
    let under = page(f.workspace, Space::Teamspace(f.one), Some(top.id), "a0");
    pages::insert(&pool, &under).await.expect("saved");
    assert_eq!(
        pages::get(&pool, under.id).await.expect("read"),
        Some(under)
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn positions_sort_byte_by_byte(pool: PgPool) {
    let f = fixture(&pool).await;
    // In most locales' collations a sorts before Z; byte by byte, Z comes first.
    for position in ["a0V", "Zz", "a0", "b00", "a1"] {
        let page = page(f.workspace, Space::Teamspace(f.one), None, position);
        pages::insert(&pool, &page).await.expect("saved");
    }

    let siblings = Siblings::of(f.workspace, None, Space::Teamspace(f.one));
    let listed: Vec<String> = pages::sibling_positions(&pool, siblings, None)
        .await
        .expect("listed")
        .into_iter()
        .map(|(_, position)| position)
        .collect();
    assert_eq!(listed, ["Zz", "a0", "a0V", "a1", "b00"]);
    let malformed = page(f.workspace, Space::Teamspace(f.one), None, "a 0");
    assert!(pages::insert(&pool, &malformed).await.is_err());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn what_belongs_to_a_page_goes_with_it_and_pages_go_with_their_teamspace(pool: PgPool) {
    let f = fixture(&pool).await;
    let top = page(f.workspace, Space::Teamspace(f.two), None, "a0");
    let child = page(f.workspace, Space::Teamspace(f.two), Some(top.id), "a0");
    for page in [&top, &child] {
        pages::insert(&pool, page).await.expect("saved");
    }
    for _ in 0..2 {
        sqlx::query(
            "INSERT INTO page_updates (page_id, update, author_id, created_at)
             VALUES ($1, '\\x01', $2, $3)",
        )
        .bind(child.id)
        .bind(f.ada)
        .bind(now())
        .execute(&pool)
        .await
        .expect("logged");
    }
    sqlx::query(
        "INSERT INTO page_snapshots (page_id, state, state_vector, last_seq, updated_at)
         VALUES ($1, '\\x00', '\\x00', 2, $2)",
    )
    .bind(child.id)
    .bind(now())
    .execute(&pool)
    .await
    .expect("saved");
    favorites::insert(&pool, f.ada, child.id, f.workspace, "a0", now())
        .await
        .expect("added");
    let seqs: Vec<i64> = sqlx::query_scalar("SELECT seq FROM page_updates ORDER BY seq")
        .fetch_all(&pool)
        .await
        .expect("read");
    assert!(seqs[0] < seqs[1], "{seqs:?}");

    assert!(
        teamspaces::delete(&pool, f.workspace, f.two)
            .await
            .expect("deleted")
    );
    for count in [
        "SELECT count(*) FROM pages",
        "SELECT count(*) FROM page_updates",
        "SELECT count(*) FROM page_snapshots",
        "SELECT count(*) FROM favorites",
    ] {
        let left: i64 = sqlx::query_scalar(count)
            .fetch_one(&pool)
            .await
            .expect("counted");
        assert_eq!(left, 0, "{count}");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn pages_go_with_their_workspace_and_private_ones_with_their_owner(pool: PgPool) {
    let f = fixture(&pool).await;
    let mine = page(f.workspace, Space::Private { owner: f.ada }, None, "a0");
    pages::insert(&pool, &mine).await.expect("saved");
    assert!(
        workspaces::remove_member(&pool, f.workspace, f.ada)
            .await
            .expect("removed")
    );
    assert!(
        pages::get(&pool, mine.id).await.expect("read").is_some(),
        "private pages stay while their owner is out of the workspace"
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(f.ada)
        .execute(&pool)
        .await
        .expect("deleted");
    assert_eq!(pages::get(&pool, mine.id).await.expect("read"), None);

    let team = page(f.workspace, Space::Teamspace(f.one), None, "a0");
    pages::insert(&pool, &team).await.expect("saved");
    assert!(
        workspaces::delete(&pool, f.workspace)
            .await
            .expect("deleted")
    );
    assert_eq!(pages::get(&pool, team.id).await.expect("read"), None);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn outlines_follow_the_tree_and_the_trash(pool: PgPool) {
    let f = fixture(&pool).await;
    let at = |parent: Option<&Page>, position: &str, title: &str| {
        let mut made = page(
            f.workspace,
            Space::Teamspace(f.one),
            parent.map(|p| p.id),
            position,
        );
        made.title = title.into();
        made
    };
    // Positions that sort one way byte by byte and another in a natural collation.
    let second = at(None, "aZ", "second");
    let first = at(None, "Zz", "first");
    let under = at(Some(&second), "a0", "under");
    let deeper = at(Some(&under), "a0", "deeper");
    let late = at(Some(&second), "a1", "late");
    for made in [&second, &first, &under, &deeper, &late] {
        pages::insert(&pool, made).await.expect("saved");
    }
    let titles = |nodes: Vec<pages::OutlineNode>| {
        nodes
            .into_iter()
            .map(|node| node.title)
            .collect::<Vec<String>>()
    };

    let space = pages::space_outline(&pool, f.workspace, f.one, 100)
        .await
        .expect("listed");
    assert_eq!(space[2].parent_id, Some(second.id));
    assert_eq!(
        titles(space),
        ["first", "second", "under", "deeper", "late"]
    );
    let limited = pages::space_outline(&pool, f.workspace, f.one, 2)
        .await
        .expect("listed");
    assert_eq!(titles(limited), ["first", "second"]);
    assert_eq!(
        titles(pages::outline(&pool, second.id, 100).await.expect("listed")),
        ["second", "under", "deeper", "late"]
    );

    // A page trashed on its own leaves the outline of what is out of the trash; the
    // outline of a page in the trash holds what went there with it.
    pages::trash(&pool, late.id, f.ada, now())
        .await
        .expect("trashed");
    pages::trash(&pool, second.id, f.ada, now())
        .await
        .expect("trashed");
    assert_eq!(
        titles(pages::outline(&pool, second.id, 100).await.expect("listed")),
        ["second", "under", "deeper"]
    );
    assert_eq!(
        titles(pages::outline(&pool, late.id, 100).await.expect("listed")),
        ["late"]
    );
    assert_eq!(
        titles(
            pages::space_outline(&pool, f.workspace, f.one, 100)
                .await
                .expect("listed")
        ),
        ["first"]
    );
}
