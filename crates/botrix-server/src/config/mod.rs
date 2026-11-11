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
            triggers: Default::default(),
            access_profiles: Default::default(),
            server: Default::default(),
        }
    }

    const BASE_YAML: &str = "matrix:\n  homeserver: https://matrix.example.com\n";

    fn parse(extra: &str) -> Result<Config, serde_yaml::Error> {
        serde_yaml::from_str(&format!("{BASE_YAML}{extra}"))
    }

    #[test]
    fn config_without_triggers_or_profiles_still_parses() {
        // Arrange / Act
        let cfg = parse("").unwrap();

        // Assert
        assert!(cfg.triggers.is_empty());
        assert!(cfg.access_profiles.is_empty());
    }

    #[test]
    fn parses_triggers_and_profiles_with_defaults() {
        // Arrange
        let yaml = "\
access_profiles:
  ops:
    allowed_rooms: [\"!ops:example.com\"]
triggers:
  deploy:
    method: POST
    url: https://ci.example.com/hook
    headers:
      Authorization: \"Bearer {{secret:TOKEN}}\"
    secrets: [TOKEN]
    access_profile: ops
";

        // Act
        let cfg = parse(yaml).unwrap();

        // Assert
        let t = &cfg.triggers["deploy"];
        assert!(t.enabled, "enabled defaults to true");
        assert_eq!(t.description, "");
        assert_eq!(t.secrets, ["TOKEN"]);
        assert_eq!(t.headers["Authorization"], "Bearer {{secret:TOKEN}}");
        assert_eq!(t.access_profile.as_deref(), Some("ops"));
        assert!(t.allowed_rooms.is_empty() && t.allowed_users.is_empty());
        assert_eq!(
            cfg.access_profiles["ops"].allowed_rooms,
            ["!ops:example.com"]
        );
        assert!(validate::validate(&cfg).is_ok());
    }

    #[test]
    fn rejects_unknown_trigger_keys_so_typos_cannot_drop_restrictions() {
        // Arrange: `allowed_room` (singular) is a typo for `allowed_rooms`.
        let yaml = "\
triggers:
  deploy:
    method: POST
    url: https://ci.example.com/hook
    allowed_room: [\"!ops:example.com\"]
";

        // Act
        let result = parse(yaml);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_access_profile_keys() {
        let result = parse("access_profiles:\n  ops:\n    allowed_user: [\"@a:b.c\"]\n");
        assert!(result.is_err());
    }

    /// The first fenced `yaml` block of `markdown`.
    fn first_yaml_block(markdown: &str) -> &str {
        let after_fence = markdown
            .split_once("```yaml\n")
            .expect("document has a yaml block")
            .1;
        after_fence
            .split_once("```")
            .expect("yaml block is closed")
            .0
    }

    fn assert_documents_a_valid_trigger_setup(cfg: &Config) {
        validate::validate(cfg).expect("example config must be valid");
        let deploy = cfg
            .triggers
            .get("deploy")
            .expect("example defines a `deploy` trigger");
        assert!(
            !deploy.secrets.is_empty(),
            "example shows how to declare secrets"
        );
        assert!(
            deploy.headers.values().any(|v| v.contains("{{secret:")),
            "example shows a header that uses a secret"
        );
        assert!(
            deploy.access_profile.is_some() && !cfg.access_profiles.is_empty(),
            "example shows an access profile"
        );
    }

    #[test]
    fn config_example_file_is_valid_and_documents_triggers() {
        let cfg: Config =
            serde_yaml::from_str(include_str!("../../../../config.example.yaml")).unwrap();

        assert_documents_a_valid_trigger_setup(&cfg);
    }

    #[test]
    fn configuration_doc_full_example_is_valid_and_documents_triggers() {
        let doc = include_str!("../../../../docs/configuration.md");

        let cfg: Config = serde_yaml::from_str(first_yaml_block(doc)).unwrap();

        assert_documents_a_valid_trigger_setup(&cfg);
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
