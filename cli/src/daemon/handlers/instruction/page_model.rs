use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;

use super::model::json_literal;

const PAGE_MODEL_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn accessible_text_helpers_js() -> &'static str {
    r#"
  function referencedText(el, attr) {
    const ids = String(el.getAttribute && el.getAttribute(attr) || '').split(/\s+/).filter(Boolean);
    if (!ids.length) return '';
    const rootNode = el.getRootNode && el.getRootNode();
    const texts = [];
    for (const id of ids) {
      const ref =
        (rootNode && rootNode.getElementById && rootNode.getElementById(id)) ||
        document.getElementById(id);
      if (ref) texts.push(ref.textContent || '');
    }
    return texts.join(' ');
  }
	  function structuralLabelText(el) {
	    const labels = [];
	    const rootNode = el.getRootNode && el.getRootNode();
    const fieldset = el.closest && el.closest('fieldset');
    const legend = fieldset && fieldset.querySelector('legend');
    if (legend) labels.push(legend.textContent || '');
    const headers = String(el.getAttribute && el.getAttribute('headers') || '').split(/\s+/).filter(Boolean);
    for (const id of headers) {
      const header =
        (rootNode && rootNode.getElementById && rootNode.getElementById(id)) ||
        document.getElementById(id);
      if (header) labels.push(header.textContent || '');
    }
    const cell = el.closest && el.closest('td,th');
    if (cell) {
      const rowHeader = cell.parentElement && cell.parentElement.querySelector('th');
      if (rowHeader && rowHeader !== cell) labels.push(rowHeader.textContent || '');
      const table = cell.closest('table');
      const index = Array.from(cell.parentElement ? cell.parentElement.children : []).indexOf(cell);
      if (table && index >= 0) {
        const columnHeader = table.querySelector('thead tr th:nth-child(' + (index + 1) + ')');
        if (columnHeader) labels.push(columnHeader.textContent || '');
      }
	    }
	    return labels.join(' ');
	  }
	  function nearbyLabelText(el) {
	    if (!el || !el.tagName) return '';
	    const controlSelector = [
	      'input',
	      'textarea',
	      'select',
	      'button',
	      'a',
	      '[role=button]',
	      '[role=link]',
	      '[role=checkbox]',
	      '[role=radio]',
	      '[role=switch]',
	      '[aria-pressed]',
	      '[role=combobox]',
	      '[role=listbox]',
	      '[contenteditable]:not([contenteditable="false"])'
	    ].join(',');
	    if (!el.matches || !el.matches(controlSelector)) return '';
	    function visibleTextNode(node) {
	      if (!node || !node.textContent) return '';
	      if (node.matches && node.matches(controlSelector)) return '';
	      try {
	        const style = node.nodeType === Node.ELEMENT_NODE ? getComputedStyle(node) : null;
	        if (style && (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity || 1) === 0)) return '';
	      } catch (_) {}
	      return String(node.textContent || '').replace(/\s+/g, ' ').trim();
	    }
	    function collectSibling(start, direction) {
	      const parts = [];
	      let node = start;
	      for (let depth = 0; node && depth < 2; depth += 1, node = direction < 0 ? node.previousElementSibling : node.nextElementSibling) {
	        const text = visibleTextNode(node);
	        if (text) parts.push(text);
	      }
	      return parts;
	    }
	    const labels = [];
	    const previousLabels = collectSibling(el.previousElementSibling, -1);
	    labels.push(...previousLabels);
	    const type = String(el.getAttribute && el.getAttribute('type') || '').toLowerCase();
	    const role = String(el.getAttribute && el.getAttribute('role') || '').toLowerCase();
	    const nextLabelIsCommon = type === 'checkbox' || type === 'radio' ||
	      ['checkbox', 'radio', 'switch'].includes(role);
	    if (!previousLabels.length || nextLabelIsCommon) {
	      labels.push(...collectSibling(el.nextElementSibling, 1));
	    }
	    const parent = el.parentElement;
	    if (parent && parent.children && parent.children.length <= 8) {
	      const controlCount = Array.from(parent.children)
	        .filter(child => child.matches && child.matches(controlSelector))
	        .length;
	      if (controlCount <= 1) {
	        for (const child of Array.from(parent.children)) {
	          if (child === el) continue;
	          const text = visibleTextNode(child);
	          if (text) labels.push(text);
	        }
	      }
	    }
	    const cell = el.closest && el.closest('td,th');
	    if (cell && cell.parentElement) {
	      const siblings = Array.from(cell.parentElement.children || []);
	      const index = siblings.indexOf(cell);
	      for (const offset of [-1, 1]) {
	        const sibling = siblings[index + offset];
	        const text = visibleTextNode(sibling);
	        if (text) labels.push(text);
	      }
	    }
	    const seen = new Set();
	    return labels
	      .map(text => text.replace(/\s+/g, ' ').trim())
	      .filter(text => text && text.length <= 240)
	      .filter(text => {
	        const key = text.toLowerCase();
	        if (seen.has(key)) return false;
	        seen.add(key);
	        return true;
	      })
	      .join(' ');
	  }
	  function semanticAttributeText(el) {
	    if (!el || !el.getAttribute) return '';
    const attrs = [
      'alt',
      'label',
      'aria-description',
      'aria-keyshortcuts',
      'autocomplete',
      'data-label',
      'data-name',
      'data-title',
      'data-description',
      'data-field',
      'data-field-name',
      'data-keywords',
      'data-tags',
      'data-alias',
      'data-aliases',
      'data-action',
      'data-command',
      'data-value',
      'data-placeholder',
      'data-testid'
    ];
    return attrs.map(attr => el.getAttribute(attr) || '').join(' ');
  }
  function associatedLabelText(el) {
    if (!el) return '';
    const labels = [];
    const seen = new Set();
    function add(text) {
      const cleaned = String(text || '').replace(/\s+/g, ' ').trim();
      if (!cleaned) return;
      const key = cleaned.toLowerCase();
      if (seen.has(key)) return;
      seen.add(key);
      labels.push(cleaned);
    }
    try {
      if (el.labels) {
        for (const label of Array.from(el.labels)) add(label.textContent || '');
      }
    } catch (_) {}
    if (el.id) {
      const rootNode = el.getRootNode && el.getRootNode();
      const roots = [];
      if (rootNode && rootNode.querySelectorAll) roots.push(rootNode);
      if (document && (!rootNode || rootNode !== document)) roots.push(document);
      for (const root of roots) {
        for (const label of Array.from(root.querySelectorAll ? root.querySelectorAll('label[for]') : [])) {
          if (label.htmlFor === el.id) add(label.textContent || '');
        }
      }
    }
    const wrappingLabel = el.closest && el.closest('label');
    if (wrappingLabel) add(wrappingLabel.textContent || '');
    return labels.join(' ');
  }
  function shadowHostText(el) {
    const parts = [];
    const seen = new Set();
    function add(text) {
      const cleaned = String(text || '').replace(/\s+/g, ' ').trim();
      if (!cleaned) return;
      const key = cleaned.toLowerCase();
      if (seen.has(key)) return;
      seen.add(key);
      parts.push(cleaned);
    }
    for (let node = el; node; ) {
      const rootNode = node.getRootNode && node.getRootNode();
      const host = rootNode && rootNode.host;
      if (!host || host === node) break;
      add(host.getAttribute && host.getAttribute('aria-label'));
      add(host.getAttribute && host.getAttribute('title'));
      add(host.getAttribute && host.getAttribute('name'));
      add(host.getAttribute && host.getAttribute('placeholder'));
      add(referencedText(host, 'aria-labelledby'));
      add(referencedText(host, 'aria-describedby'));
      add(host.getAttribute && host.getAttribute('aria-description'));
      add(host.getAttribute && semanticAttributeText(host));
      add(host.id);
      add(associatedLabelText(host));
      node = host;
    }
    return parts.join(' ');
  }
  function assignedSlotText(node) {
    if (!node) return '';
    if (node.nodeType === Node.TEXT_NODE) return node.textContent || '';
    if (node.nodeType !== Node.ELEMENT_NODE) return '';
    return [
      node.textContent || '',
      node.getAttribute && node.getAttribute('aria-label') || '',
      node.getAttribute && node.getAttribute('title') || '',
      node.getAttribute && semanticAttributeText(node) || '',
      node.getAttribute && node.getAttribute('value') || ''
    ].join(' ');
  }
  function slotText(el) {
    if (!el) return '';
    const slots = [];
    if (el.tagName && el.tagName.toLowerCase() === 'slot') slots.push(el);
    if (el.querySelectorAll) slots.push(...Array.from(el.querySelectorAll('slot')));
    const texts = [];
    for (const slot of slots) {
      const assigned = slot.assignedNodes ? slot.assignedNodes({ flatten: true }) : [];
      if (assigned.length) {
        for (const node of assigned) texts.push(assignedSlotText(node));
      } else {
        texts.push(slot.textContent || '');
      }
    }
    return texts.join(' ');
  }
  function svgReferenceText(node) {
    if (!node) return '';
    const parts = [];
    const nodes = [node].concat(Array.from(node.querySelectorAll ? node.querySelectorAll('svg, use, title, desc') : []));
    for (const current of nodes.slice(0, 24)) {
      const tag = current.tagName && current.tagName.toLowerCase();
      if (tag === 'title' || tag === 'desc') parts.push(current.textContent || '');
      const href =
        current.getAttribute && (current.getAttribute('href') || current.getAttribute('xlink:href'));
      const svgOwned = tag === 'use' || tag === 'symbol' || tag === 'svg' || !!current.ownerSVGElement;
      if (svgOwned && href && href.startsWith('#')) {
        const id = href.slice(1);
        const rootNode = current.getRootNode && current.getRootNode();
        const ref =
          (rootNode && rootNode.getElementById && rootNode.getElementById(id)) ||
          document.getElementById(id);
        if (ref) {
          parts.push(ref.textContent || '');
          for (const label of Array.from(ref.querySelectorAll ? ref.querySelectorAll('title, desc') : [])) {
            parts.push(label.textContent || '');
          }
        }
      }
    }
    return parts.join(' ');
  }
"#
}

pub(super) fn availability_helpers_js() -> &'static str {
    r#"
  function attrTrue(el, attr) {
    return String(el && el.getAttribute && el.getAttribute(attr) || 'false').toLowerCase() === 'true';
  }
  function parentElementOrHost(el) {
    if (!el) return null;
    if (el.parentElement) return el.parentElement;
    const rootNode = el.getRootNode && el.getRootNode();
    return rootNode && rootNode.host ? rootNode.host : null;
  }
  function isInsideFirstLegend(el, fieldset) {
    const firstLegend = fieldset && Array.from(fieldset.children || [])
      .find(child => child.tagName && child.tagName.toLowerCase() === 'legend');
    return !!(firstLegend && firstLegend.contains(el));
  }
  function hasHiddenAncestor(el) {
    for (let node = el; node && node.nodeType === Node.ELEMENT_NODE; node = parentElementOrHost(node)) {
      if (node.hidden || node.inert || attrTrue(node, 'aria-hidden')) return true;
    }
    return false;
  }
  function hasDisabledAncestor(el) {
    for (let node = el; node && node.nodeType === Node.ELEMENT_NODE; node = parentElementOrHost(node)) {
      const tag = node.tagName && node.tagName.toLowerCase();
      if (tag === 'fieldset' && node.disabled && !isInsideFirstLegend(el, node)) return true;
      if (tag !== 'fieldset' && (node.disabled || node.hasAttribute && node.hasAttribute('disabled') || attrTrue(node, 'aria-disabled'))) return true;
    }
    return false;
  }
  function unavailableForAction(el) {
    return !el || hasHiddenAncestor(el) || hasDisabledAncestor(el);
  }
  function unavailableForRead(el) {
    return !el || hasHiddenAncestor(el);
  }
  function isReadOnlyControl(el) {
    if (!el || !el.getAttribute) return false;
    return !!el.readOnly ||
      el.getAttribute('readonly') !== null ||
      attrTrue(el, 'aria-readonly');
  }
"#
}

pub(super) fn control_semantics_helpers_js() -> &'static str {
    r#"
  function isCustomElementHost(el) {
    return !!(el && el.tagName && el.tagName.toLowerCase().includes('-'));
  }
  function hasSemanticControlMetadata(el) {
    if (!el || !el.getAttribute) return false;
    const attrs = [
      'aria-label',
      'aria-labelledby',
      'label',
      'name',
      'placeholder',
      'autocomplete',
      'data-label',
      'data-name',
      'data-field',
      'data-field-name',
      'data-testid'
    ];
    if (attrs.some(attr => String(el.getAttribute(attr) || '').trim())) return true;
    return !!associatedLabelText(el);
  }
  function isCustomWritableValueElement(el) {
    if (!isCustomElementHost(el) || !('value' in el) || isReadOnlyControl(el)) return false;
    const valueType = typeof el.value;
    if (valueType === 'function' || valueType === 'symbol') return false;
    return hasSemanticControlMetadata(el);
  }
  function customControlSemanticText(el) {
    const customParts = [
      typeof textOf === 'function' ? textOf(el) : '',
      typeof classText === 'function' ? classText(el) : '',
      el && el.id || '',
      el && el.getAttribute && (el.getAttribute('name') || ''),
      el && el.getAttribute && (el.getAttribute('data-field') || ''),
      el && el.getAttribute && (el.getAttribute('data-field-name') || ''),
      el && el.getAttribute && (el.getAttribute('data-control') || ''),
      el && el.getAttribute && (el.getAttribute('inputmode') || ''),
      el && el.getAttribute && (el.getAttribute('aria-label') || ''),
      el && el.getAttribute && (el.getAttribute('title') || ''),
      el && el.getAttribute && (el.getAttribute('placeholder') || '')
    ];
    return customParts.join(' ').replace(/\s+/g, ' ').trim();
  }
  function customValueSemanticKind(el) {
    if (!isCustomWritableValueElement(el)) return '';
    const semanticText = customControlSemanticText(el);
    if (/\b(date[\s-]?time|datetime|timestamp)\b/i.test(semanticText)) return 'datetime-local';
    if (/\bmonth\b/i.test(semanticText)) return 'month';
    if (/\bweek\b/i.test(semanticText)) return 'week';
    if (/\btime\b/i.test(semanticText)) return 'time';
    if (/\b(date|day|calendar|birthday|birthdate|departure|arrival)\b/i.test(semanticText)) return 'date';
    if (/\b(dropdown|select|listbox|menu|choice|options?)\b/i.test(semanticText)) return 'select';
    if (/\b(colou?r|hex|palette)\b/i.test(semanticText)) return 'color';
    if (/\b(spinner|spinbutton|stepper|numeric|number|quantity|count|amount|limit|retries)\b/i.test(semanticText)) return 'number';
    if (/\b(slider|range)\b/i.test(semanticText)) return 'range';
    return '';
  }
  function isCustomSelectableValueElement(el) {
    return customValueSemanticKind(el) === 'select';
  }
  function isCustomNumericValueElement(el) {
    return customValueSemanticKind(el) === 'number';
  }
  function isCustomSliderValueElement(el) {
    return customValueSemanticKind(el) === 'range';
  }
  function isCustomCheckableElement(el) {
    if (!isCustomElementHost(el) || !('checked' in el) || isReadOnlyControl(el)) return false;
    return typeof el.checked === 'boolean' && hasSemanticControlMetadata(el);
  }
	"#
}

pub(super) fn value_control_helpers_js() -> &'static str {
    r#"
  function valueControlSelector() {
    return [
      'input:not([type=hidden])',
      'textarea',
      'select',
      '[contenteditable]:not([contenteditable="false"])',
      '[role=textbox]',
      '[role=searchbox]',
      '[role=spinbutton]',
      '[role=slider]',
      '[role=combobox]'
    ].join(',');
  }
  function isEditableValueControl(el) {
    const editable = el && el.getAttribute && el.getAttribute('contenteditable');
    return !!(el && (el.isContentEditable || (editable !== null && String(editable).toLowerCase() !== 'false')));
  }
  function isWritableValueControl(el) {
    if (!el || !el.tagName || isReadOnlyControl(el)) return false;
    const tag = el.tagName.toLowerCase();
    const type = String(el.getAttribute('type') || '').toLowerCase();
    const role = String(el.getAttribute('role') || '').toLowerCase();
    const fillableTypes = new Set(['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number', 'date', 'time', 'month', 'week', 'datetime-local', 'color', 'range']);
    const writableCombobox = role === 'combobox' && tag !== 'button';
    return tag === 'textarea' ||
      tag === 'select' ||
      isEditableValueControl(el) ||
      isCustomWritableValueElement(el) ||
      (tag === 'input' && fillableTypes.has(type)) ||
      ['textbox', 'searchbox', 'spinbutton', 'slider'].includes(role) ||
      writableCombobox;
  }
  function readControlValue(el) {
    if (!el) return '';
    if ('value' in el) return String(el.value || '');
    return String(el.textContent || '');
  }
  function dispatchControlValueChange(el, value, inputType = 'insertReplacementText') {
    try {
      el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType, data: String(value) }));
    } catch (_) {
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }
  function setControlValue(el, value, options = {}) {
    const text = String(value);
    try { el.focus(); } catch (_) {}
    const tag = el.tagName && el.tagName.toLowerCase();
    const role = String(el.getAttribute && el.getAttribute('role') || '').toLowerCase();
    if (tag === 'select') {
      const normalizeOption = options.normalize || (value => String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim());
      const wanted = normalizeOption(text);
      let option = Array.from(el.options || []).find(candidate => normalizeOption(candidate.value) === wanted || normalizeOption(candidate.textContent) === wanted);
      option = option || Array.from(el.options || []).find(candidate => normalizeOption(candidate.textContent).includes(wanted) || normalizeOption(candidate.value).includes(wanted));
      if (!option) return false;
      el.value = option.value;
    } else if (isEditableValueControl(el) || (!('value' in el) && ['textbox', 'searchbox', 'combobox'].includes(role))) {
      el.textContent = text;
    } else if (!('value' in el) && ['spinbutton', 'slider'].includes(role)) {
      const number = Number(text);
      if (Number.isFinite(number)) {
        const min = el.getAttribute('aria-valuemin');
        const max = el.getAttribute('aria-valuemax');
        let next = number;
        if (min !== null && min !== '' && Number.isFinite(Number(min))) next = Math.max(next, Number(min));
        if (max !== null && max !== '' && Number.isFinite(Number(max))) next = Math.min(next, Number(max));
        el.setAttribute('aria-valuenow', String(next));
        el.setAttribute('aria-valuetext', String(next));
        el.textContent = String(next);
      } else {
        el.textContent = text;
      }
    } else {
      const next = tag === 'input' && String(el.getAttribute('type') || '').toLowerCase() === 'date' && typeof normalizeDate === 'function'
        ? (normalizeDate(text) || text)
        : text;
      const proto = Object.getPrototypeOf(el);
      const setter = proto && Object.getOwnPropertyDescriptor(proto, 'value')?.set;
      if (setter) setter.call(el, next);
      else el.value = next;
    }
    dispatchControlValueChange(el, text, options.inputType || 'insertReplacementText');
    return true;
  }
"#
}

pub(super) async fn capture_instruction_page_model(
    page: &Page,
    scope: Option<&str>,
) -> Result<Value, String> {
    let scope_json = json_literal(&scope);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let js = format!(
        r#"(() => {{
  const scopeSelector = {scope_json};
  const root = scopeSelector ? document.querySelector(scopeSelector) : document;
  if (!root) return {{ ok: false, error: 'act_instruction: scope not found: ' + scopeSelector }};

  function normalized(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
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
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    const nameAttr = el.getAttribute('name');
    if (nameAttr) {{
      const rootNode = el.getRootNode && el.getRootNode();
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(nameAttr) + ']';
      if ((rootNode && rootNode.querySelectorAll ? rootNode : document).querySelectorAll(byName).length === 1) return byName;
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
  {accessible_text_helpers_js}
  function labelText(el) {{
    const labels = [];
    labels.push(associatedLabelText(el));
    labels.push(referencedText(el, 'aria-labelledby'));
    labels.push(referencedText(el, 'aria-describedby'));
	    labels.push(el.getAttribute('aria-description') || '');
	    labels.push(semanticAttributeText(el));
	    labels.push(shadowHostText(el));
	    labels.push(structuralLabelText(el));
	    labels.push(nearbyLabelText(el));
	    const seenLabels = new Set();
	    return labels
	      .map(label => String(label || '').replace(/\s+/g, ' ').trim())
	      .filter(label => {{
	        if (!label) return false;
	        const key = label.toLowerCase();
	        if (seenLabels.has(key)) return false;
	        seenLabels.add(key);
	        return true;
	      }})
	      .join(' ');
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
      semanticAttributeText(el),
      shadowHostText(el),
      slotText(el),
      svgReferenceText(el),
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
  {control_semantics_helpers_js}
  function isClickable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const type = typeOf(el);
    const classes = String(el.className && typeof el.className === 'string' ? el.className : '');
    let pointer = false;
    try {{
      pointer = getComputedStyle(el).cursor === 'pointer';
    }} catch (_) {{}}
    return tag === 'button' || tag === 'a' || type === 'button' || type === 'submit' ||
      ['button', 'link', 'option', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'tab'].includes(role) ||
      el.hasAttribute('onclick') || el.hasAttribute('tabindex') || pointer ||
      /\b(?:alink|button|link|close|tab|ui-button|ui-menu-item|ui-menu-item-wrapper)\b/i.test(classes);
  }}
  function isFillable(el) {{
    if (isReadOnlyControl(el)) return false;
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const type = typeOf(el);
    const fillableTypes = new Set(['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number', 'date', 'time', 'month', 'week', 'datetime-local', 'color', 'range']);
    const writableCombobox = role === 'combobox' && tag !== 'button';
    return tag === 'textarea' || isEditableElement(el) ||
      isCustomWritableValueElement(el) ||
      (tag === 'input' && fillableTypes.has(type)) ||
      ['textbox', 'searchbox', 'spinbutton', 'slider'].includes(role) ||
      writableCombobox;
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
      ['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio'].includes(role) ||
      isCustomCheckableElement(el) ||
      el.hasAttribute('aria-pressed');
  }}
  function isSlider(el) {{
    return typeOf(el) === 'range' || roleOf(el) === 'slider' || /\bslider\b/i.test(String(el.className || ''));
  }}
  function isDraggable(el) {{
    const role = roleOf(el);
    return el.draggable || el.getAttribute('draggable') === 'true' ||
      ['option', 'listitem'].includes(role) ||
      /\b(?:draggable|drag-handle|draghandle)\b/i.test(String(el.className || ''));
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
  function rootContext(el) {{
    const rootNode = el.getRootNode && el.getRootNode();
    if (rootNode && rootNode.host) {{
      return {{
        kind: 'shadow-root',
        host: selector(rootNode.host),
        hostTag: rootNode.host.tagName.toLowerCase(),
      }};
    }}
    if (rootNode && rootNode.defaultView && rootNode.defaultView.frameElement) {{
      const frame = rootNode.defaultView.frameElement;
      return {{
        kind: 'iframe',
        frame: selector(frame),
        title: frame.getAttribute('title') || rootNode.title || null,
        url: rootNode.location ? rootNode.location.href : null,
      }};
    }}
    return {{ kind: 'page' }};
  }}
  function elementRecord(el) {{
    const rect = el.getBoundingClientRect();
    const affordances = {{
      clickable: isClickable(el),
      fillable: isFillable(el),
      selectable: isSelectable(el),
      checkable: isCheckable(el),
      slider: isSlider(el),
      draggable: isDraggable(el),
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
      checked: 'checked' in el ? !!el.checked : (el.getAttribute('aria-checked') || el.getAttribute('aria-pressed') || null),
      expanded: el.getAttribute('aria-expanded'),
      bounds: {{
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      }},
      affordances,
      group: nearestGroup(el),
      context: rootContext(el),
    }};
  }}
  function allRoots(start) {{
    const roots = [];
    const seenRoots = new Set();
    function addRoot(scope) {{
      if (!scope || seenRoots.has(scope)) return;
      seenRoots.add(scope);
      roots.push(scope);
      const tree = scope.querySelectorAll ? Array.from(scope.querySelectorAll('*')) : [];
      for (const el of tree) {{
        if (el.shadowRoot) addRoot(el.shadowRoot);
        if (el.tagName && el.tagName.toLowerCase() === 'iframe') {{
          try {{
            if (el.contentDocument) addRoot(el.contentDocument);
          }} catch (_) {{}}
        }}
      }}
    }}
    addRoot(start);
    return roots;
  }}
  function all(selectorText) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(root)) {{
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
    }}
    return results;
  }}

  const selectorText = [
    'button', 'a', 'input', 'textarea', 'select',
    '[role]', '[onclick]', '[tabindex]', '[draggable=true]', '[draggable="true"]', '[contenteditable]:not([contenteditable="false"])',
    'summary', 'canvas', 'svg', 'tr', 'li', 'span', 'div',
    '.alink', '.ui-button', '.ui-dialog-titlebar-close', '.ui-menu-item', '.ui-menu-item-wrapper',
    '[class*=button]', '[class*=Button]', '[class*=link]', '[class*=Link]',
    '[class*=close]', '[class*=Close]', '[class*=tab]', '[class*=Tab]'
  ].join(',');
  const baseVisibleElements = all(selectorText).filter(visible);
  const visibleElements = baseVisibleElements
    .concat(all('*').filter(el => visible(el) && (isCustomWritableValueElement(el) || isCustomCheckableElement(el))))
    .filter((el, index, list) => list.indexOf(el) === index);
  const interactive = visibleElements.filter(el => {{
    return isClickable(el) || isFillable(el) || isSelectable(el) || isCheckable(el) || isSlider(el) || isDraggable(el) || isScrollable(el);
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
    draggable: elements.filter(item => item.affordances.draggable).length,
    scrollable: elements.filter(item => item.affordances.scrollable).length,
    dialogs: all('dialog, [role=dialog]').filter(visible).length,
    forms: all('form').filter(visible).length,
    rows: all('tr, li, [role=row], .row, [data-testid*=row]').filter(visible).length,
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
