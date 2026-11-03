//! Inbound webhook system.
//!
//! - [`auth`] — Bearer token generation, hashing, and constant-time validation.
//! - [`template`] — the `${variable}` message template engine.
//! - [`handler`] — the `POST /api/v1/webhooks/notify/:identifier` route handler,
//!   which validates the token, enforces the size limit, parses the payload,
//!   renders the template, and persists a `pending` notification.

pub mod auth;
pub mod handler;
pub mod template;
