// GrandMentor — screen-reader announcer (shared ARIA live regions).
// Contract: docs/CONTRACT.md §5 (`announce`, `describeMove`, `squareLabel`).
//
//   announce('White knight to f3')                    // polite, debounced, de-duplicated
//   announce(t('a11y.puzzle.solved'), { assertive: true })
//   announceMove(moveObj)                             // natural-language move, if the user wants it
//
// Two visually hidden live regions (polite + assertive) live at the end of <body>. Messages that
// arrive within a short window are joined into one announcement so a burst (a move, then the
// opponent's reply, then "check") is read as one sentence instead of interrupting itself.
// The same text is not repeated within DEDUPE_MS. Everything is bounded (queue ≤ MAX_QUEUE).

import { t } from '../i18n.js';
import { getSetting } from '../settings.js';

const DEBOUNCE_MS = 150;
const DEDUPE_MS = 1200;
const MAX_QUEUE = 6;
const MAX_LEN = 400;

/** @type {{polite: HTMLElement|null, assertive: HTMLElement|null}} */
const regions = { polite: null, assertive: null };
const queues = { polite: [], assertive: [] };
const timers = { polite: 0, assertive: 0 };
const recent = new Map(); // text -> timestamp (bounded by pruning)

function region(kind) {
  if (typeof document === 'undefined') return null;
  let el = regions[kind];
  if (el && el.isConnected) return el;
  el = document.getElementById(`gm-announcer-${kind}`);
  if (!el) {
    el = document.createElement('div');
    el.id = `gm-announcer-${kind}`;
    el.className = 'sr-only gm-announcer';
    el.setAttribute('aria-live', kind);
    el.setAttribute('aria-atomic', 'true');
    el.setAttribute('role', kind === 'assertive' ? 'alert' : 'status');
    (document.body || document.documentElement).appendChild(el);
  }
  regions[kind] = el;
  return el;
}

function flush(kind) {
  timers[kind] = 0;
  const q = queues[kind];
  if (!q.length) return;
  const text = q.join(' ').slice(0, MAX_LEN);
  q.length = 0;
  const el = region(kind);
  if (!el) return;
  // Clear first so an identical message (after the de-dupe window) is read again.
  el.textContent = '';
  setTimeout(() => { el.textContent = text; }, 30);
}

/**
 * Announce a message to screen readers.
 * @param {string} message plain text (already translated)
 * @param {{assertive?: boolean, dedupe?: boolean}} [opts]
 *   assertive: interrupt (errors, results). dedupe: drop a repeat of the same text within 1.2 s (default true).
 */
export function announce(message, { assertive = false, dedupe = true } = {}) {
  let text = String(message ?? '').replace(/\s+/g, ' ').trim();
  if (!text) return;
  // Messages built from templates may start with a lower-case piece name ("white pawn on e2 …").
  text = text[0].toLocaleUpperCase() + text.slice(1);
  const now = Date.now();
  if (dedupe) {
    const last = recent.get(text);
    if (last && now - last < DEDUPE_MS) return;
  }
  recent.set(text, now);
  if (recent.size > 32) {
    for (const [k, ts] of recent) if (now - ts > DEDUPE_MS || recent.size > 32) recent.delete(k);
  }
  const kind = assertive ? 'assertive' : 'polite';
  const q = queues[kind];
  if (!q.includes(text)) q.push(/[.!?…]$/.test(text) ? text : `${text}.`);
  if (q.length > MAX_QUEUE) q.splice(0, q.length - MAX_QUEUE);
  if (timers[kind]) clearTimeout(timers[kind]);
  timers[kind] = setTimeout(() => flush(kind), DEBOUNCE_MS);
}

/** Drop pending (not yet spoken) messages, e.g. when leaving a page. */
export function clearAnnouncements() {
  for (const kind of ['polite', 'assertive']) {
    if (timers[kind]) clearTimeout(timers[kind]);
    timers[kind] = 0;
    queues[kind].length = 0;
  }
}

// ---------------------------------------------------------------------------
// Chess vocabulary
// ---------------------------------------------------------------------------
const ROLES = { p: 'pawn', n: 'knight', b: 'bishop', r: 'rook', q: 'queen', k: 'king' };

/** Localized piece name without colour: pieceName('n') → "knight". */
export function pieceName(role) {
  const r = ROLES[String(role || '').toLowerCase()];
  return r ? t(`a11y.pieces.${r}`) : '';
}

/** Localized piece with colour: coloredPiece('wN') → "white knight" ("caballo blanco"). */
export function coloredPiece(code) {
  const c = String(code || '');
  if (!/^[wb][KQRBNP]$/.test(c)) return '';
  return t(`a11y.coloredPieces.${c}`);
}

/** Square label for the board grid: "e4, white knight" / "e4, empty". */
export function squareLabel(square, code) {
  return code ? t('a11y.square.piece', { square, piece: coloredPiece(code) }) : t('a11y.square.empty', { square });
}

/**
 * Natural-language description of a move ("White knight to f3", "Black captures on d5, check").
 * @param {{from:string,to:string,san?:string,color?:string,piece?:string,captured?:string,promotion?:string,flags?:string}} mv
 *   a Board move object or a verbose chess.js move.
 * @returns {string}
 */
export function describeMove(mv) {
  if (!mv || !mv.to) return '';
  const side = mv.color === 'b' ? 'b' : 'w';
  const flags = String(mv.flags || '');
  const san = String(mv.san || '');
  const params = { piece: pieceName(mv.piece || 'p'), square: mv.to, from: mv.from, captured: pieceName(mv.captured), promo: pieceName(mv.promotion) };
  let base;
  if (flags.includes('k') || san.startsWith('O-O') && !san.startsWith('O-O-O')) base = t(`a11y.moves.castleShort.${side}`);
  else if (flags.includes('q') || san.startsWith('O-O-O')) base = t(`a11y.moves.castleLong.${side}`);
  else if (mv.promotion && mv.captured) base = t(`a11y.moves.capturePromote.${side}`, params);
  else if (mv.promotion) base = t(`a11y.moves.promote.${side}`, params);
  else if (flags.includes('e')) base = t(`a11y.moves.enPassant.${side}`, params);
  else if (mv.captured) base = t(`a11y.moves.capture.${side}`, params);
  else base = t(`a11y.moves.move.${side}`, params);
  if (san.endsWith('#')) return t('a11y.moves.withMate', { move: base });
  if (san.endsWith('+')) return t('a11y.moves.withCheck', { move: base });
  return base;
}

/** Announce a move in natural language when the "Announce moves" setting is on. */
export function announceMove(mv, opts) {
  let on = true;
  try { on = getSetting('announceMoves') !== false; } catch { on = true; }
  if (!on) return;
  const text = describeMove(mv);
  if (text) announce(text, opts);
}

/** Announce whose turn it is ("White to move" / "Your move"). */
export function announceTurn(color, { you = false } = {}) {
  let on = true;
  try { on = getSetting('announceMoves') !== false; } catch { on = true; }
  if (!on) return;
  announce(you ? t('a11y.turn.you') : t(color === 'b' || color === 'black' ? 'a11y.turn.black' : 'a11y.turn.white'));
}

export default announce;
