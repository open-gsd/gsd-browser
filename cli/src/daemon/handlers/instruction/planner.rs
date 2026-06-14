use super::model::{json_literal, InstructionAnalysis, InstructionIntent, InstructionKind};
use super::page_model::availability_helpers_js;
use super::parser::{
    find_elements_query, looks_like_selector, viewport_dimensions, wait_timeout_ms,
};
use super::planner_js::planner_js;
use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;

const PLAN_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn build_plan(
    page: &Page,
    instruction: &str,
    analysis: &InstructionAnalysis,
    intent: &InstructionIntent,
    scope: Option<&str>,
) -> Result<Value, String> {
    if analysis.kind == InstructionKind::Scroll {
        if let Some(plan) =
            build_scrollable_element_plan(page, instruction, analysis.direction.as_deref()).await?
        {
            return Ok(plan);
        }
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
    if analysis.kind == InstructionKind::Wait {
        let direction = analysis.direction.as_deref().unwrap_or("visible");
        let target = analysis
            .value
            .as_deref()
            .or(analysis.target_hint.as_deref())
            .unwrap_or("")
            .trim();
        let condition = if direction == "network_idle" || target.is_empty() {
            "network_idle"
        } else if looks_like_selector(target) {
            if direction == "hidden" {
                "selector_hidden"
            } else {
                "selector_visible"
            }
        } else if direction == "hidden" {
            "text_hidden"
        } else {
            "text_visible"
        };
        return Ok(json!({
            "action": "wait_for",
            "params": {
                "condition": condition,
                "value": if condition == "network_idle" { "" } else { target },
                "timeout": wait_timeout_ms(instruction).unwrap_or(10_000),
            },
            "confidence": if condition == "network_idle" { 0.75 } else { 0.9 },
            "reason": "instruction asks to wait for a generic browser condition",
            "evidence": {
                "waitDirection": direction,
                "target": target,
            }
        }));
    }
    if analysis.kind == InstructionKind::Navigate {
        let direction = analysis.direction.as_deref().unwrap_or("url");
        let (action, params, confidence) = match direction {
            "back" => ("back", json!({}), 0.92),
            "forward" => ("forward", json!({}), 0.92),
            "reload" => ("reload", json!({}), 0.92),
            _ => {
                let Some(url) = analysis.value.as_deref() else {
                    return Err("act_instruction: navigation instruction has no URL".to_string());
                };
                ("navigate", json!({ "url": url }), 0.95)
            }
        };
        return Ok(json!({
            "action": action,
            "params": params,
            "confidence": confidence,
            "reason": "instruction asks for generic browser navigation",
            "evidence": {
                "navigationDirection": direction,
                "url": analysis.value,
            }
        }));
    }
    if analysis.kind == InstructionKind::SetViewport {
        let direction = analysis.direction.as_deref().unwrap_or("preset");
        let Some(value) = analysis.value.as_deref() else {
            return Err("act_instruction: viewport instruction has no size or preset".to_string());
        };
        let params = if direction == "dimensions" {
            let Some((width, height)) = viewport_dimensions(value) else {
                return Err("act_instruction: viewport dimensions are invalid".to_string());
            };
            json!({ "width": width, "height": height })
        } else {
            json!({ "preset": value })
        };
        return Ok(json!({
            "action": "set_viewport",
            "params": params,
            "confidence": 0.92,
            "reason": "instruction asks to resize the browser viewport",
            "evidence": {
                "viewportMode": direction,
                "value": value,
            }
        }));
    }
    if analysis.kind == InstructionKind::EmulateDevice {
        let Some(device) = analysis.value.as_deref() else {
            return Err(
                "act_instruction: device emulation instruction has no device name".to_string(),
            );
        };
        return Ok(json!({
            "action": "emulate_device",
            "params": { "device": device },
            "confidence": 0.9,
            "reason": "instruction asks to emulate a named browser device profile",
            "evidence": {
                "device": device,
            }
        }));
    }
    if analysis.kind == InstructionKind::AnalyzeForm {
        let target = analysis
            .target_hint
            .as_deref()
            .or(analysis.value.as_deref())
            .unwrap_or("")
            .trim();
        if target.is_empty() || looks_like_selector(target) {
            let mut params = json!({});
            if !target.is_empty() {
                params["selector"] = json!(target);
            }
            return Ok(json!({
                "action": "analyze_form",
                "params": params,
                "confidence": 0.9,
                "reason": "instruction asks to inspect form fields and controls",
                "evidence": {
                    "target": target,
                }
            }));
        }
        return evaluate_dom_planner(
            page,
            instruction,
            analysis,
            intent,
            scope,
            "act_instruction: no matching form target found",
        )
        .await;
    }
    if analysis.kind == InstructionKind::AccessibilityTree {
        let target = analysis
            .target_hint
            .as_deref()
            .or(analysis.value.as_deref())
            .unwrap_or("")
            .trim();
        if target.is_empty() || looks_like_selector(target) {
            let mut params = json!({
                "max_depth": 10,
                "max_count": 100,
            });
            if !target.is_empty() {
                params["selector"] = json!(target);
            }
            return Ok(json!({
                "action": "accessibility_tree",
                "params": params,
                "confidence": 0.9,
                "reason": "instruction asks to inspect page accessibility roles and names",
                "evidence": {
                    "target": target,
                }
            }));
        }
        return evaluate_dom_planner(
            page,
            instruction,
            analysis,
            intent,
            scope,
            "act_instruction: no matching accessibility target found",
        )
        .await;
    }
    if analysis.kind == InstructionKind::FindElements {
        if is_form_search_instruction(instruction) {
            if let Ok(plan) = evaluate_dom_planner(
                page,
                instruction,
                analysis,
                intent,
                scope,
                "act_instruction: no actionable search form plan found",
            )
            .await
            {
                if plan.get("action").and_then(|value| value.as_str()) == Some("form_workflow") {
                    return Ok(plan);
                }
            }
        }
        let query = find_elements_query(instruction);
        let mut params = json!({ "limit": 20 });
        if let Some(role) = query.role {
            params["role"] = json!(role);
        }
        if let Some(text) = query.text {
            params["text"] = json!(text);
        }
        if let Some(selector) = query.selector {
            params["selector"] = json!(selector);
        }
        if params.get("role").is_none()
            && params.get("text").is_none()
            && params.get("selector").is_none()
        {
            return Err(
                "act_instruction: find instruction has no role, text, or selector".to_string(),
            );
        }
        return Ok(json!({
            "action": "find",
            "params": params,
            "confidence": 0.88,
            "reason": "instruction asks to find matching page elements without acting",
            "evidence": {
                "findDirection": analysis.direction,
                "target": analysis.value,
            }
        }));
    }
    if analysis.kind == InstructionKind::ReadText {
        let target = analysis
            .target_hint
            .as_deref()
            .or(analysis.value.as_deref())
            .unwrap_or("")
            .trim();
        if target.is_empty() || looks_like_selector(target) {
            let mut params = json!({ "max_length": 20_000 });
            if !target.is_empty() {
                params["selector"] = json!(target);
            }
            return Ok(json!({
                "action": "read_text",
                "params": params,
                "confidence": 0.9,
                "reason": "instruction asks to read visible text from the current page or selector",
                "evidence": {
                    "target": target,
                }
            }));
        }
        return evaluate_dom_planner(
            page,
            instruction,
            analysis,
            intent,
            scope,
            "act_instruction: no matching text target found",
        )
        .await;
    }
    if analysis.kind == InstructionKind::Screenshot {
        let direction = analysis.direction.as_deref().unwrap_or("viewport");
        let target = analysis.target_hint.as_deref().unwrap_or("").trim();
        if !target.is_empty() && !looks_like_selector(target) {
            return evaluate_dom_planner(
                page,
                instruction,
                analysis,
                intent,
                scope,
                "act_instruction: no screenshot target found",
            )
            .await;
        }
        let mut params = json!({
            "full_page": direction == "full_page",
            "format": if direction == "full_page" { "jpeg" } else { "png" },
        });
        if !target.is_empty() {
            params["selector"] = json!(target);
        }
        return Ok(json!({
            "action": "screenshot",
            "params": params,
            "confidence": 0.9,
            "reason": "instruction asks to capture screenshot evidence",
            "evidence": {
                "screenshotScope": direction,
                "target": target,
            }
        }));
    }
    if analysis.kind == InstructionKind::Assert {
        let direction = analysis.direction.as_deref().unwrap_or("visible");
        let target = analysis
            .target_hint
            .as_deref()
            .or(analysis.value.as_deref())
            .unwrap_or("")
            .trim();
        let needs_semantic_dom_target =
            matches!(direction, "value_equals" | "checked" | "unchecked")
                && !target.is_empty()
                && !looks_like_selector(target);
        if needs_semantic_dom_target {
            return evaluate_dom_planner(
                page,
                instruction,
                analysis,
                intent,
                scope,
                "act_instruction: no actionable assertion plan found",
            )
            .await;
        }
        let check = match direction {
            "no_console_errors" => json!({ "kind": "no_console_errors" }),
            "no_failed_requests" => json!({ "kind": "no_failed_requests" }),
            "value_equals" => {
                let Some(expected) = analysis.value.as_deref() else {
                    return Err(
                        "act_instruction: value assertion has no expected value".to_string()
                    );
                };
                if target.is_empty() {
                    return Err("act_instruction: value assertion has no target".to_string());
                }
                json!({ "kind": "value_equals", "selector": target, "value": expected })
            }
            "checked" | "unchecked" => {
                if target.is_empty() {
                    return Err("act_instruction: checked assertion has no target".to_string());
                }
                json!({ "kind": "checked", "selector": target, "checked": direction == "checked" })
            }
            "url_contains" => {
                if target.is_empty() {
                    return Err("act_instruction: URL assertion has no text".to_string());
                }
                json!({ "kind": "url_contains", "text": target })
            }
            "title_contains" => {
                if target.is_empty() {
                    return Err("act_instruction: title assertion has no text".to_string());
                }
                json!({ "kind": "title_contains", "text": target })
            }
            "hidden" => {
                if target.is_empty() {
                    return Err("act_instruction: hidden assertion has no target".to_string());
                }
                if looks_like_selector(target) {
                    json!({ "kind": "selector_hidden", "selector": target })
                } else {
                    json!({ "kind": "text_hidden", "text": target })
                }
            }
            _ => {
                if target.is_empty() {
                    return Err("act_instruction: visible assertion has no target".to_string());
                }
                if looks_like_selector(target) {
                    json!({ "kind": "selector_visible", "selector": target })
                } else {
                    json!({ "kind": "text_visible", "text": target })
                }
            }
        };
        return Ok(json!({
            "action": "assert",
            "params": {
                "checks": [check],
            },
            "confidence": 0.9,
            "reason": "instruction asks to verify a generic browser condition",
            "evidence": {
                "assertDirection": direction,
                "target": target,
            }
        }));
    }

    evaluate_dom_planner(
        page,
        instruction,
        analysis,
        intent,
        scope,
        "act_instruction: no actionable plan found",
    )
    .await
}

fn is_form_search_instruction(instruction: &str) -> bool {
    let lower = instruction.to_lowercase();
    (lower.starts_with("search for ")
        || lower.starts_with("filter for ")
        || lower.starts_with("find "))
        && (lower.contains(" by ")
            || lower.contains(" year ")
            || lower.contains(" from ")
            || lower.contains(" directed ")
            || lower.contains(" authored ")
            || lower.contains(" written ")
            || lower.contains(" published "))
}

async fn evaluate_dom_planner(
    page: &Page,
    instruction: &str,
    analysis: &InstructionAnalysis,
    intent: &InstructionIntent,
    scope: Option<&str>,
    fallback_error: &str,
) -> Result<Value, String> {
    let js = planner_js(instruction, analysis, intent, scope);
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "act_instruction: planner timed out".to_string())?
        .map_err(|e| {
            format!(
                "act_instruction: planner failed: {}",
                super::super::clean_cdp_error(&e)
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
            .unwrap_or(fallback_error)
            .to_string())
    }
}

async fn build_scrollable_element_plan(
    page: &Page,
    instruction: &str,
    direction: Option<&str>,
) -> Result<Option<Value>, String> {
    let lower = instruction.to_lowercase();
    if ![
        "textarea",
        "text area",
        "panel",
        "box",
        "container",
        "field",
        "text",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Ok(None);
    }
    let instruction_json = json_literal(instruction);
    let direction_json = json_literal(direction.unwrap_or("down"));
    let availability_helpers_js = availability_helpers_js();
    let js = format!(
        r#"(() => {{
  const instruction = {instruction_json};
  const direction = {direction_json};
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' &&
      style.visibility !== 'hidden' &&
      Number(style.opacity || 1) !== 0;
  }}
  function selector(el) {{
    if (!el || !el.tagName) return null;
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
  function textOf(el) {{
    return [el.innerText || el.textContent || '', el.value || '', el.id || '', el.className || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
  }}
  const candidates = Array.from(document.querySelectorAll('textarea, [role=textbox], [style*=overflow], div, section, article'))
    .filter(el => visible(el) && el.scrollHeight > el.clientHeight + 8)
    .map(el => {{
      const tag = el.tagName.toLowerCase();
      const text = textOf(el).toLowerCase();
      let score = 0.2;
      if (tag === 'textarea') score += 0.8;
      if (/\btextarea|text area\b/i.test(instruction) && tag === 'textarea') score += 0.5;
      if (/\bpanel|container|box\b/i.test(instruction) && /\bpanel|container|box\b/i.test(text)) score += 0.3;
      if (/\btext\b/i.test(instruction) && text.trim()) score += 0.15;
      const rect = el.getBoundingClientRect();
      if (rect.width * rect.height > 250000) score -= 0.4;
      return {{ el, score }};
    }})
    .filter(item => item.score >= 0.25)
    .sort((a, b) => b.score - a.score);
  if (!candidates.length) return null;
  const chosen = candidates[0].el;
  const buttons = Array.from(document.querySelectorAll('button, input[type=submit], input[type=button], [role=button]'))
    .filter(visible)
    .map(el => {{
      const text = textOf(el).toLowerCase();
      let score = /\bsubmit|done|save|confirm|continue|ok\b/i.test(text) ? 0.7 : 0;
      if (el.compareDocumentPosition(chosen) & Node.DOCUMENT_POSITION_PRECEDING) score += 0.1;
      return {{ el, score }};
    }})
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score);
  const scroll = {{
    ok: true,
    action: 'scroll_element',
    params: {{ selector: selector(chosen), direction }},
    confidence: Math.min(1, candidates[0].score),
    reason: 'matched scroll instruction to scrollable page element',
    candidate: {{ selector: selector(chosen), tag: chosen.tagName.toLowerCase() }}
  }};
  if (!/\bsubmit|done|save|confirm|continue|ok|hit\b/i.test(instruction) || !buttons.length) return scroll;
  const click = {{
    action: 'click',
    params: {{ selector: selector(buttons[0].el) }},
    confidence: Math.min(1, buttons[0].score),
    reason: 'matched completion control after scroll',
    candidate: {{ selector: selector(buttons[0].el), tag: buttons[0].el.tagName.toLowerCase() }}
  }};
  return {{
    ok: true,
    action: 'sequence',
    steps: [scroll, click],
    confidence: Math.min(scroll.confidence, click.confidence),
    reason: 'planned element scroll plus completion control'
  }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "scroll element planning timed out".to_string())?
        .map_err(|e| {
            format!(
                "scroll element planning failed: {}",
                super::super::clean_cdp_error(&e)
            )
        })?;
    Ok(result.value().cloned().filter(|value| !value.is_null()))
}
