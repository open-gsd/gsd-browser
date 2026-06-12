use crate::daemon::handlers::clean_cdp_error;
use crate::daemon::state::DaemonState;
use chromiumoxide::Page;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;

const INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);

const INSPECTION_HELPERS_JS: &str = r##"
const pi = window.__pi || {};

function normalizeText(text) {
  return String(text || "").replace(/\s+/g, " ").trim();
}

function inferRole(el) {
  return pi.inferRole ? pi.inferRole(el) : "";
}

function accessibleName(el) {
  if (pi.accessibleName) return pi.accessibleName(el);
  return normalizeText(el.innerText || el.textContent || "").slice(0, 80);
}

function isVisible(el) {
  if (pi.isVisible) return pi.isVisible(el);
  const style = window.getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden") return false;
  const rect = el.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}

function isEnabled(el) {
  if (pi.isEnabled) return pi.isEnabled(el);
  return !el.disabled;
}

function selectorHint(el) {
  if (pi.selectorHints) {
    const hints = pi.selectorHints(el);
    if (hints && hints.length > 0) return hints[0];
  }
  return "";
}

function selectorHints(el) {
  if (pi.selectorHints) {
    const hints = pi.selectorHints(el);
    if (Array.isArray(hints)) return hints;
  }
  const hint = selectorHint(el);
  return hint ? [hint] : [];
}

function simpleHash(text) {
  if (pi.simpleHash) return pi.simpleHash(text);
  let hash = 0;
  const input = String(text || "");
  for (let i = 0; i < input.length; i++) {
    hash = ((hash << 5) - hash) + input.charCodeAt(i);
    hash |= 0;
  }
  return String(hash >>> 0);
}

function frameLabel(name, index) {
  return name || (index === 0 ? "main" : `frame-${index}`);
}

function collectFrameEntries() {
  const entries = [];
  const boundaries = [];

  function walk(win, depth) {
    if (depth > 10) return;
    let name = "";
    let url = "";
    let accessible = true;

    try {
      name = win.name || "";
    } catch (err) {}

    try {
      url = win.location.href || "";
      void win.document;
    } catch (err) {
      accessible = false;
      url = "(cross-origin)";
      boundaries.push({
        name: name || "",
        url,
        reason: "cross-origin frame",
      });
    }

    const index = entries.length;
    entries.push({
      win,
      name,
      url,
      index,
      accessible,
    });

    let childCount = 0;
    try {
      childCount = win.frames.length;
    } catch (err) {
      childCount = 0;
    }

    for (let i = 0; i < childCount; i++) {
      try {
        walk(win.frames[i], depth + 1);
      } catch (err) {}
    }
  }

  walk(window.top || window, 0);
  return { entries, boundaries };
}

function contextFromEntry(entry) {
  if (!entry || !entry.accessible) {
    return {
      ok: false,
      error: "selected frame is cross-origin or unavailable",
      crossOrigin: true,
    };
  }

  return {
    ok: true,
    context: {
      win: entry.win,
      doc: entry.win.document,
      label: frameLabel(entry.name, entry.index),
      url: entry.url || "",
      index: entry.index,
    },
  };
}

function resolveFrameContext(frameSpec) {
  const { entries, boundaries } = collectFrameEntries();

  if (!frameSpec || !frameSpec.kind) {
    const resolved = contextFromEntry(entries[0]);
    if (!resolved.ok) return { ...resolved, boundaries };
    return { ok: true, context: resolved.context, boundaries };
  }

  if (frameSpec.kind === "main") {
    const resolved = contextFromEntry(entries[0]);
    if (!resolved.ok) return { ...resolved, boundaries };
    return { ok: true, context: resolved.context, boundaries };
  }

  let match = null;
  if (frameSpec.kind === "index") {
    match = entries.find((entry) => entry.index === frameSpec.value);
  } else if (frameSpec.kind === "name") {
    match = entries.find((entry) => entry.name === frameSpec.value);
  } else if (frameSpec.kind === "urlPattern") {
    match = entries.find((entry) => (entry.url || "").includes(frameSpec.value));
  }

  if (!match) {
    return {
      ok: false,
      error: "selected frame was not found",
      boundaries,
    };
  }

  const resolved = contextFromEntry(match);
  if (!resolved.ok) return { ...resolved, boundaries };
  return { ok: true, context: resolved.context, boundaries };
}

function collectContexts(frameSpec, includeFrames) {
  if (frameSpec && frameSpec.kind) {
    const resolved = resolveFrameContext(frameSpec);
    if (!resolved.ok) return resolved;
    return { ok: true, contexts: [resolved.context], boundaries: resolved.boundaries };
  }

  const { entries, boundaries } = collectFrameEntries();
  const contexts = [];
  for (const entry of entries) {
    if (!entry.accessible) continue;
    if (!includeFrames && entry.index !== 0) continue;
    contexts.push({
      win: entry.win,
      doc: entry.win.document,
      label: frameLabel(entry.name, entry.index),
      url: entry.url || "",
      index: entry.index,
    });
  }
  return { ok: true, contexts, boundaries };
}

function collectQueryRoots(root) {
  const roots = [root];
  const tree = root.querySelectorAll ? root.querySelectorAll("*") : [];
  for (const el of tree) {
    if (el.shadowRoot) {
      roots.push(el.shadowRoot);
    }
  }
  return roots;
}

function queryAllDeep(root, selector) {
  const roots = collectQueryRoots(root);
  const results = [];
  const seen = new Set();
  for (const scope of roots) {
    let matches;
    try {
      matches = selector ? scope.querySelectorAll(selector) : scope.querySelectorAll("*");
    } catch (err) {
      return { ok: false, error: "invalid selector: " + err.message };
    }
    for (const el of matches) {
      if (seen.has(el)) continue;
      seen.add(el);
      results.push(el);
    }
  }
  return { ok: true, elements: results };
}

function deepDomPath(el) {
  const path = [];
  let current = el;
  const rootDoc = el && el.ownerDocument ? el.ownerDocument : document;

  while (current && current !== rootDoc.documentElement) {
    const parent = current.parentNode;
    if (!parent) break;

    if (parent instanceof ShadowRoot) {
      const idx = Array.from(parent.children || []).indexOf(current);
      if (idx < 0) return [];
      path.push({ kind: "child", index: idx });
      path.push({ kind: "shadow" });
      current = parent.host;
      continue;
    }

    const idx = Array.from(parent.children || []).indexOf(current);
    if (idx < 0) return [];
    path.push({ kind: "child", index: idx });
    current = parent;
  }

  return path.reverse();
}

function resolveDeepPath(doc, path) {
  let current = doc.documentElement;
  for (const segment of path || []) {
    if (segment.kind === "shadow") {
      if (!current || !current.shadowRoot) return null;
      current = current.shadowRoot;
      continue;
    }

    const children = current && current.children ? current.children : null;
    if (!children || segment.index < 0 || segment.index >= children.length) {
      return null;
    }
    current = children[segment.index];
  }

  return current;
}

function absoluteCenter(context, el) {
  const rect = el.getBoundingClientRect();
  let x = rect.left + rect.width / 2;
  let y = rect.top + rect.height / 2;
  let win = context.win;

  while (win && win !== win.top) {
    const frameEl = win.frameElement;
    if (!frameEl) break;
    const frameRect = frameEl.getBoundingClientRect();
    x += frameRect.left;
    y += frameRect.top;
    win = win.parent;
  }

  return { x, y };
}

function absolutePoint(context, point) {
  let x = point.x;
  let y = point.y;
  let win = context.win;

  while (win && win !== win.top) {
    const frameEl = win.frameElement;
    if (!frameEl) break;
    const frameRect = frameEl.getBoundingClientRect();
    x += frameRect.left;
    y += frameRect.top;
    win = win.parent;
  }

  return { x, y };
}

function absoluteBounds(context, el) {
  const rect = el.getBoundingClientRect();
  let x = rect.left;
  let y = rect.top;
  let win = context.win;

  while (win && win !== win.top) {
    const frameEl = win.frameElement;
    if (!frameEl) break;
    const frameRect = frameEl.getBoundingClientRect();
    x += frameRect.left;
    y += frameRect.top;
    win = win.parent;
  }

  return { x, y, w: rect.width, h: rect.height };
}

function elementSummary(el, context) {
  const text = normalizeText(el.innerText || el.textContent || "");
  return {
    tag: el.tagName.toLowerCase(),
    role: inferRole(el),
    name: accessibleName(el),
    selector_hint: selectorHint(el),
    visible: isVisible(el),
    enabled: isEnabled(el),
    text: text.slice(0, 80),
    frameLabel: context.label,
    frameUrl: context.url,
    value: el.value !== undefined ? String(el.value) : null,
    checked: !!el.checked,
  };
}

function hitBelongsToElement(el, hit) {
  if (!hit) return false;
  if (hit === el) return true;
  if (el.contains && el.contains(hit)) return true;
  const hitRoot = hit.getRootNode && hit.getRootNode();
  if (hitRoot && hitRoot.host && (hitRoot.host === el || (el.contains && el.contains(hitRoot.host)))) {
    return true;
  }
  const root = el.getRootNode && el.getRootNode();
  if (root && root.host && hitBelongsToElement(root.host, hit)) return true;
  return false;
}

function localActionPoint(context, el) {
  if (!isVisible(el)) {
    return { ok: false, reason: "element is not visible" };
  }
  if (!isEnabled(el) || el.getAttribute("aria-disabled") === "true") {
    return { ok: false, reason: "element is disabled" };
  }

  const blocks = ["center", "nearest", "start", "end"];
  let last = null;
  for (const block of blocks) {
    if (el.scrollIntoView) {
      el.scrollIntoView({ block, inline: "center", behavior: "instant" });
    }

    const rect = el.getBoundingClientRect();
    const viewportW = context.win.innerWidth || context.doc.documentElement.clientWidth || 0;
    const viewportH = context.win.innerHeight || context.doc.documentElement.clientHeight || 0;
    const left = Math.max(0, rect.left);
    const right = Math.min(viewportW, rect.right);
    const top = Math.max(0, rect.top);
    const bottom = Math.min(viewportH, rect.bottom);
    if (right <= left || bottom <= top) {
      last = { ok: false, reason: "element is outside the viewport after scroll", block };
      continue;
    }

    const samples = [
      { x: (left + right) / 2, y: (top + bottom) / 2 },
      { x: left + (right - left) * 0.25, y: top + (bottom - top) * 0.25 },
      { x: left + (right - left) * 0.75, y: top + (bottom - top) * 0.25 },
      { x: left + (right - left) * 0.25, y: top + (bottom - top) * 0.75 },
      { x: left + (right - left) * 0.75, y: top + (bottom - top) * 0.75 },
    ];

    for (const sample of samples) {
      const hit = context.doc.elementFromPoint(sample.x, sample.y);
      if (hitBelongsToElement(el, hit)) {
        return {
          ok: true,
          x: sample.x,
          y: sample.y,
          block,
          covered: false,
          hit: hit ? elementSummary(hit, context) : null,
        };
      }
      last = {
        ok: false,
        reason: "element point is covered",
        block,
        x: sample.x,
        y: sample.y,
        hit: hit ? elementSummary(hit, context) : null,
      };
    }
  }

  const rect = el.getBoundingClientRect();
  return {
    ...(last || { ok: false, reason: "no actionable point found" }),
    fallback: { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 },
  };
}

function prepareActionTarget(context, el) {
  const point = localActionPoint(context, el);
  if (!point.ok && point.fallback) {
    return { ...point, absolute: absolutePoint(context, point.fallback) };
  }
  return { ...point, absolute: point.ok ? absolutePoint(context, point) : null };
}

function isTextInputType(el) {
  if (!el || !el.tagName) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "textarea") return true;
  if (tag !== "input") return false;
  const type = String(el.getAttribute("type") || "text").toLowerCase();
  return ![
    "button",
    "checkbox",
    "file",
    "hidden",
    "image",
    "radio",
    "range",
    "reset",
    "submit",
  ].includes(type);
}

function editableKind(el) {
  if ("value" in el && isTextInputType(el)) {
    return "value";
  }
  const contentEditable = el.getAttribute("contenteditable");
  const hasEditableAttr = contentEditable !== null && contentEditable.toLowerCase() !== "false";
  const hasTextboxRole = String(el.getAttribute("role") || "").split(/\s+/).includes("textbox");
  if (el.isContentEditable || hasEditableAttr || hasTextboxRole) {
    return "contenteditable";
  }
  return "";
}

function readEditableValue(el) {
  const kind = editableKind(el);
  if (kind === "value") return String(el.value || "");
  if (kind === "contenteditable") return String(el.textContent || "");
  return "";
}

function setNativeValue(el, value) {
  const next = String(value);
  const ownSetter = Object.getOwnPropertyDescriptor(el, "value")?.set;
  const proto = Object.getPrototypeOf(el);
  const protoSetter = proto ? Object.getOwnPropertyDescriptor(proto, "value")?.set : null;
  if (protoSetter && ownSetter !== protoSetter) {
    protoSetter.call(el, next);
  } else if (ownSetter) {
    ownSetter.call(el, next);
  } else {
    el.value = next;
  }
}

function dispatchTextEvents(context, el, value, inputType, data) {
  const base = { bubbles: true, cancelable: true, composed: true };
  try {
    el.dispatchEvent(new context.win.InputEvent("beforeinput", {
      ...base,
      inputType,
      data: data == null ? null : String(data),
    }));
  } catch (_) {
    el.dispatchEvent(new Event("beforeinput", base));
  }
  try {
    el.dispatchEvent(new context.win.InputEvent("input", {
      ...base,
      inputType,
      data: data == null ? null : String(data),
    }));
  } catch (_) {
    el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  }
  el.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  if ("value" in el) {
    el.setAttribute("value", value);
  }
}

function dispatchKeyTriplet(context, el, key) {
  const init = { key, bubbles: true, cancelable: true, composed: true, view: context.win };
  el.dispatchEvent(new context.win.KeyboardEvent("keydown", init));
  el.dispatchEvent(new context.win.KeyboardEvent("keypress", init));
  el.dispatchEvent(new context.win.KeyboardEvent("keyup", init));
}

function valuesEquivalent(el, actual, expected) {
  if (actual === expected) return true;
  const type = String(el.getAttribute("type") || "").toLowerCase();
  if ((type === "number" || type === "range") && actual !== "" && expected !== "") {
    return Number(actual) === Number(expected);
  }
  if (type === "color" && actual && expected) {
    return actual.toLowerCase() === expected.toLowerCase();
  }
  return false;
}

function robustFillElement(context, el, options) {
  const text = String(options.text || "");
  const clearFirst = !!options.clearFirst;
  const slowly = !!options.slowly;
  const submit = !!options.submit;
  const kind = editableKind(el);

  if (!kind) {
    return { ok: false, error: "element does not support text input" };
  }
  if (!isVisible(el)) {
    return { ok: false, error: "element is not visible" };
  }
  if (!isEnabled(el) || el.getAttribute("aria-disabled") === "true") {
    return { ok: false, error: "element is disabled" };
  }
  if (el.readOnly || el.getAttribute("aria-readonly") === "true") {
    return { ok: false, error: "element is read-only" };
  }

  const before = readEditableValue(el);
  const expected = slowly && !clearFirst ? before + text : text;
  if (typeof el.focus === "function") el.focus();

  if (kind === "value" && typeof el.select === "function") {
    try { el.select(); } catch (_) {}
  }

  if (clearFirst) {
    if (kind === "value") setNativeValue(el, "");
    else el.textContent = "";
    dispatchTextEvents(context, el, "", "deleteContentBackward", null);
  }

  if (slowly) {
    let next = clearFirst ? "" : before;
    for (const ch of text) {
      dispatchKeyTriplet(context, el, ch);
      next += ch;
      if (kind === "value") setNativeValue(el, next);
      else el.textContent = next;
      dispatchTextEvents(context, el, next, "insertText", ch);
    }
  } else {
    if (kind === "value") setNativeValue(el, expected);
    else el.textContent = expected;
    dispatchTextEvents(context, el, expected, "insertText", text);
  }

  let actual = readEditableValue(el);
  let method = "native-setter";
  if (!valuesEquivalent(el, actual, expected)) {
    if (kind === "value") el.value = expected;
    else el.textContent = expected;
    dispatchTextEvents(context, el, expected, "insertReplacementText", text);
    actual = readEditableValue(el);
    method = "direct-fallback";
  }

  if (!valuesEquivalent(el, actual, expected)) {
    return {
      ok: false,
      error: "value verification failed",
      expected,
      actual,
      before,
      kind,
      method,
    };
  }

  let submitted = false;
  if (submit) {
    dispatchKeyTriplet(context, el, "Enter");
    const form = el.form || el.closest("form");
    if (form) {
      if (typeof form.requestSubmit === "function") {
        form.requestSubmit();
      } else if (typeof form.submit === "function") {
        form.submit();
      }
      submitted = true;
    }
  }

  return { ok: true, before, actual, expected, kind, method, submitted };
}

function looseOptionText(text) {
  return normalizeText(text).toLowerCase().replace(/[^a-z0-9]+/g, " ").replace(/\s+/g, " ").trim();
}

function optionValues(input) {
  if (Array.isArray(input)) return input.map((value) => String(value));
  if (input === undefined || input === null) return [""];
  return [String(input)];
}

function optionTextCandidates(el) {
  return [
    el.value,
    el.label,
    el.getAttribute && el.getAttribute("value"),
    el.getAttribute && el.getAttribute("data-value"),
    el.getAttribute && el.getAttribute("data-testid"),
    el.getAttribute && el.getAttribute("aria-label"),
    el.getAttribute && el.getAttribute("title"),
    accessibleName(el),
    el.innerText,
    el.textContent,
  ].filter((value) => value !== undefined && value !== null && String(value).trim() !== "");
}

function optionMatchScore(option, wanted) {
  const rawWanted = String(wanted);
  const normalizedWanted = normalizeText(rawWanted);
  const looseWanted = looseOptionText(rawWanted);
  let best = 0;
  for (const candidate of optionTextCandidates(option)) {
    const rawCandidate = String(candidate);
    const normalizedCandidate = normalizeText(rawCandidate);
    const looseCandidate = looseOptionText(rawCandidate);
    if (rawCandidate === rawWanted || normalizedCandidate === normalizedWanted) best = Math.max(best, 100);
    if (looseCandidate === looseWanted) best = Math.max(best, 95);
    if (looseWanted.length >= 2 && looseCandidate.includes(looseWanted)) best = Math.max(best, 70);
    if (looseCandidate.length >= 3 && looseWanted.includes(looseCandidate)) best = Math.max(best, 70);
  }
  return best;
}

function selectedOptionSnapshot(selectEl) {
  return Array.from(selectEl.selectedOptions || []).map((option) => ({
    value: String(option.value || ""),
    label: String(option.label || ""),
    text: normalizeText(option.textContent || ""),
  }));
}

function dispatchSelectEvents(el) {
  el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  el.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
}

function clickLike(context, el) {
  if (typeof el.focus === "function") el.focus();
  if (typeof el.click === "function") {
    el.click();
  } else {
    el.dispatchEvent(new context.win.MouseEvent("mousedown", { bubbles: true, cancelable: true, view: context.win }));
    el.dispatchEvent(new context.win.MouseEvent("mouseup", { bubbles: true, cancelable: true, view: context.win }));
    el.dispatchEvent(new context.win.MouseEvent("click", { bubbles: true, cancelable: true, view: context.win }));
  }
}

function selectNativeOption(context, el, wantedValues) {
  const allOptions = Array.from(el.options || []);
  if (allOptions.length === 0) return { ok: false, error: "select has no options" };
  if (!wantedValues.length) return { ok: false, error: "option array cannot be empty" };
  if (!el.multiple && wantedValues.length > 1) {
    return { ok: false, error: "multiple options require a multi-select element", expected: wantedValues };
  }

  const matched = [];
  for (const wanted of wantedValues) {
    const ranked = allOptions
      .map((option) => ({ option, score: optionMatchScore(option, wanted) }))
      .filter((entry) => entry.score > 0 && !entry.option.disabled)
      .sort((a, b) => b.score - a.score);
    if (ranked.length === 0) return { ok: false, error: "option not found: " + wanted };
    matched.push({ wanted, option: ranked[0].option, score: ranked[0].score });
  }

  if (el.multiple) {
    for (const option of allOptions) option.selected = false;
    for (const entry of matched) entry.option.selected = true;
  } else {
    matched[0].option.selected = true;
    el.value = matched[0].option.value;
  }
  dispatchSelectEvents(el);

  const selected = selectedOptionSnapshot(el);
  const verified = wantedValues.every((wanted) =>
    selected.some((option) => optionMatchScore({
      value: option.value,
      label: option.label,
      getAttribute: () => "",
      innerText: option.text,
      textContent: option.text,
    }, wanted) > 0)
  );
  if (!verified) {
    return { ok: false, error: "selection verification failed", selected, expected: wantedValues };
  }

  return {
    ok: true,
    mode: "native-select",
    selected,
    matched: matched.map((entry) => ({
      wanted: entry.wanted,
      value: String(entry.option.value || ""),
      label: String(entry.option.label || ""),
      text: normalizeText(entry.option.textContent || ""),
      score: entry.score,
    })),
  };
}

function customOptionRoots(context, el) {
  const roots = [];
  const doc = context.doc;
  const add = (node) => {
    if (node && !roots.includes(node)) roots.push(node);
  };

  add(el);
  const controls = el.getAttribute && el.getAttribute("aria-controls");
  if (controls) {
    for (const id of controls.split(/\s+/)) add(doc.getElementById(id));
  }
  const owns = el.getAttribute && el.getAttribute("aria-owns");
  if (owns) {
    for (const id of owns.split(/\s+/)) add(doc.getElementById(id));
  }
  add(el.closest && el.closest("[role=listbox], [role=menu], [role=radiogroup], form, body"));
  add(doc.body);
  return roots;
}

function findCustomOption(context, el, wanted) {
  const selector = [
    "[role=option]",
    "[role=menuitem]",
    "[role=menuitemradio]",
    "[role=menuitemcheckbox]",
    "[data-value]",
    "[aria-selected]",
    "li",
    "button",
  ].join(",");

  const seen = new Set();
  const candidates = [];
  for (const root of customOptionRoots(context, el)) {
    const nodes = root.querySelectorAll ? Array.from(root.querySelectorAll(selector)) : [];
    for (const node of nodes) {
      if (seen.has(node) || node === el) continue;
      seen.add(node);
      if (!isVisible(node) || !isEnabled(node)) continue;
      const score = optionMatchScore(node, wanted);
      if (score > 0) candidates.push({ node, score });
    }
  }
  candidates.sort((a, b) => b.score - a.score);
  return candidates[0] || null;
}

function selectCustomOption(context, el, wantedValues) {
  if (wantedValues.length !== 1) {
    return { ok: false, error: "custom dropdown selection supports one option at a time" };
  }
  const wanted = wantedValues[0];

  clickLike(context, el);
  const match = findCustomOption(context, el, wanted);
  if (!match) return { ok: false, error: "option not found: " + wanted };

  clickLike(context, match.node);
  const selected = {
    value: "value" in el ? String(el.value || "") : null,
    text: normalizeText(el.innerText || el.textContent || ""),
    optionText: normalizeText(match.node.innerText || match.node.textContent || ""),
    optionRole: inferRole(match.node),
    optionSelected: match.node.getAttribute("aria-selected"),
  };
  return {
    ok: true,
    mode: "custom-option",
    selected,
    matched: [{
      wanted,
      text: selected.optionText,
      value: match.node.getAttribute("data-value") || match.node.getAttribute("value") || "",
      score: match.score,
    }],
  };
}

function selectOptionElement(context, el, options) {
  const wantedValues = optionValues(options.option);
  if (!isVisible(el)) return { ok: false, error: "select target is not visible" };
  if (!isEnabled(el) || el.getAttribute("aria-disabled") === "true") {
    return { ok: false, error: "select target is disabled" };
  }

  if (el.tagName && el.tagName.toLowerCase() === "select") {
    return selectNativeOption(context, el, wantedValues);
  }

  const role = inferRole(el) || (el.getAttribute && el.getAttribute("role")) || "";
  if (["combobox", "listbox", "menu", "button", "textbox"].includes(role) || el.hasAttribute("aria-haspopup")) {
    return selectCustomOption(context, el, wantedValues);
  }

  return { ok: false, error: "element is not a select, combobox, listbox, or menu trigger" };
}

function dispatchValueEvents(el) {
  el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  el.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
}

function parseDateValue(raw) {
  const text = String(raw || "").trim();
  if (/^\d{4}-\d{2}-\d{2}$/.test(text)) return text;
  let match = text.match(/^(\d{1,2})[\/.-](\d{1,2})[\/.-](\d{2,4})$/);
  if (match) {
    const month = match[1].padStart(2, "0");
    const day = match[2].padStart(2, "0");
    let year = match[3];
    if (year.length === 2) year = Number(year) >= 70 ? "19" + year : "20" + year;
    return `${year}-${month}-${day}`;
  }
  match = text.match(/^([A-Za-z]+)\s+(\d{1,2}),?\s+(\d{4})$/);
  if (match) {
    const months = {
      january: 1, jan: 1, february: 2, feb: 2, march: 3, mar: 3, april: 4, apr: 4,
      may: 5, june: 6, jun: 6, july: 7, jul: 7, august: 8, aug: 8, september: 9, sep: 9,
      sept: 9, october: 10, oct: 10, november: 11, nov: 11, december: 12, dec: 12,
    };
    const month = months[match[1].toLowerCase()];
    if (month) return `${match[3]}-${String(month).padStart(2, "0")}-${match[2].padStart(2, "0")}`;
  }
  return text;
}

function parseTimeValue(raw) {
  const text = String(raw || "").trim();
  if (/^\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?$/.test(text)) return text;
  const match = text.match(/^(\d{1,2})(?::(\d{2}))?\s*(am|pm)$/i);
  if (!match) return text;
  let hour = Number(match[1]);
  const minute = match[2] || "00";
  const period = match[3].toLowerCase();
  if (period === "pm" && hour < 12) hour += 12;
  if (period === "am" && hour === 12) hour = 0;
  return `${String(hour).padStart(2, "0")}:${minute}`;
}

function parseMonthValue(raw) {
  const text = String(raw || "").trim();
  if (/^\d{4}-\d{2}$/.test(text)) return text;
  const match = text.match(/^([A-Za-z]+)\s+(\d{4})$/);
  if (!match) return text;
  const months = {
    january: 1, jan: 1, february: 2, feb: 2, march: 3, mar: 3, april: 4, apr: 4,
    may: 5, june: 6, jun: 6, july: 7, jul: 7, august: 8, aug: 8, september: 9, sep: 9,
    sept: 9, october: 10, oct: 10, november: 11, nov: 11, december: 12, dec: 12,
  };
  const month = months[match[1].toLowerCase()];
  return month ? `${match[2]}-${String(month).padStart(2, "0")}` : text;
}

function parseColorValue(raw) {
  const text = String(raw || "").trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/i.test(text)) return text;
  if (/^#[0-9a-f]{3}$/i.test(text)) {
    return "#" + text.slice(1).split("").map((ch) => ch + ch).join("");
  }
  const colors = {
    black: "#000000", white: "#ffffff", red: "#ff0000", green: "#008000", blue: "#0000ff",
    yellow: "#ffff00", orange: "#ffa500", purple: "#800080", magenta: "#ff00ff",
    fuchsia: "#ff00ff", cyan: "#00ffff", aqua: "#00ffff", gray: "#808080", grey: "#808080",
  };
  return colors[text] || String(raw || "");
}

function typedValueForInput(el, rawValue) {
  const type = String(el.getAttribute("type") || "text").toLowerCase();
  const wanted = String(rawValue ?? "");
  if (type === "date") return parseDateValue(wanted);
  if (type === "time") return parseTimeValue(wanted);
  if (type === "month") return parseMonthValue(wanted);
  if (type === "week") return wanted.trim();
  if (type === "datetime-local") {
    const text = wanted.trim();
    if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(text)) return text;
    const parts = text.split(/\s+(?:at\s+)?/i);
    if (parts.length >= 2) return `${parseDateValue(parts[0])}T${parseTimeValue(parts.slice(1).join(" "))}`;
    return text;
  }
  if (type === "color") return parseColorValue(wanted);
  return wanted;
}

function setTypedControlValue(el, rawValue) {
  if (!el || !el.tagName) return null;
  const tag = el.tagName.toLowerCase();
  const role = String(el.getAttribute("role") || "").toLowerCase();
  const type = tag === "input" ? String(el.getAttribute("type") || "text").toLowerCase() : "";
  const supportedInputTypes = new Set(["range", "number", "date", "time", "month", "week", "datetime-local", "color"]);
  const supportedRole = role === "slider" || role === "spinbutton";
  if (!(tag === "input" && supportedInputTypes.has(type)) && !supportedRole) return null;

  const wanted = String(rawValue ?? "");
  const kind = type || role;
  if (wanted.trim() === "") {
    if (type === "number") {
      setNativeValue(el, "");
      dispatchValueEvents(el);
      return { ok: el.value === "", kind, expected: "", actual: String(el.value || ""), method: "typed-empty" };
    }
    return { ok: false, kind, expected: wanted, actual: String(el.value || ""), error: kind + " value cannot be empty" };
  }

  if (type === "range" || type === "number" || supportedRole) {
    const numeric = Number(wanted);
    if (!Number.isFinite(numeric)) {
      return { ok: false, kind, expected: wanted, actual: String(el.value || el.getAttribute("aria-valuenow") || ""), error: "value is not numeric" };
    }
    let next = numeric;
    const min = el.getAttribute("min") ?? el.getAttribute("aria-valuemin");
    const max = el.getAttribute("max") ?? el.getAttribute("aria-valuemax");
    if (min !== null && min !== "" && Number.isFinite(Number(min))) next = Math.max(next, Number(min));
    if (max !== null && max !== "" && Number.isFinite(Number(max))) next = Math.min(next, Number(max));
    if (tag === "input") {
      setNativeValue(el, String(next));
    } else {
      el.setAttribute("aria-valuenow", String(next));
      if ("value" in el) {
        try { el.value = String(next); } catch (_) {}
      }
    }
    dispatchValueEvents(el);
    const actual = tag === "input" ? String(el.value || "") : String(el.getAttribute("aria-valuenow") || "");
    const actualNumber = Number(actual);
    const ok = Number.isFinite(actualNumber) && Math.abs(actualNumber - next) < 1e-9;
    return {
      ok,
      kind,
      expected: String(next),
      actual,
      method: "typed-direct",
      min: min ?? null,
      max: max ?? null,
      step: el.getAttribute("step") ?? null,
      error: ok ? null : "numeric value verification failed",
    };
  }

  const next = typedValueForInput(el, wanted);
  setNativeValue(el, String(next));
  dispatchValueEvents(el);
  const actual = String(el.value || "");
  const ok = valuesEquivalent(el, actual, String(next));
  return {
    ok,
    kind,
    expected: String(next),
    actual,
    method: "typed-direct",
    min: el.getAttribute("min"),
    max: el.getAttribute("max"),
    step: el.getAttribute("step"),
    error: ok ? null : "typed value verification failed",
  };
}
"##;

fn selected_frame_value(state: &DaemonState) -> Value {
    let selected = state.selected_frame.lock().unwrap().clone();
    match selected.as_deref() {
        Some(value) if value.starts_with("name:") => {
            json!({"kind": "name", "value": value.trim_start_matches("name:")})
        }
        Some(value) if value.starts_with("index:") => {
            let parsed = value
                .trim_start_matches("index:")
                .parse::<u64>()
                .unwrap_or(0);
            json!({"kind": "index", "value": parsed})
        }
        Some(value) if value.starts_with("url:") => {
            json!({"kind": "urlPattern", "value": value.trim_start_matches("url:")})
        }
        Some(_) => json!({"kind": "main"}),
        None => Value::Null,
    }
}

fn build_js(state: &DaemonState, include_frames: bool, body: &str) -> String {
    let frame_spec = selected_frame_value(state);
    build_js_with_frame_spec(&frame_spec, include_frames, body)
}

fn build_js_with_frame_spec(frame_spec: &Value, include_frames: bool, body: &str) -> String {
    format!(
        r#"(() => {{
  const frameSpec = {frame_spec};
  const includeFrames = {include_frames};
  {helpers}
  {body}
}})()"#,
        frame_spec = serde_json::to_string(&frame_spec).unwrap(),
        include_frames = if include_frames { "true" } else { "false" },
        helpers = INSPECTION_HELPERS_JS,
        body = body,
    )
}

async fn eval_json_value(page: &Page, js: &str, label: &str) -> Result<Value, String> {
    let result = timeout(INSPECTION_TIMEOUT, page.evaluate_expression(js))
        .await
        .map_err(|_| format!("{label} timed out after 30s"))?
        .map_err(|err| format!("{label} error: {}", clean_cdp_error(&err)))?;
    let value = result.value().cloned().unwrap_or(Value::Null);
    let json_str = value
        .as_str()
        .ok_or_else(|| format!("{label} returned non-string result"))?;
    serde_json::from_str(json_str).map_err(|err| format!("{label} parse error: {err}"))
}

pub async fn eval_expression(
    page: &Page,
    state: &DaemonState,
    expression: &str,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [] }});
  }}
  try {{
    const value = resolved.context.win.eval({expression});
    // Preserve JSON-serializable objects while replacing values JSON cannot encode.
    let safeValue;
    try {{
      if (value === undefined) {{
        safeValue = null;
      }} else if (value === null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {{
        safeValue = value;
      }} else if (typeof value === 'bigint' || typeof value === 'symbol' || typeof value === 'function') {{
        safeValue = String(value);
      }} else {{
        const seen = new WeakSet();
        const encoded = JSON.stringify(value, (_key, inner) => {{
          if (inner === undefined) return null;
          if (typeof inner === 'bigint' || typeof inner === 'symbol' || typeof inner === 'function') return String(inner);
          if (inner && typeof inner === 'object') {{
            if (inner === resolved.context.win || inner === inner.window) return '[Window]';
            if (seen.has(inner)) return '[Circular]';
            seen.add(inner);
          }}
          return inner;
        }});
        safeValue = encoded === undefined ? String(value) : JSON.parse(encoded);
      }}
    }} catch (_) {{
      safeValue = '[Unserializable]';
    }}
    return JSON.stringify({{ ok: true, value: safeValue }});
  }} catch (err) {{
    return JSON.stringify({{ ok: false, error: String(err), boundaries: resolved.boundaries || [] }});
  }}
"#,
        expression = serde_json::to_string(expression).unwrap()
    );
    eval_json_value(page, &build_js(state, false, &body), "eval").await
}

pub async fn target_url(page: &Page, state: &DaemonState) -> Result<Value, String> {
    let body = r#"
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {
    return JSON.stringify({ ok: false, error: resolved.error, boundaries: resolved.boundaries || [] });
  }
  return JSON.stringify({
    ok: true,
    url: resolved.context.url,
    frameLabel: resolved.context.label,
    boundaries: resolved.boundaries || [],
  });
"#;
    eval_json_value(page, &build_js(state, false, body), "target_url").await
}

pub async fn selector_query(
    page: &Page,
    state: &DaemonState,
    selector: &str,
    include_frames: bool,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, includeFrames || !!(frameSpec && frameSpec.kind));
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}
  const matches = [];
  for (const context of resolved.contexts) {{
    const queried = queryAllDeep(context.doc, {selector});
    if (!queried.ok) {{
      return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
    }}
    for (const el of queried.elements) {{
      matches.push({{ summary: elementSummary(el, context) }});
    }}
  }}
  return JSON.stringify({{
    ok: true,
    count: matches.length,
    first: matches.length > 0 ? matches[0].summary : null,
    matches: matches.map((m) => m.summary),
    boundaries: resolved.boundaries || [],
  }});
"#,
        selector = serde_json::to_string(selector).unwrap()
    );
    eval_json_value(
        page,
        &build_js(state, include_frames, &body),
        "selector_query",
    )
    .await
}

pub async fn resolve_selector_target(
    page: &Page,
    state: &DaemonState,
    selector: &str,
    prefer_visible: bool,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, true);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}

  const matches = [];
  for (const context of resolved.contexts) {{
    const queried = queryAllDeep(context.doc, {selector});
    if (!queried.ok) {{
      return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
    }}
    for (const el of queried.elements) {{
      matches.push({{ element: el, context, summary: elementSummary(el, context) }});
    }}
  }}

  if (matches.length === 0) {{
    return JSON.stringify({{ ok: false, error: "element not found: " + {selector}, boundaries: resolved.boundaries || [] }});
  }}

  let target = null;
  if ({prefer_visible}) {{
    target = matches.find((match) => match.summary.visible) || null;
  }}
  if (!target) target = matches[0];

  const actionability = prepareActionTarget(target.context, target.element);
  const center = actionability.absolute || absoluteCenter(target.context, target.element);
  return JSON.stringify({{
    ok: true,
    count: matches.length,
    target: target.summary,
    actionability,
    center,
    boundaries: resolved.boundaries || [],
  }});
"#,
        selector = serde_json::to_string(selector).unwrap(),
        prefer_visible = if prefer_visible { "true" } else { "false" },
    );
    eval_json_value(
        page,
        &build_js(state, true, &body),
        "resolve_selector_target",
    )
    .await
}

pub async fn perform_selector_action(
    page: &Page,
    state: &DaemonState,
    selector: &str,
    action: &str,
    options: &Value,
    prefer_visible: bool,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, true);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}

  const matches = [];
  for (const context of resolved.contexts) {{
    const queried = queryAllDeep(context.doc, {selector});
    if (!queried.ok) {{
      return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
    }}
    for (const el of queried.elements) {{
      matches.push({{ element: el, context, summary: elementSummary(el, context) }});
    }}
  }}

  if (matches.length === 0) {{
    return JSON.stringify({{ ok: false, error: "element not found: " + {selector}, boundaries: resolved.boundaries || [] }});
  }}

  let target = null;
  if ({prefer_visible}) {{
    target = matches.find((match) => match.summary.visible) || null;
  }}
  if (!target) target = matches[0];

  const el = target.element;
  const context = target.context;
  const options = {options};
  const actionability = prepareActionTarget(context, el);

  try {{
    switch ({action}) {{
      case "focus":
        if (typeof el.focus === "function") el.focus();
        break;
      case "click":
        if (typeof el.focus === "function") el.focus();
        if (typeof el.click === "function") {{
          el.click();
        }} else {{
          el.dispatchEvent(new MouseEvent("click", {{ bubbles: true, cancelable: true, view: context.win }}));
        }}
        break;
      case "hover": {{
        const hoverEvents = ["pointerover", "mouseover", "mouseenter", "pointermove", "mousemove"];
        for (const eventName of hoverEvents) {{
          el.dispatchEvent(new MouseEvent(eventName, {{ bubbles: true, cancelable: true, view: context.win }}));
        }}
        break;
      }}
      case "type": {{
        const typedValue = setTypedControlValue(el, String(options.text || ""));
        if (typedValue) {{
          target.valueResult = typedValue;
          if (!typedValue.ok) throw new Error(typedValue.error || "typed value failed");
        }} else {{
          const fillResult = robustFillElement(context, el, options);
          target.fillResult = fillResult;
          if (!fillResult.ok) throw new Error(fillResult.error || "fill failed");
        }}
        break;
      }}
      case "select_option": {{
        const selection = selectOptionElement(context, el, options);
        target.selection = selection;
        if (!selection.ok) throw new Error(selection.error || "select_option failed");
        break;
      }}
      case "set_checked": {{
        const desired = !!options.checked;
        if ("checked" in el) {{
          el.checked = desired;
          el.dispatchEvent(new Event("change", {{ bubbles: true }}));
          el.dispatchEvent(new Event("input", {{ bubbles: true }}));
        }} else {{
          const role = String(el.getAttribute("role") || "").toLowerCase();
          if (!["checkbox", "radio", "switch", "menuitemcheckbox", "menuitemradio"].includes(role)) {{
            throw new Error("element does not support checked state");
          }}
          const before = String(el.getAttribute("aria-checked") || "false").toLowerCase() === "true";
          if (before !== desired && typeof el.click === "function") el.click();
          el.setAttribute("aria-checked", String(desired));
          el.dispatchEvent(new Event("change", {{ bubbles: true }}));
          el.dispatchEvent(new Event("input", {{ bubbles: true }}));
        }}
        break;
      }}
      default:
        throw new Error("unsupported selector action: " + {action});
    }}
  }} catch (err) {{
    return JSON.stringify({{
      ok: false,
      error: String(err),
      count: matches.length,
      target: elementSummary(el, context),
      actionability,
      fill: target.fillResult || null,
      selection: target.selection || null,
      valueResult: target.valueResult || null,
      boundaries: resolved.boundaries || [],
    }});
  }}

  return JSON.stringify({{
    ok: true,
    count: matches.length,
    target: elementSummary(el, context),
    actionability,
    fill: target.fillResult || null,
    selection: target.selection || null,
    valueResult: target.valueResult || null,
    center: actionability.absolute || absoluteCenter(context, el),
    boundaries: resolved.boundaries || [],
  }});
"#,
        selector = serde_json::to_string(selector).unwrap(),
        action = serde_json::to_string(action).unwrap(),
        options = serde_json::to_string(options).unwrap(),
        prefer_visible = if prefer_visible { "true" } else { "false" },
    );
    eval_json_value(
        page,
        &build_js(state, true, &body),
        "perform_selector_action",
    )
    .await
}

pub async fn text_query(
    page: &Page,
    state: &DaemonState,
    text: &str,
    include_frames: bool,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, includeFrames || !!(frameSpec && frameSpec.kind));
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}
  const search = {text}.toLowerCase();
  const matches = [];
  for (const context of resolved.contexts) {{
    const docText = normalizeText((context.doc.body && (context.doc.body.innerText || context.doc.body.textContent)) || (context.doc.documentElement && (context.doc.documentElement.innerText || context.doc.documentElement.textContent)) || "");
    if (!search || docText.toLowerCase().includes(search)) {{
      matches.push({{
        frameLabel: context.label,
        frameUrl: context.url,
        snippet: docText.slice(0, 160),
      }});
    }}
  }}
  return JSON.stringify({{
    ok: true,
    found: matches.length > 0,
    matches,
    boundaries: resolved.boundaries || [],
  }});
"#,
        text = serde_json::to_string(text).unwrap()
    );
    eval_json_value(page, &build_js(state, include_frames, &body), "text_query").await
}

pub async fn page_source(
    page: &Page,
    state: &DaemonState,
    selector: Option<&str>,
) -> Result<Value, String> {
    let body = if let Some(selector) = selector {
        format!(
            r#"
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [] }});
  }}
  const queried = queryAllDeep(resolved.context.doc, {selector});
  if (!queried.ok) {{
    return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
  }}
  const html = queried.elements.length > 0 ? queried.elements[0].outerHTML : "";
  return JSON.stringify({{
    ok: true,
    html,
    length: html.length,
    frameLabel: resolved.context.label,
    frameUrl: resolved.context.url,
    boundaries: resolved.boundaries || [],
  }});
"#,
            selector = serde_json::to_string(selector).unwrap()
        )
    } else {
        r#"
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {
    return JSON.stringify({ ok: false, error: resolved.error, boundaries: resolved.boundaries || [] });
  }
  const html = resolved.context.doc.documentElement
    ? resolved.context.doc.documentElement.outerHTML
    : "";
  return JSON.stringify({
    ok: true,
    html,
    length: html.length,
    frameLabel: resolved.context.label,
    frameUrl: resolved.context.url,
    boundaries: resolved.boundaries || [],
  });
"#
        .to_string()
    };
    eval_json_value(page, &build_js(state, false, &body), "page_source").await
}

pub async fn find_elements(
    page: &Page,
    state: &DaemonState,
    role: &str,
    text: &str,
    selector: &str,
    limit: u32,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, true);
  if (!resolved.ok) {{
    return JSON.stringify({{ elements: [], count: 0, truncated: false, error: resolved.error, boundaries: resolved.boundaries || [] }});
  }}
  const role = {role}.trim();
  const searchText = {text}.trim().toLowerCase();
  const selector = {selector}.trim();
  const limit = {limit};
  const results = [];
  for (const context of resolved.contexts) {{
    const queried = queryAllDeep(context.doc, selector || null);
    if (!queried.ok) {{
      return JSON.stringify({{ elements: [], count: 0, truncated: false, error: queried.error, boundaries: resolved.boundaries || [] }});
    }}
    for (const el of queried.elements) {{
      if (results.length >= limit) break;
      const summary = elementSummary(el, context);
      if (role && summary.role !== role) continue;
      const haystack = normalizeText((el.innerText || el.textContent || "")).toLowerCase();
      if (searchText && !haystack.includes(searchText) && !summary.name.toLowerCase().includes(searchText)) continue;
      if (!selector && !summary.visible) continue;
      results.push(summary);
    }}
    if (results.length >= limit) break;
  }}
  return JSON.stringify({{
    elements: results,
    count: results.length,
    truncated: results.length >= limit,
    boundaries: resolved.boundaries || [],
  }});
"#,
        role = serde_json::to_string(role).unwrap(),
        text = serde_json::to_string(text).unwrap(),
        selector = serde_json::to_string(selector).unwrap(),
        limit = limit,
    );
    eval_json_value(page, &build_js(state, true, &body), "find").await
}

pub async fn snapshot_elements(
    page: &Page,
    state: &DaemonState,
    selector: Option<&str>,
    interactive_only: bool,
    limit: u32,
    mode: Option<&str>,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = collectContexts(frameSpec, true);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}

  const scopeSelector = {selector};
  const interactiveOnly = {interactive_only};
  const limit = {limit};
  const mode = {mode};
  const results = [];
  const seen = new Set();
  let scopeFound = !scopeSelector;

  function nearestHeading(el) {{
    let heading = "";
    let walker = el.previousElementSibling;
    while (walker && !heading) {{
      if (/^H[1-6]$/.test(walker.tagName)) {{
        heading = normalizeText(walker.textContent || "").slice(0, 60);
      }}
      walker = walker.previousElementSibling;
    }}
    if (heading) return heading;

    let parent = el.parentElement;
    while (parent && parent !== parent.ownerDocument.body && !heading) {{
      const found = parent.querySelector("h1,h2,h3,h4,h5,h6");
      if (found) heading = normalizeText(found.textContent || "").slice(0, 60);
      parent = parent.parentElement;
    }}
    return heading;
  }}

  function formOwnership(el) {{
    const form = el.form || el.closest("form");
    if (!form) return "";
    return form.id || form.getAttribute("name") || form.getAttribute("action") || "anonymous-form";
  }}

  function includeElement(el, visible) {{
    const tag = el.tagName.toLowerCase();
    const role = inferRole(el);
    const interactive = pi.isInteractiveEl ? pi.isInteractiveEl(el) : ["a","button","input","select","textarea","summary"].includes(tag);

    if (mode === "visible_only") return visible;
    if (mode === "interactive" || (!mode && interactiveOnly)) return interactive && visible;
    if (mode === "form") return ["input","select","textarea","button","label","fieldset","legend","output","datalist"].includes(tag);
    if (mode === "dialog") return tag === "dialog" || role === "dialog" || role === "alertdialog" || el.getAttribute("aria-modal") === "true";
    if (mode === "navigation") return role === "link" || role === "navigation" || tag === "nav" || tag === "a";
    if (mode === "errors") return role === "alert" || role === "status" || el.classList.contains("error") || el.classList.contains("alert") || el.getAttribute("aria-live") !== null;
    if (mode === "headings") return /^h[1-6]$/.test(tag) || role === "heading";
    return visible;
  }}

  for (const context of resolved.contexts) {{
    let scopes = [context.doc.body || context.doc.documentElement];
    if (scopeSelector) {{
      const scoped = queryAllDeep(context.doc, scopeSelector);
      if (!scoped.ok) {{
        return JSON.stringify({{ ok: false, error: scoped.error, boundaries: resolved.boundaries || [] }});
      }}
      if (scoped.elements.length === 0) {{
        continue;
      }}
      scopeFound = true;
      scopes = scoped.elements;
    }}

    for (const scope of scopes) {{
      const queried = queryAllDeep(scope, null);
      if (!queried.ok) {{
        return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
      }}

      for (const el of queried.elements) {{
        if (results.length >= limit) break;
        if (seen.has(el)) continue;
        seen.add(el);

        const visible = isVisible(el);
        if (!includeElement(el, visible)) continue;

        const tag = el.tagName.toLowerCase();
        const text = normalizeText(el.innerText || el.textContent || "").slice(0, 200);
        const bounds = absoluteBounds(context, el);
        results.push({{
          tag,
          role: inferRole(el),
          name: accessibleName(el),
          x: bounds.x,
          y: bounds.y,
          w: bounds.w,
          h: bounds.h,
          selectorHints: selectorHints(el),
          visible,
          enabled: isEnabled(el),
          deepPath: deepDomPath(el),
          contentHash: simpleHash(`${{tag}}:${{text}}`),
          structuralSignature: `${{tag}}:${{el.childElementCount}}:${{el.attributes.length}}`,
          nearestHeading: nearestHeading(el),
          formOwnership: formOwnership(el),
          frameIndex: context.index,
          frameName: context.win.name || "",
          frameLabel: context.label,
          frameUrl: context.url,
        }});
      }}

      if (results.length >= limit) break;
    }}

    if (results.length >= limit) break;
  }}

  if (!scopeFound) {{
    return JSON.stringify({{ ok: false, error: "scope selector not found: " + scopeSelector, boundaries: resolved.boundaries || [] }});
  }}

  return JSON.stringify({{
    ok: true,
    nodes: results,
    count: results.length,
    truncated: results.length >= limit,
    boundaries: resolved.boundaries || [],
  }});
"#,
        selector = serde_json::to_string(&selector).unwrap(),
        interactive_only = if interactive_only { "true" } else { "false" },
        limit = limit,
        mode = serde_json::to_string(&mode).unwrap(),
    );
    eval_json_value(page, &build_js(state, true, &body), "snapshot").await
}

fn snapshot_node_frame_spec(node: &Value) -> Value {
    if let Some(name) = node.get("frameName").and_then(|value| value.as_str()) {
        if !name.is_empty() {
            return json!({ "kind": "name", "value": name });
        }
    }
    if let Some(url) = node.get("frameUrl").and_then(|value| value.as_str()) {
        if !url.is_empty() {
            return json!({ "kind": "urlPattern", "value": url });
        }
    }
    if let Some(index) = node.get("frameIndex").and_then(|value| value.as_u64()) {
        return json!({ "kind": "index", "value": index });
    }
    json!({ "kind": "main" })
}

pub async fn resolve_snapshot_node(page: &Page, node: &Value) -> Result<Value, String> {
    let frame_spec = snapshot_node_frame_spec(node);
    let body = format!(
        r#"
  const node = {node};
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, reason: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}
  const context = resolved.context;

  function matchesFingerprint(el) {{
    const tag = el.tagName.toLowerCase();
    if (node.tag && tag !== node.tag) return false;
    const text = normalizeText(el.innerText || el.textContent || "").slice(0, 200);
    const hash = simpleHash(`${{tag}}:${{text}}`);
    if (hash === node.contentHash) return true;
    const signature = `${{tag}}:${{el.childElementCount}}:${{el.attributes.length}}`;
    return signature === node.structuralSignature && signature !== `${{tag}}:0:0`;
  }}

  function locate() {{
    if (Array.isArray(node.deepPath) && node.deepPath.length > 0) {{
      const byPath = resolveDeepPath(context.doc, node.deepPath);
      if (byPath && byPath.tagName && byPath.tagName.toLowerCase() === node.tag) {{
        return {{ element: byPath, tier: 1 }};
      }}
    }}

    if (Array.isArray(node.selectorHints)) {{
      for (const hint of node.selectorHints) {{
        const queried = queryAllDeep(context.doc, hint);
        if (queried.ok && queried.elements.length === 1) {{
          return {{ element: queried.elements[0], tier: 2 }};
        }}
      }}
    }}

    if (node.role && node.name) {{
      const queried = queryAllDeep(context.doc, null);
      if (queried.ok) {{
        for (const el of queried.elements) {{
          if (inferRole(el) === node.role && accessibleName(el) === node.name) {{
            return {{ element: el, tier: 3 }};
          }}
        }}
      }}
    }}

    if (node.tag) {{
      const queried = queryAllDeep(context.doc, node.tag);
      if (queried.ok) {{
        for (const el of queried.elements) {{
          if (matchesFingerprint(el)) {{
            return {{ element: el, tier: 4 }};
          }}
        }}
      }}
    }}

    return null;
  }}

  const match = locate();
  if (!match) {{
    return JSON.stringify({{ ok: false, reason: "stale", boundaries: resolved.boundaries || [], frameLabel: context.label, frameUrl: context.url }});
  }}

  return JSON.stringify({{
    ok: true,
    tier: match.tier,
    selector: selectorHint(match.element),
    summary: elementSummary(match.element, context),
    frameLabel: context.label,
    frameUrl: context.url,
    boundaries: resolved.boundaries || [],
  }});
"#,
        node = serde_json::to_string(node).unwrap(),
    );
    eval_json_value(
        page,
        &build_js_with_frame_spec(&frame_spec, false, &body),
        "resolve_snapshot_node",
    )
    .await
}

pub async fn act_on_snapshot_node(
    page: &Page,
    node: &Value,
    action: &str,
    options: &Value,
) -> Result<Value, String> {
    let frame_spec = snapshot_node_frame_spec(node);
    let body = format!(
        r#"
  const node = {node};
  const action = {action};
  const options = {options};
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, reason: resolved.error, boundaries: resolved.boundaries || [], crossOrigin: !!resolved.crossOrigin }});
  }}
  const context = resolved.context;

  function matchesFingerprint(el) {{
    const tag = el.tagName.toLowerCase();
    if (node.tag && tag !== node.tag) return false;
    const text = normalizeText(el.innerText || el.textContent || "").slice(0, 200);
    const hash = simpleHash(`${{tag}}:${{text}}`);
    if (hash === node.contentHash) return true;
    const signature = `${{tag}}:${{el.childElementCount}}:${{el.attributes.length}}`;
    return signature === node.structuralSignature && signature !== `${{tag}}:0:0`;
  }}

  function locate() {{
    if (Array.isArray(node.deepPath) && node.deepPath.length > 0) {{
      const byPath = resolveDeepPath(context.doc, node.deepPath);
      if (byPath && byPath.tagName && byPath.tagName.toLowerCase() === node.tag) {{
        return {{ element: byPath, tier: 1 }};
      }}
    }}

    if (Array.isArray(node.selectorHints)) {{
      for (const hint of node.selectorHints) {{
        const queried = queryAllDeep(context.doc, hint);
        if (queried.ok && queried.elements.length === 1) {{
          return {{ element: queried.elements[0], tier: 2 }};
        }}
      }}
    }}

    if (node.role && node.name) {{
      const queried = queryAllDeep(context.doc, null);
      if (queried.ok) {{
        for (const el of queried.elements) {{
          if (inferRole(el) === node.role && accessibleName(el) === node.name) {{
            return {{ element: el, tier: 3 }};
          }}
        }}
      }}
    }}

    if (node.tag) {{
      const queried = queryAllDeep(context.doc, node.tag);
      if (queried.ok) {{
        for (const el of queried.elements) {{
          if (matchesFingerprint(el)) {{
            return {{ element: el, tier: 4 }};
          }}
        }}
      }}
    }}

    return null;
  }}

  const match = locate();
  if (!match) {{
    return JSON.stringify({{ ok: false, reason: "stale", boundaries: resolved.boundaries || [], frameLabel: context.label, frameUrl: context.url }});
  }}

  const el = match.element;
  const actionability = prepareActionTarget(context, el);

  let fillResult = null;
  let valueResult = null;
  try {{
    if (action === "click") {{
      if (el.focus) el.focus();
      if (typeof el.click === "function") {{
        el.click();
      }} else {{
        el.dispatchEvent(new MouseEvent("click", {{ bubbles: true, cancelable: true, view: context.win }}));
      }}
    }} else if (action === "dblclick") {{
      if (el.focus) el.focus();
      el.dispatchEvent(new MouseEvent("dblclick", {{ bubbles: true, cancelable: true, view: context.win }}));
    }} else if (action === "hover") {{
      const hoverEvents = ["pointerover", "mouseover", "mouseenter", "pointermove", "mousemove"];
      for (const eventName of hoverEvents) {{
        el.dispatchEvent(new MouseEvent(eventName, {{ bubbles: true, cancelable: true, view: context.win }}));
      }}
    }} else if (action === "fill") {{
      const typedValue = setTypedControlValue(el, String(options.text || ""));
      if (typedValue) {{
        valueResult = typedValue;
        if (!typedValue.ok) throw new Error(typedValue.error || "typed value failed");
      }} else {{
        fillResult = robustFillElement(context, el, options);
        if (!fillResult.ok) throw new Error(fillResult.error || "fill failed");
      }}
    }} else {{
      throw new Error("unsupported ref action: " + action);
    }}
  }} catch (err) {{
    return JSON.stringify({{
      ok: false,
      reason: String(err),
      tier: match.tier,
      selector: selectorHint(el),
      summary: elementSummary(el, context),
      actionability,
      fill: fillResult,
      valueResult,
      frameLabel: context.label,
      frameUrl: context.url,
      boundaries: resolved.boundaries || [],
    }});
  }}

  return JSON.stringify({{
    ok: true,
    tier: match.tier,
    selector: selectorHint(el),
    summary: elementSummary(el, context),
    actionability,
    fill: fillResult,
    valueResult,
    frameLabel: context.label,
    frameUrl: context.url,
    boundaries: resolved.boundaries || [],
  }});
"#,
        node = serde_json::to_string(node).unwrap(),
        action = serde_json::to_string(action).unwrap(),
        options = serde_json::to_string(options).unwrap(),
    );
    eval_json_value(
        page,
        &build_js_with_frame_spec(&frame_spec, false, &body),
        "act_on_snapshot_node",
    )
    .await
}

pub async fn region_signature(
    page: &Page,
    state: &DaemonState,
    selector: &str,
) -> Result<Value, String> {
    let body = format!(
        r#"
  const resolved = resolveFrameContext(frameSpec);
  if (!resolved.ok) {{
    return JSON.stringify({{ ok: false, error: resolved.error, boundaries: resolved.boundaries || [] }});
  }}
  const queried = queryAllDeep(resolved.context.doc, {selector});
  if (!queried.ok) {{
    return JSON.stringify({{ ok: false, error: queried.error, boundaries: resolved.boundaries || [] }});
  }}
  const html = queried.elements.length > 0 ? (queried.elements[0].innerHTML || "") : "";
  return JSON.stringify({{
    ok: true,
    html,
    boundaries: resolved.boundaries || [],
  }});
"#,
        selector = serde_json::to_string(selector).unwrap()
    );
    eval_json_value(page, &build_js(state, false, &body), "region_signature").await
}
