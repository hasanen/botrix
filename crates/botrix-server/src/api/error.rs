//! Shared HTTP error type for API handlers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

/// An error returned by an API handler, mapped to an HTTP status code.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The request is missing authentication or has an invalid session.
    #[error("unauthorized")]
    Unauthorized,
    /// The CSRF token is missing or invalid.
    #[error("invalid CSRF token")]
    CsrfRejected,
    /// The request conflicts with current state (e.g. setup already done).
    #[error("{0}")]
    Conflict(String),
    /// The requested resource does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The request body failed validation.
    #[error("{0}")]
    BadRequest(String),
    /// An unexpected internal error occurred.
    #[error("internal server error")]
    Internal,
    /// A dependency Botrix calls out to (e.g. the Matrix homeserver) failed.
    /// The message is operator-facing and safe to show.
    #[error("{0}")]
    Upstream(String),
}

/// JSON body returned for errors.
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::CsrfRejected => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        let body = ErrorBody {
            error: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

impl From<crate::db::auth::AuthError> for ApiError {
    fn from(err: crate::db::auth::AuthError) -> Self {
        use crate::db::auth::AuthError;
        match err {
            AuthError::InvalidCredentials => ApiError::Unauthorized,
            AuthError::AlreadyInitialized => {
                ApiError::Conflict("an admin account already exists".to_owned())
            }
            // Hashing failures and database errors are internal.
            AuthError::Hash(_) | AuthError::Database(_) => ApiError::Internal,
        }
    }
}

impl From<crate::db::DbError> for ApiError {
    fn from(_: crate::db::DbError) -> Self {
        ApiError::Internal
    }
}

impl From<crate::matrix::MatrixError> for ApiError {
    fn from(err: crate::matrix::MatrixError) -> Self {
        use crate::matrix::MatrixError;
        match err {
            // Caller-facing misconfiguration maps to 400.
            MatrixError::UnknownDestination(msg) => ApiError::BadRequest(msg),
            MatrixError::InvalidRoomId { room_id, reason } => {
                ApiError::BadRequest(format!("invalid room ID '{room_id}': {reason}"))
            }
            MatrixError::InvalidRecoveryKey(msg) => {
                ApiError::BadRequest(format!("recovery key failed: {msg}"))
            }
            // No active session is a conflict with the requested operation.
            MatrixError::NoSession => ApiError::Conflict("no active Matrix session".to_owned()),
            // Everything else (SDK, crypto, client build, DB) is internal.
            _ => ApiError::Internal,
        }
    }
}

impl From<tower_sessions::session::Error> for ApiError {
    fn from(_: tower_sessions::session::Error) -> Self {
        ApiError::Internal
    }
}

impl From<crate::db::triggers::TriggerError> for ApiError {
    fn from(err: crate::db::triggers::TriggerError) -> Self {
        use crate::db::triggers::TriggerError;
        match err {
            TriggerError::DuplicateCommand(_) => ApiError::Conflict(err.to_string()),
            TriggerError::Database(_) | TriggerError::Json(_) => ApiError::Internal,
        }
    }
}

impl From<crate::db::access_profiles::AccessProfileError> for ApiError {
    fn from(err: crate::db::access_profiles::AccessProfileError) -> Self {
        use crate::db::access_profiles::AccessProfileError;
        match err {
            AccessProfileError::DuplicateName(_) => ApiError::Conflict(err.to_string()),
            AccessProfileError::Database(_) | AccessProfileError::Json(_) => ApiError::Internal,
        }
    }
}
