//! Natural-language instruction planning for generic browser actions.
//!
//! This module intentionally avoids benchmark- or site-specific task names. It
//! translates short user instructions into existing primitive handlers by
//! combining verb classification with live DOM affordances.

use crate::daemon::capture::capture_compact_page_state;
use crate::daemon::handlers;
use crate::daemon::state::DaemonState;
use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;

const PLAN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstructionKind {
    Click,
    Fill,
    SelectOption,
    SetChecked,
    Drag,
    Scroll,
    Unknown,
}

impl InstructionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Fill => "fill",
            Self::SelectOption => "select_option",
            Self::SetChecked => "set_checked",
            Self::Drag => "drag",
            Self::Scroll => "scroll",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstructionAnalysis {
    kind: InstructionKind,
    value: Option<String>,
    target_hint: Option<String>,
    secondary_hint: Option<String>,
    checked: Option<bool>,
    direction: Option<String>,
}

/// Handle `act_instruction`.
///
/// Params: { instruction: string, dry_run?: bool }
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

    let analysis = analyze_instruction(instruction);
    if analysis.kind == InstructionKind::Unknown {
        return Err(format!(
            "act_instruction: could not infer a generic browser action from instruction: {instruction}"
        ));
    }

    let plan = build_plan(page, instruction, &analysis).await?;
    if dry_run {
        return Ok(json!({
            "instruction": instruction,
            "analysis": analysis_to_json(&analysis),
            "plan": plan,
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

    Ok(json!({
        "instruction": instruction,
        "analysis": analysis_to_json(&analysis),
        "plan": plan,
        "result": result,
        "state": capture_compact_page_state(page, false).await,
    }))
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
        "drag" => handlers::interaction::handle_drag(page, state, &params).await,
        "scroll" => handlers::interaction::handle_scroll(page, state, &params).await,
        other => Err(format!(
            "act_instruction: unsupported planned action: {other}"
        )),
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
  if (window.jQuery && window.jQuery(el).slider) {{
    try {{
      window.jQuery(el).slider('value', desired);
      return {{ ok: true, selector, value: Number(window.jQuery(el).slider('value')), mode: 'jquery-ui' }};
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

fn analysis_to_json(analysis: &InstructionAnalysis) -> Value {
    json!({
        "kind": analysis.kind.as_str(),
        "value": analysis.value,
        "targetHint": analysis.target_hint,
        "secondaryHint": analysis.secondary_hint,
        "checked": analysis.checked,
        "direction": analysis.direction,
    })
}

fn analyze_instruction(instruction: &str) -> InstructionAnalysis {
    let lower = instruction.to_lowercase();
    let quoted = quoted_strings(instruction);
    let mut value = quoted.first().cloned();
    let mut target_hint = None;
    let mut secondary_hint = quoted.get(1).cloned();
    let mut checked = None;
    let mut direction = None;

    let kind = if contains_any(&lower, &["drag ", "dragged ", "move "]) && lower.contains(" to ") {
        target_hint = quoted.first().cloned();
        if target_hint.is_none() || secondary_hint.is_none() {
            let (first, second) = split_around_to(instruction);
            target_hint = target_hint.or(first);
            secondary_hint = secondary_hint.or(second);
        }
        InstructionKind::Drag
    } else if lower.contains("scroll") {
        direction = if lower.contains("up") {
            Some("up".to_string())
        } else {
            Some("down".to_string())
        };
        InstructionKind::Scroll
    } else if contains_any(&lower, &["uncheck", "untick", "deselect "]) {
        checked = Some(false);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["uncheck", "untick", "deselect"]));
        InstructionKind::SetChecked
    } else if contains_any(&lower, &["check ", "tick "])
        && contains_any(&lower, &["checkbox", "box", "radio"])
    {
        checked = Some(true);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["check", "tick"]));
        InstructionKind::SetChecked
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
    {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &["type", "enter", "fill", "input", "write", "search"],
            &[" into ", " in ", " to "],
        );
        value = value.or(parsed_value);
        target_hint = field_hint(instruction).or(parsed_target);
        InstructionKind::Fill
    } else if starts_with_any(
        &lower,
        &[
            "click ", "press ", "tap ", "open ", "submit", "continue", "confirm", "save", "done",
            "next",
        ],
    ) {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["click", "press", "tap", "open"]));
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
            text.trim()
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

async fn build_plan(
    page: &Page,
    instruction: &str,
    analysis: &InstructionAnalysis,
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

    let js = planner_js(instruction, analysis);
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

fn planner_js(instruction: &str, analysis: &InstructionAnalysis) -> String {
    let instruction_json = serde_json::to_string(instruction).unwrap();
    let kind_json = serde_json::to_string(analysis.kind.as_str()).unwrap();
    let value_json = serde_json::to_string(&analysis.value).unwrap();
    let target_json = serde_json::to_string(&analysis.target_hint).unwrap();
    let secondary_json = serde_json::to_string(&analysis.secondary_hint).unwrap();
    let checked_json = serde_json::to_string(&analysis.checked).unwrap();

    format!(
        r#"(() => {{
  const instruction = {instruction_json};
  const kind = {kind_json};
  const wantedValue = {value_json};
  const targetHint = {target_json};
  const secondaryHint = {secondary_json};
  const checked = {checked_json};

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
      el.textContent || '', el.value || '', el.name || '', el.placeholder || '',
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
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    if (el.name) {{
      const sel = el.tagName.toLowerCase() + '[name=' + JSON.stringify(el.name) + ']';
      if (document.querySelectorAll(sel).length === 1) return sel;
    }}
    const type = el.getAttribute('type');
    if (type) {{
      const sel = el.tagName.toLowerCase() + '[type=' + JSON.stringify(type) + ']';
      if (document.querySelectorAll(sel).length === 1) return sel;
    }}
    const all = Array.from(document.querySelectorAll(el.tagName.toLowerCase()));
    return el.tagName.toLowerCase() + ':nth-of-type(' + (all.indexOf(el) + 1) + ')';
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
    return cleaned.split(/\s*,\s*|\s+\band\b\s+/i).map(item => item.trim()).filter(Boolean);
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
      let score = tokenScore(hint, textOf(el));
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
  const interactive = Array.from(document.querySelectorAll(
    'button, a, input, textarea, select, [role=button], [role=link], [role=option], [role=menuitem], [role=tab], [role=slider], [onclick], [tabindex], [contenteditable=true]'
  ));
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
  function sliderPlan() {{
    const valueMatch = instruction.match(/\b(?:select|set|choose|move)\s+(-?\d+(?:\.\d+)?)\s+(?:with|on|using)\s+(?:the\s+)?slider\b/i);
    if (!valueMatch) return null;
    const desired = Number(valueMatch[1]);
    const sliders = Array.from(document.querySelectorAll('input[type=range], [role=slider], .ui-slider, [class*=slider]')).filter(visible);
    for (const el of sliders) {{
      const minAttr = el.getAttribute('min') ?? el.getAttribute('aria-valuemin');
      const maxAttr = el.getAttribute('max') ?? el.getAttribute('aria-valuemax');
      let min = minAttr == null ? Number.NaN : Number(minAttr);
      let max = maxAttr == null ? Number.NaN : Number(maxAttr);
      let orientation = (el.getAttribute('aria-orientation') || '').toLowerCase();
      try {{
        if ((!Number.isFinite(min) || !Number.isFinite(max)) && window.jQuery && window.jQuery(el).slider) {{
          min = Number(window.jQuery(el).slider('option', 'min'));
          max = Number(window.jQuery(el).slider('option', 'max'));
          orientation = String(window.jQuery(el).slider('option', 'orientation') || orientation).toLowerCase();
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
  if (/\bslider\b/i.test(instruction) && /\bcheckbox\b/i.test(instruction)) {{
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
        ok: true,
        action: 'sequence',
        steps,
        confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
        reason: 'planned slider, checkbox, and follow-up click sequence'
      }};
    }}
  }}

  if (kind === 'fill') {{
    const fields = interactive.filter(el => {{
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute('type') || '').toLowerCase();
      return tag === 'textarea' || el.isContentEditable ||
        (tag === 'input' && ['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number'].includes(type));
    }});
    const ranked = best(fields, el => {{
      const t = textOf(el);
      let score = targetHint ? tokenScore(targetHint, t) : 0.2;
      if (targetHint && /\b(text|input|field|box)\b/i.test(targetHint) && score === 0) score = 0.2;
      if ((el.getAttribute('type') || '').toLowerCase() === 'search') score += 0.2;
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
        if (/\bpassword\b/i.test(instruction) && (el.getAttribute('type') || '').toLowerCase() === 'password') score += 0.6;
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
    const selects = best(interactive.filter(el => el.tagName.toLowerCase() === 'select'), el => {{
      const options = Array.from(el.options || []).map(o => (o.textContent || o.value || '').toLowerCase()).join(' ');
      return (wantedValue ? tokenScore(stripFollowUp(wantedValue), options) : 0) + (targetHint ? tokenScore(targetHint, textOf(el)) * 0.5 : 0);
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
          return tokenScore(item, text) || (text.toLowerCase().includes(item.toLowerCase()) ? 1 : 0);
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
    const rankedTarget = best(interactive.concat(Array.from(document.querySelectorAll('div, li, td, canvas, svg, [role=gridcell]'))), el => tokenScore(secondaryHint, textOf(el)));
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
    let score = tokenScore(clickHint, textOf(el));
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
}
