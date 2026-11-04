//! Botrix server library.
//!
//! Contains all business logic for the Botrix notification gateway:
//! configuration loading, database access, the Matrix integration, the
//! webhook system, the background delivery queue, the REST API, and the
//! HTTP server assembly. The `botrix` binary is a thin wrapper over this
//! library (argument parsing and process exit handling only).

#![deny(missing_docs)]
// Phased scaffolding: some types, error variants, and helpers are defined
// ahead of the phase that wires them in (see docs/implementation-plan.md §17).
// This allow is removed as modules are connected in later phases.
#![allow(dead_code)]

pub mod api;
pub mod assets;
pub mod cli;
pub mod config;
pub mod db;
pub mod matrix;
pub mod queue;
pub mod server;
pub mod session;
pub mod state;
pub mod status;
pub mod webhooks;
