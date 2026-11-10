//! Matrix client construction and session restore on startup (plan §9.2, §9.4).
//!
//! On every start the server reads `matrix_state`. If a connected session is
//! persisted, it builds a `matrix-sdk` [`Client`] against the SDK's SQLite
//! store (passphrase-protected by `BOTRIX_CRYPTO_PASSPHRASE`) and restores the
//! OAuth session from the decrypted tokens. On success the caller wires a
//! [`MatrixSender`](super::MatrixSender) into the delivery worker; on any
//! failure the state is reset to `disconnected` and the process continues with
//! the stub sender, since Matrix connectivity is not required to run.

use std::collections::HashMap;
use std::path::Path;

use matrix_sdk::authentication::oauth::registration::{
    ApplicationType, ClientMetadata, OAuthGrantType,
};
use matrix_sdk::authentication::oauth::{
    ClientId, ClientRegistrationData, OAuthSession, UserSession,
};
use matrix_sdk::authentication::SessionTokens;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{OwnedDeviceId, UserId};
use matrix_sdk::store::RoomLoadSettings;
use matrix_sdk::utils::UrlOrQuery;
use matrix_sdk::{Client, SessionMeta};
use url::Url;

use crate::config::Config;
use crate::db::matrix as db_matrix;
use crate::db::Pool;

use super::crypto;
use super::MatrixError;

/// Subdirectory under the data dir holding the SDK's SQLite stores (§9.2).
const MATRIX_STORE_SUBDIR: &str = "matrix";

/// Build a `matrix-sdk` [`Client`] with a passphrase-protected SQLite store.
///
/// The store lives at `{data_dir}/matrix`; the passphrase is the operator's
/// `BOTRIX_CRYPTO_PASSPHRASE`. The client is pointed at `homeserver` and has
/// token-refresh handling enabled so refreshed tokens can be persisted.
///
/// This does not contact the homeserver and is safe to call without network
/// access — it only opens (or creates) the local store.
pub async fn build_client(
    data_dir: &str,
    homeserver: &str,
    crypto_passphrase: &str,
) -> Result<Client, MatrixError> {
    let store_path = Path::new(data_dir).join(MATRIX_STORE_SUBDIR);
    if let Err(e) = std::fs::create_dir_all(&store_path) {
        return Err(MatrixError::ClientBuild(format!(
            "could not create store dir {}: {e}",
            store_path.display()
        )));
    }

    Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store(&store_path, Some(crypto_passphrase))
        .handle_refresh_tokens()
        .build()
        .await
        .map_err(|e| MatrixError::ClientBuild(e.to_string()))
}

/// Reconstruct the OAuth session to restore from the persisted state.
///
/// Returns `None` (rather than an error) if the row does not hold a complete,
/// connected session, so the caller can simply fall back to the stub sender.
///
/// # Errors
///
/// Returns [`MatrixError::TokenDecryption`] if a stored token cannot be
/// decrypted, or an id-parsing error if the persisted user/device IDs are
/// malformed — both indicate corrupted state worth surfacing in logs.
fn session_from_state(
    state: &db_matrix::MatrixState,
    crypto_passphrase: &str,
) -> Result<Option<OAuthSession>, MatrixError> {
    if !state.is_connected() {
        return Ok(None);
    }
    let (Some(user_id), Some(device_id), Some(client_id), Some(enc_access)) = (
        state.user_id.as_deref(),
        state.device_id.as_deref(),
        state.oauth_client_id.as_deref(),
        state.access_token.as_deref(),
    ) else {
        // Status says connected but a required field is missing: treat as not
        // restorable rather than erroring.
        return Ok(None);
    };

    let access_token = crypto::decrypt_token(crypto_passphrase, enc_access)?;
    let refresh_token = match state.refresh_token.as_deref() {
        Some(enc) => Some(crypto::decrypt_token(crypto_passphrase, enc)?),
        None => None,
    };

    let user_id = UserId::parse(user_id).map_err(|e| MatrixError::InvalidRoomId {
        room_id: user_id.to_owned(),
        reason: e.to_string(),
    })?;

    let meta = SessionMeta {
        user_id,
        device_id: OwnedDeviceId::from(device_id),
    };
    let tokens = SessionTokens {
        access_token,
        refresh_token,
    };

    Ok(Some(OAuthSession {
        client_id: ClientId::new(client_id.to_owned()),
        user: UserSession { meta, tokens },
    }))
}

/// Attempt to restore a persisted Matrix session and return a connected client.
///
/// Returns `Ok(Some(client))` when a session was restored, `Ok(None)` when no
/// connected session is persisted, and resets the state to `disconnected`
/// (logging the cause) if restore fails for any reason. Never propagates a
/// hard error for a failed restore, because Matrix connectivity is optional at
/// startup (§9.4).
pub async fn restore_client(
    pool: &Pool,
    config: &Config,
    data_dir: &str,
    crypto_passphrase: &str,
) -> Result<Option<Client>, MatrixError> {
    let state = db_matrix::get_state(pool)?;

    let session = match session_from_state(&state, crypto_passphrase) {
        Ok(Some(session)) => session,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "stored Matrix session is unusable; disconnecting");
            db_matrix::clear_session(pool)?;
            return Ok(None);
        }
    };

    let homeserver = state
        .homeserver
        .as_deref()
        .unwrap_or(&config.matrix.homeserver);

    let client = match build_client(data_dir, homeserver, crypto_passphrase).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to build Matrix client; disconnecting");
            db_matrix::set_status(pool, db_matrix::status::DISCONNECTED)?;
            return Ok(None);
        }
    };

    // Restoring the OAuth session reaches the homeserver to activate the
    // session and begin syncing; if the token was revoked or the server is
    // unreachable, fall back to disconnected rather than aborting startup.
    match client
        .oauth()
        .restore_session(session, RoomLoadSettings::default())
        .await
    {
        Ok(()) => {
            tracing::info!(user_id = ?client.user_id(), "restored Matrix session");
            Ok(Some(client))
        }
        Err(e) => {
            tracing::warn!(error = %e, "Matrix session restore failed; disconnecting");
            db_matrix::set_status(pool, db_matrix::status::DISCONNECTED)?;
            Ok(None)
        }
    }
}

/// Build the OAuth client metadata Botrix registers with the homeserver.
///
/// `redirect_uri` is where the homeserver sends the browser back after the
/// operator authorizes; `public_base` is Botrix's own URL, used for the
/// human-facing client fields. The grant type is authorization-code only.
fn client_metadata(
    redirect_uri: &str,
    public_base: &str,
) -> Result<Raw<ClientMetadata>, MatrixError> {
    use matrix_sdk::authentication::oauth::registration::Localized;

    let redirect = Url::parse(redirect_uri)
        .map_err(|e| MatrixError::Sdk(format!("invalid redirect URI '{redirect_uri}': {e}")))?;
    let client_uri = Url::parse(public_base)
        .map_err(|e| MatrixError::Sdk(format!("invalid public URL '{public_base}': {e}")))?;

    let mut metadata = ClientMetadata::new(
        ApplicationType::Web,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: vec![redirect],
        }],
        Localized::new(client_uri, []),
    );
    metadata.client_name = Some(Localized::new("Botrix".to_owned(), []));

    Raw::new(&metadata)
        .map_err(|e| MatrixError::Sdk(format!("could not serialize client metadata: {e}")))
}

/// Begin an OAuth Authorization Code login (plan §9.3, redirect variant).
///
/// Registers the client and returns the homeserver authorization URL to open
/// in the browser. The SDK holds the PKCE verifier and CSRF state inside
/// `client`, so [`complete_login`] must be called on the *same* client.
pub async fn begin_login(
    client: &Client,
    redirect_uri: &str,
    public_base: &str,
) -> Result<String, MatrixError> {
    let metadata = client_metadata(redirect_uri, public_base)?;
    let redirect = Url::parse(redirect_uri)
        .map_err(|e| MatrixError::Sdk(format!("invalid redirect URI '{redirect_uri}': {e}")))?;

    let registration: ClientRegistrationData = metadata.into();
    let data = client
        .oauth()
        .login(redirect, None, Some(registration), None)
        .build()
        .await
        .map_err(|e| MatrixError::Sdk(format!("could not start OAuth login: {e}")))?;

    Ok(data.url.to_string())
}

/// Complete an OAuth login from the redirect callback's query string.
///
/// `query` is the raw query part of the callback URL (`code=...&state=...`).
/// On success the homeserver session is active on `client`; this returns a
/// [`SessionRecord`] with the tokens **encrypted** ready to persist.
pub async fn complete_login(
    client: &Client,
    query: &str,
    homeserver: &str,
    crypto_passphrase: &str,
) -> Result<db_matrix::SessionRecord, MatrixError> {
    client
        .oauth()
        .finish_login(UrlOrQuery::Query(query.to_owned()))
        .await
        .map_err(|e| MatrixError::Sdk(format!("could not finish OAuth login: {e}")))?;

    let session = client
        .oauth()
        .full_session()
        .ok_or_else(|| MatrixError::Sdk("no session after finishing login".to_owned()))?;

    let user = session.user;
    let access_token = crypto::encrypt_token(crypto_passphrase, &user.tokens.access_token)
        .map_err(|e| MatrixError::TokenEncryption(e.to_string()))?;
    let refresh_token = match user.tokens.refresh_token.as_deref() {
        Some(t) => Some(
            crypto::encrypt_token(crypto_passphrase, t)
                .map_err(|e| MatrixError::TokenEncryption(e.to_string()))?,
        ),
        None => None,
    };

    Ok(db_matrix::SessionRecord {
        user_id: user.meta.user_id.to_string(),
        homeserver: homeserver.to_owned(),
        device_id: user.meta.device_id.to_string(),
        oauth_client_id: Some(session.client_id.as_str().to_owned()),
        access_token,
        refresh_token,
    })
}

/// Bootstrap E2EE from the account's recovery key (plan §9.5).
///
/// Opens secret storage with `recovery_key`, imports the cross-signing and
/// key-backup secrets into the local crypto store, and leaves recovery
/// `Enabled`. The key is used only for this call and is never stored; the
/// imported secrets persist in the SDK crypto store (`crypto.db`), so later
/// restarts need no key.
pub async fn recover_e2ee(client: &Client, recovery_key: &str) -> Result<(), MatrixError> {
    client
        .encryption()
        .recovery()
        .recover(recovery_key)
        .await
        .map_err(|e| MatrixError::InvalidRecoveryKey(e.to_string()))
}

/// Whether E2EE is bootstrapped on this client (recovery state `Enabled`).
pub fn e2ee_ready(client: &Client) -> bool {
    use matrix_sdk::encryption::recovery::RecoveryState;
    matches!(
        client.encryption().recovery().state(),
        RecoveryState::Enabled
    )
}

/// Build the logical destination → room-ID map from config, for the sender.
pub fn destination_map(config: &Config) -> HashMap<String, String> {
    config
        .matrix
        .destinations
        .iter()
        .map(|(name, dest)| (name.clone(), dest.room_id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::matrix::{persist_session, SessionRecord};
    use crate::db::test_pool;
    use botrix_common::config::{Config, DestinationConfig, MatrixConfig, ServerConfig};
    use std::collections::HashMap;

    const PASS: &str = "startup-passphrase";

    fn config_with_destinations() -> Config {
        let mut destinations = HashMap::new();
        destinations.insert(
            "containers".to_owned(),
            DestinationConfig {
                room_id: "!room:example.com".to_owned(),
                enabled: true,
            },
        );
        Config {
            matrix: MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations,
            },
            webhooks: HashMap::new(),
            triggers: HashMap::new(),
            access_profiles: HashMap::new(),
            server: ServerConfig::default(),
        }
    }

    #[test]
    fn destination_map_maps_names_to_room_ids() {
        let config = config_with_destinations();
        let map = destination_map(&config);
        assert_eq!(
            map.get("containers").map(String::as_str),
            Some("!room:example.com")
        );
    }

    #[test]
    fn session_from_disconnected_state_is_none() {
        let state = db_matrix::get_state(&test_pool()).unwrap();
        assert!(session_from_state(&state, PASS).unwrap().is_none());
    }

    #[test]
    fn session_from_connected_state_is_reconstructed() {
        // Arrange: persist a connected session with encrypted tokens.
        let pool = test_pool();
        let access = crypto::encrypt_token(PASS, "access-tok").unwrap();
        let refresh = crypto::encrypt_token(PASS, "refresh-tok").unwrap();
        persist_session(
            &pool,
            &SessionRecord {
                user_id: "@botrix:example.com".to_owned(),
                homeserver: "https://matrix.example.com".to_owned(),
                device_id: "DEV1".to_owned(),
                oauth_client_id: Some("client-xyz".to_owned()),
                access_token: access,
                refresh_token: Some(refresh),
            },
        )
        .unwrap();
        let state = db_matrix::get_state(&pool).unwrap();

        // Act
        let session = session_from_state(&state, PASS).unwrap().unwrap();

        // Assert: tokens decrypt back, identity is parsed.
        assert_eq!(session.user.tokens.access_token, "access-tok");
        assert_eq!(
            session.user.tokens.refresh_token.as_deref(),
            Some("refresh-tok")
        );
        assert_eq!(session.user.meta.user_id.as_str(), "@botrix:example.com");
        assert_eq!(session.user.meta.device_id.as_str(), "DEV1");
    }

    #[test]
    fn session_from_connected_state_with_wrong_passphrase_errors() {
        let pool = test_pool();
        let access = crypto::encrypt_token(PASS, "access-tok").unwrap();
        persist_session(
            &pool,
            &SessionRecord {
                user_id: "@botrix:example.com".to_owned(),
                homeserver: "https://matrix.example.com".to_owned(),
                device_id: "DEV1".to_owned(),
                oauth_client_id: Some("client-xyz".to_owned()),
                access_token: access,
                refresh_token: None,
            },
        )
        .unwrap();
        let state = db_matrix::get_state(&pool).unwrap();

        let err = session_from_state(&state, "wrong-passphrase").unwrap_err();
        assert!(matches!(err, MatrixError::TokenDecryption(_)));
    }
}
