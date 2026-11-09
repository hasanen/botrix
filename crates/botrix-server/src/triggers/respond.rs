//! Builds the chat reply for a command-trigger webhook outcome.

use crate::triggers::webhook::{WebhookError, WebhookResponse};

/// Reply used when the webhook call failed or returned a non-2xx status.
const REPLY_FAILED: &str = "Command failed";
/// Reply used when the webhook succeeded but no template output is available.
const REPLY_SUCCEEDED: &str = "Command succeeded";

/// Renders the reply text for a webhook `result`.
///
/// Errors and non-2xx statuses give "Command failed". A 2xx response with a
/// `template` and a JSON body renders the template; if rendering fails, or
/// there is no template or the body is not JSON, it gives "Command succeeded".
pub fn render_reply(
    result: &Result<WebhookResponse, WebhookError>,
    template: Option<&str>,
) -> String {
    let response = match result {
        Ok(response) if (200..300).contains(&response.status) => response,
        _ => return REPLY_FAILED.to_owned(),
    };

    let rendered = template.and_then(|template| {
        let json = serde_json::from_slice::<serde_json::Value>(&response.body).ok()?;
        crate::webhooks::template::render(template, &json).ok()
    });

    rendered.unwrap_or_else(|| REPLY_SUCCEEDED.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> Result<WebhookResponse, WebhookError> {
        Ok(WebhookResponse {
            status,
            body: body.as_bytes().to_vec(),
        })
    }

    #[test]
    fn renders_template_from_json_body() {
        let result = response(200, r#"{"a":{"b":"hello"}}"#);

        let reply = render_reply(&result, Some("Got ${a.b}"));

        assert_eq!(reply, "Got hello");
    }

    #[test]
    fn non_json_body_with_template_succeeds_generically() {
        let result = response(200, "plain text");

        let reply = render_reply(&result, Some("Got ${a.b}"));

        assert_eq!(reply, "Command succeeded");
    }

    #[test]
    fn no_template_succeeds_generically() {
        let result = response(204, r#"{"a":1}"#);

        let reply = render_reply(&result, None);

        assert_eq!(reply, "Command succeeded");
    }

    #[test]
    fn template_render_error_falls_back_to_success() {
        let result = response(200, r#"{"a":1}"#);

        let reply = render_reply(&result, Some("${missing.path}"));

        assert_eq!(reply, "Command succeeded");
    }

    #[test]
    fn non_2xx_status_fails() {
        let result = response(500, r#"{"a":{"b":"x"}}"#);

        let reply = render_reply(&result, Some("${a.b}"));

        assert_eq!(reply, "Command failed");
    }

    #[test]
    fn transport_error_fails() {
        let result = Err(WebhookError::Transport);

        let reply = render_reply(&result, Some("${a.b}"));

        assert_eq!(reply, "Command failed");
    }

    #[test]
    fn timeout_error_fails() {
        let result = Err(WebhookError::Timeout);

        let reply = render_reply(&result, None);

        assert_eq!(reply, "Command failed");
    }
}
