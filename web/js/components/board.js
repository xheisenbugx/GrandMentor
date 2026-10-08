// GrandMentor — interactive chess board.
// Contract: docs/CONTRACT.md §5 (`Board`). Styles: web/css/board.css.
//
// Rendering model
//   - 64 squares are rendered ONCE in a CSS grid; flipping only relabels them.
//   - Pieces are absolutely positioned <div>s (12.5% × 12.5%) moved with percentage
//     `transform: translate()`; animations are CSS transitions, so resizing never needs JS.
//   - Position changes are diffed old → new so moved pieces slide (castling rook, en passant,
//     promotions included), captured pieces fade out and new pieces fade in.
//   - Arrows / circles live in one SVG overlay (viewBox 0 0 8 8), badges in their own layer.
//
// Accessibility (keyboard + screen readers)
//   - The squares form an ARIA grid (8 rows × 8 gridcells, in visual order) with ONE tab stop and
//     roving focus: arrow keys move a visible cursor, Home/End jump to the row ends, PageUp/PageDown
//     to the column ends. Enter/Space picks up a piece (legal-move dots appear) and drops it on the
//     target; Esc cancels. Works the same in both orientations ("up" is always up on screen).
//   - Every cell is labelled "e4, white knight" / "e4, empty" (+ selected / legal move / check).
//   - Moves are announced in natural language through components/announcer.js when the
//     "Announce moves" setting is on (user moves, programmatic moves and one-move position changes).
//
// Memory: every listener, observer, timer and DOM node created here is released by destroy().

import { Chess, DEFAULT_POSITION } from '../../vendor/chess.js';
import { getSetting, onSettingsChange, reducedMotion } from '../settings.js';
import { classificationMeta } from '../ui.js';
import { t } from '../i18n.js';
import { playSound } from './sound.js';
import { announceMove, announce, squareLabel, coloredPiece, describeMove } from './announcer.js';

const FILES = 'abcdefgh';
const PROMO_PIECES = ['q', 'n', 'r', 'b'];
const ARROW_COLORS = new Set(['green', 'red', 'blue', 'yellow']);
const HIGHLIGHT_KINDS = new Set(['hint', 'good', 'bad', 'selected', 'target']);
const SQUARE_RE = /^[a-h][1-8]$/;
const MAX_SHAPES = 128; // bound on user + programmatic shapes

let cssInjected = false;
/** board.css is linked by index.html; inject it once for pages that don't (e.g. dev demos). */
function ensureCss() {
  if (cssInjected || typeof document === 'undefined') return;
  cssInjected = true;
  const linked = Array.from(document.querySelectorAll('link[rel="stylesheet"]'))
    .some((l) => /\/board\.css(\?|$)/.test(l.getAttribute('href') || ''));
  if (linked) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = new URL('../../css/board.css', import.meta.url).href;
  link.dataset.gmBoardCss = '';
  document.head.appendChild(link);
}

const isSquare = (s) => typeof s === 'string' && SQUARE_RE.test(s);
const fileOf = (sq) => sq.charCodeAt(0) - 97;
const rankOf = (sq) => sq.charCodeAt(1) - 49;

function prefersReducedMotion() {
  try { return reducedMotion(); } catch { return false; }
}

function normalizeFen(fen) {
  if (!fen || fen === 'start') return DEFAULT_POSITION;
  return String(fen).trim();
}

/** Create a chess.js instance; falls back to skipValidation for study positions (e.g. no kings). */
function makeChess(fen) {
  try {
    return new Chess(fen);
  } catch {
    try { return new Chess(fen, { skipValidation: true }); } catch { return null; }
  }
}

/** Map<square, code> e.g. 'e1' -> 'wK'. */
function pieceMap(chess) {
  const map = new Map();
  const rows = chess.board();
  for (const row of rows) {
    for (const cell of row) {
      if (cell) map.set(cell.square, cell.color + cell.type.toUpperCase());
    }
  }
  return map;
}

function safeColor(color) {
  if (ARROW_COLORS.has(color)) return `var(--arrow-${color})`;
  if (typeof color === 'string' && /^(#[0-9a-fA-F]{3,8}|rgba?\([\d\s.,%]+\)|hsla?\([\d\s.,%deg]+\))$/.test(color)) return color;
  return 'var(--arrow-green)';
}

export class Board {
  /**
   * @param {HTMLElement} el container (should be square, e.g. `.board-slot`)
   * @param {object} [opts]
   */
  constructor(el, opts = {}) {
    if (!el) throw new Error('Board: container element required');
    ensureCss();
    this.el = el;
    this.opts = {
      fen: 'start',
      orientation: 'white',
      interactive: true,
      movableColor: 'white',
      showCoords: undefined, // undefined = follow user settings
      showLegal: undefined,
      animationMs: undefined,
      autoQueen: undefined,
      onMove: null,
      onSquareClick: null,
      sounds: true,
      // Premoves: pieces of this colour can be queued to move while the other side is to move.
      premoveColor: null, // 'white' | 'black' | null
      onPremove: null,    // ({from,to,promotion}|null) => void, when the user sets or cancels one
      blindfold: false,   // hide the pieces (squares, coordinates and moves still work)
      keyboard: true,     // arrow keys / Enter / Esc on the focused board
      announce: true,     // announce moves to screen readers (also needs the announceMoves setting)
      label: null,        // accessible name of the board (default: "Chess board")
      ...opts,
    };
    this._destroyed = false;
    this._orientation = this.opts.orientation === 'black' ? 'black' : 'white';
    this._interactive = !!this.opts.interactive;
    this._movable = this.opts.movableColor ?? null;
    this._chess = makeChess(DEFAULT_POSITION);
    /** @type {Map<string,{el:HTMLElement, code:string}>} */
    this._pieces = new Map();
    this._timers = new Set();
    this._lastMove = null;
    this._selected = null;
    this._dests = new Map(); // to-square -> [verbose moves]
    this._drag = null;
    this._rdrag = null;
    this._promo = null;
    this._hoverSq = null;
    this._userArrows = [];  // {from,to,color}
    this._userCircles = []; // {square,color}
    this._arrows = [];
    this._circles = [];
    this._highlights = new Map(); // square -> kind
    this._badges = new Map();     // square -> classification
    this._rect = null;
    this._cursorSq = null;
    this._premoveColor = this.opts.premoveColor === 'white' || this.opts.premoveColor === 'black' ? this.opts.premoveColor : null;
    this._premove = null;     // queued premove {from,to,promotion?}
    this._selPremove = false; // current selection is a premove selection
    this._blindfold = !!this.opts.blindfold;
    this._peek = false;
    this._focusSq = null;     // keyboard cursor square
    this._kbd = false;        // keyboard mode (cursor ring visible while focused)
    this._labelCache = [];    // last aria-label per visual cell (avoid needless DOM writes)

    this._build();
    this._bind();
    this._applyConfig();
    this.setPosition(this.opts.fen, { animate: false });
  }

  // ------------------------------------------------------------------ DOM

  _build() {
    const root = document.createElement('div');
    root.className = 'gm-board';
    root.setAttribute('role', 'grid');
    root.setAttribute('aria-roledescription', t('ui.board.roleDescription'));
    root.setAttribute('aria-label', this.opts.label || t('a11y.board.label'));
    root.tabIndex = -1;

    const squares = document.createElement('div');
    squares.className = 'gm-squares';
    squares.setAttribute('role', 'presentation');
    this._sqEls = [];          // visual index (row*8+col) -> element
    this._coordRank = [];      // visual row -> span in col 0
    this._coordFile = [];      // visual col -> span in row 7
    const frag = document.createDocumentFragment();
    for (let row = 0; row < 8; row++) {
      // Row wrappers exist only for the accessibility tree (display: contents keeps the CSS grid).
      const rowEl = document.createElement('div');
      rowEl.className = 'gm-row';
      rowEl.setAttribute('role', 'row');
      frag.appendChild(rowEl);
      for (let col = 0; col < 8; col++) {
        const s = document.createElement('div');
        // Square color parity is invariant under a 180° flip, so it's fixed per visual cell.
        s.className = 'gm-sq ' + ((row + col) % 2 === 0 ? 'light' : 'dark');
        s.setAttribute('role', 'gridcell');
        s.tabIndex = -1;
        if (col === 0) {
          const c = document.createElement('span');
          c.className = 'gm-coord gm-coord-rank';
          c.setAttribute('aria-hidden', 'true');
          s.appendChild(c);
          this._coordRank[row] = c;
        }
        if (row === 7) {
          const c = document.createElement('span');
          c.className = 'gm-coord gm-coord-file';
          c.setAttribute('aria-hidden', 'true');
          s.appendChild(c);
          this._coordFile[col] = c;
        }
        this._sqEls.push(s);
        rowEl.appendChild(s);
      }
    }
    squares.appendChild(frag);

    const pieces = document.createElement('div');
    pieces.className = 'gm-pieces';
    pieces.setAttribute('aria-hidden', 'true');

    // Keyboard cursor ring + square-name tags (hover / focus), above the pieces.
    const kbd = document.createElement('div');
    kbd.className = 'gm-kbd-cursor';
    kbd.setAttribute('aria-hidden', 'true');
    const kbdName = document.createElement('span');
    kbdName.className = 'gm-sq-name';
    kbd.appendChild(kbdName);
    const hoverTag = document.createElement('div');
    hoverTag.className = 'gm-hover-name';
    hoverTag.setAttribute('aria-hidden', 'true');
    const hoverName = document.createElement('span');
    hoverName.className = 'gm-sq-name';
    hoverTag.appendChild(hoverName);

    const svgNS = 'http://www.w3.org/2000/svg';
    const svg = document.createElementNS(svgNS, 'svg');
    svg.setAttribute('class', 'gm-shapes');
    svg.setAttribute('viewBox', '0 0 8 8');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('preserveAspectRatio', 'none');

    const badges = document.createElement('div');
    badges.className = 'gm-badges';
    badges.setAttribute('aria-hidden', 'true');

    root.append(squares, pieces, svg, badges, kbd, hoverTag);
    this.el.appendChild(root);
    this.root = root;
    this._kbdEl = kbd;
    this._kbdName = kbdName;
    this._hoverTag = hoverTag;
    this._hoverName = hoverName;
    this._squaresEl = squares;
    this._piecesEl = pieces;
    this._svg = svg;
    this._badgesEl = badges;
  }

  _bind() {
    this._onPointerDown = this._onPointerDown.bind(this);
    this._onPointerMove = this._onPointerMove.bind(this);
    this._onPointerUp = this._onPointerUp.bind(this);
    this._onPointerCancel = this._onPointerCancel.bind(this);
    this._onContextMenu = (e) => e.preventDefault();
    this._onKeyDown = this._onKeyDown.bind(this);
    this._onBoardKey = this._onBoardKey.bind(this);
    this._onFocusIn = this._onFocusIn.bind(this);
    this._onFocusOut = this._onFocusOut.bind(this);
    this._onPointerLeave = () => this._showHoverName(null);
    const r = this.root;
    r.addEventListener('keydown', this._onBoardKey);
    r.addEventListener('focusin', this._onFocusIn);
    r.addEventListener('focusout', this._onFocusOut);
    r.addEventListener('pointerleave', this._onPointerLeave);
    r.addEventListener('pointerdown', this._onPointerDown);
    r.addEventListener('pointermove', this._onPointerMove);
    r.addEventListener('pointerup', this._onPointerUp);
    r.addEventListener('pointercancel', this._onPointerCancel);
    r.addEventListener('lostpointercapture', this._onPointerCancel);
    r.addEventListener('contextmenu', this._onContextMenu);

    if (typeof ResizeObserver !== 'undefined') {
      this._ro = new ResizeObserver(() => { this._rect = null; this._onResize(); });
      this._ro.observe(r);
    }
    this._offSettings = onSettingsChange((_s, key) => {
      if (['showCoords', 'showLegal', 'animationMs', 'autoQueen', 'motion'].includes(key)) this._applyConfig();
    });
  }

  _onResize() {
    // Layout is percentage based; only a live drag needs re-measuring.
    if (this._drag && this._drag.lastEvent) this._positionDrag(this._drag.lastEvent);
  }

  // ------------------------------------------------------------- settings

  _cfg(key) {
    const v = this.opts[key];
    if (v !== undefined && v !== null) return v;
    try { return getSetting(key); } catch { return undefined; }
  }

  _animMs() {
    if (prefersReducedMotion()) return 0;
    const v = Number(this._cfg('animationMs'));
    return Number.isFinite(v) ? Math.max(0, Math.min(1000, v)) : 200;
  }

  _applyConfig() {
    if (this._destroyed) return;
    const r = this.root;
    r.dataset.orientation = this._orientation;
    r.dataset.coords = this._cfg('showCoords') === false ? 'off' : 'on';
    r.style.setProperty('--gm-anim', `${this._animMs()}ms`);
    r.classList.toggle('interactive', this._interactive || !!this._premoveColor);
    r.classList.toggle('blindfold', this._blindfold);
    r.classList.toggle('peek', this._blindfold && this._peek);
    this._labelSquares();
    if (this._selected) this._showDests();
  }

  // ------------------------------------------------------------- geometry

  /** Visual [col,row] of a square for the current orientation. */
  _vis(sq) {
    const f = fileOf(sq);
    const r = rankOf(sq);
    return this._orientation === 'white' ? [f, 7 - r] : [7 - f, r];
  }

  _sqFromVis(col, row) {
    if (col < 0 || col > 7 || row < 0 || row > 7) return null;
    return this._orientation === 'white'
      ? FILES[col] + (8 - row)
      : FILES[7 - col] + (row + 1);
  }

  _sqEl(sq) {
    const [c, r] = this._vis(sq);
    return this._sqEls[r * 8 + c];
  }

  _getRect() {
    if (!this._rect) this._rect = this.root.getBoundingClientRect();
    return this._rect;
  }

  _squareAt(e) {
    const rect = this._getRect();
    if (!rect.width) return null;
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    if (x < 0 || y < 0 || x >= rect.width || y >= rect.height) return null;
    return this._sqFromVis(Math.floor((x / rect.width) * 8), Math.floor((y / rect.height) * 8));
  }

  _labelSquares() {
    for (let row = 0; row < 8; row++) {
      for (let col = 0; col < 8; col++) {
        this._sqEls[row * 8 + col].dataset.square = this._sqFromVis(col, row);
      }
    }
    for (let row = 0; row < 8; row++) {
      this._coordRank[row].textContent = this._orientation === 'white' ? String(8 - row) : String(row + 1);
    }
    for (let col = 0; col < 8; col++) {
      this._coordFile[col].textContent = this._orientation === 'white' ? FILES[col] : FILES[7 - col];
    }
    this._a11yLabels();
  }

  _placeEl(el, sq) {
    const [c, r] = this._vis(sq);
    el.style.transform = `translate(${c * 100}%, ${r * 100}%)`;
  }

  // ------------------------------------------------------------- position

  /**
   * Set the board position.
   * @param {string} fen full FEN or 'start'
   * @param {{animate?: boolean, lastMove?: [string,string]|null, sound?: boolean}} [o]
   */
  setPosition(fen, { animate = true, lastMove = null, sound = true } = {}) {
    if (this._destroyed) return false;
    const norm = normalizeFen(fen);
    const next = makeChess(norm);
    if (!next) {
      console.warn('[Board] invalid FEN ignored:', fen);
      return false;
    }
    const keep = this._takeInteraction();
    this._cancelInteraction();
    const prevFen = this._chess ? this._chess.fen() : '';
    const prevChess = this._chess;
    const prevMap = this._pieces.size ? this._currentMap() : null;
    const changed = prevFen.split(' ').slice(0, 4).join(' ') !== next.fen().split(' ').slice(0, 4).join(' ');
    // A one-move change with a known last move (opponent replies, stepping through a game) is announced.
    let spoken = null;
    if (changed && prevMap && Array.isArray(lastMove) && prevChess) spoken = this._findMove(prevChess, lastMove, next.fen());
    this._chess = next;
    this._lastMove = Array.isArray(lastMove) && isSquare(lastMove[0]) && isSquare(lastMove[1])
      ? [lastMove[0], lastMove[1]] : null;
    if (changed && (this._userArrows.length || this._userCircles.length)) {
      // Drawings belong to the position they were drawn on.
      this._userArrows = [];
      this._userCircles = [];
      this._renderShapes();
    }
    this._render({ animate });
    this._restoreInteraction(keep);
    if (sound && changed && prevMap && this._lastMove && this.opts.sounds) {
      this._landSound(this._soundForTransition(prevMap, this._lastMove), animate);
    }
    if (spoken) this._announceMove(spoken);
    return true;
  }

  /** The legal move from→to in `chess` that leads to `fenAfter` (verbose chess.js move) or null. */
  _findMove(chess, [from, to], fenAfter) {
    if (!isSquare(from) || !isSquare(to)) return null;
    try {
      const key = (f) => f.split(' ').slice(0, 4).join(' ');
      const want = key(fenAfter);
      for (const m of chess.moves({ square: from, verbose: true })) {
        if (m.to === to && key(m.after) === want) return m;
      }
    } catch { /* study positions without kings etc. */ }
    return null;
  }

  /**
   * Detach a premove selection / drag in progress so a position update (the opponent's move
   * landing) doesn't snatch the piece out of the user's hand. Pair with _restoreInteraction.
   */
  _takeInteraction() {
    if (!this._premoveColor || !this._selected || this._promo) return null;
    const from = this._selected;
    const code = this._pieces.get(from)?.code;
    if (!code) return null;
    const drag = this._drag && this._drag.from === from ? this._drag : null;
    if (drag) this._drag = null; // keep it alive through _cancelInteraction
    return { from, code, drag };
  }

  _restoreInteraction(keep) {
    if (!keep || this._destroyed) return;
    const { from, code, drag } = keep;
    const p = this._pieces.get(from);
    const ok = p && p.code === code && p.el === (drag ? drag.el : p.el) && (this._canMove(from) || this._canPremove(from));
    if (ok) {
      this._select(from);
      if (drag) this._drag = drag;
      return;
    }
    if (drag) {
      // The piece is gone or can't move any more: drop the drag quietly.
      this._drag = drag;
      this._endDrag(true);
    }
  }

  getFen() {
    return this._chess ? this._chess.fen() : DEFAULT_POSITION;
  }

  /** The chess.js instance mirroring the board (read-only use, please). */
  get chess() { return this._chess; }

  get orientation() { return this._orientation; }

  setOrientation(color) {
    const c = color === 'black' ? 'black' : 'white';
    if (c === this._orientation || this._destroyed) return;
    this._cancelInteraction();
    this._orientation = c;
    this._applyConfig();
    // Reposition instantly (a flip animation of 32 pieces is disorienting).
    this._piecesEl.classList.add('gm-instant');
    for (const [sq, p] of this._pieces) this._placeEl(p.el, sq);
    void this._piecesEl.offsetWidth; // single reflow so the transition is skipped
    this._piecesEl.classList.remove('gm-instant');
    this._renderMarks();
    this._renderShapes();
    this._renderBadges();
    if (this._focusSq && this.root.contains(document.activeElement)) this._setFocusSquare(this._focusSq);
  }

  flip() { this.setOrientation(this._orientation === 'white' ? 'black' : 'white'); }

  setInteractive(interactive, movableColor) {
    if (this._destroyed) return;
    this._interactive = !!interactive;
    if (movableColor !== undefined) this._movable = movableColor;
    const keep = this._takeInteraction();
    this._cancelInteraction();
    this._applyConfig();
    this._restoreInteraction(keep);
  }

  /** Programmatically play a move (UCI or {from,to,promotion}). Returns the move object or null. */
  move(m, { animate = true, sound = true } = {}) {
    if (this._destroyed || !this._chess) return null;
    let spec = m;
    if (typeof m === 'string') {
      if (!/^[a-h][1-8][a-h][1-8][qrbn]?$/.test(m)) return null;
      spec = { from: m.slice(0, 2), to: m.slice(2, 4), promotion: m[4] };
    }
    this._cancelInteraction();
    let mv;
    try { mv = this._chess.move(spec); } catch { return null; }
    if (!mv) return null;
    this._lastMove = [mv.from, mv.to];
    this._render({ animate });
    if (sound && this.opts.sounds) this._landSound(this._soundForMove(mv), animate);
    this._announceMove(mv);
    return this._moveObject(mv);
  }

  /**
   * True when the user may move now: the board is interactive and the side to move is movable
   * (optionally: the piece on `square` may move).
   */
  canUserMove(square) {
    if (this._destroyed || !this._chess) return false;
    if (square) return this._canMove(square);
    return this.legalMoves().some((m) => this._canMove(m.from));
  }

  /**
   * Play a move exactly as if the user had made it on the board (typed moves, assistive input):
   * only when canUserMove(from), then onMove is called and may reject it (false / Promise<false>).
   * `m` is UCI ("e7e8q") or {from,to,promotion}; a missing promotion piece defaults to a queen.
   * Returns the move object, or null when it is not allowed, illegal or was rejected right away.
   */
  playUserMove(m) {
    if (this._destroyed || !this._chess || this._promo) return null;
    let spec = m;
    if (typeof m === 'string') {
      if (!/^[a-h][1-8][a-h][1-8][qrbn]?$/.test(m)) return null;
      spec = { from: m.slice(0, 2), to: m.slice(2, 4), promotion: m[4] };
    }
    if (!spec || !isSquare(spec.from) || !isSquare(spec.to) || !this._canMove(spec.from)) return null;
    let legal = null;
    try {
      legal = this._chess.moves({ square: spec.from, verbose: true })
        .filter((x) => x.to === spec.to)
        .find((x) => !x.promotion || x.promotion === (spec.promotion || 'q')) || null;
    } catch { legal = null; }
    if (!legal) return null;
    this._cancelInteraction();
    const before = this._chess.fen();
    this._commit(legal.from, legal.to, legal.promotion || undefined, false);
    if (this._destroyed || !this._chess || this._chess.fen() === before) return null;
    return this._moveObject(legal);
  }

  /** Legal moves in the current position (verbose chess.js objects). */
  legalMoves() {
    try { return this._chess.moves({ verbose: true }); } catch { return []; }
  }

  // ------------------------------------------------------------- premoves

  /**
   * Allow queuing one premove for `color` ('white'|'black') while the other side is to move,
   * or turn premoves off (null; also clears a queued premove).
   */
  setPremoveColor(color) {
    if (this._destroyed) return;
    const c = color === 'white' || color === 'black' ? color : null;
    if (c === this._premoveColor) return;
    this._premoveColor = c;
    if (!c) this.clearPremove();
    if (this._selPremove) { this._selected = null; this._dests = new Map(); this._selPremove = false; }
    this._applyConfig();
    this._renderMarks();
  }

  /** The queued premove `{from,to,promotion?}` or null. */
  getPremove() { return this._premove ? { ...this._premove } : null; }

  /** Queue (or replace) a premove programmatically; null clears it. Does not call onPremove. */
  setPremove(pm) {
    if (this._destroyed) return;
    this._premove = pm && isSquare(pm.from) && isSquare(pm.to) && pm.from !== pm.to
      ? { from: pm.from, to: pm.to, promotion: /^[qrbn]$/.test(pm.promotion || '') ? pm.promotion : undefined }
      : null;
    this._renderMarks();
  }

  /** Cancel the queued premove. `notify` calls onPremove(null) (user-initiated cancels). */
  clearPremove(notify = false) {
    if (!this._premove) return;
    this._premove = null;
    if (this._promo && this._promo.premove) this._closePromo(true);
    if (!this._destroyed) this._renderMarks();
    if (notify) this._emitPremove();
  }

  /**
   * Try to play the queued premove in the current position. If it is legal it is played exactly
   * like a user move (onMove is called) and the move object is returned; otherwise the premove
   * is dropped and null is returned.
   */
  playPremove() {
    const pm = this._premove;
    if (!pm || this._destroyed || !this._chess) return null;
    this._premove = null;
    this._cancelInteraction();
    let legal = null;
    try {
      legal = this._chess.moves({ square: pm.from, verbose: true })
        .filter((m) => m.to === pm.to)
        .find((m) => !m.promotion || m.promotion === (pm.promotion || 'q')) || null;
    } catch { legal = null; }
    if (!legal) { this._renderMarks(); return null; }
    const before = this._chess.fen();
    this._commit(legal.from, legal.to, legal.promotion || undefined, false);
    if (this._destroyed || !this._chess || this._chess.fen() === before) return null;
    return this._moveObject(legal);
  }

  _emitPremove() {
    if (typeof this.opts.onPremove !== 'function') return;
    try { this.opts.onPremove(this.getPremove()); } catch (err) { console.error('[Board] onPremove error', err); }
  }

  /** Squares a piece could reach "in principle" (rays ignore blockers), like lichess/chess.com. */
  _premoveDests(sq) {
    const p = this._pieces.get(sq);
    if (!p) return [];
    const color = p.code[0];
    const role = p.code[1];
    const f = fileOf(sq);
    const r = rankOf(sq);
    const out = [];
    const add = (df, dr) => {
      const nf = f + df; const nr = r + dr;
      if (nf < 0 || nf > 7 || nr < 0 || nr > 7) return false;
      out.push(FILES[nf] + (nr + 1));
      return true;
    };
    const ray = (df, dr) => { for (let i = 1; i < 8; i++) if (!add(df * i, dr * i)) break; };
    if (role === 'P') {
      const dir = color === 'w' ? 1 : -1;
      add(0, dir);
      if ((color === 'w' && r === 1) || (color === 'b' && r === 6)) add(0, 2 * dir);
      add(-1, dir); add(1, dir);
    } else if (role === 'N') {
      for (const [df, dr] of [[1, 2], [2, 1], [2, -1], [1, -2], [-1, -2], [-2, -1], [-2, 1], [-1, 2]]) add(df, dr);
    } else if (role === 'K') {
      for (let df = -1; df <= 1; df++) for (let dr = -1; dr <= 1; dr++) if (df || dr) add(df, dr);
      const home = color === 'w' ? 'e1' : 'e8';
      if (sq === home) {
        const rank = color === 'w' ? '1' : '8';
        let rights = '';
        try { rights = this._chess.fen().split(' ')[2] || ''; } catch { rights = ''; }
        const [kc, qc] = color === 'w' ? ['K', 'Q'] : ['k', 'q'];
        if (rights.includes(kc)) out.push('g' + rank);
        if (rights.includes(qc)) out.push('c' + rank);
      }
    } else {
      const dirs = [];
      if (role === 'B' || role === 'Q') dirs.push([1, 1], [1, -1], [-1, 1], [-1, -1]);
      if (role === 'R' || role === 'Q') dirs.push([1, 0], [-1, 0], [0, 1], [0, -1]);
      for (const [df, dr] of dirs) ray(df, dr);
    }
    // Never onto one of our own pieces (clicking those re-selects instead).
    return [...new Set(out)].filter((to) => this._pieces.get(to)?.code[0] !== color);
  }

  _canPremove(sq) {
    if (!this._premoveColor || !this._chess) return false;
    const p = this._pieces.get(sq);
    if (!p) return false;
    const c = this._premoveColor === 'white' ? 'w' : 'b';
    let turn;
    try { turn = this._chess.turn(); } catch { return false; }
    return p.code[0] === c && turn !== c;
  }

  _queuePremove(from, to, dragged) {
    const p = this._pieces.get(from);
    const color = p ? p.code[0] : null;
    const lastRank = color === 'w' ? '8' : '1';
    const isPromo = p && p.code[1] === 'P' && to[1] === lastRank;
    this._selected = null;
    this._dests = new Map();
    this._selPremove = false;
    if (p) { if (dragged) this._placeEl(p.el, from); }
    if (isPromo && !this._cfg('autoQueen')) {
      this._renderMarks();
      this._openPromo(from, to, false, true);
      return;
    }
    this._premove = { from, to, promotion: isPromo ? 'q' : undefined };
    this._renderMarks();
    this._emitPremove();
  }

  // ------------------------------------------------------------- blindfold

  /** Hide (true) or show the pieces. Moves keep working; coordinates stay visible. */
  setBlindfold(on) {
    if (this._destroyed) return;
    this._blindfold = !!on;
    if (!this._blindfold) this._peek = false;
    this._applyConfig();
  }

  /** While blindfolded, temporarily reveal the pieces (e.g. while a "peek" button is held). */
  setPeek(on) {
    if (this._destroyed) return;
    this._peek = !!on && this._blindfold;
    this._applyConfig();
  }

  get blindfold() { return this._blindfold; }

  _currentMap() {
    const m = new Map();
    for (const [sq, p] of this._pieces) m.set(sq, p.code);
    return m;
  }

  _newPieceEl(code, sq) {
    const el = document.createElement('div');
    el.className = `gm-piece ${code}`;
    el.dataset.piece = code;
    this._placeEl(el, sq);
    return el;
  }

  /** Play a move sound at the moment the piece touches the board (like a real piece). */
  _landSound(name, animated) {
    const ms = animated ? this._animMs() : 0;
    if (ms > 40) this._later(() => { if (!this._destroyed) playSound(name); }, Math.round(ms * 0.85));
    else playSound(name);
  }

  _later(fn, ms) {
    const id = setTimeout(() => { this._timers.delete(id); fn(); }, ms);
    this._timers.add(id);
    return id;
  }

  /**
   * Diff the DOM pieces against the chess.js position and animate the differences.
   * @param {{animate?: boolean, instant?: string[]}} o instant = squares whose arriving piece must not slide
   */
  _render({ animate = true, instant = [] } = {}) {
    const ms = animate ? this._animMs() : 0;
    const target = pieceMap(this._chess);
    const old = this._pieces;
    const next = new Map();
    const vanished = []; // {sq, el, code}
    const appeared = []; // {sq, code}

    for (const [sq, p] of old) {
      if (target.get(sq) === p.code) next.set(sq, p);
      else vanished.push({ sq, ...p });
    }
    for (const [sq, code] of target) {
      if (!next.has(sq)) appeared.push({ sq, code });
    }

    const instantSet = new Set(instant);
    const instantEls = [];
    for (const a of appeared) {
      // Find the closest vanished piece of the same kind to slide in.
      let best = -1;
      let bestD = Infinity;
      for (let i = 0; i < vanished.length; i++) {
        const v = vanished[i];
        if (!v || v.code !== a.code) continue;
        const d = Math.abs(fileOf(v.sq) - fileOf(a.sq)) + Math.abs(rankOf(v.sq) - rankOf(a.sq));
        if (d < bestD) { bestD = d; best = i; }
      }
      if (best >= 0) {
        const v = vanished[best];
        vanished[best] = null;
        v.el.classList.remove('gm-fade-in', 'gm-fading', 'gm-dragging', 'gm-hidden');
        if (!ms || instantSet.has(a.sq)) instantEls.push(v.el);
        else v.el.classList.add('gm-moving');
        this._placeEl(v.el, a.sq);
        next.set(a.sq, { el: v.el, code: a.code });
        if (ms && !instantSet.has(a.sq)) {
          const el = v.el;
          this._later(() => el.classList.remove('gm-moving'), ms + 30);
        }
      } else {
        const el = this._newPieceEl(a.code, a.sq);
        if (ms && !instantSet.has(a.sq)) {
          el.classList.add('gm-fade-in');
          this._later(() => el.classList.remove('gm-fade-in'), ms + 30);
        }
        this._piecesEl.appendChild(el);
        next.set(a.sq, { el, code: a.code });
      }
    }

    if (instantEls.length) {
      for (const el of instantEls) el.style.transition = 'none';
      void this._piecesEl.offsetWidth;
      for (const el of instantEls) el.style.transition = '';
    }

    for (const v of vanished) {
      if (!v) continue;
      const captured = target.has(v.sq); // something lands on this square
      if (!ms || instantSet.has(v.sq)) {
        v.el.remove();
      } else if (captured) {
        // Stay put under the attacker, then vanish the moment it lands.
        const el = v.el;
        el.style.zIndex = '1';
        this._later(() => el.remove(), Math.round(ms * 0.85));
      } else {
        v.el.classList.add('gm-fading');
        const el = v.el;
        this._later(() => el.remove(), ms + 30);
      }
    }

    this._pieces = next;
    this._renderMarks();
  }

  // ----------------------------------------------------- square markings

  _renderMarks() {
    if (this._destroyed) return;
    for (const s of this._sqEls) {
      s.classList.remove('last', 'selected', 'check', 'dest', 'capture', 'hover', 'premove', 'premove-dest',
        'hl-hint', 'hl-good', 'hl-bad', 'hl-selected', 'hl-target');
    }
    if (this._lastMove) {
      for (const sq of this._lastMove) this._sqEl(sq).classList.add('last');
    }
    if (this._premove) {
      this._sqEl(this._premove.from).classList.add('premove');
      this._sqEl(this._premove.to).classList.add('premove');
    }
    for (const [sq, kind] of this._highlights) this._sqEl(sq).classList.add(`hl-${kind}`);
    const kingSq = this._checkedKing();
    if (kingSq) this._sqEl(kingSq).classList.add('check');
    if (this._selected) {
      this._sqEl(this._selected).classList.add('selected');
      if (this._cfg('showLegal') !== false) {
        for (const [to, moves] of this._dests) {
          const isCap = this._pieces.has(to) || moves.some((m) => m.flags.includes('e'));
          const el = this._sqEl(to);
          el.classList.add(isCap ? 'capture' : 'dest');
          if (this._selPremove) el.classList.add('premove-dest');
        }
      }
    }
    if (this._hoverSq) this._sqEl(this._hoverSq).classList.add('hover');
    this._a11yLabels();
  }

  // ---------------------------------------------------------- accessibility

  /** Refresh every cell's accessible name and the roving tab stop. Cheap: 64 cached attributes. */
  _a11yLabels() {
    if (this._destroyed || !this._sqEls.length) return;
    const hidePieces = this._blindfold && !this._peek;
    const check = this._checkedKing();
    const showLegal = this._cfg('showLegal') !== false;
    const tabSq = this._focusSq || this._defaultFocusSquare();
    const keyboard = this.opts.keyboard !== false;
    for (let i = 0; i < 64; i++) {
      const el = this._sqEls[i];
      const sq = el.dataset.square;
      if (!sq) continue;
      const code = this._pieces.get(sq)?.code;
      let label = hidePieces ? sq : squareLabel(sq, code);
      const extra = [];
      if (this._selected === sq) extra.push(t('a11y.square.selected'));
      else if (this._selected && this._dests.has(sq) && showLegal) extra.push(t(code ? 'a11y.square.canCapture' : 'a11y.square.canMove'));
      if (check === sq) extra.push(t('a11y.square.inCheck'));
      if (this._lastMove && (this._lastMove[0] === sq || this._lastMove[1] === sq)) extra.push(t('a11y.square.lastMove'));
      if (this._premove && (this._premove.from === sq || this._premove.to === sq)) extra.push(t('a11y.square.premove'));
      if (extra.length) label = [label, ...extra].join(', ');
      if (this._labelCache[i] !== label) {
        el.setAttribute('aria-label', label);
        this._labelCache[i] = label;
      }
      const sel = this._selected === sq ? 'true' : 'false';
      if (el.getAttribute('aria-selected') !== sel) el.setAttribute('aria-selected', sel);
      const ti = keyboard && sq === tabSq ? 0 : -1;
      if (el.tabIndex !== ti) el.tabIndex = ti;
    }
    this._placeCursor();
  }

  /** Where the keyboard cursor starts: the selection, the last move's target, else near the user's king side. */
  _defaultFocusSquare() {
    if (this._selected) return this._selected;
    if (this._lastMove) return this._lastMove[1];
    return this._orientation === 'white' ? 'e2' : 'e7';
  }

  _placeCursor() {
    const sq = this._focusSq || this._defaultFocusSquare();
    if (!this._kbdEl || !sq) return;
    this._placeEl(this._kbdEl, sq);
    if (this._kbdName.textContent !== sq) this._kbdName.textContent = sq;
  }

  _showHoverName(sq) {
    if (!this._hoverTag) return;
    if (!sq) { this._hoverTag.classList.remove('on'); return; }
    this._placeEl(this._hoverTag, sq);
    if (this._hoverName.textContent !== sq) this._hoverName.textContent = sq;
    this._hoverTag.classList.add('on');
  }

  /** Move the keyboard cursor (and DOM focus, when the board has it) to a square. */
  _setFocusSquare(sq, { focus = true } = {}) {
    if (!isSquare(sq)) return;
    this._focusSq = sq;
    this._a11yLabels();
    if (focus) {
      const el = this._sqEl(sq);
      if (el && document.activeElement !== el) { try { el.focus({ preventScroll: true }); } catch { /* ignore */ } }
    }
  }

  /** Give the board keyboard focus (focuses the cursor square). */
  focus() {
    if (this._destroyed) return;
    this._setFocusSquare(this._focusSq || this._defaultFocusSquare());
  }

  _onFocusIn(e) {
    const cell = e.target && e.target.closest ? e.target.closest('.gm-sq') : null;
    if (cell && cell.dataset.square && cell.dataset.square !== this._focusSq) {
      this._focusSq = cell.dataset.square;
      this._a11yLabels();
    }
    // Keyboard users arrive with :focus-visible; mouse clicks don't show the ring.
    let visible = false;
    try { visible = !!(cell && cell.matches(':focus-visible')); } catch { visible = false; }
    if (visible) this._kbd = true;
    this.root.classList.toggle('kbd', this._kbd);
    this.root.classList.add('has-focus');
  }

  _onFocusOut(e) {
    if (e.relatedTarget && this.root.contains(e.relatedTarget)) return;
    this.root.classList.remove('has-focus');
  }

  _onBoardKey(e) {
    if (this._destroyed || this.opts.keyboard === false) return;
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (this._promo) {
      // Promotion picker: arrows cycle the choices; Enter/Space click natively; Esc handled globally.
      if (/^Arrow/.test(e.key)) {
        const btns = Array.from(this._promo.overlay.querySelectorAll('button'));
        const i = btns.indexOf(document.activeElement);
        const step = e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? -1 : 1;
        const next = btns[(i + step + btns.length) % btns.length];
        if (next) { e.preventDefault(); next.focus(); }
      }
      return;
    }
    const cell = e.target && e.target.closest ? e.target.closest('.gm-sq') : null;
    if (!cell) return;
    const sq = cell.dataset.square || this._focusSq;
    if (!sq) return;
    const [col, row] = this._vis(sq);
    let target = null;
    switch (e.key) {
      case 'ArrowUp': target = this._sqFromVis(col, Math.max(0, row - 1)); break;
      case 'ArrowDown': target = this._sqFromVis(col, Math.min(7, row + 1)); break;
      case 'ArrowLeft': target = this._sqFromVis(Math.max(0, col - 1), row); break;
      case 'ArrowRight': target = this._sqFromVis(Math.min(7, col + 1), row); break;
      case 'Home': target = this._sqFromVis(0, row); break;
      case 'End': target = this._sqFromVis(7, row); break;
      case 'PageUp': target = this._sqFromVis(col, 0); break;
      case 'PageDown': target = this._sqFromVis(col, 7); break;
      case 'Enter':
      case ' ':
      case 'Spacebar':
        e.preventDefault();
        e.stopPropagation();
        this._kbd = true;
        this.root.classList.add('kbd');
        this._activateSquare(sq);
        return;
      case 'Escape':
        if (this._selected || this._premove) {
          e.preventDefault();
          e.stopPropagation();
          const had = this._selected;
          this._deselect();
          if (!had && this._premove) { this.clearPremove(true); announce(t('a11y.board.premoveCancelled')); }
          else announce(t('a11y.board.cancelled'));
        }
        return;
      default:
        return;
    }
    // Handled here: keep page-level shortcuts (← → move navigation) from also reacting.
    e.preventDefault();
    e.stopPropagation();
    this._kbd = true;
    this.root.classList.add('kbd');
    if (target) this._setFocusSquare(target);
  }

  /** Enter/Space on a square: like a click (pick up, drop, or re-select). */
  _activateSquare(sq) {
    if (typeof this.opts.onSquareClick === 'function') {
      try { this.opts.onSquareClick(sq); } catch (err) { console.error(err); }
      if (this._destroyed) return;
    }
    if (this._selected && this._selected !== sq && this._dests.has(sq)) {
      this._tryMove(this._selected, sq, { dragged: false });
      if (!this._destroyed && this._promo) return; // focus is in the promotion picker
      if (!this._destroyed) this._setFocusSquare(sq);
      return;
    }
    if (this._selected === sq) {
      this._deselect();
      announce(t('a11y.board.cancelled'));
      return;
    }
    if (this._canMove(sq) || this._canPremove(sq)) {
      this._select(sq);
      const code = this._pieces.get(sq)?.code;
      const n = this._dests.size;
      announce(n
        ? t('a11y.board.pickedUp', { piece: coloredPiece(code), square: sq, count: n })
        : t('a11y.board.noMoves', { piece: coloredPiece(code), square: sq }));
      return;
    }
    if (this._selected) {
      this._deselect();
      announce(t('a11y.board.cannotMoveThere', { square: sq }));
      return;
    }
    if (this._premove) { this.clearPremove(true); announce(t('a11y.board.premoveCancelled')); return; }
    const code = this._pieces.get(sq)?.code;
    if (!this._interactive && !this._premoveColor) announce(t('a11y.board.viewOnly'));
    else if (code) announce(t('a11y.board.notYourPiece', { piece: coloredPiece(code), square: sq }));
  }

  /** Announce a move (user or programmatic) when enabled. */
  _announceMove(mv, { mine = false } = {}) {
    if (this.opts.announce === false || !mv) return;
    announceMove(mv);
    // After the opponent's move, tell the user it's their turn.
    if (!mine && this._interactive && this._movable && this._movable !== 'both') {
      let turn = null;
      try { turn = this._chess.turn(); } catch { turn = null; }
      let over = false;
      try { over = this._chess.isGameOver(); } catch { over = false; }
      if (!over && turn && (this._movable === 'white' ? 'w' : 'b') === turn) {
        let on = true;
        try { on = getSetting('announceMoves') !== false; } catch { on = true; }
        if (on) announce(t('a11y.turn.you'));
      }
    }
  }

  _checkedKing() {
    try {
      if (!this._chess.inCheck()) return null;
    } catch { return null; }
    const code = this._chess.turn() + 'K';
    for (const [sq, p] of this._pieces) if (p.code === code) return sq;
    return null;
  }

  // ------------------------------------------------------------ selection

  _canMove(sq) {
    if (!this._interactive || !this._movable || !this._chess) return false;
    const p = this._pieces.get(sq);
    if (!p) return false;
    const color = p.code[0];
    let turn;
    try { turn = this._chess.turn(); } catch { return false; }
    if (color !== turn) return false;
    if (this._movable === 'both') return true;
    return (this._movable === 'white' ? 'w' : 'b') === color;
  }

  _select(sq) {
    this._selected = sq;
    this._dests = new Map();
    this._selPremove = !this._canMove(sq) && this._canPremove(sq);
    let moves = [];
    if (this._selPremove) {
      moves = this._premoveDests(sq).map((to) => ({ from: sq, to, flags: '' }));
    } else {
      try { moves = this._chess.moves({ square: sq, verbose: true }); } catch { moves = []; }
    }
    for (const m of moves) {
      const list = this._dests.get(m.to);
      if (list) list.push(m); else this._dests.set(m.to, [m]);
    }
    this._renderMarks();
  }

  _showDests() { this._renderMarks(); }

  _deselect() {
    if (!this._selected) return;
    this._selected = null;
    this._dests = new Map();
    this._selPremove = false;
    this._renderMarks();
  }

  _cancelInteraction() {
    if (this._drag) this._endDrag(true);
    if (this._rdrag) { this._rdrag = null; this._renderShapes(); }
    if (this._promo) this._closePromo(true);
    this._hoverSq = null;
    this._selected = null;
    this._dests = new Map();
    this._selPremove = false;
  }

  // ------------------------------------------------------------- pointers

  _onPointerDown(e) {
    if (this._destroyed || this._promo) return;
    if (e.pointerType === 'mouse' && e.button !== 0 && e.button !== 2) return;
    if (this._kbd) { this._kbd = false; this.root.classList.remove('kbd'); }
    // Mouse/touch never moves DOM focus onto a square: page shortcuts (← → move navigation) keep
    // working after a click, and a focused text field (typed moves) keeps its focus.
    if (e.cancelable) e.preventDefault();
    const ae = document.activeElement;
    if (ae && ae !== document.body && !this.root.contains(ae) && typeof ae.blur === 'function'
      && !/^(INPUT|TEXTAREA|SELECT)$/.test(ae.tagName) && !ae.isContentEditable) ae.blur();
    this._rect = null; // page may have scrolled since last measurement
    const sq = this._squareAt(e);
    if (!sq) return;

    if (e.button === 2) {
      e.preventDefault();
      // Right-click cancels a queued premove (or a premove selection) instead of drawing.
      if (this._premove || this._selPremove) {
        if (this._drag) this._endDrag(true);
        this._deselect();
        this.clearPremove(true);
        return;
      }
      this._rdrag ={ from: sq, to: sq, color: this._shapeColor(e), pointerId: e.pointerId };
      try { this.root.setPointerCapture(e.pointerId); } catch { /* ignore */ }
      return;
    }

    if (e.isPrimary === false) return;
    // Left click clears user drawings (like chess.com / lichess).
    if (this._userArrows.length || this._userCircles.length) {
      this._userArrows = [];
      this._userCircles = [];
      this._renderShapes();
    }

    if (typeof this.opts.onSquareClick === 'function') {
      try { this.opts.onSquareClick(sq); } catch (err) { console.error(err); }
      if (this._destroyed) return;
    }

    // Click-click move.
    if (this._selected && this._selected !== sq && this._dests.has(sq)) {
      e.preventDefault();
      this._tryMove(this._selected, sq, { dragged: false });
      return;
    }

    if (this._canMove(sq) || this._canPremove(sq)) {
      e.preventDefault();
      const wasSelected = this._selected === sq;
      if (!wasSelected) this._select(sq);
      const p = this._pieces.get(sq);
      this._drag = {
        from: sq,
        el: p.el,
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
        moved: false,
        wasSelected,
        threshold: e.pointerType === 'mouse' ? 3 : 6,
        touch: e.pointerType !== 'mouse',
        lifted: false,
        lastEvent: null,
      };
      try { this.root.setPointerCapture(e.pointerId); } catch { /* ignore */ }
      // Lift the piece under the pointer right away, like chess.com.
      this._startDragVisual();
      this._drag.lastEvent = { clientX: e.clientX, clientY: e.clientY };
      this._positionDrag(this._drag.lastEvent);
      return;
    }

    // Clicked a square the selected piece can't go to.
    if (this._selected) this._deselect();
    // Clicking anywhere else cancels a queued premove (like chess.com / lichess).
    else if (this._premove) this.clearPremove(true);
  }

  _onPointerMove(e) {
    if (this._destroyed) return;
    if (this._rdrag && e.pointerId === this._rdrag.pointerId) {
      const sq = this._squareAt(e);
      if (sq && sq !== this._rdrag.to) {
        this._rdrag.to = sq;
        this._renderShapes();
      }
      return;
    }
    const d = this._drag;
    if (d && e.pointerId === d.pointerId) {
      d.lastEvent = { clientX: e.clientX, clientY: e.clientY };
      this._positionDrag(d.lastEvent);
      if (!d.moved) {
        if (Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < d.threshold) return;
        d.moved = true;
      }
      const sq = this._squareAt(e);
      const hover = sq && sq !== d.from && this._dests.has(sq) ? sq : (sq && sq !== d.from ? sq : null);
      if (hover !== this._hoverSq) {
        if (this._hoverSq) this._sqEl(this._hoverSq).classList.remove('hover');
        this._hoverSq = hover;
        if (hover) this._sqEl(hover).classList.add('hover');
      }
      return;
    }
    // Square-name tag under the mouse (when "show square names" is on; CSS hides it otherwise).
    if (e.pointerType === 'mouse') {
      const sq = this._squareAt(e);
      if (sq !== this._hoverNameSq) { this._hoverNameSq = sq; this._showHoverName(sq); }
    }
    // Idle hover: show a grab cursor over movable pieces (mouse only).
    if (e.pointerType === 'mouse' && (this._interactive || this._premoveColor)) {
      const sq = this._squareAt(e);
      if (sq !== this._cursorSq) {
        this._cursorSq = sq;
        const grab = sq && (this._canMove(sq) || this._canPremove(sq) || (this._selected && this._dests.has(sq)));
        this.root.classList.toggle('can-grab', !!grab);
      }
    }
  }

  _startDragVisual() {
    const d = this._drag;
    if (!d || d.lifted) return;
    d.lifted = true;
    // No ghost: like chess.com the origin square just stays highlighted.
    d.el.classList.remove('gm-moving', 'gm-fade-in');
    d.el.classList.add('gm-dragging');
    if (d.touch) d.el.classList.add('gm-touch');
    this.root.classList.add('dragging');
  }

  _positionDrag(pt) {
    const d = this._drag;
    if (!d || !d.lifted) return;
    const rect = this._getRect();
    const size = rect.width / 8;
    const x = pt.clientX - rect.left - size / 2;
    // On touch, float the piece above the finger so it stays visible.
    const y = pt.clientY - rect.top - size / 2 - (d.touch ? size * 0.45 : 0);
    d.el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
  }

  /** Put a piece element on its square without any slide. */
  _placeInstant(el, sq) {
    el.style.transition = 'none';
    this._placeEl(el, sq);
    void el.offsetWidth;
    el.style.transition = '';
  }

  _onPointerUp(e) {
    if (this._destroyed) return;
    if (this._rdrag && e.pointerId === this._rdrag.pointerId) {
      const { from, color } = this._rdrag;
      const to = this._squareAt(e) || this._rdrag.to;
      this._rdrag = null;
      try { this.root.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
      if (from === to) this._toggleCircle(from, color);
      else this._toggleArrow(from, to, color);
      return;
    }
    const d = this._drag;
    if (!d || e.pointerId !== d.pointerId) return;
    const sq = this._squareAt(e);
    const from = d.from;
    if (!d.moved) {
      this._endDrag(false, true);
      const p = this._pieces.get(from);
      if (p) this._placeInstant(p.el, from);
      if (d.wasSelected && sq === from) this._deselect(); // second click on a selected piece
      return;
    }
    this._endDrag(false, true);
    if (sq && sq !== from && this._dests.has(sq)) {
      this._tryMove(from, sq, { dragged: true });
    } else {
      // Dropped on its own square: settle instantly and stay selected.
      // Dropped elsewhere: glide back (no error sound, just like chess.com).
      const p = this._pieces.get(from);
      if (p) {
        if (sq === from) this._placeInstant(p.el, from);
        else this._placeEl(p.el, from);
      }
      if (sq !== from) this._deselect();
    }
  }

  _onPointerCancel(e) {
    if (this._rdrag && (!e || e.pointerId === this._rdrag.pointerId)) {
      this._rdrag = null;
      this._renderShapes();
    }
    if (this._drag && (!e || e.pointerId === this._drag.pointerId)) {
      // lostpointercapture also fires after a normal pointerup; _endDrag is idempotent.
      this._endDrag(true);
    }
  }

  /**
   * Finish a drag. revert = put the piece back on its square.
   * keepPosition = leave the element where it is (the caller will place it).
   */
  _endDrag(revert, keepPosition = false) {
    const d = this._drag;
    if (!d) return;
    this._drag = null;
    d.el.classList.remove('gm-dragging', 'gm-touch');
    this.root.classList.remove('dragging');
    if (this._hoverSq) {
      this._sqEl(this._hoverSq).classList.remove('hover');
      this._hoverSq = null;
    }
    try { if (this.root.hasPointerCapture?.(d.pointerId)) this.root.releasePointerCapture(d.pointerId); } catch { /* ignore */ }
    if (revert && d.lifted && !keepPosition) {
      const p = this._pieces.get(d.from);
      if (p) {
        if (d.moved) this._placeEl(p.el, d.from);
        else this._placeInstant(p.el, d.from);
      }
    }
  }

  // ---------------------------------------------------------------- moves

  _tryMove(from, to, { dragged }) {
    const candidates = this._dests.get(to) || [];
    if (!candidates.length) return;
    if (this._selPremove) {
      this._queuePremove(from, to, dragged);
      return;
    }
    const promo = candidates.some((m) => m.promotion);
    if (!promo) {
      this._commit(from, to, undefined, dragged);
      return;
    }
    if (this._cfg('autoQueen')) {
      this._commit(from, to, 'q', dragged);
      return;
    }
    this._openPromo(from, to, dragged);
  }

  _commit(from, to, promotion, dragged) {
    const prevLast = this._lastMove;
    let mv;
    try {
      mv = this._chess.move({ from, to, promotion });
    } catch {
      mv = null;
    }
    if (!mv) {
      if (this.opts.sounds) playSound('illegal');
      this._deselect();
      this._render({ animate: true });
      return;
    }
    this._selected = null;
    this._dests = new Map();
    this._lastMove = [mv.from, mv.to];
    if (this._userArrows.length || this._userCircles.length) this.clearUserShapes();
    this._render({ animate: true, instant: dragged ? [mv.to] : [] });
    const moveObj = this._moveObject(mv);
    this._announceMove(mv, { mine: true });

    let res;
    if (typeof this.opts.onMove === 'function') {
      try { res = this.opts.onMove(moveObj); } catch (err) { console.error('[Board] onMove error', err); }
    }
    if (this._destroyed) return;
    const revert = () => {
      if (this._destroyed) return;
      if (this._chess.fen() !== moveObj.fen) return; // page already moved on
      try { this._chess.undo(); } catch { /* ignore */ }
      if (this._chess.fen() !== moveObj.before) {
        const c = makeChess(moveObj.before);
        if (c) this._chess = c;
      }
      this._lastMove = prevLast;
      this._render({ animate: true });
    };
    if (res === false) {
      revert();
      return;
    }
    // A dropped piece lands instantly; a clicked move lands when its slide ends.
    if (this.opts.sounds) this._landSound(this._soundForMove(mv), !dragged);
    if (res && typeof res.then === 'function') {
      res.then((ok) => { if (ok === false) revert(); }, () => {});
    }
  }

  _moveObject(mv) {
    return {
      from: mv.from,
      to: mv.to,
      promotion: mv.promotion || undefined,
      uci: mv.from + mv.to + (mv.promotion || ''),
      san: mv.san,
      fen: mv.after,
      before: mv.before,
      captured: mv.captured || undefined,
      flags: mv.flags,
      color: mv.color,
      piece: mv.piece,
    };
  }

  _soundForMove(mv) {
    let check = false;
    try { check = this._chess.inCheck(); } catch { /* ignore */ }
    if (check) return 'check';
    if (mv.promotion) return 'promote';
    if (mv.flags.includes('k') || mv.flags.includes('q')) return 'castle';
    if (mv.captured) return 'capture';
    return 'move';
  }

  _soundForTransition(prevMap, [from, to]) {
    let check = false;
    try { check = this._chess.inCheck(); } catch { /* ignore */ }
    if (check) return 'check';
    const moved = prevMap.get(from);
    const now = this._pieces.get(to)?.code;
    if (moved && moved[1] === 'P' && now && now[1] !== 'P') return 'promote';
    if (moved && moved[1] === 'K' && Math.abs(fileOf(from) - fileOf(to)) >= 2) return 'castle';
    const victim = prevMap.get(to);
    if (victim && moved && victim[0] !== moved[0]) return 'capture';
    if (moved && moved[1] === 'P' && fileOf(from) !== fileOf(to) && !victim) return 'capture'; // en passant
    return 'move';
  }

  // ------------------------------------------------------------ promotion

  _openPromo(from, to, dragged, premove = false) {
    const color = this._pieces.get(from)?.code[0] || this._chess.turn();
    // Show the pawn on the promotion square while choosing (not for a premove: nothing moved yet).
    const p = this._pieces.get(from);
    if (p && !premove) {
      if (dragged) { p.el.style.transition = 'none'; this._placeEl(p.el, to); void p.el.offsetWidth; p.el.style.transition = ''; }
      else this._placeEl(p.el, to);
    }
    const victim = premove ? null : this._pieces.get(to);
    if (victim) victim.el.classList.add('gm-hidden');

    const overlay = document.createElement('div');
    overlay.className = premove ? 'gm-promo gm-promo-premove' : 'gm-promo';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-label', t('ui.board.promoteChoose'));
    const [col, row] = this._vis(to);
    const down = row === 0;
    const names = { q: t('common.pieces.queen'), n: t('common.pieces.knight'), r: t('common.pieces.rook'), b: t('common.pieces.bishop') };
    PROMO_PIECES.forEach((pc, i) => {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'gm-promo-choice';
      btn.dataset.piece = pc;
      btn.setAttribute('aria-label', t('ui.board.promoteTo', { piece: names[pc] }));
      btn.title = names[pc];
      const r = down ? row + i : row - i;
      btn.style.left = `${col * 12.5}%`;
      btn.style.top = `${r * 12.5}%`;
      const img = document.createElement('div');
      img.className = `gm-piece-img ${color}${pc.toUpperCase()}`;
      btn.appendChild(img);
      overlay.appendChild(btn);
    });
    const onClick = (e) => {
      e.stopPropagation();
      const btn = e.target.closest?.('.gm-promo-choice');
      if (btn) {
        const pc = btn.dataset.piece;
        this._closePromo(false);
        if (premove) {
          this._premove = { from, to, promotion: pc };
          this._renderMarks();
          this._emitPremove();
        } else {
          this._commit(from, to, pc, true);
        }
      } else {
        this._closePromo(true);
      }
    };
    const onDown = (e) => { e.stopPropagation(); };
    overlay.addEventListener('click', onClick);
    overlay.addEventListener('pointerdown', onDown);
    document.addEventListener('keydown', this._onKeyDown);
    this.root.appendChild(overlay);
    this._promo = { overlay, onClick, onDown, from, to, victim, premove };
    const first = overlay.querySelector('button');
    if (first) { try { first.focus({ preventScroll: true }); } catch { /* ignore */ } }
  }

  _onKeyDown(e) {
    if (e.key === 'Escape' && this._promo) {
      e.preventDefault();
      this._closePromo(true);
    }
  }

  _closePromo(cancel) {
    const pr = this._promo;
    if (!pr) return;
    this._promo = null;
    pr.overlay.removeEventListener('click', pr.onClick);
    pr.overlay.removeEventListener('pointerdown', pr.onDown);
    document.removeEventListener('keydown', this._onKeyDown);
    const hadFocus = pr.overlay.contains(document.activeElement);
    pr.overlay.remove();
    if (pr.victim) pr.victim.el.classList.remove('gm-hidden');
    if (hadFocus && !this._destroyed) this._later(() => { if (!this._destroyed && !this._promo) this._setFocusSquare(this._focusSq || pr.to); }, 0);
    if (cancel) {
      const p = this._pieces.get(pr.from);
      if (p) this._placeEl(p.el, pr.from);
      this._selected = null;
      this._dests = new Map();
      this._renderMarks();
    }
  }

  // --------------------------------------------------------------- shapes

  _shapeColor(e) {
    if (e.altKey && e.shiftKey) return 'yellow';
    if (e.shiftKey || e.ctrlKey) return 'red';
    if (e.altKey || e.metaKey) return 'blue';
    return 'green';
  }

  _toggleArrow(from, to, color) {
    const i = this._userArrows.findIndex((a) => a.from === from && a.to === to);
    if (i >= 0) {
      const same = this._userArrows[i].color === color;
      this._userArrows.splice(i, 1);
      if (!same) this._userArrows.push({ from, to, color });
    } else {
      this._userArrows.push({ from, to, color });
      if (this._userArrows.length > MAX_SHAPES) this._userArrows.shift();
    }
    this._renderShapes();
  }

  _toggleCircle(square, color) {
    const i = this._userCircles.findIndex((c) => c.square === square);
    if (i >= 0) {
      const same = this._userCircles[i].color === color;
      this._userCircles.splice(i, 1);
      if (!same) this._userCircles.push({ square, color });
    } else {
      this._userCircles.push({ square, color });
      if (this._userCircles.length > MAX_SHAPES) this._userCircles.shift();
    }
    this._renderShapes();
  }

  /** @param {{from:string,to:string,color?:string}[]} arrows */
  setArrows(arrows = []) {
    if (this._destroyed) return;
    this._arrows = (Array.isArray(arrows) ? arrows : [])
      .filter((a) => a && isSquare(a.from) && isSquare(a.to))
      .slice(0, MAX_SHAPES)
      .map((a) => ({ from: a.from, to: a.to, color: a.color || 'green' }));
    this._renderShapes();
  }

  clearArrows() { this.setArrows([]); }

  /** Programmatic circles: [{square, color}] (extension, not in the contract). */
  setCircles(circles = []) {
    if (this._destroyed) return;
    this._circles = (Array.isArray(circles) ? circles : [])
      .filter((c) => c && isSquare(c.square))
      .slice(0, MAX_SHAPES)
      .map((c) => ({ square: c.square, color: c.color || 'green' }));
    this._renderShapes();
  }

  /** Remove arrows/circles the user drew with right-click. */
  clearUserShapes() {
    this._userArrows = [];
    this._userCircles = [];
    this._renderShapes();
  }

  /** User-drawn shapes (e.g. to save them in a study). */
  getUserShapes() {
    return { arrows: this._userArrows.map((a) => ({ ...a })), circles: this._userCircles.map((c) => ({ ...c })) };
  }

  _arrowSvg(from, to, color, extraClass = '') {
    const [fc, fr] = this._vis(from);
    const [tc, tr] = this._vis(to);
    const x1 = fc + 0.5; const y1 = fr + 0.5;
    const x2 = tc + 0.5; const y2 = tr + 0.5;
    const dx = tc - fc; const dy = tr - fr;
    const shaft = 0.2; const headW = 0.5; const headL = 0.42;
    const col = safeColor(color);
    const knight = (Math.abs(dx) === 1 && Math.abs(dy) === 2) || (Math.abs(dx) === 2 && Math.abs(dy) === 1);
    let pts; // polyline points of the shaft (ending at the head base)
    let ux; let uy; // direction of the final segment
    if (knight) {
      // Long leg first, then the short leg.
      const cx = Math.abs(dx) === 2 ? x2 : x1;
      const cy = Math.abs(dx) === 2 ? y1 : y2;
      const lx = x2 - cx; const ly = y2 - cy;
      const len = Math.hypot(lx, ly);
      ux = lx / len; uy = ly / len;
      const sx = x1 + Math.sign(cx - x1) * 0.25 * (cx !== x1 ? 1 : 0);
      const sy = y1 + Math.sign(cy - y1) * 0.25 * (cy !== y1 ? 1 : 0);
      pts = [[sx, sy], [cx, cy], [x2 - ux * headL, y2 - uy * headL]];
    } else {
      const len = Math.hypot(x2 - x1, y2 - y1);
      ux = (x2 - x1) / len; uy = (y2 - y1) / len;
      pts = [[x1 + ux * 0.25, y1 + uy * 0.25], [x2 - ux * headL, y2 - uy * headL]];
    }
    const [bx, by] = pts[pts.length - 1];
    const px = -uy; const py = ux;
    const head = [
      [x2, y2],
      [bx + px * headW / 2, by + py * headW / 2],
      [bx - px * headW / 2, by - py * headW / 2],
    ];
    const f = (n) => n.toFixed(3);
    const line = pts.map(([x, y]) => `${f(x)},${f(y)}`).join(' ');
    // Head base overlaps the shaft end by a hair to avoid a seam; the shaft stops at the base.
    return `<g class="gm-arrow ${extraClass}" style="--c:${col}">`
      + `<polyline points="${line}" fill="none" stroke-width="${shaft}" stroke-linejoin="round" stroke-linecap="butt" style="stroke:var(--c)"/>`
      + `<polygon points="${head.map(([x, y]) => `${f(x)},${f(y)}`).join(' ')}" style="fill:var(--c)"/>`
      + '</g>';
  }

  _circleSvg(square, color, extraClass = '') {
    const [c, r] = this._vis(square);
    return `<circle class="gm-circle ${extraClass}" cx="${c + 0.5}" cy="${r + 0.5}" r="0.455" fill="none" stroke-width="0.075" style="stroke:${safeColor(color)}"/>`;
  }

  _renderShapes() {
    if (this._destroyed) return;
    let out = '';
    for (const c of this._circles) out += this._circleSvg(c.square, c.color, 'auto');
    for (const c of this._userCircles) out += this._circleSvg(c.square, c.color, 'user');
    for (const a of this._arrows) out += this._arrowSvg(a.from, a.to, a.color, 'auto');
    for (const a of this._userArrows) out += this._arrowSvg(a.from, a.to, a.color, 'user');
    const rd = this._rdrag;
    if (rd) {
      if (rd.from === rd.to) out += this._circleSvg(rd.from, rd.color, 'preview');
      else out += this._arrowSvg(rd.from, rd.to, rd.color, 'preview');
    }
    this._svg.innerHTML = out;
  }

  // ----------------------------------------------------------- highlights

  /** @param {{square:string, kind:string}[]} list */
  setHighlights(list = []) {
    if (this._destroyed) return;
    this._highlights = new Map();
    for (const h of Array.isArray(list) ? list : []) {
      if (h && isSquare(h.square)) this._highlights.set(h.square, HIGHLIGHT_KINDS.has(h.kind) ? h.kind : 'hint');
    }
    this._renderMarks();
  }

  clearHighlights() { this.setHighlights([]); }

  // --------------------------------------------------------------- badges

  /** Show a move-classification badge at the top-right of a square. */
  setBadge(square, classification) {
    if (this._destroyed || !isSquare(square)) return;
    if (!classification) this._badges.delete(square);
    else this._badges.set(square, String(classification));
    this._renderBadges();
  }

  clearBadges() {
    if (!this._badges.size) return;
    this._badges.clear();
    this._renderBadges();
  }

  _renderBadges() {
    if (this._destroyed) return;
    this._badgesEl.textContent = '';
    for (const [sq, cls] of this._badges) {
      const meta = classificationMeta(cls);
      const [c, r] = this._vis(sq);
      const b = document.createElement('div');
      b.className = 'gm-badge';
      b.dataset.cls = meta.key;
      if (c === 7) b.classList.add('edge-right');
      if (r === 0) b.classList.add('edge-top');
      b.style.setProperty('--c', String(c));
      b.style.setProperty('--r', String(r));
      b.style.setProperty('--cls', meta.cssVar);
      b.title = meta.label;
      const sym = meta.symbol || '';
      b.textContent = sym;
      if (/\p{Extended_Pictographic}/u.test(sym)) b.classList.add('emoji');
      else if (sym.length > 1) b.classList.add('wide');
      this._badgesEl.appendChild(b);
    }
  }

  // -------------------------------------------------------------- destroy

  destroy() {
    if (this._destroyed) return;
    this._cancelInteraction();
    this._destroyed = true;
    for (const id of this._timers) clearTimeout(id);
    this._timers.clear();
    const r = this.root;
    r.removeEventListener('pointerdown', this._onPointerDown);
    r.removeEventListener('pointermove', this._onPointerMove);
    r.removeEventListener('pointerup', this._onPointerUp);
    r.removeEventListener('pointercancel', this._onPointerCancel);
    r.removeEventListener('lostpointercapture', this._onPointerCancel);
    r.removeEventListener('contextmenu', this._onContextMenu);
    r.removeEventListener('keydown', this._onBoardKey);
    r.removeEventListener('focusin', this._onFocusIn);
    r.removeEventListener('focusout', this._onFocusOut);
    r.removeEventListener('pointerleave', this._onPointerLeave);
    document.removeEventListener('keydown', this._onKeyDown);
    if (this._ro) { this._ro.disconnect(); this._ro = null; }
    if (this._offSettings) { this._offSettings(); this._offSettings = null; }
    r.remove();
    this._pieces.clear();
    this._highlights.clear();
    this._badges.clear();
    this._sqEls = [];
    this._coordRank = [];
    this._coordFile = [];
    this._chess = null;
    this.opts.onMove = null;
    this.opts.onSquareClick = null;
    this.opts.onPremove = null;
    this._premove = null;
  }
}

export default Board;
