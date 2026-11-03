//! Binary entry point — parses CLI arguments and dispatches to subcommands.

#![deny(missing_docs)]
// Phase 0 scaffolding: many types, error variants, and helpers are defined
// ahead of the phase that wires them in (see docs/implementation-plan.md §17).
// This allow is removed as modules are connected in later phases.
#![allow(dead_code)]

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod api;
mod cli;
mod config;
mod db;
mod matrix;
mod queue;
mod server;
mod webhooks;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() {
    // Initialize structured logging from RUST_LOG (default: info).
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Serve(args) => cli::serve(args).await,
        Commands::Config(args) => cli::config_cmd(args).await,
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
