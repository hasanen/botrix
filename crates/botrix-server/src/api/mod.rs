//! REST API route handlers for the web UI.
//!
//! All handlers live under `/api/v1/`. Auth and settings are implemented in
//! Phase 3; the remaining modules are filled in during Phase 7.

pub mod auth;
pub mod csrf;
pub mod error;
pub mod logs;
pub mod matrix;
pub mod settings;
pub mod webhooks;
