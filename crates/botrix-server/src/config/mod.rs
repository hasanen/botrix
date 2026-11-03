//! Configuration loading and validation.
//!
//! Reads a YAML file from disk, deserializes it into [`botrix_common::config::Config`],
//! then runs all validation rules defined in [`validate`]. The process exits with a
//! descriptive error before binding any ports if validation fails.

pub mod validate;

use std::path::Path;

use thiserror::Error;

pub use botrix_common::config::Config;

/// Errors that can occur during configuration loading or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("failed to read config file '{path}': {source}")]
    ReadFile {
        /// Path of the file that could not be read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The configuration file could not be parsed as valid YAML.
    #[error("failed to parse config file '{path}': {source}")]
    ParseYaml {
        /// Path of the file that could not be parsed.
        path: String,
        /// Underlying YAML deserialization error.
        #[source]
        source: serde_yaml::Error,
    },
    /// One or more validation rules failed.
    #[error("invalid configuration: {0}")]
    Validation(#[from] validate::ValidationError),
}

/// Load and validate the configuration file at `path`.
///
/// Returns [`ConfigError`] on any I/O, parse, or validation failure.
pub fn load(path: &str) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.to_owned(),
        source,
    })?;

    let config: Config = serde_yaml::from_str(&raw).map_err(|source| ConfigError::ParseYaml {
        path: path.to_owned(),
        source,
    })?;

    validate::validate(&config)?;

    tracing::info!(path, "configuration loaded and validated");
    Ok(config)
}

/// Resolve the path to a file inside the data directory.
pub fn data_path(data_dir: &str, filename: &str) -> std::path::PathBuf {
    Path::new(data_dir).join(filename)
}
