//! Idle shutdown: the daemon exits through its normal graceful-shutdown path
//! after a configurable period without any handled IPC request.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_IDLE_SHUTDOWN_SECONDS: u64 = 3600;
const IDLE_SHUTDOWN_ENV: &str = "GSD_BROWSER_IDLE_SHUTDOWN_SECONDS";

/// `None` disables idle shutdown (env value `0`).
pub fn idle_shutdown_timeout() -> Option<Duration> {
    idle_shutdown_timeout_from_env(std::env::var(IDLE_SHUTDOWN_ENV).ok().as_deref())
}

fn idle_shutdown_timeout_from_env(value: Option<&str>) -> Option<Duration> {
    let seconds = value
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_IDLE_SHUTDOWN_SECONDS);
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

/// Tracks when the last IPC request was handled, as milliseconds elapsed
/// since the tracker was created (monotonic, cheap to update from any task).
pub struct IdleTracker {
    epoch: Instant,
    last_request_ms: AtomicU64,
}

impl IdleTracker {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            last_request_ms: AtomicU64::new(0),
        }
    }

    pub fn touch(&self) {
        self.last_request_ms
            .store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    pub fn idle_for(&self) -> Duration {
        let elapsed_ms = self.epoch.elapsed().as_millis() as u64;
        let last_ms = self.last_request_ms.load(Ordering::Relaxed);
        Duration::from_millis(elapsed_ms.saturating_sub(last_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_idle_shutdown_is_one_hour() {
        assert_eq!(
            idle_shutdown_timeout_from_env(None),
            Some(Duration::from_secs(3600))
        );
    }

    #[test]
    fn idle_shutdown_accepts_positive_seconds() {
        assert_eq!(
            idle_shutdown_timeout_from_env(Some("120")),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn idle_shutdown_zero_disables() {
        assert_eq!(idle_shutdown_timeout_from_env(Some("0")), None);
    }

    #[test]
    fn idle_shutdown_ignores_invalid_values() {
        assert_eq!(
            idle_shutdown_timeout_from_env(Some("not-a-number")),
            Some(Duration::from_secs(3600))
        );
    }

    #[test]
    fn tracker_idle_resets_on_touch() {
        let tracker = IdleTracker::new();
        std::thread::sleep(Duration::from_millis(20));
        assert!(tracker.idle_for() >= Duration::from_millis(10));
        tracker.touch();
        assert!(tracker.idle_for() < Duration::from_millis(10));
    }
}
