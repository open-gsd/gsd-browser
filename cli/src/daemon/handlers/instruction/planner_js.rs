use super::model::{json_literal, InstructionAnalysis, InstructionIntent};
use super::page_model::{
    accessible_text_helpers_js, availability_helpers_js, control_semantics_helpers_js,
};

pub(super) fn planner_js(
    instruction: &str,
    analysis: &InstructionAnalysis,
    intent: &InstructionIntent,
    scope: Option<&str>,
) -> String {
    let instruction_json = json_literal(instruction);
    let intent_json = json_literal(&intent.to_json());
    let kind_json = json_literal(analysis.kind.as_str());
    let value_json = json_literal(&analysis.value);
    let target_json = json_literal(&analysis.target_hint);
    let secondary_json = json_literal(&analysis.secondary_hint);
    let checked_json = json_literal(&analysis.checked);
    let direction_json = json_literal(&analysis.direction);
    let scope_json = json_literal(&scope);
    let accessible_text_helpers_js = accessible_text_helpers_js();
    let availability_helpers_js = availability_helpers_js();
    let control_semantics_helpers_js = control_semantics_helpers_js();
    let capability_runtime_js = capability_runtime_js();
    let text_matcher_js = text_matcher_js();
    let text_transfer_capability_js = text_transfer_capability_js();
    let capability_registry_js = capability_registry_js();

    format!(
        r#"(() => {{
  const instruction = {instruction_json};
  const intent = {intent_json};
  const kind = {kind_json};
  const wantedValue = {value_json};
  const targetHint = {target_json};
  const secondaryHint = {secondary_json};
  const checked = {checked_json};
  const direction = {direction_json};
  const scopeSelector = {scope_json};
  const clickStyle = String(wantedValue || '').toLowerCase();
  const root = scopeSelector ? document.querySelector(scopeSelector) : document;
  if (!root) return {{ ok: false, error: 'act_instruction: scope not found: ' + scopeSelector }};

  function clickParamsFor(el) {{
    const params = {{ selector: selector(el) }};
    if (clickStyle === 'right_click') params.button = 'right';
    if (clickStyle === 'double_click') params.click_count = 2;
    return params;
  }}

  {availability_helpers_js}
  function visible(el) {{
    if (unavailableForAction(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  function readableVisible(el) {{
    if (unavailableForRead(el)) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return (r.width > 0 || r.height > 0) &&
      s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
  }}
  {accessible_text_helpers_js}
  function textOf(el) {{
    return [
      el.textContent || '', el.value || '', el.getAttribute('name') || '', el.placeholder || '',
      el.getAttribute('aria-label') || '', el.getAttribute('title') || '',
	      referencedText(el, 'aria-labelledby'), referencedText(el, 'aria-describedby'),
	      el.getAttribute('aria-description') || '', semanticAttributeText(el),
	      el.getAttribute('role') || '', el.getAttribute('data-testid') || '',
	      associatedLabelText(el), structuralLabelText(el), nearbyLabelText(el), shadowHostText(el), slotText(el)
	    ].join(' ').replace(/\s+/g, ' ').trim();
	  }}
  function classText(el) {{
    if (!el) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className && typeof el.className.baseVal === 'string') return el.className.baseVal;
    return el.getAttribute && el.getAttribute('class') || '';
  }}
  function iconSemanticText(el) {{
    if (!el) return '';
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
      settings: 'settings preferences gear cog',
      gear: 'settings preferences gear cog',
      edit: 'edit pencil',
      pencil: 'edit pencil',
      upload: 'upload import',
      download: 'download export',
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
          .replace(/^(?:ui-icon-|fa-|fas-|far-|fal-|fab-|icon-|Icon-|lucide-|mdi-|material-icons?-?)/i, '')
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
  {text_matcher_js}
  function selector(el) {{
    if (el.id) return '#' + CSS.escape(el.id);
    const testId = el.getAttribute('data-testid');
    if (testId) return el.tagName.toLowerCase() + '[data-testid=' + JSON.stringify(testId) + ']';
    const href = el.getAttribute('href');
    if (href) {{
      const sel = el.tagName.toLowerCase() + '[href=' + JSON.stringify(href) + ']';
      if (all(sel).length === 1) return sel;
    }}
    const nameAttr = el.getAttribute('name');
    if (nameAttr) {{
      const sel = el.tagName.toLowerCase() + '[name=' + JSON.stringify(nameAttr) + ']';
      if (all(sel).length === 1) return sel;
    }}
    const type = el.getAttribute('type');
    if (type) {{
      const sel = el.tagName.toLowerCase() + '[type=' + JSON.stringify(type) + ']';
      if (all(sel).length === 1) return sel;
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
  function directTextOf(el) {{
    if (!el) return '';
    const direct = Array.from(el.childNodes || [])
      .filter(node => node.nodeType === Node.TEXT_NODE)
      .map(node => node.textContent || '')
      .join(' ');
    return [
      direct,
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.value || '',
      el.getAttribute('name') || '',
      el.getAttribute('data-testid') || '',
      semanticAttributeText(el),
    ].join(' ').replace(/\s+/g, ' ').trim();
  }}
  function candidate(el) {{
    const rect = el.getBoundingClientRect();
    return {{
      selector: selector(el),
      tag: el.tagName.toLowerCase(),
      type: (el.getAttribute('type') || '').toLowerCase() || null,
      role: (el.getAttribute('role') || '').toLowerCase() || null,
      text: [textOf(el), iconSemanticText(el)].join(' ').trim().slice(0, 160),
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
  function bestReadable(elements, score) {{
    const scored = [];
    for (const el of elements) {{
      if (!readableVisible(el)) continue;
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
    if (intent && intent.followUpClickHint) return intent.followUpClickHint;
    const match = instruction.match(/\b(?:and|then)\s+(?:find\s+(?:and\s+)?)?(?:click|press|tap|hit)\s+(?:on\s+)?(?:the\s+)?([^,.]+)\.?$/i);
    if (!match) return null;
    const cleaned = match[1]
      .replace(/\b(button|link|control|item|element|labelled|labeled|called|named|with|icon|icons)\b/ig, '')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .trim();
    return cleaned || match[1].replace(/^["'\s]+|["'.\s]+$/g, '').trim();
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
  function ordinalIndexFromText(text) {{
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
  function lexicalVariants(token) {{
    const lower = String(token || '').toLowerCase();
    const variants = new Set([lower]);
    if (lower.endsWith('ies') && lower.length > 4) variants.add(lower.slice(0, -3) + 'y');
    if (lower.endsWith('es') && lower.length > 3) variants.add(lower.slice(0, -2));
    if (lower.endsWith('s') && lower.length > 3) variants.add(lower.slice(0, -1));
    if (lower.endsWith('ing') && lower.length > 5) variants.add(lower.slice(0, -3));
    if (lower.endsWith('ed') && lower.length > 4) variants.add(lower.slice(0, -2));
    return variants;
  }}
  function semanticTokens(word) {{
    const base = tokens(word);
    const out = new Set(base);
    for (const token of base) {{
      for (const variant of lexicalVariants(token)) out.add(variant);
    }}
    return out;
  }}
  const semanticRelationGroups = [
    ['good', 'great', 'excellent', 'positive', 'nice', 'pleasant', 'happy', 'glad', 'gleeful', 'joyful', 'cheerful'],
    ['bad', 'evil', 'wicked', 'awful', 'terrible', 'poor', 'negative', 'wrong', 'immoral', 'sinful', 'corrupt', 'depraved'],
    ['wrong', 'incorrect', 'mistaken', 'erroneous'],
    ['real', 'genuine', 'actual'],
    ['mad', 'angry', 'furious', 'irate', 'upset', 'annoyed', 'irritated'],
    ['sad', 'unhappy', 'miserable', 'sorrowful', 'depressed', 'gloomy', 'tragic'],
    ['brave', 'bold', 'courageous', 'fearless', 'heroic'],
    ['afraid', 'scared', 'fearful', 'frightened', 'timid', 'terrified', 'panicked'],
    ['hate', 'dislike', 'detest', 'loathe', 'despise'],
    ['love', 'adore', 'like', 'enjoy', 'cherish', 'favor'],
    ['finish', 'end', 'complete', 'stop', 'conclude', 'done', 'cease', 'halt'],
    ['start', 'begin', 'open', 'commence', 'launch', 'initiate'],
    ['old', 'aged', 'ancient', 'elderly', 'mature', 'archaic'],
    ['new', 'young', 'fresh', 'recent', 'modern'],
    ['strange', 'weird', 'odd', 'unusual', 'bizarre', 'peculiar'],
    ['normal', 'usual', 'ordinary', 'regular', 'common'],
    ['quiet', 'calm', 'peaceful', 'serene', 'mild'],
    ['keep', 'retain', 'preserve', 'sustain', 'maintain'],
    ['hide', 'conceal', 'camouflage'],
    ['kill', 'slay', 'destroy', 'remove', 'eliminate', 'murder', 'assassinate'],
    ['answer', 'reply', 'response'],
    ['red', 'scarlet', 'crimson', 'vermillion'],
    ['fast', 'quick', 'rapid', 'swift', 'speedy'],
    ['slow', 'sluggish', 'delayed', 'gradual'],
    ['large', 'big', 'huge', 'giant', 'massive', 'enormous', 'gigantic'],
    ['small', 'little', 'tiny', 'mini', 'minor', 'petite'],
    ['hot', 'warm', 'heated'],
    ['cold', 'cool', 'chilly', 'freezing'],
    ['smart', 'clever', 'bright', 'intelligent'],
    ['easy', 'simple', 'plain', 'basic'],
    ['hard', 'difficult', 'complex', 'tough'],
    ['funny', 'humorous', 'amusing', 'comical', 'laughable'],
    ['fat', 'fleshy', 'plump', 'chubby'],
    ['stupid', 'dumb', 'dull', 'unwise'],
    ['delicious', 'savory', 'delectable', 'appetizing'],
    ['cut', 'slice', 'carve', 'chop']
  ];
  function semanticConceptTokens(text) {{
    const out = semanticTokens(text);
    const base = new Set(out);
    for (let index = 0; index < semanticRelationGroups.length; index++) {{
      const group = semanticRelationGroups[index];
      if (!group.some(term => base.has(term))) continue;
      out.add('concept:' + index);
      for (const term of group) out.add(term);
    }}
    return out;
  }}
  function semanticScore(hint, text) {{
    const ht = semanticConceptTokens(hint);
    if (!ht.size) return 0;
    const tt = semanticConceptTokens(text);
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
  function clickableAffordanceScore(el) {{
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    return tag === 'button' || tag === 'a' || type === 'submit' || role === 'button' ? 0.05 : 0;
  }}
  function submitLikeScore(hint, el, text) {{
    const haystack = [hint || '', instruction].join(' ');
    if (!/submit|continue|confirm|save|done|next|ok/i.test(haystack)) return 0;
    let score = 0;
    if (typeOf(el) === 'submit') score += 0.5;
    if (/submit|continue|confirm|save|done|next|ok/i.test(text)) score += 0.4;
    return score;
  }}
  function wantsCloseLike(hint) {{
    return /\b(?:x|×|close|dismiss)\b/i.test(String(hint || '')) || /\b(?:close|dismiss)\b/i.test(instruction);
  }}
  function closeLikeScore(hint, el, text) {{
    if (!wantsCloseLike(hint)) return 0;
    const classes = classText(el);
    let score = 0;
    if (/\b(?:close|dismiss)\b/i.test(text) || /\b(?:close|dismiss)\b/i.test(classes)) score += 0.75;
    if (el.getAttribute('aria-label') && /\b(?:close|dismiss)\b/i.test(el.getAttribute('aria-label'))) score += 0.75;
    if (el.getAttribute('title') && /\b(?:close|dismiss)\b/i.test(el.getAttribute('title'))) score += 0.75;
    if (classes.includes('ui-dialog-titlebar-close')) score += 0.9;
    if (/^\s*[x×]\s*$/i.test(text)) score += 0.6;
    return score;
  }}
  function scoreClickableTarget(hint, el, options = {{}}) {{
    if (!hint) return 0;
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const broadContainer = ['menu', 'menubar', 'listbox', 'tree'].includes(role) || ['ul', 'ol'].includes(tag);
    if (broadContainer && el.querySelector('button, a, [role=button], [role=link], [role=menuitem], [role=option], [onclick], [tabindex]')) return 0;
    const iconText = iconSemanticText(el);
    const text = [textOf(el), iconText].join(' ').trim();
    const includeSemantic = options.semantic === true;
    let score = Math.max(tokenScore(hint, text), exactPhraseScore(hint, text));
    const direct = [directTextOf(el), iconText].join(' ').trim();
    if (direct) score = Math.max(score, tokenScore(hint, direct), exactPhraseScore(hint, direct));
    if (!direct && text.length > String(hint || '').length + 30) score -= 0.35;
    if (includeSemantic) score = Math.max(score, semanticScore(hint, text), semanticScore(hint, direct));
    score += submitLikeScore(hint, el, text);
    score += closeLikeScore(hint, el, text);
    score += clickableAffordanceScore(el);
    score += relationScore(el, options.anchor || null);
    return score;
  }}
  function clickStepForHint(hint, anchor = null) {{
    if (!hint) return null;
    const ranked = best(clickableElements(), el => {{
      return scoreClickableTarget(hint, el, {{ anchor }});
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
  {capability_runtime_js}
  function all(selectorText, start = root) {{
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
    addRoot(start);
    const results = [];
    const seen = new Set();
    for (const scope of roots) {{
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
  let interactive = all(
    'button, a, input, textarea, select, [role=button], [role=link], [role=listbox], [role=menu], [role=tree], [role=option], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=tab], [role=slider], [role=spinbutton], [role=textbox], [role=searchbox], [role=combobox], [role=switch], [aria-pressed], [onclick], [tabindex], [draggable=true], [draggable="true"], [contenteditable]:not([contenteditable="false"]), .alink, .ui-button, .ui-dialog-titlebar-close, .ui-menu-item, .ui-menu-item-wrapper, [class*=button], [class*=Button], [class*=link], [class*=Link], [class*=close], [class*=Close], [class*=tab], [class*=Tab]'
  );
  function isLikelyClickable(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const type = typeOf(el);
    const classes = classText(el);
    if (tag === 'button' || tag === 'a' || type === 'button' || type === 'submit') return true;
    if (['button', 'link', 'option', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'tab'].includes(role)) return true;
    if (el.hasAttribute('onclick') || el.hasAttribute('tabindex')) return true;
    if (/\b(?:alink|button|link|close|tab|ui-button|ui-menu-item|ui-menu-item-wrapper)\b/i.test(classes)) return true;
    try {{
      if (getComputedStyle(el).cursor === 'pointer') return true;
    }} catch (_) {{}}
    return false;
  }}
  function clickableElements() {{
    const seen = new Set();
    const out = [];
    for (const el of interactive.concat(all('span, div, li'))) {{
      if (!visible(el) || !isLikelyClickable(el)) continue;
      const key = selector(el);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(el);
    }}
    return out;
  }}
  const textInputTypes = new Set(['', 'text', 'password', 'email', 'search', 'url', 'tel', 'number', 'date', 'time', 'month', 'week', 'datetime-local', 'color', 'range']);
  function roleOf(el) {{
    return (el.getAttribute('role') || '').toLowerCase();
  }}
  function typeOf(el) {{
    return (el.getAttribute('type') || '').toLowerCase();
  }}
  function isFileField(el) {{
    return el.tagName.toLowerCase() === 'input' && typeOf(el) === 'file';
  }}
  function isEditableElement(el) {{
    const editable = el.getAttribute('contenteditable');
    return el.isContentEditable || (editable !== null && editable.toLowerCase() !== 'false');
  }}
  {control_semantics_helpers_js}
  function isValueField(el) {{
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    if (tag === 'textarea' || tag === 'select') return true;
    if (tag === 'input') return !['button', 'submit', 'reset', 'image', 'file', 'hidden'].includes(type);
    if (isCustomWritableValueElement(el)) return true;
    if (isEditableElement(el)) return true;
    return ['textbox', 'searchbox', 'spinbutton', 'slider', 'combobox', 'listbox'].includes(role);
  }}
  function isFillableField(el) {{
    if (isReadOnlyControl(el)) return false;
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    const writableCombobox = role === 'combobox' && tag !== 'button';
    if (isCustomSelectableValueElement(el)) return false;
    return tag === 'textarea' || isEditableElement(el) ||
      isCustomWritableValueElement(el) ||
      (tag === 'input' && textInputTypes.has(type)) ||
      ['textbox', 'searchbox', 'spinbutton', 'slider'].includes(role) ||
      writableCombobox;
  }}
  function writableField(el) {{
    if (!isFillableField(el)) return false;
    if (unavailableForAction(el)) return false;
    const type = typeOf(el);
    return !['button', 'submit', 'checkbox', 'radio', 'file', 'hidden'].includes(type);
  }}
  function actionableValueField(el) {{
    if (isFileField(el)) return !hasDisabledAncestor(el);
    return (isSelectableField(el) && !isFillableField(el)) ? !unavailableForAction(el) : writableField(el);
  }}
  function writableFields(elements = interactive) {{
    return elements.filter(writableField);
  }}
  function valueFields(elements = interactive) {{
    return elements.filter(el => readableVisible(el) && isValueField(el));
  }}
  function isCheckedControl(el) {{
    const type = typeOf(el);
    const role = roleOf(el);
    return type === 'checkbox' || type === 'radio' ||
      ['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio'].includes(role) ||
      isCustomCheckableElement(el) ||
      el.hasAttribute('aria-pressed');
  }}
  function isCustomSliderControl(el) {{
    return isCustomSliderValueElement(el);
  }}
  function isCustomNumericControl(el) {{
    return isCustomNumericValueElement(el);
  }}
  function isSliderControl(el) {{
    if (typeOf(el) === 'range' || roleOf(el) === 'slider' || isCustomSliderControl(el)) return true;
    try {{
      if (!window.jQuery) return false;
      const jq = window.jQuery(el);
      return !!(jq && jq.data && (jq.data('ui-slider') || jq.data('slider')));
    }} catch (_) {{
      return false;
    }}
  }}
  function isSliderLikeControl(el) {{
    return typeOf(el) === 'range' || roleOf(el) === 'slider' || isSliderControl(el);
  }}
  function sliderControls() {{
    const candidates = all('input[type=range], [role=slider], .ui-slider, [class*=slider], [class*=range], [data-field*=slider], [data-field*=range], [data-control*=slider], [data-control*=range]');
    for (const el of all('*')) {{
      if (isCustomSliderControl(el)) candidates.push(el);
    }}
    return Array.from(new Set(candidates)).filter(el => visible(el) && isSliderControl(el));
  }}
  function numericValueFromText(value) {{
    const match = String(value ?? '').match(/-?\d+(?:\.\d+)?/);
    return match ? Number(match[0]) : NaN;
  }}
  function valueFieldActionStep(el, value, options = {{}}) {{
    const selectable = options.selectable !== undefined
      ? options.selectable
      : (isSelectableField(el) && !isFillableField(el));
    const selectedValue = selectable ? value : transformedValue(value);
    const numericValue = numericValueFromText(selectedValue);
    const baseEvidence = Object.assign({{}}, options.evidence || {{}});
    if (!selectable && isFileField(el)) {{
      return {{
        action: 'upload_file',
        params: {{ selector: selector(el), files: [String(selectedValue || '').trim()] }},
        confidence: options.confidence || 0.7,
        reason: options.fileReason || 'matched file input value for field action',
        candidate: candidate(el),
        evidence: Object.assign(baseEvidence, {{ value, valueAction: 'file_upload' }})
      }};
    }}
    if (!selectable && isSliderLikeControl(el) && Number.isFinite(numericValue)) {{
      return {{
        action: 'set_slider',
        params: {{ selector: selector(el), value: numericValue }},
        confidence: options.confidence || 0.7,
        reason: options.sliderReason || 'matched slider or range value for field action',
        candidate: candidate(el),
        evidence: Object.assign(baseEvidence, {{ value, numericValue, valueAction: 'slider' }})
      }};
    }}
    return {{
      action: selectable ? 'select_option' : 'type',
      params: selectable
        ? {{ selector: selector(el), option: value }}
        : {{ selector: selector(el), text: selectedValue, clear_first: true }},
      confidence: options.confidence || 0.7,
      reason: selectable
        ? (options.selectReason || 'matched selectable field value')
        : (options.typeReason || 'matched fillable field value'),
      candidate: candidate(el),
      evidence: baseEvidence
    }};
  }}
  function mergeInteractiveCustomControls() {{
    const seen = new Set(interactive);
    for (const el of all('*')) {{
      if (seen.has(el) || !visible(el)) continue;
      if (!isCustomWritableValueElement(el) && !isCustomCheckableElement(el)) continue;
      seen.add(el);
      interactive.push(el);
    }}
  }}
  mergeInteractiveCustomControls();
  function controlTypeScore(hint, el) {{
    const haystack = normalized([hint || '', instruction].join(' '));
    const tag = el.tagName.toLowerCase();
    const type = typeOf(el);
    const role = roleOf(el);
    let score = 0;
    if (/\b(slider|range)\b/.test(haystack) && (type === 'range' || role === 'slider' || isCustomSliderControl(el) || /\bslider\b/i.test(textOf(el)))) score += 0.7;
    if (/\b(spinner|spinbutton|stepper|numeric|number)\b/.test(haystack) && (type === 'number' || role === 'spinbutton' || isCustomNumericControl(el))) score += 0.7;
    const customKind = customValueSemanticKind(el);
    if (/\bdate\b/.test(haystack) && (type === 'date' || customKind === 'date')) score += 0.7;
    if (/\btime\b/.test(haystack) && (type === 'time' || customKind === 'time')) score += 0.7;
    if (/\bmonth\b/.test(haystack) && (type === 'month' || customKind === 'month')) score += 0.7;
    if (/\bweek\b/.test(haystack) && (type === 'week' || customKind === 'week')) score += 0.7;
    if (/\b(datetime|date time|date-time)\b/.test(haystack) && (type === 'datetime-local' || customKind === 'datetime-local')) score += 0.7;
    if (/\b(colou?r)\b/.test(haystack) && (type === 'color' || customKind === 'color')) score += 0.7;
    if (/\b(file|upload|attach|attachment|document|resume|avatar|photo|image|pdf)\b/.test(haystack) && type === 'file') score += 0.85;
    if (/\bsearch\b/.test(haystack) && (type === 'search' || role === 'searchbox')) score += 0.4;
    if (/\b(text|input|field|box)\b/.test(haystack) && (tag === 'input' || tag === 'textarea' || role === 'textbox')) score += 0.2;
    return score;
  }}
  function scoreWritableFieldTarget(el, options = {{}}) {{
    const hint = options.hint !== undefined ? options.hint : targetHint;
    const text = textOf(el);
    let score = hint
      ? Math.max(tokenScore(hint, text), exactPhraseScore(hint, text), semanticScore(hint, text))
      : (options.defaultScore ?? 0.2);
    score += controlTypeScore(hint, el);
    const context = [hint || '', text, instruction].join(' ');
    if (options.textLike && /\b(text|input|field|box|search|message|comment|editor|notes?)\b/i.test(context)) score += 0.15;
    if (hint && /\b(text|input|field|box)\b/i.test(hint) && score === 0) score = options.genericFieldScore ?? 0.2;
    if (options.searchBoost && typeOf(el) === 'search') score += 0.2;
    if (options.searchBoost && /\bsearch\b/i.test(instruction) && /\bsearch\b/i.test(text)) score += 0.5;
    if (options.passwordBoost && /\bpassword\b/i.test(instruction) && typeOf(el) === 'password') score += 0.6;
    return score;
  }}
  function rankedWritableFields(options = {{}}) {{
    const fields = options.fields || writableFields();
    return best(fields, el => scoreWritableFieldTarget(el, options));
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
  function ordinalTargetIndex(text) {{
    const lower = String(text || '').toLowerCase();
    const named = [
      ['last', -1],
      ['first', 0], ['1st', 0],
      ['second', 1], ['2nd', 1],
      ['third', 2], ['3rd', 2],
      ['fourth', 3], ['4th', 3],
      ['fifth', 4], ['5th', 4],
      ['sixth', 5], ['6th', 5],
      ['seventh', 6], ['7th', 6],
      ['eighth', 7], ['8th', 7],
      ['ninth', 8], ['9th', 8],
      ['tenth', 9], ['10th', 9],
    ];
    for (const [word, index] of named) if (lower.includes(word)) return index;
    const match = lower.match(/\b(\d+)(?:st|nd|rd|th)?\s+(?:button|link|row|card|item|option|tab|result|entry|tile|swatch)\b/);
    if (match) return Math.max(0, Number(match[1]) - 1);
    return null;
  }}
  function ordinalTargetKind() {{
    if (/\bbuttons?\b/i.test(instruction)) return 'button';
    if (/\blinks?\b/i.test(instruction)) return 'link';
    if (/\btabs?\b/i.test(instruction)) return 'tab';
    if (/\brows?\b/i.test(instruction)) return 'row';
    if (/\bcards?\b/i.test(instruction)) return 'card';
    if (/\boptions?\b/i.test(instruction)) return 'option';
    if (/\b(?:items?|results?|entries)\b/i.test(instruction)) return 'item';
    return null;
  }}
  function visualOrder(elements) {{
    return elements.slice().sort((a, b) => {{
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      const dy = (ar.top + ar.height / 2) - (br.top + br.height / 2);
      if (Math.abs(dy) > 4) return dy;
      return (ar.left + ar.width / 2) - (br.left + br.width / 2);
    }});
  }}
  function ordinalClickCandidates(kind) {{
    const clickables = clickableElements();
    return visualOrder(clickables.filter(el => {{
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const classes = classText(el);
      if (kind === 'button') return tag === 'button' || role === 'button' || ['button', 'submit'].includes(typeOf(el));
      if (kind === 'link') return tag === 'a' || role === 'link';
      if (kind === 'tab') return role === 'tab' || /\btab\b/i.test(classes);
      if (kind === 'row') return tag === 'tr' || role === 'row' || role === 'listitem' || /\brow\b/i.test(classes);
      if (kind === 'card') return /\bcard\b/i.test(classes) || ['article', 'section'].includes(tag);
      if (kind === 'option') return role === 'option' || role === 'menuitem' || tag === 'option';
      if (kind === 'item') return role === 'listitem' || tag === 'li' || /\b(?:item|result|entry|row|card)\b/i.test(classes);
      return false;
    }}));
  }}
  function ordinalClickPlan() {{
    if (!kindIs('click', 'count')) return null;
    const index = ordinalTargetIndex(instruction);
    if (index == null) return null;
    const targetKind = ordinalTargetKind();
    if (!targetKind) return null;
    const candidates = ordinalClickCandidates(targetKind);
    if (!candidates.length) return null;
    const resolvedIndex = index === -1 ? candidates.length - 1 : index;
    if (resolvedIndex < 0 || resolvedIndex >= candidates.length) return null;
    const chosen = candidates[resolvedIndex];
    const confidence = Math.min(0.92, 0.68 + (targetKind === 'button' || targetKind === 'link' || targetKind === 'tab' ? 0.12 : 0.06));
    return {{
      action: 'click',
      params: {{ selector: selector(chosen) }},
      confidence,
      reason: 'matched ordinal visible clickable target by role/type and visual order',
      candidate: candidate(chosen),
      evidence: {{ ordinalIndex: index, resolvedIndex, targetKind, candidateCount: candidates.length }}
    }};
  }}
  function checkedControlKind() {{
    if (/\b(?:radios?|radio\s+buttons?)\b/i.test(instruction)) return 'radio';
    if (/\b(?:switch(?:es)?)\b/i.test(instruction)) return 'switch';
    if (/\b(?:toggles?|toggle\s+buttons?|pressed\s+buttons?)\b/i.test(instruction)) return 'toggle';
    if (/\b(?:checkbox(?:es)?|check\s*boxes?)\b/i.test(instruction)) return 'checkbox';
    return null;
  }}
  function checkedControlMatchesKind(el, controlKind) {{
    const type = typeOf(el);
    const role = roleOf(el);
    if (controlKind === 'checkbox') return type === 'checkbox' || role === 'checkbox' || role === 'menuitemcheckbox' || isCustomCheckableElement(el);
    if (controlKind === 'radio') return type === 'radio' || role === 'radio' || role === 'menuitemradio';
    if (controlKind === 'switch') return role === 'switch' || isCustomCheckableElement(el);
    if (controlKind === 'toggle') return el.hasAttribute('aria-pressed') || role === 'switch' || isCustomCheckableElement(el);
    return false;
  }}
  function ordinalCheckedControlCandidates(controlKind) {{
    return visualOrder(interactive.filter(el => {{
      if (!isCheckedControl(el) || !visible(el)) return false;
      return checkedControlMatchesKind(el, controlKind);
    }}));
  }}
  function ordinalCheckedControlPlan() {{
    if (!kindIs('set_checked', 'select_option')) return null;
    const index = ordinalTargetIndex(instruction);
    if (index == null) return null;
    const controlKind = checkedControlKind();
    if (!controlKind) return null;
    const candidates = ordinalCheckedControlCandidates(controlKind);
    if (!candidates.length) return null;
    const resolvedIndex = index === -1 ? candidates.length - 1 : index;
    if (resolvedIndex < 0 || resolvedIndex >= candidates.length) return null;
    const chosen = candidates[resolvedIndex];
    return {{
      action: 'set_checked',
      params: {{ selector: selector(chosen), checked: checked !== false }},
      confidence: 0.9,
      reason: 'matched ordinal checked control by control type and visual order',
      candidate: candidate(chosen),
      evidence: {{ ordinalIndex: index, resolvedIndex, controlKind, candidateCount: candidates.length }}
    }};
  }}
  function groupedChoiceIntent() {{
    if (!kindIs('set_checked', 'select_option')) return null;
    const patterns = [
      /\b(?:choose|select|pick|check|tick|enable|turn\s+on)\s+(?:the\s+)?("[^"]+"|'[^']+'|[^,.]+?)\s+(?:from|in|under|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i,
      /\b(?:in|under|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[^,.]+?)[,;]?\s+(?:choose|select|pick|check|tick|enable|turn\s+on)\s+(?:the\s+)?("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (!match) continue;
      const inverted = /^\b(?:in|under|within)\b/i.test(match[0]);
      const rawGroup = inverted ? match[1] : match[2];
      if (/\b(?:section|panel|region|fieldset|form|area)\b/i.test(rawGroup)) continue;
      const option = cleanFieldPairValue(inverted ? match[2] : match[1]);
      const groupHint = cleanFieldPairLabel(rawGroup);
      if (option) return {{ option, groupHint }};
    }}
    const option = cleanFieldPairValue(kind === 'select_option' ? wantedValue : targetHint);
    const groupHint = cleanFieldPairLabel(kind === 'select_option' ? targetHint : secondaryHint);
    if (!option || !groupHint) return null;
    return {{ option, groupHint }};
  }}
  function checkedControlOptionText(el) {{
    const out = [
      directTextOf(el),
      el.getAttribute('aria-label') || '',
      referencedText(el, 'aria-labelledby'),
      el.getAttribute('title') || '',
      semanticAttributeText(el),
      el.value || '',
      el.id || '',
      el.getAttribute('name') || ''
    ];
    out.push(associatedLabelText(el));
    return out.join(' ').replace(/\s+/g, ' ').trim();
  }}
  function checkedControlGroupInfo(el) {{
    const containers = [
      el.closest('fieldset'),
      el.closest('[role=radiogroup]'),
      el.closest('[role=group]'),
      el.closest('[aria-label], [aria-labelledby]'),
      el.closest('form'),
      el.parentElement
    ].filter(Boolean);
    for (const container of containers) {{
      let explicit = '';
      if (container.tagName && container.tagName.toLowerCase() === 'fieldset') {{
        const legend = container.querySelector(':scope > legend');
        if (legend) explicit = legend.textContent || '';
      }}
      explicit = [
        explicit,
        container.getAttribute && container.getAttribute('aria-label') || '',
        container.getAttribute && referencedText(container, 'aria-labelledby') || '',
        container.getAttribute && container.getAttribute('name') || '',
        container.id || ''
      ].join(' ').replace(/\s+/g, ' ').trim();
      const text = [explicit, directTextOf(container), container.textContent || ''].join(' ').replace(/\s+/g, ' ').trim();
      if (explicit || text) return {{ container, explicit, text }};
    }}
    return {{ container: null, explicit: '', text: '' }};
  }}
	  function groupedChoiceControlPlan() {{
	    const parsed = groupedChoiceIntent();
	    if (!parsed) return null;
	    const checkedControls = scope => all('input[type=radio], input[type=checkbox], [role=radio], [role=checkbox], [role=switch], [aria-pressed], [role=menuitemradio], [role=menuitemcheckbox]', scope)
	      .concat(all('*', scope).filter(isCustomCheckableElement))
	      .filter((el, index, arr) => arr.indexOf(el) === index);
	    const controls = visualOrder(checkedControls(root)
	      .filter(el => visible(el) && isCheckedControl(el)));
    if (!controls.length) return null;
    const ranked = best(controls, el => {{
      const optionText = checkedControlOptionText(el);
      const group = checkedControlGroupInfo(el);
      const optionScore = Math.max(
        exactPhraseScore(parsed.option, optionText),
        tokenScore(parsed.option, optionText),
        semanticScore(parsed.option, optionText)
      );
      if (optionScore < 0.5) return 0;
      let groupScore = 0;
      if (parsed.groupHint) {{
        groupScore = Math.max(
          exactPhraseScore(parsed.groupHint, group.explicit),
          tokenScore(parsed.groupHint, group.explicit),
          semanticScore(parsed.groupHint, group.explicit),
          exactPhraseScore(parsed.groupHint, group.text) * 0.75,
          tokenScore(parsed.groupHint, group.text) * 0.75,
          semanticScore(parsed.groupHint, group.text) * 0.75
        );
        if (groupScore < 0.35) return 0;
      }}
      let score = 0.25 + optionScore * 0.65 + (parsed.groupHint ? groupScore * 0.45 : 0.08);
      if (checkedControlMatchesKind(el, 'radio')) score += 0.08;
      if (checkedControlMatchesKind(el, 'checkbox')) score += 0.04;
      return score;
    }});
    if (!ranked.length || ranked[0].score < 0.75) return null;
    const chosen = ranked[0].el;
    const group = checkedControlGroupInfo(chosen);
    const primary = {{
      action: 'set_checked',
      params: {{ selector: selector(chosen), checked: checked !== false }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'matched grouped choice control by option label and group label',
      candidate: candidate(chosen),
      evidence: {{
        option: parsed.option,
        groupHint: parsed.groupHint || null,
        group: group.container ? candidate(group.container) : null,
        optionText: checkedControlOptionText(chosen).slice(0, 160)
      }}
    }};
    return withFollowUp(primary, chosen);
  }}
  function ordinalFieldIndex(text) {{
    const lower = String(text || '').toLowerCase();
    const named = [
      ['last', -1],
      ['first', 0], ['1st', 0],
      ['second', 1], ['2nd', 1],
      ['third', 2], ['3rd', 2],
      ['fourth', 3], ['4th', 3],
      ['fifth', 4], ['5th', 4],
      ['sixth', 5], ['6th', 5],
      ['seventh', 6], ['7th', 6],
      ['eighth', 7], ['8th', 7],
      ['ninth', 8], ['9th', 8],
      ['tenth', 9], ['10th', 9],
    ];
    for (const [word, index] of named) {{
      const pattern = new RegExp('\\\\b' + word.replace(/[.*+?^${{}}()|[\\]\\\\]/g, '\\\\$&') + '\\\\s+(?:input\\\\s+)?(?:text\\\\s*box|textbox|input|field|textarea|text\\\\s*area|dropdown|select|combobox)s?\\\\b');
      if (pattern.test(lower)) return index;
    }}
    const match = lower.match(/\b(\d+)(?:st|nd|rd|th)?\s+(?:input\s+)?(?:text\s*box|textbox|input|field|textarea|text\s*area|dropdown|select|combobox)s?\b/);
    return match ? Math.max(0, Number(match[1]) - 1) : null;
  }}
  function ordinalFillableFields(text) {{
    const lower = String(text || '').toLowerCase();
    if (/\b(?:dropdown|select|combobox)\b/i.test(lower)) {{
      return visualOrder(interactive.filter(el => visible(el) && isSelectableField(el)));
    }}
    let fields = formFieldCandidates().filter(el => visible(el) && writableField(el));
    if (/\b(?:textarea|text\s*area)\b/i.test(lower)) fields = fields.filter(el => el.tagName.toLowerCase() === 'textarea');
    else if (/\b(?:textbox|text\s*box|input)\b/i.test(lower)) fields = fields.filter(el => {{
      const tag = el.tagName.toLowerCase();
      const type = typeOf(el);
      const role = roleOf(el);
      if (tag === 'textarea' || role === 'textbox' || role === 'searchbox') return true;
      return tag === 'input' && !['button', 'submit', 'checkbox', 'radio', 'file', 'hidden', 'range', 'color'].includes(type);
    }});
    return visualOrder(fields);
  }}
  function ordinalFieldByInstruction(text) {{
    const index = ordinalFieldIndex(text);
    if (index == null) return null;
    const fields = ordinalFillableFields(text);
    if (!fields.length) return null;
    const resolvedIndex = index === -1 ? fields.length - 1 : index;
    if (resolvedIndex < 0 || resolvedIndex >= fields.length) return null;
    return {{ el: fields[resolvedIndex], ordinalIndex: index, resolvedIndex, candidateCount: fields.length }};
  }}
  function compoundFormStepsPlan() {{
    const text = String(instruction || '');
    if (!/\b(?:and|then|,)\b/i.test(text)) return null;
    if (!/\b(?:check|tick|select|choose|enter|type|fill|input|click|press|tap)\b/i.test(text)) return null;
    const steps = [];
    const usedSelectors = new Set();
    let anchor = null;

    function pushStep(step, el) {{
      if (!step) return false;
      steps.push(step);
      if (el) {{
        anchor = el;
        usedSelectors.add(selector(el));
      }}
      return true;
    }}
    function checkedKindFromText(value) {{
      if (/\bradio(?:\s+button)?s?\b/i.test(value)) return 'radio';
      if (/\bcheckbox(?:es)?|check\s*box(?:es)?\b/i.test(value)) return 'checkbox';
      if (/\bswitch(?:es)?\b/i.test(value)) return 'switch';
      if (/\btoggle(?:s)?|toggle\s+buttons?|pressed\s+buttons?\b/i.test(value)) return 'toggle';
      return null;
    }}
    function checkedOrdinalIndex(value) {{
      const match = String(value || '').match(/\b(last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|sixth|6th|seventh|7th|eighth|8th|ninth|9th|tenth|10th|\d+(?:st|nd|rd|th)?)\s+(?:checkbox|check\s*box|radio|radio\s+button|switch|toggle)s?\b/i);
      if (!match) return null;
      const token = match[1].toLowerCase();
      const named = {{ last: -1, first: 0, '1st': 0, second: 1, '2nd': 1, third: 2, '3rd': 2, fourth: 3, '4th': 3, fifth: 4, '5th': 4, sixth: 5, '6th': 5, seventh: 6, '7th': 6, eighth: 7, '8th': 7, ninth: 8, '9th': 8, tenth: 9, '10th': 9 }};
      if (Object.prototype.hasOwnProperty.call(named, token)) return named[token];
      const numeric = token.match(/\d+/);
      return numeric ? Math.max(0, Number(numeric[0]) - 1) : null;
    }}

    const checkedClauses = [];
    const checkedPattern = /\b(?:check|tick|select|choose|turn\s+on|enable)\s+(?:the\s+)?((?:last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|sixth|6th|seventh|7th|eighth|8th|ninth|9th|tenth|10th|\d+(?:st|nd|rd|th)?)\s+(?:checkbox|check\s*box|radio|radio\s+button|switch|toggle)s?)\b/gi;
    let match;
    while ((match = checkedPattern.exec(text)) !== null) {{
      checkedClauses.push({{ text: match[1], checked: true }});
    }}
    for (const clause of checkedClauses) {{
      const controlKind = checkedKindFromText(clause.text);
      const index = checkedOrdinalIndex(clause.text);
      if (!controlKind || index == null) continue;
      const controls = visualOrder(interactive.filter(el => visible(el) && isCheckedControl(el) && checkedControlMatchesKind(el, controlKind)));
      const resolvedIndex = index === -1 ? controls.length - 1 : index;
      const chosen = controls[resolvedIndex];
      if (!chosen) continue;
      pushStep({{
        action: 'set_checked',
        params: {{ selector: selector(chosen), checked: clause.checked }},
        confidence: 0.88,
        reason: 'matched ordinal checked control inside compound form instruction',
        candidate: candidate(chosen),
        evidence: {{ controlKind, ordinalIndex: index, resolvedIndex, candidateCount: controls.length }}
      }}, chosen);
    }}

    const fillPattern = /\b(?:enter|type|fill|input|write)\s+(?:the\s+)?(?:number|value|text)?\s*("[^"]+"|'[^']+'|-?\d+(?:\.\d+)?|[^,.]+?)\s+(?:into|in|to)\s+(?:the\s+)?((?:last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|sixth|6th|seventh|7th|eighth|8th|ninth|9th|tenth|10th|\d+(?:st|nd|rd|th)?)\s+(?:input\s+)?(?:text\s*box|textbox|input|field|textarea|text\s*area)s?)\b/gi;
    while ((match = fillPattern.exec(text)) !== null) {{
      const value = cleanFieldPairValue(match[1]);
      const field = ordinalFieldByInstruction(match[2]);
      if (!value || !field || usedSelectors.has(selector(field.el))) continue;
      pushStep({{
        action: 'type',
        params: {{ selector: selector(field.el), text: transformedValue(value), clear_first: true }},
        confidence: 0.86,
        reason: 'matched ordinal fillable field inside compound form instruction',
        candidate: candidate(field.el),
        evidence: {{ value, ordinalIndex: field.ordinalIndex, resolvedIndex: field.resolvedIndex, candidateCount: field.candidateCount }}
      }}, field.el);
    }}

    const selectPattern = /\b(?:choose|select|pick)\s+(.+?)\s+from\s+(?:the\s+)?((?:last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|\d+(?:st|nd|rd|th)?)?\s*(?:dropdown|select|combobox|menu|list)(?:\s+field)?)\b/gi;
    while ((match = selectPattern.exec(text)) !== null) {{
      const option = cleanFieldPairValue(match[1]);
      let fields = ordinalFillableFields(match[2]).filter(el => isSelectableField(el));
      if (!fields.length) fields = visualOrder(interactive.filter(el => visible(el) && isSelectableField(el)));
      let chosen = null;
      const ordinal = ordinalFieldIndex(match[2]);
      if (ordinal != null) {{
        const resolvedIndex = ordinal === -1 ? fields.length - 1 : ordinal;
        chosen = fields[resolvedIndex] || null;
      }} else {{
        const ranked = best(fields, el => {{
          const options = Array.from(el.options || []).map(o => o.textContent || o.value || '').join(' ');
          let score = Math.max(tokenScore(option, options), exactPhraseScore(option, options));
          if (/\b(dropdown|select|combobox|menu|list)\b/i.test([match[2], textOf(el)].join(' '))) score += 0.25;
          return score;
        }});
        chosen = ranked[0] && ranked[0].el;
      }}
      if (!option || !chosen || usedSelectors.has(selector(chosen))) continue;
      pushStep({{
        action: 'select_option',
        params: {{ selector: selector(chosen), option }},
        confidence: 0.86,
        reason: 'matched selectable field and option inside compound form instruction',
        candidate: candidate(chosen),
        evidence: {{ option, fieldHint: match[2] }}
      }}, chosen);
    }}

    const clickFollowMatch = text.match(/\b(?:click|press|tap)\s+(?:the\s+)?(?:button|link|control)?\s*(?:label(?:ed|led)|called|named)?\s*("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9_-]*)/i);
    const explicitFollow = cleanClickHint(cleanFieldPairValue(followUpClickHint() || ''));
    const extractedFollow = clickFollowMatch ? cleanClickHint(cleanFieldPairValue(clickFollowMatch[1])) : '';
    const forbidsCompletion = /\b(?:do\s+not|don't|without)\s+(?:submit|save|continue|confirm|finish|click(?:ing)?\s+(?:submit|save|continue|confirm|done))\b/i.test(text);
    const follow = clickStepForHint(explicitFollow, anchor) ||
      clickStepForHint(extractedFollow, anchor) ||
      (!forbidsCompletion ? completionClickStep(anchor) : null);
    if (follow) steps.push(follow);
    if (steps.length < 2) return null;
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned compound form instruction across ordinal controls and fields',
      evidence: {{ stepCount: steps.length }}
    }};
  }}
  function cleanClickHint(text) {{
    return String(text || '')
      .replace(/\b(?:button|link|control|item|element|labelled|labeled|called|named|with|icon|icons)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function cleanCheckedControlHint(text) {{
    return String(text || '')
      .replace(/\b(?:checkbox(?:es)?|check\s*boxes?|radios?|radio\s+buttons?|switch(?:es)?|toggles?|control|option|button|field|the|a|an)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function orderedClickHints() {{
    if (intent && Array.isArray(intent.orderedClickHints) && intent.orderedClickHints.length) {{
      return intent.orderedClickHints;
    }}
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
      const ranked = best(clickableElements().filter(el => !used.has(selector(el))), el => {{
        return scoreClickableTarget(hint, el, {{ anchor, semantic: true }});
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
	  function cleanScopedFragment(text) {{
	    return String(text || '')
	      .replace(/^["'\s]+|["'.\s]+$/g, '')
	      .replace(/\b(?:button|link|control|icon|action|row|card|item|record|entry|result|section|panel|user|account|person|profile|contact|customer)\b/ig, ' ')
	      .replace(/^["'\s]+|["'.\s]+$/g, '')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function cleanContainerNameFragment(text) {{
	    return String(text || '')
	      .replace(/^["'\s]+|["'.\s]+$/g, '')
	      .replace(/\b(?:section|panel|region|group|fieldset|form|area)\b/ig, ' ')
	      .replace(/^["'\s]+|["'.\s]+$/g, '')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function scopedChildClickIntent() {{
	    if (kind !== 'click') return null;
	    const namedContainerPatterns = [
	      {{
	        pattern: /\b(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)(?:[,.]|$)/i,
	        action: 1,
	        item: 2
	      }},
	      {{
	        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[,;]?\s+(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i,
	        item: 1,
	        action: 2
	      }}
	    ];
	    for (const entry of namedContainerPatterns) {{
	      const match = instruction.match(entry.pattern);
	      if (!match) continue;
	      const actionHint = cleanScopedFragment(match[entry.action]);
	      const itemQuery = cleanContainerNameFragment(match[entry.item]);
	      if (actionHint && itemQuery) return {{ actionHint, itemQuery }};
	    }}
	    const patterns = [
	      /\b(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i,
      /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i,
      /\bfor\s+(?:the\s+)?(?:user|account|person|profile|contact|customer|row|item)?\s*("[^"]+"|'[^']+'|@[A-Za-z0-9_.-]+|[^,]+?),\s*(?:click|press|tap|hit)\s+(?:on\s+)?(?:the\s+)?(.+?)(?:[,.]|$)/i,
      /\b(?:click|press|tap|hit)\s+(?:the\s+)?(.+?)\s+(?:for|on)\s+("[^"]+"|'[^']+'|[A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})(?:[,.]|$)/i
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (!match) continue;
      const inverted = /^\b(?:in|inside|within)\b/i.test(match[0]);
      const forScoped = /^\bfor\b/i.test(match[0]);
      const actionHint = cleanScopedFragment(forScoped || inverted ? match[2] : match[1]);
      const itemQuery = cleanScopedFragment(forScoped || inverted ? match[1] : match[2]);
      if (actionHint && itemQuery) return {{ actionHint, itemQuery }};
    }}
    return null;
  }}
  function menuItemLike(el) {{
    if (!el) return false;
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    if (['menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'treeitem'].includes(role)) return true;
    if (tag !== 'li') return false;
    const parent = el.parentElement;
    if (!parent) return false;
    const parentTag = parent.tagName.toLowerCase();
    if (!['ul', 'ol', 'menu'].includes(parentTag) && !['menu', 'menubar', 'listbox', 'tree'].includes(roleOf(parent))) return false;
    const text = directTextOf(el) || textOf(el);
    return !!text && text.length <= 120;
  }}
  function localMenuTriggerFor(container, target) {{
    const menu = target.closest && target.closest('ul, ol, menu, [role=menu], [role=listbox], [role=tree]');
    const triggerCandidates = all('button, a, [role=button], [aria-haspopup], [aria-expanded], [onclick], [tabindex], .more, [class*=more], [class*=More], [class*=menu], [class*=Menu], [class*=overflow], [class*=Overflow], [class*=ellipsis], [class*=Ellipsis], span, div', container)
      .filter(el => visible(el) && el !== container && !el.contains(target) && (!menu || !menu.contains(el)));
    const ranked = best(triggerCandidates, el => {{
      const text = [textOf(el), directTextOf(el), iconSemanticText(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = 0;
      if (/\b(more|menu|overflow|actions?|options?|ellipsis|kebab|dropdown|expand|open)\b/i.test(text)) score += 0.65;
      if (el.getAttribute('aria-haspopup') || el.getAttribute('aria-expanded') != null) score += 0.35;
      if (isLikelyClickable(el)) score += 0.25;
      if (menu) score += relationScore(el, menu) * 0.5;
      score += relationScore(el, target) * 0.25;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0 && rect.width * rect.height < 20000) score += 0.08;
      return score;
    }});
    return ranked.length && ranked[0].score >= 0.3 ? ranked[0] : null;
  }}
  function hiddenScopedMenuClickPlan(containerRank, parsed) {{
    const container = containerRank.el;
    const candidates = all('li, [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=option], [role=treeitem], .ui-menu-item, .ui-menu-item-wrapper', container)
      .filter(el => el !== container && menuItemLike(el));
    const rankedTargets = candidates
      .map(el => ({{ el, score: scoreClickableTarget(parsed.actionHint, el, {{ anchor: container, semantic: true }}) }}))
      .filter(item => item.score >= 0.3)
      .sort((a, b) => b.score - a.score);
    if (!rankedTargets.length) return null;
    const target = rankedTargets[0];
    if (visible(target.el)) return null;
    const trigger = localMenuTriggerFor(container, target.el);
    if (!trigger) return null;
    return {{
      action: 'scoped_menu_click',
      params: {{
        container: selector(container),
        trigger: selector(trigger.el),
        action_hint: parsed.actionHint
      }},
      confidence: Math.min(1, (containerRank.score + trigger.score + target.score) / 3),
      reason: 'planned scoped local menu reveal and child action inside same container',
      candidate: {{
        container: candidate(container),
        trigger: candidate(trigger.el),
        target: candidate(target.el)
      }},
      evidence: {{
        itemQuery: parsed.itemQuery,
        actionHint: parsed.actionHint,
        container: candidate(container),
        containerScore: containerRank.score,
        triggerScore: trigger.score,
        targetScore: target.score
      }}
    }};
  }}
  function scopedContainersForQuery(itemQuery) {{
    const containers = all('tr, [role=row], [role=listitem], li, article, section, [data-testid], .card, .row, .item, .record, .result, div')
      .filter(el => {{
        if (!visible(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style'].includes(tag)) return false;
        const text = textOf(el);
        if (!text || text.length > 1500) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 8 || rect.height < 8) return false;
        return true;
      }});
    return best(containers, el => {{
      const text = textOf(el);
      let score = Math.max(tokenScore(itemQuery, text), exactPhraseScore(itemQuery, text), semanticScore(itemQuery, text));
      if (!score) return 0;
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const classes = classText(el);
      if (['tr', 'li', 'article', 'section'].includes(tag)) score += 0.08;
      if (['row', 'listitem', 'article'].includes(role)) score += 0.16;
      if (/\b(?:row|card|item|record|result|entry)\b/i.test(classes)) score += 0.14;
      const rect = el.getBoundingClientRect();
      const area = rect.width * rect.height;
      if (area > 240000) score -= 0.18;
      const nestedMatches = containers.filter(other => other !== el && el.contains(other) && Math.max(tokenScore(itemQuery, textOf(other)), exactPhraseScore(itemQuery, textOf(other))) > 0).length;
      if (nestedMatches) score -= Math.min(0.2, nestedMatches * 0.05);
      return score;
    }});
  }}
  function scopedChildClickPlan() {{
    const parsed = scopedChildClickIntent();
    if (!parsed) return null;
    const containers = scopedContainersForQuery(parsed.itemQuery);
    if (!containers.length || containers[0].score < 0.35) return null;
    for (const containerRank of containers.slice(0, 3)) {{
      const container = containerRank.el;
      const hiddenMenuPlan = hiddenScopedMenuClickPlan(containerRank, parsed);
      if (hiddenMenuPlan) return hiddenMenuPlan;
      const candidates = clickableElements().filter(el => container.contains(el) && el !== container);
      const rankedTargets = best(candidates, el => scoreClickableTarget(parsed.actionHint, el, {{ anchor: container, semantic: true }}));
      if (rankedTargets.length && rankedTargets[0].score >= 0.3) {{
        const chosen = rankedTargets[0];
        return {{
          action: 'click',
          params: {{ selector: selector(chosen.el) }},
          confidence: Math.min(1, (containerRank.score + chosen.score) / 2),
          reason: 'matched child clickable target inside container selected by visible text',
          candidate: candidate(chosen.el),
          evidence: {{
            itemQuery: parsed.itemQuery,
            actionHint: parsed.actionHint,
            container: candidate(container),
            containerScore: containerRank.score,
            targetScore: chosen.score
          }}
        }};
      }}
    }}
    return null;
  }}
	  function scopedCheckedControlIntent() {{
	    if (!kindIs('set_checked', 'select_option')) return null;
	    const namedContainerPatterns = [
	      {{
	        pattern: /\b(?:check|tick|select|choose|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)(?:[,.]|$)/i,
	        control: 1,
	        item: 2
	      }},
	      {{
	        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[,;]?\s+(?:check|tick|select|choose|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i,
	        item: 1,
	        control: 2
	      }}
	    ];
	    for (const entry of namedContainerPatterns) {{
	      const match = instruction.match(entry.pattern);
	      if (!match) continue;
	      const controlHint = cleanCheckedControlHint(match[entry.control]);
	      const itemQuery = cleanContainerNameFragment(match[entry.item]);
	      if (itemQuery) return {{ controlHint, itemQuery }};
	    }}
	    const patterns = [
	      /\b(?:check|tick|select|choose|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i,
      /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)[,;]?\s+(?:check|tick|select|choose|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (!match) continue;
      const inverted = /^\b(?:in|inside|within)\b/i.test(match[0]);
      const controlHint = cleanCheckedControlHint(inverted ? match[2] : match[1]);
      const itemQuery = cleanScopedFragment(inverted ? match[1] : match[2]);
      if (itemQuery) return {{ controlHint, itemQuery }};
    }}
    return null;
  }}
	  function scopedCheckedControlPlan() {{
	    const parsed = scopedCheckedControlIntent();
	    if (!parsed) return null;
	    const controlKind = checkedControlKind();
	    const containers = scopedContainersForQuery(parsed.itemQuery);
	    if (!containers.length || containers[0].score < 0.35) return null;
	    for (const containerRank of containers.slice(0, 3)) {{
	      const container = containerRank.el;
	      const controls = visualOrder(interactive.filter(el =>
	        container.contains(el) &&
	        isCheckedControl(el) &&
	        visible(el) &&
	        (!controlKind || checkedControlMatchesKind(el, controlKind))
	      ));
	      const rankedControls = best(controls, el => {{
	        const text = textOf(el);
	        let score = controlKind ? 0.45 : 0.28;
	        if (parsed.controlHint) {{
	          score += Math.max(tokenScore(parsed.controlHint, text), exactPhraseScore(parsed.controlHint, text), semanticScore(parsed.controlHint, text));
	        }} else {{
          score += 0.15;
        }}
        return score;
      }});
      if (!rankedControls.length || rankedControls[0].score < 0.35) continue;
      const chosen = rankedControls[0].el;
      const primary = {{
        action: 'set_checked',
        params: {{ selector: selector(chosen), checked: checked !== false }},
        confidence: Math.min(1, (containerRank.score + rankedControls[0].score) / 2),
        reason: 'matched checked control inside container selected by visible text',
        candidate: candidate(chosen),
        evidence: {{
	          itemQuery: parsed.itemQuery,
	          controlHint: parsed.controlHint,
	          controlKind: controlKind || null,
	          container: candidate(container),
          containerScore: containerRank.score,
          controlScore: rankedControls[0].score
        }}
      }};
      return withFollowUp(primary, chosen);
    }}
    return null;
  }}
	  function scopedFieldFillIntent() {{
	    if (!kindIs('fill', 'select_option')) return null;
	    const namedContainerPatterns = [
	      {{
	        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[,;]?\s+(?:set|enter|type|fill|input|choose|select)\s+(?:the\s+)?(.+?)\s+(?:to|with|as|=|:)\s*("[^"]+"|'[^']+'|.+?)(?=\s*(?:$|,\s*(?:and|then)\b)|[.]\s*$)/i,
	        item: 1,
	        field: 2,
	        value: 3
	      }},
	      {{
	        pattern: /\b(?:set|enter|type|fill|input|choose|select)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)\s+(?:to|with|as|=|:)\s*("[^"]+"|'[^']+'|.+?)(?=\s*(?:$|,\s*(?:and|then)\b)|[.]\s*$)/i,
	        field: 1,
	        item: 2,
	        value: 3
	      }},
	      {{
	        pattern: /\b(?:choose|select|pick)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:from|in)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)(?:[,.]|$)/i,
	        value: 1,
	        field: 2,
	        item: 3
	      }}
	    ];
	    for (const entry of namedContainerPatterns) {{
	      const match = instruction.match(entry.pattern);
	      if (!match) continue;
	      const fieldHint = cleanFieldPairLabel(match[entry.field]);
	      const itemQuery = cleanContainerNameFragment(match[entry.item]);
	      const value = cleanFieldPairValue(match[entry.value]);
	      if (fieldHint && itemQuery && value) return {{ fieldHint, itemQuery, value }};
	    }}
	    const patterns = [
	      /\b(?:set|enter|type|fill|input|choose|select)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:to|with|as|=|:)\s*("[^"]+"|'[^']+'|.+?)(?=\s*(?:$|,\s*(?:and|then)\b)|[.]\s*$)/i,
      /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)[,;]?\s+(?:set|enter|type|fill|input|choose|select)\s+(?:the\s+)?(.+?)\s+(?:to|with|as|=|:)\s*("[^"]+"|'[^']+'|.+?)(?=\s*(?:$|,\s*(?:and|then)\b)|[.]\s*$)/i,
      /\b(?:choose|select|pick)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:from|in)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (!match) continue;
      const inverted = /^\b(?:in|inside|within)\b/i.test(match[0]);
      const optionSelection = /^\b(?:choose|select|pick)\b/i.test(match[0]) &&
        /\s+(?:from|in)\s+(?:the\s+)?/i.test(match[0]) &&
        !/\s+(?:to|with|as)\s+|=|:/i.test(match[0]);
      const fieldHint = cleanFieldPairLabel(optionSelection ? match[2] : inverted ? match[2] : match[1]);
      const itemQuery = cleanScopedFragment(optionSelection ? match[3] : inverted ? match[1] : match[2]);
      const value = cleanFieldPairValue(optionSelection ? match[1] : match[3]);
      if (fieldHint && itemQuery && value) return {{ fieldHint, itemQuery, value }};
    }}
    return null;
  }}
  function scopedFieldFillPlan() {{
    const parsed = scopedFieldFillIntent();
    if (!parsed) return null;
    const containers = scopedContainersForQuery(parsed.itemQuery);
    if (!containers.length || containers[0].score < 0.35) return null;
    for (const containerRank of containers.slice(0, 3)) {{
      const container = containerRank.el;
      const fields = formFieldCandidates().filter(el => container.contains(el) && actionableValueField(el));
      const rankedFields = best(fields, el => {{
        const text = textOf(el);
        let score = Math.max(tokenScore(parsed.fieldHint, text), exactPhraseScore(parsed.fieldHint, text), semanticScore(parsed.fieldHint, text));
        score += controlTypeScore(parsed.fieldHint, el);
        if (isSelectableField(el)) {{
          score += Math.max(tokenScore(parsed.value, selectableOptionText(el)), exactPhraseScore(parsed.value, selectableOptionText(el))) * 0.35;
          if (/\b(dropdown|select|option|status|type|category|state|country|priority)\b/i.test([parsed.fieldHint, text].join(' '))) score += 0.14;
        }}
        if (writableField(el) && /\b(quantity|qty|count|amount|name|email|phone|address|city|zip|postal|comment|message|title|note)\b/i.test([parsed.fieldHint, text].join(' '))) score += 0.12;
        return score;
      }});
      if (!rankedFields.length || rankedFields[0].score < 0.3) continue;
      const chosen = rankedFields[0].el;
      const selectable = isSelectableField(chosen) && !isFillableField(chosen);
      const primary = valueFieldActionStep(chosen, parsed.value, {{
        selectable,
        confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
        selectReason: 'matched selectable field inside container selected by visible text',
        typeReason: 'matched fillable field inside container selected by visible text',
        sliderReason: 'matched slider or range field inside container selected by visible text',
        evidence: {{
          itemQuery: parsed.itemQuery,
          fieldHint: parsed.fieldHint,
          value: parsed.value,
          container: candidate(container),
          containerScore: containerRank.score,
          fieldScore: rankedFields[0].score
        }}
      }});
      return withFollowUp(primary, chosen);
    }}
    return null;
  }}
  function scopedMultiActionIntent() {{
    const patterns = [
      {{
        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[:,;]?\s+([\s\S]+)$/i,
        item: 1,
        body: 2,
        cleaner: cleanContainerNameFragment
      }},
      {{
        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)[:,;]?\s+([\s\S]+)$/i,
        item: 1,
        body: 2,
        cleaner: cleanScopedFragment
      }},
      {{
        pattern: /^\s*(?:for|on)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)[,;]\s+([\s\S]+)$/i,
        item: 1,
        body: 2,
        cleaner: cleanScopedFragment
      }},
      {{
        pattern: /^\s*(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:row|card|item|record|entry|result)[,;:]\s+([\s\S]+)$/i,
        item: 1,
        body: 2,
        cleaner: cleanScopedFragment
      }},
      {{
        pattern: /^\s*("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s*:\s+([\s\S]+)$/i,
        item: 1,
        body: 2,
        cleaner: cleanContainerNameFragment
      }}
    ];
    let itemQuery = '';
    let body = '';
    for (const entry of patterns) {{
      const match = instruction.match(entry.pattern);
      if (!match) continue;
      itemQuery = entry.cleaner(match[entry.item]);
      body = stripFollowUp(match[entry.body] || '');
      break;
    }}
    function cleanScopedActionClause(clause) {{
      return String(clause || '')
        .replace(/^\s*(?:[-*]|\d+[.)])\s+/, '')
        .trim();
    }}
    function startsActionClause(text) {{
      const trimmed = String(text || '').trim();
      if (!trimmed) return false;
      if (/^(?:click|press|tap|hit|submit|save|continue|confirm|done|ok|next|apply|send)\b/i.test(trimmed)) return true;
      if (/^(?:set|enter|type|fill|input|write|clear|empty|erase|append|add|choose|select|pick|check|tick|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\b/i.test(trimmed)) return true;
      if (/^[A-Za-z][A-Za-z0-9 _/-]{{0,50}}\s*(?:=|:)\s*\S/.test(trimmed)) return true;
      return false;
    }}
    function shouldSplitScopedComma(current, next) {{
      const currentTrimmed = String(current || '').trim();
      const nextTrimmed = String(next || '').trim();
      if (!nextTrimmed) return false;
      if (/^\d{{4}}\b/.test(nextTrimmed) && /\b(?:january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec)\.?\s+\d{{1,2}}(?:st|nd|rd|th)?$/i.test(currentTrimmed)) return false;
      if (startsActionClause(nextTrimmed)) return true;
      if (/[=:]/.test(currentTrimmed)) return false;
      return /^[A-Z][A-Za-z0-9 _/-]{{0,35}}\s+\S/.test(nextTrimmed);
    }}
    function splitScopedCommasSafely(text) {{
      const pieces = String(text || '').split(',');
      const out = [];
      let current = pieces.shift() || '';
      for (const piece of pieces) {{
        if (shouldSplitScopedComma(current, piece)) {{
          out.push(current);
          current = piece;
        }} else {{
          current += ',' + piece;
        }}
      }}
      if (current.trim()) out.push(current);
      return out;
    }}
    function splitScopedActionClauses(text) {{
      return String(text || '')
        .replace(/\r/g, '\n')
        .split(/\n+/)
        .flatMap(line => line.split(/\s*(?:;|\band\b|\bthen\b)\s*/i))
        .flatMap(splitScopedCommasSafely)
        .map(cleanScopedActionClause)
        .filter(Boolean);
    }}
    if (!itemQuery || !/(?:\band\b|\bthen\b|,|;|\n|\r|^\s*(?:[-*]|\d+[.)])\s+)/im.test(body)) return null;
    const clauses = splitScopedActionClauses(body)
      .filter(Boolean);
    if (clauses.length < 2) return null;
    return {{ itemQuery, clauses }};
  }}
  function scopedMultiActionPlan() {{
    const parsed = scopedMultiActionIntent();
    if (!parsed) return null;
    const containers = scopedContainersForQuery(parsed.itemQuery);
    if (!containers.length || containers[0].score < 0.35) return null;
    for (const containerRank of containers.slice(0, 3)) {{
      const container = containerRank.el;
      const steps = [];
      const usedSelectors = new Set();
      let anchor = container;
      function rankedWritableFields(fieldHint, allowUsedSelector = false) {{
        const fields = formFieldCandidates().filter(el =>
          container.contains(el) &&
          (allowUsedSelector || !usedSelectors.has(selector(el))) &&
          writableField(el)
        );
        return best(fields, el => {{
          const text = textOf(el);
          let score = Math.max(tokenScore(fieldHint, text), exactPhraseScore(fieldHint, text), semanticScore(fieldHint, text));
          score += controlTypeScore(fieldHint, el);
          if (/\b(quantity|qty|count|amount|name|email|phone|address|city|zip|postal|comment|message|title|note|notes|summary|description|details?)\b/i.test([fieldHint, text].join(' '))) score += 0.12;
          return score;
        }});
      }}
      function shorthandPair(clause) {{
        const normalized = cleanFieldPairValue(clause);
        const explicit = normalized.match(/^(.+?)\s*(?:=|:)\s*(.+)$/);
        if (explicit) {{
          const fieldHint = cleanFieldPairLabel(explicit[1]);
          const value = cleanFieldPairValue(explicit[2]);
          return fieldHint && value ? {{ fieldHint, value, explicit: true }} : null;
        }}
        const tokens = normalized.split(/\s+/).filter(Boolean);
        if (tokens.length < 2 || tokens.length > 12) return null;
        return {{ tokens, explicit: false }};
      }}
      function scopedClickStepForHint(hint, anchorEl = null) {{
        const cleanedHint = cleanClickHint(cleanFieldPairValue(hint));
        if (!cleanedHint) return null;
        const ranked = best(clickableElements().filter(el => container.contains(el)), el =>
          scoreClickableTarget(cleanedHint, el, {{ anchor: anchorEl || anchor, semantic: true }})
        );
        if (!ranked.length || ranked[0].score < 0.35) return null;
        const target = ranked[0].el;
        return {{
          step: {{
            action: 'click',
            params: {{ selector: selector(target) }},
            confidence: Math.min(1, ranked[0].score),
            reason: 'matched scoped completion or click control inside named container multi-action instruction',
            candidate: candidate(target),
            evidence: {{ itemQuery: parsed.itemQuery, hint: cleanedHint }}
          }},
          el: target
        }};
      }}
      for (const clause of parsed.clauses) {{
        let step = null;
        let chosen = null;
        let allowSelectorReuse = false;
        let match = clause.match(/^(?:set|enter|type|fill|input|write)\s+(?:the\s+)?(.+?)\s+(?:to|with|as|=|:)\s*(.+)$/i);
        if (match) {{
          const fieldHint = cleanFieldPairLabel(match[1]);
          const value = cleanFieldPairValue(match[2]);
          const fields = formFieldCandidates().filter(el =>
            container.contains(el) &&
            !usedSelectors.has(selector(el)) &&
            actionableValueField(el)
          );
          const rankedFields = best(fields, el => {{
            const text = textOf(el);
            let score = Math.max(tokenScore(fieldHint, text), exactPhraseScore(fieldHint, text), semanticScore(fieldHint, text));
            score += controlTypeScore(fieldHint, el);
            if (isSelectableField(el)) {{
              score += Math.max(tokenScore(value, selectableOptionText(el)), exactPhraseScore(value, selectableOptionText(el))) * 0.35;
              if (/\b(dropdown|select|option|status|type|category|state|country|priority)\b/i.test([fieldHint, text].join(' '))) score += 0.14;
            }}
            if (writableField(el) && /\b(quantity|qty|count|amount|name|email|phone|address|city|zip|postal|comment|message|title|note|summary)\b/i.test([fieldHint, text].join(' '))) score += 0.12;
            return score;
          }});
          if (value && rankedFields.length && rankedFields[0].score >= 0.3) {{
            chosen = rankedFields[0].el;
            const selectable = isSelectableField(chosen) && !isFillableField(chosen);
            step = valueFieldActionStep(chosen, value, {{
              selectable,
              confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
              selectReason: 'matched selectable field inside named container multi-action instruction',
              typeReason: 'matched fillable field inside named container multi-action instruction',
              sliderReason: 'matched slider or range field inside named container multi-action instruction',
              evidence: {{ itemQuery: parsed.itemQuery, fieldHint, value, clause }}
            }});
          }}
        }}
        if (!step) {{
          match = clause.match(/^(?:clear|empty|erase)\s+(?:the\s+)?(.+)$/i);
          if (match) {{
            const fieldHint = cleanFieldPairLabel(match[1]);
            const rankedFields = rankedWritableFields(fieldHint, true);
            if (rankedFields.length && rankedFields[0].score >= 0.3) {{
              chosen = rankedFields[0].el;
              allowSelectorReuse = true;
              step = {{
                action: 'type',
                params: {{ selector: selector(chosen), text: '', clear_first: true, slowly: false }},
                confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
                reason: 'matched clearable field inside named container multi-action instruction',
                candidate: candidate(chosen),
                evidence: {{ itemQuery: parsed.itemQuery, fieldHint, mode: 'clear', clause }}
              }};
            }}
          }}
        }}
        if (!step) {{
          match = clause.match(/^(?:append|add)\s+(.+?)\s+(?:to|into)\s+(?:the\s+)?(.+)$/i);
          if (match) {{
            const value = cleanFieldPairValue(match[1]);
            const fieldHint = cleanFieldPairLabel(match[2]);
            const rankedFields = rankedWritableFields(fieldHint, true);
            if (value && rankedFields.length && rankedFields[0].score >= 0.3) {{
              chosen = rankedFields[0].el;
              allowSelectorReuse = true;
              step = {{
                action: 'type',
                params: {{ selector: selector(chosen), text: transformedValue(value), clear_first: false, slowly: true }},
                confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
                reason: 'matched appendable field inside named container multi-action instruction',
                candidate: candidate(chosen),
                evidence: {{ itemQuery: parsed.itemQuery, fieldHint, value, mode: 'append', clause }}
              }};
            }}
          }}
        }}
        if (!step) {{
          match = clause.match(/^(?:choose|select|pick)\s+(.+?)\s+(?:from|in)\s+(?:the\s+)?(.+)$/i);
          if (match) {{
            const value = cleanFieldPairValue(match[1]);
            const fieldHint = cleanFieldPairLabel(match[2]);
            const fields = formFieldCandidates().filter(el =>
              container.contains(el) &&
              !usedSelectors.has(selector(el)) &&
              isSelectableField(el) &&
              actionableValueField(el)
            );
            const rankedFields = best(fields, el => {{
              const text = textOf(el);
              let score = Math.max(tokenScore(fieldHint, text), exactPhraseScore(fieldHint, text), semanticScore(fieldHint, text));
              score += Math.max(tokenScore(value, selectableOptionText(el)), exactPhraseScore(value, selectableOptionText(el))) * 0.35;
              if (/\b(dropdown|select|option|status|type|category|state|country|priority|plan|routing)\b/i.test([fieldHint, text].join(' '))) score += 0.16;
              return score;
            }});
            if (value && rankedFields.length && rankedFields[0].score >= 0.3) {{
              chosen = rankedFields[0].el;
              step = {{
                action: 'select_option',
                params: {{ selector: selector(chosen), option: value }},
                confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
                reason: 'matched selectable field inside named container multi-action instruction',
                candidate: candidate(chosen),
                evidence: {{ itemQuery: parsed.itemQuery, fieldHint, value, clause }}
              }};
            }}
          }}
        }}
        if (!step) {{
          match = clause.match(/^(?:check|tick|select|choose|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable)\s+(?:the\s+)?(.+)$/i);
          if (match) {{
            const controlHint = cleanCheckedControlHint(match[1]);
            const wantChecked = !/^(?:uncheck|untick|deselect|turn\s+off|disable)\b/i.test(clause);
            const controls = visualOrder(interactive.filter(el =>
              container.contains(el) &&
              !usedSelectors.has(selector(el)) &&
              isCheckedControl(el) &&
              visible(el)
            ));
            const rankedControls = best(controls, el => {{
              const text = textOf(el);
              let score = Math.max(tokenScore(controlHint, text), exactPhraseScore(controlHint, text), semanticScore(controlHint, text));
              if (/\b(notify|alert|archive|active|enabled|email|phone|sms|primary|default)\b/i.test([controlHint, text].join(' '))) score += 0.12;
              return score;
            }});
            if (rankedControls.length && rankedControls[0].score >= 0.3) {{
              chosen = rankedControls[0].el;
              step = {{
                action: 'set_checked',
                params: {{ selector: selector(chosen), checked: wantChecked }},
                confidence: Math.min(1, (containerRank.score + rankedControls[0].score) / 2),
                reason: 'matched checked control inside named container multi-action instruction',
                candidate: candidate(chosen),
                evidence: {{ itemQuery: parsed.itemQuery, controlHint, checked: wantChecked, clause }}
              }};
            }}
          }}
        }}
        if (!step) {{
          const normalizedClause = cleanFieldPairValue(clause);
          const explicitClick = normalizedClause.match(/^(?:click|press|tap|hit)\s+(?:the\s+)?(.+)$/i);
          const bareCompletion = normalizedClause.match(/^(submit|save|continue|confirm|done|ok|next|apply|send)$/i);
          if (explicitClick || bareCompletion) {{
            const hint = explicitClick ? explicitClick[1] : bareCompletion[1];
            const scopedClick = scopedClickStepForHint(hint, anchor);
            if (scopedClick) {{
              chosen = scopedClick.el;
              step = scopedClick.step;
            }}
          }}
        }}
        if (!step) {{
          const pair = shorthandPair(clause);
          const stateValue = pair && typeof pair.value === 'string'
            ? pair.value.match(/^(on|off|true|false|checked|unchecked|enabled|disabled|yes|no)$/i)
            : null;
          match = pair && pair.explicit && stateValue
            ? [clause, pair.fieldHint, stateValue[1]]
            : clause.match(/^(.+?)\s+(on|off|true|false|checked|unchecked|enabled|disabled|yes|no)$/i);
          if (match) {{
            const controlHint = cleanCheckedControlHint(match[1]);
            const stateWord = match[2];
            const wantChecked = /^(?:on|true|checked|enabled|yes)$/i.test(stateWord);
            const controls = visualOrder(interactive.filter(el =>
              container.contains(el) &&
              !usedSelectors.has(selector(el)) &&
              isCheckedControl(el) &&
              visible(el)
            ));
            const rankedControls = best(controls, el => {{
              const text = textOf(el);
              let score = Math.max(tokenScore(controlHint, text), exactPhraseScore(controlHint, text), semanticScore(controlHint, text));
              if (/\b(notify|alert|archive|active|enabled|email|phone|sms|primary|default)\b/i.test([controlHint, text].join(' '))) score += 0.12;
              return score;
            }});
            if (rankedControls.length && rankedControls[0].score >= 0.35) {{
              chosen = rankedControls[0].el;
              step = {{
                action: 'set_checked',
                params: {{ selector: selector(chosen), checked: wantChecked }},
                confidence: Math.min(1, (containerRank.score + rankedControls[0].score) / 2),
                reason: 'matched shorthand checked state inside named container multi-action instruction',
                candidate: candidate(chosen),
                evidence: {{ itemQuery: parsed.itemQuery, controlHint, stateWord, checked: wantChecked, clause }}
              }};
            }}
          }}
        }}
        if (!step) {{
          const normalizedClause = cleanFieldPairValue(clause);
          const blockedShorthand = /^(?:click|press|tap|submit|save|continue|confirm|done)\b/i.test(normalizedClause);
          const pair = !blockedShorthand ? shorthandPair(clause) : null;
          if (pair) {{
            const fields = formFieldCandidates().filter(el =>
              container.contains(el) &&
              !usedSelectors.has(selector(el)) &&
              actionableValueField(el)
            );
            let bestField = null;
            const pairCandidates = pair.explicit
              ? [{{ fieldHint: pair.fieldHint, value: pair.value }}]
              : pair.tokens.slice(1).map((_, index) => {{
                  const splitIndex = index + 1;
                  return {{
                    fieldHint: cleanFieldPairLabel(pair.tokens.slice(0, splitIndex).join(' ')),
                    value: cleanFieldPairValue(pair.tokens.slice(splitIndex).join(' '))
                  }};
                }});
            for (const candidatePair of pairCandidates) {{
              const fieldHint = candidatePair.fieldHint;
              const value = candidatePair.value;
              if (!fieldHint || !value) continue;
              for (const field of fields) {{
                const text = textOf(field);
                const selectable = isSelectableField(field) && !isFillableField(field);
                let score = Math.max(tokenScore(fieldHint, text), exactPhraseScore(fieldHint, text), semanticScore(fieldHint, text));
                score += controlTypeScore(fieldHint, field);
                if (selectable) {{
                  const optionsText = selectableOptionText(field);
                  score += Math.max(tokenScore(value, optionsText), exactPhraseScore(value, optionsText)) * 0.45;
                  if (/\b(dropdown|select|option|status|type|category|state|country|priority|plan|routing)\b/i.test([fieldHint, text].join(' '))) score += 0.14;
                }} else if (writableField(field)) {{
                  if (/\b(quantity|qty|count|amount|name|email|phone|address|city|zip|postal|comment|message|title|note|notes|summary|description|details?)\b/i.test([fieldHint, text].join(' '))) score += 0.12;
                }} else if (isFileField(field)) {{
                  if (/\b(file|upload|attach|attachment|document|resume|avatar|photo|image|pdf)\b/i.test([fieldHint, text].join(' '))) score += 0.18;
                }}
                if (!bestField || score > bestField.score) bestField = {{ el: field, score, fieldHint, value, selectable }};
              }}
            }}
            if (bestField && bestField.score >= 0.42) {{
              chosen = bestField.el;
              step = valueFieldActionStep(chosen, bestField.value, {{
                selectable: bestField.selectable,
                confidence: Math.min(1, (containerRank.score + bestField.score) / 2),
                selectReason: 'matched shorthand selectable field value inside named container multi-action instruction',
                typeReason: 'matched shorthand fillable field value inside named container multi-action instruction',
                sliderReason: 'matched shorthand slider or range value inside named container multi-action instruction',
                evidence: {{ itemQuery: parsed.itemQuery, fieldHint: bestField.fieldHint, value: bestField.value, clause }}
              }});
            }}
            if (!step) {{
	            const checkedControls = scope => all('input[type=radio], input[type=checkbox], [role=radio], [role=checkbox], [role=switch], [aria-pressed], [role=menuitemradio], [role=menuitemcheckbox]', scope)
	              .concat(all('*', scope).filter(isCustomCheckableElement))
	              .filter((el, index, arr) => arr.indexOf(el) === index);
	            const controls = visualOrder(checkedControls(container)
	                .filter(el => !usedSelectors.has(selector(el)) && visible(el) && isCheckedControl(el)));
              let bestChoice = null;
              for (const candidatePair of pairCandidates) {{
                const groupHint = cleanFieldPairLabel(candidatePair.fieldHint);
                const option = cleanFieldPairValue(candidatePair.value);
                if (!groupHint || !option) continue;
                for (const control of controls) {{
                  const optionText = checkedControlOptionText(control);
                  const group = checkedControlGroupInfo(control);
                  const optionScore = Math.max(
                    exactPhraseScore(option, optionText),
                    tokenScore(option, optionText),
                    semanticScore(option, optionText)
                  );
                  if (optionScore < 0.5) continue;
                  const groupScore = Math.max(
                    exactPhraseScore(groupHint, group.explicit),
                    tokenScore(groupHint, group.explicit),
                    semanticScore(groupHint, group.explicit),
                    exactPhraseScore(groupHint, group.text) * 0.75,
                    tokenScore(groupHint, group.text) * 0.75,
                    semanticScore(groupHint, group.text) * 0.75
                  );
                  if (groupScore < 0.35) continue;
                  let score = 0.25 + optionScore * 0.65 + groupScore * 0.45;
                  if (checkedControlMatchesKind(control, 'radio')) score += 0.08;
                  if (checkedControlMatchesKind(control, 'checkbox')) score += 0.04;
                  if (!bestChoice || score > bestChoice.score) bestChoice = {{ el: control, score, groupHint, option, group }};
                }}
              }}
              if (bestChoice && bestChoice.score >= 0.75) {{
                chosen = bestChoice.el;
                step = {{
                  action: 'set_checked',
                  params: {{ selector: selector(chosen), checked: true }},
                  confidence: Math.min(1, (containerRank.score + bestChoice.score) / 2),
                  reason: 'matched shorthand grouped choice inside named container multi-action instruction',
                  candidate: candidate(chosen),
                  evidence: {{
                    itemQuery: parsed.itemQuery,
                    groupHint: bestChoice.groupHint,
                    option: bestChoice.option,
                    group: bestChoice.group && bestChoice.group.container ? candidate(bestChoice.group.container) : null,
                    clause
                  }}
                }};
              }}
            }}
          }}
        }}
        if (!step || !chosen) continue;
        if (!allowSelectorReuse) usedSelectors.add(selector(chosen));
        anchor = chosen;
        steps.push(step);
      }}
      const follow = clickStepForHint(followUpClickHint(), anchor);
      if (follow) steps.push(follow);
      if (steps.length >= 2) {{
        return {{
          ok: true,
          action: 'sequence',
          steps,
          confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
          reason: 'planned multiple field/control actions inside one named container',
          evidence: {{ itemQuery: parsed.itemQuery, clauseCount: parsed.clauses.length, stepCount: steps.length, container: candidate(container) }}
        }};
      }}
    }}
    return null;
  }}
  function scopedFieldEditIntent() {{
    if (!kindIs('clear_field', 'append_field')) return null;
    const namedContainerPatterns = [
      {{
        mode: 'clear',
        pattern: /\b(?:clear|empty|erase)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)(?:[,.]|$)/i,
        field: 1,
        item: 2
      }},
      {{
        mode: 'clear',
        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[,;]?\s+(?:clear|empty|erase)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i,
        item: 1,
        field: 2
      }},
      {{
        mode: 'append',
        pattern: /\b(?:append|add)\s+("[^"]+"|'[^']+'|[^,]+?)\s+(?:to|into)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)(?:[,.]|$)/i,
        value: 1,
        field: 2,
        item: 3
      }},
      {{
        mode: 'append',
        pattern: /\b(?:in|inside|within)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{1,80}}?)\s+(?:section|panel|region|group|fieldset|form|area)[,;]?\s+(?:append|add)\s+("[^"]+"|'[^']+'|[^,]+?)\s+(?:to|into)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i,
        item: 1,
        value: 2,
        field: 3
      }}
    ];
    for (const entry of namedContainerPatterns) {{
      const match = instruction.match(entry.pattern);
      if (!match) continue;
      const fieldHint = cleanFieldPairLabel(match[entry.field]);
      const itemQuery = cleanContainerNameFragment(match[entry.item]);
      if (entry.mode === 'clear') {{
        if (fieldHint && itemQuery) return {{ mode: entry.mode, fieldHint, itemQuery, value: '' }};
      }} else {{
        const value = cleanFieldPairValue(match[entry.value]);
        if (fieldHint && itemQuery && value) return {{ mode: entry.mode, fieldHint, itemQuery, value }};
      }}
    }}
    const patterns = [
      {{ mode: 'clear', inverted: false, pattern: /\b(?:clear|empty|erase)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i }},
      {{ mode: 'clear', inverted: true, pattern: /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)[,;]?\s+(?:clear|empty|erase)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i }},
      {{ mode: 'append', inverted: false, pattern: /\b(?:append|add)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:to|into)\s+(?:the\s+)?(.+?)\s+(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)(?:[,.]|$)/i }},
      {{ mode: 'append', inverted: true, pattern: /\b(?:in|inside|within)\s+(?:the\s+)?(?:row|card|item|record|entry|result|section|panel)\s+(?:containing|with|for|named|called)\s+("[^"]+"|'[^']+'|[^,.]+?)[,;]?\s+(?:append|add)\s+("[^"]+"|'[^']+'|[^,.]+?)\s+(?:to|into)\s+(?:the\s+)?(.+?)(?:[,.]|$)/i }}
    ];
    for (const entry of patterns) {{
      const match = instruction.match(entry.pattern);
      if (!match) continue;
      if (entry.mode === 'clear') {{
        const fieldHint = cleanFieldPairLabel(entry.inverted ? match[2] : match[1]);
        const itemQuery = cleanScopedFragment(entry.inverted ? match[1] : match[2]);
        if (fieldHint && itemQuery) return {{ mode: entry.mode, fieldHint, itemQuery, value: '' }};
      }} else {{
        const value = cleanFieldPairValue(entry.inverted ? match[2] : match[1]);
        const fieldHint = cleanFieldPairLabel(entry.inverted ? match[3] : match[2]);
        const itemQuery = cleanScopedFragment(entry.inverted ? match[1] : match[3]);
        if (fieldHint && itemQuery && value) return {{ mode: entry.mode, fieldHint, itemQuery, value }};
      }}
    }}
    return null;
  }}
  function scopedFieldEditPlan() {{
    const parsed = scopedFieldEditIntent();
    if (!parsed) return null;
    const containers = scopedContainersForQuery(parsed.itemQuery);
    if (!containers.length || containers[0].score < 0.35) return null;
    for (const containerRank of containers.slice(0, 3)) {{
      const container = containerRank.el;
      const fields = formFieldCandidates().filter(el => container.contains(el) && writableField(el));
      const rankedFields = best(fields, el => {{
        const text = textOf(el);
        let score = Math.max(tokenScore(parsed.fieldHint, text), exactPhraseScore(parsed.fieldHint, text), semanticScore(parsed.fieldHint, text));
        score += controlTypeScore(parsed.fieldHint, el);
        if (/\b(notes?|comment|message|text|field|input|description|details?)\b/i.test([parsed.fieldHint, text].join(' '))) score += 0.12;
        return score;
      }});
      if (!rankedFields.length || rankedFields[0].score < 0.3) continue;
      const chosen = rankedFields[0].el;
      return {{
        action: 'type',
        params: {{
          selector: selector(chosen),
          text: transformedValue(parsed.value),
          clear_first: parsed.mode === 'clear',
          slowly: parsed.mode === 'append'
        }},
        confidence: Math.min(1, (containerRank.score + rankedFields[0].score) / 2),
        reason: parsed.mode === 'clear'
          ? 'matched clearable field inside container selected by visible text'
          : 'matched appendable field inside container selected by visible text',
        candidate: candidate(chosen),
        evidence: {{
          itemQuery: parsed.itemQuery,
          fieldHint: parsed.fieldHint,
          mode: parsed.mode,
          value: parsed.value,
          container: candidate(container),
          containerScore: containerRank.score,
          fieldScore: rankedFields[0].score
        }}
      }};
    }}
    return null;
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
    if (intent && intent.wantsOrderedValues !== true) return null;
    if (!intent && !/\b(click|press|tap|hit)\b/i.test(instruction)) return null;
    if (!intent && !/\b(numbers?|values?|items?)\b/i.test(instruction)) return null;
    const explicitOrder = intent && intent.order ? intent.order :
      /\bdescending|decreasing|reverse\b/i.test(instruction) ? 'descending' :
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
    const valueMatch =
      instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(-?\d+(?:\.\d+)?)\s+(?:with|on|using|in|into)\s+(?:the\s+)?(?:[\w-]+\s+){{0,6}}(?:slider|range)\b/i) ||
      instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(?:the\s+)?(?:[\w-]+\s+){{0,6}}(?:slider|range)\s+(?:to|at|as)\s+(-?\d+(?:\.\d+)?)\b/i);
    if (!valueMatch) return null;
    const desired = Number(valueMatch[1]);
    const sliders = sliderControls();
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
    const sliders = sliderControls();
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
  function checkboxRowsFor(cells) {{
    const measured = cells.map(el => {{
      const rect = el.getBoundingClientRect();
      return {{ el, rect, centerY: rect.top + rect.height / 2, centerX: rect.left + rect.width / 2 }};
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
  function patternTargetDigit() {{
    for (const source of [wantedValue, targetHint, instruction]) {{
      const match = String(source || '').match(/\b\d\b/);
      if (match) return match[0];
    }}
    return null;
  }}
	  function checkboxGridPatternPlan() {{
	    const mentionsRender = kind === 'render_pattern' || /\b(draw|render|make|copy|fill)\b/i.test(instruction);
	    const mentionsGrid = /\bcheckbox(?:es)?|check\s*boxes|grid|pattern\b/i.test(instruction);
	    const mentionsPattern = /\bnumber|digit|pattern|shape\b/i.test(instruction);
	    if (!mentionsRender || !mentionsGrid || !mentionsPattern) return null;
    const digit = patternTargetDigit();
    if (!digit) return null;
    const containers = all('form, fieldset, section, article, main, div, table, tbody, ul, ol, body')
      .filter(el => visible(el) || el === document.body);
	    const scored = [];
	    const seen = new Set();
	    const checkboxCells = scope => all('input[type=checkbox], [role=checkbox]', scope)
	      .concat(all('*', scope).filter(isCustomCheckableElement))
	      .filter((el, index, arr) => arr.indexOf(el) === index);
	    for (const container of containers) {{
      const key = selector(container);
      if (seen.has(key)) continue;
      seen.add(key);
	      const boxes = checkboxCells(container).filter(visible);
	      if (boxes.length < 4) continue;
	      const childWithSameBoxes = Array.from(container.children || []).some(child => {{
	        const childBoxes = checkboxCells(child).filter(visible);
	        return childBoxes.length === boxes.length;
	      }});
      if (childWithSameBoxes && container !== document.body) continue;
      const rows = checkboxRowsFor(boxes);
      if (rows.length < 2) continue;
      const cols = Math.max(...rows.map(row => row.length));
      const consistentRows = rows.every(row => row.length === cols);
      if (!consistentRows || cols < 2) continue;
      let score = 0.45;
      if (rows.length === 7 && cols === 4) score += 0.45;
      if (/\b(grid|pattern|checkbox|check)\b/i.test(textOf(container))) score += 0.15;
      if (container.id || container.getAttribute('data-testid')) score += 0.05;
      if (container.tagName.toLowerCase() === 'body') score -= 0.2;
      scored.push({{ el: container, rows, cols, boxes, score }});
    }}
    scored.sort((a, b) => b.score - a.score || a.boxes.length - b.boxes.length);
    if (!scored.length || scored[0].score < 0.55) return null;
    const chosen = scored[0];
    if (chosen.rows.length !== 7 || chosen.cols !== 4) return null;
    const primary = {{
      ok: true,
      action: 'set_checkbox_grid',
      params: {{ selector: selector(chosen.el), value: digit }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched requested digit or pattern to visible checkbox grid geometry',
      candidate: candidate(chosen.el),
      evidence: {{ rows: chosen.rows.length, cols: chosen.cols, checkboxCount: chosen.boxes.length, target: digit }}
    }};
    const follow = clickStepForHint(followUpClickHint() || (/\bsubmit\b/i.test(instruction) ? 'submit' : null), chosen.el)
      || (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(chosen.el) : null);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned checkbox grid pattern render plus completion control'
    }};
  }}
  function coordinateTarget() {{
    const match = instruction.match(/\((-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\)/);
    if (!match) return null;
    return {{ x: Number(match[1]), y: Number(match[2]), text: '(' + match[1] + ',' + match[2] + ')' }};
  }}
  function coordinateTextOf(el) {{
    return [
      el.id || '',
      el.getAttribute('data-coordinate') || '',
      el.getAttribute('data-coord') || '',
      el.getAttribute('data-point') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.textContent || '',
      (el.hasAttribute('data-x') && el.hasAttribute('data-y')) ? '(' + el.getAttribute('data-x') + ',' + el.getAttribute('data-y') + ')' : '',
    ].join(' ');
  }}
  function pointCenter(el) {{
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 || rect.height > 0) {{
      return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }};
    }}
    const svg = el.ownerSVGElement || el.closest('svg');
    if (svg && el.hasAttribute('cx') && el.hasAttribute('cy')) {{
      const point = svg.createSVGPoint();
      point.x = Number(el.getAttribute('cx'));
      point.y = Number(el.getAttribute('cy'));
      const screen = point.matrixTransform(el.getScreenCTM());
      return {{ x: screen.x, y: screen.y }};
    }}
    return null;
  }}
  function visibleGridPoint(el) {{
    if (visible(el)) return true;
    const svg = el && (el.ownerSVGElement || el.closest && el.closest('svg'));
    return !!(svg && visible(svg) && (
      (el.hasAttribute('cx') && el.hasAttribute('cy')) ||
      (el.hasAttribute('x') && el.hasAttribute('y')) ||
      el.hasAttribute('d')
    ));
  }}
  function clusterPositions(values) {{
    const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
    const clusters = [];
    for (const value of sorted) {{
      const last = clusters[clusters.length - 1];
      if (!last || Math.abs(last.center - value) > 6) {{
        clusters.push({{ center: value, values: [value] }});
      }} else {{
        last.values.push(value);
        last.center = last.values.reduce((sum, item) => sum + item, 0) / last.values.length;
      }}
    }}
    return clusters.map(cluster => cluster.center);
  }}
  function nearestPositionIndex(positions, value) {{
    let bestIndex = -1;
    let bestDistance = Infinity;
    positions.forEach((position, index) => {{
      const distance = Math.abs(position - value);
      if (distance < bestDistance) {{
        bestDistance = distance;
        bestIndex = index;
      }}
    }});
    return bestDistance <= 8 ? bestIndex : -1;
  }}
  function coordinateGridPlan() {{
    if (kind !== 'click' || !/\b(?:grid\s+)?coordinate\b/i.test(instruction)) return null;
    const coord = coordinateTarget();
    if (!coord || !Number.isFinite(coord.x) || !Number.isFinite(coord.y)) return null;
    const wanted = normalized(coord.text);
    const svgs = all('svg').filter(visible);
    for (const svg of svgs) {{
      try {{
        const points = Array.from(svg.querySelectorAll('circle, rect, ellipse'))
          .filter(visibleGridPoint)
          .map(el => {{
            const localX = Number(el.getAttribute('cx') ?? el.getAttribute('x'));
            const localY = Number(el.getAttribute('cy') ?? el.getAttribute('y'));
            const center = pointCenter(el);
            return Number.isFinite(localX) && Number.isFinite(localY) && center ? {{ el, localX, localY, center }} : null;
          }})
          .filter(Boolean);
        if (points.length < 9) continue;
        const xs = clusterPositions(points.map(point => point.localX));
        const ys = clusterPositions(points.map(point => point.localY));
        if (xs.length < 3 || ys.length < 3 || xs.length % 2 === 0 || ys.length % 2 === 0) continue;
        if (!Number.isInteger(coord.x) || !Number.isInteger(coord.y)) continue;
        const xIndex = coord.x + Math.floor(xs.length / 2);
        const yIndex = Math.floor(ys.length / 2) - coord.y;
        if (xIndex < 0 || yIndex < 0 || xIndex >= xs.length || yIndex >= ys.length) continue;
        const wantedX = xs[xIndex];
        const wantedY = ys[yIndex];
        const chosen = points
          .map(point => {{
            const distance = Math.hypot(point.localX - wantedX, point.localY - wantedY);
            return {{ ...point, distance }};
          }})
          .sort((a, b) => a.distance - b.distance)[0];
        if (!chosen || chosen.distance > 8) continue;
        return {{
          ok: true,
          action: 'click',
          params: {{ x: chosen.center.x, y: chosen.center.y }},
          confidence: 0.82,
          reason: 'mapped requested Cartesian coordinate onto regular visible SVG grid geometry',
          candidate: candidate(chosen.el),
          evidence: {{
            coordinate: coord,
            xPositions: xs.length,
            yPositions: ys.length,
            click: {{ x: Math.round(chosen.center.x), y: Math.round(chosen.center.y) }}
          }}
        }};
      }} catch (_) {{}}
    }}
    const explicitTargets = all('svg circle, svg rect, svg path, svg ellipse, svg text, [data-coordinate], [data-coord], [data-point], [data-x][data-y], [aria-label], [title], [id]').filter(el => {{
      if (!visibleGridPoint(el)) return false;
      const tag = el.tagName.toLowerCase();
      if (tag === 'html' || tag === 'body' || tag === 'script' || tag === 'style') return false;
      return el.closest('svg') || el.hasAttribute('data-coordinate') || el.hasAttribute('data-coord') || el.hasAttribute('data-point') || el.hasAttribute('data-x') || /\(-?\d/.test(el.id || '');
    }}).map(el => {{
      const text = normalized(coordinateTextOf(el));
      let score = 0;
      if (text === wanted) score += 1;
      if (text.includes(wanted)) score += 0.9;
      const dataX = Number(el.getAttribute('data-x'));
      const dataY = Number(el.getAttribute('data-y'));
      if (Number.isFinite(dataX) && Number.isFinite(dataY) && dataX === coord.x && dataY === coord.y) score += 1;
      if (el.closest('svg')) score += 0.15;
      if (el.tagName.toLowerCase() === 'circle') score += 0.15;
      return {{ el, score }};
    }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
    if (explicitTargets.length) {{
      const chosen = explicitTargets[0];
      const center = pointCenter(chosen.el);
      return {{
        ok: true,
        action: 'click',
        params: center ? {{ x: center.x, y: center.y }} : {{ selector: selector(chosen.el) }},
        confidence: Math.min(1, chosen.score),
        reason: 'matched requested coordinate to visible element coordinate metadata',
        candidate: candidate(chosen.el),
        evidence: {{ coordinate: coord }}
      }};
    }}
    for (const svg of svgs) {{
      const points = Array.from(svg.querySelectorAll('circle, rect, ellipse, [role=gridcell], [data-point]'))
        .filter(visibleGridPoint)
        .map(el => {{
          const center = pointCenter(el);
          return center ? {{ el, center }} : null;
        }})
        .filter(Boolean);
      if (points.length < 9) continue;
      const xs = clusterPositions(points.map(point => point.center.x));
      const ys = clusterPositions(points.map(point => point.center.y));
      if (xs.length < 3 || ys.length < 3 || xs.length % 2 === 0 || ys.length % 2 === 0) continue;
      if (!Number.isInteger(coord.x) || !Number.isInteger(coord.y)) continue;
      const xIndex = coord.x + Math.floor(xs.length / 2);
      const yIndex = Math.floor(ys.length / 2) - coord.y;
      if (xIndex < 0 || yIndex < 0 || xIndex >= xs.length || yIndex >= ys.length) continue;
      const wantedX = xs[xIndex];
      const wantedY = ys[yIndex];
      const chosen = points
        .map(point => {{
          const distance = Math.hypot(point.center.x - wantedX, point.center.y - wantedY);
          return {{ ...point, distance }};
        }})
        .sort((a, b) => a.distance - b.distance)[0];
      if (!chosen || chosen.distance > 10) continue;
      const center = pointCenter(chosen.el) || chosen.center;
      return {{
        ok: true,
        action: 'click',
        params: {{ x: center.x, y: center.y }},
        confidence: 0.78,
        reason: 'mapped requested Cartesian coordinate onto regular visible SVG grid geometry',
        candidate: candidate(chosen.el),
        evidence: {{
          coordinate: coord,
          xPositions: xs.length,
          yPositions: ys.length,
          click: {{ x: Math.round(center.x), y: Math.round(center.y) }}
        }}
      }};
    }}
    return null;
  }}
  function spinbuttonPlan() {{
    const valueMatch =
      instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(-?\d+(?:\.\d+)?)\s+(?:with|on|using|in|into)\s+(?:the\s+)?(?:[\w-]+\s+){{0,6}}(?:spinner|spinbutton|stepper|number|numeric)\b/i) ||
      instruction.match(/\b(?:select|set|choose|move|use|enter|input)\s+(?:the\s+)?(?:[\w-]+\s+){{0,6}}(?:spinner|spinbutton|stepper|number|numeric)\s+(?:to|at|as)\s+(-?\d+(?:\.\d+)?)\b/i);
    if (!valueMatch) return null;
    const desired = valueMatch[1];
    const fields = Array.from(new Set([
      ...all('input[type=number], [role=spinbutton], [class*=spinner], [class*=spinbutton], [class*=stepper], [data-field*=number], [data-field*=numeric], [data-field*=stepper], [data-control*=number], [data-control*=numeric], [data-control*=stepper]'),
      ...all('*').filter(isCustomNumericControl)
    ])).filter(writableField);
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
  function listSelectionPlan() {{
    if (kind !== 'select_option' || !wantedValue) return null;
    const listIntent = /\b(scroll\s+list|listbox|list|tree|treeitem|multi-?select|select)\b/i.test([instruction, targetHint || ''].join(' '));
    if (!listIntent) return null;
    const items = requestedItems(wantedValue);
    if (!items.length) return null;
    const selects = best(interactive.filter(el => {{
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      return tag === 'select' || role === 'listbox' || role === 'tree';
    }}), el => {{
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const optionsText = Array.from(el.options || el.querySelectorAll('[role=option], [role=treeitem], option'))
        .map(option => textOf(option))
        .join(' ');
      let score = 0;
      if (tag === 'select') score += 0.45;
      if (role === 'listbox') score += 0.35;
      if (role === 'tree') score += 0.35;
      if (el.multiple || el.hasAttribute('multiple') || el.getAttribute('aria-multiselectable') === 'true') score += 0.35;
      if (targetHint) score += tokenScore(targetHint, textOf(el)) * 0.5;
      const matches = items.filter(item => Math.max(tokenScore(item, optionsText), exactPhraseScore(item, optionsText)) > 0).length;
      score += matches / items.length;
      if (/\bscroll\b/i.test(instruction) && el.scrollHeight > el.clientHeight + 4) score += 0.25;
      return matches === items.length ? score : 0;
    }});
    if (!selects.length) return null;
    const chosen = selects[0];
    return withFollowUp({{
      ok: true,
      action: 'select_option',
      params: {{ selector: selector(chosen.el), option: items.length > 1 ? items : items[0] }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched list or multi-select options by requested item text, including off-view options',
      candidate: candidate(chosen.el),
      evidence: {{ requestedItems: items }}
    }}, chosen.el);
  }}
  function tabSelectionHints() {{
    const hints = [];
    const add = value => {{
      const cleaned = String(value || '')
        .replace(/\b(?:tabs?|tabbed|tab\s*panel|tabpanel|panels?|sections?|page)\b/ig, ' ')
        .replace(/\b(?:the|a|an|to|into|open|switch|select|choose|click|press|tap|go|navigate)\b/ig, ' ')
        .replace(/\s+/g, ' ')
        .trim();
      if (cleaned && !hints.some(existing => normalized(existing) === normalized(cleaned))) hints.push(cleaned);
    }};
    for (const quoted of quotedInstructionValues()) add(quoted);
    add(targetHint || '');
    const patterns = [
      /\b(?:switch|change|go|navigate)\s+(?:to|into|over\s+to)?\s+(?:the\s+)?["']?(.+?)["']?\s+tabs?\b/i,
      /\b(?:open|select|choose|click|press|tap)\s+(?:the\s+)?["']?(.+?)["']?\s+tabs?\b/i,
      /\btabs?\s+(?:called|named|labeled|labelled)\s+["']?(.+?)["']?(?:[.,;:]|$|\band\b)/i,
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (match) add(match[1]);
    }}
    return hints;
  }}
  function tabPanelFor(tab, clickTarget) {{
    const ids = [];
    const collect = value => {{
      const raw = String(value || '').trim();
      if (!raw) return;
      for (const part of raw.split(/\s+/)) ids.push(part.replace(/^#/, ''));
    }};
    collect(tab.getAttribute('aria-controls'));
    collect(clickTarget && clickTarget.getAttribute('aria-controls'));
    collect(tab.getAttribute('data-target') || tab.getAttribute('data-bs-target'));
    collect(clickTarget && (clickTarget.getAttribute('data-target') || clickTarget.getAttribute('data-bs-target')));
    const href = clickTarget && clickTarget.getAttribute('href') || tab.getAttribute('href');
    if (href && href.startsWith('#')) collect(href);
    const rootNode = tab.getRootNode && tab.getRootNode();
    for (const id of ids) {{
      if (!id) continue;
      let panel = null;
      if (rootNode && rootNode.getElementById) panel = rootNode.getElementById(id);
      if (!panel) panel = document.getElementById(id);
      if (panel) return panel;
    }}
    return null;
  }}
  function tabClickTarget(tab) {{
    const tag = tab.tagName.toLowerCase();
    if (tag === 'a' || tag === 'button' || tab.hasAttribute('onclick')) return tab;
    const child = Array.from(tab.querySelectorAll ? tab.querySelectorAll('a, button, [role=tab], [data-toggle=tab], [data-bs-toggle=tab], [onclick], [tabindex]') : [])
      .find(el => visible(el));
    return child || tab;
  }}
  function tabElements() {{
    const candidates = all('[role=tab], .ui-tabs-tab, .ui-tabs-anchor, [data-toggle=tab], [data-bs-toggle=tab], [aria-controls][class*=tab], [aria-controls][class*=Tab]');
    return Array.from(new Set(candidates)).filter(el => {{
      if (unavailableForAction(el)) return false;
      const clickTarget = tabClickTarget(el);
      return visible(el) || visible(clickTarget);
    }});
  }}
  function tabSelectionPlan() {{
    if (!/\b(?:tabs?|tabbed|tab\s*panel|tabpanel)\b/i.test(instruction)) return null;
    if (/\bswitch\s+between\s+(?:the\s+)?tabs?\b/i.test(instruction) && /\bfind\b/i.test(instruction)) return null;
    const hints = tabSelectionHints();
    if (!hints.length) return null;
    const tabs = best(tabElements(), tab => {{
      const clickTarget = tabClickTarget(tab);
      const panel = tabPanelFor(tab, clickTarget);
      const text = [
        textOf(tab),
        clickTarget && clickTarget !== tab ? textOf(clickTarget) : '',
        tab.getAttribute('aria-controls') || '',
        clickTarget && clickTarget.getAttribute('href') || '',
        panel ? textOf(panel).slice(0, 200) : ''
      ].join(' ');
      let score = 0;
      for (const hint of hints) {{
        score = Math.max(score, tokenScore(hint, text), exactPhraseScore(hint, text), semanticScore(hint, text));
      }}
      const role = roleOf(tab) || roleOf(clickTarget);
      if (role === 'tab') score += 0.2;
      if (tab.closest && tab.closest('[role=tablist], .ui-tabs-nav, [class*=tabs], [class*=Tabs]')) score += 0.15;
      if (panel) score += 0.1;
      if (tab.getAttribute('aria-selected') === 'true' || /\b(?:active|selected|current)\b/i.test(classText(tab))) score -= 0.05;
      return score;
    }});
    if (!tabs.length || tabs[0].score < 0.45) return null;
    const chosen = tabs[0].el;
    const clickTarget = tabClickTarget(chosen);
    return withFollowUp({{
      ok: true,
      action: 'click',
      params: {{ selector: selector(clickTarget) }},
      confidence: Math.min(0.95, tabs[0].score),
      reason: 'matched tab control by accessible label and tab panel linkage',
      candidate: candidate(chosen),
      evidence: {{ requestedTabs: hints, clickTarget: selector(clickTarget) }}
    }}, chosen);
  }}
  function quotedConstraint(name) {{
    const patterns = name === 'starts'
      ? [/\bstarts\s+with\s+"([^"]+)"/i, /\bstarts\s+with\s+'([^']+)'/i]
      : [/\bends\s+with\s+"([^"]+)"/i, /\bends\s+with\s+'([^']+)'/i];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (match) return match[1].trim();
    }}
    return null;
  }}
	  function completionClickStep(anchor = null) {{
	    const ranked = best(clickableElements(), el => {{
	      const text = textOf(el);
	      let score = submitLikeScore('submit', el, text);
	      score += relationScore(el, anchor);
      const form = anchor && anchor.closest ? anchor.closest('form') : null;
      if (form && form.contains(el)) score += 0.25;
      return score;
    }});
    if (!ranked.length || ranked[0].score < 0.35) return null;
    return {{
      action: 'click',
      params: {{ selector: selector(ranked[0].el) }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'matched nearby submit-like completion control after dynamic selection',
      candidate: candidate(ranked[0].el)
	    }};
	  }}
	  function pad2(value) {{
	    const number = Number(value);
	    return Number.isFinite(number) && number < 10 ? '0' + number : String(value);
	  }}
	  function requestedDate() {{
	    const source = [wantedValue || '', targetHint || '', instruction || ''].join(' ');
	    let match = source.match(/\b(\d{{1,2}})\/(\d{{1,2}})\/(\d{{4}})\b/);
	    if (match) {{
	      const month = Number(match[1]);
	      const day = Number(match[2]);
	      const year = Number(match[3]);
	      if (month >= 1 && month <= 12 && day >= 1 && day <= 31) return dateParts(year, month, day);
	    }}
	    match = source.match(/\b(\d{{4}})-(\d{{1,2}})-(\d{{1,2}})\b/);
	    if (match) {{
	      const year = Number(match[1]);
	      const month = Number(match[2]);
	      const day = Number(match[3]);
	      if (month >= 1 && month <= 12 && day >= 1 && day <= 31) return dateParts(year, month, day);
	    }}
	    const monthNames = 'january february march april may june july august september october november december'.split(' ');
	    match = source.match(/\b(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec)\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:,)?\s+(\d{{4}})\b/i);
	    if (match) {{
	      const raw = match[1].toLowerCase().slice(0, 3);
	      const month = monthNames.findIndex(name => name.slice(0, 3) === raw) + 1;
	      const day = Number(match[2]);
	      const year = Number(match[3]);
	      if (month >= 1 && day >= 1 && day <= 31) return dateParts(year, month, day);
	    }}
	    return null;
	  }}
	  function dateParts(year, month, day) {{
	    const monthNames = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
	    return {{
	      year,
	      month,
	      day,
	      dayText: String(day),
	      dayPadded: pad2(day),
	      monthText: String(month),
	      monthPadded: pad2(month),
	      monthName: monthNames[month - 1],
	      monthShort: monthNames[month - 1].slice(0, 3),
	      iso: String(year) + '-' + pad2(month) + '-' + pad2(day),
	      slash: pad2(month) + '/' + pad2(day) + '/' + String(year)
	    }};
	  }}
	  function elementDateText(el) {{
	    return [
	      el.getAttribute('data-date') || '',
	      el.getAttribute('data-day') || '',
	      el.getAttribute('data-value') || '',
	      el.getAttribute('datetime') || '',
	      el.getAttribute('aria-label') || '',
	      el.getAttribute('title') || '',
	      el.getAttribute('value') || '',
	      directTextOf(el),
	      textOf(el)
	    ].join(' ').replace(/\s+/g, ' ').trim();
	  }}
	  function dateContextText(el) {{
	    const parts = [];
	    const container = el && el.closest && el.closest('[role=dialog], [role=grid], .ui-datepicker, .datepicker, .calendar, [class*=datepicker], [class*=Datepicker], [class*=calendar], [class*=Calendar]');
	    if (container) parts.push(container.textContent || '');
	    let node = el;
	    for (let depth = 0; node && depth < 5; depth += 1, node = node.parentElement) {{
	      parts.push(directTextOf(node));
	      parts.push(node.getAttribute && (node.getAttribute('aria-label') || node.getAttribute('title') || semanticAttributeText(node)));
	    }}
	    return parts.filter(Boolean).join(' ');
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
	  function hasExactDayText(el, date) {{
	    if (!el) return false;
	    const checks = [
	      ownTextOnly(el),
	      el.getAttribute('data-day') || '',
	      el.getAttribute('aria-label') || '',
	      el.getAttribute('title') || '',
	    ];
	    for (const text of checks) {{
	      const number = exactDayNumber(text);
	      if (number === Number(date.day)) return true;
	    }}
	    const controls = Array.from(el.querySelectorAll ? el.querySelectorAll('button, a, [role=button], [role=link], [role=gridcell], [role=option]') : []);
	    return controls.some(child => visible(child) && exactDayNumber(ownTextOnly(child) || directTextOf(child)) === Number(date.day));
	  }}
	  function hasDateMetadata(el, date) {{
	    if (!el) return false;
	    const values = [
	      el.getAttribute('data-date') || '',
	      el.getAttribute('datetime') || '',
	      el.getAttribute('data-value') || '',
	      el.getAttribute('aria-label') || '',
	      el.getAttribute('title') || '',
	      el.getAttribute('value') || '',
	    ].join(' ').toLowerCase();
	    if (values.includes(date.iso.toLowerCase()) || values.includes(date.slash.toLowerCase())) return true;
	    if (values.includes(date.monthName.toLowerCase()) && values.includes(date.dayText) && values.includes(String(date.year))) return true;
	    if (values.includes(date.monthShort.toLowerCase()) && values.includes(date.dayText) && values.includes(String(date.year))) return true;
	    const dataMonth = Number(el.getAttribute('data-month'));
	    const dataYear = Number(el.getAttribute('data-year'));
	    const dataDay = Number(el.getAttribute('data-day'));
	    return dataMonth === date.month && dataYear === date.year && dataDay === date.day;
	  }}
	  function dateControlDescendantCount(el) {{
	    if (!el || !el.querySelectorAll) return 0;
	    return Array.from(el.querySelectorAll('[data-date], [data-day], [datetime], [role=gridcell], [role=option], button, a, .day, .date, .calendar-day'))
	      .filter(child => visible(child))
	      .length;
	  }}
	  function isCalendarContainer(el) {{
	    if (!el) return false;
	    const role = roleOf(el);
	    const tag = el.tagName.toLowerCase();
	    const label = [classText(el), el.id || '', el.getAttribute('aria-label') || '', role].join(' ');
	    if (/\b(?:dialog|grid)\b/i.test(role)) return true;
	    if (/\b(?:ui-datepicker|datepicker|calendar|date-picker)\b/i.test(label) && tag !== 'button' && tag !== 'a') return true;
	    return dateControlDescendantCount(el) >= 8;
	  }}
	  function dateCellClickTarget(el, date) {{
	    if (!el) return null;
	    const tag = el.tagName.toLowerCase();
	    const role = roleOf(el);
	    if (['button', 'a'].includes(tag) || ['button', 'link', 'gridcell', 'option'].includes(role) || hasDateMetadata(el, date)) return el;
	    if (tag === 'td' || /\b(?:day|date|calendar-day)\b/i.test(classText(el))) {{
	      const controls = Array.from(el.querySelectorAll ? el.querySelectorAll('button, a, [role=button], [role=link], [role=gridcell], [role=option], [data-date], [data-day], [datetime]') : []);
	      const exact = controls.find(child => visible(child) && (hasDateMetadata(child, date) || hasExactDayText(child, date)));
	      if (exact) return exact;
	      if (hasExactDayText(el, date) || hasDateMetadata(el, date)) return el;
	    }}
	    return null;
	  }}
	  function isLikelyDateCell(el, date) {{
	    const target = dateCellClickTarget(el, date);
	    if (!target) return false;
	    if (isCalendarContainer(target) && !hasDateMetadata(target, date) && !hasExactDayText(target, date)) return false;
	    if (isCalendarContainer(el) && el !== target) return true;
	    if (isCalendarContainer(el) && !hasDateMetadata(el, date)) return false;
	    return hasDateMetadata(target, date) || hasExactDayText(target, date);
	  }}
	  function dateCellScore(el, date) {{
	    if (!isLikelyDateCell(el, date)) return -1;
	    const direct = directTextOf(el).trim();
	    const text = elementDateText(el);
	    const lowerText = text.toLowerCase();
	    const context = dateContextText(el).toLowerCase();
	    let score = 0;
	    if (lowerText.includes(date.iso.toLowerCase())) score += 1.2;
	    if (lowerText.includes(date.slash.toLowerCase())) score += 1.1;
	    if (lowerText.includes(date.monthName.toLowerCase()) && lowerText.includes(date.dayText) && lowerText.includes(String(date.year))) score += 1.0;
	    if (lowerText.includes(date.monthShort.toLowerCase()) && lowerText.includes(date.dayText) && lowerText.includes(String(date.year))) score += 0.9;
	    const dataMonth = Number(el.getAttribute('data-month'));
	    const dataYear = Number(el.getAttribute('data-year'));
	    const dataDay = Number(el.getAttribute('data-day'));
	    if (dataMonth === date.month && dataYear === date.year && dataDay === date.day) score += 1.1;
	    if (/^\s*\d{{1,2}}\s*$/.test(direct) && Number(direct) === date.day) score += 0.4;
	    if (/\b(?:button|gridcell|option|link)\b/i.test([el.tagName, roleOf(el)].join(' '))) score += 0.08;
	    if (context.includes(date.monthName.toLowerCase()) && context.includes(String(date.year))) score += 0.45;
	    if (context.includes(date.monthShort.toLowerCase()) && context.includes(String(date.year))) score += 0.35;
	    if (el.getAttribute('aria-selected') === 'true' || el.getAttribute('aria-current') === 'date') score += 0.05;
	    if (/\b(?:disabled|outside|other-month|unavailable)\b/i.test(classText(el)) || el.getAttribute('aria-disabled') === 'true') score -= 0.8;
	    if (isCalendarContainer(el) && !hasDateMetadata(el, date)) score -= 1.0;
	    return score;
	  }}
	  function datePickerPlan() {{
	    const date = requestedDate();
	    if (!date) return null;
	    if (!/\b(?:date|day|calendar|select|choose|pick|book|schedule|event)\b/i.test(instruction)) return null;
	    const nativeDates = interactive.filter(el => visible(el) && typeOf(el) === 'date');
	    if (nativeDates.length && kind === 'fill') return null;
	    const raw = all([
	      '[data-date]', '[data-day]', '[datetime]',
	      '[role=gridcell]', '[role=option]',
	      'button', 'a', 'td',
	      '.day', '.date', '.calendar-day'
	    ].join(',')).filter(el => {{
	      if (!visible(el)) return false;
	      if (['script', 'style', 'input', 'select', 'textarea'].includes(el.tagName.toLowerCase())) return false;
	      const rect = el.getBoundingClientRect();
	      if (rect.width < 4 || rect.height < 4) return false;
	      return true;
	    }});
	    const seen = new Set();
	    const cells = [];
	    for (const el of raw) {{
	      const target = dateCellClickTarget(el, date);
	      if (!target || !visible(target)) continue;
	      const key = selector(target);
	      if (seen.has(key)) continue;
	      seen.add(key);
	      cells.push(target);
	    }}
	    const ranked = best(cells, el => dateCellScore(el, date));
	    if (!ranked.length || ranked[0].score < 0.6) {{
	      const openers = best(all('input, button, [role=button], [role=combobox], [aria-haspopup], [data-datepicker], [data-calendar], .date, .datepicker, .calendar, [class*=date], [class*=Date], [class*=calendar], [class*=Calendar]')
	        .filter(el => {{
	          if (!visible(el) || hasDisabledAncestor(el)) return false;
	          const tag = el.tagName.toLowerCase();
	          const type = typeOf(el);
	          if (tag === 'input' && ['hidden', 'button', 'submit', 'checkbox', 'radio', 'file'].includes(type)) return false;
	          const rect = el.getBoundingClientRect();
	          return rect.width >= 8 && rect.height >= 8;
	        }}), el => {{
	          const text = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('name') || '', el.getAttribute('placeholder') || ''].join(' ');
	          const lower = text.toLowerCase();
	          const tag = el.tagName.toLowerCase();
	          const type = typeOf(el);
	          let score = 0;
	          if (/\b(date|day|calendar|datepicker|date picker|when|departure|arrival|start|end)\b/i.test(lower)) score += 0.45;
	          if (targetHint) score += Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text)) * 0.45;
	          if (tag === 'input' && (type === 'date' || type === 'text' || !type)) score += 0.18;
	          if (tag === 'input' && (el.readOnly || el.getAttribute('readonly') != null)) score += 0.16;
	          if (el.hasAttribute('aria-haspopup') || /\b(datepicker|calendar|picker)\b/i.test(classText(el))) score += 0.2;
	          if (tag === 'button' || roleOf(el) === 'button') score += 0.08;
	          return score;
	        }});
	      if (!openers.length || openers[0].score < 0.4) return null;
	      const opener = openers[0].el;
	      const primary = {{
	        ok: true,
	        action: 'date_picker',
	        params: {{ opener: selector(opener), date }},
	        confidence: Math.min(0.92, openers[0].score),
	        reason: 'matched requested date to a date-like control that opens or accepts a calendar value',
	        candidate: candidate(opener),
	        evidence: {{ date: date.iso, openerScore: openers[0].score }}
	      }};
	      const follow = clickStepForHint(followUpClickHint(), opener) ||
	        (/\b(?:submit|continue|confirm|done|save|book|schedule)\b/i.test(instruction) ? completionClickStep(opener) : null);
	      if (!follow) return primary;
	      return {{
	        ok: true,
	        action: 'sequence',
	        steps: [primary, follow],
	        confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
	        reason: 'planned dynamic date-picker interaction plus completion control'
	      }};
	    }}
	    const chosen = ranked[0].el;
	    const primary = {{
	      ok: true,
	      action: 'click',
	      params: {{ selector: selector(chosen) }},
	      confidence: Math.min(0.96, ranked[0].score / 1.35),
	      reason: 'matched requested date to visible calendar/date-picker cell using DOM date metadata and context',
	      candidate: candidate(chosen),
	      evidence: {{ date: date.iso, score: ranked[0].score }}
	    }};
	    const follow = clickStepForHint(followUpClickHint(), chosen) ||
	      (/\b(?:submit|continue|confirm|done|save|book|schedule)\b/i.test(instruction) ? completionClickStep(chosen) : null);
	    if (!follow) return primary;
	    return {{
	      ok: true,
	      action: 'sequence',
	      steps: [primary, follow],
	      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
	      reason: 'planned date-picker selection plus completion control'
	    }};
	  }}
	  function cleanFieldPairLabel(text) {{
	    return String(text || '')
	      .replace(/^(?:set|enter|type|fill|input)\s+/i, '')
      .replace(/^(?:the|a|an)\s+/i, '')
      .replace(/\b(?:field|input|textbox|text\s+box|box|dropdown|select)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function cleanFieldPairValue(text) {{
    return stripFollowUp(String(text || ''))
      .replace(/\s+(?:and|then)\s+(?:submit|save|continue|confirm|done)\b.*$/i, '')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }}
  function fieldPairInstructions() {{
    if (!/\b(?:fill|enter|type|input|set)\b/i.test(instruction)) return [];
    const pattern = /(?:^|[,;]|\band\b)\s*(?:set|enter|type|fill|input)?\s*(?:the\s+)?([A-Za-z][A-Za-z0-9 _/-]{{1,50}}?)\s*(?:field|input|textbox|text\s+box|box|dropdown|select)?\s*(?:to|with|as|=|:)\s*("[^"]+"|'[^']+'|[^,;]+?)(?=\s*(?:[,;]|\band\b\s+(?:set|enter|type|fill|input)?\s*(?:the\s+)?[A-Za-z][A-Za-z0-9 _/-]{{1,50}}?\s*(?:field|input|textbox|text\s+box|box|dropdown|select)?\s*(?:to|with|as|=|:)|$))/gi;
    const pairs = [];
    let match;
    while ((match = pattern.exec(instruction)) !== null) {{
      const label = cleanFieldPairLabel(match[1]);
      const value = cleanFieldPairValue(match[2]);
      if (label && value) pairs.push({{ label, value }});
    }}
    const seen = new Set();
    return pairs.filter(pair => {{
      const key = normalized(pair.label);
      if (!key || seen.has(key)) return false;
      seen.add(key);
      return true;
    }});
  }}
  function quotedInstructionValues() {{
    const values = [];
    const pattern = /"([^"]+)"|'([^']+)'/g;
    let match;
    while ((match = pattern.exec(instruction)) !== null) {{
      const value = cleanFieldPairValue(match[1] || match[2]);
      if (value) values.push(value);
    }}
    return values;
  }}
  function fieldTypeHintsForValue(value) {{
    const before = instruction.slice(0, Math.max(0, instruction.indexOf('"' + value + '"')));
    const tail = before.slice(Math.max(0, before.length - 80)).toLowerCase();
    const hints = [];
    if (/\buser\s*name|username|login\b/.test(tail)) hints.push('username', 'user name', 'login');
    if (/\bpassword|passcode|pin\b/.test(tail)) hints.push('password', 'passcode', 'pin');
    if (/\bemail|e-mail\b/.test(tail)) hints.push('email');
    if (/\bname\b/.test(tail)) hints.push('name');
    return hints;
  }}
  function multiQuotedFieldPlan() {{
    if (kind !== 'fill') return null;
    const values = quotedInstructionValues();
    if (values.length < 2) return null;
    if (!/\b(?:fields?|inputs?|text\s*fields?|text\s*boxes?|textboxes?|login|sign\s*in|form)\b/i.test(instruction)) return null;
    const fields = visualOrder(formFieldCandidates().filter(writableField));
    if (fields.length < values.length) return null;
    const used = new Set();
    const steps = [];
    let anchor = null;
    for (let index = 0; index < values.length; index++) {{
      const value = values[index];
      const hints = fieldTypeHintsForValue(value);
      let ranked = [];
      if (hints.length) {{
        ranked = best(fields.filter(el => !used.has(selector(el))), el => {{
          const text = textOf(el);
          const type = typeOf(el);
          let score = 0;
          for (const hint of hints) {{
            score = Math.max(score, tokenScore(hint, text), exactPhraseScore(hint, text), semanticScore(hint, text));
            if (hint === 'password' && type === 'password') score += 0.8;
            if (hint === 'email' && type === 'email') score += 0.6;
            if (hint === 'username' && /\buser|login|name\b/i.test(text)) score += 0.35;
          }}
          return score;
        }});
      }}
      let chosen = ranked.length && ranked[0].score >= 0.28
        ? ranked[0].el
        : fields.filter(el => !used.has(selector(el)))[0];
      if (!chosen) return null;
      used.add(selector(chosen));
      anchor = chosen;
      steps.push({{
        action: 'type',
        params: {{ selector: selector(chosen), text: transformedValue(value), clear_first: true }},
        confidence: Math.min(1, Math.max(0.62, ranked.length ? ranked[0].score : 0.62)),
        reason: hints.length
          ? 'matched quoted value to field by nearby semantic cue'
          : 'matched quoted value to visible field order',
        candidate: candidate(chosen),
        evidence: {{ value, hints, ordinal: index + 1 }}
      }});
    }}
    const follow = clickStepForHint(followUpClickHint(), anchor) || completionClickStep(anchor);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned multi-value field fill sequence from quoted instruction values',
      evidence: {{ quotedValueCount: values.length }}
    }};
  }}
  function isSelectableField(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    return tag === 'select' || ['combobox', 'listbox', 'menu', 'tree'].includes(role) || el.hasAttribute('aria-haspopup') || isCustomSelectableValueElement(el);
  }}
  function elementByIdInRoot(el, id) {{
    if (!id) return null;
    const cleanId = String(id).replace(/^#/, '');
    const ownerRoot = el && el.getRootNode && el.getRootNode();
    return (ownerRoot && ownerRoot.getElementById && ownerRoot.getElementById(cleanId)) ||
      document.getElementById(cleanId);
  }}
  function selectableOptionText(el) {{
    const parts = [];
    const addOption = option => {{
      if (!option) return;
      parts.push([
        textOf(option),
        option.getAttribute('aria-label') || '',
        option.getAttribute('data-value') || '',
        option.value || ''
      ].join(' '));
    }};
    for (const option of Array.from(el.options || [])) addOption(option);
    const optionSelector = 'option, [role=option], [role=menuitem], [role=menuitemradio], [role=menuitemcheckbox], [role=treeitem], [data-value], button, li';
    for (const option of Array.from(el.querySelectorAll ? el.querySelectorAll(optionSelector) : [])) {{
      addOption(option);
    }}
    const ownedListIds = [
      el.getAttribute && el.getAttribute('aria-controls'),
      el.getAttribute && el.getAttribute('aria-owns')
    ].filter(Boolean).join(' ');
    if (ownedListIds) {{
      for (const id of ownedListIds.split(/\s+/).filter(Boolean)) {{
        const owned = elementByIdInRoot(el, id);
        if (!owned) continue;
        addOption(owned);
        for (const option of Array.from(owned.querySelectorAll ? owned.querySelectorAll(optionSelector) : [])) {{
          addOption(option);
        }}
      }}
    }}
    const nearby = [el.nextElementSibling, el.parentElement && el.parentElement.nextElementSibling]
      .filter(Boolean)
      .filter(node => {{
        const role = roleOf(node);
        const tag = node.tagName && node.tagName.toLowerCase();
        const meta = [
          classText(node),
          node.id || '',
          node.getAttribute && (node.getAttribute('data-options') || node.getAttribute('data-role') || '')
        ].join(' ');
        return ['select', 'ul', 'ol', 'menu'].includes(tag) ||
          ['listbox', 'menu', 'tree'].includes(role) ||
          /\b(options?|dropdown|select|listbox|menu)\b/i.test(meta);
      }});
    for (const node of nearby) {{
      addOption(node);
      for (const option of Array.from(node.querySelectorAll ? node.querySelectorAll(optionSelector) : [])) {{
        addOption(option);
      }}
    }}
    return parts.join(' ').replace(/\s+/g, ' ').trim();
  }}
  function formFieldCandidates() {{
    return interactive.filter(el => isFillableField(el) || isSelectableField(el) || isFileField(el));
  }}
	  function multiFieldFormPlan() {{
	    const multiQuoted = multiQuotedFieldPlan();
	    if (multiQuoted) return multiQuoted;
	    const pairs = fieldPairInstructions();
	    if (pairs.length < 2) return null;
	    const fields = formFieldCandidates().filter(actionableValueField);
	    if (fields.length < pairs.length) return null;
    const used = new Set();
    const steps = [];
    let anchor = null;
    for (const pair of pairs) {{
      const ranked = best(fields.filter(el => !used.has(selector(el))), el => {{
        const text = textOf(el);
        let score = Math.max(tokenScore(pair.label, text), exactPhraseScore(pair.label, text), semanticScore(pair.label, text));
        if (isSelectableField(el) && /\b(dropdown|select|option|state|country|province|city)\b/i.test([pair.label, text].join(' '))) score += 0.15;
        if (writableField(el) && /\b(name|email|phone|address|city|zip|postal|comment|message|title)\b/i.test([pair.label, text].join(' '))) score += 0.1;
        if (isFileField(el) && /\b(file|upload|attach|attachment|document|resume|avatar|photo|image|pdf)\b/i.test([pair.label, text].join(' '))) score += 0.18;
        return score;
      }});
      if (!ranked.length || ranked[0].score < 0.32) return null;
      const chosen = ranked[0].el;
      used.add(selector(chosen));
      anchor = chosen;
      const selectable = isSelectableField(chosen) && !isFillableField(chosen);
      steps.push(valueFieldActionStep(chosen, pair.value, {{
        selectable,
        confidence: Math.min(1, ranked[0].score),
        selectReason: 'matched labeled selectable field from multi-field instruction',
        typeReason: 'matched labeled fillable field from multi-field instruction',
        sliderReason: 'matched labeled slider or range field from multi-field instruction',
        evidence: {{ label: pair.label, value: pair.value }}
      }}));
    }}
    const follow = clickStepForHint(followUpClickHint(), anchor) || completionClickStep(anchor);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned labeled multi-field form fill sequence from instruction',
	      evidence: {{ pairs }}
	    }};
	  }}
	  function cleanTableHint(text) {{
	    return String(text || '')
	      .replace(/^["'\s]+|["'?.\s]+$/g, '')
	      .replace(/\b(?:row|column|col|cell|value|table|labeled|labelled|named|called|containing|for|with|the|a|an)\b/ig, ' ')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function tableLookupRequest() {{
	    const keyValuePatterns = [
	      /\b(?:enter|type|fill|input|write|use)?\s*(?:the\s+)?value\s+(?:of|for)\s+("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{0,60}}?)(?=\s+(?:into|in|to|as|and|then)\b|[?.;,]|$)/i,
	      /\b(?:enter|type|fill|input|write|use)\s+(?:the\s+)?("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{0,60}}?)\s+value\b/i,
	      /\b(?:look\s+up|find|read|extract)\s+(?:the\s+)?(?:value|answer)\s+(?:of|for)\s+("[^"]+"|'[^']+'|[A-Za-z][A-Za-z0-9 _/-]{{0,60}}?)(?=\s+(?:into|in|to|as|and|then)\b|[?.;,]|$)/i
	    ];
	    for (const pattern of keyValuePatterns) {{
	      const match = instruction.match(pattern);
	      if (!match) continue;
	      const keyHint = cleanTableHint(match[1]);
	      if (keyHint) return {{ mode: 'keyValue', keyHint }};
	    }}
	    if (!/\b(?:table|row|column|cell)\b/i.test(instruction)) return null;
	    const patterns = [
	      {{ row: 1, col: 2, pattern: /\brow\s+(?:labeled|labelled|named|called|for|with|containing)?\s*("[^"]+"|'[^']+'|[^,;.]+?)\s*(?:,|and)?\s*(?:column|col)\s+("[^"]+"|'[^']+'|[^,;.]+?)(?:[?.;,]|$)/i }},
	      {{ row: 2, col: 1, pattern: /\b(?:column|col)\s+("[^"]+"|'[^']+'|[^,;.]+?)\s+(?:for|in|from|at)\s+(?:row\s+)?("[^"]+"|'[^']+'|[^,;.]+?)(?:[?.;,]|$)/i }},
	      {{ row: 2, col: 1, pattern: /\bwhat\s+is\s+(?:the\s+)?("[^"]+"|'[^']+'|[^,;.]+?)\s+(?:for|of|in)\s+("[^"]+"|'[^']+'|[^,;.]+?)(?:[?.;,]|$)/i }}
	    ];
	    for (const entry of patterns) {{
	      const match = instruction.match(entry.pattern);
	      if (!match) continue;
	      const rowHint = cleanTableHint(match[entry.row]);
	      const columnHint = cleanTableHint(match[entry.col]);
	      if (rowHint && columnHint) return {{ mode: 'rowColumn', rowHint, columnHint }};
	    }}
	    return null;
	  }}
	  function tableCells(row) {{
	    return Array.from(row.querySelectorAll(':scope > th, :scope > td, :scope > [role=cell], :scope > [role=gridcell], :scope > [role=columnheader], :scope > [role=rowheader]'))
	      .filter(visible);
	  }}
	  function cellText(cell) {{
	    const direct = directTextOf(cell).replace(/\s+/g, ' ').trim();
	    if (direct) return direct;
	    return textOf(cell).replace(/\s+/g, ' ').trim();
	  }}
	  function tableLookupPlan() {{
	    const request = tableLookupRequest();
	    if (!request) return null;
	    const tables = all('table, [role=table], [role=grid], [role=treegrid]').filter(visible);
	    if (request.mode === 'keyValue') {{
	      const candidates = [];
	      for (const table of tables) {{
	        const rows = Array.from(table.querySelectorAll('tr, [role=row]')).filter(visible);
	        for (const row of rows) {{
	          const cells = tableCells(row);
	          if (cells.length < 2) continue;
	          const keyCell = cells[0];
	          const valueCell = cells[1];
	          const keyText = cellText(keyCell);
	          const value = cellText(valueCell);
	          if (!keyText || !value) continue;
	          let score = Math.max(tokenScore(request.keyHint, keyText), exactPhraseScore(request.keyHint, keyText), semanticScore(request.keyHint, keyText));
	          if (score <= 0) score = Math.max(tokenScore(request.keyHint, textOf(row)), exactPhraseScore(request.keyHint, textOf(row)), semanticScore(request.keyHint, textOf(row))) * 0.65;
	          if (score <= 0) continue;
	          if (cells.length === 2) score += 0.12;
	          if (keyCell.tagName.toLowerCase() === 'th' || roleOf(keyCell) === 'rowheader') score += 0.08;
	          score += relationScore(row, table);
	          candidates.push({{ table, row, keyCell, valueCell, value, score }});
	        }}
	      }}
	      for (const list of all('dl').filter(visible)) {{
	        const children = Array.from(list.children || []).filter(visible);
	        for (let index = 0; index < children.length - 1; index += 1) {{
	          if (children[index].tagName.toLowerCase() !== 'dt') continue;
	          const keyCell = children[index];
	          const valueCell = children[index + 1];
	          if (!valueCell || valueCell.tagName.toLowerCase() !== 'dd') continue;
	          const keyText = cellText(keyCell);
	          const value = cellText(valueCell);
	          if (!keyText || !value) continue;
	          let score = Math.max(tokenScore(request.keyHint, keyText), exactPhraseScore(request.keyHint, keyText), semanticScore(request.keyHint, keyText));
	          if (score <= 0) continue;
	          score += 0.12;
	          candidates.push({{ table: list, row: valueCell, keyCell, valueCell, value, score }});
	        }}
	      }}
	      candidates.sort((a, b) => b.score - a.score);
	      if (!candidates.length || candidates[0].score < 0.35) return null;
	      const chosen = candidates[0];
	      const value = chosen.value;
	      const valueButtons = best(clickableElements(), el => {{
	        const text = [textOf(el), directTextOf(el)].join(' ');
	        let score = Math.max(exactPhraseScore(value, text), tokenScore(value, text), semanticScore(value, text));
	        if (score > 0) score += relationScore(el, chosen.table);
	        return score;
	      }});
	      if (valueButtons.length && /\b(?:click|choose|select|press|tap)\b/i.test(instruction) && !/\b(?:enter|type|fill|input|write)\b/i.test(instruction)) {{
	        return {{
	          ok: true,
	          action: 'click',
	          params: {{ selector: selector(valueButtons[0].el) }},
	          confidence: Math.min(0.95, (chosen.score + valueButtons[0].score) / 2),
	          reason: 'looked up a key/value entry, then matched its value to a visible choice',
	          candidate: candidate(valueButtons[0].el),
	          evidence: {{ keyHint: request.keyHint, value, valueCell: candidate(chosen.valueCell) }}
	        }};
	      }}
	      const fields = best(formFieldCandidates().filter(writableField), el => {{
	        const text = textOf(el);
	        let score = Math.max(tokenScore(targetHint || 'response result value text field', text), exactPhraseScore(targetHint || 'value', text), semanticScore(targetHint || 'response result value text field', text));
	        score += relationScore(el, chosen.table);
	        if (!text) score += 0.1;
	        if (!String(el.value || '').trim()) score += 0.08;
	        return score;
	      }});
	      if (fields.length && /\b(?:enter|type|fill|input|answer|submit|respond|write|use)\b/i.test(instruction)) {{
	        const primary = {{
	          ok: true,
	          action: 'type',
	          params: {{ selector: selector(fields[0].el), text: value, clear_first: true }},
	          confidence: Math.min(0.95, (chosen.score + fields[0].score) / 2),
	          reason: 'looked up a key/value entry, then entered the value into a writable field',
	          candidate: candidate(fields[0].el),
	          evidence: {{ keyHint: request.keyHint, value, valueCell: candidate(chosen.valueCell) }}
	        }};
	        const follow = clickStepForHint(followUpClickHint(), fields[0].el) ||
	          (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(fields[0].el) : null);
	        if (!follow) return primary;
	        return {{
	          ok: true,
	          action: 'sequence',
	          steps: [primary, follow],
	          confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
	          reason: 'planned key/value lookup value entry plus completion control'
	        }};
	      }}
	      if (kind === 'read_text' || /\b(?:read|extract|what is|find|look up)\b/i.test(instruction)) {{
	        return {{
	          ok: true,
	          action: 'read_text',
	          params: {{ selector: selector(chosen.valueCell), max_length: 2000 }},
	          confidence: Math.min(0.92, chosen.score),
	          reason: 'looked up a readable value by visible key label',
	          candidate: candidate(chosen.valueCell),
	          evidence: {{ keyHint: request.keyHint, value }}
	        }};
	      }}
	      return null;
	    }}
	    for (const table of tables) {{
	      const rows = Array.from(table.querySelectorAll('tr, [role=row]')).filter(visible);
	      if (rows.length < 2) continue;
	      const headerRows = rows.filter(row => row.querySelector('th, [role=columnheader]'));
	      const headerRow = headerRows[0] || rows[0];
	      const headers = tableCells(headerRow);
	      if (headers.length < 2) continue;
	      const rankedColumns = headers.map((cell, index) => {{
	        const text = cellText(cell);
	        let score = Math.max(tokenScore(request.columnHint, text), exactPhraseScore(request.columnHint, text), semanticScore(request.columnHint, text));
	        if (cell.tagName.toLowerCase() === 'th' || roleOf(cell) === 'columnheader') score += 0.12;
	        return {{ cell, index, score }};
	      }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
	      if (!rankedColumns.length || rankedColumns[0].score < 0.35) continue;
	      const columnIndex = rankedColumns[0].index;
	      const bodyRows = rows.filter(row => row !== headerRow);
	      const rankedRows = bodyRows.map(row => {{
	        const cells = tableCells(row);
	        const rowHeader = cells.find(cell => cell.tagName.toLowerCase() === 'th' || roleOf(cell) === 'rowheader') || cells[0];
	        const text = [rowHeader ? cellText(rowHeader) : '', textOf(row)].join(' ');
	        let score = Math.max(tokenScore(request.rowHint, text), exactPhraseScore(request.rowHint, text), semanticScore(request.rowHint, text));
	        if (rowHeader) score += 0.08;
	        return {{ row, cells, score }};
	      }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
	      if (!rankedRows.length || rankedRows[0].score < 0.35) continue;
	      const chosenRow = rankedRows[0];
	      const cell = chosenRow.cells[columnIndex];
	      if (!cell) continue;
	      const value = cellText(cell);
	      if (!value) continue;
	      const valueButtons = best(clickableElements(), el => {{
	        const text = [textOf(el), directTextOf(el)].join(' ');
	        let score = Math.max(exactPhraseScore(value, text), tokenScore(value, text));
	        if (score > 0) score += relationScore(el, table);
	        return score;
	      }});
	      if (valueButtons.length &&
	        /\b(?:click|choose|select|press|tap)\b/i.test(instruction) &&
	        !/\b(?:enter|type|fill|input|write|answer|respond|use)\b/i.test(instruction)) {{
	        return {{
	          ok: true,
	          action: 'click',
	          params: {{ selector: selector(valueButtons[0].el) }},
	          confidence: Math.min(0.95, (rankedColumns[0].score + rankedRows[0].score + valueButtons[0].score) / 3),
	          reason: 'looked up a table cell by row and column, then matched its value to a visible choice',
	          candidate: candidate(valueButtons[0].el),
	          evidence: {{ rowHint: request.rowHint, columnHint: request.columnHint, value, cell: candidate(cell) }}
	        }};
	      }}
	      const fields = best(formFieldCandidates().filter(writableField), el => {{
	        const text = textOf(el);
	        let score = Math.max(tokenScore('response result value', text), exactPhraseScore('value', text), semanticScore('response result value', text));
	        score += relationScore(el, table);
	        if (!text) score += 0.08;
	        return score;
	      }});
	      if (fields.length && /\b(?:enter|type|fill|input|answer|submit|respond|write)\b/i.test(instruction)) {{
	        const primary = {{
	          ok: true,
	          action: 'type',
	          params: {{ selector: selector(fields[0].el), text: value, clear_first: true }},
	          confidence: Math.min(0.95, (rankedColumns[0].score + rankedRows[0].score + fields[0].score) / 3),
	          reason: 'looked up a table cell by row and column, then entered the value into a writable field',
	          candidate: candidate(fields[0].el),
	          evidence: {{ rowHint: request.rowHint, columnHint: request.columnHint, value, cell: candidate(cell) }}
	        }};
	        const follow = clickStepForHint(followUpClickHint(), fields[0].el) ||
	          (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(fields[0].el) : null);
	        if (!follow) return primary;
	        return {{
	          ok: true,
	          action: 'sequence',
	          steps: [primary, follow],
	          confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
	          reason: 'planned table lookup value entry plus completion control'
	        }};
	      }}
	      if (kind === 'read_text' || /\b(?:read|extract|what is|find)\b/i.test(instruction)) {{
	        return {{
	          ok: true,
	          action: 'read_text',
	          params: {{ selector: selector(cell), max_length: 2000 }},
	          confidence: Math.min(0.92, (rankedColumns[0].score + rankedRows[0].score) / 2),
	          reason: 'looked up a readable table cell by row and column labels',
	          candidate: candidate(cell),
	          evidence: {{ rowHint: request.rowHint, columnHint: request.columnHint, value }}
	        }};
	      }}
	    }}
	    return null;
	  }}
  function tableToFormFillPlan() {{
    if (!/\b(?:enter|type|fill|input|write|use)\b/i.test(instruction)) return null;
    if (!/\b(?:corresponds?|matching|matches?|each\s+label|labels?|table|form)\b/i.test(instruction)) return null;
    const entries = [];
    const tables = all('table, [role=table], [role=grid], [role=treegrid]').filter(visible);
    for (const table of tables) {{
      const rows = Array.from(table.querySelectorAll('tr, [role=row]')).filter(visible);
      for (const row of rows) {{
        const cells = tableCells(row);
        if (cells.length < 2) continue;
        const key = cleanTableHint(cellText(cells[0]));
        const value = cellText(cells[1]);
        if (!key || !value) continue;
        entries.push({{ key, value, keyCell: cells[0], valueCell: cells[1], source: table }});
      }}
    }}
    for (const list of all('dl').filter(visible)) {{
      const children = Array.from(list.children || []).filter(visible);
      for (let index = 0; index < children.length - 1; index += 1) {{
        if (children[index].tagName.toLowerCase() !== 'dt') continue;
        const key = cleanTableHint(cellText(children[index]));
        const value = cellText(children[index + 1]);
        if (key && value) entries.push({{ key, value, keyCell: children[index], valueCell: children[index + 1], source: list }});
      }}
    }}
    if (entries.length < 2 || entries.length > 80) return null;

    function fieldLabelText(field) {{
      const labels = [];
      labels.push(associatedLabelText(field));
      const previous = field.previousElementSibling;
      if (previous && visible(previous)) labels.push(textOf(previous));
      const parent = field.parentElement;
      if (parent) {{
        const direct = Array.from(parent.children || [])
          .filter(child => child !== field && visible(child) && !child.matches('input, textarea, select, button, a, [role=button]'))
          .map(child => directTextOf(child) || textOf(child))
          .join(' ');
        labels.push(direct);
      }}
      labels.push(field.getAttribute('aria-label') || '');
      labels.push(field.getAttribute('placeholder') || '');
      labels.push(field.getAttribute('name') || '');
      labels.push(textOf(field));
      return cleanTableHint(labels.join(' '));
    }}

    const fields = formFieldCandidates()
      .filter(writableField)
      .filter(visible)
      .filter(el => !String(el.value || el.textContent || '').trim())
      .map(el => {{ return {{ el, label: fieldLabelText(el) }}; }})
      .filter(item => item.label);
    if (!fields.length) return null;

    const usedFields = new Set();
    const usedEntries = new Set();
    const steps = [];
    let anchor = null;
    for (const field of fields.slice(0, 20)) {{
      const ranked = entries.map((entry, index) => {{
        let score = Math.max(
          exactPhraseScore(entry.key, field.label),
          tokenScore(entry.key, field.label),
          semanticScore(entry.key, field.label),
          exactPhraseScore(field.label, entry.key),
          tokenScore(field.label, entry.key)
        );
        if (score > 0) score += relationScore(field.el, entry.source) * 0.2;
        return {{ entry, index, score }};
      }}).filter(item => item.score >= 0.35 && !usedEntries.has(item.index))
        .sort((a, b) => b.score - a.score);
      if (!ranked.length) continue;
      const chosen = ranked[0];
      const fieldKey = selector(field.el);
      if (usedFields.has(fieldKey)) continue;
      usedFields.add(fieldKey);
      usedEntries.add(chosen.index);
      anchor = field.el;
      steps.push({{
        action: 'type',
        params: {{ selector: fieldKey, text: chosen.entry.value, clear_first: true }},
        confidence: Math.min(0.94, chosen.score),
        reason: 'filled form field by matching its label to a table key/value row',
        candidate: candidate(field.el),
        evidence: {{
          label: field.label,
          key: chosen.entry.key,
          value: chosen.entry.value,
          valueCell: candidate(chosen.entry.valueCell)
        }}
      }});
    }}
    if (!steps.length) return null;
    if (fields.length > 1 && steps.length < Math.min(fields.length, 2)) return null;
    const follow = clickStepForHint(followUpClickHint(), anchor) ||
      (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(anchor) : null);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(0.94, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned table key/value transfer into matching labeled form fields',
      evidence: {{ entryCount: entries.length, filledFields: steps.length }}
    }};
  }}
  function recordPropertyClickPlan() {{
    if (!/\b(?:find|locate|search|open|select|click)\b/i.test(instruction)) return null;
    if (!/\b(?:click|select|open|press|tap)\b/i.test(instruction)) return null;
    const patterns = [
      /\bfind\s+(.+?)\s+(?:in|within|from|on)\s+(?:the\s+)?(.+?)\s+and\s+(?:click|select|open|press|tap)\s+(?:on\s+)?(?:their|its|the)?\s*([^,.!?]+)(?:[,.!?]|$)/i,
      /\b(?:click|select|open|press|tap)\s+(?:the\s+)?([^,.!?]+?)\s+(?:for|of)\s+(.+?)(?:[,.!?]|$)/i
    ];
    let entity = null;
    let property = null;
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (!match) continue;
      if (pattern === patterns[0]) {{
        entity = match[1];
        property = match[3];
      }} else {{
        property = match[1];
        entity = match[2];
      }}
      break;
    }}
    function cleanRecordText(text) {{
      return String(text || '')
        .replace(/^["'\s]+|["'.,!?\s]+$/g, '')
        .replace(/\b(?:record|entry|item|profile|contact|user|person|customer|account|book|list|directory|their|its|the|a|an)\b/ig, ' ')
        .replace(/\s+/g, ' ')
        .trim();
    }}
    entity = cleanRecordText(entity);
    property = cleanRecordText(property);
    if (!entity || !property) return null;
    if (entity.length > 80 || property.length > 80) return null;
    const hasLikelyRecords = all('[data-record], [data-result], [data-contact], [data-person], article, section, [role=listitem], [role=row], li, tr, .card, .record, .result, .contact, .item, div')
      .filter(visible)
      .some(el => {{
        const text = textOf(el);
        return exactPhraseScore(entity, text) > 0 || tokenScore(entity, text) > 0.45;
      }});
    const hasPagination = all('a, button, [role=button], [role=link], [onclick], [tabindex], .page-link, .page-item a')
      .filter(visible)
      .some(el => /\b(next|more|page)\b|^>|\d+/i.test([textOf(el), classText(el), el.getAttribute('aria-label') || ''].join(' ')));
    if (!hasLikelyRecords && !hasPagination) return null;
    return {{
      ok: true,
      action: 'record_property_click',
      params: {{ entity, property, maxPages: 12 }},
      confidence: 0.82,
      reason: 'planned record lookup across visible or paginated records and requested property click',
      evidence: {{ entity, property, hasLikelyRecords, hasPagination }}
    }};
  }}
	  function resultPreference() {{
	    const match = instruction.match(/\b(cheapest|lowest|least expensive|shortest|fastest|longest|most expensive|highest)\b/i);
	    return match ? match[1].toLowerCase() : null;
	  }}
	  function resultOrdinalRequest() {{
	    if (!/\bresults?\b/i.test(instruction)) return null;
	    const match = instruction.match(/\b(?:click|press|tap|select|open|choose|pick|find)\s+(?:and\s+click\s+)?(?:the\s+)?((?:last|first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth|\d+(?:st|nd|rd|th)?)(?:\s+\w+){{0,2}}\s+results?)\b/i) ||
	      instruction.match(/\b((?:last|first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth|\d+(?:st|nd|rd|th)?)(?:\s+\w+){{0,2}}\s+results?)\b/i);
	    if (!match) return null;
	    return ordinalTargetIndex(match[1]);
	  }}
	  function dateParamFromRequest(date) {{
	    if (!date) return null;
	    return {{ iso: date.iso, value: date.slash, month: date.month, day: date.day, year: date.year }};
	  }}
	  function cleanWorkflowValue(text) {{
	    return String(text || '')
	      .replace(/^["'\s]+|["'.,\s]+$/g, '')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function endpointFormWorkflowRequest() {{
	    const hasEndpointAction = /\b(?:book|reserve|schedule|find|search|look for|look up|show me|show|get|compare)\b/i.test(instruction);
	    const hasImplicitEndpointRequest = /\b(?:cheapest|lowest|least expensive|shortest|fastest)\b/i.test(instruction) &&
	      /\b(?:option|service|request|route|reservation|booking)\b/i.test(instruction);
	    if (!hasEndpointAction && !hasImplicitEndpointRequest) return null;
    if (/\b(?:guess|find)\b/i.test(instruction) &&
        /\b(?:hidden|secret|unknown|target|correct|right)?\s*(?:number|value)\b/i.test(instruction) &&
        /\bfeedback\b/i.test(instruction)) return null;
	    const hasFromTo = /\bfrom\b/i.test(instruction) && /\bto\b/i.test(instruction);
	    const hasBetweenAnd = /\bbetween\b/i.test(instruction) && /\band\b/i.test(instruction);
	    if (!hasFromTo && !hasBetweenAnd) return null;
	    const date = requestedDate();
	    const patterns = [
	      /\bfrom\s*:?\s*(.+?)\s+to\s*:?\s*(.+?)\s+(?:on|for|by)\s+(\d{{1,2}}\/\d{{1,2}}\/\d{{4}}|\d{{4}}-\d{{1,2}}-\d{{1,2}})\b/i,
	      /\bfrom\s*:?\s*(.+?)\s+to\s*:?\s*(.+?)\s+(?:on|for|by)\s+(?:january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec)\.?\s+\d{{1,2}}(?:st|nd|rd|th)?(?:,)?\s+\d{{4}}\b/i,
	      /\bfrom\s*:?\s*(.+?)\s+to\s*:?\s*(.+?)(?:[,.]|$)/i,
	      /\bbetween\s*:?\s*(.+?)\s+and\s+(.+?)\s+(?:on|for|by)\s+(\d{{1,2}}\/\d{{1,2}}\/\d{{4}}|\d{{4}}-\d{{1,2}}-\d{{1,2}})\b/i,
	      /\bbetween\s*:?\s*(.+?)\s+and\s+(.+?)\s+(?:on|for|by)\s+(?:january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec)\.?\s+\d{{1,2}}(?:st|nd|rd|th)?(?:,)?\s+\d{{4}}\b/i,
	      /\bbetween\s*:?\s*(.+?)\s+and\s+(.+?)(?:[,.]|$)/i
	    ];
	    for (const pattern of patterns) {{
	      const match = instruction.match(pattern);
	      if (!match) continue;
	      const from = cleanWorkflowValue(match[1]);
	      const to = cleanWorkflowValue(match[2]);
	      if (!from || !to) continue;
	      if (/^(?:click|press|tap|submit|check|guess|try|go|ok|done|enter|continue|confirm)\b/i.test(from) ||
	          /^(?:click|press|tap|submit|check|guess|try|go|ok|done|enter|continue|confirm)\b/i.test(to)) continue;
	      return {{
	        mode: 'endpoint-form-workflow',
	        completionHint: /\bbook|reserve\b/i.test(instruction) ? 'search book continue submit' : 'search submit continue',
	        resultPreference: resultPreference(),
	        date: dateParamFromRequest(date),
	        fields: [
	          {{ label: 'from', hints: ['from', 'origin', 'source', 'departure', 'departing from', 'pickup'], value: from }},
	          {{ label: 'to', hints: ['to', 'destination', 'arrival', 'arriving to', 'dropoff'], value: to }},
	        ].concat(date ? [{{ label: 'date', hints: ['date', 'departure date', 'service date', 'day', 'when'], value: date.slash || date.iso }}] : [])
	      }};
	    }}
	    return null;
	  }}
	  function scheduledEntityFormWorkflowRequest() {{
	    if (!/\b(?:create|schedule|add|plan)\b/i.test(instruction)) return null;
	    if (!/\b(?:event|appointment|meeting|booking|reservation)\b/i.test(instruction)) return null;
	    const titleMatch = instruction.match(/\b(?:named|called|titled|title)\s+"([^"]+)"/i) ||
	      instruction.match(/\b(?:named|called|titled|title)\s+'([^']+)'/i);
	    const durationMatch = instruction.match(/\b(\d+(?:\.\d+)?)\s*(mins?|minutes?|hours?|hrs?)\b/i);
	    const betweenMatch = instruction.match(/\bbetween\s+([0-9]{{1,2}}(?::[0-9]{{2}})?\s*(?:am|pm)?)\s+and\s+([0-9]{{1,2}}(?::[0-9]{{2}})?\s*(?:am|pm)?)/i);
	    const date = requestedDate();
	    const fields = [];
	    if (titleMatch && titleMatch[1]) {{
	      fields.push({{ label: 'title', hints: ['title', 'name', 'event name', 'subject'], value: cleanWorkflowValue(titleMatch[1]) }});
	    }}
	    if (durationMatch) {{
	      fields.push({{ label: 'duration', hints: ['duration', 'length', 'minutes', 'time needed'], value: cleanWorkflowValue(durationMatch[1] + ' ' + durationMatch[2]) }});
	    }}
	    if (betweenMatch) {{
	      fields.push({{ label: 'start time', hints: ['start', 'start time', 'begin', 'from time', 'between'], value: cleanWorkflowValue(betweenMatch[1]) }});
	      fields.push({{ label: 'end time', hints: ['end', 'end time', 'until', 'to time'], value: cleanWorkflowValue(betweenMatch[2]) }});
	    }}
	    if (date) {{
	      fields.push({{ label: 'date', hints: ['date', 'event date', 'day', 'when'], value: date.iso }});
	    }}
	    if (!fields.length) return null;
	    return {{
	      mode: 'scheduled-entity-form-workflow',
	      completionHint: 'create schedule save submit done',
	      resultPreference: null,
	      date: dateParamFromRequest(date),
	      fields
	    }};
	  }}
	  function cleanSearchFacetValue(value) {{
	    return String(value || '')
	      .replace(/^["'\s]+|["'?.\s]+$/g, '')
	      .replace(/\b(?:movies?|films?|books?|products?|items?|records?|results?|entries?|users?|contacts?|articles?|pages?)\b/ig, ' ')
	      .replace(/\s+/g, ' ')
	      .trim();
	  }}
	  function facetedSearchFormWorkflowRequest() {{
	    if (!/\b(?:search|filter|find)\b/i.test(instruction)) return null;
	    if (!formFieldCandidates().some(el => visible(el))) return null;
	    const searchOrFilterVerb = /\b(?:search|filter)\b/i.test(instruction);
	    const clickThroughDiscovery = !searchOrFilterVerb &&
	      /\bclick\b/i.test(instruction) &&
	      /\b(?:link|button|item|option|tab|section|panel|menu)\b/i.test(instruction);
	    if (clickThroughDiscovery) return null;
	    const fields = [];
	    const text = stripFollowUp(instruction);
	    const categoryMatch = text.match(/\b(?:search|filter|find)\s+for\s+(.+?)(?=\s+(?:directed|authored|written|created|made|published)\s+by\b|\s+(?:from|in|for)\s+year\b|\s+year\b|[.;,]|$)/i);
	    const byMatch = text.match(/\b(?:directed|authored|written|created|made|published)\s+by\s+([A-Za-z][A-Za-z0-9 _.'-]*?)(?=\s+(?:from|in|for)\s+year\b|\s+year\b|[.;,]|$)/i);
	    const yearMatch = text.match(/\b(?:from|in|for)?\s*year\s+(\d{{4}})\b/i) || text.match(/\b(?:from|in|for)\s+(\d{{4}})\b/i);
	    const quoted = quotedInstructionValues();
	    if (categoryMatch) {{
	      const category = cleanSearchFacetValue(categoryMatch[1]);
	      if (category) fields.push({{ label: 'category', hints: ['genre', 'category', 'type', 'kind', 'query', 'search'], value: category }});
	    }} else if (quoted.length) {{
	      fields.push({{ label: 'query', hints: ['query', 'search', 'keywords', 'term', 'text'], value: quoted[0] }});
	    }}
	    if (byMatch) {{
	      const person = cleanSearchFacetValue(byMatch[1]);
	      if (person) fields.push({{ label: 'creator', hints: ['director', 'author', 'creator', 'artist', 'by', 'owner', 'person', 'name'], value: person }});
	    }}
	    if (yearMatch) {{
	      fields.push({{ label: 'year', hints: ['year', 'date'], value: yearMatch[1] }});
	    }}
		    if (!fields.length) return null;
		    const resultClickHint = cleanClickHint(followUpClickHint() || '');
		    return {{
		      mode: 'faceted-query-form-workflow',
		      completionHint: 'search filter find submit go apply',
		      resultPreference: null,
		      resultOrdinal: resultOrdinalRequest(),
		      resultClickHint: resultClickHint && resultOrdinalRequest() == null ? resultClickHint : null,
		      date: null,
		      fields
		    }};
	  }}
	  function formWorkflowRequest() {{
	    return endpointFormWorkflowRequest() || scheduledEntityFormWorkflowRequest() || facetedSearchFormWorkflowRequest();
	  }}
	  function formWorkflowPlan() {{
	    const request = formWorkflowRequest();
	    if (!request) return null;
	    return {{
	      ok: true,
	      action: 'form_workflow',
	      params: request,
	      confidence: request.resultPreference ? 0.84 : 0.82,
	      reason: 'planned generic multi-step form workflow from instruction entities and page field labels',
	      evidence: {{
	        mode: request.mode,
	        fieldCount: request.fields.length,
	        hasDate: !!request.date,
	        resultPreference: request.resultPreference || null,
	        resultOrdinal: request.resultOrdinal ?? null
	      }}
	    }};
	  }}
	  function uploadFilePlan() {{
	    if (kind !== 'upload_file') return null;
	    const filePath = wantedValue && stripFollowUp(wantedValue);
    if (!filePath) return null;
    function fileFieldContextText(el) {{
      const parts = [textOf(el), classText(el), el.id || '', el.getAttribute('name') || '', el.getAttribute('accept') || ''];
      const label = el.closest && el.closest('label');
      if (label) parts.push(textOf(label), classText(label), label.id || '');
      const container = el.closest && el.closest('label, [data-dropzone], [data-upload], [data-field], [role=button], .dropzone, .upload, .uploader, .file, .attachment, div, section');
      if (container && container !== el) {{
        parts.push(
          textOf(container),
          classText(container),
          container.id || '',
          container.getAttribute('data-dropzone') || '',
          container.getAttribute('data-upload') || '',
          container.getAttribute('data-field') || '',
          container.getAttribute('aria-label') || '',
          container.getAttribute('title') || ''
        );
      }}
      const previous = el.previousElementSibling;
      const next = el.nextElementSibling;
      for (const sibling of [previous, next]) {{
        if (!sibling) continue;
        parts.push(textOf(sibling), classText(sibling), sibling.id || '', sibling.getAttribute && (sibling.getAttribute('aria-label') || sibling.getAttribute('title') || ''));
      }}
      return parts.join(' ').replace(/\s+/g, ' ').trim();
    }}
    const inputs = all('input[type=file]').filter(el => !hasDisabledAncestor(el));
    if (!inputs.length) return null;
    const ranked = inputs.map(el => {{
      const text = fileFieldContextText(el);
      let score = targetHint ? Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text)) : 0.35;
      if (/\b(file|upload|attach|import|document|avatar|image|photo|resume)\b/i.test(text)) score += 0.25;
      if (/\b(drop\s*zone|dropzone|drop|browse|choose\s+file|select\s+file)\b/i.test(text)) score += 0.18;
      if (visible(el)) score += 0.1;
      return {{ el, score }};
    }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
    if (!ranked.length) return null;
    const chosen = ranked[0].el;
    const primary = {{
      ok: true,
      action: 'upload_file',
      params: {{ selector: selector(chosen), files: [filePath] }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'matched file-upload input by instruction and DOM labels',
      candidate: candidate(chosen),
      evidence: {{ fileCount: 1, targetHint }}
    }};
    return withFollowUp(primary, chosen);
  }}
  function autocompletePlan() {{
    if (kind !== 'fill' && kind !== 'select_option') return null;
    const startsWith = quotedConstraint('starts');
    const endsWith = quotedConstraint('ends');
    const mentionsAutocomplete = /\b(auto-?complete|autocomplete|suggestions?|starts\s+with|ends\s+with)\b/i.test(instruction);
    if (!mentionsAutocomplete && !all('input[aria-autocomplete], input[list], [role=combobox][aria-autocomplete]').some(visible)) return null;
    const query = startsWith || wantedValue;
    if (!query) return null;
    const fields = best(writableFields(), el => {{
      const text = textOf(el);
      let score = targetHint ? tokenScore(targetHint, text) : 0.25;
      if (el.getAttribute('aria-autocomplete')) score += 0.6;
      if (el.getAttribute('list')) score += 0.55;
      if (roleOf(el) === 'combobox') score += 0.45;
      if (/\b(auto-?complete|suggest|search|tag|item)\b/i.test(text)) score += 0.25;
      if (typeOf(el) === 'search') score += 0.2;
      return score;
    }});
    if (!fields.length) return null;
    const chosen = fields[0];
    const primary = {{
      ok: true,
      action: 'autocomplete_select',
      params: {{
        selector: selector(chosen.el),
        query,
        startsWith,
        endsWith,
        optionText: startsWith || endsWith ? null : stripFollowUp(wantedValue)
      }},
      confidence: Math.min(1, chosen.score + 0.15),
      reason: 'matched dynamic autocomplete or suggestion selection constraints from instruction',
      candidate: candidate(chosen.el)
    }};
    const explicitFollow = clickStepForHint(followUpClickHint(), chosen.el);
    const wantsCompletion = startsWith || endsWith || /\b(?:submit|continue|confirm|done|save)\b/i.test(instruction);
    const inferredCompletion = (!explicitFollow && wantsCompletion) ? completionClickStep(chosen.el) : null;
    const follow = explicitFollow || inferredCompletion;
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: explicitFollow
        ? 'planned autocomplete selection plus explicit follow-up click'
        : 'planned autocomplete selection plus nearby completion control for constrained item-entry workflow'
    }};
  }}
  function sliderCheckboxPlan() {{
    if (!(/\bslider\b/i.test(instruction) && /\bcheckbox\b/i.test(instruction))) return null;
    const steps = [];
    const slider = sliderPlan();
    if (slider) steps.push(slider);
    const boxes = interactive.filter(isCheckedControl);
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

  function quotedAfter(patterns) {{
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (match && match[1]) return match[1].trim();
    }}
    return null;
  }}
  function workflowActionHint() {{
    const quotedLocalAction = instruction.match(/\bclick\s+(?:on\s+)?(?:the\s+)?["']([^"']+)["']\s+(?:button|icon|action|control)?\s+(?:on|for|in)\s+(?:\d+\s+)?(?:posts?|items?|rows?|cards?|records?|entries?)\s+(?:by|from|for|with|containing|named|called)\b/i);
    if (quotedLocalAction && quotedLocalAction[1]) return quotedLocalAction[1].trim();
    if (/\b(reply|respond)\b/i.test(instruction)) return 'reply';
    if (/\brepost\b/i.test(instruction)) return 'repost';
    if (/\blike\b/i.test(instruction)) return 'like';
    if (/\bshare\b/i.test(instruction)) return 'share';
    if (/\b(?:send|forward)\b/i.test(instruction) && /\b(?:email|e-mail|message|conversation|thread)\b/i.test(instruction)) return 'forward';
    if (/\b(?:wants|waiting\s+for)\b/i.test(instruction) && /\b(?:email|e-mail|message|conversation|thread)\b/i.test(instruction)) return 'forward';
    if (/\bforward\b/i.test(instruction)) return 'forward';
    if (/\b(mark\s+(?:it\s+)?as\s+important|important|star|favorite|favourite|priority)\b/i.test(instruction)) return 'star important';
    if (/\b(delete|remove|trash)\b/i.test(instruction)) return 'delete';
    if (/\barchive\b/i.test(instruction)) return 'archive';
    const toggleAction = instruction.match(/\b(?:turn\s+on|enable|check|tick|turn\s+off|disable|uncheck|untick)\s+(?:the\s+)?([^,.]+?)(?:\s+(?:for|on|in|inside|within)\b|[,.]|$)/i);
    if (toggleAction && toggleAction[1]) return toggleAction[0].replace(/[,.]+$/g, '').trim();
    return null;
  }}
  function workflowForwardRecipient() {{
    function cleanRecipient(value) {{
      if (!value) return null;
      const cleaned = value
        .replace(/^["'\s]+|["'\s]+$/g, '')
        .replace(/[.,;:!?]+$/g, '')
        .replace(/\b(?:about|regarding|with|and|then)\b.*$/i, '')
        .trim();
      return cleaned || null;
    }}
    const patterns = [
      /\b(?:forward|send)\b.*?\b(?:to|for)\s+"([^"]+)"/i,
      /\b(?:forward|send)\b.*?\b(?:to|for)\s+'([^']+)'/i,
      /\b(?:forward|send)\b.*?\b(?:to|for)\s+([A-Za-z][A-Za-z0-9_'’-]*(?:\s+[A-Za-z][A-Za-z0-9_'’-]*){{0,3}})(?:[,.]|$)/i,
      /^([A-Za-z][A-Za-z0-9_'’-]*(?:\s+[A-Za-z][A-Za-z0-9_'’-]*){{0,3}})\s+(?:wants|is\s+waiting\s+for)\b/i,
      /\b(?:to|for)\s+([A-Za-z][A-Za-z0-9_'’-]*(?:\s+[A-Za-z][A-Za-z0-9_'’-]*){{0,3}})[\s.,;:!?]*$/i
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (match && match[1]) return cleanRecipient(match[1]);
    }}
    return null;
  }}
  function workflowFillText() {{
    const explicit = quotedAfter([
      /\b(?:reply|respond)\s+"([^"]+)"/i,
      /\b(?:reply|respond)\s+'([^']+)'/i,
      /\b(?:with|using)\s+(?:the\s+)?(?:text|message|reply|comment)\s+"([^"]+)"/i,
      /\b(?:with|using)\s+(?:the\s+)?(?:text|message|reply|comment)\s+'([^']+)'/i,
      /\b(?:saying|containing)\s+"([^"]+)"/i,
      /\b(?:saying|containing)\s+'([^']+)'/i
    ]);
    if (explicit) return explicit;
    if (/\b(?:forward|send|wants|waiting\s+for)\b/i.test(instruction) && /\b(?:email|e-mail|message|conversation|thread|information|details|content|record|item)\b/i.test(instruction)) {{
      return workflowForwardRecipient();
    }}
    return null;
  }}
  function workflowItemQuery() {{
    const patterns = [
      /\b(?:forward|send|reply|respond\s+to)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,2}})['’]s\s+(?:email|e-mail|message|conversation|thread)\b/i,
      /\b(?:information|details|content|record|item)\s+(?:sent\s+)?(?:by|from)\s+"([^"]+)"/i,
      /\b(?:information|details|content|record|item)\s+(?:sent\s+)?(?:by|from)\s+'([^']+)'/i,
      /\b(?:information|details|content|record|item)\s+(?:sent\s+)?(?:by|from)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})(?:\s+(?:and|then|to|for)\b|[,.]|$)/,
      /\b(?:email|e-mail|message|conversation|thread)\s+(?:sent\s+)?(?:by|from)\s+"([^"]+)"/i,
      /\b(?:email|e-mail|message|conversation|thread)\s+(?:sent\s+)?(?:by|from)\s+'([^']+)'/i,
      /\b(?:email|e-mail|message|conversation|thread)\s+(?:sent\s+)?(?:by|from)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})(?:\s+(?:and|then|to|for)\b|[,.]|$)/,
      /\b(?:email|e-mail|message|conversation|thread)\b[^,.?]*?\b(?:sent\s+)?(?:by|from)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})(?:\s+(?:and|then|to|for)\b|[,.?]|$)/i,
      /\b(?:email|e-mail|message|conversation|thread)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})\s+(?:sent|sent\s+to\s+you|sent\s+me)\b/,
      /\b([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})['’]?s\s+(?:email|e-mail|message|conversation|thread)\b/,
      /\b(?:find|open|select|choose)\s+(?:the\s+)?(?:email|message|conversation|thread|ticket|record|row|card|item|order)?\s*(?:by|from|for|with|containing|named|called)\s+"([^"]+)"/i,
      /\b(?:find|open|select|choose)\s+(?:the\s+)?(?:email|message|conversation|thread|ticket|record|row|card|item|order)?\s*(?:by|from|for|with|containing|named|called)\s+'([^']+)'/i,
      /\b(?:posts?|items?|rows?|cards?|records?|entries?)\s+(?:by|from|for|with|containing|named|called)\s+(@[A-Za-z0-9_.-]+|"[^"]+"|'[^']+'|[^,.;]+?)(?:\s+(?:and|then)\b|[,.]|$)/i,
      /\b(?:by|from|for|with|containing|named|called)\s+(@[A-Za-z0-9_.-]+|"[^"]+"|'[^']+'|[^,.;]+?)(?:\s+(?:and|then)\b|[,.]|$)/i,
      /\b(?:find|open|select|choose)\s+(?:the\s+)?(?:email|message|conversation|thread|ticket|record|row|card|item|order)?\s*(?:by|from|for|with|containing|named|called)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})/,
      /\b(?:email|message|conversation|thread|ticket|record|row|card|item|order)\s+(?:by|from|for|with|containing|named|called)\s+([A-Z][A-Za-z0-9_'’-]*(?:\s+[A-Z][A-Za-z0-9_'’-]*){{0,3}})/
    ];
    for (const pattern of patterns) {{
      const match = instruction.match(pattern);
      if (match && match[1]) {{
        return match[1]
          .replace(/^(?:the\s+)?(?:user|account|person|profile|contact|customer)\s+/i, '')
          .replace(/^(?:please\s+)?(?:forward|send|reply|respond)\s+/i, '')
          .replace(/\b(?:and|then|reply|respond|forward|delete|remove|trash|archive|click|mark|star|important)\b.*$/i, '')
          .replace(/[.,"']+$/g, '')
          .trim();
      }}
    }}
    return null;
  }}
  function workflowItemCount() {{
    if (/\b(?:all|every|each)\s+(?:matching\s+)?(?:posts?|items?|rows?|cards?|records?|entries?)\b/i.test(instruction) ||
      /\b(?:on|for|in)\s+(?:all|every|each)\s+(?:matching\s+)?(?:posts?|items?|rows?|cards?|records?|entries?)\b/i.test(instruction)) {{
      return 'all';
    }}
    const match = instruction.match(/\b(?:on|for|in)?\s*(\d+)\s+(?:posts?|items?|rows?|cards?|records?|entries?)\b/i);
    if (!match) return null;
    const count = Number(match[1]);
    return Number.isFinite(count) && count > 0 ? count : null;
  }}
  function scopedItemWorkflowPlan() {{
    if (scopedMultiActionIntent()) return null;
    let actionHint = workflowActionHint();
    const itemQuery = workflowItemQuery();
    if (!itemQuery) return null;
    const itemCount = workflowItemCount();
    if (!actionHint && itemCount != null && kindIs('click')) {{
      const quoted = quotedInstructionValues();
      if (quoted.length && /\b(?:button|control|icon|action)\b/i.test(instruction)) actionHint = quoted[0];
    }}
    if (!actionHint) return null;
    const allItemCount = itemCount === 'all';
    const localActionRequest = kindIs('click') && itemCount != null && /\b(?:row|card|item|post|record|entry|by|from|for|with|containing)\b/i.test(instruction);
    const correspondenceRequest = /\b(?:email|e-mail|message|conversation|thread)\b/i.test(instruction) && /\b(?:by|from|sent|wants|waiting\s+for|forward|send|reply|respond|delete|remove|trash|star|important|turn\s+on|enable|check|tick|select|turn\s+off|disable|uncheck|untick|deselect)\b/i.test(instruction);
    const transferRequest = /\b(?:forward|send)\b/i.test(instruction) && /\b(?:by|from|sent|information|details|content|record|item)\b/i.test(instruction);
    if (!/\b(find|open|select|choose)\b/i.test(instruction) && !localActionRequest && !correspondenceRequest && !transferRequest) return null;
    const fillText = workflowFillText();
    const followHint = followUpClickHint();
    const followIsSameLocalAction = followHint && actionHint &&
      (tokenScore(actionHint, followHint) >= 0.4 || tokenScore(followHint, actionHint) >= 0.4);
    const completionHint = (followHint && !followIsSameLocalAction ? followHint : null) ||
      (fillText && /\b(reply|respond|forward)\b/i.test(actionHint) ? 'send' : null);
    return {{
      ok: true,
      action: 'scoped_item_workflow',
      params: {{
        itemQuery,
        actionHint,
        fillText,
        completionHint,
        itemCount: allItemCount ? null : itemCount,
        itemCountMode: allItemCount ? 'all' : null
      }},
      confidence: fillText ? 0.84 : 0.8,
      reason: 'planned generic scoped item workflow from item-local action instruction',
      evidence: {{ itemQuery, actionHint, itemCount, itemCountMode: allItemCount ? 'all' : null, hasFillText: !!fillText }}
    }};
  }}

  function discoverClickPlan() {{
    const follow = followUpClickHint();
    const revealVerb = /\b(expand|reveal|open|show)\b/i.test(instruction);
    const sameHint = follow && targetHint && normalized(follow) === normalized(targetHint);
    const targetHintIsCompletion = targetHint && /\b(submit|continue|confirm|save|done|next|ok)\b/i.test(targetHint);
    let trigger = follow && !sameHint && !targetHintIsCompletion ? targetHint : null;
    if (trigger && /^\s*(?:find|search|look\s+up)\b/i.test(instruction)) {{
      const quoted = quotedInstructionValues();
      if (quoted.length && normalized(trigger) === normalized(quoted[0])) {{
        trigger = 'search "' + quoted[0].replace(/"/g, '\\"') + '"';
      }}
    }}
    const target = follow || targetHint || (instruction.match(/\blink\s+["']?([^"'.]+)["']?/i) || [])[1];
    if (!target) return null;
    if (!/\b(find|switch|expand|reveal|open)\b/i.test(instruction)) return null;
    const revealFirst = revealVerb && !trigger;
    return {{
      ok: true,
      action: 'discover_click',
      params: {{ target, trigger, revealFirst }},
      confidence: 0.75,
      reason: 'instruction asks to reveal content and click a target by text'
    }};
  }}

  function parseDurationSeconds(text) {{
    const match = String(text || '').match(/(?:(\d+)\s*h(?:ours?)?)?\s*(?:(\d+)\s*m(?:in(?:ute)?s?)?)?/i);
    if (!match || (!match[1] && !match[2])) return null;
    return (Number(match[1] || 0) * 3600) + (Number(match[2] || 0) * 60);
  }}
  function statusLikeNumericContainer(el, text) {{
    const meta = [
      text || '',
      el.id || '',
      classText(el),
      el.getAttribute('role') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('data-testid') || '',
    ].join(' ');
    if (/\b(time\s+left|remaining\s+time|elapsed|timer|countdown|reward|status|scoreboard|progress|debug|telemetry|hud)\b/i.test(meta)) return true;
    return ['status', 'timer', 'progressbar', 'meter'].includes(roleOf(el));
  }}
  function targetLikeRankedItem(el) {{
    const tag = el.tagName.toLowerCase();
    const role = roleOf(el);
    const meta = [classText(el), el.id || '', el.getAttribute('data-testid') || '', el.getAttribute('aria-label') || ''].join(' ');
    return tag === 'button' || tag === 'a' || ['button', 'link', 'option'].includes(role) ||
      el.hasAttribute('onclick') || el.hasAttribute('tabindex') || el.hasAttribute('data-value') ||
      el.hasAttribute('data-index') || /\b(card|item|tile|row|entry|option|choice|result)\b/i.test(meta);
  }}
  function extremeClickPlan() {{
    if (!/\b(shortest|longest|lowest|highest|smallest|largest|greatest|max(?:imum)?|cheapest|most expensive)\b/i.test(instruction)) return null;
    const wantMin = /\b(shortest|lowest|smallest|cheapest)\b/i.test(instruction);
    const buttons = clickableElements().concat(all('[onclick], [tabindex], [role=button], .card, .item, .tile, li, tr, div'))
      .filter(el => visible(el))
      .filter((el, index, arr) => arr.indexOf(el) === index);
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
        : null) || button.closest('tr, li, [role="row"], [role="listitem"], .result, .item, .card, .row, section, article, div') || button.parentElement;
      if (!container) continue;
      const text = textOf(container);
      if (statusLikeNumericContainer(container, text) || statusLikeNumericContainer(button, textOf(button))) continue;
      let metric = /duration/i.test(instruction) ? parseDurationSeconds(text) : null;
      if (metric == null) {{
        const numbers = Array.from(String(text).matchAll(/-?\d+(?:\.\d+)?/g)).map(m => Number(m[0]));
        if (numbers.length) metric = numbers[0];
      }}
      if (metric != null && Number.isFinite(metric)) {{
        let score = 0;
        if (targetLikeRankedItem(button)) score += 0.6;
        if (targetLikeRankedItem(container)) score += 0.4;
        const rect = button.getBoundingClientRect();
        const area = Math.max(1, rect.width * rect.height);
        if (area > 20 && area < 80000) score += 0.2;
        if (score <= 0.2) continue;
        scored.push({{ button, metric, score }});
      }}
    }}
    if (!scored.length) return null;
    scored.sort((a, b) => {{
      const metricOrder = wantMin ? a.metric - b.metric : b.metric - a.metric;
      return metricOrder || b.score - a.score;
    }});
    const chosen = scored[0];
    const primary = {{
      ok: true,
      action: 'click',
      params: {{ selector: selector(chosen.button) }},
      confidence: 0.82,
      reason: 'matched repeated item with requested extreme numeric or duration value',
      candidate: candidate(chosen.button),
      metric: chosen.metric
    }};
    return withFollowUp(primary, chosen.button);
  }}

  function parseCssRgb(value) {{
    const match = String(value || '').match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?/i);
    if (!match) return null;
    const alpha = match[4] == null ? 1 : Number(match[4]);
    if (!Number.isFinite(alpha) || alpha <= 0.05) return null;
    return {{ r: Number(match[1]), g: Number(match[2]), b: Number(match[3]), a: alpha }};
  }}
  function rgbHue({{ r, g, b }}) {{
    r /= 255; g /= 255; b /= 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const delta = max - min;
    if (delta === 0) return {{ hue: 0, saturation: 0, lightness: (max + min) / 2 }};
    let hue = 0;
    if (max === r) hue = 60 * (((g - b) / delta) % 6);
    else if (max === g) hue = 60 * (((b - r) / delta) + 2);
    else hue = 60 * (((r - g) / delta) + 4);
    if (hue < 0) hue += 360;
    const lightness = (max + min) / 2;
    const saturation = delta / (1 - Math.abs(2 * lightness - 1));
    return {{ hue, saturation, lightness }};
  }}
  function colorFamilyMatch(rgb, family) {{
    if (!rgb) return false;
    const {{ hue, saturation, lightness }} = rgbHue(rgb);
    if (family === 'black') return lightness < 0.18;
    if (family === 'white') return lightness > 0.88 && saturation < 0.25;
    if (family === 'gray' || family === 'grey') return saturation < 0.18 && lightness >= 0.18 && lightness <= 0.88;
    if (saturation < 0.18) return false;
    const ranges = {{
      red: [[345, 360], [0, 20]],
      scarlet: [[345, 360], [0, 20]],
      orange: [[20, 45]],
      yellow: [[45, 75]],
      olive: [[50, 90]],
      lime: [[75, 105]],
      green: [[80, 165]],
      cyan: [[165, 205]],
      aqua: [[165, 205]],
      teal: [[165, 205]],
      blue: [[205, 265]],
      navy: [[205, 265]],
      indigo: [[240, 275]],
      purple: [[265, 320]],
      violet: [[275, 320]],
      magenta: [[300, 345]],
      pink: [[320, 350]],
      brown: [[15, 45]],
      gold: [[42, 58]],
    }}[family] || [];
    return ranges.some(([from, to]) => hue >= from && hue <= to);
  }}
  function colorNameFromText(text) {{
    const match = String(text || '').match(/\b(red|scarlet|orange|yellow|olive|lime|green|cyan|aqua|teal|blue|navy|indigo|purple|violet|magenta|pink|brown|gold|black|white|gray|grey|silver)\b/i);
    return match ? match[1].toLowerCase() : null;
  }}
  function colorNameFromRgb(rgb) {{
    if (!rgb) return null;
    const hsl = rgbHue(rgb);
    if (hsl.lightness < 0.18) return 'black';
    if (hsl.lightness > 0.88 && hsl.saturation < 0.25) return 'white';
    if (hsl.saturation < 0.18) return 'gray';
    if (hsl.hue >= 45 && hsl.hue <= 75 && hsl.lightness < 0.38) return 'olive';
    const families = ['red', 'orange', 'yellow', 'lime', 'green', 'cyan', 'blue', 'purple', 'magenta', 'pink', 'brown', 'gold'];
    return families.find(family => colorFamilyMatch(rgb, family)) || null;
  }}
  function colorNameFromElement(el) {{
    if (!el) return null;
    const metadataColor = colorNameFromText([
      el.getAttribute('data-color') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('title') || '',
      el.getAttribute('fill') || '',
      el.getAttribute('stroke') || '',
      el.getAttribute('style') || '',
      classText(el),
      el.id || ''
    ].join(' '));
    if (metadataColor) return metadataColor;
    return colorNameFromRgb(elementVisualRgb(el));
  }}
  function promptColorHint() {{
    if (!/\b(colou?red|colou?r|swatch|sample|shown|box|tile|square|shape)\b/i.test(instruction)) return null;
    const promptRoots = all('#query, [data-role=query], .query, [aria-label*=query i], [aria-label*=instruction i], [class*=prompt i], [id*=prompt i]')
      .filter(visible);
    for (const root of promptRoots) {{
      const candidates = all('span, div, i, b, em, strong, svg circle, svg rect, svg path, svg polygon, svg ellipse', root)
        .filter(el => visible(el))
        .map(el => {{
          const rect = el.getBoundingClientRect();
          return {{ el, rect, color: colorNameFromElement(el) }};
        }})
        .filter(item => item.color && item.rect.width >= 4 && item.rect.height >= 4 && item.rect.width <= 80 && item.rect.height <= 80)
        .sort((a, b) => (a.rect.width * a.rect.height) - (b.rect.width * b.rect.height));
      if (candidates.length) return candidates[0].color;
    }}
    return null;
  }}
  function namedColorHex(name) {{
    const colors = {{
      red: '#ff0000',
      scarlet: '#ff2400',
      orange: '#ffa500',
      yellow: '#ffff00',
      olive: '#808000',
      lime: '#00ff00',
      green: '#008000',
      cyan: '#00ffff',
      aqua: '#00ffff',
      teal: '#008080',
      blue: '#0000ff',
      navy: '#000080',
      indigo: '#4b0082',
      purple: '#800080',
      violet: '#8f00ff',
      magenta: '#ff00ff',
      pink: '#ffc0cb',
      brown: '#a52a2a',
      gold: '#ffd700',
      black: '#000000',
      white: '#ffffff',
      gray: '#808080',
      grey: '#808080',
      silver: '#c0c0c0',
    }};
    return colors[String(name || '').toLowerCase()] || null;
  }}
  function normalizeCssHexColor(value) {{
    const raw = String(value || '').trim().replace(/^#/, '').toLowerCase();
    if (/^[0-9a-f]{{3}}$/.test(raw)) {{
      return '#' + raw.split('').map(ch => ch + ch).join('');
    }}
    if (/^[0-9a-f]{{6}}$/.test(raw)) {{
      return '#' + raw;
    }}
    return null;
  }}
  function requestedColorLiteral() {{
    const hexMatch = String(instruction || '').match(/#([0-9a-f]{{3}}|[0-9a-f]{{6}})\b/i);
    if (hexMatch) {{
      const hex = normalizeCssHexColor(hexMatch[1]);
      if (hex) return {{ label: hex, hex, source: 'hex' }};
    }}
    const nameMatch = String(instruction || '').match(/\b(red|scarlet|orange|yellow|olive|lime|green|cyan|aqua|teal|blue|navy|indigo|purple|violet|magenta|pink|brown|gold|black|white|gray|grey|silver)\b/i);
    if (!nameMatch) return null;
    const name = nameMatch[1].toLowerCase();
    const hex = namedColorHex(name);
    if (!hex) return null;
    return {{ label: name, hex, source: 'name' }};
  }}
  function elementVisualRgb(el) {{
    const style = getComputedStyle(el);
    const candidates = [
      style.backgroundColor,
      el.getAttribute('fill'),
      style.fill,
      el.getAttribute('stroke'),
      style.stroke,
      el.style && el.style.backgroundColor,
    ];
    for (const candidate of candidates) {{
      const rgb = parseCssRgb(candidate);
      if (rgb) return rgb;
    }}
    return null;
  }}
  function colorPickerInputPlan() {{
    if (!/\b(?:colou?r\s*picker|picker|colou?r\s*field|colou?r\s*input|select|choose|pick|set)\b/i.test(instruction)) return null;
    const requestedColor = requestedColorLiteral();
    if (!requestedColor) return null;
    const wantedColor = requestedColor.label;
    const hex = requestedColor.hex;
    const fields = best(writableFields(), el => {{
      if (!writableField(el)) return 0;
      const tag = el.tagName.toLowerCase();
      const type = typeOf(el);
      const customValueHost = isCustomWritableValueElement(el);
      if (tag !== 'input' && tag !== 'textarea' && !el.isContentEditable && roleOf(el) !== 'textbox' && roleOf(el) !== 'combobox' && !customValueHost) return 0;
      if (['button', 'submit', 'checkbox', 'radio', 'file', 'hidden', 'range'].includes(type)) return 0;
      const text = [
        textOf(el),
        classText(el),
        el.id || '',
        el.getAttribute('name') || '',
        el.getAttribute('data-jscolor') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || ''
      ].join(' ');
      let score = 0;
      if (type === 'color') score += 0.95;
      if (customValueHost) score += 0.15;
      if (el.hasAttribute('data-jscolor') || /\bjscolor\b/i.test(classText(el))) score += 0.9;
      if (/\bcolou?r|picker|palette|hex|rgb\b/i.test(text)) score += 0.55;
      if (/\bcolou?r\s*picker\b/i.test(instruction)) score += 0.2;
      return score;
    }});
    if (!fields.length || fields[0].score < 0.5) return null;
    const field = fields[0].el;
    const type = typeOf(field);
    const className = classText(field);
    const usesHash = type === 'color' || field.value === '' || String(field.value || '').trim().startsWith('#');
    const textValue = usesHash && !/\bjscolor\b/i.test(className) && !field.hasAttribute('data-jscolor')
      ? hex
      : hex.replace(/^#/, '');
    const primary = {{
      ok: true,
      action: 'type',
      params: {{ selector: selector(field), text: textValue, clear_first: true }},
      confidence: Math.min(1, fields[0].score),
      reason: 'matched text-backed color picker input from semantic field metadata',
      candidate: candidate(field),
      evidence: {{ color: wantedColor, hex, typedValue: textValue, inputType: type || null, source: requestedColor.source }}
    }};
    const follow = clickStepForHint(followUpClickHint() || (/\bsubmit|hit\b/i.test(instruction) ? 'submit' : null), field)
      || (/\bsubmit|hit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(field) : null);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.82, follow.confidence || 0.65),
      reason: 'planned color picker value entry plus completion control',
      evidence: primary.evidence
    }};
  }}
  function visualColorSelectionPlan() {{
    const explicitColor = colorNameFromText(instruction);
    const inferredPromptColor = explicitColor ? null : promptColorHint();
    const wantedColor = explicitColor || inferredPromptColor;
    if (!wantedColor) return null;
    const wantsPluralSelection = /\b(all|every|shades?|colou?rs?)\b/i.test(instruction);
    const hasSizeQualifier = /\b(small|smaller|smallest|tiny|little|large|larger|largest|big|bigger|biggest)\b/i.test(instruction);
    const hasVisualTargetHint = /\b(all|every|shades?|colou?rs?|swatches?|tiles?|squares?|rectangles?|circles?|dots?|round|shapes?|box(?:es)?|cells?|items?|options?)\b/i.test(instruction);
    if (!/\b(select|click|choose|pick)\b/i.test(instruction) || !hasVisualTargetHint) return null;
    if (hasSizeQualifier && !wantsPluralSelection) return null;
    const shapeMatch = instruction.match(/\b(swatch|tile|square|rectangle|circle|dot|round|shape|box|cell|item|option)\b/i);
    const shapeHint = shapeMatch ? shapeMatch[1].toLowerCase() : null;
    function visualShapeScore(el) {{
      if (!shapeHint) return 0;
      const tag = el.tagName.toLowerCase();
      const rect = el.getBoundingClientRect();
      const aspect = rect.width > 0 && rect.height > 0 ? rect.width / rect.height : 1;
      let score = Math.max(tokenScore(shapeHint, textOf(el)), exactPhraseScore(shapeHint, textOf(el)));
      if (shapeHint === 'circle' || shapeHint === 'dot' || shapeHint === 'round') {{
        if (tag === 'circle' || tag === 'ellipse') score += 0.35;
        try {{
          const radius = getComputedStyle(el).borderRadius || '';
          if (radius && radius !== '0px') score += 0.2;
        }} catch (_) {{}}
      }}
      if (shapeHint === 'square' || shapeHint === 'tile' || shapeHint === 'swatch' || shapeHint === 'box' || shapeHint === 'cell') {{
        if (tag === 'rect') score += 0.2;
        if (aspect >= 0.75 && aspect <= 1.33) score += 0.18;
      }}
      if (shapeHint === 'rectangle' && (tag === 'rect' || aspect > 1.35 || aspect < 0.74)) score += 0.25;
      if (shapeHint === 'shape' || shapeHint === 'item' || shapeHint === 'option') score += 0.05;
      return score;
    }}
    const rawCandidates = all('span, div, li, td, button, [role=button], [onclick], [tabindex], svg circle, svg rect, svg path, svg polygon, svg ellipse')
      .filter(el => {{
        if (!visible(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['body', 'html', 'script', 'style'].includes(tag)) return false;
        if (tag === 'button' || typeOf(el) === 'submit') return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 4 || rect.height < 4) return false;
        return true;
      }});
    const colored = rawCandidates.map(el => {{
      const dataColor = String(el.getAttribute('data-color') || el.getAttribute('aria-label') || el.getAttribute('title') || '').toLowerCase();
      const rgb = elementVisualRgb(el);
      const matchesData = dataColor.split(/[^a-z]+/).includes(wantedColor);
      const matchesVisual = colorFamilyMatch(rgb, wantedColor);
      return {{ el, rgb, matchesData, matchesVisual, matches: matchesData || matchesVisual }};
    }}).filter(item => item.rgb || item.matches);
    const exactMetadataMatches = colored.filter(item => item.matchesData);
    const matchingSource = exactMetadataMatches.length ? exactMetadataMatches : colored;
    const matching = matchingSource
      .filter(item => item.matches)
      .filter(item => !matchingSource.some(other => other.el !== item.el && item.el.contains(other.el) && other.matches));
    if (!matching.length || matching.length > 30) return null;
    let selected = matching;
    let confidence = 0.78;
    if (!wantsPluralSelection) {{
      const ranked = matching.map(item => {{
        const text = textOf(item.el);
        let score = 0.62 + visualShapeScore(item.el);
        if (targetHint) score += Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
        if (matching.length === 1) score += 0.2;
        return {{ item, score }};
      }}).sort((a, b) => b.score - a.score);
      if (!ranked.length) return null;
      if (matching.length > 1 && !shapeHint && !targetHint) return null;
      if (matching.length > 1 && ranked[0].score - ranked[1].score < 0.08) return null;
      selected = [ranked[0].item];
      confidence = Math.min(1, ranked[0].score);
    }}
    const steps = selected.map(item => ({{
      action: 'click',
      params: {{ selector: selector(item.el) }},
      confidence,
      reason: wantsPluralSelection
        ? 'matched visible elements rendered in requested color family'
        : 'matched visible element rendered in requested color family and visual target hint',
      candidate: candidate(item.el)
    }}));
    const anchor = selected[selected.length - 1].el;
    const follow = clickStepForHint(followUpClickHint() || (/\bsubmit\b/i.test(instruction) ? 'submit' : null), anchor)
      || (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(anchor) : null);
    if (follow) steps.push(follow);
    if (!steps.length) return null;
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence,
      reason: 'planned visual color-family selection from rendered swatches or shapes',
      evidence: {{ color: wantedColor, matched: selected.length, matchingCandidates: matching.length, coloredCandidates: colored.length, mode: wantsPluralSelection ? 'plural' : 'single', shapeHint }}
    }};
  }}

  function visualFeedbackSearchPlan() {{
    if (!kindIs('click')) return null;
    if (!/\b(?:find|locate|search|identify|discover|click|tap|press)\b/i.test(instruction)) return null;
    if (!/\b(?:area|region|zone|spot|point|location|place|surface|target)\b/i.test(instruction)) return null;
    const feedbackMatch = instruction.match(/\b(ice\s+cold|hot|warm|cold|success|correct|good|yes)\b/i);
    if (!feedbackMatch) return null;
    const targetFeedback = feedbackMatch[1].toLowerCase().replace(/\s+/g, ' ').trim();

    function surfaceLabel(el) {{
      return [
        el.id || '',
        classText(el),
        el.getAttribute('role') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        el.getAttribute('data-testid') || '',
        el.getAttribute('data-surface') || '',
        el.getAttribute('data-target') || '',
        el.getAttribute('data-area') || '',
        semanticAttributeText(el)
      ].join(' ');
    }}
    function directSurfaceText(el) {{
      return Array.from(el.childNodes || [])
        .filter(node => node.nodeType === Node.TEXT_NODE)
        .map(node => node.textContent || '')
        .join(' ')
        .replace(/\s+/g, ' ')
        .trim();
    }}
    function surfaceScore(el) {{
      const rect = el.getBoundingClientRect();
      if (rect.width < 32 || rect.height < 24) return 0;
      const area = rect.width * rect.height;
      if (area < 800 || area > 500000) return 0;
      const tag = el.tagName.toLowerCase();
      if (['html', 'body', 'script', 'style', 'input', 'textarea', 'select', 'button', 'a'].includes(tag)) return 0;
      const label = surfaceLabel(el);
      const text = directSurfaceText(el);
      let score = 0.34;
      if (/\b(?:surface|canvas|area|target|touch|hit|hotspot|board|drawing|map|field|zone|region)\b/i.test(label)) score += 0.34;
      if (el.onmousemove || el.onpointermove || el.onmouseover || el.onclick || el.getAttribute('onmousemove') || el.getAttribute('onclick')) score += 0.2;
      try {{
        const cursor = getComputedStyle(el).cursor;
        if (cursor === 'pointer' || cursor === 'crosshair') score += 0.12;
      }} catch (_) {{}}
      if (tag === 'canvas' || tag === 'svg') score += 0.18;
      if (text.length <= 20) score += 0.1;
      if (rect.width >= 60 && rect.height >= 50) score += 0.08;
      if (/\b(?:query|prompt|instruction|question)\b/i.test(label)) score -= 0.4;
      return score;
    }}

    const surfaces = all('canvas, svg, [role=application], [role=img], [data-surface], [data-target], [data-area], [data-canvas], [class*=surface], [class*=Surface], [class*=canvas], [class*=Canvas], [class*=area], [class*=Area], [class*=target], [class*=Target], [class*=board], [class*=Board], div, section')
      .filter(visible)
      .map(el => {{ return {{ el, score: surfaceScore(el) }}; }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    if (!surfaces.length || surfaces[0].score < 0.5) return null;
    const chosen = surfaces[0];
    return {{
      ok: true,
      action: 'visual_feedback_search',
      params: {{ selector: selector(chosen.el), targetFeedback }},
      confidence: Math.min(0.92, chosen.score),
      reason: 'planned pointer exploration over a visual surface using live textual feedback',
      candidate: candidate(chosen.el),
      evidence: {{
        targetFeedback,
        surfaceCandidates: surfaces.length,
        surfaceScore: chosen.score
      }}
    }};
  }}

  function treeSearchClickPlan() {{
    if (!kindIs('click')) return null;
    if (!/\b(?:tree|hierarchy|outline|folder|file|directory|nested|expand|collapse)\b/i.test(instruction)) return null;
    const quoted = instruction.match(/"([^"]+)"/) || instruction.match(/'([^']+)'/);
    const named = instruction.match(/\bnamed\s+([A-Za-z0-9_.-]+)\b/i) ||
      instruction.match(/\bcalled\s+([A-Za-z0-9_.-]+)\b/i) ||
      instruction.match(/\blabel(?:ed|led)\s+([A-Za-z0-9_.-]+)\b/i);
    const target = quoted ? quoted[1].trim() : named ? named[1].trim() : (targetHint || '').trim();
    if (!target) return null;

    function treeRootScore(el) {{
      const rect = el.getBoundingClientRect();
      if (rect.width < 20 || rect.height < 10) return 0;
      const itemCount = el.querySelectorAll ? el.querySelectorAll('li, [role=treeitem], [role=row], [aria-expanded]').length : 0;
      if (itemCount < 2) return 0;
      const meta = [
        el.id || '',
        classText(el),
        el.getAttribute('role') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        semanticAttributeText(el)
      ].join(' ');
      let score = 0.35;
      if (/\b(?:tree|filetree|folder|hierarchy|outline|directory|nav)\b/i.test(meta)) score += 0.45;
      if (el.getAttribute('role') === 'tree' || el.getAttribute('role') === 'treegrid') score += 0.25;
      score += Math.min(0.2, itemCount / 50);
      return score;
    }}

    const roots = all('[role=tree], [role=treegrid], .tree, .treeview, .filetree, [class*=tree], [class*=Tree], ul, ol')
      .filter(el => visible(el) || el.querySelector('[aria-expanded], .hitarea, [class*=expandable], [class*=collapsed]'))
      .map(el => {{ return {{ el, score: treeRootScore(el) }}; }})
      .filter(item => item.score > 0)
      .sort((a, b) => b.score - a.score);
    if (!roots.length || roots[0].score < 0.5) return null;
    return {{
      ok: true,
      action: 'tree_search_click',
      params: {{ selector: selector(roots[0].el), target }},
      confidence: Math.min(0.94, roots[0].score),
      reason: 'planned hierarchical tree expansion and exact label click',
      candidate: candidate(roots[0].el),
      evidence: {{
        target,
        treeCandidates: roots.length,
        treeScore: roots[0].score
      }}
    }};
  }}

  function itemQuantitySelectionPlan() {{
    if (!/\b(?:order|add|select|choose|pick)\b/i.test(instruction)) return null;
    if (!/\b(?:one\s+of\s+each|each\s+item|items?|quantity|quantities|cart|order)\b/i.test(instruction)) return null;
    const normalizeItemText = value => String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
    let itemText = null;
    const colon = instruction.match(/:\s*(.+)$/);
    if (colon) itemText = colon[1];
    if (!itemText) {{
      const match = instruction.match(/\b(?:order|add|select|choose|pick)\s+(?:one\s+of\s+each\s+)?(?:items?\s*)?(.+)$/i);
      if (match) itemText = match[1];
    }}
    if (!itemText) return null;
    itemText = itemText
      .replace(/\b(?:and\s+)?(?:press|click|tap|hit)\s+(?:submit|order|done|continue|save).*/i, '')
      .trim();
    const requested = itemText
      .split(/\s*,\s*|\s+\band\b\s+/i)
      .map(value => value.replace(/^["'\s]+|["'.\s]+$/g, '').trim())
      .filter(value => value.length >= 2 && !/\b(?:one|each|item|items)\b/i.test(value));
    if (!requested.length || requested.length > 12) return null;

    function rowText(el) {{
      return [
        textOf(el),
        el.getAttribute('data-item') || '',
        el.getAttribute('data-name') || '',
        el.getAttribute('data-label') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || ''
      ].join(' ').replace(/\s+/g, ' ').trim();
    }}
    function meaningfulRow(el) {{
      const rect = el.getBoundingClientRect();
      if (rect.width < 20 || rect.height < 8) return false;
      const tag = el.tagName.toLowerCase();
      if (['html', 'body', 'script', 'style', 'button', 'a', 'input', 'select', 'textarea'].includes(tag)) return false;
      if (tag === 'span' && !el.matches('[role=listitem], [onclick], [tabindex], [data-item], [data-name], [data-label]')) return false;
      return true;
    }}
    function bestRowFor(name) {{
      const wanted = normalizeItemText(name);
      const candidates = all('[data-item], [data-name], [data-label], li, tr, [role=listitem], [class*=item], [class*=Item], [class*=row], [class*=Row], div')
        .filter(el => visible(el) && meaningfulRow(el))
        .map(el => {{
          const text = rowText(el);
          const normalizedText = normalizeItemText(text);
          const dataItem = [el.getAttribute('data-item') || '', el.getAttribute('data-name') || '', el.getAttribute('data-label') || ''].join(' ');
          const normalizedData = normalizeItemText(dataItem);
          let score = 0;
          if (normalizedData && normalizedData === wanted) score += 1.2;
          if (normalizedText === wanted) score += 0.9;
          else if (normalizedText.includes(wanted)) score += 0.7;
          const wantedTokens = wanted.split(/\s+/).filter(Boolean);
          if (wantedTokens.length && wantedTokens.every(token => normalizedText.includes(token))) score = Math.max(score, 0.55);
          if (el.querySelector('button, a, [role=button], [onclick], [tabindex], .add, [class*=add], [aria-label*=add], [title*=add]')) score += 0.1;
          return {{ el, score }};
        }})
        .filter(item => item.score > 0.45)
        .filter(item => !all('[data-item], [data-name], [data-label], li, tr, [role=listitem], [class*=item], [class*=Item], [class*=row], [class*=Row], div', item.el)
          .some(child => child !== item.el &&
            visible(child) &&
            meaningfulRow(child) &&
            normalizeItemText(rowText(child)).includes(wanted) &&
            !!child.querySelector('button, a, [role=button], [onclick], [tabindex], .add, [class*=add], [aria-label*=add], [title*=add]')))
        .sort((a, b) => b.score - a.score);
      return candidates[0] || null;
    }}
    function addControl(row) {{
      const controls = Array.from(row.querySelectorAll('button, a, [role=button], [onclick], [tabindex], span, div'))
        .filter(visible)
        .map(el => {{
          const text = [directTextOf(el), textOf(el), classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
          let score = 0;
          if (/^\s*\+\s*$/.test(directTextOf(el)) || /\b(?:add|plus|increase|increment|more)\b/i.test(text)) score += 1;
          if (/\b(?:remove|minus|decrease|delete|trash)\b/i.test(text) || /^\s*-\s*$/.test(directTextOf(el))) score -= 1;
          return {{ el, score }};
        }})
        .filter(item => item.score > 0)
        .sort((a, b) => b.score - a.score);
      if (controls[0]) return controls[0].el;
      return row.matches('button, a, [role=button], [onclick], [tabindex]') ? row : null;
    }}

    const steps = [];
    const matched = [];
    for (const name of requested) {{
      const row = bestRowFor(name);
      if (!row) return null;
      const control = addControl(row.el);
      if (!control) return null;
      matched.push({{ name, row: row.el, score: row.score }});
      steps.push({{
        action: 'click',
        params: {{ selector: selector(control) }},
        confidence: Math.min(0.95, row.score),
        reason: 'selected requested item by matching its row and add/increase control',
        candidate: candidate(control),
        evidence: {{ item: name, row: candidate(row.el) }}
      }});
    }}
    const submit = clickStepForHint(followUpClickHint() || 'order', matched[matched.length - 1].row)
      || clickStepForHint('submit', matched[matched.length - 1].row)
      || completionClickStep(matched[matched.length - 1].row);
    if (submit) steps.push(submit);
    if (steps.length < requested.length) return null;
    const confidence = Math.min(0.94, matched.reduce((sum, item) => sum + Math.min(0.95, item.score), 0) / matched.length);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence,
      reason: 'planned item quantity selection from requested item names',
      evidence: {{ requested, matched: matched.map(item => item.name) }}
    }};
  }}

  function binaryRowClassificationPlan() {{
    if (!/\b(?:mark|classify|label|tag|set|choose|select|assign)\b/i.test(instruction)) return null;
    const pairMatch = instruction.match(/\b(odd|even|positive|negative|yes|no|true|false|pass|fail)\s+or\s+(odd|even|positive|negative|yes|no|true|false|pass|fail)\b/i);
    if (!pairMatch) return null;
    const labels = [pairMatch[1].toLowerCase(), pairMatch[2].toLowerCase()];
    if (labels[0] === labels[1]) return null;

    function classifyValue(text) {{
      const value = String(text || '').replace(/\s+/g, ' ').trim();
      const numberMatch = value.match(/-?\d+(?:\.\d+)?/);
      const lower = value.toLowerCase();
      if (labels.includes('odd') && labels.includes('even') && numberMatch) {{
        const number = Number(numberMatch[0]);
        if (Number.isFinite(number)) return Math.abs(number % 2) === 1 ? 'odd' : 'even';
      }}
      if (labels.includes('positive') && labels.includes('negative') && numberMatch) {{
        const number = Number(numberMatch[0]);
        if (Number.isFinite(number) && number !== 0) return number > 0 ? 'positive' : 'negative';
      }}
      if (labels.includes('true') && labels.includes('false')) {{
        if (/\b(?:true|yes|valid|enabled|active|on|pass(?:ed)?)\b/i.test(lower)) return 'true';
        if (/\b(?:false|no|invalid|disabled|inactive|off|fail(?:ed)?)\b/i.test(lower)) return 'false';
      }}
      if (labels.includes('yes') && labels.includes('no')) {{
        if (/\b(?:true|yes|valid|enabled|active|on|pass(?:ed)?)\b/i.test(lower)) return 'yes';
        if (/\b(?:false|no|invalid|disabled|inactive|off|fail(?:ed)?)\b/i.test(lower)) return 'no';
      }}
      if (labels.includes('pass') && labels.includes('fail')) {{
        if (/\b(?:true|yes|valid|enabled|active|on|pass(?:ed)?)\b/i.test(lower)) return 'pass';
        if (/\b(?:false|no|invalid|disabled|inactive|off|fail(?:ed)?)\b/i.test(lower)) return 'fail';
      }}
      return null;
    }}
    function rowValueText(row) {{
      const preferred = Array.from(row.querySelectorAll ? row.querySelectorAll('[data-value], [data-number], [class*=value], [class*=Value], [class*=number], [class*=Number], output, strong, b, span, div') : [])
        .filter(visible)
        .map(el => {{
          const text = [
            directTextOf(el),
            el.getAttribute('data-value') || '',
            el.getAttribute('data-number') || '',
            el.getAttribute('aria-label') || '',
            el.getAttribute('title') || ''
          ].join(' ').replace(/\s+/g, ' ').trim();
          const controlish = el.matches('button, a, [role=button], input, select, textarea') ||
            !!el.closest('button, a, [role=button]');
          return {{ text, controlish }};
        }})
        .filter(item => item.text && !item.controlish)
        .sort((a, b) => Number(/\d/.test(b.text)) - Number(/\d/.test(a.text)) || a.text.length - b.text.length);
      if (preferred.length) return preferred[0].text;
      return directTextOf(row) || textOf(row);
    }}
    function optionControl(row, label) {{
      const wanted = label.toLowerCase();
      const controls = Array.from(row.querySelectorAll ? row.querySelectorAll('button, input, a, [role=button], [role=radio], [role=checkbox], [onclick], [tabindex]') : [])
        .filter(visible)
        .map(el => {{
          const text = [
            directTextOf(el),
            classText(el),
            el.getAttribute('value') || '',
            el.getAttribute('name') || '',
            el.getAttribute('aria-label') || '',
            el.getAttribute('title') || ''
          ].join(' ').toLowerCase();
          const textTokens = text.split(/[^a-z0-9]+/).filter(Boolean);
          let score = 0;
          if (textTokens.includes(wanted)) score += 1;
          if (el.matches('button, [role=button], [onclick], [tabindex]')) score += 0.08;
          return {{ el, score }};
        }})
        .filter(item => item.score > 0)
        .sort((a, b) => b.score - a.score);
      return controls[0] && controls[0].el;
    }}
    function rowCandidates() {{
      return all('[role=row], tr, li, [role=listitem], [data-row], [class*=row], [class*=Row], .item, [class*=item], [class*=Item], div')
        .filter(el => {{
          if (!visible(el)) return false;
          const tag = el.tagName.toLowerCase();
          if (['html', 'body', 'script', 'style', 'button', 'a', 'input', 'select', 'textarea'].includes(tag)) return false;
          const rect = el.getBoundingClientRect();
          if (rect.width < 40 || rect.height < 10 || rect.width * rect.height > 120000) return false;
          const hasBothControls = labels.every(label => optionControl(el, label));
          if (!hasBothControls) return false;
          return !!classifyValue(rowValueText(el));
        }});
    }}

    const rows = rowCandidates()
      .filter(row => !rowCandidates().some(other => other !== row && row.contains(other)))
      .sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      }});
    if (!rows.length || rows.length > 30) return null;
    const steps = [];
    for (const row of rows) {{
      const valueText = rowValueText(row);
      const label = classifyValue(valueText);
      if (!label) return null;
      const control = optionControl(row, label);
      if (!control) return null;
      steps.push({{
        action: 'click',
        params: {{ selector: selector(control) }},
        confidence: 0.86,
        reason: 'matched row value and selected matching binary option',
        candidate: candidate(control),
        evidence: {{ value: valueText, classification: label, row: candidate(row) }}
      }});
    }}
    const follow = clickStepForHint(followUpClickHint() || 'submit', rows[rows.length - 1])
      || completionClickStep(rows[rows.length - 1]);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: 0.86,
      reason: 'planned repeated binary classification for visible rows',
      evidence: {{ labels, rowCount: rows.length }}
    }};
  }}

  function visualObjectClickPlan() {{
    if (kind !== 'click') return null;
    const explicitColor = colorNameFromText(instruction);
    const wantedColor = explicitColor || promptColorHint();
    const sizeMatch = instruction.match(/\b(small|smaller|smallest|tiny|little|large|larger|largest|big|bigger|biggest)\b/i);
    const wantedSize = sizeMatch ? sizeMatch[1].toLowerCase() : null;
    const shapeMatch = instruction.match(/\b(circle|dot|round|square|rectangle|rect|box|tile|cell|triangle|polygon|path|line|shape|object|item|symbol|text|letter|number|digit)\b/i);
    const shapeHint = shapeMatch ? shapeMatch[1].toLowerCase() : null;
    const wantsCenter = /\bcent(?:er|re)\b/i.test(instruction);
    const hasVisualCue = !!(wantedColor || wantedSize || shapeHint || wantsCenter);
    if (!hasVisualCue) return null;
    if ((shapeHint === 'item' || shapeHint === 'object') && !wantedColor && !wantedSize && !wantsCenter && !/\b(?:visual|shape|shaped|drawn|rendered|colou?red)\b/i.test(instruction)) return null;

    const stop = new Set([
      'click', 'tap', 'press', 'on', 'the', 'a', 'an', 'of', 'in', 'at', 'to', 'and', 'then',
      'colored', 'coloured', 'color', 'colour', 'center', 'centre',
      'small', 'smaller', 'smallest', 'tiny', 'little', 'large', 'larger', 'largest', 'big', 'bigger', 'biggest',
      'red', 'scarlet', 'orange', 'yellow', 'olive', 'lime', 'green', 'cyan', 'aqua', 'teal', 'blue', 'navy',
      'indigo', 'purple', 'violet', 'magenta', 'pink', 'brown', 'gold', 'black', 'white', 'gray', 'grey', 'silver',
      'circle', 'dot', 'round', 'square', 'rectangle', 'rect', 'box', 'tile', 'cell', 'triangle', 'polygon',
      'path', 'line', 'shape', 'object', 'item', 'symbol', 'text', 'letter', 'number', 'digit'
    ]);
    const targetTextTokens = tokens(stripFollowUp(targetHint || instruction))
      .filter(token => !stop.has(token) && (token.length > 1 || /^[a-z0-9]$/i.test(token)));
    const targetText = targetTextTokens.join(' ');

    function shapeNameOf(el) {{
      const tag = el.tagName.toLowerCase();
      const cue = [
        tag,
        classText(el),
        el.getAttribute('data-shape') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        el.getAttribute('role') || ''
      ].join(' ').toLowerCase();
      if (tag === 'circle' || tag === 'ellipse') return 'circle';
      if (tag === 'rect') {{
        const rect = el.getBoundingClientRect();
        const aspect = rect.width > 0 && rect.height > 0 ? rect.width / rect.height : 1;
        return aspect >= 0.78 && aspect <= 1.28 ? 'square' : 'rectangle';
      }}
      if (tag === 'polygon') {{
        const points = String(el.getAttribute('points') || '').trim().split(/\s+/).filter(Boolean);
        if (points.length === 3) return 'triangle';
        return 'polygon';
      }}
      if (tag === 'path') return cue.includes('triangle') ? 'triangle' : 'path';
      if (tag === 'line') return 'line';
      if (tag === 'text' || tag === 'tspan') return 'text';
      const rect = el.getBoundingClientRect();
      const aspect = rect.width > 0 && rect.height > 0 ? rect.width / rect.height : 1;
      const radius = getComputedStyle(el).borderRadius || '';
      if (/\b(circle|dot|round)\b/.test(cue) || (radius && radius !== '0px' && aspect >= 0.78 && aspect <= 1.28)) return 'circle';
      if (/\b(triangle)\b/.test(cue)) return 'triangle';
      if (/\b(square|box|tile|cell)\b/.test(cue) || (aspect >= 0.78 && aspect <= 1.28 && rect.width <= 180 && rect.height <= 180)) return 'square';
      if (/\b(rectangle|rect)\b/.test(cue) || aspect > 1.28 || aspect < 0.78) return 'rectangle';
      return 'object';
    }}
    function shapeMatches(wanted, actual) {{
      if (!wanted) return true;
      if (wanted === actual) return true;
      if ((wanted === 'circle' || wanted === 'dot' || wanted === 'round') && actual === 'circle') return true;
      if ((wanted === 'square' || wanted === 'box' || wanted === 'tile' || wanted === 'cell') && actual === 'square') return true;
      if ((wanted === 'rectangle' || wanted === 'rect') && actual === 'rectangle') return true;
      if ((wanted === 'number' || wanted === 'digit' || wanted === 'letter' || wanted === 'text' || wanted === 'symbol') && actual === 'text') return true;
      if (wanted === 'item') return true;
      if ((wanted === 'shape' || wanted === 'object') && actual !== 'text') return true;
      return false;
    }}
    function textKindMatches(wanted, text) {{
      if (!wanted || !['number', 'digit', 'letter', 'text', 'symbol'].includes(wanted)) return true;
      const value = String(text || '').trim();
      if (!value) return false;
      const parts = value.split(/\s+/).filter(Boolean);
      if (wanted === 'digit' || wanted === 'number') return parts.some(part => /^[0-9]+$/.test(part));
      if (wanted === 'letter') return parts.some(part => /^[a-z]$/i.test(part));
      if (wanted === 'symbol') return parts.some(part => /^[^\w\s]+$/.test(part));
      return true;
    }}
    function centerOf(el) {{
      const center = pointCenter(el);
      if (center) return center;
      const rect = el.getBoundingClientRect();
      return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }};
    }}
    function visualObjectText(el) {{
      return [
        el.textContent || '',
        directTextOf(el),
        textOf(el),
        el.getAttribute('data-value') || '',
        el.getAttribute('data-label') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        semanticAttributeText(el)
      ].join(' ');
    }}
    if (shapeHint && ['number', 'digit', 'letter', 'text', 'symbol'].includes(shapeHint)) {{
      const textItems = all('svg text, svg tspan')
        .filter(el => visible(el))
        .map(el => {{
          const rect = el.getBoundingClientRect();
          return {{
            el,
            rect,
            area: rect.width * rect.height,
            color: colorNameFromElement(el),
            text: visualObjectText(el)
          }};
        }})
        .filter(item => {{
          if (wantedColor && item.color !== wantedColor) return false;
          if (!textKindMatches(shapeHint, item.text)) return false;
          if (targetText && Math.max(tokenScore(targetText, item.text), exactPhraseScore(targetText, item.text), semanticScore(targetText, item.text)) <= 0) return false;
          return true;
        }})
        .sort((a, b) => a.area - b.area);
      if (textItems.length === 1 || (textItems.length > 1 && /\b(?:a|an|any)\b/i.test(instruction))) {{
        const chosenText = textItems[0];
        const center = centerOf(chosenText.el);
        const params = {{ x: center.x, y: center.y }};
        if (clickStyle === 'right_click') params.button = 'right';
        if (clickStyle === 'double_click') params.click_count = 2;
        return {{
          ok: true,
          action: 'click',
          params,
          confidence: 0.88,
          reason: 'matched visible SVG text by generic visual text attributes and geometry',
          candidate: candidate(chosenText.el),
          evidence: {{
            color: wantedColor,
            shape: shapeHint,
            text: targetText || null,
            matchedShape: 'text',
            candidateCount: textItems.length
          }}
        }};
      }}
    }}

    const raw = all('svg text, svg tspan, svg circle, svg rect, svg ellipse, svg polygon, svg path, svg line, [data-shape], [data-value], [data-color], [role=img], canvas, div, span, button, [role=button], [onclick], [tabindex]')
      .filter(el => {{
        if (!visible(el) && !visibleGridPoint(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'input', 'select', 'textarea'].includes(tag)) return false;
        if (typeOf(el) === 'submit') return false;
        if (el.id === 'query' || (el.closest && el.closest('#query, [data-role=query], .query'))) return false;
        const rect = el.getBoundingClientRect();
        if (tag === 'text' || tag === 'tspan') {{
          if (rect.width < 1 || rect.height < 3) return false;
        }} else if (rect.width < 3 || rect.height < 3) return false;
        const area = rect.width * rect.height;
        if (area > 300000) return false;
        const selfDescribesVisualItem = el.hasAttribute('data-shape') ||
          el.hasAttribute('data-value') ||
          el.hasAttribute('data-color') ||
          el.hasAttribute('onclick') ||
          el.hasAttribute('tabindex') ||
          roleOf(el) === 'button' ||
          roleOf(el) === 'img';
        if ((tag === 'div' || tag === 'span') && !selfDescribesVisualItem) {{
          try {{
            if (el.querySelector('svg text, svg tspan, svg circle, svg rect, svg ellipse, svg polygon, svg path, svg line, canvas, [data-shape], [data-value], [data-color]')) return false;
          }} catch (_) {{}}
        }}
        return true;
      }});
    const candidates = raw.map(el => {{
      const rect = el.getBoundingClientRect();
      const rgb = elementVisualRgb(el);
      const metadata = [
        el.getAttribute('data-color') || '',
        el.getAttribute('data-fill') || '',
        el.getAttribute('fill') || '',
        el.getAttribute('stroke') || '',
        el.getAttribute('style') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        classText(el),
        el.id || ''
      ].join(' ').toLowerCase();
      const matchesDataColor = wantedColor ? metadata.split(/[^a-z]+/).includes(wantedColor) : false;
      const matchesVisualColor = wantedColor ? colorFamilyMatch(rgb, wantedColor) : false;
      const actualShape = shapeNameOf(el);
      const text = visualObjectText(el);
      const textScore = targetText ? Math.max(tokenScore(targetText, text), exactPhraseScore(targetText, text), semanticScore(targetText, text)) : 0;
      return {{
        el,
        rect,
        area: rect.width * rect.height,
        rgb,
        actualShape,
        matchesDataColor,
        matchesVisualColor,
        textScore
      }};
    }}).filter(item => {{
      if (wantedColor && !(item.matchesDataColor || item.matchesVisualColor)) return false;
      if (shapeHint && !shapeMatches(shapeHint, item.actualShape)) return false;
      if (!textKindMatches(shapeHint, visualObjectText(item.el))) return false;
      if (targetText && item.textScore <= 0) return false;
      return true;
    }});
    if (!candidates.length || candidates.length > 80) return null;
    const areas = candidates.map(item => item.area).sort((a, b) => a - b);
    const minArea = areas[0] || 1;
    const maxArea = areas[areas.length - 1] || 1;
    const exactColorCandidates = wantedColor ? candidates.filter(item => item.matchesDataColor) : [];
    const pool = exactColorCandidates.length ? exactColorCandidates : candidates;
    const ranked = pool.map(item => {{
      let score = 0.38;
      if (wantedColor) score += item.matchesDataColor ? 0.42 : 0.22;
      if (shapeHint) score += 0.28;
      if (targetText) score += item.textScore * 0.42;
      if (wantsCenter) score += 0.12;
      if (wantedSize) {{
        const smallness = maxArea > minArea ? (maxArea - item.area) / (maxArea - minArea) : 0.5;
        const largeness = maxArea > minArea ? (item.area - minArea) / (maxArea - minArea) : 0.5;
        score += /small|tiny|little/.test(wantedSize) ? smallness * 0.28 : largeness * 0.28;
      }}
      if (item.el.closest('svg')) score += 0.08;
      if (['text', 'tspan'].includes(item.el.tagName.toLowerCase()) && targetText) score += 0.12;
      if (['button'].includes(item.el.tagName.toLowerCase()) || roleOf(item.el) === 'button') score += 0.03;
      return {{ ...item, score }};
    }}).sort((a, b) => b.score - a.score || a.area - b.area);
    if (!ranked.length || ranked[0].score < 0.55) return null;
    const allowsAnyEquivalent = !targetText && /\b(?:a|an|any)\b/i.test(instruction) && (shapeHint === 'item' || shapeHint === 'object' || shapeHint === 'shape');
    if (ranked.length > 1 && ranked[0].score - ranked[1].score < 0.04 && !allowsAnyEquivalent) return null;
    const chosen = ranked[0];
    const center = centerOf(chosen.el);
    const params = {{ x: center.x, y: center.y }};
    if (clickStyle === 'right_click') params.button = 'right';
    if (clickStyle === 'double_click') params.click_count = 2;
    const primary = {{
      ok: true,
      action: 'click',
      params,
      confidence: Math.min(1, chosen.score),
      reason: 'matched visible object by generic visual attributes and geometry',
      candidate: candidate(chosen.el),
      evidence: {{
        color: wantedColor,
        size: wantedSize,
        shape: shapeHint,
        text: targetText || null,
        matchedShape: chosen.actualShape,
        candidateCount: candidates.length
      }}
    }};
    const follow = clickStepForHint(followUpClickHint(), chosen.el)
      || (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(chosen.el) : null);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned visual object click plus completion control'
    }};
  }}
  function visualGeometrySelectionPlan() {{
    if (!kindIs('click', 'count')) return null;
    const asksShapeName = /\b(?:describes?|identify|name|kind|type)\b/i.test(instruction) &&
      /\b(?:figure|shape|object|symbol|item)\b/i.test(instruction);
    const asksSideCount = /\b(?:how\s+many|number\s+of|count)\s+sides?\b/i.test(instruction) ||
      /\bsides?\b/i.test(instruction) && /\b(?:correctly|denotes?|button|press|click)\b/i.test(instruction);
    if (!asksShapeName && !asksSideCount) return null;

    function figureShape(el) {{
      const tag = el.tagName.toLowerCase();
      const cue = [tag, classText(el), el.getAttribute('data-shape') || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ').toLowerCase();
      if (tag === 'circle' || tag === 'ellipse') return {{ label: 'circle', sides: 0 }};
      if (tag === 'rect') {{
        const rect = el.getBoundingClientRect();
        const aspect = rect.width > 0 && rect.height > 0 ? rect.width / rect.height : 1;
        return {{ label: aspect >= 0.78 && aspect <= 1.28 ? 'square' : 'rectangle', sides: 4 }};
      }}
      if (tag === 'polygon') {{
        const points = String(el.getAttribute('points') || '').trim().split(/\s+/).filter(Boolean);
        if (points.length === 3) return {{ label: 'triangle', sides: 3 }};
        return {{ label: 'polygon', sides: points.length || null }};
      }}
      if (tag === 'path') {{
        if (/\btriangle\b/.test(cue)) return {{ label: 'triangle', sides: 3 }};
        if (/\b(rectangle|square)\b/.test(cue)) return {{ label: cue.includes('square') ? 'square' : 'rectangle', sides: 4 }};
        return {{ label: 'shape', sides: null }};
      }}
      if (tag === 'text' || tag === 'tspan') {{
        const value = String(el.textContent || '').trim();
        if (/^-?\d+(?:\.\d+)?$/.test(value)) return {{ label: 'number', sides: null }};
        if (/^[a-z]$/i.test(value)) return {{ label: 'letter', sides: null }};
        return {{ label: 'text', sides: null }};
      }}
      if (/\b(circle|dot|round)\b/.test(cue)) return {{ label: 'circle', sides: 0 }};
      if (/\btriangle\b/.test(cue)) return {{ label: 'triangle', sides: 3 }};
      if (/\bsquare\b/.test(cue)) return {{ label: 'square', sides: 4 }};
      if (/\brectangle\b/.test(cue)) return {{ label: 'rectangle', sides: 4 }};
      return null;
    }}

    function canvasPolygonShape(canvas) {{
      let data;
      try {{
        const ctx = canvas.getContext && canvas.getContext('2d');
        if (!ctx) return null;
        data = ctx.getImageData(0, 0, canvas.width, canvas.height);
      }} catch (_) {{
        return null;
      }}
      const pixels = [];
      const width = data.width;
      const height = data.height;
      for (let y = 0; y < height; y++) {{
        for (let x = 0; x < width; x++) {{
          const offset = (y * width + x) * 4;
          const alpha = data.data[offset + 3];
          if (alpha < 40) continue;
          const intensity = (data.data[offset] + data.data[offset + 1] + data.data[offset + 2]) / 3;
          if (intensity > 120) continue;
          pixels.push([x, y]);
        }}
      }}
      if (pixels.length < 24) return null;
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const [x, y] of pixels) {{
        minX = Math.min(minX, x); maxX = Math.max(maxX, x);
        minY = Math.min(minY, y); maxY = Math.max(maxY, y);
      }}
      const cx = (minX + maxX) / 2;
      const cy = (minY + maxY) / 2;
      const maxRadius = Math.max(...pixels.map(([x, y]) => Math.hypot(x - cx, y - cy)));
      if (!Number.isFinite(maxRadius) || maxRadius < 8) return null;
      const sampleStride = Math.max(1, Math.ceil(pixels.length / 700));
      const sample = pixels.filter((_, index) => index % sampleStride === 0);
      function distToSegment(px, py, ax, ay, bx, by) {{
        const dx = bx - ax;
        const dy = by - ay;
        const len2 = dx * dx + dy * dy || 1;
        const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len2));
        const x = ax + t * dx;
        const y = ay + t * dy;
        return Math.hypot(px - x, py - y);
      }}
      function score(n, rotation, radius) {{
        const vertices = [];
        for (let i = 0; i < n; i++) {{
          const angle = rotation + i * 2 * Math.PI / n;
          vertices.push([cx + radius * Math.cos(angle), cy + radius * Math.sin(angle)]);
        }}
        let total = 0;
        for (const [px, py] of sample) {{
          let bestDistance = Infinity;
          for (let i = 0; i < n; i++) {{
            const a = vertices[i];
            const b = vertices[(i + 1) % n];
            bestDistance = Math.min(bestDistance, distToSegment(px, py, a[0], a[1], b[0], b[1]));
          }}
          total += Math.min(bestDistance, 12);
        }}
        return total / sample.length;
      }}
      let bestShape = null;
      for (let n = 3; n <= 8; n++) {{
        for (const radiusScale of [0.92, 0.98, 1.04]) {{
          const radius = maxRadius * radiusScale;
          const step = Math.PI / 90;
          for (let rotation = 0; rotation < 2 * Math.PI / n; rotation += step) {{
            const value = score(n, rotation, radius);
            if (!bestShape || value < bestShape.score) bestShape = {{ sides: n, score: value }};
          }}
        }}
      }}
      if (!bestShape || bestShape.score > 4.2) return null;
      const rect = canvas.getBoundingClientRect();
      return {{ label: 'polygon', sides: bestShape.sides, area: rect.width * rect.height, el: canvas, score: bestShape.score }};
    }}

    let figures = all('svg circle, svg ellipse, svg rect, svg polygon, svg path, svg text, svg tspan, [data-shape]')
      .filter(el => {{
        if (!visible(el) && !visibleGridPoint(el)) return false;
        if (el.id === 'query' || (el.closest && el.closest('#query, [data-role=query], .query'))) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 3 || rect.height < 3 || rect.width * rect.height > 80000) return false;
        return true;
      }})
      .map(el => {{
        const shape = figureShape(el);
        const rect = el.getBoundingClientRect();
        return shape ? {{ el, ...shape, area: rect.width * rect.height }} : null;
      }})
      .filter(Boolean)
      .sort((a, b) => b.area - a.area);
    if (!figures.length && asksSideCount) {{
      figures = all('canvas')
        .filter(el => visible(el))
        .map(canvasPolygonShape)
        .filter(Boolean)
        .sort((a, b) => b.area - a.area);
    }}
    if (!figures.length) return null;
    const figure = figures[0];
    const value = asksSideCount && figure.sides != null ? String(figure.sides) : figure.label;
    if (!value || value === 'shape' || value === 'text') return null;

    const candidates = clickableElements().filter(el => {{
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      if (!(tag === 'button' || typeOf(el) === 'button' || typeOf(el) === 'submit' || role === 'button')) return false;
      const rect = el.getBoundingClientRect();
      return rect.width >= 8 && rect.height >= 8;
    }});
    const ranked = best(candidates, el => {{
      const text = [directTextOf(el), textOf(el), String(el.value || '')].join(' ');
      let score = Math.max(exactPhraseScore(value, text), tokenScore(value, text), semanticScore(value, text));
      if (String(text || '').trim().toLowerCase() === value.toLowerCase()) score += 0.6;
      return score;
    }});
    if (!ranked.length || ranked[0].score < 0.45) return null;
    const primary = {{
      ok: true,
      action: 'click',
      params: {{ selector: selector(ranked[0].el) }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'selected a visible control matching rendered figure geometry',
      candidate: candidate(ranked[0].el),
      evidence: {{ value, shape: figure.label, sides: figure.sides, mode: asksSideCount ? 'side_count' : 'shape_name', visualScore: figure.score ?? null }}
    }};
    return primary;
  }}

  function visualGeometryClickPlan() {{
    if (kind !== 'click') return null;
    const wantsMidpoint = /\b(?:midpoint|mid-point|middle\s+point|halfway|half-way|center\s+point|centre\s+point)\b/i.test(instruction);
    const wantsBetween = /\bbetween\b/i.test(instruction);
    if (!wantsMidpoint && !wantsBetween) return null;
    if (!/\b(?:line|segment|points?|dots?|markers?|between|midpoint|halfway|center|centre)\b/i.test(instruction)) return null;

    function screenPoint(svg, x, y) {{
      const point = svg.createSVGPoint();
      point.x = Number(x);
      point.y = Number(y);
      if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) return null;
      const matrix = svg.getScreenCTM && svg.getScreenCTM();
      if (!matrix) return null;
      const screen = point.matrixTransform(matrix);
      return {{ x: screen.x, y: screen.y }};
    }}
    function midpoint(a, b) {{
      return {{ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }};
    }}
    function labelTokens() {{
      const match = instruction.match(/\bbetween\s+(?:point\s+|dot\s+|marker\s+)?([A-Za-z0-9_-]+)\s+(?:and|&)\s+(?:point\s+|dot\s+|marker\s+)?([A-Za-z0-9_-]+)\b/i) ||
        instruction.match(/\bfrom\s+(?:point\s+|dot\s+|marker\s+)?([A-Za-z0-9_-]+)\s+to\s+(?:point\s+|dot\s+|marker\s+)?([A-Za-z0-9_-]+)\b/i);
      return match ? [match[1].toLowerCase(), match[2].toLowerCase()] : [];
    }}
    function pointCue(el) {{
      return [
        directTextOf(el),
        textOf(el),
        el.id || '',
        classText(el),
        el.getAttribute('data-label') || '',
        el.getAttribute('data-value') || '',
        el.getAttribute('data-point') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        semanticAttributeText(el)
      ].join(' ').toLowerCase();
    }}
    function pointFromElement(el) {{
      const tag = el.tagName.toLowerCase();
      const svg = el.ownerSVGElement || el.closest('svg');
      if (svg && (tag === 'circle' || tag === 'ellipse')) {{
        return screenPoint(svg, el.getAttribute('cx'), el.getAttribute('cy')) || pointCenter(el);
      }}
      if (svg && tag === 'rect') {{
        const x = Number(el.getAttribute('x'));
        const y = Number(el.getAttribute('y'));
        const width = Number(el.getAttribute('width'));
        const height = Number(el.getAttribute('height'));
        if ([x, y, width, height].every(Number.isFinite)) return screenPoint(svg, x + width / 2, y + height / 2) || pointCenter(el);
      }}
      return pointCenter(el);
    }}
    function visiblePointMarkers(surface) {{
      const raw = Array.from(surface.querySelectorAll ? surface.querySelectorAll('circle, ellipse, rect, [data-point], [data-marker], [class*=point], [class*=Point], [class*=dot], [class*=Dot], [class*=marker], [class*=Marker]') : [])
        .filter(el => visible(el) || visibleGridPoint(el));
      const surfaceRect = surface.getBoundingClientRect();
      const surfaceArea = Math.max(1, surfaceRect.width * surfaceRect.height);
      return raw.map(el => {{
        const point = pointFromElement(el);
        const rect = el.getBoundingClientRect();
        if (!point) return null;
        const area = Math.max(1, rect.width * rect.height);
        let score = 0.35;
        const cue = pointCue(el);
        if (/\b(?:point|dot|marker|endpoint|anchor|target)\b/i.test(cue)) score += 0.28;
        if (area / surfaceArea < 0.04) score += 0.24;
        if (['circle', 'ellipse'].includes(el.tagName.toLowerCase())) score += 0.16;
        return {{ el, point, cue, area, score }};
      }}).filter(Boolean);
    }}
    function labeledPair(markers) {{
      const labels = labelTokens();
      if (labels.length !== 2) return null;
      const chosen = labels.map(label => {{
        const ranked = markers.map(marker => {{
          let score = 0;
          const cueTokens = tokens(marker.cue);
          if (cueTokens.includes(label)) score += 0.85;
          score += Math.max(tokenScore(label, marker.cue), exactPhraseScore(label, marker.cue)) * 0.5;
          return {{ marker, score: score + marker.score * 0.2 }};
        }}).filter(item => item.score > 0).sort((a, b) => b.score - a.score);
        return ranked.length && ranked[0].score >= 0.35 ? ranked[0].marker.el : null;
      }});
      if (!chosen[0] || !chosen[1] || chosen[0] === chosen[1]) return null;
      const first = markers.find(marker => marker.el === chosen[0]);
      const second = markers.find(marker => marker.el === chosen[1]);
      return first && second ? [first, second] : null;
    }}
    function lineMidpoints(surface) {{
      if (surface.tagName.toLowerCase() !== 'svg') return [];
      return Array.from(surface.querySelectorAll('line'))
        .filter(el => visible(el))
        .map(el => {{
          const a = screenPoint(surface, el.getAttribute('x1'), el.getAttribute('y1'));
          const b = screenPoint(surface, el.getAttribute('x2'), el.getAttribute('y2'));
          if (!a || !b) return null;
          const length = Math.hypot(b.x - a.x, b.y - a.y);
          if (length < 12) return null;
          let score = 0.68;
          const cue = pointCue(el);
          if (/\bline|segment|edge\b/i.test(instruction)) score += 0.24;
          if (/\bline|segment|edge\b/i.test(cue)) score += 0.16;
          return {{ point: midpoint(a, b), score, evidence: {{ source: 'line', length: Math.round(length) }}, anchor: el }};
        }}).filter(Boolean);
    }}

    const surfaces = best(drawingSurfaceCandidates().filter(el => {{
      const tag = el.tagName.toLowerCase();
      return tag === 'svg' || tag === 'canvas' || roleOf(el) === 'img' || roleOf(el) === 'application';
    }}), el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = 0.42;
      if (/\b(?:geometry|graph|drawing|surface|canvas|svg|figure|diagram|line|point)\b/i.test(text)) score += 0.26;
      if (el.tagName.toLowerCase() === 'svg') score += 0.18;
      const lines = el.querySelectorAll ? el.querySelectorAll('line').length : 0;
      const markers = el.querySelectorAll ? el.querySelectorAll('circle, ellipse, [data-point], [data-marker]').length : 0;
      if (lines) score += 0.24;
      if (markers >= 2) score += 0.2;
      return score;
    }});

    const plans = [];
    for (const rankedSurface of surfaces.slice(0, 5)) {{
      const surface = rankedSurface.el;
      const markers = visiblePointMarkers(surface);
      const pair = labeledPair(markers) ||
        (markers.length === 2 ? markers.sort((a, b) => b.score - a.score) : null);
      if (pair && pair.length === 2) {{
        plans.push({{
          point: midpoint(pair[0].point, pair[1].point),
          score: rankedSurface.score + 0.4 + pair[0].score * 0.15 + pair[1].score * 0.15,
          surface,
          evidence: {{
            source: 'point_pair',
            labels: labelTokens(),
            first: {{ x: Math.round(pair[0].point.x), y: Math.round(pair[0].point.y) }},
            second: {{ x: Math.round(pair[1].point.x), y: Math.round(pair[1].point.y) }}
          }}
        }});
      }}
      for (const line of lineMidpoints(surface)) {{
        plans.push({{
          point: line.point,
          score: rankedSurface.score + line.score,
          surface,
          evidence: line.evidence
        }});
      }}
    }}
    plans.sort((a, b) => b.score - a.score);
    if (!plans.length || plans[0].score < 0.74) return null;
    const chosen = plans[0];
    const primary = {{
      ok: true,
      action: 'click',
      params: {{ x: chosen.point.x, y: chosen.point.y }},
      confidence: Math.min(1, chosen.score),
      reason: 'derived visible geometry point from page shapes and markers',
      candidate: candidate(chosen.surface),
      evidence: {{
        ...chosen.evidence,
        point: {{ x: Math.round(chosen.point.x), y: Math.round(chosen.point.y) }}
      }}
    }};
    const follow = clickStepForHint(followUpClickHint(), chosen.surface) ||
      (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(chosen.surface) : null);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned visual geometry click plus completion control'
    }};
  }}

  function countPlan() {{
    const countIntent = kind === 'count' ||
      /\b(?:total\s+number\s+of|number\s+of|how\s+many|count)\b/i.test(instruction) &&
        /\b(?:type|enter|input|write|fill|answer|textbox|text\s*box|field|press|submit)\b/i.test(instruction);
    if (!countIntent) return null;
    function parseRgb(value) {{
      const match = String(value || '').match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/i);
      return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
    }}
    function colorMatches(value, color) {{
      const raw = String(value || '').toLowerCase();
      if (!raw || !color) return false;
      if (raw.includes(color)) return true;
      const rgb = parseRgb(raw);
      if (!rgb) return false;
      const targets = {{
        red: [[255, 0, 0]], scarlet: [[255, 0, 0]],
        orange: [[255, 165, 0]],
        yellow: [[255, 255, 0]],
        olive: [[128, 128, 0]],
        lime: [[0, 255, 0]],
        green: [[0, 128, 0], [0, 255, 0]],
        cyan: [[0, 255, 255]], aqua: [[0, 255, 255]], teal: [[0, 128, 128]],
        blue: [[0, 0, 255]], navy: [[0, 0, 128]], indigo: [[75, 0, 130]],
        magenta: [[255, 0, 255]], purple: [[128, 0, 128], [160, 32, 240]],
        violet: [[238, 130, 238]],
        pink: [[255, 192, 203]],
        brown: [[165, 42, 42]],
        gold: [[255, 215, 0]],
        black: [[0, 0, 0]], white: [[255, 255, 255]],
        gray: [[128, 128, 128]], grey: [[128, 128, 128]], silver: [[192, 192, 192]]
      }}[color] || [];
      return targets.some(([r, g, b]) => Math.abs(rgb[0] - r) <= 32 && Math.abs(rgb[1] - g) <= 32 && Math.abs(rgb[2] - b) <= 32);
    }}
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
    const colorMatch = instruction.match(/\b(red|scarlet|orange|yellow|olive|lime|green|cyan|aqua|teal|blue|navy|indigo|purple|violet|magenta|pink|brown|gold|black|white|gray|grey|silver)\b/i);
    const wantedColor = colorMatch ? colorMatch[1].toLowerCase() : null;
    let value = null;
    if (/\bdigits?\b/i.test(instruction)) {{
      value = textItems.filter(item => /^\d$/.test(item.text) && (!wantedColor || colorMatches(item.fill, wantedColor))).length;
    }}
    if (value == null) {{
      if (colorMatch && /\bletters?\b/i.test(instruction)) {{
        value = textItems.filter(item => /^[a-z]$/i.test(item.text) && colorMatches(item.fill, wantedColor)).length;
      }} else if (colorMatch && /\bitems?\b/i.test(instruction)) {{
        value = svgItems.filter(item => colorMatches(item.fill, wantedColor)).length;
      }}
    }}
    if (value == null) {{
      const sizeMatch = instruction.match(/\b(small|large|big|tiny)\s+(?:letter\s+)?([a-z])s?\b/i);
      const letterMatch = sizeMatch || instruction.match(/\b(?:letter\s+)?([a-z])s?\b/i);
      if (letterMatch) {{
        const sizeWord = sizeMatch ? sizeMatch[1].toLowerCase() : null;
        const letter = (sizeMatch ? sizeMatch[2] : letterMatch[1]).toLowerCase();
        const sizes = textItems.map(item => item.size).filter(Number.isFinite).sort((a, b) => a - b);
        const median = sizes.length ? sizes[Math.floor(sizes.length / 2)] : 0;
        value = textItems.filter(item => {{
          if (item.text.toLowerCase() !== letter) return false;
          if (!sizeWord) return true;
          if (sizeWord === 'small' || sizeWord === 'tiny') return !item.size || item.size <= median;
          return item.size >= median;
        }}).length;
      }}
    }}
    if (value == null && /\b(circles?|rectangles?|squares?|triangles?|polygons?)\b/i.test(instruction)) {{
      const tag = /circles?/i.test(instruction) ? 'circle' :
        /rectangles?|squares?/i.test(instruction) ? 'rect' :
        /triangles?|polygons?/i.test(instruction) ? 'polygon' : null;
      if (tag) {{
        value = all('svg ' + tag).filter(el => {{
          if (!visible(el)) return false;
          if (!wantedColor) return true;
          return colorMatches(String(el.getAttribute('fill') || getComputedStyle(el).fill || '').toLowerCase(), wantedColor);
        }}).length;
      }}
    }}
    if (value == null) {{
      const nounMatch = instruction.match(/\b(?:total\s+number\s+of|number\s+of|how\s+many|count)\s+(?:the\s+|all\s+)?([a-z][a-z0-9_-]*)\b/i);
      const nounRaw = nounMatch ? nounMatch[1].toLowerCase() : null;
      const singular = nounRaw
        ? nounRaw.replace(/ies$/, 'y').replace(/(?:ches|shes|xes|zes|ses)$/, match => match.slice(0, -2)).replace(/s$/, '')
        : null;
      const nounVariants = singular ? Array.from(new Set([nounRaw, singular, singular + 's'])) : [];
      const tagForNoun = singular === 'circle' || singular === 'dot' ? 'circle' :
        singular === 'rectangle' || singular === 'square' || singular === 'block' || singular === 'tile' || singular === 'cell' ? 'rect' :
        singular === 'triangle' || singular === 'polygon' ? 'polygon' : null;
      function nounText(el) {{
        return [
          el.tagName.toLowerCase(),
          el.id || '',
          classText(el),
          el.getAttribute('role') || '',
          el.getAttribute('aria-label') || '',
          el.getAttribute('title') || '',
          el.getAttribute('data-testid') || '',
          el.getAttribute('data-role') || '',
          el.getAttribute('data-kind') || '',
          el.getAttribute('data-type') || '',
        ].join(' ').toLowerCase();
      }}
      function nounMatches(el) {{
        if (!nounVariants.length) return false;
        const meta = nounText(el);
        if (nounVariants.some(noun => new RegExp('(?:^|[^a-z0-9])' + noun.replace(/[.*+?^${{}}()|[\]\\]/g, '\\$&') + '(?:[^a-z0-9]|$)', 'i').test(meta))) return true;
        if (tagForNoun && el.closest('svg') && el.tagName.toLowerCase() === tagForNoun) return true;
        return false;
      }}
      if (singular) {{
        let visualItems = all('body *')
          .filter(el => visible(el))
          .filter(el => {{
            const tag = el.tagName.toLowerCase();
            if (['html', 'body', 'script', 'style', 'noscript', 'template', 'input', 'textarea', 'select', 'option', 'button'].includes(tag)) return false;
            if (el.closest('button, a, input, textarea, select, [role=button], [role=link]')) return false;
            const rect = el.getBoundingClientRect();
            if (rect.width < 3 || rect.height < 3 || rect.width * rect.height < 9 || rect.width * rect.height > 120000) return false;
            return nounMatches(el);
          }});
        visualItems = visualItems.filter((el, index, arr) => arr.indexOf(el) === index);
        visualItems = visualItems.filter(el => !visualItems.some(other => other !== el && el.contains(other) && visible(other) && nounMatches(other)));
        if (visualItems.length) value = visualItems.length;
      }}
    }}
    if (value == null) return null;
    const buttons = best(clickableElements().filter(el => el.tagName.toLowerCase() === 'button' || (el.getAttribute('role') || '').toLowerCase() === 'button'), el => {{
      return (textOf(el).trim() === String(value)) ? 1 : 0;
    }});
    if (buttons.length) {{
      return {{
        ok: true,
        action: 'click',
        params: {{ selector: selector(buttons[0].el) }},
        confidence: 0.8,
        reason: 'counted visible page items and matched a numeric control',
        candidate: candidate(buttons[0].el),
        value
      }};
    }}
    const fields = writableFields().filter(el => {{
      if (!visible(el) || !writableField(el)) return false;
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute('type') || '').toLowerCase();
      return tag === 'textarea' ||
        (tag === 'input' && ['', 'text', 'number'].includes(type)) ||
        isCustomWritableValueElement(el);
    }});
    if (fields.length) {{
      const primary = {{
        ok: true,
        action: 'type',
        params: {{ selector: selector(fields[0]), text: String(value), clear_first: true }},
        confidence: 0.75,
        reason: 'counted visible page items and matched a writable field',
        candidate: candidate(fields[0]),
        value
      }};
      const follow = clickStepForHint(followUpClickHint(), fields[0]) ||
        (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(fields[0]) : null);
      if (!follow) return primary;
      return {{
        ok: true,
        action: 'sequence',
        steps: [primary, follow],
        confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
        reason: 'planned visible count value fill plus completion control'
      }};
    }}
    return null;
  }}

  {text_transfer_capability_js}

  function deriveAndActPlan() {{
    const text = instruction.toLowerCase();
    const hasExplicitFillValue = !!wantedValue && /\b(?:enter|type|fill|input|write)\b/i.test(instruction);
    const asksToDeriveFill = /\b(solve|calculate|answer|type\s+the\s+text|enter\s+the\s+text|text\s+below|displayed\s+text|shown\s+text)\b/i.test(instruction);
    const asksLastWord = /\blast\s+word\b/i.test(instruction) && /\b(text\s*area|textarea|scroll|field)\b/i.test(instruction);
    const asksOrdinalWord = ordinalIndex(instruction) != null &&
      /\b(word|token)\b/i.test(instruction) &&
      /\b(find|type|enter|input|write|fill|answer|textbox|text\s*box|field)\b/i.test(instruction);
    const asksForExtremeChoice = /\b(find|pick|choose|select|click)\b/i.test(instruction) &&
      /\b(greatest|highest|largest|max(?:imum)?)\b/i.test(instruction);
    const hasTextField = interactive.some(el => visible(el) && writableField(el));
    const hasNumericChoice = /\b(greatest|highest|largest|max(?:imum)?)\b/i.test(instruction) &&
      all('[data-value], [data-index], button, a, [role=button], [onclick], [tabindex], .card, .item, .tile, div, span')
        .some(el => visible(el) && /-?\d/.test(textOf(el)));
    if (hasExplicitFillValue && !asksLastWord && !asksOrdinalWord && !asksForExtremeChoice && !/\b(?:solve|calculate|math|problem|equation)\b/i.test(instruction)) return null;
    if (!asksToDeriveFill && !asksLastWord && !asksOrdinalWord && !asksForExtremeChoice) return null;
    const likelyDerivedField = (asksToDeriveFill || asksOrdinalWord) && /\b(solve|calculate|answer|type|enter|input|write|fill|textbox|text\s*box|field)\b/i.test(instruction);
    if ((asksToDeriveFill || asksLastWord || asksOrdinalWord) && !hasTextField && !likelyDerivedField) return null;
    if (asksForExtremeChoice && !hasNumericChoice) return null;
    if (/\b(solve|calculate|math|problem)\b/i.test(instruction) && !/(?:-?\d+\s*[+\-x*/]\s*-?\d+|-?\d+\s*[+\-x*/]\s*x|x\s*[+\-x*/]\s*-?\d+)\s*=/.test(textOf(document.body || document.documentElement))) return null;
    return {{
      ok: true,
      action: 'derive_and_act',
      params: {{ instruction }},
      confidence: 0.9,
      reason: asksForExtremeChoice
        ? 'planned generic visible numeric extreme selection from page content'
        : 'planned generic derived answer fill from visible page content',
      evidence: {{ asksToDeriveFill, asksLastWord, asksOrdinalWord, asksForExtremeChoice, hasTextField, hasNumericChoice }}
    }};
  }}

  function numericConstraintRequest() {{
    const text = instruction.toLowerCase();
	    const less = text.match(/\b(?:less\s+than|below|under|smaller\s+than|lower\s+than)\s*[$€£]?\s*(-?\d+(?:\.\d+)?)\b/i);
	    const greater = text.match(/\b(?:greater\s+than|above|over|more\s+than|larger\s+than|higher\s+than)\s*[$€£]?\s*(-?\d+(?:\.\d+)?)\b/i);
	    const atLeast = text.match(/\b(?:at\s+least|minimum|min(?:imum)?\s+of)\s*[$€£]?\s*(-?\d+(?:\.\d+)?)\b/i);
	    const atMost = text.match(/\b(?:at\s+most|maximum|max(?:imum)?\s+of|no\s+more\s+than)\s*[$€£]?\s*(-?\d+(?:\.\d+)?)\b/i);
	    const equals = text.match(/\b(?:equal\s+to|equals?|exactly)\s*[$€£]?\s*(-?\d+(?:\.\d+)?)\b/i);
    const wantsOdd = /\bodd\b/i.test(instruction);
    const wantsEven = /\beven\b/i.test(instruction);
    const constraints = {{
      lessThan: less ? Number(less[1]) : null,
      greaterThan: greater ? Number(greater[1]) : null,
      min: atLeast ? Number(atLeast[1]) : null,
      max: atMost ? Number(atMost[1]) : null,
      equals: equals ? Number(equals[1]) : null,
      parity: wantsOdd ? 'odd' : wantsEven ? 'even' : null
    }};
    const hasConstraint = Object.values(constraints).some(value => value != null);
    if (!hasConstraint) return null;
    if ([constraints.lessThan, constraints.greaterThan, constraints.min, constraints.max, constraints.equals].some(value => value != null && !Number.isFinite(value))) return null;
	    return constraints;
	  }}

  function conditionalActionHint() {{
    const beforeCondition = String(instruction || '').split(/\b(?:when|once|after|as\s+soon\s+as|if)\b/i)[0].trim();
    const explicitObject = beforeCondition.match(/\b(?:click|press|tap|select|choose)\s+(?:the\s+)?(.+?)$/i);
    if (explicitObject && explicitObject[1]) {{
      return explicitObject[1]
        .replace(/\b(?:button|link|control|action|the|a|an)\b/ig, ' ')
        .replace(/\s+/g, ' ')
        .trim();
    }}
    const verb = beforeCondition.match(/\b(buy|sell|open|start|stop|submit|save|confirm|continue|send|go)\b/i);
    return verb ? verb[1].toLowerCase() : '';
  }}

  function conditionalSourceHint() {{
    const metric = String(instruction || '').match(/\b(price|value|amount|total|number|count|score|balance|rate|quote|metric)\b/i);
    return metric ? metric[1].toLowerCase() : '';
  }}

  function hasVisibleNumericCandidateForCondition(sourceHint) {{
    return all('[data-value], [data-number], [aria-valuenow], output, [role=status], [aria-live], .value, .metric, .amount, .price, .total, .current, .number, .display, div, span, td, p')
      .some(el => {{
        if (!readableVisible(el)) return false;
        if (el.closest && el.closest('button, a, input, textarea, select, [role=button], [role=link]')) return false;
        const text = textOf(el);
        if (!/(?:[$€£]\s*)?-?\d+(?:\.\d+)?/.test(text)) return false;
        const meta = [el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || '', text].join(' ');
        if (/\b(query|prompt|instruction|question|task|goal|timer|time\s*left|scoreboard)\b/i.test(meta)) return false;
        return !sourceHint || tokenScore(sourceHint, meta) > 0 || /\b(price|value|amount|total|current|quote|rate|number|metric|balance)\b/i.test(meta);
      }});
  }}

  function conditionalValueActionPlan() {{
    const constraints = numericConstraintRequest();
    if (!constraints) return null;
    if (!/\b(?:when|once|after|as\s+soon\s+as|if)\b/i.test(instruction)) return null;
    if (!/\b(?:click|press|tap|select|choose|buy|sell|open|start|stop|submit|save|confirm|continue|send|go)\b/i.test(instruction)) return null;
    const actionHint = conditionalActionHint();
    const sourceHint = conditionalSourceHint();
    const actionCandidates = clickableElements().filter(el => {{
      const text = [textOf(el), el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      if (!actionHint) return /\b(?:submit|ok|done|continue|confirm|go)\b/i.test(text);
      return Math.max(tokenScore(actionHint, text), exactPhraseScore(actionHint, text)) > 0 ||
        text.toLowerCase().includes(actionHint.toLowerCase());
    }});
    if (!actionCandidates.length) return null;
    if (!hasVisibleNumericCandidateForCondition(sourceHint)) return null;
    return {{
      ok: true,
      action: 'conditional_value_action',
      params: {{ instruction, constraints, actionHint, sourceHint, maxWaitMs: 9000, pollMs: 90 }},
      confidence: 0.82,
      reason: 'planned conditional action after monitoring visible numeric page value',
      candidate: candidate(actionCandidates[0]),
      evidence: {{ constraints, actionHint, sourceHint, candidateCount: actionCandidates.length }}
    }};
  }}

  function commandSurfaceRequest() {{
    if (!/\b(?:terminal|shell|console|command\s+(?:prompt|line)|cli|repl)\b/i.test(instruction)) return false;
    return /\b(?:use|run|execute|type|enter|list|show)\b/i.test(instruction);
  }}

  function commandSurfaceCandidates() {{
    function scoreSurface(el) {{
      const meta = [
        el.id || '',
        classText(el),
        el.getAttribute('role') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        textOf(el)
      ].join(' ');
      let score = 0;
      if (/\b(?:terminal|shell|console|command|prompt|cli|repl)\b/i.test(meta)) score += 1;
      if (/\b(?:ls|rm|cd|cat|help|usage|command not found)\b/i.test(meta)) score += 0.35;
      if (/(?:^|\s)(?:[$>#%])\s*$/i.test(meta) || /\b(?:user|admin|root)\s*[$>#%]\b/i.test(meta)) score += 0.45;
      return score;
    }}
    const inputs = all('input, textarea, [contenteditable]:not([contenteditable="false"]), [role=textbox], [role=searchbox], [role=combobox], [tabindex]')
      .concat(all('*').filter(isCustomWritableValueElement))
      .filter(el => {{
        if (unavailableForAction(el)) return false;
        const tag = el.tagName.toLowerCase();
        const type = (el.getAttribute('type') || '').toLowerCase();
        if (tag === 'input' && ['button', 'submit', 'checkbox', 'radio', 'range', 'color', 'file'].includes(type)) return false;
        return tag === 'input' ||
          tag === 'textarea' ||
          el.isContentEditable ||
          ['textbox', 'searchbox', 'combobox'].includes(roleOf(el)) ||
          isCustomWritableValueElement(el) ||
          el.hasAttribute('tabindex');
      }})
      .map(el => {{
        let node = el.parentElement;
        let surfaceScore = 0;
        while (node && node !== document.documentElement) {{
          surfaceScore = Math.max(surfaceScore, scoreSurface(node));
          node = node.parentElement;
        }}
        const meta = [el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || '', el.getAttribute('placeholder') || ''].join(' ');
        let score = surfaceScore;
        if (document.activeElement === el) score += 0.7;
        if (/\b(?:terminal|shell|console|command|prompt|cli|input)\b/i.test(meta)) score += 0.8;
        return {{ el, score }};
      }})
      .filter(item => item.score >= 0.45)
      .sort((a, b) => b.score - a.score);
    const surfaces = all('pre, code, textarea, [role=log], [aria-live], [class*=terminal], [class*=Terminal], [class*=console], [class*=Console], [class*=shell], [class*=Shell], [id*=terminal], [id*=console], [id*=shell]')
      .filter(readableVisible)
      .map(el => ({{ el, score: scoreSurface(el) }}))
      .filter(item => item.score >= 0.75)
      .sort((a, b) => b.score - a.score);
    return {{ inputs, surfaces }};
  }}

  function commandSurfaceActionPlan() {{
    if (!commandSurfaceRequest()) return null;
    const candidates = commandSurfaceCandidates();
    if (!candidates.inputs.length && !candidates.surfaces.length) return null;
    if (!/\b(?:list|show|run|execute|type|enter)\b/i.test(instruction)) return null;
    const primary = candidates.inputs[0] || candidates.surfaces[0];
    return {{
      ok: true,
      action: 'command_surface_action',
      params: {{ instruction }},
      confidence: candidates.inputs.length ? 0.81 : 0.68,
      reason: 'planned generic command-surface workflow from instruction and visible shell-like UI',
      candidate: primary && candidate(primary.el),
      evidence: {{ inputCount: candidates.inputs.length, surfaceCount: candidates.surfaces.length }}
    }};
  }}

	  function generateConstrainedValuePlan() {{
    const constraints = numericConstraintRequest();
    if (!constraints) return null;
    const controls = clickableElements().filter(el => visible(el));
    const hasGenerator = controls.some(el => /\b(generate|random|roll|new|refresh|create|produce)\b/i.test(textOf(el)) && !/\b(submit|done|send|save|continue|confirm)\b/i.test(textOf(el)));
    const hasWritable = writableFields().some(visible);
    if (!hasGenerator && !hasWritable) return null;
    return {{
      ok: true,
      action: 'generate_constrained_value',
      params: {{ instruction, constraints, maxAttempts: 30 }},
      confidence: hasGenerator ? 0.84 : 0.78,
      reason: hasGenerator
        ? 'planned repeated value generation until visible numeric output satisfies instruction constraints'
        : 'planned deterministic numeric value entry satisfying instruction constraints',
      evidence: {{ constraints, hasGenerator, hasWritable }}
    }};
  }}

  function feedbackLoopValuePlan() {{
    if (!kindIs('fill', 'click')) return null;
    const wantsFeedbackLoop = /\b(?:guess|find)\b/i.test(instruction) &&
      /\b(?:hidden|secret|unknown|target|correct)?\s*(?:number|value)\b/i.test(instruction);
    if (!wantsFeedbackLoop) return null;
    const fields = writableFields().filter(visible);
    if (!fields.length) return null;
    const submitControls = clickableElements().filter(el => {{
      const text = textOf(el);
      return /\b(?:submit|check|guess|try|go|ok|done|enter)\b/i.test(text) || typeOf(el) === 'submit';
    }});
    if (!submitControls.length) return null;
    const explicitRange = instruction.match(/\b(?:between|from|range\s+of|ranging\s+from)?\s*(-?\d+(?:\.\d+)?)\s*(?:-|–|—|to|through|and)\s*(-?\d+(?:\.\d+)?)\b/i);
    const numbers = explicitRange
      ? [Number(explicitRange[1]), Number(explicitRange[2])].filter(Number.isFinite)
      : Array.from(instruction.matchAll(/-?\d+(?:\.\d+)?/g)).map(match => Number(match[0])).filter(Number.isFinite);
    let min = numbers.length >= 2 ? Math.min(numbers[0], numbers[1]) : null;
    let max = numbers.length >= 2 ? Math.max(numbers[0], numbers[1]) : null;
    const field = fields[0];
    const fieldMin = Number(field.getAttribute('min'));
    const fieldMax = Number(field.getAttribute('max'));
    if (min == null && Number.isFinite(fieldMin)) min = fieldMin;
    if (max == null && Number.isFinite(fieldMax)) max = fieldMax;
    if (min == null) min = 0;
    if (max == null) max = 100;
    if (!Number.isFinite(min) || !Number.isFinite(max) || min >= max) return null;
    return {{
      ok: true,
      action: 'feedback_loop_value',
      params: {{ instruction, min, max, maxAttempts: 16 }},
      confidence: 0.9,
      reason: 'planned bounded value search using visible page feedback after each submission',
      candidate: candidate(field),
      evidence: {{ min, max, hasField: true, submitControls: submitControls.length }}
    }};
  }}

  function scrollFillPressPlan() {{
    const hasFillRequest = kind === 'fill' || /\b(?:enter|type|fill|input|write)\b/i.test(instruction);
    if (!hasFillRequest || !/\bscroll\b/i.test(instruction) || !wantedValue) return null;
    const hasBox = el => {{
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return (r.width > 0 || r.height > 0) && s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
    }};
    const scrollable = all('textarea, [style*=overflow], [role=textbox], div').find(el => hasBox(el) && el.scrollHeight > el.clientHeight + 8);
	    const fields = writableFields().filter(el => {{
	      const tag = el.tagName.toLowerCase();
	      if (tag === 'textarea') return false;
	      return true;
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
  function scrollTextExtractPlan() {{
    if (!/\b(?:first|last)\s+word\b/i.test(instruction)) return null;
    if (!/\b(?:text\s*area|textarea|scroll|text|field)\b/i.test(instruction)) return null;
    const which = /\bfirst\s+word\b/i.test(instruction) ? 'first' : 'last';
    const hasBox = el => {{
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return (r.width > 0 || r.height > 0) && s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity || 1) !== 0;
    }};
    const sources = all('textarea, [role=textbox], [contenteditable]:not([contenteditable="false"]), [style*=overflow], div')
      .filter(el => {{
        if (!hasBox(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['script', 'style', 'button', 'select'].includes(tag)) return false;
        const value = readableText(el);
        if (!value || value.trim().split(/\s+/).filter(Boolean).length < 2) return false;
        if (el.closest && el.closest('#query, [data-role=query], .query')) return false;
        if (tag === 'textarea') return true;
        return el.scrollHeight > el.clientHeight + 8 || isEditableElement(el);
      }});
    const fields = writableFields()
      .filter(el => {{
        const tag = el.tagName.toLowerCase();
        if (tag === 'textarea' && sources.includes(el)) return false;
        if (sources.some(source => source === el || source.contains(el))) return false;
        return true;
      }});
    if (!sources.length || !fields.length) return null;
    const sourceRanked = best(sources, el => {{
      const tag = el.tagName.toLowerCase();
      const text = [textOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || ''].join(' ');
      let score = 0.3;
      if (tag === 'textarea') score += 0.65;
      if (el.scrollHeight > el.clientHeight + 8) score += 0.4;
      if (/\b(?:text|textarea|scroll|source|content)\b/i.test(text)) score += 0.2;
      return score;
    }});
    const targetRanked = best(fields, el => {{
      const tag = el.tagName.toLowerCase();
      const text = textOf(el);
      let score = targetHint ? Math.max(tokenScore(targetHint, text), semanticScore(targetHint, text)) : 0.3;
      if (/\b(?:field|input|answer|text)\b/i.test(text)) score += 0.2;
      if (tag === 'input' && /\b(?:text\s*field|field|input|answer)\b/i.test([targetHint || '', instruction].join(' '))) score += 0.35;
      return score;
    }});
    if (!sourceRanked.length || !targetRanked.length) return null;
    const source = sourceRanked[0].el;
    const target = targetRanked[0].el;
    const primary = {{
      ok: true,
      action: 'scroll_text_extract',
      params: {{ source: selector(source), target: selector(target), which }},
      confidence: Math.min(1, (sourceRanked[0].score + targetRanked[0].score) / 2),
      reason: 'extracted requested word from scrollable text source into target field',
      candidate: candidate(target),
      evidence: {{ source: candidate(source), target: candidate(target), which }}
    }};
    const follow = clickStepForHint(followUpClickHint(), target) || completionClickStep(target);
    if (!follow) return primary;
    return {{
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned scroll text extraction plus completion control'
    }};
  }}
  function resizeElementPlan() {{
    if (!/\bresize\b/i.test(instruction)) return null;
    if (!/\b(?:textarea|text\s*area|field|box|panel|element|editor)\b/i.test(instruction)) return null;
    const dimension = /\bwidth\b/i.test(instruction) ? 'width' : /\bheight\b/i.test(instruction) ? 'height' : 'both';
    const larger = /\b(?:larger|bigger|increase|grow|expand|taller|wider)\b/i.test(instruction);
    const smaller = /\b(?:smaller|decrease|shrink|shorter|narrower)\b/i.test(instruction);
    const sign = smaller && !larger ? -1 : 1;
    const candidates = all('textarea, [contenteditable]:not([contenteditable="false"]), [role=textbox], .resizable, [class*=resize], [style*=resize], div')
      .filter(el => {{
        if (!visible(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'button', 'input', 'select'].includes(tag)) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 20 || rect.height < 20 || rect.width * rect.height > 250000) return false;
        const style = getComputedStyle(el);
        const meta = [tag, el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || '', style.resize || ''].join(' ');
        return tag === 'textarea' || /\b(?:resize|resizable|textbox|editor|field|panel|box)\b/i.test(meta);
      }});
    const ranked = best(candidates, el => {{
      const tag = el.tagName.toLowerCase();
      const style = getComputedStyle(el);
      const text = [textOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', style.resize || ''].join(' ');
      let score = 0.25;
      if (tag === 'textarea') score += 0.75;
      if (style.resize && style.resize !== 'none') score += 0.3;
      if (/\b(?:resize|resizable|textarea|text\s*area|field|editor)\b/i.test(text)) score += 0.25;
      if (/\btextarea|text\s*area\b/i.test(instruction) && tag === 'textarea') score += 0.4;
      return score;
    }});
    if (!ranked.length) return null;
    const el = ranked[0].el;
    const rect = el.getBoundingClientRect();
    const delta = 36 * sign;
    let from = {{ x: rect.right - 3, y: rect.bottom - 3 }};
    let to = {{ x: from.x + delta, y: from.y + delta }};
    if (dimension === 'height') to = {{ x: from.x, y: from.y + delta }};
    if (dimension === 'width') to = {{ x: from.x + delta, y: from.y }};
    const drag = {{
      ok: true,
      action: 'drag',
      params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 18 }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'matched resize instruction to coordinate drag from element resize edge',
      candidate: candidate(el),
      evidence: {{
        dimension,
        direction: sign > 0 ? 'larger' : 'smaller',
        from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
        to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
      }}
    }};
    const follow = clickStepForHint(followUpClickHint(), el) || completionClickStep(el);
    if (!follow) return drag;
    return {{
      ok: true,
      action: 'sequence',
      steps: [drag, follow],
      confidence: Math.min(drag.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned resize drag plus completion control'
    }};
  }}
  function geometryDragPlan() {{
    if (kind !== 'drag') return null;
    if (!/\b(smaller|small|little|larger|large|big|inside|within|into|contain|contained)\b/i.test(instruction)) return null;
    function elementCueText(el) {{
      return [
        el.id || '',
        classText(el),
        el.getAttribute('data-testid') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        directTextOf(el),
      ].join(' ');
    }}
    function hasVisualBoxStyle(el) {{
      const style = getComputedStyle(el);
      const bg = String(style.backgroundColor || '').toLowerCase();
      const border = Number.parseFloat(style.borderTopWidth || '0') +
        Number.parseFloat(style.borderRightWidth || '0') +
        Number.parseFloat(style.borderBottomWidth || '0') +
        Number.parseFloat(style.borderLeftWidth || '0');
      if (border > 0) return true;
      if (bg && bg !== 'transparent' && bg !== 'rgba(0, 0, 0, 0)') return true;
      if (el.draggable || el.getAttribute('draggable') === 'true') return true;
      if (/\b(box|card|tile|drag|draggable|target|drop|large|small)\b/i.test(elementCueText(el))) return true;
      return false;
    }}
    const rawBoxes = all('div, span, button, li, td, canvas, svg, [draggable=true], [draggable="true"], [role=button], [role=img], [data-drop], [data-dropzone]')
      .filter(el => {{
        if (!visible(el)) return false;
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        if (rect.width < 8 || rect.height < 8 || area < 64 || area > 250000) return false;
        return hasVisualBoxStyle(el);
      }});
    const boxes = rawBoxes.filter(el => {{
      const rect = el.getBoundingClientRect();
      return !rawBoxes.some(other => {{
        if (other === el || !el.contains(other)) return false;
        const otherRect = other.getBoundingClientRect();
        return (otherRect.width * otherRect.height) < (rect.width * rect.height) * 0.8;
      }});
    }}).map(el => {{
      const rect = el.getBoundingClientRect();
      const cue = elementCueText(el);
      return {{ el, rect, area: rect.width * rect.height, cue }};
    }});
    if (boxes.length < 2) return null;
    const wantSmall = /\b(smaller|small|little|tiny)\b/i.test(instruction);
    const wantLarge = /\b(larger|large|big)\b/i.test(instruction);
    let bestPair = null;
    for (const source of boxes) {{
      for (const target of boxes) {{
        if (source.el === target.el) continue;
        if (target.area <= source.area * 1.15) continue;
        let score = 0.35 + Math.min(0.35, (target.area / Math.max(1, source.area)) / 20);
        if (wantSmall && /\b(small|smaller|tiny|little|source|drag)\b/i.test(source.cue)) score += 0.45;
        if (wantLarge && /\b(large|larger|big|target|drop)\b/i.test(target.cue)) score += 0.45;
        if (source.el.draggable || source.el.getAttribute('draggable') === 'true') score += 0.25;
        const sameParent = source.el.parentElement && source.el.parentElement === target.el.parentElement;
        if (sameParent) score += 0.2;
        if (/\binside|within|into|contain|contained\b/i.test(instruction)) score += 0.15;
        if (!bestPair || score > bestPair.score) bestPair = {{ source, target, score }};
      }}
    }}
    if (!bestPair || bestPair.score < 0.55) return null;
    const sourceRect = bestPair.source.rect;
    const targetRect = bestPair.target.rect;
    const drag = {{
      ok: true,
      action: 'drag',
      params: {{
        source: selector(bestPair.source.el),
        target: selector(bestPair.target.el),
        steps: 24,
      }},
      confidence: Math.min(1, bestPair.score),
      reason: 'matched relative-size geometry drag from visible page boxes',
      candidate: {{
        source: candidate(bestPair.source.el),
        target: candidate(bestPair.target.el),
      }},
      evidence: {{
        sourceArea: Math.round(sourceRect.width * sourceRect.height),
        targetArea: Math.round(targetRect.width * targetRect.height),
      }}
    }};
    const follow = completionClickStep(bestPair.target.el);
    if (!follow) return drag;
    return {{
      ok: true,
      action: 'sequence',
      steps: [drag, follow],
      confidence: Math.min(drag.confidence || 0.7, follow.confidence || 0.65),
      reason: 'planned relative geometry drag plus visible completion control'
    }};
  }}
  function listReorderDragPlan() {{
    if (kind !== 'drag') return null;
    const text = stripFollowUp(instruction);
    const relativeMatch = text.match(/\b(?:drag|move)\s+(?:the\s+)?(.+?)\s+(up|down|to\s+the\s+top|to\s+top|to\s+the\s+bottom|to\s+bottom)(?:\s+by\s+(\d+|one|two|three|four|five)\s+(?:positions?|places?|slots?|rows?|items?))?/i);
    const positionMatch = text.match(/\b(?:drag|move)\s+(?:the\s+)?(.+?)\s+to\s+(?:the\s+)?(\d+|one|two|three|four|five)(?:st|nd|rd|th)?\s+(?:position|place|slot|row|item)\b/i);
    if (!relativeMatch && !positionMatch) return null;
    const numberWords = {{ one: 1, two: 2, three: 3, four: 4, five: 5 }};
    const rawLabel = relativeMatch ? relativeMatch[1] : positionMatch[1];
    const label = String(rawLabel || '')
      .replace(/\b(?:item|row|option|entry|card)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
    const direction = relativeMatch ? String(relativeMatch[2] || '').toLowerCase().replace(/\s+/g, ' ') : 'position';
    const distanceToken = String(relativeMatch && relativeMatch[3] || 'one').toLowerCase();
    const distance = Number.parseInt(distanceToken, 10) || numberWords[distanceToken] || 1;
    const positionToken = positionMatch ? String(positionMatch[2] || '').toLowerCase() : null;
    const requestedPosition = positionToken ? (Number.parseInt(positionToken, 10) || numberWords[positionToken] || null) : null;
    if (!label) return null;

    function itemCue(el) {{
      return [directTextOf(el), textOf(el), el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
    }}
    function visibleItem(el) {{
      if (!visible(el)) return false;
      const tag = el.tagName.toLowerCase();
      if (['html', 'body', 'script', 'style', 'input', 'textarea', 'select'].includes(tag)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width < 8 || rect.height < 8 || rect.width * rect.height > 180000) return false;
      const role = roleOf(el);
      const meta = [el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      if (['div', 'span'].includes(tag)) {{
        return el.draggable || el.getAttribute('draggable') === 'true' ||
          ['listitem', 'option', 'row', 'treeitem'].includes(role) ||
          /\b(?:item|row|option|entry|sortable|draggable|drag-handle|handle)\b/i.test(meta);
      }}
      const cue = [itemCue(el), meta].join(' ');
      return ['li', 'tr'].includes(tag) || ['listitem', 'option', 'row', 'treeitem'].includes(role) ||
        el.draggable || el.getAttribute('draggable') === 'true' || /\b(?:item|row|option|entry|sortable|draggable|drag)\b/i.test(cue);
    }}
    const sourceRanked = best(all('[draggable=true], [draggable="true"], [role=listitem], [role=option], [role=row], [role=treeitem], li, tr, div, span'), el => {{
      if (!visibleItem(el)) return 0;
      const cue = itemCue(el);
      let score = Math.max(tokenScore(label, cue), exactPhraseScore(label, cue), semanticScore(label, cue));
      if (directTextOf(el).trim().toLowerCase() === label.toLowerCase()) score += 0.35;
      if (el.draggable || el.getAttribute('draggable') === 'true') score += 0.2;
      return score;
    }});
    if (!sourceRanked.length || sourceRanked[0].score < 0.35) return null;
    const source = sourceRanked[0].el;
    let container = source.parentElement && (source.parentElement.closest('[role=list], [role=listbox], [role=tree], ul, ol, tbody, table, [data-sortable], [class*=sortable], [class*=Sortable], [class*=list], [class*=List]') || source.parentElement);
    if (container === source) container = source.parentElement;
    if (!container) return null;
    let items = Array.from(container.children || []).filter(visibleItem);
    if (!items.includes(source)) {{
      const containing = items.find(item => item.contains(source));
      if (containing) {{
        items = items.map(item => item.contains(source) ? containing : item);
      }}
    }}
    items = items
      .filter((item, index, allItems) => allItems.indexOf(item) === index)
      .sort((a, b) => {{
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return Math.abs(ar.top - br.top) > 4 ? ar.top - br.top : ar.left - br.left;
      }});
    const sourceItem = items.find(item => item === source || item.contains(source) || source.contains(item));
    const sourceIndex = sourceItem ? items.indexOf(sourceItem) : -1;
    if (sourceIndex < 0 || items.length < 2) return null;
    let targetIndex = sourceIndex;
    if (requestedPosition) targetIndex = Math.max(0, Math.min(items.length - 1, requestedPosition - 1));
    else if (direction.includes('top')) targetIndex = 0;
    else if (direction.includes('bottom')) targetIndex = items.length - 1;
    else if (direction === 'up') targetIndex = Math.max(0, sourceIndex - distance);
    else if (direction === 'down') targetIndex = Math.min(items.length - 1, sourceIndex + distance);
    if (targetIndex === sourceIndex) return null;
    const from = rectCenter(sourceItem.getBoundingClientRect());
    const targetRect = items[targetIndex].getBoundingClientRect();
    let to = rectCenter(targetRect);
    if (direction === 'up' || direction.includes('top') || (requestedPosition && targetIndex < sourceIndex)) {{
      to = {{ x: targetRect.left + targetRect.width / 2, y: targetRect.top + Math.max(2, targetRect.height * 0.2) }};
    }} else if (direction === 'down' || direction.includes('bottom') || (requestedPosition && targetIndex > sourceIndex)) {{
      to = {{ x: targetRect.left + targetRect.width / 2, y: targetRect.bottom - Math.max(2, targetRect.height * 0.2) }};
    }}
    const drag = {{
      ok: true,
      action: 'drag',
      params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 28 }},
      confidence: Math.min(1, sourceRanked[0].score + 0.18),
      reason: 'matched relative list reorder instruction to visible item order',
      candidate: {{ source: candidate(sourceItem), target: candidate(items[targetIndex]) }},
      evidence: {{
        label,
        direction,
        distance,
        requestedPosition,
        sourceIndex,
        targetIndex,
        itemCount: items.length,
        from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
        to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
      }}
    }};
    return withFollowUp(drag, sourceItem);
  }}
  function gridSlotDragPlan() {{
    if (kind !== 'drag') return null;
    const text = stripFollowUp(instruction);
    const match = text.match(/\b(?:drag|move)\s+(?:the\s+)?(.+?)\s+(?:to\s+(?:the\s+)?(?:(top|center|middle|bottom)\s+(left|center|middle|right)|(left|center|middle|right)\s+(top|center|middle|bottom)|center)|(?:(up|down|left|right)\s+by\s+(one|two|three|four|five|\d+)))\b/i);
    if (!match) return null;
    const rawLabel = String(match[1] || '')
      .replace(/\b(?:item|row|option|entry|card)\b/ig, ' ')
      .replace(/^["'\s]+|["'.\s]+$/g, '')
      .replace(/\s+/g, ' ')
      .trim();
    if (!rawLabel) return null;
    const rowWord = (match[2] || match[5] || (/to\s+(?:the\s+)?center\b/i.test(text) ? 'center' : '')).toLowerCase();
    const colWord = (match[3] || match[4] || (/to\s+(?:the\s+)?center\b/i.test(text) ? 'center' : '')).toLowerCase();
    const relativeDir = (match[6] || '').toLowerCase();
    const numberWords = {{ one: 1, two: 2, three: 3, four: 4, five: 5 }};
    const distanceToken = String(match[7] || 'one').toLowerCase();
    const distance = Number.parseInt(distanceToken, 10) || numberWords[distanceToken] || 1;
    function itemCue(el) {{
      return [directTextOf(el), textOf(el), el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
    }}
    function visibleGridItem(el) {{
      if (!visible(el)) return false;
      const tag = el.tagName.toLowerCase();
      if (['html', 'body', 'script', 'style', 'input', 'textarea', 'select', 'button'].includes(tag)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width < 8 || rect.height < 8 || rect.width * rect.height > 180000) return false;
      const role = roleOf(el);
      const meta = [el.id || '', classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      return ['li', 'td', 'div', 'span'].includes(tag) || ['listitem', 'option', 'row', 'gridcell'].includes(role) ||
        el.draggable || el.getAttribute('draggable') === 'true' || /\b(?:item|row|option|entry|sortable|draggable|handle|cell)\b/i.test(meta);
    }}
    function cluster(values, tolerance = 6) {{
      const sorted = values.slice().sort((a, b) => a - b);
      const out = [];
      for (const value of sorted) {{
        const existing = out.find(item => Math.abs(item - value) <= tolerance);
        if (existing == null) out.push(value);
      }}
      return out;
    }}
    const containers = all('[role=list], [role=listbox], [role=grid], ul, ol, tbody, table, [data-sortable], [class*=sortable], [class*=Sortable], [class*=grid], [class*=Grid], div')
      .filter(el => visible(el) && el !== document.body && el !== document.documentElement);
    let bestGrid = null;
    for (const container of containers) {{
      const children = Array.from(container.children || []).filter(visibleGridItem);
      if (children.length < 4 || children.length > 80) continue;
      const items = children.map(el => {{
        const rect = el.getBoundingClientRect();
        return {{ el, rect, center: rectCenter(rect), cue: itemCue(el) }};
      }}).filter(item => item.rect.width >= 8 && item.rect.height >= 8);
      const xs = cluster(items.map(item => item.center.x));
      const ys = cluster(items.map(item => item.center.y));
      if (xs.length < 2 || ys.length < 2 || items.length < xs.length * ys.length * 0.6) continue;
      const source = best(items.map(item => item.el), el => {{
        const cue = itemCue(el);
        let score = Math.max(tokenScore(rawLabel, cue), exactPhraseScore(rawLabel, cue), semanticScore(rawLabel, cue));
        if ((directTextOf(el) || '').trim().toLowerCase() === rawLabel.toLowerCase()) score += 0.35;
        return score;
      }})[0];
      if (!source || source.score < 0.35) continue;
      const sourceItem = items.find(item => item.el === source.el || item.el.contains(source.el) || source.el.contains(item.el));
      if (!sourceItem) continue;
      const colIndex = xs.reduce((bestIndex, value, index) => Math.abs(value - sourceItem.center.x) < Math.abs(xs[bestIndex] - sourceItem.center.x) ? index : bestIndex, 0);
      const rowIndex = ys.reduce((bestIndex, value, index) => Math.abs(value - sourceItem.center.y) < Math.abs(ys[bestIndex] - sourceItem.center.y) ? index : bestIndex, 0);
      let targetRow = rowIndex;
      let targetCol = colIndex;
      if (relativeDir === 'up') targetRow = Math.max(0, rowIndex - distance);
      else if (relativeDir === 'down') targetRow = Math.min(ys.length - 1, rowIndex + distance);
      else if (relativeDir === 'left') targetCol = Math.max(0, colIndex - distance);
      else if (relativeDir === 'right') targetCol = Math.min(xs.length - 1, colIndex + distance);
      else {{
        if (rowWord === 'top') targetRow = 0;
        else if (rowWord === 'bottom') targetRow = ys.length - 1;
        else if (rowWord === 'center' || rowWord === 'middle') targetRow = Math.floor((ys.length - 1) / 2);
        if (colWord === 'left') targetCol = 0;
        else if (colWord === 'right') targetCol = xs.length - 1;
        else if (colWord === 'center' || colWord === 'middle') targetCol = Math.floor((xs.length - 1) / 2);
      }}
      if (targetRow === rowIndex && targetCol === colIndex) continue;
      const score = source.score + 0.35 + Math.min(0.2, (xs.length * ys.length) / 80);
      if (!bestGrid || score > bestGrid.score) {{
        bestGrid = {{ container, items, sourceItem, sourceScore: source.score, xs, ys, rowIndex, colIndex, targetRow, targetCol, score }};
      }}
    }}
    if (!bestGrid) return null;
    const from = bestGrid.sourceItem.center;
    const to = {{ x: bestGrid.xs[bestGrid.targetCol], y: bestGrid.ys[bestGrid.targetRow] }};
    const drag = {{
      ok: true,
      action: 'drag',
      params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 30 }},
      confidence: Math.min(1, bestGrid.score),
      reason: 'matched two-dimensional grid drag instruction to visible item slots',
      candidate: {{ source: candidate(bestGrid.sourceItem.el), target: candidate(bestGrid.container) }},
      evidence: {{
        label: rawLabel,
        sourceRow: bestGrid.rowIndex,
        sourceCol: bestGrid.colIndex,
        targetRow: bestGrid.targetRow,
        targetCol: bestGrid.targetCol,
        rows: bestGrid.ys.length,
        cols: bestGrid.xs.length,
        from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
        to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
      }}
    }};
    return withFollowUp(drag, bestGrid.container);
  }}
  function numericSortDragPlan() {{
    if (kind !== 'drag') return null;
    if (!/\b(?:sort|order|arrange)\b/i.test(instruction) || !/\b(?:numbers?|numeric|lowest|highest|increasing|decreasing|ascending|descending)\b/i.test(instruction)) return null;
    const ascending = !/\b(?:decreasing|descending|highest\s+(?:number\s+)?at\s+the\s+top|largest\s+(?:number\s+)?at\s+the\s+top)\b/i.test(instruction);
    function itemNumber(el) {{
      const match = (directTextOf(el) || textOf(el)).match(/-?\d+(?:\.\d+)?/);
      return match ? Number(match[0]) : null;
    }}
    function visibleSortItem(el) {{
      if (!visible(el)) return false;
      const tag = el.tagName.toLowerCase();
      if (['html', 'body', 'script', 'style', 'input', 'textarea', 'select', 'button'].includes(tag)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width < 10 || rect.height < 8 || rect.width * rect.height > 180000) return false;
      return itemNumber(el) != null;
    }}
    const containers = all('[role=list], [role=listbox], ul, ol, tbody, table, [data-sortable], [class*=sortable], [class*=Sortable], div')
      .filter(el => visible(el) && el !== document.body && el !== document.documentElement);
    let bestList = null;
    for (const container of containers) {{
      const children = Array.from(container.children || []).filter(visibleSortItem);
      if (children.length < 2 || children.length > 30) continue;
      const items = children.map((el, index) => {{
        const rect = el.getBoundingClientRect();
        return {{ el, index, rect, center: rectCenter(rect), value: itemNumber(el) }};
      }}).filter(item => Number.isFinite(item.value));
      if (items.length < 2) continue;
      const sorted = items.slice().sort((a, b) => ascending ? a.value - b.value : b.value - a.value);
      const alreadySorted = sorted.every((item, index) => item.el === items[index].el);
      let score = 0.55;
      const cue = [textOf(container), classText(container), container.id || '', container.getAttribute('role') || ''].join(' ');
      if (/\b(?:sortable|list|order|numbers?)\b/i.test(cue)) score += 0.2;
      if (items.some(item => item.el.draggable || item.el.getAttribute('draggable') === 'true')) score += 0.1;
      if (alreadySorted) score += 0.08;
      if (!bestList || score > bestList.score) bestList = {{ container, items, sorted, alreadySorted, score }};
    }}
    if (!bestList) return null;
    if (bestList.alreadySorted) {{
      const follow = completionClickStep(bestList.container);
      if (!follow) return null;
      return {{
        ok: true,
        action: 'sequence',
        steps: [follow],
        confidence: Math.min(1, bestList.score),
        reason: 'recognized visible numeric list already in requested sorted order and planned completion control',
        evidence: {{
          order: ascending ? 'ascending' : 'descending',
          values: bestList.items.map(item => item.value),
          sortedValues: bestList.sorted.map(item => item.value),
          moves: 0
        }}
      }};
    }}
    const current = bestList.items.slice();
    const slotCenters = bestList.items.map(item => item.center);
    const steps = [];
    for (let targetIndex = 0; targetIndex < bestList.sorted.length; targetIndex++) {{
      const desired = bestList.sorted[targetIndex];
      const currentIndex = current.findIndex(item => item.el === desired.el);
      if (currentIndex < 0 || currentIndex === targetIndex) continue;
      const from = slotCenters[currentIndex];
      const target = slotCenters[targetIndex];
      const to = currentIndex > targetIndex
        ? {{ x: target.x, y: target.y - Math.max(3, bestList.items[targetIndex].rect.height * 0.35) }}
        : {{ x: target.x, y: target.y + Math.max(3, bestList.items[targetIndex].rect.height * 0.35) }};
      steps.push({{
        ok: true,
        action: 'drag',
        params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 28 }},
        confidence: Math.min(1, bestList.score),
        reason: 'moved visible numeric list item toward sorted order',
        candidate: {{ source: candidate(desired.el), target: candidate(bestList.container) }},
        evidence: {{
          value: desired.value,
          fromIndex: currentIndex,
          toIndex: targetIndex,
          from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
          to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
        }}
      }});
      const [moved] = current.splice(currentIndex, 1);
      current.splice(targetIndex, 0, moved);
    }}
    if (!steps.length) return null;
    const follow = completionClickStep(bestList.container);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, bestList.score),
      reason: 'planned visible numeric list sorting by repeated drag gestures',
      evidence: {{
        order: ascending ? 'ascending' : 'descending',
        values: bestList.items.map(item => item.value),
        sortedValues: bestList.sorted.map(item => item.value),
        moves: steps.filter(step => step.action === 'drag').length
      }}
    }};
  }}
  function visualShapePartitionDragPlan() {{
    if (kind !== 'drag') return null;
    const text = stripFollowUp(instruction);
    if (!/\b(?:drag|move)\b/i.test(text) || !/\b(?:all|every|everything\s+else|all\s+else)\b/i.test(text)) return null;
    if (!/\b(?:into|onto|to|inside|within)\b/i.test(text)) return null;

    const partitionMatch = text.match(/\b(?:drag|move)\s+(?:all|every)\s+(.+?)\s+(?:into|onto|to|inside|within)\s+(?:the\s+)?(.+?)(?=\s*(?:,|and)\s+(?:(?:drag|move)\s+)?(?:everything|all)\s+else\b|[.;]|$)(?:\s*(?:,|and)\s+(?:(?:drag|move)\s+)?(?:everything|all)\s+else\s+(?:into|onto|to|inside|within)\s+(?:the\s+)?(.+?)(?=[.;]|$))?/i);
    if (!partitionMatch) return null;
    const predicateText = String(partitionMatch[1] || '').trim();
    const primaryTargetText = String(partitionMatch[2] || '').trim();
    const elseTargetText = String(partitionMatch[3] || '').trim();
    if (!predicateText || !primaryTargetText) return null;

    const colorWords = ['red', 'scarlet', 'orange', 'yellow', 'olive', 'lime', 'green', 'cyan', 'aqua', 'teal', 'blue', 'navy', 'indigo', 'purple', 'violet', 'magenta', 'pink', 'brown', 'gold', 'black', 'white', 'gray', 'grey', 'silver'];
    const shapeAliases = {{
      circle: ['circle', 'circles', 'dot', 'dots', 'round'],
      square: ['square', 'squares', 'box', 'boxes', 'tile', 'tiles', 'cell', 'cells'],
      rectangle: ['rectangle', 'rectangles', 'rect', 'rects'],
      triangle: ['triangle', 'triangles'],
      polygon: ['polygon', 'polygons'],
      path: ['path', 'paths'],
      line: ['line', 'lines']
    }};
    function descriptorTokens(value) {{
      return String(value || '').toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
    }}
    function shapeDescriptors(value) {{
      const tokenSet = new Set(descriptorTokens(value));
      const out = [];
      for (const [shape, aliases] of Object.entries(shapeAliases)) {{
        if (aliases.some(alias => tokenSet.has(alias))) out.push(shape);
      }}
      if (tokenSet.has('shape') || tokenSet.has('shapes') || tokenSet.has('item') || tokenSet.has('items') || tokenSet.has('object') || tokenSet.has('objects')) out.push('object');
      return out;
    }}
    function colorDescriptors(value) {{
      const tokenSet = new Set(descriptorTokens(value));
      return colorWords.filter(color => tokenSet.has(color));
    }}
    const wantedShapes = shapeDescriptors(predicateText).filter(shape => shape !== 'object');
    const wantedColors = colorDescriptors(predicateText);
    const acceptsAnyObject = shapeDescriptors(predicateText).includes('object') && !wantedShapes.length;
    if (!wantedShapes.length && !wantedColors.length && !acceptsAnyObject) return null;

    function cueText(el) {{
      return [
        el.id || '',
        classText(el),
        el.getAttribute('data-testid') || '',
        el.getAttribute('data-shape') || '',
        el.getAttribute('data-color') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        directTextOf(el)
      ].join(' ');
    }}
    function shapeNameOfDragItem(el) {{
      const tag = el.tagName.toLowerCase();
      const cue = cueText(el).toLowerCase();
      if (tag === 'circle' || tag === 'ellipse') return 'circle';
      if (tag === 'rect') {{
        const rect = el.getBoundingClientRect();
        const aspect = rect.width > 0 && rect.height > 0 ? rect.width / rect.height : 1;
        return aspect >= 0.78 && aspect <= 1.28 ? 'square' : 'rectangle';
      }}
      if (tag === 'polygon') {{
        const points = String(el.getAttribute('points') || '').trim().split(/\s+/).filter(Boolean);
        return points.length === 3 ? 'triangle' : 'polygon';
      }}
      if (tag === 'path') return cue.includes('triangle') ? 'triangle' : 'path';
      if (tag === 'line') return 'line';
      if (/\b(circle|dot|round)\b/.test(cue)) return 'circle';
      if (/\b(triangle)\b/.test(cue)) return 'triangle';
      if (/\b(square|box|tile|cell)\b/.test(cue)) return 'square';
      if (/\b(rectangle|rect)\b/.test(cue)) return 'rectangle';
      return 'object';
    }}
    function shapeMatchesDescriptor(actual, wanted) {{
      if (!wanted || wanted === 'object') return true;
      if (actual === wanted) return true;
      if (wanted === 'circle' && actual === 'circle') return true;
      if (wanted === 'square' && actual === 'square') return true;
      if (wanted === 'rectangle' && actual === 'rectangle') return true;
      if (wanted === 'triangle' && actual === 'triangle') return true;
      if (wanted === 'polygon' && (actual === 'polygon' || actual === 'triangle')) return true;
      return false;
    }}
    function parseRgb(value) {{
      const match = String(value || '').match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/i);
      return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
    }}
    function strokeRgb(el) {{
      const style = getComputedStyle(el);
      return parseRgb(style.stroke || el.getAttribute('stroke') || style.borderTopColor || '');
    }}
    function itemColorMatches(el, color) {{
      const metadata = cueText(el).toLowerCase().split(/[^a-z]+/);
      if (metadata.includes(color)) return true;
      return colorFamilyMatch(elementVisualRgb(el), color) || colorFamilyMatch(strokeRgb(el), color);
    }}
    function isTargetLikeRect(el) {{
      const tag = el.tagName.toLowerCase();
      const rect = el.getBoundingClientRect();
      const cue = cueText(el);
      const style = getComputedStyle(el);
      const fill = String(style.fill || el.getAttribute('fill') || '').toLowerCase();
      const bg = String(style.backgroundColor || '').toLowerCase();
      const stroke = String(style.stroke || el.getAttribute('stroke') || '').toLowerCase();
      const borderWidth = Number.parseFloat(style.borderTopWidth || '0') +
        Number.parseFloat(style.borderRightWidth || '0') +
        Number.parseFloat(style.borderBottomWidth || '0') +
        Number.parseFloat(style.borderLeftWidth || '0');
      const hasOutline = borderWidth > 0 || (stroke && stroke !== 'none' && stroke !== 'transparent');
      const transparentFill = !fill || fill === 'none' || fill === 'transparent' || fill === 'rgba(0, 0, 0, 0)';
      const transparentBackground = !bg || bg === 'transparent' || bg === 'rgba(0, 0, 0, 0)';
      return /\b(?:target|drop|dropzone|box|container|bucket|zone|bin)\b/i.test(cue) ||
        el.hasAttribute('data-drop') || el.hasAttribute('data-dropzone') ||
        ((tag === 'rect' || tag === 'div' || tag === 'section') && hasOutline && (transparentFill || transparentBackground) && rect.width >= 24 && rect.height >= 24);
    }}
    const targetCandidates = all('svg rect, [data-drop], [data-dropzone], [droppable], [role=region], div, section')
      .filter(el => {{
        if (!visible(el)) return false;
        if (el.id === 'query' || (el.closest && el.closest('#query, [data-role=query], .query'))) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 24 || rect.height < 24 || rect.width * rect.height < 700 || rect.width * rect.height > 220000) return false;
        return isTargetLikeRect(el);
      }}).map(el => {{
        const rect = el.getBoundingClientRect();
        return {{ el, rect, center: rectCenter(rect), cue: cueText(el), area: rect.width * rect.height }};
      }});
    if (!targetCandidates.length) return null;

    function cleanTargetDescriptor(value) {{
      return String(value || '').toLowerCase()
        .replace(/\b(?:the|a|an|box|boxes|container|target|area|zone|bucket|bin|square|rectangle|rect)\b/g, ' ')
        .replace(/\s+/g, ' ')
        .trim();
    }}
    function targetScore(target, descriptor, fallbackSide) {{
      const clean = cleanTargetDescriptor(descriptor);
      const tokens = descriptorTokens(clean);
      let score = 0.25;
      const cue = target.cue;
      if (clean) score += Math.max(tokenScore(clean, cue), exactPhraseScore(clean, cue), semanticScore(clean, cue)) * 0.55;
      for (const color of colorDescriptors(clean)) {{
        if (colorFamilyMatch(elementVisualRgb(target.el), color) || colorFamilyMatch(strokeRgb(target.el), color) || descriptorTokens(cue).includes(color)) score += 0.45;
      }}
      const sideTokens = tokens.filter(token => ['left', 'right', 'top', 'bottom', 'center', 'middle'].includes(token));
      for (const side of sideTokens.length ? sideTokens : (fallbackSide ? [fallbackSide] : [])) {{
        if (side === 'left') score += 0.3 * (1 - target.center.x / Math.max(1, window.innerWidth));
        if (side === 'right') score += 0.3 * (target.center.x / Math.max(1, window.innerWidth));
        if (side === 'top') score += 0.3 * (1 - target.center.y / Math.max(1, window.innerHeight));
        if (side === 'bottom') score += 0.3 * (target.center.y / Math.max(1, window.innerHeight));
        if (side === 'center' || side === 'middle') {{
          const dx = Math.abs(target.center.x - window.innerWidth / 2) / Math.max(1, window.innerWidth);
          const dy = Math.abs(target.center.y - window.innerHeight / 2) / Math.max(1, window.innerHeight);
          score += 0.25 * (1 - Math.min(1, dx + dy));
        }}
      }}
      if (isTargetLikeRect(target.el)) score += 0.16;
      return score;
    }}
    function chooseTarget(descriptor, exclude) {{
      const ranked = targetCandidates
        .filter(target => !exclude || target.el !== exclude.el)
        .map(target => {{ return {{ ...target, score: targetScore(target, descriptor) }}; }})
        .sort((a, b) => b.score - a.score || b.area - a.area);
      if (!ranked.length || ranked[0].score < 0.35) return null;
      return ranked[0];
    }}
    const primaryTarget = chooseTarget(primaryTargetText, null);
    if (!primaryTarget) return null;
    const elseTarget = elseTargetText ? chooseTarget(elseTargetText, primaryTarget) : null;
    if (elseTargetText && !elseTarget) return null;

    const targetElements = new Set(targetCandidates.map(target => target.el));
    const itemCandidates = all('svg circle, svg ellipse, svg polygon, svg path, svg rect, svg line, [data-shape], [data-color], [draggable=true], [draggable="true"]')
      .filter(el => {{
        if (!visible(el) && !visibleGridPoint(el)) return false;
        if (targetElements.has(el) || isTargetLikeRect(el)) return false;
        if (el.id === 'query' || (el.closest && el.closest('#query, [data-role=query], .query'))) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 4 || rect.height < 4 || rect.width * rect.height > Math.max(12000, primaryTarget.area * 0.85)) return false;
        return true;
      }}).map(el => {{
        const rect = el.getBoundingClientRect();
        const actualShape = shapeNameOfDragItem(el);
        const matchesShape = acceptsAnyObject || !wantedShapes.length || wantedShapes.some(shape => shapeMatchesDescriptor(actualShape, shape));
        const matchesColor = !wantedColors.length || wantedColors.some(color => itemColorMatches(el, color));
        return {{
          el,
          rect,
          center: rectCenter(rect),
          actualShape,
          matchesPredicate: matchesShape && matchesColor
        }};
      }});
    const primaryItems = itemCandidates.filter(item => item.matchesPredicate);
    const elseItems = elseTarget ? itemCandidates.filter(item => !item.matchesPredicate) : [];
    if (!primaryItems.length || primaryItems.length + elseItems.length > 80) return null;

    function dropPoint(target, index, count) {{
      const rect = target.rect;
      const columns = Math.max(1, Math.ceil(Math.sqrt(Math.max(1, count))));
      const col = index % columns;
      const row = Math.floor(index / columns);
      const rows = Math.max(1, Math.ceil(count / columns));
      const x = rect.left + rect.width * ((col + 1) / (columns + 1));
      const y = rect.top + rect.height * ((row + 1) / (rows + 1));
      return {{ x, y }};
    }}
    const steps = [];
    const sortedPrimary = visualOrder(primaryItems.map(item => item.el)).map(el => primaryItems.find(item => item.el === el)).filter(Boolean);
    sortedPrimary.forEach((item, index) => {{
      const to = dropPoint(primaryTarget, index, sortedPrimary.length);
      steps.push({{
        ok: true,
        action: 'drag',
        params: {{ from_x: item.center.x, from_y: item.center.y, to_x: to.x, to_y: to.y, steps: 24 }},
        confidence: 0.82,
        reason: 'dragged visible shape matching requested visual predicate into target region',
        candidate: {{ source: candidate(item.el), target: candidate(primaryTarget.el) }},
        evidence: {{
          predicate: predicateText,
          target: primaryTargetText,
          shape: item.actualShape,
          matched: true,
          from: {{ x: Math.round(item.center.x), y: Math.round(item.center.y) }},
          to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
        }}
      }});
    }});
    if (elseTarget) {{
      const sortedElse = visualOrder(elseItems.map(item => item.el)).map(el => elseItems.find(item => item.el === el)).filter(Boolean);
      sortedElse.forEach((item, index) => {{
        const to = dropPoint(elseTarget, index, sortedElse.length);
        steps.push({{
          ok: true,
          action: 'drag',
          params: {{ from_x: item.center.x, from_y: item.center.y, to_x: to.x, to_y: to.y, steps: 24 }},
          confidence: 0.78,
          reason: 'dragged remaining visible shape into alternate target region',
          candidate: {{ source: candidate(item.el), target: candidate(elseTarget.el) }},
          evidence: {{
            predicate: predicateText,
            target: elseTargetText,
            shape: item.actualShape,
            matched: false,
            from: {{ x: Math.round(item.center.x), y: Math.round(item.center.y) }},
            to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
          }}
        }});
      }});
    }}
    const follow = completionClickStep(primaryTarget.el);
    if (follow) steps.push(follow);
    return {{
      ok: true,
      action: 'sequence',
      steps,
      confidence: elseTarget ? 0.8 : 0.84,
      reason: 'planned visual shape partitioning by generic SVG/DOM geometry and attributes',
      evidence: {{
        predicate: predicateText,
        shapes: wantedShapes,
        colors: wantedColors,
        primaryTarget: primaryTargetText,
        elseTarget: elseTargetText || null,
        primaryCount: primaryItems.length,
        elseCount: elseItems.length
      }}
    }};
  }}
  function directionVectorFromInstruction() {{
    const text = instruction.toLowerCase();
    if (/\b(?:vertical|north\s*(?:-|to)\s*south|south\s*(?:-|to)\s*north)\b/.test(text)) return {{ name: 'vertical', dx: 0, dy: 1 }};
    if (/\b(?:horizontal|east\s*(?:-|to)\s*west|west\s*(?:-|to)\s*east)\b/.test(text)) return {{ name: 'horizontal', dx: 1, dy: 0 }};
    if (/\b(?:left\s*(?:-|to)\s*right|from\s+left\s+to\s+right)\b/.test(text)) return {{ name: 'right', dx: 1, dy: 0 }};
    if (/\b(?:right\s*(?:-|to)\s*left|from\s+right\s+to\s+left)\b/.test(text)) return {{ name: 'left', dx: -1, dy: 0 }};
    if (/\b(?:top\s*(?:-|to)\s*bottom|from\s+top\s+to\s+bottom|downward|downwards)\b/.test(text)) return {{ name: 'down', dx: 0, dy: 1 }};
    if (/\b(?:bottom\s*(?:-|to)\s*top|from\s+bottom\s+to\s+top|upward|upwards)\b/.test(text)) return {{ name: 'up', dx: 0, dy: -1 }};
    const up = /\b(up|above|north)\b/.test(text);
    const down = /\b(down|below|south)\b/.test(text);
    const left = /\b(left|west)\b/.test(text);
    const right = /\b(right|east)\b/.test(text);
    let dx = (right ? 1 : 0) + (left ? -1 : 0);
    let dy = (down ? 1 : 0) + (up ? -1 : 0);
    if (!dx && !dy) return null;
    const length = Math.hypot(dx, dy) || 1;
    dx /= length;
    dy /= length;
    const name = [dy < 0 ? 'up' : dy > 0 ? 'down' : '', dx < 0 ? 'left' : dx > 0 ? 'right' : ''].filter(Boolean).join('-') || 'directional';
    return {{ name, dx, dy }};
  }}
  function rectCenter(rect) {{
    return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }};
  }}
  function lineCoordinatePairs() {{
    const match = instruction.match(/\bfrom\s*\(?\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*\)?\s*(?:to|through)\s*\(?\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*\)?/i);
    if (!match) return null;
    return {{
      from: {{ x: Number(match[1]), y: Number(match[2]) }},
      to: {{ x: Number(match[3]), y: Number(match[4]) }}
    }};
  }}
  function surfacePoint(rect, point) {{
    const x = Number(point.x);
    const y = Number(point.y);
    if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
    if (x >= 0 && y >= 0 && x <= rect.width && y <= rect.height) {{
      return {{ x: rect.left + x, y: rect.top + y }};
    }}
    return {{ x, y }};
  }}
  function drawingSurfaceCandidates() {{
    return all('canvas, svg, [role=application], [role=img], [data-drawing], [data-canvas], [class*=canvas], [class*=Canvas], [class*=draw], [class*=Draw], [class*=surface], [class*=Surface], [class*=whiteboard], [class*=Whiteboard]')
      .filter(el => {{
        if (!visible(el)) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 40 || rect.height < 30) return false;
        const area = rect.width * rect.height;
        if (area > 900000) return false;
        return true;
      }});
  }}
  function lineOrientationFromInstruction() {{
    const text = instruction.toLowerCase();
    if (/\bvertical\b/.test(text)) return 'vertical';
    if (/\bhorizontal\b/.test(text)) return 'horizontal';
    return null;
  }}
  function markerCenterOnSurface(surface) {{
    if (!/\b(?:through|cross|intersect|pass(?:es)?\s+through|runs?\s+through|over|across|around|center(?:ed)?|centre(?:d)?|marked|marker|point|dot|target)\b/i.test(instruction)) return null;
    const markerHint = targetHint || instruction;
    const raw = Array.from(surface.querySelectorAll ? surface.querySelectorAll('circle, ellipse, rect, polygon, path, line, [data-point], [data-marker], [class*=dot], [class*=Dot], [class*=point], [class*=Point], [class*=marker], [class*=Marker]') : [])
      .filter(el => visible(el));
    const ranked = best(raw, el => {{
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return 0;
      const tag = el.tagName.toLowerCase();
      const text = [textOf(el), directTextOf(el), classText(el), el.id || ''].join(' ');
      let score = Math.max(tokenScore(markerHint, text), exactPhraseScore(markerHint, text), semanticScore(markerHint, text)) * 0.4;
      if (/\b(?:dot|point|marker|target)\b/i.test(instruction)) {{
        if (tag === 'circle' || tag === 'ellipse') score += 0.65;
        if (/\b(?:dot|point|marker|target)\b/i.test(text)) score += 0.45;
      }}
      const area = rect.width * rect.height;
      const surfaceRect = surface.getBoundingClientRect();
      const surfaceArea = Math.max(1, surfaceRect.width * surfaceRect.height);
      if (area > 0 && area / surfaceArea < 0.05) score += 0.18;
      if (tag === 'path' || tag === 'line') score -= 0.2;
      return score;
    }});
    if (!ranked.length || ranked[0].score < 0.35) return null;
    return rectCenter(ranked[0].el.getBoundingClientRect());
  }}
  function circleDrawPlan() {{
    if (kind !== 'drag') return null;
    if (!/\b(draw|sketch|stroke)\b/i.test(instruction) || !/\b(circle|round|ellipse|oval|arc)\b/i.test(instruction)) return null;
    const surfaces = best(drawingSurfaceCandidates(), el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = 0.48;
      if (targetHint) score += Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text)) * 0.45;
      const tag = el.tagName.toLowerCase();
      if (tag === 'canvas') score += 0.25;
      if (tag === 'svg') score += 0.18;
      if (/\b(draw|canvas|surface|whiteboard|sketch|paint)\b/i.test(text)) score += 0.28;
      return score;
    }});
    if (!surfaces.length) return null;
    const surface = surfaces[0].el;
    const rect = surface.getBoundingClientRect();
    const center = markerCenterOnSurface(surface) || rectCenter(rect);
    const explicitRadius = (() => {{
      const match = instruction.match(/\b(?:radius|r)\s*(?:of\s*)?(\d+(?:\.\d+)?)\b/i);
      if (!match) return null;
      const value = Number(match[1]);
      return Number.isFinite(value) && value > 0 ? value : null;
    }})();
    const maxRadius = Math.max(6, Math.min(
      center.x - rect.left - 3,
      rect.right - center.x - 3,
      center.y - rect.top - 3,
      rect.bottom - center.y - 3
    ));
    const radius = Math.max(6, Math.min(explicitRadius || Math.min(rect.width, rect.height) * 0.24, maxRadius));
    const pointCount = /\barc\b/i.test(instruction) ? 24 : 48;
    const fullCircle = !/\barc\b/i.test(instruction);
    const end = fullCircle ? Math.PI * 2 : Math.PI * 1.5;
    const start = -Math.PI / 2;
    const points = [];
    for (let index = 0; index <= pointCount; index++) {{
      const angle = start + (end * index / pointCount);
      points.push({{
        x: center.x + Math.cos(angle) * radius,
        y: center.y + Math.sin(angle) * radius
      }});
    }}
    const draw = {{
      ok: true,
      action: 'draw_path',
      params: {{ points, button: 'left', step_delay_ms: 6 }},
      confidence: Math.min(1, surfaces[0].score),
      reason: 'matched shape drawing instruction to continuous mouse path on visible drawing surface',
      candidate: candidate(surface),
      evidence: {{
        surface: selector(surface),
        shape: fullCircle ? 'circle' : 'arc',
        center: {{ x: Math.round(center.x), y: Math.round(center.y) }},
        radius: Math.round(radius),
        pointCount: points.length
      }}
    }};
    const follow = clickStepForHint(followUpClickHint(), surface) ||
      (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(surface) : null);
    if (!follow) return draw;
    return {{
      ok: true,
      action: 'sequence',
      steps: [draw, follow],
      confidence: Math.min(draw.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned shape drawing path plus completion control'
    }};
  }}
  function angleBisectorPlan() {{
    if (kind !== 'drag') return null;
    if (!/\b(?:bisect|bisects|bisector|halve|split)\b/i.test(instruction) || !/\bangle\b/i.test(instruction)) return null;
    const surfaces = best(drawingSurfaceCandidates().filter(el => el.tagName.toLowerCase() === 'svg'), el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = 0.55;
      if (/\b(?:angle|geometry|graph|grid|svg|drawing|surface)\b/i.test(text)) score += 0.25;
      const pointCount = Array.from(el.querySelectorAll('circle, ellipse, [data-point], [data-marker]')).filter(visible).length;
      if (pointCount >= 3) score += 0.35;
      return score;
    }});
    for (const rankedSurface of surfaces) {{
      const svg = rankedSurface.el;
      function localPoint(el) {{
        const tag = el.tagName.toLowerCase();
        let x = Number(el.getAttribute('cx'));
        let y = Number(el.getAttribute('cy'));
        if ((tag === 'rect' || tag === 'foreignObject') && (!Number.isFinite(x) || !Number.isFinite(y))) {{
          x = Number(el.getAttribute('x')) + Number(el.getAttribute('width')) / 2;
          y = Number(el.getAttribute('y')) + Number(el.getAttribute('height')) / 2;
        }}
        if (!Number.isFinite(x) || !Number.isFinite(y)) {{
          try {{
            const box = el.getBBox();
            x = box.x + box.width / 2;
            y = box.y + box.height / 2;
          }} catch (_) {{}}
        }}
        return Number.isFinite(x) && Number.isFinite(y) ? {{ x, y }} : null;
      }}
	      function viewportPoint(point) {{
	        try {{
	          const svgPoint = svg.createSVGPoint();
	          svgPoint.x = point.x;
	          svgPoint.y = point.y;
          const matrix = svg.getScreenCTM();
          if (matrix) {{
            const transformed = svgPoint.matrixTransform(matrix);
            return {{ x: transformed.x, y: transformed.y }};
          }}
        }} catch (_) {{}}
        const rect = svg.getBoundingClientRect();
        const viewBox = svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0
          ? svg.viewBox.baseVal
          : {{ x: 0, y: 0, width: Number(svg.getAttribute('width')) || rect.width, height: Number(svg.getAttribute('height')) || rect.height }};
        return {{
          x: rect.left + ((point.x - viewBox.x) / Math.max(1, viewBox.width)) * rect.width,
	          y: rect.top + ((point.y - viewBox.y) / Math.max(1, viewBox.height)) * rect.height
	        }};
	      }}
	      function drawingEventPoint(point) {{
	        const rect = svg.getBoundingClientRect();
	        const hasViewBox = !!(svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0 && svg.viewBox.baseVal.height > 0);
	        if (!hasViewBox) {{
	          const parent = svg.parentElement;
	          if (parent && parent !== document.body && parent !== document.documentElement) {{
	            const parentRect = parent.getBoundingClientRect();
	            const offsetX = rect.left - parentRect.left;
	            const offsetY = rect.top - parentRect.top;
	            const parentLooksLikeDrawingFrame =
	              parentRect.width <= rect.width + 80 &&
	              parentRect.height <= rect.height + 120 &&
	              offsetX >= 0 && offsetY >= 0 &&
	              offsetX <= 16 && offsetY <= 16;
	            if (parentLooksLikeDrawingFrame) {{
	              const width = Number(svg.getAttribute('width')) || rect.width || 1;
	              const height = Number(svg.getAttribute('height')) || rect.height || 1;
	              return {{
	                x: parentRect.left + (point.x / Math.max(1, width)) * rect.width,
	                y: parentRect.top + (point.y / Math.max(1, height)) * rect.height,
	                mode: 'container-local-event'
	              }};
	            }}
	          }}
	        }}
	        const standard = viewportPoint(point);
	        return {{ x: standard.x, y: standard.y, mode: 'svg-screen-ctm' }};
	      }}
	      function svgCoordinateBounds() {{
	        if (svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0 && svg.viewBox.baseVal.height > 0) {{
	          const box = svg.viewBox.baseVal;
	          return {{ minX: box.x, minY: box.y, maxX: box.x + box.width, maxY: box.y + box.height }};
	        }}
	        const rect = svg.getBoundingClientRect();
	        const width = Number(svg.getAttribute('width')) || rect.width || 1;
	        const height = Number(svg.getAttribute('height')) || rect.height || 1;
	        return {{ minX: 0, minY: 0, maxX: width, maxY: height }};
	      }}
	      function maxDistanceInsideBounds(origin, dx, dy) {{
	        const bounds = svgCoordinateBounds();
	        const candidates = [];
	        if (dx > 0) candidates.push((bounds.maxX - origin.x) / dx);
	        if (dx < 0) candidates.push((bounds.minX - origin.x) / dx);
	        if (dy > 0) candidates.push((bounds.maxY - origin.y) / dy);
	        if (dy < 0) candidates.push((bounds.minY - origin.y) / dy);
	        const positive = candidates.filter(value => Number.isFinite(value) && value > 0);
	        return positive.length ? Math.max(0, Math.min(...positive) - 2) : 0;
	      }}
      const rawPoints = Array.from(svg.querySelectorAll('circle, ellipse, [data-point], [data-marker]'))
        .filter(visible)
        .map(el => {{
          const point = localPoint(el);
          if (!point) return null;
          const meta = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('fill') || '', el.getAttribute('stroke') || '', el.getAttribute('data-role') || ''].join(' ');
          return {{ el, point, meta }};
        }})
        .filter(Boolean);
      if (rawPoints.length < 3) continue;
      const lines = Array.from(svg.querySelectorAll('line')).map(line => ({{
        x1: Number(line.getAttribute('x1')),
        y1: Number(line.getAttribute('y1')),
        x2: Number(line.getAttribute('x2')),
        y2: Number(line.getAttribute('y2'))
      }})).filter(line => [line.x1, line.y1, line.x2, line.y2].every(Number.isFinite));
      function lineIncidence(point) {{
        let count = 0;
        for (const line of lines) {{
          if (Math.hypot(point.x - line.x1, point.y - line.y1) <= 6 || Math.hypot(point.x - line.x2, point.y - line.y2) <= 6) count += 1;
        }}
        return count;
      }}
      const rankedVertices = rawPoints.map(item => {{
        let score = lineIncidence(item.point) * 0.4;
        if (/\b(?:vertex|origin|center|centre|anchor|blue|start)\b/i.test(item.meta)) score += 0.8;
        if (/\b(?:endpoint|target|black)\b/i.test(item.meta)) score -= 0.3;
        return {{ ...item, score }};
      }}).sort((a, b) => b.score - a.score);
      const vertex = rankedVertices[0];
      if (!vertex || vertex.score < 0.35) continue;
      const endpoints = rawPoints
        .filter(item => item.el !== vertex.el)
        .map(item => {{
          let score = 0.3;
          if (/\b(?:endpoint|target|black)\b/i.test(item.meta)) score += 0.45;
          score += Math.min(0.35, Math.max(0, Math.hypot(item.point.x - vertex.point.x, item.point.y - vertex.point.y)) / 250);
          return {{ ...item, score }};
        }})
        .sort((a, b) => b.score - a.score)
        .slice(0, 2);
      if (endpoints.length < 2) continue;
      const vectors = endpoints.map(item => {{
        const dx = item.point.x - vertex.point.x;
        const dy = item.point.y - vertex.point.y;
        const length = Math.hypot(dx, dy);
        return length > 0 ? {{ dx: dx / length, dy: dy / length, length }} : null;
      }});
      if (!vectors[0] || !vectors[1]) continue;
      let dx = vectors[0].dx + vectors[1].dx;
      let dy = vectors[0].dy + vectors[1].dy;
      let directionLength = Math.hypot(dx, dy);
      if (directionLength < 0.01) {{
        dx = ((endpoints[0].point.x + endpoints[1].point.x) / 2) - vertex.point.x;
        dy = ((endpoints[0].point.y + endpoints[1].point.y) / 2) - vertex.point.y;
        directionLength = Math.hypot(dx, dy);
      }}
      if (directionLength < 0.01) continue;
      dx /= directionLength;
      dy /= directionLength;
	      const rayDistance = Math.min(vectors[0].length, vectors[1].length) * 0.7;
	      const boundsDistance = maxDistanceInsideBounds(vertex.point, dx, dy) * 0.815;
	      const distance = Math.max(12, rayDistance, boundsDistance);
	      const targetLocal = {{ x: vertex.point.x + dx * distance, y: vertex.point.y + dy * distance }};
	      const target = drawingEventPoint(targetLocal);
	      const click = {{
	        ok: true,
	        action: 'click',
	        params: {{ x: target.x, y: target.y, button: 'left' }},
        confidence: Math.min(1, rankedSurface.score),
        reason: 'computed visible SVG angle bisector from vertex and ray endpoints',
        candidate: candidate(svg),
        evidence: {{
	          surface: selector(svg),
	          vertex: {{ x: Math.round(vertex.point.x), y: Math.round(vertex.point.y) }},
	          endpoints: endpoints.map(item => ({{ x: Math.round(item.point.x), y: Math.round(item.point.y) }})),
	          target: {{ x: Math.round(target.x), y: Math.round(target.y) }},
	          coordinateMode: target.mode
	        }}
	      }};
      const follow = clickStepForHint(followUpClickHint(), svg) ||
        (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(svg) : null);
      if (!follow) return click;
      return {{
        ok: true,
        action: 'sequence',
        steps: [click, follow],
        confidence: Math.min(click.confidence || 0.78, follow.confidence || 0.65),
        reason: 'planned SVG angle-bisector click plus completion control'
      }};
    }}
    return null;
  }}
  function perpendicularPointConstructionPlan() {{
    if (!/\b(?:right\s*angle|perpendicular|orthogonal|90\s*(?:degree|deg)?)\b/i.test(instruction)) return null;
    if (!/\b(?:add|create|make|place|draw|click|point|angle)\b/i.test(instruction)) return null;
    const surfaces = best(drawingSurfaceCandidates().filter(el => el.tagName.toLowerCase() === 'svg'), el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = 0.55;
      if (/\b(?:angle|geometry|graph|grid|svg|drawing|surface|diagram)\b/i.test(text)) score += 0.25;
      const pointCount = Array.from(el.querySelectorAll('circle, ellipse, [data-point], [data-marker]')).filter(visible).length;
      const lineCount = Array.from(el.querySelectorAll('line, polyline, path')).filter(visible).length;
      if (pointCount >= 2) score += 0.3;
      if (lineCount >= 1) score += 0.2;
      return score;
    }});
    for (const rankedSurface of surfaces) {{
      const svg = rankedSurface.el;
      function localPoint(el) {{
        const tag = el.tagName.toLowerCase();
        let x = Number(el.getAttribute('cx'));
        let y = Number(el.getAttribute('cy'));
        if ((tag === 'rect' || tag === 'foreignObject') && (!Number.isFinite(x) || !Number.isFinite(y))) {{
          x = Number(el.getAttribute('x')) + Number(el.getAttribute('width')) / 2;
          y = Number(el.getAttribute('y')) + Number(el.getAttribute('height')) / 2;
        }}
        if (!Number.isFinite(x) || !Number.isFinite(y)) {{
          try {{
            const box = el.getBBox();
            x = box.x + box.width / 2;
            y = box.y + box.height / 2;
          }} catch (_) {{}}
        }}
        return Number.isFinite(x) && Number.isFinite(y) ? {{ x, y }} : null;
      }}
      function viewportPoint(point) {{
        try {{
          const svgPoint = svg.createSVGPoint();
          svgPoint.x = point.x;
          svgPoint.y = point.y;
          const matrix = svg.getScreenCTM();
          if (matrix) {{
            const transformed = svgPoint.matrixTransform(matrix);
            return {{ x: transformed.x, y: transformed.y }};
          }}
        }} catch (_) {{}}
        const rect = svg.getBoundingClientRect();
        const viewBox = svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0
          ? svg.viewBox.baseVal
          : {{ x: 0, y: 0, width: Number(svg.getAttribute('width')) || rect.width, height: Number(svg.getAttribute('height')) || rect.height }};
        return {{
          x: rect.left + ((point.x - viewBox.x) / Math.max(1, viewBox.width)) * rect.width,
          y: rect.top + ((point.y - viewBox.y) / Math.max(1, viewBox.height)) * rect.height
        }};
      }}
      function drawingEventPoint(point) {{
        const rect = svg.getBoundingClientRect();
        const hasViewBox = !!(svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0 && svg.viewBox.baseVal.height > 0);
        if (!hasViewBox) {{
          const parent = svg.parentElement;
          if (parent && parent !== document.body && parent !== document.documentElement) {{
            const parentRect = parent.getBoundingClientRect();
            const offsetX = rect.left - parentRect.left;
            const offsetY = rect.top - parentRect.top;
            const parentLooksLikeDrawingFrame =
              parentRect.width <= rect.width + 80 &&
              parentRect.height <= rect.height + 120 &&
              offsetX >= 0 && offsetY >= 0 &&
              offsetX <= 16 && offsetY <= 16;
            if (parentLooksLikeDrawingFrame) {{
              const width = Number(svg.getAttribute('width')) || rect.width || 1;
              const height = Number(svg.getAttribute('height')) || rect.height || 1;
              return {{
                x: parentRect.left + (point.x / Math.max(1, width)) * rect.width,
                y: parentRect.top + (point.y / Math.max(1, height)) * rect.height,
                mode: 'container-local-event'
              }};
            }}
          }}
        }}
        const standard = viewportPoint(point);
        return {{ x: standard.x, y: standard.y, mode: 'svg-screen-ctm' }};
      }}
      function svgCoordinateBounds() {{
        if (svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width > 0 && svg.viewBox.baseVal.height > 0) {{
          const box = svg.viewBox.baseVal;
          return {{ minX: box.x, minY: box.y, maxX: box.x + box.width, maxY: box.y + box.height }};
        }}
        const rect = svg.getBoundingClientRect();
        const width = Number(svg.getAttribute('width')) || rect.width || 1;
        const height = Number(svg.getAttribute('height')) || rect.height || 1;
        return {{ minX: 0, minY: 0, maxX: width, maxY: height }};
      }}
      function maxDistanceInsideBounds(origin, dx, dy) {{
        const bounds = svgCoordinateBounds();
        const candidates = [];
        if (dx > 0) candidates.push((bounds.maxX - origin.x) / dx);
        if (dx < 0) candidates.push((bounds.minX - origin.x) / dx);
        if (dy > 0) candidates.push((bounds.maxY - origin.y) / dy);
        if (dy < 0) candidates.push((bounds.minY - origin.y) / dy);
        const positive = candidates.filter(value => Number.isFinite(value) && value > 0);
        return positive.length ? Math.max(0, Math.min(...positive) - 3) : 0;
      }}
      const points = Array.from(svg.querySelectorAll('circle, ellipse, rect, [data-point], [data-marker]'))
        .filter(visible)
        .map(el => {{
          const point = localPoint(el);
          if (!point) return null;
          const meta = [textOf(el), directTextOf(el), classText(el), el.id || '', el.getAttribute('fill') || '', el.getAttribute('stroke') || '', el.getAttribute('data-role') || ''].join(' ');
          return {{ el, point, meta }};
        }})
        .filter(Boolean);
      if (points.length < 2) continue;
      const lines = Array.from(svg.querySelectorAll('line')).map(line => ({{
        x1: Number(line.getAttribute('x1')),
        y1: Number(line.getAttribute('y1')),
        x2: Number(line.getAttribute('x2')),
        y2: Number(line.getAttribute('y2'))
      }})).filter(line => [line.x1, line.y1, line.x2, line.y2].every(Number.isFinite));
      function nearestPoint(local) {{
        return points
          .map(item => ({{ ...item, distance: Math.hypot(item.point.x - local.x, item.point.y - local.y) }}))
          .sort((a, b) => a.distance - b.distance)[0] || null;
      }}
      let pair = null;
      for (const line of lines) {{
        const first = nearestPoint({{ x: line.x1, y: line.y1 }});
        const second = nearestPoint({{ x: line.x2, y: line.y2 }});
        if (first && second && first.el !== second.el && first.distance <= 8 && second.distance <= 8) {{
          pair = [first, second];
          break;
        }}
      }}
      if (!pair && points.length >= 2) pair = points.slice(0, 2);
      if (!pair) continue;
      function vertexScore(item) {{
        let score = 0.2;
        if (/\b(?:vertex|origin|anchor|start|active|selected|blue|primary)\b/i.test(item.meta)) score += 0.85;
        if (/\b(?:endpoint|target|black|fixed)\b/i.test(item.meta)) score -= 0.2;
        return score;
      }}
      const vertex = vertexScore(pair[0]) >= vertexScore(pair[1]) ? pair[0] : pair[1];
      const anchor = vertex.el === pair[0].el ? pair[1] : pair[0];
      const dx = anchor.point.x - vertex.point.x;
      const dy = anchor.point.y - vertex.point.y;
      const segmentLength = Math.hypot(dx, dy);
      if (segmentLength < 4) continue;
      const directions = [
        {{ dx: -dy / segmentLength, dy: dx / segmentLength }},
        {{ dx: dy / segmentLength, dy: -dx / segmentLength }}
      ].map(direction => {{
        const maxDistance = maxDistanceInsideBounds(vertex.point, direction.dx, direction.dy);
        return {{ ...direction, maxDistance }};
      }}).filter(direction => direction.maxDistance >= 12)
        .sort((a, b) => b.maxDistance - a.maxDistance);
      if (!directions.length) continue;
      const direction = directions[0];
      const distance = Math.max(12, Math.min(direction.maxDistance * 0.82, Math.max(24, segmentLength * 0.75)));
      const targetLocal = {{
        x: vertex.point.x + direction.dx * distance,
        y: vertex.point.y + direction.dy * distance
      }};
      const target = drawingEventPoint(targetLocal);
      const click = {{
        ok: true,
        action: 'click',
        params: {{ x: target.x, y: target.y, button: 'left' }},
        confidence: Math.min(1, rankedSurface.score),
        reason: 'computed perpendicular point from visible SVG segment and vertex',
        candidate: candidate(svg),
        evidence: {{
          surface: selector(svg),
          vertex: {{ x: Math.round(vertex.point.x), y: Math.round(vertex.point.y) }},
          anchor: {{ x: Math.round(anchor.point.x), y: Math.round(anchor.point.y) }},
          target: {{ x: Math.round(target.x), y: Math.round(target.y) }},
          coordinateMode: target.mode
        }}
      }};
      const follow = clickStepForHint(followUpClickHint(), svg) ||
        (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(svg) : null);
      if (!follow) return click;
      return {{
        ok: true,
        action: 'sequence',
        steps: [click, follow],
        confidence: Math.min(click.confidence || 0.78, follow.confidence || 0.65),
        reason: 'planned perpendicular point construction plus completion control'
      }};
    }}
    return null;
  }}
  function lineDrawPlan() {{
    if (kind !== 'drag') return null;
    if (!/\b(draw|sketch|stroke)\b/i.test(instruction) || !/\b(line|stroke|path)\b/i.test(instruction)) return null;
    const surfaces = best(drawingSurfaceCandidates(), el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || ''].join(' ');
      let score = 0.45;
      if (targetHint) score += Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text)) * 0.5;
      const tag = el.tagName.toLowerCase();
      if (tag === 'canvas') score += 0.25;
      if (tag === 'svg') score += 0.15;
      if (/\b(draw|canvas|surface|whiteboard|sketch)\b/i.test(text)) score += 0.25;
      return score;
    }});
    if (!surfaces.length) return null;
    const surface = surfaces[0].el;
    const rect = surface.getBoundingClientRect();
    const explicit = lineCoordinatePairs();
    let from = null;
    let to = null;
    if (explicit) {{
      from = surfacePoint(rect, explicit.from);
      to = surfacePoint(rect, explicit.to);
    }}
    if (!from || !to) {{
      const orientation = lineOrientationFromInstruction();
      const markerCenter = markerCenterOnSurface(surface);
      const center = markerCenter || rectCenter(rect);
      const direction = directionVectorFromInstruction();
      const margin = Math.max(8, Math.min(rect.width, rect.height) * 0.18);
      if (orientation === 'vertical') {{
        from = {{ x: center.x, y: rect.top + margin }};
        to = {{ x: center.x, y: rect.bottom - margin }};
      }} else if (orientation === 'horizontal') {{
        from = {{ x: rect.left + margin, y: center.y }};
        to = {{ x: rect.right - margin, y: center.y }};
      }} else if (direction && Math.abs(direction.dy) > 0 && Math.abs(direction.dx) > 0) {{
        from = {{ x: rect.left + (direction.dx > 0 ? margin : rect.width - margin), y: rect.top + (direction.dy > 0 ? margin : rect.height - margin) }};
        to = {{ x: rect.left + (direction.dx > 0 ? rect.width - margin : margin), y: rect.top + (direction.dy > 0 ? rect.height - margin : margin) }};
      }} else if (direction && Math.abs(direction.dy) > Math.abs(direction.dx)) {{
        from = {{ x: rect.left + rect.width / 2, y: rect.top + (direction.dy > 0 ? margin : rect.height - margin) }};
        to = {{ x: rect.left + rect.width / 2, y: rect.top + (direction.dy > 0 ? rect.height - margin : margin) }};
      }} else {{
        const leftToRight = !direction || direction.dx >= 0;
        from = {{ x: rect.left + (leftToRight ? margin : rect.width - margin), y: rect.top + rect.height / 2 }};
        to = {{ x: rect.left + (leftToRight ? rect.width - margin : margin), y: rect.top + rect.height / 2 }};
      }}
    }}
    const drag = {{
      ok: true,
      action: 'drag',
      params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 32 }},
      confidence: Math.min(1, surfaces[0].score),
      reason: 'matched drawing instruction to coordinate drag across visible drawing surface',
      candidate: candidate(surface),
      evidence: {{
        surface: selector(surface),
        from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
        to: {{ x: Math.round(to.x), y: Math.round(to.y) }},
        explicitCoordinates: !!explicit
      }}
    }};
    const follow = clickStepForHint(followUpClickHint(), surface) ||
      (/\b(?:submit|continue|confirm|done|save)\b/i.test(instruction) ? completionClickStep(surface) : null);
    if (!follow) return drag;
    return {{
      ok: true,
      action: 'sequence',
      steps: [drag, follow],
      confidence: Math.min(drag.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned drawing drag plus completion control'
    }};
  }}
  function visualOrientationPlan() {{
    if (kind !== 'drag') return null;
    const intentText = [instruction, targetHint || '', wantedValue || ''].join(' ');
    if (!/\b(?:active|front|facing|face|side|orient|rotate|turn|spin|move\s+around)\b/i.test(intentText)) return null;
    if (/\b(?:draw|sketch|line|circle|smaller|larger|inside|within|reorder|position|slot|row)\b/i.test(intentText)) return null;

    function orientationTarget() {{
      const quoted = instruction.match(/["']([^"']{{1,80}})["']/);
      if (quoted && quoted[1]) return quoted[1].trim();
      const patterns = [
        /\b(?:side|face|value|number|label)\s+([A-Za-z0-9][A-Za-z0-9 _.-]{{0,40}})\s+(?:is|becomes?|as)?\s*(?:active|front|facing)\b/i,
        /\b(?:make|set|show|move|turn|rotate|orient)\b[^,.]*?\b([A-Za-z0-9][A-Za-z0-9 _.-]{{0,40}})\s+(?:active|front|facing)\b/i,
        /\b(?:active|front|facing)\s+(?:side|face|value|number|label)?\s*([A-Za-z0-9][A-Za-z0-9 _.-]{{0,40}})\b/i
      ];
      for (const pattern of patterns) {{
        const match = instruction.match(pattern);
        if (!match || !match[1]) continue;
        const value = match[1]
          .replace(/\b(?:side|face|user|viewer|front|active|facing|the|a|an|to|of|around)\b/ig, ' ')
          .replace(/[.,;:]+$/g, '')
          .replace(/\s+/g, ' ')
          .trim();
        if (value) return value;
      }}
      return wantedValue || null;
    }}

    const target = orientationTarget();
    if (!target) return null;

    function cueText(el) {{
      return [
        el.id || '',
        classText(el),
        el.getAttribute('role') || '',
        el.getAttribute('aria-roledescription') || '',
        el.getAttribute('aria-label') || '',
        el.getAttribute('title') || '',
        el.getAttribute('data-testid') || '',
        directTextOf(el)
      ].join(' ');
    }}
    function activeFaceTexts(el) {{
      return all('.active, [aria-selected="true"], [aria-current="true"], [data-active="true"], [data-state="active"], [class*=active i], [class*=current i], [class*=front i], [class*=selected i]', el)
        .filter(visible)
        .map(child => directTextOf(child) || textOf(child))
        .map(text => text.replace(/\s+/g, ' ').trim())
        .filter(Boolean);
    }}
    function faceTexts(el) {{
      return all('*', el)
        .filter(visible)
        .map(child => directTextOf(child) || textOf(child))
        .map(text => text.replace(/\s+/g, ' ').trim())
        .filter(text => text && text.length <= 80);
    }}
    const candidates = all('canvas, svg, [role=img], [aria-roledescription], [data-active], [data-state], [class*=rotat i], [class*=orient i], [class*=cube i], [class*=carousel i], [class*=viewport i], [class*=viewer i], [class*=stage i], [class*=surface i], section, article, div')
      .filter(el => {{
        if (!visible(el)) return false;
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        if (rect.width < 20 || rect.height < 20 || area < 400 || area > 500000) return false;
        const tag = el.tagName.toLowerCase();
        if (tag === 'body' || tag === 'html') return false;
        return true;
      }});
    const ranked = best(candidates, el => {{
      const faces = faceTexts(el);
      const active = activeFaceTexts(el);
      const cue = cueText(el);
      let score = 0;
      if (faces.some(text => exactPhraseScore(target, text) > 0 || tokenScore(target, text) > 0.75)) score += 0.55;
      if (active.length) score += 0.35;
      if (/\b(?:rotat|orient|spin|turn|cube|face|side|carousel|viewport|viewer|dial|object|surface|stage)\b/i.test(cue)) score += 0.32;
      if (faces.length >= 2) score += Math.min(0.25, faces.length * 0.035);
      const rect = el.getBoundingClientRect();
      const area = rect.width * rect.height;
      if (area > 180000) score -= 0.15;
      if (active.some(text => exactPhraseScore(target, text) > 0 || tokenScore(target, text) > 0.75)) score += 0.2;
      return score;
    }});
    if (!ranked.length || ranked[0].score < 0.62) return null;
    const surface = ranked[0].el;
    const orient = {{
      ok: true,
      action: 'orient_visual',
      params: {{ selector: selector(surface), targetText: target, maxAttempts: 32 }},
      confidence: Math.min(1, ranked[0].score),
      reason: 'matched visual orientation target from active/front face affordances',
      candidate: candidate(surface),
      evidence: {{
        targetText: target,
        activeTexts: activeFaceTexts(surface).slice(0, 8),
        visibleFaceTexts: faceTexts(surface).slice(0, 16)
      }}
    }};
    const follow = completionClickStep(surface);
    if (!follow) return orient;
    return {{
      ok: true,
      action: 'sequence',
      steps: [orient, follow],
      confidence: Math.min(orient.confidence || 0.74, follow.confidence || 0.65),
      reason: 'planned visual orientation plus completion control'
    }};
  }}
  function directionalDragPlan() {{
    if (kind !== 'drag') return null;
    if (/\b(draw|sketch|stroke)\b/i.test(instruction)) return null;
    const direction = directionVectorFromInstruction();
    if (!direction) return null;
    const sourceHint = String(targetHint || instruction)
      .replace(/\b(?:drag|move|slide|pull|push)\b/ig, ' ')
      .replace(/\b(?:left|right|up|down|above|below|north|south|east|west|upward|upwards|downward|downwards)\b/ig, ' ')
      .replace(/\b(?:then|and)\s+(?:click|press|tap|hit|submit|save|confirm|done)\b.*$/ig, ' ')
      .replace(/\s+/g, ' ')
      .trim();
    function isSvgGraphicElement(el) {{
      const tag = el && el.tagName ? el.tagName.toLowerCase() : '';
      return ['circle', 'rect', 'ellipse', 'polygon', 'path', 'line', 'polyline'].includes(tag) && !!el.closest('svg');
    }}
    const sourceCandidates = all('[draggable=true], [draggable="true"], [data-draggable], [role=button], [role=img], svg circle, svg rect, svg ellipse, svg polygon, svg path, img, canvas, div, span, button')
      .filter(el => {{
        if (!visible(el) && !visibleGridPoint(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'input', 'select', 'textarea'].includes(tag)) return false;
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        if (rect.width < 6 || rect.height < 6 || area < 36 || area > 250000) return false;
        return true;
      }});
    const rankedSources = best(sourceCandidates, el => {{
      const text = [textOf(el), directTextOf(el), classText(el), el.id || ''].join(' ');
      const tag = el.tagName.toLowerCase();
      const visualText = [text, tag, el.getAttribute('fill') || '', el.getAttribute('stroke') || ''].join(' ');
      let score = sourceHint ? Math.max(tokenScore(sourceHint, text), exactPhraseScore(sourceHint, text), semanticScore(sourceHint, text)) : 0.15;
      if (el.draggable || el.getAttribute('draggable') === 'true' || el.hasAttribute('data-draggable')) score += 0.35;
      if (/\b(drag|draggable|token|item|shape|object|source|piece|marker)\b/i.test(visualText)) score += 0.2;
      if (isSvgGraphicElement(el)) {{
        score += 0.12;
        if (!sourceHint || /\b(?:item|object|shape|thing|piece|marker|token)\b/i.test(sourceHint)) score += 0.28;
      }} else if (el.closest('svg')) {{
        score += 0.08;
      }}
      const rect = el.getBoundingClientRect();
      const area = rect.width * rect.height;
      if (area > 120000) score -= 0.25;
      return score;
    }});
    if (!rankedSources.length || rankedSources[0].score < 0.28) return null;
    const source = rankedSources[0].el;
    const sourceRect = source.getBoundingClientRect();
    const from = rectCenter(sourceRect);
    const targetCandidates = all('[data-drop], [data-dropzone], [droppable], [role=region], [role=gridcell], [role=listbox], div, section, article, td, li, canvas, svg')
      .filter(el => {{
        if (!visible(el) || el === source || source.contains(el)) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width < 12 || rect.height < 12) return false;
        const center = rectCenter(rect);
        const vx = center.x - from.x;
        const vy = center.y - from.y;
        const distance = Math.hypot(vx, vy);
        if (distance < 12) return false;
        const alignment = (vx / distance) * direction.dx + (vy / distance) * direction.dy;
        return alignment > 0.72;
      }});
    const rankedTargets = best(targetCandidates, el => {{
      const rect = el.getBoundingClientRect();
      const center = rectCenter(rect);
      const distance = Math.hypot(center.x - from.x, center.y - from.y);
      let score = 0.25 + Math.max(0, 1 - distance / 1000) * 0.3;
      const text = [textOf(el), directTextOf(el), classText(el), el.id || ''].join(' ');
      if (el.hasAttribute('data-drop') || el.hasAttribute('data-dropzone') || el.hasAttribute('droppable')) score += 0.35;
      if (/\b(drop|target|zone|destination)\b/i.test(text)) score += 0.2;
      return score;
    }});
    let to = null;
    let target = null;
    if (rankedTargets.length) {{
      target = rankedTargets[0].el;
      to = rectCenter(target.getBoundingClientRect());
    }} else {{
      const distance = Math.max(48, Math.min(160, Math.max(sourceRect.width, sourceRect.height) * 2.5));
      to = {{
        x: Math.max(1, Math.min(window.innerWidth - 1, from.x + direction.dx * distance)),
        y: Math.max(1, Math.min(window.innerHeight - 1, from.y + direction.dy * distance))
      }};
    }}
    const dragPlan = {{
      ok: true,
      action: 'drag',
      params: {{ from_x: from.x, from_y: from.y, to_x: to.x, to_y: to.y, steps: 24 }},
      confidence: Math.min(1, rankedSources[0].score + (target ? 0.15 : 0)),
      reason: target
        ? 'matched directional drag source to aligned visible drop target'
        : 'matched directional drag source and inferred target coordinates from instruction direction',
      candidate: target ? {{ source: candidate(source), target: candidate(target) }} : {{ source: candidate(source) }},
      evidence: {{
        direction: direction.name,
        sourceHint,
        inferredTarget: !target,
        from: {{ x: Math.round(from.x), y: Math.round(from.y) }},
        to: {{ x: Math.round(to.x), y: Math.round(to.y) }}
      }}
    }};
    return withFollowUp(dragPlan, source);
  }}
  function menuPathPlan() {{
    const menuLikeCount = all('[role=menu], [role=menubar], [role=tree], ul, nav, .ui-menu, .menu, [class*=menu], [class*=Menu]').filter(visible).length;
    const path = intent && Array.isArray(intent.menuPath) && intent.menuPath.length
      ? intent.menuPath
      : (kind === 'select_option' && wantedValue ? [wantedValue] : []);
    if (!path.length || (!menuLikeCount && path.length < 2)) return null;
    const candidates = clickableElements().concat(all('li, div, span, [role=menuitem], [role=treeitem], .ui-menu-item, .ui-menu-item-wrapper'));
    const visibleMatches = path.filter(label => {{
      return best(candidates, el => {{
        const text = [textOf(el), iconSemanticText(el)].join(' ');
        return Math.max(tokenScore(label, text), exactPhraseScore(label, text), semanticScore(label, text));
      }}).length > 0;
    }}).length;
    if (!menuLikeCount && visibleMatches === 0) return null;
    if (path.length === 1 && visibleMatches === 0) return null;
    return {{
      action: 'select_menu_path',
      params: {{ path }},
      confidence: visibleMatches === path.length ? 0.86 : 0.72,
      reason: path.length > 1 ? 'matched hierarchical menu or tree path from structured instruction intent' : 'matched visible menu item for select instruction',
      evidence: {{ menuLikeCount, visiblePathMatches: visibleMatches, pathLength: path.length }}
    }};
  }}
  function analyzeFormPlan() {{
    if (kind !== 'analyze_form') return null;
    const formFieldSelector = 'input:not([type=hidden]), select, textarea, [contenteditable]:not([contenteditable="false"]), [role~="textbox"], [role~="searchbox"], [role~="spinbutton"], [role~="slider"], [role~="combobox"], [aria-haspopup]';
    if (!targetHint) {{
      return {{
        ok: true,
        action: 'analyze_form',
        params: {{}},
        confidence: 0.9,
        reason: 'planned form analysis from instruction'
      }};
    }}
    const forms = best(all('form, [role=form], section, article, dialog, [role=dialog], [data-testid], .form, .Form, .checkout, .signup, .login, div'), el => {{
      const fields = all(formFieldSelector, el).filter(visible);
      if (!fields.length) return 0;
      const text = [textOf(el), directTextOf(el), iconSemanticText(el)].join(' ');
      const direct = [directTextOf(el), iconSemanticText(el)].join(' ');
      const directScore = Math.max(tokenScore(targetHint, direct), exactPhraseScore(targetHint, direct), semanticScore(targetHint, direct));
      const contentScore = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      let score = Math.max(directScore, contentScore * 0.55);
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const classes = classText(el);
      if (tag === 'form' || role === 'form') score += 0.25;
      if (/\b(form|checkout|signup|sign up|login|fields?|inputs?)\b/i.test([targetHint, classes, text].join(' '))) score += 0.12;
      score += Math.min(0.2, fields.length * 0.03);
      if (tag === 'body' || tag === 'html') score -= 0.35;
      return score;
    }});
    if (!forms.length) return null;
    const chosen = forms[0];
    return {{
      ok: true,
      action: 'analyze_form',
      params: {{ selector: selector(chosen.el) }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched form analysis target by semantic DOM text and field affordances',
      candidate: candidate(chosen.el)
    }};
  }}
  if (kind === 'analyze_form') {{
    const formPlan = analyzeFormPlan();
    if (formPlan) return formPlan;
    return {{ ok: false, error: 'act_instruction: no analyzable form target found' }};
  }}
  function accessibilityTreePlan() {{
    if (kind !== 'accessibility_tree') return null;
    if (!targetHint) {{
      return {{
        ok: true,
        action: 'accessibility_tree',
        params: {{ max_depth: 10, max_count: 100 }},
        confidence: 0.9,
        reason: 'planned page accessibility tree inspection'
      }};
    }}
    const elements = best(all('main, section, article, aside, header, footer, nav, form, dialog, table, tbody, tr, ul, ol, li, [role=dialog], [role=region], [role=main], [role=navigation], [role=form], [role=group], [role=row], [role=list], [role=table], [data-testid], .card, .panel, .modal, .row, div'), el => {{
      const text = [textOf(el), directTextOf(el), iconSemanticText(el)].join(' ');
      const direct = [directTextOf(el), iconSemanticText(el)].join(' ');
      const directScore = Math.max(tokenScore(targetHint, direct), exactPhraseScore(targetHint, direct), semanticScore(targetHint, direct));
      const contentScore = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      let score = Math.max(directScore, contentScore * 0.6);
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const classes = classText(el);
      if (['main', 'section', 'article', 'aside', 'header', 'footer', 'nav', 'form', 'dialog', 'table', 'tr', 'li'].includes(tag)) score += 0.12;
      if (['dialog', 'region', 'main', 'navigation', 'form', 'group', 'row', 'list', 'table'].includes(role)) score += 0.12;
      if (/\b(card|panel|modal|dialog|region|section|nav|navigation|table|row|list|form)\b/i.test([targetHint, classes, text].join(' '))) score += 0.08;
      if (tag === 'body' || tag === 'html') score -= 0.3;
      return score;
    }});
    if (!elements.length) return null;
    const chosen = elements[0];
    return {{
      ok: true,
      action: 'accessibility_tree',
      params: {{ selector: selector(chosen.el), max_depth: 10, max_count: 100 }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched accessibility tree target by semantic DOM text and landmark affordances',
      candidate: candidate(chosen.el)
    }};
  }}
  if (kind === 'accessibility_tree') {{
    const treePlan = accessibilityTreePlan();
    if (treePlan) return treePlan;
    return {{ ok: false, error: 'act_instruction: no accessibility tree target found' }};
  }}
  function readTextPlan() {{
    if (kind !== 'read_text') return null;
    if (!targetHint) {{
      return {{
        ok: true,
        action: 'read_text',
        params: {{ max_length: 20000 }},
        confidence: 0.9,
        reason: 'planned visible page text read'
      }};
    }}
    const elements = best(all('main, section, article, aside, header, footer, nav, form, dialog, table, tbody, tr, li, [role=dialog], [role=region], [role=main], [role=group], [role=row], [data-testid], .card, .panel, .modal, .row, div, p, output, pre, code, span'), el => {{
      const text = [textOf(el), directTextOf(el), iconSemanticText(el)].join(' ');
      const direct = [directTextOf(el), iconSemanticText(el)].join(' ');
      const directScore = Math.max(tokenScore(targetHint, direct), exactPhraseScore(targetHint, direct), semanticScore(targetHint, direct));
      const contentScore = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      let score = Math.max(directScore, contentScore * 0.65);
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      const classes = classText(el);
      if (['main', 'section', 'article', 'form', 'dialog', 'table', 'tr', 'li'].includes(tag) || ['dialog', 'region', 'main', 'group', 'row'].includes(role)) score += 0.1;
      if (/\b(card|panel|modal|dialog|region|section|row|item|summary|details?)\b/i.test([targetHint, classes, text].join(' '))) score += 0.08;
      const rect = el.getBoundingClientRect();
      const area = rect.width * rect.height;
      if (area > 0 && area < 32) score -= 0.1;
      if (text.length < String(targetHint || '').length) score -= 0.1;
      if (tag === 'body' || tag === 'html') score -= 0.3;
      return score;
    }});
    if (!elements.length) return null;
    const chosen = elements[0];
    return {{
      ok: true,
      action: 'read_text',
      params: {{ selector: selector(chosen.el), max_length: 20000 }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched readable page region by semantic DOM text and labels',
      candidate: candidate(chosen.el)
    }};
  }}
  if (kind === 'read_text') {{
    const readPlan = readTextPlan();
    if (readPlan) return readPlan;
    return {{ ok: false, error: 'act_instruction: no readable text target found' }};
  }}
  function semanticAssertPlan() {{
    if (kind !== 'assert') return null;
    const mode = String(direction || 'visible');
    if (mode === 'value_equals') {{
      if (!targetHint || wantedValue == null) return null;
      const fields = bestReadable(valueFields(), el => {{
        const text = textOf(el);
        let score = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
        score += controlTypeScore(targetHint, el);
        if (/\b(field|input|textbox|text box|value)\b/i.test([targetHint, text].join(' '))) score += 0.1;
        return score;
      }});
      if (!fields.length) return null;
      const chosen = fields[0];
      return {{
        ok: true,
        action: 'assert',
        params: {{ checks: [{{ kind: 'value_equals', selector: selector(chosen.el), value: String(wantedValue) }}] }},
        confidence: Math.min(1, chosen.score),
        reason: 'matched value assertion target by field labels and DOM affordances',
        candidate: candidate(chosen.el)
      }};
    }}
    if (mode === 'checked' || mode === 'unchecked') {{
      const target = targetHint || wantedValue;
      if (!target) return null;
      const controls = bestReadable(interactive.filter(el => readableVisible(el) && isCheckedControl(el)), el => {{
        const text = textOf(el);
        let score = Math.max(tokenScore(target, text), exactPhraseScore(target, text), semanticScore(target, text));
        if (/\b(check|checkbox|radio|switch|toggle|option)\b/i.test([target, text].join(' '))) score += 0.12;
        return score;
      }});
      if (!controls.length) return null;
      const chosen = controls[0];
      return {{
        ok: true,
        action: 'assert',
        params: {{ checks: [{{ kind: 'checked', selector: selector(chosen.el), checked: mode === 'checked' }}] }},
        confidence: Math.min(1, controls[0].score),
        reason: 'matched checked-state assertion target by labels and DOM affordances',
        candidate: candidate(chosen.el)
      }};
    }}
    return null;
  }}
  if (kind === 'assert') {{
    const assertPlan = semanticAssertPlan();
    if (assertPlan) return assertPlan;
    return {{ ok: false, error: 'act_instruction: no matching assertion target found' }};
  }}
  function screenshotPlan() {{
    if (kind !== 'screenshot') return null;
    const mode = String(direction || 'viewport');
    if (!targetHint) {{
      return {{
        ok: true,
        action: 'screenshot',
        params: {{ full_page: mode === 'full_page', format: mode === 'full_page' ? 'jpeg' : 'png' }},
        confidence: 0.9,
        reason: 'planned page screenshot from capture instruction'
      }};
    }}
    const elements = best(all('main, section, article, aside, header, footer, nav, form, dialog, [role=dialog], [role=region], [role=main], [role=group], [data-testid], .card, .panel, .modal, div, button, a'), el => {{
      const text = [textOf(el), directTextOf(el), iconSemanticText(el)].join(' ');
      let score = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      const tag = el.tagName.toLowerCase();
      const role = roleOf(el);
      if (['main', 'section', 'article', 'form', 'dialog'].includes(tag) || ['dialog', 'region', 'main', 'group'].includes(role)) score += 0.08;
      const classes = classText(el);
      if (/\b(card|panel|modal|dialog|region|section)\b/i.test([targetHint, classes].join(' '))) score += 0.08;
      return score;
    }});
    if (!elements.length) return null;
    const chosen = elements[0];
    return {{
      ok: true,
      action: 'screenshot',
      params: {{ selector: selector(chosen.el), format: 'png' }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched screenshot target by semantic DOM text and labels',
      candidate: candidate(chosen.el)
    }};
  }}
  if (kind === 'screenshot') {{
    const shotPlan = screenshotPlan();
    if (shotPlan) return shotPlan;
    return {{ ok: false, error: 'act_instruction: no matching screenshot target found' }};
  }}
  {capability_registry_js}
  const capabilityPlan = chooseCapabilityPlan(capabilityRegistry());
  if (capabilityPlan) return capabilityPlan;
  const derivedPlan = deriveAndActPlan();
  if (derivedPlan) return derivedPlan;

  if (kind === 'press_key') {{
    if (!wantedValue) return {{ ok: false, error: 'act_instruction: key press instruction has no key value' }};
    return {{
      ok: true,
      action: 'press',
      params: {{ key: wantedValue }},
      confidence: 0.95,
      reason: 'matched keyboard key press instruction',
      evidence: {{ key: wantedValue }}
    }};
  }}

  if (kind === 'focus') {{
    const ordinalField = ordinalFieldByInstruction(instruction);
    if (ordinalField) {{
      return {{
        ok: true,
        action: 'focus',
        params: {{ selector: selector(ordinalField.el) }},
        confidence: 0.88,
        reason: 'matched ordinal focusable field by control type and visual order',
        candidate: candidate(ordinalField.el),
        evidence: {{
          ordinalIndex: ordinalField.ordinalIndex,
          resolvedIndex: ordinalField.resolvedIndex,
          candidateCount: ordinalField.candidateCount
        }}
      }};
    }}
    const fields = writableFields();
    const ranked = best(fields, el => {{
      const t = textOf(el);
      let score = targetHint ? tokenScore(targetHint, t) : 0.2;
      score += controlTypeScore(targetHint, el);
      if (targetHint && /\b(textbox|text box|input|field|box)\b/i.test(targetHint) && score === 0) score = 0.25;
      return score;
    }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no focusable field found' }};
    const chosen = ranked[0];
    return {{
      ok: true, action: 'focus',
      params: {{ selector: selector(chosen.el) }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched focusable field from instruction and DOM labels',
      candidate: candidate(chosen.el)
    }};
  }}

  if (kind === 'hover') {{
    const ranked = best(clickableElements().concat(interactive), el => {{
      const text = [textOf(el), iconSemanticText(el)].join(' ');
      let score = targetHint ? Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text)) : 0.15;
      if (/\b(menu|nav|item|trigger|card|row|button|link)\b/i.test(text)) score += 0.05;
      return score;
    }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no hover target found' }};
    const chosen = ranked[0];
    return {{
      ok: true,
      action: 'hover',
      params: {{ selector: selector(chosen.el) }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched hover target from instruction and DOM labels',
      candidate: candidate(chosen.el)
    }};
  }}

  if (kind === 'clear_field') {{
    const ranked = rankedWritableFields({{ defaultScore: 0.25, textLike: true }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no clearable field found' }};
    const chosen = ranked[0];
    return {{
      ok: true,
      action: 'type',
      params: {{ selector: selector(chosen.el), text: '', clear_first: true }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched clearable field from instruction and DOM labels',
      candidate: candidate(chosen.el)
    }};
  }}

  if (kind === 'append_field') {{
    const ranked = rankedWritableFields({{ defaultScore: 0.25, textLike: true }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no appendable field found' }};
    if (!wantedValue) return {{ ok: false, error: 'act_instruction: append instruction has no text value' }};
    const chosen = ranked[0];
    return {{
      ok: true,
      action: 'type',
      params: {{ selector: selector(chosen.el), text: transformedValue(wantedValue), clear_first: false, slowly: true }},
      confidence: Math.min(1, chosen.score),
      reason: 'matched appendable field from instruction and DOM labels',
      candidate: candidate(chosen.el)
    }};
  }}

	  if (kind === 'fill') {{
	    const fields = writableFields();
	    const ranked = rankedWritableFields({{ fields, defaultScore: 0.2, genericFieldScore: 0.2, searchBoost: true }});
    if (!ranked.length) return {{ ok: false, error: 'act_instruction: no fillable field found' }};
    if (!wantedValue) return {{ ok: false, error: 'act_instruction: fill instruction has no text value' }};
    const textValue = transformedValue(wantedValue);
    const repeatedFields = /\bboth\s+(?:text\s+)?(?:fields?|inputs?)\b/i.test(instruction) ||
      /\ball\s+(?:text\s+)?(?:fields?|inputs?)\b/i.test(instruction);
    if (repeatedFields) {{
      const count = /\bboth\b/i.test(instruction) ? Math.min(2, fields.length) : fields.length;
	      const repeatedRanked = rankedWritableFields({{ fields, defaultScore: 0.2, genericFieldScore: 0.2, passwordBoost: true }});
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
	      return isSelectableField(el);
	    }}), el => {{
	      const options = selectableOptionText(el).toLowerCase();
	      const targetText = textOf(el);
	      let score = (wantedValue ? tokenScore(stripFollowUp(wantedValue), options) : 0) +
	        (targetHint ? Math.max(tokenScore(targetHint, targetText), exactPhraseScore(targetHint, targetText), semanticScore(targetHint, targetText)) * 0.7 : 0.2);
	      if (/\b(dropdown|select|option|list|menu|combo)\b/i.test(instruction) && isSelectableField(el)) score += 0.35;
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
    const boxes = interactive.filter(isCheckedControl);
    const items = requestedItems(wantedValue);
    if (boxes.length && (items.length || /^nothing$/i.test(stripFollowUp(wantedValue)))) {{
      const used = new Set();
      const steps = [];
      let followAnchor = boxes[boxes.length - 1] || null;
      for (const item of items) {{
        const rankedBoxes = best(boxes.filter(el => !used.has(selector(el))), el => {{
          const text = checkedControlOptionText(el);
          return Math.max(semanticScore(item, text), tokenScore(item, text), exactPhraseScore(item, text));
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
      const follow = clickStepForHint(followUpClickHint(), followAnchor)
        || (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(followAnchor) : null);
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
    const optionClicks = best(clickableElements(), el => {{
      const text = [textOf(el), iconSemanticText(el)].join(' ');
      return Math.max(tokenScore(wantedValue || targetHint, text), exactPhraseScore(wantedValue || targetHint, text));
    }});
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
    const boxes = interactive.filter(isCheckedControl);
    const items = requestedItems(targetHint);
    if (checked !== false && items.length > 1) {{
      const used = new Set();
      const steps = [];
      let followAnchor = boxes[boxes.length - 1] || null;
      for (const item of items) {{
        const rankedBoxes = best(boxes.filter(el => !used.has(selector(el))), el => {{
          const text = checkedControlOptionText(el);
          return Math.max(semanticScore(item, text), tokenScore(item, text), exactPhraseScore(item, text));
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
      const follow = clickStepForHint(followUpClickHint(), followAnchor)
        || (/\bsubmit|continue|confirm|done|save\b/i.test(instruction) ? completionClickStep(followAnchor) : null);
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
    const ranked = best(boxes, el => {{
      if (!targetHint) return 0.2;
      const text = checkedControlOptionText(el);
      let score = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      const controlKind = checkedControlKind();
      if (controlKind && checkedControlMatchesKind(el, controlKind)) score += 0.2;
      return score;
    }});
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
    const anglePlan = angleBisectorPlan();
    if (anglePlan) return anglePlan;
    const circlePlan = circleDrawPlan();
    if (circlePlan) return circlePlan;
    const linePlan = lineDrawPlan();
    if (linePlan) return linePlan;
    const numericSortPlan = numericSortDragPlan();
    if (numericSortPlan) return numericSortPlan;
    const gridSlotPlan = gridSlotDragPlan();
    if (gridSlotPlan) return gridSlotPlan;
    const reorderPlan = listReorderDragPlan();
    if (reorderPlan) return reorderPlan;
    const shapePartitionPlan = visualShapePartitionDragPlan();
    if (shapePartitionPlan) return shapePartitionPlan;
    const visualOrientation = visualOrientationPlan();
    if (visualOrientation) return visualOrientation;
    const directionalPlan = directionalDragPlan();
    if (directionalPlan) return directionalPlan;
    const geometryPlan = geometryDragPlan();
    if (geometryPlan) return geometryPlan;
    const sourceCandidates = interactive.concat(all('[draggable=true], [draggable="true"], [role=option], [role=listitem], li, div, span, img, svg, canvas'));
    const targetCandidates = interactive.concat(all('[data-drop], [data-dropzone], [droppable], [role=listbox], [role=gridcell], [role=region], div, li, td, canvas, svg, section, article'));
    const rankedSource = best(sourceCandidates, el => {{
      const text = textOf(el);
      let score = Math.max(tokenScore(targetHint, text), exactPhraseScore(targetHint, text), semanticScore(targetHint, text));
      if (el.draggable || el.getAttribute('draggable') === 'true') score += 0.35;
      if (['option', 'listitem', 'button'].includes(roleOf(el))) score += 0.08;
      return score;
    }});
    const rankedTarget = best(targetCandidates, el => {{
      const text = textOf(el);
      let score = Math.max(tokenScore(secondaryHint, text), exactPhraseScore(secondaryHint, text), semanticScore(secondaryHint, text));
      if (el.hasAttribute('data-drop') || el.hasAttribute('data-dropzone') || el.hasAttribute('droppable')) score += 0.25;
      if (['listbox', 'gridcell', 'region'].includes(roleOf(el))) score += 0.08;
      return score;
    }});
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
  const clickables = best(clickableElements(), el => {{
    return scoreClickableTarget(clickHint, el);
  }});
  if (!clickables.length) return {{ ok: false, error: 'act_instruction: no clickable target found' }};
  const chosen = clickables[0];
  const onlyVisibleClickable = clickables.length === 1 && /\b(?:click|press|tap|hit)\b/i.test(instruction);
  const confidence = onlyVisibleClickable
    ? Math.max(0.35, Math.min(1, chosen.score))
    : Math.min(1, chosen.score);
  return {{
    ok: true, action: 'click',
    params: clickParamsFor(chosen.el),
    confidence,
    reason: onlyVisibleClickable
      ? 'matched the only visible clickable target for click instruction'
      : 'matched clickable element by instruction text',
    candidate: candidate(chosen.el),
    evidence: {{ onlyVisibleClickable }}
  }};
}})()"#
    )
}

pub(super) fn text_matcher_js() -> &'static str {
    r#"
  function normalized(text) {
    return String(text || '').toLocaleLowerCase().normalize('NFKC').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
  }
  function tokens(text) {
    const raw = String(text || '').normalize('NFKC').match(/[\p{L}\p{N}]+/gu) || [];
    return raw
      .map(token => {
        const lower = token.toLocaleLowerCase();
        const keepShort = /^\p{N}$/u.test(token) || /^\p{Lu}$/u.test(token) || /[^\p{Script=Latin}]/u.test(token);
        return lower.length > 1 || keepShort ? lower : '';
      })
      .filter(Boolean);
  }
  function tokenScore(hint, text) {
    const wanted = tokens(hint);
    if (!wanted.length) return 0;
    const have = new Set(tokens(text));
    let hits = 0;
    for (const token of wanted) if (have.has(token)) hits++;
    return hits / wanted.length;
  }
  function exactPhraseScore(hint, text) {
    const rawHint = String(hint || '').toLocaleLowerCase().normalize('NFKC').replace(/\s+/g, ' ').trim();
    const rawText = String(text || '').toLocaleLowerCase().normalize('NFKC').replace(/\s+/g, ' ').trim();
    if (rawHint && rawText) {
      const rawVariants = [rawHint, rawHint.replace(/^(?:on|the|a|an)\s+/, '')].filter(Boolean);
      for (const variant of rawVariants) {
        if (rawText === variant) return 1;
        if (rawText.includes(variant)) return 0.95;
      }
    }
    const h = normalized(hint);
    const t = normalized(text);
    if (!h || !t) return 0;
    const compactText = t.replace(/\s+/g, '');
    for (const variant of [h, h.replace(/^(?:on|the|a|an)\s+/, '')]) {
      if (!variant) continue;
      if (t === variant) return 1;
      if (t.includes(variant)) return 0.9;
      if (compactText.includes(variant.replace(/\s+/g, ''))) return 0.85;
    }
    return 0;
  }
"#
}

fn capability_runtime_js() -> &'static str {
    r#"
  function capability(name, priority, build, options = {}) {
    return {
      name,
      priority,
      build,
      applies: options.applies || (() => true),
      strategy: options.strategy || 'candidate-generator',
      category: options.category || 'interaction',
      expectedEffect: options.expectedEffect || null
    };
  }
  function normalizePlan(plan, capability) {
    if (!plan || !plan.action) return null;
    plan.ok = true;
    plan.confidence = planConfidence(plan);
    plan.capability = {
      name: capability.name,
      priority: capability.priority || 0,
      strategy: capability.strategy || 'candidate-generator',
      category: capability.category || 'interaction',
      expectedEffect: capability.expectedEffect || null
    };
    return plan;
  }
  function planStepCount(plan) {
    if (!plan) return 0;
    if (Array.isArray(plan.steps)) return plan.steps.length;
    if (Array.isArray(plan.fields)) return plan.fields.length;
    if (plan.action === 'form_workflow' && plan.params && Array.isArray(plan.params.fields)) return plan.params.fields.length + 1;
    if (plan.action === 'scoped_item_workflow' && plan.params && (plan.params.replyText || plan.params.actionHint)) return 2;
    return 0;
  }
  function hasExplicitMultiActionRequest() {
    if (/(?:;|\n|\r)/.test(instruction)) return true;
    if (/\bthen\b/i.test(instruction)) return true;
    if (/\band\s+(?:click|press|tap|hit|submit|save|continue|confirm|done|apply|send|choose|select|pick|check|tick|turn\s+on|enable|uncheck|untick|deselect|turn\s+off|disable|set|enter|type|fill|input|write|clear|empty|erase|append|add)\b/i.test(instruction)) return true;
    if (/^\s*[^:]{1,80}:\s*.+(?:;|\n|\r|\band\b|\bthen\b)/i.test(instruction)) return true;
    return false;
  }
  function effectivePlanRank(plan) {
    const capability = plan && plan.capability ? plan.capability : {};
    let rank = capability.priority || 0;
    const steps = planStepCount(plan);
    const explicitMulti = hasExplicitMultiActionRequest();
    if (explicitMulti && steps >= 2) rank += 34;
    else if (steps >= 3) rank += 14;
    if (explicitMulti && ['sequence', 'workflow'].includes(capability.category || '')) rank += 8;
    if (explicitMulti && !['sequence', 'workflow'].includes(capability.category || '') && steps <= 1) rank -= 8;
    return rank;
  }
  function chooseCapabilityPlan(capabilities) {
    const candidates = [];
    for (const capability of capabilities) {
      let plan = null;
      try {
        if (capability.applies && !capability.applies()) continue;
        plan = capability.build();
      } catch (error) {
        continue;
      }
      plan = normalizePlan(plan, capability);
      if (plan) candidates.push(plan);
    }
    if (!candidates.length) return null;
    candidates.sort((a, b) => {
      const ap = effectivePlanRank(a);
      const bp = effectivePlanRank(b);
      if (bp !== ap) return bp - ap;
      return planConfidence(b) - planConfidence(a);
    });
    const chosen = candidates[0];
    const alternates = candidates.slice(1, 4);
    chosen.alternates = alternates.map(plan => ({
      action: plan.action,
      confidence: planConfidence(plan),
      reason: plan.reason || null,
      capability: plan.capability || null
    }));
    chosen.alternatePlans = alternates.map(plan => ({
      ...plan,
      alternates: undefined,
      alternatePlans: undefined
    }));
    return chosen;
  }
  function kindIs(...names) {
    return names.includes(kind);
  }
  function instructionHas(pattern) {
    return pattern.test(instruction);
  }
  function hasCompoundFormActionRequest() {
    return instructionHas(/\b(?:and|then|,)\b/i) &&
      instructionHas(/\b(?:check|tick|select|choose|enter|type|fill|input|click|press|tap)\b/i);
  }
  function hasSelectableChoiceRequest() {
    return instructionHas(/\b(?:choose|select|pick|check|tick|enable|turn\s+on)\b/i);
  }
  function hasScopedTargetReference() {
    return instructionHas(/\b(?:in|inside|within|for|on)\b/i) &&
      instructionHas(/\b(?:row|card|item|record|entry|result|section|panel|containing|named|called|user|account|person|profile|contact|customer)\b/i);
  }
  function hasScopedContainerReference() {
    return instructionHas(/\b(?:in|inside|within)\b/i) &&
      instructionHas(/\b(?:row|card|item|record|entry|result|section|panel|region|group|fieldset|form|area)\b/i);
  }
  function hasTextSelectionRequest() {
    return instructionHas(/\b(?:highlight|select)\b/i) &&
      instructionHas(/\b(?:text|paragraph|sentence|word|editor|contents?|everything)\b/i);
  }
  function hasEditorStyleRequest() {
    return instructionHas(/\btext\s+editor|editor\b/i) &&
      instructionHas(/\b(?:bold|italic|italics|underlin(?:e|ed)|style|color|colou?r)\b/i);
  }
  function hasWorkflowIntent() {
    return scopedItemWorkflowPlan() != null ||
      (/\b(find|open|select|choose)\b/i.test(instruction) &&
      /\b(reply|respond|forward|repost|like|share|mark\s+(?:it\s+)?as\s+important|important|star|favorite|favourite|priority|delete|remove|trash|archive)\b/i.test(instruction));
  }
  function hasDiscoveryIntent() {
    return /\b(find|expand|reveal|open)\b/i.test(instruction) ||
      /\bswitch\s+(?:to|into|back|tabs?|windows?|pages?|frames?)\b/i.test(instruction);
  }
"#
}

fn text_transfer_capability_js() -> &'static str {
    r#"
  function transcriptionInstructionRequest() {
    if (!/\b(?:type|enter|input|write|fill)\b/i.test(instruction)) return null;
    if (!/\b(?:text|characters?|letters?|code|token|value)\b/i.test(instruction)) return null;
    if (!/\b(?:below|above|shown|displayed|visible|on\s+(?:the\s+)?page|from\s+(?:the\s+)?page)\b/i.test(instruction)) return null;
    if (/\b(?:solve|calculate|math|equation|last\s+word|\b\d+(?:st|nd|rd|th)?\s+word)\b/i.test(instruction)) return null;
    return { targetHint: copyTargetHint() || targetHint || 'text field' };
  }
  function rawVisibleSourceText(el) {
    if (!el) return '';
    const explicit = el.getAttribute && (el.getAttribute('data-text') || el.getAttribute('data-copy-text') || el.getAttribute('data-value'));
    if (explicit != null) return String(explicit).replace(/\s+/g, ' ').trim();
    if ('value' in el && String(el.value || '').trim()) return String(el.value);
    return String(el.textContent || '').replace(/\s+/g, ' ').trim();
  }
  function promptLikeTextSource(el, value) {
    const meta = [
      el.id || '',
      classText(el),
      el.getAttribute('role') || '',
      el.getAttribute('aria-label') || '',
      el.getAttribute('data-testid') || '',
      value || ''
    ].join(' ');
    return /\b(query|prompt|instruction|question|task|goal|timer|time\s*left|score|reward|status|submit|done|continue|press)\b/i.test(meta);
  }
  function visibleTextTranscriptionPlan() {
    const request = transcriptionInstructionRequest();
    if (!request) return null;
    const fields = best(formFieldCandidates().filter(writableField), el => {
      const text = textOf(el);
      let score = Math.max(tokenScore(request.targetHint, text), exactPhraseScore(request.targetHint, text), semanticScore(request.targetHint, text));
      if (!String(el.value || '').trim()) score += 0.25;
      if (el.tagName.toLowerCase() === 'input' || el.tagName.toLowerCase() === 'textarea') score += 0.2;
      if (/\btext\s*field|text\s*box|textbox|input\b/i.test(instruction)) score += 0.2;
      return score;
    });
    if (!fields.length) return null;
    const target = fields[0].el;
    const targetRect = target.getBoundingClientRect();
    const sourceSelector = [
      '[data-text]',
      '[data-copy-text]',
      '[data-value]',
      'output',
      'pre',
      'code',
      'samp',
      'kbd',
      'p',
      'label',
      'div',
      'span',
      'svg text'
    ].join(',');
    const candidates = all(sourceSelector)
      .filter(el => readableVisible(el) && el !== target && !el.contains(target) && !target.contains(el))
      .filter(el => !el.closest || !el.closest('button, a, [role=button], [role=link], input, textarea, select'))
      .map(el => {
        const value = rawVisibleSourceText(el);
        const rect = el.getBoundingClientRect();
        const meta = [el.id || '', classText(el), el.getAttribute('role') || '', el.getAttribute('aria-label') || '', el.getAttribute('data-testid') || ''].join(' ');
        const hasInteractiveDescendant = el.querySelector && el.querySelector('button, input, textarea, select, [role=button], [role=link]');
        const nestedTextChild = all(sourceSelector, el)
          .some(child => child !== el && readableVisible(child) && rawVisibleSourceText(child) && rawVisibleSourceText(child).length >= value.length * 0.8);
        let score = 0;
        if (!value || value.length > 500) score = -10;
        if (promptLikeTextSource(el, value)) score -= 2.5;
        if (hasInteractiveDescendant) score -= 1.2;
        if (nestedTextChild && !/\b(source|display|sample|text|token|code)\b/i.test(meta)) score -= 0.8;
        if (/\b(source|display|sample|text|token|code)\b/i.test(meta)) score += 1.2;
        if (el.hasAttribute('data-text') || el.hasAttribute('data-copy-text') || el.hasAttribute('data-value')) score += 0.9;
        if (['output', 'pre', 'code', 'samp', 'kbd'].includes(el.tagName.toLowerCase())) score += 0.6;
        if (rect.bottom <= targetRect.top + 8) score += 0.35;
        if (/\bbelow\b/i.test(instruction) && rect.top < targetRect.top) score += 0.35;
        if (value.length >= 2 && value.length <= 80) score += 0.35;
        if (/^[\p{L}\p{N}\s.,:;!?'"()/_-]+$/u.test(value)) score += 0.15;
        return { el, value, score, area: rect.width * rect.height };
      })
      .filter(item => item.score > 0 && item.value && !/\b(?:submit|press\s+submit|type\s+the\s+text|enter\s+the\s+text)\b/i.test(item.value))
      .filter((item, index, arr) => arr.findIndex(other => other.value === item.value) === index)
      .sort((a, b) => b.score - a.score || a.area - b.area || a.value.length - b.value.length);
    if (!candidates.length || candidates[0].score < 0.45) return null;
    const source = candidates[0];
    const primary = {
      ok: true,
      action: 'type',
      params: { selector: selector(target), text: source.value, clear_first: true },
      confidence: Math.min(0.96, 0.58 + Math.min(0.28, source.score / 6) + Math.min(0.1, fields[0].score / 6)),
      reason: 'transcribed compact visible page text into a writable field',
      candidate: candidate(target),
      evidence: { source: candidate(source.el), copiedLength: source.value.length, targetHint: request.targetHint }
    };
    const follow = clickStepForHint(followUpClickHint(), target) ||
      (/\b(?:submit|continue|confirm|done|save|press|hit)\b/i.test(instruction) ? completionClickStep(target) : null);
    if (!follow) return primary;
    return {
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.76, follow.confidence || 0.65),
      reason: 'planned visible text transcription plus completion control',
      evidence: primary.evidence
    };
  }
  function copySourceHint() {
    const patterns = [
      /\b(?:copy|paste|transfer)\s+(?:the\s+)?(?:text|value|code|content|token)?\s*(?:from|in)\s+(?:the\s+)?([^,.]+?)(?:\s+(?:and|then|to|into|in)\b|[,.]|$)/i,
      /\bfrom\s+(?:the\s+)?([^,.]+?)(?:\s+(?:and|then|to|into|in)\b|[,.]|$)/i
    ];
    for (const pattern of patterns) {
      const match = instruction.match(pattern);
      if (match && match[1]) return match[1].replace(/\b(text|value|field|box|area|element)\b/ig, '').trim();
    }
    return null;
  }
  function copyTargetHint() {
    const patterns = [
      /\b(?:into|to|in)\s+(?:the\s+)?([^,.]+?)(?:\s+(?:and|then)\b|[,.]|$)/i,
      /\bpaste\s+(?:it\s+)?(?:into|to|in)\s+(?:the\s+)?([^,.]+?)(?:\s+(?:and|then)\b|[,.]|$)/i
    ];
    for (const pattern of patterns) {
      const match = instruction.match(pattern);
      if (match && match[1]) return match[1].replace(/\b(field|box|input|textbox|text box|area)\b/ig, '').trim();
    }
    return targetHint;
  }
  function readableText(el) {
    if (!el) return '';
    const attr = el.getAttribute && (el.getAttribute('data-copy-text') || el.getAttribute('data-value') || el.getAttribute('aria-label') || el.getAttribute('title'));
    const value = 'value' in el ? el.value : '';
    const text = el.textContent || '';
    return String(attr || value || text || '').replace(/\s+/g, ' ').trim();
  }
  function exactReadableText(el) {
    if (!el) return '';
    const attr = el.getAttribute && (el.getAttribute('data-copy-text') ?? el.getAttribute('data-value'));
    if (attr != null) return String(attr);
    if ('value' in el) return String(el.value);
    if (el.isContentEditable) return String(el.textContent || '');
    return readableText(el);
  }
  function copyTextTransferPlan() {
    if (!/\b(copy|paste|transfer)\b/i.test(instruction)) return null;
    const sourceHint = copySourceHint();
    const destinationHint = copyTargetHint();
    const fields = interactive.filter(writableField);
    if (!fields.length) return null;
    const targetRanked = best(fields, el => {
      const text = textOf(el);
      let score = destinationHint ? Math.max(tokenScore(destinationHint, text), exactPhraseScore(destinationHint, text)) : 0.25;
      if (!readableText(el)) score += 0.15;
      if (/\b(target|destination|answer|result|output|verification|code|paste|input)\b/i.test(text)) score += 0.25;
      if (/\b(text\s*box|textbox|text\s*input|input)\b/i.test(destinationHint || instruction) && el.tagName.toLowerCase() === 'input') score += 0.45;
      if (/\b(text\s*area|textarea)\b/i.test(destinationHint || '') && el.tagName.toLowerCase() === 'textarea') score += 0.45;
      if (/\bcopy\b/i.test(instruction) && el.tagName.toLowerCase() === 'textarea' && readableText(el)) score -= 0.6;
      return score;
    });
    if (!targetRanked.length) return null;
    const target = targetRanked[0].el;
    const sourceCandidates = all('input, textarea, output, pre, code, samp, kbd, [data-copy-text], [data-value], [aria-label], [title], [role=textbox], [contenteditable]:not([contenteditable="false"]), p, label, div, span')
      .filter(el => visible(el) && el !== target && !target.contains(el));
    const sourceOrdinal = ordinalIndexFromText(sourceHint || instruction);
    const sourceWantsTextarea = /\b(text\s*area|textarea)\b/i.test([sourceHint || '', instruction].join(' '));
    if (sourceOrdinal != null || sourceWantsTextarea) {
      const typedSources = sourceCandidates.filter(el => {
        const tag = el.tagName.toLowerCase();
        if (sourceWantsTextarea && tag !== 'textarea') return false;
        return readableText(el) && readableText(el).length <= 500;
      });
      const ordered = typedSources.slice().sort((a, b) => {
        const ar = a.getBoundingClientRect();
        const br = b.getBoundingClientRect();
        return ar.top - br.top || ar.left - br.left;
      });
      const source = sourceOrdinal === -1 ? ordered[ordered.length - 1] : ordered[sourceOrdinal || 0];
      if (source) {
        const textValue = transformedValue(exactReadableText(source));
        const primary = {
          ok: true,
          action: 'type',
          params: { selector: selector(target), text: textValue, clear_first: true },
          confidence: 0.88,
          reason: 'copied visible source text into a writable target field by source type or ordinal',
          candidate: candidate(target),
          evidence: { source: candidate(source), sourceHint, targetHint: destinationHint, copiedLength: textValue.length }
        };
        const follow = clickStepForHint(followUpClickHint(), target) || completionClickStep(target);
        if (!follow) return primary;
        return {
          ok: true,
          action: 'sequence',
          steps: [primary, follow],
          confidence: Math.min(primary.confidence || 0.8, follow.confidence || 0.65),
          reason: 'planned visible text transfer plus completion control'
        };
      }
    }
    const rankedSources = best(sourceCandidates, el => {
      const value = readableText(el);
      if (!value || value.length > 500) return 0;
      if (writableField(el) && !el.readOnly && !el.disabled && !sourceHint) return 0;
      const text = [textOf(el), value].join(' ');
      let score = sourceHint ? Math.max(tokenScore(sourceHint, text), exactPhraseScore(sourceHint, text)) : 0.25;
      if (el.readOnly || el.getAttribute('aria-readonly') === 'true') score += 0.25;
      if (el.hasAttribute('data-copy-text') || el.hasAttribute('data-value')) score += 0.35;
      if (/\b(source|copy|token|code|value|readonly|read only)\b/i.test(text)) score += 0.25;
      if (el.compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING) score += 0.12;
      const rect = el.getBoundingClientRect();
      const area = Math.max(1, rect.width * rect.height);
      if (area > 160000) score -= 0.4;
      return score;
    });
    if (!rankedSources.length || rankedSources[0].score < 0.3) return null;
    const source = rankedSources[0].el;
    const textValue = transformedValue(exactReadableText(source));
    if (!textValue) return null;
    const primary = {
      ok: true,
      action: 'type',
      params: { selector: selector(target), text: textValue, clear_first: true },
      confidence: Math.min(1, 0.65 + Math.min(0.25, rankedSources[0].score / 4) + Math.min(0.15, targetRanked[0].score / 5)),
      reason: 'copied visible source text into a writable target field',
      candidate: candidate(target),
      evidence: { source: candidate(source), sourceHint, targetHint: destinationHint, copiedLength: textValue.length }
    };
    const follow = clickStepForHint(followUpClickHint(), target) || completionClickStep(target);
    if (!follow) return primary;
    return {
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence || 0.75, follow.confidence || 0.65),
      reason: 'planned visible text transfer plus completion control'
    };
  }
  function textBlockCandidates() {
    return visualOrder(all('p, article, section, main, [role=article], [role=document], [data-text], .paragraph, .body, .content, #randomText, div')
      .filter(el => {
        if (!visible(el)) return false;
        const tag = el.tagName.toLowerCase();
        if (['html', 'body', 'script', 'style', 'button', 'input', 'textarea', 'select'].includes(tag)) return false;
        const text = directTextOf(el) || textOf(el);
        if (!text || text.trim().length < 2 || text.length > 5000) return false;
        if (el.closest && el.closest('#query, [data-role=query], .query')) return false;
        if (el.querySelector && el.querySelector('button, input, textarea, select')) return false;
        return true;
      }));
  }
  function textSelectionPlan() {
    const editorStyleRequest = /\btext\s+editor|editor\b/i.test(instruction) && /\b(?:bold|italic|italics|underlin(?:e|ed)|style|color|colou?r)\b/i.test(instruction);
    if (!editorStyleRequest && (!/\b(?:highlight|select)\b/i.test(instruction) || !/\btext|paragraph|sentence|word|editor|contents?|everything\b/i.test(instruction))) return null;
    if (editorStyleRequest) return editorStylePlan();
    const ordinal = ordinalIndexFromText(instruction);
    const paragraphCandidates = visualOrder(all('p').filter(el => visible(el) && textOf(el).trim().length > 0));
    let chosen = null;
    if (ordinal != null && paragraphCandidates.length) {
      chosen = paragraphCandidates[ordinal < 0 ? paragraphCandidates.length - 1 : Math.min(ordinal, paragraphCandidates.length - 1)];
    }
    if (!chosen) {
      const ranked = best(textBlockCandidates(), el => {
        const tag = el.tagName.toLowerCase();
        const meta = [tag, el.id || '', classText(el), el.getAttribute('role') || ''].join(' ');
        let score = 0.2;
        if (tag === 'p') score += 0.5;
        if (/\b(?:paragraph|body|content|article|text)\b/i.test(meta)) score += 0.35;
        if (el.id === 'randomText') score += 0.45;
        if (/\bparagraph\b/i.test(instruction) && (tag === 'p' || /\bparagraph\b/i.test(meta))) score += 0.25;
        const rect = el.getBoundingClientRect();
        const area = rect.width * rect.height;
        if (area > 180000) score -= 0.25;
        if ((textOf(el) || '').length < 8) score -= 0.2;
        return score;
      });
      if (ranked.length && ranked[0].score >= 0.35) chosen = ranked[0].el;
    }
    if (!chosen) return null;
    const primary = {
      ok: true,
      action: 'select_text',
      params: { selector: selector(chosen) },
      confidence: 0.86,
      reason: 'selected requested visible text block',
      candidate: candidate(chosen),
      evidence: { ordinalIndex: ordinal, selectedTextLength: textOf(chosen).length }
    };
    const follow = clickStepForHint(followUpClickHint(), chosen) || completionClickStep(chosen);
    if (!follow) return primary;
    return {
      ok: true,
      action: 'sequence',
      steps: [primary, follow],
      confidence: Math.min(primary.confidence, follow.confidence || 0.65),
      reason: 'planned text selection plus completion control'
    };
  }
  function requestedEditorStyle() {
    if (/\bbold\b/i.test(instruction)) return { key: 'bold', query: 'bold' };
    if (/\b(?:italic|italics)\b/i.test(instruction)) return { key: 'italic', query: 'italic' };
    if (/\bunderlin(?:e|ed)\b/i.test(instruction)) return { key: 'underline', query: 'underline' };
    return null;
  }
  function editorStyleButton(style) {
    const candidates = all('button, [role=button], [onclick], [tabindex], .ql-bold, .ql-italic, .ql-underline')
      .filter(el => visible(el));
    const ranked = best(candidates, el => {
      const text = [textOf(el), directTextOf(el), iconSemanticText(el), classText(el), el.getAttribute('aria-label') || '', el.getAttribute('title') || ''].join(' ');
      let score = Math.max(tokenScore(style.query, text), exactPhraseScore(style.query, text), semanticScore(style.query, text));
      if (new RegExp('(?:^|\\\\s|-)ql-' + style.key + '(?:\\\\s|$)', 'i').test(classText(el))) score += 1.0;
      if (el.tagName.toLowerCase() === 'button') score += 0.12;
      return score;
    });
    return ranked.length && ranked[0].score >= 0.35 ? ranked[0] : null;
  }
  function editorStylePlan() {
    const style = requestedEditorStyle();
    if (!style) return null;
    const editors = all('.ql-editor, [contenteditable]:not([contenteditable="false"]), [role=textbox], textarea')
      .filter(el => visible(el) && (textOf(el).trim() || String(el.value || '').trim()));
    if (!editors.length) return null;
    const rankedEditors = best(editors, el => {
      const text = [textOf(el), el.getAttribute('aria-label') || '', classText(el), el.id || ''].join(' ');
      let score = 0.4;
      if (/\b(?:editor|body|message|content)\b/i.test(text)) score += 0.3;
      if (/\bql-editor\b/i.test(classText(el))) score += 0.55;
      return score;
    });
    const editor = rankedEditors[0].el;
    const steps = [
      {
        action: 'format_text',
        params: { selector: selector(editor), style: style.key },
        confidence: Math.min(1, rankedEditors[0].score),
        reason: 'formatted editor contents using available editor APIs or contenteditable commands',
        candidate: candidate(editor)
      }
    ];
    const follow = clickStepForHint(followUpClickHint(), editor) || completionClickStep(editor);
    if (follow) steps.push(follow);
    return {
      ok: true,
      action: 'sequence',
      steps,
      confidence: Math.min(1, steps.reduce((sum, step) => sum + (step.confidence || 0.5), 0) / steps.length),
      reason: 'planned editor text formatting plus completion control',
      evidence: { style: style.key, editor: candidate(editor) }
    };
  }
"#
}

fn capability_registry_js() -> &'static str {
    r#"
  function capabilityRegistry() {
    return [
      capability('derived-value', 118, deriveAndActPlan, {
        category: 'reasoning',
        expectedEffect: 'derived_value_or_action_completed'
      }),
      capability('feedback-loop-value', 119, feedbackLoopValuePlan, {
        category: 'reasoning',
        expectedEffect: 'generated_value_submitted'
      }),
      capability('conditional-value-action', 121, conditionalValueActionPlan, {
        category: 'reasoning',
        expectedEffect: 'conditional_action_completed'
      }),
      capability('command-surface-action', 122, commandSurfaceActionPlan, {
        category: 'workflow',
        expectedEffect: 'command_surface_completed'
      }),
      capability('numeric-constraint-generation', 118, generateConstrainedValuePlan, {
        category: 'reasoning',
        expectedEffect: 'generated_value_submitted'
      }),
      capability('scoped-item-workflow', 118, scopedItemWorkflowPlan, {
        applies: hasWorkflowIntent,
        category: 'workflow',
        expectedEffect: 'item_action_completed'
      }),
      capability('form-result-workflow', 117, formWorkflowPlan, {
        applies: () => formWorkflowRequest() != null,
        category: 'workflow',
        expectedEffect: 'form_workflow_completed'
      }),
      capability('compound-form-steps', 122, compoundFormStepsPlan, {
        applies: hasCompoundFormActionRequest,
        category: 'sequence',
        expectedEffect: 'compound_form_sequence_completed'
      }),
      capability('item-quantity-selection', 123, itemQuantitySelectionPlan, {
        category: 'selection',
        expectedEffect: 'item_quantities_selected'
      }),
      capability('binary-row-classification', 124, binaryRowClassificationPlan, {
        category: 'classification',
        expectedEffect: 'row_classification_completed'
      }),
      capability('record-property-lookup', 123, recordPropertyClickPlan, {
        category: 'navigation',
        expectedEffect: 'record_property_clicked'
      }),
      capability('text-selection', 116, textSelectionPlan, {
        applies: () => hasTextSelectionRequest() || hasEditorStyleRequest(),
        category: 'text',
        expectedEffect: 'text_selected'
      }),
      capability('visual-geometry-selection', 115, visualGeometrySelectionPlan, {
        applies: () => kindIs('click', 'count') && (
          /\b(?:describes?|identify|name|kind|type)\b/i.test(instruction) && /\b(?:figure|shape|object|symbol|item)\b/i.test(instruction) ||
          /\bsides?\b/i.test(instruction) && /\b(?:button|press|click|correctly|denotes?)\b/i.test(instruction)
        ),
        category: 'reasoning',
        expectedEffect: 'visual_geometry_value_selected'
      }),
      capability('visual-geometry-click', 116, visualGeometryClickPlan, {
        applies: () => kindIs('click') && /\b(?:midpoint|mid-point|middle\s+point|halfway|half-way|center\s+point|centre\s+point|between)\b/i.test(instruction),
        category: 'geometry',
        expectedEffect: 'visual_object_clicked'
      }),
      capability('perpendicular-point-construction', 116, perpendicularPointConstructionPlan, {
        applies: () => /\b(?:right\s*angle|perpendicular|orthogonal|90\s*(?:degree|deg)?)\b/i.test(instruction),
        category: 'geometry',
        expectedEffect: 'visual_object_clicked'
      }),
      capability('visual-feedback-search', 121, visualFeedbackSearchPlan, {
        applies: () => kindIs('click') &&
          /\b(?:find|locate|search|identify|discover|click|tap|press)\b/i.test(instruction) &&
          /\b(?:area|region|zone|spot|point|location|place|surface|target)\b/i.test(instruction) &&
          /\b(?:ice\s+cold|hot|warm|cold|success|correct|good|yes)\b/i.test(instruction),
        category: 'visual',
        expectedEffect: 'pointer_feedback_target_clicked'
      }),
      capability('hierarchical-tree-search', 122, treeSearchClickPlan, {
        applies: () => kindIs('click') &&
          /\b(?:tree|hierarchy|outline|folder|file|directory|nested|expand|collapse)\b/i.test(instruction) &&
          /"[^"]+"|'[^']+'|\bnamed\s+[A-Za-z0-9_.-]+|\bcalled\s+[A-Za-z0-9_.-]+|\blabel(?:ed|led)\s+[A-Za-z0-9_.-]+/i.test(instruction),
        category: 'navigation',
        expectedEffect: 'tree_target_clicked'
      }),
      capability('scrollable-text-extract', 120, scrollTextExtractPlan, {
        applies: () => /\b(?:first|last)\s+word\b/i.test(instruction) && /\b(?:text\s*area|textarea|scroll|text|field)\b/i.test(instruction),
        category: 'read',
        expectedEffect: 'scroll_text_value_used'
      }),
      capability('element-resize', 119, resizeElementPlan, {
        applies: () => /\bresize\b/i.test(instruction) && /\b(?:textarea|text\s*area|field|box|panel|element|editor)\b/i.test(instruction),
        category: 'geometry',
        expectedEffect: 'element_resized'
      }),
      capability('menu-path-selection', 112, menuPathPlan, {
        applies: () => kindIs('select_option') && !/\b(?:tree|treeitem|listbox|multi-?select|scroll\s+list)\b/i.test([instruction, targetHint || ''].join(' ')),
        category: 'selection',
        expectedEffect: 'menu_path_selected'
      }),
      capability('ordered-click-sequence', 110, orderedClickSequencePlan, {
        applies: () => kindIs('click'),
        category: 'sequence',
        expectedEffect: 'ordered_targets_clicked'
      }),
      capability('scoped-child-click', 109, scopedChildClickPlan, {
        applies: () => kindIs('click') && hasScopedTargetReference(),
        category: 'selection',
        expectedEffect: 'scoped_target_clicked'
      }),
      capability('ordinal-click-target', 108, ordinalClickPlan, {
        applies: () => kindIs('click') && /\b(?:last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|sixth|6th|seventh|7th|eighth|8th|ninth|9th|tenth|10th|\d+(?:st|nd|rd|th)?)\s+(?:button|link|row|card|item|option|tab|result|entry|tile|swatch)s?\b/i.test(instruction),
        category: 'selection',
        expectedEffect: 'ordinal_target_clicked'
      }),
      capability('ordered-value-clicks', 105, orderedValueClickPlan, {
        applies: () => kindIs('click') && intent && intent.wantsOrderedValues === true,
        category: 'sequence',
        expectedEffect: 'ordered_numeric_targets_clicked'
      }),
      capability('coordinate-grid-click', 103, coordinateGridPlan, {
        applies: () => kindIs('click') && /\b(?:grid\s+)?coordinate\b/i.test(instruction),
        category: 'geometry',
        expectedEffect: 'coordinate_target_clicked'
      }),
      capability('checkbox-grid-pattern', 100, checkboxGridPatternPlan, {
        applies: () => kindIs('render_pattern') || /\bcheckbox(?:es)?|check\s*boxes|grid|pattern\b/i.test(instruction),
        category: 'pattern',
        expectedEffect: 'checkbox_pattern_rendered'
      }),
      capability('scoped-checked-control', 100, scopedCheckedControlPlan, {
        applies: () => kindIs('set_checked', 'select_option') && hasScopedContainerReference(),
        category: 'form_control',
        expectedEffect: 'scoped_checked_control_set'
      }),
      capability('grouped-choice-control', 113, groupedChoiceControlPlan, {
        applies: () => kindIs('set_checked', 'select_option') && hasSelectableChoiceRequest(),
        category: 'form_control',
        expectedEffect: 'grouped_choice_control_set'
      }),
      capability('ordinal-checked-control', 99, ordinalCheckedControlPlan, {
        applies: () => kindIs('set_checked', 'select_option') && /\b(?:last|first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|sixth|6th|seventh|7th|eighth|8th|ninth|9th|tenth|10th|\d+(?:st|nd|rd|th)?)\s+(?:checkbox|check\s*box|radio|radio\s+button|switch|toggle)s?\b/i.test(instruction),
        category: 'form_control',
        expectedEffect: 'ordinal_checked_control_set'
      }),
      capability('color-picker-input', 118, colorPickerInputPlan, {
        applies: () => kindIs('fill', 'select_option', 'click') && /\b(?:colou?r\s*picker|colou?r\s*field|colou?r\s*input|picker)\b/i.test(instruction),
        category: 'form_control',
        expectedEffect: 'color_picker_value_set'
      }),
      capability('visual-color-selection', 117, visualColorSelectionPlan, {
        applies: () => kindIs('click', 'select_option') && /\b(red|scarlet|orange|yellow|olive|lime|green|cyan|aqua|teal|blue|navy|indigo|purple|violet|magenta|pink|brown|gold|black|white|gray|grey|silver)\b/i.test(instruction),
        category: 'visual',
        expectedEffect: 'visual_color_targets_selected'
      }),
      capability('visual-object-click', 116, visualObjectClickPlan, {
        applies: () => kindIs('click') && /\b(?:cent(?:er|re)|small|smaller|smallest|tiny|little|large|larger|largest|big|bigger|biggest|circle|dot|round|square|rectangle|rect|box|tile|cell|triangle|polygon|path|line|shape|object|item|symbol|letter|number|digit)\b/i.test(instruction),
        category: 'visual',
        expectedEffect: 'visual_object_clicked'
      }),
      capability('date-picker-selection', 96, datePickerPlan, {
        applies: () => kindIs('fill', 'select_option', 'click') && requestedDate() != null,
        category: 'form_control',
        expectedEffect: 'calendar_date_selected'
      }),
      capability('multi-slider-values', 95, multiSliderPlan, {
        applies: () => kindIs('fill') && /\bsliders?\b/i.test(instruction),
        category: 'form_control',
        expectedEffect: 'slider_values_set'
      }),
      capability('scoped-multi-action', 95, scopedMultiActionPlan, {
        applies: () => scopedMultiActionIntent() != null,
        category: 'sequence',
        expectedEffect: 'scoped_multi_action_completed'
      }),
      capability('scoped-field-fill', 94, scopedFieldFillPlan, {
        applies: () => kindIs('fill', 'select_option') && hasScopedContainerReference(),
        category: 'form_control',
        expectedEffect: 'scoped_field_filled'
      }),
      capability('scoped-field-edit', 93, scopedFieldEditPlan, {
        applies: () => kindIs('clear_field', 'append_field') && hasScopedContainerReference(),
        category: 'form_control',
        expectedEffect: 'scoped_field_edited'
      }),
      capability('slider-checkbox-sequence', 92, sliderCheckboxPlan, {
        applies: () => kindIs('fill') && /\bslider\b/i.test(instruction) && /\bcheckbox\b/i.test(instruction),
        category: 'sequence',
        expectedEffect: 'slider_and_checkbox_values_set'
      }),
      capability('multi-field-form-fill', 91, multiFieldFormPlan, {
        applies: () => kindIs('fill'),
        category: 'form_control',
        expectedEffect: 'form_fields_filled'
      }),
      capability('file-upload', 91, uploadFilePlan, {
        applies: () => kindIs('upload_file'),
        category: 'form_control',
        expectedEffect: 'file_uploaded'
      }),
      capability('single-slider-value', 90, () => {
        const plan = sliderPlan();
        if (!plan) return null;
        return withFollowUp(plan, all(plan.params.selector)[0] || null);
      }, {
        applies: () => kindIs('fill') && !/\bcheckbox\b/i.test(instruction),
        category: 'form_control',
        expectedEffect: 'slider_value_set'
      }),
      capability('autocomplete-selection', 89, autocompletePlan, {
        applies: () => kindIs('fill', 'select_option'),
        category: 'form_control',
        expectedEffect: 'autocomplete_option_selected'
      }),
      capability('spinbutton-value', 88, spinbuttonPlan, {
        applies: () => kindIs('fill'),
        category: 'form_control',
        expectedEffect: 'numeric_value_set'
      }),
      capability('copy-text-transfer', 121, copyTextTransferPlan, {
        applies: () => /\b(copy|paste|transfer)\b/i.test(instruction),
        category: 'text_transfer',
        expectedEffect: 'text_copied_to_target'
      }),
      capability('visible-text-transcription', 120, visibleTextTranscriptionPlan, {
        category: 'text_transfer',
        expectedEffect: 'visible_text_entered'
      }),
      capability('table-to-form-fill', 122, tableToFormFillPlan, {
        applies: () => /\b(?:enter|type|fill|input|write|use)\b/i.test(instruction) &&
          /\b(?:corresponds?|matching|matches?|each\s+label|labels?|table|form)\b/i.test(instruction),
        category: 'read',
        expectedEffect: 'table_values_filled'
      }),
      capability('table-cell-lookup', 119, tableLookupPlan, {
        applies: () => tableLookupRequest() != null,
        category: 'read',
        expectedEffect: 'table_cell_value_used'
      }),
      capability('list-option-selection', 87, listSelectionPlan, {
        applies: () => kindIs('select_option'),
        category: 'selection',
        expectedEffect: 'list_options_selected'
      }),
      capability('tab-selection', 116, tabSelectionPlan, {
        applies: () => kindIs('click', 'select_option') && /\b(?:tabs?|tabbed|tab\s*panel|tabpanel)\b/i.test(instruction),
        category: 'navigation',
        expectedEffect: 'target_revealed_or_clicked'
      }),
      capability('discovery-click', 115, discoverClickPlan, {
        applies: hasDiscoveryIntent,
        category: 'discovery',
        expectedEffect: 'target_revealed_or_clicked'
      }),
      capability('extreme-click', 120, extremeClickPlan, {
        applies: () => (kindIs('click', 'select_option') || /\b(?:click|tap|press|pick|choose|select)\b/i.test(instruction)) &&
          !/\b(?:sort|order|arrange)\b/i.test(instruction) &&
          /\b(shortest|longest|lowest|highest|smallest|largest|greatest|max(?:imum)?|cheapest|most expensive)\b/i.test(instruction),
        category: 'selection',
        expectedEffect: 'ranked_target_clicked'
      }),
      capability('visible-count-value', 78, countPlan, {
        applies: () => kindIs('count') || (
          /\b(?:total\s+number\s+of|number\s+of|how\s+many|count)\b/i.test(instruction) &&
          /\b(?:type|enter|input|write|fill|answer|textbox|text\s*box|field|press|submit)\b/i.test(instruction)
        ),
        category: 'read',
        expectedEffect: 'visible_count_value_used'
      }),
      capability('scroll-fill-press', 75, scrollFillPressPlan, {
        applies: () => kindIs('fill', 'scroll') && /\b(?:enter|type|fill|input|write)\b/i.test(instruction) && /\bscroll\b/i.test(instruction),
        category: 'sequence',
        expectedEffect: 'scroll_fill_and_submit_completed'
      })
    ];
  }
"#
}
