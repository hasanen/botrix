//! Runs a parsed bot command: authorization, webhook call, and reply text.
//!
//! This module has no Matrix SDK types; the caller supplies the parsed command
//! and the message fields as plain strings and sends back the returned reply.
//! Logged messages never contain secret values or webhook response bodies.

use std::collections::HashMap;
use std::sync::Arc;

use botrix_common::types::LogLevel;

use crate::db::logs::{record, Component};
use crate::db::models::{AccessProfile, CommandTrigger};
use crate::db::{access_profiles, triggers, Pool};
use crate::matrix::command::ParsedCommand;
use crate::matrix::crypto::decrypt_trigger_secret;

use super::authz::is_authorized;
use super::help::build_help;
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
    /// Database pool.
    pub pool: Pool,
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
        return match help_reply(ctx, msg) {
            Ok(text) => text,
            Err(e) => {
                log(LogLevel::Error, "help failed", Some(&e));
                REPLY_FAILED.to_owned()
            }
        };
    }

    let trigger = match triggers::get_by_command(&ctx.pool, &parsed.command) {
        Ok(Some(t)) if t.enabled => t,
        Ok(_) => {
            log(LogLevel::Info, "unknown command", None);
            return REPLY_UNKNOWN.to_owned();
        }
        Err(e) => {
            log(
                LogLevel::Error,
                "trigger lookup failed",
                Some(&e.to_string()),
            );
            return REPLY_FAILED.to_owned();
        }
    };

    let profile = match load_profile(&ctx.pool, &trigger) {
        Ok(p) => p,
        Err(e) => {
            log(LogLevel::Error, "access profile lookup failed", Some(&e));
            return REPLY_FAILED.to_owned();
        }
    };
    if !is_authorized(&trigger, profile.as_ref(), &msg.room_id, &msg.sender) {
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
    let request = match build_request(ctx, &trigger, &payload) {
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

/// Builds the help text over all triggers.
fn help_reply(ctx: &HandlerContext, msg: &IncomingMessage) -> Result<String, String> {
    let all = triggers::list(&ctx.pool).map_err(|e| e.to_string())?;
    let mut entries = Vec::with_capacity(all.len());
    for t in all {
        let profile = load_profile(&ctx.pool, &t)?;
        entries.push((t, profile));
    }
    Ok(build_help(&entries, &msg.room_id, &msg.sender))
}

/// Loads the trigger's access profile, if it references one.
fn load_profile(pool: &Pool, trigger: &CommandTrigger) -> Result<Option<AccessProfile>, String> {
    match trigger.access_profile_id {
        Some(id) => access_profiles::get_by_id(pool, id).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Decrypts secrets, resolves header placeholders, and builds the request.
///
/// Errors are descriptions safe to log (secret names at most, never values).
pub(crate) fn build_request(
    ctx: &HandlerContext,
    trigger: &CommandTrigger,
    payload: &CommandPayload,
) -> Result<WebhookRequest, String> {
    let mut secrets = HashMap::new();
    for s in triggers::list_secrets(&ctx.pool, trigger.id).map_err(|e| e.to_string())? {
        let value = decrypt_trigger_secret(&ctx.crypto_passphrase, &s.ciphertext)
            .map_err(|_| format!("cannot decrypt secret {}", s.name))?;
        secrets.insert(s.name, value);
    }

    let mut headers = Vec::new();
    for h in triggers::list_headers(&ctx.pool, trigger.id).map_err(|e| e.to_string())? {
        let value = resolve_secrets(&h.value, &secrets).map_err(|e: WebhookError| e.to_string())?;
        headers.push((h.name, value));
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
    use crate::db::triggers::TriggerInput;
    use crate::matrix::crypto::encrypt_trigger_secret;
    use crate::triggers::webhook::{FakeCaller, WebhookResponse};

    const PASS: &str = "test-passphrase";
    const SECRET: &str = "s3cr3t-value-xyz";

    struct Fixture {
        ctx: HandlerContext,
        fake: Arc<FakeCaller>,
    }

    fn fixture() -> Fixture {
        let fake = Arc::new(FakeCaller::new());
        let ctx = HandlerContext {
            pool: test_pool(),
            crypto_passphrase: Arc::from(PASS),
            caller: fake.clone(),
        };
        Fixture { ctx, fake }
    }

    fn add_trigger(
        pool: &Pool,
        command: &str,
        enabled: bool,
        template: Option<&str>,
        rooms: &[String],
    ) -> CommandTrigger {
        triggers::create(
            pool,
            &TriggerInput {
                command,
                description: "Does a thing",
                enabled,
                method: "POST",
                url: "https://example.com/hook",
                response_template: template,
                access_profile_id: None,
                allowed_rooms: rooms,
                allowed_users: &[],
            },
        )
        .unwrap()
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

    fn add_secret_header(pool: &Pool, trigger_id: i64) {
        let cipher = encrypt_trigger_secret(PASS, SECRET).unwrap();
        triggers::upsert_secret(pool, trigger_id, "TOKEN", &cipher).unwrap();
        triggers::set_headers(
            pool,
            trigger_id,
            &[("Authorization".into(), "Bearer {{secret:TOKEN}}".into())],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn help_lists_visible_triggers() {
        // Arrange
        let f = fixture();
        add_trigger(&f.ctx.pool, "deploy", true, None, &[]);
        add_trigger(&f.ctx.pool, "hidden", true, None, &["!other:x".to_string()]);

        // Act
        let reply = handle_command(&f.ctx, &parsed("help", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Available commands:\ndeploy - Does a thing");
        assert!(f.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn unknown_command_replies_and_logs() {
        // Arrange
        let f = fixture();

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
        let f = fixture();
        add_trigger(&f.ctx.pool, "deploy", false, None, &[]);

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
        let f = fixture();
        add_trigger(&f.ctx.pool, "deploy", true, None, &["!other:x".to_string()]);

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
        let f = fixture();
        let t = add_trigger(&f.ctx.pool, "deploy", true, Some("Got ${a.b}"), &[]);
        add_secret_header(&f.ctx.pool, t.id);
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
    async fn success_non_json_replies_generic() {
        // Arrange
        let f = fixture();
        add_trigger(&f.ctx.pool, "deploy", true, Some("Got ${a.b}"), &[]);
        f.fake.push_response(ok("plain text"));

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command succeeded");
    }

    #[tokio::test]
    async fn webhook_error_replies_failed_and_logs() {
        // Arrange
        let f = fixture();
        let t = add_trigger(&f.ctx.pool, "deploy", true, None, &[]);
        add_secret_header(&f.ctx.pool, t.id);
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
    async fn missing_secret_replies_failed_and_logs() {
        // Arrange
        let f = fixture();
        let t = add_trigger(&f.ctx.pool, "deploy", true, None, &[]);
        triggers::set_headers(
            &f.ctx.pool,
            t.id,
            &[("Authorization".into(), "Bearer {{secret:NOPE}}".into())],
        )
        .unwrap();

        // Act
        let reply = handle_command(&f.ctx, &parsed("deploy", ""), &message()).await;

        // Assert
        assert_eq!(reply, "Command failed");
        assert!(f.fake.requests().is_empty());
        assert_eq!(logs(&f.ctx.pool).len(), 1);
    }
}
