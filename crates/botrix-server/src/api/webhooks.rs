//! Webhook management API handlers (authenticated).
//!
//! - `GET    /api/v1/webhooks`            — list configured webhooks + token status
//! - `POST   /api/v1/webhooks/:id/token`  — generate a new token (plaintext once)
//! - `DELETE /api/v1/webhooks/:id/token`  — revoke the current token

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::webhooks::{TokenGenerateResponse, WebhookInfo};
use botrix_common::types::LogLevel;
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::logs::{record, Component};
use crate::db::webhooks as db_webhooks;
use crate::session;
use crate::state::AppState;
use crate::webhooks::auth as token_auth;

/// Require an authenticated session.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

/// `GET /api/v1/webhooks` — list all configured webhooks with token status.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<WebhookInfo>>, ApiError> {
    require_user(&session).await?;

    let mut items = Vec::with_capacity(state.config.webhooks.len());
    for (id, cfg) in &state.config.webhooks {
        let has_token = db_webhooks::has_token(&state.pool, id)?;
        items.push(WebhookInfo {
            id: id.clone(),
            enabled: cfg.enabled,
            destination: cfg.destination.clone(),
            has_token,
        });
    }
    // Stable ordering for a predictable UI.
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(items))
}

/// `POST /api/v1/webhooks/:id/token` — generate a new token.
///
/// Returns the plaintext token exactly once; only its hash is stored.
pub async fn generate_token(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TokenGenerateResponse>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    // Only allow tokens for webhooks that exist in config.
    if !state.config.webhooks.contains_key(&id) {
        return Err(ApiError::BadRequest(format!("unknown webhook '{id}'")));
    }

    let plaintext = token_auth::generate_token();
    let hash = token_auth::hash_token(&plaintext);
    db_webhooks::set_token_hash(&state.pool, &id, &hash)?;
    // Only the webhook id is logged, never the token.
    record(
        &state.pool,
        LogLevel::Info,
        Component::Webhook,
        "token generated",
        Some(&id),
    );

    Ok(Json(TokenGenerateResponse { token: plaintext }))
}

/// `DELETE /api/v1/webhooks/:id/token` — revoke the current token.
pub async fn revoke_token(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    let revoked = db_webhooks::revoke_token(&state.pool, &id)?;
    if revoked {
        record(
            &state.pool,
            LogLevel::Info,
            Component::Webhook,
            "token revoked",
            Some(&id),
        );
    }
    Ok(Json(serde_json::json!({ "revoked": revoked })))
}
