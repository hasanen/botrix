//! Pure view-model helpers for the command trigger edit form.
//!
//! The form keeps everything as strings/bools so it can bind to inputs.
//! Secret values are write-only: [`TriggerForm::from_dto`] only ever carries a
//! secret's name and whether it is set; a value exists in the form only after
//! the user types one.

use botrix_common::api::triggers::{HeaderDto, SecretInput, TriggerDto, TriggerInput};

/// An editable outbound header row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeaderForm {
    /// Header name.
    pub name: String,
    /// Header value template (may contain `{{secret:NAME}}`).
    pub value: String,
}

/// An editable secret row.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretForm {
    /// Secret name.
    pub name: String,
    /// Whether the server already stores a value for this secret.
    pub is_set: bool,
    /// Newly entered value; blank means "leave unchanged".
    pub new_value: String,
    /// Remove the stored secret.
    pub clear: bool,
}

impl std::fmt::Debug for SecretForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretForm")
            .field("name", &self.name)
            .field("is_set", &self.is_set)
            .field("new_value", &"[redacted]")
            .field("clear", &self.clear)
            .finish()
    }
}

impl SecretForm {
    /// The change this row represents in a create/update request.
    fn to_input(&self) -> SecretInput {
        if self.clear {
            SecretInput {
                name: self.name.trim().to_owned(),
                value: None,
                clear: true,
            }
        } else if self.new_value.trim().is_empty() {
            SecretInput {
                name: self.name.trim().to_owned(),
                value: None,
                clear: false,
            }
        } else {
            SecretInput {
                name: self.name.trim().to_owned(),
                value: Some(self.new_value.clone()),
                clear: false,
            }
        }
    }
}

/// Editable state of a trigger, using strings for text inputs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriggerForm {
    /// Command word.
    pub command: String,
    /// Description shown by `help`.
    pub description: String,
    /// Whether the trigger is active.
    pub enabled: bool,
    /// HTTP method.
    pub method: String,
    /// Webhook URL.
    pub url: String,
    /// Response template; blank means none.
    pub response_template: String,
    /// Linked access profile, if any.
    pub access_profile_id: Option<i64>,
    /// Allowed rooms, one per line or comma separated.
    pub allowed_rooms: String,
    /// Allowed users, one per line or comma separated.
    pub allowed_users: String,
    /// Outbound headers.
    pub headers: Vec<HeaderForm>,
    /// Secrets.
    pub secrets: Vec<SecretForm>,
}

/// Split on newlines/commas, trim, drop empties and dedupe, keeping order.
pub fn parse_allowlist(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in text.split(['\n', '\r', ',']) {
        let item = item.trim();
        if !item.is_empty() && !out.iter().any(|existing| existing == item) {
            out.push(item.to_owned());
        }
    }
    out
}

impl TriggerForm {
    /// Build an editing form from a server DTO. Never contains secret values.
    pub fn from_dto(dto: &TriggerDto) -> TriggerForm {
        TriggerForm {
            command: dto.command.clone(),
            description: dto.description.clone(),
            enabled: dto.enabled,
            method: dto.method.clone(),
            url: dto.url.clone(),
            response_template: dto.response_template.clone().unwrap_or_default(),
            access_profile_id: dto.access_profile_id,
            allowed_rooms: dto.allowed_rooms.join("\n"),
            allowed_users: dto.allowed_users.join("\n"),
            headers: dto
                .headers
                .iter()
                .map(|h| HeaderForm {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect(),
            secrets: dto
                .secrets
                .iter()
                .map(|s| SecretForm {
                    name: s.name.clone(),
                    is_set: s.is_set,
                    new_value: String::new(),
                    clear: false,
                })
                .collect(),
        }
    }

    /// Convert to a create/update request body.
    pub fn to_input(&self) -> TriggerInput {
        let template = self.response_template.trim();
        TriggerInput {
            command: self.command.trim().to_owned(),
            description: self.description.trim().to_owned(),
            enabled: self.enabled,
            method: self.method.trim().to_owned(),
            url: self.url.trim().to_owned(),
            response_template: if template.is_empty() {
                None
            } else {
                Some(self.response_template.clone())
            },
            access_profile_id: self.access_profile_id,
            allowed_rooms: parse_allowlist(&self.allowed_rooms),
            allowed_users: parse_allowlist(&self.allowed_users),
            headers: self
                .headers
                .iter()
                .filter(|h| !h.name.trim().is_empty())
                .map(|h| HeaderDto {
                    name: h.name.trim().to_owned(),
                    value: h.value.clone(),
                })
                .collect(),
            secrets: self
                .secrets
                .iter()
                .filter(|s| !s.name.trim().is_empty())
                .map(SecretForm::to_input)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use botrix_common::api::triggers::SecretStatusDto;

    const TYPED: &str = "typed-secret-value";

    fn dto() -> TriggerDto {
        TriggerDto {
            id: 7,
            command: "deploy".into(),
            description: "Deploy".into(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com/hook".into(),
            response_template: Some("{{status}}".into()),
            access_profile_id: Some(2),
            allowed_rooms: vec!["!a:x".into(), "!b:x".into()],
            allowed_users: vec!["@u:x".into()],
            headers: vec![HeaderDto {
                name: "Authorization".into(),
                value: "Bearer {{secret:TOKEN}}".into(),
            }],
            secrets: vec![SecretStatusDto {
                name: "TOKEN".into(),
                is_set: true,
            }],
        }
    }

    fn secret(name: &str, new_value: &str, clear: bool) -> SecretForm {
        SecretForm {
            name: name.into(),
            is_set: true,
            new_value: new_value.into(),
            clear,
        }
    }

    fn input_with(secret: SecretForm) -> TriggerInput {
        let mut form = TriggerForm::from_dto(&dto());
        form.secrets = vec![secret];
        form.to_input()
    }

    #[test]
    fn parse_allowlist_splits_on_newlines_and_commas() {
        assert_eq!(
            parse_allowlist("!a:x\n!b:x, !c:x\r\n!d:x"),
            ["!a:x", "!b:x", "!c:x", "!d:x"]
        );
    }

    #[test]
    fn parse_allowlist_trims_and_drops_empties() {
        assert_eq!(parse_allowlist("  !a:x  ,, \n\n ,!b:x,"), ["!a:x", "!b:x"]);
        assert!(parse_allowlist("  \n , ").is_empty());
        assert!(parse_allowlist("").is_empty());
    }

    #[test]
    fn parse_allowlist_dedupes_preserving_first_occurrence_order() {
        assert_eq!(
            parse_allowlist("!b:x, !a:x\n!b:x\n!a:x,!c:x"),
            ["!b:x", "!a:x", "!c:x"]
        );
    }

    #[test]
    fn from_dto_copies_fields_and_joins_allowlists_by_line() {
        let form = TriggerForm::from_dto(&dto());

        assert_eq!(form.command, "deploy");
        assert!(form.enabled);
        assert_eq!(form.response_template, "{{status}}");
        assert_eq!(form.access_profile_id, Some(2));
        assert_eq!(form.allowed_rooms, "!a:x\n!b:x");
        assert_eq!(form.allowed_users, "@u:x");
        assert_eq!(form.headers.len(), 1);
    }

    #[test]
    fn from_dto_secrets_show_only_name_and_is_set() {
        let form = TriggerForm::from_dto(&dto());

        assert_eq!(form.secrets.len(), 1);
        assert_eq!(form.secrets[0].name, "TOKEN");
        assert!(form.secrets[0].is_set);
        assert!(form.secrets[0].new_value.is_empty());
        assert!(!form.secrets[0].clear);
    }

    #[test]
    fn from_dto_without_template_gives_blank_template() {
        let mut d = dto();
        d.response_template = None;

        assert_eq!(TriggerForm::from_dto(&d).response_template, "");
    }

    #[test]
    fn untouched_form_round_trips_to_equivalent_input_with_unchanged_secret() {
        let input = TriggerForm::from_dto(&dto()).to_input();

        assert_eq!(input.command, "deploy");
        assert_eq!(input.response_template.as_deref(), Some("{{status}}"));
        assert_eq!(input.allowed_rooms, ["!a:x", "!b:x"]);
        assert_eq!(input.headers[0].name, "Authorization");
        assert_eq!(input.secrets.len(), 1);
    }

    #[test]
    fn blank_template_becomes_none() {
        let mut form = TriggerForm::from_dto(&dto());
        form.response_template = "   ".into();

        assert_eq!(form.to_input().response_template, None);
    }

    #[test]
    fn blank_secret_means_unchanged() {
        let input = input_with(secret("TOKEN", "", false));

        assert_eq!(input.secrets[0].value, None);
        assert!(!input.secrets[0].clear);
    }

    #[test]
    fn whitespace_only_secret_means_unchanged() {
        let input = input_with(secret("TOKEN", "   ", false));

        assert_eq!(input.secrets[0].value, None);
        assert!(!input.secrets[0].clear);
    }

    #[test]
    fn entered_secret_means_set() {
        let input = input_with(secret("TOKEN", TYPED, false));

        assert_eq!(input.secrets[0].value.as_deref(), Some(TYPED));
        assert!(!input.secrets[0].clear);
    }

    #[test]
    fn clear_flag_means_clear_and_drops_any_typed_value() {
        let input = input_with(secret("TOKEN", TYPED, true));

        assert_eq!(input.secrets[0].value, None);
        assert!(input.secrets[0].clear);
    }

    #[test]
    fn secret_rows_without_a_name_are_dropped() {
        let input = input_with(secret("  ", TYPED, false));

        assert!(input.secrets.is_empty());
    }

    #[test]
    fn header_rows_without_a_name_are_dropped() {
        let mut form = TriggerForm::from_dto(&dto());
        form.headers.push(HeaderForm::default());

        assert_eq!(form.to_input().headers.len(), 1);
    }

    #[test]
    fn allowlists_are_parsed_in_to_input() {
        let mut form = TriggerForm::from_dto(&dto());
        form.allowed_rooms = "!a:x, !a:x\n!z:x".into();

        assert_eq!(form.to_input().allowed_rooms, ["!a:x", "!z:x"]);
    }

    #[test]
    fn form_debug_never_prints_a_typed_secret() {
        let debug = format!("{:?}", secret("TOKEN", TYPED, false));

        assert!(!debug.contains(TYPED), "leaked: {debug}");
    }
}
