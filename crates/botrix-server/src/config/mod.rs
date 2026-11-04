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

    let mut config = config;
    apply_public_url_override(&mut config, std::env::var("BOTRIX_PUBLIC_URL").ok());

    validate::validate(&config)?;

    tracing::info!(path, "configuration loaded and validated");
    Ok(config)
}

/// Apply the `BOTRIX_PUBLIC_URL` environment override onto `config`.
///
/// A non-empty value overrides `server.public_url`; an empty or absent value
/// leaves the file's value untouched. Pulled out as a pure function so the
/// precedence can be tested without touching the process environment.
fn apply_public_url_override(config: &mut Config, env_value: Option<String>) {
    if let Some(value) = env_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.server.public_url = Some(trimmed.to_owned());
        }
    }
}

/// Resolve the path to a file inside the data directory.
pub fn data_path(data_dir: &str, filename: &str) -> std::path::PathBuf {
    Path::new(data_dir).join(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            matrix: botrix_common::config::MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations: Default::default(),
            },
            webhooks: Default::default(),
            server: Default::default(),
        }
    }

    #[test]
    fn env_sets_public_url_when_file_has_none() {
        let mut c = config();
        apply_public_url_override(&mut c, Some("https://botrix.example.com".to_owned()));
        assert_eq!(
            c.server.public_url.as_deref(),
            Some("https://botrix.example.com")
        );
    }

    #[test]
    fn env_overrides_the_file_value() {
        let mut c = config();
        c.server.public_url = Some("https://from-file.example.com".to_owned());
        apply_public_url_override(&mut c, Some("https://from-env.example.com".to_owned()));
        assert_eq!(
            c.server.public_url.as_deref(),
            Some("https://from-env.example.com")
        );
    }

    #[test]
    fn absent_or_empty_env_keeps_the_file_value() {
        let mut c = config();
        c.server.public_url = Some("https://from-file.example.com".to_owned());
        apply_public_url_override(&mut c, None);
        apply_public_url_override(&mut c, Some("   ".to_owned()));
        assert_eq!(
            c.server.public_url.as_deref(),
            Some("https://from-file.example.com")
        );
    }
}
