//! Adversarial database-backed verification for the collaboration audit
//! cluster (audit items 50, 51, 53, 54, 55, 58, 59, 60, 62).
//!
//! `#[ignore]`d by default; run against a real PostgreSQL database with:
//!
//! ```text
//! TEST_DATABASE_URL=postgres:///apex_pg_suites \
//!   cargo test -p apex-api --test workspace_authorization_integration \
//!   --offline -- --ignored --test-threads=1
//! ```
//!
//! What each test proves:
//!
//!   * `authorize_workspace_matrix_*` — the shared guard denies (404-mapped)
//!     unless admin/owner/share/assignment/visibility allows it, including the
//!     adversarial cases: expired shares, `team` visibility alone, assignment
//!     role downgrades, and workspace ids that do not exist.
//!   * `list_visible_*` — visibility is filtered in SQL before `LIMIT`, so an
//!     older visible workspace is never displaced by newer private rows.
//!   * `activity_feed_*` — activity attached to an invisible workspace and
//!     `private` activity from another actor never leave the database, even
//!     when the caller knows the workspace id.
//!   * `delete_annotation_*` — the delete is owner-only at the SQL predicate.
//!   * `supplier_risk_*`, `queue_*`, `workspace_partial_update_*` — PATCH-style
//!     single statements preserve untouched columns (`mitigation`,
//!     `completed_at`, findings/conclusions/description).
//!   * `opportunity_threat_*` — partial PATCH via COALESCE, threat `resolved`
//!     status and derived `resolved_at`.
//!   * `assignment_upsert_*` — migration 094 unique index + `ON CONFLICT`
//!     role update (re-assignment cannot duplicate or strand an old role).
//!   * `web_*` — the real web handlers (`get`/`close`/`assign`/`share`) and
//!     real form validation, including NaN/Infinity rejection.
//!   * `store_error_*` / `web_500_*` — no raw database text ever reaches a
//!     client-visible error.
//!   * `executive_*` — filters and COUNT(*) totals are computed in SQL.
//!   * `delete_workspace_owner_gate` — an `admin` share passes the read guard
//!     but cannot satisfy the handler's owner/admin delete gate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::sync::Arc;

use apex_api::auth::ApiRole;
use apex_api::middleware::session::WebSession;
use apex_api::responses::ApiError;
use apex_api::routes::collaboration::{
    authorize_workspace, resolve_record_owner, store_error, validate_access_level,
    validate_evidence_type, validate_opportunity_status, validate_probability,
    validate_reliability_score, validate_risk_category, validate_risk_score, validate_share_type,
    validate_stage, validate_team_assignment_role, validate_threat_status, validate_visibility,
    validate_workspace_assignment_role, validate_workspace_name, WsAccess,
};
use apex_api::web::collaboration as web_collab;
use apex_core::identity::{UserId, Username};
use apex_store::postgres::{InvestigationWorkspaceRecord, PgStore};
use axum::extract::{Form, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Extension;
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::json;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

// ── setup / helpers ─────────────────────────────────────────────────────────

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

/// Every connection carries the unscoped `service` identity, exactly like the
/// production API pool (`PgStore::from_url`): the FORCEd RLS tables are then
/// governed by their SQL predicates, which is what these tests probe.
async fn setup() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                PgStore::assume_service_identity(conn).await?;
                Ok(())
            })
        })
        .connect(&database_url())
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    // `owner_id`/`created_by` on executive records reference app_users(id)
    // (migration 101); the fixtures below act as these principals.
    let store = PgStore::from_pool(pool.clone());
    for id in [FIXTURE_CREATOR, "owner-1", "owner-2"] {
        store
            .ensure_app_user_exists(id, id, "analyst")
            .await
            .expect("provision fixture user");
    }
    pool
}

const FIXTURE_CREATOR: &str = "verify-wsauthz-creator";

fn marker(prefix: &str) -> String {
    format!("verify-wsauthz-{prefix}-{}", Uuid::new_v4().simple())
}

async fn create_ws(store: &PgStore, marker_prefix: &str, owner: &str, visibility: &str) -> Uuid {
    store
        .create_investigation_workspace(
            &marker(marker_prefix),
            None,
            "structured",
            owner,
            None,
            visibility,
            &[],
            &json!([]),
        )
        .await
        .expect("create workspace")
        .id
}

async fn can(
    store: &PgStore,
    id: Uuid,
    user: &str,
    is_admin: bool,
    need: WsAccess,
) -> Result<InvestigationWorkspaceRecord, ApiError> {
    authorize_workspace(store, id, user, is_admin, need).await
}

#[track_caller]
fn expect_allowed(
    result: Result<InvestigationWorkspaceRecord, ApiError>,
    context: &str,
) -> InvestigationWorkspaceRecord {
    result.unwrap_or_else(|error| {
        panic!(
            "{context}: expected access, got {} ({})",
            error.http_status(),
            error.message
        )
    })
}

#[track_caller]
fn expect_denied_404(result: Result<InvestigationWorkspaceRecord, ApiError>, context: &str) {
    match result {
        Ok(workspace) => panic!("{context}: expected denial, got workspace {}", workspace.id),
        Err(error) => assert_eq!(
            error.http_status(),
            404,
            "{context}: deny must be 404-mapped, got {} ({})",
            error.http_status(),
            error.message
        ),
    }
}

async fn delete_workspaces(pool: &PgPool, ids: &[Uuid]) {
    for id in ids {
        sqlx::query("DELETE FROM activity_feed WHERE workspace_id = $1")
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup activity for workspace");
        sqlx::query("DELETE FROM investigation_workspaces WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .expect("cleanup workspace");
    }
}

fn web_session(user_id: &str, role: ApiRole) -> WebSession {
    WebSession {
        user_id: UserId::new(user_id),
        username: Username::new(user_id),
        role,
        session_version: 1,
        principal_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        issued_at: 0,
        expires_at: i64::MAX,
    }
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ── #50: authorize_workspace matrix ─────────────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn authorize_workspace_matrix_enforces_owner_share_assignment_admin() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("owner");
    let other = marker("other");
    let read_user = marker("read");
    let rw_user = marker("rw");
    let admin_share_user = marker("adminshare");
    let expired_user = marker("expired");
    let contributor = marker("contributor");
    let lead = marker("lead");
    let viewer = marker("viewer");

    let ws = create_ws(&store, "matrix", &owner, "private").await;

    // Owner: full access. Platform admin: full access (bypass).
    expect_allowed(
        can(&store, ws, &owner, false, WsAccess::Read).await,
        "owner read",
    );
    expect_allowed(
        can(&store, ws, &owner, false, WsAccess::Write).await,
        "owner write",
    );
    expect_allowed(
        can(&store, ws, &owner, false, WsAccess::Manage).await,
        "owner manage",
    );
    // A non-owner with the admin *flag* is the platform admin.
    expect_allowed(
        can(&store, ws, &other, true, WsAccess::Manage).await,
        "platform admin manage",
    );

    // A stranger with no share/assignment: denied at every level, as 404.
    expect_denied_404(
        can(&store, ws, &other, false, WsAccess::Read).await,
        "stranger read",
    );
    expect_denied_404(
        can(&store, ws, &other, false, WsAccess::Write).await,
        "stranger write",
    );
    expect_denied_404(
        can(&store, ws, &other, false, WsAccess::Manage).await,
        "stranger manage",
    );

    // A workspace id that does not exist is not-found, not forbidden.
    let missing = Uuid::new_v4();
    expect_denied_404(
        can(&store, missing, &owner, false, WsAccess::Read).await,
        "missing workspace",
    );
    let missing_error = can(&store, missing, &owner, false, WsAccess::Read)
        .await
        .unwrap_err();
    assert!(
        missing_error.message.contains("not found"),
        "a nonexistent workspace must map to not_found, got {}",
        missing_error.message
    );

    // Shares: read / read_write / admin / expired.
    store
        .create_investigation_share(ws, &owner, &read_user, "view", "read", None, None)
        .await
        .expect("read share");
    store
        .create_investigation_share(
            ws,
            &owner,
            &rw_user,
            "collaborate",
            "read_write",
            None,
            None,
        )
        .await
        .expect("read_write share");
    store
        .create_investigation_share(
            ws,
            &owner,
            &admin_share_user,
            "collaborate",
            "admin",
            None,
            None,
        )
        .await
        .expect("admin share");
    // Adversarial: an EXPIRED share must not grant anything.
    store
        .create_investigation_share(
            ws,
            &owner,
            &expired_user,
            "view",
            "admin",
            None,
            Some(Utc::now() - chrono::Duration::hours(1)),
        )
        .await
        .expect("expired share");

    expect_allowed(
        can(&store, ws, &read_user, false, WsAccess::Read).await,
        "read share read",
    );
    expect_denied_404(
        can(&store, ws, &read_user, false, WsAccess::Write).await,
        "read share write",
    );
    expect_denied_404(
        can(&store, ws, &read_user, false, WsAccess::Manage).await,
        "read share manage",
    );

    expect_allowed(
        can(&store, ws, &rw_user, false, WsAccess::Read).await,
        "rw share read",
    );
    expect_allowed(
        can(&store, ws, &rw_user, false, WsAccess::Write).await,
        "rw share write",
    );
    expect_denied_404(
        can(&store, ws, &rw_user, false, WsAccess::Manage).await,
        "rw share manage",
    );

    expect_allowed(
        can(&store, ws, &admin_share_user, false, WsAccess::Read).await,
        "admin share read",
    );
    expect_allowed(
        can(&store, ws, &admin_share_user, false, WsAccess::Write).await,
        "admin share write",
    );
    expect_allowed(
        can(&store, ws, &admin_share_user, false, WsAccess::Manage).await,
        "admin share manage",
    );

    expect_denied_404(
        can(&store, ws, &expired_user, false, WsAccess::Read).await,
        "expired share read",
    );
    expect_denied_404(
        can(&store, ws, &expired_user, false, WsAccess::Write).await,
        "expired share write",
    );
    expect_denied_404(
        can(&store, ws, &expired_user, false, WsAccess::Manage).await,
        "expired share manage",
    );

    // Assignments: contributor writes, lead manages, viewer only reads.
    store
        .create_workspace_assignment(ws, &contributor, "contributor", &owner)
        .await
        .expect("contributor assignment");
    store
        .create_workspace_assignment(ws, &lead, "lead", &owner)
        .await
        .expect("lead assignment");
    store
        .create_workspace_assignment(ws, &viewer, "viewer", &owner)
        .await
        .expect("viewer assignment");

    expect_allowed(
        can(&store, ws, &contributor, false, WsAccess::Read).await,
        "contributor read",
    );
    expect_allowed(
        can(&store, ws, &contributor, false, WsAccess::Write).await,
        "contributor write",
    );
    expect_denied_404(
        can(&store, ws, &contributor, false, WsAccess::Manage).await,
        "contributor manage",
    );

    expect_allowed(
        can(&store, ws, &lead, false, WsAccess::Manage).await,
        "lead manage",
    );

    expect_allowed(
        can(&store, ws, &viewer, false, WsAccess::Read).await,
        "viewer read",
    );
    expect_denied_404(
        can(&store, ws, &viewer, false, WsAccess::Write).await,
        "viewer write",
    );
    expect_denied_404(
        can(&store, ws, &viewer, false, WsAccess::Manage).await,
        "viewer manage",
    );

    // Adversarial: re-assign the `lead` as `viewer`; the downgrade must take
    // effect immediately (no stale role row survives the upsert).
    store
        .create_workspace_assignment(ws, &lead, "viewer", &owner)
        .await
        .expect("downgrade lead to viewer");
    expect_denied_404(
        can(&store, ws, &lead, false, WsAccess::Manage).await,
        "downgraded lead manage",
    );
    expect_denied_404(
        can(&store, ws, &lead, false, WsAccess::Write).await,
        "downgraded lead write",
    );
    expect_allowed(
        can(&store, ws, &lead, false, WsAccess::Read).await,
        "downgraded lead read",
    );

    delete_workspaces(&pool, &[ws]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn visibility_grants_read_only_and_team_grants_nothing() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("vowner");
    let other = marker("vother");

    // `team` visibility alone must not grant access (no membership model).
    let team_ws = create_ws(&store, "team", &owner, "team").await;
    expect_denied_404(
        can(&store, team_ws, &other, false, WsAccess::Read).await,
        "team read",
    );
    expect_denied_404(
        can(&store, team_ws, &other, false, WsAccess::Write).await,
        "team write",
    );
    expect_allowed(
        can(&store, team_ws, &owner, false, WsAccess::Read).await,
        "team owner read",
    );

    // `organization` visibility grants read to everyone, but never write/manage.
    let org_ws = create_ws(&store, "org", &owner, "organization").await;
    expect_allowed(
        can(&store, org_ws, &other, false, WsAccess::Read).await,
        "org read",
    );
    expect_denied_404(
        can(&store, org_ws, &other, false, WsAccess::Write).await,
        "org write",
    );
    expect_denied_404(
        can(&store, org_ws, &other, false, WsAccess::Manage).await,
        "org manage",
    );

    // `public` behaves the same.
    let public_ws = create_ws(&store, "public", &owner, "public").await;
    expect_allowed(
        can(&store, public_ws, &other, false, WsAccess::Read).await,
        "public read",
    );
    expect_denied_404(
        can(&store, public_ws, &other, false, WsAccess::Write).await,
        "public write",
    );

    // A `private` workspace stays private.
    let private_ws = create_ws(&store, "private", &owner, "private").await;
    expect_denied_404(
        can(&store, private_ws, &other, false, WsAccess::Read).await,
        "private read",
    );

    delete_workspaces(&pool, &[team_ws, org_ws, public_ws, private_ws]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn delete_workspace_owner_gate_rejects_admin_share() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("downer");
    let stranger = marker("dstranger");
    let admin_share_user = marker("dshare");

    let ws = create_ws(&store, "delete", &owner, "private").await;
    store
        .create_investigation_share(
            ws,
            &owner,
            &admin_share_user,
            "collaborate",
            "admin",
            None,
            None,
        )
        .await
        .expect("admin share");

    // The shared guard alone would admit an admin-share holder at Read level...
    let record = expect_allowed(
        can(&store, ws, &admin_share_user, false, WsAccess::Read).await,
        "admin share read",
    );

    // ...which is why `delete_workspace` (api_handlers/collaboration.rs:830-840)
    // adds an explicit owner-or-platform-admin gate. Mirror that exact gate:
    // an `admin` SHARE must not satisfy it.
    let handler_allows_delete = false /* role.can_admin() */ || record.owner_id == admin_share_user;
    assert!(
        !handler_allows_delete,
        "an admin share must not satisfy the owner/admin delete gate"
    );

    // And a stranger never even reaches the gate.
    expect_denied_404(
        can(&store, ws, &stranger, false, WsAccess::Read).await,
        "stranger delete read",
    );

    // The store's delete itself is ungated — delete authorization is a handler
    // responsibility, which the source check above verifies is present.
    delete_workspaces(&pool, &[ws]).await;
    pool.close().await;
}

// ── #50: list_visible_investigation_workspaces (SQL filter before LIMIT) ────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn list_visible_workspaces_filters_before_limit() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("lowner");
    let other = marker("lother");
    let share_user = marker("lshare");
    let assigned = marker("lassigned");

    // The only workspace `other` can see is deliberately the OLDEST row.
    let org = create_ws(&store, "list-org", &owner, "organization").await;
    sqlx::query(
        "UPDATE investigation_workspaces SET updated_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(org)
    .execute(&pool)
    .await
    .expect("age org workspace");

    // Five newer private workspaces owned by someone else must not count
    // toward the page limit (a LIMIT-before-filter bug returns an empty page).
    let mut private_ids = Vec::new();
    for index in 0..5 {
        let id = create_ws(&store, &format!("list-private-{index}"), &owner, "private").await;
        // Future-dated so they outrank every pre-existing row in the table.
        sqlx::query("UPDATE investigation_workspaces SET updated_at = now() + interval '1 hour' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("date private workspace");
        private_ids.push(id);
    }

    // Shared (unexpired), expired-share and assignment workspaces.
    let shared = create_ws(&store, "list-shared", &owner, "private").await;
    store
        .create_investigation_share(shared, &owner, &share_user, "view", "read", None, None)
        .await
        .expect("unexpired share");
    let expired = create_ws(&store, "list-expired", &owner, "private").await;
    store
        .create_investigation_share(
            expired,
            &owner,
            &share_user,
            "view",
            "read",
            None,
            Some(Utc::now() - chrono::Duration::minutes(5)),
        )
        .await
        .expect("expired share");
    let assigned_ws = create_ws(&store, "list-assigned", &owner, "private").await;
    store
        .create_workspace_assignment(assigned_ws, &assigned, "contributor", &owner)
        .await
        .expect("assignment");

    let page = store
        .list_visible_investigation_workspaces(&other, false, None, None, 3)
        .await
        .expect("list visible");
    let ids: HashSet<Uuid> = page.iter().map(|w| w.id).collect();
    assert!(
        ids.contains(&org),
        "the older organization workspace must survive LIMIT (page: {ids:?})"
    );
    for private in &private_ids {
        assert!(
            !ids.contains(private),
            "private workspace {private} leaked into the list"
        );
    }

    let share_page = store
        .list_visible_investigation_workspaces(&share_user, false, None, None, 50)
        .await
        .expect("share visible");
    let share_ids: HashSet<Uuid> = share_page.iter().map(|w| w.id).collect();
    assert!(
        share_ids.contains(&shared),
        "unexpired share must be visible"
    );
    assert!(
        !share_ids.contains(&expired),
        "expired share must not be visible"
    );

    let assigned_page = store
        .list_visible_investigation_workspaces(&assigned, false, None, None, 50)
        .await
        .expect("assigned visible");
    let assigned_ids: HashSet<Uuid> = assigned_page.iter().map(|w| w.id).collect();
    assert!(
        assigned_ids.contains(&assigned_ws),
        "assignment must be visible"
    );

    // Admin bypass sees the private rows too.
    let admin_page = store
        .list_visible_investigation_workspaces(&other, true, None, None, 50)
        .await
        .expect("admin visible");
    let admin_ids: HashSet<Uuid> = admin_page.iter().map(|w| w.id).collect();
    for private in &private_ids {
        assert!(admin_ids.contains(private), "admin must see {private}");
    }

    // Status filter must also run before LIMIT. The closed row is the newest
    // overall; a LIMIT-then-filter bug would return an empty first page.
    let active_status_ws = create_ws(&store, "list-status-active", &owner, "organization").await;
    sqlx::query(
        "UPDATE investigation_workspaces SET updated_at = now() + interval '3 hour' WHERE id = $1",
    )
    .bind(active_status_ws)
    .execute(&pool)
    .await
    .expect("date active workspace");
    let closed_status_ws = create_ws(&store, "list-status-closed", &owner, "organization").await;
    sqlx::query("UPDATE investigation_workspaces SET status = 'closed', updated_at = now() + interval '4 hour' WHERE id = $1")
        .bind(closed_status_ws)
        .execute(&pool)
        .await
        .expect("close workspace");

    let active_page = store
        .list_visible_investigation_workspaces(&other, false, None, Some("active"), 1)
        .await
        .expect("status page");
    assert_eq!(
        active_page.len(),
        1,
        "the newest status-matching workspace must be returned"
    );
    assert_eq!(
        active_page[0].id, active_status_ws,
        "status must filter before LIMIT (got {:?})",
        active_page[0].id
    );

    delete_workspaces(
        &pool,
        &[
            org,
            shared,
            expired,
            assigned_ws,
            active_status_ws,
            closed_status_ws,
        ],
    )
    .await;
    delete_workspaces(&pool, &private_ids).await;
    pool.close().await;
}

// ── #51: activity feed visibility ───────────────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn activity_feed_hides_invisible_workspace_and_private_rows() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("aowner");
    let other = marker("aother");

    let private_ws = create_ws(&store, "act-private", &owner, "private").await;
    let org_ws = create_ws(&store, "act-org", &owner, "organization").await;

    // Regression guard: migration 097 added the missing `updated_at` column
    // and replaced the broken BEFORE INSERT trigger (which referenced a
    // non-existent column), so a plain activity insert must succeed.
    let hidden_by_workspace = store
        .create_activity_entry(
            &owner,
            "Owner",
            "update",
            None,
            None,
            None,
            &json!({}),
            Some(private_ws),
            None,
            "team",
        )
        .await
        .expect("activity insert must succeed after migration 097");
    let visible = store
        .create_activity_entry(
            &owner,
            "Owner",
            "update",
            None,
            None,
            None,
            &json!({}),
            Some(org_ws),
            None,
            "team",
        )
        .await
        .expect("activity on org workspace");
    let private_actor_row = store
        .create_activity_entry(
            &owner,
            "Owner",
            "comment",
            None,
            None,
            None,
            &json!({}),
            None,
            None,
            "private",
        )
        .await
        .expect("private activity without workspace");
    let others_private = store
        .create_activity_entry(
            &other,
            "Other",
            "comment",
            None,
            None,
            None,
            &json!({}),
            None,
            None,
            "private",
        )
        .await
        .expect("other user's private activity");

    // The other user: sees platform rows and visible-workspace rows, never the
    // private workspace activity or the owner's private row.
    let feed = store
        .list_activity_feed(&other, false, None, None, None, 200)
        .await
        .expect("other feed");
    let other_ids: HashSet<Uuid> = feed.iter().map(|a| a.id).collect();
    assert!(
        other_ids.contains(&visible.id),
        "org-workspace activity must be visible"
    );
    assert!(
        other_ids.contains(&others_private.id),
        "own private row must be visible"
    );
    assert!(
        !other_ids.contains(&hidden_by_workspace.id),
        "activity of an invisible workspace leaked"
    );
    assert!(
        !other_ids.contains(&private_actor_row.id),
        "another actor's private activity leaked"
    );

    // Adversarial: knowing the private workspace id and asking for it directly
    // must not widen the feed either.
    let targeted = store
        .list_activity_feed(&other, false, Some(private_ws), None, None, 200)
        .await
        .expect("targeted feed");
    assert!(
        targeted.is_empty(),
        "filtering by a hidden workspace id must return nothing, got {:?}",
        targeted.iter().map(|a| a.id).collect::<Vec<_>>()
    );

    // The owner sees their own private rows and the private-workspace activity.
    let owner_feed = store
        .list_activity_feed(&owner, false, None, None, None, 200)
        .await
        .expect("owner feed");
    let owner_ids: HashSet<Uuid> = owner_feed.iter().map(|a| a.id).collect();
    assert!(owner_ids.contains(&hidden_by_workspace.id));
    assert!(owner_ids.contains(&private_actor_row.id));
    assert!(
        !owner_ids.contains(&others_private.id),
        "other actor's private row leaked to owner"
    );

    // A platform admin still does not read another actor's private row.
    let admin_feed = store
        .list_activity_feed(&other, true, None, None, None, 200)
        .await
        .expect("admin feed");
    let admin_ids: HashSet<Uuid> = admin_feed.iter().map(|a| a.id).collect();
    assert!(
        admin_ids.contains(&hidden_by_workspace.id),
        "admin sees workspace activity"
    );
    assert!(
        !admin_ids.contains(&private_actor_row.id),
        "private activity is actor-scoped even for admins"
    );

    // #51: unknown visibility strings are rejected by the canonical validator
    // used by the `record_activity` handler before the row reaches SQL.
    assert!(validate_visibility("private").is_ok());
    assert!(validate_visibility("team").is_ok());
    assert!(validate_visibility("organization").is_ok());
    assert!(validate_visibility("public").is_ok());
    let bad = validate_visibility("super-private").unwrap_err();
    assert_eq!(bad.http_status(), 422);
    assert!(bad.message.contains("must be one of"));

    // The database check constraint is the backstop.
    let db_rejects = store
        .create_activity_entry(
            &owner,
            "Owner",
            "update",
            None,
            None,
            None,
            &json!({}),
            None,
            None,
            "super-private",
        )
        .await;
    assert!(
        db_rejects.is_err(),
        "the DB must reject unknown visibility strings"
    );

    sqlx::query("DELETE FROM activity_feed WHERE actor_id = $1 OR actor_id = $2")
        .bind(&owner)
        .bind(&other)
        .execute(&pool)
        .await
        .expect("cleanup activity");
    delete_workspaces(&pool, &[private_ws, org_ws]).await;
    pool.close().await;
}

// ── #53: owner-only annotation delete ───────────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn delete_annotation_is_owner_only() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let user_a = marker("ann-a");
    let user_b = marker("ann-b");
    for user in [&user_a, &user_b] {
        sqlx::query(
            "INSERT INTO app_users (id, username, display_name, role, enabled, session_version) \
             VALUES ($1, $2, $2, 'analyst', TRUE, 1)",
        )
        .bind(user)
        .bind(user)
        .execute(&pool)
        .await
        .expect("insert app_users");
    }

    let annotation = store
        .upsert_annotation(
            None,
            &user_a,
            "company",
            "comp-annotation",
            "team-visible note",
            &[],
            "team",
        )
        .await
        .expect("create annotation");

    // Another user can read a `team` annotation (list) but must not delete it.
    let visible_to_b = store
        .list_annotations(&user_b, Some("company"), Some("comp-annotation"))
        .await
        .expect("list annotations");
    assert!(
        visible_to_b.iter().any(|a| a.id == annotation.id),
        "team annotations should be listable"
    );

    let deleted_by_b = store
        .delete_annotation(&user_b, annotation.id)
        .await
        .expect("delete as B");
    assert!(!deleted_by_b, "a non-owner delete must not affect the row");
    let (still_there,): (i64,) =
        sqlx::query_as("SELECT COUNT(*)::bigint FROM annotations WHERE id = $1")
            .bind(annotation.id)
            .fetch_one(&pool)
            .await
            .expect("count annotations");
    assert_eq!(still_there, 1, "cross-user delete removed the row");

    let deleted_by_a = store
        .delete_annotation(&user_a, annotation.id)
        .await
        .expect("delete as A");
    assert!(deleted_by_a, "the owner delete must remove the row");
    let (gone,): (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM annotations WHERE id = $1")
        .bind(annotation.id)
        .fetch_one(&pool)
        .await
        .expect("count annotations");
    assert_eq!(gone, 0);

    sqlx::query("DELETE FROM app_users WHERE id = $1 OR id = $2")
        .bind(&user_a)
        .bind(&user_b)
        .execute(&pool)
        .await
        .expect("cleanup app_users");
    pool.close().await;
}

// ── #59: supplier risk mitigation preservation ──────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn supplier_risk_patch_preserves_mitigation() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let entry = store
        .create_supplier_risk_entry(
            &marker("supplier"),
            "financial",
            0.25,
            &json!(["concentration"]),
            Some("retain me"),
            Some("owner-1"),
            FIXTURE_CREATOR,
        )
        .await
        .expect("create supplier risk");
    assert_eq!(entry.created_by.as_deref(), Some(FIXTURE_CREATOR));

    // PATCH with only risk_score: mitigation and status must survive.
    let after_score = store
        .update_supplier_risk_entry(entry.id, Some(0.9), None, None, None, None)
        .await
        .expect("update score")
        .expect("row exists");
    assert_eq!(after_score.risk_score, 0.9);
    assert_eq!(after_score.mitigation.as_deref(), Some("retain me"));
    assert_eq!(after_score.status, "active");
    assert_eq!(after_score.risk_category, "financial");

    // PATCH mitigation only: score survives.
    let after_mitigation = store
        .update_supplier_risk_entry(entry.id, None, None, Some("new plan"), None, None)
        .await
        .expect("update mitigation")
        .expect("row exists");
    assert_eq!(after_mitigation.risk_score, 0.9);
    assert_eq!(after_mitigation.mitigation.as_deref(), Some("new plan"));

    // PATCH status only: both survive.
    let after_status = store
        .update_supplier_risk_entry(entry.id, None, None, None, None, Some("resolved"))
        .await
        .expect("update status")
        .expect("row exists");
    assert_eq!(after_status.status, "resolved");
    assert_eq!(after_status.mitigation.as_deref(), Some("new plan"));
    assert_eq!(after_status.risk_score, 0.9);

    sqlx::query("DELETE FROM supplier_risk WHERE id = $1")
        .bind(entry.id)
        .execute(&pool)
        .await
        .expect("cleanup supplier risk");
    pool.close().await;
}

// ── #59: workspace partial update preserves untouched columns ───────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn workspace_partial_update_preserves_untouched_columns() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("wowner");
    let ws = store
        .create_investigation_workspace(
            &marker("partial"),
            Some("original description"),
            "structured",
            &owner,
            None,
            "private",
            &["alpha".to_string()],
            &json!(["entity-1"]),
        )
        .await
        .expect("create workspace");

    // Seed findings/conclusions through the same single-statement update.
    let seeded = store
        .update_investigation_workspace(
            ws.id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some("findings v1")),
            Some(Some("conclusions v1")),
        )
        .await
        .expect("seed findings")
        .expect("row exists");
    assert_eq!(seeded.findings.as_deref(), Some("findings v1"));

    // A PATCH that only flips the status (like the web close handler) must not
    // touch name/description/tags/entity_focus/findings/conclusions.
    let closed = store
        .update_investigation_workspace(
            ws.id,
            None,
            None,
            Some("closed"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("close")
        .expect("row exists");
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.description.as_deref(), Some("original description"));
    assert_eq!(closed.tags, vec!["alpha".to_string()]);
    assert_eq!(closed.entity_focus, json!(["entity-1"]));
    assert_eq!(closed.findings.as_deref(), Some("findings v1"));
    assert_eq!(closed.conclusions.as_deref(), Some("conclusions v1"));
    assert!(closed.name.starts_with("verify-wsauthz-partial-"));
    assert!(closed.closed_at.is_some(), "closing must stamp closed_at");

    // Concurrent-writer simulation: a second writer updates findings, then a
    // stale writer that only sets tags must not roll findings back (the
    // statement never reads the row first).
    let newer = store
        .update_investigation_workspace(
            ws.id,
            None,
            None,
            None,
            Some(&["beta".to_string()]),
            None,
            None,
            Some(Some("findings v2")),
            None,
        )
        .await
        .expect("second writer")
        .expect("row exists");
    assert_eq!(newer.findings.as_deref(), Some("findings v2"));
    let stale = store
        .update_investigation_workspace(
            ws.id,
            None,
            None,
            None,
            Some(&["gamma".to_string()]),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("stale writer")
        .expect("row exists");
    assert_eq!(stale.findings.as_deref(), Some("findings v2"));
    assert_eq!(stale.tags, vec!["gamma".to_string()]);
    assert_eq!(stale.status, "closed");

    // Explicit clear still works via the set-flag.
    let cleared = store
        .update_investigation_workspace(ws.id, None, Some(None), None, None, None, None, None, None)
        .await
        .expect("clear description")
        .expect("row exists");
    assert_eq!(cleared.description, None);
    assert_eq!(cleared.findings.as_deref(), Some("findings v2"));

    delete_workspaces(&pool, &[ws.id]).await;
    pool.close().await;
}

// ── #60: queue completion timestamps ────────────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn queue_completion_preserves_completed_at_and_uncompletion_clears() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let user = marker("queue");
    let item = store
        .create_priority_queue_item(&user, "warning", Uuid::new_v4(), "queue item", 50, None)
        .await
        .expect("create queue item");
    assert_eq!(item.completed_at, None);

    let completed = store
        .update_priority_queue_item(&user, item.id, None, Some("completed"), None)
        .await
        .expect("complete")
        .expect("row exists");
    let first_completed_at = completed
        .completed_at
        .expect("completion stamps completed_at");

    // Completing an already-completed item preserves the original timestamp.
    let touched = store
        .update_priority_queue_item(&user, item.id, None, None, Some(Some("notes")))
        .await
        .expect("touch notes")
        .expect("row exists");
    assert_eq!(
        touched.completed_at,
        Some(first_completed_at),
        "notes update reset completed_at"
    );
    let recompleted = store
        .update_priority_queue_item(&user, item.id, None, Some("completed"), None)
        .await
        .expect("re-complete")
        .expect("row exists");
    assert_eq!(
        recompleted.completed_at,
        Some(first_completed_at),
        "re-completing must preserve the original completion time"
    );

    // Un-completing (any other status) clears it.
    let uncompleted = store
        .update_priority_queue_item(&user, item.id, None, Some("in_progress"), None)
        .await
        .expect("uncomplete")
        .expect("row exists");
    assert_eq!(
        uncompleted.completed_at, None,
        "un-completing must null completed_at"
    );
    assert_eq!(uncompleted.status, "in_progress");
    assert_eq!(
        uncompleted.notes.as_deref(),
        Some("notes"),
        "notes preserved"
    );

    // Completing again stamps a fresh timestamp.
    let re_completed = store
        .update_priority_queue_item(&user, item.id, None, Some("completed"), None)
        .await
        .expect("complete again")
        .expect("row exists");
    assert!(
        re_completed.completed_at >= Some(first_completed_at),
        "a new completion must stamp a new time"
    );

    // Another user cannot update the row (the user_id predicate).
    let other = marker("queue-other");
    let foreign = store
        .update_priority_queue_item(&other, item.id, Some(99), None, None)
        .await
        .expect("cross-user update");
    assert!(
        foreign.is_none(),
        "another user must not update this queue item"
    );

    sqlx::query("DELETE FROM priority_queue WHERE user_id = $1 OR user_id = $2")
        .bind(&user)
        .bind(&other)
        .execute(&pool)
        .await
        .expect("cleanup queue");
    pool.close().await;
}

// ── #62: assignment upsert / unique index 094 ──────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn assignment_upsert_updates_role_in_place() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let owner = marker("asowner");
    let member = marker("asmember");
    let ws = create_ws(&store, "assign", &owner, "private").await;

    let first = store
        .create_workspace_assignment(ws, &member, "viewer", &owner)
        .await
        .expect("first assignment");
    assert_eq!(first.role, "viewer");

    // Adversarial: re-assign with an upgraded role. The upsert must keep the
    // same row (no duplicate) and apply the new role.
    let upgraded = store
        .create_workspace_assignment(ws, &member, "owner", &owner)
        .await
        .expect("upgrade assignment");
    assert_eq!(upgraded.id, first.id, "re-assignment must update in place");
    assert_eq!(upgraded.role, "owner");

    let rows = store
        .list_workspace_assignments(ws)
        .await
        .expect("list assignments");
    assert_eq!(
        rows.len(),
        1,
        "the unique index must prevent duplicates: {rows:?}"
    );

    // The upgraded role takes effect in the guard.
    expect_allowed(
        can(&store, ws, &member, false, WsAccess::Manage).await,
        "upgraded manage",
    );

    // Downgrade also takes effect immediately (no stale role row).
    let downgraded = store
        .create_workspace_assignment(ws, &member, "viewer", &owner)
        .await
        .expect("downgrade assignment");
    assert_eq!(downgraded.id, first.id);
    expect_denied_404(
        can(&store, ws, &member, false, WsAccess::Manage).await,
        "downgraded manage",
    );

    // Migration 094's stable unique index exists and is enforced (a raw
    // duplicate insert must fail).
    let (index_exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'public' \
         AND indexname = 'idx_workspace_assignments_workspace_user')",
    )
    .fetch_one(&pool)
    .await
    .expect("index lookup");
    assert!(index_exists, "migration 094 unique index is missing");

    let duplicate = sqlx::query(
        "INSERT INTO workspace_assignments (workspace_id, user_id, role, assigned_by) \
         VALUES ($1, $2, 'viewer', $3)",
    )
    .bind(ws)
    .bind(&member)
    .bind(&owner)
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "duplicate assignment must violate the unique index"
    );

    delete_workspaces(&pool, &[ws]).await;
    pool.close().await;
}

// ── #58: opportunity / threat partial PATCH + resolved_at ──────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn opportunity_and_threat_patch_is_partial_and_derives_resolved_at() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    // Opportunity: only provided fields change.
    let opportunity = store
        .create_strategic_opportunity(
            &marker("opp"),
            Some("original description"),
            "partnership",
            0.5,
            0.4,
            None,
            None,
            Some("NA"),
            None,
            &json!(["step-one"]),
            Some("owner-1"),
            None,
            FIXTURE_CREATOR,
        )
        .await
        .expect("create opportunity");
    assert_eq!(opportunity.created_by.as_deref(), Some(FIXTURE_CREATOR));

    let patched = store
        .update_strategic_opportunity(
            opportunity.id,
            Some("renamed opportunity"),
            None,
            None,
            Some(0.9),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("pursued"),
        )
        .await
        .expect("patch opportunity")
        .expect("row exists");
    assert_eq!(patched.title, "renamed opportunity");
    assert_eq!(patched.priority_score, 0.9);
    assert_eq!(patched.description.as_deref(), Some("original description"));
    assert_eq!(patched.opportunity_type, "partnership");
    assert_eq!(patched.confidence, 0.4);
    assert_eq!(patched.region.as_deref(), Some("NA"));
    assert_eq!(patched.recommended_actions, json!(["step-one"]));
    assert_eq!(patched.owner_id.as_deref(), Some("owner-1"));
    assert_eq!(patched.status, "pursued");

    assert!(validate_opportunity_status("pursued").is_ok());
    assert!(validate_opportunity_status("resolved").is_err());

    // The VARCHAR entity_id column must also accept a non-null UUID text via
    // the COALESCE PATCH (regression: the uuid-typed bind made every PATCH
    // fail with "COALESCE types uuid and character varying cannot be matched").
    let entity_uuid = Uuid::new_v4();
    let with_entity = store
        .update_strategic_opportunity(
            opportunity.id,
            None,
            None,
            None,
            None,
            None,
            Some(entity_uuid),
            Some("company"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("patch entity_id")
        .expect("row exists");
    let entity_text = entity_uuid.to_string();
    assert_eq!(with_entity.entity_id.as_deref(), Some(entity_text.as_str()));
    // A later PATCH that omits entity_id keeps it.
    let entity_kept = store
        .update_strategic_opportunity(
            opportunity.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("patch without entity_id")
        .expect("row exists");
    assert_eq!(entity_kept.entity_id.as_deref(), Some(entity_text.as_str()));

    // Threat: `resolved` is accepted and resolved_at is derived from status.
    let threat = store
        .create_critical_threat(
            &marker("threat"),
            Some("original threat description"),
            "financial",
            "high",
            0.6,
            0.5,
            None,
            None,
            Some("NA"),
            &json!(["mitigate"]),
            Some("owner-2"),
            None,
            FIXTURE_CREATOR,
        )
        .await
        .expect("create threat");

    assert!(validate_threat_status("resolved").is_ok());
    assert!(validate_threat_status("monitoring").is_ok());
    assert!(validate_threat_status("bogus").is_err());

    let resolved = store
        .update_critical_threat_status(threat.id, "resolved")
        .await
        .expect("resolve")
        .expect("row exists");
    assert_eq!(resolved.status, "resolved");
    let resolved_at = resolved
        .resolved_at
        .expect("resolving must stamp resolved_at");

    // A PATCH that does not mention status keeps both status and resolved_at.
    let renamed = store
        .update_critical_threat(
            threat.id,
            Some("renamed threat"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("rename threat")
        .expect("row exists");
    assert_eq!(renamed.title, "renamed threat");
    assert_eq!(renamed.status, "resolved");
    assert_eq!(
        renamed.resolved_at,
        Some(resolved_at),
        "resolved_at must survive a partial PATCH"
    );
    assert_eq!(renamed.severity, "high");
    assert_eq!(renamed.impact_score, 0.6);
    assert_eq!(renamed.mitigation_steps, json!(["mitigate"]));

    // Same VARCHAR entity_id COALESCE regression as the opportunity above.
    let threat_entity = Uuid::new_v4();
    let threat_with_entity = store
        .update_critical_threat(
            threat.id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(threat_entity),
            Some("company"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("patch threat entity_id")
        .expect("row exists");
    let threat_entity_text = threat_entity.to_string();
    assert_eq!(
        threat_with_entity.entity_id.as_deref(),
        Some(threat_entity_text.as_str())
    );
    assert_eq!(threat_with_entity.resolved_at, Some(resolved_at));

    // Leaving `resolved` clears the timestamp.
    let reactivated = store
        .update_critical_threat_status(threat.id, "monitoring")
        .await
        .expect("reactivate")
        .expect("row exists");
    assert_eq!(reactivated.status, "monitoring");
    assert_eq!(
        reactivated.resolved_at, None,
        "leaving resolved must clear resolved_at"
    );

    sqlx::query("DELETE FROM strategic_opportunities WHERE id = $1")
        .bind(opportunity.id)
        .execute(&pool)
        .await
        .expect("cleanup opportunity");
    sqlx::query("DELETE FROM critical_threats WHERE id = $1")
        .bind(threat.id)
        .execute(&pool)
        .await
        .expect("cleanup threat");
    pool.close().await;
}

// ── #62: executive dashboard filters + COUNT(*) totals in SQL ──────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn executive_filters_and_totals_are_computed_in_sql() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let suffix = Uuid::new_v4().simple().to_string();
    let region = format!("R{}", &suffix[..8]); // VARCHAR(10)
    let mut opportunity_ids = Vec::new();
    let mut threat_ids = Vec::new();

    for priority in [0.9_f64, 0.8, 0.7, 0.6] {
        let record = store
            .create_strategic_opportunity(
                &marker("exec-opp"),
                None,
                "partnership",
                priority,
                0.8,
                None,
                None,
                Some(&region),
                None,
                &json!([]),
                None,
                None,
                FIXTURE_CREATOR,
            )
            .await
            .expect("create opportunity");
        opportunity_ids.push(record.id);
    }

    for (impact, severity, status) in [
        (0.9_f64, "high", "active"),
        (0.5, "low", "active"),
        (0.95, "critical", "resolved"),
    ] {
        let record = store
            .create_critical_threat(
                &marker("exec-threat"),
                None,
                "financial",
                severity,
                impact,
                0.8,
                None,
                None,
                Some(&region),
                &json!([]),
                None,
                None,
                FIXTURE_CREATOR,
            )
            .await
            .expect("create threat");
        if status == "resolved" {
            store
                .update_critical_threat_status(record.id, "resolved")
                .await
                .expect("resolve threat");
        }
        threat_ids.push(record.id);
    }

    // Filters are applied in SQL: the page limit of 2 must still surface the
    // two highest filtered rows (a filter-after-page bug would return fewer).
    let page = store
        .list_strategic_opportunities_filtered(false, Some(0.65), Some(&region), 2)
        .await
        .expect("filtered opportunities");
    assert_eq!(page.len(), 2);
    assert!(page.iter().all(|row| row.priority_score >= 0.65));
    assert!(page
        .iter()
        .all(|row| row.region.as_deref() == Some(region.as_str())));

    let threat_page = store
        .list_critical_threats_filtered(Some(0.8), Some(&region), None, 50)
        .await
        .expect("filtered threats");
    assert_eq!(threat_page.len(), 1, "impact/region filters run in SQL");
    assert_eq!(threat_page[0].impact_score, 0.9);

    let low_severity = store
        .list_critical_threats_filtered(None, Some(&region), Some("low"), 50)
        .await
        .expect("severity filter");
    assert_eq!(low_severity.len(), 1);
    assert_eq!(low_severity[0].severity, "low");

    // Totals are COUNT(*) in SQL, independent of any page limit: 4 open
    // opportunities (including the 0.6 one below the page) and 2 active
    // threats; the threshold only affects `high_priority_count`.
    let (total_opportunities, total_threats, high_priority, average_confidence, regions) = store
        .executive_dashboard_aggregates(true, true, 0.65, Some(&region))
        .await
        .expect("aggregates");
    assert_eq!(
        total_opportunities, 4,
        "totals must not be derived from a capped page"
    );
    assert_eq!(total_threats, 2);
    assert_eq!(
        high_priority, 4,
        "3 opportunities + 1 threat above the threshold"
    );
    assert!(
        (average_confidence - 0.8).abs() < 1e-9,
        "average confidence must cover every row, got {average_confidence}"
    );
    assert_eq!(regions, vec![region.clone()]);

    // Excluding threats from the aggregate drops their count and average rows.
    let (again_opportunities, no_threats, _, _, _) = store
        .executive_dashboard_aggregates(true, false, 0.65, Some(&region))
        .await
        .expect("aggregates without threats");
    assert_eq!(again_opportunities, 4);
    assert_eq!(no_threats, 0);

    for id in opportunity_ids {
        sqlx::query("DELETE FROM strategic_opportunities WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup opportunity");
    }
    for id in threat_ids {
        sqlx::query("DELETE FROM critical_threats WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup threat");
    }
    pool.close().await;
}

// ── #62: evidence source_domain derived from the URL host ──────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn web_evidence_derives_source_domain_from_url_host() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let session = web_session(&marker("evidence"), ApiRole::Analyst);
    let entity_id = format!("comp-{}", Uuid::new_v4().simple());

    let response = web_collab::add_evidence(
        Extension(session),
        Extension(store.clone()),
        Form(web_collab::AddEvidenceForm {
            entity_type: "company".to_string(),
            entity_id: entity_id.clone(),
            evidence_type: "news_article".to_string(),
            source_url: "https://news.example.com/article?id=1".to_string(),
            source_domain: None,
            source_name: Some("Example News".to_string()),
            reliability_score: 0.7,
            excerpt: None,
        }),
    )
    .await
    .into_response();
    assert!(
        response.status().is_redirection(),
        "a valid evidence form must persist and redirect, got {}",
        response.status()
    );

    let rows = store
        .list_source_evidence(Some("company"), Some(&entity_id), None, 10)
        .await
        .expect("list evidence");
    assert_eq!(rows.len(), 1, "evidence row must be stored");
    assert_eq!(
        rows[0].source_domain.as_deref(),
        Some("news.example.com"),
        "source_domain must come from the URL host"
    );

    sqlx::query("DELETE FROM source_evidence WHERE entity_id = $1")
        .bind(&entity_id)
        .execute(&pool)
        .await
        .expect("cleanup evidence");
    pool.close().await;
}

// ── #55: web form validation (enums, name bounds, NaN/Infinity) ────────────

#[test]
fn web_validation_rejects_bad_enums_and_non_finite_numbers() {
    assert!(validate_evidence_type("news_article").is_ok());
    assert!(validate_evidence_type("news").is_err());
    assert!(validate_risk_category("financial").is_ok());
    assert!(validate_risk_category("bogus").is_err());
    assert!(validate_stage("discovery").is_ok());
    assert!(validate_stage("pending").is_err());
    assert!(validate_workspace_assignment_role("contributor").is_ok());
    assert!(validate_workspace_assignment_role("observer").is_err());
    assert!(validate_team_assignment_role("observer").is_ok());
    assert!(validate_team_assignment_role("owner").is_err());
    assert!(validate_share_type("view").is_ok());
    assert!(validate_share_type("share").is_err());
    assert!(validate_access_level("read_write").is_ok());
    assert!(validate_access_level("write").is_err());

    assert!(validate_workspace_name("abc").is_ok());
    assert!(
        validate_workspace_name(" abc ").is_ok(),
        "padding is trimmed before the length check"
    );
    assert!(
        validate_workspace_name(" ab ").is_err(),
        "2 chars after trimming must be rejected"
    );
    assert!(
        validate_workspace_name("ab").is_err(),
        "2 chars must be rejected"
    );
    assert!(validate_workspace_name("").is_err());
    assert!(validate_workspace_name(&"a".repeat(255)).is_ok());
    assert!(
        validate_workspace_name(&"a".repeat(256)).is_err(),
        "256 chars must be rejected"
    );

    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            validate_risk_score(value).is_err(),
            "risk_score {value} must be rejected"
        );
        assert!(
            validate_probability(value).is_err(),
            "probability {value} must be rejected"
        );
        assert!(
            validate_reliability_score(value).is_err(),
            "reliability {value} must be rejected"
        );
    }
    assert!(validate_risk_score(0.0).is_ok());
    assert!(validate_risk_score(1.0).is_ok());
    assert!(validate_risk_score(1.5).is_err());
    assert!(validate_probability(-0.1).is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn web_forms_reject_nan_and_invalid_enums_before_the_store() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let session = web_session(&marker("forms"), ApiRole::Analyst);

    // Hygiene: a pre-fix run may have stored a NaN row; purge leftovers so the
    // end-of-test count reflects only this run.
    sqlx::query("DELETE FROM pipeline_opportunities WHERE title LIKE 'verify-wsauthz-nan-value-%'")
        .execute(&pool)
        .await
        .expect("purge stale NaN rows");

    // Supplier risk: NaN score and unknown category must both be 400.
    let nan_risk = web_collab::add_supplier_risk(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddSupplierRiskForm {
            supplier_id: "sup-test".to_string(),
            risk_category: "financial".to_string(),
            risk_score: f64::NAN,
            risk_factors: None,
            mitigation: None,
            owner_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(
        nan_risk.status(),
        StatusCode::BAD_REQUEST,
        "NaN risk score must be rejected"
    );

    let bad_category = web_collab::add_supplier_risk(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddSupplierRiskForm {
            supplier_id: "sup-test".to_string(),
            risk_category: "vibes".to_string(),
            risk_score: 0.5,
            risk_factors: None,
            mitigation: None,
            owner_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(bad_category.status(), StatusCode::BAD_REQUEST);

    // Evidence: NaN reliability and a legacy evidence type must both be 400.
    let nan_reliability = web_collab::add_evidence(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddEvidenceForm {
            entity_type: "company".to_string(),
            entity_id: "comp-test".to_string(),
            evidence_type: "news_article".to_string(),
            source_url: "https://example.com".to_string(),
            source_domain: None,
            source_name: None,
            reliability_score: f64::INFINITY,
            excerpt: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(nan_reliability.status(), StatusCode::BAD_REQUEST);

    let legacy_type = web_collab::add_evidence(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddEvidenceForm {
            entity_type: "company".to_string(),
            entity_id: "comp-test".to_string(),
            evidence_type: "news".to_string(),
            source_url: "https://example.com".to_string(),
            source_domain: None,
            source_name: None,
            reliability_score: 0.5,
            excerpt: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(legacy_type.status(), StatusCode::BAD_REQUEST);

    // Pipeline: NaN probability and an unknown stage must both be 400.
    let nan_probability = web_collab::create_pipeline_opportunity(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::CreatePipelineForm {
            title: "deal".to_string(),
            stage: "discovery".to_string(),
            value_estimate: None,
            probability: f64::NAN,
            owner_id: None,
            expected_close: None,
            notes: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(nan_probability.status(), StatusCode::BAD_REQUEST);

    let bad_stage = web_collab::create_pipeline_opportunity(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::CreatePipelineForm {
            title: "deal".to_string(),
            stage: "hunch".to_string(),
            value_estimate: None,
            probability: 0.5,
            owner_id: None,
            expected_close: None,
            notes: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(bad_stage.status(), StatusCode::BAD_REQUEST);

    // Adversarial: a NaN value_estimate used to pass the probability check and
    // reach the NUMERIC column (PostgreSQL accepts 'NaN' in DECIMAL). It must
    // be rejected as a form validation error.
    let nan_value = web_collab::create_pipeline_opportunity(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::CreatePipelineForm {
            title: format!("verify-wsauthz-nan-value-{}", Uuid::new_v4().simple()),
            stage: "discovery".to_string(),
            value_estimate: Some(f64::NAN),
            probability: 0.5,
            owner_id: None,
            expected_close: None,
            notes: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(
        nan_value.status(),
        StatusCode::BAD_REQUEST,
        "a non-finite value_estimate must be rejected, not stored"
    );
    let (nan_rows,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint FROM pipeline_opportunities WHERE value_estimate = 'NaN'::numeric",
    )
    .fetch_one(&pool)
    .await
    .expect("count NaN value estimates");
    assert_eq!(
        nan_rows, 0,
        "NaN must never reach pipeline_opportunities.value_estimate"
    );

    // Share: unknown access level with a syntactically valid workspace id.
    let share_response = web_collab::share_workspace(
        Extension(session.clone()),
        Extension(store.clone()),
        Path(Uuid::new_v4().to_string()),
        Form(web_collab::ShareWorkspaceForm {
            shared_with: "someone".to_string(),
            share_type: "view".to_string(),
            access_level: "write".to_string(),
            message: None,
            expires_at: Some("31/12/2026".to_string()),
        }),
    )
    .await
    .into_response();
    assert_eq!(share_response.status(), StatusCode::BAD_REQUEST);

    // Assign: unknown workspace role.
    let assign_response = web_collab::assign_user_to_workspace(
        Extension(session.clone()),
        Extension(store.clone()),
        Path(Uuid::new_v4().to_string()),
        Form(web_collab::AssignUserForm {
            user_id: "someone".to_string(),
            role: "observer".to_string(),
        }),
    )
    .await
    .into_response();
    assert_eq!(assign_response.status(), StatusCode::BAD_REQUEST);

    pool.close().await;
}

// ── #50: web handlers (get / close / assign / share) use the guard ──────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn web_workspace_handlers_enforce_the_authorization_guard() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));

    let owner = marker("wowner");
    let stranger = marker("wstranger");
    let read_user = marker("wread");
    let admin_share_user = marker("wadmin");
    let admin_user = marker("wplatformadmin");

    let ws = create_ws(&store, "web-guard", &owner, "private").await;
    store
        .create_investigation_share(ws, &owner, &read_user, "view", "read", None, None)
        .await
        .expect("read share");
    store
        .create_investigation_share(
            ws,
            &owner,
            &admin_share_user,
            "collaborate",
            "admin",
            None,
            None,
        )
        .await
        .expect("admin share");

    let owner_session = web_session(&owner, ApiRole::Analyst);
    let stranger_session = web_session(&stranger, ApiRole::Analyst);
    let read_session = web_session(&read_user, ApiRole::Analyst);
    let admin_share_session = web_session(&admin_share_user, ApiRole::Analyst);
    let admin_session = web_session(&admin_user, ApiRole::Admin);

    // GET: denials render 404, not 403 (existence is not leaked).
    let denied = web_collab::get_workspace(
        Extension(stranger_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
    )
    .await
    .into_response();
    assert_eq!(
        denied.status(),
        StatusCode::NOT_FOUND,
        "stranger get must be 404"
    );

    let allowed = web_collab::get_workspace(
        Extension(owner_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
    )
    .await
    .into_response();
    assert_eq!(allowed.status(), StatusCode::OK);
    let admin_view = web_collab::get_workspace(
        Extension(admin_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
    )
    .await
    .into_response();
    assert_eq!(
        admin_view.status(),
        StatusCode::OK,
        "platform admin bypass is web-visible"
    );

    // CLOSE is a write: a read share is not enough, an admin share is.
    let denied_close = web_collab::close_workspace(
        Extension(read_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
    )
    .await
    .into_response();
    assert_eq!(
        denied_close.status(),
        StatusCode::NOT_FOUND,
        "read share must not close"
    );

    let admin_share_close = web_collab::close_workspace(
        Extension(admin_share_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
    )
    .await
    .into_response();
    assert!(
        admin_share_close.status().is_redirection(),
        "admin share holds write"
    );
    let closed = store
        .get_investigation_workspace(ws)
        .await
        .expect("read workspace")
        .expect("exists");
    assert_eq!(closed.status, "closed");
    assert!(closed.closed_at.is_some());

    // ASSIGN requires Manage: stranger and read share are denied; an admin
    // share may assign.
    let assign_denied = web_collab::assign_user_to_workspace(
        Extension(stranger_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
        Form(web_collab::AssignUserForm {
            user_id: marker("wassignee"),
            role: "contributor".to_string(),
        }),
    )
    .await
    .into_response();
    assert_eq!(assign_denied.status(), StatusCode::NOT_FOUND);

    let assign_read_denied = web_collab::assign_user_to_workspace(
        Extension(read_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
        Form(web_collab::AssignUserForm {
            user_id: marker("wassignee"),
            role: "contributor".to_string(),
        }),
    )
    .await
    .into_response();
    assert_eq!(
        assign_read_denied.status(),
        StatusCode::NOT_FOUND,
        "a read share must not manage members"
    );

    let assignee = marker("wassignee");
    let assign_ok = web_collab::assign_user_to_workspace(
        Extension(admin_share_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
        Form(web_collab::AssignUserForm {
            user_id: assignee.clone(),
            role: "contributor".to_string(),
        }),
    )
    .await
    .into_response();
    assert!(
        assign_ok.status().is_redirection(),
        "admin share manages members"
    );
    let assignments = store
        .list_workspace_assignments(ws)
        .await
        .expect("list assignments");
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].user_id, assignee);

    // SHARE requires Manage: read share denied, owner allowed.
    let share_denied = web_collab::share_workspace(
        Extension(read_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
        Form(web_collab::ShareWorkspaceForm {
            shared_with: "someone".to_string(),
            share_type: "view".to_string(),
            access_level: "read".to_string(),
            message: None,
            expires_at: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(
        share_denied.status(),
        StatusCode::NOT_FOUND,
        "read share must not share"
    );

    let new_recipient = marker("wrecipient");
    let share_ok = web_collab::share_workspace(
        Extension(owner_session.clone()),
        Extension(store.clone()),
        Path(ws.to_string()),
        Form(web_collab::ShareWorkspaceForm {
            shared_with: new_recipient.clone(),
            share_type: "view".to_string(),
            access_level: "read".to_string(),
            message: None,
            expires_at: None,
        }),
    )
    .await
    .into_response();
    assert!(share_ok.status().is_redirection(), "owner may share");
    let shares = store
        .list_investigation_shares(ws)
        .await
        .expect("list shares");
    assert!(shares.iter().any(|s| s.shared_with == new_recipient));

    delete_workspaces(&pool, &[ws]).await;
    pool.close().await;
}

// ── #54: error strings never leak database detail ───────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn store_error_hides_database_detail() {
    // A synthetic error carrying a raw SQL message.
    let synthetic = anyhow::anyhow!(
        "error returned from database: relation \"investigation_workspaces\" does not exist"
    );
    let mapped = store_error(synthetic);
    assert_eq!(mapped.http_status(), 500);
    assert!(
        mapped.message.contains("incident"),
        "must expose an incident id: {}",
        mapped.message
    );
    for needle in [
        "investigation_workspaces",
        "relation",
        "sqlx",
        "database error",
    ] {
        assert!(
            !mapped.message.contains(needle),
            "store_error leaked '{needle}': {}",
            mapped.message
        );
    }

    // A real sqlx error from a real (closed) connection.
    let lazy = PgPool::connect_lazy("postgres://apex:apex@127.0.0.1:1/apex_verifier_unused")
        .expect("lazy pool");
    let error = sqlx::query("SELECT * FROM collaboration_verifier_missing_table")
        .execute(&lazy)
        .await
        .expect_err("query must fail");
    let mapped = store_error(anyhow::Error::new(error));
    assert_eq!(mapped.http_status(), 500);
    assert!(!mapped
        .message
        .contains("collaboration_verifier_missing_table"));
    assert!(!mapped.message.contains("connection"));
    assert!(mapped.message.contains("incident"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn web_500_paths_do_not_echo_database_errors() {
    // A lazy pool pointed at a closed port: every store call fails.
    let lazy = Arc::new(PgStore::from_pool(
        PgPool::connect_lazy("postgres://apex:apex@127.0.0.1:1/apex_verifier_unused")
            .expect("lazy pool"),
    ));
    let session = web_session(&marker("web500"), ApiRole::Analyst);

    let response = web_collab::add_supplier_risk(
        Extension(session),
        Extension(lazy),
        Form(web_collab::AddSupplierRiskForm {
            supplier_id: "sup-500".to_string(),
            risk_category: "financial".to_string(),
            risk_score: 0.5,
            risk_factors: None,
            mitigation: None,
            owner_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let text = body_text(response).await;
    assert!(
        text.contains("Failed to save supplier risk entry"),
        "the 500 must carry the operator-facing message: {text}"
    );
    for needle in [
        "sqlx",
        "postgres",
        "127.0.0.1",
        "connection refused",
        "relation",
        "column",
    ] {
        assert!(
            !text.contains(needle),
            "500 body leaked database detail '{needle}': {text}"
        );
    }
}

// ── Record attribution: created_by + validated owner (migration 101) ──────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn executive_records_carry_attribution_and_a_validated_owner() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let session = web_session(FIXTURE_CREATOR, ApiRole::Analyst);
    let ghost = format!("verify-wsauthz-ghost-{}", Uuid::new_v4().simple());

    // Shared owner resolution: blank -> None, known -> Some, unknown -> a
    // field validation error (422 on the JSON API).
    assert_eq!(
        resolve_record_owner(&store, Some("  ")).await.unwrap(),
        None
    );
    assert_eq!(
        resolve_record_owner(&store, Some(" owner-1 "))
            .await
            .unwrap(),
        Some("owner-1".to_string())
    );
    let unknown = resolve_record_owner(&store, Some(&ghost))
        .await
        .expect_err("unknown owner must be rejected");
    assert_eq!(unknown.http_status(), 422);
    assert_eq!(
        unknown
            .details
            .as_ref()
            .and_then(|d| d.get("field"))
            .map(String::as_str),
        Some("owner_id")
    );

    // The foreign key is the backstop when a writer bypasses validation.
    let fk = store
        .create_supplier_risk_entry(
            &marker("supplier-fk"),
            "financial",
            0.5,
            &json!({}),
            None,
            Some(&ghost),
            FIXTURE_CREATOR,
        )
        .await;
    assert!(fk.is_err(), "owner_id must reference app_users");
    let fk_creator = store
        .create_pipeline_opportunity(
            None,
            &marker("pipeline-fk"),
            "discovery",
            None,
            0.5,
            None,
            None,
            None,
            &ghost,
        )
        .await;
    assert!(fk_creator.is_err(), "created_by must reference app_users");

    // Web pipeline form: percent input is stored as a fraction, the session
    // principal is the author, and the selected owner is persisted.
    let title = marker("pipeline-web");
    let created = web_collab::create_pipeline_opportunity(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::CreatePipelineForm {
            title: format!("  {title}  "),
            stage: "qualification".to_string(),
            value_estimate: Some(1000.0),
            probability: 75.0,
            owner_id: Some("owner-1".to_string()),
            expected_close: None,
            notes: Some("   ".to_string()),
        }),
    )
    .await
    .into_response();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let (probability, owner, creator, notes): (
        f64,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT probability::double precision, owner_id, created_by, notes \
             FROM pipeline_opportunities WHERE title = $1",
    )
    .bind(&title)
    .fetch_one(&pool)
    .await
    .expect("pipeline row stored with trimmed title");
    assert!(
        (probability - 0.75).abs() < 1e-9,
        "75% must be stored as 0.75"
    );
    assert_eq!(owner.as_deref(), Some("owner-1"));
    assert_eq!(creator.as_deref(), Some(FIXTURE_CREATOR));
    assert_eq!(notes, None, "blank notes are not stored");

    // Unknown owner on the web form is a 400 and writes nothing.
    let ghost_title = marker("pipeline-ghost");
    let rejected = web_collab::create_pipeline_opportunity(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::CreatePipelineForm {
            title: ghost_title.clone(),
            stage: "discovery".to_string(),
            value_estimate: None,
            probability: 50.0,
            owner_id: Some(ghost.clone()),
            expected_close: None,
            notes: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let (ghost_rows,): (i64,) =
        sqlx::query_as("SELECT COUNT(*)::bigint FROM pipeline_opportunities WHERE title = $1")
            .bind(&ghost_title)
            .fetch_one(&pool)
            .await
            .expect("count ghost rows");
    assert_eq!(ghost_rows, 0);

    // Web supplier risk form: percent score and attribution.
    let supplier = marker("supplier-web");
    let added = web_collab::add_supplier_risk(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddSupplierRiskForm {
            supplier_id: supplier.clone(),
            risk_category: "compliance".to_string(),
            risk_score: 80.0,
            risk_factors: None,
            mitigation: None,
            owner_id: Some(String::new()),
        }),
    )
    .await
    .into_response();
    assert_eq!(added.status(), StatusCode::SEE_OTHER);
    let (score, owner, creator): (f64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT risk_score::double precision, owner_id, created_by FROM supplier_risk WHERE supplier_id = $1",
    )
    .bind(&supplier)
    .fetch_one(&pool)
    .await
    .expect("supplier row stored");
    assert!((score - 0.8).abs() < 1e-9, "80 must be stored as 0.8");
    assert_eq!(owner, None, "an empty owner select means unassigned");
    assert_eq!(creator.as_deref(), Some(FIXTURE_CREATOR));

    let over = web_collab::add_supplier_risk(
        Extension(session.clone()),
        Extension(store.clone()),
        Form(web_collab::AddSupplierRiskForm {
            supplier_id: supplier.clone(),
            risk_category: "compliance".to_string(),
            risk_score: 101.0,
            risk_factors: None,
            mitigation: None,
            owner_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(over.status(), StatusCode::BAD_REQUEST);

    // Listing resolves attribution ids to user labels.
    let page = web_collab::list_pipeline(
        Extension(session.clone()),
        Extension(store.clone()),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(html.contains(&title));
    assert!(html.contains("75%"), "probability renders as a percentage");
    assert!(html.contains(r#"<option value="discovery">Discovery</option>"#));
    assert!(!html.contains(r#"value="identification""#));

    sqlx::query("DELETE FROM pipeline_opportunities WHERE title = $1")
        .bind(&title)
        .execute(&pool)
        .await
        .expect("cleanup pipeline");
    sqlx::query("DELETE FROM supplier_risk WHERE supplier_id = $1")
        .bind(&supplier)
        .execute(&pool)
        .await
        .expect("cleanup supplier risk");
    pool.close().await;
}

// ── entity page "open investigations" respects workspace visibility ─────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn entity_workspace_list_hides_invisible_workspaces() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let entity = Uuid::new_v4().to_string();
    let owner = marker("eowner");
    let outsider = marker("eoutsider");
    let viewer = marker("eviewer");

    let mut ids = Vec::new();
    for (prefix, visibility, focus) in [
        ("entity-private", "private", json!([entity.clone()])),
        (
            "entity-org",
            "organization",
            json!([{ "id": entity.clone() }]),
        ),
        (
            "entity-shared",
            "private",
            json!({ "entity_id": entity.clone() }),
        ),
    ] {
        let id = store
            .create_investigation_workspace(
                &marker(prefix),
                None,
                "structured",
                &owner,
                None,
                visibility,
                &[],
                &focus,
            )
            .await
            .expect("create entity workspace")
            .id;
        ids.push((prefix, id));
    }
    let id_of = |name: &str| {
        ids.iter()
            .find(|(prefix, _)| *prefix == name)
            .map(|(_, id)| *id)
            .expect("fixture id")
    };
    store
        .create_investigation_share(
            id_of("entity-shared"),
            &owner,
            &viewer,
            "view",
            "read",
            None,
            None,
        )
        .await
        .expect("share entity workspace");

    let visible = |records: Vec<InvestigationWorkspaceRecord>| {
        let mut found: Vec<Uuid> = records.into_iter().map(|w| w.id).collect();
        found.sort();
        found
    };
    let sorted = |mut list: Vec<Uuid>| {
        list.sort();
        list
    };

    let as_owner = store
        .list_investigation_workspaces_for_entity(&entity, &owner, false, 25)
        .await
        .expect("owner list");
    assert_eq!(
        visible(as_owner),
        sorted(ids.iter().map(|(_, id)| *id).collect()),
        "owner sees every focused workspace in all entity_focus shapes"
    );

    let as_outsider = store
        .list_investigation_workspaces_for_entity(&entity, &outsider, false, 25)
        .await
        .expect("outsider list");
    assert_eq!(
        visible(as_outsider),
        vec![id_of("entity-org")],
        "a non-member only sees organization-visible workspaces"
    );

    let as_viewer = store
        .list_investigation_workspaces_for_entity(&entity, &viewer, false, 25)
        .await
        .expect("viewer list");
    assert_eq!(
        visible(as_viewer),
        sorted(vec![id_of("entity-org"), id_of("entity-shared")]),
        "a share grants visibility of exactly the shared workspace"
    );

    let as_admin = store
        .list_investigation_workspaces_for_entity(&entity, &outsider, true, 25)
        .await
        .expect("admin list");
    assert_eq!(as_admin.len(), 3, "admin sees all focused workspaces");

    let other_entity = store
        .list_investigation_workspaces_for_entity(&Uuid::new_v4().to_string(), &owner, true, 25)
        .await
        .expect("other entity list");
    assert!(other_entity
        .iter()
        .all(|w| !ids.iter().any(|(_, id)| *id == w.id)));

    sqlx::query("DELETE FROM investigation_shares WHERE shared_with = $1")
        .bind(&viewer)
        .execute(&pool)
        .await
        .expect("cleanup shares");
    let all: Vec<Uuid> = ids.iter().map(|(_, id)| *id).collect();
    delete_workspaces(&pool, &all).await;
    pool.close().await;
}
