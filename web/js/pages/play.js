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
import { getSettings } from '../settings.js';
import { Board } from '../components/board.js';
import { EvalBar } from '../components/evalbar.js';
import { MoveList } from '../components/movelist.js';
import { ChessClock } from '../components/clock.js';

export const title = 'Play vs Bots';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
const PREFS_KEY = 'grandmentor.play.prefs.v1';
const SAVE_KEY = 'grandmentor.play.current.v1';
const MAX_THINK_MS = 6000;
const BOT_CHAT_MS = 4500;
const DRAW_OFFER_COOLDOWN_PLIES = 10;

const TIME_CONTROLS = [
  { id: 'none', label: 'No clock', sub: 'Relaxed', initial: 0, inc: 0 },
  { id: '1+0', label: '1 min', sub: 'Bullet', initial: 60e3, inc: 0 },
  { id: '3+2', label: '3 | 2', sub: 'Blitz', initial: 180e3, inc: 2e3 },
  { id: '5+0', label: '5 min', sub: 'Blitz', initial: 300e3, inc: 0 },
  { id: '10+0', label: '10 min', sub: 'Rapid', initial: 600e3, inc: 0 },
  { id: '15+10', label: '15 | 10', sub: 'Rapid', initial: 900e3, inc: 10e3 },
  { id: '30+0', label: '30 min', sub: 'Classical', initial: 1800e3, inc: 0 },
];
const TC_BY_ID = Object.fromEntries(TIME_CONTROLS.map((t) => [t.id, t]));

const CATEGORIES = [
  { id: 'coach', label: 'Coach', blurb: 'Friendly teachers who explain their ideas.' },
  { id: 'beginner', label: 'Beginner', blurb: 'Just learning the moves? Start here.' },
  { id: 'intermediate', label: 'Intermediate', blurb: 'Solid club players who punish loose pieces.' },
  { id: 'advanced', label: 'Advanced', blurb: 'Strong and sharp. Bring your best tactics.' },
  { id: 'master', label: 'Master', blurb: 'Master-level strength. Good luck!' },
];

const MODES = {
  challenge: { label: 'Challenge', desc: 'No help at all, just you and the bot.', opts: { hints: false, takebacks: false, evalBar: false, coach: false } },
  friendly: { label: 'Friendly', desc: 'Hints, takebacks, the eval bar and a coach who comments on your moves.', opts: { hints: true, takebacks: true, evalBar: true, coach: true } },
  custom: { label: 'Custom', desc: 'Pick exactly the help you want.', opts: null },
};

const OPTION_DEFS = [
  { key: 'hints', title: 'Hints', desc: 'A light bulb that shows a good move when you are stuck.', ic: 'hint' },
  { key: 'takebacks', title: 'Takebacks', desc: 'Undo your last move if you slip.', ic: 'undo' },
  { key: 'evalBar', title: 'Evaluation bar', desc: 'A bar beside the board that shows who is winning.', ic: 'chart' },
  { key: 'coach', title: 'Coach mode', desc: 'The mentor rates each of your moves and explains why.', ic: 'mentor' },
];

const PIECE_NAMES = { p: 'pawn', n: 'knight', b: 'bishop', r: 'rook', q: 'queen', k: 'king' };
const PIECE_VALUES = { p: 1, n: 3, b: 3, r: 5, q: 9, k: 0 };
const START_COUNTS = { p: 8, n: 2, b: 2, r: 2, q: 1 };
const GLYPHS = { w: { p: '♙', n: '♘', b: '♗', r: '♖', q: '♕' }, b: { p: '♟', n: '♞', b: '♝', r: '♜', q: '♛' } };
const NOTABLE_CLS = new Set(['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder', 'forced']);
const GOOD_CLS = new Set(['brilliant', 'great', 'best', 'excellent', 'book', 'forced']);
const TERMINATION_TEXT = {
  checkmate: 'by checkmate',
  resignation: 'by resignation',
  timeout: 'on time',
  stalemate: 'by stalemate',
  'threefold repetition': 'by threefold repetition',
  'insufficient material': 'by insufficient material',
  'timeout vs insufficient material': 'timeout vs insufficient material',
  '50-move rule': 'by the 50-move rule',
  agreement: 'by agreement',
  abandoned: 'game abandoned',
};

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
    tc: TC_BY_ID[p.tc] ? p.tc : 'none',
  };
}

function loadSavedGame() {
  const s = loadJson(SAVE_KEY);
  if (!s || s.v !== 1 || !s.bot || !s.bot.id || !Array.isArray(s.moves) || !s.moves.length) return null;
  if (s.userColor !== 'w' && s.userColor !== 'b') return null;
  return s;
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
  const v = String(value || '🙂');
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
export async function mount(root, { params = {} } = {}) {
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
        title: 'Something went wrong',
        text: e && e.message ? e.message : 'The game screen could not be opened.',
        action: { label: 'Back to bots', icon: 'robot', onClick: () => showSetup(null) },
      })));
    }
  };
  bag.add(() => { if (viewDispose) { const d = viewDispose; viewDispose = null; d(); } });

  const ctx = { bots: [], profile: null, stats: null, onStats: null };

  await ensureCss();
  if (bag.disposed) return bag.dispose;
  host.appendChild(loadingBlock('Waking up the bots…'));

  // Non-critical data (name for saved games, recommended bot).
  api.get('/api/profile', { signal: ac.signal }).then((p) => { ctx.profile = p; }).catch(() => {});
  api.get('/api/stats', { signal: ac.signal }).then((s) => { ctx.stats = s; if (ctx.onStats) ctx.onStats(); }).catch(() => {});

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
      if (!ctx.bots.length) throw new Error('No bots are available right now.');
      showSetup(params.botId || null);
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      setView(() => {
        host.appendChild(h('div', { class: 'page' }, emptyState({
          icon: 'robot',
          title: 'The bots are not answering',
          text: `${e.message || 'Something went wrong.'} Make sure the GrandMentor server is running, then try again.`,
          action: { label: 'Try again', icon: 'refresh', onClick: () => { setView(() => { host.appendChild(loadingBlock('Waking up the bots…')); return null; }); load(); } },
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
    const ladder = bots.filter((b) => b.category !== 'coach').sort((a, b) => (a.elo || 0) - (b.elo || 0));
    const pick = ladder.find((b) => !(per.get(b.id)?.won > 0));
    return (pick || ladder[ladder.length - 1] || bots[0]).id;
  };

  const state = {
    botId: (preselectId && byId.has(preselectId) && preselectId) || (prefs.botId && byId.has(prefs.botId) && prefs.botId) || recommendedId(),
    color: prefs.color,
    mode: prefs.mode,
    opts: { ...prefs.opts },
    tc: prefs.tc,
  };
  if (preselectId && !byId.has(preselectId)) toast('That bot was not found, so we picked one for you.', 'warning');

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
    previewBoard = new Board(previewBoardEl, { fen: START_FEN, orientation: state.color === 'black' ? 'black' : 'white', interactive: false, movableColor: null, showCoords: s.showCoords, sounds: false, animationMs: s.animationMs });
    bag.add(() => previewBoard.destroy());
  } catch (e) { console.warn('[play] preview board unavailable', e); }

  // ---- Right: panel ------------------------------------------------------
  const resumeSlot = h('div');
  const heroSlot = h('div', { class: 'play-hero' });
  const groups = h('div', { class: 'bot-groups' });
  const tileById = new Map();

  for (const cat of [...CATEGORIES, { id: '__other', label: 'More bots', blurb: '' }]) {
    const list = bots.filter((b) => (cat.id === '__other' ? !CATEGORIES.some((c) => c.id === b.category) : b.category === cat.id))
      .sort((a, b) => (a.elo || 0) - (b.elo || 0));
    if (!list.length) continue;
    const grid = h('div', { class: 'bot-grid', role: 'listbox', 'aria-label': `${cat.label} bots` });
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
  const colorRow = h('div', { class: 'color-choice', role: 'radiogroup', 'aria-label': 'Play as' },
    ...[['white', 'White', h('span', { class: 'color-swatch white' }, '♚')], ['random', 'Random', h('span', { class: 'color-swatch random' }, '?')], ['black', 'Black', h('span', { class: 'color-swatch black' }, '♚')]]
      .map(([id, label, sw]) => (colorBtns[id] = h('button', { type: 'button', class: 'color-btn', role: 'radio', 'aria-checked': 'false', onClick: () => { state.color = id; refreshOptions(); } }, sw, h('span', null, label)))));

  // Mode + toggles
  const modeBtns = {};
  const modeSeg = h('div', { class: 'segmented block', role: 'radiogroup', 'aria-label': 'Help level' },
    ...Object.entries(MODES).map(([id, m]) => (modeBtns[id] = h('button', { type: 'button', role: 'radio', onClick: () => setMode(id) }, m.label))));
  const modeDesc = h('p', { class: 'muted text-sm' });
  const optInputs = {};
  const optionList = h('div', { class: 'play-options' }, ...OPTION_DEFS.map((d) => {
    const input = h('input', { type: 'checkbox', onChange: () => { state.opts[d.key] = input.checked; state.mode = matchMode(state.opts); refreshOptions(); } });
    optInputs[d.key] = input;
    return h('label', { class: 'play-option' },
      h('span', { class: 'play-option-icon', html: icon(d.ic) }),
      h('span', { class: 'play-option-text' }, h('span', { class: 'semibold' }, d.title), h('span', { class: 'subtle text-xs' }, d.desc)),
      h('span', { class: 'switch' }, input, h('span', { class: 'switch-track' })));
  }));

  // Time control
  const tcBtns = {};
  const tcRow = h('div', { class: 'tc-grid', role: 'radiogroup', 'aria-label': 'Time control' },
    ...TIME_CONTROLS.map((t) => (tcBtns[t.id] = h('button', { type: 'button', class: 'tc-btn', role: 'radio', onClick: () => { state.tc = t.id; refreshOptions(); } },
      h('span', { class: 'tc-label' }, t.label), h('span', { class: 'tc-sub' }, t.sub)))));

  const playBtn = h('button', { type: 'button', class: 'btn btn-primary btn-xl btn-block play-cta', onClick: () => play() });

  const panel = h('aside', { class: 'play-setup-panel card card-flush' },
    h('div', { class: 'play-setup-head' }, h('div', { class: 'page-header-icon', html: icon('robot') }),
      h('div', null, h('h1', { class: 'page-title' }, 'Play vs Bots'), h('p', { class: 'page-subtitle' }, 'Pick an opponent, choose your help, and have fun.'))),
    h('div', { class: 'play-setup-scroll' },
      resumeSlot,
      heroSlot,
      h('h2', { class: 'play-section-title' }, 'Choose your opponent'),
      groups,
      h('h2', { class: 'play-section-title' }, 'I play as'),
      colorRow,
      h('h2', { class: 'play-section-title' }, 'Help'),
      modeSeg, modeDesc, optionList,
      h('h2', { class: 'play-section-title' }, 'Time'),
      tcRow),
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
    resumeSlot.appendChild(h('div', { class: 'resume-card' },
      botAvatar(b, 'avatar-sm'),
      h('div', { class: 'resume-text' }, h('div', { class: 'semibold' }, `Game in progress vs ${b.name}`),
        h('div', { class: 'subtle text-xs' }, `${Math.ceil(saved.moves.length / 2)} move${Math.ceil(saved.moves.length / 2) === 1 ? '' : 's'} played · you are ${colorName(saved.userColor)}`)),
      h('button', { type: 'button', class: 'btn btn-primary btn-sm', html: icon('play') + '<span>Resume</span>', onClick: () => onResume(saved) }),
      h('button', {
        type: 'button', class: 'btn btn-ghost btn-icon btn-sm', 'aria-label': 'Discard saved game', 'data-tooltip': 'Discard', html: icon('trash'),
        onClick: async () => {
          const ok = await confirmDialog({ title: 'Discard this game?', message: 'The unfinished game will be deleted and not saved to your library.', confirmLabel: 'Discard', danger: true });
          if (!ok || bag.disposed) return;
          removeKey(SAVE_KEY);
          saved = null;
          renderResume();
        },
      })));
  }

  function renderHero() {
    const b = byId.get(state.botId);
    if (!b) return;
    const rec = b.id === recommendedId();
    heroSlot.replaceChildren(
      botAvatar(b, 'avatar-xl'),
      h('div', { class: 'play-hero-main' },
        h('div', { class: 'play-hero-name' }, b.name, h('span', { class: 'play-hero-elo' }, `${b.elo ?? ''}`)),
        h('div', { class: 'row-sm row-wrap' },
          b.style && String(b.style).toLowerCase() !== String(b.category || '').toLowerCase() ? h('span', { class: 'badge badge-info' }, b.style) : null,
          b.category ? h('span', { class: `badge level-${b.category === 'coach' ? 'beginner' : b.category}` }, b.category) : null,
          rec ? h('span', { class: 'badge badge-gold', html: icon('star-filled', { size: 12 }) + ' Recommended' }) : null),
        b.description ? h('p', { class: 'muted text-sm play-hero-desc' }, b.description) : null,
        b.greeting ? h('div', { class: 'bubble bubble-bot play-mobile-only text-sm' }, b.greeting) : null));
    previewAvatarSlot.replaceChildren(botAvatar(b, 'avatar-xl'));
    previewName.replaceChildren(h('span', { class: 'semibold' }, b.name), ' ', h('span', { class: 'subtle' }, `(${b.elo ?? '?'})`));
    previewBubble.textContent = b.greeting || `Hi! I'm ${b.name}. Ready when you are.`;
    previewBubble.classList.remove('pop-in');
    void previewBubble.offsetWidth; // restart the pop animation
    previewBubble.classList.add('pop-in');
    playBtn.innerHTML = icon('play') + `<span>Play ${escapeHtml(b.name)}</span>`;
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
    modeDesc.textContent = MODES[state.mode].desc;
    for (const d of OPTION_DEFS) optInputs[d.key].checked = !!state.opts[d.key];
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
      saveJson(PREFS_KEY, { botId: state.botId, color: state.color, mode: state.mode, opts: state.opts, tc: state.tc });
      const userColor = state.color === 'white' ? 'w' : state.color === 'black' ? 'b' : (Math.random() < 0.5 ? 'w' : 'b');
      onPlay({ bot, userColor, colorChoice: state.color, opts: { ...state.opts }, tcId: state.tc });
    };
    if (saved) {
      confirmDialog({ title: 'Start a new game?', message: `You have an unfinished game vs ${saved.bot.name}. Starting a new game will discard it.`, confirmLabel: 'Start new game' })
        .then((ok) => { if (ok && !bag.disposed) { removeKey(SAVE_KEY); proceed(); } });
      return;
    }
    proceed();
  }

  ctx.onStats = () => { if (!bag.disposed) { renderHero(); refreshTiles(); } };
  bag.add(() => { ctx.onStats = null; });

  renderResume();
  renderHero();
  refreshTiles();
  refreshOptions();
  // Bring the selected tile into view inside the scroll area.
  bag.raf(() => { const t = tileById.get(state.botId); if (t && t.scrollIntoView) t.scrollIntoView({ block: 'nearest' }); });

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
    startFen: (resume && typeof resume.startFen === 'string' && resume.startFen) || START_FEN,
    opts: { hints: true, takebacks: true, evalBar: true, coach: false, ...((resume ? resume.opts : cfg.opts) || {}) },
    tc: TC_BY_ID[resume ? resume.tcId : cfg.tcId] || TC_BY_ID.none,
    moves: [], sans: [], fens: [], lastMoves: [], cls: [],
    result: null, termination: '',
    hintsUsed: resume ? (resume.hintsUsed | 0) : 0,
    takebacksUsed: resume ? (resume.takebacksUsed | 0) : 0,
    openingName: resume ? (resume.openingName || null) : null,
    drawOfferPly: resume ? (resume.drawOfferPly ?? -99) : -99,
    savedId: null,
    savePromise: null,
  };
  // Prefer the freshest bot profile from the server if we have it.
  const fresh = ctx.bots.find((b) => b.id === g.bot.id);
  if (fresh) g.bot = fresh;
  const bot = g.bot;
  const userC = g.userColor;
  const botC = other(userC);

  let chess;
  try { chess = new Chess(g.startFen); } catch { chess = new Chess(START_FEN); g.startFen = START_FEN; }
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
  }

  const plies = () => g.moves.length;
  let viewPly = null;           // null = live position
  let botToken = 0;
  let botPending = false;
  let botAc = null;
  let botTimer = null;
  let chatTimer = null;
  let openingAc = null;
  let hint = { fen: null, stage: 0, move: null, loading: false };
  let analysis = { fen: null, best: null, score: null, depth: 0 };
  let gameOverModal = null;
  let ended = false;
  let discarded = false;

  // ---- DOM ---------------------------------------------------------------
  const s = getSettings();
  const mkBar = (c) => {
    const isBot = c === botC;
    const captures = h('div', { class: 'player-captures' });
    const thinking = isBot ? h('span', { class: 'thinking-dots', 'aria-label': `${bot.name} is thinking`, hidden: true }, h('i'), h('i'), h('i')) : null;
    const chat = isBot ? h('div', { class: 'bot-chat bubble', role: 'status', 'aria-live': 'polite', hidden: true }) : null;
    const clockSlot = h('div', { class: 'clock-slot' });
    const name = isBot ? bot.name : ((ctx.profile && ctx.profile.name) || 'You');
    const rating = isBot ? `(${bot.elo ?? '?'})` : null;
    const el = h('div', { class: `player-bar play-bar ${isBot ? 'is-bot' : 'is-user'}` },
      isBot ? botAvatar(bot) : avatarNode(ctx.profile && ctx.profile.avatar, 'avatar-round user-avatar'),
      h('div', { style: 'min-width:0' },
        h('div', { class: 'player-name' }, name, ' ', rating ? h('span', { class: 'player-rating' }, rating) : null, thinking),
        captures),
      clockSlot,
      chat);
    return { el, captures, thinking, chat, clockSlot };
  };
  const bars = { [userC]: mkBar(userC), [botC]: mkBar(botC) };
  const evalSlot = h('div', { class: 'evalbar-slot' });
  const boardSlot = h('div', { class: 'board-slot' });
  const boardRow = h('div', { class: 'board-row' }, evalSlot, boardSlot);
  const main = h('div', { class: 'game-main' });

  const openingEl = h('div', { class: 'play-opening truncate' });
  const modeBadges = h('div', { class: 'row-sm' },
    g.opts.coach ? h('span', { class: 'badge badge-primary', title: 'Coach mode is on' }, '🎓 Coach') : null,
    g.tc.id !== 'none' ? h('span', { class: 'badge', html: icon('clock', { size: 12 }) + ' ' + g.tc.label }) : null,
    !g.opts.hints && !g.opts.takebacks && !g.opts.evalBar && !g.opts.coach ? h('span', { class: 'badge badge-danger' }, 'Challenge') : null);
  const statusEl = h('div', { class: 'play-status', role: 'status', 'aria-live': 'polite' });
  const coachText = h('div', { class: 'bubble bubble-mentor coach-text md' });
  const coachBox = h('div', { class: 'coach-box mentor-row', hidden: true },
    h('div', { class: 'avatar avatar-sm avatar-round coach-avatar', 'aria-hidden': 'true' }, '🎓'), coachText);
  const moveListEl = h('div', { class: 'panel-body play-moves' });
  const navBtn = (ic, label, fn) => h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': label, 'data-tooltip': label, html: icon(ic), onClick: fn });
  const nav = {
    first: navBtn('first', 'First move', () => goto(0)),
    prev: navBtn('chevron-left', 'Previous move (←)', () => goto(currentPly() - 1)),
    next: navBtn('chevron-right', 'Next move (→)', () => goto(currentPly() + 1)),
    last: navBtn('last', 'Back to the game', () => goto(plies())),
  };
  const liveChip = h('button', { type: 'button', class: 'btn btn-sm btn-secondary live-chip', hidden: true, onClick: () => goto(plies()), html: icon('play', { size: 14 }) + '<span>Back to game</span>' });
  const navBar = h('div', { class: 'toolbar play-nav' }, nav.first, nav.prev, liveChip, h('div', { class: 'spacer' }), nav.next, nav.last);

  const actionBtn = (ic, label, fn, extra = '') => h('button', { type: 'button', class: `btn btn-secondary play-action ${extra}`.trim(), onClick: fn, html: icon(ic) + `<span>${label}</span>` });
  const acts = {
    hint: actionBtn('hint', 'Hint', () => onHint(), 'act-hint'),
    takeback: actionBtn('undo', 'Takeback', () => onTakeback()),
    flip: actionBtn('flip', 'Flip', () => onFlip()),
    draw: actionBtn('handshake', 'Draw', () => onOfferDraw()),
    resign: actionBtn('flag', 'Resign', () => onResign(), 'act-resign'),
    newGame: actionBtn('plus', 'New', () => onNewGame()),
  };
  acts.hint.setAttribute('aria-label', 'Hint: show a good move');
  if (!g.opts.hints) acts.hint.hidden = true;
  if (!g.opts.takebacks) acts.takeback.hidden = true;
  const actionsEl = h('div', { class: 'play-actions' }, ...Object.values(acts));
  const afterEl = h('div', { class: 'play-after', hidden: true });

  const panel = h('aside', { class: 'game-panel' },
    h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header play-panel-head' }, h('span', { html: icon('openings') }), openingEl, h('div', { class: 'spacer' }), modeBadges),
      statusEl,
      coachBox,
      moveListEl,
      navBar),
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
    showCoords: s.showCoords, showLegal: s.showLegal, animationMs: s.animationMs, sounds: s.sounds,
    onMove: (m) => onUserMove(m),
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
      onLowTime: (c) => { if (c === colorName(userC)) playSoundSafe('notify'); },
    });
    bag.add(() => clock.destroy());
  }

  // Lazy sound module (optional; board plays move sounds itself).
  let playSoundFn = null;
  import('../components/sound.js').then((m) => { playSoundFn = typeof m.playSound === 'function' ? m.playSound : null; }).catch(() => {});
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
    const can = isLive() && !g.result && !botPending && userToMove();
    try { board.setInteractive(can, can ? colorName(userC) : null); } catch { /* ignore */ }
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
    openingEl.textContent = g.openingName || (plies() ? 'Game in progress' : 'Starting position');
    openingEl.title = openingEl.textContent;
  }

  function renderStatus() {
    statusEl.className = 'play-status';
    statusEl.replaceChildren();
    if (g.result) {
      statusEl.classList.add('done');
      statusEl.append(h('span', { html: icon('trophy', { size: 16 }) }), h('span', null, resultHeadline() + ' ' + (TERMINATION_TEXT[g.termination] || '')));
      return;
    }
    if (!isLive()) {
      statusEl.classList.add('browsing');
      statusEl.append(h('span', { html: icon('eye', { size: 16 }) }), h('span', null, `Viewing move ${Math.ceil(viewPly / 2) || 0}. Press → or "Back to game" to continue.`));
      return;
    }
    if (botPending) {
      statusEl.classList.add('thinking');
      statusEl.append(h('span', { class: 'thinking-dots' }, h('i'), h('i'), h('i')), h('span', null, `${bot.name} is thinking…`));
      return;
    }
    if (userToMove()) {
      statusEl.classList.add('your-turn');
      const inCheck = chess.inCheck();
      statusEl.append(h('span', { class: `turn-dot ${colorName(userC)}` }),
        h('span', null, inCheck ? 'Your king is in check! Get it to safety.' : `Your move. You play ${colorName(userC)}.`));
    } else {
      statusEl.append(h('span', { class: `turn-dot ${colorName(botC)}` }), h('span', null, `${bot.name} to move.`));
    }
  }

  function setBotError(message) {
    statusEl.className = 'play-status error';
    statusEl.replaceChildren(
      h('span', { html: icon('alert', { size: 16 }) }),
      h('span', { class: 'grow' }, `${bot.name} could not move: ${message}`),
      h('button', { type: 'button', class: 'btn btn-sm btn-primary', onClick: () => requestBotMove() }, 'Retry'));
  }

  function renderActions() {
    const live = isLive();
    const over = !!g.result;
    acts.hint.disabled = over || !live || botPending || !userToMove() || hint.loading;
    acts.hint.classList.toggle('stage-2', hint.fen === chess.fen() && hint.stage >= 1);
    acts.takeback.disabled = over || !canTakeback();
    acts.draw.disabled = over || plies() < 2;
    acts.resign.disabled = over;
    actionsEl.hidden = over;
    afterEl.hidden = !over;
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
  function analyzeLive() {
    if (!engine || g.result) return;
    if (!userToMove()) { engine.stop(); return; }
    const fen = chess.fen();
    try {
      engine.analyze(fen, { multipv: 1, movetime_ms: 3000 }, (info) => {
        if (fen !== chess.fen() || g.result) return;
        const line = info && info.lines && info.lines[0];
        if (!line || !line.score) return;
        analysis = { fen, best: (line.moves && line.moves[0]) || analysis.best, score: line.score, depth: info.depth || 0 };
        if (evalbar) evalbar.set(line.score);
      }, () => { /* engine hiccup: the bar just stays where it was */ });
    } catch (e) { console.warn('[play] engine', e); }
  }

  async function bestMoveFor(fen) {
    if (analysis.fen === fen && analysis.best && analysis.depth >= 8) return { uci: analysis.best, score: analysis.score };
    const info = await api.post('/api/engine/analyze', { fen, movetime_ms: 900, multipv: 1 }, { signal: gac.signal, timeout: 15000 });
    const line = info && info.lines && info.lines[0];
    if (!line || !line.moves || !line.moves[0]) throw new Error('No move found');
    if (fen === chess.fen() && (!analysis.fen || analysis.fen !== fen || (info.depth || 0) >= analysis.depth)) {
      analysis = { fen, best: line.moves[0], score: line.score, depth: info.depth || 0 };
    }
    return { uci: line.moves[0], score: line.score };
  }

  // ---- Opening name ------------------------------------------------------
  function lookupOpening() {
    if (plies() > 30 || plies() === 0) return;
    if (openingAc) openingAc.abort();
    const myAc = new AbortController();
    openingAc = myAc;
    const fen = chess.fen();
    api.get('/api/openings/lookup' + qs({ fen }), { signal: myAc.signal, timeout: 8000 }).then((m) => {
      if (openingAc === myAc) openingAc = null;
      if (bag.disposed || !m || !m.opening || !m.opening.name) return;
      g.openingName = m.opening.name;
      renderOpening();
    }).catch(() => { if (openingAc === myAc) openingAc = null; });
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

  function onUserMove(m) {
    if (!m || !isLive() || g.result || botPending || !userToMove()) return false;
    const fenBefore = chess.fen();
    let mv = null;
    try { mv = chess.move({ from: m.from, to: m.to, promotion: m.promotion || undefined }); } catch { mv = null; }
    if (!mv) return false;
    recordMove(mv);
    const ply = plies();
    const uci = g.moves[ply - 1];
    clearHint();
    pressClock(userC);
    // Let the board finish its own move handling before we sync it.
    queueMicrotask(() => {
      if (bag.disposed) return;
      refreshAll(false);
      afterMove(fenBefore, uci, ply, true);
    });
    return true;
  }

  function pressClock(mover) {
    if (!clock || g.result) return;
    if (!clock.running) clock.start(colorName(other(mover)));
    else clock.press();
  }

  function afterMove(fenBefore, uci, ply, byUser) {
    lookupOpening();
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
      setBotError(e.message || 'network error');
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
      setBotError('it suggested an illegal move');
      return;
    }
    recordMove(mv);
    pressClock(botC);
    refreshAll(true);
    if (bm.chat) botSay(bm.chat);
    afterMove(fenBefore, g.moves[plies() - 1], plies(), false);
  }

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
      let text = r.explanation ? String(r.explanation) : `**${san}** is ${cls ? classificationMeta(cls).label.toLowerCase() : 'played'}.`;
      if (cls && !GOOD_CLS.has(cls) && r.best_move_san && r.best_move_san !== san && !text.includes(r.best_move_san)) {
        text += `\n\nBetter was **${r.best_move_san}**.`;
      }
      coachSay(text, cls);
      if (evalbar && r.eval_after && plies() === ply && !g.result) evalbar.set(r.eval_after);
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
        if (!mv) throw new Error('No hint available');
        hint.move = { from: mv.from, to: mv.to, san: mv.san, piece: mv.piece };
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        toast('The coach could not find a hint right now. Try again in a moment.', 'warning');
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
      coachSay(`Look at your **${PIECE_NAMES[hint.move.piece] || 'piece'}** on **${hint.move.from}**. Can you find a strong move with it? Press *Hint* again to see the move.`);
    } else {
      coachSay(`Try **${hint.move.san}**: move from ${hint.move.from} to ${hint.move.to}.`);
    }
    applyHintVisual();
    renderActions();
    persist();
  }

  // ---- Takeback ----------------------------------------------------------
  function minPly() {
    // The user cannot take back the bot's opening move when playing black.
    return plyColor(1) === userC ? 0 : 1;
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
    if (!canTakeback()) { toast('Nothing to take back yet.', 'info'); return; }
    const n = takebackCount();
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
    if (clock && clock.running && !g.result) clock.start(colorName(chess.turn()));
    refreshAll(true);
    if (!coachBox.hidden) coachSay('Move taken back. Take your time and look for checks, captures and threats.');
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
    const ok = await confirmDialog({ title: 'Resign this game?', message: `${bot.name} will win. You can still review the game afterwards.`, confirmLabel: 'Resign', danger: true });
    if (!ok || bag.disposed || g.result) return;
    endGame(userC === 'w' ? '0-1' : '1-0', 'resignation');
  }

  async function onOfferDraw() {
    if (g.result || plies() < 2) return;
    if (plies() - g.drawOfferPly < DRAW_OFFER_COOLDOWN_PLIES) {
      toast(`${bot.name} already said no. Try again in a few moves.`, 'info');
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
      botSay('A draw sounds fair. Well played!');
      endGame('1/2-1/2', 'agreement');
    } else {
      botSay(moveNo <= 30 ? "It's too early for a draw. Let's play on!" : "No thanks, I think there's still something to play for.");
      toast(`${bot.name} declined your draw offer.`, 'info');
      persist();
    }
  }

  async function onNewGame() {
    if (!g.result && plies() >= 2) {
      const ok = await confirmDialog({ title: 'Start a new game?', message: 'This game will be counted as a loss (resignation) and saved to your library.', confirmLabel: 'Resign & start new', danger: true });
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
    return o === 'win' ? 'You won!' : o === 'loss' ? `${bot.name} won` : 'Draw';
  }

  function endGame(result, termination, { silent = false } = {}) {
    if (g.result) return;
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
    if (evalbar && result === '1/2-1/2') evalbar.set({ cp: 0 });
    else if (evalbar && termination === 'checkmate') evalbar.set({ mate: 0 }, { mated: result === '1-0' ? 'black' : 'white' });
    refreshAll(false);
    const o = userOutcome();
    if (!silent) {
      playSoundSafe('gameEnd');
      botSay(o === 'win' ? 'Well played! You got me this time.' : o === 'loss' ? 'Good game! Want to see where it turned?' : 'A hard-fought draw. Good game!');
    }
    g.savePromise = saveGame();
    renderAfter();
    if (!silent) bag.timeout(() => { if (!bag.disposed) showGameOver(); }, termination === 'checkmate' ? 650 : 250);
  }

  async function saveGame() {
    if (g.savedId) return g.savedId;
    if (plies() < 2) return null; // aborted games are not saved (like chess.com)
    const userName = (ctx.profile && ctx.profile.name) || 'You';
    const tags = ['vs-bot'];
    if (g.hintsUsed || g.takebacksUsed) tags.push('assisted');
    if (g.opts.coach) tags.push('coach');
    const notesParts = [];
    if (g.hintsUsed) notesParts.push(`Hints used: ${g.hintsUsed}`);
    if (g.takebacksUsed) notesParts.push(`Takebacks: ${g.takebacksUsed}`);
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
        if (!bag.disposed) toast('Game saved to your library', 'success', { duration: 2200 });
        return rec.id;
      }
      return null;
    } catch (e) {
      if (!bag.disposed) toast(`Could not save the game: ${e.message || 'server error'}`, 'error');
      return null;
    }
  }

  async function goReview() {
    let id = await g.savePromise;
    if (!id && plies() >= 2) { g.savePromise = saveGame(); id = await g.savePromise; }
    if (!id) {
      if (plies() < 2) toast('This game was too short to review.', 'info');
      return false;
    }
    location.hash = `#/review/${id}`;
    return true;
  }

  function rematch() {
    const nextColor = g.colorChoice === 'random' ? other(userC) : userC;
    onRematch({ bot, userColor: nextColor, colorChoice: g.colorChoice, opts: { ...g.opts }, tcId: g.tc.id });
  }

  function renderAfter() {
    afterEl.replaceChildren(
      h('button', { type: 'button', class: 'btn btn-primary btn-lg btn-block', html: icon('sparkles') + '<span>Game Review</span>', onClick: async (e) => { const b = e.currentTarget; b.classList.add('loading'); try { await goReview(); } finally { b.classList.remove('loading'); } } }),
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('refresh') + '<span>Rematch</span>', onClick: () => rematch() }),
        h('button', { type: 'button', class: 'btn btn-secondary grow', html: icon('robot') + '<span>New bot</span>', onClick: () => onNewBot(bot.id) })),
      h('div', { class: 'row-sm' },
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('analysis') + '<span>Analyze</span>', onClick: () => { location.hash = `#/analysis?fen=${encodeURIComponent(chess.fen())}`; } }),
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm grow', html: icon('flip') + '<span>Flip</span>', onClick: () => onFlip() })));
  }

  function showGameOver() {
    if (gameOverModal || bag.disposed) return;
    const o = userOutcome();
    const you = h('div', { class: 'go-player' }, avatarNode(ctx.profile && ctx.profile.avatar, 'avatar-lg avatar-round'), h('div', { class: 'semibold' }, (ctx.profile && ctx.profile.name) || 'You'));
    const them = h('div', { class: 'go-player' }, botAvatar(bot, 'avatar-lg'), h('div', { class: 'semibold' }, bot.name), h('div', { class: 'subtle text-xs' }, String(bot.elo ?? '')));
    const scoreText = g.result === '1/2-1/2' ? '½ – ½' : (userOutcome() === 'win' ? '1 – 0' : '0 – 1');
    const chips = h('div', { class: 'row-sm row-wrap go-chips' },
      h('span', { class: 'badge' }, `${Math.ceil(plies() / 2)} move${Math.ceil(plies() / 2) === 1 ? '' : 's'}`),
      g.openingName ? h('span', { class: 'badge badge-info' }, g.openingName) : null,
      g.hintsUsed ? h('span', { class: 'badge badge-warning' }, `${g.hintsUsed} hint${g.hintsUsed > 1 ? 's' : ''}`) : null,
      g.takebacksUsed ? h('span', { class: 'badge badge-warning' }, `${g.takebacksUsed} takeback${g.takebacksUsed > 1 ? 's' : ''}`) : null);
    const body = h('div', { class: `go-body go-${o}` },
      o === 'win' ? confetti() : null,
      h('div', { class: 'result-hero' },
        h('div', { class: 'go-icon', html: icon(o === 'win' ? 'trophy' : o === 'loss' ? 'flag' : 'handshake') }),
        h('div', { class: 'result-hero-title' }, resultHeadline()),
        h('div', { class: 'result-hero-sub' }, TERMINATION_TEXT[g.termination] || g.termination)),
      h('div', { class: 'go-players' }, you, h('div', { class: 'go-score' }, scoreText), them),
      chips,
      h('p', { class: 'muted text-sm text-center go-tip' }, plies() >= 2
        ? (o === 'win' ? 'Great job! Review the game to see your best moves.' : 'Every game is a lesson. Game Review shows where things turned.')
        : 'The game ended before it really started, so it was not saved.'));
    const actions = [
      { label: 'New bot', kind: 'ghost', icon: 'robot', onClick: () => { onNewBot(bot.id); } },
      { label: 'Rematch', kind: 'secondary', icon: 'refresh', onClick: () => { rematch(); } },
    ];
    if (plies() >= 2) actions.push({ label: 'Game Review', kind: 'primary', icon: 'sparkles', autofocus: true, onClick: () => goReview() });
    gameOverModal = modal({ title: 'Game over', body, actions, onClose: () => { gameOverModal = null; } });
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
    if (g.result || ended || discarded || !plies()) return;
    saveJson(SAVE_KEY, {
      v: 1, bot, userColor: userC, colorChoice: g.colorChoice, startFen: g.startFen, moves: g.moves, opts: g.opts, tcId: g.tc.id,
      clocks: clock ? clock.getTimes() : null, hintsUsed: g.hintsUsed, takebacksUsed: g.takebacksUsed,
      cls: g.cls, openingName: g.openingName, drawOfferPly: g.drawOfferPly, updatedAt: new Date().toISOString(),
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
  if (resume) {
    lookupOpening();
    toast(`Welcome back! Your game vs ${bot.name} is restored.`, 'info', { duration: 2500 });
  }

  if (!checkEnd()) {
    if (!userToMove()) {
      if (!resume) botSay(bot.greeting || `Hi! I'm ${bot.name}. Good luck!`);
      if (clock && plies() > 0) clock.start(colorName(botC));
      requestBotMove();
    } else {
      if (!resume) botSay(bot.greeting || `Hi! I'm ${bot.name}. Your move!`);
      if (clock && plies() > 0) clock.start(colorName(userC));
      analyzeLive();
      if (g.opts.coach && !plies()) coachSay(`Hi! I'm your coach. I'll rate every move you make vs **${bot.name}**. Have fun, and remember: develop your pieces and castle early!`);
    }
  }
}
