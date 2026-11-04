//! API request and response types shared between the frontend and backend.
//!
//! All types in this module must be serializable with `serde` and must compile
//! for both native targets and `wasm32-unknown-unknown`.

pub mod auth;
pub mod logs;
pub mod matrix;
pub mod notifications;
pub mod settings;
pub mod webhooks;

#[cfg(test)]
mod redaction_tests {
    //! Types that carry secrets must not reveal them through `{:?}`, so a
    //! stray `tracing::debug!(?request)` can never write a password or token
    //! to the logs.

    use super::auth::{LoginRequest, SetupRequest};
    use super::matrix::RecoveryKeyRequest;
    use super::settings::ChangePasswordRequest;
    use super::webhooks::TokenGenerateResponse;

    const SECRET: &str = "s3cr3t-value-that-must-not-leak";

    fn assert_redacted(debug: String) {
        assert!(!debug.contains(SECRET), "leaked secret: {debug}");
        assert!(debug.contains("[redacted]"), "no redaction marker: {debug}");
    }

    #[test]
    fn login_request_hides_the_password_but_keeps_the_username() {
        let debug = format!(
            "{:?}",
            LoginRequest {
                username: "alice".into(),
                password: SECRET.into(),
            }
        );
        assert!(debug.contains("alice"));
        assert_redacted(debug);
    }

    #[test]
    fn setup_request_hides_the_password() {
        assert_redacted(format!(
            "{:?}",
            SetupRequest {
                username: "alice".into(),
                password: SECRET.into(),
            }
        ));
    }

    #[test]
    fn change_password_request_hides_both_passwords() {
        let debug = format!(
            "{:?}",
            ChangePasswordRequest {
                current_password: SECRET.into(),
                new_password: SECRET.into(),
            }
        );
        assert_redacted(debug);
    }

    #[test]
    fn recovery_key_request_hides_the_key() {
        assert_redacted(format!(
            "{:?}",
            RecoveryKeyRequest {
                recovery_key: SECRET.into(),
            }
        ));
    }

    #[test]
    fn token_response_hides_the_token() {
        assert_redacted(format!(
            "{:?}",
            TokenGenerateResponse {
                token: SECRET.into(),
            }
        ));
    }

    #[test]
    fn redaction_does_not_change_serialization() {
        // The wire format must be unaffected: only `Debug` is customized.
        let json = serde_json::to_string(&LoginRequest {
            username: "alice".into(),
            password: SECRET.into(),
        })
        .unwrap();
        assert!(json.contains(SECRET));
    }
}
