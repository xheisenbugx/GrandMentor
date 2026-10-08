// GrandMentor — Play a friend (#/local): local two-player "pass-and-play" games on one device.
// Setup screen (names, clock, start position, options) and game screen (board, clocks, captures,
// move list, takeback / draw requests, per-side resign, auto-flip, optional eval bar).
// Finished games are saved to the Library (POST /api/games, bot_id null) and can be reviewed.
// Unfinished games survive a reload (localStorage, SAVE_KEY). Contract: docs/CONTRACT.md "Local two-player".
//
// Leak policy: every view (setup / game) owns a disposables() bag; switching view or unmounting
// disposes it (components destroyed, timers cleared, fetches aborted, sockets closed, dialogs closed).

import { Chess } from '/vendor/chess.js';
import { api, isAbort, EngineClient } from '../api.js';
import { h, icon, toast, modal, confirmDialog, disposables, emptyState, escapeHtml } from '../ui.js';
import { getSettings, pieceUrl } from '../settings.js';
import { Board } from '../components/board.js';
import { createMoveInput } from '../components/moveinput.js';
import { announce } from '../components/announcer.js';
import { EvalBar } from '../components/evalbar.js';
import { MoveList } from '../components/movelist.js';
import { ChessClock } from '../components/clock.js';
import { playSound } from '../components/sound.js';
import { t } from '../i18n.js';

export const title = () => t('local.title');

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
const PREFS_KEY = 'grandmentor.local.prefs.v1';
const SAVE_KEY = 'grandmentor.local.current.v1';
const CSS_HREF = '/css/local.css';
const MAX_NAME = 40;
const MAX_FEN = 120;
const MAX_EVALS = 600;   // bounded eval cache (positions per game)
const MAX_PLIES = 1000;  // hard bound on stored moves

// Same presets as Play (labels resolved at render time).
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

function tcLabel(tc) {
  if (tc.id === 'none') return t('local.tc.none');
  const minutes = Math.round(tc.initial / 60e3);
  return tc.inc ? `${minutes} | ${Math.round(tc.inc / 1e3)}` : t('local.tc.minutes', { count: minutes });
}
const tcSpeed = (tc) => t(`local.tc.speed.${tc.speed}`);

const OPTION_DEFS = [
  { key: 'autoFlip', ic: 'flip' },
  { key: 'showLegal', ic: 'target' },
  { key: 'evalBar', ic: 'chart' },
];

const PIECE_VALUES = { p: 1, n: 3, b: 3, r: 5, q: 9, k: 0 };
const START_COUNTS = { p: 8, n: 2, b: 2, r: 2, q: 1 };
const GLYPHS = { w: { p: '♙', n: '♘', b: '♗', r: '♖', q: '♕' }, b: { p: '♟', n: '♞', b: '♝', r: '♜', q: '♛' } };

// Termination ids (sent to the server untranslated) -> message keys under local.termination.
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
};
const terminationText = (term) => (TERMINATION_KEYS[term] ? t(`local.termination.${TERMINATION_KEYS[term]}`) : String(term || ''));

const colorName = (c) => (c === 'w' ? 'white' : 'black');
const other = (c) => (c === 'w' ? 'b' : 'w');
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const sideLabel = (c) => t(c === 'w' ? 'local.white' : 'local.black');

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

function cleanName(v) {
  return typeof v === 'string' ? v.replace(/\s+/g, ' ').trim().slice(0, MAX_NAME) : '';
}

function isTouchLike() {
  try { return window.matchMedia('(max-width: 1024px), (pointer: coarse)').matches; } catch { return false; }
}

function loadPrefs() {
  const p = loadJson(PREFS_KEY);
  const has = p && typeof p === 'object';
  const opt = (k, dflt) => (has && typeof p[k] === 'boolean' ? p[k] : dflt);
  return {
    stored: !!has,
    white: has ? cleanName(p.white) : '',
    black: has ? cleanName(p.black) : '',
    tc: has && TC_BY_ID[p.tc] ? p.tc : 'none',
    autoFlip: opt('autoFlip', isTouchLike()),
    showLegal: opt('showLegal', getSettings().showLegal !== false),
    evalBar: opt('evalBar', false),
  };
}

function loadSavedGame() {
  const s = loadJson(SAVE_KEY);
  if (!s || s.v !== 1 || !Array.isArray(s.moves) || !s.moves.length || s.moves.length > MAX_PLIES) return null;
  if (typeof s.startFen !== 'string' || !normalizeFen(s.startFen)) return null;
  return s;
}

/** Validate a FEN with chess.js and return the full 6-field FEN, or null. */
function normalizeFen(raw) {
  let fen = String(raw || '').trim().replace(/\s+/g, ' ');
  if (!fen || fen.length > MAX_FEN) return null;
  const parts = fen.split(' ');
  if (parts.length === 4) fen += ' 0 1';
  else if (parts.length === 5) fen += ' 1';
  try { return new Chess(fen).fen(); } catch { return null; }
}

function isFinished(fen) {
  try { const c = new Chess(fen); return c.isGameOver(); } catch { return true; }
}

function parseUci(uci) {
  if (typeof uci !== 'string' || !/^[a-h][1-8][a-h][1-8][qrbn]?$/.test(uci)) return null;
  return { from: uci.slice(0, 2), to: uci.slice(2, 4), promotion: uci[4] || undefined };
}

/** Inject /css/local.css once; resolves when loaded (or after a short timeout). */
function ensureCss() {
  if (document.querySelector(`link[href="${CSS_HREF}"]`)) return Promise.resolve();
  return new Promise((resolve) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = CSS_HREF;
    link.dataset.pageCss = 'local';
    let done = false;
    const finish = () => { if (!done) { done = true; clearTimeout(timer); resolve(); } };
    const timer = setTimeout(finish, 800);
    link.addEventListener('load', finish, { once: true });
    link.addEventListener('error', finish, { once: true });
    document.head.appendChild(link);
  });
}

function sideAvatar(c, cls = '') {
  return h('div', { class: `lc-avatar ${c === 'w' ? 'white' : 'black'} ${cls}`.trim(), 'aria-hidden': 'true' },
    h('img', { src: pieceUrl(c === 'w' ? 'wK' : 'bK'), alt: '', draggable: 'false' }));
}

function isTyping(e) {
  const el = e.target;
  if (!el || !(el instanceof Element)) return false;
  return el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName);
}

function playSoundSafe(name) {
  try { playSound(name); } catch { /* ignore */ }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export async function mount(root, { query = {} } = {}) {
  const bag = disposables();
  const ac = new AbortController();
  bag.add(() => ac.abort());

  const host = h('div', { class: 'lc-root' });
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
      console.error('[local] view failed', e);
      host.replaceChildren(h('div', { class: 'page' }, emptyState({
        icon: 'alert',
        title: t('local.error.title'),
        text: t('local.error.text'),
        action: { label: t('local.error.back'), icon: 'users', onClick: () => { removeKey(SAVE_KEY); showSetup(null); } },
      })));
    }
  };
  bag.add(() => { if (viewDispose) { const d = viewDispose; viewDispose = null; d(); } });

  const ctx = { profile: null, onProfile: null };

  await ensureCss();
  if (bag.disposed) return bag.dispose;

  api.get('/api/profile', { signal: ac.signal }).then((p) => {
    ctx.profile = p && typeof p === 'object' ? p : null;
    if (ctx.onProfile) ctx.onProfile();
  }).catch(() => {});

  const showSetup = (linkFen) => setView(() => renderSetup(host, ctx, {
    linkFen,
    onPlay: (cfg) => showGame(cfg),
    onResume: (saved) => showGame({ resume: saved }),
  }));
  const showGame = (cfg) => setView(() => renderGame(host, ctx, cfg, {
    onNewGame: () => showSetup(null),
    onRematch: (next) => showGame(next),
  }));

  const linkFen = typeof query.fen === 'string' && query.fen ? query.fen : null;
  showSetup(linkFen);
  return bag.dispose;
}

// ---------------------------------------------------------------------------
// Setup screen
// ---------------------------------------------------------------------------
function renderSetup(host, ctx, { linkFen, onPlay, onResume }) {
  const bag = disposables();
  const prefs = loadPrefs();
  let saved = loadSavedGame();

  const linkNorm = linkFen ? normalizeFen(linkFen) : null;
  const state = {
    tc: prefs.tc,
    custom: !!linkFen,
    fenText: linkFen ? String(linkFen).slice(0, MAX_FEN) : '',
    opts: { autoFlip: prefs.autoFlip, showLegal: prefs.showLegal, evalBar: prefs.evalBar },
  };
  if (linkFen && !linkNorm) toast(t('local.setup.fenLinkInvalid'), 'warning');

  // ---- Left: preview board ------------------------------------------------
  const previewBoardEl = h('div', { class: 'lc-preview-board' });
  const previewCaption = h('div', { class: 'lc-preview-caption' });
  const preview = h('section', { class: 'lc-preview', 'aria-hidden': 'true' },
    h('div', { class: 'lc-preview-hero' },
      h('div', { class: 'lc-preview-pair' }, sideAvatar('w', 'lg'), sideAvatar('b', 'lg')),
      h('div', { class: 'stack-sm', style: 'min-width:0' },
        h('div', { class: 'lc-preview-title' }, t('local.setup.heroTitle')),
        h('div', { class: 'muted text-sm' }, t('local.setup.heroText')))),
    previewBoardEl,
    previewCaption);

  let previewBoard = null;
  try {
    const s = getSettings();
    previewBoard = new Board(previewBoardEl, { fen: START_FEN, orientation: 'white', interactive: false, movableColor: null, showCoords: s.showCoords, sounds: false, animationMs: s.animationMs, keyboard: false, announce: false });
    bag.add(() => previewBoard.destroy());
  } catch (e) { console.warn('[local] preview board unavailable', e); }

  // ---- Right: panel -------------------------------------------------------
  const resumeSlot = h('div');

  const mkNameInput = (c) => h('input', {
    class: 'input lc-name-input', type: 'text', maxlength: String(MAX_NAME), autocomplete: 'off', spellcheck: 'false',
    placeholder: sideLabel(c), 'aria-label': t(c === 'w' ? 'local.setup.whiteName' : 'local.setup.blackName'),
    value: c === 'w' ? prefs.white : prefs.black,
  });
  const whiteInput = mkNameInput('w');
  const blackInput = mkNameInput('b');
  const swapBtn = h('button', {
    type: 'button', class: 'btn btn-ghost btn-icon lc-swap', 'aria-label': t('local.setup.swap'), 'data-tooltip': t('local.setup.swap'), html: icon('flip'),
    onClick: () => { const w = whiteInput.value; whiteInput.value = blackInput.value; blackInput.value = w; },
  });
  const players = h('div', { class: 'lc-players' },
    h('label', { class: 'lc-player-row' }, sideAvatar('w'), h('span', { class: 'lc-player-side' }, sideLabel('w')), whiteInput),
    swapBtn,
    h('label', { class: 'lc-player-row' }, sideAvatar('b'), h('span', { class: 'lc-player-side' }, sideLabel('b')), blackInput));

  // Prefill White with the profile name the first time (players can swap or edit it).
  const fillProfileName = () => {
    const name = cleanName(ctx.profile && ctx.profile.name);
    if (!prefs.stored && name && !whiteInput.value && !blackInput.value) whiteInput.value = name;
  };
  ctx.onProfile = () => { if (!bag.disposed) fillProfileName(); };
  bag.add(() => { ctx.onProfile = null; });
  fillProfileName();

  // Time control
  const tcBtns = {};
  const tcRow = h('div', { class: 'lc-tc-grid', role: 'radiogroup', 'aria-label': t('local.setup.timeAria') },
    ...TIME_CONTROLS.map((tc) => (tcBtns[tc.id] = h('button', { type: 'button', class: 'lc-tc-btn', role: 'radio', onClick: () => { state.tc = tc.id; refresh(); } },
      h('span', { class: 'lc-tc-label' }, tcLabel(tc)), h('span', { class: 'lc-tc-sub' }, tcSpeed(tc))))));

  // Start position
  const posBtns = {
    standard: h('button', { type: 'button', role: 'radio', onClick: () => { state.custom = false; refresh(); } }, t('local.setup.standard')),
    custom: h('button', { type: 'button', role: 'radio', onClick: () => { state.custom = true; refresh(); bag.raf(() => fenInput.focus()); } }, t('local.setup.custom')),
  };
  const posSeg = h('div', { class: 'segmented block', role: 'radiogroup', 'aria-label': t('local.setup.position') }, posBtns.standard, posBtns.custom);
  const fenInput = h('textarea', {
    class: 'textarea mono lc-fen', rows: '2', maxlength: String(MAX_FEN), spellcheck: 'false', autocomplete: 'off',
    placeholder: START_FEN, 'aria-label': t('local.setup.fenLabel'),
  });
  fenInput.value = state.fenText;
  const fenMsg = h('div', { class: 'help lc-fen-msg', role: 'status', 'aria-live': 'polite' });
  const fenBox = h('div', { class: 'field lc-fen-box' }, h('label', { class: 'label' }, t('local.setup.fenLabel')), fenInput, fenMsg);
  bag.on(fenInput, 'input', () => { state.fenText = fenInput.value; refresh(); });

  // Options
  const optInputs = {};
  const optionList = h('div', { class: 'lc-options' }, ...OPTION_DEFS.map((d) => {
    const input = h('input', { type: 'checkbox', onChange: () => { state.opts[d.key] = input.checked; } });
    input.checked = !!state.opts[d.key];
    optInputs[d.key] = input;
    return h('label', { class: 'lc-option' },
      h('span', { class: 'lc-option-icon', html: icon(d.ic) }),
      h('span', { class: 'lc-option-text' }, h('span', { class: 'semibold' }, t(`local.options.${d.key}.title`)), h('span', { class: 'subtle text-xs' }, t(`local.options.${d.key}.desc`))),
      h('span', { class: 'switch' }, input, h('span', { class: 'switch-track' })));
  }));

  const playBtn = h('button', { type: 'button', class: 'btn btn-primary btn-xl btn-block lc-cta', html: icon('play') + `<span>${escapeHtml(t('local.setup.start'))}</span>`, onClick: () => start() });

  const panel = h('aside', { class: 'lc-setup-panel card card-flush' },
    h('div', { class: 'lc-setup-head' }, h('div', { class: 'page-header-icon', html: icon('users') }),
      h('div', null, h('h1', { class: 'page-title' }, t('local.title')), h('p', { class: 'page-subtitle' }, t('local.subtitle')))),
    h('div', { class: 'lc-setup-scroll' },
      resumeSlot,
      h('h2', { class: 'lc-section-title' }, t('local.setup.players')),
      players,
      h('h2', { class: 'lc-section-title' }, t('local.setup.time')),
      tcRow,
      h('h2', { class: 'lc-section-title' }, t('local.setup.position')),
      posSeg,
      fenBox,
      h('h2', { class: 'lc-section-title' }, t('local.setup.options')),
      optionList),
    h('div', { class: 'lc-setup-foot' }, playBtn));

  host.appendChild(h('div', { class: 'lc-setup page-enter' }, preview, panel));

  /** Current start FEN, or null when the custom FEN is invalid / finished. */
  function chosenFen() {
    if (!state.custom) return START_FEN;
    const fen = normalizeFen(state.fenText);
    if (!fen || isFinished(fen)) return null;
    return fen;
  }

  function refresh() {
    for (const [id, btn] of Object.entries(tcBtns)) {
      btn.classList.toggle('active', state.tc === id);
      btn.setAttribute('aria-checked', state.tc === id ? 'true' : 'false');
    }
    for (const [id, btn] of Object.entries(posBtns)) {
      const on = (id === 'custom') === state.custom;
      btn.classList.toggle('active', on);
      btn.setAttribute('aria-checked', on ? 'true' : 'false');
    }
    fenBox.hidden = !state.custom;
    let fen = START_FEN;
    let ok = true;
    fenMsg.className = 'help lc-fen-msg';
    if (state.custom) {
      const norm = normalizeFen(state.fenText);
      if (!state.fenText.trim()) { ok = false; fenMsg.textContent = t('local.setup.fenHelp'); }
      else if (!norm) { ok = false; fenMsg.textContent = t('local.setup.fenInvalid'); fenMsg.classList.add('text-danger'); }
      else if (isFinished(norm)) { ok = false; fenMsg.textContent = t('local.setup.fenOver'); fenMsg.classList.add('text-danger'); }
      else {
        fen = norm;
        fenMsg.textContent = t(norm.split(' ')[1] === 'b' ? 'local.setup.fenOkBlack' : 'local.setup.fenOkWhite');
        fenMsg.classList.add('lc-ok');
      }
    }
    playBtn.disabled = !ok;
    previewCaption.textContent = state.custom && ok ? t('local.setup.previewCustom') : t('local.setup.previewStandard');
    if (previewBoard) { try { previewBoard.setPosition(fen, { animate: false, sound: false }); } catch { /* ignore */ } }
  }

  function renderResume() {
    resumeSlot.replaceChildren();
    if (!saved) return;
    const moveCount = Math.ceil(saved.moves.length / 2);
    resumeSlot.appendChild(h('div', { class: 'lc-resume' },
      h('div', { class: 'lc-preview-pair sm' }, sideAvatar('w'), sideAvatar('b')),
      h('div', { class: 'lc-resume-text' },
        h('div', { class: 'semibold lc-resume-title' }, t('local.resume.title', { white: cleanName(saved.white) || sideLabel('w'), black: cleanName(saved.black) || sideLabel('b') })),
        h('div', { class: 'subtle text-xs' }, t('local.resume.detail', { count: moveCount }))),
      h('button', { type: 'button', class: 'btn btn-primary btn-sm', html: icon('play') + `<span>${escapeHtml(t('local.resume.resume'))}</span>`, onClick: () => onResume(saved) }),
      h('button', {
        type: 'button', class: 'btn btn-ghost btn-icon btn-sm', 'aria-label': t('local.resume.discardAria'), 'data-tooltip': t('local.resume.discard'), html: icon('trash'),
        onClick: async () => {
          const ok = await confirmDialog({ title: t('local.resume.discardTitle'), message: t('local.resume.discardMessage'), confirmLabel: t('local.resume.discard'), danger: true });
          if (!ok || bag.disposed) return;
          removeKey(SAVE_KEY);
          saved = null;
          renderResume();
        },
      })));
  }

  function start() {
    const fen = chosenFen();
    if (!fen) { refresh(); fenInput.focus(); return; }
    const white = cleanName(whiteInput.value);
    const black = cleanName(blackInput.value);
    const proceed = () => {
      saveJson(PREFS_KEY, { white, black, tc: state.tc, ...state.opts });
      onPlay({ white, black, startFen: fen, tcId: state.tc, opts: { ...state.opts } });
    };
    if (saved) {
      confirmDialog({ title: t('local.resume.replaceTitle'), message: t('local.resume.replaceMessage'), confirmLabel: t('local.resume.replaceConfirm') })
        .then((ok) => { if (ok && !bag.disposed) { removeKey(SAVE_KEY); saved = null; proceed(); } });
      return;
    }
    proceed();
  }

  bag.on(panel, 'keydown', (e) => {
    if (e.key === 'Enter' && e.target instanceof HTMLInputElement && e.target.type === 'text') { e.preventDefault(); start(); }
  });

  renderResume();
  refresh();
  return bag.dispose;
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

function buildGame(bag, host, ctx, cfg, { onNewGame, onRematch }) {
  const resume = cfg.resume || null;
  const src = resume || cfg;
  const g = {
    names: { w: cleanName(src.white), b: cleanName(src.black) },
    startFen: normalizeFen(src.startFen) || START_FEN,
    tc: TC_BY_ID[src.tcId] || TC_BY_ID.none,
    opts: { autoFlip: false, showLegal: true, evalBar: false, ...(src.opts && typeof src.opts === 'object' ? src.opts : {}) },
    moves: [], sans: [], fens: [], lastMoves: [],
    result: null, termination: '',
    takebacks: resume ? (resume.takebacks | 0) : 0,
    savedId: null,
    savePromise: null,
  };
  const nameOf = (c) => g.names[c] || sideLabel(c);
  const isDefaultName = (c) => !g.names[c] || g.names[c] === sideLabel(c);

  const chess = new Chess(g.startFen);
  g.fens.push(chess.fen());
  g.lastMoves.push(null);
  if (resume) {
    for (const uci of resume.moves) {
      const p = parseUci(uci);
      let mv = null;
      if (p) { try { mv = chess.move(p); } catch { mv = null; } }
      if (!mv) break;
      g.moves.push(uci); g.sans.push(mv.san); g.fens.push(chess.fen()); g.lastMoves.push([mv.from, mv.to]);
    }
  }

  const plies = () => g.moves.length;
  let viewPly = null;           // null = live position
  let asking = false;           // a takeback / draw / resign dialog is open
  let flipTimer = null;
  let gameOverModal = null;
  const dialogs = new Set();
  let ended = false;
  let discarded = false;
  const evals = new Map();
  let engine = null;
  let evalbar = null;

  // ---- DOM ---------------------------------------------------------------
  const s = getSettings();
  const mkBar = (c) => {
    const captures = h('div', { class: 'player-captures lc-captures' });
    const clockSlot = h('div', { class: 'clock-slot' });
    const pill = h('span', { class: 'lc-turn-pill' }, t('local.turn.pill'));
    const el = h('div', { class: `player-bar lc-bar ${c === 'w' ? 'is-white' : 'is-black'}` },
      sideAvatar(c),
      h('div', { class: 'lc-bar-main' },
        h('div', { class: 'player-name lc-bar-name' }, h('span', { class: 'truncate' }, nameOf(c)), pill),
        captures),
      clockSlot);
    return { el, captures, clockSlot };
  };
  const bars = { w: mkBar('w'), b: mkBar('b') };
  const evalSlot = h('div', { class: 'evalbar-slot' });
  const boardSlot = h('div', { class: 'board-slot' });
  const boardRow = h('div', { class: 'board-row' }, evalSlot, boardSlot);
  const main = h('div', { class: 'game-main' });

  const turnEl = h('div', { class: 'lc-turn', role: 'status', 'aria-live': 'polite' });
  const moveListEl = h('div', { class: 'panel-body lc-moves' });
  const navBtn = (ic, label, fn) => h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': label, 'data-tooltip': label, html: icon(ic), onClick: fn });
  const nav = {
    first: navBtn('first', t('local.nav.first'), () => goto(0)),
    prev: navBtn('chevron-left', t('local.nav.prev'), () => goto(currentPly() - 1)),
    next: navBtn('chevron-right', t('local.nav.next'), () => goto(currentPly() + 1)),
    last: navBtn('last', t('local.nav.last'), () => goto(plies())),
  };
  const navBar = h('div', { class: 'toolbar lc-nav' }, nav.first, nav.prev, h('div', { class: 'spacer' }), nav.next, nav.last);

  const headBadges = h('div', { class: 'row-sm' },
    g.tc.id !== 'none' ? h('span', { class: 'badge', html: icon('clock', { size: 12 }) + ' ' + escapeHtml(tcLabel(g.tc)) }) : null,
    g.startFen !== START_FEN ? h('span', { class: 'badge badge-info' }, t('local.game.customBadge')) : null);

  const actionBtn = (ic, label, fn, extra = '') => h('button', { type: 'button', class: `btn btn-secondary lc-action ${extra}`.trim(), onClick: fn, html: icon(ic) + `<span>${escapeHtml(label)}</span>` });
  const acts = {
    takeback: actionBtn('undo', t('local.actions.takeback'), () => onTakeback()),
    draw: actionBtn('handshake', t('local.actions.draw'), () => onOfferDraw()),
    resign: actionBtn('flag', t('local.actions.resign'), () => onResign(), 'act-resign'),
    flip: actionBtn('flip', t('local.actions.flip'), () => onFlip()),
    evalBar: actionBtn('chart', t('local.actions.eval'), () => toggleEval(), 'act-eval'),
    newGame: actionBtn('plus', t('local.actions.newGame'), () => onNew()),
  };
  acts.takeback.setAttribute('aria-label', t('local.actions.takebackAria'));
  acts.draw.setAttribute('aria-label', t('local.actions.drawAria'));
  acts.evalBar.setAttribute('aria-label', t('local.actions.evalAria'));
  acts.newGame.setAttribute('aria-label', t('local.actions.newGameAria'));
  const actionsEl = h('div', { class: 'lc-actions' }, ...Object.values(acts));
  const afterEl = h('div', { class: 'lc-after', hidden: true });

  let board = null;
  const moveInput = createMoveInput({ board: () => board });
  bag.add(() => moveInput.destroy());
  const panel = h('aside', { class: 'game-panel' },
    turnEl,
    h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header lc-panel-head' }, h('span', { html: icon('list') }), h('span', null, t('local.game.moves')), h('div', { class: 'spacer' }), headBadges),
      moveListEl,
      navBar),
    moveInput.el,
    afterEl,
    actionsEl);

  const layout = h('div', { class: 'game-layout lc-game page-enter' }, main, panel);
  host.appendChild(layout);

  // ---- Components --------------------------------------------------------
  const initialOrientation = g.opts.autoFlip ? colorName(chess.turn())
    : (resume && resume.orientation === 'black' ? 'black' : 'white');
  board = new Board(boardSlot, {
    fen: chess.fen(),
    orientation: initialOrientation,
    interactive: true,
    movableColor: 'both',
    showCoords: s.showCoords, showLegal: !!g.opts.showLegal, animationMs: s.animationMs, sounds: s.sounds,
    onMove: (m) => onBoardMove(m),
  });
  bag.add(() => board.destroy());

  const movelist = new MoveList(moveListEl, { onSelect: (ply) => goto(ply), emptyText: t('local.game.noMoves') });
  bag.add(() => movelist.destroy());

  let clock = null;
  if (g.tc.id !== 'none') {
    const initial = resume && resume.clocks && Number.isFinite(resume.clocks.white) && Number.isFinite(resume.clocks.black)
      ? { white: Math.max(0, resume.clocks.white), black: Math.max(0, resume.clocks.black) } : g.tc.initial;
    clock = new ChessClock(null, {
      initialMs: initial, incrementMs: g.tc.inc,
      slots: { white: bars.w.clockSlot, black: bars.b.clockSlot },
      onFlag: (c) => onFlag(c),
      onLowTime: () => playSoundSafe('notify'),
    });
    bag.add(() => clock.destroy());
  }

  bag.add(() => {
    if (flipTimer) { clearTimeout(flipTimer); flipTimer = null; }
    for (const d of [...dialogs]) d.close();
    dialogs.clear();
    if (gameOverModal) gameOverModal.close();
  });

  // ---- Eval bar (optional, engine WebSocket) ------------------------------
  function enableEval(on) {
    g.opts.evalBar = !!on;
    layout.classList.toggle('no-eval', !g.opts.evalBar);
    acts.evalBar.classList.toggle('active', g.opts.evalBar);
    acts.evalBar.setAttribute('aria-pressed', g.opts.evalBar ? 'true' : 'false');
    if (g.opts.evalBar) {
      if (!evalbar) {
        evalbar = new EvalBar(evalSlot, { orientation: board.orientation });
        evalbar.set(null);
      }
      if (!engine) engine = new EngineClient();
      refreshEval();
    } else {
      if (engine) { engine.close(); engine = null; }
      if (evalbar) { evalbar.destroy(); evalbar = null; }
      evalSlot.replaceChildren();
    }
  }
  bag.add(() => { if (engine) engine.close(); engine = null; if (evalbar) evalbar.destroy(); evalbar = null; });

  function toggleEval() {
    enableEval(!g.opts.evalBar);
    persist();
  }

  function rememberEval(fen, score, opts) {
    if (!fen || !score) return;
    evals.delete(fen);
    evals.set(fen, { score, opts: opts || null });
    if (evals.size > MAX_EVALS) evals.delete(evals.keys().next().value);
  }

  function refreshEval() {
    if (!evalbar) return;
    const fen = g.fens[currentPly()];
    const hit = evals.get(fen);
    if (hit) evalbar.set(hit.score, hit.opts || undefined);
    if (!engine) return;
    if (hit && (g.result || hit.final)) { engine.stop(); return; }
    try {
      engine.analyze(fen, { multipv: 1, movetime_ms: 2500 }, (info, done) => {
        const line = info && info.lines && info.lines[0];
        if (!line || !line.score) return;
        rememberEval(fen, line.score);
        if (done) { const e = evals.get(fen); if (e) e.final = true; }
        if (evalbar && g.fens[currentPly()] === fen) evalbar.set(line.score);
      }, () => { /* engine hiccup: the bar just stays where it was */ });
    } catch (e) { console.warn('[local] engine', e); }
  }

  // ---- Rendering helpers -------------------------------------------------
  function currentPly() { return viewPly === null ? plies() : viewPly; }
  function isLive() { return viewPly === null; }

  function placeBars() {
    const bottom = board.orientation === 'black' ? 'b' : 'w';
    main.replaceChildren(bars[other(bottom)].el, boardRow, bars[bottom].el);
  }

  function updateInteractivity() {
    const can = isLive() && !g.result && !asking;
    try { board.setInteractive(can, can ? 'both' : null); } catch { /* ignore */ }
  }

  function renderCaptures() {
    const counts = { w: { p: 0, n: 0, b: 0, r: 0, q: 0, k: 0 }, b: { p: 0, n: 0, b: 0, r: 0, q: 0, k: 0 } };
    const p = currentPly();
    let pos = chess;
    if (p !== plies()) { try { pos = new Chess(g.fens[p]); } catch { pos = chess; } }
    for (const row of pos.board()) for (const sq of row) if (sq) counts[sq.color][sq.type]++;
    const mat = { w: 0, b: 0 };
    for (const c of ['w', 'b']) for (const k of Object.keys(PIECE_VALUES)) mat[c] += PIECE_VALUES[k] * counts[c][k];
    for (const c of ['w', 'b']) {
      const opp = other(c);
      const frag = document.createDocumentFragment();
      for (const k of ['p', 'b', 'n', 'r', 'q']) {
        const missing = Math.max(0, START_COUNTS[k] - counts[opp][k]);
        if (!missing) continue;
        frag.appendChild(h('span', { class: `lc-cap lc-cap-${opp}` }, GLYPHS[opp][k].repeat(Math.min(missing, 9))));
      }
      const diff = mat[c] - mat[opp];
      if (diff > 0) frag.appendChild(h('span', { class: 'lc-cap-diff' }, `+${diff}`));
      bars[c].captures.replaceChildren(frag);
    }
  }

  function renderMoveList() {
    const parts = g.startFen.split(' ');
    movelist.setMoves(g.sans.map((san) => ({ san })), { startColor: parts[1] === 'b' ? 'black' : 'white', startMoveNumber: Number(parts[5]) || 1 });
    movelist.setCurrent(currentPly());
  }

  function renderNav() {
    const p = currentPly();
    nav.first.disabled = p <= 0;
    nav.prev.disabled = p <= 0;
    nav.next.disabled = p >= plies();
    nav.last.disabled = p >= plies();
  }

  function renderBars() {
    const turn = g.result ? null : chess.turn();
    for (const c of ['w', 'b']) bars[c].el.classList.toggle('is-turn', isLive() && turn === c);
  }

  let lastTurnKey = '';
  function renderTurn() {
    turnEl.className = 'lc-turn';
    turnEl.replaceChildren();
    let key;
    if (g.result) {
      key = 'done';
      turnEl.classList.add('done');
      const win = g.result === '1-0' ? 'w' : g.result === '0-1' ? 'b' : null;
      turnEl.append(
        win ? sideAvatar(win, 'turn') : h('span', { class: 'lc-turn-icon', html: icon('handshake') }),
        h('div', { class: 'lc-turn-text' },
          h('div', { class: 'lc-turn-title' }, resultHeadline()),
          h('div', { class: 'lc-turn-sub' }, terminationText(g.termination))));
    } else if (!isLive()) {
      key = 'browse';
      turnEl.classList.add('browsing');
      turnEl.append(
        h('span', { class: 'lc-turn-icon', html: icon('eye') }),
        h('div', { class: 'lc-turn-text' },
          h('div', { class: 'lc-turn-title' }, t('local.turn.viewing')),
          h('div', { class: 'lc-turn-sub' }, t('local.turn.viewingSub'))),
        h('button', { type: 'button', class: 'btn btn-sm btn-primary', onClick: () => goto(plies()) }, t('local.turn.back')));
    } else {
      const c = chess.turn();
      const check = chess.inCheck();
      key = `${c}:${plies()}`;
      turnEl.classList.add(c === 'w' ? 'is-white' : 'is-black');
      if (check) turnEl.classList.add('check');
      let sub;
      if (plies() === 0) sub = t('local.turn.start', { name: nameOf(c) });
      else if (!isDefaultName(c)) sub = t('local.turn.pass', { name: nameOf(c) });
      else sub = t('local.turn.passGeneric');
      turnEl.append(
        sideAvatar(c, 'turn'),
        h('div', { class: 'lc-turn-text' },
          h('div', { class: 'lc-turn-title' }, t(`local.turn.${check ? 'inCheck' : 'toMove'}.${colorName(c)}`)),
          h('div', { class: 'lc-turn-sub' }, sub)));
    }
    if (key !== lastTurnKey) {
      lastTurnKey = key;
      turnEl.classList.add('pop');
    }
  }

  function renderActions() {
    const over = !!g.result;
    acts.takeback.disabled = over || asking || plies() === 0;
    acts.draw.disabled = over || asking || plies() < 2;
    acts.resign.disabled = over || asking;
    actionsEl.hidden = over;
    afterEl.hidden = !over;
  }

  function syncBoard(animate = true) {
    const p = currentPly();
    try { board.setPosition(g.fens[p], { animate, lastMove: g.lastMoves[p] || null }); } catch (e) { console.error(e); }
    movelist.setCurrent(p);
    renderCaptures();
    renderNav();
    renderBars();
    renderTurn();
    renderActions();
    updateInteractivity();
  }

  function refreshAll(animate = true) {
    renderMoveList();
    syncBoard(animate);
  }

  function setOrientation(color) {
    if (board.orientation === color) return;
    try { board.setOrientation(color); } catch { return; }
    if (evalbar) evalbar.setOrientation(color);
    placeBars();
  }

  /** With auto-flip on, turn the board to the side to move once the move has landed. */
  function scheduleAutoFlip(delay) {
    if (!g.opts.autoFlip || g.result) return;
    if (flipTimer) clearTimeout(flipTimer);
    flipTimer = setTimeout(() => {
      flipTimer = null;
      if (bag.disposed || g.result || !isLive()) return;
      setOrientation(colorName(chess.turn()));
    }, delay);
  }

  // ---- Moves -------------------------------------------------------------
  function onBoardMove(m) {
    if (!m || !isLive() || g.result || asking || plies() >= MAX_PLIES) return false;
    let mv = null;
    try { mv = chess.move({ from: m.from, to: m.to, promotion: m.promotion || undefined }); } catch { mv = null; }
    if (!mv) return false;
    g.moves.push(mv.from + mv.to + (mv.promotion || ''));
    g.sans.push(mv.san);
    g.fens.push(chess.fen());
    g.lastMoves.push([mv.from, mv.to]);
    viewPly = null;
    pressClock(mv.color);
    // Let the board finish its own move handling before we sync it.
    queueMicrotask(() => {
      if (bag.disposed) return;
      refreshAll(false);
      if (checkEnd()) return;
      refreshEval();
      persist();
      scheduleAutoFlip((Number(getSettings().animationMs) || 200) + 450);
    });
    return true;
  }

  function pressClock(mover) {
    if (!clock || g.result) return;
    if (!clock.running) clock.start(colorName(other(mover)));
    else clock.press();
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
    let handled = true;
    switch (e.key) {
      case 'ArrowLeft': goto(currentPly() - 1); break;
      case 'ArrowRight': goto(currentPly() + 1); break;
      case 'Home': case 'ArrowUp': goto(0); break;
      case 'End': case 'ArrowDown': goto(plies()); break;
      case 'f': case 'F': onFlip(); break;
      default: handled = false;
    }
    if (handled) e.preventDefault();
  });

  // ---- Requests between players (takeback, draw, resign) ------------------
  /** Pause the clock and block the board while the players decide. */
  function beginAsk() {
    asking = true;
    if (clock) clock.pause();
    renderActions(); updateInteractivity();
  }
  function endAsk() {
    asking = false;
    if (bag.disposed || g.result) return;
    if (clock && plies() > 0 && !clock.flagged) clock.start(colorName(chess.turn()));
    renderActions(); updateInteractivity();
  }

  function ask({ title, message, ic, yes, no, danger = false }) {
    return new Promise((resolve) => {
      let answer = false;
      const body = h('div', { class: 'lc-ask' }, h('div', { class: 'lc-ask-icon', html: icon(ic) }), h('p', null, message));
      const m = modal({
        title, body,
        actions: [
          { label: no, kind: 'ghost' },
          { label: yes, kind: danger ? 'danger' : 'primary', autofocus: true, onClick: () => { answer = true; } },
        ],
        onClose: () => { dialogs.delete(m); resolve(answer); },
      });
      dialogs.add(m);
    });
  }

  async function onTakeback() {
    if (g.result || asking) return;
    if (!plies()) { toast(t('local.takeback.nothing'), 'info'); return; }
    if (!isLive()) goto(plies());
    const asker = other(chess.turn());   // the player who made the last move
    const decider = chess.turn();
    const san = g.sans[plies() - 1];
    beginAsk();
    playSoundSafe('notify');
    const ok = await ask({
      title: t('local.takeback.title', { name: nameOf(decider) }),
      message: t('local.takeback.message', { asker: nameOf(asker), san }),
      ic: 'undo', yes: t('local.takeback.allow'), no: t('local.takeback.decline'),
    });
    if (bag.disposed || g.result) return;
    if (!ok) { endAsk(); toast(t('local.takeback.declined'), 'info'); return; }
    chess.undo();
    g.moves.pop(); g.sans.pop(); g.fens.pop(); g.lastMoves.pop();
    g.takebacks++;
    viewPly = null;
    endAsk();
    if (clock && plies() === 0) { clock.pause(); }
    refreshAll(true);
    if (g.opts.autoFlip) setOrientation(colorName(chess.turn()));
    refreshEval();
    persist();
    if (!plies()) removeKey(SAVE_KEY);
    toast(t('local.takeback.done', { name: nameOf(asker) }), 'success', { duration: 2000 });
  }

  async function onOfferDraw() {
    if (g.result || asking || plies() < 2) return;
    const offerer = chess.turn();
    const decider = other(offerer);
    beginAsk();
    playSoundSafe('notify');
    const ok = await ask({
      title: t('local.draw.title', { name: nameOf(decider) }),
      message: t('local.draw.message', { offerer: nameOf(offerer) }),
      ic: 'handshake', yes: t('local.draw.accept'), no: t('local.draw.decline'),
    });
    if (bag.disposed || g.result) return;
    if (ok) { asking = false; endGame('1/2-1/2', 'agreement'); return; }
    endAsk();
    toast(t('local.draw.declined'), 'info');
  }

  function onResign() {
    if (g.result || asking) return;
    beginAsk();
    let who = null;
    const sideBtn = (c) => h('button', {
      type: 'button', class: `lc-resign-btn ${c === 'w' ? 'white' : 'black'}`,
      onClick: () => { who = c; m.close(); },
    }, sideAvatar(c), h('span', { class: 'lc-resign-text' },
      h('span', { class: 'semibold truncate' }, t('local.resign.side', { name: nameOf(c) })),
      h('span', { class: 'subtle text-xs' }, t('local.resign.winner', { name: nameOf(other(c)) }))),
    h('span', { html: icon('flag') }));
    const body = h('div', { class: 'stack-sm' },
      h('p', { class: 'muted text-sm', style: 'margin:0' }, t('local.resign.message')),
      sideBtn(chess.turn()), sideBtn(other(chess.turn())));
    const m = modal({
      title: t('local.resign.title'), body,
      actions: [{ label: t('local.resign.cancel'), kind: 'ghost' }],
      onClose: () => {
        dialogs.delete(m);
        if (bag.disposed || g.result) return;
        if (who) { asking = false; endGame(who === 'w' ? '0-1' : '1-0', 'resignation'); } else endAsk();
      },
    });
    dialogs.add(m);
  }

  function onFlip() {
    setOrientation(board.orientation === 'white' ? 'black' : 'white');
    persist();
  }

  async function onNew() {
    if (!g.result && plies() >= 1) {
      const ok = await confirmDialog({ title: t('local.newGame.title'), message: t('local.newGame.message'), confirmLabel: t('local.newGame.confirm'), danger: true });
      if (!ok || bag.disposed) return;
    }
    if (!g.result) { discarded = true; removeKey(SAVE_KEY); }
    onNewGame();
  }

  function onFlag(color) {
    if (g.result) return;
    const flagged = color === 'white' ? 'w' : 'b';
    const winner = other(flagged);
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

  function resultHeadline() {
    if (g.result === '1-0') return t('local.result.wins', { name: nameOf('w') });
    if (g.result === '0-1') return t('local.result.wins', { name: nameOf('b') });
    return t('local.result.draw');
  }

  function endGame(result, termination) {
    if (g.result) return;
    g.result = result;
    g.termination = termination;
    if (clock) clock.pause();
    if (flipTimer) { clearTimeout(flipTimer); flipTimer = null; }
    viewPly = null;
    removeKey(SAVE_KEY);
    if (result === '1/2-1/2') rememberEval(chess.fen(), { cp: 0 });
    else if (termination === 'checkmate') rememberEval(chess.fen(), { mate: 0 }, { mated: result === '1-0' ? 'black' : 'white' });
    refreshAll(false);
    refreshEval();
    announce(t(result === '1-0' ? 'a11y.result.whiteWins' : result === '0-1' ? 'a11y.result.blackWins' : 'a11y.result.draw'), { assertive: true });
    playSoundSafe('gameEnd');
    g.savePromise = saveGame();
    renderAfter();
    bag.timeout(() => { if (!bag.disposed) showGameOver(); }, termination === 'checkmate' ? 650 : 250);
  }

  function userColor() {
    const me = cleanName(ctx.profile && ctx.profile.name);
    if (!me) return null;
    if (g.names.w === me && g.names.b !== me) return 'white';
    if (g.names.b === me && g.names.w !== me) return 'black';
    return null;
  }

  async function saveGame() {
    if (g.savedId) return g.savedId;
    if (plies() < 2) return null; // aborted games are not saved
    const body = {
      white: nameOf('w'),
      black: nameOf('b'),
      result: g.result,
      termination: g.termination,
      start_fen: g.startFen,
      moves: g.moves.slice(),
      bot_id: null,
      user_color: userColor(),
      time_control: g.tc.id === 'none' ? null : g.tc.id,
      opening_name: null,
      notes: g.takebacks ? t('local.notes.takebacks', { count: g.takebacks }) : '',
      tags: ['local'],
    };
    try {
      const rec = await api.post('/api/games', body, { timeout: 20000 });
      if (rec && rec.id != null) {
        g.savedId = rec.id;
        if (!bag.disposed) toast(t('local.save.saved'), 'success', { duration: 2200 });
        return rec.id;
      }
      return null;
    } catch (e) {
      if (!bag.disposed && !isAbort(e)) toast(t('local.save.failed', { message: (e && e.message) || t('local.save.serverError') }), 'error');
      return null;
    }
  }

  async function goReview() {
    let id = await g.savePromise;
    if (!id && plies() >= 2) { g.savePromise = saveGame(); id = await g.savePromise; }
    if (!id) {
      if (plies() < 2) toast(t('local.save.tooShort'), 'info');
      return false;
    }
    location.hash = `#/review/${id}`;
    return true;
  }

  function rematch() {
    onRematch({ white: g.names.b, black: g.names.w, startFen: g.startFen, tcId: g.tc.id, opts: { ...g.opts } });
  }

  function renderAfter() {
    afterEl.replaceChildren(
      plies() >= 2 ? h('button', { type: 'button', class: 'btn btn-primary btn-lg btn-block', html: icon('sparkles') + `<span>${escapeHtml(t('local.after.review'))}</span>`, onClick: async (e) => { const b = e.currentTarget; b.classList.add('loading'); try { await goReview(); } finally { b.classList.remove('loading'); } } }) : null,
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('refresh') + `<span>${escapeHtml(t('local.after.rematch'))}</span>`, onClick: () => rematch() }),
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('users') + `<span>${escapeHtml(t('local.after.newGame'))}</span>`, onClick: () => onNewGame() })),
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('analysis') + `<span>${escapeHtml(t('local.after.analyze'))}</span>`, onClick: () => { location.hash = `#/analysis?fen=${encodeURIComponent(chess.fen())}`; } }),
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('flip') + `<span>${escapeHtml(t('local.actions.flip'))}</span>`, onClick: () => onFlip() })));
  }

  function showGameOver() {
    if (gameOverModal || bag.disposed) return;
    const win = g.result === '1-0' ? 'w' : g.result === '0-1' ? 'b' : null;
    const player = (c) => h('div', { class: ['lc-go-player', win === c && 'winner'] }, sideAvatar(c, 'lg'), h('div', { class: 'semibold truncate' }, nameOf(c)), h('div', { class: 'subtle text-xs' }, sideLabel(c)));
    const scoreText = g.result === '1/2-1/2' ? '½ – ½' : g.result === '1-0' ? '1 – 0' : '0 – 1';
    const body = h('div', { class: ['lc-go', win ? 'lc-go-win' : 'lc-go-draw'] },
      win ? confetti() : null,
      h('div', { class: 'result-hero' },
        h('div', { class: 'lc-go-icon', html: icon(win ? 'trophy' : 'handshake') }),
        h('div', { class: 'result-hero-title' }, resultHeadline()),
        h('div', { class: 'result-hero-sub' }, terminationText(g.termination))),
      h('div', { class: 'lc-go-players' }, player('w'), h('div', { class: 'lc-go-score' }, scoreText), player('b')),
      h('div', { class: 'row-sm row-wrap lc-go-chips' },
        h('span', { class: 'badge' }, t('local.gameOver.moves', { count: Math.ceil(plies() / 2) })),
        g.tc.id !== 'none' ? h('span', { class: 'badge' }, tcLabel(g.tc)) : null,
        g.takebacks ? h('span', { class: 'badge badge-warning' }, t('local.gameOver.takebacks', { count: g.takebacks })) : null),
      h('p', { class: 'muted text-sm text-center lc-go-tip' }, plies() >= 2 ? t('local.gameOver.tip') : t('local.gameOver.tooShort')));
    const actions = [
      { label: t('local.after.newGame'), kind: 'ghost', icon: 'users', onClick: () => { onNewGame(); } },
      { label: t('local.after.rematch'), kind: 'secondary', icon: 'refresh', onClick: () => { rematch(); } },
    ];
    if (plies() >= 2) actions.push({ label: t('local.after.review'), kind: 'primary', icon: 'sparkles', autofocus: true, onClick: () => goReview() });
    gameOverModal = modal({ title: t('local.gameOver.title'), body, actions, onClose: () => { gameOverModal = null; } });
  }

  function confetti() {
    const wrap = h('div', { class: 'lc-confetti', 'aria-hidden': 'true' });
    const colors = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--accent)', 'var(--cls-brilliant)', 'var(--cls-mistake)'];
    for (let i = 0; i < 36; i++) {
      wrap.appendChild(h('i', { style: { '--x': `${Math.round(Math.random() * 100)}%`, '--d': `${(Math.random() * 0.6).toFixed(2)}s`, '--r': `${Math.round(Math.random() * 720 - 360)}deg`, '--c': colors[i % colors.length], '--dx': `${Math.round(Math.random() * 120 - 60)}px` } }));
    }
    return wrap;
  }

  // ---- Persistence (resume unfinished games) -----------------------------
  function persist() {
    if (g.result || ended || discarded || !plies()) return;
    saveJson(SAVE_KEY, {
      v: 1, white: g.names.w, black: g.names.b, startFen: g.startFen, moves: g.moves, tcId: g.tc.id,
      opts: g.opts, orientation: board.orientation, clocks: clock ? clock.getTimes() : null,
      takebacks: g.takebacks, updatedAt: new Date().toISOString(),
    });
  }
  bag.on(window, 'pagehide', () => persist());
  bag.on(document, 'visibilitychange', () => { if (document.visibilityState === 'hidden') persist(); });
  // Persist (with current clock times) before the components are torn down.
  bag.add(() => { if (!ended) { persist(); ended = true; } });

  // ---- Start -------------------------------------------------------------
  placeBars();
  enableEval(!!g.opts.evalBar);
  refreshAll(false);
  if (resume) toast(t('local.resume.restored'), 'info', { duration: 2500 });
  if (!checkEnd()) {
    if (clock && plies() > 0) clock.start(colorName(chess.turn()));
    persist();
  }
}
