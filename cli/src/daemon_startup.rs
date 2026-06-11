use std::time::Duration;

const DEFAULT_DAEMON_STARTUP_TIMEOUT_SECONDS: u64 = 10;
const DAEMON_STARTUP_TIMEOUT_ENV: &str = "GSD_BROWSER_DAEMON_STARTUP_TIMEOUT_SECONDS";

pub fn daemon_startup_timeout() -> Duration {
    daemon_startup_timeout_from_env(std::env::var(DAEMON_STARTUP_TIMEOUT_ENV).ok().as_deref())
}

fn daemon_startup_timeout_from_env(value: Option<&str>) -> Duration {
    value
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_DAEMON_STARTUP_TIMEOUT_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_startup_timeout_is_ten_seconds() {
        assert_eq!(
            daemon_startup_timeout_from_env(None),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn startup_timeout_accepts_positive_seconds() {
        assert_eq!(
            daemon_startup_timeout_from_env(Some("30")),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn startup_timeout_ignores_invalid_values() {
        assert_eq!(
            daemon_startup_timeout_from_env(Some("not-a-number")),
            Duration::from_secs(10)
        );
        assert_eq!(
            daemon_startup_timeout_from_env(Some("0")),
            Duration::from_secs(10)
        );
    }
}
