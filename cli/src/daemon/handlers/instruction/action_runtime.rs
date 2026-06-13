use chromiumoxide::cdp::browser_protocol::input::{DispatchMouseEventType, MouseButton};
use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::{sleep, timeout};

use crate::daemon::capture::capture_compact_page_state;
use crate::daemon::handlers;
use crate::daemon::input_dispatch::{dispatch_mouse, mouse_button, mouse_buttons_mask_for_button};
use crate::daemon::inspection;
use crate::daemon::settle::{ensure_mutation_counter, settle_after_action};
use crate::daemon::state::DaemonState;
use gsd_browser_common::types::SettleOptions;

use super::model::json_literal;
use super::page_model::{
    accessible_text_helpers_js, availability_helpers_js, control_semantics_helpers_js,
    value_control_helpers_js,
};
use super::PLAN_TIMEOUT;

pub(super) async fn handle_read_text(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params.get("selector").and_then(|value| value.as_str());
    let max_length = params
        .get("max_length")
        .and_then(|value| value.as_u64())
        .unwrap_or(20_000)
        .clamp(200, 100_000) as usize;
    let selector_json = json_literal(&selector);
    let availability_helpers_js = availability_helpers_js();

    let js = format!(
        r#"(() => {{
  const selector = {selector_json};
  const maxLength = {max_length};

  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForRead(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' &&
      style.visibility !== 'hidden' &&
      Number(style.opacity || 1) !== 0;
  }}
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
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
  function allRoots(start) {{
    const roots = [];
    const seen = new Set();
    function add(root) {{
      if (!root || seen.has(root)) return;
      seen.add(root);
      roots.push(root);
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
    add(start || document);
    return roots;
  }}
  function findTarget() {{
    if (!selector) return document.body || document.documentElement;
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selector)) return root;
        const found = root.querySelector && root.querySelector(selector);
        if (found) return found;
      }} catch (_) {{}}
    }}
    return null;
  }}
  function visibleText(root) {{
    if (!root) return '';
    const chunks = [];
    const seen = new Set();
    function add(text) {{
      const cleaned = String(text || '').replace(/\s+/g, ' ').trim();
      if (!cleaned || seen.has(cleaned)) return;
      seen.add(cleaned);
      chunks.push(cleaned);
    }}
    function walk(node) {{
      if (!node) return;
      if (node.nodeType === Node.TEXT_NODE) {{
        if (node.parentElement && visible(node.parentElement)) add(node.textContent);
        return;
      }}
      if (node.nodeType !== Node.ELEMENT_NODE && node.nodeType !== Node.DOCUMENT_NODE && node.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
      const el = node.nodeType === Node.ELEMENT_NODE ? node : null;
      if (el) {{
        const tag = el.tagName.toLowerCase();
        if (['script', 'style', 'noscript', 'template'].includes(tag)) return;
        if (!visible(el)) return;
        if ('value' in el && /^(input|textarea|select)$/i.test(tag)) add(el.value);
        add(el.getAttribute('aria-label'));
        add(el.getAttribute('title'));
      }}
      for (const child of Array.from(node.childNodes || [])) walk(child);
      if (el && el.shadowRoot) walk(el.shadowRoot);
    }}
    walk(root);
    return chunks.join('\n').replace(/\n{{3,}}/g, '\n\n').trim();
  }}

  const target = findTarget();
  if (!target) return {{ ok: false, error: 'read_text target not found', selector }};
  const text = visibleText(target);
  const length = text.length;
  return {{
    ok: true,
    selector: selectorFor(target),
    text: text.slice(0, maxLength),
    length,
    truncated: length > maxLength,
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "read_text timed out".to_string())?
        .map_err(|e| {
            format!(
                "read_text failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if !value
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(value
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("read_text failed")
            .to_string());
    }
    Ok(value)
}

pub(super) async fn handle_select_text(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "select_text requires selector".to_string())?;
    let target_text = params.get("text").and_then(|value| value.as_str());
    let selector_json = json_literal(selector);
    let target_text_json = json_literal(&target_text);

    let js = format!(
        r#"(() => {{
  const selectorText = {selector_json};
  const targetText = {target_text_json};

  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
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
  function allRoots(start) {{
    const roots = [];
    const seen = new Set();
    function add(root) {{
      if (!root || seen.has(root)) return;
      seen.add(root);
      roots.push(root);
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
    add(start || document);
    return roots;
  }}
  function findTarget() {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const found = root.querySelector && root.querySelector(selectorText);
        if (found) return found;
      }} catch (_) {{}}
    }}
    return null;
  }}
  function findTextRange(root, wanted) {{
    if (!wanted) return null;
    const needle = String(wanted);
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {{
      acceptNode(node) {{
        if (!node.textContent || !node.textContent.includes(needle)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      }}
    }});
    const node = walker.nextNode();
    if (!node) return null;
    const start = node.textContent.indexOf(needle);
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, start + needle.length);
    return range;
  }}
  function quillFor(el) {{
    try {{
      if (window.Quill && window.Quill.find) {{
        const q = window.Quill.find(el);
        if (q && q.setSelection) return q;
      }}
    }} catch (_) {{}}
    try {{
      if (window.editor && window.editor.root && (window.editor.root === el || el.contains(window.editor.root) || window.editor.root.contains(el))) {{
        return window.editor;
      }}
    }} catch (_) {{}}
    return null;
  }}

  const target = findTarget();
  if (!target) return {{ ok: false, error: 'select_text target not found', selector: selectorText }};

  const tag = target.tagName ? target.tagName.toLowerCase() : '';
  let selected = '';
  let mode = 'dom-range';

  if ((tag === 'input' || tag === 'textarea') && typeof target.setSelectionRange === 'function') {{
    const value = String(target.value || '');
    const start = targetText ? value.indexOf(String(targetText)) : 0;
    const from = start >= 0 ? start : 0;
    const to = targetText && start >= 0 ? start + String(targetText).length : value.length;
    target.focus();
    target.setSelectionRange(from, to);
    selected = value.slice(from, to);
    mode = tag;
    }} else {{
      const quill = quillFor(target);
      if (quill && !targetText) {{
        const length = Math.max(0, quill.getLength ? quill.getLength() - 1 : String(target.textContent || '').length);
        target.focus && target.focus();
        quill.setSelection(0, length, 'api');
        selected = String(target.textContent || '').trim();
        mode = 'quill';
      }} else {{
      const matchedRange = targetText ? findTextRange(target, targetText) : null;
      const range = matchedRange || document.createRange();
      if (!matchedRange) range.selectNodeContents(target);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      if (target.focus) target.focus();
      selected = selection.toString();
    }}
  }}

  document.dispatchEvent(new Event('selectionchange', {{ bubbles: true }}));
  target.dispatchEvent(new Event('select', {{ bubbles: true }}));

  return {{
    ok: true,
    selector: selectorFor(target),
    requestedText: targetText,
    selected,
    mode
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "select_text timed out".to_string())?
        .map_err(|e| {
            format!(
                "select_text failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if !value
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(value
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("select_text failed")
            .to_string());
    }
    Ok(json!({
        "selection": value,
    }))
}

pub(super) async fn handle_format_text(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "format_text requires selector".to_string())?;
    let style = params
        .get("style")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "format_text requires style".to_string())?;
    let target_text = params.get("text").and_then(|value| value.as_str());
    let color = params.get("color").and_then(|value| value.as_str());
    let selector_json = json_literal(selector);
    let style_json = json_literal(style);
    let target_text_json = json_literal(&target_text);
    let color_json = json_literal(&color);

    let js = format!(
        r#"(() => {{
  const selectorText = {selector_json};
  const style = {style_json};
  const targetText = {target_text_json};
  const requestedColor = {color_json};

  function allRoots(start) {{
    const roots = [];
    const seen = new Set();
    function add(root) {{
      if (!root || seen.has(root)) return;
      seen.add(root);
      roots.push(root);
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
    add(start || document);
    return roots;
  }}
  function findTarget() {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const found = root.querySelector && root.querySelector(selectorText);
        if (found) return found;
      }} catch (_) {{}}
    }}
    return null;
  }}
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
  function quillFor(el) {{
    try {{
      if (window.Quill && window.Quill.find) {{
        const q = window.Quill.find(el);
        if (q && q.formatText) return q;
      }}
    }} catch (_) {{}}
    try {{
      if (window.editor && window.editor.root && (window.editor.root === el || el.contains(window.editor.root) || window.editor.root.contains(el))) {{
        return window.editor;
      }}
    }} catch (_) {{}}
    return null;
  }}
  function quillAttribute() {{
    if (/^bold$/i.test(style)) return ['bold', true];
    if (/^italic$/i.test(style)) return ['italic', true];
    if (/^underline$/i.test(style)) return ['underline', true];
    if (/^color$/i.test(style)) return ['color', requestedColor || true];
    return [style, true];
  }}
  function commandName() {{
    if (/^bold$/i.test(style)) return 'bold';
    if (/^italic$/i.test(style)) return 'italic';
    if (/^underline$/i.test(style)) return 'underline';
    if (/^color$/i.test(style)) return 'foreColor';
    return style;
  }}
  function selectDomContents(el) {{
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    return selection.toString();
  }}

  const target = findTarget();
  if (!target) return {{ ok: false, error: 'format_text target not found', selector: selectorText }};
  const quill = quillFor(target);
  let mode = 'contenteditable';
  let formattedText = '';

  if (quill) {{
    const text = quill.getText ? quill.getText().replace(/\n$/, '') : String(target.textContent || '');
    let start = 0;
    let length = text.length;
    if (targetText) {{
      const found = text.indexOf(String(targetText));
      if (found >= 0) {{
        start = found;
        length = String(targetText).length;
      }}
    }}
    const [attr, value] = quillAttribute();
    quill.formatText(start, length, attr, value, 'api');
    quill.setSelection(start, length, 'api');
    formattedText = text.slice(start, start + length);
    mode = 'quill';
  }} else {{
    target.focus && target.focus();
    formattedText = selectDomContents(target);
    try {{
      document.execCommand(commandName(), false, requestedColor || null);
    }} catch (_) {{}}
  }}

  target.dispatchEvent(new Event('input', {{ bubbles: true }}));
  target.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return {{
    ok: true,
    selector: selectorFor(target),
    mode,
    style,
    color: requestedColor,
    targetText,
    formattedText
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "format_text timed out".to_string())?
        .map_err(|e| {
            format!(
                "format_text failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if !value
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(value
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("format_text failed")
            .to_string());
    }
    Ok(json!({
        "formatText": value,
    }))
}

pub(super) async fn handle_focus_element(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    ensure_mutation_counter(page).await;
    let action_result =
        inspection::perform_selector_action(page, state, selector, "focus", &json!({}), true)
            .await?;
    if !action_result
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(action_result
            .get("error")
            .or_else(|| action_result.get("reason"))
            .and_then(|value| value.as_str())
            .unwrap_or("focus failed")
            .to_string());
    }
    let selector_json = json_literal(selector);
    let focus_event_js = format!(
        r#"(() => {{
  const selectorText = {selector_json};
  function allRoots(start) {{
    const roots = [];
    const seen = new Set();
    function add(root) {{
      if (!root || seen.has(root)) return;
      seen.add(root);
      roots.push(root);
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
    add(start || document);
    return roots;
  }}
  for (const root of allRoots(document)) {{
    try {{
      const el = root.matches && root.matches(selectorText) ? root : root.querySelector && root.querySelector(selectorText);
      if (!el) continue;
      if (typeof el.focus === 'function') el.focus();
      el.dispatchEvent(new FocusEvent('focus', {{ bubbles: false }}));
      el.dispatchEvent(new FocusEvent('focusin', {{ bubbles: true }}));
      return true;
    }} catch (_) {{}}
  }}
  return false;
}})()"#
    );
    let _ = timeout(PLAN_TIMEOUT, page.evaluate_expression(&focus_event_js)).await;
    let settle = settle_after_action(
        page,
        &SettleOptions {
            timeout_ms: 500,
            check_focus_stability: true,
            ..SettleOptions::default()
        },
    )
    .await;
    Ok(json!({
        "focused": {
            "selector": selector,
            "frameLabel": action_result.get("target").and_then(|value| value.get("frameLabel")).cloned().unwrap_or(Value::Null),
            "frameUrl": action_result.get("target").and_then(|value| value.get("frameUrl")).cloned().unwrap_or(Value::Null),
            "actionability": action_result.get("actionability").cloned().unwrap_or(Value::Null),
        },
        "settle": serde_json::to_value(&settle).unwrap_or(json!({})),
        "state": capture_compact_page_state(page, false).await,
        "boundaries": action_result.get("boundaries").cloned().unwrap_or(json!([])),
    }))
}

pub(super) async fn handle_set_checkbox_grid(page: &Page, params: &Value) -> Result<Value, String> {
    let target = params
        .get("value")
        .or_else(|| params.get("target"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "set_checkbox_grid requires a target value".to_string())?;
    let selector = params
        .get("selector")
        .or_else(|| params.get("container"))
        .and_then(|v| v.as_str());
    let target_json = json_literal(target);
    let selector_json = json_literal(&selector);
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let js = format!(
        r#"(() => {{
  const target = {target_json};
  const containerSelector = {selector_json};
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function findOne(selectorText) {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const match = root.querySelector && root.querySelector(selectorText);
        if (match) return match;
      }} catch (_) {{}}
    }}
    return null;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const root of allRoots(start)) {{
      try {{
        if (root.matches && root.matches(selectorText) && !seen.has(root)) {{
          seen.add(root);
          results.push(root);
        }}
        const matches = root.querySelectorAll ? Array.from(root.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  const container = containerSelector ? findOne(containerSelector) : document;
  if (!container) return {{ ok: false, error: 'checkbox grid container not found: ' + containerSelector }};

	  {availability_helpers_js}
  function isReadOnlyControl(el) {{
    return !!el.readOnly ||
      el.getAttribute('readonly') !== null ||
      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
  }}
  {control_semantics_helpers_js}
	  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity || 1) !== 0;
  }}
	  function setChecked(el, next) {{
	    if ('checked' in el) {{
	      const proto = Object.getPrototypeOf(el);
	      const setter = proto && Object.getOwnPropertyDescriptor(proto, 'checked')?.set;
	      if (setter) setter.call(el, next);
	      else el.checked = next;
	      el.dispatchEvent(new Event('input', {{ bubbles: true }}));
	      el.dispatchEvent(new Event('change', {{ bubbles: true }}));
	      return el.checked === next;
    }}
    el.setAttribute('aria-checked', next ? 'true' : 'false');
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
	    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
	    return el.getAttribute('aria-checked') === (next ? 'true' : 'false');
	  }}
  function readChecked(el) {{
    if ('checked' in el) return !!el.checked;
    return el.getAttribute('aria-checked') === 'true';
  }}
  function checkboxCells(scope) {{
    return all('input[type=checkbox], [role=checkbox]', scope)
      .concat(all('*', scope).filter(isCustomCheckableElement))
      .filter((el, index, arr) => arr.indexOf(el) === index);
  }}
  function checkboxRows(cells) {{
    const measured = cells.map((el, index) => {{
      const rect = el.getBoundingClientRect();
      return {{ el, index, rect, centerY: rect.top + rect.height / 2, centerX: rect.left + rect.width / 2 }};
    }}).sort((a, b) => a.centerY - b.centerY || a.centerX - b.centerX);
    const heights = measured.map(item => item.rect.height).filter(value => value > 0).sort((a, b) => a - b);
    const medianHeight = heights.length ? heights[Math.floor(heights.length / 2)] : 16;
    const tolerance = Math.max(6, medianHeight * 0.55);
    const rows = [];
    for (const item of measured) {{
      let row = rows.find(candidate => Math.abs(candidate.centerY - item.centerY) <= tolerance);
      if (!row) {{
        row = {{ centerY: item.centerY, items: [] }};
        rows.push(row);
      }}
      row.items.push(item);
      row.centerY = row.items.reduce((sum, cell) => sum + cell.centerY, 0) / row.items.length;
    }}
    return rows
      .sort((a, b) => a.centerY - b.centerY)
      .map(row => row.items.sort((a, b) => a.centerX - b.centerX).map(item => item.el));
  }}
  const glyphs = {{
    '0': [[0,1,1,0],[1,0,0,1],[1,0,0,1],[1,0,0,1],[1,0,0,1],[1,0,0,1],[0,1,1,0]],
    '1': [[0,0,1,0],[0,1,1,0],[0,0,1,0],[0,0,1,0],[0,0,1,0],[0,0,1,0],[0,0,1,0]],
    '2': [[0,1,1,0],[1,0,0,1],[1,0,0,1],[0,0,1,0],[0,1,0,0],[1,0,0,0],[1,1,1,1]],
    '3': [[0,1,1,0],[1,0,0,1],[0,0,0,1],[0,1,1,0],[0,0,0,1],[1,0,0,1],[0,1,1,0]],
    '4': [[1,0,0,0],[1,0,1,0],[1,0,1,0],[1,1,1,1],[0,0,1,0],[0,0,1,0],[0,0,1,0]],
    '5': [[1,1,1,1],[1,0,0,0],[1,0,0,0],[1,1,1,0],[0,0,0,1],[1,0,0,1],[0,1,1,0]],
    '6': [[0,1,1,0],[1,0,0,1],[1,0,0,0],[1,1,1,0],[1,0,0,1],[1,0,0,1],[0,1,1,0]],
    '7': [[1,1,1,1],[1,0,0,1],[0,0,0,1],[0,0,1,0],[0,0,1,0],[0,1,0,0],[0,1,0,0]],
    '8': [[0,1,1,0],[1,0,0,1],[1,0,0,1],[0,1,1,0],[1,0,0,1],[1,0,0,1],[0,1,1,0]],
    '9': [[0,1,1,0],[1,0,0,1],[1,0,0,1],[0,1,1,1],[0,0,0,1],[1,0,0,1],[0,1,1,0]],
  }};
  const digitMatch = String(target || '').match(/\b\d\b/);
  if (!digitMatch || !glyphs[digitMatch[0]]) {{
    return {{ ok: false, error: 'checkbox grid renderer currently supports single digit glyph targets' }};
  }}
  const pattern = glyphs[digitMatch[0]];
	  const boxes = checkboxCells(container).filter(visible);
  if (boxes.length < 4) return {{ ok: false, error: 'no visible checkbox grid cells found' }};
  const rows = checkboxRows(boxes);
  const cols = rows.length ? Math.max(...rows.map(row => row.length)) : 0;
  if (rows.length !== pattern.length || rows.some(row => row.length !== pattern[0].length)) {{
    return {{ ok: false, error: 'checkbox grid dimensions do not match supported digit glyph', rows: rows.length, cols }};
  }}
  let checkedCount = 0;
  let changed = 0;
  const checkedPositions = [];
  for (let rowIndex = 0; rowIndex < rows.length; rowIndex++) {{
    for (let colIndex = 0; colIndex < rows[rowIndex].length; colIndex++) {{
      const next = pattern[rowIndex][colIndex] === 1;
      const box = rows[rowIndex][colIndex];
	      const before = readChecked(box);
      if (next) {{
        checkedCount += 1;
        checkedPositions.push([rowIndex, colIndex]);
      }}
      if (before !== next) changed += 1;
      setChecked(box, next);
    }}
  }}
  return {{
    ok: true,
    target: digitMatch[0],
    mode: 'digit-glyph-checkbox-grid',
    selector: containerSelector,
    rows: rows.length,
    cols,
    checkedCount,
    changed,
    checkedPositions
  }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "set_checkbox_grid timed out".to_string())?
        .map_err(|e| {
            format!(
                "set_checkbox_grid failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "checkboxGrid": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("set_checkbox_grid failed")
            .to_string())
    }
}

pub(super) async fn handle_autocomplete_select(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: query".to_string())?;
    let starts_with = params.get("startsWith").and_then(|v| v.as_str());
    let ends_with = params.get("endsWith").and_then(|v| v.as_str());
    let option_text = params.get("optionText").and_then(|v| v.as_str());

    let type_result = inspection::perform_selector_action(
        page,
        state,
        selector,
        "type",
        &json!({
            "text": query,
            "clearFirst": true,
            "slowly": false,
            "submit": false,
        }),
        true,
    )
    .await?;
    if !type_result
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(type_result
            .get("error")
            .or_else(|| type_result.get("reason"))
            .and_then(|value| value.as_str())
            .unwrap_or("autocomplete typing failed")
            .to_string());
    }

    let selector_json = json_literal(selector);
    let query_json = json_literal(query);
    let starts_json = json_literal(&starts_with);
    let ends_json = json_literal(&ends_with);
    let option_json = json_literal(&option_text);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(async () => {{
  const inputSelector = {selector_json};
  const query = {query_json};
  const startsWith = {starts_json};
  const endsWith = {ends_json};
  const optionText = {option_json};
  const input = findOne(inputSelector);
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  if (!input) return {{ ok: false, error: 'autocomplete_select input not found: ' + inputSelector }};
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity || 1) !== 0;
  }}
	  function normalized(text) {{
	    return String(text || '').toLowerCase().replace(/\s+/g, ' ').trim();
	  }}
	  function isReadOnlyControl(el) {{
	    return !!el.readOnly ||
	      el.getAttribute('readonly') !== null ||
	      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
	  }}
	  {accessible_text_helpers_js}
	  {control_semantics_helpers_js}
	  {value_control_helpers_js}
	  function labelText(el) {{
    const labels = [];
    labels.push(associatedLabelText(el));
    labels.push(referencedText(el, 'aria-labelledby'));
    labels.push(referencedText(el, 'aria-describedby'));
    labels.push(structuralLabelText(el));
    labels.push(semanticAttributeText(el));
    return labels.join(' ');
  }}
  function selectorFor(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
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
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope.matches && scope.matches(selectorText) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function findOne(selectorText) {{
    return all(selectorText)[0] || null;
  }}
  function rootGetElementById(rootNode, id) {{
    return (rootNode && rootNode.getElementById && rootNode.getElementById(id)) || document.getElementById(id);
  }}
  function textOf(el) {{
    return [
      el.textContent || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('data-value') || '',
      el.getAttribute('value') || '',
      labelText(el),
      semanticAttributeText(el),
      slotText(el),
      svgReferenceText(el),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function readInputValue() {{
    return 'value' in input ? String(input.value || '') : String(input.textContent || '');
  }}
		  function setInputValue(value, mode) {{
		    setControlValue(input, value);
	    return {{ mode, inputValue: readInputValue() }};
	  }}
  function itemText(item) {{
    if (item == null) return '';
    if (typeof item === 'string') return item;
    return String(item.label || item.value || item.text || '');
  }}
  async function jqueryAutocompleteItems() {{
    try {{
      if (!window.jQuery) return [];
      const jq = window.jQuery(input);
      if (!jq || !jq.autocomplete) return [];
      const hasWidget = jq.data('ui-autocomplete') || jq.data('autocomplete');
      if (!hasWidget) return [];
      try {{ jq.autocomplete('search', query); }} catch (_) {{}}
      const source = jq.autocomplete('option', 'source');
      if (Array.isArray(source)) return source;
      if (typeof source === 'function') {{
        return await new Promise(resolve => {{
          let settled = false;
          const finish = items => {{
            if (settled) return;
            settled = true;
            resolve(Array.isArray(items) ? items : []);
          }};
          try {{
            source.call(input, {{ term: query }}, finish);
          }} catch (_) {{
            finish([]);
          }}
          setTimeout(() => finish([]), 600);
        }});
      }}
    }} catch (_) {{}}
    return [];
  }}
  function dispatchChoiceClick(el) {{
    el.scrollIntoView({{ block: 'nearest', inline: 'nearest' }});
    for (const type of ['mouseover', 'mousedown', 'mouseup', 'click']) {{
      el.dispatchEvent(new MouseEvent(type, {{ bubbles: true, cancelable: true, view: window }}));
    }}
  }}
  function scoreText(text) {{
    const value = normalized(text);
    if (!value) return 0;
    let score = 0;
    if (optionText) {{
      const exact = normalized(optionText);
      if (value === exact) score += 2;
      else if (value.includes(exact)) score += 1.2;
    }}
    if (startsWith) {{
      const prefix = normalized(startsWith);
      if (value.startsWith(prefix)) score += 1.5;
      else if (value.includes(prefix)) score += 0.45;
      else return 0;
    }}
    if (endsWith) {{
      const suffix = normalized(endsWith);
      if (value.endsWith(suffix)) score += 1.5;
      else return 0;
    }}
    if (!optionText && !startsWith && !endsWith) {{
      const q = normalized(query);
      if (value === q) score += 1.3;
      else if (value.startsWith(q)) score += 1;
      else if (value.includes(q)) score += 0.35;
    }}
    return score;
  }}
  function optionElements() {{
    const selectors = [
      '[role=option]',
      '[role=menuitem]',
      '.ui-autocomplete .ui-menu-item-wrapper',
      '.ui-autocomplete .ui-menu-item',
      '.ui-menu-item-wrapper',
      '.ui-menu-item',
      '[aria-selected]',
      'datalist option'
    ].join(',');
    const seen = new Set();
    const out = [];
    for (const el of all(selectors)) {{
      const key = selectorFor(el);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(el);
    }}
    return out;
  }}
  function activeDescendantMatch() {{
    const activeId = input.getAttribute('aria-activedescendant');
    if (!activeId) return null;
    const active = rootGetElementById(input.getRootNode && input.getRootNode(), activeId);
    if (!active) return null;
    const text = textOf(active);
    const score = scoreText(text);
    return score > 0 ? {{ el: active, text, score }} : null;
  }}
  async function acceptActiveDescendant(match, mode) {{
    if (visible(match.el)) {{
      dispatchChoiceClick(match.el);
      await delay(120);
    }}
    if (!normalized(readInputValue()).includes(normalized(match.text))) {{
      setInputValue(match.text, mode + '-set');
    }}
    return {{
      ok: true,
      selector: inputSelector,
      query,
      startsWith,
      endsWith,
      selected: match.text,
      selectedSelector: selectorFor(match.el),
      mode,
      inputValue: readInputValue(),
    }};
  }}
  input.focus();
  input.dispatchEvent(new KeyboardEvent('keydown', {{ key: query.slice(-1) || '', bubbles: true, cancelable: true }}));
  input.dispatchEvent(new KeyboardEvent('keyup', {{ key: query.slice(-1) || '', bubbles: true, cancelable: true }}));
  await delay(650);

  const activeInitial = activeDescendantMatch();
  if (activeInitial) return await acceptActiveDescendant(activeInitial, 'aria-activedescendant');

  const listId = input.getAttribute('list');
  if (listId) {{
    const list = rootGetElementById(input.getRootNode && input.getRootNode(), listId);
    if (list) {{
      const datalistMatches = Array.from(list.querySelectorAll('option')).map(option => {{
        const text = option.value || option.textContent || '';
        return {{ option, text, score: scoreText(text) }};
      }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
      if (datalistMatches.length) {{
        const chosen = datalistMatches[0];
        const set = setInputValue(chosen.text, 'datalist');
        return {{
          ok: true,
          selector: inputSelector,
          query,
          startsWith,
          endsWith,
          selected: chosen.text,
          selectedSelector: selectorFor(chosen.option),
          mode: set.mode,
          inputValue: set.inputValue,
        }};
      }}
    }}
  }}

  const sourceMatches = (await jqueryAutocompleteItems()).map(item => {{
    const text = itemText(item);
    return {{ item, text, score: scoreText(text) }};
  }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
  if (sourceMatches.length) {{
    const chosen = sourceMatches[0];
    const set = setInputValue(chosen.text, 'jquery-ui-source');
    return {{
      ok: true,
      selector: inputSelector,
      query,
      startsWith,
      endsWith,
      selected: chosen.text,
      selectedSelector: null,
      mode: set.mode,
      inputValue: set.inputValue,
    }};
  }}

  const ranked = optionElements()
    .filter(el => visible(el) || el.tagName.toLowerCase() === 'option')
    .map(el => {{
      const text = textOf(el);
      return {{ el, text, score: scoreText(text) }};
    }})
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score);
  if (ranked.length) {{
    const chosen = ranked[0];
    dispatchChoiceClick(chosen.el);
    await delay(120);
    if (!normalized(readInputValue()).includes(normalized(chosen.text))) {{
      setInputValue(chosen.text, 'visible-option-fallback-set');
    }}
    return {{
      ok: true,
      selector: inputSelector,
      query,
      startsWith,
      endsWith,
      selected: chosen.text,
      selectedSelector: selectorFor(chosen.el),
      mode: 'visible-option-click',
      inputValue: readInputValue(),
    }};
  }}

  input.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'ArrowDown', bubbles: true, cancelable: true }}));
  input.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'ArrowDown', bubbles: true, cancelable: true }}));
  await delay(80);
  const activeAfterArrow = activeDescendantMatch();
  if (activeAfterArrow) return await acceptActiveDescendant(activeAfterArrow, 'aria-activedescendant');
  input.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true, cancelable: true }}));
  input.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'Enter', bubbles: true, cancelable: true }}));
  await delay(120);
  const fallbackValue = readInputValue();
  if (scoreText(fallbackValue) > 0) {{
    return {{
      ok: true,
      selector: inputSelector,
      query,
      startsWith,
      endsWith,
      selected: fallbackValue,
      selectedSelector: null,
      mode: 'keyboard-fallback',
      inputValue: fallbackValue,
    }};
  }}
  return {{
    ok: false,
    error: 'autocomplete_select found no matching suggestion',
    selector: inputSelector,
    query,
    startsWith,
    endsWith,
    visibleSuggestions: optionElements().filter(visible).map(el => textOf(el)).slice(0, 20),
    inputValue: readInputValue(),
  }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "autocomplete_select timed out".to_string())?
        .map_err(|e| {
            format!(
                "autocomplete_select failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "autocomplete": value,
            "typed": {
                "selector": selector,
                "text_length": query.len(),
                "actual": value.get("inputValue").cloned().unwrap_or(Value::Null),
                "actionability": type_result.get("actionability").cloned().unwrap_or(Value::Null),
                "valueResult": type_result.get("valueResult").cloned().unwrap_or(Value::Null),
            },
            "state": capture_compact_page_state(page, false).await,
            "boundaries": type_result.get("boundaries").cloned().unwrap_or(json!([])),
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("autocomplete_select failed")
            .to_string())
    }
}

pub(super) async fn handle_click_ordered_values(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
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
    let order_json = json_literal(order);
    let max_clicks_json = json_literal(&max_clicks);
    let availability_helpers_js = availability_helpers_js();
    let js = format!(
        r#"(async () => {{
  const order = {order_json};
  const maxClicks = {max_clicks_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function allRoots(root = document) {{
    const roots = [];
    const queue = [root];
    const seen = new Set();
    while (queue.length) {{
      const current = queue.shift();
      if (!current || seen.has(current)) continue;
      seen.add(current);
      roots.push(current);
      if (!current.querySelectorAll) continue;
      for (const el of Array.from(current.querySelectorAll('*'))) {{
        if (el.shadowRoot) queue.push(el.shadowRoot);
        if (el.tagName === 'IFRAME') {{
          try {{
            if (el.contentDocument) queue.push(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    return roots;
  }}
  function all(query) {{
    const out = [];
    const seen = new Set();
    for (const root of allRoots()) {{
      if (!root.querySelectorAll) continue;
      for (const el of Array.from(root.querySelectorAll(query))) {{
        if (seen.has(el)) continue;
        seen.add(el);
        out.push(el);
      }}
    }}
    return out;
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
    for (const el of all(query)) {{
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
                crate::daemon::handlers::clean_cdp_error(&e)
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

pub(super) async fn handle_select_menu_path(page: &Page, params: &Value) -> Result<Value, String> {
    let path = params
        .get("path")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing required parameter: path".to_string())?
        .iter()
        .filter_map(|value| value.as_str().map(|text| text.trim().to_string()))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if path.is_empty() {
        return Err("select_menu_path requires at least one path item".to_string());
    }
    let path_json = json_literal(&path);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let text_matcher_js = super::planner_js::text_matcher_js();
    let js = format!(
        r#"(async () => {{
  const path = {path_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  {accessible_text_helpers_js}
  {text_matcher_js}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
      if (parts.length >= 6) break;
    }}
    return parts.join(' > ');
  }}
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope.matches && scope.matches(selectorText) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function menuRole(el) {{
    return String(el && el.getAttribute ? el.getAttribute('role') || '' : '').toLowerCase();
  }}
  function classText(el) {{
    return String(el && el.className || '');
  }}
  function iconSemanticText(el) {{
    const nodes = [el].concat(Array.from(el.querySelectorAll ? el.querySelectorAll('[class*=icon], [class*=Icon], svg, use') : []));
    const out = [];
    const aliases = {{
      disk: 'save disk',
      save: 'save disk',
      play: 'play start',
      stop: 'stop',
      seekstart: 'seek start previous prev rewind beginning first',
      seekend: 'seek end next forward last',
      stepbackward: 'step backward previous prev',
      stepforward: 'step forward next',
      backward: 'back previous prev',
      forward: 'forward next',
      prev: 'previous prev back',
      previous: 'previous prev back',
      next: 'next forward',
      zoomin: 'zoom in plus magnify',
      zoomout: 'zoom out minus magnify',
      plus: 'plus add zoom in',
      minus: 'minus remove zoom out',
      print: 'print',
      trash: 'trash delete remove',
      delete: 'delete trash remove',
      close: 'close dismiss x',
      search: 'search find magnify',
    }};
    for (const node of nodes.slice(0, 24)) {{
      const raw = [
        classText(node),
        node.getAttribute && semanticAttributeText(node),
        node.getAttribute && node.getAttribute('href'),
        node.getAttribute && node.getAttribute('xlink:href'),
        node.getAttribute && node.getAttribute('data-icon'),
        node.getAttribute && node.getAttribute('icon'),
        node.getAttribute && node.getAttribute('aria-label'),
        node.getAttribute && node.getAttribute('title'),
        svgReferenceText(node),
      ].filter(Boolean).join(' ');
      for (const token of raw.split(/\s+/)) {{
        const cleaned = token
          .replace(/^#/, '')
          .replace(/^(?:ui-icon-|fa-|fas-|far-|fal-|fab-|icon-|Icon-|lucide-|mdi-|material-icons?-?)/, '')
          .replace(/[^A-Za-z0-9]+/g, ' ')
          .trim();
        if (!cleaned) continue;
        out.push(token, cleaned);
        const compact = cleaned.replace(/\s+/g, '').toLowerCase();
        if (aliases[compact]) out.push(aliases[compact]);
      }}
    }}
    return out.join(' ');
  }}
  function isMenuItemNode(el) {{
    const role = menuRole(el);
    const cls = classText(el);
    return ['menuitem', 'menuitemcheckbox', 'menuitemradio', 'treeitem', 'option', 'tab'].includes(role) ||
      /ui-menu-item-wrapper|ui-menu-item|menuitem|menu-item|treeitem|tree-item|option|tab/i.test(cls);
  }}
  function cleanLabel(text) {{
    return String(text || '')
      .replace(/\b(menuitem|menu item|treeitem|tree item|option|tab)\b/ig, ' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function preferredMenuTarget(el) {{
    if (!el) return el;
    const child = Array.from(el.children || []).find(isMenuItemNode);
    if (/ui-menu-item\b/.test(classText(el)) && child) return child;
    if (isMenuItemNode(el) && menuRole(el) !== 'menu') return el;
    return child || el;
  }}
  function directText(el) {{
    if (!el) return '';
    const semantic = [
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      semanticAttributeText(el),
      slotText(el),
      svgReferenceText(el),
    ].join(' ');
    const direct = Array.from(el.childNodes)
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .trim();
    if (direct) return cleanLabel([direct, semantic].join(' '));
    const wrapper = Array.from(el.children || []).find(isMenuItemNode);
    if (wrapper) return directText(wrapper);
    return cleanLabel([el.textContent || '', semantic].join(' '));
  }}
  function isMenuCandidate(el) {{
    const tag = el.tagName.toLowerCase();
    const role = menuRole(el);
    const cls = classText(el);
    const style = getComputedStyle(el);
    return tag === 'li' || tag === 'a' || tag === 'button' ||
      isMenuItemNode(el) ||
      (['div', 'span'].includes(tag) && (role || /ui-menu-item|ui-menu-item-wrapper|menuitem|menu-item|option|tree|tab/i.test(cls) || style.cursor === 'pointer'));
  }}
  function scoreCandidate(el, label, depth) {{
    if (!visible(el) || !isMenuCandidate(el)) return 0;
    const target = preferredMenuTarget(el);
    if (!visible(target) || !isMenuCandidate(target)) return 0;
    const role = menuRole(target);
    const tag = target.tagName.toLowerCase();
    const cls = classText(target);
    if (role === 'menu' || role === 'menubar' || tag === 'ul' || tag === 'ol' || target.id === 'area') return 0;
    const text = [directText(target), iconSemanticText(target)].join(' ');
    const nText = norm(text);
    const nLabel = norm(label);
    if (!nText || !nLabel) return 0;
    let score = 0;
    if (nText === nLabel) score += 1;
    else if (nText.startsWith(nLabel + ' ')) score += 0.78;
    else if (nText.includes(nLabel)) score += 0.55;
    else if (exactPhraseScore(label, text) > 0) score += exactPhraseScore(label, text) * 0.85;
    else if (tokenScore(label, text) > 0) score += tokenScore(label, text) * 0.65;
    else return 0;
    if (nText !== nLabel && nText.length > nLabel.length + 24) score -= 0.45;
    if (nText.split(' ').length > 5 && nText !== nLabel) score -= 0.35;
    if (/ui-menu-item-wrapper/.test(cls) || role.includes('menuitem')) score += 0.35;
    else if (/ui-menu-item/.test(cls)) score += 0.2;
    if (target.closest('[role=menu], [role=menubar], [role=tree], ul, nav, .ui-menu')) score += 0.15;
    if (target !== el) score += 0.05;
    const rect = target.getBoundingClientRect();
    score -= Math.max(0, rect.top) / 100000;
    score -= depth * 0.001;
    return score;
  }}
  function candidates(label, depth) {{
    const query = [
      '[role=menuitem]', '[role=menuitemcheckbox]', '[role=menuitemradio]', '[role=treeitem]', '[role=option]', '[role=tab]',
      '.ui-menu-item-wrapper', '.ui-menu-item', '[class*=menuitem]', '[class*=menu-item]', '[class*=MenuItem]', 'li', 'a', 'button', '[onclick]', '[tabindex]',
      'div[role]', 'span[role]', 'div[class*=menu]', 'span[class*=menu]', 'div[class*=Menu]', 'span[class*=Menu]', 'div[class*=option]', 'span[class*=option]', 'div[class*=tab]', 'span[class*=tab]'
    ].join(',');
    return all(query)
      .map(el => {{
        const target = preferredMenuTarget(el);
        return {{ el: target, score: scoreCandidate(el, label, depth) }};
      }})
      .filter(item => item.score > 0)
      .filter((item, index, items) => items.findIndex(other => other.el === item.el) === index)
      .sort((a, b) => b.score - a.score);
  }}
  function eventInit(el) {{
    const r = el.getBoundingClientRect();
    return {{ bubbles: true, cancelable: true, view: window, clientX: r.left + Math.max(1, r.width / 2), clientY: r.top + Math.max(1, r.height / 2) }};
  }}
  function jQueryMenuWidget(el, init) {{
    const jq = window.jQuery || window.$;
    if (!jq || !jq.fn || !jq.fn.menu) return null;
    const row = el.closest('li');
    if (!row) return null;
    let menu = row.closest('.ui-menu');
    while (menu && menu.parentElement) {{
      const parentMenu = menu.parentElement.closest('.ui-menu');
      if (!parentMenu) break;
      menu = parentMenu;
    }}
    if (!menu) return null;
    let instance = null;
    try {{ instance = jq(menu).menu('instance'); }} catch (_) {{}}
    if (!instance) return null;
    const event = jq.Event('mousemove');
    event.target = el;
    event.currentTarget = row;
    event.pageX = init.clientX + window.scrollX;
    event.pageY = init.clientY + window.scrollY;
    return {{ jq, row: jq(row), instance, event }};
  }}
  function activate(el, finalStep) {{
    const init = eventInit(el);
    const nodes = [
      el,
      el.closest('[role=menuitem], [role=treeitem], .ui-menu-item-wrapper'),
      el.closest('li'),
      el.closest('.ui-menu-item')
    ].filter((node, index, all) => node && all.indexOf(node) === index);
    for (const node of nodes) {{
      for (const type of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'mousemove']) {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        node.dispatchEvent(event);
      }}
    }}
    const widget = jQueryMenuWidget(el, init);
    if (widget) {{
      try {{ widget.instance.focus(widget.event, widget.row); }} catch (_) {{}}
      if (!finalStep) {{
        try {{ widget.instance.expand(widget.event); }} catch (_) {{}}
      }}
    }}
    if (el.focus) {{
      try {{ el.focus(); }} catch (_) {{}}
    }}
    if (finalStep) {{
      el.dispatchEvent(new MouseEvent('mousedown', init));
      el.dispatchEvent(new MouseEvent('mouseup', init));
      el.dispatchEvent(new MouseEvent('click', init));
      if (widget) {{
        try {{ widget.instance.select(widget.event); }} catch (_) {{}}
      }}
    }}
  }}
  const selected = [];
  for (let index = 0; index < path.length; index++) {{
    const label = path[index];
    const finalStep = index === path.length - 1;
    const match = candidates(label, index)[0];
    if (!match) return {{ ok: false, error: 'select_menu_path could not find visible menu item: ' + label, path, selected }};
    const target = match.el;
    activate(target, finalStep);
    selected.push({{ label, selector: selector(target), text: directText(target), score: match.score }});
    await delay(finalStep ? 120 : 180);
  }}
  return {{ ok: true, path, selected }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "select_menu_path timed out".to_string())?
        .map_err(|e| {
            format!(
                "select_menu_path failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "menuPath": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("select_menu_path failed")
            .to_string())
    }
}

pub(super) async fn handle_scoped_menu_click(page: &Page, params: &Value) -> Result<Value, String> {
    let container = params
        .get("container")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "scoped_menu_click requires container".to_string())?;
    let action_hint = params
        .get("action_hint")
        .or_else(|| params.get("actionHint"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| "scoped_menu_click requires action_hint".to_string())?;
    let trigger = params.get("trigger").and_then(|value| value.as_str());
    let container_json = json_literal(container);
    let trigger_json = json_literal(&trigger);
    let action_hint_json = json_literal(action_hint);
    let availability_helpers_js = availability_helpers_js();
    let js = format!(
        r#"async () => {{
  const containerSelector = {container_json};
  const triggerSelector = {trigger_json};
  const actionHint = {action_hint_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const root of allRoots(start)) {{
      try {{
        if (root.matches && root.matches(selectorText) && !seen.has(root)) {{
          seen.add(root);
          results.push(root);
        }}
        const matches = root.querySelectorAll ? Array.from(root.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function findOne(selectorText, start = document) {{
    if (!selectorText) return null;
    return all(selectorText, start)[0] || null;
  }}
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
  function textOf(el) {{
    if (!el) return '';
    return [
      el.innerText || el.textContent || '',
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('class') || '',
      el.id || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function normalized(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9@]+/g, ' ').replace(/\s+/g, ' ').trim();
  }}
  function tokens(text) {{
    return String(text || '').match(/[A-Za-z0-9@]+/g)?.map(token => token.toLowerCase()).filter(Boolean) || [];
  }}
  function tokenScore(hint, text) {{
    const wanted = tokens(hint);
    if (!wanted.length) return 0;
    const have = new Set(tokens(text));
    return wanted.filter(token => have.has(token)).length / wanted.length;
  }}
  function scoreTarget(el) {{
    const text = textOf(el);
    const direct = normalized(text);
    const wanted = normalized(actionHint);
    let score = 0;
    if (direct === wanted) score += 2;
    if (wanted && direct.includes(wanted)) score += 1.3;
    score += tokenScore(actionHint, text);
    const role = String(el.getAttribute && el.getAttribute('role') || '').toLowerCase();
    const tag = el.tagName.toLowerCase();
    const cls = String(el.getAttribute('class') || '');
    if (score > 0 && (tag === 'li' || role.includes('menuitem') || role === 'option' || /menu|option|item/i.test(cls))) score += 0.35;
    return score;
  }}
  function eventInit(el) {{
    const rect = el.getBoundingClientRect();
    return {{
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      clientX: rect.left + Math.max(1, rect.width / 2),
      clientY: rect.top + Math.max(1, rect.height / 2)
    }};
  }}
  function clickLike(el) {{
    if (!el) return;
    if (el.scrollIntoView) {{
      try {{ el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }}); }} catch (_) {{}}
    }}
    const init = eventInit(el);
    for (const type of ['pointerover', 'mouseover', 'pointerenter', 'mouseenter', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
      el.dispatchEvent(event);
    }}
  }}
  const container = findOne(containerSelector);
  if (!container) return {{ ok: false, error: 'scoped_menu_click container not found', container: containerSelector }};
  if (container.scrollIntoView) {{
    try {{ container.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'instant' }}); }} catch (_) {{}}
  }}
  await delay(50);
  const trigger = findOne(triggerSelector, container) || findOne(triggerSelector) ||
    all('button, a, [role=button], [aria-haspopup], [aria-expanded], [onclick], [tabindex], .more, [class*=more], [class*=More], [class*=menu], [class*=Menu], [class*=overflow], [class*=Overflow], [class*=ellipsis], [class*=Ellipsis], span, div', container)
      .filter(el => el !== container && visible(el))
      .sort((a, b) => {{
        const as = /\b(more|menu|overflow|actions?|options?|ellipsis|kebab|dropdown|expand|open)\b/i.test(textOf(a)) ? 0 : 1;
        const bs = /\b(more|menu|overflow|actions?|options?|ellipsis|kebab|dropdown|expand|open)\b/i.test(textOf(b)) ? 0 : 1;
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return as - bs || (br.top - ar.top) || (br.left - ar.left);
      }})[0];
  if (!trigger) return {{ ok: false, error: 'scoped_menu_click trigger not found', container: containerSelector }};
  clickLike(trigger);
  await delay(120);
  const candidates = all('li, [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], [role=treeitem], .ui-menu-item, .ui-menu-item-wrapper, button, a, [role=button]', container)
    .filter(el => el !== container)
    .map(el => ({{ el, score: scoreTarget(el), visible: visible(el), text: textOf(el) }}))
    .filter(item => item.score > 0 && item.visible)
    .sort((a, b) => b.score - a.score);
  if (!candidates.length) {{
    return {{
      ok: false,
      error: 'scoped_menu_click target not found after reveal',
      actionHint,
      triggerText: textOf(trigger),
      visibleMenuText: all('li, [role=menuitem], [role=option], button, a', container).filter(visible).map(textOf).slice(0, 20)
    }};
  }}
  const target = candidates[0].el;
  clickLike(target);
  await delay(80);
  return {{
    ok: true,
    container: containerSelector,
    trigger: {{ text: textOf(trigger) }},
    target: {{ text: textOf(target), score: candidates[0].score }}
  }};
}}"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_function(&js))
        .await
        .map_err(|_| "scoped_menu_click timed out".to_string())?
        .map_err(|e| {
            format!(
                "scoped_menu_click failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "scopedMenuClick": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("scoped_menu_click failed")
            .to_string())
    }
}

pub(super) async fn handle_set_slider(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let value = params
        .get("value")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing required parameter: value".to_string())?;
    let selector_json = json_literal(selector);
    let value_json = json_literal(&value);
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(() => {{
  const selector = {selector_json};
  const desired = {value_json};
  function isReadOnlyControl(el) {{
    if (!el || !el.getAttribute) return false;
    return el.disabled || el.readOnly || el.getAttribute('aria-readonly') === 'true' || el.getAttribute('aria-disabled') === 'true';
  }}
  function isCustomWritableValueElement(_el) {{
    return false;
  }}
  {value_control_helpers_js}
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function findOne(selectorText) {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const match = root.querySelector && root.querySelector(selectorText);
        if (match) return match;
      }} catch (_) {{}}
    }}
    return null;
  }}
  const el = findOne(selector);
  if (!el) return {{ ok: false, error: 'slider not found: ' + selector }};
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
  if (el.matches('input[type=range]') || 'value' in el || el.getAttribute('role') === 'slider') {{
    try {{
      const ok = setControlValue(el, desired);
      if (!ok) return {{ ok: false, error: 'unable to set slider value' }};
      const observed = Number(readControlValue(el) || el.getAttribute('aria-valuenow'));
      let mode = 'custom-value-slider';
      if (el.matches('input[type=range]')) mode = 'native-range';
      else if (el.getAttribute('role') === 'slider' && !('value' in el)) mode = 'aria';
      return {{
        ok: true,
        selector,
        value: Number.isFinite(observed) ? observed : desired,
        mode
      }};
    }} catch (error) {{
      return {{ ok: false, error: String(error && error.message || error) }};
    }}
  }}
  return {{ ok: false, error: 'matched element is not a supported slider' }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "set_slider timed out".to_string())?
        .map_err(|e| {
            format!(
                "set_slider failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
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

pub(super) async fn handle_scroll_element(page: &Page, params: &Value) -> Result<Value, String> {
    let selector = params
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: selector".to_string())?;
    let direction = params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down");
    let selector_json = json_literal(selector);
    let direction_json = json_literal(direction);
    let js = format!(
        r#"(() => {{
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const current of tree) {{
        if (current.shadowRoot) add(current.shadowRoot);
        if (current.tagName && current.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (current.contentDocument) add(current.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function findOne(selectorText) {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const match = root.querySelector && root.querySelector(selectorText);
        if (match) return match;
      }} catch (_) {{}}
    }}
    return null;
  }}
  const direction = {direction_json};
  const el = findOne({selector_json});
  if (!el) return {{ ok: false, error: 'scroll_element target not found' }};
  if (/\b(?:up|top|start|beginning)\b/i.test(direction)) {{
    el.scrollTop = 0;
  }} else if (/\b(?:middle|center|centre)\b/i.test(direction)) {{
    el.scrollTop = Math.max(0, (el.scrollHeight - el.clientHeight) / 2);
  }} else {{
    el.scrollTop = el.scrollHeight;
  }}
  el.dispatchEvent(new Event('scroll', {{ bubbles: true }}));
  return {{ ok: true, selector: {selector_json}, direction, scrollTop: el.scrollTop, scrollHeight: el.scrollHeight }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "scroll_element timed out".to_string())?
        .map_err(|e| {
            format!(
                "scroll_element failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
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

pub(super) async fn handle_scroll_text_extract(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "scroll_text_extract requires source".to_string())?;
    let target = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "scroll_text_extract requires target".to_string())?;
    let which = params
        .get("which")
        .and_then(|v| v.as_str())
        .unwrap_or("last");
    let source_json = json_literal(source);
    let target_json = json_literal(target);
    let which_json = json_literal(which);
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(() => {{
  const sourceSelector = {source_json};
  const targetSelector = {target_json};
  const which = {which_json};
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{ if (scope.contentDocument) add(scope.contentDocument); }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const current of tree) {{
        if (current.shadowRoot) add(current.shadowRoot);
        if (current.tagName && current.tagName.toLowerCase() === 'iframe') {{
          try {{ if (current.contentDocument) add(current.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function findOne(selectorText) {{
    for (const root of allRoots(document)) {{
      try {{
        if (root.matches && root.matches(selectorText)) return root;
        const match = root.querySelector && root.querySelector(selectorText);
        if (match) return match;
      }} catch (_) {{}}
    }}
    return null;
  }}
	  function textValue(el) {{
	    if (!el) return '';
	    const tag = el.tagName ? el.tagName.toLowerCase() : '';
	    if (tag === 'textarea' || tag === 'input') return String(el.value || '');
	    return String(el.innerText || el.textContent || '');
	  }}
	  {value_control_helpers_js}
	  function setValue(el, value) {{
	    return setControlValue(el, value, {{ inputType: 'insertText' }});
	  }}
  const source = findOne(sourceSelector);
  const target = findOne(targetSelector);
  if (!source) return {{ ok: false, error: 'scroll_text_extract source not found' }};
  if (!target) return {{ ok: false, error: 'scroll_text_extract target not found' }};
  if (/\b(?:first|top|start|beginning)\b/i.test(which)) source.scrollTop = 0;
  else source.scrollTop = source.scrollHeight;
  source.dispatchEvent(new Event('scroll', {{ bubbles: true }}));
  const words = textValue(source).trim().split(/\s+/).filter(Boolean);
  if (!words.length) return {{ ok: false, error: 'scroll_text_extract source has no words' }};
  const value = /\b(?:first|top|start|beginning)\b/i.test(which) ? words[0] : words[words.length - 1];
  setValue(target, value);
  return {{
    ok: true,
    source: sourceSelector,
    target: targetSelector,
    which,
    value,
    wordCount: words.length,
    scrollTop: source.scrollTop
  }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "scroll_text_extract timed out".to_string())?
        .map_err(|e| {
            format!(
                "scroll_text_extract failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "scrollTextExtract": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("scroll_text_extract failed")
            .to_string())
    }
}

pub(super) async fn handle_draw_path(page: &Page, params: &Value) -> Result<Value, String> {
    let raw_points = params
        .get("points")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "draw_path requires points".to_string())?;
    if raw_points.len() < 2 {
        return Err("draw_path requires at least two points".to_string());
    }
    if raw_points.len() > 240 {
        return Err("draw_path accepts at most 240 points".to_string());
    }

    let mut points = Vec::with_capacity(raw_points.len());
    for point in raw_points {
        let x = point
            .get("x")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| "draw_path point is missing x".to_string())?;
        let y = point
            .get("y")
            .and_then(|value| value.as_f64())
            .ok_or_else(|| "draw_path point is missing y".to_string())?;
        if !x.is_finite() || !y.is_finite() {
            return Err("draw_path points must be finite coordinates".to_string());
        }
        points.push((x, y));
    }

    let button_name = params
        .get("button")
        .and_then(|value| value.as_str())
        .unwrap_or("left");
    let button = mouse_button(Some(button_name))?;
    let button_mask = mouse_buttons_mask_for_button(&button);
    let delay_ms = params
        .get("step_delay_ms")
        .or_else(|| params.get("stepDelayMs"))
        .and_then(|value| value.as_u64())
        .unwrap_or(8)
        .min(100);
    let (start_x, start_y) = points[0];

    dispatch_mouse(
        page,
        DispatchMouseEventType::MouseMoved,
        start_x,
        start_y,
        MouseButton::None,
        0,
        0,
        0,
        None,
        None,
    )
    .await
    .map_err(|error| format!("draw_path: move to start failed: {error}"))?;

    dispatch_mouse(
        page,
        DispatchMouseEventType::MousePressed,
        start_x,
        start_y,
        button.clone(),
        button_mask,
        1,
        0,
        None,
        None,
    )
    .await
    .map_err(|error| format!("draw_path: mouse down failed: {error}"))?;

    for (index, (x, y)) in points.iter().copied().enumerate().skip(1) {
        dispatch_mouse(
            page,
            DispatchMouseEventType::MouseMoved,
            x,
            y,
            MouseButton::None,
            button_mask,
            0,
            0,
            None,
            None,
        )
        .await
        .map_err(|error| format!("draw_path: move step {index} failed: {error}"))?;
        if delay_ms > 0 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    let (end_x, end_y) = points[points.len() - 1];
    dispatch_mouse(
        page,
        DispatchMouseEventType::MouseReleased,
        end_x,
        end_y,
        button,
        0,
        1,
        0,
        None,
        None,
    )
    .await
    .map_err(|error| format!("draw_path: mouse up failed: {error}"))?;

    Ok(json!({
        "drawPath": {
            "points": points.len(),
            "button": button_name,
            "from": { "x": start_x, "y": start_y },
            "to": { "x": end_x, "y": end_y }
        },
        "state": capture_compact_page_state(page, false).await,
    }))
}

pub(super) async fn handle_orient_visual(page: &Page, params: &Value) -> Result<Value, String> {
    let target_text = params
        .get("targetText")
        .or_else(|| params.get("target_text"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "orient_visual requires targetText".to_string())?;
    let max_attempts = params
        .get("maxAttempts")
        .or_else(|| params.get("max_attempts"))
        .and_then(|value| value.as_u64())
        .unwrap_or(28)
        .clamp(1, 64) as usize;
    let selector = params.get("selector").and_then(|value| value.as_str());
    let selector_json = json_literal(&selector);
    let target_json = json_literal(target_text);
    let availability_helpers_js = availability_helpers_js();

    async fn drag_between(
        page: &Page,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
    ) -> Result<(), String> {
        let button = mouse_button(Some("left"))?;
        let button_mask = mouse_buttons_mask_for_button(&button);
        let steps = 28;

        dispatch_mouse(
            page,
            DispatchMouseEventType::MouseMoved,
            from_x,
            from_y,
            MouseButton::None,
            0,
            0,
            0,
            None,
            None,
        )
        .await
        .map_err(|error| format!("orient_visual: move to surface failed: {error}"))?;

        dispatch_mouse(
            page,
            DispatchMouseEventType::MousePressed,
            from_x,
            from_y,
            button.clone(),
            button_mask,
            1,
            0,
            None,
            None,
        )
        .await
        .map_err(|error| format!("orient_visual: mouse down failed: {error}"))?;

        for index in 1..=steps {
            let ratio = index as f64 / steps as f64;
            let x = from_x + (to_x - from_x) * ratio;
            let y = from_y + (to_y - from_y) * ratio;
            dispatch_mouse(
                page,
                DispatchMouseEventType::MouseMoved,
                x,
                y,
                MouseButton::None,
                button_mask,
                0,
                0,
                None,
                None,
            )
            .await
            .map_err(|error| format!("orient_visual: move step {index} failed: {error}"))?;
            sleep(Duration::from_millis(8)).await;
        }

        dispatch_mouse(
            page,
            DispatchMouseEventType::MouseReleased,
            to_x,
            to_y,
            button,
            0,
            1,
            0,
            None,
            None,
        )
        .await
        .map_err(|error| format!("orient_visual: mouse up failed: {error}"))?;
        Ok(())
    }

    let inspect = |attempt: usize| {
        format!(
            r#"(() => {{
  const requestedSelector = {selector_json};
  const targetText = {target_json};
  const attempt = {attempt};
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  const targetNorm = norm(targetText);
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 &&
      style.display !== 'none' && style.visibility !== 'hidden' &&
      Number(style.opacity || 1) !== 0;
  }}
  function textOf(el) {{
    return [
      el.textContent || '',
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('alt') || '')
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function directTextOf(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
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
  function matchesTarget(text) {{
    const value = norm(text);
    return !!value && (value === targetNorm || value.includes(targetNorm) || targetNorm.includes(value));
  }}
  function stateFor(container) {{
    const rect = container.getBoundingClientRect();
    const allChildren = Array.from(container.querySelectorAll('*')).filter(visible);
    const active = allChildren.filter(el => {{
      const className = String(el.className || '');
      const ariaSelected = el.getAttribute('aria-selected') === 'true';
      const ariaCurrent = el.getAttribute('aria-current') === 'true';
      const dataActive = el.getAttribute('data-active') === 'true' || el.getAttribute('data-state') === 'active';
      return ariaSelected || ariaCurrent || dataActive || /\b(active|current|front|selected)\b/i.test(className);
    }});
    const faceTexts = allChildren
      .map(el => directTextOf(el) || textOf(el))
      .map(text => text.replace(/\s+/g, ' ').trim())
      .filter(text => text && text.length <= 80);
    const activeTexts = active
      .map(el => directTextOf(el) || textOf(el))
      .map(text => text.replace(/\s+/g, ' ').trim())
      .filter(Boolean);
    return {{
      selector: selectorFor(container),
      rect: {{ x: rect.left, y: rect.top, width: rect.width, height: rect.height }},
      activeTexts,
      faceTexts: Array.from(new Set(faceTexts)).slice(0, 24),
      matched: activeTexts.some(matchesTarget),
      targetPresent: faceTexts.some(matchesTarget),
      cue: [
        container.id || '',
        String(container.className || ''),
        container.getAttribute('role') || '',
        container.getAttribute('aria-roledescription') || '',
        container.getAttribute('aria-label') || '',
        container.getAttribute('data-testid') || ''
      ].join(' ')
    }};
  }}
  function candidateScore(container) {{
    if (!visible(container)) return -1;
    const rect = container.getBoundingClientRect();
    const area = rect.width * rect.height;
    if (rect.width < 20 || rect.height < 20 || area < 400 || area > 500000) return -1;
    const state = stateFor(container);
    let score = 0;
    if (state.targetPresent) score += 0.55;
    if (state.activeTexts.length) score += 0.35;
    if (/\b(rotat|orient|spin|turn|cube|face|side|carousel|viewport|viewer|dial|object|surface|stage)\b/i.test(state.cue)) score += 0.3;
    if (state.faceTexts.length >= 2) score += Math.min(0.25, state.faceTexts.length * 0.035);
    const tag = container.tagName.toLowerCase();
    if (tag === 'canvas' || tag === 'svg') score += 0.12;
    if (tag === 'body' || tag === 'html') score -= 0.6;
    return score;
  }}
  let chosen = null;
  if (requestedSelector) {{
    const requested = document.querySelector(requestedSelector);
    if (!requested) return {{ ok: false, error: 'orient_visual target selector not found', selector: requestedSelector }};
    chosen = requested;
  }} else {{
    const candidates = Array.from(document.querySelectorAll('canvas, svg, [role=img], [aria-roledescription], [data-active], [data-state], [class*=rotat i], [class*=orient i], [class*=cube i], [class*=carousel i], [class*=viewport i], [class*=viewer i], [class*=stage i], [class*=surface i], section, article, div'))
      .map(el => ({{ el, score: candidateScore(el) }}))
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    if (!candidates.length) return {{ ok: false, error: 'orient_visual could not find a visual surface with active/front state' }};
    chosen = candidates[0].el;
  }}
  const state = stateFor(chosen);
  return {{
    ok: true,
    attempt,
    selector: state.selector,
    rect: state.rect,
    activeTexts: state.activeTexts,
    faceTexts: state.faceTexts,
    matched: state.matched,
    targetPresent: state.targetPresent
  }};
}})()"#
        )
    };

    let mut observations = Vec::new();
    let mut last_state = json!({});
    let gestures = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (0.85, 0.45),
        (-0.85, -0.45),
        (0.45, 0.85),
        (-0.45, -0.85),
    ];

    for attempt in 0..=max_attempts {
        let js = inspect(attempt);
        let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
            .await
            .map_err(|_| "orient_visual timed out while inspecting state".to_string())?
            .map_err(|error| {
                format!(
                    "orient_visual failed while inspecting state: {}",
                    crate::daemon::handlers::clean_cdp_error(&error)
                )
            })?;
        let state = result.value().cloned().unwrap_or_else(|| json!({}));
        if !state
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Err(state
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("orient_visual could not inspect visual state")
                .to_string());
        }
        last_state = state.clone();
        observations.push(json!({
            "attempt": attempt,
            "activeTexts": state.get("activeTexts").cloned().unwrap_or_else(|| json!([])),
        }));
        if state
            .get("matched")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Ok(json!({
                "orientedVisual": {
                    "targetText": target_text,
                    "selector": state.get("selector").cloned().unwrap_or(Value::Null),
                    "attempts": attempt,
                    "activeTexts": state.get("activeTexts").cloned().unwrap_or_else(|| json!([])),
                    "observations": observations,
                },
                "state": capture_compact_page_state(page, false).await,
            }));
        }
        if attempt == max_attempts {
            break;
        }

        let rect = state
            .get("rect")
            .and_then(|value| value.as_object())
            .ok_or_else(|| "orient_visual state is missing surface bounds".to_string())?;
        let x = rect
            .get("x")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let y = rect
            .get("y")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let width = rect
            .get("width")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let height = rect
            .get("height")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        if width <= 0.0 || height <= 0.0 {
            return Err("orient_visual surface has invalid bounds".to_string());
        }
        let center_x = x + width / 2.0;
        let center_y = y + height / 2.0;
        let distance = width.max(height).clamp(80.0, 260.0);
        let (dx, dy) = gestures[attempt % gestures.len()];
        let from_x = center_x - dx * distance * 0.45;
        let from_y = center_y - dy * distance * 0.45;
        let to_x = center_x + dx * distance * 0.45;
        let to_y = center_y + dy * distance * 0.45;
        drag_between(page, from_x, from_y, to_x, to_y).await?;
        sleep(Duration::from_millis(450)).await;
    }

    Err(format!(
        "orient_visual could not make {target_text:?} active/front after {max_attempts} gestures; last active texts: {}",
        last_state
            .get("activeTexts")
            .cloned()
            .unwrap_or_else(|| json!([]))
    ))
}
pub(super) async fn handle_derive_and_act(page: &Page, params: &Value) -> Result<Value, String> {
    let params_json = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(async () => {{
  const params = {params_json};
  const instruction = String(params.instruction || '');
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      try {{ if (document.querySelectorAll(byName).length === 1) return byName; }} catch (_) {{}}
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 6) {{
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
    }}
    return parts.join(' > ');
  }}
  function all(query, root = document) {{
    const out = [];
    const seen = new Set();
    function collect(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      try {{
        if (scope.matches && scope.matches(query)) out.push(scope);
        if (scope.querySelectorAll) out.push(...Array.from(scope.querySelectorAll(query)));
      }} catch (_) {{}}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) collect(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) collect(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    collect(root);
    return Array.from(new Set(out));
  }}
	  function directText(el) {{
	    return Array.from(el.childNodes || [])
	      .filter(node => node.nodeType === Node.TEXT_NODE)
	      .map(node => node.textContent || '')
	      .join(' ')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
  {accessible_text_helpers_js}
  {control_semantics_helpers_js}
  {value_control_helpers_js}
	  function textOf(el) {{
	    return [
	      directText(el),
	      el.textContent || '',
	      el.value || '',
	      el.getAttribute && (el.getAttribute('aria-label') || ''),
	      el.getAttribute && (el.getAttribute('title') || ''),
	      el.getAttribute && (el.getAttribute('data-value') || ''),
	      el.getAttribute && (el.getAttribute('value') || ''),
	      el.getAttribute && (el.getAttribute('name') || ''),
	      el.placeholder || '',
	      referencedText(el, 'aria-labelledby'),
	      referencedText(el, 'aria-describedby'),
	      el.getAttribute && (el.getAttribute('aria-description') || ''),
	      semanticAttributeText(el),
	      associatedLabelText(el),
	      structuralLabelText(el),
	      nearbyLabelText(el),
	      shadowHostText(el),
	      slotText(el),
	    ].join(' ').replace(/\s+/g, ' ').trim();
	  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
	  function isReadOnlyControl(el) {{
	    return !!el.readOnly ||
	      el.getAttribute('readonly') !== null ||
	      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
	  }}
	  function writableField(el) {{
	    return visible(el) && isWritableValueControl(el);
	  }}
	  function fields() {{
	    return all(valueControlSelector())
	      .concat(all('*').filter(isCustomWritableValueElement))
	      .filter(writableField);
	  }}
	  function setValue(el, value) {{
	    return setControlValue(el, value);
	  }}
  function click(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
    try {{ el.click(); }} catch (_) {{}}
  }}
  function submitLike(anchor) {{
    const controls = all('button, input[type=submit], input[type=button], a, [role=button], [onclick], [tabindex]')
      .filter(visible)
      .map(el => {{
        const text = textOf(el).toLowerCase();
        let score = 0;
        if (/\b(submit|done|ok|send|search|save|continue|confirm)\b/.test(text)) score += 1;
        if ((el.getAttribute('type') || '').toLowerCase() === 'submit') score += 1;
        if (anchor) {{
          const form = anchor.closest && anchor.closest('form');
          if (form && form.contains(el)) score += 0.5;
          if (anchor.parentElement && anchor.parentElement.contains(el)) score += 0.2;
        }}
        return {{ el, score }};
      }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    return controls[0] && controls[0].el;
  }}
  function ordinalIndex(text) {{
    const raw = String(text || '').toLowerCase();
    if (/\blast\b/.test(raw)) return -1;
    const words = {{
      first: 0, second: 1, third: 2, fourth: 3, fifth: 4,
      sixth: 5, seventh: 6, eighth: 7, ninth: 8, tenth: 9
    }};
    for (const [word, index] of Object.entries(words)) {{
      if (new RegExp('(?:^|\\\\b)' + word + '(?:\\\\b|$)').test(raw)) return index;
    }}
    const match = raw.match(/\b(\d+)(?:st|nd|rd|th)?\b/);
    if (!match) return null;
    const index = Number(match[1]) - 1;
    return Number.isFinite(index) && index >= 0 ? index : null;
  }}
  function ordinalClickTarget(hint) {{
    const index = ordinalIndex(hint || instruction);
    if (index == null) return null;
    const wants = String(hint || instruction).toLowerCase();
    const rejectControlText = el => {{
      const text = textOf(el);
      const meta = [text, el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
      return !text || /\b(search|submit|done|ok|send|save|continue|next|previous|prev|page|pagination)\b/i.test(meta);
    }};
    const scopedResultLinks = () => {{
      const scoped = all('#page-content a, [data-results] a, [data-result] a, [role=list] a, [role=feed] a, main a, section a, article a')
        .filter(visible)
        .filter(el => !rejectControlText(el))
        .filter((el, i, arr) => arr.indexOf(el) === i);
      scoped.sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      }});
      return scoped;
    }};
    if (/\bresult\b/i.test(wants)) {{
      const scopedLinks = scopedResultLinks();
      const scopedChoice = index === -1 ? scopedLinks[scopedLinks.length - 1] : scopedLinks[index];
      return scopedChoice || null;
    }}
    if (/\b(result|link)\b/i.test(wants)) {{
      const scopedLinks = scopedResultLinks();
      const scopedChoice = index === -1 ? scopedLinks[scopedLinks.length - 1] : scopedLinks[index];
      if (scopedChoice) return scopedChoice;
    }}
    let candidates = all('a, button, [role=link], [role=button], [onclick], [tabindex], tr, li, .result, .item, .card, .row, div')
      .filter(visible)
      .filter(el => {{
        const tag = el.tagName.toLowerCase();
        const text = textOf(el);
        if (rejectControlText(el)) return false;
        if (/\b(result|link)\b/i.test(wants)) return tag === 'a' || (el.getAttribute('role') || '').toLowerCase() === 'link' || /\bresult\b/i.test(String(el.className || ''));
        if (/\b(row|entry)\b/i.test(wants)) return tag === 'tr' || /\b(row|entry)\b/i.test(String(el.className || ''));
        if (/\b(card|item|option)\b/i.test(wants)) return tag === 'li' || /\b(card|item|option)\b/i.test(String(el.className || '')) || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
        return tag === 'a' || tag === 'button' || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
      }});
    candidates = candidates.filter(el => !candidates.some(other => other !== el && el.contains(other) && visible(other) && textOf(other)));
    candidates.sort((a, b) => {{
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.top - br.top || ar.left - br.left;
    }});
    if (!candidates.length) return null;
    return index === -1 ? candidates[candidates.length - 1] : candidates[index] || null;
  }}
  async function resolveOrdinalClickTarget(hint) {{
    const index = ordinalIndex(hint || instruction);
    if (index == null) return null;
    const wants = String(hint || instruction).toLowerCase();
    let target = ordinalClickTarget(hint);
    if (target || !/\bresult\b/i.test(wants) || index === -1) return target;
    const resultLinks = () => all('#page-content a, [data-results] a, [data-result] a, [role=list] a, [role=feed] a, main a, section a, article a')
      .filter(visible)
        .filter(el => {{
          const text = textOf(el);
          const meta = [text, el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
          return text && !/\b(search|submit|done|ok|send|save|continue|next|previous|prev|page|pagination)\b/i.test(meta);
        }})
      .filter((el, i, arr) => arr.indexOf(el) === i)
      .sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      }});
    const currentLinks = resultLinks();
    if (!currentLinks.length || index < currentLinks.length) return currentLinks[index] || null;
    const pageSize = currentLinks.length;
    const targetPageNumber = Math.floor(index / pageSize) + 1;
    const withinPageIndex = index % pageSize;
    const pageControls = all('a, button, [role=link], [role=button], [onclick], [tabindex]')
      .filter(visible)
      .filter(el => {{
        const text = textOf(el).trim();
        const meta = [text, classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
        return text === String(targetPageNumber) || /\b(next|more)\b|^>$/i.test(meta);
      }})
      .sort((a, b) => {{
        const at = textOf(a).trim() === String(targetPageNumber) ? 0 : 1;
        const bt = textOf(b).trim() === String(targetPageNumber) ? 0 : 1;
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return at - bt || ar.top - br.top || ar.left - br.left;
      }});
    if (!pageControls.length) return null;
    click(pageControls[0]);
    await delay(250);
    const afterLinks = resultLinks();
    return afterLinks[withinPageIndex] || null;
  }}
  function pageText() {{
    const chunks = [];
    for (const el of all('body, main, section, article, form, div, span, p, label, output, pre, code, svg text')) {{
      if (!visible(el) && el.tagName.toLowerCase() !== 'text') continue;
      const tag = el.tagName.toLowerCase();
      if (['script', 'style', 'button', 'input', 'textarea', 'select'].includes(tag)) continue;
      const text = directText(el) || (tag === 'text' ? textOf(el) : '');
      if (text) chunks.push(text);
    }}
    return chunks.join(' ').replace(/\s+/g, ' ').trim();
  }}
  function arithmeticValue() {{
    const text = pageText();
    function formatNumber(value) {{
      if (!Number.isFinite(value)) return null;
      const rounded = Math.round(value);
      if (Math.abs(value - rounded) < 1e-9) return String(rounded);
      return String(Number(value.toFixed(6))).replace(/\.0+$/, '');
    }}
    function solveEquation(left, op, right, result, xOnLeft) {{
      const known = Number(xOnLeft ? right : left);
      const target = Number(result);
      if (!Number.isFinite(known) || !Number.isFinite(target)) return null;
      if (xOnLeft) {{
        if (op === '+') return formatNumber(target - known);
        if (op === '-') return formatNumber(target + known);
        if (op === 'x' || op === '*') return known === 0 ? null : formatNumber(target / known);
        if (op === '/') return formatNumber(target * known);
      }} else {{
        if (op === '+') return formatNumber(target - known);
        if (op === '-') return formatNumber(known - target);
        if (op === 'x' || op === '*') return known === 0 ? null : formatNumber(target / known);
        if (op === '/') return target === 0 ? null : formatNumber(known / target);
      }}
      return null;
    }}
    let equation = text.match(/\bx\s*([+\-x*/])\s*(-?\d+)\s*=\s*(-?\d+)\b/i);
    if (equation) {{
      const answer = solveEquation('x', equation[1].toLowerCase(), equation[2], equation[3], true);
      if (answer != null) return answer;
    }}
    equation = text.match(/\b(-?\d+)\s*([+\-x*/])\s*x\s*=\s*(-?\d+)\b/i);
    if (equation) {{
      const answer = solveEquation(equation[1], equation[2].toLowerCase(), 'x', equation[3], false);
      if (answer != null) return answer;
    }}
    const match = text.match(/\b(-?\d+)\s*([+\-x*])\s*(-?\d+)\s*=/i);
    if (!match) return null;
    const a = Number(match[1]);
    const b = Number(match[3]);
    const op = match[2].toLowerCase();
    if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
    if (op === '+') return String(a + b);
    if (op === '-') return String(a - b);
    if (op === 'x' || op === '*') return String(a * b);
    return null;
  }}
  function sourceToCopy() {{
    const ordinal = ordinalIndex(instruction);
    const wantsTextarea = /\btext\s*area|textarea\b/i.test(instruction);
    let sourceFields = fields()
      .filter(el => String(el.value || el.textContent || '').trim())
      .filter(el => !wantsTextarea || el.tagName.toLowerCase() === 'textarea')
      .map(el => {{
        const raw = el.value != null ? String(el.value) : String(el.textContent || '');
        return {{ el, value: raw, nonEmpty: raw.trim() }};
      }});
    sourceFields.sort((a, b) => {{
      const ar = a.el.getBoundingClientRect();
      const br = b.el.getBoundingClientRect();
      return ar.top - br.top || ar.left - br.left;
    }});
    if (ordinal != null && sourceFields.length) {{
      return ordinal === -1 ? sourceFields[sourceFields.length - 1] : sourceFields[ordinal] || null;
    }}
    sourceFields.sort((a, b) => b.value.length - a.value.length);
    if (sourceFields.length) return sourceFields[0];
    const candidates = all('[data-copy], [data-value], output, pre, code, p, div, span')
      .filter(el => visible(el))
      .map(el => ({{ el, value: textOf(el) }}))
      .filter(item => item.value && item.value.length <= 1000 && !/submit|copy|paste|press/i.test(item.value))
      .sort((a, b) => b.value.length - a.value.length);
    return candidates[0] || null;
  }}
  function ordinalWordValue() {{
    if (!/\b(word|token)\b/i.test(instruction)) return null;
    const ordinal = ordinalIndex(instruction);
    if (ordinal == null) return null;
    if (!/\b(find|type|enter|input|write|fill|answer|textbox|text\s*box|field)\b/i.test(instruction)) return null;
    const wordPattern = /[A-Za-z0-9]+(?:['’_-][A-Za-z0-9]+)*/g;
    const wordsOf = value => String(value || '').match(wordPattern) || [];
    const readSource = el => {{
      try {{
        if (el.scrollHeight > el.clientHeight) {{
          el.scrollTop = el.scrollHeight;
          el.dispatchEvent(new Event('scroll', {{ bubbles: true }}));
        }}
      }} catch (_) {{}}
      return String(el.value != null ? el.value : (directText(el) || textOf(el) || ''))
        .replace(/\s+/g, ' ')
        .trim();
    }};
    const sources = all('p, article, section, main, [role=document], [data-text], [data-value], .paragraph, .text, .content, #randomText, div, span')
      .filter(el => visible(el))
      .filter(el => {{
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'button', 'input', 'textarea', 'select'].includes(tag)) return false;
        if (el.closest && el.closest('button, a, [role=button], [role=link], input, textarea, select')) return false;
        if (el.querySelector && el.querySelector('button, input, textarea, select')) return false;
        return true;
      }})
      .map(el => {{
        const value = readSource(el);
        const words = wordsOf(value);
        const rect = el.getBoundingClientRect();
        const meta = [el.tagName.toLowerCase(), el.id || '', el.getAttribute('class') || '', el.getAttribute('role') || ''].join(' ');
        let score = 0;
        if (el.tagName.toLowerCase() === 'p') score += 1.5;
        if (/\b(paragraph|passage|content|text|body|article|document|randomText)\b/i.test(meta)) score += 0.8;
        if (el.hasAttribute('data-text') || el.hasAttribute('data-value')) score += 0.5;
        if (directText(el)) score += 0.3;
        if (/\bparagraph|passage|text|sentence|below\b/i.test(instruction)) score += 0.2;
        if (rect.width * rect.height > 180000) score -= 0.5;
        return {{ el, value, words, score }};
      }})
      .filter(item => item.value && item.words.length && (ordinal === -1 || item.words.length > ordinal))
      .filter(item => item.value.length <= 5000 && !/\b(submit|done|ok|send|save|continue)\b/i.test(item.value))
      .filter((item, index, arr) => arr.findIndex(other => other.value === item.value) === index)
      .sort((a, b) => b.score - a.score || a.value.length - b.value.length);
    const source = sources[0];
    if (!source) return null;
    const index = ordinal === -1 ? source.words.length - 1 : ordinal;
    return source.words[index] || null;
  }}
  function displayedTextValue() {{
    const ordinalWord = ordinalWordValue();
    if (ordinalWord) return ordinalWord;
    if (/\blast\s+word\b/i.test(instruction)) {{
      const readSource = el => {{
        try {{
          if (el.scrollHeight > el.clientHeight) {{
            el.scrollTop = el.scrollHeight;
            el.dispatchEvent(new Event('scroll', {{ bubbles: true }}));
          }}
        }} catch (_) {{}}
        return String(el.value != null ? el.value : el.textContent || '').replace(/\s+/g, ' ').trim();
      }};
      const sourceFields = all('textarea, [role=textbox], [contenteditable]:not([contenteditable="false"])')
        .filter(visible)
        .map(el => ({{ el, value: readSource(el) }}))
        .filter(item => item.value && item.value.length > 10)
        .sort((a, b) => {{
          const at = a.el.tagName.toLowerCase();
          const bt = b.el.tagName.toLowerCase();
          return (bt === 'textarea') - (at === 'textarea') || b.value.length - a.value.length;
        }});
      const fallbackSources = all('pre, code, p, output, [data-value], [data-text], div, span')
        .filter(visible)
        .filter(el => !el.closest('button, a, [role=button], [role=link], input, select'))
        .map(el => ({{ el, value: readSource(el) }}))
        .filter(item => item.value && item.value.length > 10 && !/\b(submit|search|done|ok|send|save|continue)\b/i.test(item.value))
        .sort((a, b) => b.value.length - a.value.length);
      const source = sourceFields[0] || fallbackSources[0];
      if (source) {{
        const match = source.value.match(/[A-Za-z0-9_'’-]+(?=[^A-Za-z0-9_'’-]*$)/);
        if (match) return match[0];
      }}
    }}
    const explicit = all('[data-text], [data-copy-text], [data-value], output, pre, code')
      .filter(visible)
      .map(el => ({{ el, value: textOf(el).trim() }}))
      .filter(item => item.value && item.value.length <= 200 && !/submit|type|press|copy|paste/i.test(item.value))
      .sort((a, b) => b.value.length - a.value.length);
    if (explicit.length) return explicit[0].value;
    const candidates = all('p, div, span, label, output, pre, code, svg text')
      .filter(el => visible(el))
      .map(el => {{
        const rect = el.getBoundingClientRect();
        return {{ el, value: textOf(el).trim(), area: rect.width * rect.height }};
      }})
      .filter(item => item.value && item.value.length <= 200 && !/submit|type|press|copy|paste|solve|answer/i.test(item.value))
      .sort((a, b) => b.value.length - a.value.length || b.area - a.area);
    return candidates[0] && candidates[0].value;
  }}
  function targetField(source) {{
    const candidates = fields()
      .filter(el => !source || el !== source)
      .map(el => {{
        const tag = el.tagName.toLowerCase();
        const type = (el.getAttribute('type') || '').toLowerCase();
        const empty = !String(el.value || el.textContent || '').trim();
        let score = empty ? 1 : 0;
        if (tag === 'input') score += 0.2;
        if (type === 'text' || type === '') score += 0.1;
        if (source && source.parentElement && source.parentElement.contains(el)) score += 0.2;
        return {{ el, score }};
      }})
      .sort((a, b) => b.score - a.score);
    return candidates[0] && candidates[0].el;
  }}
  async function fillDerivedValue(value, source, mode) {{
    const target = targetField(source);
    if (!target) return {{ ok: false, error: 'derive_and_act could not find target text field', mode }};
    setValue(target, value);
    await delay(80);
    const submit = submitLike(target);
    if (submit && /\b(submit|done|send|search|press|click|when done)\b/i.test(instruction)) {{
      click(submit);
      await delay(180);
    }}
    return {{
      ok: true,
      mode,
      value,
      target: selector(target),
      submitted: !!submit,
      submit: submit ? selector(submit) : null,
    }};
  }}
  function greatestNumericCard() {{
    if (!/\b(greatest|highest|largest|max(?:imum)?)\b/i.test(instruction)) return null;
    function classText(el) {{
      if (!el) return '';
      if (typeof el.className === 'string') return el.className;
      if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
      return el.getAttribute && el.getAttribute('class') || '';
    }}
    function roleOf(el) {{
      return (el.getAttribute('role') || '').toLowerCase();
    }}
    function statusLike(el, text) {{
      const meta = [
        text || '',
        el.id || '',
        classText(el),
        roleOf(el),
        el.getAttribute('aria-label') || '',
        el.getAttribute('data-testid') || '',
      ].join(' ');
      if (/\b(time\s+left|remaining\s+time|elapsed|timer|countdown|reward|status|scoreboard|progress|debug|telemetry|hud)\b/i.test(meta)) return true;
      return ['status', 'timer', 'progressbar', 'meter'].includes(roleOf(el));
    }}
    function targetLike(el) {{
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const meta = [classText(el), el.id || '', el.getAttribute('data-testid') || '', el.getAttribute('aria-label') || ''].join(' ');
      return tag === 'button' || tag === 'a' || ['button', 'link', 'option'].includes(role) ||
        el.hasAttribute('onclick') || el.hasAttribute('tabindex') || el.hasAttribute('data-value') ||
        el.hasAttribute('data-index') || /\b(card|item|tile|row|entry|option|choice|result)\b/i.test(meta);
    }}
    const candidates = all('[data-value], [data-index], button, a, [role=button], [onclick], [tabindex], .card, .item, .tile, div, span')
      .filter(el => visible(el))
      .map(el => {{
        const text = textOf(el);
        const match = text.match(/-?\d+(?:\.\d+)?/);
        if (!match) return null;
        if (statusLike(el, text)) return null;
        if (!targetLike(el)) return null;
        const value = Number(match[0]);
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        if (!Number.isFinite(value) || area < 20 || area > 50000) return null;
        return {{ el, value, area, score: targetLike(el) ? 1 : 0 }};
      }})
      .filter(Boolean)
      .filter(item => !all('[data-value], [data-index], button, a, [role=button], [onclick], [tabindex], .card, .item, .tile, div, span', item.el)
        .some(child => child !== item.el && visible(child) && /-?\d/.test(textOf(child)) && child.getBoundingClientRect().width * child.getBoundingClientRect().height < item.area * 0.8))
      .sort((a, b) => b.value - a.value || b.score - a.score || a.area - b.area);
    return candidates[0] || null;
  }}

  const greatest = greatestNumericCard();
  if (greatest) {{
    click(greatest.el);
    await delay(120);
    const submit = submitLike(greatest.el);
    if (submit) {{
      click(submit);
      await delay(180);
    }}
    return {{ ok: true, mode: 'extreme-visible-number-click', value: String(greatest.value), target: selector(greatest.el), submitted: !!submit, submit: submit ? selector(submit) : null }};
  }}

  const followHint = (instruction.match(/\b(?:then|and)\s+(?:find\s+(?:and\s+)?)?(?:click|pick|choose|select)\s+(?:the\s+)?([^,.]+)\.?$/i) || [])[1] || '';
  if (followHint && ordinalIndex(followHint) != null && /\b(type|enter|input|search|fill|write|use)\b/i.test(instruction)) {{
    const quoted = Array.from(instruction.matchAll(/"([^"]+)"/g)).map(match => match[1]);
    const value = quoted[0] || '';
    const target = targetField(null);
    if (value && target) {{
      setValue(target, value);
      await delay(80);
      const submit = submitLike(target);
      if (submit) {{
        click(submit);
        await delay(250);
      }}
      const ordinalTarget = await resolveOrdinalClickTarget(followHint);
      if (ordinalTarget) {{
        click(ordinalTarget);
        await delay(180);
        return {{ ok: true, mode: 'fill-submit-click-ordinal', value, target: selector(target), ordinalTarget: selector(ordinalTarget), submitted: !!submit, submit: submit ? selector(submit) : null }};
      }}
      return {{ ok: false, error: 'derive_and_act could not find ordinal follow-up target', mode: 'fill-submit-click-ordinal', followHint }};
    }}
  }}

  const arithmetic = arithmeticValue();
  if (arithmetic != null && /\b(solve|math|answer|calculate|problem)\b/i.test(instruction)) {{
    return await fillDerivedValue(arithmetic, null, 'arithmetic-visible-text');
  }}

  if (/\b(copy|paste|duplicate|transfer)\b/i.test(instruction)) {{
    const source = sourceToCopy();
    if (source) return await fillDerivedValue(source.value, source.el, 'copy-visible-source-to-field');
  }}

  if (/\b(type|enter|input|write|fill|find)\b/i.test(instruction) && /\b(text|below|shown|displayed|word|token)\b/i.test(instruction)) {{
    const value = displayedTextValue();
    if (value) {{
      const mode = ordinalIndex(instruction) != null && /\b(word|token)\b/i.test(instruction)
        ? 'ordinal-visible-word-to-field'
        : 'visible-text-to-field';
      return await fillDerivedValue(value, null, mode);
    }}
  }}

  return {{ ok: false, error: 'derive_and_act could not derive a safe generic action from visible page state' }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "derive_and_act timed out".to_string())?
        .map_err(|e| {
            format!(
                "derive_and_act failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "derivedValue": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("derive_and_act failed")
            .to_string())
    }
}

pub(super) async fn handle_generate_constrained_value(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let constraints = params
        .get("constraints")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let max_attempts = params
        .get("maxAttempts")
        .or_else(|| params.get("max_attempts"))
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .clamp(1, 100);
    let instruction_json = json_literal(instruction);
    let constraints_json = json_literal(&constraints);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();

    let js = format!(
        r#"(async () => {{
  const instruction = {instruction_json};
  const constraints = {constraints_json};
  const maxAttempts = {max_attempts};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      try {{ if (document.querySelectorAll(byName).length === 1) return byName; }} catch (_) {{}}
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 6) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
    }}
    return parts.join(' > ');
  }}
  function all(query, root = document) {{
    const out = [];
    const seen = new Set();
    function collect(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      try {{
        if (scope.matches && scope.matches(query)) out.push(scope);
        if (scope.querySelectorAll) out.push(...Array.from(scope.querySelectorAll(query)));
      }} catch (_) {{}}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) collect(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) collect(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    collect(root);
    return Array.from(new Set(out));
  }}
	  function directText(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function textOf(el) {{
    return [
      directText(el),
      el.value || '',
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('data-value') || ''),
      el.getAttribute && (el.getAttribute('value') || ''),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
  function click(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
  }}
			  function isReadOnlyControl(el) {{
			    return !!el.readOnly ||
			      el.getAttribute('readonly') !== null ||
			      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
		  }}
		  {accessible_text_helpers_js}
		  {control_semantics_helpers_js}
		  {value_control_helpers_js}
		  function writableField(el) {{
		    return visible(el) && isWritableValueControl(el);
		  }}
	  function setValue(el, value) {{
	    return setControlValue(el, value);
	  }}
  function satisfies(value) {{
    const number = Number(value);
    if (!Number.isFinite(number)) return false;
    if (constraints.equals != null && number !== Number(constraints.equals)) return false;
    if (constraints.lessThan != null && !(number < Number(constraints.lessThan))) return false;
    if (constraints.greaterThan != null && !(number > Number(constraints.greaterThan))) return false;
    if (constraints.min != null && !(number >= Number(constraints.min))) return false;
    if (constraints.max != null && !(number <= Number(constraints.max))) return false;
    if (constraints.parity === 'odd' && Math.abs(number % 2) !== 1) return false;
    if (constraints.parity === 'even' && number % 2 !== 0) return false;
    return true;
  }}
  function deterministicValue() {{
    const preferred = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, -1, -2, -3, -4, -5, 11, 12, 13, 14, 15];
    for (const value of preferred) if (satisfies(value)) return value;
    for (let value = -1000; value <= 1000; value++) if (satisfies(value)) return value;
    return null;
  }}
  function submitLike(anchor) {{
    const controls = all('button, input[type=submit], input[type=button], a, [role=button], [onclick], [tabindex]')
      .filter(visible)
      .map(el => {{
        const text = textOf(el).toLowerCase();
        let score = 0;
        if (/\b(submit|done|ok|send|save|continue|confirm)\b/.test(text)) score += 1;
        if ((el.getAttribute('type') || '').toLowerCase() === 'submit') score += 1;
        if (anchor) {{
          const form = anchor.closest && anchor.closest('form');
          if (form && form.contains(el)) score += 0.5;
          if (anchor.parentElement && anchor.parentElement.contains(el)) score += 0.2;
        }}
        return {{ el, score }};
      }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    return controls[0] && controls[0].el;
  }}
  function snapshotTextBySelector() {{
    const map = new Map();
    for (const el of all('body *').filter(visible)) {{
      const key = selector(el);
      if (key) map.set(key, textOf(el));
    }}
    return map;
  }}
  function numericOutputs(beforeText) {{
    return all('output, [role=status], [aria-live], [data-value], [data-number], [data-result], [data-output], .value, .result, .output, .display, .number, .current, div, span, p')
      .filter(visible)
      .filter(el => !el.closest('button, a, input, textarea, select, [role=button], [role=link]'))
      .map(el => {{
        const text = textOf(el);
        const exact = text.match(/^\s*(-?\d+(?:\.\d+)?)\s*$/);
        if (!exact) return null;
        const key = selector(el);
        const meta = [el.id || '', classText(el), el.getAttribute('role') || '', el.getAttribute('aria-label') || ''].join(' ');
        const rect = el.getBoundingClientRect();
        let score = 0.4;
        if (beforeText && key && beforeText.get(key) !== text) score += 1;
        if (/\b(display|result|output|value|number|current|generated)\b/i.test(meta)) score += 0.8;
        if (el.tagName.toLowerCase() === 'output') score += 0.6;
        if (rect.width * rect.height > 10) score += 0.2;
        return {{ el, value: Number(exact[1]), text: exact[1], score }};
      }})
      .filter(Boolean)
      .sort((a, b) => b.score - a.score);
  }}

		  const fields = all(valueControlSelector())
		    .concat(all('*').filter(isCustomWritableValueElement))
		    .filter(writableField);
  const generatorControls = all('button, input[type=button], a, [role=button], [onclick], [tabindex]')
    .filter(visible)
    .map(el => {{
      const text = textOf(el);
      let score = 0;
      if (/\b(generate|random|roll|new|refresh|create|produce)\b/i.test(text)) score += 1;
      if (/\b(submit|done|send|save|continue|confirm)\b/i.test(text)) score -= 2;
      return {{ el, score }};
    }})
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score);

  if (!generatorControls.length && fields.length) {{
    const value = deterministicValue();
    if (value == null) return {{ ok: false, error: 'no deterministic value satisfies numeric constraints', constraints }};
    const target = fields[0];
    setValue(target, String(value));
    await delay(80);
    const submit = submitLike(target);
    if (submit) {{
      click(submit);
      await delay(180);
    }}
    return {{
      ok: true,
      mode: 'deterministic-value-to-field',
      value,
      constraints,
      target: selector(target),
      submitted: !!submit,
      submit: submit ? selector(submit) : null,
      attempts: 0
    }};
  }}

  if (!generatorControls.length) return {{ ok: false, error: 'no generator control or writable field found for numeric constraints', constraints }};
  const generator = generatorControls[0].el;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {{
    const before = snapshotTextBySelector();
    click(generator);
    await delay(90);
    const match = numericOutputs(before).find(item => satisfies(item.value));
    if (match) {{
      const submit = submitLike(match.el) || submitLike(generator);
      if (submit) {{
        click(submit);
        await delay(180);
      }}
      return {{
        ok: true,
        mode: 'generated-visible-value',
        value: match.value,
        constraints,
        generator: selector(generator),
        output: selector(match.el),
        attempts: attempt,
        submitted: !!submit,
        submit: submit ? selector(submit) : null
      }};
    }}
  }}
  return {{
    ok: false,
    error: 'generated values did not satisfy numeric constraints before attempt limit',
    constraints,
    generator: selector(generator),
    attempts: maxAttempts
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "generate_constrained_value timed out".to_string())?
        .map_err(|e| {
            format!(
                "generate_constrained_value failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "generatedValue": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("generate_constrained_value failed")
            .to_string())
    }
}

pub(super) async fn handle_conditional_value_action(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let constraints = params
        .get("constraints")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let action_hint = params
        .get("actionHint")
        .or_else(|| params.get("action_hint"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let source_hint = params
        .get("sourceHint")
        .or_else(|| params.get("source_hint"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let max_wait_ms = params
        .get("maxWaitMs")
        .or_else(|| params.get("max_wait_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or(9_000)
        .clamp(250, 9_500);
    let poll_ms = params
        .get("pollMs")
        .or_else(|| params.get("poll_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or(90)
        .clamp(30, 500);
    let instruction_json = json_literal(instruction);
    let constraints_json = json_literal(&constraints);
    let action_hint_json = json_literal(action_hint);
    let source_hint_json = json_literal(source_hint);
    let availability_helpers_js = availability_helpers_js();

    let js = format!(
        r#"(async () => {{
  const instruction = {instruction_json};
  const constraints = {constraints_json};
  const actionHint = {action_hint_json};
  const sourceHint = {source_hint_json};
  const maxWaitMs = {max_wait_ms};
  const pollMs = {poll_ms};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute && el.getAttribute('data-testid');
    if (testId) {{
      const byTestId = el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
      try {{ if (document.querySelectorAll(byTestId).length === 1) return byTestId; }} catch (_) {{}}
    }}
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      try {{ if (document.querySelectorAll(byName).length === 1) return byName; }} catch (_) {{}}
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 6) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
    }}
    return parts.join(' > ');
  }}
  function all(query, root = document) {{
    const out = [];
    const seen = new Set();
    function collect(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      try {{
        if (scope.matches && scope.matches(query)) out.push(scope);
        if (scope.querySelectorAll) out.push(...Array.from(scope.querySelectorAll(query)));
      }} catch (_) {{}}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) collect(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) collect(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    collect(root);
    return Array.from(new Set(out));
  }}
  function directText(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
  function textOf(el) {{
    return [
      directText(el),
      el.textContent || '',
      el.value || '',
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('data-value') || ''),
      el.getAttribute && (el.getAttribute('value') || ''),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function tokenize(text) {{
    return String(text || '').toLowerCase().match(/[a-z0-9]+/g) || [];
  }}
  function tokenScore(hint, text) {{
    const wanted = tokenize(hint);
    if (!wanted.length) return 0;
    const haystack = new Set(tokenize(text));
    let hits = 0;
    for (const token of wanted) if (haystack.has(token)) hits++;
    return hits / wanted.length;
  }}
  function parseNumber(text) {{
    const raw = String(text || '').replace(/,/g, '');
    const matches = Array.from(raw.matchAll(/[$€£]?\s*(-?\d+(?:\.\d+)?)/g));
    if (!matches.length) return null;
    const money = matches.find(match => /[$€£]/.test(match[0]));
    const chosen = money || matches[matches.length - 1];
    const value = Number(chosen[1]);
    return Number.isFinite(value) ? value : null;
  }}
  function satisfies(value) {{
    const number = Number(value);
    if (!Number.isFinite(number)) return false;
    const epsilon = 1e-9;
    const displayEpsilon = 0.005;
    if (constraints.equals != null && Math.abs(number - Number(constraints.equals)) > displayEpsilon) return false;
    if (constraints.lessThan != null && !(number < Number(constraints.lessThan) || Math.abs(number - Number(constraints.lessThan)) <= displayEpsilon)) return false;
    if (constraints.greaterThan != null && !(number > Number(constraints.greaterThan) || Math.abs(number - Number(constraints.greaterThan)) <= displayEpsilon)) return false;
    if (constraints.min != null && !(number + epsilon >= Number(constraints.min))) return false;
    if (constraints.max != null && !(number - epsilon <= Number(constraints.max))) return false;
    return true;
  }}
  function click(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
    try {{ el.click(); }} catch (_) {{}}
  }}
  function controlLike(el) {{
    const tag = el.tagName.toLowerCase();
    const role = (el.getAttribute('role') || '').toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    return tag === 'button' || tag === 'a' || role === 'button' || role === 'link' ||
      type === 'button' || type === 'submit' || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
  }}
  function promptLike(el, text) {{
    const meta = [el.id || '', classText(el), el.getAttribute('role') || '', el.getAttribute('aria-label') || '', text || ''].join(' ');
    return /\b(query|prompt|instruction|question|task|goal|timer|time\s*left|score|reward|status)\b/i.test(meta);
  }}
  function actionControls() {{
    const hint = actionHint || (instruction.match(/\b(click|press|tap|select|choose|buy|sell|open|start|stop|submit|save)\b/i) || [])[1] || '';
    return all('button, input[type=button], input[type=submit], a, [role=button], [role=link], [onclick], [tabindex]')
      .filter(el => visible(el) && controlLike(el))
      .map(el => {{
        const text = textOf(el);
        const meta = [text, el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
        let score = Math.max(tokenScore(hint, meta), hint && meta.toLowerCase().includes(String(hint).toLowerCase()) ? 0.8 : 0);
        if (!hint && /\b(submit|ok|done|continue|confirm|go)\b/i.test(meta)) score += 0.35;
        if (/\b(cancel|close|dismiss|delete|remove)\b/i.test(meta) && !/\b(cancel|close|dismiss|delete|remove)\b/i.test(hint)) score -= 0.8;
        if (el.tagName.toLowerCase() === 'button') score += 0.12;
        return {{ el, score, text }};
      }})
      .filter(item => item.score > 0.15)
      .sort((a, b) => b.score - a.score);
  }}
  function numericSources() {{
    return all('[data-value], [data-number], [aria-valuenow], output, [role=status], [aria-live], .value, .metric, .amount, .price, .total, .current, .number, .display, div, span, td, p')
      .filter(el => visible(el))
      .filter(el => !el.closest('button, a, input, textarea, select, [role=button], [role=link]'))
      .map(el => {{
        const text = textOf(el);
        const number = parseNumber(text);
        if (number == null) return null;
        const meta = [el.id || '', classText(el), el.getAttribute('role') || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || '', text].join(' ');
        const rect = el.getBoundingClientRect();
        let score = 0.2;
        if (sourceHint) score += Math.max(tokenScore(sourceHint, meta), meta.toLowerCase().includes(String(sourceHint).toLowerCase()) ? 0.7 : 0);
        if (/\b(price|value|amount|total|current|quote|rate|number|metric|balance)\b/i.test(meta)) score += 0.75;
        if (/[$€£]/.test(text)) score += 0.45;
        if (el.hasAttribute('data-value') || el.hasAttribute('data-number') || el.hasAttribute('aria-valuenow')) score += 0.35;
        if (promptLike(el, text)) score -= 1.6;
        if (el.querySelector && el.querySelector('button, input, textarea, select, [role=button], [role=link]')) score -= 0.5;
        const area = rect.width * rect.height;
        if (area > 0 && area < 100000) score += 0.15;
        return {{ el, value: number, text, score }};
      }})
      .filter(Boolean)
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
  }}

  const actions = actionControls();
  if (!actions.length) return {{ ok: false, error: 'conditional_value_action could not find a matching action control', actionHint }};
  const action = actions[0].el;
  const started = performance.now();
  const observations = [];
  let bestSource = null;
  while (performance.now() - started <= maxWaitMs) {{
    const sources = numericSources();
    if (sources.length) bestSource = sources[0];
    for (const source of sources.slice(0, 8)) {{
      observations.push({{ value: source.value, text: source.text, selector: selector(source.el), elapsedMs: Math.round(performance.now() - started) }});
      if (satisfies(source.value)) {{
        click(action);
        await delay(180);
        return {{
          ok: true,
          mode: 'conditional-visible-value-action',
          value: source.value,
          sourceText: source.text,
          constraints,
          source: selector(source.el),
          action: selector(action),
          actionText: textOf(action),
          elapsedMs: Math.round(performance.now() - started),
          observations: observations.slice(-20)
        }};
      }}
    }}
    await delay(pollMs);
  }}
  return {{
    ok: false,
    error: 'conditional_value_action timed out before visible numeric value satisfied constraints',
    constraints,
    action: selector(action),
    actionText: textOf(action),
    source: bestSource ? selector(bestSource.el) : null,
    lastValue: bestSource ? bestSource.value : null,
    observations: observations.slice(-20),
    elapsedMs: Math.round(performance.now() - started)
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "conditional_value_action timed out".to_string())?
        .map_err(|e| {
            format!(
                "conditional_value_action failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "conditionalAction": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("conditional_value_action failed")
            .to_string())
    }
}

pub(super) async fn handle_command_surface_action(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let instruction_json = json_literal(instruction);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();

    let js = format!(
        r#"(async () => {{
  const instruction = {instruction_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

  {availability_helpers_js}
  function visible(el) {{
    if (!el || unavailableForRead(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' &&
      style.visibility !== 'hidden' &&
      Number(style.opacity || 1) !== 0;
  }}
  function actionable(el) {{
    if (!el || unavailableForAction(el)) return false;
    const style = getComputedStyle(el);
    return style.display !== 'none' && style.visibility !== 'hidden';
  }}
  function selector(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      try {{ if (document.querySelectorAll(byName).length === 1) return byName; }} catch (_) {{}}
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 6) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
    }}
    return parts.join(' > ');
  }}
  function all(query, root = document) {{
    const out = [];
    const seen = new Set();
    function collect(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      try {{
        if (scope.matches && scope.matches(query)) out.push(scope);
        if (scope.querySelectorAll) out.push(...Array.from(scope.querySelectorAll(query)));
      }} catch (_) {{}}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) collect(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) collect(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    collect(root);
    return Array.from(new Set(out));
  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
	  function directText(el) {{
	    return Array.from(el.childNodes || [])
	      .filter(node => node.nodeType === Node.TEXT_NODE)
	      .map(node => node.textContent || '')
	      .join(' ')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
  {accessible_text_helpers_js}
  function isReadOnlyControl(el) {{
    return !!el.readOnly ||
      el.getAttribute('readonly') !== null ||
      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
  }}
  {control_semantics_helpers_js}
	  function textOf(el) {{
	    return [
	      directText(el),
	      el.textContent || '',
	      el.value || '',
	      el.getAttribute && (el.getAttribute('aria-label') || ''),
	      el.getAttribute && (el.getAttribute('title') || ''),
	      el.getAttribute && (el.getAttribute('placeholder') || ''),
	      el.getAttribute && (el.getAttribute('role') || ''),
	      el.getAttribute && (el.getAttribute('name') || ''),
	      referencedText(el, 'aria-labelledby'),
	      referencedText(el, 'aria-describedby'),
	      el.getAttribute && (el.getAttribute('aria-description') || ''),
	      semanticAttributeText(el),
	      associatedLabelText(el),
	      structuralLabelText(el),
	      nearbyLabelText(el),
	      shadowHostText(el),
	      slotText(el),
	    ].join(' ').replace(/\s+/g, ' ').trim();
	  }}
  function terminalScore(el) {{
    const meta = [
      el.id || '',
      classText(el),
      el.getAttribute && (el.getAttribute('role') || ''),
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      textOf(el)
    ].join(' ');
    let score = 0;
    if (/\b(?:terminal|shell|console|command|prompt|cli|repl)\b/i.test(meta)) score += 1.2;
    if (/(?:^|\s|[$>#%])(?:user|admin|root)?\s*[$>#%]\s*$/i.test(meta) || /\b(?:\$|>|#)\s*$/.test(meta)) score += 0.5;
    if (/\b(?:ls|rm|cd|cat|help|usage|command not found)\b/i.test(meta)) score += 0.35;
    if (el.tagName && ['pre', 'code', 'textarea'].includes(el.tagName.toLowerCase())) score += 0.15;
    return score;
  }}
	  function commandInputs() {{
	    const active = document.activeElement && isCommandInput(document.activeElement) ? [document.activeElement] : [];
	    const candidates = active
	      .concat(all('input, textarea, [contenteditable]:not([contenteditable="false"]), [role=textbox], [role=searchbox], [role=combobox], [tabindex]'))
	      .concat(all('*').filter(isCustomWritableValueElement))
	      .filter(isCommandInput)
      .map(el => {{
        const container = commandContainerFor(el);
        let score = 0.15;
        if (active.includes(el)) score += 0.8;
        if (container) score += terminalScore(container);
        const meta = [el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || '', el.getAttribute('placeholder') || ''].join(' ');
        if (/\b(?:terminal|shell|console|command|prompt|cli|input)\b/i.test(meta)) score += 0.9;
        return {{ el, container, score }};
      }})
      .filter(item => item.score >= 0.4)
      .sort((a, b) => b.score - a.score);
    return candidates;
  }}
  function isCommandInput(el) {{
    if (!el || !el.tagName || !actionable(el)) return false;
    const tag = el.tagName.toLowerCase();
	    const type = (el.getAttribute('type') || '').toLowerCase();
	    const role = (el.getAttribute('role') || '').toLowerCase();
	    if (tag === 'input' && ['button', 'submit', 'checkbox', 'radio', 'range', 'color', 'file'].includes(type)) return false;
	    return tag === 'input' ||
	      tag === 'textarea' ||
	      el.isContentEditable ||
	      ['textbox', 'searchbox', 'combobox'].includes(role) ||
	      isCustomWritableValueElement(el) ||
	      el.hasAttribute('tabindex');
	  }}
  function commandContainerFor(el) {{
    let node = el && el.parentElement;
    while (node && node !== document.documentElement) {{
      if (terminalScore(node) > 0.4) return node;
      node = node.parentElement;
    }}
    const surfaces = all('pre, code, textarea, [role=log], [aria-live], [class*=terminal], [class*=Terminal], [class*=console], [class*=Console], [class*=shell], [class*=Shell], [id*=terminal], [id*=console], [id*=shell], div, section')
      .filter(visible)
      .map(surface => ({{ surface, score: terminalScore(surface) }}))
      .filter(item => item.score > 0.5)
      .sort((a, b) => b.score - a.score);
    return surfaces[0] && surfaces[0].surface;
  }}
  function lines(container) {{
    const root = container || document.body || document.documentElement;
    const specific = Array.from(root.querySelectorAll ? root.querySelectorAll('[role=log], [aria-live], pre, code, output, .output, .terminal-output, .line, .terminal-line, [class*=output], [class*=Output], [class*=line], [class*=Line]') : [])
      .filter(visible)
      .map(el => textOf(el))
      .filter(Boolean);
    const text = textOf(root);
    const split = text.split(/\n|(?:\s{{2,}})/).map(part => part.replace(/\s+/g, ' ').trim()).filter(Boolean);
    return Array.from(new Set(specific.concat(split)));
  }}
  function keyCodeFor(key) {{
    if (key === 'Enter') return 13;
    if (key === 'Backspace') return 8;
    if (key === ' ') return 32;
    if (key.length === 1) return key.toUpperCase().charCodeAt(0);
    return 0;
  }}
  function dispatchKey(target, type, key) {{
    const code = key.length === 1
      ? (/^[a-z]$/i.test(key) ? 'Key' + key.toUpperCase() : /^[0-9]$/.test(key) ? 'Digit' + key : key)
      : key;
    const init = {{ key, code, bubbles: true, cancelable: true, keyCode: keyCodeFor(key), which: keyCodeFor(key), charCode: key.length === 1 ? key.charCodeAt(0) : 0 }};
    const event = new KeyboardEvent(type, init);
    try {{ Object.defineProperty(event, 'keyCode', {{ get: () => init.keyCode }}); }} catch (_) {{}}
    try {{ Object.defineProperty(event, 'which', {{ get: () => init.which }}); }} catch (_) {{}}
    target.dispatchEvent(event);
  }}
  async function submitCommand(input, command) {{
    try {{ input.focus(); }} catch (_) {{}}
    if (input.select) {{
      try {{ input.select(); }} catch (_) {{}}
    }}
    for (const ch of command) {{
      dispatchKey(input, 'keydown', ch);
      dispatchKey(input, 'keyup', ch);
      await delay(5);
    }}
    dispatchKey(input, 'keydown', 'Enter');
    dispatchKey(input, 'keyup', 'Enter');
    await delay(140);
  }}
  function commandIntent() {{
    if (/\b(?:list|show)\b/i.test(instruction) && /\b(?:file|directory|folder|entry|entries)\b/i.test(instruction)) return 'list';
    const runMatch = instruction.match(/\b(?:run|execute|type|enter)\s+`([^`]+)`/i) ||
      instruction.match(/\b(?:run|execute|type|enter)\s+"([^"]+)"/i);
    if (runMatch) return {{ run: runMatch[1] }};
    return null;
  }}

  const intent = commandIntent();
  if (!intent) return {{ ok: false, error: 'command_surface_action could not infer a command intent from instruction' }};
  const inputs = commandInputs();
  if (!inputs.length) return {{ ok: false, error: 'command_surface_action could not find a command-like input surface' }};
  const input = inputs[0].el;
  const container = inputs[0].container || commandContainerFor(input);
  const beforeLines = lines(container);

  if (typeof intent === 'object' && intent.run) {{
    await submitCommand(input, intent.run);
    return {{
      ok: true,
      mode: 'command-surface-run',
      command: intent.run,
      input: selector(input),
      container: container ? selector(container) : null,
      output: lines(container).slice(-12)
    }};
  }}
  if (intent === 'list') {{
    await submitCommand(input, 'ls');
    return {{
      ok: true,
      mode: 'command-surface-list',
      command: 'ls',
      input: selector(input),
      container: container ? selector(container) : null,
      output: lines(container).slice(-12)
    }};
  }}
  return {{ ok: false, error: 'command_surface_action did not find a safe generic command workflow', intent }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "command_surface_action timed out".to_string())?
        .map_err(|e| {
            format!(
                "command_surface_action failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "commandSurfaceAction": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("command_surface_action failed")
            .to_string())
    }
}

pub(super) async fn handle_feedback_loop_value(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let min = params.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let max = params.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0);
    let max_attempts = params
        .get("maxAttempts")
        .or_else(|| params.get("max_attempts"))
        .and_then(|v| v.as_u64())
        .unwrap_or(16)
        .clamp(1, 60);
    let instruction_json = json_literal(instruction);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();

    let js = format!(
        r#"(async () => {{
  const instruction = {instruction_json};
  let low = Math.ceil({min});
  let high = Math.floor({max});
  const maxAttempts = {max_attempts};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      try {{ if (document.querySelectorAll(byName).length === 1) return byName; }} catch (_) {{}}
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 6) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const siblings = Array.from(parent.children).filter(child => child.tagName === node.tagName);
        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
      }}
      parts.unshift(part);
      node = parent;
    }}
    return parts.join(' > ');
  }}
  function all(query, root = document) {{
    const out = [];
    const seen = new Set();
    function collect(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      try {{
        if (scope.matches && scope.matches(query)) out.push(scope);
        if (scope.querySelectorAll) out.push(...Array.from(scope.querySelectorAll(query)));
      }} catch (_) {{}}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) collect(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) collect(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    collect(root);
    return Array.from(new Set(out));
  }}
  function directText(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function textOf(el) {{
    return [
      directText(el),
      el.value || '',
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('data-value') || ''),
      el.getAttribute && (el.getAttribute('value') || ''),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
  function click(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
  }}
			  function isReadOnlyControl(el) {{
			    return !!el.readOnly ||
			      el.getAttribute('readonly') !== null ||
			      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
		  }}
		  {accessible_text_helpers_js}
		  {control_semantics_helpers_js}
		  {value_control_helpers_js}
		  function writableField(el) {{
		    return visible(el) && isWritableValueControl(el);
		  }}
	  function setValue(el, value) {{
	    return setControlValue(el, value);
	  }}
  function submitLike(anchor) {{
    const controls = all('button, input[type=submit], input[type=button], a, [role=button], [onclick], [tabindex]')
      .filter(visible)
      .map(el => {{
        const text = textOf(el).toLowerCase();
        let score = 0;
        if (/\b(submit|check|guess|try|go|ok|done|enter|continue|confirm)\b/.test(text)) score += 1;
        if ((el.getAttribute('type') || '').toLowerCase() === 'submit') score += 1;
        if (anchor) {{
          const form = anchor.closest && anchor.closest('form');
          if (form && form.contains(el)) score += 0.5;
          if (anchor.parentElement && anchor.parentElement.contains(el)) score += 0.2;
        }}
        if (/\b(reset|clear|cancel)\b/.test(text)) score -= 2;
        return {{ el, score }};
      }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    return controls[0] && controls[0].el;
  }}
  function visibleFeedbackText() {{
    const chunks = [];
    const seen = new Set();
    for (const el of all('output, [role=status], [aria-live], [data-feedback], [data-result], .feedback, .result, .status, .message, .error, .notice, p, div, span').filter(visible)) {{
      if (el.closest('button, a, input, textarea, select, [role=button], [role=link]')) continue;
      const text = textOf(el);
      if (!text || seen.has(text)) continue;
      const meta = [el.id || '', classText(el), el.getAttribute('role') || '', el.getAttribute('aria-label') || ''].join(' ');
      if (!/\b(feedback|result|status|message|error|notice|output|hint)\b/i.test(meta) && text.length > 160) continue;
      seen.add(text);
      chunks.push(text);
    }}
    return chunks.join(' | ');
  }}
  function classifyFeedback(text) {{
    const lower = String(text || '').toLowerCase();
    if (/\b(correct|success|succeeded|done|complete|completed|you got it|well done|right|yes)\b/.test(lower)) return 'success';
    if (/\b(too\s*high|too\s*large|lower|less|smaller|decrease|down)\b/.test(lower)) return 'too_high';
    if (/\b(too\s*low|too\s*small|higher|more|larger|increase|up)\b/.test(lower)) return 'too_low';
    if (/\b(wrong|incorrect|try again|nope|not quite)\b/.test(lower)) return 'wrong';
    return 'unknown';
  }}

  if (!Number.isFinite(low) || !Number.isFinite(high) || low > high) {{
    return {{ ok: false, error: 'feedback_loop_value has invalid numeric bounds', low, high }};
  }}
		  const fields = all(valueControlSelector())
		    .concat(all('*').filter(isCustomWritableValueElement))
		    .filter(writableField);
  if (!fields.length) return {{ ok: false, error: 'feedback_loop_value could not find a writable field' }};
  const field = fields[0];
  const submit = submitLike(field);
  if (!submit) return {{ ok: false, error: 'feedback_loop_value could not find a submit/check control', target: selector(field) }};

  const attempts = [];
  const tried = new Set();
  let lastFeedback = '';
  for (let attempt = 1; attempt <= maxAttempts && low <= high; attempt++) {{
    let guess = Math.floor((low + high) / 2);
    if (tried.has(guess)) {{
      guess = Array.from({{ length: high - low + 1 }}, (_, index) => low + index).find(value => !tried.has(value));
      if (guess == null) break;
    }}
    tried.add(guess);
    setValue(field, String(guess));
    await delay(60);
    click(submit);
    await delay(180);
    const feedback = visibleFeedbackText();
    const classification = classifyFeedback(feedback);
    attempts.push({{ attempt, value: guess, feedback, classification }});
    lastFeedback = feedback;
    if (classification === 'success') {{
      return {{
        ok: true,
        mode: 'feedback-loop-value',
        value: guess,
        target: selector(field),
        submit: selector(submit),
        submitted: true,
        attempts: attempts.length,
        history: attempts,
        feedback
      }};
    }}
    if (classification === 'too_high') {{
      high = guess - 1;
    }} else if (classification === 'too_low') {{
      low = guess + 1;
    }} else {{
      if (guess === low) low += 1;
      else high = guess - 1;
    }}
  }}
  return {{
    ok: false,
    error: 'feedback_loop_value did not observe success before exhausting attempts',
    target: selector(field),
    submit: selector(submit),
    attempts: attempts.length,
    history: attempts,
    lastFeedback
  }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "feedback_loop_value timed out".to_string())?
        .map_err(|e| {
            format!(
                "feedback_loop_value failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "generatedValue": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("feedback_loop_value failed")
            .to_string())
    }
}

pub(super) async fn handle_record_property_click(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let entity = params
        .get("entity")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "record_property_click requires entity".to_string())?;
    let property = params
        .get("property")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "record_property_click requires property".to_string())?;
    let max_pages = params
        .get("maxPages")
        .or_else(|| params.get("max_pages"))
        .and_then(|value| value.as_u64())
        .unwrap_or(12)
        .clamp(1, 50);
    let entity_json = json_literal(entity);
    let property_json = json_literal(property);
    let availability_helpers_js = availability_helpers_js();

    let js = format!(
        r#"(async () => {{
  const entity = {entity_json};
  const property = {property_json};
  const maxPages = {max_pages};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function directText(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function textOf(el) {{
    if (!el) return '';
    return [
      directText(el),
      el.textContent || '',
      el.value || '',
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('name') || '',
      el.getAttribute && el.getAttribute('class') || '',
      el.getAttribute && el.getAttribute('data-label') || '',
      el.getAttribute && el.getAttribute('data-value') || '',
      el.getAttribute && el.getAttribute('data-name') || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
      if (parts.length >= 6) break;
    }}
    return parts.join(' > ');
  }}
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{ if (scope.contentDocument) add(scope.contentDocument); }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) add(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(query, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope !== document && scope.matches && scope.matches(query) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(query)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function clickElement(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2, button: 0, buttons: 1 }};
    for (const type of ['pointerover', 'mouseover', 'mousemove', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
  }}
  function entityScore(el) {{
    const entityNorm = norm(entity);
    const text = norm(textOf(el));
    if (!entityNorm || !text) return 0;
    if (text === entityNorm) return 1.4;
    if (text.split(/\s+/).includes(entityNorm)) return 1.2;
    if (text.includes(entityNorm)) return 0.95;
    const entityTokens = entityNorm.split(/\s+/).filter(Boolean);
    if (entityTokens.length && entityTokens.every(token => text.includes(token))) return 0.65;
    return 0;
  }}
  function propertyScore(el) {{
    const propertyNorm = norm(property)
      .replace(/\bnumber\b/g, '')
      .replace(/\s+/g, ' ')
      .trim();
    const tokens = propertyNorm.split(/\s+/).filter(Boolean);
    const texts = [
      directText(el),
      el.textContent || '',
      el.getAttribute && el.getAttribute('class') || '',
      el.getAttribute && el.getAttribute('name') || '',
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('href') || '',
      el.parentElement ? textOf(el.parentElement) : ''
    ].join(' ');
    const textNorm = norm(texts);
    if (!tokens.length) return 0;
    let score = 0;
    if (tokens.every(token => textNorm.split(/\s+/).includes(token))) score += 1.0;
    else if (tokens.some(token => textNorm.split(/\s+/).includes(token))) score += 0.45;
    const tag = el.tagName.toLowerCase();
    const role = String(el.getAttribute('role') || '').toLowerCase();
    if (tag === 'a' || tag === 'button' || role === 'button' || role === 'link' || el.hasAttribute('onclick') || el.hasAttribute('tabindex')) score += 0.18;
    return score;
  }}
  function recordScopes() {{
    const entityNorm = norm(entity);
    const raw = all('[data-record], [data-result], [data-contact], [data-person], article, section, [role=listitem], [role=row], li, tr, .card, .record, .result, .contact, .item, div')
      .filter(visible)
      .filter(el => {{
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'a', 'button', 'input', 'select', 'textarea'].includes(tag)) return false;
        const classes = String(el.getAttribute('class') || '');
        if (/\bproperty\b/i.test(classes) && !el.hasAttribute('data-record')) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 30 || rect.height < 12 || rect.width * rect.height > 220000) return false;
        return norm(textOf(el)).includes(entityNorm);
      }});
    return raw.map(el => {{
      let score = entityScore(el);
      const descendants = all('h1, h2, h3, h4, strong, b, [data-name], [class*=name], [class*=Name]', el)
        .filter(visible)
        .map(entityScore)
        .sort((a, b) => b - a);
      if (descendants[0]) score += descendants[0] * 0.4;
      if (el.querySelector('a, button, [role=button], [role=link], [onclick], [tabindex]')) score += 0.08;
      return {{ el, score }};
    }}).filter(item => item.score > 0.4)
      .filter(item => !raw.some(other => other !== item.el && item.el.contains(other) && entityScore(other) >= entityScore(item.el)))
      .sort((a, b) => b.score - a.score);
  }}
  function propertyControl(scope) {{
    const controls = all('a, button, [role=button], [role=link], [onclick], [tabindex], span, div', scope)
      .filter(visible)
      .map(el => {{
        const tag = el.tagName.toLowerCase();
        const role = String(el.getAttribute('role') || '').toLowerCase();
        const actionish = tag === 'a' || tag === 'button' || role === 'button' || role === 'link' || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
        let score = propertyScore(el);
        if (actionish) score += 0.35;
        if (!actionish && el.querySelector('a, button, [role=button], [role=link], [onclick], [tabindex]')) score -= 0.6;
        return {{ el, score }};
      }})
      .filter(item => item.score > 0.45)
      .sort((a, b) => b.score - a.score);
    return controls[0] && controls[0].el;
  }}
  function paginationControls() {{
    return all('a, button, [role=button], [role=link], [onclick], [tabindex], .page-link, .page-item a')
      .map(el => {{
        const rawText = String(textOf(el) || '').trim().toLowerCase();
        const text = norm(rawText);
        const cls = norm(el.getAttribute('class') || '');
        const parentCls = norm(el.parentElement && el.parentElement.getAttribute('class') || '');
        let score = 0;
        if (/^(>|›|»|next|more)$/.test(rawText) || /^(next|more)$/.test(text)) score += 1;
        if (/\bnext\b/.test(text) || /\bnext\b/.test(cls) || /\bnext\b/.test(parentCls)) score += 0.8;
        if (/^\d+$/.test(text) && !/\bactive\b/.test(cls) && !/\bactive\b/.test(parentCls)) score += 0.35;
        if (/\b(prev|previous|disabled|active)\b/.test(cls) || /\b(prev|previous|disabled|active)\b/.test(parentCls) || /^(<|‹|«|prev|previous)$/.test(rawText) || /^(prev|previous)$/.test(text)) score -= 1;
        return {{ el, score }};
      }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
  }}

  const triedPages = [];
  for (let pageIndex = 0; pageIndex < maxPages; pageIndex++) {{
    const scopes = recordScopes();
    if (scopes.length) {{
      const scope = scopes[0].el;
      const control = propertyControl(scope);
      if (control) {{
        clickElement(control);
        await delay(120);
        return {{
          ok: true,
          entity,
          property,
          record: selectorFor(scope),
          clicked: selectorFor(control),
          triedPages,
          mode: 'record-property-click'
        }};
      }}
    }}
    if (norm(textOf(document.body || document.documentElement)).includes(norm(entity))) {{
      const globalControl = propertyControl(document.body || document.documentElement);
      if (globalControl) {{
        clickElement(globalControl);
        await delay(120);
        return {{
          ok: true,
          entity,
          property,
          record: 'document',
          clicked: selectorFor(globalControl),
          triedPages,
          mode: 'record-property-click-global'
        }};
      }}
    }}
    const pages = paginationControls();
    if (!pages.length) break;
    const next = pages[0].el;
    triedPages.push(selectorFor(next));
    const beforeText = norm(textOf(document.body || document.documentElement));
    clickElement(next);
    await delay(120);
    const afterSyntheticText = norm(textOf(document.body || document.documentElement));
    if (afterSyntheticText === beforeText && typeof next.click === 'function') {{
      try {{ next.click(); }} catch (_) {{}}
    }}
    await delay(220);
  }}
  return {{ ok: false, error: 'record_property_click could not find requested record/property', entity, property, triedPages }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "record_property_click timed out".to_string())?
        .map_err(|e| {
            format!(
                "record_property_click failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "recordPropertyClick": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("record_property_click failed")
            .to_string())
    }
}

pub(super) async fn handle_tree_search_click(page: &Page, params: &Value) -> Result<Value, String> {
    let target = params
        .get("target")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "tree_search_click requires target".to_string())?;
    let selector = params.get("selector").and_then(|value| value.as_str());
    let target_json = json_literal(target);
    let selector_json = json_literal(&selector);
    let availability_helpers_js = availability_helpers_js();

    let js = format!(
        r#"(async () => {{
  const target = {target_json};
  const selectorText = {selector_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function directText(el) {{
    return Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function labelText(el) {{
    return [
      directText(el),
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('data-label') || '',
      el.getAttribute && el.getAttribute('data-value') || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
      if (parts.length >= 6) break;
    }}
    return parts.join(' > ');
  }}
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{ if (scope.contentDocument) add(scope.contentDocument); }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) add(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(query, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope !== document && scope.matches && scope.matches(query) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(query)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function clickElement(el) {{
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2, button: 0, buttons: 1 }};
    for (const type of ['pointerover', 'mouseover', 'mousemove', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
  }}
  function findRoot() {{
    if (selectorText) {{
      for (const root of allRoots(document)) {{
        try {{
          if (root.matches && root.matches(selectorText)) return root;
          const found = root.querySelector && root.querySelector(selectorText);
          if (found) return found;
        }} catch (_) {{}}
      }}
    }}
    const roots = all('[role=tree], [role=treegrid], .tree, .treeview, .filetree, [class*=tree], [class*=Tree], ul, ol')
      .filter(el => {{
        const rect = el.getBoundingClientRect();
        if (rect.width < 20 || rect.height < 10) return false;
        const items = el.querySelectorAll ? el.querySelectorAll('li, [role=treeitem], [role=row], [aria-expanded], ul, ol').length : 0;
        return items >= 2;
      }})
      .map(el => {{
        const text = norm([
          el.id || '',
          el.getAttribute('class') || '',
          el.getAttribute('role') || '',
          el.getAttribute('aria-label') || '',
          el.getAttribute('title') || ''
        ].join(' '));
        let score = 0.3;
        if (/\b(?:tree|filetree|folder|hierarchy|outline|nav)\b/.test(text)) score += 0.45;
        if (el.getAttribute('role') === 'tree' || el.getAttribute('role') === 'treegrid') score += 0.25;
        score += Math.min(0.2, (el.querySelectorAll ? el.querySelectorAll('li, [role=treeitem]').length : 0) / 50);
        return {{ el, score }};
      }})
      .sort((a, b) => b.score - a.score);
    return roots[0] && roots[0].score >= 0.45 ? roots[0].el : null;
  }}
  function targetLabels(root) {{
    return all('li, [role=treeitem], [role=row], span, a, button, [tabindex], [onclick]', root)
      .filter(el => {{
        if (!visible(el)) return false;
        const text = labelText(el);
        if (!text) return false;
        return norm(text) === norm(target);
      }})
      .sort((a, b) => {{
        const aAction = a.matches('a, button, [role=treeitem], [role=row], li, [onclick], [tabindex]') ? 0 : 1;
        const bAction = b.matches('a, button, [role=treeitem], [role=row], li, [onclick], [tabindex]') ? 0 : 1;
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return aAction - bAction || ar.top - br.top || ar.left - br.left;
      }});
  }}
  function expansionControls(root) {{
    const controls = all('[aria-expanded=false], .hitarea, [class*=hitarea], [class*=expandable], [class*=collapsed], [class*=closed], [role=button]', root)
      .map(el => {{
        if (el.matches('[aria-expanded=false], [class*=expandable], [class*=collapsed], [class*=closed]')) {{
          const directControl = Array.from(el.children || []).find(child =>
            child.matches && child.matches('.hitarea, [class*=hitarea], button, [role=button], [aria-label*=Expand], [aria-label*=expand]')
          );
          if (directControl && visible(directControl)) return directControl;
        }}
        return el;
      }})
      .filter(visible)
      .filter(el => {{
        const text = norm(labelText(el));
        if (text && text === norm(target)) return false;
        const cls = String(el.getAttribute('class') || '');
        const aria = el.getAttribute('aria-expanded');
        if (aria === 'false') return true;
        if (/\b(?:hitarea|expandable|collapsed|closed)\b/i.test(cls)) return true;
        if (el.querySelector && el.querySelector('ul, ol, [role=group]')) return true;
        return false;
      }});
    return controls.sort((a, b) => {{
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.top - br.top || ar.left - br.left;
    }});
  }}

  const root = findRoot();
  if (!root) return {{ ok: false, error: 'tree_search_click could not find a hierarchical tree/list root', target }};
  const tried = [];
  for (let pass = 0; pass < 8; pass++) {{
    const targetMatches = targetLabels(root);
    if (targetMatches.length) {{
      const chosen = targetMatches[0];
      clickElement(chosen);
      await delay(120);
      return {{
        ok: true,
        target,
        clicked: selectorFor(chosen),
        tree: selectorFor(root),
        tried,
        mode: 'tree-search-click'
      }};
    }}
    const controls = expansionControls(root).filter(control => !tried.includes(selectorFor(control)));
    if (!controls.length) break;
    for (const control of controls.slice(0, 12)) {{
      const key = selectorFor(control);
      tried.push(key);
      clickElement(control);
      await delay(60);
      const after = targetLabels(root);
      if (after.length) {{
        const chosen = after[0];
        clickElement(chosen);
        await delay(120);
        return {{
          ok: true,
          target,
          clicked: selectorFor(chosen),
          tree: selectorFor(root),
          discoveredBy: key,
          tried,
          mode: 'tree-search-click'
        }};
      }}
    }}
  }}
  return {{ ok: false, error: 'tree_search_click could not reveal target: ' + target, target, tree: selectorFor(root), tried }};
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "tree_search_click timed out".to_string())?
        .map_err(|e| {
            format!(
                "tree_search_click failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "treeSearchClick": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("tree_search_click failed")
            .to_string())
    }
}

pub(super) async fn handle_visual_feedback_search(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let selector = params.get("selector").and_then(|value| value.as_str());
    let target_feedback = params
        .get("targetFeedback")
        .or_else(|| params.get("target_feedback"))
        .and_then(|value| value.as_str())
        .unwrap_or("hot");
    let selector_json = json_literal(&selector);
    let target_feedback_json = json_literal(target_feedback);
    let availability_helpers_js = availability_helpers_js();

    let js = format!(
        r#"(async () => {{
  const selectorText = {selector_json};
  const targetFeedback = String({target_feedback_json} || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

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
  function selectorFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
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
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{ if (scope.contentDocument) add(scope.contentDocument); }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{ if (el.contentDocument) add(el.contentDocument); }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope !== document && scope.matches && scope.matches(selectorText) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function textOf(el) {{
    if (!el) return '';
    return [
      el.textContent || '',
      el.value || '',
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('data-feedback') || '',
      el.getAttribute && el.getAttribute('data-state') || '',
      el.getAttribute && el.getAttribute('class') || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function findSurface() {{
    if (selectorText) {{
      for (const root of allRoots(document)) {{
        try {{
          if (root.matches && root.matches(selectorText) && visible(root)) return root;
          const found = root.querySelector && root.querySelector(selectorText);
          if (found && visible(found)) return found;
        }} catch (_) {{}}
      }}
    }}
    const candidates = all('canvas, svg, [role=application], [role=img], [data-surface], [data-target], [data-area], [data-canvas], [class*=surface], [class*=Surface], [class*=canvas], [class*=Canvas], [class*=area], [class*=Area], [class*=target], [class*=Target], [class*=board], [class*=Board], div, section')
      .filter(el => {{
        if (!visible(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'input', 'textarea', 'select', 'button', 'a'].includes(tag)) return false;
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        return rect.width >= 32 && rect.height >= 24 && area >= 800 && area <= 500000;
      }})
      .map(el => {{
        const rect = el.getBoundingClientRect();
        const label = norm([
          el.id || '',
          el.getAttribute('class') || '',
          el.getAttribute('role') || '',
          el.getAttribute('aria-label') || '',
          el.getAttribute('title') || '',
          el.getAttribute('data-testid') || '',
          el.getAttribute('data-surface') || '',
          el.getAttribute('data-target') || '',
          el.getAttribute('data-area') || ''
        ].join(' '));
        const style = getComputedStyle(el);
        const directText = norm(Array.from(el.childNodes || [])
          .filter(node => node.nodeType === Node.TEXT_NODE)
          .map(node => node.textContent || '')
          .join(' '));
        let score = 0.24;
        if (/\b(?:surface|canvas|area|target|touch|hit|hotspot|board|drawing|map|field|zone|region)\b/i.test(label)) score += 0.32;
        if (el.onmousemove || el.onpointermove || el.onmouseover || el.onclick || el.getAttribute('onmousemove') || el.getAttribute('onclick')) score += 0.22;
        if (style.cursor === 'pointer' || style.cursor === 'crosshair') score += 0.12;
        if (['canvas', 'svg'].includes(el.tagName.toLowerCase())) score += 0.16;
        if (directText.length <= 20) score += 0.1;
        if (rect.width >= 60 && rect.height >= 50) score += 0.08;
        return {{ el, score, area: rect.width * rect.height }};
      }})
      .sort((a, b) => b.score - a.score || a.area - b.area);
    return candidates.length && candidates[0].score >= 0.42 ? candidates[0].el : null;
  }}
  function feedbackCandidates(surface) {{
    const surfaceRect = surface.getBoundingClientRect();
    return all('output, [role=status], [aria-live], [data-feedback], [data-state], .status, .feedback, .signal, .message, .hint, div, span, p, strong, em, label')
      .filter(visible)
      .filter(el => {{
        if (el === document.body || el === document.documentElement) return false;
        const rect = el.getBoundingClientRect();
        const raw = textOf(el);
        return rect.width > 0 && rect.height > 0 && rect.width * rect.height <= 90000 && raw.length <= 80;
      }})
      .map(el => {{
        const rect = el.getBoundingClientRect();
        const dx = Math.max(0, Math.max(surfaceRect.left - rect.right, rect.left - surfaceRect.right));
        const dy = Math.max(0, Math.max(surfaceRect.top - rect.bottom, rect.top - surfaceRect.bottom));
        return {{ el, distance: Math.hypot(dx, dy) }};
      }})
      .sort((a, b) => a.distance - b.distance)
      .slice(0, 80);
  }}
  function feedbackFromText(raw) {{
    const value = norm(raw);
    if (!value) return null;
    const exact = new Set(['hot', 'warm', 'cold', 'ice cold', 'success', 'correct', 'good', 'yes', 'no', 'too high', 'too low']);
    if (exact.has(value)) return value;
    if (/\bice\s+cold\b/.test(value)) return 'ice cold';
    if (/\bhot\b/.test(value) && value.length <= 24) return 'hot';
    if (/\bwarm\b/.test(value) && value.length <= 24) return 'warm';
    if (/\bcold\b/.test(value) && value.length <= 24) return 'cold';
    if (/\bcorrect|success|good|yes\b/.test(value) && value.length <= 30) return 'correct';
    if (/\bincorrect|wrong|no\b/.test(value) && value.length <= 30) return 'wrong';
    return null;
  }}
  function feedbackScore(feedback) {{
    const value = norm(feedback);
    if (!value) return 0;
    if (targetFeedback && value === targetFeedback) return 10;
    if (targetFeedback === 'hot') {{
      if (value === 'hot') return 10;
      if (value === 'warm') return 6;
      if (value === 'cold') return 2;
      if (value === 'ice cold') return 0.5;
    }}
    if (targetFeedback === 'cold') {{
      if (value === 'cold') return 10;
      if (value === 'ice cold') return 7;
      if (value === 'warm') return 2;
      if (value === 'hot') return 0.5;
    }}
    if (['correct', 'success', 'good', 'yes'].includes(value)) return 8;
    if (value === 'warm') return 4;
    if (value === 'hot') return 5;
    if (value === 'cold') return 2;
    return value === targetFeedback ? 10 : 0;
  }}
  function eventInit(x, y, buttons = 0) {{
    return {{ bubbles: true, cancelable: true, composed: true, view: window, clientX: x, clientY: y, screenX: window.screenX + x, screenY: window.screenY + y, button: 0, buttons }};
  }}
  function dispatchPointer(surface, type, x, y, buttons = 0) {{
    const cx = Math.max(0, Math.min(window.innerWidth - 1, x));
    const cy = Math.max(0, Math.min(window.innerHeight - 1, y));
    const init = eventInit(cx, cy, buttons);
    const target = document.elementFromPoint(cx, cy) || surface;
    const targets = [target, surface].filter((el, index, arr) => el && arr.indexOf(el) === index);
    for (const el of targets) {{
      try {{
        if (window.PointerEvent && type !== 'click') el.dispatchEvent(new PointerEvent(type.replace(/^mouse/, 'pointer'), init));
      }} catch (_) {{}}
      try {{ el.dispatchEvent(new MouseEvent(type, init)); }} catch (_) {{}}
    }}
  }}
  function currentFeedback(candidates) {{
    let best = null;
    for (const item of candidates) {{
      const raw = textOf(item.el);
      const feedback = feedbackFromText(raw);
      if (!feedback) continue;
      const score = feedbackScore(feedback) + Math.max(0, 1 - item.distance / 600) * 0.2;
      if (!best || score > best.score) best = {{ feedback, raw: raw.slice(0, 80), score, selector: selectorFor(item.el) }};
    }}
    return best;
  }}
  function samplePoint(surface, candidates, point) {{
    dispatchPointer(surface, 'mousemove', point.x, point.y, 0);
    const feedback = currentFeedback(candidates);
    return {{
      ...point,
      feedback: feedback && feedback.feedback || null,
      feedbackText: feedback && feedback.raw || null,
      feedbackSelector: feedback && feedback.selector || null,
      score: feedback ? feedback.score : 0
    }};
  }}
  function addPoint(points, x, y, rect, seen) {{
    const px = Math.max(rect.left + 1, Math.min(rect.right - 1, x));
    const py = Math.max(rect.top + 1, Math.min(rect.bottom - 1, y));
    const key = Math.round(px) + ',' + Math.round(py);
    if (seen.has(key)) return;
    seen.add(key);
    points.push({{ x: px, y: py }});
  }}
  function clickPoint(surface, point) {{
    dispatchPointer(surface, 'mousemove', point.x, point.y, 0);
    dispatchPointer(surface, 'mousedown', point.x, point.y, 1);
    dispatchPointer(surface, 'mouseup', point.x, point.y, 0);
    dispatchPointer(surface, 'click', point.x, point.y, 0);
  }}
  function response(surface, rect, chosen, sampled, partial) {{
    return {{
      ok: true,
      partial,
      selector: selectorFor(surface),
      targetFeedback,
      clicked: {{ x: Math.round(chosen.x), y: Math.round(chosen.y) }},
      surfaceBounds: {{ x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) }},
      feedback: chosen.feedback,
      feedbackText: chosen.feedbackText,
      feedbackSelector: chosen.feedbackSelector,
      sampled,
      mode: 'visual-feedback-search'
    }};
  }}

  const surface = findSurface();
  if (!surface) return {{ ok: false, error: 'visual_feedback_search could not identify an interactive visual surface' }};
  try {{ surface.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
  await delay(20);
  const rect = surface.getBoundingClientRect();
  if (rect.width < 2 || rect.height < 2) return {{ ok: false, error: 'visual_feedback_search surface has invalid bounds', selector: selectorFor(surface) }};
  const candidates = feedbackCandidates(surface);
  if (!candidates.length) return {{ ok: false, error: 'visual_feedback_search could not identify feedback containers near the surface', selector: selectorFor(surface) }};

  const points = [];
  const seen = new Set();
  const coarseStep = Math.max(8, Math.min(16, Math.floor(Math.min(rect.width, rect.height) / 8)));
  for (let y = rect.top + 2; y <= rect.bottom - 2; y += coarseStep) {{
    for (let x = rect.left + 2; x <= rect.right - 2; x += coarseStep) addPoint(points, x, y, rect, seen);
  }}
  addPoint(points, rect.left + rect.width / 2, rect.top + rect.height / 2, rect, seen);
  const results = points.map(point => samplePoint(surface, candidates, point));
  results.sort((a, b) => b.score - a.score);

  const refineSeeds = results.slice(0, 10).filter(point => point.score > 0);
  for (const seed of refineSeeds) {{
    for (const radius of [coarseStep, Math.max(4, coarseStep / 2), 4]) {{
      const step = radius <= 4 ? 2 : Math.max(3, Math.floor(radius / 2));
      for (let y = seed.y - radius; y <= seed.y + radius; y += step) {{
        for (let x = seed.x - radius; x <= seed.x + radius; x += step) {{
          const point = {{ x: Math.max(rect.left + 1, Math.min(rect.right - 1, x)), y: Math.max(rect.top + 1, Math.min(rect.bottom - 1, y)) }};
          const key = Math.round(point.x) + ',' + Math.round(point.y);
          if (seen.has(key)) continue;
          seen.add(key);
          const result = samplePoint(surface, candidates, point);
          results.push(result);
          if (norm(result.feedback) === targetFeedback) {{
            results.sort((a, b) => b.score - a.score);
            const chosen = results[0];
            clickPoint(surface, chosen);
            return response(surface, rect, chosen, results.length, false);
          }}
        }}
      }}
    }}
  }}

  results.sort((a, b) => b.score - a.score);
  const chosen = results[0];
  if (!chosen || chosen.score <= 0) {{
    return {{ ok: false, error: 'visual_feedback_search sampled the surface but did not observe recognizable feedback', selector: selectorFor(surface), sampled: results.length }};
  }}
  clickPoint(surface, chosen);
  return response(surface, rect, chosen, results.length, norm(chosen.feedback) !== targetFeedback);
}})()"#
    );

    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "visual_feedback_search timed out".to_string())?
        .map_err(|e| {
            format!(
                "visual_feedback_search failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "visualFeedbackSearch": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("visual_feedback_search failed")
            .to_string())
    }
}

pub(super) async fn handle_discover_click(
    page: &Page,
    state: &DaemonState,
    params: &Value,
) -> Result<Value, String> {
    let target = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: target".to_string())?;
    let trigger = params.get("trigger").and_then(|v| v.as_str());
    let reveal_first = params
        .get("revealFirst")
        .or_else(|| params.get("reveal_first"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let target_json = json_literal(target);
    let trigger_json = json_literal(&trigger);
    let reveal_first_json = json_literal(&reveal_first);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let text_matcher_js = super::planner_js::text_matcher_js();
    let js = format!(
        r#"(async () => {{
  const target = {target_json};
  const trigger = {trigger_json};
  const revealFirst = {reveal_first_json};
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const startedAt = performance.now();
  const timeBudgetMs = 8200;
  const hasDiscoveryTime = () => performance.now() - startedAt < timeBudgetMs;
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function norm(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  {accessible_text_helpers_js}
  {text_matcher_js}
  function labelText(el) {{
    const labels = [];
    labels.push(associatedLabelText(el));
    labels.push(referencedText(el, 'aria-labelledby'));
    labels.push(referencedText(el, 'aria-describedby'));
    labels.push(structuralLabelText(el));
    labels.push(semanticAttributeText(el));
    return labels.join(' ');
  }}
  function textOf(el) {{
    return [
      el.textContent || '',
      el.value || '',
      el.placeholder || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('name') || '',
      el.getAttribute('role') || '',
      labelText(el),
      semanticAttributeText(el),
      slotText(el),
      svgReferenceText(el)
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function classText(el) {{
    return String(el && el.className && typeof el.className === 'string' ? el.className : '');
  }}
  function iconSemanticText(el) {{
    const nodes = [el].concat(Array.from(el && el.querySelectorAll ? el.querySelectorAll('[class*=icon], [class*=Icon], svg, use') : []));
    const out = [];
    const aliases = {{
      disk: 'save disk',
      play: 'play start',
      stop: 'stop',
      seekstart: 'seek start previous prev rewind beginning first',
      seekend: 'seek end next forward last',
      stepbackward: 'step backward previous prev',
      stepforward: 'step forward next',
      prev: 'previous prev back',
      previous: 'previous prev back',
      next: 'next forward',
      zoomin: 'zoom in plus magnify',
      zoomout: 'zoom out minus magnify',
      plus: 'plus add zoom in',
      minus: 'minus remove zoom out',
      print: 'print',
      trash: 'trash delete remove',
      close: 'close dismiss x',
      search: 'search find magnify',
    }};
    for (const node of nodes.slice(0, 24)) {{
      const raw = [
        classText(node),
        node.getAttribute && semanticAttributeText(node),
        node.getAttribute && node.getAttribute('href'),
        node.getAttribute && node.getAttribute('xlink:href'),
        node.getAttribute && node.getAttribute('data-icon'),
        node.getAttribute && node.getAttribute('icon'),
        node.getAttribute && node.getAttribute('aria-label'),
        node.getAttribute && node.getAttribute('title'),
        svgReferenceText(node),
      ].filter(Boolean).join(' ');
      for (const token of raw.split(/\s+/)) {{
        const cleaned = token
          .replace(/^#/, '')
          .replace(/^(?:ui-icon-|fa-|fas-|far-|fal-|fab-|icon-|lucide-|mdi-|material-icons?-?)/i, '')
          .replace(/[^A-Za-z0-9]+/g, ' ')
          .trim();
        if (!cleaned) continue;
        out.push(token, cleaned);
        const compact = cleaned.replace(/\s+/g, '').toLowerCase();
        if (aliases[compact]) out.push(aliases[compact]);
      }}
    }}
    return out.join(' ');
  }}
  function directTextOf(el) {{
    const direct = Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ');
    return [
      direct,
      el.value || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      semanticAttributeText(el),
      slotText(el),
      svgReferenceText(el)
    ].join(' ');
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
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
  function allRoots(start = document) {{
    const roots = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) add(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) add(scope.contentDocument);
        }} catch (_) {{}}
      }}
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) add(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) add(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    add(start || document);
    return roots;
  }}
  function all(selectorText, start = document) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(start)) {{
      try {{
        if (scope.matches && scope.matches(selectorText) && !seen.has(scope)) {{
          seen.add(scope);
          results.push(scope);
        }}
        const matches = scope.querySelectorAll ? Array.from(scope.querySelectorAll(selectorText)) : [];
        for (const el of matches) {{
          if (seen.has(el)) continue;
          seen.add(el);
          results.push(el);
        }}
      }} catch (_) {{}}
    }}
    return results;
  }}
  function preferredActionTarget(el) {{
    if (!el) return el;
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classes = classText(el);
    if (role.includes('menuitem') || role === 'option') return el;
    if (role === 'tab' || /\bui-tabs-tab\b/i.test(classes)) {{
      const tabChild = Array.from(el.querySelectorAll ? el.querySelectorAll('a, button, [role=tab], .ui-tabs-anchor') : [])
        .find(child => visible(child));
      if (tabChild) return tabChild;
    }}
    const child = Array.from(el.children || []).find(child => {{
      const childRole = String(child.getAttribute('role') || '').toLowerCase();
      const childClasses = classText(child);
      return childRole.includes('menuitem') || childRole === 'option' || /ui-menu-item-wrapper|menuitem|menu-item/i.test(childClasses);
    }});
    if (child && (/ui-menu-item|menuitem|menu-item/i.test(classes) || role === 'none' || el.tagName.toLowerCase() === 'li')) {{
      return child;
    }}
    return el;
  }}
  function cleanTargetHint(text) {{
    return String(text || '')
      .replace(/\b(?:button|link|control|item|element|labelled|labeled|called|named|with|icon|icons|action|panel|section)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function ordinalIndex(text) {{
    const raw = String(text || '').toLowerCase();
    if (/\blast\b/.test(raw)) return -1;
    const words = {{
      first: 0, second: 1, third: 2, fourth: 3, fifth: 4,
      sixth: 5, seventh: 6, eighth: 7, ninth: 8, tenth: 9
    }};
    for (const [word, index] of Object.entries(words)) {{
      if (new RegExp('(?:^|\\\\b)' + word + '(?:\\\\b|$)').test(raw)) return index;
    }}
    const match = raw.match(/\b(\d+)(?:st|nd|rd|th)?\b/);
    if (!match) return null;
    const index = Number(match[1]) - 1;
    return Number.isFinite(index) && index >= 0 ? index : null;
  }}
  function targetClickables(wanted = target) {{
    const ordinal = ordinalIndex(wanted);
    if (ordinal != null && /\b(result|link|row|card|item|entry|option)\b/i.test(wanted || '')) {{
      const wantedLower = String(wanted || '').toLowerCase();
      const rejectOrdinalControl = el => {{
        const text = textOf(el);
        const meta = [text, el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
        return !text || /\b(search|submit|done|ok|send|save|continue|next|previous|prev|page|pagination)\b/i.test(meta);
      }};
      if (/\b(result|link)\b/i.test(wantedLower)) {{
        let scopedLinks = all('#page-content a, [data-results] a, [data-result] a, [role=list] a, [role=feed] a, main a, section a, article a')
          .filter(visible)
          .map(preferredActionTarget)
          .filter(el => !rejectOrdinalControl(el));
        scopedLinks = scopedLinks.filter((el, index, arr) => arr.indexOf(el) === index);
        scopedLinks.sort((a, b) => {{
          const ar = a.getBoundingClientRect();
          const br = b.getBoundingClientRect();
          return ar.top - br.top || ar.left - br.left;
        }});
        const scopedChoice = ordinal === -1 ? scopedLinks[scopedLinks.length - 1] : scopedLinks[ordinal];
        if (/\bresult\b/i.test(wantedLower) && scopedLinks.length) return scopedChoice ? [scopedChoice] : [];
        if (scopedChoice) return [scopedChoice];
      }}
      let ordinalCandidates = all('a, button, [role=link], [role=button], [onclick], [tabindex], tr, li, .result, .item, .card, .row, div')
        .filter(visible)
        .map(preferredActionTarget)
        .filter(el => {{
          const tag = el.tagName.toLowerCase();
          const role = String(el.getAttribute('role') || '').toLowerCase();
          if (rejectOrdinalControl(el)) return false;
          if (/\b(result|link)\b/i.test(wantedLower)) return tag === 'a' || role === 'link' || /\bresult\b/i.test(classText(el));
          if (/\b(row|entry)\b/i.test(wantedLower)) return tag === 'tr' || /\b(row|entry)\b/i.test(classText(el));
          if (/\b(card|item|option)\b/i.test(wantedLower)) return tag === 'li' || role === 'option' || /\b(card|item|option)\b/i.test(classText(el)) || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
          return tag === 'a' || tag === 'button' || role === 'link' || role === 'button' || el.hasAttribute('onclick') || el.hasAttribute('tabindex');
        }});
      ordinalCandidates = ordinalCandidates.filter((el, index, arr) => arr.indexOf(el) === index);
      ordinalCandidates = ordinalCandidates.filter(el => !ordinalCandidates.some(other => other !== el && el.contains(other) && visible(other) && textOf(other)));
      ordinalCandidates.sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      }});
      const chosen = ordinal === -1 ? ordinalCandidates[ordinalCandidates.length - 1] : ordinalCandidates[ordinal];
      if (chosen) return [chosen];
    }}
    const targetNorm = norm(wanted);
    const cleanedTarget = cleanTargetHint(wanted);
    const scored = all('a, button, input[type=button], input[type=submit], [role=link], [role=button], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], .alink, .ui-menu-item, .ui-menu-item-wrapper, svg text, svg tspan, svg [class*=title], svg [class*=slice], [onclick], [tabindex]')
      .filter(visible)
      .map(el => {{
        el = preferredActionTarget(el);
        const tag = el.tagName.toLowerCase();
        const role = String(el.getAttribute('role') || '').toLowerCase();
        const classes = classText(el);
        const directText = [directTextOf(el), iconSemanticText(el)].join(' ');
        const allTextRaw = [textOf(el), iconSemanticText(el)].join(' ');
        const direct = norm(directText);
        const allText = norm(allTextRaw);
        let score = 0;
        if (direct === targetNorm) score += 3;
        else if (direct.split(' ').includes(targetNorm)) score += 2.5;
        else if (targetNorm.length > 2 && direct.includes(targetNorm)) score += 2;
        else if (targetNorm.length > 2 && (tag === 'button' || tag === 'a' || role === 'button' || role === 'link') && allText.includes(targetNorm)) score += 1;
        if (cleanedTarget && targetNorm.length > 2) {{
          score = Math.max(
            score,
            exactPhraseScore(cleanedTarget, directText) * 2.4,
            tokenScore(cleanedTarget, directText) * 1.8,
            exactPhraseScore(cleanedTarget, allTextRaw) * 1.8,
            tokenScore(cleanedTarget, allTextRaw) * 1.2
          );
        }}
        if (score > 0 && (tag === 'button' || tag === 'input' || role === 'button')) score += 0.3;
        if (score > 0 && el.closest('svg')) score += 0.2;
        if (score > 0 && (role.includes('menuitem') || role === 'option' || /ui-menu-item|ui-menu-item-wrapper|menuitem|menu-item/i.test(classes))) score += 0.25;
        if (['ul', 'ol'].includes(tag) || ['menu', 'menubar', 'listbox', 'tree'].includes(role)) score = 0;
        return {{ el, score }};
      }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    return scored.map(item => item.el);
  }}
  function actionTargetScoreInScope(wanted, el) {{
    const targetNorm = norm(cleanTargetHint(wanted) || wanted);
    if (!targetNorm) return 0;
    el = preferredActionTarget(el);
    const tag = el.tagName.toLowerCase();
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classes = classText(el);
    const directRaw = [directTextOf(el), iconSemanticText(el)].join(' ');
    const allRaw = [textOf(el), iconSemanticText(el)].join(' ');
    const direct = norm(directRaw);
    const allText = norm(allRaw);
    const directTokens = direct.split(' ').filter(Boolean);
    let score = 0;
    if (direct === targetNorm) score += 100;
    else if (directTokens.includes(targetNorm)) score += targetNorm.length <= 2 ? 85 : 70;
    else if (targetNorm.length > 2 && direct.includes(targetNorm)) score += 45;
    else if (targetNorm.length > 2 && exactPhraseScore(targetNorm, directRaw) > 0) score += 35;
    else if (targetNorm.length > 2 && tokenScore(targetNorm, directRaw) > 0.95) score += 30;

    if (score === 0 && targetNorm.length > 2) {{
      if ((tag === 'a' || tag === 'button' || role === 'link' || role === 'button') && allText === targetNorm) score += 28;
      else if ((tag === 'a' || tag === 'button' || role === 'link' || role === 'button') && allText.includes(targetNorm)) score += 18;
    }}
    if (score === 0) return 0;
    if (tag === 'a' || role === 'link' || /\balink\b/i.test(classes)) score += 12;
    if (tag === 'button' || role === 'button') score += 8;
    if (role.includes('menuitem') || role === 'option' || /ui-menu-item|ui-menu-item-wrapper|menuitem|menu-item/i.test(classes)) score += 6;
    if (['div', 'section', 'article', 'main', 'ul', 'ol', 'li'].includes(tag) && !el.hasAttribute('onclick') && !el.hasAttribute('tabindex')) score -= 20;
    const childAction = el.querySelector && el.querySelector('a, button, [role=link], [role=button], [onclick], [tabindex], .alink');
    if (childAction && childAction !== el) score -= 15;
    return Math.max(0, score);
  }}
  function idScopedElement(control, id) {{
    if (!id) return null;
    const cleanId = String(id).replace(/^#/, '');
    const rootNode = control && control.getRootNode && control.getRootNode();
    return (rootNode && rootNode.getElementById && rootNode.getElementById(cleanId)) || document.getElementById(cleanId);
  }}
  function revealScopes(control) {{
    const scopes = [];
    const seen = new Set();
    function add(scope) {{
      if (!scope || seen.has(scope)) return;
      seen.add(scope);
      scopes.push(scope);
    }}
    function addControlled(node) {{
      if (!node || !node.getAttribute) return;
      const controls = node.getAttribute('aria-controls');
      if (controls) {{
        for (const id of controls.split(/\s+/).filter(Boolean)) add(idScopedElement(node, id));
      }}
      const targetAttr = node.getAttribute('data-target') || node.getAttribute('data-bs-target');
      if (targetAttr && /^#[A-Za-z0-9_-]+$/.test(targetAttr)) add(idScopedElement(node, targetAttr));
      const href = node.getAttribute('href');
      if (href && /^#[A-Za-z0-9_-]+$/.test(href)) add(idScopedElement(node, href));
    }}
    addControlled(control);
    const owner = control && control.closest && control.closest("[aria-controls], [data-target], [data-bs-target], [href^='#'], [role=tab], .ui-tabs-tab");
    addControlled(owner);
    if (owner && owner !== control) addControlled(owner.querySelector && owner.querySelector("[aria-controls], [data-target], [data-bs-target], [href^='#']"));
    if (control && control.tagName && control.tagName.toLowerCase() === 'summary') add(control.closest('details'));
    if (control && control.nextElementSibling) add(control.nextElementSibling);
    if (control && control.parentElement && control.parentElement.nextElementSibling) add(control.parentElement.nextElementSibling);
    return scopes.filter(scope => scope && (scope === document || visible(scope) || Array.from(scope.querySelectorAll ? scope.querySelectorAll('*') : []).some(visible)));
  }}
  function targetInRevealScope(control, wanted) {{
    const candidates = [];
    const seen = new Set();
    const candidateSelector = 'a, button, input[type=button], input[type=submit], [role=link], [role=button], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], .alink, .ui-menu-item, .ui-menu-item-wrapper, svg text, svg tspan, [onclick], [tabindex]';
    for (const scope of revealScopes(control)) {{
      for (let el of all(candidateSelector, scope)) {{
        if (!visible(el)) continue;
        el = preferredActionTarget(el);
        if (!el || seen.has(el) || !visible(el)) continue;
        seen.add(el);
        const score = actionTargetScoreInScope(wanted, el);
        if (score > 0) candidates.push({{ el, score }});
      }}
    }}
    candidates.sort((a, b) => {{
      if (b.score !== a.score) return b.score - a.score;
      const ar = a.el.getBoundingClientRect();
      const br = b.el.getBoundingClientRect();
      return ar.top - br.top || ar.left - br.left;
    }});
    return candidates[0] ? candidates[0].el : null;
  }}
  async function nestedMenuTargetInRevealScope(control, wanted) {{
    const candidates = [];
    const seen = new Set();
    const candidateSelector = 'a, button, input[type=button], input[type=submit], [role=link], [role=button], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], .alink, .ui-menu-item, .ui-menu-item-wrapper, svg text, svg tspan, [onclick], [tabindex]';
    for (const scope of revealScopes(control)) {{
      const scopeLooksLikeMenu = scope && scope.matches && scope.matches('.ui-menu, [role=menu], [role=menubar], [role=listbox], ul, ol');
      const hasMenu = scopeLooksLikeMenu || !!(scope && scope.querySelector && scope.querySelector('.ui-menu, [role=menu], [role=menubar], [role=listbox]'));
      if (!hasMenu) continue;
      for (let el of all(candidateSelector, scope)) {{
        el = preferredActionTarget(el);
        if (!el || seen.has(el)) continue;
        seen.add(el);
        const score = actionTargetScoreInScope(wanted, el);
        if (score > 0) candidates.push({{ el, score }});
      }}
    }}
    candidates.sort((a, b) => b.score - a.score);
    for (const candidate of candidates) {{
      let targetEl = candidate.el;
      if (visible(targetEl)) return targetEl;
      const row = targetEl.closest && targetEl.closest('li');
      if (!row) continue;
      const ancestors = [];
      let menu = row.parentElement;
      while (menu) {{
        const parentRow = menu.parentElement && menu.parentElement.closest && menu.parentElement.closest('li');
        if (!parentRow) break;
        const parentTarget = preferredActionTarget(parentRow);
        if (parentTarget) ancestors.unshift(parentTarget);
        menu = parentRow.parentElement;
      }}
      for (const ancestor of ancestors) {{
        if (!visible(ancestor)) continue;
        clickElement(ancestor);
        await delay(160);
      }}
      targetEl = preferredActionTarget(candidate.el);
      if (targetEl && visible(targetEl)) return targetEl;
    }}
    return null;
  }}
  function textMatchStrength(wanted, el) {{
    const rawTarget = String(cleanTargetHint(wanted) || wanted || '').replace(/\s+/g, ' ').trim();
    if (!rawTarget || !el) return 'none';
    const directRaw = [directTextOf(el), iconSemanticText(el)].join(' ').replace(/\s+/g, ' ').trim();
    const allRaw = [textOf(el), iconSemanticText(el)].join(' ').replace(/\s+/g, ' ').trim();
    if (directRaw === rawTarget) return 'exact';
    if (directRaw.split(/\s+/).includes(rawTarget)) return 'exact-token';
    if (allRaw === rawTarget) return 'exact-all';
    const targetNorm = norm(rawTarget);
    const directNorm = norm(directRaw);
    if (directNorm === targetNorm || directNorm.split(' ').includes(targetNorm)) return 'normalized';
    return 'fuzzy';
  }}
  function shouldDeferRevealedTarget(wanted, el) {{
    const strength = textMatchStrength(wanted, el);
    return strength === 'normalized' || strength === 'fuzzy';
  }}
  function resultLinks() {{
    return all('#page-content a, [data-results] a, [data-result] a, [role=list] a, [role=feed] a, main a, section a, article a')
      .filter(visible)
      .map(preferredActionTarget)
      .filter(el => {{
        const text = textOf(el);
        const meta = [text, el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
        return text && !/\b(search|submit|done|ok|send|save|continue|next|previous|prev|page|pagination)\b/i.test(meta);
      }})
      .filter((el, index, arr) => arr.indexOf(el) === index)
      .sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      }});
  }}
  async function targetAfterPagination(wanted) {{
    const ordinal = ordinalIndex(wanted);
    const wantsResult = /\bresult\b/i.test(String(wanted || ''));
    if (ordinal == null || !wantsResult) {{
      return targetClickables(wanted)[0] || null;
    }}
    if (ordinal === -1) return resultLinks().slice(-1)[0] || null;
    const indexedResult = all('[data-result]')
      .map(preferredActionTarget)
      .find(el => visible(el) && String(el.getAttribute('data-result') || '') === String(ordinal));
    if (indexedResult) return indexedResult;
    const links = resultLinks();
    if (!links.length || ordinal < links.length) return links[ordinal] || null;
    const pageSize = links.length;
    const targetPageNumber = Math.floor(ordinal / pageSize) + 1;
    const withinPageIndex = ordinal % pageSize;
    const controls = all('a, button, [role=link], [role=button], [onclick], [tabindex]')
      .filter(visible)
      .filter(el => {{
        const text = textOf(el).trim();
        const meta = [text, classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('role') || ''].join(' ');
        return text === String(targetPageNumber) || /\b(next|more)\b|^>$/i.test(meta);
      }})
      .sort((a, b) => {{
        const at = textOf(a).trim() === String(targetPageNumber) ? 0 : 1;
        const bt = textOf(b).trim() === String(targetPageNumber) ? 0 : 1;
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return at - bt || ar.top - br.top || ar.left - br.left;
      }});
    if (!controls.length) return null;
    clickElement(controls[0]);
    for (let i = 0; i < 8; i++) {{
      await delay(80);
      const indexedAfterPage = all('[data-result]')
        .map(preferredActionTarget)
        .find(el => visible(el) && String(el.getAttribute('data-result') || '') === String(ordinal));
      if (indexedAfterPage) return indexedAfterPage;
    }}
    return resultLinks()[withinPageIndex] || null;
  }}
  function isReadOnlyControl(el) {{
    return !!el.readOnly ||
      el.getAttribute('readonly') !== null ||
      (el.getAttribute('aria-readonly') || '').toLowerCase() === 'true';
  }}
  {control_semantics_helpers_js}
  {value_control_helpers_js}
  function fillQuotedTriggerValue(text) {{
    const quoted = Array.from(String(text || '').matchAll(/"([^"]+)"/g)).map(match => match[1]);
    const value = quoted[0];
    if (!value) return null;
    const fields = all(valueControlSelector())
      .concat(all('*').filter(isCustomWritableValueElement))
      .filter(el => visible(el) && isWritableValueControl(el))
      .sort((a, b) => {{
        const av = readControlValue(a).trim() ? 1 : 0;
        const bv = readControlValue(b).trim() ? 1 : 0;
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return av - bv || ar.top - br.top || ar.left - br.left;
      }});
    const field = fields[0];
    if (!field) return null;
    setControlValue(field, value);
    return {{ selector: selector(field), value }};
  }}
  function eventInit(el) {{
    const r = el.getBoundingClientRect();
    return {{ bubbles: true, cancelable: true, view: window, clientX: r.left + Math.max(1, r.width / 2), clientY: r.top + Math.max(1, r.height / 2) }};
  }}
  function jQueryMenuWidget(el, init) {{
    const jq = window.jQuery || window.$;
    if (!jq || !jq.fn || !jq.fn.menu) return null;
    const row = el.closest('li');
    if (!row) return null;
    let menu = row.closest('.ui-menu');
    while (menu && menu.parentElement) {{
      const parentMenu = menu.parentElement.closest('.ui-menu');
      if (!parentMenu) break;
      menu = parentMenu;
    }}
    if (!menu) return null;
    let instance = null;
    try {{ instance = jq(menu).menu('instance'); }} catch (_) {{}}
    if (!instance) return null;
    const event = jq.Event('mousemove');
    event.target = el;
    event.currentTarget = row;
    event.pageX = init.clientX + window.scrollX;
    event.pageY = init.clientY + window.scrollY;
    return {{ jq, row: jq(row), rowElement: row, instance, event }};
  }}
  function rowHasSubmenu(row) {{
    if (!row) return false;
    return Array.from(row.children || []).some(child => child.matches && child.matches('ul, .ui-menu, [role=menu], [role=menubar]'));
  }}
  function wantsBrowserClick(el) {{
    if (!el) return false;
    const tag = el.tagName.toLowerCase();
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classes = classText(el);
    if (isInPageActionLink(el)) return false;
    if (tag === 'a' && (el.hasAttribute('data-result') || el.closest('#page-content, [data-results], [data-result], [role=list], [role=feed]'))) return true;
    if (role.includes('menuitem') || role === 'option' || /ui-menu-item|ui-menu-item-wrapper|menuitem|menu-item/i.test(classes) || el.closest('.ui-menu, [role=menu], [role=menubar], [role=listbox]')) return false;
    return false;
  }}
  function isInPageActionLink(el) {{
    if (!el || el.tagName.toLowerCase() !== 'a') return false;
    if (isTabControlLink(el)) return false;
    const href = String(el.getAttribute('href') || '');
    return href === '#' || href.startsWith('#') || /^javascript:/i.test(href);
  }}
  function isTabControlLink(el) {{
    if (!el || el.tagName.toLowerCase() !== 'a') return false;
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classes = classText(el);
    if (role === 'tab' || /\bui-tabs-anchor\b/i.test(classes)) return true;
    const parent = el.closest('[role=tab], .ui-tabs-tab');
    if (parent) return true;
    return !!el.closest('.ui-tabs, [role=tablist]');
  }}
  function clearHashOnlyNavigation() {{
    if (!window.location.hash) return;
    try {{
      window.history.replaceState(window.history.state, document.title, window.location.pathname + window.location.search);
    }} catch (_) {{}}
  }}
  function clickElement(el) {{
    el = preferredActionTarget(el);
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    const init = eventInit(el);
    const inPageActionLink = isInPageActionLink(el);
    const originalHref = inPageActionLink ? el.getAttribute('href') : null;
    if (inPageActionLink) {{
      try {{ el.removeAttribute('href'); }} catch (_) {{}}
    }}
    for (const type of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'mousemove']) {{
      const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
      el.dispatchEvent(event);
    }}
    const widget = jQueryMenuWidget(el, init);
    const hasSubmenu = widget && rowHasSubmenu(widget.rowElement);
    if (widget) {{
      try {{ widget.instance.focus(widget.event, widget.row); }} catch (_) {{}}
      if (hasSubmenu) {{
        try {{ widget.instance.expand(widget.event); }} catch (_) {{}}
        return;
      }}
    }}
    if (!widget && !inPageActionLink) {{
      try {{
        if (typeof el.click === 'function') {{
          el.click();
          return;
        }}
      }} catch (_) {{}}
    }}
    const targets = [el, widget && widget.rowElement].filter((node, index, all) => node && all.indexOf(node) === index);
    for (const target of targets) {{
      for (const type of ['mouseover', 'mousedown', 'mouseup', 'click']) {{
        target.dispatchEvent(new MouseEvent(type, init));
      }}
    }}
    if (inPageActionLink && originalHref != null) {{
      try {{ el.setAttribute('href', originalHref); }} catch (_) {{}}
    }}
    if (inPageActionLink && window.location.hash) {{
      clearHashOnlyNavigation();
    }}
    if (inPageActionLink) setTimeout(clearHashOnlyNavigation, 0);
    if (widget && widget.rowElement && typeof widget.rowElement.click === 'function') {{
      try {{ widget.rowElement.click(); }} catch (_) {{}}
    }}
    if (widget) {{
      try {{ widget.instance.select(widget.event); }} catch (_) {{}}
    }}
  }}
  function expansionState(el) {{
    const aria = el.getAttribute('aria-expanded');
    if (aria === 'true') return 'expanded';
    if (aria === 'false') return 'collapsed';
    if (el.tagName.toLowerCase() === 'summary') {{
      const details = el.closest('details');
      if (details && details.open) return 'expanded';
      if (details) return 'collapsed';
    }}
    const classes = String(el.className || '');
    if (/\b(ui-accordion-header-active|ui-state-active|active|open|expanded)\b/i.test(classes)) return 'expanded';
    const controls = el.getAttribute('aria-controls');
    if (controls) {{
      const rootNode = el.getRootNode && el.getRootNode();
      const panel =
        (rootNode && rootNode.getElementById && rootNode.getElementById(controls)) ||
        document.getElementById(controls);
      if (panel && visible(panel)) return 'expanded';
    }}
    const next = el.nextElementSibling;
    if (next && visible(next)) return 'expanded';
    return 'unknown';
  }}
  function hasMenuSubmenu(el) {{
    const row = el && el.closest ? el.closest('li') : null;
    return rowHasSubmenu(row || el);
  }}
  function canRevealMore(el) {{
    if (!el) return false;
    const tag = el.tagName.toLowerCase();
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const classes = String(el.className || '');
    if (el.getAttribute('aria-expanded') === 'false') return true;
    if (el.hasAttribute('aria-haspopup')) return true;
    if (tag === 'summary' || tag === 'details') return true;
    if (/\b(ui-accordion-header|ui-tabs-anchor|ui-tabs-tab|accordion|collapsible|expand)\b/i.test(classes)) return true;
    if (role.includes('menuitem') || /ui-menu-item|ui-menu-item-wrapper|menuitem|menu-item/i.test(classes)) {{
      return hasMenuSubmenu(el);
    }}
    if (role === 'option') return false;
    return ['tab', 'button'].includes(role) || tag === 'button' || tag === 'a' || el.hasAttribute('data-toggle');
  }}
  async function waitForReveal(control, wanted) {{
    for (let i = 0; i < 18 && hasDiscoveryTime(); i++) {{
      const scoped = control ? targetInRevealScope(control, wanted) : null;
      if (scoped && !shouldDeferRevealedTarget(wanted, scoped)) return scoped;
      const nestedMenu = control ? await nestedMenuTargetInRevealScope(control, wanted) : null;
      if (nestedMenu) return nestedMenu;
      if (scoped) return scoped;
      const found = await targetAfterPagination(wanted);
      if (found) return found;
      if (control && expansionState(control) === 'expanded') {{
        await delay(60);
        const scopedAfterExpanded = targetInRevealScope(control, wanted);
        if (scopedAfterExpanded && !shouldDeferRevealedTarget(wanted, scopedAfterExpanded)) return scopedAfterExpanded;
        const nestedAfterExpanded = await nestedMenuTargetInRevealScope(control, wanted);
        if (nestedAfterExpanded) return nestedAfterExpanded;
        if (scopedAfterExpanded) return scopedAfterExpanded;
        const afterExpanded = await targetAfterPagination(wanted);
        if (afterExpanded) return afterExpanded;
      }}
      await delay(60);
    }}
    return null;
  }}
  function discoveryControls() {{
    const controls = all('[aria-expanded=false], [role=tab], [role=button], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], button, summary, details > summary, .ui-tabs-anchor, .ui-tabs-tab, .ui-accordion-header, .ui-menu-item, .ui-menu-item-wrapper, svg [class*=spreader], [data-toggle], [data-testid*=tab], [data-testid*=section], [class*=accordion], [class*=Accordion], [class*=collapsible], [class*=Collapsible], [class*=expand], [class*=Expand]')
      .filter(el => visible(el) && canRevealMore(el));
    return controls.map(el => {{
      const tag = el.tagName.toLowerCase();
      const role = String(el.getAttribute('role') || '').toLowerCase();
      const classes = String(el.className || '');
      const text = norm(textOf(el));
      let score = 0;
      if (el.getAttribute('aria-expanded') === 'false') score += 5;
      if (tag === 'summary') score += 4;
      if (/\b(ui-accordion-header|accordion|collapsible|expand)\b/i.test(classes)) score += 4;
      if (['tab', 'button'].includes(role)) score += 1;
      if (/\b(ui-tabs-anchor|ui-tabs-tab)\b/i.test(classes)) score += 3;
      if (el.closest('svg') && /\bspreader\b/i.test(classes)) score += 4;
      if (/\b(expand|open|show|section|details|more)\b/i.test(text)) score += 2;
      if (norm(target) && text.includes(norm(target))) score -= 2;
      return {{ el, score }};
    }}).sort((a, b) => b.score - a.score).map(item => item.el);
  }}
  const tried = [];
  if (trigger) {{
    const filledTrigger = fillQuotedTriggerValue(trigger);
    const opener = targetClickables(trigger)[0];
    if (opener) {{
      const key = selector(opener);
      tried.push(key);
      clickElement(opener);
      const found = await waitForReveal(opener, target);
      if (found) {{
        const browserClickRecommended = wantsBrowserClick(found);
        const hashCleanupRecommended = isInPageActionLink(found);
        if (!browserClickRecommended) clickElement(found);
        await delay(browserClickRecommended ? 260 : 140);
        clearHashOnlyNavigation();
        return {{ ok: true, target, trigger, clicked: selector(found), discoveredBy: key, mode: 'triggered-discover-click', tried, filledTrigger, browserClickRecommended, hashCleanupRecommended }};
      }}
    }}
  }}
  let deferredMatch = null;
  for (let pass = 0; pass < 2 && hasDiscoveryTime(); pass++) {{
    const direct = revealFirst && pass === 0 ? null : await targetAfterPagination(target);
    if (direct) {{
      const browserClickRecommended = wantsBrowserClick(direct);
      const hashCleanupRecommended = isInPageActionLink(direct);
      if (!browserClickRecommended) clickElement(direct);
      await delay(browserClickRecommended ? 260 : 140);
      clearHashOnlyNavigation();
      return {{ ok: true, target, clicked: selector(direct), mode: pass === 0 ? 'direct' : 'after-discovery', tried, browserClickRecommended, hashCleanupRecommended }};
    }}
    const controls = discoveryControls();
    for (let controlIndex = 0; controlIndex < controls.length && hasDiscoveryTime(); controlIndex++) {{
      const control = controls[controlIndex];
      const key = selector(control);
      if (tried.includes(key)) continue;
      tried.push(key);
      clickElement(control);
      const found = await waitForReveal(control, target);
      if (found) {{
        const hasMoreControls = controls.slice(controlIndex + 1).some(next => !tried.includes(selector(next)));
        if (hasMoreControls && shouldDeferRevealedTarget(target, found)) {{
          if (!deferredMatch) deferredMatch = {{ control, key, found }};
          continue;
        }}
        const browserClickRecommended = wantsBrowserClick(found);
        const hashCleanupRecommended = isInPageActionLink(found);
        if (!browserClickRecommended) clickElement(found);
        await delay(browserClickRecommended ? 260 : 140);
        clearHashOnlyNavigation();
        return {{
          ok: true,
          target,
          clicked: selector(found),
          discoveredBy: key,
          mode: 'discover-click',
          revealState: expansionState(control),
          tried,
          browserClickRecommended,
          hashCleanupRecommended
        }};
      }}
    }}
  }}
  if (deferredMatch) {{
    clickElement(deferredMatch.control);
    await delay(120);
    const found = targetInRevealScope(deferredMatch.control, target) || deferredMatch.found;
    const browserClickRecommended = wantsBrowserClick(found);
    const hashCleanupRecommended = isInPageActionLink(found);
    if (!browserClickRecommended) clickElement(found);
    await delay(browserClickRecommended ? 260 : 140);
    clearHashOnlyNavigation();
    return {{
      ok: true,
      target,
      clicked: selector(found),
      discoveredBy: deferredMatch.key,
      mode: 'discover-click-deferred',
      revealState: expansionState(deferredMatch.control),
      tried,
      browserClickRecommended,
      hashCleanupRecommended
    }};
  }}
  if (tried.length) {{
    return {{
      ok: true,
      partial: true,
      target,
      targetFound: false,
      tried,
      mode: 'discovery-actions-exhausted',
      message: 'Discovery actions were attempted, but no visible clickable target matched after the page changed.'
    }};
  }}
  return {{ ok: false, error: 'discover_click could not reveal clickable target: ' + target, target, tried }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "discover_click timed out".to_string())?
        .map_err(|e| {
            format!(
                "discover_click failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let browser_click = if value
            .get("browserClickRecommended")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            if let Some(selector) = value.get("clicked").and_then(|v| v.as_str()) {
                match handlers::interaction::handle_click(
                    page,
                    state,
                    &json!({ "selector": selector }),
                )
                .await
                {
                    Ok(result) => json!({ "ok": true, "result": result }),
                    Err(error) => json!({ "ok": false, "error": error }),
                }
            } else {
                Value::Null
            }
        } else {
            Value::Null
        };
        let mut hash_cleanup = Value::Null;
        if value
            .get("hashCleanupRecommended")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let mut cleanup_results = Vec::new();
            for _ in 0..4 {
                sleep(Duration::from_millis(60)).await;
                match page
                    .evaluate_expression(
                        r#"(() => {
                          if (location.hash) {
                            try {
                              history.replaceState(history.state, document.title, location.pathname + location.search);
                            } catch (_) {}
                          }
                          return location.href;
                        })()"#,
                    )
                    .await
                {
                    Ok(result) => cleanup_results.push(result.value().cloned().unwrap_or(Value::Null)),
                    Err(error) => cleanup_results.push(json!({ "error": error.to_string() })),
                }
            }
            hash_cleanup = json!(cleanup_results);
        }
        Ok(json!({
            "discoverClick": value,
            "browserClick": browser_click,
            "hashCleanup": hash_cleanup,
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
