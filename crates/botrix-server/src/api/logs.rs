//! Log event API handlers.
//!
//! - `GET /api/v1/logs` — paginated log events, newest first
//!   (`?page=1&per_page=50`). Optional filters: `?component=trigger` limits to
//!   one component and `?command=deploy` to the events of one trigger command
//!   (case-insensitive). Blank filter values are ignored.

use axum::extract::{Query, State};
use axum::Json;
use botrix_common::api::logs::LogListResponse;
use serde::Deserialize;
use tower_sessions::Session;

use super::error::ApiError;
use super::pagination::{Pagination, DEFAULT_LOGS_PER_PAGE};
use crate::db::history::{self, LogFilter};
use crate::session;
use crate::state::AppState;

/// `?component=&command=` query parameters, both optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogFilterParams {
    component: Option<String>,
    command: Option<String>,
}

impl LogFilterParams {
    /// The component filter, if one was given.
    fn component(&self) -> Option<&str> {
        non_blank(self.component.as_deref())
    }

    /// The command filter, if one was given.
    fn command(&self) -> Option<&str> {
        non_blank(self.command.as_deref())
    }

    fn filter(&self) -> LogFilter<'_> {
        LogFilter {
            component: self.component(),
            command: self.command(),
        }
    }
}

/// The trimmed value, or `None` if it is missing or blank.
fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// `GET /api/v1/logs` — return one page of log events, newest first.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(pagination): Query<Pagination>,
    Query(params): Query<LogFilterParams>,
) -> Result<Json<LogListResponse>, ApiError> {
    session::current_user(&session)
        .await
        .ok_or(ApiError::Unauthorized)?;
    let page = pagination.page();
    let per_page = pagination.per_page(DEFAULT_LOGS_PER_PAGE);
    let (items, total) =
        history::list_logs_filtered(&state.pool, page, per_page, &params.filter())?;
    Ok(Json(LogListResponse {
        items,
        page,
        per_page,
        total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_and_missing_filters_are_ignored() {
        let params = LogFilterParams {
            component: Some("  ".into()),
            command: None,
        };

        assert_eq!(params.filter(), LogFilter::default());
    }

    #[test]
    fn filter_values_are_trimmed() {
        let params = LogFilterParams {
            component: Some(" trigger ".into()),
            command: Some("\techo\n".into()),
        };

        assert_eq!(
            params.filter(),
            LogFilter {
                component: Some("trigger"),
                command: Some("echo"),
            }
        );
    }
}
