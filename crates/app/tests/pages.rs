//! Pages: where they are and who reaches them, nesting and order, moving, the trash,
//! shares, favorites, the trees, and the access changes the services tell.

mod support;

use std::fmt::Debug;
use std::time::Duration;

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::pages::{AccessChange, Location, NewPage, PageError, PageUpdate, PageView, Tree};
use neon_app::workspaces::{NewTeamspace, TeamspaceUpdate};
use neon_domain::auth::Principal;
use neon_domain::pages::{Cover, Font, MAX_DEPTH, PageAccess, PageStyle, Placement, Space};
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use serde_json::json;
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

/// Ada owns Design, with Bob as an admin, Cy and Dee as members and Gus as a guest; Eve is
/// in no workspace. Besides General, Cy made and owns Open, Closed and Secret, which is
/// private.
struct Team {
    h: Harness,
    id: Uuid,
    general: Uuid,
    open: Uuid,
    closed: Uuid,
    secret: Uuid,
    ada: Principal,
    bob: Principal,
    cy: Principal,
    dee: Principal,
    gus: Principal,
    eve: Principal,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
    let eve = h.person("eve@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &dee, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;
    let list = h
        .workspaces
        .list_teamspaces(&ada, id)
        .await
        .expect("listed");
    let general = list.teamspaces[0].teamspace.id;
    let mut made = Vec::new();
    for (name, access) in [
        ("Open", TeamspaceAccess::Open),
        ("Closed", TeamspaceAccess::Closed),
        ("Secret", TeamspaceAccess::Private),
    ] {
        let entry = h
            .workspaces
            .create_teamspace(
                &cy,
                id,
                NewTeamspace {
                    name: name.to_owned(),
                    icon: None,
                    hue: None,
                    access,
                },
            )
            .await
            .expect("created");
        made.push(entry.teamspace.id);
    }
    h.changes.take();
    Team {
        h,
        id,
        general,
        open: made[0],
        closed: made[1],
        secret: made[2],
        ada,
        bob,
        cy,
        dee,
        gus,
        eve,
    }
}

impl Team {
    async fn create(
        &self,
        by: &Principal,
        at: Location,
        placement: Placement,
        title: &str,
    ) -> Result<PageView, PageError> {
        self.h
            .pages
            .create_page(
                by,
                self.id,
                at,
                placement,
                NewPage {
                    title: title.to_owned(),
                    ..NewPage::default()
                },
            )
            .await
    }

    /// A page `by` makes last at `at`.
    async fn page(&self, by: &Principal, at: Location, title: &str) -> Uuid {
        self.create(by, at, Placement::Last, title)
            .await
            .unwrap_or_else(|error| panic!("{title} is created: {error}"))
            .page
            .id
    }

    async fn access(&self, who: &Principal, id: Uuid) -> Result<PageAccess, PageError> {
        self.h.pages.page(who, id).await.map(|view| view.access)
    }

    async fn share(&self, by: &Principal, page: Uuid, with: &Principal, access: PageAccess) {
        self.h
            .pages
            .share_page(by, page, with.user.id, access)
            .await
            .expect("shared");
    }

    async fn teamspace_titles(&self, who: &Principal, teamspace: Uuid) -> Vec<String> {
        let tree = self
            .h
            .pages
            .teamspace_tree(who, teamspace, None, None)
            .await
            .expect("listed");
        titles(&tree)
    }
}

fn titles(tree: &Tree) -> Vec<String> {
    tree.pages.iter().map(|page| page.title.clone()).collect()
}

#[track_caller]
fn assert_not_found<T: Debug>(result: Result<T, PageError>) {
    assert!(
        matches!(result, Err(PageError::NotFound(_))),
        "want not found: {result:?}"
    );
}

#[track_caller]
fn assert_forbidden<T: Debug>(result: Result<T, PageError>) -> &'static str {
    match result {
        Err(PageError::Forbidden(reason)) => reason,
        other => panic!("want forbidden: {other:?}"),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn pages_nest_and_keep_the_order_they_are_placed_in(pool: PgPool) {
    let t = team(pool).await;
    let top = Location::Teamspace(t.general);
    let a = t.page(&t.ada, top, "A").await;
    let c = t.page(&t.ada, top, "C").await;
    let b = t
        .create(&t.ada, top, Placement::After(a), "B")
        .await
        .expect("B");
    t.create(&t.ada, top, Placement::First, "Zero")
        .await
        .expect("Zero");
    t.create(&t.ada, top, Placement::Before(c), "B2")
        .await
        .expect("B2");
    let a1 = t.page(&t.ada, Location::Under(a), "A1").await;
    t.page(&t.ada, Location::Under(a), "A2").await;
    t.create(&t.ada, Location::Under(a), Placement::First, "A0")
        .await
        .expect("A0");

    let tree =
        t.h.pages
            .teamspace_tree(&t.dee, t.general, None, None)
            .await
            .expect("listed");
    assert_eq!(
        titles(&tree),
        ["Zero", "A", "A0", "A1", "A2", "B", "B2", "C"]
    );
    assert_eq!(tree.next, None);
    let entry = |title: &str| {
        tree.pages
            .iter()
            .find(|page| page.title == title)
            .expect("listed")
    };
    assert_eq!(entry("A").child_count, 3);
    assert_eq!(entry("A1").parent_id, Some(a));
    assert_eq!(entry("B").parent_id, None);
    assert_eq!(entry("A1").space, Space::Teamspace(t.general));
    assert_eq!(entry("A1").access, PageAccess::Edit);
    assert_eq!(b.page.parent_id, None);
    assert_eq!(b.access, PageAccess::Edit);
    assert_eq!(b.page.created_by, Some(t.ada.user.id));
    assert_eq!(b.page.style, PageStyle::default());

    let beside_a_subpage = t.create(&t.ada, top, Placement::After(a1), "X").await;
    assert!(
        matches!(beside_a_subpage, Err(PageError::InvalidInput(_))),
        "{beside_a_subpage:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_teamspace_s_members_edit_its_pages_and_others_view_them_unless_it_is_private(
    pool: PgPool,
) {
    let t = team(pool).await;
    let general = t
        .page(&t.cy, Location::Teamspace(t.general), "General")
        .await;
    let open = t.page(&t.cy, Location::Teamspace(t.open), "Open").await;
    let closed = t.page(&t.cy, Location::Teamspace(t.closed), "Closed").await;
    let secret = t.page(&t.cy, Location::Teamspace(t.secret), "Secret").await;

    for who in [&t.ada, &t.bob, &t.cy, &t.dee] {
        assert_eq!(t.access(who, general).await.ok(), Some(PageAccess::Edit));
    }
    for page in [open, closed] {
        assert_eq!(t.access(&t.cy, page).await.ok(), Some(PageAccess::Edit));
        for outside in [&t.ada, &t.bob, &t.dee] {
            assert_eq!(
                t.access(outside, page).await.ok(),
                Some(PageAccess::View),
                "the workspace's owners and admins too view a teamspace they are not in"
            );
        }
    }
    assert_eq!(t.access(&t.cy, secret).await.ok(), Some(PageAccess::Edit));
    for outside in [&t.ada, &t.bob, &t.dee, &t.gus, &t.eve] {
        assert_not_found(t.access(outside, secret).await);
    }
    for page in [general, open, closed] {
        assert_not_found(t.access(&t.gus, page).await);
        assert_not_found(t.access(&t.eve, page).await);
    }

    // The trees follow the teamspaces.
    assert_eq!(t.teamspace_titles(&t.dee, t.closed).await, ["Closed"]);
    assert_not_found(t.h.pages.teamspace_tree(&t.dee, t.secret, None, None).await);
    assert_not_found(t.h.pages.teamspace_tree(&t.ada, t.secret, None, None).await);
    assert_not_found(
        t.h.pages
            .teamspace_tree(&t.gus, t.general, None, None)
            .await,
    );
    assert_not_found(
        t.h.pages
            .teamspace_tree(&t.eve, t.general, None, None)
            .await,
    );
    assert_not_found(
        t.h.pages
            .teamspace_tree(&t.dee, Uuid::now_v7(), None, None)
            .await,
    );

    // Viewers change nothing.
    assert_forbidden(
        t.h.pages
            .update_page(&t.dee, closed, PageUpdate::default())
            .await,
    );
    assert_forbidden(t.h.pages.trash_page(&t.dee, closed).await);
    assert_forbidden(
        t.create(&t.dee, Location::Under(closed), Placement::Last, "x")
            .await,
    );
    let reason = assert_forbidden(
        t.create(&t.dee, Location::Teamspace(t.closed), Placement::Last, "x")
            .await,
    );
    assert_eq!(
        reason,
        "only the teamspace's members add pages to it: join it first"
    );
    assert_not_found(
        t.create(&t.dee, Location::Teamspace(t.secret), Placement::Last, "x")
            .await,
    );

    // Joining makes an editor.
    t.h.workspaces
        .join_teamspace(&t.dee, t.id, t.open)
        .await
        .expect("joined");
    assert_eq!(t.access(&t.dee, open).await.ok(), Some(PageAccess::Edit));
    t.create(&t.dee, Location::Teamspace(t.open), Placement::Last, "Mine")
        .await
        .expect("a member adds pages");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn private_pages_are_their_owner_s_alone(pool: PgPool) {
    let t = team(pool).await;
    let notes = t.page(&t.cy, Location::Private, "Notes").await;
    let draft = t.page(&t.cy, Location::Under(notes), "Draft").await;

    assert_eq!(
        t.h.pages.page(&t.cy, notes).await.expect("seen").page.space,
        Space::Private {
            owner: t.cy.user.id
        }
    );
    for other in [&t.ada, &t.bob, &t.dee, &t.gus, &t.eve] {
        assert_not_found(t.access(other, notes).await);
        assert_not_found(t.access(other, draft).await);
    }
    let mine =
        t.h.pages
            .private_tree(&t.cy, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(titles(&mine), ["Notes", "Draft"]);
    let theirs =
        t.h.pages
            .private_tree(&t.dee, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(theirs, Tree::default());
    assert_eq!(
        assert_forbidden(t.h.pages.private_tree(&t.gus, t.id, None, None).await),
        "guests have no private pages: they see the pages shared with them"
    );
    assert_forbidden(
        t.create(&t.gus, Location::Private, Placement::Last, "x")
            .await,
    );
    assert_not_found(t.h.pages.private_tree(&t.eve, t.id, None, None).await);
    assert_not_found(
        t.create(&t.eve, Location::Private, Placement::Last, "x")
            .await,
    );

    // Moved into a teamspace, the pages take its access.
    let shared_space = t.page(&t.cy, Location::Teamspace(t.general), "Team").await;
    t.h.pages
        .move_page(&t.cy, notes, Location::Under(shared_space), Placement::Last)
        .await
        .expect("moved");
    assert_eq!(t.access(&t.dee, draft).await.ok(), Some(PageAccess::Edit));
    assert_eq!(
        t.h.pages
            .page(&t.dee, draft)
            .await
            .expect("seen")
            .page
            .space,
        Space::Teamspace(t.general)
    );
    assert_eq!(
        titles(
            &t.h.pages
                .private_tree(&t.cy, t.id, None, None)
                .await
                .expect("listed")
        ),
        Vec::<String>::new()
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_guest_reaches_only_the_pages_shared_with_them_and_those_under_them(pool: PgPool) {
    let t = team(pool).await;
    let top = Location::Teamspace(t.general);
    let root = t.page(&t.ada, top, "Root").await;
    let shared = t.page(&t.ada, Location::Under(root), "Shared").await;
    let below = t.page(&t.ada, Location::Under(shared), "Below").await;
    let beside = t.page(&t.ada, top, "Beside").await;
    t.share(&t.ada, shared, &t.gus, PageAccess::View).await;

    assert_eq!(t.access(&t.gus, shared).await.ok(), Some(PageAccess::View));
    assert_eq!(t.access(&t.gus, below).await.ok(), Some(PageAccess::View));
    assert_not_found(t.access(&t.gus, root).await);
    assert_not_found(t.access(&t.gus, beside).await);
    let tree =
        t.h.pages
            .shared_tree(&t.gus, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(titles(&tree), ["Shared", "Below"]);
    assert_eq!(tree.pages[0].parent_id, None, "the page above stays hidden");
    assert_eq!(tree.pages[1].parent_id, Some(shared));
    assert_eq!(tree.pages[0].access, PageAccess::View);
    let crumbs = t.h.pages.breadcrumbs(&t.gus, below).await.expect("listed");
    let crumbs: Vec<&str> = crumbs.iter().map(|page| page.title.as_str()).collect();
    assert_eq!(crumbs, ["Shared", "Below"]);
    let all = t.h.pages.breadcrumbs(&t.dee, below).await.expect("listed");
    assert_eq!(all.len(), 3, "a member sees the whole way down");

    // A view share is no edit share.
    assert_forbidden(
        t.h.pages
            .update_page(&t.gus, below, PageUpdate::default())
            .await,
    );
    t.h.pages
        .change_share(&t.ada, shared, t.gus.user.id, PageAccess::Edit)
        .await
        .expect("changed");
    let renamed =
        t.h.pages
            .update_page(
                &t.gus,
                below,
                PageUpdate {
                    title: Some("Renamed".to_owned()),
                    ..PageUpdate::default()
                },
            )
            .await
            .expect("an edit share edits");
    assert_eq!(renamed.page.last_edited_by, Some(t.gus.user.id));
    let made = t.page(&t.gus, Location::Under(below), "By the guest").await;
    assert_eq!(t.access(&t.gus, made).await.ok(), Some(PageAccess::Edit));

    // Guests know no other member.
    assert_eq!(
        assert_forbidden(t.h.pages.shares(&t.gus, shared).await),
        "guests see no member but themselves"
    );
    assert_forbidden(
        t.h.pages
            .share_page(&t.gus, shared, t.dee.user.id, PageAccess::View)
            .await,
    );
    assert_not_found(
        t.h.pages
            .teamspace_tree(&t.gus, t.general, None, None)
            .await,
    );

    t.h.pages
        .remove_share(&t.ada, shared, t.gus.user.id)
        .await
        .expect("removed");
    assert_not_found(t.access(&t.gus, below).await);
    assert_eq!(
        t.h.pages
            .shared_tree(&t.gus, t.id, None, None)
            .await
            .expect("listed"),
        Tree::default()
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn access_is_the_most_of_what_the_teamspace_and_every_share_above_give(pool: PgPool) {
    let t = team(pool).await;
    let outer = t.page(&t.cy, Location::Teamspace(t.closed), "Outer").await;
    let inner = t.page(&t.cy, Location::Under(outer), "Inner").await;
    let deepest = t.page(&t.cy, Location::Under(inner), "Deepest").await;
    t.share(&t.cy, inner, &t.dee, PageAccess::Edit).await;

    assert_eq!(t.access(&t.dee, outer).await.ok(), Some(PageAccess::View));
    assert_eq!(t.access(&t.dee, inner).await.ok(), Some(PageAccess::Edit));
    assert_eq!(t.access(&t.dee, deepest).await.ok(), Some(PageAccess::Edit));
    t.page(&t.dee, Location::Under(deepest), "Allowed").await;
    assert_forbidden(
        t.create(&t.dee, Location::Under(outer), Placement::Last, "x")
            .await,
    );

    // A view share takes nothing from a teamspace's member.
    t.share(&t.cy, outer, &t.ada, PageAccess::View).await;
    t.h.workspaces
        .join_teamspace(&t.ada, t.id, t.closed)
        .await
        .expect("an owner joins a closed teamspace");
    assert_eq!(t.access(&t.ada, outer).await.ok(), Some(PageAccess::Edit));

    // A share reaches into a private teamspace, for that page alone.
    let hidden = t.page(&t.cy, Location::Teamspace(t.secret), "Hidden").await;
    t.page(&t.cy, Location::Teamspace(t.secret), "Unshared")
        .await;
    t.share(&t.cy, hidden, &t.dee, PageAccess::View).await;
    assert_eq!(t.access(&t.dee, hidden).await.ok(), Some(PageAccess::View));
    let shared =
        t.h.pages
            .shared_tree(&t.dee, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(titles(&shared), ["Inner", "Deepest", "Allowed", "Hidden"]);
    assert_not_found(t.h.pages.teamspace_tree(&t.dee, t.secret, None, None).await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn shared_pages_leave_out_those_of_the_callers_teamspaces(pool: PgPool) {
    let t = team(pool).await;
    t.h.workspaces
        .join_teamspace(&t.dee, t.id, t.open)
        .await
        .expect("Dee joins the open teamspace");
    let roadmap = t.page(&t.cy, Location::Teamspace(t.open), "Roadmap").await;
    let q1 = t.page(&t.cy, Location::Under(roadmap), "Q1").await;
    t.page(&t.cy, Location::Under(q1), "Hiring").await;
    t.share(&t.cy, q1, &t.dee, PageAccess::Edit).await;
    let outside = t
        .page(&t.cy, Location::Teamspace(t.closed), "Outside")
        .await;
    t.share(&t.cy, outside, &t.dee, PageAccess::View).await;
    let shared = |who: Principal| {
        let pages = t.h.pages.clone();
        let id = t.id;
        async move {
            pages
                .shared_tree(&who, id, None, None)
                .await
                .expect("listed")
        }
    };

    // Q1 shows in the open teamspace's tree, so only the closed one's page is listed.
    assert_eq!(titles(&shared(t.dee.clone()).await), ["Outside"]);
    // Out of the teamspace, the share is how Dee reaches Q1, with the page under it.
    t.h.workspaces
        .leave_teamspace(&t.dee, t.id, t.open)
        .await
        .expect("Dee leaves");
    assert_eq!(
        titles(&shared(t.dee.clone()).await),
        ["Q1", "Hiring", "Outside"]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn shares_are_listed_with_those_inherited_from_above(pool: PgPool) {
    let t = team(pool).await;
    let above = t
        .page(&t.ada, Location::Teamspace(t.general), "Above")
        .await;
    let page = t.page(&t.ada, Location::Under(above), "Page").await;
    t.share(&t.ada, above, &t.dee, PageAccess::View).await;
    t.share(&t.ada, page, &t.gus, PageAccess::Edit).await;

    let listed = t.h.pages.shares(&t.cy, page).await.expect("listed");
    let seen: Vec<(Uuid, Uuid, PageAccess, bool)> = listed
        .iter()
        .map(|s| (s.person.id, s.share.page_id, s.share.access, s.inherited))
        .collect();
    assert_eq!(
        seen,
        [
            (t.gus.user.id, page, PageAccess::Edit, false),
            (t.dee.user.id, above, PageAccess::View, true),
        ]
    );
    assert_eq!(listed[0].person.email.as_deref(), Some("gus@example.com"));
    assert_eq!(
        t.h.pages.shares(&t.cy, above).await.expect("listed").len(),
        1
    );

    // An inherited share changes on its own page.
    assert_not_found(
        t.h.pages
            .change_share(&t.ada, page, t.dee.user.id, PageAccess::Edit)
            .await,
    );
    assert_not_found(t.h.pages.remove_share(&t.ada, page, t.dee.user.id).await);
    let changed =
        t.h.pages
            .change_share(&t.ada, above, t.dee.user.id, PageAccess::Edit)
            .await
            .expect("changed");
    assert_eq!(changed.share.access, PageAccess::Edit);

    // What a share may not be.
    assert!(matches!(
        t.h.pages
            .share_page(&t.ada, page, t.ada.user.id, PageAccess::View)
            .await,
        Err(PageError::InvalidInput(_))
    ));
    assert_not_found(
        t.h.pages
            .share_page(&t.ada, page, t.eve.user.id, PageAccess::View)
            .await,
    );
    assert!(matches!(
        t.h.pages
            .share_page(&t.ada, above, t.dee.user.id, PageAccess::View)
            .await,
        Err(PageError::AlreadyShared)
    ));
    let closed = t.page(&t.cy, Location::Teamspace(t.closed), "Closed").await;
    assert_eq!(
        assert_forbidden(
            t.h.pages
                .share_page(&t.dee, closed, t.bob.user.id, PageAccess::View)
                .await
        ),
        "you may view this page, not edit it"
    );
    assert_eq!(
        t.h.pages
            .shares(&t.dee, closed)
            .await
            .expect("viewers list")
            .len(),
        0
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn moves_take_the_subpages_refuse_cycles_and_need_edit_access_at_both_ends(pool: PgPool) {
    let t = team(pool).await;
    let general = Location::Teamspace(t.general);
    let a = t.page(&t.cy, general, "A").await;
    let b = t.page(&t.cy, Location::Under(a), "B").await;
    let c = t.page(&t.cy, Location::Under(b), "C").await;
    t.page(&t.cy, Location::Teamspace(t.open), "O").await;
    let move_to = |who, page, to| t.h.pages.move_page(who, page, to, Placement::Last);

    assert!(matches!(
        move_to(&t.cy, a, Location::Under(c)).await,
        Err(PageError::Cycle)
    ));
    assert!(matches!(
        move_to(&t.cy, a, Location::Under(a)).await,
        Err(PageError::Cycle)
    ));
    // Dee edits B but is not in Open, nor edits Closed's pages.
    assert_forbidden(move_to(&t.dee, b, Location::Teamspace(t.open)).await);
    let closed = t.page(&t.cy, Location::Teamspace(t.closed), "K").await;
    assert_forbidden(move_to(&t.dee, b, Location::Under(closed)).await);
    assert_not_found(move_to(&t.dee, b, Location::Teamspace(t.secret)).await);
    // Nor does she edit what is in Open, to move it out.
    let open_page = t.page(&t.cy, Location::Teamspace(t.open), "P").await;
    assert_forbidden(move_to(&t.dee, open_page, general).await);

    let moved = move_to(&t.cy, b, Location::Teamspace(t.open))
        .await
        .expect("moved");
    assert_eq!(moved.page.parent_id, None);
    assert_eq!(moved.page.space, Space::Teamspace(t.open));
    assert_eq!(
        t.h.pages.page(&t.cy, c).await.expect("seen").page.space,
        Space::Teamspace(t.open),
        "the subpages go along"
    );
    assert_eq!(t.teamspace_titles(&t.cy, t.general).await, ["A"]);
    assert_eq!(
        t.teamspace_titles(&t.cy, t.open).await,
        ["O", "P", "B", "C"]
    );
    assert_eq!(t.access(&t.dee, c).await.ok(), Some(PageAccess::View));

    move_to(&t.cy, c, Location::Private).await.expect("moved");
    assert_not_found(t.access(&t.dee, c).await);
    assert_eq!(
        t.h.pages.page(&t.cy, c).await.expect("seen").page.space,
        Space::Private {
            owner: t.cy.user.id
        }
    );

    // Pages move within their workspace.
    let other = t.h.workspace(&t.cy, "Other").await.workspace.id;
    let elsewhere =
        t.h.pages
            .create_page(
                &t.cy,
                other,
                Location::Private,
                Placement::Last,
                NewPage::default(),
            )
            .await
            .expect("created")
            .page
            .id;
    assert!(matches!(
        move_to(&t.cy, a, Location::Under(elsewhere)).await,
        Err(PageError::InvalidInput(_))
    ));
    assert_not_found(
        t.h.pages
            .create_page(
                &t.cy,
                t.id,
                Location::Under(elsewhere),
                Placement::Last,
                NewPage::default(),
            )
            .await,
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn moving_within_a_place_reorders(pool: PgPool) {
    let t = team(pool).await;
    let top = Location::Teamspace(t.general);
    let a = t.page(&t.ada, top, "A").await;
    t.page(&t.ada, top, "D").await;
    let e = t.page(&t.ada, top, "E").await;
    let reorder = |page, placement| t.h.pages.move_page(&t.ada, page, top, placement);

    reorder(e, Placement::Before(a)).await.expect("moved");
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["E", "A", "D"]);
    reorder(e, Placement::After(a)).await.expect("moved");
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["A", "E", "D"]);
    reorder(a, Placement::Last).await.expect("moved");
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["E", "D", "A"]);
    reorder(a, Placement::First).await.expect("moved");
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["A", "E", "D"]);
    assert!(matches!(
        reorder(a, Placement::After(a)).await,
        Err(PageError::InvalidInput(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_share_moves_a_page_only_within_its_space(pool: PgPool) {
    let t = team(pool).await;
    let move_to = |who, page, to| t.h.pages.move_page(who, page, to, Placement::Last);
    const SHARED_STAYS: &str = "a page shared with you moves only within its space: those in \
                                its teamspace, or its owner, may move it elsewhere";

    // Cy's private Diary, with Entry under it, shared with Dee to edit.
    let diary = t.page(&t.cy, Location::Private, "Diary").await;
    let entry = t.page(&t.cy, Location::Under(diary), "Entry").await;
    t.share(&t.cy, diary, &t.dee, PageAccess::Edit).await;
    for page in [diary, entry] {
        for to in [Location::Private, Location::Teamspace(t.general)] {
            assert_eq!(
                assert_forbidden(move_to(&t.dee, page, to).await),
                SHARED_STAYS
            );
        }
    }
    let dee_s = t
        .page(&t.dee, Location::Teamspace(t.general), "Dee's")
        .await;
    assert_eq!(
        assert_forbidden(move_to(&t.dee, entry, Location::Under(dee_s)).await),
        SHARED_STAYS
    );
    assert_eq!(t.access(&t.cy, diary).await.ok(), Some(PageAccess::Edit));
    assert_eq!(t.access(&t.cy, entry).await.ok(), Some(PageAccess::Edit));
    // Within Cy's pages, under a page Dee edits, it moves.
    let note = t.page(&t.dee, Location::Under(diary), "Note").await;
    let moved = move_to(&t.dee, entry, Location::Under(note))
        .await
        .expect("moved");
    assert_eq!(
        moved.page.space,
        Space::Private {
            owner: t.cy.user.id
        }
    );

    // A private teamspace's page shared with someone outside it stays in it.
    let plan = t.page(&t.cy, Location::Teamspace(t.secret), "Plan").await;
    t.share(&t.cy, plan, &t.dee, PageAccess::Edit).await;
    t.share(&t.cy, plan, &t.gus, PageAccess::Edit).await;
    for to in [Location::Teamspace(t.general), Location::Under(dee_s)] {
        assert_eq!(
            assert_forbidden(move_to(&t.dee, plan, to).await),
            SHARED_STAYS
        );
    }
    assert_not_found(t.access(&t.ada, plan).await);
    // Nor does a guest with shares in both move one page under the other.
    t.share(&t.dee, dee_s, &t.gus, PageAccess::Edit).await;
    assert_eq!(
        assert_forbidden(move_to(&t.gus, plan, Location::Under(dee_s)).await),
        SHARED_STAYS
    );
    assert_not_found(t.access(&t.bob, plan).await);

    // Those its space lets edit it move it elsewhere, and a share's holder may add their
    // own pages under it.
    let moved = move_to(&t.cy, plan, Location::Teamspace(t.open))
        .await
        .expect("moved");
    assert_eq!(moved.page.space, Space::Teamspace(t.open));
    let mine = t.page(&t.dee, Location::Private, "Mine").await;
    let moved = move_to(&t.dee, mine, Location::Under(diary))
        .await
        .expect("moved");
    assert_eq!(
        moved.page.space,
        Space::Private {
            owner: t.cy.user.id
        }
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn trashing_takes_the_subpages_and_restoring_brings_back_what_went_with_it(pool: PgPool) {
    let t = team(pool).await;
    let a = t.page(&t.dee, Location::Teamspace(t.general), "A").await;
    let b = t.page(&t.dee, Location::Under(a), "B").await;
    let c = t.page(&t.dee, Location::Under(b), "C").await;
    t.page(&t.dee, Location::Under(a), "B2").await;
    let trashed_alone = t.h.pages.trash_page(&t.dee, c).await.expect("trashed");
    assert_eq!(trashed_alone.page.trashed_with, Some(c));
    t.h.time.advance(Duration::from_secs(60));
    let trashed = t.h.pages.trash_page(&t.dee, a).await.expect("trashed");

    assert_eq!(trashed.page.trashed_by, Some(t.dee.user.id));
    assert_eq!(trashed.page.trashed_with, Some(a));
    assert_eq!(trashed.access, PageAccess::Edit);
    let b_now = t.h.pages.page(&t.dee, b).await.expect("editors see it");
    assert_eq!(b_now.page.trashed_with, Some(a));
    assert_eq!(
        t.h.pages
            .page(&t.dee, c)
            .await
            .expect("seen")
            .page
            .trashed_with,
        Some(c),
        "trashed before, on its own"
    );
    assert_eq!(
        t.teamspace_titles(&t.dee, t.general).await,
        Vec::<String>::new()
    );
    assert_eq!(
        assert_forbidden(
            t.h.pages
                .update_page(&t.dee, a, PageUpdate::default())
                .await
        ),
        "this page is in the trash: restore it first"
    );
    assert_forbidden(t.h.pages.trash_page(&t.dee, a).await);
    assert_forbidden(
        t.create(&t.dee, Location::Under(a), Placement::Last, "x")
            .await,
    );
    assert_not_found(t.h.pages.restore_page(&t.dee, Uuid::nil()).await);

    let listed =
        t.h.pages
            .trash(&t.dee, t.id, None, None)
            .await
            .expect("listed");
    let ids: Vec<Uuid> = listed.pages.iter().map(|view| view.page.id).collect();
    assert_eq!(ids, [a, c], "pages trashed on their own, latest first");

    let restored = t.h.pages.restore_page(&t.dee, a).await.expect("restored");
    assert_eq!(restored.page.trashed_at, None);
    assert_eq!(
        t.teamspace_titles(&t.dee, t.general).await,
        ["A", "B", "B2"]
    );
    t.h.pages.restore_page(&t.dee, c).await.expect("restored");
    assert_eq!(
        t.teamspace_titles(&t.dee, t.general).await,
        ["A", "B", "C", "B2"],
        "back under its parent, where it was"
    );
    // One out of the trash has nothing to restore.
    assert!(matches!(
        t.h.pages.restore_page(&t.dee, a).await,
        Err(PageError::InvalidInput(message)) if message.contains("not in the trash")
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_trashed_page_shows_only_to_those_who_could_edit_it(pool: PgPool) {
    let t = team(pool).await;
    let page = t.page(&t.cy, Location::Teamspace(t.closed), "Closed").await;
    t.share(&t.cy, page, &t.gus, PageAccess::View).await;
    t.h.pages.trash_page(&t.cy, page).await.expect("trashed");

    for viewer in [&t.dee, &t.ada, &t.gus] {
        assert_not_found(t.access(viewer, page).await);
        assert_not_found(t.h.pages.restore_page(viewer, page).await);
        assert_not_found(t.h.pages.delete_page(viewer, page).await);
    }
    let dee_trash =
        t.h.pages
            .trash(&t.dee, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(dee_trash.pages, vec![]);
    let cy_trash =
        t.h.pages
            .trash(&t.cy, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(cy_trash.pages.len(), 1);
    assert_not_found(t.h.pages.trash(&t.eve, t.id, None, None).await);
    assert_eq!(
        t.h.pages
            .trash(&t.gus, t.id, None, None)
            .await
            .expect("listed")
            .pages,
        vec![]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_whose_parent_is_in_the_trash_is_restored_at_the_top_of_its_space(pool: PgPool) {
    let t = team(pool).await;
    let a = t.page(&t.ada, Location::Teamspace(t.general), "A").await;
    let b = t.page(&t.ada, Location::Under(a), "B").await;
    let c = t.page(&t.ada, Location::Under(b), "C").await;
    t.h.pages.trash_page(&t.ada, b).await.expect("trashed");
    t.h.pages.trash_page(&t.ada, a).await.expect("trashed");

    let restored = t.h.pages.restore_page(&t.ada, c).await.expect("restored");
    assert_eq!(restored.page.parent_id, None);
    assert_eq!(
        t.h.pages
            .page(&t.ada, b)
            .await
            .expect("seen")
            .page
            .trashed_with,
        Some(b),
        "what it went with stays in the trash"
    );
    t.h.pages.restore_page(&t.ada, b).await.expect("restored");
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["C", "B"]);
    t.h.pages.restore_page(&t.ada, a).await.expect("restored");
    assert_eq!(
        t.teamspace_titles(&t.ada, t.general).await,
        ["C", "B", "A"],
        "C took its old place while it was in the trash, so it comes back last"
    );

    // Someone who may not add pages at the top restores the page above first.
    let p = t.page(&t.ada, Location::Teamspace(t.general), "P").await;
    let q = t.page(&t.ada, Location::Under(p), "Q").await;
    t.page(&t.ada, Location::Under(q), "R").await;
    t.share(&t.ada, p, &t.gus, PageAccess::Edit).await;
    t.h.pages
        .trash_page(&t.gus, p)
        .await
        .expect("an editor trashes");
    let refused = assert_forbidden(t.h.pages.restore_page(&t.gus, q).await);
    assert!(refused.contains("restore that page first"), "{refused}");
    t.h.pages.restore_page(&t.gus, p).await.expect("restored");
    let tree =
        t.h.pages
            .shared_tree(&t.gus, t.id, None, None)
            .await
            .expect("listed");
    assert_eq!(titles(&tree), ["P", "Q", "R"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_a_page_in_the_trash_is_deleted_for_good_with_everything_under_it(pool: PgPool) {
    let t = team(pool).await;
    let a = t.page(&t.ada, Location::Teamspace(t.general), "A").await;
    let b = t.page(&t.ada, Location::Under(a), "B").await;
    t.share(&t.ada, b, &t.gus, PageAccess::View).await;

    assert!(matches!(
        t.h.pages.delete_page(&t.ada, a).await,
        Err(PageError::InvalidInput(message))
            if message == "only a page in the trash is deleted for good: trash it first"
    ));
    t.h.pages.trash_page(&t.ada, a).await.expect("trashed");
    t.h.pages.delete_page(&t.ada, a).await.expect("deleted");

    assert_not_found(t.access(&t.ada, a).await);
    assert_not_found(t.access(&t.ada, b).await);
    assert_eq!(t.h.scalar::<i64>("SELECT count(*) FROM pages").await, 0);
    assert_eq!(
        t.h.scalar::<i64>("SELECT count(*) FROM page_shares").await,
        0
    );
    assert_not_found(t.h.pages.delete_page(&t.ada, a).await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_trash_is_purged_after_its_retention(pool: PgPool) {
    let t = team(pool).await;
    t.h.set("pages.trash_retention", json!("2d")).await;
    let old = t.page(&t.ada, Location::Teamspace(t.general), "Old").await;
    t.page(&t.ada, Location::Under(old), "Old child").await;
    let recent = t
        .page(&t.ada, Location::Teamspace(t.general), "Recent")
        .await;
    t.page(&t.ada, Location::Teamspace(t.general), "Kept").await;
    t.h.pages.trash_page(&t.ada, old).await.expect("trashed");
    t.h.time.advance(Duration::from_secs(24 * 3600));
    t.h.pages.trash_page(&t.ada, recent).await.expect("trashed");
    t.h.time.advance(Duration::from_secs(25 * 3600));
    t.h.changes.take();

    assert_eq!(t.h.pages.purge_trash().await.expect("purged"), 2);
    assert_eq!(
        t.h.changes.take(),
        vec![AccessChange::Pages {
            workspace_id: t.id,
            page_id: old,
            user_id: None,
        }],
        "what was trashed on its own is told, as its subtree is gone"
    );
    assert_not_found(t.access(&t.ada, old).await);
    assert!(t.access(&t.ada, recent).await.is_ok());
    assert_eq!(t.teamspace_titles(&t.ada, t.general).await, ["Kept"]);
    assert_eq!(t.h.pages.purge_trash().await.expect("purged"), 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_s_title_icon_cover_and_style_change(pool: PgPool) {
    let t = team(pool).await;
    let created =
        t.h.pages
            .create_page(
                &t.cy,
                t.id,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage {
                    title: "  Roadmap ".to_owned(),
                    icon: Some("🗺".to_owned()),
                    cover: Some(Cover::Dusk),
                    style: PageStyle {
                        font: Font::Serif,
                        small_text: true,
                        full_width: false,
                    },
                    content: None,
                },
            )
            .await
            .expect("created");
    assert_eq!(created.page.title, "Roadmap");
    let id = created.page.id;
    t.h.time.advance(Duration::from_secs(60));

    let changed =
        t.h.pages
            .update_page(
                &t.dee,
                id,
                PageUpdate {
                    title: Some(String::new()),
                    icon: Some(None),
                    cover: Some(Some(Cover::Teal)),
                    font: Some(Font::Mono),
                    small_text: None,
                    full_width: Some(true),
                },
            )
            .await
            .expect("changed");
    assert_eq!(changed.page.title, "", "an empty title shows as Untitled");
    assert_eq!(changed.page.icon, None);
    assert_eq!(changed.page.cover, Some(Cover::Teal));
    assert_eq!(
        changed.page.style,
        PageStyle {
            font: Font::Mono,
            small_text: true,
            full_width: true
        }
    );
    assert_eq!(changed.page.last_edited_by, Some(t.dee.user.id));
    assert!(changed.page.updated_at > created.page.updated_at);
    assert_eq!(
        t.h.pages.page(&t.ada, id).await.expect("seen").page,
        changed.page
    );

    for bad in [
        PageUpdate {
            title: Some("x".repeat(501)),
            ..PageUpdate::default()
        },
        PageUpdate {
            title: Some("two\nlines".to_owned()),
            ..PageUpdate::default()
        },
        PageUpdate {
            icon: Some(Some("x".repeat(17))),
            ..PageUpdate::default()
        },
    ] {
        assert!(matches!(
            t.h.pages.update_page(&t.dee, id, bad).await,
            Err(PageError::InvalidInput(_))
        ));
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn favorites_keep_their_order_and_list_only_what_is_still_seen(pool: PgPool) {
    let t = team(pool).await;
    let top = Location::Teamspace(t.general);
    let one = t.page(&t.ada, top, "One").await;
    let two = t.page(&t.ada, top, "Two").await;
    let three = t.page(&t.ada, top, "Three").await;
    t.share(&t.ada, three, &t.gus, PageAccess::View).await;
    let favorites = |who| t.h.pages.favorites(who, Some(t.id));
    let names = |list: Vec<PageView>| -> Vec<String> {
        list.into_iter().map(|view| view.page.title).collect()
    };

    for page in [one, two] {
        t.h.pages
            .add_favorite(&t.dee, page, Placement::Last)
            .await
            .expect("added");
    }
    t.h.pages
        .add_favorite(&t.dee, three, Placement::First)
        .await
        .expect("added");
    assert_eq!(
        names(favorites(&t.dee).await.expect("listed")),
        ["Three", "One", "Two"]
    );
    t.h.pages
        .move_favorite(&t.dee, three, Placement::After(two))
        .await
        .expect("moved");
    assert_eq!(
        names(favorites(&t.dee).await.expect("listed")),
        ["One", "Two", "Three"]
    );
    assert!(matches!(
        t.h.pages.add_favorite(&t.dee, one, Placement::Last).await,
        Err(PageError::AlreadyFavorite)
    ));
    assert_not_found(t.h.pages.move_favorite(&t.ada, one, Placement::First).await);

    // What is in the trash, or no longer seen, is left out, and comes back.
    t.h.pages.trash_page(&t.ada, two).await.expect("trashed");
    assert_eq!(
        names(favorites(&t.dee).await.expect("listed")),
        ["One", "Three"]
    );
    assert_forbidden(t.h.pages.add_favorite(&t.ada, two, Placement::Last).await);
    t.h.pages.restore_page(&t.ada, two).await.expect("restored");
    assert_eq!(
        names(favorites(&t.dee).await.expect("listed")),
        ["One", "Two", "Three"]
    );
    t.h.pages
        .add_favorite(&t.gus, three, Placement::Last)
        .await
        .expect("a guest adds what is shared with them");
    assert_not_found(t.h.pages.add_favorite(&t.gus, one, Placement::Last).await);
    t.h.pages
        .remove_share(&t.ada, three, t.gus.user.id)
        .await
        .expect("removed");
    assert_eq!(favorites(&t.gus).await.expect("listed"), vec![]);
    t.share(&t.ada, three, &t.gus, PageAccess::View).await;
    assert_eq!(names(favorites(&t.gus).await.expect("listed")), ["Three"]);

    // Across workspaces, or in one.
    let other = t.h.workspace(&t.dee, "Other").await.workspace.id;
    let mine =
        t.h.pages
            .create_page(
                &t.dee,
                other,
                Location::Private,
                Placement::Last,
                NewPage::default(),
            )
            .await
            .expect("created")
            .page
            .id;
    t.h.pages
        .add_favorite(&t.dee, mine, Placement::First)
        .await
        .expect("added");
    assert_eq!(
        t.h.pages
            .favorites(&t.dee, None)
            .await
            .expect("listed")
            .len(),
        4
    );
    assert_eq!(
        t.h.pages
            .favorites(&t.dee, Some(other))
            .await
            .expect("listed")[0]
            .page
            .id,
        mine
    );

    t.h.pages
        .remove_favorite(&t.dee, one)
        .await
        .expect("removed");
    assert_not_found(t.h.pages.remove_favorite(&t.dee, one).await);
    assert_eq!(
        names(favorites(&t.dee).await.expect("listed")),
        ["Two", "Three"]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn leaving_a_workspace_ends_its_shares_and_favorites_and_keeps_private_pages(pool: PgPool) {
    let t = team(pool).await;
    let page = t.page(&t.ada, Location::Teamspace(t.general), "Page").await;
    let private = t.page(&t.dee, Location::Private, "Mine").await;
    t.share(&t.ada, page, &t.dee, PageAccess::Edit).await;
    t.h.pages
        .add_favorite(&t.dee, page, Placement::Last)
        .await
        .expect("added");
    t.h.workspaces
        .remove_member(&t.ada, t.id, t.dee.user.id)
        .await
        .expect("removed");

    assert_eq!(
        t.h.scalar::<i64>("SELECT count(*) FROM page_shares").await,
        0
    );
    assert_eq!(t.h.scalar::<i64>("SELECT count(*) FROM favorites").await, 0);
    assert_not_found(t.access(&t.dee, page).await);
    assert_not_found(t.access(&t.dee, private).await);
    t.h.bring_in(&t.ada, t.id, &t.dee, WorkspaceRole::Member)
        .await;
    assert_eq!(t.access(&t.dee, private).await.ok(), Some(PageAccess::Edit));

    // Made a guest, a member keeps only what is shared with them.
    t.share(&t.ada, page, &t.dee, PageAccess::View).await;
    t.h.workspaces
        .change_member_role(&t.ada, t.id, t.dee.user.id, WorkspaceRole::Guest)
        .await
        .expect("changed");
    assert_not_found(t.access(&t.dee, private).await);
    assert_eq!(t.access(&t.dee, page).await.ok(), Some(PageAccess::View));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn trees_come_in_parts_in_the_order_of_the_tree(pool: PgPool) {
    let t = team(pool).await;
    for i in 0..4 {
        let parent = t
            .page(&t.ada, Location::Teamspace(t.general), &format!("P{i}"))
            .await;
        for j in 0..2 {
            t.page(&t.ada, Location::Under(parent), &format!("P{i}.{j}"))
                .await;
        }
    }
    let whole =
        t.h.pages
            .teamspace_tree(&t.ada, t.general, None, None)
            .await
            .expect("listed");
    assert_eq!(whole.pages.len(), 12);
    assert_eq!(&titles(&whole)[..4], ["P0", "P0.0", "P0.1", "P1"]);

    let mut parts = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let part =
            t.h.pages
                .teamspace_tree(&t.ada, t.general, after.as_deref(), Some(5))
                .await
                .expect("listed");
        assert!(part.pages.len() <= 5);
        parts.extend(titles(&part));
        match part.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(parts, titles(&whole));
    for bad in [Some(0), Some(1001)] {
        assert!(matches!(
            t.h.pages.teamspace_tree(&t.ada, t.general, None, bad).await,
            Err(PageError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        t.h.pages
            .teamspace_tree(&t.ada, t.general, Some("not a cursor"), None)
            .await,
        Err(PageError::InvalidInput(_))
    ));

    // The trash comes in parts too.
    let mut trashed = Vec::new();
    for page in whole.pages.iter().filter(|page| page.parent_id.is_none()) {
        t.h.time.advance(Duration::from_secs(1));
        t.h.pages
            .trash_page(&t.ada, page.id)
            .await
            .expect("trashed");
        trashed.push(page.id);
    }
    trashed.reverse();
    let first =
        t.h.pages
            .trash(&t.ada, t.id, None, Some(3))
            .await
            .expect("listed");
    let rest =
        t.h.pages
            .trash(&t.ada, t.id, first.next.as_deref(), Some(3))
            .await
            .expect("listed");
    let ids: Vec<Uuid> = first
        .pages
        .iter()
        .chain(&rest.pages)
        .map(|view| view.page.id)
        .collect();
    assert_eq!(ids, trashed);
    assert!(matches!(
        t.h.pages.trash(&t.ada, t.id, Some("x.y"), None).await,
        Err(PageError::InvalidInput(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn pages_nest_only_so_deep(pool: PgPool) {
    let t = team(pool).await;
    let top = t.page(&t.ada, Location::Teamspace(t.general), "1").await;
    let mut deepest = top;
    for depth in 2..=MAX_DEPTH {
        deepest = t
            .page(&t.ada, Location::Under(deepest), &depth.to_string())
            .await;
    }
    assert!(matches!(
        t.create(
            &t.ada,
            Location::Under(deepest),
            Placement::Last,
            "too deep"
        )
        .await,
        Err(PageError::TooDeep)
    ));
    let other = t
        .page(&t.ada, Location::Teamspace(t.general), "Other")
        .await;
    t.page(&t.ada, Location::Under(other), "Its child").await;
    let parent_of_deepest =
        t.h.pages
            .page(&t.ada, deepest)
            .await
            .expect("seen")
            .page
            .parent_id
            .expect("a parent");
    assert!(matches!(
        t.h.pages
            .move_page(
                &t.ada,
                other,
                Location::Under(parent_of_deepest),
                Placement::Last
            )
            .await,
        Err(PageError::TooDeep)
    ));
    assert_eq!(
        t.h.pages
            .breadcrumbs(&t.ada, deepest)
            .await
            .expect("listed")
            .len(),
        MAX_DEPTH
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn siblings_get_fresh_positions_when_one_would_grow_too_long(pool: PgPool) {
    let t = team(pool).await;
    let top = Location::Teamspace(t.general);
    let a = t.page(&t.ada, top, "A").await;
    let b = t.page(&t.ada, top, "B").await;
    // Keys as long as they may be, side by side, with no room left at that length.
    let long = format!("a0{}", "V".repeat(61));
    for (id, key) in [(a, format!("{long}V")), (b, format!("{long}W"))] {
        sqlx::query("UPDATE pages SET position = $2 WHERE id = $1")
            .bind(id)
            .bind(key)
            .execute(&t.h.pool)
            .await
            .expect("set");
    }

    let between = t
        .create(&t.ada, top, Placement::After(a), "Between")
        .await
        .expect("created");
    assert!(
        between.page.position.len() <= 4,
        "{}",
        between.page.position
    );
    assert_eq!(
        t.teamspace_titles(&t.ada, t.general).await,
        ["A", "Between", "B"]
    );
    let longest: i32 =
        t.h.scalar("SELECT max(char_length(position)) FROM pages")
            .await;
    assert!(longest <= 4, "{longest}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_change_that_may_alter_access_is_told(pool: PgPool) {
    let t = team(pool).await;
    let ws = t.id;
    let page = t.page(&t.ada, Location::Teamspace(t.general), "Page").await;
    let pages = |page_id, user_id| AccessChange::Pages {
        workspace_id: ws,
        page_id,
        user_id,
    };
    assert_eq!(
        t.h.changes.take(),
        vec![],
        "a new page changes nobody's access"
    );

    t.share(&t.ada, page, &t.gus, PageAccess::View).await;
    t.h.pages
        .change_share(&t.ada, page, t.gus.user.id, PageAccess::Edit)
        .await
        .expect("changed");
    t.h.pages
        .remove_share(&t.ada, page, t.gus.user.id)
        .await
        .expect("removed");
    let gus = Some(t.gus.user.id);
    assert_eq!(
        t.h.changes.take(),
        vec![pages(page, gus), pages(page, gus), pages(page, gus)]
    );

    t.h.pages
        .move_page(&t.ada, page, Location::Private, Placement::Last)
        .await
        .expect("moved");
    // Placed elsewhere among the same siblings, it changes nobody's access.
    t.page(&t.ada, Location::Private, "Sibling").await;
    t.h.pages
        .move_page(&t.ada, page, Location::Private, Placement::Last)
        .await
        .expect("reordered");
    t.h.pages.trash_page(&t.ada, page).await.expect("trashed");
    t.h.pages
        .restore_page(&t.ada, page)
        .await
        .expect("restored");
    t.h.pages.trash_page(&t.ada, page).await.expect("trashed");
    t.h.pages.delete_page(&t.ada, page).await.expect("deleted");
    assert_eq!(t.h.changes.take(), vec![pages(page, None); 5]);
    t.h.pages
        .update_page(
            &t.ada,
            t.page(&t.ada, Location::Private, "x").await,
            PageUpdate::default(),
        )
        .await
        .expect("changed");
    assert_eq!(
        t.h.changes.take(),
        vec![],
        "metadata changes nobody's access"
    );

    let teamspace = |teamspace_id, user_id| AccessChange::Teamspace {
        workspace_id: ws,
        teamspace_id,
        user_id,
    };
    let w = &t.h.workspaces;
    let dee = t.dee.user.id;
    w.join_teamspace(&t.dee, ws, t.open).await.expect("joined");
    w.change_teamspace_role(&t.cy, ws, t.open, dee, TeamspaceRole::Owner)
        .await
        .expect("changed");
    w.leave_teamspace(&t.dee, ws, t.open).await.expect("left");
    w.add_teamspace_member(&t.cy, ws, t.closed, dee, TeamspaceRole::Member)
        .await
        .expect("added");
    w.remove_teamspace_member(&t.cy, ws, t.closed, dee)
        .await
        .expect("removed");
    w.update_teamspace(
        &t.cy,
        ws,
        t.open,
        TeamspaceUpdate {
            name: Some("Renamed".to_owned()),
            ..TeamspaceUpdate::default()
        },
    )
    .await
    .expect("renamed");
    w.update_teamspace(
        &t.cy,
        ws,
        t.open,
        TeamspaceUpdate {
            access: Some(TeamspaceAccess::Private),
            ..TeamspaceUpdate::default()
        },
    )
    .await
    .expect("made private");
    w.delete_teamspace(&t.cy, ws, t.closed)
        .await
        .expect("deleted");
    assert_eq!(
        t.h.changes.take(),
        vec![
            teamspace(t.open, Some(dee)),
            teamspace(t.open, Some(dee)),
            teamspace(t.open, Some(dee)),
            teamspace(t.closed, Some(dee)),
            teamspace(t.closed, Some(dee)),
            teamspace(t.open, None),
            teamspace(t.closed, None),
        ],
        "a new name changes nobody's access"
    );

    let member = |user_id: Option<Uuid>| AccessChange::Workspace {
        workspace_id: ws,
        user_id,
    };
    w.change_member_role(&t.ada, ws, dee, WorkspaceRole::Guest)
        .await
        .expect("changed");
    w.remove_member(&t.ada, ws, dee).await.expect("removed");
    w.leave_workspace(&t.bob, ws).await.expect("left");
    w.delete_workspace(&t.ada, ws, "Design")
        .await
        .expect("deleted");
    assert_eq!(
        t.h.changes.take(),
        vec![
            member(Some(dee)),
            member(Some(dee)),
            member(Some(t.bob.user.id)),
            member(None),
        ]
    );
}

fn count(snapshotter: &Snapshotter, operation: &str, status: &str) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            match (kind, value) {
                (MetricKind::Counter, DebugValue::Counter(n))
                    if key.name() == "neon_pages_operations_total"
                        && label("operation").as_deref() == Some(operation)
                        && label("status").as_deref() == Some(status) =>
                {
                    Some(n)
                }
                _ => None,
            }
        })
        .sum()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refusals_count_as_ok_and_failures_as_errors(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let t = team(pool).await;
    let page = t.page(&t.cy, Location::Private, "Mine").await;

    assert_not_found(t.access(&t.dee, page).await);
    t.access(&t.cy, page).await.expect("seen");
    assert_eq!(count(&snapshotter, "get_page", "ok"), 2);

    t.h.pool.close().await;
    let failed = t.access(&t.cy, page).await;
    assert!(matches!(failed, Err(PageError::Database(_))), "{failed:?}");
    assert_eq!(count(&snapshotter, "get_page", "error"), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn recent_pages_are_those_the_caller_sees_the_latest_first(pool: PgPool) {
    let t = team(pool).await;
    let older = t
        .page(&t.ada, Location::Teamspace(t.general), "Older")
        .await;
    t.h.time.advance(Duration::from_secs(1));
    let secret = t
        .page(&t.cy, Location::Teamspace(t.secret), "Secret plan")
        .await;
    t.h.time.advance(Duration::from_secs(1));
    let newer = t
        .page(&t.ada, Location::Teamspace(t.general), "Newer")
        .await;
    t.h.time.advance(Duration::from_secs(1));
    let mine = t.page(&t.dee, Location::Private, "Dee's own").await;

    let ids = |list: Vec<PageView>| {
        list.into_iter()
            .map(|view| view.page.id)
            .collect::<Vec<_>>()
    };
    let recent =
        t.h.pages
            .recent_pages(&t.dee, t.id, 10)
            .await
            .expect("listed");
    assert_eq!(ids(recent), [mine, newer, older]);
    let cy =
        t.h.pages
            .recent_pages(&t.cy, t.id, 2)
            .await
            .expect("listed");
    assert_eq!(ids(cy), [newer, secret]);
    // Changed again, a page comes first; trashed, it goes.
    t.h.time.advance(Duration::from_secs(1));
    t.h.pages
        .update_page(
            &t.ada,
            older,
            PageUpdate {
                title: Some("Older, renamed".to_owned()),
                ..PageUpdate::default()
            },
        )
        .await
        .expect("renamed");
    t.h.pages.trash_page(&t.ada, newer).await.expect("trashed");
    let recent =
        t.h.pages
            .recent_pages(&t.dee, t.id, 10)
            .await
            .expect("listed");
    assert_eq!(ids(recent), [older, mine]);
    assert_not_found(t.h.pages.recent_pages(&t.eve, t.id, 10).await);
}
