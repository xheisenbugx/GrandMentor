// GrandMentor — Puzzles page.
// Routes (see app.js): #/puzzles (hub), #/puzzles?play=1[&theme=fork] (rated solver),
// #/puzzles/rush (params.mode = 'rush'), #/puzzles/daily (params.mode = 'daily'),
// #/puzzles/mistakes (params.mode = 'mistakes': spaced-repetition cards from the user's own games).
// Contract: docs/CONTRACT.md §4 (puzzle API) and §5 (Board, sound); UX: docs/FEATURES.md §3.5–3.6.
//
// Memory hygiene: every view owns a `disposables()` bag; every puzzle runner owns a timer set that is
// cleared on each new puzzle and on destroy; all fetches share an AbortController aborted on unmount.

import { h, icon, pageHeader, disposables, loadingBlock, emptyState, toast, formatClock, confirmDialog, classificationMeta } from '../ui.js';
import { api, qs, isAbort } from '../api.js';
import { getSetting } from '../settings.js';
import { Board } from '../components/board.js';
import { playSound } from '../components/sound.js';
import { Chess } from '../../vendor/chess.js';
import { t, hasKey, formatDateIntl, formatNumber, getLocale } from '../i18n.js';

const TITLE_KEYS = { rush: 'puzzles.rushTitle', daily: 'puzzles.dailyTitle', mistakes: 'puzzles.mistakes.title' };
export const title = (params) => t(TITLE_KEYS[params?.mode] || 'puzzles.title');

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

const CSS_HREF = '/css/puzzles.css';
function ensureCss() {
  if (document.querySelector('link[data-page-css="puzzles"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = CSS_HREF;
  link.dataset.pageCss = 'puzzles';
  document.head.appendChild(link);
}

/** A set of timeouts that forget themselves when they fire (no unbounded growth). */
class Timers {
  constructor() { this.ids = new Set(); }
  later(fn, ms) {
    const id = setTimeout(() => { this.ids.delete(id); fn(); }, ms);
    this.ids.add(id);
    return id;
  }
  clear() { for (const id of this.ids) clearTimeout(id); this.ids.clear(); }
}

function sound(name) {
  try { if (getSetting('sounds') !== false) playSound(name); } catch { /* audio unavailable */ }
}

function store(key, value) {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, String(value));
  } catch { /* storage unavailable */ }
  return null;
}

function todayKey(d = new Date()) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

const THEME_EMOJI = {
  mate: '♚', mateIn1: '1️⃣', mateIn2: '2️⃣', mateIn3: '3️⃣', fork: '🍴', pin: '📌', skewer: '🍢', discoveredAttack: '💥',
  hangingPiece: '🎁', backRankMate: '🧱', sacrifice: '🔥', endgame: '🏁', opening: '📖', middlegame: '⚔️', promotion: '👑',
  deflection: '↪️', trappedPiece: '🪤', doubleCheck: '⚡', smotheredMate: '🐴', kingsideAttack: '🎯', defensiveMove: '🛡️',
};
function themeLabel(id) {
  const s = String(id || '');
  if (s && hasKey(`themes.${s}`)) return t(`themes.${s}`);
  return s.replace(/([a-z])([A-Z0-9])/g, '$1 $2').replace(/^./, (c) => c.toUpperCase());
}
// Themes that describe length / evaluation are noise as filters for beginners.
const HIDDEN_FILTER_THEMES = new Set(['short', 'long', 'veryLong', 'oneMove', 'crushing', 'advantage', 'equality', 'master', 'masterVsMaster', 'superGM']);

/**
 * A puzzle normally starts with the opponent's set-up move (`moves[0]`). With `userFirst: true`
 * (mistake cards without a previous move) the solver moves first from `fen`.
 */
function validPuzzle(p) {
  return p && typeof p.id === 'string' && typeof p.fen === 'string' && Array.isArray(p.moves) && p.moves.length >= (p.userFirst ? 1 : 2);
}

/** FEN of the position the user has to solve (after the opponent's set-up move). */
function solveFen(p) {
  if (p.userFirst) return p.fen;
  try {
    const c = new Chess(p.fen);
    const m = p.moves[0];
    c.move({ from: m.slice(0, 2), to: m.slice(2, 4), promotion: m[4] || undefined });
    return c.fen();
  } catch { return p.fen; }
}

function analysisHref(p) { return `#/analysis?fen=${encodeURIComponent(solveFen(p))}`; }

function uciObj(uci) {
  return { from: uci.slice(0, 2), to: uci.slice(2, 4), promotion: uci[4] || undefined };
}

/** Animate a number in `el` from `from` to `to`. Returns a cancel function. */
function animateNumber(el, from, to, ms = 900) {
  if (!el) return () => {};
  const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  if (reduce || document.hidden || from === to || !Number.isFinite(from) || !Number.isFinite(to)) { el.textContent = String(Math.round(to)); return () => {}; }
  let id = 0;
  const start = performance.now();
  const step = (now) => {
    const k = Math.min(1, (now - start) / ms);
    const e = 1 - Math.pow(1 - k, 3);
    el.textContent = String(Math.round(from + (to - from) * e));
    if (k < 1) id = requestAnimationFrame(step); else id = 0;
  };
  id = requestAnimationFrame(step);
  return () => { if (id) cancelAnimationFrame(id); id = 0; };
}

function createBoard(slot, onMove) {
  return new Board(slot, {
    fen: 'start',
    orientation: 'white',
    interactive: false,
    movableColor: null,
    showCoords: getSetting('showCoords'),
    showLegal: getSetting('showLegal'),
    animationMs: getSetting('animationMs'),
    sounds: getSetting('sounds') !== false,
    onMove,
  });
}

function btn(label, iconName, kind, onClick, extra = {}) {
  return h('button', {
    type: 'button',
    class: `btn btn-${kind}${extra.cls ? ' ' + extra.cls : ''}`,
    html: (iconName ? icon(iconName) : '') + `<span>${label}</span>`,
    onClick,
    'aria-keyshortcuts': extra.key || null,
    title: extra.title || null,
  });
}

function sideLabel(color) { return color === 'white' ? t('puzzles.white') : t('puzzles.black'); }
function toMove(color) { return t('puzzles.toMove', { side: sideLabel(color) }); }

// ---------------------------------------------------------------------------
// PuzzleRunner: drives one puzzle on a Board (shared by rated, daily and rush).
// ---------------------------------------------------------------------------

export class PuzzleRunner {
  /**
   * hooks: onReady(), onCorrect(), onWrong(uci), onSolved(), onSolutionDone(), onError(msg)
   * opts: firstMoveDelay, replyDelay, allowRetry
   */
  constructor(board, hooks = {}, opts = {}) {
    this.board = board;
    this.hooks = hooks;
    this.opts = { firstMoveDelay: 550, replyDelay: 380, allowRetry: true, ...opts };
    this.timers = new Timers();
    this.puzzle = null;
    this.chess = null;
    this.idx = 0;
    this.state = 'idle'; // idle|intro|user|reply|wrong|solved|solution|shown|broken|dead
    this.hintStage = 0;
    this.lastMove = null;
    this.userColor = 'white';
  }

  _clearMarks() {
    try { this.board.clearArrows(); this.board.clearHighlights(); this.board.clearBadges(); } catch { /* board gone */ }
  }

  _fail(msg) {
    this.state = 'broken';
    this.board.setInteractive(false, null);
    this.hooks.onError?.(msg);
  }

  load(puzzle) {
    this.timers.clear();
    this._clearMarks();
    if (!validPuzzle(puzzle)) { this._fail(t('puzzles.errors.loadFailed')); return false; }
    let chess;
    try { chess = new Chess(puzzle.fen); } catch { this._fail(t('puzzles.errors.invalidPosition')); return false; }
    this.puzzle = puzzle;
    this.chess = chess;
    this.idx = 0;
    this.hintStage = 0;
    this.lastMove = null;
    this.state = 'intro';
    // The opponent moves first, so the solver plays the side NOT to move in the FEN
    // (unless the puzzle starts with the solver's own move).
    const userTurn = chess.turn() === 'w' ? 'white' : 'black';
    this.userColor = puzzle.userFirst ? userTurn : (userTurn === 'white' ? 'black' : 'white');
    this.board.setInteractive(false, null);
    if (this.board.orientation !== this.userColor) this.board.setOrientation(this.userColor);
    this.board.setPosition(puzzle.fen, { animate: false });
    if (puzzle.userFirst) {
      this.timers.later(() => { if (this.state === 'intro') this._toUser(); }, 120);
      return true;
    }
    this.timers.later(() => {
      if (this.state !== 'intro') return;
      if (!this._playScripted()) return;
      this._toUser();
    }, this.opts.firstMoveDelay);
    return true;
  }

  _applyUci(uci) {
    if (typeof uci !== 'string' || uci.length < 4) return null;
    try { return this.chess.move(uciObj(uci.toLowerCase())); } catch { return null; }
  }

  _playScripted() {
    const m = this._applyUci(this.puzzle.moves[this.idx]);
    if (!m) { this._fail(t('puzzles.errors.illegalMove')); return false; }
    this.idx++;
    this.lastMove = [m.from, m.to];
    this.board.setPosition(this.chess.fen(), { animate: true, lastMove: this.lastMove });
    return true;
  }

  _toUser() {
    if (this.idx >= this.puzzle.moves.length) { this._solved(); return; }
    this.state = 'user';
    this.board.setInteractive(true, this.userColor);
    this.hooks.onReady?.();
  }

  _solved() {
    this.state = 'solved';
    this.board.setInteractive(false, null);
    this.hooks.onSolved?.();
  }

  _isAltMate(uci, expected) {
    try {
      const a = new Chess(this.chess.fen());
      a.move(uciObj(expected));
      if (!a.isCheckmate()) return false;
      const b = new Chess(this.chess.fen());
      b.move(uciObj(uci));
      return b.isCheckmate();
    } catch { return false; }
  }

  /** Board onMove handler. Returns true to keep the piece where it was dropped. */
  handleMove(mv) {
    if (this.state !== 'user' || !mv || !mv.from || !mv.to) return false;
    const uci = String(mv.uci || `${mv.from}${mv.to}${mv.promotion || ''}`).toLowerCase();
    const expected = String(this.puzzle.moves[this.idx] || '').toLowerCase();
    const isLast = this.idx === this.puzzle.moves.length - 1;
    let correct = uci === expected;
    if (!correct && isLast) correct = this._isAltMate(uci, expected);
    this._clearMarks();
    this.hintStage = 0;

    if (correct) {
      const m = this._applyUci(uci);
      if (!m) return false;
      this.idx++;
      this.lastMove = [m.from, m.to];
      this.board.setHighlights([{ square: m.to, kind: 'good' }]);
      this.board.setBadge(m.to, 'best');
      sound('correct');
      if (this.idx >= this.puzzle.moves.length) { this._solved(); return true; }
      this.state = 'reply';
      this.board.setInteractive(false, null);
      this.hooks.onCorrect?.();
      this.timers.later(() => {
        if (this.state !== 'reply') return;
        this._clearMarks();
        if (!this._playScripted()) return;
        this._toUser();
      }, this.opts.replyDelay);
      return true;
    }

    // Wrong: show it briefly in red, then take it back.
    this.state = 'wrong';
    this.board.setInteractive(false, null);
    this.board.setHighlights([{ square: mv.to, kind: 'bad' }]);
    this.board.setBadge(mv.to, 'miss');
    sound('wrong');
    this.hooks.onWrong?.(uci);
    this.timers.later(() => {
      if (this.state !== 'wrong') return;
      this._clearMarks();
      this.board.setPosition(this.chess.fen(), { animate: true, lastMove: this.lastMove, sound: false });
      if (this.opts.allowRetry) {
        this.state = 'user';
        this.board.setInteractive(true, this.userColor);
      } else {
        this.state = 'failed';
      }
    }, 650);
    return true;
  }

  /** Two-stage hint: 1 = highlight the piece, 2 = show the arrow. Returns the stage reached (0 = n/a). */
  hint() {
    if (this.state !== 'user') return 0;
    const exp = String(this.puzzle.moves[this.idx] || '');
    if (exp.length < 4) return 0;
    const from = exp.slice(0, 2);
    const to = exp.slice(2, 4);
    this.hintStage = Math.min(2, this.hintStage + 1);
    if (this.hintStage === 1) {
      this.board.setHighlights([{ square: from, kind: 'hint' }]);
    } else {
      this.board.setHighlights([{ square: from, kind: 'hint' }]);
      this.board.setArrows([{ from, to, color: 'green' }]);
    }
    return this.hintStage;
  }

  /** Name of the piece the hint points at (for friendly copy). */
  hintPieceName() {
    try {
      const exp = String(this.puzzle.moves[this.idx] || '');
      const p = this.chess.get(exp.slice(0, 2));
      return t(`puzzles.pieces.${['p', 'n', 'b', 'r', 'q', 'k'].includes(p?.type) ? p.type : 'unknown'}`);
    } catch { return t('puzzles.pieces.unknown'); }
  }

  /** Animate the remaining solution moves. */
  showSolution() {
    if (!this.puzzle || ['solution', 'shown', 'broken', 'dead', 'idle'].includes(this.state)) return;
    this.timers.clear();
    this._clearMarks();
    if (this.state === 'intro' && !this.puzzle.userFirst) {
      if (!this._playScripted()) return;
    }
    this.state = 'solution';
    this.board.setInteractive(false, null);
    this.board.setPosition(this.chess.fen(), { animate: false, lastMove: this.lastMove });
    const step = () => {
      if (this.state !== 'solution') return;
      if (this.idx >= this.puzzle.moves.length) {
        this.state = 'shown';
        this.hooks.onSolutionDone?.();
        return;
      }
      const isUser = (this.idx % 2 === 1) !== !!this.puzzle.userFirst;
      if (!this._playScripted()) return;
      this._clearMarks();
      if (isUser) {
        this.board.setArrows([{ from: this.lastMove[0], to: this.lastMove[1], color: 'green' }]);
        this.board.setHighlights([{ square: this.lastMove[1], kind: 'good' }]);
      }
      this.timers.later(step, 850);
    };
    this.timers.later(step, 350);
  }

  retry() {
    if (this.puzzle) this.load(this.puzzle);
  }

  get solveFen() { return this.puzzle ? solveFen(this.puzzle) : null; }

  destroy() {
    this.state = 'dead';
    this.timers.clear();
  }
}

/** Elapsed-time stopwatch rendering into an element. One interval, only while running. */
class Stopwatch {
  constructor(el) { this.el = el; this.start = 0; this.acc = 0; this.id = 0; }
  reset() { this.stop(); this.acc = 0; this.render(); }
  run() {
    if (this.id) return;
    this.start = performance.now();
    this.id = setInterval(() => this.render(), 250);
  }
  stop() {
    if (!this.id) return;
    clearInterval(this.id);
    this.id = 0;
    this.acc += performance.now() - this.start;
    this.render();
  }
  get ms() { return this.acc + (this.id ? performance.now() - this.start : 0); }
  render() { if (this.el) this.el.textContent = formatClock(this.ms, { tenths: false }); }
  destroy() { if (this.id) clearInterval(this.id); this.id = 0; this.el = null; }
}

// ---------------------------------------------------------------------------
// Page entry
// ---------------------------------------------------------------------------

export async function mount(root, { params = {}, query = {} } = {}) {
  ensureCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const ctx = { bag, signal: ctrl.signal };

  if (params.mode === 'rush') mountRush(root, ctx);
  else if (params.mode === 'mistakes') mountMistakes(root, ctx);
  else if (params.mode === 'daily') mountSolver(root, ctx, { mode: 'daily' });
  else if (query.play || query.theme) mountSolver(root, ctx, { mode: 'rated', theme: query.theme || '' });
  else await mountHub(root, ctx);

  return () => {
    bag.dispose();
    root.replaceChildren();
  };
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

async function mountHub(root, { bag, signal }) {
  const page = h('div', { class: 'page pz-hub' },
    pageHeader({ title: t('puzzles.title'), subtitle: t('puzzles.hub.subtitle'), icon: 'puzzle' }));
  const body = h('div', { class: 'stack-lg' }, loadingBlock(t('puzzles.hub.loading')));
  page.appendChild(body);
  root.appendChild(page);

  const [profile, themes, mistakes] = await Promise.all([
    api.get('/api/profile', { signal }).catch((e) => (isAbort(e) ? null : null)),
    api.get('/api/puzzles/themes', { signal }).catch(() => []),
    api.get('/api/mistakes/summary', { signal }).catch(() => null),
  ]);
  if (bag.disposed) return;

  const rating = profile && Number.isFinite(profile.puzzle_rating) ? Math.round(profile.puzzle_rating) : null;
  const solved = profile?.puzzles_solved ?? 0;
  const failed = profile?.puzzles_failed ?? 0;
  const total = solved + failed;
  const pct = total ? Math.round((solved / total) * 100) : 0;
  const rushBest = profile?.rush_best ?? 0;
  const streak = profile?.streak_days ?? 0;
  const dailyDone = store(`gm.puzzles.daily.${todayKey()}`) === '1';
  const now = new Date();

  const rated = h('a', { class: 'card card-link card-feature pz-mode pz-mode-rated', href: '#/puzzles?play=1' },
    h('div', { class: 'pz-mode-icon', html: icon('target') }),
    h('div', { class: 'pz-mode-title' }, t('puzzles.hub.rated.title')),
    h('p', { class: 'muted' }, t('puzzles.hub.rated.desc')),
    h('div', { class: 'pz-rating-big' },
      h('span', { class: 'pz-rating-num tabular' }, rating != null ? formatNumber(rating) : '—'),
      h('span', { class: 'subtle' }, t('puzzles.hub.rated.ratingLabel'))),
    h('div', { class: 'row-sm subtle text-sm' },
      h('span', null, t('puzzles.hub.rated.solved', { count: solved })), h('span', { class: 'dot-sep' }), h('span', null, total ? t('puzzles.hub.rated.success', { pct }) : t('puzzles.hub.rated.noAttempts'))),
    h('span', { class: 'btn btn-primary btn-lg btn-block mt-3', html: icon('play') + `<span>${t('puzzles.hub.rated.cta')}</span>` }));

  const rush = h('a', { class: 'card card-link pz-mode pz-mode-rush', href: '#/puzzles/rush' },
    h('div', { class: 'pz-mode-icon', html: icon('bolt') }),
    h('div', { class: 'pz-mode-title' }, t('puzzles.rushTitle')),
    h('p', { class: 'muted' }, t('puzzles.hub.rush.desc')),
    h('div', { class: 'pz-rating-big' },
      h('span', { class: 'pz-rating-num tabular' }, String(rushBest)),
      h('span', { class: 'subtle' }, t('puzzles.hub.rush.bestLabel'))),
    h('div', { class: 'row-sm subtle text-sm' }, h('span', null, t('puzzles.rush.modes.3.short')), h('span', { class: 'dot-sep' }), h('span', null, t('puzzles.rush.modes.5.short')), h('span', { class: 'dot-sep' }), h('span', null, t('puzzles.rush.modes.survival.short'))),
    h('span', { class: 'btn btn-secondary btn-lg btn-block mt-3', html: icon('bolt') + `<span>${t('puzzles.hub.rush.cta')}</span>` }));

  const daily = h('a', { class: 'card card-link pz-mode pz-mode-daily', href: '#/puzzles/daily' },
    calendarCard(now, dailyDone),
    h('div', { class: 'pz-mode-title' }, t('puzzles.dailyTitle')),
    h('p', { class: 'muted' }, t('puzzles.hub.daily.desc')),
    h('div', { class: 'row-sm subtle text-sm' },
      h('span', { class: 'pz-flame', html: icon('fire') }), h('span', null, t('puzzles.dayStreak', { count: streak }))),
    h('span', { class: `btn ${dailyDone ? 'btn-ghost' : 'btn-secondary'} btn-lg btn-block mt-3`, html: icon(dailyDone ? 'check' : 'calendar') + `<span>${dailyDone ? t('puzzles.hub.daily.solvedAgain') : t('puzzles.hub.daily.cta')}</span>` }));

  const content = [h('div', { class: 'grid-3 pz-modes' }, rated, rush, daily)];
  // "Learn from your mistakes": first when something is due, otherwise right after the modes.
  if (mistakes && Number.isFinite(mistakes.total)) {
    const entry = mistakesEntry(mistakes);
    if ((mistakes.due | 0) > 0) content.unshift(entry); else content.push(entry);
  }

  const list = Array.isArray(themes) ? themes.filter((x) => x && x.theme && !HIDDEN_FILTER_THEMES.has(x.theme)) : [];
  list.sort((a, b) => (b.count || 0) - (a.count || 0));
  if (list.length) {
    content.push(h('section', null,
      h('h2', { class: 'section-title' }, t('puzzles.hub.themesTitle')),
      h('p', { class: 'muted mb-3' }, t('puzzles.hub.themesDesc')),
      h('div', { class: 'pz-theme-grid' }, list.slice(0, 24).map((x) =>
        h('a', { class: 'pz-theme-tile', href: `#/puzzles?play=1&theme=${encodeURIComponent(x.theme)}` },
          h('span', { class: 'pz-theme-emoji', 'aria-hidden': 'true' }, THEME_EMOJI[x.theme] || '♟'),
          h('span', { class: 'pz-theme-name' }, themeLabel(x.theme)),
          h('span', { class: 'pz-theme-count subtle tabular' }, Number.isFinite(x.count) ? formatNumber(x.count) : ''))))));
  }

  content.push(h('section', null,
    h('h2', { class: 'section-title' }, t('puzzles.hub.howTitle')),
    h('div', { class: 'grid-3 pz-howto' },
      howto('1', t('puzzles.hub.how1Title'), t('puzzles.hub.how1Text')),
      howto('2', t('puzzles.hub.how2Title'), t('puzzles.hub.how2Text')),
      howto('3', t('puzzles.hub.how3Title'), t('puzzles.hub.how3Text')))));

  body.replaceChildren(...content);
}

/** Hub banner for "Learn from your mistakes" with the due-count badge. */
function mistakesEntry(sum) {
  const due = sum.due | 0;
  const total = sum.total | 0;
  const mastered = sum.graduated | 0;
  const desc = total ? t('puzzles.hub.mistakes.desc') : t('puzzles.hub.mistakes.descEmpty');
  const stats = total
    ? h('div', { class: 'row-sm subtle text-sm pz-mx-entry-stats' },
      h('span', null, t('puzzles.mistakes.inDeck', { count: total })), h('span', { class: 'dot-sep' }),
      h('span', null, t('puzzles.mistakes.mastered', { count: mastered })))
    : null;
  const cta = due
    ? t('puzzles.hub.mistakes.ctaDue', { count: due })
    : total ? t('puzzles.hub.mistakes.ctaPractice') : t('puzzles.hub.mistakes.ctaEmpty');
  return h('a', { class: 'card card-link pz-mx-entry', href: '#/puzzles/mistakes' },
    h('div', { class: 'pz-mode-icon pz-mx-icon', html: icon('target') },
      due ? h('span', { class: 'pz-mx-badge tabular', 'aria-label': t('puzzles.mistakes.dueToday', { count: due }) }, due > 99 ? '99+' : String(due)) : null),
    h('div', { class: 'pz-mx-entry-main' },
      h('div', { class: 'row-sm' },
        h('span', { class: 'pz-mode-title' }, t('puzzles.hub.mistakes.title')),
        due ? h('span', { class: 'badge pz-mx-due-chip' }, t('puzzles.mistakes.dueToday', { count: due })) : null),
      h('p', { class: 'muted' }, desc),
      stats),
    h('span', { class: `btn ${due ? 'btn-primary' : 'btn-secondary'} pz-mx-entry-cta`, html: icon(due ? 'play' : 'arrow-right') + `<span>${cta}</span>` }));
}

function howto(n, heading, text) {
  return h('div', { class: 'card card-sm pz-howto-card' },
    h('div', { class: 'pz-howto-num' }, n),
    h('div', null, h('div', { class: 'semibold' }, heading), h('p', { class: 'muted text-sm' }, text)));
}

function calendarCard(date, done = false) {
  return h('div', { class: `pz-cal${done ? ' done' : ''}`, 'aria-label': formatDateIntl(date, { dateStyle: 'full' }) },
    h('div', { class: 'pz-cal-month' }, formatDateIntl(date, { month: 'short' }).replace(/\.$/, '')),
    h('div', { class: 'pz-cal-day tabular' }, String(date.getDate())),
    h('div', { class: 'pz-cal-wd' }, formatDateIntl(date, { weekday: 'short' }).replace(/\.$/, '')),
    done ? h('div', { class: 'pz-cal-check', html: icon('check') }) : null);
}

// ---------------------------------------------------------------------------
// Solver (rated + daily)
// ---------------------------------------------------------------------------

function mountSolver(root, { bag, signal }, { mode, theme: initialTheme = '' }) {
  const isDaily = mode === 'daily';
  const timers = new Timers();
  bag.add(() => timers.clear());
  const cancels = new Set();
  bag.add(() => { for (const c of cancels) c(); cancels.clear(); });

  const st = {
    theme: initialTheme,
    puzzle: null,
    rated: !isDaily,
    failed: false,
    recorded: false,
    rating: null,
    streak: 0,
    bestStreak: 0,
    history: [],
    loading: false,
    feedback: 'loading', // loading|ready|correct|wrong|solved|solvedLate|solution|error
    errorMsg: '',
    hintMsg: '',
  };

  // ----- DOM -----
  const slot = h('div', { class: 'board-slot' });
  const turnBar = h('div', { class: 'pz-turnbar' });
  const flipBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': t('puzzles.solver.flipBoard'), 'data-tooltip': t('puzzles.solver.flipBoard'), html: icon('flip') });
  const analyzeLink = h('a', { class: 'btn btn-ghost btn-sm', href: '#/analysis', html: icon('analysis') + `<span>${t('puzzles.solver.analyze')}</span>` });
  const themeLink = h('span', { class: 'pz-theme-current subtle text-sm' });
  const toolbar = h('div', { class: 'toolbar pz-toolbar' }, flipBtn, h('div', { class: 'spacer' }), themeLink, analyzeLink);

  const ratingNum = h('span', { class: 'pz-score-num tabular' }, '—');
  const deltaEl = h('span', { class: 'pz-delta' });
  const streakNum = h('span', { class: 'tabular' }, '0');
  const timeEl = h('span', { class: 'tabular' }, '0:00');
  const stopwatch = new Stopwatch(timeEl);
  bag.add(() => stopwatch.destroy());

  const status = h('div', { class: 'pz-status', 'aria-live': 'polite' });
  const info = h('div', { class: 'pz-info' });
  const historyEl = h('div', { class: 'pz-history', 'aria-label': t('puzzles.solver.sessionHistory') });
  const actions = h('div', { class: 'pz-actions' });
  const chipsEl = h('div', { class: 'chip-row pz-chips' });

  const topCard = isDaily
    ? h('div', { class: 'pz-daily-head' }, calendarCard(new Date(), store(`gm.puzzles.daily.${todayKey()}`) === '1'),
      h('div', null,
        h('div', { class: 'pz-daily-title' }, t('puzzles.dailyTitle')),
        h('div', { class: 'muted text-sm' }, formatDateIntl(new Date(), { weekday: 'long', month: 'long', day: 'numeric' })),
        h('div', { class: 'row-sm text-sm mt-1' }, h('span', { class: 'pz-flame', html: icon('fire') }), h('span', { class: 'pz-daily-streak' }, '—'))))
    : h('div', { class: 'pz-scoreboard' },
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, t('puzzles.solver.rating')), h('div', { class: 'row-sm' }, ratingNum, deltaEl)),
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, t('puzzles.solver.streak')), h('div', { class: 'pz-score-sm row-sm' }, h('span', { class: 'pz-flame', html: icon('fire') }), streakNum)),
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, t('puzzles.solver.time')), h('div', { class: 'pz-score-sm row-sm' }, h('span', { class: 'subtle', html: icon('timer') }), timeEl)));

  const panelBody = h('div', { class: 'panel-body stack' },
    topCard, status, info,
    isDaily ? null : h('div', { class: 'stack-sm' }, h('div', { class: 'stat-label' }, t('puzzles.solver.theme')), chipsEl),
    isDaily ? null : h('div', { class: 'stack-sm' }, h('div', { class: 'stat-label' }, t('puzzles.solver.thisSession')), historyEl));

  const panel = h('div', { class: 'panel grow' },
    h('div', { class: 'panel-header', html: icon(isDaily ? 'calendar' : 'puzzle') + `<span>${isDaily ? t('puzzles.dailyTitle') : t('puzzles.title')}</span>` },
      h('div', { class: 'spacer' }),
      h('a', { class: 'btn btn-ghost btn-sm', href: '#/puzzles', html: icon('grid') + `<span>${t('puzzles.solver.allModes')}</span>` })),
    panelBody,
    h('div', { class: 'panel-footer' }, actions));

  const layout = h('div', { class: 'game-layout no-eval pz-layout', style: '--board-chrome: 124px' },
    h('div', { class: 'game-main' }, turnBar, h('div', { class: 'board-row' }, slot), toolbar),
    h('aside', { class: 'game-panel' }, panel));
  const page = h('div', { class: 'page page-wide pz-page' }, layout);
  root.appendChild(page);

  const board = createBoard(slot, (mv) => runner.handleMove(mv));
  bag.add(() => board.destroy());

  const runner = new PuzzleRunner(board, {
    onReady() {
      stopwatch.run();
      if (st.feedback === 'loading' || st.feedback === 'error') st.feedback = 'ready';
      st.hintMsg = '';
      render();
    },
    onCorrect() { st.feedback = 'correct'; st.hintMsg = ''; render(); },
    onWrong() {
      st.feedback = 'wrong';
      st.hintMsg = '';
      markFailed();
      render();
      const s = slot; s.classList.remove('shake'); void s.offsetWidth; s.classList.add('shake');
    },
    onSolved() {
      stopwatch.stop();
      st.feedback = st.failed ? 'solvedLate' : 'solved';
      if (!st.failed) {
        st.streak++; st.bestStreak = Math.max(st.bestStreak, st.streak);
        if (st.rated) record(true);
        pushHistory(true);
      }
      if (isDaily) store(`gm.puzzles.daily.${todayKey()}`, '1');
      try { toastOnce(); } catch { /* noop */ }
      render();
    },
    onSolutionDone() { st.feedback = 'solution'; render(); },
    onError(msg) { stopwatch.stop(); st.feedback = 'error'; st.errorMsg = msg; render(); },
  });
  bag.add(() => runner.destroy());

  function toastOnce() {
    if (isDaily) toast(t('puzzles.solver.dailyToast'), 'success');
  }

  function markFailed() {
    if (st.failed) return;
    st.failed = true;
    st.streak = 0;
    if (st.rated) record(false);
    pushHistory(false);
  }

  function pushHistory(solved) {
    if (!st.puzzle || !st.rated) return;
    st.history.push({ puzzle: st.puzzle, solved });
    if (st.history.length > 30) st.history.splice(0, st.history.length - 30);
    renderHistory();
  }

  async function record(solved) {
    if (st.recorded || !st.puzzle) return;
    st.recorded = true;
    const p = st.puzzle;
    try {
      const res = await api.post(`/api/puzzles/${encodeURIComponent(p.id)}/attempt`, { solved, time_ms: Math.round(stopwatch.ms) }, { signal });
      if (bag.disposed || !res) return;
      const from = Number.isFinite(st.rating) ? st.rating : Math.round(res.rating - (res.delta || 0));
      st.rating = Math.round(res.rating);
      cancels.forEach((c) => c()); cancels.clear();
      cancels.add(animateNumber(ratingNum, from, st.rating));
      const d = Math.round(res.delta || 0);
      deltaEl.textContent = d > 0 ? `+${d}` : d < 0 ? `${d}` : '±0';
      deltaEl.className = `pz-delta pop-in ${d > 0 ? 'up' : d < 0 ? 'down' : ''}`;
      if (st.puzzle === p) render();
    } catch (e) {
      if (!isAbort(e)) toast(t('puzzles.errors.saveFailed', { message: e.message }), 'warning');
    }
  }

  // ----- Rendering -----
  function render() {
    const color = runner.userColor;
    streakNum.textContent = String(st.streak);
    const done = ['solved', 'solvedLate', 'solution'].includes(st.feedback);

    // Turn banner above the board
    let bar;
    if (st.feedback === 'loading') bar = [h('span', { class: 'spinner' }), h('span', null, t('puzzles.solver.bar.loading'))];
    else if (st.feedback === 'error') bar = [h('span', { html: icon('alert') }), h('span', null, t('puzzles.solver.bar.error'))];
    else if (st.feedback === 'solved' || st.feedback === 'solvedLate') bar = [h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, t('puzzles.solver.bar.solved'))];
    else if (st.feedback === 'solution') bar = [h('span', { html: icon('eye') }), h('span', null, t('puzzles.solver.bar.solution'))];
    else bar = [h('span', { class: `pz-side pz-side-${color}` }), h('span', null, toMove(color))];
    turnBar.className = `pz-turnbar state-${st.feedback}`;
    turnBar.replaceChildren(...bar);

    // Status card
    let kind = 'neutral', ic = 'target', head = '', sub = '';
    switch (st.feedback) {
      case 'loading': head = t('puzzles.solver.status.loadingHead'); sub = t('puzzles.solver.status.loadingSub'); ic = 'clock'; break;
      case 'ready': head = toMove(color); sub = t('puzzles.solver.status.readySub', { side: sideLabel(color) }); break;
      case 'correct': kind = 'good'; ic = 'check-circle'; head = t('puzzles.solver.status.correctHead'); sub = t('puzzles.solver.status.correctSub'); break;
      case 'wrong': kind = 'bad'; ic = 'x-circle'; head = t('puzzles.solver.status.wrongHead'); sub = st.rated ? t('puzzles.solver.status.wrongSubRated') : t('puzzles.solver.status.wrongSub'); break;
      case 'solved': kind = 'good'; ic = 'trophy'; head = isDaily ? t('puzzles.solver.status.dailySolvedHead') : t('puzzles.solver.status.solvedHead'); sub = isDaily ? t('puzzles.solver.status.dailySolvedSub') : t('puzzles.solver.status.solvedSub'); break;
      case 'solvedLate': kind = 'good'; ic = 'check-circle'; head = t('puzzles.solver.status.lateHead'); sub = st.rated ? t('puzzles.solver.status.lateSubRated') : t('puzzles.solver.status.lateSub'); break;
      case 'solution': ic = 'eye'; head = t('puzzles.solver.status.solutionHead'); sub = t('puzzles.solver.status.solutionSub'); break;
      case 'error': kind = 'bad'; ic = 'alert'; head = t('puzzles.solver.status.errorHead'); sub = st.errorMsg || t('puzzles.solver.status.errorSub'); break;
      default: break;
    }
    if (st.hintMsg && st.feedback === 'ready') { sub = st.hintMsg; }
    if (!st.rated && !isDaily && st.puzzle && st.feedback === 'ready') sub = t('puzzles.solver.practiceSuffix', { text: sub });
    status.className = `pz-status pz-status-${kind}`;
    status.replaceChildren(
      h('div', { class: 'pz-status-icon', html: icon(ic) }),
      h('div', { class: 'pz-status-text' }, h('div', { class: 'pz-status-head' }, head), h('div', { class: 'pz-status-sub' }, sub)));

    // Puzzle info (themes only after finishing, to avoid spoilers)
    const p = st.puzzle;
    if (p) {
      info.replaceChildren(...[
        h('div', { class: 'row-sm text-sm' },
          h('span', { class: 'subtle' }, t('puzzles.solver.puzzleRating')), h('span', { class: 'semibold tabular' }, String(p.rating ?? '?')),
          !st.rated && !isDaily ? h('span', { class: 'badge' }, t('puzzles.solver.practice')) : null),
        done && Array.isArray(p.themes) && p.themes.length
          ? h('div', { class: 'pz-tags' }, p.themes.filter((x) => !HIDDEN_FILTER_THEMES.has(x)).slice(0, 6).map((x) => h('span', { class: 'badge' }, themeLabel(x))))
          : null].filter(Boolean));
      analyzeLink.href = analysisHref(p);
    } else {
      info.replaceChildren();
    }

    // Actions
    const list = [];
    const next = btn(isDaily ? t('puzzles.solver.buttons.morePuzzles') : t('puzzles.solver.buttons.nextPuzzle'), 'arrow-right', 'primary', isDaily ? () => { location.hash = '#/puzzles?play=1'; } : () => nextPuzzle(), { key: 'n', cls: 'pz-grow' });
    if (st.feedback === 'loading') {
      list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', null, { cls: 'pz-grow' }), btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', null));
      list.forEach((b) => { b.disabled = true; });
    } else if (st.feedback === 'error') {
      if (!isDaily) list.push(btn(t('puzzles.solver.buttons.tryAnother'), 'refresh', 'primary', () => nextPuzzle(), { cls: 'pz-grow' }));
      else list.push(btn(t('puzzles.solver.buttons.reload'), 'refresh', 'primary', () => loadDaily(), { cls: 'pz-grow' }));
    } else if (done) {
      list.push(btn(t('puzzles.solver.buttons.retry'), 'refresh', 'ghost', () => retry(), { key: 'r' }), next);
    } else {
      list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', () => doHint(), { key: 'h', title: st.rated && !st.failed ? t('puzzles.solver.hintCostsTitle') : null }));
      list.push(btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', () => doSolution()));
      if (st.failed && st.feedback !== 'correct') {
        list.push(btn(t('puzzles.solver.buttons.retry'), 'refresh', 'ghost', () => retry(), { key: 'r' }));
        if (!isDaily) list.push(btn(t('puzzles.solver.buttons.next'), 'arrow-right', 'primary', () => nextPuzzle(), { key: 'n', cls: 'pz-grow' }));
      }
    }
    actions.replaceChildren(...list);

    themeLink.textContent = st.theme ? t('puzzles.solver.currentTheme', { theme: themeLabel(st.theme) }) : '';
  }

  function renderHistory() {
    if (isDaily) return;
    if (!st.history.length) {
      historyEl.replaceChildren(h('span', { class: 'subtle text-sm' }, t('puzzles.solver.historyEmpty')));
      return;
    }
    historyEl.replaceChildren(...st.history.map((e, i) => h('button', {
      type: 'button',
      class: `pz-dot ${e.solved ? 'ok' : 'bad'}${e.puzzle === st.puzzle ? ' current' : ''}`,
      'aria-label': t(e.solved ? 'puzzles.solver.historyAriaSolved' : 'puzzles.solver.historyAriaMissed', { n: i + 1, rating: e.puzzle.rating }),
      title: t(e.solved ? 'puzzles.solver.historyTitleSolved' : 'puzzles.solver.historyTitleMissed', { rating: e.puzzle.rating }),
      html: icon(e.solved ? 'check' : 'x'),
      onClick: () => startPuzzle(e.puzzle, { rated: false }),
    })));
  }

  function renderChips(themes) {
    if (isDaily) return;
    const list = (themes || []).filter((x) => x && x.theme && !HIDDEN_FILTER_THEMES.has(x.theme));
    list.sort((a, b) => (b.count || 0) - (a.count || 0));
    const top = list.slice(0, 14).map((x) => x.theme);
    if (st.theme && !top.includes(st.theme)) top.unshift(st.theme);
    const mk = (value, label) => h('button', {
      type: 'button', class: `chip${st.theme === value ? ' active' : ''}`, 'aria-pressed': String(st.theme === value),
      onClick: () => {
        if (st.theme === value) return;
        st.theme = value;
        // Keep the URL shareable without remounting the page.
        try { history.replaceState(null, '', `#/puzzles?play=1${value ? '&theme=' + encodeURIComponent(value) : ''}`); } catch { /* ignore */ }
        renderChips(themes);
        nextPuzzle();
      },
    }, label);
    chipsEl.replaceChildren(mk('', t('puzzles.solver.all')), ...top.map((x) => mk(x, themeLabel(x))));
  }

  // ----- Actions -----
  function startPuzzle(p, { rated = !isDaily } = {}) {
    timers.clear();
    st.puzzle = p;
    st.rated = rated;
    st.failed = false;
    st.recorded = false;
    st.feedback = 'loading';
    st.hintMsg = '';
    deltaEl.textContent = '';
    deltaEl.className = 'pz-delta';
    stopwatch.reset();
    render();
    renderHistory();
    runner.load(p);
  }

  let fetchSeq = 0;
  async function nextPuzzle() {
    if (isDaily) return;
    const seq = ++fetchSeq;
    runner.timers.clear();
    board.setInteractive(false, null);
    st.feedback = 'loading';
    st.puzzle = null;
    stopwatch.reset();
    render();
    const prevId = st.history.length ? st.history[st.history.length - 1].puzzle.id : null;
    try {
      let p = await api.get('/api/puzzles/next' + qs({ theme: st.theme }), { signal });
      if (validPuzzle(p) && p.id === prevId) {
        const again = await api.get('/api/puzzles/next' + qs({ theme: st.theme }), { signal }).catch(() => null);
        if (validPuzzle(again)) p = again;
      }
      if (bag.disposed || seq !== fetchSeq) return;
      if (!validPuzzle(p)) throw new Error(t('puzzles.errors.noPuzzleForTheme'));
      startPuzzle(p, { rated: true });
    } catch (e) {
      if (isAbort(e) || bag.disposed || seq !== fetchSeq) return;
      st.feedback = 'error';
      st.errorMsg = e.status === 404 ? t('puzzles.errors.noThemeMatch') : e.message;
      render();
    }
  }

  async function loadDaily() {
    st.feedback = 'loading';
    render();
    try {
      const p = await api.get('/api/puzzles/daily', { signal });
      if (bag.disposed) return;
      if (!validPuzzle(p)) throw new Error(t('puzzles.errors.dailyUnavailable'));
      startPuzzle(p, { rated: false });
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      st.feedback = 'error';
      st.errorMsg = e.message;
      render();
    }
  }

  function doHint() {
    const stage = runner.hint();
    if (!stage) return;
    if (st.rated) markFailed();
    st.hintMsg = stage === 1 ? t('puzzles.solver.hintPiece', { piece: runner.hintPieceName() }) : t('puzzles.solver.hintArrow');
    render();
  }

  function doSolution() {
    if (st.rated) markFailed();
    stopwatch.stop();
    st.hintMsg = '';
    runner.showSolution();
    st.feedback = 'solution';
    render();
  }

  function retry() {
    if (!st.puzzle) return;
    const keepFailed = st.failed;
    st.feedback = 'loading';
    st.hintMsg = '';
    render();
    runner.retry();
    st.failed = keepFailed || st.rated; // a retried rated puzzle never counts again
    if (st.rated) st.recorded = true;
  }

  // ----- Events -----
  bag.on(flipBtn, 'click', () => board.flip());
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
    const tg = e.target;
    if (tg && (tg.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tg.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    const k = e.key.toLowerCase();
    const done = ['solved', 'solvedLate', 'solution'].includes(st.feedback);
    if (k === 'f') { board.flip(); e.preventDefault(); }
    else if (k === 'h' && !done) { doHint(); e.preventDefault(); }
    else if (k === 'r' && (done || st.failed)) { retry(); e.preventDefault(); }
    else if ((k === 'n' || k === 'arrowright') && (done || (st.failed && !isDaily))) {
      e.preventDefault();
      if (isDaily) location.hash = '#/puzzles?play=1'; else nextPuzzle();
    }
  });

  // ----- Boot -----
  render();
  renderHistory();
  if (isDaily) {
    loadDaily();
    api.get('/api/profile', { signal }).then((pr) => {
      if (bag.disposed || !pr) return;
      const el = panelBody.querySelector('.pz-daily-streak');
      if (el) el.textContent = t('puzzles.dayStreak', { count: pr.streak_days ?? 0 });
    }).catch(() => {});
  } else {
    api.get('/api/profile', { signal }).then((pr) => {
      if (bag.disposed || !pr || !Number.isFinite(pr.puzzle_rating)) return;
      if (!Number.isFinite(st.rating)) { st.rating = Math.round(pr.puzzle_rating); ratingNum.textContent = String(st.rating); }
    }).catch(() => {});
    renderChips([]);
    api.get('/api/puzzles/themes', { signal }).then((list) => { if (!bag.disposed) renderChips(Array.isArray(list) ? list : []); }).catch(() => {});
    nextPuzzle();
  }
}

// ---------------------------------------------------------------------------
// My mistakes: spaced-repetition cards built from the user's own reviewed games
// (API: /api/mistakes*, see docs/CONTRACT.md "Learn from your mistakes").
// ---------------------------------------------------------------------------

const MASTERY_STEPS = 3;

/** Localized "in 3 hours" for a number of seconds from now. */
function relTime(secs) {
  const s = Math.max(0, Number(secs) || 0);
  if (s < 60) return t('puzzles.mistakes.inAMoment');
  let rtf;
  try { rtf = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' }); } catch { return ''; }
  if (s < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (s < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  return rtf.format(Math.round(s / 86400), 'day');
}

function sanSuffix(cls) { return cls === 'blunder' ? '??' : (cls === 'mistake' || cls === 'miss') ? '?' : ''; }

/** Turn a mistake card into a PuzzleRunner puzzle. */
function cardPuzzle(card) {
  const sol = Array.isArray(card.solution) && card.solution.length ? card.solution.slice() : [card.best_uci];
  if (card.prev_fen && card.prev_uci) return { id: `mistake-${card.id}`, fen: card.prev_fen, moves: [card.prev_uci, ...sol] };
  return { id: `mistake-${card.id}`, fen: card.fen, moves: sol, userFirst: true };
}

function confettiBurst(count = 36) {
  return h('div', { class: 'pz-confetti', 'aria-hidden': 'true' },
    Array.from({ length: count }, (_, i) => h('span', {
      style: { '--x': `${(i * 37) % 100}%`, '--d': `${(i % 9) * 70}ms`, '--r': `${(i * 53) % 360}deg`, '--c': `var(--${['primary', 'gold', 'info', 'accent', 'cls-brilliant'][i % 5]})` },
    })));
}

function masteryPips(streak) {
  const n = Math.max(0, Math.min(MASTERY_STEPS, streak | 0));
  return h('div', { class: 'pz-mx-pips', role: 'img', 'aria-label': t('puzzles.mistakes.masteryAria', { n, total: MASTERY_STEPS }), title: t('puzzles.mistakes.masteryAria', { n, total: MASTERY_STEPS }) },
    Array.from({ length: MASTERY_STEPS }, (_, i) => h('span', { class: `pz-mx-pip${i < n ? ' on' : ''}` })));
}

function mountMistakes(root, { bag, signal }) {
  const page = h('div', { class: 'page page-wide pz-page pz-mx' });
  root.appendChild(page);
  let viewBag = null;
  const swap = () => {
    if (viewBag) viewBag.dispose();
    viewBag = disposables();
    page.replaceChildren();
    return viewBag;
  };
  bag.add(() => { if (viewBag) viewBag.dispose(); viewBag = null; });

  let summary = null;
  let lastId = null;
  let practiceMode = false;
  let loadSeq = 0;

  const header = (subtitle) => pageHeader({
    title: t('puzzles.mistakes.title'), icon: 'target', subtitle,
    breadcrumbs: [{ label: t('puzzles.title'), href: '#/puzzles' }, { label: t('puzzles.mistakes.title') }],
  });

  function statsRow() {
    const s = summary || {};
    const stat = (value, label, cls = '') => h('div', { class: `pz-score pz-mx-stat ${cls}` },
      h('div', { class: 'pz-mx-stat-num tabular' }, formatNumber(value | 0)), h('div', { class: 'stat-label' }, label));
    return h('div', { class: 'pz-scoreboard pz-mx-scoreboard' },
      stat(s.due, t('puzzles.mistakes.statDue'), (s.due | 0) ? 'is-due' : ''),
      stat(s.total, t('puzzles.mistakes.statDeck')),
      stat(s.graduated, t('puzzles.mistakes.statMastered'), 'is-mastered'));
  }

  // ----- Boot: backfill from reviewed games (bounded per call), then the first card -----
  async function boot() {
    const vb = swap();
    page.append(h('div', { class: 'page' }, header(t('puzzles.mistakes.subtitle')), loadingBlock(t('puzzles.mistakes.loading'))));
    for (let i = 0; i < 3; i++) {
      try {
        const r = await api.post('/api/mistakes/sync', {}, { signal });
        if (vb.disposed) return;
        if (r?.summary) summary = r.summary;
        if (r?.added) toast(t('puzzles.mistakes.addedToast', { count: r.added }), 'success');
        if (!r?.more) break;
      } catch (e) {
        if (isAbort(e) || vb.disposed) return;
        break;
      }
    }
    loadNext();
  }

  async function loadNext() {
    const seq = ++loadSeq;
    let res;
    try {
      res = await api.get('/api/mistakes/next' + qs({ exclude: lastId }), { signal });
    } catch (e) {
      if (isAbort(e) || bag.disposed || seq !== loadSeq) return;
      showError(e.status === 404 ? t('puzzles.mistakes.unavailable') : e.message);
      return;
    }
    if (bag.disposed || seq !== loadSeq) return;
    summary = res?.summary || summary;
    const card = res?.card;
    if (!card) { showEmpty(); return; }
    if (!res.due && !practiceMode) { showCaughtUp(card); return; }
    showCard(card, { practice: !res.due });
  }

  function showError(msg) {
    swap();
    page.append(h('div', { class: 'page' }, header(),
      emptyState({ icon: 'alert', title: t('puzzles.mistakes.errorTitle'), text: msg, action: { label: t('puzzles.solver.buttons.reload'), icon: 'refresh', onClick: () => boot() } })));
  }

  // ----- Nothing in the deck (or everything mastered) -----
  function showEmpty() {
    const vb = swap();
    const s = summary || {};
    const allMastered = (s.total | 0) > 0;
    const card = h('div', { class: 'card pz-mx-done' },
      allMastered ? confettiBurst() : null,
      h('div', { class: 'pz-mx-done-emoji', 'aria-hidden': 'true' }, allMastered ? '🏆' : '🌱'),
      h('h2', { class: 'pz-mx-done-title' }, t(allMastered ? 'puzzles.mistakes.allMasteredTitle' : 'puzzles.mistakes.emptyTitle')),
      h('p', { class: 'muted pz-mx-done-text' }, t(allMastered ? 'puzzles.mistakes.allMasteredText' : 'puzzles.mistakes.emptyText')),
      allMastered ? statsRow() : h('ol', { class: 'pz-mx-steps' },
        h('li', null, t('puzzles.mistakes.emptyStep1')),
        h('li', null, t('puzzles.mistakes.emptyStep2')),
        h('li', null, t('puzzles.mistakes.emptyStep3'))),
      h('div', { class: 'row row-wrap pz-mx-done-actions' },
        h('a', { class: 'btn btn-primary btn-lg', href: '#/play', html: icon('play') + `<span>${t('puzzles.mistakes.playBot')}</span>` }),
        h('a', { class: 'btn btn-secondary', href: '#/library', html: icon('library') + `<span>${t('puzzles.mistakes.reviewGame')}</span>` }),
        h('a', { class: 'btn btn-ghost', href: '#/puzzles?play=1', html: icon('puzzle') + `<span>${t('puzzles.mistakes.ratedPuzzles')}</span>` })));
    page.append(h('div', { class: 'page' }, header(t('puzzles.mistakes.subtitle')), card));
    if (allMastered) vb.timeout(() => card.querySelector('.pz-confetti')?.remove(), 4000);
  }

  // ----- Everything due is done: celebrate, show when the next one is due -----
  function showCaughtUp(nextCard) {
    const vb = swap();
    const s = summary || {};
    const secs = Number.isFinite(s.next_due_in_secs) ? s.next_due_in_secs : nextCard?.due_in_secs;
    const card = h('div', { class: 'card pz-mx-done' },
      confettiBurst(),
      h('div', { class: 'pz-mx-done-emoji', 'aria-hidden': 'true' }, '🎉'),
      h('h2', { class: 'pz-mx-done-title' }, t('puzzles.mistakes.caughtUpTitle')),
      h('p', { class: 'muted pz-mx-done-text' }, t('puzzles.mistakes.caughtUpText')),
      Number.isFinite(secs) ? h('div', { class: 'pz-mx-next-due' }, h('span', { html: icon('clock') }), h('span', null, t('puzzles.mistakes.nextDue', { when: relTime(secs) }))) : null,
      statsRow(),
      h('div', { class: 'row row-wrap pz-mx-done-actions' },
        h('a', { class: 'btn btn-primary btn-lg', href: '#/puzzles?play=1', html: icon('puzzle') + `<span>${t('puzzles.mistakes.ratedPuzzles')}</span>` }),
        btn(t('puzzles.mistakes.practiceAnyway'), 'refresh', 'secondary', () => { practiceMode = true; showCard(nextCard, { practice: true }); }),
        h('a', { class: 'btn btn-ghost', href: '#/puzzles', html: icon('grid') + `<span>${t('puzzles.solver.allModes')}</span>` })));
    page.append(h('div', { class: 'page' }, header(t('puzzles.mistakes.subtitle')), card));
    vb.timeout(() => card.querySelector('.pz-confetti')?.remove(), 4000);
  }

  // ----- Solve one card -----
  function showCard(card, { practice }) {
    const vb = swap();
    lastId = card.id;
    const st = { card, practice, feedback: 'loading', failed: false, recorded: false, hintMsg: '', outcome: null, saving: false };

    const slot = h('div', { class: 'board-slot' });
    const turnBar = h('div', { class: 'pz-turnbar' });
    const flipBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': t('puzzles.solver.flipBoard'), 'data-tooltip': t('puzzles.solver.flipBoard'), html: icon('flip') });
    const analyzeLink = h('a', { class: 'btn btn-ghost btn-sm', href: `#/analysis?fen=${encodeURIComponent(card.fen)}`, html: icon('analysis') + `<span>${t('puzzles.solver.analyze')}</span>` });
    const timeEl = h('span', { class: 'tabular' }, '0:00');
    const stopwatch = new Stopwatch(timeEl);
    vb.add(() => stopwatch.destroy());
    const toolbar = h('div', { class: 'toolbar pz-toolbar' }, flipBtn, h('div', { class: 'spacer' }),
      h('span', { class: 'row-sm subtle text-sm' }, h('span', { html: icon('timer') }), timeEl), analyzeLink);

    const progress = h('div', { class: 'pz-mx-progress' });
    const context = h('div', { class: 'pz-mx-context' });
    const status = h('div', { class: 'pz-status', 'aria-live': 'polite' });
    const after = h('div', { class: 'pz-mx-after stack-sm' });
    const actions = h('div', { class: 'pz-actions' });

    const panel = h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header', html: icon('target') + `<span>${t('puzzles.mistakes.title')}</span>` },
        h('div', { class: 'spacer' }),
        h('a', { class: 'btn btn-ghost btn-sm', href: '#/puzzles', html: icon('grid') + `<span>${t('puzzles.solver.allModes')}</span>` })),
      h('div', { class: 'panel-body stack' }, progress, context, status, after),
      h('div', { class: 'panel-footer' }, actions));

    page.append(h('div', { class: 'game-layout no-eval pz-layout', style: '--board-chrome: 124px' },
      h('div', { class: 'game-main' }, turnBar, h('div', { class: 'board-row' }, slot), toolbar),
      h('aside', { class: 'game-panel' }, panel)));

    const board = createBoard(slot, (mv) => runner.handleMove(mv));
    vb.add(() => board.destroy());
    const runner = new PuzzleRunner(board, {
      onReady() {
        stopwatch.run();
        if (st.feedback === 'loading') st.feedback = 'ready';
        render();
      },
      onCorrect() { st.feedback = 'correct'; st.hintMsg = ''; render(); },
      onWrong() {
        st.feedback = 'wrong';
        st.hintMsg = '';
        markFailed();
        render();
        slot.classList.remove('shake'); void slot.offsetWidth; slot.classList.add('shake');
      },
      onSolved() {
        stopwatch.stop();
        st.feedback = st.failed ? 'solvedLate' : 'solved';
        if (!st.failed) record(true);
        render();
      },
      onSolutionDone() { st.feedback = 'solution'; render(); },
      onError(msg) { stopwatch.stop(); st.feedback = 'error'; st.hintMsg = msg; render(); },
    });
    vb.add(() => runner.destroy());

    function markFailed() {
      if (st.failed) return;
      st.failed = true;
      record(false);
    }

    async function record(solved) {
      if (st.recorded) return;
      st.recorded = true;
      st.saving = true;
      try {
        const res = await api.post(`/api/mistakes/${encodeURIComponent(card.id)}/attempt`, { solved, time_ms: Math.round(stopwatch.ms) }, { signal });
        if (vb.disposed || !res) return;
        st.outcome = { ...res, solved };
        if (res.summary) summary = res.summary;
        if (res.graduated_now) sound('gameEnd');
      } catch (e) {
        if (isAbort(e) || vb.disposed) return;
        toast(t('puzzles.errors.saveFailed', { message: e.message }), 'warning');
      } finally {
        st.saving = false;
      }
      if (!vb.disposed) render();
    }

    function renderContext() {
      const meta = classificationMeta(card.classification);
      const opp = String(card.opponent || '').trim();
      const fromLine = [
        opp ? t('puzzles.mistakes.fromGame', { opponent: opp }) : t('puzzles.mistakes.fromAGame'),
        card.move_number ? t('puzzles.mistakes.moveN', { n: card.move_number }).replace(/ /g, '\u00a0') : null,
      ].filter(Boolean).join(' · ');
      const streak = st.outcome?.card?.streak ?? card.streak;
      context.replaceChildren(
        h('div', { class: 'pz-mx-from' },
          h('div', { class: 'pz-mx-from-icon', html: icon(card.bot_id ? 'robot' : 'swords') }),
          h('div', { class: 'pz-mx-from-main' },
            h('div', { class: 'pz-mx-from-line subtle text-sm' }, fromLine),
            h('div', { class: 'pz-mx-played' },
              h('span', null, t('puzzles.mistakes.youPlayed')),
              h('span', { class: 'pz-mx-san', style: { color: meta.cssVar } }, `${card.played_san || card.played_uci}${sanSuffix(card.classification)}`),
              h('span', { class: 'badge pz-mx-cls', style: { '--cls': meta.cssVar } }, meta.label))),
          masteryPips(streak)),
        h('div', { class: 'row-sm pz-mx-tags' },
          card.phase ? h('span', { class: 'badge' }, t(`puzzles.mistakes.phase.${['opening', 'middlegame', 'endgame'].includes(card.phase) ? card.phase : 'middlegame'}`)) : null,
          st.practice ? h('span', { class: 'badge pz-mx-practice' }, t('puzzles.mistakes.practiceBadge')) : null));
    }

    function render() {
      const color = runner.userColor;
      const done = ['solved', 'solvedLate', 'solution'].includes(st.feedback);
      progress.replaceChildren(statsRow());
      renderContext();

      // Turn banner
      let bar;
      if (st.feedback === 'loading') bar = [h('span', { class: 'spinner' }), h('span', null, t('puzzles.mistakes.bar.loading'))];
      else if (st.feedback === 'error') bar = [h('span', { html: icon('alert') }), h('span', null, t('puzzles.solver.bar.error'))];
      else if (st.feedback === 'solved' || st.feedback === 'solvedLate') bar = [h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, t('puzzles.mistakes.bar.solved'))];
      else if (st.feedback === 'solution') bar = [h('span', { html: icon('eye') }), h('span', null, t('puzzles.solver.bar.solution'))];
      else bar = [h('span', { class: `pz-side pz-side-${color}` }), h('span', null, t('puzzles.mistakes.bar.find', { side: sideLabel(color) }))];
      turnBar.className = `pz-turnbar state-${st.feedback}`;
      turnBar.replaceChildren(...bar);

      // Status card
      const o = st.outcome;
      let kind = 'neutral', ic = 'target', head = '', sub = '';
      switch (st.feedback) {
        case 'loading': ic = 'clock'; head = t('puzzles.solver.status.loadingHead'); sub = t('puzzles.mistakes.status.loadingSub'); break;
        case 'ready': head = t('puzzles.mistakes.status.readyHead'); sub = t('puzzles.mistakes.status.readySub', { side: sideLabel(color) }); break;
        case 'correct': kind = 'good'; ic = 'check-circle'; head = t('puzzles.solver.status.correctHead'); sub = t('puzzles.solver.status.correctSub'); break;
        case 'wrong': kind = 'bad'; ic = 'x-circle'; head = t('puzzles.solver.status.wrongHead'); sub = t('puzzles.mistakes.status.wrongSub'); break;
        case 'solved':
          kind = 'good'; ic = 'trophy';
          head = o?.graduated_now ? t('puzzles.mistakes.status.masteredHead') : t('puzzles.mistakes.status.solvedHead');
          sub = o?.graduated_now ? t('puzzles.mistakes.status.masteredSub') : t('puzzles.mistakes.status.solvedSub');
          break;
        case 'solvedLate': kind = 'good'; ic = 'check-circle'; head = t('puzzles.mistakes.status.lateHead'); sub = t('puzzles.mistakes.status.lateSub'); break;
        case 'solution': ic = 'eye'; head = t('puzzles.mistakes.status.solutionHead'); sub = t('puzzles.mistakes.status.solutionSub'); break;
        case 'error': kind = 'bad'; ic = 'alert'; head = t('puzzles.solver.status.errorHead'); sub = st.hintMsg || t('puzzles.solver.status.errorSub'); break;
        default: break;
      }
      if (st.hintMsg && st.feedback === 'ready') sub = st.hintMsg;
      status.className = `pz-status pz-status-${kind}${o?.graduated_now && st.feedback === 'solved' ? ' pz-mx-mastered' : ''}`;
      status.replaceChildren(
        h('div', { class: 'pz-status-icon', html: icon(ic) }),
        h('div', { class: 'pz-status-text' }, h('div', { class: 'pz-status-head' }, head), h('div', { class: 'pz-status-sub' }, sub)));

      // After solving: coach, best move, schedule, links
      if (done) {
        const next = o?.card;
        let schedule = '';
        if (st.saving) schedule = t('puzzles.mistakes.saving');
        else if (o && !o.solved) schedule = t('puzzles.mistakes.schedule.again', { when: relTime(next?.due_in_secs) });
        else if (o?.graduated_now) schedule = t('puzzles.mistakes.schedule.graduated');
        else if (o && !o.counted) schedule = t('puzzles.mistakes.schedule.practice');
        else if (next) schedule = t('puzzles.mistakes.schedule.next', { when: relTime(next.due_in_secs) });
        const explanation = String(card.explanation || '').trim();
        after.replaceChildren(...[
          h('div', { class: 'pz-mx-best' },
            h('span', { class: 'pz-mx-best-icon', html: icon('star') }),
            h('span', null, t('puzzles.mistakes.bestWas')), h('strong', { class: 'pz-mx-san' }, card.best_san || card.best_uci),
            h('span', { class: 'subtle' }, t('puzzles.mistakes.insteadOf', { san: card.played_san || card.played_uci }))),
          explanation ? h('div', { class: 'pz-mx-coach' },
            h('div', { class: 'pz-mx-coach-icon', html: icon('mentor') }),
            h('div', { class: 'pz-mx-coach-body' },
              h('div', { class: 'stat-label' }, t('puzzles.mistakes.coachSays')),
              h('p', null, explanation))) : null,
          schedule ? h('div', { class: 'pz-mx-schedule subtle text-sm' }, h('span', { html: icon('calendar') }), h('span', null, schedule)) : null,
          card.game_id ? h('a', { class: 'btn btn-ghost btn-sm pz-mx-review-link', href: `#/review/${encodeURIComponent(card.game_id)}`, html: icon('analysis') + `<span>${t('puzzles.mistakes.openReview')}</span>` }) : null,
        ].filter(Boolean));
      } else {
        after.replaceChildren();
      }

      // Actions
      const list = [];
      if (st.feedback === 'loading') {
        list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', null, { cls: 'pz-grow' }), btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', null));
        list.forEach((b) => { b.disabled = true; });
      } else if (st.feedback === 'error') {
        list.push(btn(t('puzzles.mistakes.buttons.remove'), 'trash', 'ghost', () => removeCard()));
        list.push(btn(t('puzzles.mistakes.buttons.skip'), 'arrow-right', 'primary', () => loadNext(), { cls: 'pz-grow' }));
      } else if (done) {
        list.push(btn(t('puzzles.solver.buttons.retry'), 'refresh', 'ghost', () => retry(), { key: 'r' }));
        list.push(btn(t('puzzles.mistakes.buttons.remove'), 'trash', 'ghost', () => removeCard(), { title: t('puzzles.mistakes.removeTitle') }));
        list.push(btn(t('puzzles.mistakes.buttons.next'), 'arrow-right', 'primary', () => loadNext(), { key: 'n', cls: 'pz-grow' }));
      } else {
        list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', () => doHint(), { key: 'h', cls: 'pz-grow' }));
        list.push(btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', () => doSolution()));
      }
      actions.replaceChildren(...list);
    }

    function doHint() {
      const stage = runner.hint();
      if (!stage) return;
      markFailed();
      st.hintMsg = stage === 1 ? t('puzzles.solver.hintPiece', { piece: runner.hintPieceName() }) : t('puzzles.solver.hintArrow');
      render();
    }

    function doSolution() {
      markFailed();
      stopwatch.stop();
      st.hintMsg = '';
      runner.showSolution();
      st.feedback = 'solution';
      render();
    }

    function retry() {
      st.feedback = 'loading';
      st.hintMsg = '';
      render();
      runner.retry();
    }

    async function removeCard() {
      const ok = await confirmDialog({ title: t('puzzles.mistakes.removeTitle'), message: t('puzzles.mistakes.removeText'), confirmLabel: t('puzzles.mistakes.removeConfirm'), danger: true });
      if (!ok || vb.disposed) return;
      try {
        const res = await api.del(`/api/mistakes/${encodeURIComponent(card.id)}`, { signal });
        if (vb.disposed) return;
        if (res?.summary) summary = res.summary;
        toast(t('puzzles.mistakes.removedToast'), 'info');
        lastId = null;
        loadNext();
      } catch (e) {
        if (!isAbort(e) && !vb.disposed) toast(e.message, 'error');
      }
    }

    // ----- Events -----
    vb.on(flipBtn, 'click', () => board.flip());
    vb.on(window, 'keydown', (e) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
      const tg = e.target;
      if (tg && (tg.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tg.tagName))) return;
      if (document.querySelector('.modal-backdrop')) return;
      const k = e.key.toLowerCase();
      const done = ['solved', 'solvedLate', 'solution'].includes(st.feedback);
      if (k === 'f') { board.flip(); e.preventDefault(); }
      else if (k === 'h' && !done && st.feedback !== 'loading') { doHint(); e.preventDefault(); }
      else if (k === 'r' && done) { retry(); e.preventDefault(); }
      else if ((k === 'n' || k === 'arrowright') && done) { e.preventDefault(); loadNext(); }
    });

    render();
    if (!runner.load(cardPuzzle(card))) render();
  }

  boot();
}

// ---------------------------------------------------------------------------
// Puzzle Rush
// ---------------------------------------------------------------------------

// Labels are resolved at render time (rushText) so a language switch takes effect.
const RUSH_MODES = {
  '3': { ms: 180000, icon: 'timer' },
  '5': { ms: 300000, icon: 'clock' },
  survival: { ms: 0, icon: 'shield' },
};
function rushText(mode, field) { return t(`puzzles.rush.modes.${mode}.${field}`); }

function rushBest(mode) {
  const v = Number(store(`gm.puzzles.rush.best.${mode}`));
  return Number.isFinite(v) && v > 0 ? v : 0;
}

function mountRush(root, { bag, signal }) {
  const page = h('div', { class: 'page page-wide pz-page pz-rush' });
  root.appendChild(page);
  let viewBag = null;
  const swap = () => {
    if (viewBag) viewBag.dispose();
    viewBag = disposables();
    page.replaceChildren();
    return viewBag;
  };
  bag.add(() => { if (viewBag) viewBag.dispose(); viewBag = null; });

  let serverBest = 0;
  api.get('/api/profile', { signal }).then((pr) => {
    if (bag.disposed || !pr) return;
    serverBest = Number(pr.rush_best) || 0;
    const el = page.querySelector('.pz-rush-overall');
    if (el) el.textContent = String(serverBest);
  }).catch(() => {});

  // ----- Mode select -----
  function showSelect() {
    swap();
    const cards = Object.entries(RUSH_MODES).map(([key, m]) => h('button', {
      type: 'button', class: 'card card-link pz-rush-mode', onClick: () => startRun(key),
    },
    h('div', { class: 'pz-mode-icon', html: icon(m.icon) }),
    h('div', { class: 'pz-mode-title' }, rushText(key, 'label')),
    h('p', { class: 'muted text-sm' }, rushText(key, 'desc')),
    h('div', { class: 'pz-rush-best' }, h('span', { class: 'subtle text-sm' }, t('puzzles.rush.best')), h('span', { class: 'tabular bold' }, String(rushBest(key))))));

    page.append(h('div', { class: 'page' },
      pageHeader({
        title: t('puzzles.rushTitle'), icon: 'bolt', subtitle: t('puzzles.rush.subtitle'),
        breadcrumbs: [{ label: t('puzzles.title'), href: '#/puzzles' }, { label: t('puzzles.rushTitle') }],
      }),
      h('div', { class: 'card pz-rush-hero' },
        h('div', { class: 'pz-strikes lg', 'aria-hidden': 'true' }, [0, 1, 2].map(() => h('span', { class: 'pz-strike', html: icon('x') }))),
        h('div', null,
          h('div', { class: 'semibold' }, t('puzzles.rush.strikesTitle')),
          h('p', { class: 'muted text-sm' }, t('puzzles.rush.strikesText'))),
        h('div', { class: 'pz-rush-overall-wrap' }, h('div', { class: 'stat-label' }, t('puzzles.rush.allTimeBest')), h('div', { class: 'pz-rush-overall stat-value tabular' }, String(serverBest)))),
      h('div', { class: 'grid-3 pz-modes mt-4' }, cards)));
  }

  // ----- Run -----
  function startRun(modeKey) {
    const mode = RUSH_MODES[modeKey] ? modeKey : '3';
    const cfg = RUSH_MODES[mode];
    const vb = swap();
    const timers = new Timers();
    vb.add(() => timers.clear());

    const run = { mode, score: 0, strikes: 0, results: [], queue: [], seen: new Set(), fetching: null, exhausted: false, lastRating: 0, ended: false, current: null, startedAt: 0, deadline: 0, clockId: 0 };

    const slot = h('div', { class: 'board-slot' });
    const turnBar = h('div', { class: 'pz-turnbar' }, h('span', { class: 'spinner' }), h('span', null, t('puzzles.rush.getReady')));
    const clockEl = h('div', { class: 'pz-rush-clock tabular' }, cfg.ms ? formatClock(cfg.ms, { tenths: false }) : '0:00');
    const scoreEl = h('div', { class: 'pz-rush-score tabular' }, '0');
    const strikesEl = h('div', { class: 'pz-strikes' }, [0, 1, 2].map(() => h('span', { class: 'pz-strike', html: icon('x') })));
    const tilesEl = h('div', { class: 'pz-tiles' });
    const quitBtn = btn(t('puzzles.rush.endRun'), 'flag', 'ghost', () => endRun('quit'));

    const panel = h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header', html: icon('bolt') + `<span>${t('puzzles.rush.panelTitle', { mode: rushText(mode, 'short') })}</span>` }),
      h('div', { class: 'panel-body stack' },
        h('div', { class: 'pz-rush-top' },
          h('div', null, h('div', { class: 'stat-label' }, cfg.ms ? t('puzzles.rush.timeLeft') : t('puzzles.rush.time')), clockEl),
          h('div', { class: 'text-center' }, h('div', { class: 'stat-label' }, t('puzzles.rush.score')), scoreEl)),
        h('div', { class: 'row between' }, h('div', { class: 'stat-label' }, t('puzzles.rush.strikes')), strikesEl),
        h('div', { class: 'stat-label' }, t('puzzles.rush.results')),
        tilesEl),
      h('div', { class: 'panel-footer' }, quitBtn));

    page.append(h('div', { class: 'game-layout no-eval pz-layout', style: '--board-chrome: 124px' },
      h('div', { class: 'game-main' }, turnBar, h('div', { class: 'board-row' }, slot), h('div', { class: 'toolbar pz-toolbar' },
        h('span', { class: 'subtle text-sm' }, t('puzzles.rush.harder')))),
      h('aside', { class: 'game-panel' }, panel)));

    const board = createBoard(slot, (mv) => runner.handleMove(mv));
    vb.add(() => board.destroy());
    const runner = new PuzzleRunner(board, {
      onReady() {
        const c = runner.userColor;
        turnBar.className = 'pz-turnbar';
        turnBar.replaceChildren(h('span', { class: `pz-side pz-side-${c}` }), h('span', null, toMove(c)));
      },
      onCorrect() {
        turnBar.className = 'pz-turnbar state-correct';
        turnBar.replaceChildren(h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, t('puzzles.rush.correct')));
      },
      onSolved() {
        if (run.ended) return;
        run.score++;
        scoreEl.textContent = String(run.score);
        scoreEl.classList.remove('bump'); void scoreEl.offsetWidth; scoreEl.classList.add('bump');
        addTile(true);
        turnBar.className = 'pz-turnbar state-solved';
        turnBar.replaceChildren(h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, t('puzzles.rush.solved')));
        timers.later(nextPuzzle, 420);
      },
      onWrong() {
        if (run.ended) return;
        run.strikes++;
        const s = strikesEl.children[run.strikes - 1];
        if (s) s.classList.add('on', 'pop-in');
        addTile(false);
        turnBar.className = 'pz-turnbar state-wrong';
        turnBar.replaceChildren(h('span', { html: icon('x-circle') }), h('span', null, run.strikes >= 3 ? t('puzzles.rush.strikeThree') : t('puzzles.rush.strikeN', { n: run.strikes })));
        if (run.strikes >= 3) timers.later(() => endRun('strikes'), 800);
        else timers.later(nextPuzzle, 800);
      },
      onError() { if (!run.ended) timers.later(nextPuzzle, 200); },
    }, { firstMoveDelay: 320, replyDelay: 260, allowRetry: false });
    vb.add(() => runner.destroy());

    function addTile(ok) {
      const p = run.current;
      if (!p) return;
      run.results.push({ puzzle: p, solved: ok });
      tilesEl.appendChild(h('a', {
        class: `pz-tile pop-in ${ok ? 'ok' : 'bad'}`, href: analysisHref(p), target: '_self',
        title: t(ok ? 'puzzles.rush.tileSolved' : 'puzzles.rush.tileMissed', { rating: p.rating }),
        html: icon(ok ? 'check' : 'x') + `<span>${Number(p.rating) || ''}</span>`,
      }));
      tilesEl.scrollTop = tilesEl.scrollHeight;
    }

    function fetchMore() {
      if (run.fetching || run.exhausted) return run.fetching;
      run.fetching = api.get('/api/puzzles/rush' + qs({ count: 40 }), { signal }).then((list) => {
        if (vb.disposed) return;
        const fresh = (Array.isArray(list) ? list : []).filter((p) => validPuzzle(p) && !run.seen.has(p.id));
        if (!fresh.length) { run.exhausted = true; return; }
        fresh.sort((a, b) => (a.rating || 0) - (b.rating || 0));
        // Keep difficulty climbing: prefer puzzles at least as hard as the last one queued.
        const floor = (run.queue.length ? run.queue[run.queue.length - 1].rating : run.lastRating) - 50;
        const harder = fresh.filter((p) => (p.rating || 0) >= floor);
        for (const p of (harder.length >= 5 ? harder : fresh)) { run.seen.add(p.id); run.queue.push(p); }
      }).catch((e) => {
        if (!isAbort(e) && !vb.disposed && !run.queue.length) toast(t('puzzles.errors.loadPuzzlesFailed', { message: e.message }), 'error');
        if (!run.queue.length) run.exhausted = true;
      }).finally(() => { run.fetching = null; });
      return run.fetching;
    }

    async function nextPuzzle() {
      if (run.ended || vb.disposed) return;
      if (run.queue.length < 10) fetchMore();
      if (!run.queue.length) {
        board.setInteractive(false, null);
        if (run.fetching) await run.fetching;
        if (run.ended || vb.disposed) return;
        if (!run.queue.length) { endRun('empty'); return; }
      }
      const p = run.queue.shift();
      run.current = p;
      run.lastRating = p.rating || run.lastRating;
      turnBar.className = 'pz-turnbar';
      runner.load(p);
    }

    function tick() {
      const now = performance.now();
      if (cfg.ms) {
        const left = Math.max(0, run.deadline - now);
        clockEl.textContent = formatClock(left, { tenths: left < 10000 });
        clockEl.classList.toggle('low', left < 15000);
        if (left <= 0) endRun('time');
      } else {
        clockEl.textContent = formatClock(now - run.startedAt, { tenths: false });
      }
    }
    function stopClock() { if (run.clockId) clearInterval(run.clockId); run.clockId = 0; }
    vb.add(stopClock);

    async function endRun(reason) {
      if (run.ended) return;
      run.ended = true;
      stopClock();
      timers.clear();
      runner.destroy();
      board.setInteractive(false, null);
      sound('gameEnd');
      const prevLocal = rushBest(mode);
      let best = Math.max(serverBest, run.score);
      try {
        const res = await api.post('/api/puzzles/rush', { score: run.score }, { signal });
        if (res && Number.isFinite(res.best)) best = res.best;
      } catch (e) {
        if (isAbort(e)) return;
      }
      if (bag.disposed) return;
      const isPb = run.score > prevLocal && run.score > 0;
      if (isPb) store(`gm.puzzles.rush.best.${mode}`, run.score);
      serverBest = Math.max(serverBest, best);
      showEnd({ mode, score: run.score, results: run.results.slice(), reason, isPb, modeBest: Math.max(prevLocal, run.score), overall: serverBest });
    }

    // Boot: fetch first batch, then start the clock with the first puzzle.
    (async () => {
      await fetchMore();
      if (vb.disposed || run.ended) return;
      if (!run.queue.length) {
        page.replaceChildren(emptyState({ icon: 'puzzle', title: t('puzzles.rush.emptyTitle'), text: t('puzzles.rush.emptyText'), action: { label: t('puzzles.rush.backToPuzzles'), href: '#/puzzles' } }));
        return;
      }
      run.startedAt = performance.now();
      run.deadline = run.startedAt + cfg.ms;
      run.clockId = setInterval(tick, 100);
      sound('notify');
      nextPuzzle();
    })();
  }

  // ----- End screen -----
  function showEnd({ mode, score, results, reason, isPb, modeBest, overall }) {
    const vb = swap();
    const reasonText = t(reason === 'time' ? 'puzzles.rush.end.reasonTime' : reason === 'strikes' ? 'puzzles.rush.end.reasonStrikes' : reason === 'empty' ? 'puzzles.rush.end.reasonEmpty' : 'puzzles.rush.end.reasonQuit');
    const solvedCount = results.filter((r) => r.solved).length;
    const hardest = results.filter((r) => r.solved).reduce((m, r) => Math.max(m, r.puzzle.rating || 0), 0);

    const confetti = isPb ? h('div', { class: 'pz-confetti', 'aria-hidden': 'true' },
      Array.from({ length: 36 }, (_, i) => h('span', {
        style: { '--x': `${(i * 37) % 100}%`, '--d': `${(i % 9) * 70}ms`, '--r': `${(i * 53) % 360}deg`, '--c': `var(--${['primary', 'gold', 'info', 'accent', 'cls-brilliant'][i % 5]})` },
      }))) : null;

    const card = h('div', { class: 'card pz-end' },
      confetti,
      h('div', { class: 'result-hero' },
        h('div', { class: 'subtle semibold' }, t('puzzles.rush.end.header', { mode: rushText(mode, 'label'), reason: reasonText })),
        h('div', { class: 'pz-end-score tabular pop-in' }, String(score)),
        h('div', { class: 'result-hero-title' }, t(isPb ? 'puzzles.rush.end.newBest' : score > 0 ? 'puzzles.rush.end.niceRun' : 'puzzles.rush.end.keepPractising')),
        h('div', { class: 'result-hero-sub muted' }, isPb ? t('puzzles.rush.end.beatBest') : t('puzzles.rush.end.modeBest', { mode: rushText(mode, 'short'), best: modeBest }))),
      h('div', { class: 'grid-3 pz-end-stats' },
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, t('puzzles.rush.end.solved')), h('div', { class: 'stat-value tabular' }, String(solvedCount))),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, t('puzzles.rush.end.hardest')), h('div', { class: 'stat-value tabular' }, hardest ? String(hardest) : '—')),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, t('puzzles.rush.allTimeBest')), h('div', { class: 'stat-value tabular' }, String(overall)))),
      results.length ? h('div', { class: 'stack-sm' },
        h('div', { class: 'stat-label' }, t('puzzles.rush.end.review')),
        h('div', { class: 'pz-tiles pz-tiles-end' }, results.map((r, i) => h('a', {
          class: `pz-tile ${r.solved ? 'ok' : 'bad'}`, href: analysisHref(r.puzzle),
          title: t(r.solved ? 'puzzles.rush.end.tileSolved' : 'puzzles.rush.end.tileMissed', { n: i + 1, rating: r.puzzle.rating }),
          html: icon(r.solved ? 'check' : 'x') + `<span>${Number(r.puzzle.rating) || ''}</span>`,
        })))) : null,
      h('div', { class: 'row row-wrap pz-end-actions' },
        btn(t('puzzles.rush.end.playAgain'), 'refresh', 'primary', () => startRun(mode), { cls: 'btn-lg' }),
        btn(t('puzzles.rush.end.changeMode'), 'grid', 'secondary', () => showSelect()),
        h('a', { class: 'btn btn-ghost', href: '#/puzzles', html: icon('puzzle') + `<span>${t('puzzles.rush.end.allPuzzles')}</span>` })));
    page.append(h('div', { class: 'pz-end-wrap' }, card));
    if (isPb) vb.timeout(() => { if (confetti) confetti.remove(); }, 4000);
  }

  showSelect();
}
