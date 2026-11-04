//! Notification history API handlers.
//!
//! - `GET /api/v1/notifications` — paginated notification history
//!   (`?page=1&per_page=25`)

use axum::extract::{Query, State};
use axum::Json;
use botrix_common::api::notifications::NotificationListResponse;
use tower_sessions::Session;

use super::error::ApiError;
use super::pagination::{Pagination, DEFAULT_NOTIFICATIONS_PER_PAGE};
use crate::db::history;
use crate::session;
use crate::state::AppState;

/// `GET /api/v1/notifications` — return one page of notifications, newest first.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(pagination): Query<Pagination>,
) -> Result<Json<NotificationListResponse>, ApiError> {
    session::current_user(&session)
        .await
        .ok_or(ApiError::Unauthorized)?;

    let page = pagination.page();
    let per_page = pagination.per_page(DEFAULT_NOTIFICATIONS_PER_PAGE);
    let (items, total) = history::list_notifications(&state.pool, page, per_page)?;

    Ok(Json(NotificationListResponse {
        items,
        page,
        per_page,
        total,
    }))
}
