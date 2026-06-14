use chromiumoxide::Page;
use serde_json::{json, Value};
use tokio::time::timeout;

use super::model::json_literal;
use super::page_model::{
    accessible_text_helpers_js, availability_helpers_js, control_semantics_helpers_js,
    value_control_helpers_js,
};
use super::PLAN_TIMEOUT;
use crate::daemon::capture::capture_compact_page_state;

pub(super) async fn handle_scoped_item_workflow(
    page: &Page,
    params: &Value,
) -> Result<Value, String> {
    let item_query = params
        .get("itemQuery")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "scoped_item_workflow requires itemQuery".to_string())?;
    let action_hint = params
        .get("actionHint")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "scoped_item_workflow requires actionHint".to_string())?;
    let fill_text = params.get("fillText").and_then(|v| v.as_str());
    let completion_hint = params.get("completionHint").and_then(|v| v.as_str());
    let item_count_all = params
        .get("itemCountMode")
        .or_else(|| params.get("item_count_mode"))
        .and_then(|v| v.as_str())
        .map(|value| value.eq_ignore_ascii_case("all"))
        .unwrap_or(false)
        || params
            .get("allItems")
            .or_else(|| params.get("all_items"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let item_count = params
        .get("itemCount")
        .or_else(|| params.get("item_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 50);

    let item_json = json_literal(item_query);
    let action_json = json_literal(action_hint);
    let fill_json = json_literal(&fill_text);
    let completion_json = json_literal(&completion_hint);
    let item_count_json = json_literal(&item_count);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let text_matcher_js = super::planner_js::text_matcher_js();

    let js = format!(
        r#"(async () => {{
  const itemQuery = {item_json};
  const actionHint = {action_json};
  const fillText = {fill_json};
  const completionHint = {completion_json};
  const itemCount = {item_count_json};
  const itemCountAll = {item_count_all};
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function isReadOnlyControl(el) {{
    return !!el.readOnly || el.getAttribute('readonly') !== null || el.getAttribute('aria-readonly') === 'true';
  }}
  function classText(el) {{
    return String(el && el.className && typeof el.className === 'string' ? el.className : '');
  }}
	  {accessible_text_helpers_js}
	  {control_semantics_helpers_js}
	  {value_control_helpers_js}
	  function textOf(el) {{
    if (!el) return '';
    return [
      el.id || '',
      el.textContent || '',
      el.value || '',
      el.placeholder || '',
      el.getAttribute && el.getAttribute('aria-label') || '',
      el.getAttribute && el.getAttribute('title') || '',
      el.getAttribute && el.getAttribute('name') || '',
      el.getAttribute && el.getAttribute('data-testid') || '',
      el.getAttribute && semanticAttributeText(el) || '',
      el.getAttribute && el.getAttribute('aria-description') || '',
      referencedText(el, 'aria-labelledby'),
      referencedText(el, 'aria-describedby'),
      el.getAttribute && el.getAttribute('role') || '',
      associatedLabelText(el),
      structuralLabelText(el),
      slotText(el),
      classText(el)
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function iconText(el) {{
    const aliases = {{
      star: 'star important favorite favourite',
      important: 'important star priority',
      trash: 'trash delete remove',
      delete: 'delete trash remove',
      archive: 'archive',
      reply: 'reply respond',
      repost: 'repost',
      like: 'like favorite favourite heart',
      share: 'share',
      forward: 'forward',
      send: 'send submit'
    }};
    const nodes = [el].concat(Array.from(el && el.querySelectorAll ? el.querySelectorAll('[class*=icon], [class*=Icon], svg, use, path') : []));
    const parts = [];
    for (const node of nodes.slice(0, 20)) {{
      const raw = [
      classText(node),
      node.getAttribute && semanticAttributeText(node),
      node.id,
      node.getAttribute && node.getAttribute('href'),
      node.getAttribute && node.getAttribute('xlink:href'),
        node.getAttribute && node.getAttribute('data-icon'),
        node.getAttribute && node.getAttribute('aria-label'),
        node.getAttribute && node.getAttribute('title'),
        svgReferenceText(node)
      ].filter(Boolean).join(' ');
      parts.push(raw);
      for (const token of raw.split(/[^A-Za-z0-9]+/)) {{
        const compact = token.replace(/^(?:ui-icon-|fa-|fas-|far-|icon-|lucide-|mdi-)/i, '').toLowerCase();
        if (aliases[compact]) parts.push(aliases[compact]);
      }}
    }}
    return parts.join(' ');
  }}
  {text_matcher_js}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const testId = el.getAttribute && el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      if (document.querySelectorAll(byName).length === 1) return byName;
    }}
    const parts = [];
    let node = el;
    while (node && node.nodeType === Node.ELEMENT_NODE && node !== document.documentElement && parts.length < 5) {{
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
  function candidate(el) {{
    const rect = el.getBoundingClientRect();
    return {{
      selector: selector(el),
      tag: el.tagName.toLowerCase(),
      role: (el.getAttribute('role') || '').toLowerCase() || null,
      text: [textOf(el), iconText(el)].join(' ').slice(0, 180),
      bounds: {{ x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }}
    }};
  }}
  function isClickable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = (el.getAttribute('role') || '').toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'button' || tag === 'a' || type === 'button' || type === 'submit') return true;
    if (['button', 'link', 'menuitem', 'option', 'tab'].includes(role)) return true;
    if (isCustomCheckableElement(el)) return true;
    if (el.hasAttribute('onclick') || el.hasAttribute('tabindex')) return true;
    if (/\b(button|btn|link|clickable|icon|star|favorite|favourite|important|reply|respond|repost|like|heart|share|forward|delete|remove|trash|archive|send|submit|save|done|close)\b/i.test(directMetadata(el))) return true;
    try {{
      if (getComputedStyle(el).cursor === 'pointer') return true;
    }} catch (_) {{}}
    return false;
  }}
  function all(root, selectorText) {{
    const results = [];
    const seen = new Set();
    for (const scope of allRoots(root || document)) {{
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
    }}
    return results;
  }}
  function allRoots(start) {{
    const roots = [];
    const seenRoots = new Set();
    function addRoot(scope) {{
      if (!scope || seenRoots.has(scope)) return;
      seenRoots.add(scope);
      roots.push(scope);
      if (scope.shadowRoot) addRoot(scope.shadowRoot);
      if (scope.tagName && scope.tagName.toLowerCase() === 'iframe') {{
        try {{
          if (scope.contentDocument) addRoot(scope.contentDocument);
        }} catch (_) {{}}
      }}
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
    addRoot(start || document);
    return roots;
  }}
  function clickables(root) {{
    const query = 'button, a, input[type=button], input[type=submit], [role=button], [role=link], [onclick], [tabindex], svg, [class*=icon], [class*=Icon], span, div';
    const seen = new Set();
    const out = [];
    const candidates = all(root, query).concat(all(root, '*').filter(isCustomCheckableElement));
    for (const el of candidates) {{
      if (!visible(el) || !isClickable(el)) continue;
      const key = selector(el);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(el);
    }}
    if (root && root !== document && visible(root) && isClickable(root)) out.unshift(root);
    return out;
  }}
  function scoreAction(el, hint) {{
    const text = [textOf(el), iconText(el)].join(' ');
    let score = Math.max(tokenScore(hint, text), exactPhraseScore(hint, text));
    const normalizedHint = normalized(hint);
    const role = (el.getAttribute('role') || '').toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    const checkable = ['checkbox', 'radio'].includes(type) ||
      ['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio'].includes(role) ||
      el.hasAttribute('aria-pressed') ||
      isCustomCheckableElement(el);
    if (checkable && /\b(?:enable|disable|turn on|turn off|toggle|check|uncheck|select|deselect)\b/.test(normalizedHint)) score += 0.35;
    if (/star|important|favorite|favourite/.test(normalizedHint) && /\b(star|important|favorite|favourite|priority)\b/i.test(text)) score += 0.7;
    if (/delete|remove|trash/.test(normalizedHint) && /\b(delete|remove|trash)\b/i.test(text)) score += 0.7;
    if (/reply|respond/.test(normalizedHint) && /\b(reply|respond)\b/i.test(text)) score += 0.7;
    if (/repost/.test(normalizedHint) && /\brepost\b/i.test(text)) score += 0.7;
    if (/like|favorite|favourite|heart/.test(normalizedHint) && /\b(like|favorite|favourite|heart)\b/i.test(text)) score += 0.7;
    if (/share/.test(normalizedHint) && /\bshare\b/i.test(text)) score += 0.7;
    if (/forward/.test(normalizedHint) && /\bforward\b/i.test(text)) score += 0.7;
    if (/send|submit|save|done/.test(normalizedHint) && /\b(send|submit|save|done|ok)\b/i.test(text)) score += 0.7;
    return score;
  }}
  function directMetadata(el) {{
    if (!el) return '';
    return [
      el.id || '',
      classText(el),
      el.value || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('name') || '',
      el.getAttribute('data-testid') || '',
      el.getAttribute('role') || ''
    ].join(' ');
  }}
  function directMetadataScore(el, hint) {{
    const metadata = directMetadata(el);
    return Math.max(tokenScore(hint, metadata), exactPhraseScore(hint, metadata));
  }}
  function actionMatchScore(el, hint) {{
    let score = scoreAction(el, hint);
    const direct = directMetadataScore(el, hint);
    if (direct >= 0.7) score += 0.25;
    const tag = el.tagName.toLowerCase();
    const role = (el.getAttribute('role') || '').toLowerCase();
    if (['button', 'a', 'input'].includes(tag) || ['button', 'link', 'menuitem', 'option', 'tab'].includes(role)) score += 0.15;
    const rect = el.getBoundingClientRect();
    const area = Math.max(1, rect.width * rect.height);
    if (area <= 2500) score += 0.12;
    if (tag === 'div' && area >= 4000) score -= 0.15;
    return score;
  }}
  function ranked(elements, score) {{
    return elements
      .filter(visible)
      .map(el => ({{ el, score: score(el) }}))
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
  }}
  function actionCandidates(root, hint) {{
    const seen = new Set();
    const out = [];
    const add = el => {{
      const key = selector(el);
      if (seen.has(key)) return;
      seen.add(key);
      out.push(el);
    }};
    for (const el of clickables(root)) add(el);
    const metadataQuery = 'button, a, input[type=button], input[type=submit], [role=button], [role=link], [onclick], [tabindex], [id], [aria-label], [title], [data-testid], [class], span, div, svg';
    const metadataCandidates = all(root, metadataQuery).concat(all(root, '*').filter(isCustomCheckableElement));
    for (const el of metadataCandidates) {{
      if (!visible(el)) continue;
      if (isClickable(el) || scoreAction(el, hint) >= 0.8) add(el);
    }}
    return out;
  }}
  function bestActionDescendant(el, hint, parentScore) {{
    if (!el || !el.querySelectorAll) return null;
    const parentRect = el.getBoundingClientRect();
    const parentArea = Math.max(1, parentRect.width * parentRect.height);
    const query = 'button, a, input[type=button], input[type=submit], [role=button], [role=link], [onclick], [tabindex], [id], [aria-label], [title], [data-testid], [class], span, div, svg';
    const descendants = all(el, query)
      .concat(all(el, '*').filter(isCustomCheckableElement))
      .filter(child => child !== el && visible(child));
    const matches = ranked(descendants, child => {{
      let score = actionMatchScore(child, hint);
      const direct = directMetadataScore(child, hint);
      if (direct >= 0.7) score += 0.35;
      const rect = child.getBoundingClientRect();
      const area = Math.max(1, rect.width * rect.height);
      if (area < parentArea * 0.7) score += 0.1;
      return score;
    }});
    return matches.find(item => item.score >= Math.max(0.55, parentScore - 0.2)) || null;
  }}
  function chooseActionMatch(root, hint, minScore = 0.55) {{
    const matches = ranked(actionCandidates(root, hint), el => actionMatchScore(el, hint));
    const refined = matches.filter(item => {{
      return !matches.some(other => {{
        return other.el !== item.el && item.el.contains(other.el) && other.score >= item.score - 0.05;
      }});
    }});
    if (!refined.length || refined[0].score < minScore) return null;
    return bestActionDescendant(refined[0].el, hint, refined[0].score) || refined[0];
  }}
  function findItem() {{
    const containers = all(document, 'tr, [role=listitem], li, article, section, .card, .row, .item, [data-testid], div')
      .filter(el => visible(el) && el !== el.ownerDocument.body && el !== el.ownerDocument.documentElement);
    const scored = [];
    for (const el of containers) {{
      const text = textOf(el);
      let score = Math.max(exactPhraseScore(itemQuery, text), tokenScore(itemQuery, text));
      if (score <= 0) continue;
      const action = chooseActionMatch(el, actionHint, 0.45);
      const rect = el.getBoundingClientRect();
      const area = Math.max(1, rect.width * rect.height);
      if (['tr', 'li', 'article', 'section'].includes(el.tagName.toLowerCase())) score += 0.18;
      if ((el.getAttribute('role') || '').toLowerCase() === 'listitem') score += 0.18;
      if (clickables(el).length) score += 0.18;
      if (action) score += 0.45;
      if (!fillText && !action) score -= 0.35;
      if (area > 180000) score -= 0.35;
      if (area < 200) score -= 0.2;
      scored.push({{ el, score, area, actionScore: action ? action.score : 0 }});
    }}
    scored.sort((a, b) => b.score - a.score || b.actionScore - a.actionScore || a.area - b.area);
    return scored[0] || null;
  }}
  function findItems() {{
    const containers = all(document, 'tr, [role=listitem], li, article, section, .card, .row, .item, [data-testid], div')
      .filter(el => visible(el) && el !== el.ownerDocument.body && el !== el.ownerDocument.documentElement);
    const scored = [];
    for (const el of containers) {{
      const text = textOf(el);
      let score = Math.max(exactPhraseScore(itemQuery, text), tokenScore(itemQuery, text));
      if (score <= 0) continue;
      const action = chooseActionMatch(el, actionHint, 0.45);
      if (!action) continue;
      const rect = el.getBoundingClientRect();
      const area = Math.max(1, rect.width * rect.height);
      if (['tr', 'li', 'article', 'section'].includes(el.tagName.toLowerCase())) score += 0.18;
      if ((el.getAttribute('role') || '').toLowerCase() === 'listitem') score += 0.18;
      if (clickables(el).length) score += 0.18;
      if (/\b(media|post|message|card|row|item|record|entry|result)\b/i.test(classText(el))) score += 0.18;
      if (area > 180000) score -= 0.35;
      if (area < 200) score -= 0.2;
      scored.push({{ el, score, area, actionScore: action.score }});
    }}
    const compact = scored.filter(item => {{
      return !scored.some(other => {{
        if (other.el === item.el) return false;
        if (!item.el.contains(other.el)) return false;
        if (other.actionScore < item.actionScore - 0.15) return false;
        return other.area < item.area * 0.8;
      }});
    }});
    compact.sort((a, b) => b.score - a.score || a.area - b.area);
    return compact;
  }}
  function clickElement(el) {{
    el.scrollIntoView({{ block: 'center', inline: 'center' }});
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
    try {{
      el.click();
    }} catch (_) {{
      try {{ el.dispatchEvent(new MouseEvent('click', init)); }} catch (_) {{}}
    }}
  }}
  function chooseOpenTarget(item) {{
    function looksLikeEmbeddedAction(el) {{
      if (!el || el === item.el) return false;
      const tag = el.tagName.toLowerCase();
      const role = (el.getAttribute('role') || '').toLowerCase();
      const metadata = [directMetadata(el), textOf(el), iconText(el)].join(' ');
      if (['button', 'input', 'select', 'textarea'].includes(tag)) return true;
      if (['button', 'menuitem', 'option', 'tab', 'checkbox', 'radio', 'switch'].includes(role)) return true;
      return /\b(star|favorite|favourite|important|reply|respond|forward|delete|remove|trash|archive|like|share|send|submit|save|done|close)\b/i.test(metadata);
    }}
    const direct = clickables(item.el).filter(el => scoreAction(el, actionHint) < 0.5 && !looksLikeEmbeddedAction(el));
    const byQuery = ranked(direct, el => Math.max(exactPhraseScore(itemQuery, textOf(el)), tokenScore(itemQuery, textOf(el))));
    if (byQuery.length) return byQuery[0].el;
    if (isClickable(item.el)) return item.el;
    const links = direct.filter(el => ['a'].includes(el.tagName.toLowerCase()) || (el.getAttribute('role') || '').toLowerCase() === 'link');
    return links[0] || (isClickable(item.el) ? item.el : null);
  }}
  function findAction(scope) {{
    const scopes = [scope, document].filter(Boolean);
    for (const current of scopes) {{
      const match = chooseActionMatch(current, actionHint, 0.55);
      if (match) return match;
    }}
    return null;
  }}
	  function isFillable(el) {{
	    return isWritableValueControl(el);
	  }}
	  function writableField(el) {{
	    return visible(el) && isFillable(el);
	  }}
	  function fillField(text) {{
	    const fields = all(document, valueControlSelector())
	      .concat(all(document, '*').filter(isCustomWritableValueElement))
	      .filter(writableField);
    const matches = ranked(fields, el => {{
      let score = 0.3;
      const label = textOf(el);
      if (/\bforward\b/i.test(actionHint) && /\b(to|recipient|forward|email|e-mail|address)\b/i.test(label)) score += 0.75;
      if (!/\bforward\b/i.test(actionHint) && /\b(reply|message|comment|body|text|compose)\b/i.test(label)) score += 0.55;
      if (/\b(reply|respond)\b/i.test(actionHint) && /\b(reply|message|comment|body|text|compose)\b/i.test(label)) score += 0.25;
      if (el === document.activeElement) score += 0.25;
      return score;
    }});
	    const field = (matches[0] || {{ el: fields[0] }}).el;
	    if (!field) return null;
	    setControlValue(field, text, {{ inputType: 'insertText' }});
	    return field;
	  }}

  const steps = [];
  if (!fillText && (itemCountAll || itemCount > 1)) {{
    const items = findItems();
    if (!items.length) return {{ ok: false, error: 'scoped_item_workflow could not find items containing: ' + itemQuery }};
    const selected = itemCountAll ? items.slice(0, 50) : items.slice(0, itemCount);
    const requestedCount = itemCountAll ? selected.length : itemCount;
    for (const item of selected) {{
      const action = findAction(item.el);
      if (!action) continue;
      clickElement(action.el);
      await sleep(160);
      steps.push({{ action: 'click_action_in_item', item: candidate(item.el), target: candidate(action.el), score: action.score }});
    }}
    if (steps.length < requestedCount) {{
      return {{ ok: false, error: 'scoped_item_workflow could not click enough matching item actions', requested: requestedCount, clicked: steps.length, itemQuery, actionHint, steps }};
    }}
    if (completionHint) {{
      const complete = chooseActionMatch(document, completionHint, 0.55);
      if (complete) {{
        clickElement(complete.el);
        await sleep(220);
        steps.push({{ action: 'complete', target: candidate(complete.el), score: complete.score }});
      }}
    }}
    return {{ ok: true, mode: 'scoped-item-workflow', itemCount: requestedCount, itemCountMode: itemCountAll ? 'all' : 'exact', itemQuery, actionHint, steps }};
  }}

  const item = findItem();
  if (!item) return {{ ok: false, error: 'scoped_item_workflow could not find item containing: ' + itemQuery }};

  const actionBeforeOpen = !fillText && findAction(item.el);
  if (actionBeforeOpen) {{
    clickElement(actionBeforeOpen.el);
    await sleep(180);
    steps.push({{ action: 'click_action_in_item', target: candidate(actionBeforeOpen.el), score: actionBeforeOpen.score }});
    if (completionHint) {{
      const complete = chooseActionMatch(document, completionHint, 0.55);
      if (complete) {{
        clickElement(complete.el);
        await sleep(220);
        steps.push({{ action: 'complete', target: candidate(complete.el), score: complete.score }});
      }}
    }}
    return {{ ok: true, mode: 'scoped-item-workflow', item: candidate(item.el), steps }};
  }}

  const opener = chooseOpenTarget(item);
  if (opener) {{
    clickElement(opener);
    await sleep(220);
    steps.push({{ action: 'open_item', target: candidate(opener) }});
  }}

  const action = findAction(item.el) || findAction(document);
  if (!action) return {{ ok: false, error: 'scoped_item_workflow could not find action: ' + actionHint, item: candidate(item.el), steps }};
  clickElement(action.el);
  await sleep(220);
  steps.push({{ action: 'click_action', target: candidate(action.el), score: action.score }});

  if (fillText) {{
    const field = fillField(fillText);
    if (!field) return {{ ok: false, error: 'scoped_item_workflow could not find fill field after action', item: candidate(item.el), steps }};
    await sleep(80);
    steps.push({{ action: 'fill_text', target: candidate(field), text: fillText }});
    const submitHint = completionHint || (/reply|respond|forward/i.test(actionHint) ? 'send' : 'submit');
    const submit = chooseActionMatch(document, submitHint, 0.55);
    if (submit) {{
      clickElement(submit.el);
      await sleep(220);
      steps.push({{ action: 'complete', target: candidate(submit.el), score: submit.score }});
    }}
  }}

  return {{ ok: true, mode: 'scoped-item-workflow', item: candidate(item.el), steps }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "scoped_item_workflow timed out".to_string())?
        .map_err(|e| {
            format!(
                "scoped_item_workflow failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "scopedWorkflow": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("scoped_item_workflow failed")
            .to_string())
    }
}

pub(super) async fn handle_form_workflow(page: &Page, params: &Value) -> Result<Value, String> {
    let params_json = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(async () => {{
  const params = {params_json};
  const fields = Array.isArray(params.fields) ? params.fields : [];
  const completionHint = String(params.completionHint || 'submit');
  const resultPreference = params.resultPreference || null;
  const resultClickHint = params.resultClickHint || null;
  const resultOrdinal = Number.isInteger(params.resultOrdinal) ? params.resultOrdinal : null;
  const requestedDate = params.date || null;
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return (rect.width > 0 || rect.height > 0) &&
      style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity || 1) !== 0;
  }}
  function isReadOnlyControl(el) {{
    return !!el.readOnly || el.getAttribute('readonly') !== null || el.getAttribute('aria-readonly') === 'true';
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
      try {{
        if (root.matches && root.matches(query) && !seen.has(root)) {{
          seen.add(root);
          out.push(root);
        }}
        if (!root.querySelectorAll) continue;
        for (const el of Array.from(root.querySelectorAll(query))) {{
          if (seen.has(el)) continue;
          seen.add(el);
          out.push(el);
        }}
      }} catch (_) {{}}
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
    const name = el.getAttribute && el.getAttribute('name');
    if (name) {{
      const sel = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      if (all(sel).length === 1) return sel;
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
  function normalize(text) {{
    return String(text || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  }}
  function tokens(text) {{
    return normalize(text).split(/\s+/).filter(Boolean);
  }}
	  function tokenScore(hint, text) {{
    const wanted = tokens(hint);
    if (!wanted.length) return 0;
    const have = new Set(tokens(text));
    let hits = 0;
    for (const token of wanted) if (have.has(token)) hits++;
    return hits / wanted.length;
	  }}
	  {accessible_text_helpers_js}
	  {control_semantics_helpers_js}
	  {value_control_helpers_js}
	  function textOf(el) {{
    return [
      el.textContent || '',
      el.value || '',
      el.getAttribute && (el.getAttribute('name') || ''),
      el.getAttribute && (el.getAttribute('placeholder') || ''),
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('aria-description') || ''),
      el.getAttribute && (el.getAttribute('autocomplete') || ''),
      el.getAttribute && (el.getAttribute('data-testid') || ''),
      el.getAttribute && (el.id || el.getAttribute('class') || ''),
      el.getAttribute && (el.getAttribute('role') || ''),
      el.getAttribute && semanticAttributeText(el),
      referencedText(el, 'aria-labelledby'),
      referencedText(el, 'aria-describedby'),
      structuralLabelText(el),
      slotText(el),
      svgReferenceText(el),
      nearbyText(el),
      associatedLabelText(el)
    ].filter(Boolean).join(' ').replace(/\s+/g, ' ').trim();
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
  function nearbyText(el) {{
    const parts = [];
    let node = el;
    for (let depth = 0; node && depth < 4; depth += 1, node = node.parentElement) {{
      if (node.previousElementSibling) parts.push(directText(node.previousElementSibling), node.previousElementSibling.textContent || '');
      if (node.nextElementSibling) parts.push(directText(node.nextElementSibling));
      parts.push(node.getAttribute && (node.getAttribute('aria-label') || node.getAttribute('title') || node.getAttribute('data-testid') || node.id || classText(node)));
    }}
    return parts.filter(Boolean).join(' ').replace(/\s+/g, ' ').trim();
  }}
  function localFieldLabelText(el) {{
    const parts = [];
    let node = el ? el.parentElement : null;
    for (let depth = 0; node && depth < 4; depth += 1, node = node.parentElement) {{
      for (const child of Array.from(node.children || [])) {{
        if (child === el || child.contains(el)) continue;
        if (child.querySelector && child.querySelector('input, textarea, select, [contenteditable], [role=textbox], [role=searchbox], [role=spinbutton], [role=slider], [role=combobox]')) continue;
        const text = [directText(child), child.textContent || '', child.getAttribute && (child.getAttribute('aria-label') || child.getAttribute('title') || child.id || classText(child))].filter(Boolean).join(' ');
        if (text) parts.push(text);
      }}
      parts.push(node.getAttribute && (node.getAttribute('aria-label') || node.getAttribute('title') || node.getAttribute('data-testid') || node.id || classText(node)));
    }}
    return parts.filter(Boolean).join(' ').replace(/\s+/g, ' ').trim();
  }}
  function isDateRequest(request) {{
    const hints = Array.isArray(request && request.hints) ? request.hints.join(' ') : '';
    return /\b(date|day|when|calendar)\b/i.test([request && request.label || '', hints].join(' '));
  }}
  function isDateLikeControl(el) {{
    const type = (el.getAttribute('type') || '').toLowerCase();
    const role = (el.getAttribute('role') || '').toLowerCase();
    const metadata = [textOf(el), nearbyText(el), localFieldLabelText(el), el.id || '', classText(el), el.getAttribute('name') || '', el.getAttribute('placeholder') || '', role, type].join(' ');
    return type === 'date' || /\b(date|day|when|calendar|datepicker|date-picker|depart|arrival)\b/i.test(metadata);
  }}
	  function fieldCandidates(request = null) {{
	    const allowReadonlyDate = isDateRequest(request);
	    return all(valueControlSelector())
	      .concat(all('*').filter(isCustomWritableValueElement))
	      .filter(el => visible(el) && (isWritableValueControl(el) || (allowReadonlyDate && isDateLikeControl(el))));
	  }}
	  function isFieldLikeControl(el) {{
	    const tag = el.tagName.toLowerCase();
	    const type = (el.getAttribute('type') || '').toLowerCase();
	    const role = (el.getAttribute('role') || '').toLowerCase();
	    if (tag === 'textarea' || tag === 'select') return true;
	    if (tag === 'input' && !['button', 'submit', 'reset', 'image'].includes(type)) return true;
	    if (el.isContentEditable || el.getAttribute('contenteditable') != null) return true;
	    return ['textbox', 'searchbox', 'spinbutton', 'combobox', 'listbox', 'slider'].includes(role) ||
	      isCustomWritableValueElement(el);
  }}
  function scoreField(field, request) {{
    const text = [textOf(field), nearbyText(field), localFieldLabelText(field)].join(' ');
    const hints = Array.isArray(request.hints) && request.hints.length ? request.hints : [request.label || ''];
    let score = 0;
    for (const hint of hints) score = Math.max(score, tokenScore(hint, text), normalize(text).includes(normalize(hint)) ? 0.9 : 0);
    const type = (field.getAttribute('type') || '').toLowerCase();
    const autocomplete = (field.getAttribute('autocomplete') || '').toLowerCase();
    const name = (field.getAttribute('name') || '').toLowerCase();
    const haystack = normalize([text, name, autocomplete, type].join(' '));
    if (hints.some(hint => /\b(from|origin|source|departure)\b/i.test(hint)) && /\b(from|origin|source|departure|depart|pickup)\b/.test(haystack)) score += 0.35;
    if (hints.some(hint => /\b(to|destination|arrival|target)\b/i.test(hint)) && /\b(to|destination|arrival|arrive|dropoff|target)\b/.test(haystack)) score += 0.35;
    if (hints.some(hint => /\b(date|day|when)\b/i.test(hint)) && (type === 'date' || /\b(date|day|when|depart)\b/.test(haystack))) score += 0.35;
    if (hints.some(hint => /\b(title|name|event|subject)\b/i.test(hint)) && /\b(title|name|event|subject)\b/.test(haystack)) score += 0.35;
    if (hints.some(hint => /\b(duration|length)\b/i.test(hint)) && /\b(duration|length|minutes|mins|time)\b/.test(haystack)) score += 0.3;
    if (hints.some(hint => /\b(start|begin|from time)\b/i.test(hint)) && /\b(start|begin|from|time)\b/.test(haystack)) score += 0.3;
    if (hints.some(hint => /\b(end|until|to time)\b/i.test(hint)) && /\b(end|until|to|time)\b/.test(haystack)) score += 0.3;
    return score;
  }}
  function dispatchMouseClick(el) {{
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    el.dispatchEvent(new MouseEvent('mouseover', init));
    el.dispatchEvent(new MouseEvent('mousedown', init));
    el.dispatchEvent(new MouseEvent('mouseup', init));
    el.dispatchEvent(new MouseEvent('click', init));
  }}
  function chooseVisibleSuggestion(el, value) {{
    const wanted = normalize(value);
    if (!wanted) return null;
    const options = all('[role=option], .ui-menu-item-wrapper, .ui-autocomplete [id], .ui-autocomplete li, datalist option')
      .filter(visible)
      .filter(option => !(option.matches && option.matches('.ui-menu-item') && option.querySelector('.ui-menu-item-wrapper, [role=option]')))
      .map(option => {{
        const text = textOf(option) || directText(option) || option.getAttribute('value') || '';
        let score = Math.max(tokenScore(value, text), normalize(text).includes(wanted) ? 0.9 : 0, wanted.includes(normalize(text)) ? 0.45 : 0);
        if (option.closest && option.closest('.ui-autocomplete')) score += 0.08;
        if (option.matches && option.matches('.ui-menu-item-wrapper, [role=option]')) score += 0.08;
        return {{ option, text, score }};
      }})
      .filter(item => item.score >= 0.35)
      .sort((a, b) => b.score - a.score);
    if (!options.length) return null;
	    const chosen = options[0].option;
	    if (chosen.tagName && chosen.tagName.toLowerCase() === 'option') {{
	      const optionValue = chosen.getAttribute('value') || chosen.textContent || value;
	      setControlValue(el, optionValue, {{ normalize }});
	    }} else {{
	      dispatchMouseClick(chosen);
	    }}
    return {{ selector: selector(chosen), text: options[0].text, score: options[0].score }};
  }}
  function openSuggestions(el, value) {{
    try {{
      if (window.jQuery && window.jQuery.fn && window.jQuery.fn.autocomplete) {{
        const jq = window.jQuery(el);
        if (jq.data('ui-autocomplete') || jq.data('autocomplete') || jq.hasClass('ui-autocomplete-input')) {{
          jq.autocomplete('search', String(value || el.value || ''));
          return true;
        }}
      }}
    }} catch (_) {{}}
    return false;
  }}
		  function setNativeValue(el, value) {{
		    const tag = el.tagName.toLowerCase();
		    const ok = setControlValue(el, value, {{ normalize }});
		    if (!ok) return false;
		    const role = (el.getAttribute('role') || '').toLowerCase();
		    const hasSuggestions = tag === 'input' ||
		      role === 'combobox' ||
		      el.getAttribute('aria-autocomplete') ||
		      el.getAttribute('aria-controls') ||
		      isCustomWritableValueElement(el);
		    if (hasSuggestions) openSuggestions(el, value);
		    if (hasSuggestions) chooseVisibleSuggestion(el, value);
		    return true;
	  }}
  function normalizeDate(value) {{
    const text = String(value || '').trim();
    let match = text.match(/^(\d{{1,2}})\/(\d{{1,2}})\/(\d{{4}})$/);
    if (match) return match[3] + '-' + match[1].padStart(2, '0') + '-' + match[2].padStart(2, '0');
    match = text.match(/^(\d{{4}})-(\d{{1,2}})-(\d{{1,2}})$/);
    if (match) return match[1] + '-' + match[2].padStart(2, '0') + '-' + match[3].padStart(2, '0');
    return null;
  }}
  function dateCellScore(el, iso) {{
    const text = [el.getAttribute('data-date'), el.getAttribute('datetime'), el.getAttribute('aria-label'), el.getAttribute('title'), directText(el), textOf(el)].filter(Boolean).join(' ');
    if (text.includes(iso)) return 1;
    const day = String(Number(iso.slice(8, 10)));
    if (/^\s*\d{{1,2}}\s*$/.test(directText(el)) && directText(el) === day) return 0.45;
    return 0;
  }}
  function clickTarget(el) {{
    dispatchMouseClick(el);
  }}
  function clickResultTarget(el) {{
    const tag = el && el.tagName ? el.tagName.toLowerCase() : '';
    const href = tag === 'a' ? String(el.getAttribute('href') || '').trim() : '';
    let preventDefault = null;
    if (tag === 'a' && (!href || href === '#' || /^javascript:/i.test(href))) {{
      preventDefault = event => event.preventDefault();
      el.addEventListener('click', preventDefault, {{ capture: true, once: true }});
    }}
    try {{
      dispatchMouseClick(el);
    }} finally {{
      if (preventDefault) {{
        try {{ el.removeEventListener('click', preventDefault, {{ capture: true }}); }} catch (_) {{}}
      }}
    }}
  }}
  function submitTarget(el) {{
    clickTarget(el);
    const form = el && (el.form || (el.closest && el.closest('form')));
    if (!form) return;
    try {{
      if (typeof SubmitEvent === 'function') {{
        form.dispatchEvent(new SubmitEvent('submit', {{ bubbles: true, cancelable: true, submitter: el }}));
      }} else {{
        form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }}));
      }}
    }} catch (_) {{
      try {{ form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }})); }} catch (_) {{}}
    }}
  }}
  function fillRequest(request, used) {{
    const candidates = fieldCandidates(request)
      .filter(el => !used.has(selector(el)))
      .map(el => ({{ el, score: scoreField(el, request) }}))
      .filter(item => item.score >= 0.3)
      .sort((a, b) => b.score - a.score);
    if (!candidates.length) return null;
    const chosen = candidates[0].el;
    used.add(selector(chosen));
    const current = String(chosen.value || '');
    const wanted = String(request.value || '');
    if (current && wanted && normalize(current).includes(normalize(wanted)) && tokens(current).length > tokens(wanted).length) {{
      return {{ ok: true, selector: selector(chosen), label: request.label || null, value: current, preserved: true, score: candidates[0].score }};
    }}
    const ok = setNativeValue(chosen, String(request.value || ''));
    return {{ ok, selector: selector(chosen), label: request.label || null, value: request.value || '', score: candidates[0].score }};
  }}
  function selectDate(iso, used) {{
    if (!iso) return null;
    const dateField = fillRequest({{ label: 'date', hints: ['date', 'day', 'when', 'departure date', 'event date'], value: iso }}, used);
    if (dateField) return {{ mode: 'field', ...dateField }};
    const cells = all('[data-date], [datetime], [role=gridcell], button, td, a, div, span')
      .filter(visible)
      .map(el => ({{ el, score: dateCellScore(el, iso) }}))
      .filter(item => item.score >= 0.45)
      .sort((a, b) => b.score - a.score);
    if (!cells.length) return null;
    clickTarget(cells[0].el);
    return {{ mode: 'cell', selector: selector(cells[0].el), value: iso, score: cells[0].score }};
  }}
  function submitControl() {{
    const controls = all('button, input[type=submit], input[type=button], a, [role=button], [onclick], [tabindex], [class*=submit], [class*=Submit], [class*=final], [class*=Final], div, span')
      .filter(visible)
      .filter(el => !isFieldLikeControl(el))
      .filter(el => !(el.closest && el.closest('.ui-datepicker, .datepicker, [class*=datepicker], [class*=Datepicker], [role=dialog]')))
      .map(el => {{
        const text = textOf(el);
        const metadata = [text, classText(el), el.id || '', el.getAttribute('role') || '', el.getAttribute('type') || ''].join(' ');
        const controlMetadata = [classText(el), el.id || '', el.getAttribute('role') || '', el.getAttribute('type') || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
        let score = Math.max(tokenScore(completionHint, text), tokenScore('submit search continue confirm save create book reserve schedule done', text));
        const completionTokens = new Set(tokens(completionHint));
        for (const token of tokens(text)) if (completionTokens.has(token)) score += 0.35;
        if (/\b(?:submit|search|continue|confirm|save|create|book|reserve|schedule|done|apply|go|final)\b/i.test(metadata)) score += 0.2;
        if ((el.getAttribute('type') || '').toLowerCase() === 'submit') score += 0.5;
        const tag = el.tagName.toLowerCase();
        const role = (el.getAttribute('role') || '').toLowerCase();
        const actionText = /\b(?:submit|search|continue|confirm|save|create|book|reserve|schedule|done|apply|go|final)\b/i.test(metadata);
        const actionableMetadata = /\b(?:submit|search|continue|confirm|save|create|book|reserve|schedule|done|apply|go|final|button|btn|action)\b/i.test(controlMetadata);
        let pointer = false;
        try {{
          pointer = getComputedStyle(el).cursor === 'pointer';
          if (pointer) score += 0.2;
        }} catch (_) {{}}
        const explicitAction = role === 'button' || el.hasAttribute('tabindex') || el.hasAttribute('onclick') || pointer || actionableMetadata;
        if ((tag === 'div' || tag === 'span') && !explicitAction) {{
          score = 0;
        }}
        if (tag === 'div' && !actionableMetadata && !actionText) {{
          score -= 0.25;
        }}
        return {{ el, score }};
      }})
      .filter(item => item.score >= 0.25)
      .filter((item, _index, items) => !items.some(other =>
        other.el !== item.el &&
        item.el.contains(other.el) &&
        other.score >= 0.25
      ))
      .sort((a, b) => b.score - a.score);
    return controls[0] || null;
  }}
  function durationMinutes(text) {{
    const raw = String(text || '').toLowerCase();
    const hm = raw.match(/(\d+(?:\.\d+)?)\s*h(?:ours?|rs?)?\s*(\d+)?\s*m?/);
    if (hm) return Number(hm[1]) * 60 + Number(hm[2] || 0);
    const min = raw.match(/(\d+(?:\.\d+)?)\s*(?:m|mins?|minutes?)\b/);
    if (min) return Number(min[1]);
    return null;
  }}
  function timeHalfHourIndex(text) {{
    const raw = String(text || '').trim().toLowerCase();
    const match = raw.match(/\b(\d{{1,2}})(?::([0-9]{{2}}))?\s*(am|pm)\b/);
    if (!match) return null;
    let hour = Number(match[1]);
    const minute = Number(match[2] || 0);
    const meridiem = match[3];
    if (!Number.isFinite(hour) || hour < 1 || hour > 12 || !Number.isFinite(minute)) return null;
    if (hour === 12) hour = 0;
    if (meridiem === 'pm') hour += 12;
    return hour * 2 + (minute >= 30 ? 1 : 0);
  }}
  function requestField(pattern) {{
    return fields.find(field => pattern.test(String(field.label || '')) || (Array.isArray(field.hints) && field.hints.some(hint => pattern.test(String(hint || '')))));
  }}
  function timeSlotIndex(el) {{
    const id = el.id || '';
    let match = id.match(/\b(?:hh|slot|time)[-_]?(\d+)\b/i);
    if (match) return Number(match[1]);
    for (const attr of ['data-index', 'data-slot', 'data-half-hour', 'data-time-index']) {{
      const value = el.getAttribute && el.getAttribute(attr);
      if (value != null && /^-?\d+$/.test(String(value))) return Number(value);
    }}
    const time = el.getAttribute && (el.getAttribute('data-time') || el.getAttribute('aria-label') || el.getAttribute('title'));
    const parsed = timeHalfHourIndex(time);
    return parsed == null ? null : parsed;
  }}
  function mouseEventOn(el, type) {{
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    el.dispatchEvent(new MouseEvent(type, init));
  }}
  function createScheduledBlockFromGrid() {{
    const scheduleContext = String(params.mode || '') + ' ' + fields.map(field => field.label || '').join(' ');
    if (!/\b(?:scheduled|schedule|appointment|meeting|booking|reservation|duration|start|end|time)\b/i.test(scheduleContext)) return null;
    const durationField = requestField(/\b(duration|length)\b/i);
    const startField = requestField(/\b(start|begin|from time|between)\b/i);
    const endField = requestField(/\b(end|until|to time)\b/i);
    const duration = durationMinutes(durationField && durationField.value);
    const startIndex = timeHalfHourIndex(startField && startField.value);
    const endIndex = timeHalfHourIndex(endField && endField.value);
    if (!Number.isFinite(duration) || startIndex == null || endIndex == null || endIndex <= startIndex) return null;
    const durationSlots = Math.max(1, Math.round(duration / 30));
    const slots = all('[data-time], [data-slot], [data-index], [role=gridcell], .half-hour, .time-slot, .timeslot, [class*=slot], [class*=Slot], [class*=half-hour], [class*=time]')
      .filter(el => {{
        if (!visible(el)) return false;
        const index = timeSlotIndex(el);
        if (index == null || !Number.isFinite(index)) return false;
        const rect = el.getBoundingClientRect();
        return rect.width >= 4 && rect.height >= 4;
      }})
      .map(el => ({{ el, index: timeSlotIndex(el) }}))
      .sort((a, b) => a.index - b.index);
    if (!slots.length) return null;
    const byIndex = new Map();
    for (const slot of slots) if (!byIndex.has(slot.index)) byIndex.set(slot.index, slot.el);
    let chosenStart = startIndex;
    while (chosenStart + durationSlots > endIndex && chosenStart > startIndex) chosenStart -= 1;
    if (chosenStart + durationSlots > endIndex) chosenStart = Math.max(startIndex, endIndex - durationSlots);
    let startEl = byIndex.get(chosenStart);
    let endEl = byIndex.get(chosenStart + durationSlots - 1);
    if (!startEl || !endEl) {{
      const usable = slots.filter(slot => slot.index >= startIndex && slot.index < endIndex);
      if (usable.length < durationSlots) return null;
      startEl = usable[0].el;
      endEl = usable[Math.min(usable.length - 1, durationSlots - 1)].el;
      chosenStart = usable[0].index;
    }}
    try {{ startEl.scrollIntoView({{ block: 'center', inline: 'nearest' }}); }} catch (_) {{}}
    mouseEventOn(startEl, 'mouseover');
    mouseEventOn(startEl, 'mousedown');
    mouseEventOn(endEl, 'mousemove');
    const created = all('#newEvent, [data-new-event], [data-new-item], [data-new-block], [class*=newEvent], [class*=new-event], [class*=newItem], [class*=new-item], [class*=newBlock], [class*=new-block], [class*=event]')
      .filter(visible)
      .sort((a, b) => (b.id === 'newEvent' ? 1 : 0) - (a.id === 'newEvent' ? 1 : 0))[0];
    if (created) mouseEventOn(created, 'mouseup');
    else mouseEventOn(endEl, 'mouseup');
    return {{
      mode: 'timeline-drag',
      startIndex: chosenStart,
      durationSlots,
      startSelector: selector(startEl),
      endSelector: selector(endEl),
      consumedLabels: ['duration', 'start time', 'end time']
    }};
  }}
  function priceValue(text) {{
    const match = String(text || '').match(/[$€£]\s*([0-9][0-9,]*(?:\.\d+)?)/);
    return match ? Number(match[1].replace(/,/g, '')) : null;
  }}
  function resultText(el) {{
    const parts = [directText(el)];
    for (const child of Array.from(el.querySelectorAll ? el.querySelectorAll('*') : [])) {{
      const text = directText(child);
      if (text) parts.push(text);
    }}
    parts.push(textOf(el));
    return parts.filter(Boolean).join(' ').replace(/\s+/g, ' ').trim();
  }}
  function actionableResultContainer(el) {{
    let node = el;
    for (let depth = 0; node && depth < 6; depth += 1, node = node.parentElement) {{
      const action = Array.from(node.querySelectorAll ? node.querySelectorAll('button, a, [role=button], input[type=button], input[type=submit]') : []).find(visible);
      if (!action) continue;
      const text = resultText(node);
      if (text && text.length <= 1600) return {{ container: node, action }};
    }}
    const fallback = Array.from(el.querySelectorAll ? el.querySelectorAll('button, a, [role=button], input[type=button], input[type=submit]') : []).find(visible);
    return {{ container: el, action: fallback || el }};
  }}
  function chooseRankedResult() {{
    if (!resultPreference) return null;
    const pref = String(resultPreference).toLowerCase();
    const containers = all('tr, li, article, section, [role=row], [role=listitem], [data-result], .result, .card, div')
      .filter(el => {{
        if (!visible(el)) return false;
        const text = resultText(el);
        if (!text || text.length > 1200) return false;
        const rect = el.getBoundingClientRect();
        return rect.width >= 20 && rect.height >= 10;
      }});
    const scored = [];
    for (const el of containers) {{
      const text = resultText(el);
      let metric = null;
      if (/\b(cheapest|lowest|least expensive|price)\b/.test(pref)) metric = priceValue(text);
      else if (/\b(shortest|fastest|duration|time)\b/.test(pref)) metric = durationMinutes(text);
      else if (/\b(longest|most expensive|highest)\b/.test(pref)) metric = priceValue(text) ?? durationMinutes(text);
      if (metric == null || !Number.isFinite(metric)) continue;
      const nested = containers.some(other => {{
        if (other === el || !el.contains(other)) return false;
        const otherText = resultText(other);
        return otherText.length > 0 && (priceValue(otherText) != null || durationMinutes(otherText) != null);
      }});
      if (nested && el.tagName.toLowerCase() === 'div') continue;
      scored.push({{ el, metric, text }});
    }}
    if (!scored.length) return null;
    const descending = /\b(longest|most expensive|highest|largest)\b/.test(pref);
    scored.sort((a, b) => descending ? b.metric - a.metric : a.metric - b.metric);
    const chosen = actionableResultContainer(scored[0].el);
    const action = chosen.action;
    clickTarget(action);
    return {{ selector: selector(action), container: selector(chosen.container), metric: scored[0].metric, preference: resultPreference, text: resultText(chosen.container).slice(0, 180) }};
  }}
  function isPaginationControl(el) {{
    const meta = [
      directText(el),
      el.textContent || '',
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.id || '',
      classText(el),
      el.getAttribute && (el.getAttribute('role') || '')
    ].filter(Boolean).join(' ');
    if (el.closest && el.closest('[aria-label*=pagination i], [class*=pagination i], #pagination, nav')) return true;
    return /\b(?:next|previous|prev|page|pagination)\b/i.test(meta) || /^[\s>›»]+$/.test(String(directText(el) || el.textContent || '').trim());
  }}
  function isDisabledControl(el) {{
    const meta = [classText(el), el.getAttribute && (el.getAttribute('aria-label') || ''), el.getAttribute && (el.getAttribute('aria-disabled') || '')].join(' ');
    return !!el.disabled || el.getAttribute('disabled') != null || /\b(?:disabled|inactive)\b/i.test(meta) || el.getAttribute('aria-disabled') === 'true';
  }}
  function likelyResultRoots() {{
    const roots = all('#page-content, #results, #result, [id*=result i], [class*=result i], [data-results], [role=list], [role=table], main, article, section')
      .filter(visible)
      .filter(el => {{
        const controls = Array.from(el.querySelectorAll ? el.querySelectorAll('a, button, [role=link], [role=button], [data-result], [onclick], [tabindex]') : []).filter(visible);
        return controls.some(control => !isPaginationControl(control) && resultText(control));
      }});
    const filtered = roots.filter(root => !roots.some(other => other !== root && other.contains(root)));
    return filtered.length ? filtered : [document.body || document.documentElement];
  }}
	  function resultActionCandidates() {{
    const selectors = [
      '[data-result]',
      'a[href]',
      'button',
      '[role=link]',
      '[role=button]',
      '[onclick]',
      '[tabindex]',
      '.result',
      '[class*=result i]',
      'li',
      'tr',
      '[role=listitem]',
      '[role=row]',
      'article'
    ].join(',');
    const out = [];
    const seen = new Set();
    for (const root of likelyResultRoots()) {{
      const candidates = Array.from(root.querySelectorAll ? root.querySelectorAll(selectors) : [])
        .filter(visible)
        .filter(el => !isDisabledControl(el))
        .filter(el => !isFieldLikeControl(el))
        .filter(el => !isPaginationControl(el))
        .filter(el => {{
          const tag = el.tagName.toLowerCase();
          const type = (el.getAttribute('type') || '').toLowerCase();
          if ((tag === 'button' || tag === 'input') && ['submit', 'reset'].includes(type)) return false;
          const text = resultText(el);
          if (!text) return false;
          if (text.length > 1000) return false;
          const meta = [text, classText(el), el.id || '', el.getAttribute('role') || '', el.getAttribute('aria-label') || ''].join(' ');
          if (/\b(?:search|submit|filter|apply|go|done|save|continue)\b/i.test(meta) && !/\bresults?\b/i.test(meta) && !el.hasAttribute('data-result')) return false;
          return true;
        }});
      for (const el of candidates) {{
        const action = actionableResultContainer(el).action || el;
        const key = selector(action);
        if (!key || seen.has(key)) continue;
        seen.add(key);
        out.push(action);
      }}
    }}
    return out
      .filter((el, _index, arr) => !arr.some(other => other !== el && el.contains(other) && resultText(other)))
      .sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return (ar.top - br.top) || (ar.left - br.left);
	      }});
	  }}
	  function chooseNamedResultAction() {{
	    if (!resultClickHint) return null;
	    const hint = String(resultClickHint || '').trim();
	    if (!hint) return null;
	    const candidates = resultActionCandidates()
	      .map(el => {{
	        const text = resultText(el);
	        const direct = directText(el);
	        const meta = [text, direct, classText(el), el.id || '', el.getAttribute && (el.getAttribute('aria-label') || ''), el.getAttribute && (el.getAttribute('title') || '')].join(' ');
	        const hintNorm = normalize(hint);
	        const directNorm = normalize(direct);
	        const metaNorm = normalize(meta);
	        let score = Math.max(tokenScore(hint, direct) * 2, tokenScore(hint, meta) * 1.2);
	        if (hintNorm && directNorm === hintNorm) score += 1.4;
	        else if (hintNorm && directNorm.split(' ').includes(hintNorm)) score += 0.8;
	        else if (hintNorm && metaNorm.includes(hintNorm)) score += 0.5;
	        const tag = el.tagName.toLowerCase();
	        const role = (el.getAttribute('role') || '').toLowerCase();
	        if (score > 0 && (tag === 'a' || tag === 'button' || role === 'link' || role === 'button')) score += 0.25;
	        return {{ el, score, text }};
	      }})
	      .filter(item => item.score >= 0.45)
	      .sort((a, b) => b.score - a.score);
	    if (!candidates.length) return null;
	    const chosen = candidates[0];
	    clickResultTarget(chosen.el);
	    return {{ selector: selector(chosen.el), hint, score: chosen.score, text: chosen.text.slice(0, 180) }};
	  }}
	  function resultSignature() {{
    return resultActionCandidates()
      .map(el => [selector(el), el.getAttribute && (el.getAttribute('data-result') || ''), resultText(el).slice(0, 80)].join(':'))
      .join('|');
  }}
  function nextPageControl() {{
    const controls = all('a, button, [role=button], [role=link], [onclick], [tabindex], .page-link, .page-item, li, span')
      .filter(visible)
      .filter(el => !isDisabledControl(el))
      .filter(el => !isFieldLikeControl(el))
      .map(el => {{
        const raw = [directText(el), el.textContent || '', el.getAttribute && (el.getAttribute('aria-label') || ''), el.getAttribute && (el.getAttribute('title') || '')].filter(Boolean).join(' ').trim();
        const meta = [raw, classText(el), el.id || '', el.getAttribute && (el.getAttribute('role') || '')].join(' ');
        let score = 0;
        if (/\bnext\b/i.test(meta)) score += 0.85;
        if (/^[\s>›»]+$/.test(raw)) score += 0.8;
        if (el.closest && el.closest('[aria-label*=pagination i], [class*=pagination i], #pagination')) score += 0.25;
        if (/\bdisabled|inactive\b/i.test(meta)) score -= 1;
        return {{ el, score }};
      }})
      .filter(item => item.score >= 0.65)
      .filter((item, _index, items) => !items.some(other => other.el !== item.el && item.el.contains(other.el) && other.score >= item.score))
      .sort((a, b) => b.score - a.score);
    return controls[0] ? controls[0].el : null;
  }}
  async function chooseOrdinalResult() {{
    if (resultOrdinal == null) return null;
    const maxPages = Number.isFinite(Number(params.maxResultPages)) ? Math.max(1, Number(params.maxResultPages)) : 12;
    let seen = 0;
    const visited = new Set();
    for (let pageIndex = 0; pageIndex < maxPages; pageIndex += 1) {{
      await delay(pageIndex === 0 ? 0 : 180);
      const candidates = resultActionCandidates();
      const exact = resultOrdinal >= 0 ? candidates.find(el => String(el.getAttribute && (el.getAttribute('data-result') || '')) === String(resultOrdinal)) : null;
      if (exact) {{
        clickResultTarget(exact);
        return {{ selector: selector(exact), ordinal: resultOrdinal, pageIndex, mode: 'data-result', text: resultText(exact).slice(0, 180) }};
      }}
      if (candidates.length) {{
        if (resultOrdinal === -1) {{
          const next = nextPageControl();
          if (!next) {{
            const chosen = candidates[candidates.length - 1];
            clickResultTarget(chosen);
            return {{ selector: selector(chosen), ordinal: resultOrdinal, pageIndex, mode: 'last-visible-result', text: resultText(chosen).slice(0, 180) }};
          }}
        }} else if (resultOrdinal < seen + candidates.length) {{
          const chosen = candidates[Math.max(0, resultOrdinal - seen)];
          clickResultTarget(chosen);
          return {{ selector: selector(chosen), ordinal: resultOrdinal, pageIndex, mode: 'accumulated-visual-order', text: resultText(chosen).slice(0, 180) }};
        }}
      }}
      seen += candidates.length;
      const before = resultSignature();
      if (visited.has(before)) break;
      visited.add(before);
      const next = nextPageControl();
      if (!next) break;
      clickTarget(next);
      for (const wait of [120, 220, 420]) {{
        await delay(wait);
        const after = resultSignature();
        if (after && after !== before) break;
      }}
    }}
    return null;
  }}

  const used = new Set();
  const filled = [];
  const timelineEvent = createScheduledBlockFromGrid();
  if (timelineEvent) await delay(150);
  const consumedLabels = new Set((timelineEvent && timelineEvent.consumedLabels || []).map(normalize));
  const requestedUnconsumedFields = fields.filter(field =>
    field &&
    field.value != null &&
    field.value !== '' &&
    !consumedLabels.has(normalize(field.label || ''))
  );
  for (const field of fields) {{
    if (!field || field.value == null || field.value === '') continue;
    if (consumedLabels.has(normalize(field.label || ''))) continue;
    const result = fillRequest(field, used);
    if (result) {{
      filled.push(result);
      const filledEl = all(result.selector)[0];
      if (filledEl && filledEl.tagName && filledEl.tagName.toLowerCase() === 'input') {{
        openSuggestions(filledEl, String(field.value || ''));
        await delay(120);
        chooseVisibleSuggestion(filledEl, String(field.value || ''));
      }}
    }}
  }}
  const dateIso = requestedDate && (requestedDate.iso || normalizeDate(requestedDate.value || requestedDate));
  const dateAlreadyFilled = filled.some(item => /\bdate|day|when\b/i.test(String(item.label || '')) && item.ok);
  const dateResult = dateIso && !dateAlreadyFilled ? selectDate(dateIso, used) : null;
  const submit = submitControl();
  if (submit) {{
    submitTarget(submit.el);
    await delay(250);
  }}
  let rankedResult = null;
	  if (resultPreference) {{
    for (const wait of [100, 250, 500]) {{
      await delay(wait);
      rankedResult = chooseRankedResult();
      if (rankedResult) break;
    }}
  }}
  let ordinalResult = null;
  if (resultOrdinal != null) {{
    for (const wait of [100, 250, 500]) {{
      await delay(wait);
      ordinalResult = await chooseOrdinalResult();
      if (ordinalResult) break;
    }}
	  }}
	  let namedResult = null;
	  if (resultClickHint && !rankedResult && !ordinalResult) {{
	    for (const wait of [100, 250, 500]) {{
	      await delay(wait);
	      namedResult = chooseNamedResultAction();
	      if (namedResult) break;
	    }}
	  }}
	  if (!filled.length && !dateResult && !submit && !rankedResult && !ordinalResult && !namedResult && !timelineEvent) {{
    return {{ ok: false, error: 'form_workflow could not match fields, date, submit, or result controls' }};
  }}
  if (requestedUnconsumedFields.length && !filled.length && !dateResult && !timelineEvent) {{
    return {{
      ok: false,
      error: 'form_workflow could not fill any requested fields',
      requestedFields: requestedUnconsumedFields.map(field => field.label || null).filter(Boolean)
    }};
  }}
  return {{
    ok: true,
    filled,
    timelineEvent,
    date: dateResult,
	    submitted: submit ? {{ selector: selector(submit.el), score: submit.score }} : null,
	    rankedResult,
	    ordinalResult,
	    namedResult,
	    mode: params.mode || 'generic-form-workflow'
  }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "form_workflow timed out".to_string())?
        .map_err(|e| {
            format!(
                "form_workflow failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({
            "formWorkflow": value,
            "state": capture_compact_page_state(page, false).await,
        }))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("form_workflow failed")
            .to_string())
    }
}

pub(super) async fn handle_date_picker(page: &Page, params: &Value) -> Result<Value, String> {
    let params_json = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
    let availability_helpers_js = availability_helpers_js();
    let value_control_helpers_js = value_control_helpers_js();
    let js = format!(
        r#"(async () => {{
  const params = {params_json};
  const date = params.date || {{}};
  const openerSelector = params.opener || null;
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
  function isReadOnlyControl(el) {{
    if (!el || !el.getAttribute) return false;
    return el.disabled || el.readOnly || el.getAttribute('aria-readonly') === 'true' || el.getAttribute('aria-disabled') === 'true';
  }}
  function isCustomWritableValueElement(_el) {{
    return false;
  }}
  {value_control_helpers_js}
  function all(selectorText, start = document) {{
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
    const out = [];
    const seen = new Set();
    for (const root of roots) {{
      try {{
        if (root.matches && root.matches(selectorText) && !seen.has(root)) {{
          seen.add(root);
          out.push(root);
        }}
        for (const el of Array.from(root.querySelectorAll ? root.querySelectorAll(selectorText) : [])) {{
          if (seen.has(el)) continue;
          seen.add(el);
          out.push(el);
        }}
      }} catch (_) {{}}
    }}
    return out;
  }}
  function selector(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + CSS.escape(el.id);
const href = el.getAttribute && el.getAttribute('href');
if (href) {{
  const byHref = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
  try {{ if (document.querySelectorAll(byHref).length === 1) return byHref; }} catch (_) {{}}
}}
    const name = el.getAttribute('name');
    if (name) {{
      const byName = el.tagName.toLowerCase() + '[name=' + JSON.stringify(name) + ']';
      if (all(byName).length === 1) return byName;
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
  function directText(el) {{
    if (!el) return '';
    return [
      Array.from(el.childNodes || []).filter(node => node.nodeType === Node.TEXT_NODE).map(node => node.textContent || '').join(' '),
      el.getAttribute && (el.getAttribute('aria-label') || ''),
      el.getAttribute && (el.getAttribute('title') || ''),
      el.getAttribute && (el.getAttribute('data-date') || ''),
      el.getAttribute && (el.getAttribute('datetime') || ''),
      el.value || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
  function candidate(el) {{
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    return {{ selector: selector(el), text: directText(el), role: el.getAttribute('role') || null, bounds: {{ x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) }} }};
  }}
  function click(el) {{
    if (!el) return false;
    try {{ el.scrollIntoView({{ block: 'center', inline: 'center' }}); }} catch (_) {{}}
    try {{ el.focus && el.focus(); }} catch (_) {{}}
    for (const type of ['focus', 'focusin']) {{
      try {{ el.dispatchEvent(new FocusEvent(type, {{ bubbles: true, cancelable: false, view: window }})); }} catch (_) {{}}
    }}
    const rect = el.getBoundingClientRect();
    const init = {{ bubbles: true, cancelable: true, view: window, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }};
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {{
      try {{
        const event = type.startsWith('pointer') && window.PointerEvent ? new PointerEvent(type, init) : new MouseEvent(type, init);
        el.dispatchEvent(event);
      }} catch (_) {{}}
    }}
    return true;
  }}
  function setValue(el, value) {{
    if (!el) return false;
    const previousReadonly = el.readOnly;
    try {{ el.readOnly = false; }} catch (_) {{}}
    const ok = setControlValue(el, value);
    try {{ el.readOnly = previousReadonly; }} catch (_) {{}}
    return ok;
  }}
  function openDateControl(el) {{
    if (!el) return false;
    click(el);
    try {{
      if (typeof el.showPicker === 'function') el.showPicker();
    }} catch (_) {{}}
    try {{
      if (window.jQuery && window.jQuery.fn && window.jQuery.fn.datepicker) {{
        const jq = window.jQuery(el);
        if (jq.data('datepicker') || jq.hasClass('hasDatepicker')) jq.datepicker('show');
      }}
    }} catch (_) {{}}
    return true;
  }}
  function closeDateControl(el) {{
    try {{
      if (window.jQuery && window.jQuery.fn && window.jQuery.fn.datepicker) {{
        const jq = window.jQuery(el);
        if (jq.data('datepicker') || jq.hasClass('hasDatepicker')) jq.datepicker('hide');
      }}
    }} catch (_) {{}}
    try {{ document.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Escape', bubbles: true }})); }} catch (_) {{}}
    try {{ window.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Escape', bubbles: true }})); }} catch (_) {{}}
    try {{ el && el.blur && el.blur(); }} catch (_) {{}}
    for (const popup of all('[role=dialog], [role=grid], .ui-datepicker, .datepicker, [class*=datepicker], [class*=Datepicker], [data-datepicker-popup]')) {{
      if (!visible(popup)) continue;
      if (el && (popup === el || popup.contains(el))) continue;
      const text = (popup.textContent || '').toLowerCase();
      const classes = classText(popup).toLowerCase();
      if (!/\b(?:date|datepicker|calendar|ui-datepicker)\b/.test([text, classes].join(' '))) continue;
      try {{
        popup.setAttribute('aria-hidden', 'true');
        popup.style.display = 'none';
        popup.style.visibility = 'hidden';
      }} catch (_) {{}}
    }}
  }}
  function dateText(el) {{
    return [
      el.getAttribute('data-date') || '',
      el.getAttribute('data-day') || '',
      el.getAttribute('data-value') || '',
      el.getAttribute('datetime') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('value') || '',
      directText(el),
      el.textContent || ''
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
	  function contextText(el) {{
	    const parts = [];
	    const container = el && el.closest && el.closest('[role=dialog], [role=grid], .ui-datepicker, .datepicker, .calendar, [class*=datepicker], [class*=Datepicker], [class*=calendar], [class*=Calendar]');
	    if (container) parts.push(container.textContent || '');
    let node = el;
    for (let depth = 0; node && depth < 6; depth += 1, node = node.parentElement) {{
      parts.push(directText(node));
      parts.push(node.getAttribute && (node.getAttribute('aria-label') || node.getAttribute('title') || ''));
	    }}
	    return parts.join(' ').replace(/\s+/g, ' ').trim();
	  }}
	  function roleOf(el) {{
	    return String(el && el.getAttribute && el.getAttribute('role') || '').toLowerCase();
	  }}
	  function ownTextOnly(el) {{
	    return Array.from(el && el.childNodes || [])
	      .filter(node => node.nodeType === Node.TEXT_NODE)
	      .map(node => node.textContent || '')
	      .join(' ')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function exactDayNumber(text) {{
	    const match = String(text || '').trim().match(/^(\d{{1,2}})(?:st|nd|rd|th)?$/i);
	    if (!match) return null;
	    const number = Number(match[1]);
	    return Number.isFinite(number) ? number : null;
	  }}
	  function hasExactDayText(el) {{
	    if (!el) return false;
	    const checks = [
	      ownTextOnly(el),
	      el.getAttribute('data-day') || '',
	      el.getAttribute('aria-label') || '',
	      el.getAttribute('title') || '',
	    ];
	    for (const text of checks) {{
	      if (exactDayNumber(text) === Number(date.day)) return true;
	    }}
	    const controls = Array.from(el.querySelectorAll ? el.querySelectorAll('button, a, [role=button], [role=link], [role=gridcell], [role=option]') : []);
	    return controls.some(child => visible(child) && exactDayNumber(ownTextOnly(child) || directText(child)) === Number(date.day));
	  }}
	  function hasDateMetadata(el) {{
	    if (!el) return false;
	    const values = [
	      el.getAttribute('data-date') || '',
	      el.getAttribute('datetime') || '',
	      el.getAttribute('data-value') || '',
	      el.getAttribute('aria-label') || '',
	      el.getAttribute('title') || '',
	      el.getAttribute('value') || '',
	    ].join(' ').toLowerCase();
	    if (date.iso && values.includes(String(date.iso).toLowerCase())) return true;
	    if (date.slash && values.includes(String(date.slash).toLowerCase())) return true;
	    if (date.monthName && values.includes(String(date.monthName).toLowerCase()) && values.includes(String(date.day)) && values.includes(String(date.year))) return true;
	    if (date.monthShort && values.includes(String(date.monthShort).toLowerCase()) && values.includes(String(date.day)) && values.includes(String(date.year))) return true;
	    return Number(el.getAttribute('data-month')) === Number(date.month) &&
	      Number(el.getAttribute('data-year')) === Number(date.year) &&
	      Number(el.getAttribute('data-day')) === Number(date.day);
	  }}
	  function dateControlDescendantCount(el) {{
	    if (!el || !el.querySelectorAll) return 0;
	    return Array.from(el.querySelectorAll('[data-date], [data-day], [datetime], [role=gridcell], [role=option], button, a, .day, .date, .calendar-day'))
	      .filter(child => visible(child))
	      .length;
	  }}
	  function isCalendarContainer(el) {{
	    if (!el) return false;
	    const tag = el.tagName.toLowerCase();
	    const role = roleOf(el);
	    const label = [classText(el), el.id || '', el.getAttribute('aria-label') || '', role].join(' ');
	    if (/\b(?:dialog|grid)\b/i.test(role)) return true;
	    if (/\b(?:ui-datepicker|datepicker|calendar|date-picker)\b/i.test(label) && tag !== 'button' && tag !== 'a') return true;
	    return dateControlDescendantCount(el) >= 8;
	  }}
	  function dateCellClickTarget(el) {{
	    if (!el) return null;
	    const tag = el.tagName.toLowerCase();
	    const role = roleOf(el);
	    if (['button', 'a'].includes(tag) || ['button', 'link', 'gridcell', 'option'].includes(role) || hasDateMetadata(el)) return el;
	    if (tag === 'td' || /\b(?:day|date|calendar-day)\b/i.test(classText(el))) {{
	      const controls = Array.from(el.querySelectorAll ? el.querySelectorAll('button, a, [role=button], [role=link], [role=gridcell], [role=option], [data-date], [data-day], [datetime]') : []);
	      const exact = controls.find(child => visible(child) && (hasDateMetadata(child) || hasExactDayText(child)));
	      if (exact) return exact;
	      if (hasExactDayText(el) || hasDateMetadata(el)) return el;
	    }}
	    return null;
	  }}
	  function isLikelyDateCell(el) {{
	    const target = dateCellClickTarget(el);
	    if (!target) return false;
	    if (isCalendarContainer(target) && !hasDateMetadata(target) && !hasExactDayText(target)) return false;
	    if (isCalendarContainer(el) && el !== target) return true;
	    if (isCalendarContainer(el) && !hasDateMetadata(el)) return false;
	    return hasDateMetadata(target) || hasExactDayText(target);
	  }}
	  function scoreCell(el) {{
	    if (!isLikelyDateCell(el)) return -1;
	    const text = dateText(el).toLowerCase();
	    const context = contextText(el).toLowerCase();
	    const direct = directText(el).trim();
	    let score = 0;
    if (date.iso && text.includes(String(date.iso).toLowerCase())) score += 1.2;
    if (date.slash && text.includes(String(date.slash).toLowerCase())) score += 1.1;
    if (date.monthName && text.includes(String(date.monthName).toLowerCase()) && text.includes(String(date.day)) && text.includes(String(date.year))) score += 1.0;
    if (date.monthShort && text.includes(String(date.monthShort).toLowerCase()) && text.includes(String(date.day)) && text.includes(String(date.year))) score += 0.9;
    if (/^\s*\d{{1,2}}\s*$/.test(direct) && Number(direct) === Number(date.day)) score += 0.4;
    if (date.monthName && context.includes(String(date.monthName).toLowerCase()) && context.includes(String(date.year))) score += 0.45;
	    if (date.monthShort && context.includes(String(date.monthShort).toLowerCase()) && context.includes(String(date.year))) score += 0.35;
	    if (Number(el.getAttribute('data-month')) === Number(date.month) && Number(el.getAttribute('data-year')) === Number(date.year) && Number(el.getAttribute('data-day')) === Number(date.day)) score += 1.1;
	    if (/\b(?:disabled|outside|other-month|unavailable)\b/i.test(classText(el)) || el.getAttribute('aria-disabled') === 'true') score -= 0.8;
	    if (isCalendarContainer(el) && !hasDateMetadata(el)) score -= 1.0;
	    return score;
	  }}
	  function visibleDateCells() {{
	    const seen = new Set();
	    return all('[data-date], [data-day], [datetime], [role=gridcell], [role=option], button, a, td, .day, .date, .calendar-day')
	      .filter(el => {{
	        if (!visible(el)) return false;
	        const tag = el.tagName.toLowerCase();
	        if (['script', 'style', 'input', 'select', 'textarea'].includes(tag)) return false;
	        const rect = el.getBoundingClientRect();
	        return rect.width >= 4 && rect.height >= 4;
	      }})
	      .map(el => dateCellClickTarget(el))
	      .filter(el => {{
	        if (!el || !visible(el)) return false;
	        const key = selector(el);
	        if (seen.has(key)) return false;
	        seen.add(key);
	        return true;
	      }})
	      .map(el => ({{ el, score: scoreCell(el) }}))
	      .filter(item => item.score >= 0.6)
	      .sort((a, b) => b.score - a.score);
	  }}

  const opener = openerSelector ? all(openerSelector)[0] : null;
  const steps = [];
  if (opener) {{
    openDateControl(opener);
    steps.push({{ action: 'open_date_picker', target: candidate(opener) }});
    await delay(260);
  }}

  const cells = visibleDateCells();
  if (cells.length) {{
    const chosen = cells[0].el;
    click(chosen);
    steps.push({{ action: 'select_date_cell', target: candidate(chosen), score: cells[0].score }});
    return {{ ok: true, mode: 'date_picker_cell', date: date.iso || date.slash || null, steps }};
  }}

  if (opener && 'value' in opener) {{
    const value = date.slash || date.iso || '';
    if (value) {{
      setValue(opener, value);
      closeDateControl(opener);
      await delay(60);
      steps.push({{ action: 'set_date_value', target: candidate(opener), value }});
      return {{ ok: true, mode: 'date_value_fallback', date: value, steps }};
    }}
  }}

  return {{ ok: false, error: 'date_picker could not open or select the requested date', steps }};
}})()"#
    );
    let result = timeout(PLAN_TIMEOUT, page.evaluate_expression(&js))
        .await
        .map_err(|_| "date_picker timed out".to_string())?
        .map_err(|e| {
            format!(
                "date_picker failed: {}",
                crate::daemon::handlers::clean_cdp_error(&e)
            )
        })?;
    let value = result.value().cloned().unwrap_or_else(|| json!({}));
    if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(value)
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("date_picker failed")
            .to_string())
    }
}
