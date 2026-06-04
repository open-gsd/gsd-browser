use gsd_browser_common::session::{
    load_session_manifest, manifest_path_for, now_epoch_secs, save_session_manifest,
    SessionHealthStatus,
};
use gsd_browser_common::{
    ipc, named_pipe_name_for, pid_path_for, state_dir, validate_session_name, DaemonRequest,
    DaemonResponse,
};
use serde_json::{json, Value};
use std::fs;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::time::{sleep, timeout};

fn read_daemon_pid(session: Option<&str>) -> Option<u32> {
    let pid_file = pid_path_for(session);
    let pid_str = fs::read_to_string(pid_file).ok()?;
    pid_str.trim().parse().ok()
}

fn cleanup_daemon_artifacts(session: Option<&str>) {
    let _ = fs::remove_file(pid_path_for(session));
}

fn write_stopped_manifest(session: Option<&str>, reason: &str) -> Result<(), String> {
    let mut manifest = load_session_manifest(session)?.unwrap_or_default();
    let now = now_epoch_secs();
    manifest.session_name = session.map(str::to_string);
    manifest.daemon_pid = None;
    manifest.health = SessionHealthStatus::Stopped;
    manifest.health_reason = reason.to_string();
    manifest.last_updated_at = Some(now);
    manifest.last_heartbeat_at = Some(now);
    manifest.socket_path = named_pipe_name_for(session);
    save_session_manifest(session, &manifest)
}

fn configure_detached_daemon_process(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

fn spawn_daemon_process(
    browser_path: Option<&str>,
    cdp_url: Option<&str>,
    session: Option<&str>,
    identity_scope: Option<&str>,
    identity_key: Option<&str>,
    identity_project_id: Option<&str>,
    no_narration_delay: bool,
) -> Result<Child, Box<dyn std::error::Error>> {
    let exe =
        std::env::current_exe().map_err(|e| format!("cannot determine current executable: {e}"))?;
    let mut cmd = Command::new(&exe);
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

    cmd.stdin(Stdio::null());
    if std::env::var_os("GSD_BROWSER_DEBUG").is_some() {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    configure_detached_daemon_process(&mut cmd);

    cmd.spawn()
        .map_err(|e| format!("failed to start daemon ({:?}): {}", exe, e).into())
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

async fn wait_for_spawned_daemon(
    session: Option<&str>,
    child: &mut Child,
    max_wait: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(50);

    while start.elapsed() < max_wait {
        if pipe_connectable(session).await {
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

pub fn is_daemon_alive(session: Option<&str>) -> bool {
    read_daemon_pid(validate_session_name(session).ok().flatten()).is_some()
}

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
    if pipe_connectable(session).await {
        return Ok(());
    }

    fs::create_dir_all(state_dir())?;
    if let Some(parent) = pid_path_for(session).parent() {
        fs::create_dir_all(parent)?;
    }

    let lock_file = gsd_browser_common::lock_path_for(session);
    if let Some(parent) = lock_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_file);
    let Ok(_lock) = lock else {
        return connect_pipe(session, Duration::from_secs(10))
            .await
            .map(|_| ());
    };

    cleanup_daemon_artifacts(session);
    let mut child = spawn_daemon_process(
        browser_path,
        cdp_url,
        session,
        identity_scope,
        identity_key,
        identity_project_id,
        no_narration_delay,
    )?;

    let result = wait_for_spawned_daemon(session, &mut child, Duration::from_secs(10)).await;
    let _ = fs::remove_file(lock_file);
    if result.is_err() {
        cleanup_daemon_artifacts(session);
    }
    result
}

pub fn stop_daemon(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    if let Some(pid) = read_daemon_pid(session) {
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .arg("/F")
            .status();
    }

    cleanup_daemon_artifacts(session);
    let _ = write_stopped_manifest(session, "daemon stopped");
    Ok(())
}

pub async fn collect_health(session: Option<&str>) -> Result<Value, Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    if let Ok(resp) = send_once("health", json!({}), session).await {
        if let Some(result) = resp.result {
            return Ok(result);
        }
    }

    let mut manifest = load_session_manifest(session)?.unwrap_or_default();
    manifest.session_name = session.map(str::to_string);
    manifest.socket_path = named_pipe_name_for(session);
    let pipe_connected = pipe_connectable(session).await;
    let daemon_alive = pipe_connected || read_daemon_pid(session).is_some();

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

    manifest.health = status;
    if !reason.is_empty() {
        manifest.health_reason = reason.clone();
    }
    manifest.daemon_pid = read_daemon_pid(session).map(|pid| pid as i32);
    manifest.last_updated_at = Some(now_epoch_secs());
    if status == SessionHealthStatus::Unhealthy || status == SessionHealthStatus::Stopped {
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
            "daemonAlive": daemon_alive,
            "socketConnected": pipe_connected,
            "browserConnected": false
        },
        "activePage": {
            "id": manifest.active_page_id.unwrap_or(0),
            "url": manifest.active_page_url,
            "title": manifest.active_page_title
        }
    }))
}

pub async fn send_request(
    method: &str,
    params: Value,
    browser_path: Option<&str>,
    cdp_url: Option<&str>,
    session: Option<&str>,
) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;
    if let Ok(resp) = send_once(method, params.clone(), session).await {
        return Ok(resp);
    }

    let identity_scope = std::env::var("GSD_BROWSER_IDENTITY_SCOPE").ok();
    let identity_key = std::env::var("GSD_BROWSER_IDENTITY_KEY").ok();
    let identity_project_id = std::env::var("GSD_BROWSER_IDENTITY_PROJECT").ok();
    let no_narration_delay = std::env::var_os("GSD_BROWSER_NO_NARRATION_DELAY").is_some();

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

async fn send_once(
    method: &str,
    params: Value,
    session: Option<&str>,
) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let mut stream = connect_pipe(session, Duration::from_secs(5)).await?;
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
