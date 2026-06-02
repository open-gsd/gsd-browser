use gsd_browser_common::session::manifest_path_for;
use gsd_browser_common::{validate_session_name, DaemonResponse};
use serde_json::{json, Value};

const WINDOWS_DAEMON_UNSUPPORTED: &str =
    "native Windows daemon runtime is not supported yet; use WSL, macOS, or Linux for browser automation commands";

fn unsupported_error() -> Box<dyn std::error::Error> {
    WINDOWS_DAEMON_UNSUPPORTED.into()
}

pub fn is_daemon_alive(_session: Option<&str>) -> bool {
    false
}

pub async fn start_daemon(
    _browser_path: Option<&str>,
    _cdp_url: Option<&str>,
    session: Option<&str>,
    _identity_scope: Option<&str>,
    _identity_key: Option<&str>,
    _identity_project_id: Option<&str>,
    _no_narration_delay: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = validate_session_name(session)?;
    Err(unsupported_error())
}

pub fn stop_daemon(session: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let _ = validate_session_name(session)?;
    Ok(())
}

pub async fn collect_health(session: Option<&str>) -> Result<Value, Box<dyn std::error::Error>> {
    let session = validate_session_name(session)?;

    Ok(json!({
        "session": {
            "name": session,
            "status": "stopped",
            "reason": WINDOWS_DAEMON_UNSUPPORTED,
            "daemonPid": null,
            "browserPid": null,
            "socketPath": null,
            "manifestPath": manifest_path_for(session).to_string_lossy().to_string(),
            "launchMode": null,
            "cdpUrl": null,
            "websocketUrl": null,
            "browserUserDataDir": null,
            "identityScope": null,
            "identityProjectId": null,
            "identityKey": null,
            "lastHeartbeatAt": null,
            "lastUpdatedAt": null,
            "daemonAlive": false,
            "socketConnected": false,
            "browserConnected": false
        },
        "activePage": {
            "id": 0,
            "url": "",
            "title": ""
        }
    }))
}

pub async fn send_request(
    _method: &str,
    _params: Value,
    _browser_path: Option<&str>,
    _cdp_url: Option<&str>,
    session: Option<&str>,
) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let _ = validate_session_name(session)?;
    Err(unsupported_error())
}
