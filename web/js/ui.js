// GrandMentor UI helpers: DOM builder, icons, toast, modal, formatting.
// Contract: docs/CONTRACT.md §5. Class vocabulary: docs/STYLEGUIDE.md.

import { t, getLocale } from './i18n.js';
import { getSetting } from './settings.js';
import { announce } from './components/announcer.js';

/**
 * t() with an English fallback for code that can run before i18n has loaded (global error
 * handlers, toasts raised during boot): never show a raw key such as "common.dismiss".
 */
export function tOr(key, fallback, params) {
  const v = t(key, params);
  return v === key ? fallback : v;
}

// ---------------------------------------------------------------------------
// Escaping & DOM helper
// ---------------------------------------------------------------------------
const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

/** Escape a value for safe insertion into HTML text or attribute context. */
/**
 * The player's chosen name, or '' when unset. The server's default profile name ("Player") counts
 * as unset so pages can show a localized fallback instead.
 */
export function displayName(name) {
  const n = typeof name === 'string' ? name.trim() : '';
  return n === 'Player' ? '' : n;
}

export function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, (c) => ESC[c]);
}

const SVG_NS = 'http://www.w3.org/2000/svg';
const SVG_TAGS = new Set(['svg', 'path', 'circle', 'rect', 'line', 'polyline', 'polygon', 'g', 'defs', 'marker', 'text', 'ellipse', 'linearGradient', 'stop', 'use']);

/**
 * Tiny hyperscript DOM helper.
 *   h('button', { class: 'btn btn-primary', onClick: fn, disabled: true }, 'Play')
 * attrs:
 *   class | className  string (or array, falsy entries skipped)
 *   style              string or object ({ '--board-size': '400px', width: '10px' })
 *   dataset            object -> data-* attributes
 *   html               trusted HTML string set as innerHTML (e.g. icon('play'))
 *   onXxx              function -> addEventListener('xxx', fn). Listeners are
 *                      collected with the element; detach the subtree to free them.
 *   true/false         boolean attributes; null/undefined/false are skipped
 *   ref                function called with the created element
 * children: strings/numbers (text nodes), Nodes, arrays (flattened), null/false (skipped).
 */
export function h(tag, attrs, ...children) {
  const isSvg = SVG_TAGS.has(tag);
  const el = isSvg ? document.createElementNS(SVG_NS, tag) : document.createElement(tag);
  if (attrs && (typeof attrs !== 'object' || attrs instanceof Node || Array.isArray(attrs))) {
    children.unshift(attrs);
    attrs = null;
  }
  if (attrs) {
    for (const [key, val] of Object.entries(attrs)) {
      if (val == null || val === false) continue;
      if (key === 'class' || key === 'className') {
        const cls = Array.isArray(val) ? val.filter(Boolean).join(' ') : String(val);
        if (isSvg) el.setAttribute('class', cls); else el.className = cls;
      } else if (key === 'style') {
        if (typeof val === 'string') el.style.cssText = val;
        else for (const [p, v] of Object.entries(val)) {
          if (v == null) continue;
          if (p.startsWith('--') || p.includes('-')) el.style.setProperty(p, String(v));
          else el.style[p] = v;
        }
      } else if (key === 'dataset') {
        for (const [k, v] of Object.entries(val)) if (v != null) el.dataset[k] = String(v);
      } else if (key === 'html') {
        el.innerHTML = val;
      } else if (key === 'ref') {
        if (typeof val === 'function') val(el);
      } else if (key.startsWith('on') && typeof val === 'function') {
        el.addEventListener(key.slice(2).toLowerCase(), val);
      } else if (val === true) {
        el.setAttribute(key, '');
      } else if (!isSvg && (key === 'value' || key === 'checked' || key === 'selected') ) {
        el[key] = val;
      } else {
        el.setAttribute(key, String(val));
      }
    }
  }
  appendChildren(el, children);
  return el;
}

function appendChildren(el, children) {
  for (const child of children) {
    if (child == null || child === false || child === true) continue;
    if (Array.isArray(child)) appendChildren(el, child);
    else if (child instanceof Node) el.appendChild(child);
    else el.appendChild(document.createTextNode(String(child)));
  }
}

/** Parse a trusted HTML string (e.g. from icon()) into a single Node. */
export function htmlToNode(html) {
  const t = document.createElement('template');
  t.innerHTML = String(html).trim();
  return t.content.firstChild;
}

/** Remove all children of an element. */
export function clear(el) {
  if (el) el.replaceChildren();
  return el;
}

// ---------------------------------------------------------------------------
// Icons — 24x24 stroke icons using currentColor. icon(name, {size, cls})
// ---------------------------------------------------------------------------
const S = (body) => body; // readability marker
const ICONS = {
  // Navigation
  home: S('<path d="M3 10.5 12 3l9 7.5"/><path d="M5 9.5V20a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1V9.5"/>'),
  play: S('<path d="M7 21h11"/><path d="M8 18h9c0-3.5-1-6.5-2-8.5 1.5-1 2.5-2.8 2-5-1.8.3-3 .8-4 1.7L11 4 9.5 6.8C7.2 8 6 10.2 6 12.5l2.5.5 2.5-1.8-.6 2.5C9 15 8 16.3 8 18Z"/><circle cx="12.6" cy="8.6" r=".6" fill="currentColor"/>'),
  puzzle: S('<path d="M10 3.5a2 2 0 1 1 4 0V6h4a1 1 0 0 1 1 1v4h-2.5a2 2 0 1 0 0 4H19v4a1 1 0 0 1-1 1h-4v-2.5a2 2 0 1 0-4 0V20H6a1 1 0 0 1-1-1v-4h2.5a2 2 0 1 0 0-4H5V7a1 1 0 0 1 1-1h4V3.5Z"/>'),
  learn: S('<path d="m2 9 10-5 10 5-10 5L2 9Z"/><path d="M6 11v5c0 1.7 2.7 3 6 3s6-1.3 6-3v-5"/><path d="M22 9v6"/>'),
  openings: S('<path d="M4 4.5A1.5 1.5 0 0 1 5.5 3H11v17H5.5A1.5 1.5 0 0 0 4 21.5v-17Z"/><path d="M20 4.5A1.5 1.5 0 0 0 18.5 3H13v17h5.5a1.5 1.5 0 0 1 1.5 1.5v-17Z"/><path d="M7 8h1.5M7 11h1.5M15.5 8H17M15.5 11H17"/>'),
  endgames: S('<path d="M12 2v4"/><path d="M10 4h4"/><path d="M8 21h8"/><path d="M7.5 18h9l-1-6.5A3.5 3.5 0 0 0 12 6a3.5 3.5 0 0 0-3.5 5.5l-1 6.5Z"/><path d="M9 11.5h6"/>'),
  analysis: S('<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/><path d="M8 13l2-2.5 2 1.5 2-3"/>'),
  library: S('<path d="M4 4h4v16H4z"/><path d="M10 4h4v16h-4z"/><path d="m15.5 5.2 3.8-1 3.7 14.5-3.8 1z"/>'),
  profile: S('<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-7 8-7s8 3 8 7"/>'),
  settings: S('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"/>'),
  more: S('<circle cx="5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="19" cy="12" r="1.5"/>'),
  menu: S('<path d="M4 6h16M4 12h16M4 18h16"/>'),

  // Chess pieces (outline)
  knight: S('<path d="M7 21h11"/><path d="M8 18h9c0-3.5-1-6.5-2-8.5 1.5-1 2.5-2.8 2-5-1.8.3-3 .8-4 1.7L11 4 9.5 6.8C7.2 8 6 10.2 6 12.5l2.5.5 2.5-1.8-.6 2.5C9 15 8 16.3 8 18Z"/><circle cx="12.6" cy="8.6" r=".6" fill="currentColor"/>'),
  king: S('<path d="M12 2v4M10 4h4"/><path d="M6 21h12"/><path d="M7 18h10l1-5.5c.3-1.8-1-3.5-2.8-3.5-1.4 0-2.6.8-3.2 2-.6-1.2-1.8-2-3.2-2C7 9 5.7 10.7 6 12.5L7 18Z"/>'),
  queen: S('<path d="M6 21h12"/><path d="M7 18h10l2-10-4.5 4L12 5l-2.5 7L5 8l2 10Z"/><circle cx="5" cy="6.5" r="1.3"/><circle cx="12" cy="3.5" r="1.3"/><circle cx="19" cy="6.5" r="1.3"/>'),
  pawn: S('<circle cx="12" cy="6.5" r="3"/><path d="M9.5 11h5"/><path d="M10 11c0 3-1.5 5-3 7h10c-1.5-2-3-4-3-7"/><path d="M6 21h12"/>'),
  rook: S('<path d="M6 21h12"/><path d="M7 18h10"/><path d="M8 18l1-9h6l1 9"/><path d="M6 4h3v2h2V4h2v2h2V4h3v3l-2 2H8L6 7V4Z"/>'),
  bishop: S('<path d="M6 21h12"/><path d="M8 18h8"/><path d="M9 18c-.5-2 0-4 1-5.5C8 11 8 8 12 4c4 4 4 7 2 8.5 1 1.5 1.5 3.5 1 5.5"/><path d="m12 9 2-2"/><circle cx="12" cy="3" r="1"/>'),
  crown: S('<path d="M3 7.5 7.5 11 12 4l4.5 7L21 7.5 19 18H5L3 7.5Z"/><path d="M5 21h14"/>'),
  board: S('<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M3 15h18M9 3v18M15 3v18"/><path d="M3 3h6v6H3zM9 9h6v6H9zM15 15h6v6h-6zM15 3h6v6h-6zM3 15h6v6H3z" fill="currentColor" fill-opacity=".25" stroke="none"/>'),

  // Board / navigation controls
  'chevron-left': S('<path d="m15 18-6-6 6-6"/>'),
  'chevron-right': S('<path d="m9 18 6-6-6-6"/>'),
  'chevron-up': S('<path d="m18 15-6-6-6 6"/>'),
  'chevron-down': S('<path d="m6 9 6 6 6-6"/>'),
  first: S('<path d="m11 17-5-5 5-5"/><path d="m18 17-5-5 5-5"/>'),
  last: S('<path d="m13 17 5-5-5-5"/><path d="m6 17 5-5-5-5"/>'),
  'arrow-left': S('<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/>'),
  'arrow-right': S('<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>'),
  flip: S('<path d="M7 4v16"/><path d="m3 8 4-4 4 4"/><path d="M17 20V4"/><path d="m21 16-4 4-4-4"/>'),
  undo: S('<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>'),
  redo: S('<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/>'),
  refresh: S('<path d="M21 12a9 9 0 0 1-15.5 6.2L3 15.5"/><path d="M3 12A9 9 0 0 1 18.5 5.8L21 8.5"/><path d="M21 3v5.5h-5.5"/><path d="M3 21v-5.5h5.5"/>'),
  'play-circle': S('<circle cx="12" cy="12" r="9"/><path d="m10 8.5 5.5 3.5-5.5 3.5v-7Z" fill="currentColor"/>'),
  pause: S('<rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/>'),

  // Actions
  close: S('<path d="M18 6 6 18M6 6l12 12"/>'),
  x: S('<path d="M18 6 6 18M6 6l12 12"/>'),
  check: S('<path d="M20 6 9 17l-5-5"/>'),
  plus: S('<path d="M12 5v14M5 12h14"/>'),
  minus: S('<path d="M5 12h14"/>'),
  search: S('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'),
  filter: S('<path d="M3 5h18l-7 8.5V19l-4 2v-7.5L3 5Z"/>'),
  edit: S('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5Z"/>'),
  trash: S('<path d="M3 6h18"/><path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2"/><path d="M19 6l-1 14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1L5 6"/><path d="M10 11v6M14 11v6"/>'),
  copy: S('<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"/>'),
  download: S('<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>'),
  upload: S('<path d="M12 21V9"/><path d="m7 14 5-5 5 5"/><path d="M5 3h14"/>'),
  share: S('<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="m8.6 13.5 6.8 4M15.4 6.5l-6.8 4"/>'),
  external: S('<path d="M14 4h6v6"/><path d="M20 4 10 14"/><path d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5"/>'),
  save: S('<path d="M5 3h11l5 5v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h1Z"/><path d="M7 3v5h8V3"/><rect x="7" y="13" width="10" height="8"/>'),
  link: S('<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/>'),
  eye: S('<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>'),
  'eye-off': S('<path d="M10.6 5.1A10 10 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-3 3.9"/><path d="M6.6 6.6A17 17 0 0 0 2 12s3.6 7 10 7a9.7 9.7 0 0 0 5.4-1.6"/><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/><path d="m2 2 20 20"/>'),
  lock: S('<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>'),
  globe: S('<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a14 14 0 0 1 0 18a14 14 0 0 1 0-18Z"/>'),

  // Status & feedback
  info: S('<circle cx="12" cy="12" r="9"/><path d="M12 11v5"/><circle cx="12" cy="7.8" r=".6" fill="currentColor"/>'),
  alert: S('<path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4"/><circle cx="12" cy="17" r=".6" fill="currentColor"/>'),
  'check-circle': S('<circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/>'),
  'x-circle': S('<circle cx="12" cy="12" r="9"/><path d="m15 9-6 6M9 9l6 6"/>'),
  help: S('<circle cx="12" cy="12" r="9"/><path d="M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6"/><circle cx="12" cy="17" r=".6" fill="currentColor"/>'),
  hint: S('<path d="M9 18h6"/><path d="M10 21h4"/><path d="M12 3a6 6 0 0 0-3.5 10.9c.6.4 1 1.1 1 1.8V16h5v-.3c0-.7.4-1.4 1-1.8A6 6 0 0 0 12 3Z"/>'),
  star: S('<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1 6.2L12 17.3 6.5 20.2l1-6.2L3 9.6l6.2-.9L12 3Z"/>'),
  'star-filled': S('<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1 6.2L12 17.3 6.5 20.2l1-6.2L3 9.6l6.2-.9L12 3Z" fill="currentColor"/>'),
  heart: S('<path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z"/>'),
  trophy: S('<path d="M8 21h8M12 17v4"/><path d="M7 4h10v5a5 5 0 0 1-10 0V4Z"/><path d="M17 5h3v2a3 3 0 0 1-3 3M7 5H4v2a3 3 0 0 0 3 3"/>'),
  medal: S('<circle cx="12" cy="15" r="6"/><path d="M8.5 10 6 3h4l2 4 2-4h4l-2.5 7"/><path d="m12 12.5.9 1.8 2 .3-1.4 1.4.3 2-1.8-1-1.8 1 .3-2-1.4-1.4 2-.3.9-1.8Z" fill="currentColor" stroke="none"/>'),
  target: S('<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1" fill="currentColor"/>'),
  flag: S('<path d="M5 21V4"/><path d="M5 4h12l-2 4 2 4H5"/>'),
  handshake: S('<path d="m11 17 2 2a1.4 1.4 0 0 0 2-2"/><path d="m14 14 2.5 2.5a1.4 1.4 0 0 0 2-2l-3.9-3.9a2.8 2.8 0 0 0-4 0l-.9.9a1.4 1.4 0 0 1-2-2l2.8-2.8a4 4 0 0 1 4.9-.6l.5.3a3 3 0 0 0 2 .3L21 6"/><path d="m21 5 1 9-2 2"/><path d="M3 5 2 14l6.5 6.5a1.4 1.4 0 0 0 2-2"/><path d="M3 6h8"/>'),
  bolt: S('<path d="M13 2 4 14h7l-1 8 9-12h-7l1-8Z"/>'),
  fire: S('<path d="M12 22c4 0 7-2.7 7-6.8 0-3.2-2-5.7-3.5-7.2.2 1.8-.6 3.3-1.8 3.8C14 8 12.5 4.5 9.5 2.5 9.8 6 7.5 8 6.3 9.8A8 8 0 0 0 5 15.2C5 19.3 8 22 12 22Z"/>'),
  clock: S('<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>'),
  timer: S('<circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M10 2h4M12 2v3"/>'),
  calendar: S('<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M3 10h18M8 3v4M16 3v4"/>'),
  chart: S('<path d="M3 3v18h18"/><path d="m7 15 4-4 3 3 6-7"/>'),
  sparkles: S('<path d="M12 3 13.8 8.2 19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3Z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8L19 15Z"/>'),
  shield: S('<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6l-8-3Z"/>'),
  swords: S('<path d="M14.5 17.5 3 6V3h3l11.5 11.5"/><path d="m13 19 6-6M16 16l4 4M19 21l2-2"/><path d="M14.5 6.5 18 3h3v3l-3.5 3.5"/><path d="m5 14 4 4M7 17l-3 3M3 19l2 2"/>'),
  robot: S('<rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 4v4"/><circle cx="12" cy="3" r="1"/><circle cx="9" cy="13.5" r="1.3" fill="currentColor"/><circle cx="15" cy="13.5" r="1.3" fill="currentColor"/><path d="M9.5 17h5"/><path d="M2 13v3M22 13v3"/>'),
  mentor: S('<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.4A8 8 0 1 1 21 12Z"/><path d="M9 11h.01M12 11h.01M15 11h.01" stroke-width="3"/>'),
  chat: S('<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.4A8 8 0 1 1 21 12Z"/>'),
  send: S('<path d="M22 2 11 13"/><path d="M22 2 15 22l-4-9-9-4 20-7Z"/>'),
  book: S('<path d="M4 19.5V5a2 2 0 0 1 2-2h14v15H6a2 2 0 0 0-2 2Zm0 0A2 2 0 0 0 6 21.5h14"/>'),
  folder: S('<path d="M3 6a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6Z"/>'),
  tag: S('<path d="M3 12V4a1 1 0 0 1 1-1h8l9 9-9 9-9-9Z"/><circle cx="7.5" cy="7.5" r="1.3" fill="currentColor"/>'),
  user: S('<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-7 8-7s8 3 8 7"/>'),
  users: S('<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20c0-3.6 2.9-6 6.5-6s6.5 2.4 6.5 6"/><path d="M16 4.5a3.5 3.5 0 0 1 0 7M18 14c2.3.6 3.5 2.6 3.5 6"/>'),
  sun: S('<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>'),
  moon: S('<path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8Z"/>'),
  volume: S('<path d="M11 5 6 9H2v6h4l5 4V5Z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14"/>'),
  'volume-off': S('<path d="M11 5 6 9H2v6h4l5 4V5Z"/><path d="m22 9-6 6M16 9l6 6"/>'),
  sidebar: S('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/><path d="m15 10-2 2 2 2"/>'),
  grid: S('<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>'),
  list: S('<path d="M8 6h13M8 12h13M8 18h13"/><circle cx="4" cy="6" r="1" fill="currentColor"/><circle cx="4" cy="12" r="1" fill="currentColor"/><circle cx="4" cy="18" r="1" fill="currentColor"/>'),
  palette: S('<path d="M12 22a10 10 0 1 1 10-10c0 2.8-2.2 4-4.5 4H16a2 2 0 0 0-1.5 3.3c.4.5.5 1 .5 1.4 0 .8-1.2 1.3-3 1.3Z"/><circle cx="7.5" cy="10.5" r="1.2" fill="currentColor"/><circle cx="12" cy="7" r="1.2" fill="currentColor"/><circle cx="16.5" cy="10.5" r="1.2" fill="currentColor"/>'),
  keyboard: S('<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>'),
  wifi: S('<path d="M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M2 9a15 15 0 0 1 20 0"/><circle cx="12" cy="19.5" r=".8" fill="currentColor"/>'),
  'wifi-off': S('<path d="m2 2 20 20"/><path d="M8.5 16a5 5 0 0 1 7 0M5 12.5a10 10 0 0 1 4.2-2.4M16.6 10.9a10 10 0 0 1 2.4 1.6M2 9a15 15 0 0 1 4.3-2.8M10.7 5.1A15 15 0 0 1 22 9"/><circle cx="12" cy="19.5" r=".8" fill="currentColor"/>'),
};

/** Names available to icon(). */
export const ICON_NAMES = Object.freeze(Object.keys(ICONS));

/**
 * Inline SVG string for an icon. Unknown names render a neutral circle.
 * @param {string} name
 * @param {{size?: number, cls?: string, label?: string, strokeWidth?: number}} [opts]
 */
export function icon(name, opts = {}) {
  const body = ICONS[name] || '<circle cx="12" cy="12" r="8"/>';
  const size = opts.size ? ` width="${Number(opts.size) || 20}" height="${Number(opts.size) || 20}"` : '';
  const cls = opts.cls ? ` class="${escapeHtml(opts.cls)}"` : '';
  const a11y = opts.label ? ` role="img" aria-label="${escapeHtml(opts.label)}"` : ' aria-hidden="true"';
  const sw = Number(opts.strokeWidth) || 2;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"${size}${cls}${a11y} fill="none" stroke="currentColor" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" focusable="false">${body}</svg>`;
}

/** Same as icon() but returns an SVG Node. */
export function iconNode(name, opts) {
  return htmlToNode(icon(name, opts));
}

/** Brand mark (the crowned-knight app icon) as an <img> HTML string. */
export function brandMark(size = 36) {
  const s = Number(size) || 36;
  const src = s > 64 ? '/img/icons/mark-128.png' : '/img/icons/mark-64.png';
  return `<img class="brand-mark" src="${src}" width="${s}" height="${s}" alt="" aria-hidden="true" decoding="async">`;
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/**
 * Format an engine Score (white POV) for display: {cp:123} -> "+1.2", {cp:-40} -> "-0.4",
 * {mate:3} -> "M3", {mate:-2} -> "-M2", {mate:0} -> "#". Null/invalid -> "–".
 * opts.pov = 'black' flips the sign (for side-to-move displays). opts.digits (default 1).
 */
export function formatScore(score, opts = {}) {
  if (!score || typeof score !== 'object') return '–';
  const flip = opts.pov === 'black' ? -1 : 1;
  if (typeof score.mate === 'number') {
    const m = score.mate * flip;
    if (m === 0) return '#';
    return m > 0 ? `M${m}` : `-M${-m}`;
  }
  if (typeof score.cp === 'number') {
    const pawns = (score.cp * flip) / 100;
    const digits = opts.digits ?? 1;
    const s = Math.abs(pawns).toFixed(digits);
    if (Number(s) === 0) return (0).toFixed(digits);
    return (pawns > 0 ? '+' : '-') + s;
  }
  return '–';
}

/** Numeric value of a score in pawns for charts, clamped to ±clamp (mate = ±clamp). White POV. */
export function scoreToNumber(score, clamp = 10) {
  if (!score) return 0;
  if (typeof score.mate === 'number') return score.mate >= 0 ? clamp : -clamp;
  if (typeof score.cp === 'number') return Math.max(-clamp, Math.min(clamp, score.cp / 100));
  return 0;
}

/** Lichess win% (0..100) for white from a Score. */
export function winPercent(score) {
  if (!score) return 50;
  if (typeof score.mate === 'number') return score.mate >= 0 ? 100 : 0;
  const cp = Math.max(-1000, Math.min(1000, Number(score.cp) || 0));
  return 50 + 50 * (2 / (1 + Math.exp(-0.00368208 * cp)) - 1);
}

// Labels/descriptions are i18n keys (ui.cls.<key>.label / .description), resolved at call time.
// Colour-blind-safe palette (Okabe–Ito based): good moves in blues, bad moves in yellow → orange →
// purple → vermillion, so the two halves stay apart with deuteranopia / protanopia. Symbols are
// always shown with the colour. Selected by the `cbPalette` setting (<html data-cls-palette="cb">).
const CLS_CB = {
  brilliant: '#1fb5c9', great: '#4a90e2', best: '#56b4e9', excellent: '#7fc4ec', good: '#a3bccc', book: '#b8977a',
  inaccuracy: '#f0e442', mistake: '#e69f00', miss: '#cc79a7', blunder: '#f0712c', forced: '#a3bccc',
};

const CLS = {
  brilliant: { color: '#26c2a3', symbol: '!!' },
  great: { color: '#5c8bb0', symbol: '!' },
  best: { color: '#81b64c', symbol: '★' },
  excellent: { color: '#96bc4b', symbol: '👍' },
  good: { color: '#96af8b', symbol: '✓' },
  book: { color: '#a88865', symbol: '📖' },
  inaccuracy: { color: '#f7c631', symbol: '?!' },
  mistake: { color: '#ffa459', symbol: '?' },
  miss: { color: '#ff7769', symbol: '✗' },
  blunder: { color: '#fa412d', symbol: '??' },
  forced: { color: '#96af8b', symbol: '→' },
};

/** Ordered list of classification keys (best → worst, then forced). */
export const CLASSIFICATIONS = Object.freeze(['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder', 'forced']);

/**
 * Metadata for a move classification.
 * @returns {{key:string,label:string,color:string,cssVar:string,symbol:string,description:string}}
 */
export function classificationMeta(cls) {
  const key = String(cls || '').toLowerCase();
  const m = CLS[key];
  if (!m) return { key, label: key ? key[0].toUpperCase() + key.slice(1) : '', color: '#96af8b', cssVar: 'var(--cls-good)', symbol: '', description: '' };
  let cb = false;
  try { cb = getSetting('cbPalette') === true; } catch { cb = false; }
  return { key, ...m, color: cb ? CLS_CB[key] : m.color, label: t(`ui.cls.${key}.label`), description: t(`ui.cls.${key}.description`), cssVar: `var(--cls-${key})` };
}

/** <span class="cls-badge" data-cls="..."> element for a classification. */
export function classificationBadge(cls, { large = false } = {}) {
  const m = classificationMeta(cls);
  return h('span', { class: large ? 'cls-badge lg' : 'cls-badge', dataset: { cls: m.key }, title: m.label, role: 'img', 'aria-label': m.label }, m.symbol);
}

const PIECE_FIGURINES = { K: '♔', Q: '♕', R: '♖', B: '♗', N: '♘' };
/** Convert SAN to figurine notation ("Nf3" -> "♘f3") when notation === 'figurine'. */
export function formatSan(san, notation = 'san') {
  if (notation !== 'figurine' || !san) return san ?? '';
  return String(san).replace(/[KQRBN]/g, (p) => PIECE_FIGURINES[p]);
}

/** Milliseconds -> "m:ss" (or "h:mm:ss"); under 10s shows tenths ("0:07.3") when tenths=true. */
export function formatClock(ms, { tenths = true } = {}) {
  const t = Math.max(0, Math.floor(Number(ms) || 0));
  const totalSec = Math.floor(t / 1000);
  const hh = Math.floor(totalSec / 3600);
  const mm = Math.floor((totalSec % 3600) / 60);
  const ss = totalSec % 60;
  if (hh > 0) return `${hh}:${String(mm).padStart(2, '0')}:${String(ss).padStart(2, '0')}`;
  if (tenths && t < 10000) return `${mm}:${String(ss).padStart(2, '0')}.${Math.floor((t % 1000) / 100)}`;
  return `${mm}:${String(ss).padStart(2, '0')}`;
}

/** Friendly relative time from an ISO string/Date: "just now", "5 min ago", "Yesterday", "Mar 3" (localized). */
export function formatRelative(date) {
  const d = date instanceof Date ? date : new Date(date);
  if (Number.isNaN(d.getTime())) return '';
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 45) return t('ui.time.justNow');
  if (diff < 3600) return t('ui.time.minutesAgo', { count: Math.round(diff / 60) });
  if (diff < 86400) return t('ui.time.hoursAgo', { count: Math.round(diff / 3600) });
  if (diff < 172800) return t('ui.time.yesterday');
  if (diff < 604800) return t('ui.time.daysAgo', { count: Math.round(diff / 86400) });
  return d.toLocaleDateString(getLocale(), { month: 'short', day: 'numeric', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' });
}

/** Format a date (ISO/Date) as "Mar 3, 2026" (localized). */
export function formatDate(date) {
  const d = date instanceof Date ? date : new Date(date);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleDateString(getLocale(), { month: 'short', day: 'numeric', year: 'numeric' });
}

/**
 * Markdown-lite (lesson steps / mentor text) -> safe HTML string.
 * Supports **bold**, *italic*, `code`, line breaks, blank-line paragraphs.
 */
export function mdLite(text) {
  const esc = escapeHtml(text ?? '');
  return esc
    .split(/\n{2,}/)
    .map((para) => '<p>' + para
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      .replace(/(^|[^*])\*(?!\s)(.+?)\*(?!\*)/g, '$1<em>$2</em>')
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\n/g, '<br>') + '</p>')
    .join('');
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

/** Debounce; returned function has .cancel(). */
export function debounce(fn, ms = 200) {
  let t = null;
  const d = (...args) => {
    if (t) clearTimeout(t);
    t = setTimeout(() => { t = null; fn(...args); }, ms);
  };
  d.cancel = () => { if (t) clearTimeout(t); t = null; };
  return d;
}

/**
 * Collects disposers so pages can clean up with one call.
 *   const bag = disposables();
 *   bag.on(window, 'keydown', fn);   bag.timeout(fn, 500);   bag.add(() => board.destroy());
 *   return bag.dispose;              // from mount()
 */
export function disposables() {
  const list = [];
  let disposed = false;
  const add = (fn) => {
    if (typeof fn !== 'function') return fn;
    if (disposed) { try { fn(); } catch (e) { console.error(e); } return fn; }
    list.push(fn);
    return fn;
  };
  return {
    add,
    on(target, type, handler, options) {
      target.addEventListener(type, handler, options);
      add(() => target.removeEventListener(type, handler, options));
    },
    timeout(fn, ms) { const id = setTimeout(fn, ms); add(() => clearTimeout(id)); return id; },
    interval(fn, ms) { const id = setInterval(fn, ms); add(() => clearInterval(id)); return id; },
    raf(fn) { const id = requestAnimationFrame(fn); add(() => cancelAnimationFrame(id)); return id; },
    get disposed() { return disposed; },
    dispose() {
      if (disposed) return;
      disposed = true;
      while (list.length) {
        const fn = list.pop();
        try { fn(); } catch (e) { console.error('dispose error', e); }
      }
    },
  };
}

/** Copy text to clipboard; toasts on success/failure. */
export async function copyText(text, successMsg) {
  try {
    await navigator.clipboard.writeText(String(text));
    toast(successMsg || t('ui.copied'), 'success');
    return true;
  } catch {
    toast(t('ui.copyFailed'), 'error');
    return false;
  }
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------
const TOAST_MAX = 4;
const TOAST_ICONS = { info: 'info', success: 'check-circle', warning: 'alert', error: 'x-circle' };

function toastHost() {
  let host = document.getElementById('toasts');
  if (!host) {
    host = h('div', { id: 'toasts', class: 'toasts' });
    document.body.appendChild(host);
  }
  return host;
}

/**
 * Show a toast. kind: 'info' | 'success' | 'warning' | 'error'.
 * opts.duration (ms, default 3500; 0 = sticky). Returns a dismiss() function.
 */
export function toast(msg, kind = 'info', opts = {}) {
  const k = TOAST_ICONS[kind] ? kind : 'info';
  const host = toastHost();
  const duration = opts.duration ?? (k === 'error' ? 5000 : 3500);
  let timer = null;
  let gone = false;

  // Screen readers hear toasts through the shared announcer (errors interrupt, the rest is polite),
  // so the toast stack itself is not a live region (no double announcements).
  const el = h('div', { class: `toast toast-${k}` });
  el.innerHTML = icon(TOAST_ICONS[k]);
  const closeBtn = h('button', { class: 'toast-close', type: 'button', 'aria-label': tOr('common.dismiss', 'Dismiss'), html: icon('close') });
  el.append(h('div', { class: 'toast-msg' }, String(msg ?? '')), closeBtn);
  try { announce(String(msg ?? ''), { assertive: k === 'error' || k === 'warning' }); } catch { /* ignore */ }

  const dismiss = () => {
    if (gone) return;
    gone = true;
    if (timer) clearTimeout(timer);
    el.classList.add('leaving');
    const remove = () => el.remove();
    el.addEventListener('animationend', remove, { once: true });
    setTimeout(remove, 400); // fallback when animations are disabled
  };
  closeBtn.addEventListener('click', dismiss);
  el.addEventListener('mouseenter', () => { if (timer) { clearTimeout(timer); timer = null; } });
  el.addEventListener('mouseleave', () => { if (!gone && duration > 0 && !timer) timer = setTimeout(dismiss, 1500); });

  host.appendChild(el);
  // Bounded: drop the oldest beyond TOAST_MAX.
  const live = host.querySelectorAll('.toast:not(.leaving)');
  if (live.length > TOAST_MAX) {
    for (let i = 0; i < live.length - TOAST_MAX; i++) live[i].remove();
  }
  if (duration > 0) timer = setTimeout(dismiss, duration);
  return dismiss;
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------
const openModals = [];

/**
 * Open a modal dialog.
 * @param {{title?: string, body?: Node|string, actions?: Array<{label:string, kind?:string, icon?:string, onClick?:(close:Function)=>any, autofocus?:boolean}>,
 *          size?: 'lg', dismissible?: boolean, onClose?: Function, className?: string}} opts
 *   body as string is treated as TEXT (escaped); pass a Node for rich content.
 *   action.kind: 'primary' | 'secondary' | 'ghost' | 'danger' | 'outline' (default 'secondary').
 *   action.onClick(close): return false to keep the modal open; may be async.
 * @returns {{close: Function, el: HTMLElement, body: HTMLElement}}
 */
export function modal({ title = '', body = '', actions, size, dismissible = true, onClose, className } = {}) {
  if (actions === undefined) actions = [{ label: t('common.ok'), kind: 'primary' }];
  const prevFocus = document.activeElement;
  const titleId = `modal-title-${Math.random().toString(36).slice(2, 8)}`;
  const bodyEl = h('div', { class: 'modal-body' });
  if (body instanceof Node) bodyEl.appendChild(body);
  else if (body) bodyEl.appendChild(h('p', null, String(body)));

  const header = h('div', { class: 'modal-header' },
    title ? h('h2', { class: 'modal-title', id: titleId }, title) : h('div', { class: 'spacer' }),
    dismissible ? h('button', { class: 'btn btn-ghost btn-icon btn-sm', type: 'button', 'aria-label': t('common.close'), html: icon('close'), onClick: () => close() }) : null,
  );

  let closed = false;
  const footer = actions && actions.length ? h('div', { class: 'modal-footer' }) : null;
  for (const a of actions || []) {
    const btn = h('button', { class: `btn btn-${a.kind || 'secondary'}`, type: 'button' });
    if (a.icon) btn.innerHTML = icon(a.icon);
    btn.appendChild(document.createTextNode(a.label));
    btn.addEventListener('click', async () => {
      if (closed) return;
      try {
        if (a.onClick) {
          btn.classList.add('loading');
          const r = await a.onClick(close);
          btn.classList.remove('loading');
          if (r === false) return;
        }
        close();
      } catch (e) {
        btn.classList.remove('loading');
        toast(e?.message || t('common.somethingWentWrong'), 'error');
      }
    });
    if (a.autofocus) btn.dataset.autofocus = '1';
    footer.appendChild(btn);
  }

  const dialog = h('div', { class: ['modal', size === 'lg' && 'modal-lg', className], role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': title ? titleId : null, tabindex: '-1' },
    header, bodyEl, footer);
  const backdrop = h('div', { class: 'modal-backdrop' }, dialog);

  const onKey = (e) => {
    if (openModals[openModals.length - 1] !== api) return;
    if (e.key === 'Escape' && dismissible) { e.preventDefault(); close(); }
    else if (e.key === 'Tab') trapFocus(e, dialog);
  };
  const onBackdrop = (e) => { if (e.target === backdrop && dismissible) close(); };

  function close() {
    if (closed) return;
    closed = true;
    document.removeEventListener('keydown', onKey, true);
    backdrop.removeEventListener('mousedown', onBackdrop);
    const i = openModals.indexOf(api);
    if (i >= 0) openModals.splice(i, 1);
    backdrop.classList.add('closing');
    const remove = () => backdrop.remove();
    backdrop.addEventListener('animationend', remove, { once: true });
    setTimeout(remove, 350);
    if (prevFocus && typeof prevFocus.focus === 'function' && document.contains(prevFocus)) {
      try { prevFocus.focus({ preventScroll: true }); } catch { /* ignore */ }
    }
    if (onClose) { try { onClose(); } catch (e) { console.error(e); } }
  }

  const api = { close, el: dialog, body: bodyEl };
  openModals.push(api);
  document.addEventListener('keydown', onKey, true);
  backdrop.addEventListener('mousedown', onBackdrop);
  document.body.appendChild(backdrop);
  const focusTarget = dialog.querySelector('[data-autofocus], input, select, textarea') || dialog.querySelector('.modal-footer .btn-primary') || dialog.querySelector('button') || dialog;
  requestAnimationFrame(() => { if (!closed) focusTarget.focus({ preventScroll: true }); });
  return api;
}

const FOCUSABLE = 'a[href], area[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

/** Visible, keyboard-focusable elements inside `container`, in DOM order. */
export function focusableIn(container) {
  return Array.from(container.querySelectorAll(FOCUSABLE)).filter((el) => {
    if (el.closest('[inert], [hidden], [aria-hidden="true"]')) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 || r.height > 0;
  });
}

function trapFocus(e, container) {
  const items = focusableIn(container);
  if (!items.length) { e.preventDefault(); container.focus(); return; }
  const first = items[0];
  const last = items[items.length - 1];
  const inside = container.contains(document.activeElement);
  if (e.shiftKey && (document.activeElement === first || !inside || document.activeElement === container)) { e.preventDefault(); last.focus(); }
  else if (!e.shiftKey && (document.activeElement === last || !inside)) { e.preventDefault(); first.focus(); }
}

/** Close every open modal (the router calls this on navigation). */
export function closeAllModals() {
  for (const m of openModals.slice()) m.close();
}

/** Promise-based confirm dialog. Resolves true/false. */
export function confirmDialog({ title, message = '', confirmLabel, cancelLabel, danger = false } = {}) {
  title = title ?? t('ui.confirmTitle');
  confirmLabel = confirmLabel ?? t('common.confirm');
  cancelLabel = cancelLabel ?? t('common.cancel');
  return new Promise((resolve) => {
    let result = false;
    modal({
      title,
      body: message,
      actions: [
        { label: cancelLabel, kind: 'ghost' },
        { label: confirmLabel, kind: danger ? 'danger' : 'primary', autofocus: true, onClick: () => { result = true; } },
      ],
      onClose: () => resolve(result),
    });
  });
}

// ---------------------------------------------------------------------------
// Ready-made building blocks
// ---------------------------------------------------------------------------

/**
 * Page header block.
 * pageHeader({ title, subtitle, icon: 'puzzle', actions: [Node...], breadcrumbs: [{label, href}] })
 */
export function pageHeader({ title, subtitle, icon: iconName, actions = [], breadcrumbs } = {}) {
  const crumbs = breadcrumbs && breadcrumbs.length
    ? h('nav', { class: 'breadcrumbs', 'aria-label': t('ui.breadcrumb') },
      breadcrumbs.flatMap((b, i) => [
        i > 0 ? htmlToNode(icon('chevron-right')) : null,
        b.href ? h('a', { href: b.href }, b.label) : h('span', null, b.label),
      ]))
    : null;
  return h('header', { class: 'page-header' },
    h('div', { class: 'stack-sm', style: 'min-width:0' },
      crumbs,
      h('div', { class: 'page-header-main' },
        iconName ? h('div', { class: 'page-header-icon', html: icon(iconName) }) : null,
        h('div', { style: 'min-width:0' },
          h('h1', { class: 'page-title' }, title || ''),
          subtitle ? h('p', { class: 'page-subtitle' }, subtitle) : null))),
    actions.length ? h('div', { class: 'page-actions' }, actions) : null);
}

/** Empty state block. emptyState({ icon, title, text, action: {label, href|onClick, icon} }) */
export function emptyState({ icon: iconName = 'info', emoji, title, text = '', action } = {}) {
  if (title == null) title = t('ui.emptyTitle');
  let btn = null;
  if (action) {
    btn = h(action.href ? 'a' : 'button', { class: `btn ${action.kind ? 'btn-' + action.kind : 'btn-primary'}`, href: action.href, type: action.href ? null : 'button', onClick: action.onClick });
    if (action.icon) btn.innerHTML = icon(action.icon);
    btn.appendChild(document.createTextNode(action.label));
  }
  return h('div', { class: 'empty-state' },
    emoji ? h('div', { class: 'empty-state-icon' }, emoji) : h('div', { class: 'empty-state-icon', html: icon(iconName) }),
    h('div', { class: 'empty-state-title' }, title),
    text ? h('p', null, text) : null,
    btn);
}

/** Loading block with spinner and optional label. */
export function loadingBlock(label) {
  if (label == null) label = t('common.loading');
  return h('div', { class: 'loading-center', role: 'status' }, h('div', { class: 'spinner spinner-lg' }), h('div', null, label));
}

/** Skeleton placeholders: skeleton('card', 3) | skeleton('text', 4) | skeleton('list', 5). */
export function skeleton(kind = 'text', count = 3) {
  const n = Math.max(1, Math.min(24, count | 0));
  if (kind === 'card') {
    return h('div', { class: 'grid-auto', 'aria-hidden': 'true' }, Array.from({ length: n }, () => h('div', { class: 'skeleton skeleton-card' })));
  }
  if (kind === 'list') {
    return h('div', { class: 'card card-flush', 'aria-hidden': 'true' }, Array.from({ length: n }, () =>
      h('div', { class: 'list-row' }, h('div', { class: 'skeleton skeleton-avatar' }),
        h('div', { class: 'list-row-main' }, h('div', { class: 'skeleton skeleton-text', style: 'width:45%' }), h('div', { class: 'skeleton skeleton-text', style: 'width:70%' })))));
  }
  return h('div', { 'aria-hidden': 'true' }, Array.from({ length: n }, (_, i) =>
    h('div', { class: 'skeleton skeleton-text', style: `width:${i === n - 1 ? 60 : 100 - (i % 3) * 8}%` })));
}

/**
 * Render a friendly "coming soon" card into root (used by placeholder pages).
 * Returns a cleanup function.
 */
export function comingSoon(root, { title, icon: iconName = 'sparkles', text } = {}) {
  if (text == null) text = t('ui.comingSoonText');
  const page = h('div', { class: 'page' },
    h('div', { class: 'card placeholder-card' },
      h('div', { class: 'empty-state-icon', html: icon(iconName) }),
      h('h2', { class: 'mb-2' }, title || t('ui.comingSoon')),
      h('p', { class: 'muted' }, text),
      h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' },
        h('a', { class: 'btn btn-primary', href: '#/play', html: icon('play') + `<span>${escapeHtml(t('ui.playABot'))}</span>` }),
        h('a', { class: 'btn btn-ghost', href: '#/', html: icon('home') + `<span>${escapeHtml(t('nav.home'))}</span>` }))));
  root.appendChild(page);
  return () => page.remove();
}
