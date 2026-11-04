//! Background Matrix sync loop.
//!
//! The SDK only processes to-device messages — where room keys and
//! secret-storage data arrive — while a sync is running. Without this, the bot
//! cannot send to encrypted rooms (no Megolm sessions) and room membership
//! never updates. [`spawn_sync`] runs `Client::sync` in a background task and
//! restarts it with a bounded backoff if it returns an error (e.g. a transient
//! network drop), until the client is logged out.

use std::time::Duration;

use matrix_sdk::config::SyncSettings;
use matrix_sdk::Client;
use tokio::task::JoinHandle;

/// Long-poll timeout for each sync request.
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);
/// Backoff after a failed sync before retrying.
const RETRY_MIN: Duration = Duration::from_secs(1);
/// Upper bound on the retry backoff.
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Spawn the background sync loop for `client`, returning its task handle.
///
/// Runs until the process exits or the client is logged out. Errors are logged
/// and retried with exponential backoff rather than ending the loop, so a
/// transient outage does not silently stop delivery to encrypted rooms.
pub fn spawn_sync(client: Client) -> JoinHandle<()> {
    tokio::spawn(async move {
        let settings = SyncSettings::new().timeout(SYNC_TIMEOUT);
        let mut backoff = RETRY_MIN;
        loop {
            match client.sync(settings.clone()).await {
                // `sync` only returns on error; `Ok` would mean a clean stop.
                Ok(()) => {
                    tracing::info!("Matrix sync loop ended");
                    return;
                }
                Err(err) => {
                    tracing::warn!(error = %err, retry_in = ?backoff, "Matrix sync failed; retrying");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RETRY_MAX);
                }
            }
        }
    })
}
