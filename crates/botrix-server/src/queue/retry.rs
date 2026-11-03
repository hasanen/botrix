//! Pure retry/backoff decision logic for the notification worker.
//!
//! Separated from the worker loop so the state machine and backoff schedule
//! are unit-testable without sleeping real durations (§11.2, §11.3).

use std::time::Duration;

/// Maximum number of delivery attempts before a notification is marked failed.
pub const MAX_ATTEMPTS: i64 = 3;

/// The outcome of a single delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The message was delivered successfully.
    Success,
    /// Delivery failed but may be retried.
    TransientFailure,
    /// Delivery failed permanently and must not be retried.
    PermanentFailure,
}

/// What the worker should do with a notification after an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Mark delivered; no further action.
    Delivered,
    /// Schedule another attempt after `delay`.
    Retry {
        /// How long to wait before the next attempt.
        delay: Duration,
    },
    /// Give up; mark failed permanently.
    Failed,
}

/// Decide the next transition given the number of attempts already made
/// (including the one that just completed) and its `outcome`.
///
/// Backoff schedule (§11.3):
/// - attempt 1 → 2: wait 30 seconds
/// - attempt 2 → 3: wait 5 minutes
/// - after 3 attempts: failed
pub fn decide(attempts: i64, outcome: Outcome) -> Transition {
    match outcome {
        Outcome::Success => Transition::Delivered,
        Outcome::PermanentFailure => Transition::Failed,
        Outcome::TransientFailure => {
            if attempts >= MAX_ATTEMPTS {
                Transition::Failed
            } else {
                Transition::Retry {
                    delay: backoff_delay(attempts),
                }
            }
        }
    }
}

/// Backoff delay before the next attempt, given the number of attempts made.
fn backoff_delay(attempts: i64) -> Duration {
    match attempts {
        1 => Duration::from_secs(30),
        2 => Duration::from_secs(5 * 60),
        // Defensive default; `decide` never calls this at >= MAX_ATTEMPTS.
        _ => Duration::from_secs(5 * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_delivered() {
        assert_eq!(decide(1, Outcome::Success), Transition::Delivered);
    }

    #[test]
    fn permanent_failure_fails_immediately() {
        assert_eq!(decide(1, Outcome::PermanentFailure), Transition::Failed);
    }

    #[test]
    fn first_transient_failure_retries_after_30s() {
        assert_eq!(
            decide(1, Outcome::TransientFailure),
            Transition::Retry {
                delay: Duration::from_secs(30)
            }
        );
    }

    #[test]
    fn second_transient_failure_retries_after_5min() {
        assert_eq!(
            decide(2, Outcome::TransientFailure),
            Transition::Retry {
                delay: Duration::from_secs(300)
            }
        );
    }

    #[test]
    fn third_transient_failure_is_failed() {
        assert_eq!(decide(3, Outcome::TransientFailure), Transition::Failed);
    }

    #[test]
    fn beyond_max_attempts_is_failed() {
        assert_eq!(decide(4, Outcome::TransientFailure), Transition::Failed);
    }
}
