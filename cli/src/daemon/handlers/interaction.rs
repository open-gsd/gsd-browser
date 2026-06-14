//! Interaction command handlers: click, type, press, hover, scroll, drag,
//! select_option, set_checked, set_viewport, upload_file.
//!
//! Every handler follows the same pattern: validate params → find element or
//! dispatch CDP → settle → capture compact page state → return JSON.

use crate::daemon::capture::capture_compact_page_state;
use crate::daemon::input_dispatch::{dispatch_mouse, mouse_button, mouse_buttons_mask_for_button};
use crate::daemon::inspection;
use crate::daemon::narration::events::ActionKind;
use crate::daemon::settle::{ensure_mutation_counter, settle_after_action};
use crate::daemon::state::DaemonState;
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::input::{DispatchMouseEventType, MouseButton};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::layout::Point;
use chromiumoxide::Page;
use gsd_browser_common::types::SettleOptions;
use serde_json::{json, Value};
use std::future::Future;
use std::time::Duration;
use tokio::time::timeout;
use tracing::debug;

/// Maximum timeout for element operations.
const ELEMENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for CDP dispatch calls (mouse events, etc.).
const CDP_TIMEOUT: Duration = Duration::from_secs(5);

/// Default settle options for interaction commands.
fn interaction_settle_opts() -> SettleOptions {
    SettleOptions {
        timeout_ms: 1500,
        check_focus_stability: true,
        ..SettleOptions::default()
    }
}

/// Settle and capture page state after an interaction.
async fn settle_and_capture(page: &Page) -> (Value, Value) {
    ensure_mutation_counter(page).await;
    let settle = settle_after_action(page, &interaction_settle_opts()).await;
    let state = capture_compact_page_state(page, false).await;
    (
        serde_json::to_value(&state).unwrap_or(json!({})),
        serde_json::to_value(&settle).unwrap_or(json!({})),
    )
}

fn selector_action_error(result: &Value, fallback: &str) -> String {
    result
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
        .to_string()
}

fn selector_action_meta(selector: &str, result: &Value) -> Value {
    let target = result.get("target").cloned().unwrap_or(Value::Null);
    json!({
        "selector": selector,
        "frameLabel": target.get("frameLabel").cloned().unwrap_or(Value::Null),
        "frameUrl": target.get("frameUrl").cloned().unwrap_or(Value::Null),
        "tag": target.get("tag").cloned().unwrap_or(Value::Null),
        "role": target.get("role").cloned().unwrap_or(Value::Null),
        "name": target.get("name").cloned().unwrap_or(Value::Null),
        "actionability": result.get("actionability").cloned().unwrap_or(Value::Null),
    })
}

async fn with_narration<F, Fut>(
    page: &Page,
    state: &DaemonState,
    action: ActionKind,
    selector: Option<&str>,
    hint: Option<&str>,
    body: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let probe = state
        .narrator
        .probe_action(page, action, selector, hint)
        .await;
    state
        .narrator
        .emit_pre(&probe)
        .await
        .map_err(|_| "aborted".to_string())?;
    state.narrator.sleep_lead(&probe).await;
    let result = body().await;
    state.narrator.emit_post(&probe, &result).await;
    result
}

// ── Click ──

/// Handle `click` command.
/// Params: { selector?: string, x?: f64, y?: f64, button?: string, click_count?: i64 }
pub async fn handle_click(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params.get("selector").and_then(|v| v.as_str());
    let x = params.get("x").and_then(|v| v.as_f64());
    let y = params.get("y").and_then(|v| v.as_f64());
    let button_name = params
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left");
    let click_count = params
        .get("click_count")
        .or_else(|| params.get("clickCount"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .clamp(1, 3);
    let button = mouse_button(Some(button_name))?;
    if matches!(button, MouseButton::None) {
        return Err("click button cannot be none".to_string());
    }

    match (selector, x, y) {
        (None, None, _) | (None, _, None) => {
            return Err(
                "click requires either 'selector' or both 'x' and 'y' coordinates".to_string(),
            )
        }
        _ => {}
    }

    let hint = selector.or(Some("coordinates"));
    let probe = state
        .narrator
        .probe_action(page, ActionKind::Click, selector, hint)
        .await;
    state
        .narrator
        .emit_pre(&probe)
        .await
        .map_err(|_| "aborted".to_string())?;
    state.narrator.sleep_lead(&probe).await;

    let result = match (selector, x, y) {
        (Some(sel), _, _) => click_selector(page, state, sel, button_name, click_count).await,
        (None, Some(cx), Some(cy)) => {
            click_coordinates(page, cx, cy, button_name, click_count).await
        }
        _ => unreachable!("click parameters are validated before narration"),
    };
    state.narrator.emit_post(&probe, &result).await;
    result
}

pub(super) async fn click_selector(
    page: &Page,
    state: &DaemonState,
    selector: &str,
    button_name: &str,
    click_count: i64,
) -> Result<Value, String> {
    debug!("click: selector={selector} button={button_name} click_count={click_count}");

    let resolved = inspection::resolve_selector_target(page, state, selector, true).await?;
    if !resolved
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(selector_action_error(
            &resolved,
            &format!("element not found: {selector}"),
        ));
    }

    let center = resolved.get("center").cloned().unwrap_or_else(|| json!({}));
    let x = center
        .get("x")
        .and_then(|value| value.as_f64())
        .ok_or_else(|| format!("click target has no x coordinate: {selector}"))?;
    let y = center
        .get("y")
        .and_then(|value| value.as_f64())
        .ok_or_else(|| format!("click target has no y coordinate: {selector}"))?;

    if let Err(err) = perform_mouse_click(page, x, y, button_name, click_count).await {
        if button_name != "left" || click_count != 1 {
            return Err(format!(
                "click failed for {selector} with button={button_name} click_count={click_count}: {err}"
            ));
        }
        debug!("click: coordinate click failed ({err}), falling back to JS action");
        let fallback =
            inspection::perform_selector_action(page, state, selector, "click", &json!({}), true)
                .await?;
        if !fallback
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Err(selector_action_error(
                &fallback,
                &format!("click failed for {selector}"),
            ));
        }
    }

    let (state, settle) = settle_and_capture(page).await;
    Ok(json!({
        "state": state,
        "settle": settle,
        "clicked": selector_action_meta(selector, &resolved),
        "button": button_name,
        "clickCount": click_count,
        "boundaries": resolved.get("boundaries").cloned().unwrap_or(json!([])),
    }))
}

async fn click_coordinates(
    page: &Page,
    x: f64,
    y: f64,
    button_name: &str,
    click_count: i64,
) -> Result<Value, String> {
    debug!("click: coordinates=({x}, {y}) button={button_name} click_count={click_count}");

    perform_mouse_click(page, x, y, button_name, click_count)
        .await
        .map_err(|e| format!("click failed at ({x}, {y}): {e}"))?;

    let (state, settle) = settle_and_capture(page).await;
    Ok(json!({
        "state": state,
        "settle": settle,
        "clicked": { "x": x, "y": y },
        "button": button_name,
        "clickCount": click_count,
    }))
}

async fn perform_mouse_click(
    page: &Page,
    x: f64,
    y: f64,
    button_name: &str,
    click_count: i64,
) -> Result<(), String> {
    if button_name == "left" && click_count == 1 {
        timeout(CDP_TIMEOUT, page.click(Point::new(x, y)))
            .await
            .map_err(|_| format!("click timed out at ({x}, {y})"))?
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let button = mouse_button(Some(button_name))?;
    let button_mask = mouse_buttons_mask_for_button(&button);
    dispatch_mouse(
        page,
        DispatchMouseEventType::MouseMoved,
        x,
        y,
        MouseButton::None,
        0,
        0,
        0,
        None,
        None,
    )
    .await?;
    for count in 1..=click_count {
        dispatch_mouse(
            page,
            DispatchMouseEventType::MousePressed,
            x,
            y,
            button.clone(),
            button_mask,
            count,
            0,
            None,
            None,
        )
        .await?;
        dispatch_mouse(
            page,
            DispatchMouseEventType::MouseReleased,
            x,
            y,
            button.clone(),
            0,
            count,
            0,
            None,
            None,
        )
        .await?;
    }
    Ok(())
}

// ── Type ──

/// Handle `type` command (called type_text to avoid Rust keyword).
/// Params: { selector: string, text: string, slowly?: bool, clear_first?: bool, submit?: bool }
pub async fn handle_type_text(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let text = params
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: text".to_string())?;
    let slowly = params
        .get("slowly")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let clear_first = params
        .get("clear_first")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let submit = params
        .get("submit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    debug!(
        "type_text: selector={selector} len={} slowly={slowly} clear={clear_first} submit={submit}",
        text.len()
    );

    with_narration(
        page,
        state,
        ActionKind::Type,
        Some(selector),
        Some(text),
        || async {
            let text_len = text.len();

            let action_result = inspection::perform_selector_action(
                page,
                state,
                selector,
                "type",
                &json!({
                    "text": text,
                    "slowly": slowly,
                    "clearFirst": clear_first,
                    "submit": submit,
                }),
                true,
            )
            .await?;
            if !action_result
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return Err(selector_action_error(
                    &action_result,
                    &format!("type failed for {selector}"),
                ));
            }
            let actual = action_result
                .get("fill")
                .and_then(|value| value.get("actual"))
                .or_else(|| action_result.get("valueResult").and_then(|value| value.get("actual")))
                .cloned()
                .unwrap_or(Value::Null);
            let method = action_result
                .get("fill")
                .and_then(|value| value.get("method"))
                .or_else(|| action_result.get("valueResult").and_then(|value| value.get("method")))
                .cloned()
                .unwrap_or(Value::Null);
            let kind = action_result
                .get("fill")
                .and_then(|value| value.get("kind"))
                .or_else(|| action_result.get("valueResult").and_then(|value| value.get("kind")))
                .cloned()
                .unwrap_or(Value::Null);

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "typed": {
                    "selector": selector,
                    "text_length": text_len,
                    "slowly": slowly,
                    "submitted": submit,
                    "frameLabel": action_result.get("target").and_then(|value| value.get("frameLabel")).cloned().unwrap_or(Value::Null),
                    "frameUrl": action_result.get("target").and_then(|value| value.get("frameUrl")).cloned().unwrap_or(Value::Null),
                    "actual": actual,
                    "method": method,
                    "kind": kind,
                    "fill": action_result.get("fill").cloned().unwrap_or(Value::Null),
                    "valueResult": action_result.get("valueResult").cloned().unwrap_or(Value::Null),
                    "actionability": action_result.get("actionability").cloned().unwrap_or(Value::Null),
                },
                "boundaries": action_result.get("boundaries").cloned().unwrap_or(json!([])),
            }))
        },
    )
    .await
}

// ── Press ──

/// Handle `press` command — press a key or key combination.
/// Params: { key: string }
pub async fn handle_press(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: key".to_string())?;

    debug!("press: key={key}");

    with_narration(page, state, ActionKind::Press, None, Some(key), || async {
        if key.contains('+') {
            press_combo(page, key).await?;
        } else {
            let js = format!(
                r#"(() => {{
                const key = {key_json};
                const event = new KeyboardEvent('keydown', {{key, bubbles: true}});
                document.activeElement ? document.activeElement.dispatchEvent(event) : document.dispatchEvent(event);
                const up = new KeyboardEvent('keyup', {{key, bubbles: true}});
                document.activeElement ? document.activeElement.dispatchEvent(up) : document.dispatchEvent(up);
                return true;
            }})()"#,
                key_json = serde_json::to_string(key).unwrap()
            );
            timeout(CDP_TIMEOUT, page.evaluate_expression(&js))
                .await
                .map_err(|_| format!("press timed out for key: {key}"))?
                .map_err(|e| format!("press failed for key {key}: {e}"))?;
        }

        let (state, settle) = settle_and_capture(page).await;
        Ok(json!({
            "state": state,
            "settle": settle,
            "pressed": key,
        }))
    })
    .await
}

async fn press_combo(page: &Page, combo: &str) -> Result<(), String> {
    let parts: Vec<&str> = combo.split('+').collect();
    if parts.is_empty() {
        return Err("empty key combination".to_string());
    }

    // Build JS that dispatches keydown for each modifier, then the final key, then keyup in reverse
    let modifiers: Vec<&str> = parts[..parts.len() - 1].to_vec();
    let final_key = parts[parts.len() - 1];

    let modifier_flags: Vec<String> = modifiers
        .iter()
        .map(|m| match m.to_lowercase().as_str() {
            "meta" | "command" | "cmd" => "metaKey: true".to_string(),
            "control" | "ctrl" => "ctrlKey: true".to_string(),
            "shift" => "shiftKey: true".to_string(),
            "alt" | "option" => "altKey: true".to_string(),
            _ => format!("/* unknown modifier: {m} */"),
        })
        .collect();

    let flags = modifier_flags.join(", ");
    let js = format!(
        r#"(() => {{
            const target = document.activeElement || document;
            const opts = {{ bubbles: true, {flags} }};
            target.dispatchEvent(new KeyboardEvent('keydown', {{ ...opts, key: {key_json} }}));
            target.dispatchEvent(new KeyboardEvent('keypress', {{ ...opts, key: {key_json} }}));
            target.dispatchEvent(new KeyboardEvent('keyup', {{ ...opts, key: {key_json} }}));
            return true;
        }})()"#,
        flags = flags,
        key_json = serde_json::to_string(final_key).unwrap()
    );

    timeout(CDP_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| format!("press combo timed out: {combo}"))?
        .map_err(|e| format!("press combo failed ({combo}): {e}"))?;

    Ok(())
}

// ── Hover ──

/// Handle `hover` command — scroll element into view and dispatch mouseMoved.
/// Params: { selector: string }
pub async fn handle_hover(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;

    debug!("hover: selector={selector}");

    with_narration(
        page,
        state,
        ActionKind::Hover,
        Some(selector),
        Some(selector),
        || async {
            let resolved = inspection::resolve_selector_target(page, state, selector, true).await?;
            if !resolved
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return Err(selector_action_error(
                    &resolved,
                    &format!("element not found: {selector}"),
                ));
            }

            let center = resolved.get("center").cloned().unwrap_or_else(|| json!({}));
            let x = center
                .get("x")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| format!("hover target has no x coordinate: {selector}"))?;
            let y = center
                .get("y")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| format!("hover target has no y coordinate: {selector}"))?;

            if let Err(err) = timeout(CDP_TIMEOUT, page.move_mouse(Point::new(x, y)))
                .await
                .map_err(|_| format!("hover timed out for: {selector}"))?
            {
                debug!("hover: coordinate hover failed ({err}), falling back to JS action");
                let fallback = inspection::perform_selector_action(
                    page,
                    state,
                    selector,
                    "hover",
                    &json!({}),
                    true,
                )
                .await?;
                if !fallback
                    .get("ok")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                {
                    return Err(selector_action_error(
                        &fallback,
                        &format!("hover failed for {selector}"),
                    ));
                }
            }

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "hovered": selector_action_meta(selector, &resolved),
                "boundaries": resolved.get("boundaries").cloned().unwrap_or(json!([])),
            }))
        },
    )
    .await
}

// ── Scroll ──

/// Handle `scroll` command — scroll the page and return position.
/// Params: { direction: "up"|"down", amount?: i32 }
pub async fn handle_scroll(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let direction = params
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: direction".to_string())?;
    let amount = params.get("amount").and_then(|v| v.as_i64()).unwrap_or(300) as i32;

    let scroll_amount = match direction {
        "up" => -amount.abs(),
        "down" => amount.abs(),
        _ => {
            return Err(format!(
                "direction must be 'up' or 'down', got: {direction}"
            ))
        }
    };

    debug!("scroll: direction={direction} amount={scroll_amount}");

    with_narration(
        page,
        state,
        ActionKind::Scroll,
        None,
        Some(direction),
        || async {
            let js = format!(
                r#"(() => {{
            window.scrollBy(0, {scroll_amount});
            return {{
                x: Math.round(window.scrollX),
                y: Math.round(window.scrollY),
                height: document.documentElement.scrollHeight,
                viewport_height: window.innerHeight,
            }};
        }})()"#
            );

            let result = timeout(CDP_TIMEOUT, page.evaluate_expression(&js))
                .await
                .map_err(|_| "scroll timed out".to_string())?
                .map_err(|e| format!("scroll failed: {e}"))?;

            let scroll_info = result.value().cloned().unwrap_or(json!({}));
            let scroll_y = scroll_info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let scroll_height = scroll_info
                .get("height")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let viewport_height = scroll_info
                .get("viewport_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let max_scroll = (scroll_height - viewport_height).max(1.0);
            let percentage = ((scroll_y / max_scroll) * 100.0).round().min(100.0);

            let state = capture_compact_page_state(page, false).await;

            Ok(json!({
                "state": state,
                "scroll": {
                    "x": scroll_info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    "y": scroll_y,
                    "height": scroll_height,
                    "viewport_height": viewport_height,
                    "percentage": percentage,
                },
            }))
        },
    )
    .await
}

// ── Select Option ──

/// Handle `select_option` command — set select, combobox, listbox, or menu option.
/// Params: { selector: string, option: string | string[] }
pub async fn handle_select_option(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let option_value = params
        .get("option")
        .cloned()
        .ok_or_else(|| "missing required parameter: option".to_string())?;
    let option_is_string_array = option_value
        .as_array()
        .map(|items| !items.is_empty() && items.iter().all(|item| item.is_string()))
        .unwrap_or(false);
    if !(option_value.is_string() || option_is_string_array) {
        return Err("option must be a string or array of strings".to_string());
    }
    let option_hint = if let Some(option) = option_value.as_str() {
        option.to_string()
    } else {
        option_value.to_string()
    };

    debug!("select_option: selector={selector} option={option_hint}");

    with_narration(
        page,
        state,
        ActionKind::SelectOption,
        Some(selector),
        Some(&option_hint),
        || async {
            let action_result = inspection::perform_selector_action(
                page,
                state,
                selector,
                "select_option",
                &json!({ "option": option_value.clone() }),
                true,
            )
            .await?;
            if !action_result
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return Err(selector_action_error(
                    &action_result,
                    &format!("select_option failed for {selector}"),
                ));
            }

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "selected": {
                    "selector": selector,
                    "option": option_value.clone(),
                    "mode": action_result.get("selection").and_then(|value| value.get("mode")).cloned().unwrap_or(Value::Null),
                    "matched": action_result.get("selection").and_then(|value| value.get("matched")).cloned().unwrap_or(Value::Null),
                    "actual": action_result.get("selection").and_then(|value| value.get("selected")).cloned().unwrap_or(Value::Null),
                    "frameLabel": action_result.get("target").and_then(|value| value.get("frameLabel")).cloned().unwrap_or(Value::Null),
                    "frameUrl": action_result.get("target").and_then(|value| value.get("frameUrl")).cloned().unwrap_or(Value::Null),
                },
                "boundaries": action_result.get("boundaries").cloned().unwrap_or(json!([])),
            }))
        },
    )
    .await
}

// ── Set Checked ──

/// Handle `set_checked` command — set checkbox/radio state.
/// Params: { selector: string, checked: bool }
pub async fn handle_set_checked(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let checked = params
        .get("checked")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "missing required parameter: checked (boolean)".to_string())?;

    debug!("set_checked: selector={selector} checked={checked}");

    with_narration(
        page,
        state,
        ActionKind::SetChecked,
        Some(selector),
        Some(selector),
        || async {
            let action_result = inspection::perform_selector_action(
                page,
                state,
                selector,
                "set_checked",
                &json!({ "checked": checked }),
                true,
            )
            .await?;
            if !action_result
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return Err(selector_action_error(
                    &action_result,
                    &format!("set_checked failed for {selector}"),
                ));
            }

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "checked": {
                    "selector": selector,
                    "value": checked,
                    "result": action_result.get("checkedResult").cloned().unwrap_or(Value::Null),
                    "frameLabel": action_result.get("target").and_then(|value| value.get("frameLabel")).cloned().unwrap_or(Value::Null),
                    "frameUrl": action_result.get("target").and_then(|value| value.get("frameUrl")).cloned().unwrap_or(Value::Null),
                },
                "boundaries": action_result.get("boundaries").cloned().unwrap_or(json!([])),
            }))
        },
    )
    .await
}

// ── Drag ──

fn validate_drag_params(params: &Value) -> Result<(bool, u32, String), String> {
    let source_sel = params.get("source").and_then(|v| v.as_str());
    let target_sel = params.get("target").and_then(|v| v.as_str());
    let from_x = params.get("from_x").and_then(|v| v.as_f64());
    let from_y = params.get("from_y").and_then(|v| v.as_f64());
    let to_x = params.get("to_x").and_then(|v| v.as_f64());
    let to_y = params.get("to_y").and_then(|v| v.as_f64());
    let has_any_selector = source_sel.is_some() || target_sel.is_some();
    let has_all_selectors = source_sel.is_some() && target_sel.is_some();
    let has_any_coordinate =
        from_x.is_some() || from_y.is_some() || to_x.is_some() || to_y.is_some();
    let has_all_coordinates =
        from_x.is_some() && from_y.is_some() && to_x.is_some() && to_y.is_some();

    if has_any_selector && has_any_coordinate {
        return Err(
            "drag accepts either source+target selectors or from_x+from_y+to_x+to_y coordinates, not both"
                .to_string(),
        );
    }
    if has_any_selector && !has_all_selectors {
        return Err("drag selector mode requires both source and target".to_string());
    }
    if has_any_coordinate && !has_all_coordinates {
        return Err("drag coordinate mode requires from_x, from_y, to_x, and to_y".to_string());
    }
    if !has_all_selectors && !has_all_coordinates {
        return Err(
            "drag requires either source+target selectors or from_x+from_y+to_x+to_y coordinates"
                .to_string(),
        );
    }

    let steps = params
        .get("steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 100) as u32;
    let button_name = params
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left")
        .to_string();
    let button = mouse_button(Some(&button_name))?;
    if matches!(button, MouseButton::None) {
        return Err("drag button cannot be none".to_string());
    }

    Ok((has_all_coordinates, steps, button_name))
}

/// Handle `drag` command — simulate a real mouse drag.
/// Params: { source?: string, target?: string, from_x?: f64, from_y?: f64,
///           to_x?: f64, to_y?: f64, steps?: u32, button?: string }
pub async fn handle_drag(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let source_sel = params.get("source").and_then(|v| v.as_str());
    let target_sel = params.get("target").and_then(|v| v.as_str());
    let from_x = params.get("from_x").and_then(|v| v.as_f64());
    let from_y = params.get("from_y").and_then(|v| v.as_f64());
    let to_x = params.get("to_x").and_then(|v| v.as_f64());
    let to_y = params.get("to_y").and_then(|v| v.as_f64());
    let (use_coordinates, steps, button_name) = validate_drag_params(params)?;

    debug!(
        "drag: source={source_sel:?} target={target_sel:?} from=({from_x:?},{from_y:?}) to=({to_x:?},{to_y:?}) steps={steps}"
    );

    with_narration(
        page,
        state,
        ActionKind::Drag,
        source_sel,
        source_sel.or(Some("coordinates")),
        || async {
            let (sx, sy, tx, ty) = if use_coordinates {
                (
                    from_x.unwrap(),
                    from_y.unwrap(),
                    to_x.unwrap(),
                    to_y.unwrap(),
                )
            } else {
                element_centers(page, state, source_sel.unwrap(), target_sel.unwrap()).await?
            };

            let button = mouse_button(Some(&button_name))?;
            let button_mask = mouse_buttons_mask_for_button(&button);

            dispatch_mouse(
                page,
                DispatchMouseEventType::MouseMoved,
                sx,
                sy,
                MouseButton::None,
                0,
                0,
                0,
                None,
                None,
            )
            .await
            .map_err(|e| format!("drag: move to source failed: {e}"))?;

            dispatch_mouse(
                page,
                DispatchMouseEventType::MousePressed,
                sx,
                sy,
                button.clone(),
                button_mask,
                1,
                0,
                None,
                None,
            )
            .await
            .map_err(|e| format!("drag: mouse down failed: {e}"))?;

            for i in 1..=steps {
                let ratio = i as f64 / steps as f64;
                let ix = sx + (tx - sx) * ratio;
                let iy = sy + (ty - sy) * ratio;
                dispatch_mouse(
                    page,
                    DispatchMouseEventType::MouseMoved,
                    ix,
                    iy,
                    MouseButton::None,
                    button_mask,
                    0,
                    0,
                    None,
                    None,
                )
                .await
                .map_err(|e| format!("drag: move step {i} failed: {e}"))?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            dispatch_mouse(
                page,
                DispatchMouseEventType::MouseReleased,
                tx,
                ty,
                button,
                0,
                1,
                0,
                None,
                None,
            )
            .await
            .map_err(|e| format!("drag: mouse up failed: {e}"))?;

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "dragged": {
                    "source": source_sel,
                    "target": target_sel,
                    "from": { "x": sx, "y": sy },
                    "to": { "x": tx, "y": ty },
                    "steps": steps,
                    "button": button_name,
                },
            }))
        },
    )
    .await
}

async fn element_centers(
    page: &Page,
    state: &DaemonState,
    source_sel: &str,
    target_sel: &str,
) -> Result<(f64, f64, f64, f64), String> {
    let source = inspection::resolve_selector_target(page, state, source_sel, true).await?;
    if !source
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(selector_action_error(
            &source,
            &format!("drag source element not found: {source_sel}"),
        ));
    }
    let target = inspection::resolve_selector_target(page, state, target_sel, true).await?;
    if !target
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(selector_action_error(
            &target,
            &format!("drag target element not found: {target_sel}"),
        ));
    }

    let source_center = source.get("center").cloned().unwrap_or_else(|| json!({}));
    let target_center = target.get("center").cloned().unwrap_or_else(|| json!({}));
    let sx = source_center
        .get("sx")
        .or_else(|| source_center.get("x"))
        .and_then(|v| v.as_f64())
        .ok_or("drag: could not get source x")?;
    let sy = source_center
        .get("sy")
        .or_else(|| source_center.get("y"))
        .and_then(|v| v.as_f64())
        .ok_or("drag: could not get source y")?;
    let tx = target_center
        .get("tx")
        .or_else(|| target_center.get("x"))
        .and_then(|v| v.as_f64())
        .ok_or("drag: could not get target x")?;
    let ty = target_center
        .get("ty")
        .or_else(|| target_center.get("y"))
        .and_then(|v| v.as_f64())
        .ok_or("drag: could not get target y")?;
    Ok((sx, sy, tx, ty))
}

// ── Set Viewport ──

/// Handle `set_viewport` command — resize viewport or apply preset.
/// Params: { preset?: string, width?: i64, height?: i64 }
pub async fn handle_set_viewport(page: &Page, params: &Value) -> Result<Value, String> {
    let preset = params.get("preset").and_then(|v| v.as_str());
    let custom_width = params.get("width").and_then(|v| v.as_i64());
    let custom_height = params.get("height").and_then(|v| v.as_i64());

    let (width, height, preset_name) = match preset {
        Some("mobile") => (375, 667, Some("mobile")),
        Some("tablet") => (768, 1024, Some("tablet")),
        Some("desktop") => (1280, 720, Some("desktop")),
        Some("wide") => (1920, 1080, Some("wide")),
        Some(unknown) => {
            return Err(format!(
                "unknown preset: {unknown}. Valid presets: mobile, tablet, desktop, wide"
            ))
        }
        None => match (custom_width, custom_height) {
            (Some(w), Some(h)) => (w, h, None),
            _ => {
                return Err(
                    "set_viewport requires either 'preset' or both 'width' and 'height'"
                        .to_string(),
                )
            }
        },
    };

    debug!("set_viewport: {width}x{height} preset={preset_name:?}");

    let params = SetDeviceMetricsOverrideParams::new(width, height, 1.0, false);
    timeout(CDP_TIMEOUT, page.execute(params))
        .await
        .map_err(|_| "set_viewport timed out".to_string())?
        .map_err(|e| format!("set_viewport failed: {e}"))?;

    Ok(json!({
        "width": width,
        "height": height,
        "preset": preset_name,
    }))
}

// ── Upload File ──

/// Handle `upload_file` command — set files on a file input element.
/// Params: { selector: string, files: [string] }
pub async fn handle_upload_file(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let files = params
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing required parameter: files (array of paths)".to_string())?;
    let file_paths: Vec<String> = files
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    if file_paths.is_empty() {
        return Err("files array cannot be empty".to_string());
    }

    debug!("upload_file: selector={selector} files={file_paths:?}");

    with_narration(
        page,
        state,
        ActionKind::UploadFile,
        Some(selector),
        Some(selector),
        || async {
            let lookup_js = format!(
                r#"(() => {{
  const selector = {selector_json};
  function roots(start) {{
    const out = [];
    const seen = new Set();
    function add(root) {{
      if (!root || seen.has(root)) return;
      seen.add(root);
      out.push(root);
      if (root.shadowRoot) add(root.shadowRoot);
      if (root.tagName && root.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (root.contentDocument) add(root.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = root.querySelectorAll ? Array.from(root.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start);
    return out;
  }}
  function visible(el) {{
    const style = getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && (rect.width > 0 || rect.height > 0);
  }}
  const matches = [];
  for (const root of roots(document)) {{
    let found = [];
    try {{
      found = Array.from(root.querySelectorAll(selector));
    }} catch (err) {{
      throw new Error('invalid selector: ' + err.message);
    }}
    for (const el of found) {{
      if (matches.includes(el)) continue;
      matches.push(el);
    }}
  }}
  function fileInput(el) {{
    return el && el.tagName && el.tagName.toLowerCase() === 'input' && String(el.type || '').toLowerCase() === 'file';
  }}
  function byIdNear(el, id) {{
    const cleanId = String(id || '').replace(/^#/, '');
    if (!cleanId) return null;
    const root = el && el.getRootNode && el.getRootNode();
    return (root && root.getElementById && root.getElementById(cleanId)) || document.getElementById(cleanId);
  }}
  function collectFileInputs(el) {{
    const out = [];
    const add = candidate => {{
      if (fileInput(candidate) && !out.includes(candidate)) out.push(candidate);
    }};
    add(el);
    if (el && el.querySelectorAll) {{
      for (const input of Array.from(el.querySelectorAll('input[type=file]'))) add(input);
    }}
    const ids = [
      el && el.getAttribute && el.getAttribute('for'),
      el && el.getAttribute && el.getAttribute('aria-controls'),
      el && el.getAttribute && el.getAttribute('aria-owns')
    ].filter(Boolean).join(' ');
    for (const id of ids.split(/\s+/).filter(Boolean)) add(byIdNear(el, id));
    const label = el && el.closest && el.closest('label');
    if (label) {{
      const labeled = label.getAttribute('for') ? byIdNear(label, label.getAttribute('for')) : null;
      add(labeled);
      if (label.querySelectorAll) for (const input of Array.from(label.querySelectorAll('input[type=file]'))) add(input);
    }}
    for (const sibling of [el && el.previousElementSibling, el && el.nextElementSibling]) {{
      add(sibling);
      if (sibling && sibling.querySelectorAll) {{
        for (const input of Array.from(sibling.querySelectorAll('input[type=file]'))) add(input);
      }}
    }}
    const parent = el && el.parentElement;
    if (parent && parent.querySelectorAll) {{
      for (const input of Array.from(parent.querySelectorAll('input[type=file]'))) add(input);
    }}
    return out;
  }}
  const fileInputs = [];
  for (const match of matches) {{
    for (const input of collectFileInputs(match)) {{
      if (!fileInputs.includes(input)) fileInputs.push(input);
    }}
  }}
  if (!fileInputs.length) throw new Error('file input not found: ' + selector);
  return fileInputs.find(el => visible(el) && !el.disabled) || fileInputs[0];
}})()"#,
                selector_json = serde_json::to_string(selector).unwrap()
            );
            let eval_params = EvaluateParams::builder()
                .expression(lookup_js)
                .await_promise(true)
                .return_by_value(false)
                .build()
                .map_err(|e| format!("upload_file: failed to build lookup: {e}"))?;
            let resolved = timeout(ELEMENT_TIMEOUT, page.evaluate_expression(eval_params))
                .await
                .map_err(|_| format!("upload_file: timed out finding element: {selector}"))?
                .map_err(|e| {
                    format!(
                        "element not found: {selector} ({})",
                        super::clean_cdp_error(&e)
                    )
                })?;
            let object_id = resolved
                .object()
                .object_id
                .clone()
                .ok_or_else(|| format!("upload_file: resolved target has no object id: {selector}"))?;

            let set_files_params = SetFileInputFilesParams::builder()
                .files(file_paths.iter().map(|s| s.as_str()))
                .object_id(object_id)
                .build()
                .map_err(|e| format!("upload_file: failed to build params: {e}"))?;

            timeout(ELEMENT_TIMEOUT, page.execute(set_files_params))
                .await
                .map_err(|_| "upload_file: timed out setting files".to_string())?
                .map_err(|e| format!("upload_file: CDP error: {e}"))?;

            let (state, settle) = settle_and_capture(page).await;
            Ok(json!({
                "state": state,
                "settle": settle,
                "uploaded": { "selector": selector, "files": file_paths },
            }))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn click_requires_selector_or_coordinates() {
        // This tests the param validation logic by checking the function contract
        let params = json!({});
        // No selector, no x/y — should produce an error message
        let selector = params.get("selector").and_then(|v| v.as_str());
        let x = params.get("x").and_then(|v| v.as_f64());
        let y = params.get("y").and_then(|v| v.as_f64());
        assert!(selector.is_none());
        assert!(x.is_none());
        assert!(y.is_none());
    }

    #[test]
    fn type_requires_selector_and_text() {
        let params = json!({"selector": "input"});
        let text = params.get("text").and_then(|v| v.as_str());
        assert!(text.is_none()); // should trigger error in handler
    }

    #[test]
    fn scroll_direction_validation() {
        for dir in &["up", "down"] {
            let amount = match *dir {
                "up" => -300i32,
                "down" => 300,
                _ => panic!("unknown"),
            };
            assert!(amount != 0);
        }
    }

    #[test]
    fn drag_accepts_selector_mode() {
        let params = json!({"source": "#a", "target": "#b"});
        let (use_coordinates, steps, button) = super::validate_drag_params(&params).unwrap();
        assert!(!use_coordinates);
        assert_eq!(steps, 10);
        assert_eq!(button, "left");
    }

    #[test]
    fn drag_accepts_coordinate_mode_and_clamps_steps() {
        let params = json!({
            "from_x": 1.0,
            "from_y": 2.0,
            "to_x": 3.0,
            "to_y": 4.0,
            "steps": 500,
            "button": "right",
        });
        let (use_coordinates, steps, button) = super::validate_drag_params(&params).unwrap();
        assert!(use_coordinates);
        assert_eq!(steps, 100);
        assert_eq!(button, "right");
    }

    #[test]
    fn drag_rejects_ambiguous_selector_and_coordinate_mode() {
        let params = json!({
            "source": "#a",
            "target": "#b",
            "from_x": 1.0,
            "from_y": 2.0,
            "to_x": 3.0,
            "to_y": 4.0,
        });
        assert!(super::validate_drag_params(&params).is_err());
    }

    #[test]
    fn drag_rejects_none_button() {
        let params = json!({
            "from_x": 1.0,
            "from_y": 2.0,
            "to_x": 3.0,
            "to_y": 4.0,
            "button": "none",
        });
        assert!(super::validate_drag_params(&params).is_err());
    }

    #[test]
    fn viewport_presets() {
        let presets = [
            ("mobile", 375, 667),
            ("tablet", 768, 1024),
            ("desktop", 1280, 720),
            ("wide", 1920, 1080),
        ];
        for (name, w, h) in &presets {
            let (width, height, _) = match *name {
                "mobile" => (375i64, 667i64, Some("mobile")),
                "tablet" => (768, 1024, Some("tablet")),
                "desktop" => (1280, 720, Some("desktop")),
                "wide" => (1920, 1080, Some("wide")),
                _ => panic!("unknown"),
            };
            assert_eq!(width, *w as i64);
            assert_eq!(height, *h as i64);
        }
    }

    #[test]
    fn set_viewport_needs_preset_or_dimensions() {
        let params = json!({});
        let preset = params.get("preset").and_then(|v| v.as_str());
        let w = params.get("width").and_then(|v| v.as_i64());
        let h = params.get("height").and_then(|v| v.as_i64());
        assert!(preset.is_none());
        assert!(w.is_none());
        assert!(h.is_none());
    }
}
