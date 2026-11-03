//! Binary entry point — parses CLI arguments and dispatches to subcommands.
//!
//! All logic lives in the `botrix_server` library crate; this binary only
//! initializes logging, parses arguments, and maps errors to an exit code.

#![deny(missing_docs)]

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use botrix_server::cli::{self, Cli, Commands};

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
