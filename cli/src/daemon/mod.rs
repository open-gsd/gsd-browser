pub mod capture;
pub mod handlers;
pub mod helpers;
pub mod input_dispatch;
pub mod inspection;
pub mod logs;
pub mod narration;
pub mod settle;
pub mod state;
pub mod view;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::network::EnableParams as NetworkEnableParams;
use chromiumoxide::cdp::browser_protocol::page::EnableParams as PageEnableParams;
use chromiumoxide::cdp::js_protocol::runtime::EnableParams as RuntimeEnableParams;
use chromiumoxide::Page;
use futures::StreamExt;
use gsd_browser_common::cloud::CloudToolRequest;
use gsd_browser_common::session::{
    now_epoch_secs, save_session_manifest, session_dir_for, SessionHealthStatus, SessionManifest,
};
use gsd_browser_common::{
    config::Config,
    identity::{identity_profile_dir, IdentityScope},
    ipc, pid_path_for, socket_path_for, state_dir,
    types::CompactPageState,
    validate_session_name, DaemonRequest, DaemonResponse, ERR_INTERNAL, ERR_INVALID_REQUEST,
    ERR_METHOD_NOT_FOUND,
};
use logs::DaemonLogs;
use serde_json::json;
use state::{DaemonState, SessionRuntime};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

const PAGE_URL_TIMEOUT: Duration = Duration::from_secs(2);

const DEFAULT_VIEWPORT_WIDTH: i64 = 1920;
const DEFAULT_VIEWPORT_HEIGHT: i64 = 1080;

/// Entry point for the daemon server. Called when the binary is invoked
/// with the hidden `_daemon` subcommand.
pub async fn run(
    browser_path: Option<String>,
    session: Option<String>,
    cdp_url: Option<String>,
    identity_scope: Option<String>,
    identity_key: Option<String>,
    identity_project_id: Option<String>,
    no_narration_delay: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing — respect GSD_BROWSER_DEBUG for verbose output
    let filter = if std::env::var("GSD_BROWSER_DEBUG").is_ok() {
        "debug"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    run_daemon(
        browser_path,
        session,
        cdp_url,
        identity_scope,
        identity_key,
        identity_project_id,
        no_narration_delay,
    )
    .await
}

async fn shutdown_signal() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sigterm = signal(SignalKind::terminate())?;

        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }

        Ok(())
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

fn browser_profile_dir(
    session: Option<&str>,
    identity_scope: Option<IdentityScope>,
    identity_key: Option<&str>,
    identity_project_id: Option<&str>,
) -> Result<PathBuf, String> {
    match (identity_scope, identity_key) {
        (Some(scope), Some(key)) => identity_profile_dir(scope, identity_project_id, key),
        (None, None) => Ok(session_dir_for(session).join("browser-profile")),
        _ => Err("identity profile requires both identity scope and key".to_string()),
    }
}

fn cleanup_browser_profile_singletons(profile_dir: &Path) {
    for artifact in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let path = profile_dir.join(artifact);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };

        let result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };

        if let Err(err) = result {
            warn!(
                "[gsd-browser-daemon] failed to remove browser profile artifact {:?}: {}",
                path, err
            );
        }
    }
}

pub(crate) async fn set_default_viewport(page: &Page) {
    let params = SetDeviceMetricsOverrideParams::new(
        DEFAULT_VIEWPORT_WIDTH,
        DEFAULT_VIEWPORT_HEIGHT,
        1.0,
        false,
    );
    if let Err(err) = page.execute(params).await {
        warn!("[gsd-browser-daemon] default viewport override failed (non-fatal): {err}");
        return;
    }
    info!(
        "[gsd-browser-daemon] default viewport set to {}x{}",
        DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT
    );
}
/// Apply stealth patches for anti-detection when --stealth / backend=stealth is active.
/// - Patches common CDP automation markers via preload JS and current-page JS
/// - Spoofs realistic navigator properties, hardware, locale, plugins
/// - Clears webdriver flag and automation-controlled hints
/// - Sets matching Client Hints via emulation (best effort)
/// This keeps the rest of the daemon (handlers using Page) unchanged.
async fn apply_stealth_patches(page: &Page, _config: &Config) {
    info!("[gsd-browser-daemon] applying stealth patches (UA/hardware/locale spoofing + CDP signal patches)");

    // 1. Core navigator.webdriver + automation flags removal.
    let stealth_js = r#"
    (() => {
        try {
            // webdriver
            Object.defineProperty(navigator, 'webdriver', { get: () => false, configurable: true });

            // cdc_ / $cdc_  (common chromiumoxide / chromedriver markers)
            for (const key of Object.keys(window)) {
                if (/^cdc_[a-zA-Z0-9]{22,}/i.test(key) || key.startsWith('$cdc_')) {
                    try { delete window[key]; } catch(e){}
                }
            }

            // navigator.webdriver related
            if (navigator.webdriver === undefined) {
                Object.defineProperty(navigator, 'webdriver', { get: () => false });
            }

            // Chrome object
            if (!window.chrome) {
                Object.defineProperty(window, 'chrome', { value: { runtime: {} }, configurable: true });
            }

            // Permissions
            const originalQuery = window.navigator.permissions.query;
            window.navigator.permissions.query = (parameters) => (
                parameters.name === 'notifications' ?
                    Promise.resolve({ state: Notification.permission }) :
                    originalQuery(parameters)
            );

            // Plugins / mimeTypes (make non-empty like real browser)
            Object.defineProperty(navigator, 'plugins', {
                get: () => [ { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer' } ],
                configurable: true
            });

            // Hardware / locale spoof (reasonable desktop values; can be refined per-profile later)
            Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 8, configurable: true });
            Object.defineProperty(navigator, 'deviceMemory', { get: () => 8, configurable: true });
            Object.defineProperty(navigator, 'platform', { get: () => 'MacIntel', configurable: true });

            // Languages
            Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'], configurable: true });
            Object.defineProperty(navigator, 'language', { get: () => 'en-US', configurable: true });

            // WebGL vendor / renderer spoof (generic but plausible)
            const getParameter = WebGLRenderingContext.prototype.getParameter;
            WebGLRenderingContext.prototype.getParameter = function(param) {
                if (param === 37445) return 'Intel Inc.'; // UNMASKED_VENDOR_WEBGL
                if (param === 37446) return 'Intel Iris OpenGL Engine'; // UNMASKED_RENDERER_WEBGL
                return getParameter.apply(this, [param]);
            };

            // Remove automation-controlled from document
            document.documentElement.setAttribute('data-automation-controlled', 'false');

            console.debug('[gsd-browser] stealth patches applied');
            return true;
        } catch (e) {
            console.warn('[gsd-browser] stealth patch partial failure', e);
            return false;
        }
    })();
    "#;

    if let Err(e) = page.evaluate_on_new_document(stealth_js).await {
        warn!("[gsd-browser-daemon] stealth preload patch failed (non-fatal): {e}");
    }
    if let Err(e) = page.evaluate_expression(stealth_js).await {
        warn!("[gsd-browser-daemon] stealth current-page patch failed (non-fatal): {e}");
    }

    // 2. Emulation: realistic UA override (v0.9 chromiumoxide compatible; advanced Client Hints require newer CDP)
    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
    let ua_override =
        chromiumoxide::cdp::browser_protocol::emulation::SetUserAgentOverrideParams::new(ua);
    if let Err(e) = page.execute(ua_override).await {
        warn!("[gsd-browser-daemon] SetUserAgentOverride (stealth) failed (non-fatal): {e}");
    } else {
        debug!("[gsd-browser-daemon] stealth UA override applied");
    }

    // 3. Also set a plausible locale / tz via emulation if available (best effort)
    // (Timezone/prefs often handled via launch profile or prefs; CDP has limited direct tz control)
    info!("[gsd-browser-daemon] stealth patches complete (realistic UA/hardware/locale + CDP signals)");
}

async fn run_daemon(
    browser_path_arg: Option<String>,
    session_arg: Option<String>,
    cdp_url_arg: Option<String>,
    identity_scope_arg: Option<String>,
    identity_key_arg: Option<String>,
    identity_project_id_arg: Option<String>,
    no_narration_delay: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load config (layers 1-4: defaults → user → project → env vars)
    let config = Config::load();
    info!(
        "[gsd-browser-daemon] config loaded (settle timeout={}ms, screenshot quality={})",
        config.settle.timeout_ms, config.screenshot.quality
    );

    // CLI flags override config
    let effective_browser_path = browser_path_arg.or_else(|| config.browser.path.clone());
    let effective_cdp_url = cdp_url_arg.or_else(|| config.browser.cdp_url.clone());

    let session = validate_session_name(session_arg.as_deref())
        .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
    let identity_scope = identity_scope_arg
        .as_deref()
        .map(IdentityScope::parse)
        .transpose()
        .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
    if identity_scope.is_none() && identity_key_arg.is_some() {
        return Err("--identity-key requires --identity-scope".into());
    }
    if identity_scope.is_none() && identity_project_id_arg.is_some() {
        return Err("--identity-project requires --identity-scope".into());
    }
    if identity_project_id_arg.is_some() && !matches!(identity_scope, Some(IdentityScope::Project))
    {
        return Err("--identity-project is only valid with --identity-scope=project".into());
    }
    if matches!(identity_scope, Some(IdentityScope::Project)) && identity_project_id_arg.is_none() {
        return Err("project identity requires --identity-project".into());
    }
    if identity_scope.is_some() && identity_key_arg.is_none() {
        return Err("identity profile requires --identity-key".into());
    }

    // Ensure state directory exists
    let state = state_dir();
    fs::create_dir_all(&state)?;

    // For session mode, ensure session subdir exists
    let sock_path = socket_path_for(session);
    let pid_file_path = pid_path_for(session);
    if let Some(parent) = sock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Clean up stale socket if exists
    if sock_path.exists() {
        // Check if old PID is alive
        let stale = if pid_file_path.exists() {
            let old_pid = fs::read_to_string(&pid_file_path)?
                .trim()
                .parse::<i32>()
                .ok();
            match old_pid {
                Some(pid) => {
                    // Check if process is alive via kill(pid, 0)
                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        None, // signal 0: check if process exists
                    )
                    .is_err()
                }
                None => true,
            }
        } else {
            true
        };

        if stale {
            warn!("[gsd-browser-daemon] removing stale socket");
            let _ = fs::remove_file(&sock_path);
            let _ = fs::remove_file(&pid_file_path);
        } else {
            return Err("daemon already running (socket exists and PID is alive)".into());
        }
    }

    // Write PID file
    fs::write(&pid_file_path, process::id().to_string())?;
    info!(
        "[gsd-browser-daemon] PID {} written to {:?}",
        process::id(),
        pid_file_path
    );

    let launch_mode = if effective_cdp_url.is_some() {
        "attached".to_string()
    } else {
        "launched".to_string()
    };
    let start_ts = now_epoch_secs();
    let starting_manifest = SessionManifest {
        manifest_version: 1,
        session_name: session.map(str::to_string),
        daemon_pid: Some(process::id() as i32),
        socket_path: sock_path.to_string_lossy().to_string(),
        daemon_started_at: Some(start_ts),
        browser_started_at: Some(start_ts),
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        launch_mode: launch_mode.clone(),
        cdp_url: effective_cdp_url.clone(),
        health: SessionHealthStatus::Starting,
        health_reason: "daemon starting".to_string(),
        last_updated_at: Some(start_ts),
        identity_scope,
        identity_project_id: identity_project_id_arg.clone(),
        identity_key: identity_key_arg.clone(),
        ..SessionManifest::default()
    };
    save_session_manifest(session, &starting_manifest)
        .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;

    let (browser, mut handler) = if let Some(ref cdp_url) = effective_cdp_url {
        // Connect to an already-running Chrome instance via CDP
        info!(
            "[gsd-browser-daemon] connecting to existing Chrome at {}",
            cdp_url
        );

        // chromiumoxide needs the WebSocket debugger URL. If the user passed
        // an HTTP endpoint (e.g. http://localhost:9222), fetch /json/version
        // to discover the ws URL automatically.
        let ws_url = if cdp_url.starts_with("ws://") || cdp_url.starts_with("wss://") {
            cdp_url.clone()
        } else {
            let version_url = format!("{}/json/version", cdp_url.trim_end_matches('/'));
            let body: serde_json::Value = reqwest::get(&version_url)
                .await
                .map_err(|e| {
                    format!("failed to reach Chrome debug endpoint at {version_url}: {e}")
                })?
                .json()
                .await
                .map_err(|e| format!("invalid JSON from {version_url}: {e}"))?;
            body["webSocketDebuggerUrl"]
                .as_str()
                .ok_or_else(|| format!("Chrome at {cdp_url} did not return webSocketDebuggerUrl — is --remote-debugging-port enabled?"))?
                .to_string()
        };

        info!("[gsd-browser-daemon] resolved WebSocket URL: {}", ws_url);
        let result =
            Browser::connect(&ws_url)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    format!("failed to connect to Chrome CDP at {ws_url}: {e}").into()
                })?;
        info!("[gsd-browser-daemon] connected to existing Chrome successfully");
        result
    } else {
        // Launch a new Chrome instance
        let chrome_path =
            gsd_browser_common::chrome::find_chrome(effective_browser_path.as_deref())
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        info!(
            "[gsd-browser-daemon] launching Chrome from {:?}",
            chrome_path
        );

        let profile_dir = browser_profile_dir(
            session,
            identity_scope,
            identity_key_arg.as_deref(),
            identity_project_id_arg.as_deref(),
        )
        .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
        fs::create_dir_all(&profile_dir)?;
        cleanup_browser_profile_singletons(&profile_dir);

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .user_data_dir(&profile_dir)
            .window_size(1920, 1080)
            .arg("--window-size=1920,1080");

        // Apply user-provided extra args from config
        for arg in &config.browser.args {
            builder = builder.arg(arg.as_str());
        }

        // Stealth / anti-detection launch args (when --stealth or backend=stealth/chaser)
        let effective_backend = config.browser.backend.as_deref().unwrap_or("chromiumoxide");
        let stealth_enabled = config.browser.stealth
            || effective_backend == "stealth"
            || effective_backend == "chaser-oxide";
        if stealth_enabled {
            info!(
                "[gsd-browser-daemon] stealth mode enabled (backend={})",
                effective_backend
            );
            let stealth_args = [
                "--disable-blink-features=AutomationControlled",
                "--disable-features=IsolateOrigins,site-per-process,TranslateUI",
                "--disable-site-isolation-trials",
                "--disable-web-security",
                "--disable-client-side-phishing-detection",
                "--disable-sync",
                "--disable-default-apps",
                "--disable-extensions",
                "--no-first-run",
                "--no-default-browser-check",
                "--no-pings",
                "--disable-background-networking",
                "--disable-background-timer-throttling",
                "--disable-backgrounding-occluded-windows",
                "--disable-breakpad",
                "--disable-component-update",
                "--disable-domain-reliability",
                "--disable-features=MediaRouter",
                "--metrics-recording-only",
                "--mute-audio",
                // Common realistic viewport / hardware hints via args (emulation later)
                "--force-device-scale-factor=1",
            ];
            for a in &stealth_args {
                builder = builder.arg(*a);
            }
            // Spoof a realistic UA (can be overridden by device emulation later)
            builder = builder.arg("--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
        }

        if !config.browser.headless {
            builder = builder.with_head();
        }
        let browser_config = builder
            .build()
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        let result = Browser::launch(browser_config).await?;
        info!(
            "[gsd-browser-daemon] Chrome launched successfully (stealth={})",
            stealth_enabled
        );
        result
    };

    // Handler must be polled continuously — spawn it
    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if let Err(err) = event {
                error!("[gsd-browser-daemon] browser handler error: {err}");
            }
        }
    });
    let browser = Arc::new(tokio::sync::Mutex::new(browser));

    // Create initial page
    let page = browser.lock().await.new_page("about:blank").await?;
    set_default_viewport(&page).await;
    info!("[gsd-browser-daemon] initial page created");

    // Inject browser-side helpers and install mutation counter
    helpers::inject_helpers(&page).await;
    settle::ensure_mutation_counter(&page).await;
    info!("[gsd-browser-daemon] browser helpers injected, mutation counter installed");

    // Apply stealth patches (CDP signals, navigator spoofing, realistic hardware/locale) if enabled
    let effective_backend = config.browser.backend.as_deref().unwrap_or("chromiumoxide");
    let stealth_enabled = config.browser.stealth
        || effective_backend == "stealth"
        || effective_backend == "chaser-oxide";
    if stealth_enabled {
        apply_stealth_patches(&page, &config).await;
    }

    // Enable CDP domains for event listening
    if let Err(e) = page.execute(RuntimeEnableParams::default()).await {
        warn!("[gsd-browser-daemon] Runtime.enable failed (non-fatal): {e}");
    } else {
        debug!("[gsd-browser-daemon] Runtime domain enabled");
    }
    if let Err(e) = page.execute(NetworkEnableParams::default()).await {
        warn!("[gsd-browser-daemon] Network.enable failed (non-fatal): {e}");
    } else {
        debug!("[gsd-browser-daemon] Network domain enabled");
    }
    if let Err(e) = page.execute(PageEnableParams::default()).await {
        warn!("[gsd-browser-daemon] Page.enable failed (non-fatal): {e}");
    } else {
        debug!("[gsd-browser-daemon] Page domain enabled");
    }
    info!("[gsd-browser-daemon] CDP domains enabled");

    // Create log buffers and spawn event listeners
    let daemon_logs = Arc::new(DaemonLogs::new());
    let (browser_pid, browser_user_data_dir, websocket_url) = {
        let mut browser_guard = browser.lock().await;
        let browser_pid = browser_guard
            .get_mut_child()
            .and_then(|child| child.as_mut_inner().id());
        let browser_user_data_dir = browser_guard
            .config()
            .and_then(|cfg| cfg.user_data_dir.as_ref())
            .map(|path| path.display().to_string());
        let websocket_url = browser_guard.websocket_address().clone();
        (browser_pid, browser_user_data_dir, websocket_url)
    };
    let daemon_state = Arc::new(DaemonState::new_with_session_and_options(
        SessionRuntime {
            session_name: session.map(str::to_string),
            launch_mode: launch_mode.clone(),
            cdp_url: effective_cdp_url.clone(),
            websocket_url: Some(websocket_url),
            browser_pid,
            browser_user_data_dir,
            identity_scope,
            identity_project_id: identity_project_id_arg.clone(),
            identity_key: identity_key_arg.clone(),
            socket_path: sock_path.to_string_lossy().to_string(),
        },
        no_narration_delay,
    ));
    logs::spawn_console_listener(&page, daemon_logs.console.clone()).await;
    logs::spawn_exception_listener(&page, daemon_logs.console.clone()).await;
    logs::spawn_network_listener(
        &page,
        daemon_logs.network.clone(),
        daemon_logs.current_recording_seq.clone(),
    )
    .await;
    logs::spawn_dialog_listener(&page, daemon_logs.dialog.clone()).await;
    info!("[gsd-browser-daemon] event listeners spawned");

    // PR-2 wiring (per-action network slicing): give RecordingStore the tagger (for listener stamping)
    // and the network buffer (for slice extraction at event emission time in record_event).
    {
        let mut rec = daemon_state.recordings.lock().await;
        rec.set_network_tagger(daemon_logs.current_recording_seq.clone());
        rec.set_network_buffer(daemon_logs.network.clone());
    }

    // Register initial page in the PageRegistry
    {
        let page_arc = Arc::new(page);
        let mut pages = daemon_state.pages.lock().unwrap();
        pages.register(page_arc, String::new(), "about:blank".to_string());
    }

    // Spawn the always-on target lifecycle tracker.
    // This subscribes to Target.targetCreated (and related events) so that
    // tabs opened via window.open(), target=_blank, or other clients appear
    // in list-pages / switch-page and have helpers injected.
    // Must be after the initial registration and before we start accepting commands.
    {
        let tb = Arc::clone(&browser);
        let ts = Arc::clone(&daemon_state);
        tokio::spawn(async move {
            handlers::pages::spawn_core_target_tracker(tb, ts).await;
        });
    }

    // Bind Unix socket
    let listener = UnixListener::bind(&sock_path)?;
    info!("[gsd-browser-daemon] listening on {:?}", sock_path);

    if let Some(page) = daemon_state.pages.lock().unwrap().active_page() {
        let state = Arc::clone(&daemon_state);
        tokio::spawn(async move {
            let _ = handlers::session::sync_session_manifest(
                page.as_ref(),
                &state,
                Some(SessionHealthStatus::Healthy),
                None,
            )
            .await;
        });
    }

    // Trap termination signals so `daemon stop` can shut Chrome down cleanly.
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        info!("[gsd-browser-daemon] connection accepted");
                        let logs = Arc::clone(&daemon_logs);
                        let state = Arc::clone(&daemon_state);
                        let browser = Arc::clone(&browser);
                        tokio::spawn(handle_connection(stream, logs, state, browser));
                    }
                    Err(e) => {
                        error!("[gsd-browser-daemon] accept error: {e}");
                    }
                }
            }
            _ = &mut shutdown => {
                info!("[gsd-browser-daemon] shutting down...");
                break;
            }
        }
    }

    // Clean shutdown
    if let Some(page) = daemon_state.pages.lock().unwrap().active_page() {
        let _ = handlers::session::sync_session_manifest(
            page.as_ref(),
            &daemon_state,
            Some(SessionHealthStatus::Stopped),
            Some("daemon stopped".to_string()),
        )
        .await;
    } else {
        let _ = handlers::session::mark_session_stopped(&daemon_state, "daemon stopped").await;
    }
    drop(listener);
    {
        let mut browser = browser.lock().await;
        let _ = browser.close().await;
        let _ = browser.wait().await;
    }
    handler_task.abort();
    let _ = fs::remove_file(&sock_path);
    let _ = fs::remove_file(&pid_file_path);
    info!("[gsd-browser-daemon] shutdown complete");

    Ok(())
}

async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    logs: Arc<DaemonLogs>,
    state: Arc<DaemonState>,
    browser: Arc<tokio::sync::Mutex<Browser>>,
) {
    let raw = match ipc::read_message(&mut stream).await {
        Ok(data) if data.is_empty() => return,
        Ok(data) => data,
        Err(e) => {
            error!("[gsd-browser-daemon] read error: {e}");
            return;
        }
    };

    let request: DaemonRequest = match serde_json::from_slice(&raw) {
        Ok(r) => r,
        Err(e) => {
            let resp = DaemonResponse::error(0, ERR_INTERNAL, format!("invalid request: {e}"));
            let payload = serde_json::to_vec(&resp).unwrap();
            let _ = ipc::write_message(&mut stream, &payload).await;
            return;
        }
    };

    info!(
        "[gsd-browser-daemon] request: method={} id={}",
        request.method, request.id
    );

    // Resolve the active page from the registry
    let page = {
        let pages = state.pages.lock().unwrap();
        pages.active_page()
    };

    let response = match page {
        Some(page) => dispatch(&request, &page, &logs, &state, &browser).await,
        None => DaemonResponse::error(
            request.id,
            ERR_INTERNAL,
            "no active page in registry".to_string(),
        ),
    };

    let payload = serde_json::to_vec(&response).unwrap();
    if let Err(e) = ipc::write_message(&mut stream, &payload).await {
        error!("[gsd-browser-daemon] write error: {e}");
    }
}

async fn dispatch(
    req: &DaemonRequest,
    page: &Page,
    logs: &DaemonLogs,
    state: &Arc<DaemonState>,
    browser: &Arc<tokio::sync::Mutex<Browser>>,
) -> DaemonResponse {
    // Determine if this method should be timeline-recorded
    let record_timeline = matches!(
        req.method.as_str(),
        "navigate"
            | "back"
            | "forward"
            | "reload"
            | "click"
            | "type"
            | "press"
            | "hover"
            | "scroll"
            | "select_option"
            | "set_checked"
            | "drag"
            | "snapshot"
            | "click_ref"
            | "hover_ref"
            | "fill_ref"
            | "assert"
            | "diff"
            | "wait_for"
            | "batch"
            | "fill_form"
            | "act"
    );

    // Params summary for timeline (truncated to 80 chars)
    let params_summary = if record_timeline {
        let s = req.params.to_string();
        if s.len() > 80 {
            format!("{}…", s.chars().take(79).collect::<String>())
        } else {
            s
        }
    } else {
        String::new()
    };

    // Record before-URL and begin action
    let action_id = if record_timeline {
        let before_url = bounded_page_url(page).await;
        let mut timeline = state.timeline.lock().unwrap();
        Some(timeline.begin_action(&req.method, &params_summary, &before_url))
    } else {
        None
    };

    // Dedicated per-dispatch before-state capture *for recording events only*.
    // Captured here (pre-dispatch_inner) using the page for *this* task.
    // Combined with symmetric post-dispatch recording_after, this delivers
    // race-free, correctly-paired before/after (with domHash + real session info)
    // for every record_timeline command — including assert, snapshot, diff, wait_for, batch.
    // Decouples evidence bundle material from global DiffState (which serves the "diff" tool
    // and can be mutated by handlers). Directly addresses replayable assertion correctness.
    let recording_before: Option<CompactPageState> = if record_timeline {
        Some(capture::capture_compact_page_state(page, false).await)
    } else {
        None
    };

    // Early (pre-dispatch_inner) session meta capture — paired with recording_before.
    // This ensures the "before" side of the session object in the recording event
    // reflects true pre-action state (cookies + storage counts/hash), even for
    // mutating actions (login, token writes, etc.). The late capture (below) serves "after".
    // Reuses the exact same lightweight helper (CDP + JS patterns from save-state).
    // Minimal change to deliver precise before/after session for replayable evidence.
    let recording_session_before: Option<serde_json::Value> = if record_timeline {
        Some(capture_basic_session_meta(page).await)
    } else {
        None
    };

    // Also store before-state in DiffState for navigate/click/etc.
    if matches!(
        req.method.as_str(),
        "navigate"
            | "back"
            | "forward"
            | "reload"
            | "click"
            | "type"
            | "press"
            | "hover"
            | "click_ref"
            | "hover_ref"
            | "fill_ref"
            | "fill_form"
            | "act"
    ) {
        let before_state = capture::capture_compact_page_state(page, false).await;
        let mut diff = state.diff.lock().unwrap();
        diff.before = Some(before_state);
    }

    // PR-2: arm the tagger *before* dispatch_inner (and settle) and *claim* the seq.
    // prepare_for_next_recorded_event now advances next_seq and stores pending_seq.
    // This tags networks during the action and guarantees monotonic seqs even under pause
    // interleaving (no reuse/pollution for replay artifacts). Return value captured for
    // future use / audit (the store's pending_seq is the source of truth).
    if record_timeline {
        let mut recs = state.recordings.lock().await;
        let _claimed_seq = recs.prepare_for_next_recorded_event();
    }

    let response = dispatch_inner(req, page, logs, state, browser).await;

    // Finish action in timeline
    if let Some(id) = action_id {
        let after_url = bounded_page_url(page).await;
        let (status, error) = if response.error.is_some() {
            (
                "error",
                response
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or(""),
            )
        } else {
            ("ok", "")
        };
        let mut timeline = state.timeline.lock().unwrap();
        timeline.finish_action(id, &after_url, status, error);
    }

    // Store after-state in DiffState for state-mutating methods
    if matches!(
        req.method.as_str(),
        "navigate"
            | "back"
            | "forward"
            | "reload"
            | "click"
            | "type"
            | "press"
            | "hover"
            | "click_ref"
            | "hover_ref"
            | "fill_ref"
            | "fill_form"
            | "act"
    ) {
        let after_state = capture::capture_compact_page_state(page, false).await;
        let mut diff = state.diff.lock().unwrap();
        diff.after = Some(after_state);
    }

    // Dedicated per-dispatch after-state capture for recording (post-dispatch_inner + settle).
    // Paired with recording_before above for consistent event enrichment.
    let recording_after: Option<CompactPageState> = if record_timeline {
        Some(capture::capture_compact_page_state(page, false).await)
    } else {
        None
    };

    if response.error.is_none() && should_sync_session_manifest(req.method.as_str()) {
        let _ = handlers::session::sync_session_manifest(page, state, None, None).await;
    }

    if record_timeline {
        let title = bounded_page_title(page).await;
        let url = bounded_page_url(page).await;
        let command_val = req.params.clone();

        // Use the *per-dispatch* recording_before/after (captured at exact boundaries for this action).
        // This guarantees correct pairing + domHash + session info even under concurrency or for
        // read-only record_timeline cmds (assert, snapshot, wait_for, etc.). Global DiffState is
        // left for the "diff" tool.
        let (before_val, after_val) = {
            let b = recording_before
                .as_ref()
                .map_or(serde_json::json!({}), |s| {
                    enrich_compact_for_recording(s, "before")
                });
            let a = recording_after.as_ref().map_or(serde_json::json!({}), |s| {
                enrich_compact_for_recording(s, "after")
            });
            (b, a)
        };

        let network_val = {
            let snaps: Vec<serde_json::Value> = logs
                .network
                .snapshot()
                .into_iter()
                .rev()
                .take(5)
                .map(|e| serde_json::json!({"url": e.url, "status": e.status, "method": e.method, "resourceType": e.resource_type}))
                .collect();
            serde_json::json!({ "recent": snaps, "tagging": "per-action-for-replayable-evidence", "note": "compat snapshot (PR-1); prefer networkSlice (PR-2) in the emitted event for authoritative per-action tagged requests" })
        };

        // Late (post-dispatch_inner) session meta capture — serves the "after" side.
        // Paired with the early recording_session_before (captured pre-dispatch_inner)
        // so that before/after session objects are correctly timed for mutating actions.
        // This resolves the asymmetry for precise replayable before/after comparisons
        // (login flows, auth handoff, storage writes, etc.).
        let session_meta_after = capture_basic_session_meta(page).await;

        // Merge correctly-timed session info (evolvable under before/after).
        let mut before_val = before_val;
        let mut after_val = after_val;
        if let serde_json::Value::Object(m) = &mut before_val {
            if let Some(early) = &recording_session_before {
                m.insert("session".to_string(), early.clone());
            } else {
                m.insert("session".to_string(), session_meta_after.clone());
            }
        }
        if let serde_json::Value::Object(m) = &mut after_val {
            m.insert("session".to_string(), session_meta_after);
        }

        let mut recordings = state.recordings.lock().await;
        if let Err(e) = recordings.record_event(view::recording::RecordingEventInput {
            source: "cli".to_string(),
            owner: "agent".to_string(),
            kind: req.method.clone(),
            url: url.clone(),
            title,
            redacted: false,
            command: command_val,
            before: before_val,
            after: after_val,
            network: network_val,
        }) {
            // Critical for durable replayable artifacts (CI, human review, export).
            // Never silent-drop; log with context so sequences stay trustworthy.
            error!(
                "[gsd-browser-daemon] record_event failed for kind={} url={} : {e}",
                req.method, url
            );
        }
    }

    response
}

async fn bounded_page_url(page: &Page) -> String {
    match timeout(PAGE_URL_TIMEOUT, page.url()).await {
        Ok(Ok(Some(url))) => url,
        Ok(Ok(None)) => String::new(),
        Ok(Err(err)) => {
            warn!("[gsd-browser-daemon] page url error: {err}");
            String::new()
        }
        Err(_) => {
            warn!("[gsd-browser-daemon] page url timed out");
            String::new()
        }
    }
}

async fn bounded_page_title(page: &Page) -> String {
    match timeout(PAGE_URL_TIMEOUT, page.get_title()).await {
        Ok(Ok(Some(title))) => title,
        Ok(Ok(None)) => String::new(),
        Ok(Err(err)) => {
            warn!("[gsd-browser-daemon] page title error: {err}");
            String::new()
        }
        Err(_) => {
            warn!("[gsd-browser-daemon] page title timed out");
            String::new()
        }
    }
}

/// Enrich a CompactPageState snapshot for inclusion in a RecordingEvent's before/after.
/// Adds domHash + sessionStateHash (via existing compute fns) + guards against silent loss
/// on serialization (Issue 6). Returns a Value suitable for the evolvable event schema.
/// Extracted to eliminate duplication (Issue 9) and centralize evidence-path logic.
fn enrich_compact_for_recording(state: &CompactPageState, position: &str) -> serde_json::Value {
    let mut v = match serde_json::to_value(state) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                "[gsd-browser-daemon] failed to serialize CompactPageState for recording {}: {e}",
                position
            );
            return serde_json::json!({
                "error": "serialization_failed",
                "position": position
            });
        }
    };
    if let serde_json::Value::Object(m) = &mut v {
        m.insert(
            "domHash".to_string(),
            serde_json::json!(view::recording::compute_dom_hash(state)),
        );
        // sessionStateHash will be overridden/enhanced by the per-event session_meta merge
        // in the caller (which has page access for real counts from save-state patterns).
        m.insert(
            "sessionStateHash".to_string(),
            serde_json::json!(view::recording::compute_session_state_hash()),
        );
    }
    v
}

/// Lightweight best-effort session meta (cookie count + storage key counts + composite hash)
/// captured via the same CDP/JS patterns as handle_save_state in state_persist.rs.
/// Used to populate real `session` object under before/after in recording events.
/// Basic scope for PR-1 (counts + hash of summary, no full values) to keep bundles small
/// while providing the raw material for replay/restore assertions. Full fidelity via
/// explicit browser_save_state.
async fn capture_basic_session_meta(page: &Page) -> serde_json::Value {
    // Cookies via CDP (best effort, non-blocking on error)
    let cookie_count = match page
        .execute(chromiumoxide::cdp::browser_protocol::network::GetCookiesParams::default())
        .await
    {
        Ok(resp) => resp.result.cookies.len() as u64,
        Err(_) => 0,
    };

    // Storage key counts via tiny JS (mirrors state_persist but only lengths)
    let (ls_count, ss_count) = {
        let js = r#"(() => {
            const ls = Object.keys(localStorage || {}).length;
            const ss = Object.keys(sessionStorage || {}).length;
            return {ls, ss};
        })()"#;
        match tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            page.evaluate_expression(js),
        )
        .await
        {
            Ok(Ok(eval_res)) => {
                if let Ok(val) = eval_res.into_value::<serde_json::Value>() {
                    (
                        val.get("ls").and_then(|v| v.as_u64()).unwrap_or(0),
                        val.get("ss").and_then(|v| v.as_u64()).unwrap_or(0),
                    )
                } else {
                    (0, 0)
                }
            }
            _ => (0, 0),
        }
    };

    let composite = format!("v1|c:{}|ls:{}|ss:{}", cookie_count, ls_count, ss_count);
    let hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(composite.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    serde_json::json!({
        "stateHash": format!("sha256:{}", hash),
        "cookieCount": cookie_count,
        "localStorageKeys": ls_count,
        "sessionStorageKeys": ss_count,
        "note": "basic-pr1-from-save-state-patterns; full content via save-state"
    })
}

fn should_sync_session_manifest(method: &str) -> bool {
    matches!(
        method,
        "navigate"
            | "back"
            | "forward"
            | "reload"
            | "click"
            | "type"
            | "press"
            | "hover"
            | "scroll"
            | "select_option"
            | "set_checked"
            | "drag"
            | "set_viewport"
            | "upload_file"
            | "click_ref"
            | "hover_ref"
            | "fill_ref"
            | "fill_form"
            | "act"
            | "batch"
            | "cloud_user_input"
            | "switch_page"
            | "close_page"
            | "select_frame"
            | "mock_route"
            | "block_urls"
            | "clear_routes"
            | "emulate_device"
            | "save_state"
            | "restore_state"
            | "vault_save"
            | "vault_login"
            | "trace_start"
            | "trace_stop"
    )
}

pub(crate) async fn dispatch_inner(
    req: &DaemonRequest,
    page: &Page,
    logs: &DaemonLogs,
    state: &Arc<DaemonState>,
    browser: &Arc<tokio::sync::Mutex<Browser>>,
) -> DaemonResponse {
    match req.method.as_str() {
        "ping" => DaemonResponse::success(req.id, json!({"pong": true})),
        "cloud_session_status" => {
            match handlers::cloud::handle_cloud_session_status(page, state).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "cloud_frame" => {
            match handlers::cloud::handle_cloud_frame(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "cloud_refs" => match handlers::cloud::handle_cloud_refs(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "cloud_methods" => match handlers::cloud_manifest::handle_cloud_methods() {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "cloud_tool" => dispatch_cloud_tool(req, page, logs, state, browser).await,
        "cloud_user_input" => {
            match handlers::cloud::handle_cloud_user_input(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "cloud_identity_list" => match handlers::cloud::handle_cloud_identity_list(&req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "cloud_identity_save" => match handlers::cloud::handle_cloud_identity_save(&req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "cloud_identity_revoke" => {
            match handlers::cloud::handle_cloud_identity_revoke(&req.params) {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "health" => match handlers::session::handle_health(page, state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "goal" => match handlers::narration_cmds::handle_goal(state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "pause" => match handlers::narration_cmds::handle_pause(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "resume" => match handlers::narration_cmds::handle_resume(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "step" => match handlers::narration_cmds::handle_step(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "abort" => match handlers::narration_cmds::handle_abort(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "control_state" => match handlers::narration_cmds::handle_control_state(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "takeover" => match handlers::narration_cmds::handle_takeover(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "release_control" => match handlers::narration_cmds::handle_release_control(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "sensitive_on" => match handlers::narration_cmds::handle_sensitive_on(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "sensitive_off" => match handlers::narration_cmds::handle_sensitive_off(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "view_status" => match handlers::narration_cmds::handle_view_status(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "view" => match handlers::narration_cmds::handle_view(state, page, browser).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "annotations" => match handlers::narration_cmds::handle_annotations(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "annotation_get" => {
            match handlers::narration_cmds::handle_annotation_get(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "annotation_clear" => {
            match handlers::narration_cmds::handle_annotation_clear(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "annotation_resolve" => {
            match handlers::narration_cmds::handle_annotation_resolve(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "annotation_export" => {
            match handlers::narration_cmds::handle_annotation_export(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "annotation_request" => {
            match handlers::narration_cmds::handle_annotation_request(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "record_start" => {
            match handlers::narration_cmds::handle_record_start(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "record_stop" => match handlers::narration_cmds::handle_record_stop(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "record_pause" => match handlers::narration_cmds::handle_record_pause(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "record_resume" => match handlers::narration_cmds::handle_record_resume(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "recordings" => match handlers::narration_cmds::handle_recordings(state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "recording_get" => {
            match handlers::narration_cmds::handle_recording_get(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "recording_export" => {
            match handlers::narration_cmds::handle_recording_export(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "recording_discard" => {
            match handlers::narration_cmds::handle_recording_discard(state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "recording_validate" => {
            match handlers::narration_cmds::handle_recording_validate(&req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "navigate" => match handlers::navigate::handle_navigate(page, &req.params, state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check URL is valid and reachable"}),
            ),
        },
        "back" => match handlers::navigate::handle_back(page, state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "forward" => match handlers::navigate::handle_forward(page, state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "reload" => match handlers::navigate::handle_reload(page, state).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "console" => match handlers::inspect::handle_console(logs, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "network" => match handlers::inspect::handle_network(logs, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "dialog" => match handlers::inspect::handle_dialog(logs, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "eval" => match handlers::inspect::handle_eval(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "click" => match handlers::interaction::handle_click(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check selector is valid and element exists"}),
            ),
        },
        "type" => match handlers::interaction::handle_type_text(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check selector targets an input/textarea element"}),
            ),
        },
        "press" => match handlers::interaction::handle_press(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "hover" => match handlers::interaction::handle_hover(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check selector is valid and element exists"}),
            ),
        },
        "scroll" => match handlers::interaction::handle_scroll(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "select_option" => {
            match handlers::interaction::handle_select_option(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "set_checked" => {
            match handlers::interaction::handle_set_checked(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "drag" => match handlers::interaction::handle_drag(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "set_viewport" => {
            match handlers::interaction::handle_set_viewport(page, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "upload_file" => {
            match handlers::interaction::handle_upload_file(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "screenshot" => match handlers::screenshot::handle_screenshot(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check selector is valid or try without --selector"}),
            ),
        },
        "accessibility_tree" => {
            match handlers::inspect::handle_accessibility_tree(page, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "find" => match handlers::inspect::handle_find(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "page_source" => {
            match handlers::inspect::handle_page_source(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "wait_for" => match handlers::wait::handle_wait_for(page, logs, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "timeline" => match handlers::timeline::handle_timeline(state, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "snapshot" => match handlers::refs::handle_snapshot(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "get_ref" => match handlers::refs::handle_get_ref(state, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "click_ref" => match handlers::refs::handle_click_ref(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check ref is valid and element still exists on page"}),
            ),
        },
        "hover_ref" => match handlers::refs::handle_hover_ref(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check ref is valid and element still exists on page"}),
            ),
        },
        "fill_ref" => match handlers::refs::handle_fill_ref(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check ref targets an input/textarea element"}),
            ),
        },
        "assert" => match handlers::assert_cmd::handle_assert(page, logs, state, &req.params).await
        {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "diff" => match handlers::assert_cmd::handle_diff(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "batch" => match handlers::batch::handle_batch(page, logs, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "list_pages" => match handlers::pages::handle_list_pages(state) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "switch_page" => match handlers::pages::handle_switch_page(state, &req.params).await {
            Ok((result, _new_page)) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "close_page" => match handlers::pages::handle_close_page(state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "list_frames" => match handlers::pages::handle_list_frames(page).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "select_frame" => match handlers::pages::handle_select_frame(state, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "analyze_form" => match handlers::forms::handle_analyze_form(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "fill_form" => match handlers::forms::handle_fill_form(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check field identifiers match form labels/names/placeholders"}),
            ),
        },
        "find_best" => match handlers::intent::handle_find_best(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "act" => match handlers::intent::handle_act(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error_with_data(
                req.id,
                ERR_INTERNAL,
                &msg,
                json!({"retryHint": "Check intent is valid and matching elements exist on page"}),
            ),
        },
        "session_summary" => {
            match handlers::session::handle_session_summary(page, logs, state).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "debug_bundle" => {
            match handlers::session::handle_debug_bundle(page, logs, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "visual_diff" => match handlers::visual_diff::handle_visual_diff(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "zoom_region" => match handlers::visual_diff::handle_zoom_region(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "save_pdf" => match handlers::pdf::handle_save_pdf(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "extract" => match handlers::extract::handle_extract(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "mock_route" => {
            match handlers::network_mock::handle_mock_route(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "block_urls" => {
            match handlers::network_mock::handle_block_urls(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "clear_routes" => {
            match handlers::network_mock::handle_clear_routes(page, state, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "emulate_device" => {
            match handlers::device::handle_emulate_device(page, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "save_state" => match handlers::state_persist::handle_save_state(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "restore_state" => {
            match handlers::state_persist::handle_restore_state(page, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "vault_save" => match handlers::auth_vault::handle_vault_save(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "vault_login" => {
            match handlers::auth_vault::handle_vault_login(page, &req.params, state).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "vault_list" => match handlers::auth_vault::handle_vault_list(page, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "action_cache" => match handlers::advanced::handle_action_cache(state, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "check_injection" => {
            match handlers::advanced::handle_check_injection(page, &req.params).await {
                Ok(result) => DaemonResponse::success(req.id, result),
                Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
            }
        }
        "generate_test" => match handlers::codegen::handle_generate_test(state, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "har_export" => match handlers::har::handle_har_export(logs, &req.params) {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "trace_start" => match handlers::traces::handle_trace_start(page, state, &req.params).await
        {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        "trace_stop" => match handlers::traces::handle_trace_stop(page, state, &req.params).await {
            Ok(result) => DaemonResponse::success(req.id, result),
            Err(msg) => DaemonResponse::error(req.id, ERR_INTERNAL, msg),
        },
        _ => DaemonResponse::error(
            req.id,
            ERR_METHOD_NOT_FOUND,
            format!("method not found: {}", req.method),
        ),
    }
}

async fn dispatch_cloud_tool(
    req: &DaemonRequest,
    page: &Page,
    logs: &DaemonLogs,
    state: &Arc<DaemonState>,
    browser: &Arc<tokio::sync::Mutex<Browser>>,
) -> DaemonResponse {
    let tool_req: CloudToolRequest = match serde_json::from_value(req.params.clone()) {
        Ok(value) => value,
        Err(err) => return DaemonResponse::error(req.id, ERR_INVALID_REQUEST, err.to_string()),
    };
    let Some(method) = handlers::cloud_methods::cloud_tool_method(&tool_req.method) else {
        return DaemonResponse::error(
            req.id,
            ERR_METHOD_NOT_FOUND,
            format!("unsupported cloud tool method: {}", tool_req.method),
        );
    };
    debug!(
        "[gsd-browser-daemon] cloud_tool dispatch: method={} category={}",
        method.name,
        method.category.as_str()
    );

    let forwarded = DaemonRequest {
        jsonrpc: req.jsonrpc.clone(),
        id: req.id,
        method: tool_req.method,
        params: tool_req.params,
    };
    Box::pin(dispatch(&forwarded, page, logs, state, browser)).await
}
