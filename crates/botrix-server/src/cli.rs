//! CLI argument definitions and top-level command dispatch.

use clap::{Parser, Subcommand};
use thiserror::Error;

/// Errors that can occur during CLI command execution.
#[derive(Debug, Error)]
pub enum CliError {
    /// Configuration loading or validation failed.
    #[error("configuration error: {0}")]
    Config(#[from] crate::config::ConfigError),
    /// Server startup failed.
    #[error("server error: {0}")]
    Server(#[from] crate::server::ServerError),
    /// The application database could not be read.
    #[error("database error: {0}")]
    Database(#[from] crate::db::DbError),
}

/// Botrix — Matrix notification gateway.
#[derive(Debug, Parser)]
#[command(name = "botrix", version, about)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start the HTTP server.
    Serve(ServeArgs),
    /// Configuration utilities.
    Config(ConfigArgs),
    /// Report current Matrix connection and delivery queue status.
    Status(StatusArgs),
}

/// Arguments for the `status` subcommand.
#[derive(Debug, Parser)]
pub struct StatusArgs {
    /// Path to the data directory.
    ///
    /// Defaults to the value of `BOTRIX_DATA_DIR`, or `/data`.
    #[arg(long, env = "BOTRIX_DATA_DIR", default_value = "/data")]
    pub data_dir: String,
}

/// Arguments for the `serve` subcommand.
#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// Path to the configuration file.
    ///
    /// Defaults to the value of `BOTRIX_CONFIG`, or `/config/config.yaml`.
    #[arg(long, env = "BOTRIX_CONFIG", default_value = "/config/config.yaml")]
    pub config: String,

    /// Path to the data directory.
    ///
    /// Defaults to the value of `BOTRIX_DATA_DIR`, or `/data`.
    #[arg(long, env = "BOTRIX_DATA_DIR", default_value = "/data")]
    pub data_dir: String,
}

/// Arguments for the `config` subcommand.
#[derive(Debug, Parser)]
pub struct ConfigArgs {
    /// Config sub-command.
    #[command(subcommand)]
    pub command: ConfigCommands,
}

/// Subcommands under `botrix config`.
#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Validate a configuration file and exit.
    Validate(ValidateArgs),
}

/// Arguments for `botrix config validate`.
#[derive(Debug, Parser)]
pub struct ValidateArgs {
    /// Path to the configuration file to validate.
    #[arg(long, env = "BOTRIX_CONFIG", default_value = "/config/config.yaml")]
    pub config: String,
}

/// Dispatch for `botrix serve`.
pub async fn serve(args: ServeArgs) -> Result<(), CliError> {
    let config = crate::config::load(&args.config)?;
    crate::server::run(config, &args.data_dir).await?;
    Ok(())
}

/// Dispatch for `botrix status`.
pub fn status(args: StatusArgs) -> Result<(), CliError> {
    let db_path = format!("{}/botrix.db", args.data_dir);
    let pool = crate::db::open_read_only(&db_path)?;
    let report = crate::status::StatusReport::load(&pool)?;
    println!("{report}");
    Ok(())
}

/// Dispatch for `botrix config ...`.
pub async fn config_cmd(args: ConfigArgs) -> Result<(), CliError> {
    match args.command {
        ConfigCommands::Validate(v) => {
            crate::config::load(&v.config)?;
            println!("Configuration is valid.");
            Ok(())
        }
    }
}
