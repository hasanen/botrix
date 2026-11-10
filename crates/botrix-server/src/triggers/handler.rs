//! Runs a parsed bot command: authorization, webhook call, and reply text.
//!
//! Triggers, access profiles and headers come from the loaded configuration;
//! only secret values are read from the database. This module has no Matrix
//! SDK types; the caller supplies the parsed command and the message fields as
//! plain strings and sends back the returned reply. Logged messages never
//! contain secret values or webhook response bodies.

use std::collections::HashMap;
use std::sync::Arc;

use botrix_common::config::{Config, TriggerConfig};
use botrix_common::types::LogLevel;

use crate::db::logs::{record, Component};
use crate::db::{triggers, Pool};
use crate::matrix::command::ParsedCommand;
use crate::matrix::crypto::decrypt_trigger_secret;

use super::authz::is_authorized;
use super::help::build_help;
use super::lookup::{find_trigger, profile_for};
use super::respond::render_reply;
use super::webhook::{
    resolve_secrets, CommandPayload, WebhookCaller, WebhookError, WebhookRequest,
};

/// Reply for an unknown or disabled command.
const REPLY_UNKNOWN: &str = "unknown command";
/// Reply when the sender may not run the command in this room.
const REPLY_UNAUTHORIZED: &str = "not authorized";
/// Reply for any internal or webhook failure.
const REPLY_FAILED: &str = "Command failed";
/// The built-in command that lists available triggers.
const HELP_COMMAND: &str = "help";

/// Shared dependencies for handling commands.
#[derive(Clone)]
pub struct HandlerContext {
    /// Database pool (holds the encrypted trigger secrets and the logs).
    pub pool: Pool,
    /// The loaded configuration: triggers, access profiles and headers.
    pub config: Arc<Config>,
    /// `BOTRIX_CRYPTO_PASSPHRASE`, used to decrypt trigger secrets. Never logged.
    pub crypto_passphrase: Arc<str>,
    /// Performs the webhook HTTP call.
    pub caller: Arc<dyn WebhookCaller>,
}

/// The parts of an incoming Matrix message the handler needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    /// The original message text.
    pub raw_message: String,
    /// Matrix user ID of the sender.
    pub sender: String,
    /// Matrix room ID.
    pub room_id: String,
    /// Matrix event ID.
    pub event_id: String,
}

/// Handles `parsed` sent in `msg` and returns the reply text.
pub async fn handle_command(
    ctx: &HandlerContext,
    parsed: &ParsedCommand,
    msg: &IncomingMessage,
) -> String {
    let log = |level: LogLevel, message: &str, extra: Option<&str>| {
        log_event(ctx, level, message, parsed, msg, extra);
    };

    if parsed.command == HELP_COMMAND {
        return build_help(&ctx.config, &msg.room_id, &msg.sender);
    }

    let (command, trigger) = match find_trigger(&ctx.config, &parsed.command) {
        Some((command, trigger)) if trigger.enabled => (command, trigger),
        _ => {
            log(LogLevel::Info, "unknown command", None);
            return REPLY_UNKNOWN.to_owned();
        }
    };

    let profile = profile_for(&ctx.config, trigger);
    if !is_authorized(trigger, profile, &msg.room_id, &msg.sender) {
        log(LogLevel::Warn, "unauthorized command", None);
        return REPLY_UNAUTHORIZED.to_owned();
    }

    let payload = CommandPayload {
        command: parsed.command.clone(),
        arguments: parsed.arguments.clone(),
        args: parsed.args.clone(),
        raw_message: msg.raw_message.clone(),
        sender: msg.sender.clone(),
        room_id: msg.room_id.clone(),
        event_id: msg.event_id.clone(),
    };
    let request = match build_request(ctx, &command, trigger, &payload) {
        Ok(r) => r,
        Err(e) => {
            log(LogLevel::Error, "command setup failed", Some(&e));
            return REPLY_FAILED.to_owned();
        }
    };

    let result = ctx.caller.call(request).await;
    match &result {
        Ok(resp) if (200..300).contains(&resp.status) => {
            log(LogLevel::Info, "command succeeded", None);
        }
        Ok(resp) => {
            let detail = format!("status {}", resp.status);
            log(LogLevel::Error, "command failed", Some(&detail));
        }
        Err(e) => log(LogLevel::Error, "command failed", Some(&e.to_string())),
    }
    render_reply(&result, trigger.response_template.as_deref())
}

/// Decrypts the declared secrets, resolves header placeholders, and builds the request.
///
/// Only secrets declared on the trigger are read; stored values for names the
/// config no longer declares are ignored. Errors are descriptions safe to log
/// (secret names at most, never values).
fn build_request(
    ctx: &HandlerContext,
    command: &str,
    trigger: &TriggerConfig,
    payload: &CommandPayload,
) -> Result<WebhookRequest, String> {
    let mut secrets = HashMap::new();
    for s in triggers::list_secrets(&ctx.pool, command).map_err(|e| e.to_string())? {
        if !trigger.secrets.contains(&s.name) {
            continue;
        }
        let value = decrypt_trigger_secret(&ctx.crypto_passphrase, &s.ciphertext)
            .map_err(|_| format!("cannot decrypt secret {}", s.name))?;
        secrets.insert(s.name, value);
    }

    let mut headers = Vec::with_capacity(trigger.headers.len());
    for (name, template) in &trigger.headers {
        let value = resolve_secrets(template, &secrets).map_err(|e: WebhookError| e.to_string())?;
        headers.push((name.clone(), value));
    }

    let body = serde_json::to_value(payload).map_err(|e| e.to_string())?;

    Ok(WebhookRequest {
        method: trigger.method.clone(),
        url: trigger.url.clone(),
        headers,
        body,
    })
}

/// Records a trigger event with command, sender, and room as details.
fn log_event(
    ctx: &HandlerContext,
    level: LogLevel,
    message: &str,
    parsed: &ParsedCommand,
    msg: &IncomingMessage,
    extra: Option<&str>,
) {
    let mut details = format!(
        "command={} sender={} room={}",
        parsed.command, msg.sender, msg.room_id
    );
    if let Some(extra) = extra {
        details.push_str(" error=");
        details.push_str(extra);
    }
    record(
        &ctx.pool,
        level,
        Component::Trigger,
        message,
        Some(&details),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history::list_logs;
    use crate::db::test_pool;
    use crate::matrix::crypto::encrypt_trigger_secret;
    use crate::triggers::webhook::{FakeCaller, WebhookResponse};
    use botrix_common::config::MatrixConfig;

    const PASS: &str = "test-passphrase";
    const SECRET: &str = "s3cr3t-value-xyz";

    struct Fixture {
        ctx: HandlerContext,
        fake: Arc<FakeCaller>,
    }

    fn fixture(triggers: Vec<(&str, TriggerConfig)>) -> Fixture {
        let fake = Arc::new(FakeCaller::new());
        let config = Config {
            matrix: MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations: Default::default(),
            },
            webhooks: Default::default(),
            triggers: triggers
                .into_iter()
                .map(|(name, t)| (name.to_owned(), t))
                .collect(),
            access_profiles: Default::default(),
            server: Default::default(),
        };
        let ctx = HandlerContext {
            pool: test_pool(),
            config: Arc::new(config),
            crypto_passphrase: Arc::from(PASS),
            caller: fake.clone(),
        };
        Fixture { ctx, fake }
    }

    fn trig(enabled: bool, template: Option<&str>, rooms: &[&str]) -> TriggerConfig {
        TriggerConfig {
            description: "Does a thing".into(),
            enabled,
            method: "POST".into(),
            url: "https://example.com/hook".into(),
            headers: Default::default(),
            secrets: Vec::new(),
            response_template: template.map(String::from),
            access_profile: None,
            allowed_rooms: rooms.iter().map(|s| s.to_string()).collect(),
            allowed_users: Vec::new(),
        }
    }

    /// A trigger with an `Authorization` header built from the declared `TOKEN` secret.
    fn trig_with_secret(template: Option<&str>) -> TriggerConfig {
        let mut t = trig(true, template, &[]);
        t.secrets = vec!["TOKEN".into()];
        t.headers
            .insert("Authorization".into(), "Bearer {{secret:TOKEN}}".into());
        t
    }

    fn store_secret(pool: &Pool, command: &str, name: &str, value: &str) {
        let cipher = encrypt_trigger_secret(PASS, value).unwrap();
        triggers::upsert_secret(pool, command, name, &cipher).unwrap();
    }

    fn parsed(command: &str, arguments: &str) -> ParsedCommand {
        ParsedCommand {
            command: command.into(),
            arguments: arguments.into(),
            args: arguments.split_whitespace().map(String::from).collect(),
        }
    }

    fn message() -> IncomingMessage {
        IncomingMessage {
            raw_message: "@bot: deploy prod".into(),
            sender: "@alice:x".into(),
            room_id: "!room:x".into(),
            event_id: "$ev".into(),
        }
    }

    fn ok(body: &str) -> WebhookResponse {
        WebhookResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        }
    }

    fn logs(pool: &Pool) -> Vec<(String, String, Option<String>)> {
        let (items, _) = list_logs(pool, 1, 100).unwrap();
        items
            .into_iter()
            .map(|l| (l.component, l.message, l.details))
            .collect()
    }

    fn assert_no_secret_in_logs(pool: &Pool) {
        for (_, message, details) in logs(pool) {
            assert!(!message.contains(SECRET));
            assert!(!details.unwrap_or_default().contains(SECRET));
        }
    }

    #[tokio::test]
    async fn help_lists_visible_triggers() {
        // Arrange
        let f = fixture(vec![
            ("deploy", trig(true, None, &[])),
            ("hidden", trig(true, None, &["!other:x"])),
        ]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("help", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Available commands:\ndeploy - Does a thing");
        assert!(f.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn unknown_command_replies_and_logs() {
        // Arrange
        let f = fixture(vec![]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("nope", ""), &message()).await;

        // Assert
        assert_eq!(reply, "unknown command");
        let rows = logs(&f.ctx.pool);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "trigger");
        let details = rows[0].2.clone().unwrap();
        assert!(details.contains("nope") && details.contains("@alice:x"));
        assert!(details.contains("!room:x"));
    }

    #[tokio::test]
    async fn disabled_command_is_unknown() {
        // Arrange
        let f = fixture(vec![("deploy", trig(false, None, &[]))]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "unknown command");
        assert_eq!(logs(&f.ctx.pool).len(), 1);
        assert!(f.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn unauthorized_replies_and_logs() {
        // Arrange
        let f = fixture(vec![("deploy", trig(true, None, &["!other:x"]))]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "not authorized");
        assert_eq!(logs(&f.ctx.pool).len(), 1);
        assert!(f.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn success_json_renders_template_and_resolves_secret() {
        // Arrange
        let f = fixture(vec![("deploy", trig_with_secret(Some("Got ${a.b}")))]);
        store_secret(&f.ctx.pool, "deploy", "TOKEN", SECRET);
        f.fake.push_response(ok(r#"{"a":{"b":"hello"}}"#));

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", "prod now"), &message()).await;

        // Assert
        assert_eq!(reply, "Got hello");
        let reqs = f.fake.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].url, "https://example.com/hook");
        assert_eq!(
            reqs[0].headers,
            vec![("Authorization".to_string(), format!("Bearer {SECRET}"))]
        );
        assert_eq!(reqs[0].body["command"], "deploy");
        assert_eq!(reqs[0].body["arguments"], "prod now");
        assert_eq!(reqs[0].body["args"], serde_json::json!(["prod", "now"]));
        assert_eq!(reqs[0].body["sender"], "@alice:x");
        assert_eq!(reqs[0].body["event_id"], "$ev");
        assert_no_secret_in_logs(&f.ctx.pool);
    }

    #[tokio::test]
    async fn literal_config_headers_are_sent_as_written() {
        // Arrange
        let mut t = trig(true, None, &[]);
        t.headers.insert("X-Source".into(), "botrix".into());
        let f = fixture(vec![("deploy", t)]);
        f.fake.push_response(ok("done"));

        // Act
        handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(
            f.fake.requests()[0].headers,
            vec![("X-Source".to_string(), "botrix".to_string())]
        );
    }

    #[tokio::test]
    async fn command_matching_and_secret_lookup_are_case_insensitive() {
        // Arrange: the config key is mixed case, secrets are stored lowercase.
        let f = fixture(vec![("Deploy", trig_with_secret(None))]);
        store_secret(&f.ctx.pool, "deploy", "TOKEN", SECRET);
        f.fake.push_response(ok("done"));

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command succeeded");
        assert_eq!(
            f.fake.requests()[0].headers,
            vec![("Authorization".to_string(), format!("Bearer {SECRET}"))]
        );
    }

    #[tokio::test]
    async fn success_non_json_replies_generic() {
        // Arrange
        let f = fixture(vec![("deploy", trig(true, Some("Got ${a.b}"), &[]))]);
        f.fake.push_response(ok("plain text"));

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command succeeded");
    }

    #[tokio::test]
    async fn webhook_error_replies_failed_and_logs() {
        // Arrange
        let f = fixture(vec![("deploy", trig_with_secret(None))]);
        store_secret(&f.ctx.pool, "deploy", "TOKEN", SECRET);
        f.fake.push_error(WebhookError::Timeout);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command failed");
        let rows = logs(&f.ctx.pool);
        assert!(rows.iter().any(|(_, m, _)| m == "command failed"));
        assert_no_secret_in_logs(&f.ctx.pool);
    }

    #[tokio::test]
    async fn declared_secret_that_is_not_set_replies_failed_and_names_it_in_the_log() {
        // Arrange: TOKEN is declared and used by a header, but no value is stored.
        let f = fixture(vec![("deploy", trig_with_secret(None))]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command failed");
        assert!(f.fake.requests().is_empty());
        let rows = logs(&f.ctx.pool);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2.clone().unwrap().contains("missing secret: TOKEN"));
    }

    #[tokio::test]
    async fn stored_values_for_undeclared_secret_names_are_ignored() {
        // Arrange: the header references TOKEN, but the trigger does not declare
        // it (config validation prevents this at startup). A stored TOKEN must
        // not satisfy the placeholder.
        let mut t = trig(true, None, &[]);
        t.headers
            .insert("Authorization".into(), "Bearer {{secret:TOKEN}}".into());
        let f = fixture(vec![("deploy", t)]);
        store_secret(&f.ctx.pool, "deploy", "TOKEN", SECRET);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command failed");
        assert!(f.fake.requests().is_empty());
        assert_no_secret_in_logs(&f.ctx.pool);
    }

    #[tokio::test]
    async fn secrets_of_other_commands_are_not_used() {
        // Arrange
        let f = fixture(vec![
            ("deploy", trig_with_secret(None)),
            ("build", trig_with_secret(None)),
        ]);
        store_secret(&f.ctx.pool, "build", "TOKEN", SECRET);

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command failed");
        assert!(f.fake.requests().is_empty());
    }
}
