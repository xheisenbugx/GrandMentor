// GrandMentor — Play vs Bots (#/play, #/play/:botId).
// Setup screen (bot picker + options) and game screen (board, eval bar, clocks, coach, hints,
// takebacks, draw offers, game-over modal, auto-save, resume from localStorage).
// Contract: docs/CONTRACT.md §4–§5, UX: docs/FEATURES.md §3.3–§3.4, classes: docs/STYLEGUIDE.md.
//
// Leak policy: every view (setup / game) owns a disposables() bag; switching view or unmounting
// disposes it (components destroyed, timers cleared, fetches aborted, sockets closed).

import { Chess } from '/vendor/chess.js';
import { api, qs, isAbort, EngineClient } from '../api.js';
import {
  h, icon, toast, modal, confirmDialog, disposables, loadingBlock, emptyState,
  classificationMeta, mdLite, escapeHtml,
} from '../ui.js';
import { DEFAULT_AVATAR } from '../ui.js';
import { getSettings } from '../settings.js';
import { Board } from '../components/board.js';
import { parseMoveText, looksLikeMove } from '../components/moveinput.js';
import { announce } from '../components/announcer.js';
import { EvalBar } from '../components/evalbar.js';
import { MoveList } from '../components/movelist.js';
import { ChessClock } from '../components/clock.js';
import { t, hasKey } from '../i18n.js';
import { parseLine, numberedSans, MAX_LINE_PLIES } from '../components/practice.js';

export const title = () => t('play.title');

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
const PREFS_KEY = 'grandmentor.play.prefs.v1';
const SAVE_KEY = 'grandmentor.play.current.v1';
const MAX_THINK_MS = 6000;
const BOT_CHAT_MS = 4500;
const MAX_EVALS = 600;            // bounded eval cache (positions per game)
const DRAW_OFFER_COOLDOWN_PLIES = 10;

// Labels are resolved at render time with t() (see tcLabel / tcSpeed / docs/I18N.md).
const TIME_CONTROLS = [
  { id: 'none', speed: 'relaxed', initial: 0, inc: 0 },
  { id: '1+0', speed: 'bullet', initial: 60e3, inc: 0 },
  { id: '3+2', speed: 'blitz', initial: 180e3, inc: 2e3 },
  { id: '5+0', speed: 'blitz', initial: 300e3, inc: 0 },
  { id: '10+0', speed: 'rapid', initial: 600e3, inc: 0 },
  { id: '15+10', speed: 'rapid', initial: 900e3, inc: 10e3 },
  { id: '30+0', speed: 'classical', initial: 1800e3, inc: 0 },
];
const TC_BY_ID = Object.fromEntries(TIME_CONTROLS.map((tc) => [tc.id, tc]));

/** "No clock", "5 min" or "3 | 2". */
function tcLabel(tc) {
  if (tc.id === 'none') return t('play.tc.none');
  const minutes = Math.round(tc.initial / 60e3);
  return tc.inc ? `${minutes} | ${Math.round(tc.inc / 1e3)}` : t('play.tc.minutes', { count: minutes });
}
const tcSpeed = (tc) => t(`play.tc.speed.${tc.speed}`);

const CATEGORIES = ['coach', 'beginner', 'intermediate', 'advanced', 'master'];
const categoryLabel = (id) => (hasKey(`play.categories.${id}.label`) ? t(`play.categories.${id}.label`) : String(id || ''));
const styleLabel = (id) => (hasKey(`play.styles.${id}`) ? t(`play.styles.${id}`) : String(id || ''));

const MODES = {
  challenge: { opts: { hints: false, takebacks: false, evalBar: false, coach: false } },
  friendly: { opts: { hints: true, takebacks: true, evalBar: true, coach: true } },
  custom: { opts: null },
};

const OPTION_DEFS = [
  { key: 'hints', ic: 'hint' },
  { key: 'takebacks', ic: 'undo' },
  { key: 'evalBar', ic: 'chart' },
  { key: 'coach', ic: 'mentor' },
];
// Board & move options (independent of the help level above).
const EXTRA_DEFS = [
  { key: 'premoves', ic: 'bolt', def: true },
  { key: 'confirmMove', ic: 'check-circle', def: false },
  { key: 'typeMoves', ic: 'keyboard', def: false },
  { key: 'blindfold', ic: 'eye-off', def: false },
];
const DEFAULT_EXTRAS = Object.fromEntries(EXTRA_DEFS.map((d) => [d.key, d.def]));
const ADAPTIVE_ID = 'adaptive';
const PIECE_VALUES = { p: 1, n: 3, b: 3, r: 5, q: 9, k: 0 };
const START_COUNTS = { p: 8, n: 2, b: 2, r: 2, q: 1 };
const GLYPHS = { w: { p: '♙', n: '♘', b: '♗', r: '♖', q: '♕' }, b: { p: '♟', n: '♞', b: '♝', r: '♜', q: '♛' } };
const NOTABLE_CLS = new Set(['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder', 'forced']);
const GOOD_CLS = new Set(['brilliant', 'great', 'best', 'excellent', 'book', 'forced']);
// Termination ids (sent to the server untranslated) -> message keys under play.termination.
const TERMINATION_KEYS = {
  checkmate: 'checkmate',
  resignation: 'resignation',
  timeout: 'timeout',
  stalemate: 'stalemate',
  'threefold repetition': 'threefold',
  'insufficient material': 'insufficientMaterial',
  'timeout vs insufficient material': 'timeoutVsInsufficient',
  '50-move rule': 'fiftyMove',
  agreement: 'agreement',
  abandoned: 'abandoned',
};
const terminationText = (term) => (TERMINATION_KEYS[term] ? t(`play.termination.${TERMINATION_KEYS[term]}`) : String(term || ''));

const colorName = (c) => (c === 'w' ? 'white' : 'black');
const other = (c) => (c === 'w' ? 'b' : 'w');
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

// ---------------------------------------------------------------------------
// Storage helpers (never throw)
// ---------------------------------------------------------------------------
function loadJson(key) {
  try { const v = localStorage.getItem(key); return v ? JSON.parse(v) : null; } catch { return null; }
}
function saveJson(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage full or blocked */ }
}
function removeKey(key) {
  try { localStorage.removeItem(key); } catch { /* ignore */ }
}

function loadPrefs() {
  const p = loadJson(PREFS_KEY) || {};
  const mode = MODES[p.mode] ? p.mode : 'friendly';
  const s = getSettings();
  const baseOpts = { ...MODES.friendly.opts, evalBar: s.showEvalBar !== false };
  const opts = mode === 'custom' && p.opts && typeof p.opts === 'object'
    ? Object.fromEntries(OPTION_DEFS.map((d) => [d.key, !!p.opts[d.key]]))
    : (MODES[mode].opts ? { ...MODES[mode].opts } : baseOpts);
  return {
    botId: typeof p.botId === 'string' ? p.botId : null,
    color: ['white', 'black', 'random'].includes(p.color) ? p.color : 'random',
    mode,
    opts,
    extras: normalizeExtras(p.extras),
    tc: TC_BY_ID[p.tc] ? p.tc : 'none',
  };
}

function normalizeExtras(x) {
  const src = x && typeof x === 'object' ? x : {};
  return Object.fromEntries(EXTRA_DEFS.map((d) => [d.key, typeof src[d.key] === 'boolean' ? src[d.key] : d.def]));
}

/**
 * Validate a start position from the URL (`#/play?fen=...`). Returns
 * `{ fen, turn }` for a playable position or `{ error }` with a friendly message.
 */
function checkCustomFen(raw) {
  const fen = String(raw || '').trim().replace(/\s+/g, ' ');
  if (!fen) return null;
  if (fen.length > 120) return { error: t('play.custom.invalid') };
  let c;
  try { c = new Chess(fen); } catch { return { error: t('play.custom.invalid') }; }
  if (c.isGameOver()) return { error: t('play.custom.finished') };
  const full = c.fen();
  if (full.split(' ').slice(0, 4).join(' ') === START_FEN.split(' ').slice(0, 4).join(' ')) return null; // just the normal start
  return { fen: full, turn: c.turn() };
}

// Typed moves are parsed by the shared move-input component (SAN or UCI, forgiving about case).
const parseTypedMove = (chess, text) => parseMoveText(chess, text);

function loadSavedGame() {
  const s = loadJson(SAVE_KEY);
  if (!s || s.v !== 1 || !s.bot || !s.bot.id || !Array.isArray(s.moves) || !s.moves.length) return null;
  if (s.userColor !== 'w' && s.userColor !== 'b') return null;
  return s;
}

/**
 * Opening practice from the URL (`#/play?opening=<id>&line=<uci moves>&color=w|b`). Resolves to
 * `{ id, name, moves, sans, book, color, fen }`, `{ error }` (friendly text) or null when absent.
 * `book` is the opening's main line when the played line follows it (used for "left the book").
 */
async function resolvePractice(query, signal) {
  const id = typeof query.opening === 'string' ? query.opening.trim() : '';
  const rawLine = typeof query.line === 'string' ? query.line : '';
  if (!id && !rawLine.trim()) return null;
  let opening = null;
  if (id) {
    if (!/^[A-Za-z0-9_-]{1,80}$/.test(id)) return { error: t('practice.error.notFound') };
    try { opening = await api.get(`/api/openings/${encodeURIComponent(id)}`, { signal, timeout: 10000 }); } catch (e) {
      if (isAbort(e)) throw e;
      return { error: t('practice.error.notFound') };
    }
    if (!opening || typeof opening !== 'object') return { error: t('practice.error.notFound') };
  }
  const main = opening && Array.isArray(opening.uci) ? opening.uci.filter((u) => typeof u === 'string').slice(0, MAX_LINE_PLIES) : [];
  const parsed = parseLine(rawLine.trim() ? rawLine : main.join(' '));
  if (!parsed || parsed.error) {
    const key = parsed && parsed.error === 'tooLong' ? 'practice.error.tooLong' : parsed && parsed.error === 'finished' ? 'practice.error.finished' : 'practice.error.invalid';
    return { error: t(key, { count: MAX_LINE_PLIES }) };
  }
  const n = Math.min(parsed.moves.length, main.length);
  const followsBook = main.length > 0 && parsed.moves.slice(0, n).every((u, i) => u === main[i]);
  let name = opening && typeof opening.name === 'string' ? opening.name : '';
  if (!name) {
    try {
      const m = await api.get('/api/openings/lookup' + qs({ fen: parsed.fen }), { signal, timeout: 8000 });
      if (m && m.opening && typeof m.opening.name === 'string') name = m.opening.name;
    } catch (e) { if (isAbort(e)) throw e; }
  }
  const qc = String(query.color || '').toLowerCase();
  const color = qc === 'w' || qc === 'white' ? 'w' : qc === 'b' || qc === 'black' ? 'b' : (opening && opening.side === 'black' ? 'b' : 'w');
  return {
    id: opening && typeof opening.id === 'string' ? opening.id : null,
    name: name.slice(0, 120),
    moves: parsed.moves, sans: parsed.sans, fen: parsed.fen,
    book: followsBook ? main : parsed.moves.slice(),
    color,
  };
}

/** A practice record restored from storage (resume), or null if it does not look right. */
function sanitizePractice(p, moveCount) {
  if (!p || typeof p !== 'object' || !Array.isArray(p.moves)) return null;
  const moves = p.moves.filter((u) => typeof u === 'string' && /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(u)).slice(0, MAX_LINE_PLIES);
  if (!moves.length || moves.length > moveCount) return null;
  const book = Array.isArray(p.book) ? p.book.filter((u) => typeof u === 'string').slice(0, MAX_LINE_PLIES) : moves.slice();
  return { id: typeof p.id === 'string' ? p.id : null, name: typeof p.name === 'string' ? p.name.slice(0, 120) : '', moves, sans: [], book, color: p.color === 'b' ? 'b' : 'w' };
}

/** Inject /css/play.css once; resolves when loaded (or after a short timeout). */
function ensureCss() {
  const existing = document.querySelector('link[href="/css/play.css"]');
  if (existing) return Promise.resolve();
  return new Promise((resolve) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = '/css/play.css';
    let done = false;
    const finish = () => { if (!done) { done = true; clearTimeout(t); resolve(); } };
    const t = setTimeout(finish, 800);
    link.addEventListener('load', finish, { once: true });
    link.addEventListener('error', finish, { once: true });
    document.head.appendChild(link);
  });
}

function avatarNode(value, cls = '') {
  const v = String(value || DEFAULT_AVATAR);
  const isImg = /^(https?:|\/|data:image)/.test(v);
  return h('div', { class: `avatar ${cls}`.trim(), 'aria-hidden': 'true' }, isImg ? h('img', { src: v, alt: '' }) : v);
}

function botAvatar(bot, size = '') {
  return h('div', { class: `avatar avatar-round bot-avatar cat-${bot.category || 'beginner'} ${size}`.trim(), 'aria-hidden': 'true' }, bot.avatar || '🤖');
}

function parseUci(uci) {
  if (typeof uci !== 'string' || !/^[a-h][1-8][a-h][1-8][qrbn]?$/.test(uci)) return null;
  return { from: uci.slice(0, 2), to: uci.slice(2, 4), promotion: uci[4] || undefined };
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export async function mount(root, { params = {}, query = {} } = {}) {
  const bag = disposables();
  const ac = new AbortController();
  bag.add(() => ac.abort());

  const host = h('div', { class: 'play-root' });
  root.appendChild(host);
  bag.add(() => host.remove());

  let viewDispose = null;
  const setView = (factory) => {
    if (viewDispose) { const d = viewDispose; viewDispose = null; d(); }
    host.replaceChildren();
    if (bag.disposed) return;
    try {
      viewDispose = factory();
    } catch (e) {
      console.error('[play] view failed', e);
      host.replaceChildren(h('div', { class: 'page' }, emptyState({
        icon: 'alert',
        title: t('play.error.title'),
        text: e && e.message ? e.message : t('play.error.gameScreen'),
        action: { label: t('play.error.backToBots'), icon: 'robot', onClick: () => showSetup(null) },
      })));
    }
  };
  bag.add(() => { if (viewDispose) { const d = viewDispose; viewDispose = null; d(); } });

  // Custom start position from the URL: #/play?fen=<FEN>[&color=w|b][&bot=<id>].
  const custom = query.fen ? checkCustomFen(query.fen) : null;
  const ctx = {
    bots: [], profile: null, stats: null, onStats: null, estimate: null, onEstimate: null,
    signal: ac.signal,
    custom: custom && custom.fen ? { fen: custom.fen, color: ['w', 'b'].includes(query.color) ? query.color : custom.turn } : null,
    customError: custom && custom.error ? custom.error : null,
    practice: null,       // opening practice: see resolvePractice()
  };

  await ensureCss();
  if (bag.disposed) return bag.dispose;
  host.appendChild(loadingBlock(t('play.loading')));

  // Non-critical data (name for saved games, recommended bot, estimated rating).
  api.get('/api/profile', { signal: ac.signal }).then((p) => { ctx.profile = p; }).catch(() => {});
  api.get('/api/stats', { signal: ac.signal }).then((s) => { ctx.stats = s; if (ctx.onStats) ctx.onStats(); }).catch(() => {});
  ctx.refreshEstimate = () => api.get('/api/adaptive/estimate', { signal: ac.signal })
    .then((e) => { if (e && typeof e === 'object') { ctx.estimate = e; if (ctx.onEstimate) ctx.onEstimate(); } })
    .catch(() => {});
  ctx.refreshEstimate();

  const showSetup = (preselectId) => setView(() => renderSetup(host, ctx, {
    preselectId,
    onPlay: (cfg) => showGame(cfg),
    onResume: (saved) => showGame({ resume: saved }),
  }));
  const showGame = (cfg) => setView(() => renderGame(host, ctx, cfg, {
    onNewBot: (botId) => showSetup(botId),
    onRematch: (next) => showGame(next),
  }));

  const load = async () => {
    try {
      const bots = await api.get('/api/bots', { signal: ac.signal });
      if (bag.disposed) return;
      ctx.bots = Array.isArray(bots) ? bots.filter((b) => b && typeof b.id === 'string') : [];
      if (!ctx.bots.length) throw new Error(t('play.error.noBots'));
      if (!ctx.practice && (query.opening || query.line)) {
        const pr = await resolvePractice(query, ac.signal);
        if (bag.disposed) return;
        if (pr && pr.error) ctx.customError = pr.error;
        else if (pr) { ctx.practice = pr; ctx.custom = null; ctx.customError = null; }
      }
      showSetup(params.botId || (typeof query.bot === 'string' && query.bot) || null);
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      setView(() => {
        host.appendChild(h('div', { class: 'page' }, emptyState({
          icon: 'robot',
          title: t('play.error.botsDown'),
          text: t('play.error.botsDownText', { message: e.message || t('play.error.generic') }),
          action: { label: t('play.error.tryAgain'), icon: 'refresh', onClick: () => { setView(() => { host.appendChild(loadingBlock(t('play.loading'))); return null; }); load(); } },
        })));
        return null;
      });
    }
  };
  await load();
  return bag.dispose;
}

// ---------------------------------------------------------------------------
// Setup screen
// ---------------------------------------------------------------------------
function renderSetup(host, ctx, { preselectId, onPlay, onResume }) {
  const bag = disposables();
  const prefs = loadPrefs();
  const bots = ctx.bots;
  const byId = new Map(bots.map((b) => [b.id, b]));
  let saved = loadSavedGame();

  const recommendedId = () => {
    const per = new Map(((ctx.stats && ctx.stats.per_bot) || []).map((r) => [r.bot_id, r]));
    const ladder = bots.filter((b) => b.category !== 'coach' && b.id !== ADAPTIVE_ID).sort((a, b) => (a.elo || 0) - (b.elo || 0));
    const pick = ladder.find((b) => !(per.get(b.id)?.won > 0));
    return (pick || ladder[ladder.length - 1] || bots[0]).id;
  };

  // Opening practice suggests the adaptive bot (it plays at your level) unless a bot was picked.
  const practiceBot = ctx.practice ? (byId.has(ADAPTIVE_ID) ? ADAPTIVE_ID : recommendedId()) : null;
  const state = {
    botId: (preselectId && byId.has(preselectId) && preselectId) || practiceBot || (prefs.botId && byId.has(prefs.botId) && prefs.botId) || recommendedId(),
    color: ctx.practice ? (ctx.practice.color === 'b' ? 'black' : 'white') : ctx.custom ? (ctx.custom.color === 'b' ? 'black' : 'white') : prefs.color,
    mode: prefs.mode,
    opts: { ...prefs.opts },
    extras: { ...prefs.extras },
    tc: prefs.tc,
  };
  if (preselectId && !byId.has(preselectId)) toast(t('play.setup.botNotFound'), 'warning');
  if (ctx.customError) {
    toast(ctx.customError, 'warning', { duration: 5000 });
    ctx.customError = null;
  }
  const startFen = () => (ctx.custom ? ctx.custom.fen : START_FEN);
  const previewFen = () => (ctx.practice ? ctx.practice.fen : startFen());

  // ---- Left: board preview with the selected bot -------------------------
  const previewBubble = h('div', { class: 'bubble bubble-bot pop-in' });
  const previewName = h('div', { class: 'play-preview-name' });
  const previewAvatarSlot = h('div', { class: 'play-preview-avatar' });
  const previewBoardEl = h('div', { class: 'play-preview-board' });
  const preview = h('section', { class: 'play-preview', 'aria-hidden': 'true' },
    h('div', { class: 'play-preview-bot' }, previewAvatarSlot, h('div', { class: 'stack-sm', style: 'min-width:0' }, previewName, previewBubble)),
    previewBoardEl);

  let previewBoard = null;
  try {
    const s = getSettings();
    previewBoard = new Board(previewBoardEl, { fen: previewFen(), orientation: state.color === 'black' ? 'black' : 'white', interactive: false, movableColor: null, showCoords: s.showCoords, sounds: false, animationMs: s.animationMs, keyboard: false, announce: false });
    bag.add(() => previewBoard.destroy());
  } catch (e) { console.warn('[play] preview board unavailable', e); }

  // ---- Right: panel ------------------------------------------------------
  const resumeSlot = h('div');
  const customSlot = h('div');
  const practiceSlot = h('div');
  const heroSlot = h('div', { class: 'play-hero' });
  const groups = h('div', { class: 'bot-groups' });
  const tileById = new Map();
  const estimateEl = h('div', { class: 'play-estimate', hidden: true });

  // The adaptive bot gets its own featured card above the categories.
  const adaptiveBot = byId.get(ADAPTIVE_ID) || null;
  const adaptiveLevel = h('span', { class: 'adaptive-level' });
  if (adaptiveBot) {
    const card = h('button', {
      type: 'button', class: 'bot-tile adaptive-card', role: 'option', 'aria-selected': 'false', dataset: { botId: adaptiveBot.id },
      onClick: () => select(adaptiveBot.id), onDblclick: () => { select(adaptiveBot.id); play(); },
    },
    botAvatar({ ...adaptiveBot, category: 'adaptive' }),
    h('span', { class: 'adaptive-text' },
      h('span', { class: 'adaptive-title' }, h('span', { class: 'semibold' }, adaptiveBot.name), h('span', { class: 'badge badge-gold' }, t('play.adaptive.badge'))),
      h('span', { class: 'subtle text-xs' }, t('play.adaptive.tagline')),
      adaptiveLevel));
    tileById.set(adaptiveBot.id, card);
    groups.appendChild(h('section', { class: 'bot-group' }, h('div', { role: 'listbox', 'aria-label': t('play.adaptive.groupAria') }, card)));
  }

  for (const catId of [...CATEGORIES, '__other']) {
    const isOther = catId === '__other';
    const cat = {
      id: catId,
      label: isOther ? t('play.categories.more') : categoryLabel(catId),
      blurb: isOther ? '' : t(`play.categories.${catId}.blurb`),
    };
    const list = bots.filter((b) => b.id !== ADAPTIVE_ID)
      .filter((b) => (isOther ? !CATEGORIES.includes(b.category) : b.category === cat.id))
      .sort((a, b) => (a.elo || 0) - (b.elo || 0));
    if (!list.length) continue;
    const grid = h('div', { class: 'bot-grid', role: 'listbox', 'aria-label': t('play.categories.groupAria', { category: cat.label }) });
    for (const b of list) {
      const tile = h('button', {
        type: 'button', class: 'bot-tile', role: 'option', 'aria-selected': 'false', dataset: { botId: b.id },
        title: `${b.name} (${b.elo})`, onClick: () => select(b.id), onDblclick: () => { select(b.id); play(); },
      }, botAvatar(b), h('span', { class: 'bot-tile-name truncate' }, b.name), h('span', { class: 'bot-tile-elo' }, String(b.elo ?? '')));
      tileById.set(b.id, tile);
      grid.appendChild(tile);
    }
    groups.appendChild(h('section', { class: 'bot-group' },
      h('div', { class: 'bot-group-head' }, h('span', { class: `badge level-${cat.id === 'coach' ? 'beginner' : cat.id}` }, cat.label), cat.blurb ? h('span', { class: 'subtle text-xs' }, cat.blurb) : null),
      grid));
  }

  // Colour chooser
  const colorBtns = {};
  const colorRow = h('div', { class: 'color-choice', role: 'radiogroup', 'aria-label': t('play.setup.playAsAria') },
    ...[['white', t('play.color.white'), h('span', { class: 'color-swatch white' }, '♚')], ['random', t('play.color.random'), h('span', { class: 'color-swatch random' }, '?')], ['black', t('play.color.black'), h('span', { class: 'color-swatch black' }, '♚')]]
      .map(([id, label, sw]) => (colorBtns[id] = h('button', { type: 'button', class: 'color-btn', role: 'radio', 'aria-checked': 'false', onClick: () => { state.color = id; refreshOptions(); } }, sw, h('span', null, label)))));

  // Mode + toggles
  const modeBtns = {};
  const modeSeg = h('div', { class: 'segmented block', role: 'radiogroup', 'aria-label': t('play.setup.helpLevelAria') },
    ...Object.keys(MODES).map((id) => (modeBtns[id] = h('button', { type: 'button', role: 'radio', onClick: () => setMode(id) }, t(`play.modes.${id}.label`)))));
  const modeDesc = h('p', { class: 'muted text-sm' });
  const optInputs = {};
  const optionList = h('div', { class: 'play-options' }, ...OPTION_DEFS.map((d) => {
    const input = h('input', { type: 'checkbox', onChange: () => { state.opts[d.key] = input.checked; state.mode = matchMode(state.opts); refreshOptions(); } });
    optInputs[d.key] = input;
    return h('label', { class: 'play-option' },
      h('span', { class: 'play-option-icon', html: icon(d.ic) }),
      h('span', { class: 'play-option-text' }, h('span', { class: 'semibold' }, t(`play.options.${d.key}.title`)), h('span', { class: 'subtle text-xs' }, t(`play.options.${d.key}.desc`))),
      h('span', { class: 'switch' }, input, h('span', { class: 'switch-track' })));
  }));

  // Board & move options
  const extraInputs = {};
  const extraList = h('div', { class: 'play-options' }, ...EXTRA_DEFS.map((d) => {
    const input = h('input', { type: 'checkbox', onChange: () => { state.extras[d.key] = input.checked; } });
    extraInputs[d.key] = input;
    return h('label', { class: 'play-option' },
      h('span', { class: 'play-option-icon', html: icon(d.ic) }),
      h('span', { class: 'play-option-text' }, h('span', { class: 'semibold' }, t(`play.extras.${d.key}.title`)), h('span', { class: 'subtle text-xs' }, t(`play.extras.${d.key}.desc`))),
      h('span', { class: 'switch' }, input, h('span', { class: 'switch-track' })));
  }));

  // Time control
  const tcBtns = {};
  const tcRow = h('div', { class: 'tc-grid', role: 'radiogroup', 'aria-label': t('play.setup.timeControlAria') },
    ...TIME_CONTROLS.map((tc) => (tcBtns[tc.id] = h('button', { type: 'button', class: 'tc-btn', role: 'radio', onClick: () => { state.tc = tc.id; refreshOptions(); } },
      h('span', { class: 'tc-label' }, tcLabel(tc)), h('span', { class: 'tc-sub' }, tcSpeed(tc))))));

  const playBtn = h('button', { type: 'button', class: 'btn btn-primary btn-xl btn-block play-cta', onClick: () => play() });

  const panel = h('aside', { class: 'play-setup-panel card card-flush' },
    h('div', { class: 'play-setup-head' }, h('div', { class: 'page-header-icon', html: icon('robot') }),
      h('div', { style: 'min-width:0' }, h('h1', { class: 'page-title' }, t('play.title')), h('p', { class: 'page-subtitle' }, t('play.setup.subtitle')), estimateEl)),
    h('div', { class: 'play-setup-scroll' },
      resumeSlot,
      customSlot,
      practiceSlot,
      heroSlot,
      h('h2', { class: 'play-section-title' }, t('play.setup.chooseOpponent')),
      groups,
      h('h2', { class: 'play-section-title' }, t('play.setup.playAs')),
      colorRow,
      h('h2', { class: 'play-section-title' }, t('play.setup.help')),
      modeSeg, modeDesc, optionList,
      h('h2', { class: 'play-section-title' }, t('play.setup.boardOptions')),
      extraList,
      h('h2', { class: 'play-section-title' }, t('play.setup.time')),
      tcRow,
      h('a', { class: 'play-local-link', href: '#/local' },
        h('span', { class: 'play-option-icon', html: icon('users') }),
        h('span', { class: 'play-option-text' }, h('span', { class: 'semibold' }, t('play.setup.localTitle')), h('span', { class: 'subtle text-xs' }, t('play.setup.localDesc'))),
        h('span', { class: 'play-local-chevron', html: icon('chevron-right') }))),
    h('div', { class: 'play-setup-foot' }, playBtn));

  const layout = h('div', { class: 'play-setup page-enter' }, preview, panel);
  host.appendChild(layout);

  function matchMode(opts) {
    for (const [id, m] of Object.entries(MODES)) {
      if (m.opts && OPTION_DEFS.every((d) => !!m.opts[d.key] === !!opts[d.key])) return id;
    }
    return 'custom';
  }
  function setMode(id) {
    state.mode = id;
    if (MODES[id].opts) state.opts = { ...MODES[id].opts };
    refreshOptions();
  }

  function renderResume() {
    resumeSlot.replaceChildren();
    if (!saved) return;
    const b = saved.bot;
    const moveCount = Math.ceil(saved.moves.length / 2);
    resumeSlot.appendChild(h('div', { class: 'resume-card' },
      botAvatar(b, 'avatar-sm'),
      h('div', { class: 'resume-text' }, h('div', { class: 'semibold' }, t('play.resume.title', { name: b.name })),
        h('div', { class: 'subtle text-xs' }, t(saved.userColor === 'w' ? 'play.resume.detailWhite' : 'play.resume.detailBlack', { count: moveCount }))),
      h('button', { type: 'button', class: 'btn btn-primary btn-sm', html: icon('play') + `<span>${escapeHtml(t('play.resume.resume'))}</span>`, onClick: () => onResume(saved) }),
      h('button', {
        type: 'button', class: 'btn btn-ghost btn-icon btn-sm', 'aria-label': t('play.resume.discardAria'), 'data-tooltip': t('play.resume.discard'), html: icon('trash'),
        onClick: async () => {
          const ok = await confirmDialog({ title: t('play.resume.discardTitle'), message: t('play.resume.discardMessage'), confirmLabel: t('play.resume.discard'), danger: true });
          if (!ok || bag.disposed) return;
          removeKey(SAVE_KEY);
          saved = null;
          renderResume();
        },
      })));
  }

  function renderCustom() {
    customSlot.replaceChildren();
    if (!ctx.custom) return;
    customSlot.appendChild(h('div', { class: 'custom-card' },
      h('span', { class: 'play-option-icon', html: icon('board') }),
      h('div', { class: 'resume-text' },
        h('div', { class: 'semibold' }, t('play.custom.title')),
        h('div', { class: 'subtle text-xs' }, t(ctx.custom.fen.split(' ')[1] === 'b' ? 'play.custom.detailBlack' : 'play.custom.detailWhite'))),
      h('button', {
        type: 'button', class: 'btn btn-ghost btn-sm', html: icon('refresh', { size: 14 }) + `<span>${escapeHtml(t('play.custom.useStandard'))}</span>`,
        onClick: () => {
          ctx.custom = null;
          // Drop the position from the URL too, without remounting the page.
          try { history.replaceState(null, '', '#/play'); } catch { /* ignore */ }
          if (previewBoard) { try { previewBoard.setPosition(START_FEN, { animate: true }); } catch { /* ignore */ } }
          renderCustom();
        },
      })));
  }

  function renderPractice() {
    practiceSlot.replaceChildren();
    const pr = ctx.practice;
    if (!pr) return;
    practiceSlot.appendChild(h('div', { class: 'custom-card practice-card' },
      h('span', { class: 'play-option-icon', html: icon('openings') }),
      h('div', { class: 'resume-text' },
        h('div', { class: 'semibold' }, t('practice.setup.title', { name: pr.name || t('practice.yourLine') })),
        h('div', { class: 'subtle text-xs practice-line' }, numberedSans(pr.sans)),
        h('div', { class: 'subtle text-xs' }, t('practice.setup.detail'))),
      h('button', {
        type: 'button', class: 'btn btn-ghost btn-sm', html: icon('refresh', { size: 14 }) + `<span>${escapeHtml(t('play.custom.useStandard'))}</span>`,
        onClick: () => {
          ctx.practice = null;
          try { history.replaceState(null, '', '#/play'); } catch { /* ignore */ }
          if (previewBoard) { try { previewBoard.setPosition(START_FEN, { animate: true }); } catch { /* ignore */ } }
          renderPractice();
        },
      })));
  }

  function renderEstimate() {
    const e = ctx.estimate;
    if (adaptiveBot) {
      const lvl = e && Number.isFinite(e.bot_level) ? e.bot_level : adaptiveBot.elo;
      adaptiveBot.elo = lvl;
      adaptiveLevel.textContent = t('play.adaptive.level', { level: lvl });
    }
    if (!e || e.rating == null) {
      estimateEl.hidden = false;
      estimateEl.replaceChildren(h('span', { html: icon('chart', { size: 14 }) }), h('span', null, t('play.estimate.none')));
      return;
    }
    estimateEl.hidden = false;
    // Native replaceChildren() would print a null child as the text "null": drop it first.
    estimateEl.replaceChildren(...[
      h('span', { html: icon('chart', { size: 14 }) }),
      h('span', null, t('play.estimate.label'), ' ', h('strong', null, `~${e.rating}`)),
      e.provisional ? h('span', { class: 'badge', title: t('play.estimate.provisionalTitle', { count: Math.max(0, 5 - (e.games | 0)) }) }, t('play.estimate.provisional')) : null,
    ].filter(Boolean));
  }

  function renderHero() {
    const b = byId.get(state.botId);
    if (!b) return;
    const rec = b.id === recommendedId();
    const adaptive = b.id === ADAPTIVE_ID;
    const look = adaptive ? { ...b, category: 'adaptive' } : b;
    const eloText = adaptive ? `~${b.elo ?? ''}` : `${b.elo ?? ''}`;
    heroSlot.replaceChildren(
      botAvatar(look, 'avatar-xl'),
      h('div', { class: 'play-hero-main' },
        h('div', { class: 'play-hero-name' }, b.name, h('span', { class: 'play-hero-elo' }, eloText)),
        h('div', { class: 'row-sm row-wrap' },
          adaptive ? h('span', { class: 'badge badge-gold no-cap', html: icon('bolt', { size: 12 }) + ' ' + escapeHtml(t('play.adaptive.badge')) }) : null,
          !adaptive && b.style && String(b.style).toLowerCase() !== String(b.category || '').toLowerCase() ? h('span', { class: 'badge badge-info' }, styleLabel(b.style)) : null,
          !adaptive && b.category ? h('span', { class: `badge level-${b.category === 'coach' ? 'beginner' : b.category}` }, categoryLabel(b.category)) : null,
          rec ? h('span', { class: 'badge badge-gold', html: icon('star-filled', { size: 12 }) + ' ' + escapeHtml(t('play.setup.recommended')) }) : null),
        b.description ? h('p', { class: 'muted text-sm play-hero-desc' }, b.description) : null,
        b.greeting ? h('div', { class: 'bubble bubble-bot play-mobile-only text-sm' }, b.greeting) : null));
    previewAvatarSlot.replaceChildren(botAvatar(look, 'avatar-xl'));
    previewName.replaceChildren(h('span', { class: 'semibold' }, b.name), ' ', h('span', { class: 'subtle' }, `(${adaptive ? eloText : (b.elo ?? '?')})`));
    previewBubble.textContent = b.greeting || t('play.setup.defaultGreeting', { name: b.name });
    previewBubble.classList.remove('pop-in');
    void previewBubble.offsetWidth; // restart the pop animation
    previewBubble.classList.add('pop-in');
    playBtn.innerHTML = icon('play') + `<span>${escapeHtml(t('play.setup.playBot', { name: b.name }))}</span>`;
  }

  function refreshTiles() {
    const rec = recommendedId();
    for (const [id, tile] of tileById) {
      const sel = id === state.botId;
      tile.classList.toggle('selected', sel);
      tile.setAttribute('aria-selected', sel ? 'true' : 'false');
      tile.classList.toggle('recommended', id === rec);
    }
  }

  function refreshOptions() {
    for (const [id, btn] of Object.entries(colorBtns)) {
      btn.classList.toggle('active', state.color === id);
      btn.setAttribute('aria-checked', state.color === id ? 'true' : 'false');
    }
    for (const [id, btn] of Object.entries(modeBtns)) {
      btn.classList.toggle('active', state.mode === id);
      btn.setAttribute('aria-checked', state.mode === id ? 'true' : 'false');
    }
    modeDesc.textContent = t(`play.modes.${state.mode}.desc`);
    for (const d of OPTION_DEFS) optInputs[d.key].checked = !!state.opts[d.key];
    for (const d of EXTRA_DEFS) extraInputs[d.key].checked = !!state.extras[d.key];
    for (const [id, btn] of Object.entries(tcBtns)) {
      btn.classList.toggle('active', state.tc === id);
      btn.setAttribute('aria-checked', state.tc === id ? 'true' : 'false');
    }
    if (previewBoard) {
      try { previewBoard.setOrientation(state.color === 'black' ? 'black' : 'white'); } catch { /* ignore */ }
    }
  }

  function select(id) {
    if (!byId.has(id)) return;
    state.botId = id;
    renderHero();
    refreshTiles();
  }

  function play() {
    const bot = byId.get(state.botId);
    if (!bot) return;
    const proceed = () => {
      // A colour picked for a one-off custom position should not become the default.
      const color = ctx.custom || ctx.practice ? prefs.color : state.color;
      saveJson(PREFS_KEY, { botId: state.botId, color, mode: state.mode, opts: state.opts, extras: state.extras, tc: state.tc });
      const userColor = state.color === 'white' ? 'w' : state.color === 'black' ? 'b' : (Math.random() < 0.5 ? 'w' : 'b');
      onPlay({ bot, userColor, colorChoice: state.color, opts: { ...state.opts }, extras: { ...state.extras }, tcId: state.tc, startFen: startFen(), practice: ctx.practice || null });
    };
    if (saved) {
      confirmDialog({ title: t('play.newGame.title'), message: t('play.newGame.discardUnfinished', { name: saved.bot.name }), confirmLabel: t('play.newGame.start') })
        .then((ok) => { if (ok && !bag.disposed) { removeKey(SAVE_KEY); proceed(); } });
      return;
    }
    proceed();
  }

  ctx.onStats = () => { if (!bag.disposed) { renderHero(); refreshTiles(); } };
  ctx.onEstimate = () => { if (!bag.disposed) { renderEstimate(); renderHero(); } };
  bag.add(() => { ctx.onStats = null; ctx.onEstimate = null; });

  renderResume();
  renderCustom();
  renderPractice();
  renderEstimate();
  renderHero();
  refreshTiles();
  refreshOptions();
  // Bring the selected tile into view inside the scroll area.
  // (Not for a custom position: its card at the top must stay visible.)
  if (!ctx.custom && !ctx.practice) bag.raf(() => { const t = tileById.get(state.botId); if (t && t.scrollIntoView) t.scrollIntoView({ block: 'nearest' }); });

  bag.on(window, 'keydown', (e) => {
    if (e.key === 'Enter' && !e.defaultPrevented && !isTyping(e) && !document.querySelector('.modal-backdrop') && document.activeElement === document.body) play();
  });

  return bag.dispose;
}

function isTyping(e) {
  const t = e.target;
  if (!t || !(t instanceof Element)) return false;
  return t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName);
}

// ---------------------------------------------------------------------------
// Game screen
// ---------------------------------------------------------------------------
function renderGame(host, ctx, cfg, cbs) {
  const bag = disposables();
  try {
    buildGame(bag, host, ctx, cfg, cbs);
  } catch (e) {
    bag.dispose();
    throw e;
  }
  return bag.dispose;
}

function buildGame(bag, host, ctx, cfg, { onNewBot, onRematch }) {
  const gac = new AbortController();
  bag.add(() => gac.abort());

  // ---- Game state --------------------------------------------------------
  const resume = cfg.resume || null;
  const g = {
    bot: resume ? resume.bot : cfg.bot,
    userColor: resume ? resume.userColor : cfg.userColor,
    colorChoice: resume ? (resume.colorChoice || 'random') : (cfg.colorChoice || 'random'),
    startFen: (resume ? (typeof resume.startFen === 'string' && resume.startFen) : (typeof cfg.startFen === 'string' && cfg.startFen)) || START_FEN,
    opts: { hints: true, takebacks: true, evalBar: true, coach: false, ...((resume ? resume.opts : cfg.opts) || {}) },
    extras: normalizeExtras(resume ? resume.extras : cfg.extras),
    tc: TC_BY_ID[resume ? resume.tcId : cfg.tcId] || TC_BY_ID.none,
    moves: [], sans: [], fens: [], lastMoves: [], cls: [],
    result: null, termination: '',
    hintsUsed: resume ? (resume.hintsUsed | 0) : 0,
    takebacksUsed: resume ? (resume.takebacksUsed | 0) : 0,
    openingName: resume ? (resume.openingName || null) : null,
    drawOfferPly: resume ? (resume.drawOfferPly ?? -99) : -99,
    savedId: null,
    savePromise: null,
    ratingUpdate: null,   // response of POST /api/adaptive/result
    // Opening practice: { id, name, moves (pre-played UCI), book (main line UCI), color }.
    practice: resume ? sanitizePractice(resume.practice, Array.isArray(resume.moves) ? resume.moves.length : 0)
      : (cfg.practice && Array.isArray(cfg.practice.moves) && cfg.practice.moves.length ? cfg.practice : null),
    prefill: 0,           // plies that were on the board before the game began
    outOfBookPly: 0,      // ply at which the opening book ran out (0 = not yet)
  };
  // A practice game is a normal game from the standard start.
  if (g.practice) { g.startFen = START_FEN; g.prefill = g.practice.moves.length; }
  // Prefer the freshest bot profile from the server if we have it.
  const fresh = ctx.bots.find((b) => b.id === g.bot.id);
  if (fresh) g.bot = fresh;
  const bot = g.bot;
  const userC = g.userColor;
  const botC = other(userC);

  let chess;
  try { chess = new Chess(g.startFen); } catch { chess = new Chess(START_FEN); g.startFen = START_FEN; }
  const customStart = g.startFen.split(' ').slice(0, 4).join(' ') !== START_FEN.split(' ').slice(0, 4).join(' ');
  g.fens.push(chess.fen());
  g.lastMoves.push(null);
  if (resume) {
    for (let i = 0; i < resume.moves.length; i++) {
      const p = parseUci(resume.moves[i]);
      let mv = null;
      if (p) { try { mv = chess.move(p); } catch { mv = null; } }
      if (!mv) break;
      g.moves.push(resume.moves[i]); g.sans.push(mv.san); g.fens.push(chess.fen()); g.lastMoves.push([mv.from, mv.to]);
      g.cls[i + 1] = Array.isArray(resume.cls) ? resume.cls[i + 1] || null : null;
    }
    if (g.practice && g.moves.length < g.prefill) { g.practice = null; g.prefill = 0; }
  } else if (g.practice) {
    for (const uci of g.practice.moves) {
      const p = parseUci(uci);
      let mv = null;
      if (p) { try { mv = chess.move(p); } catch { mv = null; } }
      if (!mv) break;
      g.moves.push(uci); g.sans.push(mv.san); g.fens.push(chess.fen()); g.lastMoves.push([mv.from, mv.to]);
    }
    g.prefill = g.moves.length;
    if (!g.prefill) g.practice = null;
  }

  const plies = () => g.moves.length;
  /** Plies actually played in this game (not counting a practice's pre-played moves). */
  const playedPlies = () => plies() - g.prefill;
  let viewPly = null;           // null = live position
  let botToken = 0;
  let botPending = false;
  let botAc = null;
  let botTimer = null;
  let chatTimer = null;
  let openingAc = null;
  let hint = { fen: null, stage: 0, move: null, loading: false };
  let analysis = { fen: null, best: null, score: null, depth: 0 };
  const evals = new Map();       // fen -> { score, opts } so browsing back and forth redraws the eval bar instantly
  let gameOverModal = null;
  let goRatingEl = null;         // rating line inside the game-over modal
  let ended = false;
  let discarded = false;
  let pendingConfirm = null;     // { fenBefore, uci, ply } while a move waits for Confirm / Undo
  let premoveRunning = false;
  let peeking = false;

  // ---- DOM ---------------------------------------------------------------
  const s = getSettings();
  const mkBar = (c) => {
    const isBot = c === botC;
    const captures = h('div', { class: 'player-captures' });
    const thinking = isBot ? h('span', { class: 'thinking-dots', 'aria-label': t('play.status.botThinkingAria', { name: bot.name }), hidden: true }, h('i'), h('i'), h('i')) : null;
    const chat = isBot ? h('div', { class: 'bot-chat bubble', role: 'status', 'aria-live': 'polite', hidden: true }) : null;
    const clockSlot = h('div', { class: 'clock-slot' });
    const name = isBot ? bot.name : ((ctx.profile && ctx.profile.name) || t('play.you'));
    const rating = isBot ? `(${bot.id === ADAPTIVE_ID ? '~' : ''}${bot.elo ?? '?'})` : null;
    const el = h('div', { class: `player-bar play-bar ${isBot ? 'is-bot' : 'is-user'}` },
      isBot ? botAvatar(bot.id === ADAPTIVE_ID ? { ...bot, category: 'adaptive' } : bot) : avatarNode(ctx.profile && ctx.profile.avatar, 'avatar-round user-avatar'),
      h('div', { style: 'min-width:0' },
        h('div', { class: 'player-name' }, name, ' ', rating ? h('span', { class: 'player-rating' }, rating) : null, thinking),
        captures),
      chat,
      clockSlot);
    return { el, captures, thinking, chat, clockSlot };
  };
  const bars = { [userC]: mkBar(userC), [botC]: mkBar(botC) };
  const evalSlot = h('div', { class: 'evalbar-slot' });
  const boardSlot = h('div', { class: 'board-slot' });
  const boardRow = h('div', { class: 'board-row' }, evalSlot, boardSlot);
  const main = h('div', { class: 'game-main' });

  const openingEl = h('div', { class: 'play-opening truncate' });
  const modeBadges = h('div', { class: 'row-sm' },
    g.opts.coach ? h('span', { class: 'badge badge-primary', title: t('play.game.coachOnTitle') }, '🎓 ' + t('play.game.coachBadge')) : null,
    g.tc.id !== 'none' ? h('span', { class: 'badge', html: icon('clock', { size: 12 }) + ' ' + escapeHtml(tcLabel(g.tc)) }) : null,
    g.extras.blindfold ? h('span', { class: 'badge badge-info', html: icon('eye-off', { size: 12 }) + ' ' + escapeHtml(t('play.blindfold.badge')) }) : null,
    !g.opts.hints && !g.opts.takebacks && !g.opts.evalBar && !g.opts.coach ? h('span', { class: 'badge badge-danger' }, t('play.modes.challenge.label')) : null);
  const statusEl = h('div', { class: 'play-status', role: 'status', 'aria-live': 'polite' });
  // "Practising: <opening>" banner with a gentle note once the game leaves the book.
  const practiceNote = h('div', { class: 'practice-note text-xs', role: 'status', 'aria-live': 'polite', hidden: true });
  const practiceEl = g.practice ? h('div', { class: 'practice-banner' },
    h('div', { class: 'practice-banner-title text-sm' }, h('span', { html: icon('openings', { size: 14 }) }),
      h('span', { class: 'truncate' }, t('practice.game.banner', { name: g.practice.name || t('practice.yourLine') }))),
    practiceNote) : null;
  const coachText = h('div', { class: 'bubble bubble-mentor coach-text md' });
  const coachBox = h('div', { class: 'coach-box mentor-row', hidden: true },
    h('div', { class: 'avatar avatar-sm avatar-round coach-avatar', 'aria-hidden': 'true' }, '🎓'), coachText);
  const moveListEl = h('div', { class: 'panel-body play-moves' });
  const navBtn = (ic, label, fn) => h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': label, 'data-tooltip': label, html: icon(ic), onClick: fn });
  const nav = {
    first: navBtn('first', t('play.nav.first'), () => goto(0)),
    prev: navBtn('chevron-left', t('play.nav.prev'), () => goto(currentPly() - 1)),
    next: navBtn('chevron-right', t('play.nav.next'), () => goto(currentPly() + 1)),
    last: navBtn('last', t('play.nav.last'), () => goto(plies())),
  };
  const liveChip = h('button', { type: 'button', class: 'btn btn-sm btn-secondary live-chip', hidden: true, onClick: () => goto(plies()), html: icon('play', { size: 14 }) + `<span>${escapeHtml(t('play.nav.backToGame'))}</span>` });
  const navBar = h('div', { class: 'toolbar play-nav' }, nav.first, nav.prev, liveChip, h('div', { class: 'spacer' }), nav.next, nav.last);

  const actionBtn = (ic, label, fn, extra = '') => h('button', { type: 'button', class: `btn btn-secondary play-action ${extra}`.trim(), onClick: fn, html: icon(ic) + `<span>${escapeHtml(label)}</span>` });
  const acts = {
    hint: actionBtn('hint', t('play.actions.hint'), () => onHint(), 'act-hint'),
    takeback: actionBtn('undo', t('play.actions.takeback'), () => onTakeback()),
    flip: actionBtn('flip', t('play.actions.flip'), () => onFlip()),
    draw: actionBtn('handshake', t('play.actions.draw'), () => onOfferDraw()),
    resign: actionBtn('flag', t('play.actions.resign'), () => onResign(), 'act-resign'),
    newGame: actionBtn('plus', t('play.actions.newGame'), () => onNewGame()),
  };
  acts.hint.setAttribute('aria-label', t('play.actions.hintAria'));
  acts.draw.setAttribute('aria-label', t('play.actions.drawAria'));
  acts.newGame.setAttribute('aria-label', t('play.actions.newGameAria'));
  if (!g.opts.hints) acts.hint.hidden = true;
  if (!g.opts.takebacks) acts.takeback.hidden = true;
  const actionsEl = h('div', { class: 'play-actions' }, ...Object.values(acts));

  // Move confirmation bar (option "confirmMove").
  const confirmText = h('span', { class: 'grow' });
  const confirmBar = h('div', { class: 'play-confirm', role: 'group', 'aria-label': t('play.confirm.aria'), hidden: true },
    confirmText,
    h('button', { type: 'button', class: 'btn btn-secondary btn-sm', html: icon('undo', { size: 16 }) + `<span>${escapeHtml(t('play.confirm.undo'))}</span>`, onClick: () => undoPending() }),
    h('button', { type: 'button', class: 'btn btn-primary btn-sm', html: icon('check', { size: 16 }) + `<span>${escapeHtml(t('play.confirm.confirm'))}</span>`, onClick: () => confirmPending() }));

  // Typed moves (option "typeMoves", always on in blindfold) and the blindfold "peek" button.
  const showEntry = g.extras.typeMoves || g.extras.blindfold;
  const moveInput = h('input', {
    type: 'text', class: 'input play-move-input', autocomplete: 'off', autocapitalize: 'off', spellcheck: 'false',
    maxlength: '12', placeholder: t('play.typed.placeholder'), 'aria-label': t('play.typed.aria'),
  });
  const moveFeedback = h('div', { class: 'play-move-feedback text-xs', role: 'status', 'aria-live': 'polite' });
  const peekBtn = g.extras.blindfold ? h('button', {
    type: 'button', class: 'btn btn-secondary btn-sm play-peek', 'aria-pressed': 'false',
    html: icon('eye', { size: 16 }) + `<span>${escapeHtml(t('play.blindfold.peek'))}</span>`,
    title: t('play.blindfold.peekTitle'),
  }) : null;
  const moveEntry = showEntry ? h('form', { class: 'play-move-entry', onSubmit: (e) => { e.preventDefault(); onTypedMove(); } },
    h('div', { class: 'play-move-row' },
      moveInput,
      h('button', { type: 'submit', class: 'btn btn-primary btn-sm', html: icon('send', { size: 16 }), 'aria-label': t('play.typed.submit'), title: t('play.typed.submit') }),
      peekBtn),
    moveFeedback) : null;
  const afterEl = h('div', { class: 'play-after', hidden: true });

  const panel = h('aside', { class: 'game-panel' },
    h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header play-panel-head' }, h('span', { html: icon('openings') }), openingEl, h('div', { class: 'spacer' }), modeBadges),
      practiceEl,
      statusEl,
      coachBox,
      moveListEl,
      navBar),
    confirmBar,
    moveEntry,
    afterEl,
    actionsEl);

  const layout = h('div', { class: ['game-layout play-game page-enter', !g.opts.evalBar && 'no-eval'] }, main, panel);
  host.appendChild(layout);

  // ---- Components --------------------------------------------------------
  const board = new Board(boardSlot, {
    fen: chess.fen(),
    orientation: colorName(userC),
    interactive: true,
    movableColor: colorName(userC),
    showCoords: g.extras.blindfold ? true : s.showCoords, showLegal: s.showLegal, animationMs: s.animationMs, sounds: s.sounds,
    blindfold: g.extras.blindfold,
    onMove: (m) => onUserMove(m, { premove: premoveRunning }),
    onPremove: () => renderStatus(),
  });
  bag.add(() => board.destroy());

  let evalbar = null;
  let engine = null;
  if (g.opts.evalBar) {
    evalbar = new EvalBar(evalSlot, { orientation: colorName(userC) });
    evalbar.set({ cp: 0 });
    bag.add(() => evalbar.destroy());
    engine = new EngineClient();
    bag.add(() => engine.close());
  }

  const movelist = new MoveList(moveListEl, { onSelect: (ply) => goto(ply) });
  bag.add(() => movelist.destroy());

  let clock = null;
  if (g.tc.id !== 'none') {
    const initial = resume && resume.clocks && Number.isFinite(resume.clocks.white) && Number.isFinite(resume.clocks.black)
      ? { white: resume.clocks.white, black: resume.clocks.black } : g.tc.initial;
    clock = new ChessClock(null, {
      initialMs: initial, incrementMs: g.tc.inc,
      slots: { [colorName(userC)]: bars[userC].clockSlot, [colorName(botC)]: bars[botC].clockSlot },
      onFlag: (c) => onFlag(c),
      onLowTime: (c) => { if (c === colorName(userC)) playSoundSafe('lowTime'); },
    });
    bag.add(() => clock.destroy());
  }

  // Lazy sound module (optional; board plays move sounds itself).
  let playSoundFn = null;
  import('../components/sound.js').then((m) => {
    playSoundFn = typeof m.playSound === 'function' ? m.playSound : null;
    if (!resume && !bag.disposed) playSoundSafe('gameStart'); // a fresh game, not a resumed one
  }).catch(() => {});
  function playSoundSafe(name) {
    if (!playSoundFn || getSettings().sounds === false) return;
    try { playSoundFn(name); } catch { /* ignore */ }
  }

  bag.add(() => {
    botToken++;
    if (botTimer) clearTimeout(botTimer);
    if (chatTimer) clearTimeout(chatTimer);
    if (botAc) botAc.abort();
    if (openingAc) openingAc.abort();
    if (gameOverModal) gameOverModal.close();
  });

  // ---- Rendering helpers -------------------------------------------------
  function currentPly() { return viewPly === null ? plies() : viewPly; }
  function isLive() { return viewPly === null; }
  function userToMove() { return chess.turn() === userC; }

  function placeBars() {
    const orient = board.orientation === 'black' ? 'b' : 'w';
    main.replaceChildren(bars[other(orient)].el, boardRow, bars[orient].el);
  }

  function updateInteractivity() {
    const can = isLive() && !g.result && !botPending && !pendingConfirm && userToMove();
    try { board.setInteractive(can, can ? colorName(userC) : null); } catch { /* ignore */ }
    // Premoves can be queued while the bot is to move (and survive its move landing).
    const pre = g.extras.premoves && isLive() && !g.result && !pendingConfirm;
    try { board.setPremoveColor(pre ? colorName(userC) : null); } catch { /* ignore */ }
  }

  function renderCaptures() {
    const counts = { w: { p: 0, n: 0, b: 0, r: 0, q: 0, k: 0 }, b: { p: 0, n: 0, b: 0, r: 0, q: 0, k: 0 } };
    const p = currentPly();
    const pos = p === plies() ? chess : new Chess(g.fens[p]);
    for (const row of pos.board()) for (const sq of row) if (sq) counts[sq.color][sq.type]++;
    let mat = { w: 0, b: 0 };
    for (const c of ['w', 'b']) for (const t of Object.keys(PIECE_VALUES)) mat[c] += PIECE_VALUES[t] * counts[c][t];
    for (const c of ['w', 'b']) {
      const opp = other(c);
      const frag = document.createDocumentFragment();
      for (const t of ['p', 'b', 'n', 'r', 'q']) {
        const missing = Math.max(0, START_COUNTS[t] - counts[opp][t]);
        if (!missing) continue;
        frag.appendChild(h('span', { class: `cap-group cap-${opp}` }, GLYPHS[opp][t].repeat(missing)));
      }
      const diff = mat[c] - mat[opp];
      if (diff > 0) frag.appendChild(h('span', { class: 'cap-diff' }, `+${diff}`));
      bars[c].captures.replaceChildren(frag);
    }
  }

  function renderMoveList() {
    const parts = g.startFen.split(' ');
    movelist.setMoves(g.sans.map((san, i) => {
      const cls = g.cls[i + 1];
      return { san, classification: cls && NOTABLE_CLS.has(cls) ? cls : undefined };
    }), { startColor: parts[1] === 'b' ? 'black' : 'white', startMoveNumber: Number(parts[5]) || 1 });
    movelist.setCurrent(currentPly());
  }

  function renderNav() {
    const p = currentPly();
    nav.first.disabled = p <= 0;
    nav.prev.disabled = p <= 0;
    nav.next.disabled = p >= plies();
    nav.last.disabled = p >= plies();
    liveChip.hidden = isLive() || !!g.result;
  }

  function renderOpening() {
    openingEl.textContent = g.openingName || (g.practice && g.practice.name) || (customStart ? t('play.custom.inGame') : (plies() ? t('play.game.inProgress') : t('play.game.startingPosition')));
    openingEl.title = openingEl.textContent;
  }

  function renderStatus() {
    statusEl.className = 'play-status';
    statusEl.replaceChildren();
    if (g.result) {
      statusEl.classList.add('done');
      statusEl.append(h('span', { html: icon('trophy', { size: 16 }) }), h('span', null, t('play.status.result', { result: resultHeadline(), reason: terminationText(g.termination) })));
      return;
    }
    if (!isLive()) {
      statusEl.classList.add('browsing');
      statusEl.append(h('span', { html: icon('eye', { size: 16 }) }), h('span', null, t('play.status.viewing', { move: Math.ceil(viewPly / 2) || 0, back: t('play.nav.backToGame') })));
      return;
    }
    if (pendingConfirm) {
      statusEl.classList.add('your-turn');
      statusEl.append(h('span', { html: icon('check-circle', { size: 16 }) }), h('span', null, t('play.confirm.status')));
      return;
    }
    const pm = board.getPremove();
    if (botPending) {
      statusEl.classList.add('thinking');
      statusEl.append(h('span', { class: 'thinking-dots' }, h('i'), h('i'), h('i')), h('span', { class: 'grow' }, t('play.status.botThinking', { name: bot.name })));
      if (pm) statusEl.append(premoveChip(pm));
      return;
    }
    if (userToMove()) {
      statusEl.classList.add('your-turn');
      const inCheck = chess.inCheck();
      statusEl.append(h('span', { class: `turn-dot ${colorName(userC)}` }),
        h('span', null, inCheck ? t('play.status.inCheck') : t(userC === 'w' ? 'play.status.yourMoveWhite' : 'play.status.yourMoveBlack')));
    } else {
      statusEl.append(h('span', { class: `turn-dot ${colorName(botC)}` }), h('span', null, t('play.status.botToMove', { name: bot.name })));
    }
  }

  function premoveChip(pm) {
    return h('button', {
      type: 'button', class: 'premove-chip', title: t('play.premove.cancelTitle'),
      onClick: () => { board.clearPremove(); renderStatus(); },
    }, h('span', { html: icon('bolt', { size: 12 }) }), t('play.premove.queued', { from: pm.from, to: pm.to }), h('span', { html: icon('x', { size: 12 }) }));
  }

  function setBotError(message) {
    statusEl.className = 'play-status error';
    statusEl.replaceChildren(
      h('span', { html: icon('alert', { size: 16 }) }),
      h('span', { class: 'grow' }, t('play.status.botError', { name: bot.name, message })),
      h('button', { type: 'button', class: 'btn btn-sm btn-primary', onClick: () => requestBotMove() }, t('play.status.retry')));
  }

  function renderActions() {
    const live = isLive();
    const over = !!g.result;
    acts.hint.disabled = over || !live || botPending || !!pendingConfirm || !userToMove() || hint.loading;
    acts.hint.classList.toggle('stage-2', hint.fen === chess.fen() && hint.stage >= 1);
    acts.takeback.disabled = over || !!pendingConfirm || !canTakeback();
    acts.draw.disabled = over || playedPlies() < 2;
    acts.resign.disabled = over;
    actionsEl.hidden = over;
    afterEl.hidden = !over;
    confirmBar.hidden = !pendingConfirm || over;
    if (pendingConfirm) confirmText.textContent = t('play.confirm.prompt', { san: g.sans[pendingConfirm.ply - 1] || '' });
    if (moveEntry) moveEntry.hidden = over;
  }

  function renderBadge() {
    try { board.clearBadges(); } catch { return; }
    const d = currentPly();
    // Show the coach badge on the user's most recent move visible at this ply.
    for (const u of [d, d - 1]) {
      if (u < 1 || !g.cls[u]) continue;
      const moverIsUser = plyColor(u) === userC;
      if (!moverIsUser) continue;
      const lm = g.lastMoves[u];
      if (!lm) continue;
      if (u === d - 1) {
        const later = g.lastMoves[d];
        if (later && (later[0] === lm[1] || later[1] === lm[1])) continue; // square changed
      }
      try { board.setBadge(lm[1], g.cls[u]); } catch { /* ignore */ }
      break;
    }
  }

  /** Colour that played ply p (1-based). */
  function plyColor(p) {
    const firstColor = g.startFen.split(' ')[1] === 'b' ? 'b' : 'w';
    return (p % 2 === 1) ? firstColor : other(firstColor);
  }

  function syncBoard(animate = true) {
    const p = currentPly();
    try { board.setPosition(g.fens[p], { animate, lastMove: g.lastMoves[p] || null }); } catch (e) { console.error(e); }
    if (isLive()) applyHintVisual(); else { try { board.clearArrows(); board.clearHighlights(); } catch { /* ignore */ } }
    renderBadge();
    movelist.setCurrent(p);
    renderCaptures();
    renderNav();
    renderStatus();
    renderActions();
    updateInteractivity();
  }

  function refreshAll(animate = true) {
    renderMoveList();
    renderOpening();
    syncBoard(animate);
  }

  // ---- Coach / bot chat --------------------------------------------------
  function coachSay(markdownText, cls) {
    coachBox.hidden = false;
    coachText.innerHTML = '';
    if (cls) {
      const meta = classificationMeta(cls);
      coachText.appendChild(h('div', { class: 'coach-cls', dataset: { cls: meta.key } }, h('span', { class: 'cls-badge', dataset: { cls: meta.key } }, meta.symbol), h('span', { class: 'cls-text', dataset: { cls: meta.key } }, meta.label)));
    }
    coachText.appendChild(h('div', { html: mdLite(markdownText) }));
    coachText.classList.remove('pop-in'); void coachText.offsetWidth; coachText.classList.add('pop-in');
  }

  function botSay(text) {
    const chat = bars[botC].chat;
    if (!chat || !text) return;
    chat.textContent = String(text);
    chat.title = String(text);
    chat.hidden = false;
    chat.classList.remove('pop-in'); void chat.offsetWidth; chat.classList.add('pop-in');
    if (chatTimer) clearTimeout(chatTimer);
    chatTimer = setTimeout(() => { chatTimer = null; chat.hidden = true; }, BOT_CHAT_MS);
  }

  function setThinking(on) {
    const t = bars[botC].thinking;
    if (t) t.hidden = !on;
  }

  // ---- Engine (eval bar) -------------------------------------------------
  function rememberEval(fen, score, opts) {
    if (!fen || !score) return;
    evals.delete(fen);
    evals.set(fen, { score, opts: opts || null });
    if (evals.size > MAX_EVALS) evals.delete(evals.keys().next().value);
  }

  /** Show the cached eval of the position on the board (live or browsed). */
  function showEval(fen) {
    const hit = evalbar && evals.get(fen);
    if (hit) evalbar.set(hit.score, hit.opts || undefined);
    return !!hit;
  }

  function analyzeLive() {
    if (!engine || g.result) return;
    if (!isLive()) { analyzeViewed(); return; }
    if (!userToMove()) { engine.stop(); return; }
    const fen = chess.fen();
    showEval(fen);
    try {
      engine.analyze(fen, { multipv: 1, movetime_ms: 3000 }, (info) => {
        if (fen !== chess.fen() || g.result) return;
        const line = info && info.lines && info.lines[0];
        if (!line || !line.score) return;
        analysis = { fen, best: (line.moves && line.moves[0]) || analysis.best, score: line.score, depth: info.depth || 0 };
        rememberEval(fen, line.score);
        if (evalbar && isLive()) evalbar.set(line.score);
      }, () => { /* engine hiccup: the bar just stays where it was */ });
    } catch (e) { console.warn('[play] engine', e); }
  }

  /** Eval bar for a position the engine isn't following live (browsing, bot's turn, game over). */
  function analyzeViewed() {
    if (!evalbar) return;
    const fen = g.fens[currentPly()];
    if (showEval(fen)) { if (engine) engine.stop(); return; }
    if (!engine) return;
    try {
      engine.analyze(fen, { multipv: 1, movetime_ms: 1500 }, (info) => {
        const line = info && info.lines && info.lines[0];
        if (!line || !line.score) return;
        rememberEval(fen, line.score);
        if (g.fens[currentPly()] === fen) evalbar.set(line.score);
      }, () => { /* engine hiccup: keep the cached value */ });
    } catch (e) { console.warn('[play] engine', e); }
  }

  /** Called whenever the viewed ply changes. */
  function refreshEval() {
    if (!evalbar) return;
    if (isLive() && !g.result && !botPending && userToMove()) analyzeLive();
    else analyzeViewed();
  }

  async function bestMoveFor(fen) {
    if (analysis.fen === fen && analysis.best && analysis.depth >= 8) return { uci: analysis.best, score: analysis.score };
    const info = await api.post('/api/engine/analyze', { fen, movetime_ms: 900, multipv: 1 }, { signal: gac.signal, timeout: 15000 });
    const line = info && info.lines && info.lines[0];
    if (!line || !line.moves || !line.moves[0]) throw new Error(t('play.hint.noMove'));
    if (fen === chess.fen() && (!analysis.fen || analysis.fen !== fen || (info.depth || 0) >= analysis.depth)) {
      analysis = { fen, best: line.moves[0], score: line.score, depth: info.depth || 0 };
    }
    return { uci: line.moves[0], score: line.score };
  }

  // ---- Opening name ------------------------------------------------------
  function lookupOpening() {
    if (plies() > 30 || plies() === 0 || customStart) return;
    if (openingAc) openingAc.abort();
    const myAc = new AbortController();
    openingAc = myAc;
    const fen = chess.fen();
    const atPly = plies();
    api.get('/api/openings/lookup' + qs({ fen }), { signal: myAc.signal, timeout: 8000 }).then((m) => {
      if (openingAc === myAc) openingAc = null;
      if (bag.disposed) return;
      // Practice: the first position the book does not know means "out of book".
      if (!m && g.practice && !g.outOfBookPly && atPly === plies() && g.fens[atPly] === fen) {
        g.outOfBookPly = atPly;
        renderPracticeNote();
      }
      if (!m || !m.opening || !m.opening.name) return;
      g.openingName = m.opening.name;
      renderOpening();
    }).catch(() => { if (openingAc === myAc) openingAc = null; });
  }

  // ---- Opening practice --------------------------------------------------
  /** First ply (1-based) where the game left the opening's main line, or 0. */
  function bookDeviation() {
    if (!g.practice) return 0;
    const book = g.practice.book || [];
    const n = Math.min(book.length, plies());
    for (let i = g.prefill; i < n; i++) if (g.moves[i] !== book[i]) return i + 1;
    return 0;
  }

  function renderPracticeNote() {
    if (!practiceEl) return;
    if (g.outOfBookPly > plies()) g.outOfBookPly = 0; // taken back
    const dev = bookDeviation();
    let text = '';
    if (dev) {
      if (plyColor(dev) === userC) {
        let san = '';
        try { const mv = new Chess(g.fens[dev - 1]).move(parseUci(g.practice.book[dev - 1])); san = mv ? mv.san : ''; } catch { san = ''; }
        text = san ? t('practice.game.leftBookUser', { san }) : t('practice.game.leftBookUserPlain');
      } else {
        text = t('practice.game.leftBookBot', { name: bot.name });
      }
    } else if (g.outOfBookPly) {
      text = t('practice.game.outOfBook');
    }
    practiceNote.textContent = text;
    practiceNote.hidden = !text;
  }

  // ---- Moves -------------------------------------------------------------
  function recordMove(mv) {
    g.moves.push(mv.from + mv.to + (mv.promotion || ''));
    g.sans.push(mv.san);
    g.fens.push(chess.fen());
    g.lastMoves.push([mv.from, mv.to]);
    g.cls.length = plies() + 1;
    viewPly = null;
  }

  function onUserMove(m, { premove = false } = {}) {
    if (!m || !isLive() || g.result || botPending || pendingConfirm || !userToMove()) return false;
    const fenBefore = chess.fen();
    let mv = null;
    try { mv = chess.move({ from: m.from, to: m.to, promotion: m.promotion || undefined }); } catch { mv = null; }
    if (!mv) return false;
    recordMove(mv);
    const ply = plies();
    const uci = g.moves[ply - 1];
    clearHint();
    if (g.extras.confirmMove && !premove) {
      // Wait for Confirm / Undo before the move counts (clock keeps running).
      pendingConfirm = { fenBefore, uci, ply };
      queueMicrotask(() => {
        if (bag.disposed) return;
        refreshAll(false);
      });
      return true;
    }
    pressClock(userC);
    // Let the board finish its own move handling before we sync it.
    queueMicrotask(() => {
      if (bag.disposed) return;
      refreshAll(false);
      afterMove(fenBefore, uci, ply, true);
    });
    return true;
  }

  function confirmPending() {
    const pc = pendingConfirm;
    if (!pc || g.result) return;
    pendingConfirm = null;
    pressClock(userC);
    refreshAll(false);
    afterMove(pc.fenBefore, pc.uci, pc.ply, true);
  }

  /** Take the unconfirmed move back. */
  function undoPending({ render = true } = {}) {
    if (!pendingConfirm) return;
    pendingConfirm = null;
    chess.undo();
    g.moves.pop(); g.sans.pop(); g.fens.pop(); g.lastMoves.pop();
    g.cls.length = plies() + 1;
    viewPly = null;
    if (render) refreshAll(true);
  }

  // ---- Typed moves -------------------------------------------------------
  function setMoveFeedback(text, kind) {
    moveFeedback.textContent = text || '';
    moveFeedback.className = `play-move-feedback text-xs ${kind ? 'is-' + kind : ''}`.trim();
    moveInput.classList.toggle('input-error', kind === 'error');
  }

  function onTypedMove() {
    const text = moveInput.value.trim();
    if (!text) return;
    if (g.result) return;
    if (pendingConfirm) { setMoveFeedback(t('play.typed.confirmFirst'), 'error'); return; }
    if (!isLive()) goto(plies());
    if (!userToMove() || botPending) {
      // Not our turn: queue it as a premove if possible.
      if (!g.extras.premoves) { setMoveFeedback(t('play.typed.wait', { name: bot.name }), 'error'); return; }
      const parts = chess.fen().split(' ');
      parts[1] = userC; parts[3] = '-';
      let probe = null;
      try { probe = new Chess(parts.join(' ')); } catch { probe = null; }
      const pm = probe && parseTypedMove(probe, text);
      if (!pm) { setMoveFeedback(t('play.typed.wait', { name: bot.name }), 'error'); return; }
      board.setPremove(pm);
      renderStatus();
      moveInput.value = '';
      setMoveFeedback(t('play.typed.premoved', { san: pm.san }), 'ok');
      return;
    }
    const mv = parseTypedMove(chess, text);
    if (!mv) {
      setMoveFeedback(looksLikeMove(text) ? t('play.typed.illegal', { move: text }) : t('play.typed.unknown', { move: text }), 'error');
      return;
    }
    const shown = board.move({ from: mv.from, to: mv.to, promotion: mv.promotion });
    if (!onUserMove({ from: mv.from, to: mv.to, promotion: mv.promotion })) {
      if (shown) syncBoard(false);
      setMoveFeedback(t('play.typed.illegal', { move: text }), 'error');
      return;
    }
    moveInput.value = '';
    setMoveFeedback(t('play.typed.played', { san: mv.san }), 'ok');
  }
  if (moveEntry) {
    bag.on(moveInput, 'input', () => { if (moveInput.classList.contains('input-error')) setMoveFeedback('', null); });
  }

  // ---- Blindfold peek (press and hold) ----------------------------------
  function setPeek(on) {
    if (peeking === on) return;
    peeking = on;
    try { board.setPeek(on); } catch { /* ignore */ }
    if (peekBtn) { peekBtn.setAttribute('aria-pressed', on ? 'true' : 'false'); peekBtn.classList.toggle('active', on); }
  }
  if (peekBtn) {
    bag.on(peekBtn, 'pointerdown', (e) => { e.preventDefault(); setPeek(true); try { peekBtn.setPointerCapture(e.pointerId); } catch { /* ignore */ } });
    for (const ev of ['pointerup', 'pointercancel', 'lostpointercapture', 'blur']) bag.on(peekBtn, ev, () => setPeek(false));
    bag.on(peekBtn, 'keydown', (e) => { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); setPeek(true); } });
    bag.on(peekBtn, 'keyup', (e) => { if (e.key === ' ' || e.key === 'Enter') setPeek(false); });
    bag.on(peekBtn, 'contextmenu', (e) => e.preventDefault());
  }

  function pressClock(mover) {
    if (!clock || g.result) return;
    if (!clock.running) clock.start(colorName(other(mover)));
    else clock.press();
  }

  function afterMove(fenBefore, uci, ply, byUser) {
    lookupOpening();
    renderPracticeNote();
    if (checkEnd()) return;
    if (byUser) {
      if (g.opts.coach) coachExplain(fenBefore, uci, ply);
      requestBotMove();
    } else {
      analyzeLive();
    }
    persist();
  }

  async function requestBotMove() {
    if (g.result || bag.disposed) return;
    const token = ++botToken;
    if (botAc) botAc.abort();
    const myAc = new AbortController();
    botAc = myAc;
    botPending = true;
    setThinking(true);
    if (engine) engine.stop();
    renderStatus(); renderActions(); updateInteractivity();
    const t0 = performance.now();
    try {
      const bm = await api.post('/api/bot/move', { bot_id: bot.id, start_fen: g.startFen, moves: g.moves.slice() }, { signal: myAc.signal, timeout: 30000 });
      if (token !== botToken || bag.disposed || g.result) return;
      let delay = clamp(Number(bm && bm.think_ms) || 0, 0, MAX_THINK_MS) - (performance.now() - t0);
      if (clock) delay = Math.min(delay, clock.getTimes()[colorName(botC)] / 40);
      if (delay > 0) {
        await new Promise((resolve) => { botTimer = setTimeout(() => { botTimer = null; resolve(); }, delay); });
        if (token !== botToken || bag.disposed || g.result) return;
      }
      applyBotMove(bm);
    } catch (e) {
      if (token !== botToken || bag.disposed || isAbort(e)) return;
      botPending = false;
      setThinking(false);
      renderActions();
      setBotError(e.message || t('play.status.networkError'));
    } finally {
      if (botAc === myAc) botAc = null;
    }
  }

  function applyBotMove(bm) {
    const p = parseUci(bm && bm.uci);
    const fenBefore = chess.fen();
    let mv = null;
    if (p) { try { mv = chess.move(p); } catch { mv = null; } }
    botPending = false;
    setThinking(false);
    if (!mv) {
      renderActions();
      setBotError(t('play.status.illegalMove'));
      return;
    }
    recordMove(mv);
    pressClock(botC);
    refreshAll(true);
    if (bm.chat) botSay(bm.chat);
    afterMove(fenBefore, g.moves[plies() - 1], plies(), false);
    runPremove();
  }

  /** Play the queued premove now that it's our turn (dropped quietly if it became illegal). */
  function runPremove() {
    if (!board.getPremove()) return;
    if (g.result || !isLive() || botPending || !userToMove()) { board.clearPremove(); renderStatus(); return; }
    premoveRunning = true;
    let played = null;
    try { played = board.playPremove(); } finally { premoveRunning = false; }
    if (!played) {
      renderStatus();
      setMoveFeedbackSafe(t('play.premove.dropped'));
    }
  }
  function setMoveFeedbackSafe(text) { if (moveEntry) setMoveFeedback(text, 'error'); }

  // ---- Coach mode --------------------------------------------------------
  async function coachExplain(fenBefore, uci, ply) {
    try {
      const r = await api.post('/api/mentor/explain', { fen: fenBefore, move_uci: uci }, { signal: gac.signal, timeout: 20000 });
      if (bag.disposed || g.moves[ply - 1] !== uci || g.fens[ply - 1] !== fenBefore || !r) return;
      const cls = typeof r.classification === 'string' ? r.classification : null;
      g.cls[ply] = cls;
      renderMoveList();
      renderBadge();
      const san = g.sans[ply - 1];
      let text = r.explanation ? String(r.explanation)
        : (cls ? t('play.coach.classified', { san, label: classificationMeta(cls).label }) : t('play.coach.played', { san }));
      if (cls && !GOOD_CLS.has(cls) && r.best_move_san && r.best_move_san !== san && !text.includes(r.best_move_san)) {
        text += '\n\n' + t('play.coach.betterWas', { san: r.best_move_san });
      }
      coachSay(text, cls);
      if (r.eval_after) rememberEval(g.fens[ply], r.eval_after);
      if (evalbar && r.eval_after && plies() === ply && isLive() && !g.result) evalbar.set(r.eval_after);
      persist();
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      // Coach is optional: stay quiet but log for debugging.
      console.warn('[play] coach unavailable', e && e.message);
    }
  }

  // ---- Hints -------------------------------------------------------------
  function clearHint() {
    hint = { fen: null, stage: 0, move: null, loading: false };
    try { board.clearArrows(); board.clearHighlights(); } catch { /* ignore */ }
  }

  function applyHintVisual() {
    try { board.clearArrows(); board.clearHighlights(); } catch { return; }
    if (!hint.move || hint.fen !== chess.fen() || g.result) return;
    if (hint.stage >= 1) board.setHighlights([{ square: hint.move.from, kind: 'hint' }]);
    if (hint.stage >= 2) board.setArrows([{ from: hint.move.from, to: hint.move.to, color: 'green' }]);
  }

  async function onHint() {
    if (!g.opts.hints || g.result || botPending || !userToMove() || hint.loading) return;
    if (!isLive()) goto(plies());
    const fen = chess.fen();
    if (hint.fen !== fen) hint = { fen, stage: 0, move: null, loading: false };
    if (hint.stage >= 2) { applyHintVisual(); return; }
    if (!hint.move) {
      hint.loading = true;
      acts.hint.classList.add('loading');
      renderActions();
      try {
        const best = await bestMoveFor(fen);
        if (bag.disposed || chess.fen() !== fen) return;
        const p = parseUci(best.uci);
        const probe = new Chess(fen);
        const mv = p ? probe.move(p) : null;
        if (!mv) throw new Error(t('play.hint.none'));
        hint.move = { from: mv.from, to: mv.to, san: mv.san, piece: mv.piece };
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        toast(t('play.hint.failed'), 'warning');
        return;
      } finally {
        if (!bag.disposed) {
          hint.loading = false;
          acts.hint.classList.remove('loading');
          renderActions();
        }
      }
    }
    if (hint.fen !== chess.fen()) return;
    hint.stage++;
    if (hint.stage === 1) {
      g.hintsUsed++;
      coachSay(t('play.hint.stage1', {
        piece: t(hasKey(`play.pieces.${hint.move.piece}`) ? `play.pieces.${hint.move.piece}` : 'play.pieces.generic'),
        square: hint.move.from,
        hint: t('play.actions.hint'),
      }));
    } else {
      coachSay(t('play.hint.stage2', { san: hint.move.san, from: hint.move.from, to: hint.move.to }));
    }
    applyHintVisual();
    renderActions();
    persist();
  }

  // ---- Takeback ----------------------------------------------------------
  function minPly() {
    // The user cannot take back the bot's opening move when playing black,
    // nor the moves that were already on the board in an opening practice.
    const base = g.prefill || 0;
    return plyColor(base + 1) === userC ? base : base + 1;
  }
  function takebackCount() {
    if (botPending) return 1;
    if (userToMove()) return 2;
    return 1;
  }
  function canTakeback() {
    if (!g.opts.takebacks || g.result) return false;
    return plies() - takebackCount() >= minPly() && plies() > 0;
  }

  function onTakeback() {
    if (!canTakeback()) { toast(t('play.takeback.nothing'), 'info'); return; }
    const n = takebackCount();
    try { board.clearPremove(); } catch { /* ignore */ }
    botToken++;
    if (botAc) { botAc.abort(); botAc = null; }
    if (botTimer) { clearTimeout(botTimer); botTimer = null; }
    botPending = false;
    setThinking(false);
    for (let i = 0; i < n; i++) {
      chess.undo();
      g.moves.pop(); g.sans.pop(); g.fens.pop(); g.lastMoves.pop();
    }
    g.cls.length = plies() + 1;
    g.takebacksUsed++;
    if (plies() === 0) g.openingName = null;
    viewPly = null;
    clearHint();
    renderPracticeNote();
    if (clock && clock.running && !g.result) clock.start(colorName(chess.turn()));
    refreshAll(true);
    if (!coachBox.hidden) coachSay(t('play.takeback.coach'));
    if (!userToMove()) requestBotMove(); else analyzeLive();
    persist();
  }

  // ---- Browsing ----------------------------------------------------------
  function goto(p) {
    const target = clamp(p | 0, 0, plies());
    const nextView = target === plies() ? null : target;
    if (nextView === viewPly) return;
    viewPly = nextView;
    syncBoard(true);
    refreshEval();
  }

  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || isTyping(e)) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (pendingConfirm) {
      if (e.key === 'Enter') { e.preventDefault(); confirmPending(); return; }
      if (e.key === 'Escape' || e.key === 'Backspace') { e.preventDefault(); undoPending(); return; }
    }
    if (e.key === 'Escape' && board.getPremove()) { e.preventDefault(); board.clearPremove(); renderStatus(); return; }
    let handled = true;
    switch (e.key) {
      case 'ArrowLeft': goto(currentPly() - 1); break;
      case 'ArrowRight': goto(currentPly() + 1); break;
      case 'Home': case 'ArrowUp': goto(0); break;
      case 'End': case 'ArrowDown': goto(plies()); break;
      default: handled = false;
    }
    if (handled) e.preventDefault();
  });

  // ---- Flip, resign, draw, new game -------------------------------------
  function onFlip() {
    try { board.flip(); } catch { return; }
    if (evalbar) evalbar.setOrientation(board.orientation);
    placeBars();
  }

  async function onResign() {
    if (g.result) return;
    const ok = await confirmDialog({ title: t('play.resign.title'), message: t('play.resign.message', { name: bot.name }), confirmLabel: t('play.resign.confirm'), danger: true });
    if (!ok || bag.disposed || g.result) return;
    endGame(userC === 'w' ? '0-1' : '1-0', 'resignation');
  }

  async function onOfferDraw() {
    if (g.result || playedPlies() < 2) return;
    if (plies() - g.drawOfferPly < DRAW_OFFER_COOLDOWN_PLIES) {
      toast(t('play.draw.cooldown', { name: bot.name }), 'info');
      return;
    }
    g.drawOfferPly = plies();
    acts.draw.classList.add('loading');
    let score = null;
    try {
      const fen = chess.fen();
      score = analysis.fen === fen && analysis.score ? analysis.score : (await bestMoveFor(fen)).score;
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
    } finally {
      if (!bag.disposed) acts.draw.classList.remove('loading');
    }
    if (bag.disposed || g.result) return;
    const nearZero = score && typeof score.cp === 'number' && Math.abs(score.cp) <= 50;
    const moveNo = Number(g.fens[plies()].split(' ')[5]) || Math.ceil(plies() / 2);
    if (nearZero && moveNo > 30) {
      botSay(t('play.draw.accept'));
      endGame('1/2-1/2', 'agreement');
    } else {
      botSay(moveNo <= 30 ? t('play.draw.tooEarly') : t('play.draw.decline'));
      toast(t('play.draw.declined', { name: bot.name }), 'info');
      persist();
    }
  }

  async function onNewGame() {
    if (!g.result && playedPlies() >= 2) {
      const ok = await confirmDialog({ title: t('play.newGame.title'), message: t('play.newGame.resignMessage'), confirmLabel: t('play.newGame.resignAndStart'), danger: true });
      if (!ok || bag.disposed) return;
      if (!g.result) endGame(userC === 'w' ? '0-1' : '1-0', 'resignation', { silent: true });
    } else if (!g.result) {
      discarded = true;
      removeKey(SAVE_KEY);
    }
    onNewBot(bot.id);
  }

  function onFlag(color) {
    if (g.result) return;
    const flagged = color === 'white' ? 'w' : 'b';
    const winner = other(flagged);
    // Winner needs mating material, otherwise it's a draw.
    let hasMaterial = false;
    for (const row of chess.board()) for (const sq of row) {
      if (sq && sq.color === winner && sq.type !== 'k') { hasMaterial = true; break; }
    }
    if (!hasMaterial) endGame('1/2-1/2', 'timeout vs insufficient material');
    else endGame(winner === 'w' ? '1-0' : '0-1', 'timeout');
  }

  // ---- Game end ----------------------------------------------------------
  function checkEnd() {
    if (g.result) return true;
    if (chess.isCheckmate()) endGame(chess.turn() === 'w' ? '0-1' : '1-0', 'checkmate');
    else if (chess.isStalemate()) endGame('1/2-1/2', 'stalemate');
    else if (chess.isInsufficientMaterial()) endGame('1/2-1/2', 'insufficient material');
    else if (chess.isThreefoldRepetition()) endGame('1/2-1/2', 'threefold repetition');
    else if (chess.isDrawByFiftyMoves()) endGame('1/2-1/2', '50-move rule');
    return !!g.result;
  }

  function userOutcome() {
    if (!g.result || g.result === '1/2-1/2') return 'draw';
    const whiteWon = g.result === '1-0';
    return (whiteWon === (userC === 'w')) ? 'win' : 'loss';
  }

  function resultHeadline() {
    const o = userOutcome();
    return o === 'win' ? t('play.result.youWon') : o === 'loss' ? t('play.result.botWon', { name: bot.name }) : t('play.result.draw');
  }

  function endGame(result, termination, { silent = false } = {}) {
    if (g.result) return;
    // An unconfirmed move never counts (e.g. the clock ran out while it was pending).
    if (pendingConfirm) undoPending({ render: false });
    try { board.clearPremove(); } catch { /* ignore */ }
    g.result = result;
    g.termination = termination;
    botToken++;
    if (botAc) { botAc.abort(); botAc = null; }
    if (botTimer) { clearTimeout(botTimer); botTimer = null; }
    botPending = false;
    setThinking(false);
    if (clock) clock.pause();
    if (engine) engine.stop();
    clearHint();
    viewPly = null;
    removeKey(SAVE_KEY);
    if (result === '1/2-1/2') rememberEval(chess.fen(), { cp: 0 });
    else if (termination === 'checkmate') rememberEval(chess.fen(), { mate: 0 }, { mated: result === '1-0' ? 'black' : 'white' });
    showEval(chess.fen());
    if (g.extras.blindfold) { try { board.setBlindfold(false); } catch { /* ignore */ } }
    refreshAll(false);
    const o = userOutcome();
    if (!silent) {
      announce(o === 'win' ? t(termination === 'checkmate' ? 'a11y.result.youWinMate' : 'a11y.result.youWin') : o === 'loss' ? t('a11y.result.youLose') : t('a11y.result.draw'), { assertive: true });
      playSoundSafe('gameEnd');
      botSay(o === 'win' ? t('play.botChat.userWon') : o === 'loss' ? t('play.botChat.userLost') : t('play.botChat.draw'));
    }
    g.savePromise = saveGame();
    renderAfter();
    if (!silent) bag.timeout(() => { if (!bag.disposed) showGameOver(); }, termination === 'checkmate' ? 650 : 250);
  }

  async function saveGame() {
    if (g.savedId) return g.savedId;
    if (playedPlies() < 2) return null; // aborted games are not saved (like chess.com)
    const userName = (ctx.profile && ctx.profile.name) || t('play.you');
    const tags = ['vs-bot'];
    if (g.hintsUsed || g.takebacksUsed) tags.push('assisted');
    if (g.opts.coach) tags.push('coach');
    if (customStart) tags.push('custom-position');
    if (g.practice) tags.push('opening-practice');
    if (g.extras.blindfold) tags.push('blindfold');
    const notesParts = [];
    if (g.hintsUsed) notesParts.push(t('play.notes.hints', { count: g.hintsUsed }));
    if (g.takebacksUsed) notesParts.push(t('play.notes.takebacks', { count: g.takebacksUsed }));
    if (g.practice) notesParts.push(t('practice.notes', { name: g.practice.name || t('practice.yourLine') }));
    const body = {
      white: userC === 'w' ? userName : bot.name,
      black: userC === 'b' ? userName : bot.name,
      result: g.result,
      termination: g.termination,
      start_fen: g.startFen,
      moves: g.moves.slice(),
      bot_id: bot.id,
      user_color: colorName(userC),
      time_control: g.tc.id === 'none' ? null : g.tc.id,
      opening_name: g.openingName || null,
      notes: notesParts.join(' · '),
      tags,
    };
    try {
      const rec = await api.post('/api/games', body, { timeout: 20000 });
      if (rec && rec.id != null) {
        g.savedId = rec.id;
        if (!bag.disposed) toast(t('play.save.saved'), 'success', { duration: 2200 });
        reportResult(rec.id);
        return rec.id;
      }
      return null;
    } catch (e) {
      if (!bag.disposed) toast(t('play.save.failed', { message: e.message || t('play.save.serverError') }), 'error');
      return null;
    }
  }

  /** Update the estimated rating / adaptive level with the saved game (counted once per game). */
  async function reportResult(gameId) {
    try {
      const r = await api.post('/api/adaptive/result', { game_id: gameId }, { signal: ctx.signal, timeout: 15000 });
      if (!r || typeof r !== 'object') return;
      if (r.estimate) {
        ctx.estimate = r.estimate;
        const ab = ctx.bots.find((x) => x.id === ADAPTIVE_ID);
        if (ab && Number.isFinite(r.estimate.bot_level)) ab.elo = r.estimate.bot_level;
        if (ctx.onEstimate) ctx.onEstimate();
      }
      if (bag.disposed) return;
      g.ratingUpdate = r;
      renderRatingLine();
    } catch { /* the estimate is a bonus: stay quiet */ }
  }

  function renderRatingLine() {
    if (!goRatingEl) return;
    const r = g.ratingUpdate;
    goRatingEl.replaceChildren();
    if (!r || r.skipped || typeof r.rating !== 'number') { goRatingEl.hidden = true; return; }
    goRatingEl.hidden = false;
    const d = r.delta | 0;
    const deltaCls = d > 0 ? 'up' : d < 0 ? 'down' : 'flat';
    // Native append() would print a null child as the text "null": drop it first.
    goRatingEl.append(...[
      h('span', { class: 'go-rating-label' }, t('play.estimate.label')),
      h('strong', { class: 'go-rating-value' }, `~${r.rating}`),
      r.previous == null
        ? h('span', { class: 'go-delta new' }, t('play.estimate.first'))
        : h('span', { class: `go-delta ${deltaCls}` }, d > 0 ? `+${d}` : d < 0 ? `−${Math.abs(d)}` : '±0'),
      r.provisional ? h('span', { class: 'badge' }, t('play.estimate.provisional')) : null,
    ].filter(Boolean));
    if (bot.id === ADAPTIVE_ID && r.bot_level_delta) {
      goRatingEl.append(h('div', { class: 'go-level subtle text-xs' },
        t(r.bot_level_delta > 0 ? 'play.adaptive.levelUp' : 'play.adaptive.levelDown', { name: bot.name, level: r.bot_level })));
    }
  }

  async function goReview() {
    let id = await g.savePromise;
    if (!id && playedPlies() >= 2) { g.savePromise = saveGame(); id = await g.savePromise; }
    if (!id) {
      if (playedPlies() < 2) toast(t('play.save.tooShort'), 'info');
      return false;
    }
    location.hash = `#/review/${id}`;
    return true;
  }

  function rematch() {
    const nextColor = g.colorChoice === 'random' ? other(userC) : userC;
    // The adaptive bot may have changed level: use the fresh profile next game.
    const nextBot = bot.id === ADAPTIVE_ID && g.ratingUpdate && g.ratingUpdate.estimate
      ? { ...bot, elo: g.ratingUpdate.estimate.bot_level } : bot;
    onRematch({ bot: nextBot, userColor: nextColor, colorChoice: g.colorChoice, opts: { ...g.opts }, extras: { ...g.extras }, tcId: g.tc.id, startFen: g.startFen, practice: g.practice });
  }

  function renderAfter() {
    afterEl.replaceChildren(
      h('button', { type: 'button', class: 'btn btn-primary btn-lg btn-block', html: icon('sparkles') + `<span>${escapeHtml(t('play.after.review'))}</span>`, onClick: async (e) => { const b = e.currentTarget; b.classList.add('loading'); try { await goReview(); } finally { b.classList.remove('loading'); } } }),
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('refresh') + `<span>${escapeHtml(t('play.after.rematch'))}</span>`, onClick: () => rematch() }),
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('robot') + `<span>${escapeHtml(t('play.after.newBot'))}</span>`, onClick: () => onNewBot(bot.id) })),
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('analysis') + `<span>${escapeHtml(t('play.after.analyze'))}</span>`, onClick: () => { location.hash = `#/analysis?fen=${encodeURIComponent(chess.fen())}`; } }),
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('flip') + `<span>${escapeHtml(t('play.actions.flip'))}</span>`, onClick: () => onFlip() })));
  }

  function showGameOver() {
    if (gameOverModal || bag.disposed) return;
    const o = userOutcome();
    goRatingEl = h('div', { class: 'go-rating', role: 'status', 'aria-live': 'polite', hidden: true });
    renderRatingLine();
    const you = h('div', { class: 'go-player' }, avatarNode(ctx.profile && ctx.profile.avatar, 'avatar-lg avatar-round'), h('div', { class: 'semibold' }, (ctx.profile && ctx.profile.name) || t('play.you')));
    const them = h('div', { class: 'go-player' }, botAvatar(bot.id === ADAPTIVE_ID ? { ...bot, category: 'adaptive' } : bot, 'avatar-lg'), h('div', { class: 'semibold' }, bot.name), h('div', { class: 'subtle text-xs' }, (bot.id === ADAPTIVE_ID ? '~' : '') + String(bot.elo ?? '')));
    const scoreText = g.result === '1/2-1/2' ? '½ – ½' : (userOutcome() === 'win' ? '1 – 0' : '0 – 1');
    const chips = h('div', { class: 'row-sm row-wrap go-chips' },
      h('span', { class: 'badge' }, t('play.gameOver.moves', { count: Math.ceil(plies() / 2) })),
      g.openingName ? h('span', { class: 'badge badge-info' }, g.openingName) : null,
      g.hintsUsed ? h('span', { class: 'badge badge-warning' }, t('play.gameOver.hints', { count: g.hintsUsed })) : null,
      g.takebacksUsed ? h('span', { class: 'badge badge-warning' }, t('play.gameOver.takebacks', { count: g.takebacksUsed })) : null);
    const body = h('div', { class: `go-body go-${o}` },
      o === 'win' ? confetti() : null,
      h('div', { class: 'result-hero' },
        h('div', { class: 'go-icon', html: icon(o === 'win' ? 'trophy' : o === 'loss' ? 'flag' : 'handshake') }),
        h('div', { class: 'result-hero-title' }, resultHeadline()),
        h('div', { class: 'result-hero-sub' }, terminationText(g.termination))),
      h('div', { class: 'go-players' }, you, h('div', { class: 'go-score' }, scoreText), them),
      goRatingEl,
      chips,
      h('p', { class: 'muted text-sm text-center go-tip' }, playedPlies() >= 2
        ? (o === 'win' ? t('play.gameOver.tipWin') : t('play.gameOver.tipOther'))
        : t('play.gameOver.tooShort')));
    const actions = [
      { label: t('play.after.newBot'), kind: 'ghost', icon: 'robot', onClick: () => { onNewBot(bot.id); } },
      { label: t('play.after.rematch'), kind: 'secondary', icon: 'refresh', onClick: () => { rematch(); } },
    ];
    if (playedPlies() >= 2) actions.push({ label: t('play.after.review'), kind: 'primary', icon: 'sparkles', autofocus: true, onClick: () => goReview() });
    gameOverModal = modal({ title: t('play.gameOver.title'), body, actions, onClose: () => { gameOverModal = null; goRatingEl = null; } });
  }

  function confetti() {
    const wrap = h('div', { class: 'confetti', 'aria-hidden': 'true' });
    const colors = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--accent)', 'var(--cls-brilliant)', 'var(--cls-mistake)'];
    for (let i = 0; i < 36; i++) {
      wrap.appendChild(h('i', { style: { '--x': `${Math.round(Math.random() * 100)}%`, '--d': `${(Math.random() * 0.6).toFixed(2)}s`, '--r': `${Math.round(Math.random() * 720 - 360)}deg`, '--c': colors[i % colors.length], '--dx': `${Math.round(Math.random() * 120 - 60)}px` } }));
    }
    return wrap;
  }

  // ---- Persistence (resume unfinished games) -----------------------------
  function persist() {
    if (g.result || ended || discarded || playedPlies() <= 0) return;
    // An unconfirmed move is not part of the game yet.
    const moves = pendingConfirm ? g.moves.slice(0, -1) : g.moves;
    if (!moves.length) return;
    saveJson(SAVE_KEY, {
      v: 1, bot, userColor: userC, colorChoice: g.colorChoice, startFen: g.startFen, moves, opts: g.opts, extras: g.extras, tcId: g.tc.id,
      clocks: clock ? clock.getTimes() : null, hintsUsed: g.hintsUsed, takebacksUsed: g.takebacksUsed,
      cls: g.cls, openingName: g.openingName, drawOfferPly: g.drawOfferPly,
      practice: g.practice ? { id: g.practice.id, name: g.practice.name, moves: g.practice.moves, book: g.practice.book, color: g.practice.color } : null,
      updatedAt: new Date().toISOString(),
    });
  }
  bag.on(window, 'pagehide', () => persist());
  bag.on(document, 'visibilitychange', () => { if (document.visibilityState === 'hidden') persist(); });
  // Persist (with current clock times) before the components are torn down.
  bag.add(() => { if (!ended) { persist(); ended = true; } });

  // ---- Start -------------------------------------------------------------
  placeBars();
  if (resume && resume.cls) renderMoveList();
  refreshAll(false);
  renderOpening();
  if (g.practice) { lookupOpening(); renderPracticeNote(); }
  if (resume) {
    if (!g.practice) lookupOpening();
    toast(t('play.resume.restored', { name: bot.name }), 'info', { duration: 2500 });
  }

  if (!checkEnd()) {
    if (!userToMove()) {
      if (!resume) botSay(bot.greeting || t('play.botChat.greetingBotFirst', { name: bot.name }));
      if (clock && playedPlies() > 0) clock.start(colorName(botC));
      requestBotMove();
    } else {
      if (!resume) botSay(bot.greeting || t('play.botChat.greetingUserFirst', { name: bot.name }));
      if (clock && playedPlies() > 0) clock.start(colorName(userC));
      analyzeLive();
      if (g.opts.coach && !playedPlies()) coachSay(t('play.coach.intro', { name: bot.name }));
    }
  }
}
