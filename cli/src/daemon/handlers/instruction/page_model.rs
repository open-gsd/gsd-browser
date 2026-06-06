use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;

const PAGE_MODEL_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn capture_instruction_page_model(
    page: &Page,
    scope: Option<&str>,
) -> Result<Value, String> {
    let scope_json = serde_json::to_string(&scope).unwrap();
    let js = format!(
        r#"(() => {{
  const scopeSelector = {scope_json};
  const root = scopeSelector ? document.querySelector(scopeSelector) : document;
  if (!root) return {{ ok: false, error: 'act_instruction: scope not found: ' + scopeSelector }};

  function normalized(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function visible(el) {{
    if (!el || el.hidden || el.disabled) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' &&
      style.visibility !== 'hidden' &&
      Number(style.opacity || 1) !== 0;
  }}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    const nameAttr = el.getAttribute('name');
    if (nameAttr) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(nameAttr) + ']';
      if (document.querySelectorAll(byName).length === 1) return byName;
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
  function labelText(el) {{
    const labels = [];
    if (el.id) {{
      for (const label of document.querySelectorAll('label[for=' + JSON.stringify(el.id) + ']')) {{
        labels.push(label.textContent || '');
      }}
    }}
    const wrappingLabel = el.closest('label');
    if (wrappingLabel) labels.push(wrappingLabel.textContent || '');
    return labels.join(' ').trim();
  }}
  function textOf(el) {{
    return [
      el.textContent || '',
      el.value || '',
      el.getAttribute('name') || '',
      el.placeholder || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('role') || '',
      el.getAttribute('data-testid') || '',
      labelText(el),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
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
  function isClickable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const type = typeOf(el);
    return tag === 'button' || tag === 'a' || type === 'button' || type === 'submit' ||
      ['button', 'link', 'option', 'menuitem', 'tab'].includes(role) ||
      el.hasAttribute('onclick') || el.hasAttribute('tabindex');
  }}
  function isFillable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const type = typeOf(el);
    const fillableTypes = new Set(['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number', 'date', 'time', 'month', 'week', 'datetime-local', 'color', 'range']);
    return tag === 'textarea' || isEditableElement(el) ||
      (tag === 'input' && fillableTypes.has(type)) ||
      ['textbox', 'searchbox', 'spinbutton', 'slider'].includes(role);
  }}
  function isSelectable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    return tag === 'select' || ['combobox', 'listbox', 'menu'].includes(role) || el.hasAttribute('aria-haspopup');
  }}
  function isCheckable(el) {{
    const type = typeOf(el);
    const role = roleOf(el);
    return ['checkbox', 'radio'].includes(type) ||
      ['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio'].includes(role);
  }}
  function isSlider(el) {{
    return typeOf(el) === 'range' || roleOf(el) === 'slider' || /\bslider\b/i.test(String(el.className || ''));
  }}
  function isScrollable(el) {{
    return el.scrollHeight > el.clientHeight + 8 || el.scrollWidth > el.clientWidth + 8;
  }}
  function nearestGroup(el) {{
    const group = el.closest('form, dialog, [role=dialog], tr, li, article, section, .card, .row, [data-testid*=row], [data-testid*=card]');
    if (!group) return null;
    return {{
      selector: selector(group),
      tag: group.tagName.toLowerCase(),
      role: roleOf(group) || null,
      text: textOf(group).slice(0, 220),
    }};
  }}
  function elementRecord(el) {{
    const rect = el.getBoundingClientRect();
    const affordances = {{
      clickable: isClickable(el),
      fillable: isFillable(el),
      selectable: isSelectable(el),
      checkable: isCheckable(el),
      slider: isSlider(el),
      scrollable: isScrollable(el),
    }};
    return {{
      selector: selector(el),
      tag: el.tagName.toLowerCase(),
      role: roleOf(el) || null,
      type: typeOf(el) || null,
      name: el.getAttribute('name') || null,
      text: textOf(el).slice(0, 220),
      normalizedText: normalized(textOf(el)).slice(0, 220),
      label: labelText(el).slice(0, 160) || null,
      value: 'value' in el ? String(el.value || '').slice(0, 120) : null,
      checked: 'checked' in el ? !!el.checked : (el.getAttribute('aria-checked') || null),
      expanded: el.getAttribute('aria-expanded'),
      bounds: {{
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      }},
      affordances,
      group: nearestGroup(el),
    }};
  }}

  const selectorText = [
    'button', 'a', 'input', 'textarea', 'select',
    '[role]', '[onclick]', '[tabindex]', '[contenteditable]:not([contenteditable="false"])',
    'summary', 'canvas', 'svg', 'tr', 'li'
  ].join(',');
  const all = Array.from(root.querySelectorAll(selectorText));
  if (root !== document && root.matches && root.matches(selectorText)) all.unshift(root);
  const visibleElements = all.filter(visible);
  const interactive = visibleElements.filter(el => {{
    return isClickable(el) || isFillable(el) || isSelectable(el) || isCheckable(el) || isSlider(el) || isScrollable(el);
  }});
  const elements = interactive.slice(0, 120).map(elementRecord);
  const summary = {{
    url: location.href,
    title: document.title || '',
    bodyTextLength: String(document.body && document.body.innerText || '').length,
    visibleElements: visibleElements.length,
    interactiveElements: interactive.length,
    clickable: elements.filter(item => item.affordances.clickable).length,
    fillable: elements.filter(item => item.affordances.fillable).length,
    selectable: elements.filter(item => item.affordances.selectable).length,
    checkable: elements.filter(item => item.affordances.checkable).length,
    sliders: elements.filter(item => item.affordances.slider).length,
    scrollable: elements.filter(item => item.affordances.scrollable).length,
    dialogs: Array.from(document.querySelectorAll('dialog, [role=dialog]')).filter(visible).length,
    forms: Array.from(document.querySelectorAll('form')).filter(visible).length,
    rows: Array.from(document.querySelectorAll('tr, li, [role=row], .row, [data-testid*=row]')).filter(visible).length,
  }};
  return {{
    ok: true,
    version: 1,
    scope: scopeSelector,
    summary,
    elements,
  }};
}})()"#
    );
    let result = timeout(PAGE_MODEL_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "act_instruction: page model capture timed out".to_string())?
        .map_err(|e| {
            format!(
                "act_instruction: page model capture failed: {}",
                super::super::clean_cdp_error(&e)
            )
        })?;
    let model = result.value().cloned().unwrap_or_else(|| json!({}));
    if model
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        Ok(model)
    } else {
        Err(model
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("act_instruction: page model capture failed")
            .to_string())
    }
}
