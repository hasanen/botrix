//! Access profile management API handlers (authenticated).
//!
//! - `GET    /api/v1/access-profiles`      — list profiles
//! - `POST   /api/v1/access-profiles`      — create a profile
//! - `PUT    /api/v1/access-profiles/{id}` — replace a profile
//! - `DELETE /api/v1/access-profiles/{id}` — delete a profile
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::triggers::{AccessProfileDto, AccessProfileInput};
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::access_profiles as db_profiles;
use crate::db::models::AccessProfile;
use crate::session;
use crate::state::AppState;

/// Require an authenticated session.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

fn to_dto(p: AccessProfile) -> AccessProfileDto {
    AccessProfileDto {
        id: p.id,
        name: p.name,
        allowed_rooms: p.allowed_rooms,
        allowed_users: p.allowed_users,
    }
}

/// Trim the name and reject empty values.
fn validated_name(input: &AccessProfileInput) -> Result<&str, ApiError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }
    Ok(name)
}

/// `GET /api/v1/access-profiles` — list all profiles ordered by name.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<AccessProfileDto>>, ApiError> {
    require_user(&session).await?;
    let profiles = db_profiles::list(&state.pool)?;
    Ok(Json(profiles.into_iter().map(to_dto).collect()))
}

/// `POST /api/v1/access-profiles` — create a profile.
pub async fn create(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(input): Json<AccessProfileInput>,
) -> Result<Json<AccessProfileDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let name = validated_name(&input)?;
    let profile = db_profiles::create(
        &state.pool,
        name,
        &input.allowed_rooms,
        &input.allowed_users,
    )?;
    Ok(Json(to_dto(profile)))
}

/// `PUT /api/v1/access-profiles/{id}` — replace a profile.
pub async fn update(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<AccessProfileInput>,
) -> Result<Json<AccessProfileDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let name = validated_name(&input)?;
    let profile = db_profiles::update(
        &state.pool,
        id,
        name,
        &input.allowed_rooms,
        &input.allowed_users,
    )?
    .ok_or_else(|| ApiError::NotFound(format!("access profile {id} not found")))?;
    Ok(Json(to_dto(profile)))
}

/// `DELETE /api/v1/access-profiles/{id}` — delete a profile.
pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    if !db_profiles::delete(&state.pool, id)? {
        return Err(ApiError::NotFound(format!("access profile {id} not found")));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}
