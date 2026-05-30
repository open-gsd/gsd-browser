const WINDOWS_DAEMON_UNSUPPORTED: &str =
    "native Windows daemon runtime is not supported yet; use WSL, macOS, or Linux for browser automation commands";

pub async fn run(
    _browser_path: Option<String>,
    _session: Option<String>,
    _cdp_url: Option<String>,
    _identity_scope: Option<String>,
    _identity_key: Option<String>,
    _identity_project_id: Option<String>,
    _no_narration_delay: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    Err(WINDOWS_DAEMON_UNSUPPORTED.into())
}
