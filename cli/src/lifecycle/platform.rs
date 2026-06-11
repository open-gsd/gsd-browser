use gsd_browser_common::session::{SessionHealthStatus, SessionManifest};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// Result of platform-specific health classification when the daemon's own
/// health endpoint was unreachable.
pub(crate) struct OfflineHealth {
    pub status: SessionHealthStatus,
    pub reason: String,
    pub daemon_alive: bool,
    pub socket_connected: bool,
    pub daemon_pid: Option<i32>,
}

/// What genuinely differs between Unix (socket/flock/SIGTERM) and Windows
/// (named pipe/lock file/taskkill) daemon lifecycle handling. Everything else
/// — orchestration, retries, health envelope, request framing — is shared in
/// `lifecycle::` and dispatches statically through `CurrentPlatform`.
pub(crate) trait Platform {
    type Stream: AsyncRead + AsyncWrite + Unpin;
    type StartupLock;

    /// Unix clears `browser_pid` when writing a stopped manifest; Windows does not.
    const CLEARS_BROWSER_PID_ON_STOP: bool;
    /// Unix refuses to implicitly replace a non-stopped named session; Windows does not.
    const GUARDS_IMPLICIT_REPLACEMENT: bool;
    /// Unix re-checks daemon liveness after acquiring the startup lock.
    const CHECKS_ALIVE_UNDER_LOCK: bool;

    /// Endpoint string stored in manifests (socket path vs pipe name).
    fn endpoint_display(session: Option<&str>) -> String;

    fn is_daemon_alive(session: Option<&str>) -> bool;

    /// Connect a request stream to the daemon endpoint.
    async fn connect(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<Self::Stream, Box<dyn std::error::Error>>;

    /// Single readiness probe used inside spawn-wait poll loops.
    async fn probe(session: Option<&str>) -> bool;

    /// Cheap endpoint readiness used by ensure-before-send and the
    /// alive-under-lock check (Unix: socket file exists; Windows: pipe connects).
    async fn endpoint_ready(session: Option<&str>) -> bool;

    /// True when `start_daemon` can return immediately without locking
    /// (Windows checks pipe connectivity up front; Unix never short-circuits).
    async fn ready_short_circuit(session: Option<&str>) -> bool;

    fn prepare_state_dirs(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>>;

    /// `Ok(None)` means another process holds the startup lock.
    fn acquire_startup_lock(
        session: Option<&str>,
    ) -> Result<Option<Self::StartupLock>, Box<dyn std::error::Error>>;

    fn release_startup_lock(session: Option<&str>, lock: Self::StartupLock);

    fn configure_detached_daemon_process(cmd: &mut std::process::Command);

    /// Wait for the daemon endpoint when another process is starting it.
    async fn wait_ready(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<(), Box<dyn std::error::Error>>;

    /// Terminate the daemon process. Returns whether a PID file was present
    /// (Unix gates browser-cleanup error propagation on it).
    fn terminate_daemon(session: Option<&str>) -> Result<bool, Box<dyn std::error::Error>>;

    /// Remove stale transport artifacts before (re)starting.
    fn cleanup_daemon_artifacts(session: Option<&str>);

    /// Files removed on explicit stop (Unix: pid + socket; Windows: pid + lock).
    fn remove_stop_artifacts(session: Option<&str>);

    /// Kill browser processes still holding the session profile
    /// (Unix only; Windows intentionally does nothing today).
    fn cleanup_session_browser_processes(
        manifest: Option<&SessionManifest>,
    ) -> Result<(), Box<dyn std::error::Error>>;

    /// Ask the daemon's own health endpoint; `None` when unreachable.
    async fn try_daemon_health(session: Option<&str>) -> Option<serde_json::Value>;

    /// Classify session health when the daemon health endpoint was unreachable.
    /// The Unix and Windows state machines differ slightly and are preserved as-is.
    async fn classify_offline_health(
        session: Option<&str>,
        manifest: &SessionManifest,
    ) -> OfflineHealth;
}

#[cfg(unix)]
pub(crate) type CurrentPlatform = super::unix::UnixPlatform;
#[cfg(windows)]
pub(crate) type CurrentPlatform = super::windows::WindowsPlatform;
