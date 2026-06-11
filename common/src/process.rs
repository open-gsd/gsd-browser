//! Process discovery and termination helpers shared by the CLI client
//! (`daemon stop` cleanup) and the daemon itself (pre-launch cleanup of
//! orphaned browsers still bound to a session's profile directory).

#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
pub fn wait_for_process_exit(pid: i32, max_wait: Duration) -> bool {
    let pid = nix::unistd::Pid::from_raw(pid);
    let start = Instant::now();
    while start.elapsed() < max_wait {
        match nix::sys::signal::kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => return true,
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    matches!(
        nix::sys::signal::kill(pid, None),
        Err(nix::errno::Errno::ESRCH)
    )
}

/// SIGTERM the process, escalating to SIGKILL if it does not exit promptly.
/// An already-dead process is treated as success.
#[cfg(unix)]
pub fn terminate_process(pid: i32, label: &str) -> Result<(), String> {
    let raw_pid = nix::unistd::Pid::from_raw(pid);
    match nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGTERM) {
        Ok(()) => {
            if !wait_for_process_exit(pid, Duration::from_secs(3)) {
                nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGKILL)
                    .map_err(|e| format!("failed to force stop {label} (PID {pid}): {e}"))?;
                if !wait_for_process_exit(pid, Duration::from_secs(1)) {
                    return Err(format!("failed to stop {label} (PID {pid}): still alive"));
                }
            }
            Ok(())
        }
        Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(e) => Err(format!("failed to stop {label} (PID {pid}): {e}")),
    }
}

/// Find live processes whose command line references the given profile
/// directory. Excludes the current process.
#[cfg(unix)]
pub fn pids_using_profile(profile_dir: &str) -> Result<Vec<i32>, String> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .map_err(|e| format!("failed to run ps: {e}"))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let current_pid = std::process::id() as i32;
    let processes = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in processes.lines() {
        if !line.contains(profile_dir) {
            continue;
        }
        let Some(pid_str) = line.split_whitespace().next() else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<i32>() else {
            continue;
        };
        if pid != current_pid {
            pids.push(pid);
        }
    }
    Ok(pids)
}

/// Kill every live process still bound to the profile directory. Returns the
/// PIDs that were terminated. Used before launching Chrome so an orphaned
/// browser holding the profile cannot break the CDP handshake.
#[cfg(unix)]
pub fn kill_processes_using_profile(profile_dir: &str) -> Result<Vec<i32>, String> {
    let pids = pids_using_profile(profile_dir)?;
    for pid in &pids {
        terminate_process(*pid, "orphaned browser process")?;
    }
    Ok(pids)
}

#[cfg(not(unix))]
pub fn pids_using_profile(_profile_dir: &str) -> Result<Vec<i32>, String> {
    Ok(Vec::new())
}

#[cfg(not(unix))]
pub fn kill_processes_using_profile(_profile_dir: &str) -> Result<Vec<i32>, String> {
    Ok(Vec::new())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    /// Create a temp dir with a marker file standing in for a profile path.
    fn create_marker_dir(tag: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("gsd-browser-test-profile-{tag}-"))
            .tempdir()
            .expect("create temp profile dir");
        let marker_file = dir.path().join("profile-marker");
        std::fs::write(&marker_file, b"").expect("create marker file");
        (dir, marker_file.display().to_string())
    }

    /// Spawn a long-running process whose command line contains the marker
    /// path, mimicking Chrome holding `--user-data-dir=<profile>`.
    fn spawn_holding_marker(tag: &str) -> (tempfile::TempDir, String, std::process::Child) {
        let (dir, marker) = create_marker_dir(tag);
        let child = Command::new("tail")
            .args(["-f", &marker])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn marker process");
        std::thread::sleep(Duration::from_millis(200));
        (dir, marker, child)
    }

    /// Spawn a marker-holding process that is NOT a child of the test process
    /// (the intermediate shell exits), matching the real orphaned-Chrome
    /// scenario. A direct child would linger as a zombie after SIGTERM,
    /// defeating the liveness check in `wait_for_process_exit`.
    fn spawn_orphaned_holder(tag: &str) -> (tempfile::TempDir, String, i32) {
        let (dir, marker) = create_marker_dir(tag);
        let output = Command::new("sh")
            .args([
                "-c",
                &format!("tail -f '{marker}' >/dev/null 2>&1 & echo $!"),
            ])
            .stdin(Stdio::null())
            .output()
            .expect("spawn orphaned marker process");
        let pid: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("parse orphaned holder PID");
        std::thread::sleep(Duration::from_millis(200));
        (dir, marker, pid)
    }

    #[test]
    fn pids_using_profile_finds_marked_process() {
        let (_dir, marker, mut child) = spawn_holding_marker("find");

        let pids = pids_using_profile(&marker).expect("pids_using_profile");
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            pids.contains(&(child.id() as i32)),
            "expected {pids:?} to contain child PID {}",
            child.id()
        );
    }

    #[test]
    fn pids_using_profile_returns_empty_for_unused_dir() {
        let marker = format!(
            "/tmp/gsd-browser-test-profile-unused-{}-nothing-references-this",
            std::process::id()
        );
        let pids = pids_using_profile(&marker).expect("pids_using_profile");
        assert!(pids.is_empty(), "expected no PIDs, got {pids:?}");
    }

    #[test]
    fn kill_processes_using_profile_terminates_holders() {
        let (_dir, marker, pid) = spawn_orphaned_holder("kill");

        let result = kill_processes_using_profile(&marker);
        // Best-effort cleanup if the kill failed, so the holder never leaks.
        if result.is_err() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let killed = result.expect("kill_processes_using_profile");
        assert!(
            killed.contains(&pid),
            "expected killed {killed:?} to contain holder PID {pid}"
        );
        assert!(
            wait_for_process_exit(pid, Duration::from_secs(2)),
            "holder should be dead after cleanup"
        );
    }

    #[test]
    fn terminate_process_treats_dead_pid_as_success() {
        let (_dir, _marker, mut child) = spawn_holding_marker("dead");
        let pid = child.id() as i32;
        let _ = child.kill();
        let _ = child.wait();
        assert!(terminate_process(pid, "test process").is_ok());
    }
}
