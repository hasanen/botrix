//! Authentication and session API handlers.
//!
//! Endpoints:
//! - `GET  /api/v1/auth/csrf`   — issue a CSRF token (sets the signed cookie)
//! - `POST /api/v1/setup`       — first-run admin creation (once only)
//! - `POST /api/v1/auth/login`  — authenticate and start a session
//! - `POST /api/v1/auth/logout` — end the current session
//! - `GET  /api/v1/auth/me`     — return the current session info

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::auth::{CsrfResponse, LoginRequest, SessionInfo, SetupRequest};
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::auth as db_auth;
use crate::session;
use crate::state::AppState;

/// `GET /api/v1/auth/csrf` — return a CSRF token and set the signed cookie.
///
/// Returns the `CsrfToken` as part of the response so its `IntoResponseParts`
/// impl writes the cookie, alongside the JSON body carrying the authenticity
/// token for the client to echo in `X-CSRF-Token`.
pub async fn csrf(token: CsrfToken) -> Result<(CsrfToken, Json<CsrfResponse>), ApiError> {
    let authenticity_token = token.authenticity_token().map_err(|_| ApiError::Internal)?;
    Ok((
        token,
        Json(CsrfResponse {
            token: authenticity_token,
        }),
    ))
}

/// `POST /api/v1/setup` — create the first admin account.
///
/// Rejected with `409 Conflict` if an admin already exists.
pub async fn setup(
    State(state): State<AppState>,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<SetupRequest>,
) -> Result<Json<SessionInfo>, ApiError> {
    verify_csrf(&token, &headers)?;
    validate_credentials(&req.username, &req.password)?;

    db_auth::create_admin(&state.pool, &req.username, &req.password)?;

    Ok(Json(SessionInfo {
        username: req.username,
    }))
}

/// `POST /api/v1/auth/login` — authenticate and establish a session.
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<SessionInfo>, ApiError> {
    verify_csrf(&token, &headers)?;

    let username = db_auth::verify_login(&state.pool, &req.username, &req.password)?;
    session::log_in(&session, &username).await?;

    Ok(Json(SessionInfo { username }))
}

/// `POST /api/v1/auth/logout` — end the current session.
pub async fn logout(
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    verify_csrf(&token, &headers)?;
    session::log_out(&session).await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

/// `GET /api/v1/auth/me` — return the current session info, or 401.
pub async fn me(session: Session) -> Result<Json<SessionInfo>, ApiError> {
    match session::current_user(&session).await {
        Some(username) => Ok(Json(SessionInfo { username })),
        None => Err(ApiError::Unauthorized),
    }
}

/// Minimal credential validation for the setup endpoint.
fn validate_credentials(username: &str, password: &str) -> Result<(), ApiError> {
    if username.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "username must not be empty".to_owned(),
        ));
    }
    if password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_owned(),
        ));
    }
    Ok(())
}
