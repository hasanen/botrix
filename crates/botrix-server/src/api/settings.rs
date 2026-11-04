//! Settings API handlers: session settings and password change.
//!
//! All endpoints require an authenticated session. State-mutating endpoints
//! also require a valid CSRF token.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::settings::{
    AppInfo, ChangePasswordRequest, Settings, UpdateSettingsRequest,
};
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::{auth as db_auth, settings as db_settings};
use crate::session;
use crate::state::AppState;

/// Require an authenticated session, returning the username.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

/// `GET /api/v1/settings` — return the current session settings.
pub async fn get_settings(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Settings>, ApiError> {
    require_user(&session).await?;
    let settings = db_settings::get_settings(&state.pool)?;
    Ok(Json(settings))
}

/// `PUT /api/v1/settings` — update session timeouts.
pub async fn update_settings(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<Json<Settings>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    if req.idle_timeout_secs <= 0 || req.absolute_lifetime_secs <= 0 {
        return Err(ApiError::BadRequest("timeouts must be positive".to_owned()));
    }
    if req.idle_timeout_secs > req.absolute_lifetime_secs {
        return Err(ApiError::BadRequest(
            "idle timeout must not exceed absolute lifetime".to_owned(),
        ));
    }
    if req.absolute_lifetime_secs > db_settings::MAX_ABSOLUTE_LIFETIME_SECS {
        return Err(ApiError::BadRequest(
            "absolute lifetime must not exceed 30 days".to_owned(),
        ));
    }

    let settings = Settings {
        idle_timeout_secs: req.idle_timeout_secs,
        absolute_lifetime_secs: req.absolute_lifetime_secs,
    };
    db_settings::update_settings(&state.pool, &settings)?;
    Ok(Json(settings))
}

/// `GET /api/v1/settings/info` — application version and build info.
///
/// The build hash is taken from the `BOTRIX_BUILD_HASH` environment variable
/// at compile time and is `null` when the binary was built without it.
pub async fn app_info(session: Session) -> Result<Json<AppInfo>, ApiError> {
    require_user(&session).await?;
    Ok(Json(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build_hash: option_env!("BOTRIX_BUILD_HASH").map(str::to_owned),
    }))
}

/// `POST /api/v1/settings/password` — change the admin password.
pub async fn change_password(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let username = require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    if req.new_password.len() < 8 {
        return Err(ApiError::BadRequest(
            "new password must be at least 8 characters".to_owned(),
        ));
    }

    db_auth::change_password(
        &state.pool,
        &username,
        &req.current_password,
        &req.new_password,
    )?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}
