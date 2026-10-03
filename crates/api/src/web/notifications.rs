use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Path,
    response::{IntoResponse, Redirect},
    Extension,
};
use uuid::Uuid;

use super::{safe_relative_href, PageContext};
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{PgStore, WarningListFilters};

#[derive(Clone, Debug)]
pub struct NotificationItem {
    pub id: String,
    pub category: String,
    pub title: String,
    pub body: String,
    /// Internal action link. `None` means the stored value was absent or not
    /// a safe relative path, in which case the template renders no link
    /// (audit #52: never emit a stored URL into `href` unfiltered).
    pub action_url: Option<String>,
    pub entity_label: String,
    pub created_at: String,
    pub is_read: bool,
}

#[derive(Template)]
#[template(path = "pages/notifications.html")]
pub struct NotificationsPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub unread_count: i64,
    pub total: i64,
    /// Rows on this page only (page size 25).
    pub notifications: Vec<NotificationItem>,
    pub page: i64,
    pub total_pages: i64,
    /// "all" or "unread" — drives the filter chips.
    pub filter: String,
    /// Base href ending in `&` (or `?`) ready for `page=N`.
    pub page_base_href: String,
    /// Set when any backing query failed, so a storage error never renders as
    /// an empty inbox.
    pub degraded_notice: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct NotificationsQuery {
    pub page: Option<i64>,
    pub filter: Option<String>,
}

const NOTIFICATIONS_PAGE_SIZE: i64 = 25;

pub async fn list_notifications_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    axum::extract::Query(query): axum::extract::Query<NotificationsQuery>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let filter = match query.filter.as_deref() {
        Some("unread") => "unread",
        _ => "all",
    };
    let include_read = filter == "all";
    let page = query.page.unwrap_or(1).max(1);

    let warning_count_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web notifications page)",
        |_| false,
    );
    DegradedNotice::capture(&warning_count_state, &mut degraded_notice);
    let warning_count = warning_count_state.into_loaded_or(0);
    let ctx = PageContext::from_session(&session, "/notifications", warning_count);

    let notifications_state = DataState::from_result(
        store
            .list_notifications_paged(
                &session.user_id,
                include_read,
                NOTIFICATIONS_PAGE_SIZE,
                (page - 1) * NOTIFICATIONS_PAGE_SIZE,
            )
            .await,
        "list_notifications failed (web notifications page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&notifications_state, &mut degraded_notice);
    let notifications = notifications_state.into_items();

    let unread_count_state = DataState::from_result(
        store.unread_notification_count(&session.user_id).await,
        "unread_notification_count failed (web notifications page)",
        |_| false,
    );
    DegradedNotice::capture(&unread_count_state, &mut degraded_notice);
    let unread_count = unread_count_state.into_loaded_or(0);

    let total_state = DataState::from_result(
        store
            .count_notifications(&session.user_id, include_read)
            .await,
        "count_notifications failed (web notifications page)",
        |_| false,
    );
    DegradedNotice::capture(&total_state, &mut degraded_notice);
    let total = total_state.into_loaded_or(0);
    let total_pages = ((total + NOTIFICATIONS_PAGE_SIZE - 1) / NOTIFICATIONS_PAGE_SIZE).max(1);

    let page = NotificationsPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        unread_count,
        degraded_notice,
        total,
        page,
        total_pages,
        filter: filter.to_string(),
        page_base_href: format!("/notifications?filter={filter}&"),
        notifications: notifications
            .into_iter()
            .map(|notification| NotificationItem {
                id: notification.id.to_string(),
                category: notification.category.replace('_', " "),
                title: notification.title,
                body: notification.body,
                action_url: match notification.action_url.as_deref() {
                    None => Some("/notifications".to_string()),
                    Some(raw) => safe_relative_href(raw),
                },
                entity_label: match (notification.entity_type, notification.entity_id) {
                    (Some(entity_type), Some(entity_id)) => {
                        format!("{} · {}", entity_type, entity_id)
                    }
                    (Some(entity_type), None) => entity_type,
                    _ => String::new(),
                },
                created_at: notification.created_at.format("%Y-%m-%d %H:%M").to_string(),
                is_read: notification.is_read,
            })
            .collect(),
    };

    super::render_template(&page)
}

/// POST /notifications/read-all — mark the operator's entire inbox read.
pub async fn mark_all_notifications_read(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    if let Err(error) = store.mark_all_notifications_read(&session.user_id).await {
        tracing::error!(%error, "mark_all_notifications_read: write failed");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to mark notifications read",
        )
            .into_response();
    }
    Redirect::to("/notifications").into_response()
}

pub async fn mark_notification_read(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            // A malformed notification ID is a validation error: silently
            // redirecting would read as "marked read" when nothing was.
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid notification ID",
            )
                .into_response();
        }
    };
    if let Err(error) = store.mark_notification_read(&session.user_id, uuid).await {
        // Authoritative persistence: do not redirect as if the read state
        // was stored when the write failed.
        tracing::error!(%error, notification_id = %id, "mark_notification_read: write failed");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to mark notification read",
        )
            .into_response();
    }

    Redirect::to("/notifications").into_response()
}

/// POST /notifications/:id/delete — permanently delete one of the operator's
/// own notifications. A delete is a real DELETE: an inbox of thousands never
/// shrinks through marking read, which is why clearing needs its own route.
pub async fn delete_notification(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            // A malformed notification ID is a validation error: redirecting
            // would read as "deleted" when nothing was.
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid notification ID",
            )
                .into_response();
        }
    };
    match store.delete_notification(&session.user_id, uuid).await {
        Ok(true) => Redirect::to("/notifications").into_response(),
        Ok(false) => (axum::http::StatusCode::NOT_FOUND, "Notification not found").into_response(),
        Err(error) => {
            tracing::error!(%error, notification_id = %id, "delete_notification: delete failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete notification",
            )
                .into_response()
        }
    }
}

/// POST /notifications/clear-all — delete the operator's entire inbox
/// (read and unread), scoped to the session principal.
pub async fn clear_all_notifications(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    match store.delete_all_notifications(&session.user_id).await {
        Ok(deleted) => {
            tracing::info!(
                user_id = %session.user_id,
                deleted,
                "clear_all_notifications: inbox cleared"
            );
            Redirect::to("/notifications").into_response()
        }
        Err(error) => {
            tracing::error!(%error, "clear_all_notifications: delete failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to clear notifications",
            )
                .into_response()
        }
    }
}

/// POST /notifications/clear-read — delete only the already-read notifications.
pub async fn clear_read_notifications(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    match store.delete_read_notifications(&session.user_id).await {
        Ok(deleted) => {
            tracing::info!(
                user_id = %session.user_id,
                deleted,
                "clear_read_notifications: read notifications cleared"
            );
            Redirect::to("/notifications").into_response()
        }
        Err(error) => {
            tracing::error!(%error, "clear_read_notifications: delete failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to clear read notifications",
            )
                .into_response()
        }
    }
}
