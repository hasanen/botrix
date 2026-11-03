//! Background notification delivery worker.
//!
//! - [`sender`] — the [`MessageSender`](sender::MessageSender) boundary the
//!   worker delivers through (implemented by the Matrix subsystem in Phase 6).
//! - [`retry`] — pure retry/backoff decision logic and schedule.
//! - [`worker`] — the Tokio polling loop that claims `pending` notifications,
//!   delivers them, and applies the retry decision.

pub mod retry;
pub mod sender;
pub mod worker;
