// GrandMentor — Puzzles page.
// Routes (see app.js): #/puzzles (hub), #/puzzles?play=1[&theme=fork] (rated solver),
// #/puzzles/rush (params.mode = 'rush'), #/puzzles/daily (params.mode = 'daily').
// Contract: docs/CONTRACT.md §4 (puzzle API) and §5 (Board, sound); UX: docs/FEATURES.md §3.5–3.6.
//
// Memory hygiene: every view owns a `disposables()` bag; every puzzle runner owns a timer set that is
// cleared on each new puzzle and on destroy; all fetches share an AbortController aborted on unmount.

import { h, icon, pageHeader, disposables, loadingBlock, emptyState, toast, formatClock } from '../ui.js';
import { api, qs, isAbort } from '../api.js';
import { getSetting } from '../settings.js';
import { Board } from '../components/board.js';
import { playSound } from '../components/sound.js';
import { Chess } from '../../vendor/chess.js';

export const title = (params) => (params?.mode === 'rush' ? 'Puzzle Rush' : params?.mode === 'daily' ? 'Daily Puzzle' : 'Puzzles');

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

const THEME_LABELS = {
  mate: 'Checkmate', mateIn1: 'Mate in 1', mateIn2: 'Mate in 2', mateIn3: 'Mate in 3', mateIn4: 'Mate in 4', mateIn5: 'Mate in 5+',
  fork: 'Fork', pin: 'Pin', skewer: 'Skewer', discoveredAttack: 'Discovered attack', doubleCheck: 'Double check',
  hangingPiece: 'Hanging piece', backRankMate: 'Back-rank mate', smotheredMate: 'Smothered mate', sacrifice: 'Sacrifice',
  deflection: 'Deflection', decoy: 'Decoy', attraction: 'Attraction', clearance: 'Clearance', interference: 'Interference',
  intermezzo: 'In-between move', quietMove: 'Quiet move', defensiveMove: 'Defensive move', zugzwang: 'Zugzwang',
  xRayAttack: 'X-ray attack', trappedPiece: 'Trapped piece', capturingDefender: 'Remove the defender', exposedKing: 'Exposed king',
  kingsideAttack: 'Kingside attack', queensideAttack: 'Queenside attack', promotion: 'Promotion', underPromotion: 'Underpromotion',
  advancedPawn: 'Advanced pawn', enPassant: 'En passant', castling: 'Castling', opening: 'Opening', middlegame: 'Middlegame',
  endgame: 'Endgame', rookEndgame: 'Rook endgame', pawnEndgame: 'Pawn endgame', queenEndgame: 'Queen endgame',
  bishopEndgame: 'Bishop endgame', knightEndgame: 'Knight endgame', short: 'Short', long: 'Long', veryLong: 'Very long',
  oneMove: 'One-move', crushing: 'Crushing', advantage: 'Advantage', equality: 'Equality', master: 'Master games',
  arabianMate: 'Arabian mate', anastasiaMate: "Anastasia's mate", bodenMate: "Boden's mate", hookMate: 'Hook mate',
  doubleBishopMate: 'Two-bishop mate', dovetailMate: 'Dovetail mate', attackingF2F7: 'Attacking f2/f7',
};
const THEME_EMOJI = {
  mate: '♚', mateIn1: '1️⃣', mateIn2: '2️⃣', mateIn3: '3️⃣', fork: '🍴', pin: '📌', skewer: '🍢', discoveredAttack: '💥',
  hangingPiece: '🎁', backRankMate: '🧱', sacrifice: '🔥', endgame: '🏁', opening: '📖', middlegame: '⚔️', promotion: '👑',
  deflection: '↪️', trappedPiece: '🪤', doubleCheck: '⚡', smotheredMate: '🐴', kingsideAttack: '🎯', defensiveMove: '🛡️',
};
function themeLabel(t) {
  const s = String(t || '');
  return THEME_LABELS[s] || s.replace(/([a-z])([A-Z0-9])/g, '$1 $2').replace(/^./, (c) => c.toUpperCase());
}
// Themes that describe length / evaluation are noise as filters for beginners.
const HIDDEN_FILTER_THEMES = new Set(['short', 'long', 'veryLong', 'oneMove', 'crushing', 'advantage', 'equality', 'master', 'masterVsMaster', 'superGM']);

function validPuzzle(p) {
  return p && typeof p.id === 'string' && typeof p.fen === 'string' && Array.isArray(p.moves) && p.moves.length >= 2;
}

/** FEN of the position the user has to solve (after the opponent's set-up move). */
function solveFen(p) {
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
    const t = Math.min(1, (now - start) / ms);
    const e = 1 - Math.pow(1 - t, 3);
    el.textContent = String(Math.round(from + (to - from) * e));
    if (t < 1) id = requestAnimationFrame(step); else id = 0;
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

function sideLabel(color) { return color === 'white' ? 'White' : 'Black'; }

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
    if (!validPuzzle(puzzle)) { this._fail('This puzzle could not be loaded.'); return false; }
    let chess;
    try { chess = new Chess(puzzle.fen); } catch { this._fail('This puzzle has an invalid position.'); return false; }
    this.puzzle = puzzle;
    this.chess = chess;
    this.idx = 0;
    this.hintStage = 0;
    this.lastMove = null;
    this.state = 'intro';
    // The opponent moves first, so the solver plays the side NOT to move in the FEN.
    this.userColor = chess.turn() === 'w' ? 'black' : 'white';
    this.board.setInteractive(false, null);
    if (this.board.orientation !== this.userColor) this.board.setOrientation(this.userColor);
    this.board.setPosition(puzzle.fen, { animate: false });
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
    if (!m) { this._fail('This puzzle contains an illegal move. Skip to the next one.'); return false; }
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
      return { p: 'pawn', n: 'knight', b: 'bishop', r: 'rook', q: 'queen', k: 'king' }[p?.type] || 'piece';
    } catch { return 'piece'; }
  }

  /** Animate the remaining solution moves. */
  showSolution() {
    if (!this.puzzle || ['solution', 'shown', 'broken', 'dead', 'idle'].includes(this.state)) return;
    this.timers.clear();
    this._clearMarks();
    if (this.state === 'intro') {
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
      const isUser = this.idx % 2 === 1;
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
    pageHeader({ title: 'Puzzles', subtitle: 'Sharpen your tactics one move at a time', icon: 'puzzle' }));
  const body = h('div', { class: 'stack-lg' }, loadingBlock('Loading your puzzles…'));
  page.appendChild(body);
  root.appendChild(page);

  const [profile, themes] = await Promise.all([
    api.get('/api/profile', { signal }).catch((e) => (isAbort(e) ? null : null)),
    api.get('/api/puzzles/themes', { signal }).catch(() => []),
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
    h('div', { class: 'pz-mode-title' }, 'Rated Puzzles'),
    h('p', { class: 'muted' }, 'Puzzles matched to your level. Solve them to raise your puzzle rating.'),
    h('div', { class: 'pz-rating-big' },
      h('span', { class: 'pz-rating-num tabular' }, rating != null ? String(rating) : '—'),
      h('span', { class: 'subtle' }, 'puzzle rating')),
    h('div', { class: 'row-sm subtle text-sm' },
      h('span', null, `${solved} solved`), h('span', { class: 'dot-sep' }), h('span', null, total ? `${pct}% success` : 'No attempts yet')),
    h('span', { class: 'btn btn-primary btn-lg btn-block mt-3', html: icon('play') + '<span>Solve puzzles</span>' }));

  const rush = h('a', { class: 'card card-link pz-mode pz-mode-rush', href: '#/puzzles/rush' },
    h('div', { class: 'pz-mode-icon', html: icon('bolt') }),
    h('div', { class: 'pz-mode-title' }, 'Puzzle Rush'),
    h('p', { class: 'muted' }, 'Solve as many as you can against the clock. Three mistakes and you’re out!'),
    h('div', { class: 'pz-rating-big' },
      h('span', { class: 'pz-rating-num tabular' }, String(rushBest)),
      h('span', { class: 'subtle' }, 'best score')),
    h('div', { class: 'row-sm subtle text-sm' }, h('span', null, '3 min'), h('span', { class: 'dot-sep' }), h('span', null, '5 min'), h('span', { class: 'dot-sep' }), h('span', null, 'Survival')),
    h('span', { class: 'btn btn-secondary btn-lg btn-block mt-3', html: icon('bolt') + '<span>Start a rush</span>' }));

  const daily = h('a', { class: 'card card-link pz-mode pz-mode-daily', href: '#/puzzles/daily' },
    calendarCard(now, dailyDone),
    h('div', { class: 'pz-mode-title' }, 'Daily Puzzle'),
    h('p', { class: 'muted' }, 'A fresh puzzle every day — the same one for everybody. It won’t change your rating.'),
    h('div', { class: 'row-sm subtle text-sm' },
      h('span', { class: 'pz-flame', html: icon('fire') }), h('span', null, `${streak} day streak`)),
    h('span', { class: `btn ${dailyDone ? 'btn-ghost' : 'btn-secondary'} btn-lg btn-block mt-3`, html: icon(dailyDone ? 'check' : 'calendar') + `<span>${dailyDone ? 'Solved — view again' : 'Solve today’s puzzle'}</span>` }));

  const content = [h('div', { class: 'grid-3 pz-modes' }, rated, rush, daily)];

  const list = Array.isArray(themes) ? themes.filter((t) => t && t.theme && !HIDDEN_FILTER_THEMES.has(t.theme)) : [];
  list.sort((a, b) => (b.count || 0) - (a.count || 0));
  if (list.length) {
    content.push(h('section', null,
      h('h2', { class: 'section-title' }, 'Train a theme'),
      h('p', { class: 'muted mb-3' }, 'Pick a pattern to practise. Each puzzle is still rated.'),
      h('div', { class: 'pz-theme-grid' }, list.slice(0, 24).map((t) =>
        h('a', { class: 'pz-theme-tile', href: `#/puzzles?play=1&theme=${encodeURIComponent(t.theme)}` },
          h('span', { class: 'pz-theme-emoji', 'aria-hidden': 'true' }, THEME_EMOJI[t.theme] || '♟'),
          h('span', { class: 'pz-theme-name' }, themeLabel(t.theme)),
          h('span', { class: 'pz-theme-count subtle tabular' }, String(t.count ?? '')))))));
  }

  content.push(h('section', null,
    h('h2', { class: 'section-title' }, 'How puzzles work'),
    h('div', { class: 'grid-3 pz-howto' },
      howto('1', 'Watch the move', 'Your opponent makes a move. Now it’s your turn — the banner tells you which colour you play.'),
      howto('2', 'Find the best reply', 'Look for checks, captures and threats first. Stuck? Tap Hint to see which piece to move.'),
      howto('3', 'Keep going', 'Some puzzles take several moves. Get them all right to win rating points!'))));

  body.replaceChildren(...content);
}

function howto(n, t, text) {
  return h('div', { class: 'card card-sm pz-howto-card' },
    h('div', { class: 'pz-howto-num' }, n),
    h('div', null, h('div', { class: 'semibold' }, t), h('p', { class: 'muted text-sm' }, text)));
}

function calendarCard(date, done = false) {
  return h('div', { class: `pz-cal${done ? ' done' : ''}`, 'aria-label': date.toDateString() },
    h('div', { class: 'pz-cal-month' }, date.toLocaleDateString(undefined, { month: 'short' })),
    h('div', { class: 'pz-cal-day tabular' }, String(date.getDate())),
    h('div', { class: 'pz-cal-wd' }, date.toLocaleDateString(undefined, { weekday: 'short' })),
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
  const flipBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': 'Flip board', 'data-tooltip': 'Flip board', html: icon('flip') });
  const analyzeLink = h('a', { class: 'btn btn-ghost btn-sm', href: '#/analysis', html: icon('analysis') + '<span>Analyze</span>' });
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
  const historyEl = h('div', { class: 'pz-history', 'aria-label': 'Session history' });
  const actions = h('div', { class: 'pz-actions' });
  const chipsEl = h('div', { class: 'chip-row pz-chips' });

  const topCard = isDaily
    ? h('div', { class: 'pz-daily-head' }, calendarCard(new Date(), store(`gm.puzzles.daily.${todayKey()}`) === '1'),
      h('div', null,
        h('div', { class: 'pz-daily-title' }, 'Daily Puzzle'),
        h('div', { class: 'muted text-sm' }, new Date().toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' })),
        h('div', { class: 'row-sm text-sm mt-1' }, h('span', { class: 'pz-flame', html: icon('fire') }), h('span', { class: 'pz-daily-streak' }, '—'))))
    : h('div', { class: 'pz-scoreboard' },
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, 'Rating'), h('div', { class: 'row-sm' }, ratingNum, deltaEl)),
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, 'Streak'), h('div', { class: 'pz-score-sm row-sm' }, h('span', { class: 'pz-flame', html: icon('fire') }), streakNum)),
      h('div', { class: 'pz-score' }, h('div', { class: 'stat-label' }, 'Time'), h('div', { class: 'pz-score-sm row-sm' }, h('span', { class: 'subtle', html: icon('timer') }), timeEl)));

  const panelBody = h('div', { class: 'panel-body stack' },
    topCard, status, info,
    isDaily ? null : h('div', { class: 'stack-sm' }, h('div', { class: 'stat-label' }, 'Theme'), chipsEl),
    isDaily ? null : h('div', { class: 'stack-sm' }, h('div', { class: 'stat-label' }, 'This session'), historyEl));

  const panel = h('div', { class: 'panel grow' },
    h('div', { class: 'panel-header', html: icon(isDaily ? 'calendar' : 'puzzle') + `<span>${isDaily ? 'Daily Puzzle' : 'Puzzles'}</span>` },
      h('div', { class: 'spacer' }),
      h('a', { class: 'btn btn-ghost btn-sm', href: '#/puzzles', html: icon('grid') + '<span>All modes</span>' })),
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
    if (isDaily) toast('Daily puzzle solved! See you tomorrow 🎉', 'success');
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
      if (!isAbort(e)) toast(`Couldn’t save your result: ${e.message}`, 'warning');
    }
  }

  // ----- Rendering -----
  function render() {
    const color = runner.userColor;
    streakNum.textContent = String(st.streak);
    const done = ['solved', 'solvedLate', 'solution'].includes(st.feedback);

    // Turn banner above the board
    let bar;
    if (st.feedback === 'loading') bar = [h('span', { class: 'spinner' }), h('span', null, 'Loading puzzle…')];
    else if (st.feedback === 'error') bar = [h('span', { html: icon('alert') }), h('span', null, 'Puzzle unavailable')];
    else if (st.feedback === 'solved' || st.feedback === 'solvedLate') bar = [h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, 'Solved!')];
    else if (st.feedback === 'solution') bar = [h('span', { html: icon('eye') }), h('span', null, 'Solution')];
    else bar = [h('span', { class: `pz-side pz-side-${color}` }), h('span', null, `${sideLabel(color)} to move`)];
    turnBar.className = `pz-turnbar state-${st.feedback}`;
    turnBar.replaceChildren(...bar);

    // Status card
    let kind = 'neutral', ic = 'target', head = '', sub = '';
    switch (st.feedback) {
      case 'loading': head = 'Getting your puzzle ready…'; sub = 'Watch your opponent’s move.'; ic = 'clock'; break;
      case 'ready': head = `${sideLabel(color)} to move`; sub = `Find the best move for ${sideLabel(color)}.`; break;
      case 'correct': kind = 'good'; ic = 'check-circle'; head = 'Correct!'; sub = 'Keep going — find the next move.'; break;
      case 'wrong': kind = 'bad'; ic = 'x-circle'; head = 'Not quite'; sub = st.rated ? 'That move doesn’t work. Try again, or see the solution.' : 'Try again — you’ve got this.'; break;
      case 'solved': kind = 'good'; ic = 'trophy'; head = isDaily ? 'Daily puzzle solved!' : 'Solved!'; sub = isDaily ? 'Come back tomorrow for a new one.' : 'Great job. Ready for the next one?'; break;
      case 'solvedLate': kind = 'good'; ic = 'check-circle'; head = 'You found it!'; sub = st.rated ? 'It won’t count for rating this time, but you learned the idea.' : 'Nice persistence.'; break;
      case 'solution': ic = 'eye'; head = 'Here’s the solution'; sub = 'Study the idea, then try it yourself with Retry.'; break;
      case 'error': kind = 'bad'; ic = 'alert'; head = 'Something went wrong'; sub = st.errorMsg || 'Please try another puzzle.'; break;
      default: break;
    }
    if (st.hintMsg && st.feedback === 'ready') { sub = st.hintMsg; }
    if (!st.rated && !isDaily && st.puzzle && st.feedback === 'ready') sub += ' (Practice — not rated.)';
    status.className = `pz-status pz-status-${kind}`;
    status.replaceChildren(
      h('div', { class: 'pz-status-icon', html: icon(ic) }),
      h('div', { class: 'pz-status-text' }, h('div', { class: 'pz-status-head' }, head), h('div', { class: 'pz-status-sub' }, sub)));

    // Puzzle info (themes only after finishing, to avoid spoilers)
    const p = st.puzzle;
    if (p) {
      info.replaceChildren(...[
        h('div', { class: 'row-sm text-sm' },
          h('span', { class: 'subtle' }, 'Puzzle rating'), h('span', { class: 'semibold tabular' }, String(p.rating ?? '?')),
          !st.rated && !isDaily ? h('span', { class: 'badge' }, 'Practice') : null),
        done && Array.isArray(p.themes) && p.themes.length
          ? h('div', { class: 'pz-tags' }, p.themes.filter((t) => !HIDDEN_FILTER_THEMES.has(t)).slice(0, 6).map((t) => h('span', { class: 'badge' }, themeLabel(t))))
          : null].filter(Boolean));
      analyzeLink.href = analysisHref(p);
    } else {
      info.replaceChildren();
    }

    // Actions
    const list = [];
    const next = btn(isDaily ? 'More puzzles' : 'Next puzzle', 'arrow-right', 'primary', isDaily ? () => { location.hash = '#/puzzles?play=1'; } : () => nextPuzzle(), { key: 'n', cls: 'pz-grow' });
    if (st.feedback === 'loading') {
      list.push(btn('Hint', 'hint', 'secondary', null, { cls: 'pz-grow' }), btn('Solution', 'eye', 'ghost', null));
      list.forEach((b) => { b.disabled = true; });
    } else if (st.feedback === 'error') {
      if (!isDaily) list.push(btn('Try another', 'refresh', 'primary', () => nextPuzzle(), { cls: 'pz-grow' }));
      else list.push(btn('Reload', 'refresh', 'primary', () => loadDaily(), { cls: 'pz-grow' }));
    } else if (done) {
      list.push(btn('Retry', 'refresh', 'ghost', () => retry(), { key: 'r' }), next);
    } else {
      list.push(btn('Hint', 'hint', 'secondary', () => doHint(), { key: 'h', title: st.rated && !st.failed ? 'Using a hint counts as a miss' : null }));
      list.push(btn('Solution', 'eye', 'ghost', () => doSolution()));
      if (st.failed && st.feedback !== 'correct') {
        list.push(btn('Retry', 'refresh', 'ghost', () => retry(), { key: 'r' }));
        if (!isDaily) list.push(btn('Next', 'arrow-right', 'primary', () => nextPuzzle(), { key: 'n', cls: 'pz-grow' }));
      }
    }
    actions.replaceChildren(...list);

    themeLink.textContent = st.theme ? themeLabel(st.theme) : '';
  }

  function renderHistory() {
    if (isDaily) return;
    if (!st.history.length) {
      historyEl.replaceChildren(h('span', { class: 'subtle text-sm' }, 'Your results will appear here.'));
      return;
    }
    historyEl.replaceChildren(...st.history.map((e, i) => h('button', {
      type: 'button',
      class: `pz-dot ${e.solved ? 'ok' : 'bad'}${e.puzzle === st.puzzle ? ' current' : ''}`,
      'aria-label': `Puzzle ${i + 1}: ${e.solved ? 'solved' : 'missed'} (rating ${e.puzzle.rating}). Practise again.`,
      title: `${e.solved ? 'Solved' : 'Missed'} · ${e.puzzle.rating} — click to practise`,
      html: icon(e.solved ? 'check' : 'x'),
      onClick: () => startPuzzle(e.puzzle, { rated: false }),
    })));
  }

  function renderChips(themes) {
    if (isDaily) return;
    const list = (themes || []).filter((t) => t && t.theme && !HIDDEN_FILTER_THEMES.has(t.theme));
    list.sort((a, b) => (b.count || 0) - (a.count || 0));
    const top = list.slice(0, 14).map((t) => t.theme);
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
    chipsEl.replaceChildren(mk('', 'All'), ...top.map((t) => mk(t, themeLabel(t))));
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
      if (!validPuzzle(p)) throw new Error('No puzzle found for this theme.');
      startPuzzle(p, { rated: true });
    } catch (e) {
      if (isAbort(e) || bag.disposed || seq !== fetchSeq) return;
      st.feedback = 'error';
      st.errorMsg = e.status === 404 ? 'No puzzles match this theme right now. Try “All”.' : e.message;
      render();
    }
  }

  async function loadDaily() {
    st.feedback = 'loading';
    render();
    try {
      const p = await api.get('/api/puzzles/daily', { signal });
      if (bag.disposed) return;
      if (!validPuzzle(p)) throw new Error('Today’s puzzle is not available.');
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
    st.hintMsg = stage === 1 ? `Hint: look at your ${runner.hintPieceName()}.` : 'Hint: follow the green arrow.';
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
    const t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
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
      if (el) el.textContent = `${pr.streak_days ?? 0} day streak`;
    }).catch(() => {});
  } else {
    api.get('/api/profile', { signal }).then((pr) => {
      if (bag.disposed || !pr || !Number.isFinite(pr.puzzle_rating)) return;
      if (!Number.isFinite(st.rating)) { st.rating = Math.round(pr.puzzle_rating); ratingNum.textContent = String(st.rating); }
    }).catch(() => {});
    renderChips([]);
    api.get('/api/puzzles/themes', { signal }).then((t) => { if (!bag.disposed) renderChips(Array.isArray(t) ? t : []); }).catch(() => {});
    nextPuzzle();
  }
}

// ---------------------------------------------------------------------------
// Puzzle Rush
// ---------------------------------------------------------------------------

const RUSH_MODES = {
  '3': { label: '3 Minutes', short: '3 min', ms: 180000, icon: 'timer', desc: 'A quick sprint against the clock.' },
  '5': { label: '5 Minutes', short: '5 min', ms: 300000, icon: 'clock', desc: 'More time to think — the classic.' },
  survival: { label: 'Survival', short: 'Survival', ms: 0, icon: 'shield', desc: 'No clock. Go until three mistakes.' },
};

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
    h('div', { class: 'pz-mode-title' }, m.label),
    h('p', { class: 'muted text-sm' }, m.desc),
    h('div', { class: 'pz-rush-best' }, h('span', { class: 'subtle text-sm' }, 'Best'), h('span', { class: 'tabular bold' }, String(rushBest(key))))));

    page.append(h('div', { class: 'page' },
      pageHeader({
        title: 'Puzzle Rush', icon: 'bolt', subtitle: 'Solve as many puzzles as you can. They get harder as you go.',
        breadcrumbs: [{ label: 'Puzzles', href: '#/puzzles' }, { label: 'Puzzle Rush' }],
      }),
      h('div', { class: 'card pz-rush-hero' },
        h('div', { class: 'pz-strikes lg', 'aria-hidden': 'true' }, [0, 1, 2].map(() => h('span', { class: 'pz-strike', html: icon('x') }))),
        h('div', null,
          h('div', { class: 'semibold' }, 'Three strikes and you’re out'),
          h('p', { class: 'muted text-sm' }, 'One wrong move counts as a strike and skips to the next puzzle. Rush never changes your puzzle rating.')),
        h('div', { class: 'pz-rush-overall-wrap' }, h('div', { class: 'stat-label' }, 'All-time best'), h('div', { class: 'pz-rush-overall stat-value tabular' }, String(serverBest)))),
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
    const turnBar = h('div', { class: 'pz-turnbar' }, h('span', { class: 'spinner' }), h('span', null, 'Get ready…'));
    const clockEl = h('div', { class: 'pz-rush-clock tabular' }, cfg.ms ? formatClock(cfg.ms, { tenths: false }) : '0:00');
    const scoreEl = h('div', { class: 'pz-rush-score tabular' }, '0');
    const strikesEl = h('div', { class: 'pz-strikes' }, [0, 1, 2].map(() => h('span', { class: 'pz-strike', html: icon('x') })));
    const tilesEl = h('div', { class: 'pz-tiles' });
    const quitBtn = btn('End run', 'flag', 'ghost', () => endRun('quit'));

    const panel = h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header', html: icon('bolt') + `<span>Puzzle Rush · ${cfg.short}</span>` }),
      h('div', { class: 'panel-body stack' },
        h('div', { class: 'pz-rush-top' },
          h('div', null, h('div', { class: 'stat-label' }, cfg.ms ? 'Time left' : 'Time'), clockEl),
          h('div', { class: 'text-center' }, h('div', { class: 'stat-label' }, 'Score'), scoreEl)),
        h('div', { class: 'row between' }, h('div', { class: 'stat-label' }, 'Strikes'), strikesEl),
        h('div', { class: 'stat-label' }, 'Results'),
        tilesEl),
      h('div', { class: 'panel-footer' }, quitBtn));

    page.append(h('div', { class: 'game-layout no-eval pz-layout', style: '--board-chrome: 124px' },
      h('div', { class: 'game-main' }, turnBar, h('div', { class: 'board-row' }, slot), h('div', { class: 'toolbar pz-toolbar' },
        h('span', { class: 'subtle text-sm' }, 'Puzzles get harder as your score climbs'))),
      h('aside', { class: 'game-panel' }, panel)));

    const board = createBoard(slot, (mv) => runner.handleMove(mv));
    vb.add(() => board.destroy());
    const runner = new PuzzleRunner(board, {
      onReady() {
        const c = runner.userColor;
        turnBar.className = 'pz-turnbar';
        turnBar.replaceChildren(h('span', { class: `pz-side pz-side-${c}` }), h('span', null, `${sideLabel(c)} to move`));
      },
      onCorrect() {
        turnBar.className = 'pz-turnbar state-correct';
        turnBar.replaceChildren(h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, 'Correct — keep going'));
      },
      onSolved() {
        if (run.ended) return;
        run.score++;
        scoreEl.textContent = String(run.score);
        scoreEl.classList.remove('bump'); void scoreEl.offsetWidth; scoreEl.classList.add('bump');
        addTile(true);
        turnBar.className = 'pz-turnbar state-solved';
        turnBar.replaceChildren(h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, 'Solved!'));
        timers.later(nextPuzzle, 420);
      },
      onWrong() {
        if (run.ended) return;
        run.strikes++;
        const s = strikesEl.children[run.strikes - 1];
        if (s) s.classList.add('on', 'pop-in');
        addTile(false);
        turnBar.className = 'pz-turnbar state-wrong';
        turnBar.replaceChildren(h('span', { html: icon('x-circle') }), h('span', null, run.strikes >= 3 ? 'Strike three!' : `Strike ${run.strikes}`));
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
        title: `${ok ? 'Solved' : 'Missed'} · rating ${p.rating}`,
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
        if (!isAbort(e) && !vb.disposed && !run.queue.length) toast(`Couldn’t load puzzles: ${e.message}`, 'error');
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
        page.replaceChildren(emptyState({ icon: 'puzzle', title: 'No puzzles available', text: 'The puzzle set could not be loaded. Is the server running?', action: { label: 'Back to puzzles', href: '#/puzzles' } }));
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
    const cfg = RUSH_MODES[mode];
    const reasonText = reason === 'time' ? 'Time’s up!' : reason === 'strikes' ? 'Three strikes!' : reason === 'empty' ? 'You solved every puzzle we had!' : 'Run ended';
    const solvedCount = results.filter((r) => r.solved).length;
    const hardest = results.filter((r) => r.solved).reduce((m, r) => Math.max(m, r.puzzle.rating || 0), 0);

    const confetti = isPb ? h('div', { class: 'pz-confetti', 'aria-hidden': 'true' },
      Array.from({ length: 36 }, (_, i) => h('span', {
        style: { '--x': `${(i * 37) % 100}%`, '--d': `${(i % 9) * 70}ms`, '--r': `${(i * 53) % 360}deg`, '--c': `var(--${['primary', 'gold', 'info', 'accent', 'cls-brilliant'][i % 5]})` },
      }))) : null;

    const card = h('div', { class: 'card pz-end' },
      confetti,
      h('div', { class: 'result-hero' },
        h('div', { class: 'subtle semibold' }, `${cfg.label} · ${reasonText}`),
        h('div', { class: 'pz-end-score tabular pop-in' }, String(score)),
        h('div', { class: 'result-hero-title' }, isPb ? 'New personal best! 🎉' : score > 0 ? 'Nice run!' : 'Keep practising!'),
        h('div', { class: 'result-hero-sub muted' }, isPb ? 'You beat your previous best.' : `Your best in ${cfg.short}: ${modeBest}`)),
      h('div', { class: 'grid-3 pz-end-stats' },
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'Solved'), h('div', { class: 'stat-value tabular' }, String(solvedCount))),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'Hardest solved'), h('div', { class: 'stat-value tabular' }, hardest ? String(hardest) : '—')),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'All-time best'), h('div', { class: 'stat-value tabular' }, String(overall)))),
      results.length ? h('div', { class: 'stack-sm' },
        h('div', { class: 'stat-label' }, 'Review your puzzles'),
        h('div', { class: 'pz-tiles pz-tiles-end' }, results.map((r, i) => h('a', {
          class: `pz-tile ${r.solved ? 'ok' : 'bad'}`, href: analysisHref(r.puzzle),
          title: `#${i + 1} · ${r.solved ? 'Solved' : 'Missed'} · rating ${r.puzzle.rating} — open in analysis`,
          html: icon(r.solved ? 'check' : 'x') + `<span>${Number(r.puzzle.rating) || ''}</span>`,
        })))) : null,
      h('div', { class: 'row row-wrap pz-end-actions' },
        btn('Play again', 'refresh', 'primary', () => startRun(mode), { cls: 'btn-lg' }),
        btn('Change mode', 'grid', 'secondary', () => showSelect()),
        h('a', { class: 'btn btn-ghost', href: '#/puzzles', html: icon('puzzle') + '<span>All puzzles</span>' })));
    page.append(h('div', { class: 'pz-end-wrap' }, card));
    if (isPb) vb.timeout(() => { if (confetti) confetti.remove(); }, 4000);
  }

  showSelect();
}
