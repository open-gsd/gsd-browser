use super::platform::{OfflineHealth, Platform};
use super::send_once;
use gsd_browser_common::process::{pids_using_profile, terminate_process};
use gsd_browser_common::session::{SessionHealthStatus, SessionManifest};
use gsd_browser_common::{pid_path_for, socket_path_for, state_dir};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::time::{sleep, timeout};

pub(crate) struct UnixPlatform;

pub(crate) struct FlockGuard {
    file: File,
}

fn read_daemon_pid(session: Option<&str>) -> Option<i32> {
    let pid_file = pid_path_for(session);
    let pid_str = fs::read_to_string(pid_file).ok()?;
    pid_str.trim().parse().ok()
}

async fn wait_for_socket(
    session: Option<&str>,
    max_wait: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let sock = socket_path_for(session);
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    while start.elapsed() < max_wait {
        if sock.exists() {
            // Try connecting to verify it's actually listening
            if UnixStream::connect(&sock).await.is_ok() {
                return Ok(());
            }
        }
        sleep(poll_interval).await;
    }

    Err(format!(
        "daemon did not start within {}s — re-run with GSD_BROWSER_DEBUG=1 for startup logs",
        max_wait.as_secs()
    )
    .into())
}

impl Platform for UnixPlatform {
    type Stream = UnixStream;
    type StartupLock = FlockGuard;

    const CLEARS_BROWSER_PID_ON_STOP: bool = true;
    const GUARDS_IMPLICIT_REPLACEMENT: bool = true;
    const CHECKS_ALIVE_UNDER_LOCK: bool = true;

    fn endpoint_display(session: Option<&str>) -> String {
        socket_path_for(session).to_string_lossy().to_string()
    }

    /// Check if daemon is alive: PID file exists, process alive.
    fn is_daemon_alive(session: Option<&str>) -> bool {
        read_daemon_pid(session)
            .map(|pid| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok())
            .unwrap_or(false)
    }

    async fn connect(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<Self::Stream, Box<dyn std::error::Error>> {
        let sock = socket_path_for(session);
        let stream = timeout(max_wait, UnixStream::connect(&sock))
            .await
            .map_err(|_| "timeout connecting to daemon")?
            .map_err(|e| format!("cannot connect to daemon socket: {e}"))?;
        Ok(stream)
    }

    async fn probe(session: Option<&str>) -> bool {
        let sock = socket_path_for(session);
        sock.exists() && UnixStream::connect(&sock).await.is_ok()
    }

    async fn endpoint_ready(session: Option<&str>) -> bool {
        socket_path_for(session).exists()
    }

    async fn ready_short_circuit(_session: Option<&str>) -> bool {
        false
    }

    fn prepare_state_dirs(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let sock = socket_path_for(session);
        if let Some(parent) = sock.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(state_dir())?;
        Ok(())
    }

    fn acquire_startup_lock(
        session: Option<&str>,
    ) -> Result<Option<Self::StartupLock>, Box<dyn std::error::Error>> {
        // Advisory lock to prevent race conditions
        let lock_file = gsd_browser_common::lock_path_for(session);
        if let Some(parent) = lock_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock_fd = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_file)?;

        // Try to acquire exclusive lock (non-blocking first)
        use std::os::unix::io::AsRawFd;
        let fd = lock_fd.as_raw_fd();
        let lock_result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if lock_result != 0 {
            return Ok(None);
        }
        Ok(Some(FlockGuard { file: lock_fd }))
    }

    fn release_startup_lock(_session: Option<&str>, lock: Self::StartupLock) {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::flock(lock.file.as_raw_fd(), libc::LOCK_UN) };
    }

    fn configure_detached_daemon_process(cmd: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;

        // The daemon must survive the lifecycle of the foreground CLI command.
        // Creating a new session keeps it out of the parent's process group.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    async fn wait_ready(
        session: Option<&str>,
        max_wait: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        wait_for_socket(session, max_wait).await
    }

    fn terminate_daemon(session: Option<&str>) -> Result<bool, Box<dyn std::error::Error>> {
        let pid_file = pid_path_for(session);
        if !pid_file.exists() {
            return Ok(false);
        }
        let pid_str = fs::read_to_string(&pid_file)?;
        let pid: i32 = pid_str.trim().parse().map_err(|_| "invalid PID file")?;
        terminate_process(pid, "daemon")?;
        Ok(true)
    }

    fn cleanup_daemon_artifacts(session: Option<&str>) {
        let _ = fs::remove_file(socket_path_for(session));
        let _ = fs::remove_file(pid_path_for(session));
    }

    fn remove_stop_artifacts(session: Option<&str>) {
        let _ = fs::remove_file(pid_path_for(session));
        let _ = fs::remove_file(socket_path_for(session));
    }

    fn cleanup_session_browser_processes(
        manifest: Option<&SessionManifest>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(manifest) = manifest else {
            return Ok(());
        };
        let Some(profile_dir) = manifest.browser_user_data_dir.as_deref() else {
            return Ok(());
        };

        let mut pids = BTreeSet::new();
        if let Some(pid) = manifest.browser_pid.and_then(|pid| i32::try_from(pid).ok()) {
            pids.insert(pid);
        }
        for pid in pids_using_profile(profile_dir)? {
            pids.insert(pid);
        }

        for pid in pids {
            terminate_process(pid, "browser process")?;
        }
        Ok(())
    }

    async fn try_daemon_health(session: Option<&str>) -> Option<serde_json::Value> {
        let socket_path = socket_path_for(session);
        if !socket_path.exists() {
            return None;
        }
        let socket_connected = timeout(
            Duration::from_millis(300),
            UnixStream::connect(&socket_path),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some();
        if !socket_connected {
            return None;
        }
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
        let pid = read_daemon_pid(session);
        let daemon_alive = Self::is_daemon_alive(session);
        let socket_path = socket_path_for(session);
        let socket_exists = socket_path.exists();
        let socket_connected = if socket_exists {
            timeout(
                Duration::from_millis(300),
                UnixStream::connect(&socket_path),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .is_some()
        } else {
            false
        };

        let (status, reason) = if daemon_alive && !socket_connected {
            (
                SessionHealthStatus::Degraded,
                "daemon PID is alive but the socket is unavailable".to_string(),
            )
        } else if !daemon_alive && socket_exists {
            (
                SessionHealthStatus::Unhealthy,
                "daemon socket exists without a live daemon PID".to_string(),
            )
        } else if daemon_alive && socket_connected {
            (SessionHealthStatus::Healthy, String::new())
        } else if manifest.health == SessionHealthStatus::Stopped {
            (SessionHealthStatus::Stopped, manifest.health_reason.clone())
        } else if manifest.session_name.is_some() || manifest.daemon_pid.is_some() {
            (
                SessionHealthStatus::Unhealthy,
                "session metadata exists but no live daemon is running".to_string(),
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
            socket_connected,
            daemon_pid: pid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Platform, UnixPlatform};
    use gsd_browser_common::session::SessionManifest;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    /// Self-heal path: a stale manifest pointing at a profile dir held by an
    /// orphaned process should get that process killed. The holder is spawned
    /// through a shell that exits so it is not our child (a direct child would
    /// linger as a zombie after SIGTERM and defeat the liveness check).
    #[test]
    fn cleanup_session_browser_processes_kills_profile_holders() {
        let dir = tempfile::Builder::new()
            .prefix("gsd-browser-test-selfheal-")
            .tempdir()
            .expect("create temp profile dir");
        let marker = dir.path().join("browser-profile").display().to_string();
        std::fs::create_dir_all(&marker).expect("create marker dir");
        let marker_file = format!("{marker}/SingletonLock");
        std::fs::write(&marker_file, b"").expect("create marker file");

        let output = Command::new("sh")
            .args([
                "-c",
                &format!("tail -f '{marker_file}' >/dev/null 2>&1 & echo $!"),
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("spawn orphaned profile holder");
        let pid: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("parse holder PID");
        thread::sleep(Duration::from_millis(200));

        let manifest = SessionManifest {
            browser_user_data_dir: Some(marker.clone()),
            ..SessionManifest::default()
        };
        let result = UnixPlatform::cleanup_session_browser_processes(Some(&manifest));
        if result.is_err() {
            // Best-effort cleanup so the holder never leaks on failure.
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        result.expect("cleanup should succeed");

        assert!(
            gsd_browser_common::process::wait_for_process_exit(pid, Duration::from_secs(2)),
            "orphaned profile holder should be terminated"
        );
    }

    #[test]
    fn detached_daemon_process_starts_in_its_own_session() {
        let parent_sid = unsafe { libc::getsid(0) };
        assert!(parent_sid > 0, "parent session id should be available");

        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        UnixPlatform::configure_detached_daemon_process(&mut cmd);

        let mut child = cmd.spawn().expect("spawn detached child");
        thread::sleep(Duration::from_millis(50));

        let child_pid = child.id() as libc::pid_t;
        let child_sid = unsafe { libc::getsid(child_pid) };
        let child_pgid = unsafe { libc::getpgid(child_pid) };

        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(
            child_sid, child_pid,
            "detached child should become a session leader"
        );
        assert_eq!(
            child_pgid, child_pid,
            "detached child should become its own process group leader"
        );
        assert_ne!(
            child_sid, parent_sid,
            "detached child should not remain in the parent's session"
        );
    }
}
