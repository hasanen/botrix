//! API request and response types shared between the frontend and backend.
//!
//! All types in this module must be serializable with `serde` and must compile
//! for both native targets and `wasm32-unknown-unknown`.

pub mod auth;
pub mod logs;
pub mod matrix;
pub mod notifications;
pub mod settings;
pub mod webhooks;
