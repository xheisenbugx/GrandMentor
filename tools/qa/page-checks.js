// In-page checks for tools/qa/sweep.mjs. This file is evaluated inside the app page (not imported
// by Node); it must stay a single self-contained function expression.
//
// Input (window.__qaData, injected by the sweep): { keys: [...i18n keys], english: [...phrases] }.
// Returns { scrollWidth, innerWidth, hscroll, overflow[], rawKeys[], english[], layerOverlaps[], buttonOverlaps[] }.
(() => {
  const data = window.__qaData || { keys: [], english: [] };
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const MAX = 8;

  const describe = (el) => {
    if (!el || el.nodeType !== 1) return String(el);
    let s = el.tagName.toLowerCase();
    if (el.id) s += '#' + el.id;
    const cls = typeof el.className === 'string' ? el.className.trim().split(/\s+/).filter(Boolean).slice(0, 3) : [];
    if (cls.length) s += '.' + cls.join('.');
    const text = (el.getAttribute('aria-label') || el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 40);
    return text ? `${s} "${text}"` : s;
  };
  const visible = (el) => {
    if (typeof el.checkVisibility === 'function' && !el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1;
  };
  const styleCache = new Map();
  const cs = (el) => { let s = styleCache.get(el); if (!s) { s = getComputedStyle(el); styleCache.set(el, s); } return s; };
  const clips = (s) => s.overflowX !== 'visible' || s.overflowY !== 'visible';

  /**
   * Bounding rect intersected with every clipping ancestor (overflow != visible), in viewport coords.
   * With `ignoreGuards`, wide ancestors reaching the viewport's right edge do not clip: a page-level
   * `overflow-x: hidden` hides a too-wide element instead of fixing it, so it must still be reported.
   */
  const rectCache = new Map();
  function visibleRect(el, ignoreGuards = false) {
    const ck = ignoreGuards ? 'g' : 'n';
    let entry = rectCache.get(el);
    if (entry && ck in entry) return entry[ck];
    const r = el.getBoundingClientRect();
    let box = { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    let fixed = cs(el).position === 'fixed';
    for (let a = el.parentElement; a && a !== document.body && a !== document.documentElement && !fixed; a = a.parentElement) {
      const s = cs(a);
      if (clips(s)) {
        const ar = a.getBoundingClientRect();
        const guard = ignoreGuards && ar.right >= vw - 1 && ar.width >= vw * 0.6 && s.overflowX !== 'auto' && s.overflowX !== 'scroll';
        if (!guard) box = { left: Math.max(box.left, ar.left), top: Math.max(box.top, ar.top), right: Math.min(box.right, ar.right), bottom: Math.min(box.bottom, ar.bottom) };
      }
      if (s.position === 'fixed') fixed = true;
    }
    const out = box.right - box.left > 1 && box.bottom - box.top > 1 ? box : null;
    entry = entry || {};
    entry[ck] = out;
    rectCache.set(el, entry);
    return out;
  }
  /** Scroll layer: 'page', 'fixed' (viewport coords) or the nearest sticky ancestor. */
  function layerOf(el) {
    for (let a = el; a && a !== document.body; a = a.parentElement) {
      const p = cs(a).position;
      if (p === 'fixed') return 'fixed';
      if (p === 'sticky') return a;
    }
    return 'page';
  }
  const inter = (a, b) => {
    const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
    const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    return w > 2 && h > 2 ? { w: Math.round(w), h: Math.round(h) } : null;
  };
  const related = (a, b) => a === b || a.contains(b) || b.contains(a);

  const all = [...document.body.querySelectorAll('*')].filter((el) => !['SCRIPT', 'STYLE', 'TEMPLATE', 'NOSCRIPT'].includes(el.tagName));
  const shown = all.filter(visible);

  // 1. Horizontal page scroll.
  const se = document.scrollingElement || document.documentElement;
  const scrollWidth = Math.max(se.scrollWidth, document.body.scrollWidth);
  const hscroll = scrollWidth > vw + 1;

  // 2. Visible elements sticking out of the viewport horizontally (topmost offenders only).
  const overflowSet = new Set();
  for (const el of shown) {
    const r = visibleRect(el, true);
    if (!r) continue;
    const out = (r.right > vw + 1 && r.left < vw - 1) || (r.left < -1 && r.right > 1);
    if (out) overflowSet.add(el);
  }
  const overflow = [...overflowSet].filter((el) => !overflowSet.has(el.parentElement)).slice(0, MAX).map((el) => {
    const r = visibleRect(el, true);
    return { el: describe(el), left: Math.round(r.left), right: Math.round(r.right) };
  });

  // 3. Raw i18n keys in visible text or accessible attributes.
  const keys = new Set(data.keys);
  const keyRe = /\b[a-z][a-zA-Z0-9]*(?:\.[a-zA-Z0-9_-]+)+\b/g;
  const rawKeys = [];
  const seenKeys = new Set();
  const addKey = (k, where) => { if (keys.has(k) && !seenKeys.has(k)) { seenKeys.add(k); rawKeys.push({ key: k, where }); } };
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const p = n.parentElement;
    if (!p || !n.nodeValue.includes('.') || !visible(p)) continue;
    for (const m of n.nodeValue.matchAll(keyRe)) addKey(m[0], describe(p));
  }
  for (const el of document.querySelectorAll('[placeholder],[aria-label],[title],[alt]')) {
    for (const attr of ['placeholder', 'aria-label', 'title', 'alt']) {
      const v = el.getAttribute(attr);
      if (v && v.includes('.')) for (const m of v.matchAll(keyRe)) addKey(m[0], `${describe(el)} [${attr}]`);
    }
  }
  if (document.title) for (const m of document.title.matchAll(keyRe)) addKey(m[0], 'document.title');

  // 4. English UI phrases visible on a non-English page.
  const english = [];
  if (data.english.length) {
    const text = document.body.innerText.replace(/\s+/g, ' ');
    for (const phrase of data.english) {
      if (text.includes(phrase)) { english.push(phrase); if (english.length >= MAX) break; }
    }
  }

  // 5a. Floating layers (speech bubbles, toasts, popovers, tooltips) over the board.
  const boards = shown.filter((el) => el.classList.contains('gm-board'));
  const layerSel = '.toast, .bubble, [role="tooltip"], [class*="popover"], [class*="tooltip"]';
  const layers = shown.filter((el) => el.matches(layerSel));
  const layerOverlaps = [];
  for (const L of layers) {
    const lr = visibleRect(L);
    if (!lr) continue;
    for (const B of boards) {
      if (related(L, B)) continue;
      const br = visibleRect(B);
      const x = br && inter(lr, br);
      if (x) layerOverlaps.push({ layer: describe(L), board: describe(B), overlap: `${x.w}x${x.h}px` });
    }
  }

  // 5b. Buttons overlapping each other (same scroll layer, not nested).
  const btns = shown.filter((el) => el.matches('button, a.btn, .btn, [role="button"]'));
  const buttonOverlaps = [];
  for (let i = 0; i < btns.length && buttonOverlaps.length < MAX; i++) {
    const a = btns[i], ar = visibleRect(a);
    if (!ar) continue;
    for (let j = i + 1; j < btns.length; j++) {
      const b = btns[j];
      if (related(a, b) || layerOf(a) !== layerOf(b)) continue;
      const br = visibleRect(b);
      const x = br && inter(ar, br);
      if (x) { buttonOverlaps.push({ a: describe(a), b: describe(b), overlap: `${x.w}x${x.h}px` }); if (buttonOverlaps.length >= MAX) break; }
    }
  }

  return { innerWidth: vw, innerHeight: vh, scrollWidth, hscroll, overflow, rawKeys: rawKeys.slice(0, MAX), english, layerOverlaps: layerOverlaps.slice(0, MAX), buttonOverlaps };
})()
