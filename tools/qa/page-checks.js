// In-page checks for tools/qa/sweep.mjs. This file is evaluated inside the app page (not imported
// by Node); it must stay a single self-contained function expression.
//
// Input (window.__qaData, injected by the sweep): { keys: [...i18n keys], english: [...phrases] }.
// Returns { scrollWidth, innerWidth, hscroll, overflow[], rawKeys[], junk[], english[], layerOverlaps[], buttonOverlaps[],
//           a11y: { names[], alt[], hiddenFocus[], dupIds[], contrast[], touch[] } }.
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

  // 3b. Leaked JS values: a missing value rendered as text, e.g. `el.append(x ? node : null)` prints "null".
  const junkRe = /(?:^|[^\w.-])(null|undefined|NaN|\[object Object\])(?![\w-])/;
  const junk = [];
  const junkWalker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = junkWalker.nextNode(); n && junk.length < MAX; n = junkWalker.nextNode()) {
    const p = n.parentElement;
    if (!p || p.closest('pre, code, textarea, script, style') || !visible(p)) continue;
    const m = n.nodeValue.match(junkRe);
    if (m) junk.push({ text: m[1], where: describe(p) });
  }
  for (const el of document.querySelectorAll('[placeholder],[aria-label],[title],[alt]')) {
    if (junk.length >= MAX) break;
    for (const attr of ['placeholder', 'aria-label', 'title', 'alt']) {
      const m = (el.getAttribute(attr) || '').match(junkRe);
      if (m) { junk.push({ text: m[1], where: `${describe(el)} [${attr}]` }); break; }
    }
  }

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

  // 6. Accessibility (approximate, no axe-core): names, alt text, hidden focusables, duplicate ids,
  //    text contrast and touch-target size. Each list is capped at MAX entries.
  const a11y = { names: [], alt: [], hiddenFocus: [], dupIds: [], contrast: [], touch: [] };
  const textOf = (el) => (el ? (el.textContent || '').replace(/\s+/g, ' ').trim() : '');
  const accName = (el) => {
    const label = el.getAttribute('aria-label');
    if (label && label.trim()) return label.trim();
    const lb = el.getAttribute('aria-labelledby');
    if (lb) { const tx = lb.split(/\s+/).map((id) => textOf(document.getElementById(id))).join(' ').trim(); if (tx) return tx; }
    if (el.matches('input, select, textarea, meter, progress')) {
      if (el.id) { const l = document.querySelector(`label[for="${CSS.escape(el.id)}"]`); if (l && textOf(l)) return textOf(l); }
      const wrap = el.closest('label'); if (wrap && textOf(wrap)) return textOf(wrap);
      if (el.matches('input[type="submit"], input[type="button"], input[type="reset"]') && el.value) return el.value;
    } else {
      // Visible text plus alt text of images / labelled SVGs inside.
      const tx = textOf(el);
      if (tx) return tx;
      const img = el.querySelector('img[alt]:not([alt=""]), [role="img"][aria-label]');
      if (img) return img.getAttribute('alt') || img.getAttribute('aria-label');
    }
    const title = el.getAttribute('title');
    return title && title.trim() ? title.trim() : '';
  };
  const hiddenFromAT = (el) => !!el.closest('[aria-hidden="true"], [inert]');
  const interactiveSel = 'button, a[href], input:not([type="hidden"]), select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="radio"], [role="switch"], [role="tab"], [role="slider"], [role="menuitem"], [role="option"], [role="gridcell"][tabindex="0"]';
  const interactive = [...document.querySelectorAll(interactiveSel)];
  for (const el of interactive) {
    if (a11y.names.length >= MAX) break;
    if (hiddenFromAT(el)) continue;
    // Hidden-but-labelled controls (custom switches use a 1px input) still need a name.
    const isSwitchInput = el.matches('.switch input');
    if (!isSwitchInput && !visible(el)) continue;
    if (!accName(el)) a11y.names.push(describe(el));
  }
  for (const img of document.querySelectorAll('img')) {
    if (a11y.alt.length >= MAX) break;
    if (!img.hasAttribute('alt') && !hiddenFromAT(img) && visible(img)) a11y.alt.push(describe(img) + ` src=${(img.getAttribute('src') || '').slice(0, 60)}`);
  }
  for (const el of document.querySelectorAll('svg[role="img"], [role="img"]:not(svg)')) {
    if (a11y.alt.length >= MAX) break;
    if (!hiddenFromAT(el) && visible(el) && !accName(el)) a11y.alt.push(describe(el) + ' (role=img without a label)');
  }
  const focusSel = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
  for (const el of document.querySelectorAll(focusSel)) {
    if (a11y.hiddenFocus.length >= MAX) break;
    if (el.closest('[inert]')) continue;
    if (el.closest('[aria-hidden="true"]') && el.tabIndex >= 0) {
      const s2 = cs(el);
      if (s2.display !== 'none' && s2.visibility !== 'hidden') a11y.hiddenFocus.push(describe(el) + ' (inside aria-hidden)');
    }
  }
  const ids = new Map();
  for (const el of document.querySelectorAll('[id]')) { const id = el.id; if (id) ids.set(id, (ids.get(id) || 0) + 1); }
  for (const [id, n] of ids) { if (n > 1 && a11y.dupIds.length < MAX) a11y.dupIds.push(`#${id} ×${n}`); }

  // Contrast: text colour vs. the first opaque background up the tree (semi-transparent layers are
  // blended; background images/gradients make the result unknown and are skipped).
  const parseColor = (c) => {
    const m = String(c).match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const blend = (top, bottom) => ({ r: top.r * top.a + bottom.r * (1 - top.a), g: top.g * top.a + bottom.g * (1 - top.a), b: top.b * top.a + bottom.b * (1 - top.a), a: 1 });
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const bgOf = (el) => {
    const layers = [];
    for (let a = el; a; a = a.parentElement) {
      const s3 = cs(a);
      if (s3.backgroundImage && s3.backgroundImage !== 'none') return null;
      if (parseFloat(s3.opacity) < 1) return null; // faded (usually disabled) content: skip
      if (s3.filter && s3.filter !== 'none') return null;
      const c = parseColor(s3.backgroundColor);
      if (c && c.a > 0) { layers.push(c); if (c.a >= 0.99) break; }
    }
    let base = { r: 255, g: 255, b: 255, a: 1 };
    const root = parseColor(cs(document.body).backgroundColor);
    if (root && root.a >= 0.99) base = root;
    for (let i = layers.length - 1; i >= 0; i--) base = layers[i].a >= 0.99 ? layers[i] : blend(layers[i], base);
    return base;
  };
  const seenContrast = new Set();
  const tw = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = tw.nextNode(); n && a11y.contrast.length < MAX; n = tw.nextNode()) {
    const el = n.parentElement;
    if (!el || !n.nodeValue.trim() || seenContrast.has(el)) continue;
    seenContrast.add(el);
    // Board and eval bar: text sits on sibling layers (squares, the fill), not on an ancestor background.
    if (el.closest('.gm-board, .gm-evalbar, [aria-hidden="true"], .sr-only, noscript, [disabled], .btn.loading') || !visible(el)) continue;
    if (el.closest('button:disabled, [aria-disabled="true"], input:disabled')) continue;
    const s4 = cs(el);
    const fg = parseColor(s4.color);
    if (!fg || fg.a < 0.5) continue;
    if (s4.webkitTextFillColor && /rgba\(0, 0, 0, 0\)|transparent/.test(s4.webkitTextFillColor)) continue; // gradient text
    const bg = bgOf(el);
    if (!bg) continue;
    const fgc = fg.a < 1 ? blend(fg, bg) : fg;
    const r = ratio(fgc, bg);
    const size = parseFloat(s4.fontSize);
    const bold = Number(s4.fontWeight) >= 700;
    const large = size >= 24 || (bold && size >= 18.66);
    const need = large ? 3 : 4.5;
    if (r + 0.05 < need) a11y.contrast.push({ el: describe(el), ratio: Math.round(r * 100) / 100, need, fg: s4.color, bg: `rgb(${Math.round(bg.r)}, ${Math.round(bg.g)}, ${Math.round(bg.b)})` });
  }

  // Touch targets on phones: buttons and form controls at least 40×40 CSS px (inline text links exempt).
  if (vw <= 860) {
    const targets = document.querySelectorAll('button, .btn, [role="button"], input:not([type="hidden"]):not([type="range"]), select, [role="tab"], [role="radio"], [role="checkbox"]');
    for (const el of targets) {
      if (a11y.touch.length >= MAX) break;
      if (hiddenFromAT(el) || el.closest('.gm-board') || el.matches('.switch input')) continue;
      if (el.matches('input[type="checkbox"], input[type="radio"]') && cs(el).opacity === '0') continue;
      if (!visible(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.width < 39.5 || r.height < 39.5) a11y.touch.push(`${describe(el)} ${Math.round(r.width)}×${Math.round(r.height)}`);
    }
  }

  return { innerWidth: vw, innerHeight: vh, scrollWidth, hscroll, overflow, rawKeys: rawKeys.slice(0, MAX), junk, english, layerOverlaps: layerOverlaps.slice(0, MAX), buttonOverlaps, a11y };
})()
