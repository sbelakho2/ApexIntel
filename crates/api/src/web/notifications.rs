use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Path,
    response::{IntoResponse, Redirect},
    Extension,
};
use uuid::Uuid;

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{PgStore, WarningListFilters};

#[derive(Clone, Debug)]
pub struct NotificationItem {
    pub id: String,
    pub category: String,
    pub title: String,
    pub body: String,
    pub action_url: String,
    pub entity_label: String,
    pub created_at: String,
    pub is_read: bool,
}

#[derive(Template)]
#[template(path = "pages/notifications.html")]
pub struct NotificationsPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub unread_count: i64,
    pub total: i64,
    pub notifications: Vec<NotificationItem>,
    /// Set when any backing query failed, so a storage error never renders as
    /// an empty inbox.
    pub degraded_notice: Option<String>,
}

pub async fn list_notifications_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;

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
        store.list_notifications(&session.user_id, true, 100).await,
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

    let page = NotificationsPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        unread_count,
        degraded_notice,
        total: notifications.len() as i64,
        notifications: notifications
            .into_iter()
            .map(|notification| NotificationItem {
                id: notification.id.to_string(),
                category: notification.category.replace('_', " "),
                title: notification.title,
                body: notification.body,
                action_url: notification
                    .action_url
                    .unwrap_or_else(|| "/notifications".to_string()),
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

pub async fn mark_notification_read(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let _ = store.mark_notification_read(&session.user_id, uuid).await;
    }

    Redirect::to("/notifications")
}
