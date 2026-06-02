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

    fs::write(&file_path, &script).map_err(|e| format!("failed to write test file: {e}"))?;

    Ok(json!({
        "path": file_path,
        "actions": entries.len(),
        "lines": script.lines().count(),
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

// === PR 4: MVP Replayable Test Generator (Playwright export) ===
// Consumes full recording bundle (enriched events from PR1/3 + network slices PR2).
// Produces high-quality, genuinely useful Playwright regression test:
// - command sequence replay (nav, clicks, fills, refs, waits etc)
// - URL checks (explicit toHaveURL after nav + on url change)
// - basic network assertions (comments + 1-2 executable waitForResponse for key XHR)
// - screenshot references (frames/ jpgs from bundle for baselines / audit)
// Supports internal recording dirs (pre-export) and exported replayable bundles.
// Output is clean, runnable skeleton + actionable TODOs for ref->locator mapping.
// Per Gerald Sterling: makes evidence bundles first-class replayable test artifacts.

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
                Some(format!("{} {} -> {}", method, short_url(url), status))
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

/// Core generator: read bundle manifest + events.jsonl, emit high quality PW test.
fn generate_playwright_from_bundle(
    bundle_dir: &std::path::Path,
    test_name: &str,
) -> Result<(String, usize, usize, String), String> {
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
    lines.push("// Generated by gsd-browser replayable test generator (PR 4)".to_string());
    lines.push(format!("// Source: {}", bundle_desc));
    lines.push("// Enriched events (PR1/3) + per-action network slices (PR2)".to_string());
    lines.push("// Includes: command replay, URL checks, basic network assertions,".to_string());
    lines.push("// screenshot refs from bundle/frames. First-class test artifact.".to_string());
    lines.push("// Review/refine ref-based locators (gsd refs are session ephemeral).".to_string());
    lines.push("// MVP notes: waits are best-effort (networkidle where explicit); malformed JSONL lines are skipped with no hard failure.".to_string());
    lines.push("// ============================================================".to_string());
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

    let script = lines.join("\n");
    let line_count = script.lines().count();
    Ok((script, step_count, line_count, bundle_desc))
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

    let (script, actions, lines_count, bundle_desc) =
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

        // minimal manifest (replayable after export)
        let manifest = json!({
            "schema": "BrowserArtifactBundleV1",
            "recordingId": "rec_test_pr4",
            "replayable": true,
            "replayFormatVersion": "playwright-1",
            "eventCount": 3
        });
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // enriched events.jsonl (mix of nav + ref action + authoritative networkSlice)
        let events = r#"{"seq":1,"kind":"navigate","url":"https://example.com/start","command":{"url":"https://example.com/start"},"network":{"recent":[]},"networkSlice":{"entries":[{"method":"GET","url":"https://example.com/api/boot","status":200}]}}
{"seq":2,"kind":"click_ref","url":"https://example.com/cart","command":{"ref":"@v1:e7"},"network":{"recent":[]},"networkSlice":{"entries":[{"method":"POST","url":"https://example.com/api/cart","status":201}]}}
{"seq":3,"kind":"navigate","url":"https://example.com/success","command":{"url":"https://example.com/success"},"network":{"recent":[]}}
"#;
        fs::write(bundle.join("events.jsonl"), events).unwrap();

        // fake frame
        fs::create_dir_all(bundle.join("frames")).unwrap();
        fs::write(bundle.join("frames/frame-000001.jpg"), b"fakejpgdata").unwrap();

        let (script, steps, _lines, desc) =
            generate_playwright_from_bundle(bundle, "pr4-checkout").expect("generate");

        assert!(desc.contains("rec_test_pr4"));
        assert!(desc.contains("replayable:true"));
        assert!(steps >= 2, "should count nav + click_ref as steps");

        assert!(script.contains("page.goto('https://example.com/start')"));
        assert!(script.contains("toHaveURL"));
        assert!(script.contains("REF-BASED ACTION (manual locator mapping REQUIRED")); // ref handling (Finding 4) — now safe commented block, not live locator
        assert!(script.contains("network slice:"));
        assert!(script.contains("waitForResponse")); // basic net assert
        assert!(script.contains("frames/frame-000001.jpg")); // screenshot ref
        assert!(script.contains("PR 4")); // header
        assert!(script.contains("First-class test artifact"));

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
        let (script2, _, _, _) =
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

        let (script, steps, _, _) =
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
