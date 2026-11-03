//! CSRF protection helpers.
//!
//! Botrix uses the double-submit pattern via `axum_csrf`: a signed cookie
//! holds a random token, and the client echoes the matching authenticity
//! token in the `X-CSRF-Token` header on every state-mutating request.
//!
//! Flow:
//! 1. The SPA calls `GET /api/v1/auth/csrf`, which returns the authenticity
//!    token and sets the signed cookie.
//! 2. On each mutating request, the SPA sends that token in `X-CSRF-Token`.
//! 3. Handlers call [`verify_csrf`] to confirm the header matches the cookie.

use axum::http::HeaderMap;
use axum_csrf::CsrfToken;

use super::error::ApiError;

/// Header the client uses to submit the CSRF authenticity token.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Verify the `X-CSRF-Token` header against the request's CSRF cookie.
///
/// Returns [`ApiError::CsrfRejected`] if the header is missing or does not
/// match the signed cookie token.
pub fn verify_csrf(token: &CsrfToken, headers: &HeaderMap) -> Result<(), ApiError> {
    let submitted = headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::CsrfRejected)?;

    token.verify(submitted).map_err(|_| ApiError::CsrfRejected)
}
