//! Axum route handler for inbound webhook notifications.
//!
//! `POST /api/v1/webhooks/notify/:identifier`
//!
//! Processing order (§6.7, §11.1):
//! token validation → size check → JSON parse → config lookup →
//! template render → sanitize → persist (`pending`) → `202 Accepted`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::auth::{parse_bearer, verify_token};
use super::template;
use crate::db::webhooks as db_webhooks;
use crate::state::AppState;

/// Outcome of processing an inbound webhook, mapped to an HTTP status.
#[derive(Debug)]
enum WebhookResponse {
    /// Accepted and enqueued for delivery (`202`).
    Accepted,
    /// Missing or invalid Bearer token (`401`).
    Unauthorized,
    /// Unknown or disabled webhook identifier (`404`).
    NotFound,
    /// Body exceeded the configured size limit (`413`).
    PayloadTooLarge,
    /// Malformed JSON or template error (`400`).
    BadRequest(String),
    /// Internal persistence failure (`500`).
    Internal,
}

impl IntoResponse for WebhookResponse {
    fn into_response(self) -> Response {
        match self {
            WebhookResponse::Accepted => (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({ "status": "accepted" })),
            )
                .into_response(),
            WebhookResponse::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
            }
            WebhookResponse::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
            WebhookResponse::PayloadTooLarge => {
                (StatusCode::PAYLOAD_TOO_LARGE, "payload too large").into_response()
            }
            WebhookResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            WebhookResponse::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
        }
    }
}

/// Handle an inbound webhook notification.
pub async fn notify(
    State(state): State<AppState>,
    Path(identifier): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&state, &identifier, &headers, &body)
        .await
        .into_response()
}

async fn process(
    state: &AppState,
    identifier: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> WebhookResponse {
    // 1. Look up the webhook config. A missing or disabled webhook is 404,
    //    so we don't reveal which identifiers exist to unauthenticated callers.
    let webhook = match state.config.webhooks.get(identifier) {
        Some(w) if w.enabled => w,
        _ => return WebhookResponse::NotFound,
    };

    // 2. Validate the Bearer token in constant time against the stored hash.
    let presented = parse_bearer(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    let stored_hash = match db_webhooks::get_token_hash(&state.pool, identifier).await {
        Ok(Some(hash)) => hash,
        Ok(None) => return WebhookResponse::Unauthorized,
        Err(err) => {
            tracing::error!(error = %err, identifier, "token lookup failed");
            return WebhookResponse::Internal;
        }
    };
    match presented {
        Some(token) if verify_token(token, &stored_hash) => {}
        _ => return WebhookResponse::Unauthorized,
    }

    // 3. Enforce the body size limit before parsing.
    if body.len() > state.config.server.max_body_bytes {
        return WebhookResponse::PayloadTooLarge;
    }

    // 4. Parse the JSON payload.
    let payload: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(err) => return WebhookResponse::BadRequest(format!("invalid JSON payload: {err}")),
    };

    // 5. Render the message template (also sanitizes substituted values).
    let rendered = match template::render(&webhook.msg_template, &payload) {
        Ok(msg) => msg,
        Err(err) => return WebhookResponse::BadRequest(err.to_string()),
    };

    // 6. Persist the notification as `pending` for the background worker.
    match db_webhooks::insert_notification(&state.pool, identifier, &webhook.destination, &rendered)
        .await
    {
        Ok(_) => WebhookResponse::Accepted,
        Err(err) => {
            tracing::error!(error = %err, identifier, "failed to persist notification");
            WebhookResponse::Internal
        }
    }
}
