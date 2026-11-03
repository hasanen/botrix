//! Tokio background task that delivers pending notifications to Matrix.
//!
//! Each poll claims a batch of `pending` notifications, attempts delivery via
//! a [`MessageSender`], and applies the [`retry`](super::retry) decision:
//! delivered, retried after a backoff delay, or failed. The pure decision and
//! backoff schedule live in [`super::retry`]; this module performs the I/O.

use std::sync::Arc;
use std::time::Duration;

use crate::db::webhooks as db_webhooks;
use crate::db::Pool;

use super::retry::{decide, Outcome, Transition};
use super::sender::{MessageSender, SendError};

/// Maximum notifications claimed per poll.
const BATCH_SIZE: i64 = 20;

/// How often the worker polls for pending notifications.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Spawn the background delivery worker, returning its join handle.
///
/// The worker runs until the process exits.
pub fn spawn(pool: Pool, sender: Arc<dyn MessageSender>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(err) = poll_once(&pool, sender.as_ref()).await {
                tracing::error!(error = %err, "notification worker poll failed");
            }
        }
    })
}

/// Process a single batch of pending notifications.
///
/// Returns an error only if claiming the batch failed; per-message delivery
/// failures are handled individually and do not abort the batch.
pub async fn poll_once(pool: &Pool, sender: &dyn MessageSender) -> Result<(), crate::db::DbError> {
    let batch = db_webhooks::claim_pending(pool, BATCH_SIZE)?;
    for item in batch {
        process_one(pool, sender, item).await;
    }
    Ok(())
}

/// Deliver a single claimed notification and apply the resulting transition.
async fn process_one(
    pool: &Pool,
    sender: &dyn MessageSender,
    item: db_webhooks::PendingNotification,
) {
    let outcome = match sender.send(&item.destination, &item.rendered_message).await {
        Ok(()) => Outcome::Success,
        Err(SendError::Transient(msg)) => {
            tracing::warn!(id = item.id, error = %msg, "transient delivery failure");
            Outcome::TransientFailure
        }
        Err(SendError::Permanent(msg)) => {
            tracing::error!(id = item.id, error = %msg, "permanent delivery failure");
            Outcome::PermanentFailure
        }
    };

    // `attempts` is the count *before* this attempt; the attempt we just made
    // brings the total to `attempts + 1`, which is what `decide` evaluates.
    let attempts_including_this = item.attempts + 1;
    match decide(attempts_including_this, outcome) {
        Transition::Delivered => {
            if let Err(err) = db_webhooks::mark_delivered(pool, item.id) {
                tracing::error!(id = item.id, error = %err, "failed to mark delivered");
            }
        }
        Transition::Failed => {
            let reason = match outcome {
                Outcome::PermanentFailure => "permanent delivery failure",
                _ => "max retry attempts exhausted",
            };
            if let Err(err) = db_webhooks::mark_failed(pool, item.id, reason) {
                tracing::error!(id = item.id, error = %err, "failed to mark failed");
            }
        }
        Transition::Retry { delay } => {
            // Return to `pending` after the backoff delay so the next poll can
            // re-claim it. Spawned so a slow retry does not block the batch.
            let pool = pool.clone();
            let id = item.id;
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if let Err(err) =
                    db_webhooks::mark_for_retry(&pool, id, "transient failure; retrying")
                {
                    tracing::error!(id, error = %err, "failed to mark for retry");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::db::webhooks::insert_notification;

    use crate::db::test_pool;

    fn status_of(pool: &Pool, id: i64) -> String {
        pool.get()
            .unwrap()
            .query_row(
                "SELECT status FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// A fake sender whose behavior is configurable per call.
    struct FakeSender {
        /// Outcomes to return, consumed in order; last one repeats.
        script: Mutex<Vec<Result<(), SendError>>>,
        calls: AtomicUsize,
    }

    impl FakeSender {
        fn always_ok() -> Self {
            Self {
                script: Mutex::new(vec![Ok(())]),
                calls: AtomicUsize::new(0),
            }
        }

        fn always(err: SendError) -> Self {
            Self {
                script: Mutex::new(vec![Err(err)]),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl MessageSender for FakeSender {
        async fn send(&self, _destination: &str, _message: &str) -> Result<(), SendError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let script = self.script.lock().unwrap();
            // Clone the matching entry (or the last) so the script can repeat.
            let idx = (self.calls.load(Ordering::SeqCst) - 1).min(script.len() - 1);
            match &script[idx] {
                Ok(()) => Ok(()),
                Err(SendError::Transient(m)) => Err(SendError::Transient(m.clone())),
                Err(SendError::Permanent(m)) => Err(SendError::Permanent(m.clone())),
            }
        }
    }

    #[tokio::test]
    async fn successful_delivery_marks_delivered() {
        // Arrange
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "hi").unwrap();
        let sender = FakeSender::always_ok();

        // Act
        poll_once(&pool, &sender).await.unwrap();

        // Assert
        assert_eq!(status_of(&pool, id), "delivered");
        assert_eq!(sender.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn permanent_failure_marks_failed_without_retry() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "hi").unwrap();
        let sender = FakeSender::always(SendError::Permanent("no such room".into()));

        poll_once(&pool, &sender).await.unwrap();

        assert_eq!(status_of(&pool, id), "failed");
    }

    #[tokio::test]
    async fn transient_failure_on_third_attempt_marks_failed() {
        // Arrange: a row that has already failed twice (attempts = 2). The
        // third attempt failing transiently should mark it failed.
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "hi").unwrap();
        pool.get()
            .unwrap()
            .execute(
                "UPDATE notifications SET attempts = 2 WHERE id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        let sender = FakeSender::always(SendError::Transient("timeout".into()));

        // Act
        poll_once(&pool, &sender).await.unwrap();

        // Assert: attempts become 3 and the row is failed (no more retries).
        assert_eq!(status_of(&pool, id), "failed");
        let attempts: i64 = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT attempts FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 3);
    }

    #[tokio::test]
    async fn claim_during_poll_prevents_double_processing() {
        // Two pending rows, a sender that always succeeds: a single poll should
        // deliver both exactly once.
        let pool = test_pool();
        let a = insert_notification(&pool, "wud", "containers", "a").unwrap();
        let b = insert_notification(&pool, "wud", "containers", "b").unwrap();
        let sender = FakeSender::always_ok();

        poll_once(&pool, &sender).await.unwrap();

        assert_eq!(status_of(&pool, a), "delivered");
        assert_eq!(status_of(&pool, b), "delivered");
        assert_eq!(sender.calls.load(Ordering::SeqCst), 2);
    }
}
