//! Minimal MCP stdio server for gsd-browser.
//!
//! Exposes the rich browser automation surface as MCP tools so agents
//! (Cursor, Claude Desktop, VS Code + Copilot, etc.) can discover and call them
//! automatically.
//!
//! Design goals for the prototype:
//! - Reuse the existing daemon_client for all real work (auto-start, sessions,
//!   JSON output, error handling, etc.).
//! - Keep the implementation small and dependency-light for the first slice.
//! - Make the most valuable commands available as tools on day one.

use crate::Cli;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json as AxumJson, Router,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::net::IpAddr;
use std::sync::Arc;

const LATEST_MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_MCP_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Options for hosting the MCP server over HTTP.
#[derive(Clone, Debug)]
pub struct HttpServerOptions {
    pub host: String,
    pub port: u16,
    pub auth_token: Option<String>,
    pub auth_verify_url: Option<String>,
    pub allow_no_auth: bool,
}

#[derive(Clone)]
struct HttpState {
    cli: Cli,
    auth_token: Option<String>,
    auth_verify_url: Option<String>,
    http_client: reqwest::Client,
}

/// Top-level entry point called from the `mcp` subcommand.
pub async fn run_stdio_server(cli: &Cli) -> crate::CmdResult {
    // Run the stdio MCP loop in a dedicated thread. We keep error handling
    // simple for the prototype (any panic or error becomes a string).
    let cli = cli.clone();
    let handle = std::thread::spawn(move || {
        if let Err(e) = run_stdio_loop(&cli) {
            eprintln!("MCP server exited with error: {e}");
        }
    });

    // For an MCP server we intentionally run forever (until stdin closes or
    // the process is killed). The main thread just waits.
    let _ = handle.join();
    Ok(())
}

/// Host the MCP server as a stateless Streamable HTTP endpoint.
///
/// POST JSON-RPC requests to `/mcp`. For public binds, require a bearer token
/// unless the operator explicitly passes `--no-auth`.
pub async fn run_http_server(cli: &Cli, options: HttpServerOptions) -> crate::CmdResult {
    validate_http_options(&options).map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;

    let bind_addr = format_bind_address(&options.host, options.port);
    let state = Arc::new(HttpState {
        cli: cli.clone(),
        auth_token: if options.allow_no_auth {
            None
        } else {
            options.auth_token.clone()
        },
        auth_verify_url: if options.allow_no_auth {
            None
        } else {
            options.auth_verify_url.clone()
        },
        http_client: reqwest::Client::new(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", post(handle_http_mcp))
        .with_state(state);

    eprintln!(
        "gsd-browser MCP HTTP listening on http://{bind_addr}/mcp{}",
        if options.allow_no_auth {
            ""
        } else if options.auth_verify_url.is_some() {
            " (remote bearer auth required)"
        } else if options.auth_token.is_some() {
            " (bearer auth required)"
        } else {
            ""
        }
    );

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> impl IntoResponse {
    AxumJson(json!({
        "ok": true,
        "server": "gsd-browser",
        "transport": "streamable-http",
        "mcpPath": "/mcp"
    }))
}

async fn handle_http_mcp(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    AxumJson(request): AxumJson<Value>,
) -> Response {
    if let Err(response) = authorize_http_request(
        &headers,
        state.auth_token.as_deref(),
        state.auth_verify_url.as_deref(),
        &state.http_client,
        &request,
    )
    .await
    {
        return response;
    }

    if is_json_rpc_notification(&request) {
        return StatusCode::ACCEPTED.into_response();
    }

    let cli = state.cli.clone();
    match tokio::task::spawn_blocking(move || handle_request(&request, &cli)).await {
        Ok(response) => (StatusCode::OK, AxumJson(response)).into_response(),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("MCP request task failed: {err}"),
        ),
    }
}

async fn authorize_http_request(
    headers: &HeaderMap,
    auth_token: Option<&str>,
    auth_verify_url: Option<&str>,
    http_client: &reqwest::Client,
    request: &Value,
) -> Result<(), Response> {
    if auth_token.is_none() && auth_verify_url.is_none() {
        return Ok(());
    }

    let provided = bearer_token(headers);

    if auth_token.is_some_and(|expected| provided == Some(expected)) {
        return Ok(());
    }

    let Some(auth_verify_url) = auth_verify_url else {
        let mut response = json_error(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
        return Err(response);
    };

    let Some(provided) = provided else {
        let mut response = json_error(StatusCode::UNAUTHORIZED, "missing bearer token");
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
        return Err(response);
    };

    verify_remote_bearer(http_client, auth_verify_url, provided, request).await
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let mut parts = value.split_whitespace();
            let scheme = parts.next()?;
            let token = parts.next()?;
            if parts.next().is_none() && scheme.eq_ignore_ascii_case("bearer") {
                Some(token)
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
}

async fn verify_remote_bearer(
    http_client: &reqwest::Client,
    auth_verify_url: &str,
    token: &str,
    request: &Value,
) -> Result<(), Response> {
    let response = http_client
        .post(auth_verify_url)
        .bearer_auth(token)
        .json(&build_auth_verification_body(request))
        .send()
        .await
        .map_err(|err| {
            tracing::warn!("MCP auth verifier request failed: {err}");
            json_error(StatusCode::BAD_GATEWAY, "auth verifier unavailable")
        })?;
    let status = response.status();

    if status.is_success() {
        let body = response.json::<Value>().await.unwrap_or_else(|_| json!({}));
        if body.get("ok").and_then(|value| value.as_bool()) == Some(true) {
            return Ok(());
        }

        return Err(json_error(
            StatusCode::UNAUTHORIZED,
            "auth verifier rejected bearer token",
        ));
    }

    let body = response.json::<Value>().await.unwrap_or_else(|_| json!({}));
    let message = body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or("auth verifier rejected bearer token");
    let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    Err(json_error(status, message))
}

fn build_auth_verification_body(request: &Value) -> Value {
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("mcp");
    let is_tool_call = method == "tools/call";
    let tool_name = if is_tool_call {
        request
            .get("params")
            .and_then(|params| params.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or("tools/call")
    } else {
        method
    };
    let request_id = request
        .get("id")
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_else(|| "null".to_string());

    json!({
        "billable": is_tool_call,
        "recordUsage": is_tool_call,
        "requestId": request_id,
        "runtime": "gsd-browser",
        "toolName": tool_name
    })
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        AxumJson(json!({
            "error": {
                "message": message.into()
            }
        })),
    )
        .into_response()
}

fn validate_http_options(options: &HttpServerOptions) -> Result<(), String> {
    if options.allow_no_auth
        || options
            .auth_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
        || options
            .auth_verify_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
    {
        return Ok(());
    }
    if is_loopback_bind_host(&options.host) {
        return Ok(());
    }
    Err(
        "refusing to expose unauthenticated gsd-browser MCP on a non-loopback host; set GSD_BROWSER_MCP_AUTH_TOKEN, pass --auth-token, set GSD_BROWSER_MCP_AUTH_VERIFY_URL, pass --auth-verify-url, or explicitly pass --no-auth"
            .to_string(),
    )
}

fn is_loopback_bind_host(host: &str) -> bool {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    trimmed
        .parse::<IpAddr>()
        .map(|addr| addr.is_loopback())
        .unwrap_or(false)
}

fn format_bind_address(host: &str, port: u16) -> String {
    let trimmed = host.trim();
    if trimmed.starts_with('[') || !trimmed.contains(':') {
        format!("{trimmed}:{port}")
    } else {
        format!("[{trimmed}]:{port}")
    }
}

fn is_json_rpc_notification(request: &Value) -> bool {
    request.get("id").is_none()
        && request
            .get("method")
            .and_then(|method| method.as_str())
            .is_some()
}

fn negotiated_protocol_version(request: &Value) -> &'static str {
    let requested = request
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(|value| value.as_str());

    requested
        .and_then(|version| {
            SUPPORTED_MCP_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| *supported == version)
        })
        .unwrap_or(LATEST_MCP_PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn public_http_bind_requires_auth_by_default() {
        let err = validate_http_options(&HttpServerOptions {
            host: "0.0.0.0".to_string(),
            port: 8788,
            auth_token: None,
            auth_verify_url: None,
            allow_no_auth: false,
        })
        .expect_err("public unauthenticated bind should fail");

        assert!(err.contains("refusing to expose unauthenticated"));
    }

    #[test]
    fn loopback_http_bind_can_run_without_auth() {
        validate_http_options(&HttpServerOptions {
            host: "127.0.0.1".to_string(),
            port: 8788,
            auth_token: None,
            auth_verify_url: None,
            allow_no_auth: false,
        })
        .expect("loopback dev bind should be allowed");
    }

    #[test]
    fn public_http_bind_accepts_auth_token() {
        validate_http_options(&HttpServerOptions {
            host: "0.0.0.0".to_string(),
            port: 8788,
            auth_token: Some("secret".to_string()),
            auth_verify_url: None,
            allow_no_auth: false,
        })
        .expect("bearer token should allow public bind");
    }

    #[test]
    fn public_http_bind_accepts_remote_auth_verifier() {
        validate_http_options(&HttpServerOptions {
            host: "0.0.0.0".to_string(),
            port: 8788,
            auth_token: None,
            auth_verify_url: Some("https://mcp.opengsd.dev/api/mcp/tokens/verify".to_string()),
            allow_no_auth: false,
        })
        .expect("remote auth verifier should allow public bind");
    }

    #[tokio::test]
    async fn bearer_auth_header_is_required_when_token_configured() {
        let mut headers = HeaderMap::new();
        let http_client = reqwest::Client::new();
        assert!(
            authorize_http_request(&headers, Some("secret"), None, &http_client, &json!({}))
                .await
                .is_err()
        );

        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_static("Bearer secret"),
        );
        authorize_http_request(&headers, Some("secret"), None, &http_client, &json!({}))
            .await
            .expect("valid bearer token");
    }

    #[test]
    fn auth_verification_body_bills_tool_calls_only() {
        let body = build_auth_verification_body(&json!({
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "browser_navigate"
            }
        }));

        assert_eq!(body["billable"], true);
        assert_eq!(body["recordUsage"], true);
        assert_eq!(body["runtime"], "gsd-browser");
        assert_eq!(body["toolName"], "browser_navigate");

        let body = build_auth_verification_body(&json!({
            "id": 8,
            "method": "tools/list"
        }));

        assert_eq!(body["billable"], false);
        assert_eq!(body["recordUsage"], false);
        assert_eq!(body["toolName"], "tools/list");
    }

    #[test]
    fn ipv6_bind_addresses_are_bracketed() {
        assert_eq!(format_bind_address("::1", 8788), "[::1]:8788");
        assert_eq!(format_bind_address("[::1]", 8788), "[::1]:8788");
    }

    #[test]
    fn initialize_negotiates_supported_protocol_versions() {
        let cli = Cli::parse_from(["gsd-browser", "mcp"]);

        let latest = handle_request(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18"
                }
            }),
            &cli,
        );
        assert_eq!(latest["result"]["protocolVersion"], "2025-06-18");

        let fallback = handle_request(
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialize",
                "params": {
                    "protocolVersion": "1999-01-01"
                }
            }),
            &cli,
        );
        assert_eq!(fallback["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn notifications_are_detected_without_requiring_response() {
        assert!(is_json_rpc_notification(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })));

        assert!(!is_json_rpc_notification(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        })));
    }
}

/// The actual line-oriented JSON-RPC loop over stdin/stdout.
fn run_stdio_loop(cli: &Cli) -> crate::CmdResult {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("MCP: stdin read error: {e}");
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let error = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {e}")
                    }
                });
                let _ = writeln!(stdout, "{}", error);
                let _ = stdout.flush();
                continue;
            }
        };

        if is_json_rpc_notification(&request) {
            continue;
        }

        let response = handle_request(&request, cli);

        let response_str = serde_json::to_string(&response).unwrap();
        writeln!(stdout, "{}", response_str)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_request(request: &Value, cli: &Cli) -> Value {
    let jsonrpc = "2.0";
    let id = request.get("id").cloned().unwrap_or(json!(null));
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "initialize" => {
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "result": {
                    "protocolVersion": negotiated_protocol_version(request),
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "listChanged": false },
                        "prompts": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "gsd-browser",
                        "title": "gsd-browser",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "Use tools/list, resources/list, and prompts/list to discover the live gsd-browser surface. Prefer browser_snapshot or gsd-browser://latest-snapshot before ref-based actions."
                }
            })
        }

        "tools/list" => {
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "result": {
                    "tools": build_tool_list()
                }
            })
        }

        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match handle_tool_call_blocking(tool_name, arguments, cli) {
                Ok(result) => {
                    // Powerful, standardized agent envelope for maximum value.
                    // Every successful tool returns:
                    // - summary: short, clear outcome
                    // - structured_data: full parsable result (when JSON)
                    // - suggested_next_actions: concrete hints to keep the agent on the optimal path
                    // - evidence_refs: pointers to recordings, viewer, annotations, debug bundles
                    let parsed: Option<Value> = serde_json::from_str(&result).ok();

                    let summary = format!("✅ {} completed successfully", tool_name);

                    let mut suggested_next: Vec<String> = vec![];
                    if tool_name.contains("snapshot") || tool_name.contains("navigate") {
                        suggested_next.push("Re-snapshot (browser_snapshot) before any _ref or interaction tools after page changes".to_string());
                        suggested_next.push("Use browser_wait_for with network_idle or selector_visible on dynamic pages".to_string());
                    }
                    if tool_name.contains("act")
                        || tool_name.contains("click_ref")
                        || tool_name.contains("fill_ref")
                    {
                        suggested_next.push("Always re-snapshot after actions that cause navigation or major DOM updates".to_string());
                    }
                    if tool_name.contains("view")
                        || tool_name.contains("takeover")
                        || tool_name.contains("annotation")
                    {
                        suggested_next.push("Leverage the live viewer + annotations for human collaboration and evidence".to_string());
                    }

                    let evidence_refs = if tool_name.contains("record")
                        || tool_name.contains("annotation")
                        || tool_name.contains("debug")
                        || tool_name.contains("visual")
                    {
                        json!({"note": "Recordings, annotations and evidence bundles are first-class. Use browser resources and recording tools to manage them."})
                    } else {
                        json!(null)
                    };

                    let envelope = json!({
                        "summary": summary,
                        "structured_data": parsed,
                        "suggested_next_actions": suggested_next,
                        "evidence_refs": evidence_refs,
                        "raw_fallback": if parsed.is_some() { Value::Null } else { json!(result) }
                    });

                    json!({
                        "jsonrpc": jsonrpc,
                        "id": id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("```json\n{}\n```", serde_json::to_string_pretty(&envelope).unwrap())
                                }
                            ]
                        }
                    })
                }
                Err(err) => {
                    json!({
                        "jsonrpc": jsonrpc,
                        "id": id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("❌ Error in {}: {}", tool_name, err)
                                }
                            ],
                            "isError": true
                        }
                    })
                }
            }
        }

        "resources/list" => {
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "result": {
                    "resources": [
                        {
                            "uri": "gsd-browser://current-state",
                            "name": "Current Page State",
                            "description": "Latest known page title, URL, and high-level structure",
                            "mimeType": "application/json"
                        },
                        {
                            "uri": "gsd-browser://latest-snapshot",
                            "name": "Latest Snapshot + Refs",
                            "description": "Most recent versioned element snapshot with refs",
                            "mimeType": "application/json"
                        },
                        {
                            "uri": "gsd-browser://active-recordings",
                            "name": "Active Recordings",
                            "description": "List of in-progress and completed recording bundles",
                            "mimeType": "application/json"
                        },
                        {
                            "uri": "gsd-browser://current-refs",
                            "name": "Current Refs",
                            "description": "Fresh interactive snapshot with versioned element refs",
                            "mimeType": "application/json"
                        },
                        {
                            "uri": "gsd-browser://timeline",
                            "name": "Timeline",
                            "description": "Recent browser action timeline for the active session",
                            "mimeType": "application/json"
                        }
                    ]
                }
            })
        }

        "resources/read" => {
            let uri = request
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let session = request
                .get("params")
                .and_then(|p| p.get("session"))
                .and_then(|v| v.as_str());

            // Make key resources actually useful by performing real work where possible.
            if uri == "gsd-browser://latest-snapshot" {
                // Make it real: perform snapshot and return the actual structured data with refs
                let rt = tokio::runtime::Runtime::new().unwrap();
                let params = json!({ "limit": 30, "interactive_only": true });
                let resp = rt.block_on(crate::daemon_client::send_request(
                    "snapshot",
                    params,
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                ));
                let text = if let Ok(r) = resp {
                    if let Some(data) = r.result {
                        serde_json::to_string_pretty(&data).unwrap_or_default()
                    } else {
                        "Snapshot call succeeded but no data returned.".to_string()
                    }
                } else {
                    format!(
                        "Snapshot failed: {}",
                        resp.err().map(|e| e.to_string()).unwrap_or_default()
                    )
                };
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": text
                        }]
                    }
                })
            } else if uri == "gsd-browser://current-state" {
                // Rich current state via debug bundle (blocking)
                let rt = tokio::runtime::Runtime::new().unwrap();
                let resp = rt.block_on(crate::daemon_client::send_request(
                    "debug_bundle",
                    json!({}),
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                ));
                let text = if let Ok(r) = resp {
                    serde_json::to_string_pretty(&r.result.unwrap_or(json!({})))
                        .unwrap_or("Debug bundle unavailable".to_string())
                } else {
                    "Use browser_debug_bundle tool for full current state.".to_string()
                };
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": text
                        }]
                    }
                })
            } else if uri == "gsd-browser://current-refs" {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let resp = rt.block_on(crate::daemon_client::send_request(
                    "snapshot",
                    json!({"limit": 20}),
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                ));
                let text = if let Ok(r) = resp {
                    serde_json::to_string_pretty(&r.result.unwrap_or(json!({}))).unwrap_or_default()
                } else {
                    "Snapshot for refs unavailable".to_string()
                };
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": text
                        }]
                    }
                })
            } else if uri == "gsd-browser://active-recordings" {
                // Real data via blocking call (resources/read is sync)
                let rt = tokio::runtime::Runtime::new().unwrap();
                let resp = rt.block_on(crate::daemon_client::send_request(
                    "recordings",
                    json!({}),
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                ));
                let text = if let Ok(r) = resp {
                    if let Some(data) = r.result {
                        format!(
                            "Active recordings:\n{}",
                            serde_json::to_string_pretty(&data).unwrap_or_default()
                        )
                    } else {
                        "No active recordings.".to_string()
                    }
                } else {
                    "Use browser_recordings tool for live list.".to_string()
                };
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": text
                        }]
                    }
                })
            } else if uri == "gsd-browser://timeline" {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let resp = rt.block_on(crate::daemon_client::send_request(
                    "timeline",
                    json!({}),
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                ));
                let text = if let Ok(r) = resp {
                    serde_json::to_string_pretty(&r.result.unwrap_or(json!({}))).unwrap_or_default()
                } else {
                    "Timeline not available - use the timeline tool.".to_string()
                };
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": text
                        }]
                    }
                })
            } else {
                json!({
                    "jsonrpc": jsonrpc,
                    "id": id,
                    "result": {
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": "Unknown or not yet wired resource. Use tools for now."
                        }]
                    }
                })
            }
        }

        "prompts/list" => {
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "result": {
                    "prompts": [
                        {
                            "name": "robust_login_flow",
                            "description": "Perform a reliable login using vault or manual credentials, with proper waiting and assertion.",
                            "arguments": [
                                {"name": "url", "description": "Login page URL", "required": true},
                                {"name": "profile", "description": "Vault profile name (optional if using manual creds)", "required": false}
                            ]
                        },
                        {
                            "name": "full_page_audit",
                            "description": "Comprehensive audit of the current page: snapshot, console, network, accessibility, and visual state.",
                            "arguments": []
                        },
                        {
                            "name": "create_evidence_bundle",
                            "description": "Record a full user flow with annotations and produce a shareable evidence package.",
                            "arguments": [{"name": "name", "description": "Recording name", "required": true}]
                        },
                        {
                            "name": "autonomous_research_task",
                            "description": "High-level autonomous research: navigate, gather structured data, take screenshots, note findings, and produce an evidence bundle.",
                            "arguments": [
                                {"name": "start_url", "description": "Starting URL", "required": true},
                                {"name": "goal", "description": "What to research or accomplish", "required": true}
                            ]
                        },
                        {
                            "name": "evidence_creation_workflow",
                            "description": "Complete evidence workflow: start recording, perform actions with annotations, stop, export bundle, and summarize for audit.",
                            "arguments": [{"name": "name", "description": "Recording/evidence name", "required": true}]
                        },
                        {
                            "name": "debug_stuck_agent_flow",
                            "description": "Debugging workflow for when an agent is stuck: gather debug bundle, console, network, snapshot, timeline, and suggest next actions or human handoff.",
                            "arguments": []
                        }
                    ]
                }
            })
        }

        "prompts/get" => {
            let name = request
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Rich, executable multi-step prompts that agents can follow or use as templates.
            let messages = match name {
                "robust_login_flow" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "STEP 1: browser_navigate to the login URL.\nSTEP 2: If present, browser_act('accept_cookies').\nSTEP 3: Use browser_vault_login if a profile exists, otherwise browser_snapshot + targeted browser_fill_ref / browser_click_ref for username/password/submit.\nSTEP 4: browser_wait_for network_idle or url_contains dashboard.\nSTEP 5: browser_assert for logged-in indicators (user menu, dashboard URL, etc.).\nSTEP 6: Optionally browser_save_state for reuse.\nAlways re-snapshot after navigation and use refs for precision."}}),
                ],
                "full_page_audit" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "Parallel where possible:\n- browser_snapshot (mode: visible_only or interactive)\n- browser_console\n- browser_network\n- browser_debug_bundle\n- browser_visual_diff against a known baseline if one exists\n\nThen synthesize: security notes (via check_injection if relevant), performance observations, broken elements, and recommended next actions. Include refs and evidence links."}}),
                ],
                "create_evidence_bundle" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "1. browser_record_start with a clear name.\n2. Perform the exact reproduction steps using refs or act for precision.\n3. At key moments use browser_annotation_request to capture human or agent observations.\n4. browser_record_stop.\n5. browser_recording_export or use the bundle ID for validation/export.\nThis creates a high-fidelity, shareable reproduction package with annotations."}}),
                ],
                "autonomous_research_task" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "You are an autonomous researcher.\n1. Start at start_url.\n2. Use browser_snapshot + browser_act or refs to explore relevant sections.\n3. Use browser_extract for structured data where possible.\n4. Take key screenshots with browser_screenshot.\n5. If login needed, use vault or form tools.\n6. At the end: browser_debug_bundle + start a recording if the flow was long, add annotations for key findings.\n7. Summarize findings with refs and evidence links.\nGoal: {goal}"}}),
                ],
                "evidence_creation_workflow" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "1. browser_record_start with the provided name.\n2. Perform the target actions using refs/act for precision.\n3. Use browser_annotation_request at key decision points for observations.\n4. browser_record_stop.\n5. browser_recording_export (or use the ID).\n6. Add final annotations if needed and summarize the bundle for audit."}}),
                ],
                "debug_stuck_agent_flow" => vec![
                    json!({"role": "user", "content": {"type": "text", "text": "1. Call browser_debug_bundle immediately.\n2. browser_console + browser_network + browser_snapshot + browser_timeline (via tool or resource).\n3. Check for stale refs or console errors.\n4. If human input needed, use browser_view + browser_annotation_request or browser_takeover.\n5. Suggest concrete next tools or handoff to human with evidence links."}}),
                ],
                _ => vec![],
            };
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "result": {
                    "description": "Detailed executable agent workflow prompt with steps and best practices.",
                    "messages": messages
                }
            })
        }

        _ => {
            json!({
                "jsonrpc": jsonrpc,
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", method)
                }
            })
        }
    }
}

/// Define a powerful, comprehensive tool surface optimized for serious agentic workflows.
/// Descriptions emphasize why the tool is valuable for agents and include usage notes.
fn build_tool_list() -> Vec<Value> {
    vec![
        // === Core Navigation & State ===
        json!({
            "name": "browser_navigate",
            "description": "Navigate to a URL. This is the primary entry point for most workflows. Returns structured page state including title, URL, headings, and element counts. Always follow with browser_snapshot for interaction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full destination URL (must include protocol)" },
                    "session": { "type": "string", "description": "Named session for parallel/isolated browser instances" }
                },
                "required": ["url"]
            }
        }),
        // === Snapshot & Refs - This is a core differentiator ===
        json!({
            "name": "browser_snapshot",
            "description": "CRITICAL for reliable interaction. Captures interactive elements and assigns stable, versioned refs (e.g. @v1:e3). Always re-snapshot after navigation, form submission, or significant DOM changes. Refs become stale otherwise. Supports different modes for different tasks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": { "type": "string" },
                    "limit": { "type": "integer", "default": 40, "description": "Max elements to return (increase for complex pages)" },
                    "mode": { "type": "string", "description": "interactive (default), form, dialog, navigation, errors, headings, visible_only" },
                    "selector": { "type": "string", "description": "Optional scope (e.g. 'form' or '#checkout')" }
                }
            }
        }),
        json!({
            "name": "browser_get_ref",
            "description": "Get detailed metadata for a specific versioned ref (role, text, bounding box, selector hints, etc.). Useful for debugging or when an agent needs to understand an element before acting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "e.g. @v1:e3" },
                    "session": { "type": "string" }
                },
                "required": ["ref"]
            }
        }),
        // === Precise Interaction via Refs ===
        json!({
            "name": "browser_click_ref",
            "description": "Click using a versioned ref from a recent snapshot. Preferred over CSS selectors for reliability. Re-snapshot afterward if the page changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string" },
                    "session": { "type": "string" }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "browser_fill_ref",
            "description": "Fill an input/textarea using a versioned ref. Supports clear_first and submit options. More reliable than selector-based typing on dynamic pages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string" },
                    "text": { "type": "string" },
                    "session": { "type": "string" },
                    "clear_first": { "type": "boolean", "default": false },
                    "submit": { "type": "boolean", "default": false },
                    "slowly": { "type": "boolean", "default": false, "description": "Type character by character (sometimes needed for tricky inputs)" }
                },
                "required": ["ref", "text"]
            }
        }),
        json!({
            "name": "browser_hover_ref",
            "description": "Hover over an element by ref (useful for revealing menus, tooltips, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string" },
                    "session": { "type": "string" }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "browser_drag",
            "description": "Perform a real mouse drag with CDP mouse down/move/up. Use source+target selectors for element drags, or from_x/from_y/to_x/to_y coordinates for sliders, drawing, and canvas-style interactions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Optional CSS selector for drag start element" },
                    "target": { "type": "string", "description": "Optional CSS selector for drag end element" },
                    "from_x": { "type": "number", "description": "Start X coordinate in viewport CSS pixels" },
                    "from_y": { "type": "number", "description": "Start Y coordinate in viewport CSS pixels" },
                    "to_x": { "type": "number", "description": "End X coordinate in viewport CSS pixels" },
                    "to_y": { "type": "number", "description": "End Y coordinate in viewport CSS pixels" },
                    "steps": { "type": "integer", "default": 10, "description": "Interpolation steps between start and end" },
                    "button": { "type": "string", "default": "left", "description": "Mouse button to hold: left, middle, right, back, or forward" },
                    "session": { "type": "string" }
                }
            }
        }),
        // === Semantic / Intent-based (huge for agents) ===
        json!({
            "name": "browser_act",
            "description": "HIGH VALUE: Perform a semantic action without needing exact selectors or refs. Uses built-in intents (submit_form, accept_cookies, primary_cta, fill_email, next_step, dismiss, etc.). Excellent first approach before falling back to snapshot + refs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent": { "type": "string", "description": "submit_form, close_dialog, primary_cta, search_field, next_step, dismiss, auth_action, back_navigation, fill_email, fill_password, fill_username, accept_cookies, main_content, pagination_next, pagination_prev" },
                    "scope": { "type": "string", "description": "Optional CSS selector to narrow the search" },
                    "session": { "type": "string" }
                },
                "required": ["intent"]
            }
        }),
        json!({
            "name": "browser_find_best",
            "description": "Find the best matching element for a semantic intent without performing the action. Returns scored candidates. Use this when you want to inspect options before choosing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent": { "type": "string" },
                    "scope": { "type": "string" },
                    "session": { "type": "string" }
                },
                "required": ["intent"]
            }
        }),
        // === Forms (very common agent need) ===
        json!({
            "name": "browser_analyze_form",
            "description": "Analyze a form and return field labels, types, current values, and submit buttons. Extremely useful before filling complex forms.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "Optional form selector (defaults to first/main form)" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_fill_form",
            "description": "Fill multiple form fields at once using human-readable labels/names/placeholders. Supports submit after filling. Much more ergonomic than individual ref fills for login/registration flows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "values": { "type": "object", "description": "Map of field label/name/placeholder to value, e.g. {\"Email\": \"user@example.com\", \"Password\": \"secret\"}" },
                    "selector": { "type": "string", "description": "Optional form selector" },
                    "submit": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                },
                "required": ["values"]
            }
        }),
        json!({
            "name": "browser_select_option",
            "description": "Select an option in a native select, ARIA combobox, listbox, or menu-like dropdown. Matches by value, label, visible text, aria-label, or partial normalized text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the select, combobox/listbox, or dropdown trigger" },
                    "option": {
                        "description": "Option text/value to select. Arrays are supported for native multi-select elements.",
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "session": { "type": "string" }
                },
                "required": ["selector", "option"]
            }
        }),
        // === Assertions & Control Flow ===
        json!({
            "name": "browser_assert",
            "description": "Run explicit structured assertions. Prefer this over hoping the page looks right. Supports url_contains, text_visible, selector_visible, value_equals, no_console_errors, no_failed_requests, and many more.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "checks": { "type": "array", "description": "Array of assertion objects" },
                    "session": { "type": "string" }
                },
                "required": ["checks"]
            }
        }),
        json!({
            "name": "browser_wait_for",
            "description": "Wait for reliable conditions before proceeding (network_idle, selector_visible, text_visible, url_contains, element_count, region_stable, etc.). Essential for robust agent flows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "condition": { "type": "string" },
                    "value": { "type": "string" },
                    "timeout": { "type": "integer", "default": 10000 },
                    "threshold": { "type": "string", "description": "For element_count, e.g. '>=3'" },
                    "session": { "type": "string" }
                },
                "required": ["condition"]
            }
        }),
        // === Visual & Evidence ===
        json!({
            "name": "browser_screenshot",
            "description": "Capture screenshots (full page, element, or region). Returns base64 or can be written to disk. Use for visual verification or as evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "full_page": { "type": "boolean", "default": false },
                    "selector": { "type": "string" },
                    "quality": { "type": "integer", "default": 80 },
                    "format": { "type": "string", "default": "jpeg" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_visual_diff",
            "description": "Powerful visual regression tool. First run creates a baseline. Subsequent runs compare against it and report differences. Excellent for catching unintended UI changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Baseline name (e.g. 'homepage' or 'checkout-flow')" },
                    "selector": { "type": "string", "description": "Optional element scope" },
                    "threshold": { "type": "number", "default": 0.1, "description": "Difference tolerance 0-1" },
                    "update_baseline": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                },
                "required": ["name"]
            }
        }),
        // === Live Viewer & Human Collaboration (unique strength) ===
        json!({
            "name": "browser_view",
            "description": "Returns a local authenticated URL to the live viewer workbench. The human can watch, take control, annotate, or record. One of the most powerful features for agent + human collaboration.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": { "type": "string" },
                    "print_only": { "type": "boolean", "default": true }
                }
            }
        }),
        json!({
            "name": "browser_takeover",
            "description": "Temporarily give full control of the page to the human via the live viewer. Use when the agent needs human judgment or input.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_release_control",
            "description": "Return control from the human back to the agent.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        // === State, Auth & Persistence ===
        json!({
            "name": "browser_save_state",
            "description": "Save cookies, localStorage, and sessionStorage under a name for later reuse. Great for preserving login state across sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "default" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_restore_state",
            "description": "Restore a previously saved browser state (cookies + storage). Combine with navigate for fast logged-in sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "default" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_vault_login",
            "description": "Login using credentials previously saved in the encrypted auth vault. Requires GSD_BROWSER_VAULT_KEY to be set in the daemon's environment *at launch time* (not after). If the daemon is already running without it, you must stop and restart the daemon after exporting the variable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": { "type": "string", "description": "Name of the saved vault profile" },
                    "session": { "type": "string" }
                },
                "required": ["profile"]
            }
        }),
        // === Diagnostics & Observability ===
        json!({
            "name": "browser_debug_bundle",
            "description": "Capture a full diagnostic bundle: screenshot + console logs + network + timeline + accessibility tree. Extremely useful when an agent is stuck or needs to explain what happened.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_console",
            "description": "Get recent console messages (errors, warnings, logs). By default snapshots (preserves buffer); pass clear:true to drain after reading.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clear": { "type": "boolean", "default": false, "description": "If true, drain the buffer after reading" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_network",
            "description": "Get recent network requests/responses. Very useful for API debugging or understanding what data the page fetched. Defaults to snapshot (safe to call before har-export); pass clear:true to drain.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "default": "all", "description": "all, errors, or fetch-xhr" },
                    "clear": { "type": "boolean", "default": false, "description": "If true, drain the buffer after reading" },
                    "session": { "type": "string" }
                }
            }
        }),
        // === Advanced / Power User ===
        json!({
            "name": "browser_extract",
            "description": "Structured data extraction using CSS selectors + hints. Much more reliable than asking the model to parse HTML.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "schema": { "type": "object", "description": "JSON schema with _selector and _attribute hints" },
                    "selector": { "type": "string", "description": "Container selector for multiple items" },
                    "multiple": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                },
                "required": ["schema"]
            }
        }),
        json!({
            "name": "browser_check_injection",
            "description": "Scan the current page for potential prompt injection attacks (visible and hidden text). Important when browsing untrusted sites as an agent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_hidden": { "type": "boolean", "default": true },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_evaluate",
            "description": "Evaluate arbitrary JavaScript in the page context and return the result (safely handles non-serializable values like Window objects).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to evaluate" },
                    "session": { "type": "string" }
                },
                "required": ["expression"]
            }
        }),
        // === Recording & Evidence Bundles (major differentiator) ===
        json!({
            "name": "browser_record_start",
            "description": "Start a bounded recording bundle. Captures actions, frames, annotations, and metadata for reproducible evidence or bug reports. Pair with browser_annotation_request for human notes during the flow.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Human-readable name for the recording (e.g. 'checkout-bug-2026-05')" },
                    "session": { "type": "string" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "browser_record_stop",
            "description": "Stop the active recording and finalize the evidence bundle. Returns recording ID for later export/validation.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_recordings",
            "description": "List all recordings (active and completed) for the current session.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_recording_get",
            "description": "Get metadata and manifest for a specific recording.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "session": { "type": "string" }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "browser_recording_export",
            "description": "Export a completed recording bundle to disk (JSONL events + assets). Use for sharing reproducible evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "output": { "type": "string", "description": "Output path or directory" },
                    "session": { "type": "string" }
                },
                "required": ["id", "output"]
            }
        }),
        // === Annotations (human + agent collaboration) ===
        json!({
            "name": "browser_annotations",
            "description": "List all annotations created in the live viewer for this session. Great for capturing human intent during a flow.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_annotation_request",
            "description": "Ask the human (via the live viewer) to create an annotation with a specific note. Extremely powerful for gathering human insight during agent execution.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Prompt/instruction shown to the human annotator" },
                    "session": { "type": "string" }
                },
                "required": ["note"]
            }
        }),
        json!({
            "name": "browser_annotation_clear",
            "description": "Clear one or all annotations in the current session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Specific annotation ID or omit for all" },
                    "all": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                }
            }
        }),
        // === More Evidence & Diagnostics ===
        json!({
            "name": "browser_har_export",
            "description": "Export the network activity as a HAR 1.2 file for analysis or evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string", "description": "Output .har file path" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_trace_start",
            "description": "Start a CDP performance trace for deep performance or debugging evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Optional trace name" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_trace_stop",
            "description": "Stop the active trace and write it to disk.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Optional custom filename" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_emulate_device",
            "description": "Emulate a specific device (viewport, UA, touch, etc.). Useful for mobile/responsive testing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": { "type": "string", "description": "Device name (e.g. 'iPhone 15', 'Pixel 7') or 'list' to see options" },
                    "session": { "type": "string" }
                },
                "required": ["device"]
            }
        }),
        // === Viewer & Control Enhancements ===
        json!({
            "name": "browser_sensitive_on",
            "description": "Enable sensitive mode: human has local control while cloud/evidence surfaces use redaction policy.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_sensitive_off",
            "description": "Disable sensitive mode.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        // === Refs Self-Healing / Resilience (for agents) ===
        json!({
            "name": "browser_find_element",
            "description": "High-resilience element finder. Tries semantic intent first (act/find_best), falls back to text/role/selector. Returns candidates with scores and suggested refs or selectors. Use when exact refs may be stale.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent": { "type": "string", "description": "Optional semantic intent" },
                    "text": { "type": "string", "description": "Text to search for" },
                    "role": { "type": "string", "description": "ARIA role" },
                    "selector": { "type": "string" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_pause",
            "description": "Pause agent actions (used with the live viewer control flow).",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_resume",
            "description": "Resume agent actions.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        // === Network & Advanced Control (for completeness) ===
        json!({
            "name": "browser_mock_route",
            "description": "Mock a network route with custom response. Powerful for testing error states, slow responses, or specific API data without real backend.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL pattern (glob, e.g. **/api/users*)" },
                    "status": { "type": "integer", "default": 200 },
                    "body": { "type": "string" },
                    "content_type": { "type": "string" },
                    "delay": { "type": "integer", "description": "Delay in ms" },
                    "headers": { "type": "object" },
                    "session": { "type": "string" }
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "browser_block_urls",
            "description": "Block specific URL patterns (ads, analytics, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "patterns": { "type": "array", "items": {"type": "string"} },
                    "session": { "type": "string" }
                },
                "required": ["patterns"]
            }
        }),
        json!({
            "name": "browser_clear_routes",
            "description": "Clear all active route mocks and URL blocks.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_generate_test",
            "description": "Generate a Playwright test script from the current timeline/actions. Excellent for turning agent explorations into maintainable tests.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "recorded-session" },
                    "output": { "type": "string" },
                    "include_assertions": { "type": "boolean", "default": true },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_generate_replayable_test",
            "description": "MVP replayable test generator (PR4). Consumes a full recording bundle (enriched events + network slices from PR1-3) and emits a high-quality Playwright test with command sequence, URL checks, basic network assertions, and screenshot references. Makes evidence bundles first-class replayable test artifacts (Gerald Sterling). Supports recordingId or bundlePath. Prefer this for regression tests over timeline generate_test.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "replayable-session" },
                    "recordingId": { "type": "string", "description": "Recording ID from browser_recordings or record_stop" },
                    "bundlePath": { "type": "string", "description": "Filesystem path to exported bundle dir (with manifest.json + events.jsonl)" },
                    "output": { "type": "string", "description": "Output .spec.ts path (defaults co-located with bundle when possible)" },
                    "session": { "type": "string" }
                },
                "anyOf": [
                    { "required": ["recordingId"] },
                    { "required": ["bundlePath"] }
                ]
            }
        }),
        json!({
            "name": "browser_vault_save",
            "description": "Save credentials to the encrypted auth vault for future use with vault_login.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": { "type": "string" },
                    "url": { "type": "string" },
                    "username": { "type": "string" },
                    "password": { "type": "string" },
                    "extra_fields": { "type": "object" },
                    "session": { "type": "string" }
                },
                "required": ["profile", "url", "username", "password"]
            }
        }),
        json!({
            "name": "browser_vault_list",
            "description": "List saved vault profiles (no secrets shown).",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        // === Action Cache & Self-Healing Intents (for long-term agent resilience) ===
        json!({
            "name": "browser_action_cache",
            "description": "Manage the intent-to-selector cache (stats, get, put, clear). Helps agents build persistent self-healing knowledge across sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "stats | get | put | clear" },
                    "intent": { "type": "string" },
                    "selector": { "type": "string" },
                    "score": { "type": "number" },
                    "session": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        // === Additional Evidence & Polish ===
        json!({
            "name": "browser_save_pdf",
            "description": "Save the current page as PDF (great for archiving evidence or reports).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "format": { "type": "string", "default": "A4" },
                    "output": { "type": "string" },
                    "session": { "type": "string" }
                }
            }
        }),
        // === Batch, Diff & Multi-Page (for complex agent flows) ===
        json!({
            "name": "browser_batch",
            "description": "Execute multiple steps atomically (navigate, click, type, wait, assert, etc.). Reduces roundtrips and ensures atomicity for complex workflows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": { "type": "array", "description": "Array of step objects. Preferred: {action: 'navigate', url: '...'}. Also accepts legacy {tool: '...', params: {...}} (auto-normalized)." },
                    "stop_on_failure": { "type": "boolean", "default": true },
                    "summary_only": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                },
                "required": ["steps"]
            }
        }),
        json!({
            "name": "browser_diff",
            "description": "Compare current page state against a previous action ID or baseline (for change detection).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "since": { "type": "integer", "description": "Action ID from timeline" },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_list_pages",
            "description": "List all open browser tabs/pages with IDs.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_switch_page",
            "description": "Switch active tab by ID from list_pages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "session": { "type": "string" }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "browser_close_page",
            "description": "Close a tab by ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "session": { "type": "string" }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "browser_list_frames",
            "description": "List all frames (main + iframes) in the current page.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_select_frame",
            "description": "Select a frame by name, index, or URL pattern for subsequent operations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "index": { "type": "integer" },
                    "url_pattern": { "type": "string" },
                    "session": { "type": "string" }
                }
            }
        }),
        // === Viewer & Control Polish ===
        json!({
            "name": "browser_goal",
            "description": "Set or clear a goal banner in the live viewer (helps human collaborators understand agent intent).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "clear": { "type": "boolean", "default": false },
                    "session": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "browser_step",
            "description": "Allow exactly one agent action through, then pause again (step-through mode for humans).",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_abort",
            "description": "Abort the next gated/pending action (used with viewer risk approvals).",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
        json!({
            "name": "browser_control_state",
            "description": "Get current shared control state (owner, mode, version) for the session.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string" } }
            }
        }),
    ]
}

/// Blocking wrapper used by the stdio loop (creates a small runtime per call for the prototype).
fn handle_tool_call_blocking(name: &str, arguments: Value, cli: &Cli) -> Result<String, String> {
    // For the MVP we create a tiny runtime per tool call. This is simple and
    // avoids complex Send bounds. In a later iteration we can share a runtime.
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(handle_tool_call(name, arguments, cli))
}

/// Execute a tool call by mapping it to the existing daemon command surface.
async fn handle_tool_call(name: &str, arguments: Value, cli: &Cli) -> Result<String, String> {
    // Build CLI-equivalent flags from the MCP arguments.
    // We reuse the exact same daemon_client::send_request path that the real CLI uses.
    // This gives us auto-start, session handling, JSON output, error formatting, etc. for free.

    let session = arguments.get("session").and_then(|v| v.as_str());

    let result = match name {
        "browser_navigate" => {
            let url = arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("url is required")?;

            let resp = crate::daemon_client::send_request(
                "navigate",
                json!({ "url": url }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_snapshot" => {
            let mut params = json!({
                "interactive_only": true,
                "limit": arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(40)
            });
            if let Some(mode) = arguments.get("mode").and_then(|v| v.as_str()) {
                params["mode"] = json!(mode);
            }

            let resp = crate::daemon_client::send_request(
                "snapshot",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_click_ref" => {
            let r#ref = arguments
                .get("ref")
                .and_then(|v| v.as_str())
                .ok_or("ref is required")?;

            let resp = crate::daemon_client::send_request(
                "click_ref",
                json!({ "ref": r#ref }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Clicked successfully".to_string()
        }

        "browser_drag" => {
            let mut params = json!({});
            for key in [
                "source", "target", "from_x", "from_y", "to_x", "to_y", "steps", "button",
            ] {
                if let Some(value) = arguments.get(key) {
                    params[key] = value.clone();
                }
            }
            let resp = crate::daemon_client::send_request(
                "drag",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_fill_ref" => {
            let r#ref = arguments
                .get("ref")
                .and_then(|v| v.as_str())
                .ok_or("ref is required")?;
            let text = arguments
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or("text is required")?;

            let mut params = json!({ "ref": r#ref, "text": text });
            if arguments
                .get("clear_first")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                params["clear_first"] = json!(true);
            }
            if arguments
                .get("submit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                params["submit"] = json!(true);
            }

            let resp = crate::daemon_client::send_request(
                "fill_ref",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Filled successfully".to_string()
        }

        "browser_select_option" => {
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or("selector is required")?;
            let option = arguments.get("option").ok_or("option is required")?.clone();
            let option_is_string_array = option
                .as_array()
                .map(|items| !items.is_empty() && items.iter().all(|item| item.is_string()))
                .unwrap_or(false);
            if !(option.is_string() || option_is_string_array) {
                return Err("option must be a string or array of strings".to_string());
            }

            let resp = crate::daemon_client::send_request(
                "select_option",
                json!({ "selector": selector, "option": option }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_wait_for" => {
            let condition = arguments
                .get("condition")
                .and_then(|v| v.as_str())
                .ok_or("condition is required")?;
            let mut params = json!({ "condition": condition });

            if let Some(value) = arguments.get("value").and_then(|v| v.as_str()) {
                params["value"] = json!(value);
            }
            if let Some(timeout) = arguments.get("timeout").and_then(|v| v.as_u64()) {
                params["timeout"] = json!(timeout);
            }

            let resp = crate::daemon_client::send_request(
                "wait_for",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Condition satisfied".to_string()
        }

        "browser_assert" => {
            let checks = arguments
                .get("checks")
                .ok_or("checks array is required")?
                .clone();

            let resp = crate::daemon_client::send_request(
                "assert",
                json!({ "checks": checks }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_screenshot" => {
            let mut params = json!({
                "full_page": arguments.get("full_page").and_then(|v| v.as_bool()).unwrap_or(false)
            });
            if let Some(sel) = arguments.get("selector").and_then(|v| v.as_str()) {
                params["selector"] = json!(sel);
            }

            let resp = crate::daemon_client::send_request(
                "screenshot",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Screenshot captured (base64 available in JSON mode)".to_string()
        }

        "browser_act" => {
            let intent = arguments
                .get("intent")
                .and_then(|v| v.as_str())
                .ok_or("intent is required")?;
            let mut params = json!({ "intent": intent });
            if let Some(scope) = arguments.get("scope").and_then(|v| v.as_str()) {
                params["scope"] = json!(scope);
            }

            let resp = crate::daemon_client::send_request(
                "act",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_view" => {
            let print_only = arguments
                .get("print_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let params = json!({ "print_only": print_only });

            let resp = crate::daemon_client::send_request(
                "view",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_get_ref" => {
            let r#ref = arguments
                .get("ref")
                .and_then(|v| v.as_str())
                .ok_or("ref is required")?;
            let resp = crate::daemon_client::send_request(
                "get_ref",
                json!({ "ref": r#ref }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_hover_ref" => {
            let r#ref = arguments
                .get("ref")
                .and_then(|v| v.as_str())
                .ok_or("ref is required")?;
            let resp = crate::daemon_client::send_request(
                "hover_ref",
                json!({ "ref": r#ref }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Hovered successfully".to_string()
        }

        "browser_find_best" => {
            let intent = arguments
                .get("intent")
                .and_then(|v| v.as_str())
                .ok_or("intent is required")?;
            let mut params = json!({ "intent": intent });
            if let Some(scope) = arguments.get("scope").and_then(|v| v.as_str()) {
                params["scope"] = json!(scope);
            }
            let resp = crate::daemon_client::send_request(
                "find_best",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_analyze_form" => {
            let mut params = json!({});
            if let Some(sel) = arguments.get("selector").and_then(|v| v.as_str()) {
                params["selector"] = json!(sel);
            }
            let resp = crate::daemon_client::send_request(
                "analyze_form",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_fill_form" => {
            let values = arguments
                .get("values")
                .ok_or("values object is required")?
                .clone();
            let mut params = json!({ "values": values, "submit": arguments.get("submit").and_then(|v| v.as_bool()).unwrap_or(false) });
            if let Some(sel) = arguments.get("selector").and_then(|v| v.as_str()) {
                params["selector"] = json!(sel);
            }
            let resp = crate::daemon_client::send_request(
                "fill_form",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_save_state" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let resp = crate::daemon_client::send_request(
                "save_state",
                json!({ "name": name }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            format!("State '{}' saved", name)
        }

        "browser_restore_state" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let resp = crate::daemon_client::send_request(
                "restore_state",
                json!({ "name": name }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            format!("State '{}' restored", name)
        }

        "browser_vault_login" => {
            let profile = arguments
                .get("profile")
                .and_then(|v| v.as_str())
                .ok_or("profile is required")?;
            let resp = crate::daemon_client::send_request(
                "vault_login",
                json!({ "profile": profile }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            format!("Vault login with profile '{}' completed", profile)
        }

        "browser_debug_bundle" => {
            let mut params = json!({});
            if let Some(name) = arguments.get("name").and_then(|v| v.as_str()) {
                params["name"] = json!(name);
            }
            let resp = crate::daemon_client::send_request(
                "debug_bundle",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_console" => {
            let clear = arguments
                .get("clear")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let resp = crate::daemon_client::send_request(
                "console",
                json!({ "clear": clear }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_network" => {
            let filter = arguments
                .get("filter")
                .and_then(|v| v.as_str())
                .unwrap_or("all");
            let clear = arguments
                .get("clear")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let resp = crate::daemon_client::send_request(
                "network",
                json!({ "filter": filter, "clear": clear }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_extract" => {
            let schema = arguments.get("schema").ok_or("schema is required")?.clone();
            let mut params = json!({ "schema": schema });
            if let Some(sel) = arguments.get("selector").and_then(|v| v.as_str()) {
                params["selector"] = json!(sel);
            }
            if arguments
                .get("multiple")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                params["multiple"] = json!(true);
            }
            let resp = crate::daemon_client::send_request(
                "extract",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_check_injection" => {
            let include_hidden = arguments
                .get("include_hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let resp = crate::daemon_client::send_request(
                "check_injection",
                json!({ "include_hidden": include_hidden }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_evaluate" => {
            let expression = arguments
                .get("expression")
                .and_then(|v| v.as_str())
                .ok_or("expression is required")?;
            let resp = crate::daemon_client::send_request(
                "eval",
                json!({ "expression": expression }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        // Recording tools
        "browser_record_start" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?;
            let resp = crate::daemon_client::send_request(
                "record_start",
                json!({ "name": name }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_record_stop" => {
            let resp = crate::daemon_client::send_request(
                "record_stop",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_recordings" => {
            let resp = crate::daemon_client::send_request(
                "recordings",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_recording_get" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let resp = crate::daemon_client::send_request(
                "recording_get",
                json!({ "id": id }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_recording_export" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let output = arguments
                .get("output")
                .and_then(|v| v.as_str())
                .ok_or("output path is required")?;
            let resp = crate::daemon_client::send_request(
                "recording_export",
                json!({ "id": id, "output": output }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        // Annotation tools
        "browser_annotations" => {
            let resp = crate::daemon_client::send_request(
                "annotations",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_annotation_request" => {
            let note = arguments
                .get("note")
                .and_then(|v| v.as_str())
                .ok_or("note is required")?;
            let resp = crate::daemon_client::send_request(
                "annotation_request",
                json!({ "note": note }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_mock_route" => {
            let url = arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("url pattern is required")?;
            let mut params = json!({ "url": url });
            if let Some(s) = arguments.get("status") {
                params["status"] = s.clone();
            }
            if let Some(b) = arguments.get("body") {
                params["body"] = b.clone();
            }
            if let Some(ct) = arguments.get("content_type") {
                params["content_type"] = ct.clone();
            }
            if let Some(d) = arguments.get("delay") {
                params["delay"] = d.clone();
            }
            if let Some(h) = arguments.get("headers") {
                params["headers"] = h.clone();
            }
            let resp = crate::daemon_client::send_request(
                "mock_route",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_block_urls" => {
            let patterns = arguments
                .get("patterns")
                .ok_or("patterns array required")?
                .clone();
            let resp = crate::daemon_client::send_request(
                "block_urls",
                json!({ "patterns": patterns }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "URLs blocked".to_string()
        }
        "browser_clear_routes" => {
            let resp = crate::daemon_client::send_request(
                "clear_routes",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "All routes cleared".to_string()
        }
        "browser_generate_test" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("recorded-session");
            let mut params = json!({ "name": name, "include_assertions": arguments.get("include_assertions").and_then(|v| v.as_bool()).unwrap_or(true) });
            if let Some(o) = arguments
                .get("output")
                .or_else(|| arguments.get("outputPath"))
            {
                params["output"] = o.clone();
                params["outputPath"] = o.clone(); // normalize for daemon handler (which reads outputPath)
            }
            let resp = crate::daemon_client::send_request(
                "generate_test",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_generate_replayable_test" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("replayable-session");
            let mut params = json!({ "name": name });
            if let Some(id) = arguments
                .get("recordingId")
                .or_else(|| arguments.get("recording_id"))
            {
                params["recordingId"] = id.clone();
            }
            if let Some(bp) = arguments
                .get("bundlePath")
                .or_else(|| arguments.get("bundle_path"))
            {
                params["bundlePath"] = bp.clone();
            }
            if let Some(o) = arguments
                .get("output")
                .or_else(|| arguments.get("outputPath"))
            {
                params["output"] = o.clone();
                params["outputPath"] = o.clone(); // normalize for daemon handler
            }
            let resp = crate::daemon_client::send_request(
                "generate_replayable_test",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_vault_save" => {
            let profile = arguments
                .get("profile")
                .and_then(|v| v.as_str())
                .ok_or("profile required")?;
            let url = arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("url required")?;
            let username = arguments
                .get("username")
                .and_then(|v| v.as_str())
                .ok_or("username required")?;
            let password = arguments
                .get("password")
                .and_then(|v| v.as_str())
                .ok_or("password required")?;
            let mut params = json!({ "profile": profile, "url": url, "username": username, "password": password });
            if let Some(extra) = arguments.get("extra_fields") {
                params["extra_fields"] = extra.clone();
            }
            let resp = crate::daemon_client::send_request(
                "vault_save",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            format!("Credentials saved to vault profile '{}'", profile)
        }
        "browser_vault_list" => {
            let resp = crate::daemon_client::send_request(
                "vault_list",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_action_cache" => {
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("stats");
            let mut params = json!({ "action": action });
            if let Some(i) = arguments.get("intent") {
                params["intent"] = i.clone();
            }
            if let Some(s) = arguments.get("selector") {
                params["selector"] = s.clone();
            }
            if let Some(sc) = arguments.get("score") {
                params["score"] = sc.clone();
            }
            let resp = crate::daemon_client::send_request(
                "action_cache",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_save_pdf" => {
            let mut params = json!({});
            if let Some(f) = arguments.get("format") {
                params["format"] = f.clone();
            }
            if let Some(o) = arguments.get("output") {
                params["output"] = o.clone();
            }
            let resp = crate::daemon_client::send_request(
                "save_pdf",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_batch" => {
            let mut steps = arguments
                .get("steps")
                .ok_or("steps array required")?
                .clone();
            // Normalize legacy MCP-style steps {tool, params} -> daemon format {action, ...inlined}
            if let Some(arr) = steps.as_array_mut() {
                for step in arr.iter_mut() {
                    if let Some(obj) = step.as_object_mut() {
                        if let Some(tool) = obj
                            .remove("tool")
                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                        {
                            obj.insert("action".to_string(), json!(tool));
                        }
                        if let Some(params) = obj.remove("params") {
                            if let Some(pobj) = params.as_object() {
                                for (k, v) in pobj {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                }
            }
            let params = json!({ "steps": steps, "stop_on_failure": arguments.get("stop_on_failure").and_then(|v| v.as_bool()).unwrap_or(true), "summary_only": arguments.get("summary_only").and_then(|v| v.as_bool()).unwrap_or(false) });
            let resp = crate::daemon_client::send_request(
                "batch",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_diff" => {
            let mut params = json!({});
            if let Some(s) = arguments.get("since") {
                params["since"] = s.clone();
            }
            let resp = crate::daemon_client::send_request(
                "diff",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_list_pages" => {
            let resp = crate::daemon_client::send_request(
                "list_pages",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_switch_page" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or("id required")?;
            let resp = crate::daemon_client::send_request(
                "switch_page",
                json!({ "id": id }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Switched page".to_string()
        }
        "browser_close_page" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or("id required")?;
            let resp = crate::daemon_client::send_request(
                "close_page",
                json!({ "id": id }),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Page closed".to_string()
        }
        "browser_list_frames" => {
            let resp = crate::daemon_client::send_request(
                "list_frames",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }
        "browser_select_frame" => {
            let mut params = json!({});
            if let Some(n) = arguments.get("name") {
                params["name"] = n.clone();
            }
            if let Some(i) = arguments.get("index") {
                params["index"] = i.clone();
            }
            if let Some(u) = arguments.get("url_pattern") {
                params["urlPattern"] = u.clone();
            }
            let resp = crate::daemon_client::send_request(
                "select_frame",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Frame selected".to_string()
        }
        "browser_goal" => {
            let mut params = json!({});
            if let Some(t) = arguments.get("text") {
                params["text"] = t.clone();
            }
            if let Some(c) = arguments.get("clear") {
                params["clear"] = c.clone();
            }
            let resp = crate::daemon_client::send_request(
                "goal",
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Goal updated".to_string()
        }
        "browser_step" => {
            let resp = crate::daemon_client::send_request(
                "step",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Step allowed".to_string()
        }
        "browser_abort" => {
            let resp = crate::daemon_client::send_request(
                "abort",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            "Action aborted".to_string()
        }
        "browser_control_state" => {
            let resp = crate::daemon_client::send_request(
                "control_state",
                json!({}),
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_takeover"
        | "browser_release_control"
        | "browser_pause"
        | "browser_resume"
        | "browser_sensitive_on"
        | "browser_sensitive_off"
        | "browser_annotation_clear"
        | "browser_har_export"
        | "browser_trace_start"
        | "browser_trace_stop"
        | "browser_emulate_device"
        | "browser_visual_diff" => {
            let method = match name {
                "browser_takeover" => "takeover",
                "browser_release_control" => "release_control",
                "browser_pause" => "pause",
                "browser_resume" => "resume",
                "browser_sensitive_on" => "sensitive_on",
                "browser_sensitive_off" => "sensitive_off",
                "browser_annotation_clear" => "annotation_clear",
                "browser_har_export" => "har_export",
                "browser_trace_start" => "trace_start",
                "browser_trace_stop" => "trace_stop",
                "browser_emulate_device" => "emulate_device",
                "browser_visual_diff" => "visual_diff",
                _ => unreachable!(),
            };
            let mut params = arguments.clone();
            if let Some(obj) = params.as_object_mut() {
                obj.remove("session");
            }
            let resp = crate::daemon_client::send_request(
                method,
                params,
                cli.browser_path.as_deref(),
                cli.cdp_url.as_deref(),
                session,
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(err) = resp.error {
                return Err(err.message);
            }
            serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
        }

        "browser_find_element" => {
            let mut params = json!({});
            if let Some(intent) = arguments.get("intent").and_then(|v| v.as_str()) {
                params["intent"] = json!(intent);
                if let Some(selector) = arguments.get("selector").and_then(|v| v.as_str()) {
                    params["scope"] = json!(selector);
                }
                let resp = crate::daemon_client::send_request(
                    "find_best",
                    params,
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                )
                .await
                .map_err(|e| e.to_string())?;
                if let Some(err) = resp.error {
                    return Err(err.message);
                }
                serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
            } else {
                if let Some(text) = arguments.get("text").and_then(|v| v.as_str()) {
                    params["text"] = json!(text);
                }
                if let Some(role) = arguments.get("role").and_then(|v| v.as_str()) {
                    params["role"] = json!(role);
                }
                if let Some(selector) = arguments.get("selector").and_then(|v| v.as_str()) {
                    params["selector"] = json!(selector);
                }
                let resp = crate::daemon_client::send_request(
                    "find",
                    params,
                    cli.browser_path.as_deref(),
                    cli.cdp_url.as_deref(),
                    session,
                )
                .await
                .map_err(|e| e.to_string())?;
                if let Some(err) = resp.error {
                    return Err(err.message);
                }
                serde_json::to_string_pretty(&resp.result.unwrap_or(json!({}))).unwrap()
            }
        }

        other => {
            return Err(format!(
                "Tool '{}' is not yet wired in the MCP server. It exists in gsd-browser — we can add it quickly. Currently available: navigate, snapshot, click_ref, fill_ref, act, wait_for, assert, screenshot, view, batch, and many more.",
                other
            ));
        }
    };

    Ok(result)
}
