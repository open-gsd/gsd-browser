//! Natural-language instruction planning for generic browser actions.
//!
//! This module intentionally avoids benchmark- or site-specific task names. It
//! translates short user instructions into existing primitive handlers by
//! combining verb classification with live DOM affordances.

mod model;
mod page_model;
mod verification;

use crate::daemon::capture::capture_compact_page_state;
use crate::daemon::handlers;
use crate::daemon::state::DaemonState;
use chromiumoxide::Page;
use model::{InstructionAnalysis, InstructionKind};
use page_model::capture_instruction_page_model;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use verification::verify_action_effect;

const PLAN_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_STEPS: usize = 8;

/// Handle `act_instruction`.
///
/// Params: { instruction: string, dry_run?: bool, scope?: string,
/// min_confidence?: number, max_steps?: number }
pub async fn handle_act_instruction(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: instruction".to_string())?;
    let dry_run = params
        .get("dry_run")
        .or_else(|| params.get("dryRun"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let scope = params.get("scope").and_then(|v| v.as_str());
    let min_confidence = params
        .get("min_confidence")
        .or_else(|| params.get("minConfidence"))
        .and_then(|v| v.as_f64());
    let max_steps = params
        .get("max_steps")
        .or_else(|| params.get("maxSteps"))
        .and_then(|v| v.as_u64())
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_STEPS);

    let analysis = analyze_instruction(instruction);
    if analysis.kind == InstructionKind::Unknown {
        return Err(format!(
            "act_instruction: could not infer a generic browser action from instruction: {instruction}"
        ));
    }

    let before_model = capture_instruction_page_model(page, scope).await?;
    let mut plan = build_plan(page, instruction, &analysis, scope).await?;
    annotate_plan_context(&mut plan, &before_model);
    let step_count = plan_step_count(&plan);
    if step_count > max_steps {
        return Ok(json!({
            "instruction": instruction,
            "analysis": analysis.to_json(),
            "plan": plan,
            "pageModel": before_model,
            "blocked": true,
            "blockReason": "max_steps_exceeded",
            "message": format!("act_instruction planned {step_count} steps, above max_steps={max_steps}; rerun with dry_run, a narrower scope, or a higher max_steps if this is intended"),
            "dryRun": dry_run,
        }));
    }
    if let Some(min_confidence) = min_confidence {
        let confidence = plan_confidence(&plan);
        if confidence < min_confidence {
            return Ok(json!({
                "instruction": instruction,
                "analysis": analysis.to_json(),
                "plan": plan,
                "pageModel": before_model,
                "blocked": true,
                "blockReason": "confidence_below_threshold",
                "message": format!("act_instruction confidence {confidence:.3} is below min_confidence={min_confidence:.3}; inspect the plan or lower the threshold to execute"),
                "dryRun": dry_run,
            }));
        }
    }
    if dry_run {
        return Ok(json!({
            "instruction": instruction,
            "analysis": analysis.to_json(),
            "plan": plan,
            "pageModel": before_model,
            "dryRun": true,
        }));
    }

    let action = plan
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "act_instruction: planner returned no action".to_string())?;

    let result = if action == "sequence" {
        let steps = plan
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "act_instruction: sequence plan has no steps".to_string())?;
        let mut results = Vec::with_capacity(steps.len());
        for step in steps {
            results.push(dispatch_planned_action(page, state, step).await?);
        }
        json!({ "steps": results })
    } else {
        dispatch_planned_action(page, state, &plan).await?
    };
    let after_model = capture_instruction_page_model(page, scope).await?;
    let verification = verify_action_effect(
        instruction,
        &analysis,
        &plan,
        &before_model,
        &after_model,
        &result,
    );

    Ok(json!({
        "instruction": instruction,
        "analysis": analysis.to_json(),
        "plan": plan,
        "result": result,
        "verification": verification,
        "pageModel": {
            "before": before_model,
            "after": after_model,
        },
        "state": capture_compact_page_state(page, false).await,
    }))
}

fn plan_step_count(plan: &Value) -> usize {
    if plan.get("action").and_then(|v| v.as_str()) == Some("sequence") {
        plan.get("steps")
            .and_then(|v| v.as_array())
            .map(|steps| steps.len())
            .unwrap_or(0)
    } else {
        1
    }
}

fn plan_confidence(plan: &Value) -> f64 {
    plan.get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn annotate_plan_context(plan: &mut Value, page_model: &Value) {
    let summary = page_model
        .get("summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let element_count = page_model
        .get("elements")
        .and_then(|value| value.as_array())
        .map(|elements| elements.len())
        .unwrap_or(0);
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "planner".to_string(),
            json!({
                "pageModelVersion": page_model.get("version").and_then(|value| value.as_i64()).unwrap_or(1),
                "pageModelSummary": summary,
                "candidateCount": element_count,
                "strategy": "intent-page-model-candidate-score-v1",
            }),
        );
    }
}

async fn dispatch_planned_action(
    page: &Page,
    state: &DaemonState,
    plan: &Value,
) -> Result<Value, String> {
    let action = plan
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "act_instruction: planner returned no action".to_string())?;
    let params = plan.get("params").cloned().unwrap_or_else(|| json!({}));
    match action {
        "click" => handlers::interaction::handle_click(page, state, &params).await,
        "type" => handlers::interaction::handle_type_text(page, state, &params).await,
        "select_option" => handlers::interaction::handle_select_option(page, state, &params).await,
        "set_checked" => handlers::interaction::handle_set_checked(page, state, &params).await,
        "set_slider" => handle_set_slider(page, &params).await,
        "discover_click" => handle_discover_click(page, &params).await,
        "click_ordered_values" => handle_click_ordered_values(page, &params).await,
        "scroll_element" => handle_scroll_element(page, &params).await,
        "drag" => handlers::interaction::handle_drag(page, state, &params).await,
        "scroll" => handlers::interaction::handle_scroll(page, state, &params).await,
        other => Err(format!(
            "act_instruction: unsupported planned action: {other}"
        )),
    }
}

async fn handle_click_ordered_values(page: &Page, params: &Value) -> Result<Value, String> {
    let order = params
        .get("order")
        .and_then(|v| v.as_str())
        .unwrap_or("ascending");
    let max_clicks = params
        .get("maxClicks")
        .or_else(|| params.get("max_clicks"))
        .and_then(|v| v.as_u64())
        .unwrap_or(12)
        .clamp(1, 50);
    let order_json = serde_json::to_string(order).unwrap();
    let max_clicks_json = serde_json::to_string(&max_clicks).unwrap();
    let js = format!(
        r#"(async () => {{
  const order = {order_json};
  const maxClicks = {max_clicks_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function visible(el) {{
    if (!el || el.hidden || el.disabled) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
      if (parts.length >= 5) break;
    }}
    return parts.join(' > ');
  }}
  function numericValue(el) {{
    const exactText = String(el.textContent || '').trim();
    if (/^-?\d+(?:\.\d+)?$/.test(exactText)) return Number(exactText);
    for (const attr of ['data-index', 'data-value', 'aria-valuenow', 'aria-posinset', 'value']) {{
      const raw = el.getAttribute(attr);
      if (raw != null && /^-?\d+(?:\.\d+)?$/.test(String(raw).trim())) return Number(raw);
    }}
    return null;
  }}
  function clickTarget(el) {{
    const rect = el.getBoundingClientRect();
    const x = rect.left + Math.max(1, rect.width / 2);
    const y = rect.top + Math.max(1, rect.height / 2);
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: x, clientY: y }};
    el.dispatchEvent(new MouseEvent('mouseover', init));
    el.dispatchEvent(new MouseEvent('mousedown', init));
    el.dispatchEvent(new MouseEvent('mouseup', init));
    el.dispatchEvent(new MouseEvent('click', init));
  }}
  function candidates(clickedKeys) {{
    const query = [
      'button', 'a', '[role=button]', '[role=link]', '[onclick]', '[tabindex]',
      'svg text', 'svg [data-index]', '[data-index]', '[data-value]', '[aria-posinset]'
    ].join(',');
    const out = [];
    for (const el of Array.from(document.querySelectorAll(query))) {{
      if (!visible(el)) continue;
      const value = numericValue(el);
      if (value == null || !Number.isFinite(value)) continue;
      const rect = el.getBoundingClientRect();
      const key = [selector(el), value, Math.round(rect.x), Math.round(rect.y), Math.round(rect.width), Math.round(rect.height)].join('|');
      if (clickedKeys.has(key)) continue;
      out.push({{ el, value, key, selector: selector(el), text: String(el.textContent || '').trim(), bounds: {{ x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }} }});
    }}
    out.sort((a, b) => order === 'descending' ? b.value - a.value : a.value - b.value);
    return out;
  }}
  const clicked = [];
  const clickedKeys = new Set();
  for (let index = 0; index < maxClicks; index++) {{
    const next = candidates(clickedKeys)[0];
    if (!next) break;
    clickedKeys.add(next.key);
    clickTarget(next.el);
    clicked.push({{ selector: next.selector, value: next.value, text: next.text, bounds: next.bounds }});
    await delay(90);
  }}
  if (!clicked.length) return {{ ok: false, error: 'click_ordered_values found no visible numeric click targets', order, maxClicks }};
  return {{ ok: true, order, clicked, count: clicked.length }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "click_ordered_values timed out".to_string())?
        .map_err(|e| {
            format!(
                "click_ordered_values failed: {}",
                super::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "orderedValues": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("click_ordered_values failed")
            .to_string())
    }
}

async fn handle_set_slider(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let value = params
        .get("value")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing required parameter: value".to_string())?;
    let selector_json = serde_json::to_string(selector).unwrap();
    let value_json = serde_json::to_string(&value).unwrap();
    let js = format!(
        r#"(() => {{
  const selector = {selector_json};
  const desired = {value_json};
  const el = document.querySelector(selector);
  if (!el) return {{ ok: false, error: 'slider not found: ' + selector }};
  if (el.matches('input[type=range]')) {{
    el.value = String(desired);
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return {{ ok: true, selector, value: Number(el.value), mode: 'native-range' }};
  }}
  if (window.jQuery) {{
    try {{
      const jq = window.jQuery(el);
      if (jq && jq.data && (jq.data('ui-slider') || jq.data('slider'))) {{
        jq.slider('value', desired);
        return {{ ok: true, selector, value: Number(jq.slider('value')), mode: 'jquery-ui' }};
      }}
    }} catch (error) {{
      return {{ ok: false, error: String(error && error.message || error) }};
    }}
  }}
  if (el.getAttribute('role') === 'slider') {{
    el.setAttribute('aria-valuenow', String(desired));
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return {{ ok: true, selector, value: desired, mode: 'aria' }};
  }}
  return {{ ok: false, error: 'matched element is not a supported slider' }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "set_slider timed out".to_string())?
        .map_err(|e| format!("set_slider failed: {}", super::clean_cdp_error(&e)))?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "slider": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("set_slider failed")
            .to_string())
    }
}

async fn handle_scroll_element(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let selector_json = serde_json::to_string(selector).unwrap();
    let js = format!(
        r#"(() => {{
  const el = document.querySelector({selector_json});
  if (!el) return {{ ok: false, error: 'scroll_element target not found' }};
  el.scrollTop = el.scrollHeight;
  el.dispatchEvent(new Event('scroll', {{ bubbles: true }}));
  return {{ ok: true, selector: {selector_json}, scrollTop: el.scrollTop, scrollHeight: el.scrollHeight }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "scroll_element timed out".to_string())?
        .map_err(|e| format!("scroll_element failed: {}", super::clean_cdp_error(&e)))?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "scrollElement": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("scroll_element failed")
            .to_string())
    }
}

async fn handle_discover_click(page: &Page, params: &Value) -> Result<Value, String> {
    let target = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: target".to_string())?;
    let target_json = serde_json::to_string(target).unwrap();
    let js = format!(
        r#"(async () => {{
  const target = {target_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function visible(el) {{
    if (!el || el.hidden || el.disabled) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function textOf(el) {{
    return [el.textContent || '', el.value || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
      if (parts.length >= 5) break;
    }}
    return parts.join(' > ');
  }}
  function targetClickables() {{
    return Array.from(document.querySelectorAll('a, button, [role=link], [role=button], .alink, [onclick], [tabindex]'))
      .filter(visible)
      .filter(el => norm(textOf(el)).split(' ').includes(norm(target)) || norm(textOf(el)).includes(norm(target)));
  }}
  function discoveryControls() {{
    return Array.from(document.querySelectorAll(
      '[aria-expanded=false], [role=tab], [role=button], button, summary, .ui-tabs-anchor, .ui-accordion-header, [data-toggle], [data-testid*=tab], [data-testid*=section]'
    )).filter(visible);
  }}
  const tried = [];
  for (let pass = 0; pass < 2; pass++) {{
    const direct = targetClickables()[0];
    if (direct) {{
      direct.click();
      await delay(80);
      return {{ ok: true, target, clicked: selector(direct), mode: pass === 0 ? 'direct' : 'after-discovery', tried }};
    }}
    for (const control of discoveryControls()) {{
      const key = selector(control);
      if (tried.includes(key)) continue;
      tried.push(key);
      control.click();
      await delay(120);
      const found = targetClickables()[0];
      if (found) {{
        found.click();
        await delay(80);
        return {{ ok: true, target, clicked: selector(found), discoveredBy: key, mode: 'discover-click', tried }};
      }}
    }}
  }}
  return {{ ok: false, error: 'discover_click could not reveal clickable target: ' + target, target, tried }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "discover_click timed out".to_string())?
        .map_err(|e| format!("discover_click failed: {}", super::clean_cdp_error(&e)))?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "discoverClick": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("discover_click failed")
            .to_string())
    }
}

fn analyze_instruction(instruction: &str) -> InstructionAnalysis {
    let lower = instruction.to_lowercase();
    let quoted = quoted_strings(instruction);
    let mut value = quoted.first().cloned();
    let mut target_hint = None;
    let mut secondary_hint = quoted.get(1).cloned();
    let mut checked = None;
    let mut direction = None;

    let kind = if starts_with_any(&lower, &["how many ", "count "]) || lower.contains(" how many ")
    {
        target_hint = trailing_hint(instruction, &["how many", "count"]);
        InstructionKind::Count
    } else if contains_any(&lower, &["drag ", "dragged ", "move "]) && lower.contains(" to ") {
        target_hint = quoted.first().cloned();
        if target_hint.is_none() || secondary_hint.is_none() {
            let (first, second) = split_around_to(instruction);
            target_hint = target_hint.or(first);
            secondary_hint = secondary_hint.or(second);
        }
        InstructionKind::Drag
    } else if contains_any(&lower, &["uncheck", "untick", "deselect "]) {
        checked = Some(false);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["uncheck", "untick", "deselect"]));
        InstructionKind::SetChecked
    } else if starts_with_any(&lower, &["check ", "tick "]) {
        checked = Some(true);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["check", "tick"]));
        InstructionKind::SetChecked
    } else if has_value_control_hint(&lower)
        && (starts_with_any(
            &lower,
            &[
                "select ", "choose ", "pick ", "set ", "move ", "use ", "enter ", "input ",
            ],
        ) || lower.contains(" with ")
            || lower.contains(" using ")
            || lower.contains(" on "))
    {
        let (mut parsed_value, mut parsed_target) = value_target_from_markers(
            instruction,
            &[
                "select", "choose", "pick", "set", "move", "use", "enter", "input",
            ],
            &[
                " with ", " using ", " on ", " into ", " in ", " to ", " as ",
            ],
        );
        let parsed_value_is_control = parsed_value
            .as_deref()
            .map(|text| has_value_control_hint(&text.to_lowercase()))
            .unwrap_or(false);
        if parsed_value_is_control && parsed_target.is_some() {
            std::mem::swap(&mut parsed_value, &mut parsed_target);
        }
        let numeric = first_number(instruction);
        let fallback_control_hint = value_control_hint(instruction);
        let numeric_control = parsed_target
            .as_deref()
            .or(fallback_control_hint.as_deref())
            .map(|text| {
                let text = text.to_lowercase();
                contains_any(
                    &text,
                    &[
                        "slider",
                        "range",
                        "spinner",
                        "spinbutton",
                        "stepper",
                        "number",
                        "numeric",
                    ],
                )
            })
            .unwrap_or(false);
        let parsed_value_has_digit = parsed_value
            .as_deref()
            .map(|text| text.chars().any(|ch| ch.is_ascii_digit()))
            .unwrap_or(false);
        let parsed_value = if (numeric_control || !parsed_value_has_digit) && numeric.is_some() {
            numeric
        } else {
            parsed_value
        };
        value = value.or(parsed_value);
        target_hint = parsed_target.or(fallback_control_hint);
        InstructionKind::Fill
    } else if starts_with_any(
        &lower,
        &["select ", "choose ", "pick ", "set dropdown", "set option"],
    ) || lower.contains(" dropdown")
        || lower.contains(" option")
    {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &["select", "choose", "pick"],
            &[" from ", " in ", " for "],
        );
        value = value.or(parsed_value);
        target_hint = parsed_target;
        InstructionKind::SelectOption
    } else if starts_with_any(
        &lower,
        &["type ", "enter ", "fill ", "input ", "write ", "search "],
    ) || contains_any(&lower, &[" into ", " in the field", " in field"])
        || (lower.contains("enter ") && !quoted.is_empty())
    {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &["type", "enter", "fill", "input", "write", "search"],
            &[" into ", " in ", " to "],
        );
        value = value.or(parsed_value);
        target_hint = field_hint(instruction).or(parsed_target);
        InstructionKind::Fill
    } else if lower.contains("scroll") {
        direction = if lower.contains("up") {
            Some("up".to_string())
        } else {
            Some("down".to_string())
        };
        InstructionKind::Scroll
    } else if starts_with_any(
        &lower,
        &[
            "click ", "press ", "tap ", "open ", "submit", "continue", "confirm", "save", "done",
            "next", "buy ", "expand ", "switch ",
        ],
    ) || lower.contains(" click ")
        || lower.contains(" press ")
        || lower.contains(" tap ")
    {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["click", "press", "tap", "open", "buy"]));
        InstructionKind::Click
    } else {
        InstructionKind::Unknown
    };

    InstructionAnalysis {
        kind,
        value: clean_hint(value),
        target_hint: clean_hint(target_hint),
        secondary_hint: clean_hint(secondary_hint),
        checked,
        direction,
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn starts_with_any(haystack: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| haystack.starts_with(prefix))
}

fn has_value_control_hint(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "slider",
            "range",
            "spinner",
            "spinbutton",
            "stepper",
            "number",
            "numeric",
            "date",
            "time",
            "month",
            "week",
            "color",
            "colour",
            "datetime",
        ],
    )
}

fn value_control_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for word in [
        "slider",
        "range",
        "spinner",
        "spinbutton",
        "stepper",
        "number",
        "numeric",
        "date",
        "time",
        "month",
        "week",
        "color",
        "colour",
        "datetime",
    ] {
        if lower.contains(word) {
            return Some(word.to_string());
        }
    }
    None
}

fn first_number(input: &str) -> Option<String> {
    let mut current = String::new();
    let mut started = false;
    let mut seen_digit = false;
    for ch in input.chars() {
        if ch.is_ascii_digit() || (ch == '-' && !started) || (ch == '.' && started) {
            current.push(ch);
            started = true;
            if ch.is_ascii_digit() {
                seen_digit = true;
            }
        } else if started {
            break;
        }
    }
    if seen_digit {
        Some(current)
    } else {
        None
    }
}

fn quoted_strings(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut active = None;
    let mut current = String::new();
    for ch in input.chars() {
        if ch == '"' || ch == '\'' {
            match active {
                Some(q) if q == ch => {
                    if !current.trim().is_empty() {
                        out.push(current.trim().to_string());
                    }
                    current.clear();
                    active = None;
                }
                None => active = Some(ch),
                _ => current.push(ch),
            }
        } else if active.is_some() {
            current.push(ch);
        }
    }
    out
}

fn trailing_hint(input: &str, verbs: &[&str]) -> Option<String> {
    let lower = input.to_lowercase();
    for verb in verbs {
        if let Some(index) = lower.find(verb) {
            let start = index + verb.len();
            let hint = input[start..]
                .trim()
                .trim_start_matches(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
                .trim();
            if !hint.is_empty() {
                return Some(hint.to_string());
            }
        }
    }
    None
}

fn split_around_to(input: &str) -> (Option<String>, Option<String>) {
    let lower = input.to_lowercase();
    if let Some(index) = lower.find(" to ") {
        let before = input[..index]
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        let after = input[index + 4..].trim().to_string();
        (clean_hint(Some(before)), clean_hint(Some(after)))
    } else {
        (None, None)
    }
}

fn value_target_from_markers(
    input: &str,
    verbs: &[&str],
    markers: &[&str],
) -> (Option<String>, Option<String>) {
    let lower = input.to_lowercase();
    let start = verbs
        .iter()
        .filter_map(|verb| {
            let index = lower.find(verb)?;
            Some(index + verb.len())
        })
        .min()
        .unwrap_or(0);
    let tail = input[start..]
        .trim()
        .trim_start_matches(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
        .trim();
    if tail.is_empty() {
        return (None, None);
    }

    let tail_lower = tail.to_lowercase();
    for marker in markers {
        if let Some(index) = tail_lower.find(marker) {
            let raw_value = tail[..index].trim();
            let raw_target = tail[index + marker.len()..].trim();
            return (
                clean_hint(Some(raw_value.to_string())),
                clean_hint(Some(raw_target.to_string())),
            );
        }
    }

    (clean_hint(Some(tail.to_string())), None)
}

fn field_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [" into ", " in ", " to "] {
        if let Some(index) = lower.find(marker) {
            let hint = input[index + marker.len()..].trim();
            if !hint.is_empty() {
                return Some(hint.to_string());
            }
        }
    }
    None
}

fn clean_hint(value: Option<String>) -> Option<String> {
    value
        .map(|text| {
            strip_follow_up_suffix(text.trim())
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\'' | '.' | ',' | ':' | ';' | '(' | ')' | '[' | ']'
                    )
                })
                .trim()
                .to_string()
        })
        .filter(|text| !text.is_empty())
}

fn strip_follow_up_suffix(text: &str) -> &str {
    let lower = text.to_lowercase();
    for marker in [
        " and click ",
        " and press ",
        " and tap ",
        " and hit ",
        " then click ",
        " then press ",
        " then tap ",
        " then hit ",
    ] {
        if let Some(index) = lower.find(marker) {
            return &text[..index];
        }
    }
    text
}

async fn build_plan(
    page: &Page,
    instruction: &str,
    analysis: &InstructionAnalysis,
    scope: Option<&str>,
) -> Result<Value, String> {
    if analysis.kind == InstructionKind::Scroll {
        return Ok(json!({
            "action": "scroll",
            "params": {
                "direction": analysis.direction.as_deref().unwrap_or("down"),
                "amount": 500,
            },
            "confidence": 0.9,
            "reason": "instruction asks to scroll",
        }));
    }

    let js = planner_js(instruction, analysis, scope);
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "act_instruction: planner timed out".to_string())?
        .map_err(|e| {
            format!(
                "act_instruction: planner failed: {}",
                super::clean_cdp_error(&e)
            )
        })?;
    let plan = result.value().cloned().unwrap_or_else(|| json!({}));
    if plan
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        Ok(plan)
    } else {
        Err(plan
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("act_instruction: no actionable plan found")
            .to_string())
    }
}

fn planner_js(instruction: &str, analysis: &InstructionAnalysis, scope: Option<&str>) -> String {
    let instruction_json = serde_json::to_string(instruction).unwrap();
    let kind_json = serde_json::to_string(analysis.kind.as_str()).unwrap();
    let value_json = serde_json::to_string(&analysis.value).unwrap();
    let target_json = serde_json::to_string(&analysis.target_hint).unwrap();
    let secondary_json = serde_json::to_string(&analysis.secondary_hint).unwrap();
    let checked_json = serde_json::to_string(&analysis.checked).unwrap();
    let scope_json = serde_json::to_string(&scope).unwrap();

    format!(
        r#"(() => {{
  const instruction = {instruction_json};
  const kind = {kind_json};
  const wantedValue = {value_json};
  const targetHint = {target_json};
  const secondaryHint = {secondary_json};
  const checked = {checked_json};
  const scopeSelector = {scope_json};
  const root = scopeSelector ? document.querySelector(scopeSelector) : document;
  if (!root) return {{ ok: false, error: 'act_instruction: scope not found: ' + scopeSelector }};

  function visible(el) {{
    if (!el || el.hidden || el.disabled) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function textOf(el) {{
    const labels = [];
    if (el.id) {{
      for (const label of document.querySelectorAll('label[for=' + JSON.stringify(el.id) + ']')) {{
        labels.push(label.textContent || '');
      }}
    }}
    const wrappingLabel = el.closest('label');
    if (wrappingLabel) labels.push(wrappingLabel.textContent || '');
    return [
      el.textContent || '', el.value || '', el.getAttribute('name') || '', el.placeholder || '',
      el.getAttribute('aria-label') || '', el.getAttribute('title') || '',
      el.getAttribute('role') || '', el.getAttribute('data-testid') || '',
      labels.join(' ')
    ].join(' ').trim();
  }}
  function tokens(text) {{
    return String(text || '').toLowerCase().split(/[^a-z0-9]+/).filter(t => t.length > 1);
  }}
  function tokenScore(hint, text) {{
    const ht = tokens(hint);
    if (!ht.length) return 0;
    const tt = new Set(tokens(text));
    let hits = 0;
    for (const token of ht) if (tt.has(token)) hits++;
    return hits / ht.length;
  }}
  function normalized(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function exactPhraseScore(hint, text) {{
    const h = normalized(hint);
    const t = normalized(text);
    if (!h || !t) return 0;
    const compactText = t.replace(/\s+/g, '');
    for (const variant of [h, h.replace(/^(?:on|the|a|an)\s+/, '')]) {{
      if (!variant) continue;
      if (t === variant) return 1;
      if (t.includes(variant)) return 0.9;
      if (compactText.includes(variant.replace(/\s+/g, ''))) return 0.85;
    }}
    return 0;
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    const nameAttr = el.getAttribute('name');
    if (nameAttr) {{
      const sel = el.tagName.toLowerCase() + '[name=' + JSON.stringify(nameAttr) + ']';
      if (document.querySelectorAll(sel).length === 1) return sel;
    }}
    const type = el.getAttribute('type');
    if (type) {{
      const sel = el.tagName.toLowerCase() + '[type=' + JSON.stringify(type) + ']';
      if (document.querySelectorAll(sel).length === 1) return sel;
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement) {{
      let part = node.tagName.toLowerCase();
      if (node.id) {{
        part += '#' + CSS.escape(node.id);
        parts.unshift(part);
        break;
      }}
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
      if (parts.length >= 5) break;
    }}
    return parts.join(' > ');
  }}
  function candidate(el) {{
    const rect = el.getBoundingClientRect();
    return {{
      selector: selector(el),
      tag: el.tagName.toLowerCase(),
      type: (el.getAttribute('type') || '').toLowerCase() || null,
      role: (el.getAttribute('role') || '').toLowerCase() || null,
      text: textOf(el).slice(0, 160),
      bounds: {{ x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }}
    }};
  }}
  function best(elements, score) {{
    const scored = [];
    for (const el of elements) {{
      if (!visible(el)) continue;
      const s = score(el);
      if (s > 0) scored.push({{ el, score: s }});
    }}
    scored.sort((a, b) => b.score - a.score);
    return scored;
  }}
  function stripFollowUp(text) {{
    return String(text || '')
      .replace(/\s+(?:and|then)\s+(?:click|press|tap|hit)\s+(?:the\s+)?[^,.]+\.?$/i, '')
      .trim();
  }}
  function followUpClickHint() {{
    const match = instruction.match(/\b(?:and|then)\s+(?:click|press|tap|hit)\s+(?:the\s+)?([^,.]+)\.?$/i);
    if (!match) return null;
    return match[1].replace(/\b(button|link|control)\b/ig, '').trim() || match[1].trim();
  }}
  function transformedValue(text) {{
    if (text == null) return text;
    if (/all\s+upper\s+case|uppercase|upper-case/i.test(instruction)) return String(text).toUpperCase();
    if (/all\s+lower\s+case|lowercase|lower-case/i.test(instruction)) return String(text).toLowerCase();
    return text;
  }}
  function requestedItems(text) {{
    const cleaned = stripFollowUp(text);
    if (!cleaned || /^nothing$/i.test(cleaned)) return [];
    return cleaned
      .replace(/^(?:words?\s+)?(?:similar|related|synonyms?)\s+to\s+/i, '')
      .split(/\s*,\s*|\s+\band\b\s+/i)
      .map(item => item.trim().replace(/^(?:words?\s+)?(?:similar|related|synonyms?)\s+to\s+/i, ''))
      .filter(Boolean);
  }}
  const semanticGroups = [
    ['small', 'mini', 'tiny', 'little'],
    ['large', 'big', 'huge'],
    ['home', 'homes', 'house', 'houses'],
    ['keep', 'retain', 'hold'],
    ['start', 'begin', 'initiate'],
    ['real', 'genuine', 'authentic'],
    ['red', 'scarlet', 'crimson'],
    ['calm', 'peaceful', 'quiet'],
    ['car', 'cars', 'automobile', 'automobiles', 'vehicle', 'vehicles'],
    ['pig', 'hog', 'swine'],
    ['like', 'similar', 'alike'],
    ['hide', 'conceal', 'cover'],
    ['cut', 'scar', 'slice'],
    ['water', 'aqua'],
  ];
  function semanticTokens(word) {{
    const base = tokens(word);
    const out = new Set(base);
    for (const token of base) {{
      const singular = token.endsWith('s') ? token.slice(0, -1) : token;
      out.add(singular);
      for (const group of semanticGroups) {{
        if (group.includes(token) || group.includes(singular)) {{
          for (const item of group) out.add(item);
        }}
      }}
    }}
    return out;
  }}
  function semanticScore(hint, text) {{
    const ht = semanticTokens(hint);
    if (!ht.size) return 0;
    const tt = semanticTokens(text);
    let hits = 0;
    for (const token of ht) if (tt.has(token)) hits++;
    if (hits) return hits / ht.size;
    const plainHint = String(hint || '').toLowerCase();
    const plainText = String(text || '').toLowerCase();
    return plainText.includes(plainHint) ? 0.75 : 0;
  }}
  function relationScore(el, anchor) {{
    if (!anchor) return 0;
    const form = anchor.closest('form');
    if (form && form.contains(el)) return 0.5;
    const anchorParent = anchor.parentElement;
    if (anchorParent && anchorParent.contains(el)) return 0.25;
    const position = anchor.compareDocumentPosition(el);
    if (position & Node.DOCUMENT_POSITION_FOLLOWING) return 0.15;
    if (position & Node.DOCUMENT_POSITION_PRECEDING) return -0.05;
    return 0;
  }}
  function clickStepForHint(hint, anchor = null) {{
    if (!hint) return null;
    const ranked = best(interactive, el => {{
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute('type') || '').toLowerCase();
      let score = Math.max(tokenScore(hint, textOf(el)), exactPhraseScore(hint, textOf(el)));
      if (/submit|continue|confirm|save|done|next|ok/i.test(hint)) {{
        if (type === 'submit') score += 0.5;
        if (/submit|continue|confirm|save|done|next|ok/i.test(textOf(el))) score += 0.4;
      }}
      if (tag === 'button' || tag === 'a' || type === 'submit' || (el.getAttribute('role') || '').toLowerCase() === 'button') score += 0.05;
      score += relationScore(el, anchor);
      return score;
    }});
    if (!ranked.length) return null;
    const chosen = ranked[0];
    return {{
      action: 'click',
      params: {{ selector: selector(chosen.el) }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched follow-up clickable element by instruction text',
      candidate: candidate(chosen.el)
    }};
  }}
  function withFollowUp(primary, anchor = null) {{
    const followHint = followUpClickHint();
    const follow = clickStepForHint(followHint, anchor);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.5, follow.confidence || 0.5),
      reason: 'planned primary action plus follow-up click from compound instruction'
    }};
  }}
  function planConfidence(plan) {{
    if (!plan) return 0;
    if (typeof plan.confidence === 'number') return plan.confidence;
    if (Array.isArray(plan.steps) && plan.steps.length) {{
      return Math.min(1, plan.steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / plan.steps.length);
    }}
    return 0.5;
  }}
  function normalizePlan(plan, capability, priority) {{
    if (!plan || !plan.action) return null;
    plan.ok = true;
    plan.confidence = planConfidence(plan);
    plan.capability = {{
      name: capability,
      priority,
      strategy: 'candidate-generator'
    }};
    return plan;
  }}
  function chooseCapabilityPlan(capabilities) {{
    const candidates = [];
    for (const capability of capabilities) {{
      let plan = null;
      try {{
        plan = capability.build();
      }} catch (error) {{
        continue;
      }}
      plan = normalizePlan(plan, capability.name, capability.priority || 0);
      if (plan) candidates.push(plan);
    }}
    if (!candidates.length) return null;
    candidates.sort((a, b) => {{
      const ap = (a.capability && a.capability.priority) || 0;
      const bp = (b.capability && b.capability.priority) || 0;
      if (bp !== ap) return bp - ap;
      return planConfidence(b) - planConfidence(a);
    }});
    const chosen = candidates[0];
    chosen.alternates = candidates.slice(1, 4).map(plan => ({{
      action: plan.action,
      confidence: planConfidence(plan),
      reason: plan.reason || null,
      capability: plan.capability || null
    }}));
    return chosen;
  }}
  function all(selectorText) {{
    const results = Array.from(root.querySelectorAll(selectorText));
    if (root !== document && root.matches && root.matches(selectorText)) {{
      results.unshift(root);
    }}
    return results;
  }}
  const interactive = all(
    'button, a, input, textarea, select, [role=button], [role=link], [role=option], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=tab], [role=slider], [role=spinbutton], [role=textbox], [role=searchbox], [role=combobox], [role=switch], [onclick], [tabindex], [contenteditable=true], [contenteditable=""]'
  );
  const textInputTypes = new Set(['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number', 'date', 'time', 'month', 'week', 'datetime-local', 'color', 'range']);
  function roleOf(el) {{
    return (el.getAttribute('role') || '').toLowerCase();
  }}
  function typeOf(el) {{
    return (el.getAttribute('type') || '').toLowerCase();
  }}
  function isEditableElement(el) {{
    const editable = el.getAttribute('contenteditable');
    return el.isContentEditable || (editable !== null && editable.toLowerCase() !== 'false');
  }}
  function isFillableField(el) {{
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    return tag === 'textarea' || isEditableElement(el) ||
      (tag === 'input' && textInputTypes.has(type)) ||
      ['textbox', 'searchbox', 'spinbutton', 'slider'].includes(role);
  }}
  function isSliderControl(el) {{
    if (typeOf(el) === 'range' || roleOf(el) === 'slider') return true;
    try {{
      if (!window.jQuery) return false;
      const jq = window.jQuery(el);
      return !!(jq && jq.data && (jq.data('ui-slider') || jq.data('slider')));
    }} catch (_) {{
      return false;
    }}
  }}
  function controlTypeScore(hint, el) {{
    const haystack = normalized([hint || '', instruction].join(' '));
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    let score = 0;
    if (/\b(slider|range)\b/.test(haystack) && (type === 'range' || role === 'slider' || /\bslider\b/i.test(textOf(el)))) score += 0.7;
    if (/\b(spinner|spinbutton|stepper|numeric|number)\b/.test(haystack) && (type === 'number' || role === 'spinbutton')) score += 0.7;
    if (/\bdate\b/.test(haystack) && type === 'date') score += 0.7;
    if (/\btime\b/.test(haystack) && type === 'time') score += 0.7;
    if (/\bmonth\b/.test(haystack) && type === 'month') score += 0.7;
    if (/\bweek\b/.test(haystack) && type === 'week') score += 0.7;
    if (/\b(datetime|date time|date-time)\b/.test(haystack) && type === 'datetime-local') score += 0.7;
    if (/\b(colou?r)\b/.test(haystack) && type === 'color') score += 0.7;
    if (/\bsearch\b/.test(haystack) && (type === 'search' || role === 'searchbox')) score += 0.4;
    if (/\b(text|input|field|box)\b/.test(haystack) && (tag === 'input' || tag === 'textarea' || role === 'textbox')) score += 0.2;
    return score;
  }}
  function ordinalIndex(text) {{
    const lower = String(text || '').toLowerCase();
    const named = [
      ['first', 0], ['1st', 0],
      ['second', 1], ['2nd', 1],
      ['third', 2], ['3rd', 2],
      ['fourth', 3], ['4th', 3],
      ['fifth', 4], ['5th', 4],
    ];
    for (const [word, index] of named) if (lower.includes(word)) return index;
    const match = lower.match(/\b(\d+)(?:st|nd|rd|th)?\s+checkbox\b/);
    if (match) return Math.max(0, Number(match[1]) - 1);
    return null;
  }}
  function cleanClickHint(text) {{
    return String(text || '')
      .replace(/\b(?:button|link|control|item|element|labelled|labeled|called|named)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function orderedClickHints() {{
    const hints = [];
    const pattern = /\b(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)(?=\s*(?:,?\s*(?:then|and)\s+(?:click|press|tap|hit)\b|[.;]|$))/gi;
    let match;
    while ((match = pattern.exec(instruction)) !== null) {{
      const hint = cleanClickHint(match[1]);
      if (hint) hints.push(hint);
    }}
    return hints;
  }}
  function orderedClickSequencePlan() {{
    const hints = orderedClickHints();
    if (hints.length < 2) return null;
    const used = new Set();
    const steps = [];
    let anchor = null;
    for (const hint of hints) {{
      const ranked = best(interactive.filter(el => !used.has(selector(el))), el => {{
        const tag = el.tagName.toLowerCase();
        const type = typeOf(el);
        let score = Math.max(tokenScore(hint, textOf(el)), exactPhraseScore(hint, textOf(el)), semanticScore(hint, textOf(el)));
        if (tag === 'button' || tag === 'a' || type === 'submit' || roleOf(el) === 'button') score += 0.08;
        score += relationScore(el, anchor);
        return score;
      }});
      if (!ranked.length) return null;
      const chosen = ranked[0];
      const key = selector(chosen.el);
      used.add(key);
      anchor = chosen.el;
      steps.push({{
        action: 'click',
        params: {{ selector: key }},
        confidence: Math.min(1, chosen.score),
        reason: 'matched ordered clickable target from instruction clause',
        candidate: candidate(chosen.el)
      }});
    }}
    return {{
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned ordered click sequence from repeated instruction clauses'
    }};
  }}
  function numericClickTargetCount() {{
    const query = [
      'button', 'a', '[role=button]', '[role=link]', '[onclick]', '[tabindex]',
      'svg text', 'svg [data-index]', '[data-index]', '[data-value]', '[aria-posinset]'
    ].join(',');
    let count = 0;
    for (const el of all(query)) {{
      if (!visible(el)) continue;
      const text = String(el.textContent || '').trim();
      const numericText = /^-?\d+(?:\.\d+)?$/.test(text);
      const numericAttr = ['data-index', 'data-value', 'aria-valuenow', 'aria-posinset', 'value'].some(attr => {{
        const raw = el.getAttribute(attr);
        return raw != null && /^-?\d+(?:\.\d+)?$/.test(String(raw).trim());
      }});
      if (numericText || numericAttr) count++;
    }}
    return count;
  }}
  function orderedValueClickPlan() {{
    if (!/\b(click|press|tap|hit)\b/i.test(instruction)) return null;
    if (!/\b(numbers?|values?|items?)\b/i.test(instruction)) return null;
    const explicitOrder = /\bdescending|decreasing|reverse\b/i.test(instruction) ? 'descending' :
      /\bascending|increasing|smallest\s+to\s+largest|lowest\s+to\s+highest|in\s+order\b/i.test(instruction) ? 'ascending' : null;
    if (!explicitOrder) return null;
    const count = numericClickTargetCount();
    if (count < 2) return null;
    return {{
      action: 'click_ordered_values',
      params: {{ order: explicitOrder, maxClicks: Math.min(50, Math.max(2, count + 4)) }},
      confidence: 0.82,
      reason: 'matched ordered numeric click instruction and visible numeric targets',
      evidence: {{ numericTargetCount: count, order: explicitOrder }}
    }};
  }}
  function sliderPlan() {{
    const valueMatch = instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(-?\d+(?:\.\d+)?)\s+(?:with|on|using|in|into)\s+(?:the\s+)?(?:slider|range)\b/i);
    if (!valueMatch) return null;
    const desired = Number(valueMatch[1]);
    const sliders = all('input[type=range], [role=slider], .ui-slider, [class*=slider]').filter(el => visible(el) && isSliderControl(el));
    for (const el of sliders) {{
      const minAttr = el.getAttribute('min') ?? el.getAttribute('aria-valuemin');
      const maxAttr = el.getAttribute('max') ?? el.getAttribute('aria-valuemax');
      let min = minAttr == null ? Number.NaN : Number(minAttr);
      let max = maxAttr == null ? Number.NaN : Number(maxAttr);
      let orientation = (el.getAttribute('aria-orientation') || '').toLowerCase();
      try {{
        if ((!Number.isFinite(min) || !Number.isFinite(max)) && window.jQuery) {{
          const jq = window.jQuery(el);
          if (jq && jq.data && (jq.data('ui-slider') || jq.data('slider'))) {{
            min = Number(jq.slider('option', 'min'));
            max = Number(jq.slider('option', 'max'));
            orientation = String(jq.slider('option', 'orientation') || orientation).toLowerCase();
          }}
        }}
      }} catch (_) {{}}
      if (!Number.isFinite(min) || !Number.isFinite(max) || max === min) continue;
      const rect = el.getBoundingClientRect();
      const ratio = Math.max(0, Math.min(1, (desired - min) / (max - min)));
      const vertical = orientation === 'vertical';
      return {{
        action: 'set_slider',
        params: {{
          selector: selector(el),
          value: desired,
          x: vertical ? rect.left + rect.width / 2 : rect.left + rect.width * ratio,
          y: vertical ? rect.bottom - rect.height * ratio : rect.top + rect.height / 2
        }},
        confidence: 0.85,
        reason: 'matched slider value from instruction and slider range metadata',
        candidate: candidate(el)
	      }};
	    }}
	    return null;
	  }}
	  function listedNumbers() {{
    const bracket = instruction.match(/\[([^\]]+)\]/);
    const source = bracket ? bracket[1] : '';
    if (!source) return [];
    return Array.from(source.matchAll(/-?\d+(?:\.\d+)?/g)).map(match => Number(match[0])).filter(Number.isFinite);
  }}
  function multiSliderPlan() {{
    const values = listedNumbers();
    if (values.length < 2 || !/\bsliders?\b/i.test(instruction)) return null;
    const sliders = all('input[type=range], [role=slider], .ui-slider, [class*=slider]').filter(el => visible(el) && isSliderControl(el));
    if (sliders.length < values.length) return null;
    const steps = values.map((value, index) => ({{
      action: 'set_slider',
      params: {{ selector: selector(sliders[index]), value }},
      confidence: 0.82,
      reason: 'matched listed value to repeated slider by document order',
      candidate: candidate(sliders[index])
    }}));
    const follow = clickStepForHint(followUpClickHint() || (/submit/i.test(instruction) ? 'submit' : null), sliders[values.length - 1]);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned repeated slider value sequence from listed instruction values'
    }};
  }}
  function spinbuttonPlan() {{
    const valueMatch = instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(-?\d+(?:\.\d+)?)\s+(?:with|on|using|in|into)\s+(?:the\s+)?(?:spinner|spinbutton|stepper|number|numeric)\b/i);
    if (!valueMatch) return null;
    const desired = valueMatch[1];
    const fields = all('input[type=number], [role=spinbutton]').filter(visible);
    if (!fields.length) return null;
    const ranked = best(fields, el => 0.4 + controlTypeScore(targetHint, el) + (targetHint ? tokenScore(targetHint, textOf(el)) * 0.4 : 0));
    const chosen = (ranked[0] || {{ el: fields[0], score: 0.7 }});
    return withFollowUp({{
      ok: true,
      action: 'type',
      params: {{ selector: selector(chosen.el), text: desired, clear_first: true }},
      confidence: Math.min(1, chosen.score || 0.75),
      reason: 'matched numeric spinner or spinbutton value from instruction',
      candidate: candidate(chosen.el)
    }}, chosen.el);
  }}
  function sliderCheckboxPlan() {{
    if (!(/\bslider\b/i.test(instruction) && /\bcheckbox\b/i.test(instruction))) return null;
    const steps = [];
    const slider = sliderPlan();
    if (slider) steps.push(slider);
    const boxes = interactive.filter(el => {{
      const type = (el.getAttribute('type') || '').toLowerCase();
      const role = (el.getAttribute('role') || '').toLowerCase();
      return type === 'checkbox' || role === 'checkbox';
    }});
    const index = ordinalIndex(instruction);
    let followAnchor = null;
    if (index != null && boxes[index]) {{
      followAnchor = boxes[index];
      steps.push({{
        action: 'set_checked',
        params: {{ selector: selector(boxes[index]), checked: true }},
        confidence: 0.9,
        reason: 'matched ordinal checkbox target from instruction',
        candidate: candidate(boxes[index])
      }});
    }}
    const follow = clickStepForHint(followUpClickHint(), followAnchor);
    if (follow) steps.push(follow);
    if (steps.length >= 2) {{
      return {{
        action: 'sequence',
        steps,
        confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
        reason: 'planned slider, checkbox, and follow-up click sequence'
      }};
    }}
    return null;
  }}

  function discoverClickPlan() {{
    const target = targetHint || (instruction.match(/\blink\s+["']?([^"'.]+)["']?/i) || [])[1];
    if (!target) return null;
    if (!/\b(find|switch|expand|reveal|open)\b/i.test(instruction)) return null;
    return {{
      ok: true,
      action: 'discover_click',
      params: {{ target }},
      confidence: 0.75,
      reason: 'instruction asks to reveal content and click a target by text'
    }};
  }}

  function parseDurationSeconds(text) {{
    const match = String(text || '').match(/(?:(\d+)\s*h(?:ours?)?)?\s*(?:(\d+)\s*m(?:in(?:ute)?s?)?)?/i);
    if (!match || (!match[1] && !match[2])) return null;
    return (Number(match[1] || 0) * 3600) + (Number(match[2] || 0) * 60);
  }}
  function extremeClickPlan() {{
    if (!/\b(shortest|longest|lowest|highest|smallest|largest|cheapest|most expensive)\b/i.test(instruction)) return null;
    const wantMin = /\b(shortest|lowest|smallest|cheapest)\b/i.test(instruction);
    const buttons = interactive.filter(el => {{
      const tag = el.tagName.toLowerCase();
      return tag === 'button' || tag === 'a' || (el.getAttribute('role') || '').toLowerCase() === 'button';
    }});
    const scored = [];
    for (const button of buttons) {{
      const ancestors = [];
      let cursor = button;
      while (cursor && cursor !== root && cursor !== document.body && ancestors.length < 6) {{
        if (cursor.nodeType === Node.ELEMENT_NODE) ancestors.push(cursor);
        cursor = cursor.parentElement;
      }}
      const container = (/\bduration\b/i.test(instruction)
        ? ancestors.find(el => /duration/i.test(textOf(el)))
        : null) || button.closest('tr, li, .flight, .card, .row, section, article, div') || button.parentElement;
      if (!container) continue;
      const text = textOf(container);
      let metric = /duration/i.test(instruction) ? parseDurationSeconds(text) : null;
      if (metric == null) {{
        const numbers = Array.from(String(text).matchAll(/-?\d+(?:\.\d+)?/g)).map(m => Number(m[0]));
        if (numbers.length) metric = numbers[0];
      }}
      if (metric != null && Number.isFinite(metric)) scored.push({{ button, metric }});
    }}
    if (!scored.length) return null;
    scored.sort((a, b) => wantMin ? a.metric - b.metric : b.metric - a.metric);
    const chosen = scored[0];
    return {{
      ok: true,
      action: 'click',
      params: {{ selector: selector(chosen.button) }},
      confidence: 0.82,
      reason: 'matched repeated item with requested extreme numeric or duration value',
      candidate: candidate(chosen.button),
      metric: chosen.metric
    }};
  }}

  function countPlan() {{
    if (kind !== 'count') return null;
    const textItems = all('svg text').filter(el => visible(el) || (el.textContent || '').trim()).map(el => {{
      const size = Number.parseFloat(getComputedStyle(el).fontSize || el.getAttribute('font-size') || '0');
      return {{ el, text: (el.textContent || '').trim(), size, fill: String(el.getAttribute('fill') || getComputedStyle(el).fill || '').toLowerCase() }};
    }});
    const svgItems = all('svg text, svg circle, svg rect, svg polygon, svg path, svg ellipse').filter(el => visible(el)).map(el => ({{
      el,
      tag: el.tagName.toLowerCase(),
      text: (el.textContent || '').trim(),
      fill: String(el.getAttribute('fill') || getComputedStyle(el).fill || '').toLowerCase()
    }}));
    let answer = null;
    if (/\bdigits?\b/i.test(instruction)) {{
      answer = textItems.filter(item => /^\d$/.test(item.text)).length;
    }}
    if (answer == null) {{
      const colorMatch = instruction.match(/\b(red|scarlet|blue|green|yellow|magenta|purple|aqua|cyan|black|white|orange|gray|grey)\b/i);
      if (colorMatch && /\bletters?\b/i.test(instruction)) {{
        const color = colorMatch[1].toLowerCase();
        answer = textItems.filter(item => /^[a-z]$/i.test(item.text) && item.fill.includes(color)).length;
      }} else if (colorMatch && /\bitems?\b/i.test(instruction)) {{
        const color = colorMatch[1].toLowerCase();
        answer = svgItems.filter(item => item.fill.includes(color)).length;
      }}
    }}
    if (answer == null) {{
      const sizeMatch = instruction.match(/\b(small|large|big|tiny)\s+(?:letter\s+)?([a-z])s?\b/i);
      const letterMatch = sizeMatch || instruction.match(/\b(?:letter\s+)?([a-z])s?\b/i);
      if (letterMatch) {{
        const sizeWord = sizeMatch ? sizeMatch[1].toLowerCase() : null;
        const letter = (sizeMatch ? sizeMatch[2] : letterMatch[1]).toLowerCase();
        const sizes = textItems.map(item => item.size).filter(Number.isFinite).sort((a, b) => a - b);
        const median = sizes.length ? sizes[Math.floor(sizes.length / 2)] : 0;
        answer = textItems.filter(item => {{
          if (item.text.toLowerCase() !== letter) return false;
          if (!sizeWord) return true;
          if (sizeWord === 'small' || sizeWord === 'tiny') return !item.size || item.size <= median;
          return item.size >= median;
        }}).length;
      }}
    }}
    if (answer == null && /\b(circles?|rectangles?|squares?|triangles?|polygons?)\b/i.test(instruction)) {{
      const tag = /circles?/i.test(instruction) ? 'circle' :
        /rectangles?|squares?/i.test(instruction) ? 'rect' :
        /triangles?|polygons?/i.test(instruction) ? 'polygon' : null;
      if (tag) answer = all('svg ' + tag).filter(visible).length;
    }}
    if (answer == null) return null;
    const buttons = best(interactive.filter(el => el.tagName.toLowerCase() === 'button' || (el.getAttribute('role') || '').toLowerCase() === 'button'), el => {{
      return (textOf(el).trim() === String(answer)) ? 1 : 0;
    }});
    if (buttons.length) {{
      return {{
        ok: true,
        action: 'click',
        params: {{ selector: selector(buttons[0].el) }},
        confidence: 0.8,
        reason: 'counted visible page items and matched numeric answer button',
        candidate: candidate(buttons[0].el),
        answer
      }};
    }}
    const fields = all('input, textarea').filter(el => {{
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute('type') || '').toLowerCase();
      return tag === 'textarea' || (tag === 'input' && ['', 'text', 'number'].includes(type));
    }});
    if (fields.length) {{
      return {{
        ok: true,
        action: 'type',
        params: {{ selector: selector(fields[0]), text: String(answer), clear_first: true }},
        confidence: 0.75,
        reason: 'counted visible page items and matched answer field',
        candidate: candidate(fields[0]),
        answer
      }};
    }}
    return null;
  }}

  function scrollFillPressPlan() {{
    if (kind !== 'fill' || !/\bscroll\b/i.test(instruction) || !wantedValue) return null;
    const hasBox = el => {{
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return (r.width > 0 || r.height > 0) && s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
    }};
    const scrollable = all('textarea, [style*=overflow], [role=textbox], div').find(el => hasBox(el) && el.scrollHeight > el.clientHeight + 8);
	    const fields = all('input, textarea, [contenteditable=true], [contenteditable=""], [role=textbox], [role=searchbox], [role=spinbutton], [role=slider]').filter(el => {{
	      const tag = el.tagName.toLowerCase();
	      if (tag === 'textarea') return false;
	      return isFillableField(el);
	    }});
    const field = fields.find(el => targetHint ? tokenScore(targetHint, textOf(el)) > 0 : true) || fields[0];
    const followHint = followUpClickHint() || secondaryHint;
    const follow = followHint ? all('button, a, input, [role=button], [role=link]').find(el => tokenScore(followHint, textOf(el)) > 0 || String(textOf(el)).toLowerCase().includes(String(followHint).toLowerCase())) : null;
    if (!scrollable || !field) return null;
    const steps = [{{
      action: 'scroll_element',
      params: {{ selector: selector(scrollable) }},
      confidence: 0.85,
      reason: 'matched scrollable element mentioned before fill action',
      candidate: candidate(scrollable)
    }}, {{
      action: 'type',
      params: {{ selector: selector(field), text: transformedValue(wantedValue), clear_first: true }},
      confidence: 0.75,
      reason: 'matched fillable field after prerequisite scroll',
      candidate: candidate(field)
    }}];
    if (follow) steps.push({{
      action: 'click',
      params: {{ selector: selector(follow) }},
      confidence: 0.75,
      reason: 'matched follow-up control after prerequisite scroll and fill',
      candidate: candidate(follow)
    }});
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned scroll, fill, and follow-up control sequence'
    }};
  }}
  const capabilityPlan = chooseCapabilityPlan([
    {{ name: 'ordered-click-sequence', priority: 110, build: orderedClickSequencePlan }},
    {{ name: 'ordered-value-clicks', priority: 105, build: orderedValueClickPlan }},
    {{ name: 'multi-slider-values', priority: 95, build: multiSliderPlan }},
    {{ name: 'slider-checkbox-sequence', priority: 92, build: sliderCheckboxPlan }},
    {{ name: 'single-slider-value', priority: 90, build: () => {{
      const plan = sliderPlan();
      if (!plan || /\bcheckbox\b/i.test(instruction)) return null;
      return withFollowUp(plan, document.querySelector(plan.params.selector));
    }} }},
    {{ name: 'spinbutton-value', priority: 88, build: spinbuttonPlan }},
    {{ name: 'discovery-click', priority: 82, build: discoverClickPlan }},
    {{ name: 'extreme-click', priority: 80, build: extremeClickPlan }},
    {{ name: 'count-answer', priority: 78, build: countPlan }},
    {{ name: 'scroll-fill-press', priority: 75, build: scrollFillPressPlan }}
  ]);
  if (capabilityPlan) return capabilityPlan;

	  if (kind === 'fill') {{
	    const fields = interactive.filter(isFillableField);
	    const ranked = best(fields, el => {{
	      const t = textOf(el);
	      let score = targetHint ? tokenScore(targetHint, t) : 0.2;
	      score += controlTypeScore(targetHint, el);
	      if (targetHint && /\b(text|input|field|box)\b/i.test(targetHint) && score === 0) score = 0.2;
	      if (typeOf(el) === 'search') score += 0.2;
	      if (/search/.test(instruction.toLowerCase()) && /search/.test(t.toLowerCase())) score += 0.5;
	      return score;
	    }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no fillable field found' }};
    if (!wantedValue) return {{ ok: false, error: 'act_instruction: fill instruction has no text value' }};
    const textValue = transformedValue(wantedValue);
    const repeatedFields = /\bboth\s+(?:text\s+)?(?:fields?|inputs?)\b/i.test(instruction) ||
      /\ball\s+(?:text\s+)?(?:fields?|inputs?)\b/i.test(instruction);
    if (repeatedFields) {{
      const count = /\bboth\b/i.test(instruction) ? Math.min(2, fields.length) : fields.length;
	      const repeatedRanked = best(fields, el => {{
	        const t = textOf(el);
	        let score = targetHint ? tokenScore(targetHint, t) : 0.2;
	        score += controlTypeScore(targetHint, el);
	        if (/\bpassword\b/i.test(instruction) && typeOf(el) === 'password') score += 0.6;
        if (targetHint && /\b(text|input|field|box)\b/i.test(targetHint) && score === 0) score = 0.2;
        return score;
      }});
      const targets = repeatedRanked.slice(0, count).map(item => item.el);
      const steps = targets.map(el => ({{
        action: 'type',
        params: {{ selector: selector(el), text: textValue, clear_first: true }},
        confidence: 0.7,
        reason: 'matched repeated fillable field from collective instruction',
        candidate: candidate(el)
      }}));
      const follow = clickStepForHint(followUpClickHint(), targets[targets.length - 1]);
      if (follow) steps.push(follow);
      if (steps.length) {{
        return {{
          ok: true,
          action: 'sequence',
          steps,
          confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
          reason: 'planned repeated field fill sequence from collective instruction'
        }};
      }}
    }}
    const chosen = ranked[0];
    return withFollowUp({{
      ok: true, action: 'type',
      params: {{ selector: selector(chosen.el), text: textValue, clear_first: true }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched fillable field from instruction and DOM labels',
      candidate: candidate(chosen.el)
    }}, chosen.el);
  }}

	  if (kind === 'select_option') {{
	    const selects = best(interactive.filter(el => {{
	      const tag = el.tagName.toLowerCase();
	      const role = roleOf(el);
	      return tag === 'select' || ['combobox', 'listbox', 'menu'].includes(role) || el.hasAttribute('aria-haspopup');
	    }}), el => {{
	      const options = Array.from(el.options || []).map(o => (o.textContent || o.value || '').toLowerCase()).join(' ');
	      let score = (wantedValue ? tokenScore(stripFollowUp(wantedValue), options) : 0) + (targetHint ? tokenScore(targetHint, textOf(el)) * 0.7 : 0.2);
	      if (/\b(dropdown|select|option|list|menu|combo)\b/i.test(instruction) && (el.tagName.toLowerCase() === 'select' || roleOf(el) === 'combobox' || el.hasAttribute('aria-haspopup'))) score += 0.35;
	      return score;
	    }});
    if (selects.length && wantedValue) {{
      const chosen = selects[0];
      return withFollowUp({{
        ok: true, action: 'select_option',
        params: {{ selector: selector(chosen.el), option: stripFollowUp(wantedValue) }},
        confidence: Math.min(1, chosen.score),
        reason: 'matched select element and option text',
        candidate: candidate(chosen.el)
      }}, chosen.el);
    }}
    const boxes = interactive.filter(el => {{
      const type = (el.getAttribute('type') || '').toLowerCase();
      const role = (el.getAttribute('role') || '').toLowerCase();
      return type === 'checkbox' || type === 'radio' || role === 'checkbox' || role === 'radio';
    }});
    const items = requestedItems(wantedValue);
    if (boxes.length && (items.length || /^nothing$/i.test(stripFollowUp(wantedValue)))) {{
      const used = new Set();
      const steps = [];
      let followAnchor = boxes[boxes.length - 1] || null;
      for (const item of items) {{
        const rankedBoxes = best(boxes.filter(el => !used.has(selector(el))), el => {{
          const text = textOf(el);
          return Math.max(semanticScore(item, text), tokenScore(item, text), text.toLowerCase().includes(item.toLowerCase()) ? 1 : 0);
        }});
        if (rankedBoxes.length) {{
          const chosen = rankedBoxes[0];
          followAnchor = chosen.el;
          used.add(selector(chosen.el));
          steps.push({{
            action: 'set_checked',
            params: {{ selector: selector(chosen.el), checked: true }},
            confidence: Math.min(1, chosen.score),
            reason: 'matched checkbox option by visible label text',
            candidate: candidate(chosen.el)
          }});
        }}
      }}
      const follow = clickStepForHint(followUpClickHint(), followAnchor);
      if (follow) steps.push(follow);
      if (steps.length) {{
        return {{
          ok: true,
          action: 'sequence',
          steps,
          confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
          reason: 'planned checkbox selection sequence from listed instruction values'
        }};
      }}
    }}
    const optionClicks = best(interactive, el => tokenScore(wantedValue || targetHint, textOf(el)));
    if (!optionClicks.length) return {{ ok: false, error: 'act_instruction: no matching option-like element found' }};
    const chosen = optionClicks[0];
    return withFollowUp({{
      ok: true, action: 'click',
      params: {{ selector: selector(chosen.el) }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched clickable option-like element by visible text',
      candidate: candidate(chosen.el)
    }}, chosen.el);
  }}

  if (kind === 'set_checked') {{
    const boxes = interactive.filter(el => {{
      const type = (el.getAttribute('type') || '').toLowerCase();
      const role = (el.getAttribute('role') || '').toLowerCase();
      return type === 'checkbox' || type === 'radio' || role === 'checkbox' || role === 'radio';
    }});
    const items = requestedItems(targetHint);
    if (checked !== false && items.length > 1) {{
      const used = new Set();
      const steps = [];
      let followAnchor = boxes[boxes.length - 1] || null;
      for (const item of items) {{
        const rankedBoxes = best(boxes.filter(el => !used.has(selector(el))), el => {{
          const text = textOf(el);
          return Math.max(semanticScore(item, text), tokenScore(item, text), text.toLowerCase().includes(item.toLowerCase()) ? 1 : 0);
        }});
        if (rankedBoxes.length) {{
          const chosen = rankedBoxes[0];
          followAnchor = chosen.el;
          used.add(selector(chosen.el));
          steps.push({{
            action: 'set_checked',
            params: {{ selector: selector(chosen.el), checked: true }},
            confidence: Math.min(1, chosen.score),
            reason: 'matched checkbox or radio item by visible label text',
            candidate: candidate(chosen.el)
          }});
        }}
      }}
      const follow = clickStepForHint(followUpClickHint(), followAnchor);
      if (follow) steps.push(follow);
      if (steps.length) {{
        return {{
          ok: true,
          action: 'sequence',
          steps,
          confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
          reason: 'planned checkbox or radio sequence from listed instruction values'
        }};
      }}
    }}
    const ranked = best(boxes, el => targetHint ? tokenScore(targetHint, textOf(el)) : 0.2);
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no checkbox or radio target found' }};
    const chosen = ranked[0];
    return withFollowUp({{
      ok: true, action: 'set_checked',
      params: {{ selector: selector(chosen.el), checked: checked !== false }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched checkbox or radio by instruction text',
      candidate: candidate(chosen.el)
    }}, chosen.el);
  }}

  if (kind === 'drag') {{
    const rankedSource = best(interactive, el => tokenScore(targetHint, textOf(el)));
    const rankedTarget = best(interactive.concat(all('div, li, td, canvas, svg, [role=gridcell]')), el => tokenScore(secondaryHint, textOf(el)));
    if (!rankedSource.length || !rankedTarget.length) return {{ ok: false, error: 'act_instruction: could not match drag source and target' }};
    return {{
      ok: true, action: 'drag',
      params: {{ source: selector(rankedSource[0].el), target: selector(rankedTarget[0].el) }},
      confidence: Math.min(1, (rankedSource[0].score + rankedTarget[0].score) / 2),
      reason: 'matched drag source and target by instruction text',
      candidate: {{ source: candidate(rankedSource[0].el), target: candidate(rankedTarget[0].el) }}
    }};
  }}

  const clickHint = targetHint || wantedValue || instruction;
  const clickables = best(interactive, el => {{
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    let score = Math.max(tokenScore(clickHint, textOf(el)), exactPhraseScore(clickHint, textOf(el)));
    if (kind === 'click' && /submit|continue|confirm|save|done|next/.test(instruction.toLowerCase())) {{
      if (type === 'submit') score += 0.5;
      if (/submit|continue|confirm|save|done|next|ok/.test(textOf(el).toLowerCase())) score += 0.4;
    }}
    if (tag === 'button' || tag === 'a' || type === 'submit' || (el.getAttribute('role') || '').toLowerCase() === 'button') score += 0.05;
    return score;
  }});
  if (!clickables.length) return {{ ok: false, error: 'act_instruction: no clickable target found' }};
  const chosen = clickables[0];
  return {{
    ok: true, action: 'click',
    params: {{ selector: selector(chosen.el) }},
    confidence: Math.min(1, chosen.score),
    reason: 'matched clickable element by instruction text',
    candidate: candidate(chosen.el)
  }};
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_generic_actions() {
        assert_eq!(
            analyze_instruction("click the Continue button").kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction("enter 'alice@example.com' into email").kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction("choose California from the State dropdown").kind,
            InstructionKind::SelectOption
        );
        assert_eq!(
            analyze_instruction("uncheck newsletter checkbox").checked,
            Some(false)
        );
        let checked = analyze_instruction("check Red, Green, and Blue");
        assert_eq!(checked.kind, InstructionKind::SetChecked);
        assert_eq!(checked.target_hint.as_deref(), Some("Red, Green, and Blue"));
        assert_eq!(
            analyze_instruction("drag card A to Done").kind,
            InstructionKind::Drag
        );
        assert_eq!(
            analyze_instruction("scroll up").direction.as_deref(),
            Some("up")
        );
    }

    #[test]
    fn quoted_text_becomes_fill_value() {
        let analysis = analyze_instruction("type \"hello world\" into the message field");
        assert_eq!(analysis.kind, InstructionKind::Fill);
        assert_eq!(analysis.value.as_deref(), Some("hello world"));
        assert_eq!(analysis.target_hint.as_deref(), Some("the message field"));
    }

    #[test]
    fn unquoted_value_target_pairs_are_split() {
        let fill = analyze_instruction("enter Alice into email");
        assert_eq!(fill.kind, InstructionKind::Fill);
        assert_eq!(fill.value.as_deref(), Some("Alice"));
        assert_eq!(fill.target_hint.as_deref(), Some("email"));

        let select = analyze_instruction("choose California from the State dropdown");
        assert_eq!(select.kind, InstructionKind::SelectOption);
        assert_eq!(select.value.as_deref(), Some("California"));
        assert_eq!(select.target_hint.as_deref(), Some("the State dropdown"));
    }

    #[test]
    fn value_controls_are_not_treated_as_dropdown_options() {
        let slider = analyze_instruction("Select -94 with the slider and hit Submit");
        assert_eq!(slider.kind, InstructionKind::Fill);
        assert_eq!(slider.value.as_deref(), Some("-94"));
        assert_eq!(slider.target_hint.as_deref(), Some("the slider"));

        let spinner = analyze_instruction("Use the spinner to select -6");
        assert_eq!(spinner.kind, InstructionKind::Fill);
        assert_eq!(spinner.value.as_deref(), Some("-6"));
        assert_eq!(spinner.target_hint.as_deref(), Some("the spinner"));

        let date = analyze_instruction("set date to 2026-06-04");
        assert_eq!(date.kind, InstructionKind::Fill);
        assert_eq!(date.value.as_deref(), Some("2026-06-04"));
        assert_eq!(date.target_hint.as_deref(), Some("date"));
    }

    #[test]
    fn sequence_helpers_report_step_count_and_confidence() {
        let plan = json!({
            "action": "sequence",
            "confidence": 0.42,
            "steps": [
                {"action": "type", "confidence": 0.7},
                {"action": "click", "confidence": 0.9}
            ]
        });
        assert_eq!(plan_step_count(&plan), 2);
        assert_eq!(plan_confidence(&plan), 0.42);
    }

    #[test]
    fn non_sequence_plan_counts_as_one_step() {
        let plan = json!({
            "action": "click",
            "confidence": 0.8,
            "params": {"selector": "#submit"}
        });
        assert_eq!(plan_step_count(&plan), 1);
        assert_eq!(plan_confidence(&plan), 0.8);
    }
}
