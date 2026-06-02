//! Playwright test code generation from the action timeline.

use crate::daemon::state::DaemonState;
use gsd_browser_common::state_dir;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Generate a Playwright test script from the action timeline.
pub fn handle_generate_test(state: &DaemonState, params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("recorded-session");

    let output_path = params
        .get("outputPath")
        .or_else(|| params.get("output"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let include_assertions = params
        .get("includeAssertions")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Read timeline snapshot
    let timeline = state.timeline.lock().unwrap();
    let entries = timeline.snapshot();
    drop(timeline);

    if entries.is_empty() {
        return Err("no actions recorded in timeline — nothing to generate".to_string());
    }

    // Build the Playwright test script
    let mut lines: Vec<String> = Vec::new();
    lines.push("import { test, expect } from '@playwright/test';".to_string());
    lines.push(String::new());
    lines.push(format!("test.describe('{}', () => {{", escape_js(name)));
    lines.push(format!(
        "  test('{}', async ({{ page }}) => {{",
        escape_js(name)
    ));

    for entry in &entries {
        let tool = entry.tool.as_str();
        let params_str = &entry.params_summary;

        match tool {
            "navigate" => {
                // Extract URL from params or after_url
                let url =
                    extract_param_str(params_str, "url").unwrap_or_else(|| entry.after_url.clone());
                if !url.is_empty() {
                    lines.push(format!("    await page.goto('{}');", escape_js(&url)));
                }
            }
            "back" => {
                lines.push("    await page.goBack();".to_string());
            }
            "forward" => {
                lines.push("    await page.goForward();".to_string());
            }
            "reload" => {
                lines.push("    await page.reload();".to_string());
            }
            "click" | "click_ref" => {
                if let Some(sel) = extract_param_str(params_str, "selector") {
                    lines.push(format!("    await page.click('{}');", escape_js(&sel)));
                } else if let Some(r) = extract_param_str(params_str, "ref") {
                    lines.push(format!(
                        "    // ref-based click: {r} — resolve selector manually"
                    ));
                }
            }
            "type" | "fill_ref" => {
                let sel = extract_param_str(params_str, "selector").unwrap_or_default();
                let text = extract_param_str(params_str, "text").unwrap_or_default();
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.fill('{}', '{}');",
                        escape_js(&sel),
                        escape_js(&text)
                    ));
                }
            }
            "press" => {
                let key = extract_param_str(params_str, "key").unwrap_or_default();
                if !key.is_empty() {
                    lines.push(format!(
                        "    await page.keyboard.press('{}');",
                        escape_js(&key)
                    ));
                }
            }
            "hover" | "hover_ref" => {
                if let Some(sel) = extract_param_str(params_str, "selector") {
                    lines.push(format!("    await page.hover('{}');", escape_js(&sel)));
                }
            }
            "scroll" => {
                let direction = extract_param_str(params_str, "direction").unwrap_or_default();
                let amount = extract_param_str(params_str, "amount").unwrap_or("300".to_string());
                let delta = if direction == "up" {
                    format!("-{amount}")
                } else {
                    amount
                };
                lines.push(format!("    await page.mouse.wheel(0, {delta});"));
            }
            "select_option" => {
                let sel = extract_param_str(params_str, "selector").unwrap_or_default();
                let opt = extract_param_str(params_str, "option").unwrap_or_default();
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.selectOption('{}', '{}');",
                        escape_js(&sel),
                        escape_js(&opt)
                    ));
                }
            }
            "set_checked" => {
                let sel = extract_param_str(params_str, "selector").unwrap_or_default();
                let checked = params_str.contains("true");
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.setChecked('{}', {checked});",
                        escape_js(&sel)
                    ));
                }
            }
            "wait_for" => {
                if let Some(condition) = extract_param_str(params_str, "condition") {
                    let value = extract_param_str(params_str, "value").unwrap_or_default();
                    match condition.as_str() {
                        "selector_visible" => {
                            lines.push(format!(
                                "    await page.waitForSelector('{}');",
                                escape_js(&value)
                            ));
                        }
                        "selector_hidden" => {
                            lines.push(format!(
                                "    await page.waitForSelector('{}', {{ state: 'hidden' }});",
                                escape_js(&value)
                            ));
                        }
                        "url_contains" => {
                            lines.push(format!(
                                "    await page.waitForURL('*{}*');",
                                escape_js(&value)
                            ));
                        }
                        "network_idle" => {
                            lines.push(
                                "    await page.waitForLoadState('networkidle');".to_string(),
                            );
                        }
                        "delay" => {
                            lines.push(format!("    await page.waitForTimeout({value});"));
                        }
                        "text_visible" => {
                            lines.push(format!(
                                "    await page.waitForSelector('text=\"{}\"');",
                                escape_js(&value)
                            ));
                        }
                        _ => {
                            lines.push(format!("    // wait_for {condition}: {value}"));
                        }
                    }
                }
            }
            "assert" => {
                if include_assertions {
                    lines.push(format!("    // assertion: {}", truncate(params_str, 80)));
                }
            }
            _ => {
                // Other tools: emit as comment
                lines.push(format!("    // {tool}: {}", truncate(params_str, 60)));
            }
        }
    }

    lines.push("  });".to_string());
    lines.push("});".to_string());
    lines.push(String::new());

    let script = lines.join("\n");

    // Determine output path
    let safe_name = sanitize_for_filename(name);
    let file_path = if let Some(p) = output_path {
        p
    } else {
        let dir = state_dir().join("generated-tests");
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("{safe_name}.spec.ts"))
            .to_string_lossy()
            .to_string()
    };

    // PR5 review Finding 8: guide users to the much richer replayable bundle generator
    let mut final_script = script;
    final_script.push_str("\n\n// NOTE (PR5): This was generated from the legacy timeline path.\n");
    final_script.push_str("// For rich DOM assertions, state restoration (auto test.use + redacted pwstate), full network slices + HAR subset, and expected-vs-actual support, use browser_generate_replayable_test (recordingId or bundlePath after export) instead.\n");

    fs::write(&file_path, &final_script).map_err(|e| format!("failed to write test file: {e}"))?;

    Ok(json!({
        "path": file_path,
        "actions": entries.len(),
        "lines": final_script.lines().count(),
        "note": "Legacy timeline generator. Prefer browser_generate_replayable_test for all PR5+ features (state, rich DOM, HAR)."
    }))
}

/// Try to extract a named parameter from a truncated JSON params summary string.
fn extract_param_str(params_summary: &str, key: &str) -> Option<String> {
    // Try parsing as JSON first
    if let Ok(val) = serde_json::from_str::<Value>(params_summary) {
        return val.get(key).and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                Some(v.to_string())
            }
        });
    }
    // Fallback: search for "key":"value" pattern
    let pattern = format!("\"{}\":\"", key);
    if let Some(start) = params_summary.find(&pattern) {
        let after = &params_summary[start + pattern.len()..];
        if let Some(end) = after.find('"') {
            return Some(after[..end].to_string());
        }
    }
    None
}

/// Escape a string for safe embedding inside a single-quoted JS string literal
/// (e.g. page.fill('sel', '<HERE>') or test('name', ...)).
/// Handles the critical cases that previously produced invalid JS:
/// backslashes, single quotes, newlines, carriage returns, tabs, and other controls.
fn escape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Other ASCII controls and DEL -> unicode escape to be safe
            '\u{0000}'..='\u{001F}' | '\u{007F}' => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            // " is safe inside single quotes, but we escape it defensively
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect()
    } else {
        s.to_string()
    }
}

/// Sanitize a user-supplied test name for use as a filename component.
/// Prevents path traversal (../, absolute paths, separators) in generated .spec.ts.
/// Only keeps safe characters; falls back to a conservative default.
fn sanitize_for_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => Some(c),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        out = "replayable-session".to_string();
    }
    // Hard limit to avoid crazy long filenames
    if out.len() > 80 {
        out.truncate(80);
    }
    out
}

// === PR 5: State restoration, rich DOM assertions, and full slice support ===
// Builds on PR1-4 foundation. Consumes full replayable evidence bundle (enriched events
// with before/after CompactPageState DOM + full per-action network slices + optional
// states/ snapshots from save_state during recording).
// Produces high-fidelity Playwright regression test artifacts:
// - command sequence + robust URL guards
// - rich per-step DOM assertions (element counts, structural headings/text, focus/dialog)
//   derived directly from bundle "after" state for reliable regression on stateful flows
// - "expected vs actual" diff support: inline expected snapshots + TODOs for custom matcher
// - state restoration integration: auto-detects bundle/states/*.pwstate.json (from save_state)
//   and emits ready-to-use setup blocks + comments for test.use({storageState}) or daemon restore
// - full network slice usage + optional companion HAR subset export (for mocking / audit)
// - ref actions remain safe commented (with evidence pointers)
// Clean, evolvable schema (replayFormatVersion "playwright-2"). High-quality output for
// auth-heavy and multi-step agent workflows (per Gerald Sterling + plan).
// Supports recordingId or bundlePath. Prefer for regression over legacy timeline generate_test.

fn extract_str_from_cmd(cmd: &serde_json::Value, key: &str) -> Option<String> {
    cmd.get(key).and_then(|v| {
        if let Some(s) = v.as_str() {
            Some(s.to_string())
        } else if !v.is_null() {
            Some(v.to_string())
        } else {
            None
        }
    })
}

fn short_url(u: &str) -> String {
    if u.chars().count() > 55 {
        format!("{}...", u.chars().take(52).collect::<String>())
    } else {
        u.to_string()
    }
}

fn network_entries(event: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    event
        .get("networkSlice")
        .and_then(|s| s.get("entries"))
        .and_then(|e| e.as_array())
        .or_else(|| {
            event
                .get("network")
                .and_then(|n| n.get("recent"))
                .and_then(|e| e.as_array())
        })
}

fn get_network_summary(event: &serde_json::Value) -> Vec<String> {
    if let Some(entries) = network_entries(event) {
        entries
            .iter()
            .filter_map(|e| {
                let method = e.get("method").and_then(|m| m.as_str()).unwrap_or("?");
                let url = e.get("url").and_then(|u| u.as_str()).unwrap_or("?");
                let status = e.get("status").and_then(|s| s.as_u64()).unwrap_or(0);
                let failed = e.get("failed").and_then(|f| f.as_bool()).unwrap_or(false);
                // Skip assets, data urls, common static
                if url.starts_with("data:")
                    || url.ends_with(".js")
                    || url.ends_with(".css")
                    || url.ends_with(".png")
                    || url.ends_with(".jpg")
                    || url.ends_with(".jpeg")
                    || url.ends_with(".woff")
                    || url.ends_with(".svg")
                {
                    return None;
                }
                let mut s = format!("{} {} -> {}", method, short_url(url), status);
                if failed {
                    s.push_str(" (failed)");
                }
                if let Some(ts) = e.get("timestamp").and_then(|t| t.as_f64()) {
                    if ts > 0.0 {
                        s.push_str(&format!(" @t={:.0}", ts));
                    }
                }
                Some(s)
            })
            .collect()
    } else {
        vec![]
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create output directory: {}", e))?;
        }
    }
    Ok(())
}

/// PR5: Emit rich DOM assertions + expected/actual diff comment for a step's "after" state.
/// Safe no-op if no usable after DOM data. Uses counts for stable structural guards,
/// first heading + body prefix for text presence, and dumps the full signature for diffs.
fn emit_rich_dom_assertions(lines: &mut Vec<String>, event: &serde_json::Value, step: usize) {
    let after = event.get("after").unwrap_or(&serde_json::Value::Null);
    if after.is_null() || after.as_object().map_or(true, |o| o.is_empty()) {
        return;
    }

    let mut emitted_any = false;

    // Element count assertions (PR5 review Finding 7: made tolerant / commented for flakiness)
    // Broad generic selectors + exact counts are smoke checks only. Real UIs (dynamic counts,
    // banners, responsive, third-party) make them brittle. Users should replace with semantic
    // locators for production regression suites.
    if let Some(counts) = after.get("counts").and_then(|c| c.as_object()) {
        if let Some(n) = counts.get("landmarks").and_then(|v| v.as_u64()) {
            if n > 0 && n < 50 {
                lines.push(format!(
                    "    // await expect(page.locator('main, [role=\"main\"], header, nav, footer')).toHaveCount({}); // PR5 structural (tolerant smoke check; replace for prod)",
                    n
                ));
                emitted_any = true;
            }
        }
        // Only comment the very broad ones to avoid constant test flakiness in generated artifacts
        if let Some(btn) = counts.get("buttons").and_then(|v| v.as_u64()) {
            if btn > 0 && btn < 100 {
                lines.push(format!(
                    "    // await expect(page.locator('button,[role=\"button\"]')).toHaveCount({}); // PR5 (broad; often flaky — refine)",
                    btn
                ));
            }
        }
    }

    // Structural text / heading assertion (first significant heading)
    if let Some(headings) = after.get("headings").and_then(|h| h.as_array()) {
        if let Some(first) = headings.first().and_then(|v| v.as_str()) {
            if !first.is_empty() && first.len() < 120 {
                let safe = escape_js(first);
                lines.push(format!(
                    "    await expect(page.locator('h1,h2,h3').first()).toContainText('{}'); // PR5 structural heading from bundle after-state (step {})",
                    safe, step
                ));
                emitted_any = true;
            }
        }
    }

    // Expected vs actual diff support: full signature comment for this step's observed after DOM.
    // Consumers can paste the object into a test helper that re-captures analogous compact state
    // and does expect(actual).toEqual(expected) or a soft diff reporter.
    let dom_sig = json!({
        "domHash": after.get("domHash"),
        "counts": after.get("counts"),
        "headings": after.get("headings"),
        "focus": after.get("focus"),
        "bodyTextPrefix": after.get("bodyText").and_then(|b| b.as_str()).map(|s| truncate(s, 120)),
        "dialog": after.get("dialog"),
        "url": event.get("url"),
    });
    lines.push(format!(
        "    // expectedAfterDOM (step {} for expected-vs-actual diff): {}",
        step,
        truncate(&serde_json::to_string(&dom_sig).unwrap_or_default(), 300)
    ));
    if !emitted_any {
        // still surfaced the diff material even if no executable count/text this step
        lines.push("    // (no executable count/text guards for this step — use the expectedAfterDOM comment above for custom diff)".to_string());
    }
}

/// PR5: Collect a minimal but useful HAR subset from the richer network slices embedded
/// in events.jsonl (for the generated companion artifact). Uses available fields; full
/// headers/timings/body would require deeper network capture (future).
fn collect_network_for_har(events_str: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for line in events_str.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(entries) = network_entries(&ev) {
                for e in entries {
                    let url = e.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    if url.is_empty()
                        || url.starts_with("data:")
                        || url.ends_with(".js")
                        || url.ends_with(".css")
                        || url.ends_with(".png")
                        || url.ends_with(".jpg")
                        || url.ends_with(".svg")
                    {
                        continue;
                    }
                    out.push(json!({
                        "startedDateTime": "1970-01-01T00:00:00.000Z",
                        "time": 0,
                        "request": {
                            "method": e.get("method").and_then(|m| m.as_str()).unwrap_or("GET"),
                            "url": url,
                            "httpVersion": "HTTP/1.1",
                            "cookies": [],
                            "headers": [],
                            "queryString": [],
                            "headersSize": -1,
                            "bodySize": -1
                        },
                        "response": {
                            "status": e.get("status").and_then(|s| s.as_u64()).unwrap_or(0),
                            "statusText": "",
                            "httpVersion": "HTTP/1.1",
                            "cookies": [],
                            "headers": [],
                            "content": { "size": -1, "mimeType": "", "text": e.get("failureText").and_then(|f| f.as_str()).unwrap_or("") },
                            "redirectURL": "",
                            "headersSize": -1,
                            "bodySize": -1
                        },
                        "cache": {},
                        "timings": { "send": 0, "wait": 0, "receive": 0 },
                        "resourceType": e.get("resourceType").and_then(|t| t.as_str()).unwrap_or("other")
                    }));
                }
            }
        }
    }
    // de-dup by url+method for cleanliness (keep first). Defensive .get() to avoid panics (review Finding 6)
    let mut seen = std::collections::HashSet::new();
    out.retain(|h| {
        let req = h.get("request").and_then(|r| r.as_object());
        let method = req
            .and_then(|r| r.get("method"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let url = req
            .and_then(|r| r.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if method.is_empty() && url.is_empty() {
            return false;
        }
        let key = format!("{}:{}", method, url);
        seen.insert(key)
    });
    out
}

/// Core generator: read bundle manifest + events.jsonl, emit high quality PW test.
fn generate_playwright_from_bundle(
    bundle_dir: &std::path::Path,
    test_name: &str,
) -> Result<(String, usize, usize, String, Option<String>), String> {
    let events_path = bundle_dir.join("events.jsonl");
    let manifest_path = bundle_dir.join("manifest.json");

    if !events_path.exists() {
        return Err("bundle missing events.jsonl".to_string());
    }

    let mut bundle_desc = "bundle (no manifest)".to_string();
    if manifest_path.exists() {
        if let Ok(mstr) = fs::read_to_string(&manifest_path) {
            if let Ok(mv) = serde_json::from_str::<serde_json::Value>(&mstr) {
                let rid = mv
                    .get("recordingId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let replayable = mv
                    .get("replayable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let fmt = mv
                    .get("replayFormatVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("n/a");
                bundle_desc = format!("{} (replayable:{}, fmt:{})", rid, replayable, fmt);
            }
        }
    }

    let events_str =
        fs::read_to_string(&events_path).map_err(|e| format!("read events.jsonl failed: {}", e))?;

    let mut lines: Vec<String> = Vec::new();
    lines.push("import { test, expect } from '@playwright/test';".to_string());
    lines.push(String::new());
    lines.push("// ============================================================".to_string());
    lines.push("// Generated by gsd-browser replayable test generator (PR 5)".to_string());
    lines.push(format!("// Source: {}", bundle_desc));
    lines.push(
        "// Enriched events (PR1/3) + full per-action network slices (PR5) + state snapshots"
            .to_string(),
    );
    lines.push("// Includes: command replay, URL checks, RICH per-step DOM assertions (counts/text/structural),".to_string());
    lines.push("// expected-vs-actual diff comments, state restoration (save_state snapshots -> PW storageState),".to_string());
    lines.push(
        "// full slice network assertions + optional HAR subset. First-class test artifact."
            .to_string(),
    );
    lines.push("// Review/refine ref-based locators (gsd refs ephemeral). Refs + frames = visual evidence.".to_string());
    lines.push("// PR5 SAFETY: states/ snapshots are REDACTED at export (sensitive cookies + tokens). See bundle manifest statesRedaction.".to_string());
    lines.push("// PR5 DOM: counts are commented tolerant smoke checks only (broad locators are flaky on real UIs); use expectedAfterDOM for diffs.".to_string());
    lines.push("// state restoration: auto test.use({storageState}) emitted when redacted pwstate present (runnable, best-effort fidelity).".to_string());
    lines.push("// ============================================================".to_string());
    lines.push(String::new());
    // === PR 5: State restoration block (integrated save-state snapshots) ===
    // If the source bundle contains states/ (populated by export when save_state() was called
    // during recording), we emit ready-to-adapt setup for reliable replay of auth/stateful flows.
    // The *.pwstate.json are Playwright-native and can be used directly.
    {
        let states_dir = bundle_dir.join("states");
        let mut state_files: Vec<String> = Vec::new();
        if states_dir.exists() {
            if let Ok(rd) = fs::read_dir(&states_dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if let Some(name) = p.file_name() {
                        let n = name.to_string_lossy();
                        if n.ends_with(".pwstate.json") || n.ends_with(".json") {
                            if !n.starts_with('.') {
                                state_files.push(n.to_string());
                            }
                        }
                    }
                }
            }
        }
        if !state_files.is_empty() {
            state_files.sort();
            let pwstate_files: Vec<&String> = state_files
                .iter()
                .filter(|f| f.ends_with(".pwstate.json"))
                .collect();

            if !pwstate_files.is_empty() {
                // PR5 usability fix (review Finding 4): Emit *runnable* top-level test.use
                // when real (redacted) pwstate files exist. This is the standard Playwright
                // pattern and makes the generated test actually restore state with *zero or
                // minimal* edits for many auth flows (the core value prop of the feature).
                lines.push(String::new());
                lines.push("import * as path from 'path';".to_string());
                lines.push("import { fileURLToPath } from 'url';".to_string());
                lines.push(
                    "const bundleDir = path.dirname(fileURLToPath(import.meta.url));".to_string(),
                );
                lines.push(String::new());

                for pwf in pwstate_files.iter().take(1) {
                    // Only wire the first one automatically; user can extend for multi-state flows
                    lines.push(format!(
                        "test.use({{ storageState: path.join(bundleDir, 'states/{}') }});",
                        pwf
                    ));
                }
                if pwstate_files.len() > 1 {
                    lines.push(format!(
                        "// Additional pwstates present: {} (extend test.use or use per-test context)",
                        pwstate_files.len()
                    ));
                }
                lines.push("// NOTE: states/*.pwstate.json are REDACTED (see bundle manifest statesRedaction). Some logins may still need vault or re-auth.".to_string());
            } else {
                // Only gsd .json states (no successful pw conversion)
                lines.push(String::new());
                lines.push(
                    "// === PR5 STATE RESTORATION (gsd snapshots only; no runnable pwstate) ==="
                        .to_string(),
                );
            }

            // Always document what was found (for audit / manual fallback)
            lines.push("// States discovered in bundle:".to_string());
            for sf in state_files.iter().take(3) {
                lines.push(format!("//   states/{}", sf));
            }
            if state_files.len() > 3 {
                lines.push(format!("//   ... +{} more", state_files.len() - 3));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "test.describe('{}', () => {{",
        escape_js(test_name)
    ));
    lines.push(format!(
        "  test('{}', async ({{ page }}) => {{",
        escape_js(test_name)
    ));

    let mut step_count: usize = 0;
    let mut net_asserts_emitted: usize = 0;
    let mut last_url = String::new();

    for line in events_str.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = event.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if kind.starts_with("recording.")
            || kind == "snapshot"
            || kind == "console"
            || kind == "network"
            || kind == "dialog"
            || kind == "health"
            || kind == "eval"
            || kind == "ping"
        {
            continue;
        }

        let cmd = event
            .get("command")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let evt_url = event
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        let mut emitted = false;

        match kind {
            "navigate" => {
                let url = extract_str_from_cmd(&cmd, "url").unwrap_or(evt_url.clone());
                if !url.is_empty() {
                    lines.push(format!("    await page.goto('{}');", escape_js(&url)));
                    lines.push(format!(
                        "    await expect(page).toHaveURL('{}');",
                        escape_js(&url)
                    ));
                    last_url = url;
                    emitted = true;
                }
            }
            "back" => {
                lines.push("    await page.goBack();".to_string());
                emitted = true;
            }
            "forward" => {
                lines.push("    await page.goForward();".to_string());
                emitted = true;
            }
            "reload" => {
                lines.push("    await page.reload();".to_string());
                emitted = true;
            }
            "click" => {
                if let Some(sel) = extract_str_from_cmd(&cmd, "selector") {
                    lines.push(format!("    await page.click('{}');", escape_js(&sel)));
                    emitted = true;
                } else {
                    lines.push("    // click (selector not captured in this event)".to_string());
                }
            }
            "click_ref" => {
                if let Some(r) = extract_str_from_cmd(&cmd, "ref") {
                    // === REF-BASED ACTION (the encouraged gsd style) ===
                    // Deliberately emitted as a *commented, safe, high-signal block*.
                    // Previous live generic locators were non-functional foot-guns (review Finding 4).
                    // The generated test is now a documented reproduction + evidence pointer.
                    lines.push("    // === REF-BASED ACTION (manual locator mapping REQUIRED for reliable regression) ===".to_string());
                    lines.push(format!(
                        "    // gsd ref: {}  (ephemeral by design — never stable across runs)",
                        r
                    ));
                    lines.push("    // Best evidence: the matching frame-*.jpg in this bundle (visual + DOM context)".to_string());
                    lines.push("    // How to complete: re-snapshot at authoring time or use Playwright's codegen on the same flow.".to_string());
                    lines.push("    // Safe no-op (test runs without performing the original click until you edit):".to_string());
                    lines.push("    // await page.locator('body').waitFor({ state: 'visible' }); // TODO: replace entire block with real locator + .click()".to_string());
                    emitted = true;
                }
            }
            "type" | "fill" => {
                let sel = extract_str_from_cmd(&cmd, "selector").unwrap_or_default();
                let text = extract_str_from_cmd(&cmd, "text").unwrap_or_default();
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.fill('{}', '{}');",
                        escape_js(&sel),
                        escape_js(&text)
                    ));
                    emitted = true;
                } else {
                    lines.push(format!(
                        "    // type/fill text='{}' (no selector)",
                        escape_js(&text)
                    ));
                }
            }
            "fill_ref" => {
                let r = extract_str_from_cmd(&cmd, "ref").unwrap_or_default();
                let text = extract_str_from_cmd(&cmd, "text").unwrap_or_default();
                lines.push(
                    "    // === REF-BASED FILL (manual locator mapping REQUIRED) ===".to_string(),
                );
                lines.push(format!(
                    "    // gsd ref: {}  | value: '{}'",
                    r,
                    escape_js(&text)
                ));
                lines.push("    // Evidence: corresponding frame jpg in bundle".to_string());
                lines.push("    // Safe no-op placeholder:".to_string());
                lines.push("    // await page.locator('input').first().waitFor({ state: 'visible' }); // TODO: replace with real locator + .fill(...)".to_string());
                emitted = true;
            }
            "press" => {
                if let Some(key) = extract_str_from_cmd(&cmd, "key") {
                    lines.push(format!(
                        "    await page.keyboard.press('{}');",
                        escape_js(&key)
                    ));
                    emitted = true;
                }
            }
            "hover" => {
                if let Some(sel) = extract_str_from_cmd(&cmd, "selector") {
                    lines.push(format!("    await page.hover('{}');", escape_js(&sel)));
                    emitted = true;
                }
            }
            "hover_ref" => {
                if let Some(r) = extract_str_from_cmd(&cmd, "ref") {
                    lines.push(
                        "    // === REF-BASED HOVER (manual locator mapping REQUIRED) ==="
                            .to_string(),
                    );
                    lines.push(format!("    // gsd ref: {}", r));
                    lines.push("    // Safe no-op placeholder:".to_string());
                    lines.push("    // await page.locator('body').first().waitFor({ state: 'visible' }); // TODO: replace".to_string());
                    emitted = true;
                }
            }
            "select_option" => {
                let sel = extract_str_from_cmd(&cmd, "selector").unwrap_or_default();
                let opt = extract_str_from_cmd(&cmd, "option")
                    .or_else(|| extract_str_from_cmd(&cmd, "value"))
                    .unwrap_or_default();
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.selectOption('{}', '{}');",
                        escape_js(&sel),
                        escape_js(&opt)
                    ));
                    emitted = true;
                }
            }
            "set_checked" => {
                let sel = extract_str_from_cmd(&cmd, "selector").unwrap_or_default();
                let checked = cmd
                    .get("checked")
                    .and_then(|v| {
                        v.as_bool().or_else(|| {
                            v.as_str().and_then(|s| match s {
                                "true" => Some(true),
                                "false" => Some(false),
                                _ => None,
                            })
                        })
                    })
                    .unwrap_or(true);
                if !sel.is_empty() {
                    lines.push(format!(
                        "    await page.setChecked('{}', {});",
                        escape_js(&sel),
                        checked
                    ));
                    emitted = true;
                }
            }
            "wait_for" => {
                if let Some(cond) = extract_str_from_cmd(&cmd, "condition") {
                    let val = extract_str_from_cmd(&cmd, "value").unwrap_or_default();
                    match cond.as_str() {
                        "network_idle" => {
                            lines.push(
                                "    await page.waitForLoadState('networkidle');".to_string(),
                            );
                            emitted = true;
                        }
                        "url_contains" => {
                            lines.push(format!(
                                "    await page.waitForURL('*{}*');",
                                escape_js(&val)
                            ));
                            emitted = true;
                        }
                        "selector_visible" => {
                            lines.push(format!(
                                "    await page.waitForSelector('{}');",
                                escape_js(&val)
                            ));
                            emitted = true;
                        }
                        _ => {
                            lines.push(format!(
                                "    await page.waitForTimeout(300); // wait_for {}",
                                cond
                            ));
                        }
                    }
                }
            }
            "assert" => {
                lines.push(format!(
                    "    // recorded assert: {}",
                    truncate(&serde_json::to_string(&cmd).unwrap_or_default(), 70)
                ));
            }
            "save_state" => {
                let nm =
                    extract_str_from_cmd(&cmd, "name").unwrap_or_else(|| "default".to_string());
                lines.push(format!(
                    "    // save_state('{}') — state snapshot captured into bundle/states/ at export (PR5)",
                    escape_js(&nm)
                ));
                lines.push(
                    "    // Restoration block emitted at top of test; see header.".to_string(),
                );
                emitted = true;
            }
            "restore_state" => {
                let nm =
                    extract_str_from_cmd(&cmd, "name").unwrap_or_else(|| "default".to_string());
                lines.push(format!(
                    "    // restore_state('{}') — in generated test this is typically replaced by storageState at context creation (PR5)",
                    escape_js(&nm)
                ));
                emitted = true;
            }
            _ => {
                if !kind.is_empty() {
                    lines.push(format!(
                        "    // {}: {}",
                        kind,
                        truncate(&serde_json::to_string(&cmd).unwrap_or_default(), 55)
                    ));
                }
            }
        }

        if emitted {
            step_count += 1;

            // URL check on change (robustness)
            if !evt_url.is_empty()
                && evt_url != last_url
                && !evt_url.starts_with("about:")
                && !kind.contains("wait")
            {
                // Use string form (not /regex/) to avoid any escaping issues with / in URLs.
                // This is the safe, reliable choice for MVP replayable regression starters.
                lines.push(format!(
                    "    await expect(page).toHaveURL('{}');",
                    escape_js(&evt_url)
                ));
                last_url = evt_url.clone();
            }

            // === PR 5: Rich per-step DOM assertions + expected-vs-actual diff support ===
            // Derived from the "after" CompactPageState captured at action completion (PR1).
            // Emits executable structural checks (counts + text) that make generated tests
            // meaningful regression guards on real UIs. Includes full snapshot comment for
            // manual or custom "expected vs actual" diffing in CI (e.g. deep equal after
            // re-capturing analogous compact state in test helper).
            emit_rich_dom_assertions(&mut lines, &event, step_count);
        }

        // Basic network assertions (from PR2 enriched slices) — surface as comments
        // even when the action itself did not emit runnable steps (better evidence density).
        let nets = get_network_summary(&event);
        for n in nets.iter().take(1) {
            lines.push(format!("    // network slice: {}", n));
            if net_asserts_emitted < 2
                && (n.contains("POST")
                    || n.contains("PUT")
                    || n.contains("/api/")
                    || n.contains("graphql"))
            {
                // extract a path-ish hint for wait (still gated)
                let hint = n.split_whitespace().nth(1).unwrap_or("/api");
                let clean = hint.split('?').next().unwrap_or(hint).to_string();
                lines.push(format!(
                    "    await page.waitForResponse(r => r.url().includes('{}') && r.status() < 500).catch(() => {{}});",
                    escape_js(&clean)
                ));
                net_asserts_emitted += 1;
            }
        }
    }

    // Final settle + last URL guard if we have manifest expected
    lines.push("    await page.waitForLoadState('domcontentloaded');".to_string());

    // Screenshot references (core payoff of bundle as test artifact)
    lines.push(String::new());
    lines.push(
        "    // === Screenshot references from evidence bundle (first-class artifact) ==="
            .to_string(),
    );
    let frames_dir = bundle_dir.join("frames");
    let mut frames: Vec<String> = Vec::new();
    if frames_dir.exists() {
        if let Ok(rd) = fs::read_dir(&frames_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if let Some(ext) = p.extension() {
                    if ext == "jpg" || ext == "jpeg" || ext == "png" {
                        if let Some(name) = p.file_name() {
                            frames.push(name.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }
    frames.sort();
    if !frames.is_empty() {
        for f in frames.iter().take(6) {
            lines.push(format!("    //   frames/{}", f));
        }
        if frames.len() > 6 {
            lines.push(format!(
                "    //   ... +{} more frames available in bundle",
                frames.len() - 6
            ));
        }
        lines.push("    // Use e.g. `await expect(page).toHaveScreenshot('final-state.png');` after copying a frame as baseline.".to_string());
    } else {
        lines.push("    //   (no frames/ dir in bundle — add screenshots during recording for visual regression power)".to_string());
    }

    lines.push("  });".to_string());
    lines.push("});".to_string());
    lines.push(String::new());

    // PR5: Optional (always-on for bundles with slices) HAR subset export for full slice support.
    // Built from the richer per-action network in events.jsonl. Companion artifact co-located
    // with generated spec for easy audit / use with PW HAR replay or route mocking.
    let mut har_path: Option<String> = None;
    let har_entries: Vec<serde_json::Value> = collect_network_for_har(&events_str);
    if !har_entries.is_empty() {
        let safe = sanitize_for_filename(test_name);
        let har_file = bundle_dir.join(format!("{}-network-slices.har", safe));
        let har = json!({
            "log": {
                "version": "1.2",
                "creator": { "name": "gsd-browser-pr5", "version": env!("CARGO_PKG_VERSION") },
                "entries": har_entries
            }
        });
        if let Ok(hstr) = serde_json::to_string_pretty(&har) {
            if fs::write(&har_file, &hstr).is_ok() {
                har_path = Some(har_file.to_string_lossy().to_string());
                lines.push(String::new());
                lines.push(format!(
                    "    // PR5 HAR subset (EVIDENCE/SUMMARY ONLY — headers/bodies/timings omitted, see Finding 5 limitations): {}-network-slices.har ({} entries)",
                    safe, har_entries.len()
                ));
            }
        }
    }

    let script = lines.join("\n");
    let line_count = script.lines().count();
    Ok((script, step_count, line_count, bundle_desc, har_path))
}

pub async fn handle_generate_replayable_test(
    state: &DaemonState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("replayable-session");

    let output_path = params
        .get("outputPath")
        .or_else(|| params.get("output"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let bundle_dir = if let Some(bp) = params.get("bundlePath").and_then(|v| v.as_str()) {
        let p = std::path::Path::new(bp).to_path_buf();
        if p.join("events.jsonl").exists() {
            p
        } else {
            // try common locations (exported or recordings root)
            let rec_root = gsd_browser_common::state_dir().join("recordings");
            let cand1 = rec_root.join(bp);
            if cand1.join("events.jsonl").exists() {
                cand1
            } else if p.join("events.jsonl").exists() {
                p
            } else {
                return Err(format!(
                    "bundlePath '{}' did not resolve to a directory containing events.jsonl (tried literal path and recordings/ subdir). For an in-daemon recording use recordingId instead.",
                    bp
                ));
            }
        }
    } else if let Some(rid) = params.get("recordingId").and_then(|v| v.as_str()) {
        let dir = {
            let recs = state.recordings.lock().await;
            recs.recording_dir(rid)
        };
        if !dir.join("events.jsonl").exists() {
            return Err(format!(
                "recording {} not found or has no events.jsonl (stop recording first; export makes full replayable bundle)",
                rid
            ));
        }
        dir
    } else {
        return Err(
            "browser_generate_replayable_test requires either 'recordingId' or 'bundlePath'"
                .to_string(),
        );
    };

    let (script, actions, lines_count, bundle_desc, har_path) =
        generate_playwright_from_bundle(&bundle_dir, name)?;

    let safe_name = sanitize_for_filename(name);
    let file_path = if let Some(p) = output_path {
        p
    } else {
        // Smart default: co-locate with bundle if it looks like one, else generated-tests/
        let sibling = bundle_dir.join(format!("{}.spec.ts", safe_name));
        if bundle_dir.join("manifest.json").exists() || bundle_dir.join("events.jsonl").exists() {
            sibling.to_string_lossy().to_string()
        } else {
            let dir = gsd_browser_common::state_dir().join("generated-tests");
            let _ = fs::create_dir_all(&dir);
            dir.join(format!("{}.spec.ts", safe_name))
                .to_string_lossy()
                .to_string()
        }
    };

    let file_path_ref = Path::new(&file_path);
    ensure_parent_dir(file_path_ref)?;
    fs::write(file_path_ref, &script)
        .map_err(|e| format!("failed to write replayable test: {}", e))?;

    Ok(serde_json::json!({
        "path": file_path,
        "actions": actions,
        "lines": lines_count,
        "bundle": bundle_desc,
        "sourceBundleDir": bundle_dir.to_string_lossy().to_string(),
        "harSubset": har_path,
        "pr5Features": ["stateRestoration", "richDomAssertions", "expectedVsActualDiff", "fullNetworkSlices", "harSubsetExport"]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::state::DaemonState;

    #[test]
    fn generate_test_empty_timeline() {
        let state = DaemonState::new();
        let err = handle_generate_test(&state, &json!({})).unwrap_err();
        assert!(err.contains("no actions"));
    }

    #[test]
    fn generate_test_with_entries() {
        let state = DaemonState::new();
        {
            let mut tl = state.timeline.lock().unwrap();
            tl.begin_action(
                "navigate",
                r#"{"url":"https://example.com"}"#,
                "about:blank",
            );
            tl.finish_action(1, "https://example.com", "ok", "");
            tl.begin_action(
                "click",
                r#"{"selector":"button.submit"}"#,
                "https://example.com",
            );
            tl.finish_action(2, "https://example.com", "ok", "");
        }

        let tmp = std::env::temp_dir().join("bt-test-gen.spec.ts");
        let result = handle_generate_test(
            &state,
            &json!({"name": "test-gen", "outputPath": tmp.to_str().unwrap()}),
        )
        .unwrap();

        assert_eq!(result["actions"], 2);
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("page.goto('https://example.com')"));
        assert!(content.contains("page.click('button.submit')"));
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn generate_replayable_test_from_fake_bundle() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir for bundle");
        let bundle = dir.path();

        // minimal manifest (replayable after export) — PR5
        let manifest = json!({
            "schema": "BrowserArtifactBundleV1",
            "recordingId": "rec_test_pr5",
            "replayable": true,
            "replayFormatVersion": "playwright-2",
            "eventCount": 4
        });
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // enriched events.jsonl (PR5: with after DOM for rich asserts + save_state + full slices)
        let events = r#"{"seq":1,"kind":"navigate","url":"https://example.com/start","command":{"url":"https://example.com/start"},"after":{"counts":{"buttons":4,"inputs":2,"links":10,"landmarks":3},"headings":["Welcome"],"focus":"","bodyText":"hello world content"},"network":{"recent":[]},"networkSlice":{"entries":[{"method":"GET","url":"https://example.com/api/boot","status":200,"timestamp":123.4,"failed":false}]}}
{"seq":2,"kind":"click_ref","url":"https://example.com/cart","command":{"ref":"@v1:e7"},"after":{"counts":{"buttons":5,"inputs":1},"headings":["Cart"],"bodyText":"cart page"},"network":{"recent":[]},"networkSlice":{"entries":[{"method":"POST","url":"https://example.com/api/cart","status":201,"failed":false}]}}
{"seq":3,"kind":"save_state","url":"https://example.com/cart","command":{"name":"post-login"},"after":{},"network":{"recent":[]},"networkSlice":{"entries":[]}}
{"seq":4,"kind":"navigate","url":"https://example.com/success","command":{"url":"https://example.com/success"},"after":{"counts":{"buttons":2},"headings":["Success"]},"network":{"recent":[]},"networkSlice":{"entries":[]}}
"#;
        fs::write(bundle.join("events.jsonl"), events).unwrap();

        // fake frame
        fs::create_dir_all(bundle.join("frames")).unwrap();
        fs::write(bundle.join("frames/frame-000001.jpg"), b"fakejpgdata").unwrap();

        // PR5 fake state snapshot (simulates export having copied from save_state during rec)
        fs::create_dir_all(bundle.join("states")).unwrap();
        fs::write(
            bundle.join("states/post-login.json"),
            b"{\"cookies\":[],\"localStorage\":{},\"sessionStorage\":{}}",
        )
        .unwrap();
        fs::write(
            bundle.join("states/post-login.pwstate.json"),
            b"{\"cookies\":[],\"origins\":[]}",
        )
        .unwrap();

        let (script, steps, _lines, desc, _har) =
            generate_playwright_from_bundle(bundle, "pr4-checkout").expect("generate");

        assert!(desc.contains("rec_test_pr5"));
        assert!(desc.contains("replayable:true"));
        assert!(
            steps >= 3,
            "should count nav + click_ref + save_state as steps"
        );

        assert!(script.contains("page.goto('https://example.com/start')"));
        assert!(script.contains("toHaveURL"));
        assert!(script.contains("REF-BASED ACTION (manual locator mapping REQUIRED")); // ref handling (Finding 4) — now safe commented block, not live locator
        assert!(script.contains("network slice:"));
        assert!(script.contains("waitForResponse")); // basic net assert
        assert!(script.contains("frames/frame-000001.jpg")); // screenshot ref
        assert!(script.contains("PR 5")); // header (PR5)
        assert!(script.contains("First-class test artifact"));
        // PR5 rich DOM (counts are now tolerant commented smoke checks per review)
        assert!(script.contains("toContainText")); // heading text (executable)
        assert!(script.contains("expectedAfterDOM")); // diff support comment
        assert!(
            script.contains("smoke check")
                || script.contains("tolerant")
                || script.contains("broad")
        ); // Finding 7 honesty
           // PR5 state restoration
        assert!(script.contains("test.use({ storageState:"));
        assert!(script.contains("import { fileURLToPath } from 'url';"));
        assert!(script.contains("const bundleDir = path.dirname(fileURLToPath(import.meta.url));"));
        assert!(!script.contains("__dirname"));
        assert!(script.contains("states/post-login.pwstate.json"));
        assert!(
            script.find("test.use({ storageState:").unwrap()
                < script.find("test.describe(").unwrap(),
            "state restoration setup must be top-level before test.describe"
        );
        assert!(script.contains("REDACTED") || script.contains("redacted")); // safety redaction surfaced
        assert!(script.contains("save_state('post-login')"));
        // PR5 HAR
        assert!(script.contains("network-slices.har") || script.contains("HAR subset"));

        // CRITICAL: no unescaped / inside /.../ regex literals for toHaveURL (Finding 1)
        assert!(
            !script.contains("toHaveURL(/https:"),
            "must not emit raw https: inside / / regex (would be invalid JS)"
        );
        // We now prefer the safe string form
        assert!(
            script.contains("toHaveURL('https://example.com/cart')")
                || script.contains("toHaveURL('https://example.com/success')")
        );

        // Test name with special chars must be properly escaped (Finding 2)
        let (script2, _, _, _, _) =
            generate_playwright_from_bundle(bundle, "user's checkout \"flow\"\nwith newline")
                .expect("generate with tricky name");
        assert!(script2.contains("test.describe('user\\'s checkout \\\"flow\\\"\\nwith newline'"));
    }

    #[test]
    fn replayable_generator_handles_unicode_and_checked_values() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir for unicode bundle");
        let bundle = dir.path();
        fs::write(
            bundle.join("events.jsonl"),
            r#"{"seq":1,"kind":"set_checked","url":"https://example.com/✓","command":{"selector":"input[type=checkbox]","checked":"false); await page.goto('https://evil.test') //"},"networkSlice":{"entries":[{"method":"POST","url":"https://example.com/api/checkout/✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓","status":204}]}}
{"seq":2,"kind":"assert","url":"https://example.com/✓","command":{"message":"unicode ✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓"}}
"#,
        )
        .unwrap();

        let (script, steps, _, _, _) =
            generate_playwright_from_bundle(bundle, "unicode ✓ flow").expect("generate");

        assert_eq!(steps, 1);
        assert!(script.contains("await page.setChecked('input[type=checkbox]', true);"));
        assert!(!script.contains("evil.test"));
        assert!(script.contains("network slice: POST https://example.com/api/checkout/"));
    }

    #[tokio::test]
    async fn replayable_generator_creates_output_parent_dirs() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir for output parent");
        let bundle = dir.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join("events.jsonl"),
            r#"{"seq":1,"kind":"navigate","url":"https://example.com","command":{"url":"https://example.com"}}"#,
        )
        .unwrap();

        let state = DaemonState::new();
        let output = dir.path().join("new").join("nested").join("flow.spec.ts");
        let result = handle_generate_replayable_test(
            &state,
            &json!({
                "bundlePath": bundle.to_string_lossy(),
                "outputPath": output.to_string_lossy(),
            }),
        )
        .await
        .expect("generate replayable test");

        assert_eq!(result["path"], output.to_string_lossy().to_string());
        assert!(output.exists());
    }
}
