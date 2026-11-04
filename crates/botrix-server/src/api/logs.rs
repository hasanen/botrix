//! Log event API handlers.
//!
//! - `GET /api/v1/logs` — paginated log events (`?page=1&per_page=50`)

use axum::extract::{Query, State};
use axum::Json;
use botrix_common::api::logs::LogListResponse;
use tower_sessions::Session;

use super::error::ApiError;
use super::pagination::{Pagination, DEFAULT_LOGS_PER_PAGE};
use crate::db::history;
use crate::session;
use crate::state::AppState;

/// `GET /api/v1/logs` — return one page of log events, newest first.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(pagination): Query<Pagination>,
) -> Result<Json<LogListResponse>, ApiError> {
    session::current_user(&session)
        .await
        .ok_or(ApiError::Unauthorized)?;

    let page = pagination.page();
    let per_page = pagination.per_page(DEFAULT_LOGS_PER_PAGE);
    let (items, total) = history::list_logs(&state.pool, page, per_page)?;

    Ok(Json(LogListResponse {
        items,
        page,
        per_page,
        total,
    }))
}
