//! Daemon lifecycle: start/stop/health/request orchestration shared across
//! platforms, with the genuinely platform-specific pieces (transport, locks,
//! process control) behind the `Platform` trait in `platform.rs`.

mod platform;
mod startup;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use gsd_browser_common::session::{
    load_session_manifest, manifest_path_for, now_epoch_secs, save_session_manifest,
    SessionHealthStatus, SessionManifest,
};
use gsd_browser_common::{ipc, validate_session_name, DaemonRequest, DaemonResponse};
use platform::{CurrentPlatform as P, Platform};
use serde_json::json;
use startup::daemon_startup_timeout;
use std::process::{Child, Stdio};
use std::time::Duration;
use tokio::time::{sleep, timeout};

pub fn is_daemon_alive(session: Option<&str>) -> bool {
    P::is_daemon_alive(session)
}

fn replacement_refused_error(
    session: Option<&str>,
    manifest: &SessionManifest,
) -> Box<dyn std::error::Error> {
    let stop_hint = match session {
        Some(name) => format!("gsd-browser --session {name} daemon stop"),
        None => "gsd-browser daemon stop".to_string(),
    };
    let session_label = session.unwrap_or("default");
    let reason = if manifest.health_reason.is_empty() {
        "session replacement requires explicit recovery".to_string()
    } else {
        manifest.health_reason.clone()
    };
    format!(
        "session '{session_label}' is in '{}' state ({reason}). Refusing to replace it automatically; run `{stop_hint}` and retry",
        manifest.health.as_str()
    )
    .into()
}

fn live_daemon_recovery_error(session: Option<&str>, context: &str) -> Box<dyn std::error::Error> {
    let stop_hint = match session {
        Some(name) => format!("gsd-browser --session {name} daemon stop"),
        None => "gsd-browser daemon stop".to_string(),
    };

    format!(
        "{context}. Refusing to replace a live browser session automatically; stop it with `{stop_hint}` and retry"
    )
    .into()
}

fn write_stopped_manifest(session: Option<&str>, reason: &str) -> Result<(), String> {
    let mut manifest = load_session_manifest(session)?.unwrap_or_default();
    let now = now_epoch_secs();
    manifest.session_name = session.map(str::to_string);
    manifest.daemon_pid = None;
    if P::CLEARS_BROWSER_PID_ON_STOP {
        manifest.browser_pid = None;
    }
    manifest.health = SessionHealthStatus::Stopped;
    manifest.health_reason = reason.to_string();
    manifest.last_updated_at = Some(now);
    manifest.last_heartbeat_at = Some(now);
    manifest.socket_path = P::endpoint_display(session);
    save_session_manifest(session, &manifest)
}

fn refuse_implicit_named_session_replacement(
    session: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(_) = session else {
        return Ok(());
    };
    let manifest = match load_session_manifest(session)? {
        Some(manifest) => manifest,
        None => return Ok(()),
    };
    if manifest.health == SessionHealthStatus::Stopped {
        return Ok(());
    }
    if is_daemon_alive(session) {
        return Ok(());
    }
    // The daemon PID is provably dead, so nothing live would be clobbered.
    // Self-heal the stale session the same way `daemon stop` does instead of
    // forcing the user to run it manually.
    if let Err(err) = P::cleanup_session_browser_processes(Some(&manifest)) {
        return Err(format!(
            "{}; automatic recovery failed: {err}",
            replacement_refused_error(session, &manifest)
        )
        .into());
    }
    P::cleanup_daemon_artifacts(session);
    write_stopped_manifest(session, "recovered stale session before restart")?;
    eprintln!(
        "[gsd-browser] session '{}' had no live daemon; cleaned up stale state, continuing start",
        session.unwrap_or("default")
    );
    Ok(())
}

/// Whether a live daemon must be restarted because it was built from a
/// different CLI version than the one issuing requests.
fn needs_version_restart(manifest_version: Option<&str>, cli_version: &str) -> bool {
    manifest_version.is_some_and(|version| version != cli_version)
}

fn build_serve_command(
    browser_path: Option<&str>,
    cdp_url: Option<&str>,
    session: Option<&str>,
    identity_scope: Option<&str>,
    identity_key: Option<&str>,
    identity_project_id: Option<&str>,
    no_narration_delay: bool,
) -> Result<std::process::Command, Box<dyn std::error::Error>> {
    // Spawn the daemon as a hidden subcommand of the current binary.
    let exe =
        std::env::current_exe().map_err(|e| format!("cannot determine current executable: {e}"))?;

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("_serve");
    if let Some(path) = browser_path {
        cmd.arg("--browser-path").arg(path);
    }
    if let Some(url) = cdp_url {
        cmd.arg("--cdp-url").arg(url);
    }
    if let Some(name) = session {
        cmd.arg("--session").arg(name);
    }
    if let Some(scope) = identity_scope {
        cmd.arg("--identity-scope").arg(scope);
    }
    if let Some(key) = identity_key {
        cmd.arg("--identity-key").arg(key);
    }
    if let Some(project_id) = identity_project_id {
        cmd.arg("--identity-project").arg(project_id);
    }
    if no_narration_delay {
        cmd.arg("--no-narration-delay");
    }

    // In debug mode, inherit daemon logs so startup failures are visible.
    cmd.stdin(Stdio::null());
    if std::env::var_os("GSD_BROWSER_DEBUG").is_some() {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    P::configure_detached_daemon_process(&mut cmd);
    Ok(cmd)
}

async fn wait_for_spawned_daemon(
    session: Option<&str>,
    child: &mut Child,
    max_wait: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    while start.elapsed() < max_wait {
        if P::probe(session).await {
            return Ok(());
        }

        if let Some(status) = child.try_wait()? {
            return Err(format!(
                "daemon exited during startup with status {status} — re-run with GSD_BROWSER_DEBUG=1 for startup logs"
            )
            .into());
        }

        sleep(poll_interval).await;
    }

    Err(format!(
        "daemon did not start within {}s — re-run with GSD_BROWSER_DEBUG=1 for startup logs",
        max_wait.as_secs()
    )
    .into())
}

/// Start the daemon process. Spawns the daemon binary in the background and
/// waits for its endpoint to become reachable.
pub async fn start_daemon(
    browser_path: Option<&str>,
    cdp_url: Option<&str>,
    session: Option<&str>,
    identity_scope: Option<&str>,
    identity_key: Option<&str>,
    identity_project_id: Option<&str>,
    no_narration_delay: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    let startup_timeout = daemon_startup_timeout();

    if P::ready_short_circuit(session).await {
        return Ok(());
    }

    P::prepare_state_dirs(session)?;

    let Some(lock) = P::acquire_startup_lock(session)? else {
        // Another process is starting the daemon — wait for its endpoint
        eprintln!("[gsd-browser] waiting for daemon start by another process...");
        return P::wait_ready(session, startup_timeout).await;
    };

    // We hold the lock — check if daemon is already alive (Unix only)
    if P::CHECKS_ALIVE_UNDER_LOCK && is_daemon_alive(session) {
        let result = if P::endpoint_ready(session).await {
            Ok(())
        } else {
            match P::wait_ready(session, startup_timeout).await {
                Ok(()) => Ok(()),
                Err(_) => Err(live_daemon_recovery_error(
                    session,
                    "daemon PID is alive but its socket is unavailable",
                )),
            }
        };

        P::release_startup_lock(session, lock);
        return result;
    }

    if P::GUARDS_IMPLICIT_REPLACEMENT {
        if let Err(err) = refuse_implicit_named_session_replacement(session) {
            P::release_startup_lock(session, lock);
            return Err(err);
        }
    }

    // Clean up stale files
    P::cleanup_daemon_artifacts(session);

    let mut cmd = match build_serve_command(
        browser_path,
        cdp_url,
        session,
        identity_scope,
        identity_key,
        identity_project_id,
        no_narration_delay,
    ) {
        Ok(cmd) => cmd,
        Err(err) => {
            P::release_startup_lock(session, lock);
            return Err(err);
        }
    };
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            P::release_startup_lock(session, lock);
            return Err(format!("failed to start daemon ({:?}): {}", cmd.get_program(), e).into());
        }
    };

    // Wait for the endpoint and fail fast if the daemon exits during startup.
    let result = wait_for_spawned_daemon(session, &mut child, startup_timeout).await;
    if result.is_err() {
        P::cleanup_daemon_artifacts(session);
    }

    P::release_startup_lock(session, lock);

    result
}

/// Stop the daemon. Treats an already-dead process as success and always
/// cleans up stale files.
pub fn stop_daemon(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    let manifest = load_session_manifest(session).ok().flatten();
    let had_pid = P::terminate_daemon(session)?;
    let cleanup_error = P::cleanup_session_browser_processes(manifest.as_ref()).err();

    // Always clean up stale files
    P::remove_stop_artifacts(session);
    let _ = write_stopped_manifest(session, "daemon stopped");

    if had_pid {
        if let Some(err) = cleanup_error {
            return Err(err);
        }
    }

    Ok(())
}

pub async fn collect_health(
    session: Option<&str>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    let manifest = load_session_manifest(session)?;

    if let Some(result) = P::try_daemon_health(session).await {
        return Ok(result);
    }

    let mut manifest = manifest.unwrap_or_default();
    manifest.session_name = session.map(str::to_string);
    manifest.socket_path = P::endpoint_display(session);

    let health = P::classify_offline_health(session, &manifest).await;

    manifest.health = health.status;
    if !health.reason.is_empty() {
        manifest.health_reason = health.reason.clone();
    }
    manifest.daemon_pid = health.daemon_pid;
    manifest.last_updated_at = Some(now_epoch_secs());
    if health.status == SessionHealthStatus::Unhealthy
        || health.status == SessionHealthStatus::Stopped
    {
        let _ = save_session_manifest(session, &manifest);
    }

    Ok(json!({
        "session": {
            "name": manifest.session_name,
            "status": manifest.health.as_str(),
            "reason": manifest.health_reason,
            "daemonPid": manifest.daemon_pid,
            "browserPid": manifest.browser_pid,
            "socketPath": manifest.socket_path,
            "manifestPath": manifest_path_for(session).to_string_lossy().to_string(),
            "launchMode": manifest.launch_mode,
            "cdpUrl": manifest.cdp_url,
            "websocketUrl": manifest.websocket_url,
            "browserUserDataDir": manifest.browser_user_data_dir,
            "identityScope": manifest.identity_scope,
            "identityProjectId": manifest.identity_project_id,
            "identityKey": manifest.identity_key.as_ref().map(|_| "<redacted>"),
            "lastHeartbeatAt": manifest.last_heartbeat_at,
            "lastUpdatedAt": manifest.last_updated_at,
            "daemonAlive": health.daemon_alive,
            "socketConnected": health.socket_connected,
            "browserConnected": false,
        },
        "activePage": {
            "id": manifest.active_page_id.unwrap_or(0),
            "url": manifest.active_page_url,
            "title": manifest.active_page_title,
        }
    }))
}

/// Send a JSON-RPC request to the daemon. Auto-starts daemon if not running,
/// and restarts a live daemon whose version differs from this CLI.
pub async fn send_request(
    method: &str,
    params: serde_json::Value,
    browser_path: Option<&str>,
    cdp_url: Option<&str>,
    session: Option<&str>,
) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    let identity_scope = std::env::var("GSD_BROWSER_IDENTITY_SCOPE").ok();
    let identity_key = std::env::var("GSD_BROWSER_IDENTITY_KEY").ok();
    let identity_project_id = std::env::var("GSD_BROWSER_IDENTITY_PROJECT").ok();
    let no_narration_delay = std::env::var_os("GSD_BROWSER_NO_NARRATION_DELAY").is_some();

    if is_daemon_alive(session) {
        let manifest_version = load_session_manifest(session)
            .ok()
            .flatten()
            .map(|manifest| manifest.daemon_version)
            .filter(|version| !version.is_empty());
        let cli_version = env!("CARGO_PKG_VERSION");
        if needs_version_restart(manifest_version.as_deref(), cli_version) {
            eprintln!(
                "[gsd-browser] daemon version {} differs from CLI {cli_version}; restarting",
                manifest_version.as_deref().unwrap_or_default()
            );
            stop_daemon(session)?;
        }
    }

    // Ensure daemon is running
    if !is_daemon_alive(session) || !P::endpoint_ready(session).await {
        start_daemon(
            browser_path,
            cdp_url,
            session,
            identity_scope.as_deref(),
            identity_key.as_deref(),
            identity_project_id.as_deref(),
            no_narration_delay,
        )
        .await?;
    }

    // Connect and send
    match send_once(method, params.clone(), session).await {
        Ok(resp) => Ok(resp),
        Err(err) => {
            if is_daemon_alive(session) {
                return Err(live_daemon_recovery_error(
                    session,
                    &format!("request failed while the daemon PID was still alive: {err}"),
                ));
            }

            // Connection failed and the daemon is gone — restart and retry once.
            if P::GUARDS_IMPLICIT_REPLACEMENT {
                refuse_implicit_named_session_replacement(session)?;
            }
            eprintln!("[gsd-browser] daemon connection failed, restarting...");
            P::cleanup_daemon_artifacts(session);
            start_daemon(
                browser_path,
                cdp_url,
                session,
                identity_scope.as_deref(),
                identity_key.as_deref(),
                identity_project_id.as_deref(),
                no_narration_delay,
            )
            .await?;
            send_once(method, params, session).await
        }
    }
}

async fn send_once(
    method: &str,
    params: serde_json::Value,
    session: Option<&str>,
) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let mut stream = P::connect(session, Duration::from_secs(5)).await?;

    let req = DaemonRequest::new(1, method, params);
    let payload = serde_json::to_vec(&req)?;

    timeout(
        Duration::from_secs(30),
        ipc::write_message(&mut stream, &payload),
    )
    .await
    .map_err(|_| "timeout writing request to daemon")??;

    let raw = timeout(Duration::from_secs(30), ipc::read_message(&mut stream))
        .await
        .map_err(|_| "timeout reading response from daemon")?
        .map_err(|e| format!("error reading response: {e}"))?;

    if raw.is_empty() {
        return Err("daemon closed connection without response".into());
    }

    let resp: DaemonResponse = serde_json::from_slice(&raw)?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::needs_version_restart;

    #[test]
    fn version_restart_skipped_without_manifest_version() {
        assert!(!needs_version_restart(None, "0.1.29"));
    }

    #[test]
    fn version_restart_skipped_when_versions_match() {
        assert!(!needs_version_restart(Some("0.1.29"), "0.1.29"));
    }

    #[test]
    fn version_restart_required_when_versions_differ() {
        assert!(needs_version_restart(Some("0.1.28"), "0.1.29"));
    }
}
