use super::platform::{OfflineHealth, Platform};
use super::send_once;
use crate::win_process::is_process_alive;
use gsd_browser_common::session::{SessionHealthStatus, SessionManifest};
use gsd_browser_common::{
    lock_path_for, named_pipe_name_for, pid_path_for, state_dir, validate_session_name,
};
use serde_json::json;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::time::sleep;

pub(crate) struct WindowsPlatform;

pub(crate) struct LockFileGuard {
    file: File,
    path: PathBuf,
}

fn read_daemon_pid(session: Option<&str>) -> Option<u32> {
    let pid_file = pid_path_for(session);
    let pid_str = fs::read_to_string(pid_file).ok()?;
    pid_str.trim().parse().ok()
}

async fn connect_pipe(
    session: Option<&str>,
    max_wait: Duration,
) -> Result<NamedPipeClient, Box<dyn std::error::Error>> {
    let pipe_name = named_pipe_name_for(session);
    let start = Instant::now();
    let poll_interval = Duration::from_millis(50);

    loop {
        match ClientOptions::new().open(&pipe_name) {
            Ok(client) => return Ok(client),
            Err(err) if start.elapsed() < max_wait => {
                let _ = err;
                sleep(poll_interval).await;
            }
            Err(err) => {
                return Err(format!("cannot connect to daemon pipe {pipe_name}: {err}").into());
            }
        }
    }
}

async fn pipe_connectable(session: Option<&str>) -> bool {
    connect_pipe(session, Duration::from_millis(300))
        .await
        .is_ok()
}

impl Platform for WindowsPlatform {
    type Stream = NamedPipeClient;
    type StartupLock = LockFileGuard;

    const CLEARS_BROWSER_PID_ON_STOP: bool = false;
    const GUARDS_IMPLICIT_REPLACEMENT: bool = false;
    const CHECKS_ALIVE_UNDER_LOCK: bool = false;

    fn endpoint_display(session: Option<&str>) -> String {
        named_pipe_name_for(session)
    }

    fn is_daemon_alive(session: Option<&str>) -> bool {
        read_daemon_pid(validate_session_name(session).ok().flatten()).is_some_and(is_process_alive)
    }

    async fn connect(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<Self::Stream, Box<dyn std::error::Error>> {
        connect_pipe(session, max_wait).await
    }

    async fn probe(session: Option<&str>) -> bool {
        pipe_connectable(session).await
    }

    async fn endpoint_ready(session: Option<&str>) -> bool {
        pipe_connectable(session).await
    }

    async fn ready_short_circuit(session: Option<&str>) -> bool {
        pipe_connectable(session).await
    }

    fn prepare_state_dirs(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(state_dir())?;
        if let Some(parent) = pid_path_for(session).parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn acquire_startup_lock(
        session: Option<&str>,
    ) -> Result<Option<Self::StartupLock>, Box<dyn std::error::Error>> {
        let lock_file = lock_path_for(session);
        if let Some(parent) = lock_file.parent() {
            fs::create_dir_all(parent)?;
        }

        let lock = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_file);
        match lock {
            Ok(file) => Ok(Some(LockFileGuard {
                file,
                path: lock_file,
            })),
            Err(_) => Ok(None),
        }
    }

    /// Drop the lock handle before deleting `daemon.lock` (required on Windows).
    fn release_startup_lock(_session: Option<&str>, lock: Self::StartupLock) {
        let LockFileGuard { file, path } = lock;
        drop(file);
        let _ = fs::remove_file(path);
    }

    fn configure_detached_daemon_process(cmd: &mut std::process::Command) {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    async fn wait_ready(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        connect_pipe(session, max_wait).await.map(|_| ())
    }

    fn terminate_daemon(session: Option<&str>) -> Result<bool, Box<dyn std::error::Error>> {
        let Some(pid) = read_daemon_pid(session) else {
            return Ok(false);
        };
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .arg("/F")
            .status();
        Ok(true)
    }

    fn cleanup_daemon_artifacts(session: Option<&str>) {
        let _ = fs::remove_file(pid_path_for(session));
    }

    fn remove_stop_artifacts(session: Option<&str>) {
        let _ = fs::remove_file(pid_path_for(session));
        let _ = fs::remove_file(lock_path_for(session));
    }

    /// Windows does not clean up browser processes holding the profile today.
    fn cleanup_session_browser_processes(
        _manifest: Option<&SessionManifest>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn try_daemon_health(session: Option<&str>) -> Option<serde_json::Value> {
        if let Ok(resp) = send_once("health", json!({}), session).await {
            if let Some(result) = resp.result {
                return Some(result);
            }
        }
        None
    }

    async fn classify_offline_health(
        session: Option<&str>,
        manifest: &SessionManifest,
    ) -> OfflineHealth {
        let pipe_connected = pipe_connectable(session).await;
        let daemon_alive = pipe_connected || Self::is_daemon_alive(session);

        let (status, reason) = if pipe_connected {
            (SessionHealthStatus::Healthy, String::new())
        } else if manifest.health == SessionHealthStatus::Stopped {
            (SessionHealthStatus::Stopped, manifest.health_reason.clone())
        } else if daemon_alive {
            (
                SessionHealthStatus::Degraded,
                "daemon PID exists but the named pipe is unavailable".to_string(),
            )
        } else if manifest.session_name.is_some() || manifest.daemon_pid.is_some() {
            (
                SessionHealthStatus::Unhealthy,
                "session metadata exists but no live daemon is reachable".to_string(),
            )
        } else {
            (
                SessionHealthStatus::Stopped,
                "daemon not running".to_string(),
            )
        };

        OfflineHealth {
            status,
            reason,
            daemon_alive,
            socket_connected: pipe_connected,
            daemon_pid: read_daemon_pid(session).map(|pid| pid as i32),
        }
    }
}
