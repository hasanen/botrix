//! Template engine — resolves `${variable.path}` references in message templates.
//!
//! See §8 of the implementation plan for the full specification.

use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Errors returned by the template renderer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    /// A variable path referenced in the template was not found in the payload.
    #[error("template variable '${{{}}}' not found in payload", .path)]
    MissingVariable { path: String },
    /// A variable path resolved to a non-scalar JSON value.
    #[error("template variable '${{{}}}' resolved to a non-scalar value (array or object)", .path)]
    NonScalarValue { path: String },
}

/// Maximum length of a single substituted value, in characters.
const MAX_VALUE_CHARS: usize = 2_048;
/// Maximum length of the fully rendered message, in characters.
const MAX_MESSAGE_CHARS: usize = 32_768;

/// Render `template` by substituting `${path}` variables from `payload`.
///
/// # Errors
///
/// Returns [`TemplateError::MissingVariable`] if a referenced path is absent
/// from `payload`, or [`TemplateError::NonScalarValue`] if the resolved value
/// is an object or array.
pub fn render(template: &str, payload: &serde_json::Value) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find("${") {
        // Append the literal segment before the variable reference.
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..]; // skip "${"

        let end = rest.find('}').ok_or_else(|| {
            // Treat unclosed "${..." as a literal — no error, append as-is.
            // (The template validator in config validation catches this earlier.)
            TemplateError::MissingVariable {
                path: rest.to_owned(),
            }
        })?;

        let path_str = &rest[..end];
        rest = &rest[end + 1..]; // skip "}"

        let value = resolve_path(path_str, payload)?;
        let sanitized = sanitize_value(&value, path_str);
        output.push_str(&sanitized);
    }

    // Append any remaining literal text after the last variable.
    output.push_str(rest);

    // Truncate the final message if it exceeds the limit.
    if output.chars().count() > MAX_MESSAGE_CHARS {
        tracing::warn!(
            max = MAX_MESSAGE_CHARS,
            "rendered message truncated to max length"
        );
        output = output.chars().take(MAX_MESSAGE_CHARS).collect();
    }

    Ok(output)
}

/// Walk `payload` using the dot-separated `path` and return the scalar value.
fn resolve_path(path: &str, payload: &serde_json::Value) -> Result<String, TemplateError> {
    let mut current = payload;

    for segment in path.split('.') {
        current = if let Ok(index) = segment.parse::<usize>() {
            current
                .as_array()
                .and_then(|arr| arr.get(index))
                .ok_or_else(|| TemplateError::MissingVariable {
                    path: path.to_owned(),
                })?
        } else {
            current
                .as_object()
                .and_then(|obj| obj.get(segment))
                .ok_or_else(|| TemplateError::MissingVariable {
                    path: path.to_owned(),
                })?
        };
    }

    coerce_to_string(current, path)
}

/// Coerce a scalar JSON value to a `String`.
fn coerce_to_string(value: &serde_json::Value, path: &str) -> Result<String, TemplateError> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok("null".to_owned()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err(TemplateError::NonScalarValue {
                path: path.to_owned(),
            })
        }
    }
}

/// Sanitize a substituted value before it is inserted into the message.
///
/// - Strips ASCII control characters (except `\n` and `\t`).
/// - Applies Unicode NFC normalization.
/// - Truncates to [`MAX_VALUE_CHARS`] characters.
fn sanitize_value(value: &str, path: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_ascii_control())
        .collect::<String>()
        .nfc()
        .collect();

    if cleaned.chars().count() > MAX_VALUE_CHARS {
        tracing::warn!(
            path,
            max = MAX_VALUE_CHARS,
            "template variable value truncated"
        );
        cleaned.chars().take(MAX_VALUE_CHARS).collect()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn simple_top_level_variable() {
        let payload = json!({ "name": "Alice" });
        assert_eq!(render("Hello ${name}!", &payload).unwrap(), "Hello Alice!");
    }

    #[test]
    fn nested_field() {
        let payload = json!({ "image": { "tag": "1.2.3" } });
        assert_eq!(render("Tag: ${image.tag}", &payload).unwrap(), "Tag: 1.2.3");
    }

    #[test]
    fn array_index_access() {
        let payload = json!({ "items": ["a", "b", "c"] });
        assert_eq!(render("${items.1}", &payload).unwrap(), "b");
    }

    #[test]
    fn number_coercion() {
        let payload = json!({ "count": 42 });
        assert_eq!(render("Count: ${count}", &payload).unwrap(), "Count: 42");
    }

    #[test]
    fn bool_coercion() {
        let payload = json!({ "ok": true });
        assert_eq!(render("${ok}", &payload).unwrap(), "true");
    }

    #[test]
    fn null_coercion() {
        let payload = json!({ "val": null });
        assert_eq!(render("${val}", &payload).unwrap(), "null");
    }

    #[test]
    fn missing_variable_returns_error() {
        let payload = json!({ "name": "Alice" });
        assert!(matches!(
            render("${missing}", &payload),
            Err(TemplateError::MissingVariable { path }) if path == "missing"
        ));
    }

    #[test]
    fn non_scalar_returns_error() {
        let payload = json!({ "obj": { "key": "val" } });
        assert!(matches!(
            render("${obj}", &payload),
            Err(TemplateError::NonScalarValue { path }) if path == "obj"
        ));
    }

    #[test]
    fn strips_ascii_control_chars() {
        let payload = json!({ "val": "hello\x00world" });
        assert_eq!(render("${val}", &payload).unwrap(), "helloworld");
    }

    #[test]
    fn preserves_newline_and_tab() {
        let payload = json!({ "val": "line1\nline2\ttab" });
        assert_eq!(render("${val}", &payload).unwrap(), "line1\nline2\ttab");
    }

    #[test]
    fn literal_text_passthrough() {
        let payload = json!({});
        assert_eq!(
            render("no variables here", &payload).unwrap(),
            "no variables here"
        );
    }

    #[test]
    fn multiple_variables() {
        let payload = json!({ "first": "A", "second": "B" });
        assert_eq!(render("${first}-${second}", &payload).unwrap(), "A-B");
    }
}
