// GrandMentor — position editor (board + piece palette + position settings).
// Contract: docs/CONTRACT.md "Position editor". Styles: web/css/editor.css (injected once).
//
//   const ed = new PositionEditor(el, { fen, orientation, onChange(fen, { valid, errors }) });
//   ed.getFen(); ed.setFen(fen); ed.flip(); ed.valid; ed.errors; ed.sideEl (slot for page actions); ed.destroy();
//
// Built on the shared Board component (rendering, coordinates, keyboard cursor, ARIA grid): the board
// is created view-only and the editor owns every pointer interaction through capture-phase listeners
// on the board slot, so Board never tries to play moves here.
//
// Input
//   - Palette: white and black pieces, a "move" hand and an eraser. Click/Enter selects a tool; drag a
//     piece straight onto the board.
//   - Board: click/tap a square to apply the selected tool (same piece again removes it); drag pieces
//     around; drop them off the board to remove them; right-click or long-press removes a piece.
//   - Keyboard: Tab to a palette button, Enter to pick it, Tab into the board, arrows to move,
//     Enter/Space to apply (with the hand tool: pick up, then drop), Delete/Backspace removes.
//
// Memory: every listener, timer and DOM node created here is released by destroy().

import { Chess, validateFen } from '../../vendor/chess.js';
import { h, icon } from '../ui.js';
import { pieceUrl, onSettingsChange } from '../settings.js';
import { t, formatList } from '../i18n.js';
import { Board } from './board.js';
import { announce, coloredPiece } from './announcer.js';

export const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
export const EMPTY_PLACEMENT = '8/8/8/8/8/8/8/8';
const PIECES = ['K', 'Q', 'R', 'B', 'N', 'P'];
const FILES = 'abcdefgh';
const MAX_FEN = 120;
const LONG_PRESS_MS = 500;
const CSS_HREF = '/css/editor.css';

/** Inject the editor stylesheet once. */
function ensureCss() {
  if (typeof document === 'undefined' || document.querySelector(`link[href="${CSS_HREF}"]`)) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = CSS_HREF;
  link.dataset.pageCss = 'editor';
  document.head.appendChild(link);
}

const sqIndex = (sq) => (8 - Number(sq[1])) * 8 + (sq.charCodeAt(0) - 97); // a8 = 0 … h1 = 63
const sqName = (idx) => FILES[idx % 8] + (8 - Math.floor(idx / 8));
const codeOf = (ch) => (ch === ch.toUpperCase() ? 'w' : 'b') + ch.toUpperCase();
const cap = (s) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);
const charOf = (code) => (code[0] === 'w' ? code[1] : code[1].toLowerCase());

/** Parse a FEN placement field into a 64-array of piece codes (a8 first). null if unreadable. */
export function parsePlacement(placement) {
  const rows = String(placement || '').split('/');
  if (rows.length !== 8) return null;
  const arr = new Array(64).fill(null);
  for (let r = 0; r < 8; r++) {
    let f = 0;
    for (const ch of rows[r]) {
      if (/[1-8]/.test(ch)) f += Number(ch);
      else if (/[KQRBNPkqrbnp]/.test(ch)) { if (f < 8) arr[r * 8 + f] = codeOf(ch); f++; }
      else return null;
    }
    if (f !== 8) return null;
  }
  return arr;
}

export function placementOf(arr) {
  const rows = [];
  for (let r = 0; r < 8; r++) {
    let s = '';
    let empty = 0;
    for (let f = 0; f < 8; f++) {
      const p = arr[r * 8 + f];
      if (!p) { empty++; continue; }
      if (empty) { s += empty; empty = 0; }
      s += charOf(p);
    }
    if (empty) s += empty;
    rows.push(s);
  }
  return rows.join('/');
}

/** Castling rights that the king/rook placement allows. */
export function possibleCastling(arr) {
  return {
    K: arr[60] === 'wK' && arr[63] === 'wR',
    Q: arr[60] === 'wK' && arr[56] === 'wR',
    k: arr[4] === 'bK' && arr[7] === 'bR',
    q: arr[4] === 'bK' && arr[0] === 'bR',
  };
}

/** En-passant target squares that make sense for this placement and side to move. */
export function epCandidates(arr, turn) {
  const out = [];
  // White to move: a black pawn just went x7→x5, so the target is x6 (row index 2).
  const [targetRow, pawnRow, fromRow, pawn] = turn === 'w' ? [2, 3, 1, 'bP'] : [5, 4, 6, 'wP'];
  const capturer = turn === 'w' ? 'wP' : 'bP';
  for (let f = 0; f < 8; f++) {
    if (arr[pawnRow * 8 + f] !== pawn || arr[targetRow * 8 + f] || arr[fromRow * 8 + f]) continue;
    const left = f > 0 && arr[pawnRow * 8 + f - 1] === capturer;
    const right = f < 7 && arr[pawnRow * 8 + f + 1] === capturer;
    if (left || right) out.push(sqName(targetRow * 8 + f));
  }
  return out;
}

/**
 * Validate an editor position. Returns a list of friendly (translated) reasons; empty = playable.
 * @param {{arr: (string|null)[], turn: 'w'|'b'}} st
 * @param {string} fen the full FEN built from st
 */
export function positionErrors(st, fen) {
  const errs = [];
  const count = (code) => st.arr.filter((p) => p === code).length;
  for (const c of ['w', 'b']) {
    const k = count(`${c}K`);
    if (k === 0) errs.push(t(`editor.errors.noKing.${c}`));
    else if (k > 1) errs.push(t(`editor.errors.tooManyKings.${c}`));
  }
  const back = [];
  for (let i = 0; i < 64; i++) {
    if ((i < 8 || i >= 56) && (st.arr[i] === 'wP' || st.arr[i] === 'bP')) back.push(sqName(i));
  }
  if (back.length) errs.push(t('editor.errors.pawnsBackRank', { squares: formatList(back) }));
  for (const c of ['w', 'b']) {
    if (count(`${c}P`) > 8) errs.push(t(`editor.errors.tooManyPawns.${c}`));
    if (st.arr.filter((p) => p && p[0] === c).length > 16) errs.push(t(`editor.errors.tooManyPieces.${c}`));
  }
  if (errs.length) return errs;
  // The side that is NOT to move must not be in check (it would mean its king can be captured).
  try {
    const parts = fen.split(' ');
    const other = new Chess(`${parts[0]} ${st.turn === 'w' ? 'b' : 'w'} - - 0 1`);
    if (other.inCheck()) errs.push(t(`editor.errors.wrongSideInCheck.${st.turn === 'w' ? 'b' : 'w'}`));
  } catch { /* reported below */ }
  if (!errs.length) {
    const v = validateFen(fen);
    if (!v.ok) errs.push(t('editor.errors.generic'));
    else {
      try { new Chess(fen); } catch { errs.push(t('editor.errors.generic')); }
    }
  }
  return errs;
}

/** Normalise a FEN typed by a user or found in a link: 1-6 fields → state, or null if unreadable. */
export function parseFen(raw) {
  const s = String(raw || '').trim().replace(/\s+/g, ' ');
  if (!s || s.length > MAX_FEN) return null;
  const p = s.split(' ');
  const arr = parsePlacement(p[0]);
  if (!arr) return null;
  const turn = p[1] === 'b' ? 'b' : 'w';
  const castling = new Set(/^[KQkq]+$/.test(p[2] || '') ? p[2].split('') : []);
  const ep = /^[a-h][36]$/.test(p[3] || '') ? p[3] : '-';
  const half = Math.min(150, Math.max(0, parseInt(p[4], 10) || 0));
  const full = Math.min(999, Math.max(1, parseInt(p[5], 10) || 1));
  return { arr, turn, castling, ep, half, full };
}

export class PositionEditor {
  /**
   * @param {HTMLElement} el container
   * @param {{fen?: string, orientation?: 'white'|'black', onChange?: (fen: string, info: {valid: boolean, errors: string[]}) => void}} [opts]
   */
  constructor(el, opts = {}) {
    if (!el) throw new Error('PositionEditor: container element required');
    ensureCss();
    this.el = el;
    this.opts = { fen: START_FEN, orientation: 'white', onChange: null, ...opts };
    this._destroyed = false;
    this._offs = [];
    this._timers = new Set();
    this._drag = null;
    this._kbdPick = null;
    this.tool = 'move';
    this.errors = [];
    this.gameOver = null;
    const parsed = parseFen(this.opts.fen) || parseFen(START_FEN);
    this.st = parsed;
    this._build();
    this.board = new Board(this.boardSlot, {
      fen: this._displayFen(),
      orientation: this.opts.orientation === 'black' ? 'black' : 'white',
      interactive: false,
      movableColor: null,
      sounds: false,
      announce: false,
      label: t('editor.boardLabel'),
    });
    this._bind();
    this.setTool('move');
    this._update({ animate: false });
  }

  // ------------------------------------------------------------------ DOM

  _build() {
    const toolBtn = (tool, label, inner) => h('button', {
      class: 'pe-tool', type: 'button', dataset: { tool }, 'aria-label': label, title: label, 'aria-pressed': 'false', html: inner,
    });
    const palette = (c) => h('div', { class: `pe-palette pe-palette-${c}`, role: 'toolbar', 'aria-label': t(`editor.palette.${c}`) },
      PIECES.map((p) => toolBtn(c + p, cap(coloredPiece(c + p)), `<img src="${pieceUrl(c + p)}" alt="" draggable="false">`)),
      c === 'w'
        ? toolBtn('move', t('editor.tools.move'), icon('grid'))
        : toolBtn('erase', t('editor.tools.erase'), icon('trash')));
    this.paletteB = palette('b');
    this.paletteW = palette('w');
    this.boardSlot = h('div', { class: 'pe-board' });
    this.hint = h('p', { class: 'pe-hint subtle text-sm' });

    const btn = (ic, label, fn) => {
      const b = h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon(ic) });
      b.appendChild(h('span', null, label));
      b.addEventListener('click', fn);
      this._offs.push(() => b.removeEventListener('click', fn));
      return b;
    };
    const quick = h('div', { class: 'pe-quick' },
      btn('refresh', t('editor.startPos'), () => this.setFen(START_FEN, { announceIt: t('editor.announce.start') })),
      btn('trash', t('editor.clear'), () => this.setFen(`${EMPTY_PLACEMENT} ${this.st.turn} - - 0 1`, { announceIt: t('editor.announce.cleared') })),
      btn('flip', t('editor.flip'), () => this.flip()));

    this.turnW = h('button', { type: 'button', role: 'radio' }, t('editor.whiteToMove'));
    this.turnB = h('button', { type: 'button', role: 'radio' }, t('editor.blackToMove'));
    const turnGroup = h('div', { class: 'segmented block', role: 'radiogroup', 'aria-label': t('editor.sideToMove') }, this.turnW, this.turnB);

    // Castling rights are toggle buttons (big enough to tap; aria-pressed for screen readers).
    this.castleBoxes = ['K', 'Q', 'k', 'q'].map((c) => {
      const el = h('button', { class: 'pe-castle', type: 'button', 'aria-pressed': 'false', html: icon('check') });
      el.appendChild(h('span', null, t(`editor.castle.${c === c.toUpperCase() ? 'w' : 'b'}${c.toUpperCase()}`)));
      return { c, el };
    });
    this.castleNote = h('p', { class: 'subtle text-xs pe-note' }, t('editor.castleNote'));

    this.epSelect = h('select', { class: 'select input-sm', 'aria-label': t('editor.enPassant') });
    const uid = Math.random().toString(36).slice(2, 8);
    this.fenInput = h('input', {
      class: 'input input-sm mono pe-fen', id: `pe-fen-${uid}`, spellcheck: 'false', autocomplete: 'off',
      maxlength: String(MAX_FEN), 'aria-describedby': `pe-fen-msg-${uid}`,
    });
    this.fenMsg = h('div', { class: 'help pe-fen-msg', id: `pe-fen-msg-${uid}`, role: 'status', 'aria-live': 'polite' });
    this.status = h('div', { class: 'pe-status', role: 'status', 'aria-live': 'polite' });

    const field = (label, ...kids) => h('div', { class: 'field' }, h('div', { class: 'label' }, label), ...kids);
    this.sideEl = h('div', { class: 'pe-actions' });
    this.controls = h('div', { class: 'pe-controls card' },
      quick,
      field(t('editor.sideToMove'), turnGroup),
      field(t('editor.castlingRights'), h('div', { class: 'pe-castling' }, this.castleBoxes.map((b) => b.el)), this.castleNote),
      field(t('editor.enPassant'), this.epSelect),
      h('div', { class: 'field' }, h('label', { class: 'label', for: `pe-fen-${uid}` }, t('editor.fen')), this.fenInput, this.fenMsg),
      this.status);

    this.root = h('div', { class: 'pe' },
      h('div', { class: 'pe-main' }, this.paletteB, this.boardSlot, this.paletteW, this.hint),
      h('div', { class: 'pe-side' }, this.controls, this.sideEl));
    this.el.appendChild(this.root);
  }

  _on(target, type, fn, options) {
    target.addEventListener(type, fn, options);
    this._offs.push(() => target.removeEventListener(type, fn, options));
  }

  _later(fn, ms) {
    const id = setTimeout(() => { this._timers.delete(id); fn(); }, ms);
    this._timers.add(id);
    return id;
  }

  _bind() {
    const slot = this.boardSlot;
    // Capture phase: the editor handles pointers/keys before the (view-only) Board sees them.
    this._on(slot, 'pointerdown', (e) => this._onBoardDown(e), true);
    this._on(slot, 'contextmenu', (e) => e.preventDefault(), true);
    this._on(slot, 'keydown', (e) => this._onBoardKey(e), true);
    for (const pal of [this.paletteW, this.paletteB]) {
      this._on(pal, 'pointerdown', (e) => this._onPaletteDown(e));
      this._on(pal, 'click', (e) => {
        const b = e.target.closest('.pe-tool');
        if (!b) return;
        if (this._suppressClick) { this._suppressClick = false; return; }
        this.setTool(b.dataset.tool, { speak: true });
      });
    }
    this._on(this.turnW, 'click', () => this._setTurn('w'));
    this._on(this.turnB, 'click', () => this._setTurn('b'));
    for (const cb of this.castleBoxes) {
      this._on(cb.el, 'click', () => {
        if (cb.el.disabled) return;
        if (this.st.castling.has(cb.c)) this.st.castling.delete(cb.c); else this.st.castling.add(cb.c);
        this._update();
      });
    }
    this._on(this.epSelect, 'change', () => { this.st.ep = this.epSelect.value || '-'; this._update(); });
    this._on(this.fenInput, 'keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); this.fenInput.blur(); }
      if (e.key === 'Escape') { this.fenInput.value = this.getFen(); this._fenTyped(); this.fenInput.blur(); }
    });
    this._on(this.fenInput, 'input', () => this._fenTyped());
    this._on(this.fenInput, 'focus', () => this.fenInput.select());
    this._on(this.fenInput, 'blur', () => { this.fenInput.value = this.getFen(); this._fenTyped(); });
    this._offs.push(onSettingsChange((_s, key) => {
      if (key !== 'pieceSet') return;
      for (const img of this.root.querySelectorAll('.pe-tool img')) {
        const code = img.closest('.pe-tool')?.dataset.tool;
        if (code) img.src = pieceUrl(code);
      }
    }));
  }

  // -------------------------------------------------------------- state

  _castlingStr() {
    const ok = possibleCastling(this.st.arr);
    return ['K', 'Q', 'k', 'q'].filter((c) => ok[c] && this.st.castling.has(c)).join('') || '-';
  }

  _epStr() {
    return epCandidates(this.st.arr, this.st.turn).includes(this.st.ep) ? this.st.ep : '-';
  }

  /** The FEN the editor currently describes (castling and en passant already sanitised). */
  getFen() {
    const s = this.st;
    return `${placementOf(s.arr)} ${s.turn} ${this._castlingStr()} ${this._epStr()} ${s.half} ${s.full}`;
  }

  _displayFen() {
    return `${placementOf(this.st.arr)} ${this.st.turn} - - 0 1`;
  }

  get valid() { return this.errors.length === 0; }

  get orientation() { return this.board ? this.board.orientation : this.opts.orientation; }

  /**
   * Load a FEN (1-6 fields). Returns false (and leaves the position alone) when it can't be read.
   * Readable but illegal positions are loaded so the user can fix them.
   */
  setFen(fen, { announceIt = null } = {}) {
    if (this._destroyed) return false;
    const p = parseFen(fen);
    if (!p) return false;
    this._cancelDrag(true);
    this.st = p;
    this._kbdPick = null;
    this._update();
    if (announceIt) announce(announceIt);
    return true;
  }

  flip() {
    if (this._destroyed) return;
    this.board.flip();
    announce(t(this.board.orientation === 'white' ? 'editor.announce.flippedWhite' : 'editor.announce.flippedBlack'));
  }

  setOrientation(color) { if (!this._destroyed) this.board.setOrientation(color === 'black' ? 'black' : 'white'); }

  /** Select a palette tool: 'move' | 'erase' | piece code ('wK' … 'bP'). */
  setTool(tool, { speak = false } = {}) {
    if (tool !== 'move' && tool !== 'erase' && !/^[wb][KQRBNP]$/.test(tool)) return;
    this.tool = tool;
    this._kbdPick = null;
    for (const b of this.root.querySelectorAll('.pe-tool')) {
      const on = b.dataset.tool === tool;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', on ? 'true' : 'false');
    }
    this.hint.textContent = tool === 'move' ? t('editor.hints.move') : tool === 'erase' ? t('editor.hints.erase') : t('editor.hints.piece', { piece: coloredPiece(tool) });
    if (speak) announce(this.hint.textContent);
  }

  _setTurn(c) {
    if (this.st.turn === c) return;
    this.st.turn = c;
    this.st.ep = '-';
    this._update();
  }

  _update({ animate = true } = {}) {
    if (this._destroyed) return;
    const s = this.st;
    this.board.setPosition(this._displayFen(), { animate, sound: false });
    if (this._kbdPick) this.board.setHighlights([{ square: this._kbdPick, kind: 'selected' }]);
    else this.board.setHighlights([]);
    const w = s.turn === 'w';
    this.turnW.classList.toggle('active', w);
    this.turnB.classList.toggle('active', !w);
    this.turnW.setAttribute('aria-checked', String(w));
    this.turnB.setAttribute('aria-checked', String(!w));
    const ok = possibleCastling(s.arr);
    for (const cb of this.castleBoxes) {
      cb.el.disabled = !ok[cb.c];
      cb.el.setAttribute('aria-pressed', String(ok[cb.c] && s.castling.has(cb.c)));
    }
    const cands = epCandidates(s.arr, s.turn);
    if (!cands.includes(s.ep)) s.ep = '-';
    const opts = [h('option', { value: '' }, t('editor.noEnPassant')), ...cands.map((sq) => h('option', { value: sq }, sq))];
    this.epSelect.replaceChildren(...opts);
    this.epSelect.value = s.ep === '-' ? '' : s.ep;
    this.epSelect.disabled = !cands.length;
    const fen = this.getFen();
    if (document.activeElement !== this.fenInput) {
      this.fenInput.value = fen;
      this.fenMsg.textContent = '';
      this.fenInput.removeAttribute('aria-invalid');
    }
    this.errors = positionErrors(s, fen);
    this._renderStatus(fen);
    if (typeof this.opts.onChange === 'function') {
      try { this.opts.onChange(fen, { valid: this.valid, errors: this.errors.slice() }); } catch (err) { console.error('[PositionEditor] onChange error', err); }
    }
  }

  _renderStatus(fen) {
    if (this.errors.length) {
      this.gameOver = null;
      this.status.className = 'pe-status callout callout-danger';
      this.status.replaceChildren(h('div', null,
        h('div', { class: 'semibold' }, t('editor.invalidTitle')),
        h('ul', { class: 'pe-errors' }, this.errors.map((e) => h('li', null, e)))));
      return;
    }
    let over = null;
    try {
      const c = new Chess(fen);
      if (c.isCheckmate()) over = 'checkmate';
      else if (c.isStalemate()) over = 'stalemate';
      else if (c.isInsufficientMaterial()) over = 'insufficient';
    } catch { over = null; }
    this.gameOver = over;
    this.status.className = `pe-status callout ${over ? 'callout-warning' : 'callout-success'}`;
    this.status.replaceChildren(h('div', null, over ? t(`editor.over.${over}`) : t(this.st.turn === 'w' ? 'editor.validWhite' : 'editor.validBlack')));
  }

  _fenTyped() {
    const raw = this.fenInput.value;
    const p = parseFen(raw);
    if (!p) {
      this.fenMsg.textContent = raw.trim() ? t('editor.fenUnreadable') : t('editor.fenEmpty');
      this.fenMsg.classList.add('text-danger');
      this.fenInput.setAttribute('aria-invalid', 'true');
      return;
    }
    this.fenMsg.textContent = '';
    this.fenMsg.classList.remove('text-danger');
    this.fenInput.removeAttribute('aria-invalid');
    this.st = p;
    this._kbdPick = null;
    this._update({ animate: false });
  }

  // --------------------------------------------------------- board edits

  _put(sq, code) {
    const i = sqIndex(sq);
    if (code && code[1] === 'K') {
      // One king per side: placing a king moves the existing one.
      const prev = this.st.arr.indexOf(code);
      if (prev >= 0 && prev !== i) this.st.arr[prev] = null;
    }
    this.st.arr[i] = code;
  }

  _applyTool(sq) {
    const i = sqIndex(sq);
    const cur = this.st.arr[i];
    if (this.tool === 'erase') {
      if (!cur) return;
      this.st.arr[i] = null;
      this._update({ animate: false });
      announce(t('editor.announce.removed', { piece: coloredPiece(cur), square: sq }));
    } else if (this.tool !== 'move') {
      if (cur === this.tool) {
        this.st.arr[i] = null;
        this._update({ animate: false });
        announce(t('editor.announce.removed', { piece: coloredPiece(cur), square: sq }));
      } else {
        this._put(sq, this.tool);
        this._update({ animate: false });
        announce(t('editor.announce.placed', { piece: coloredPiece(this.tool), square: sq }));
      }
    }
  }

  _remove(sq) {
    const i = sqIndex(sq);
    const cur = this.st.arr[i];
    if (!cur) return;
    this.st.arr[i] = null;
    this._update({ animate: false });
    announce(t('editor.announce.removed', { piece: coloredPiece(cur), square: sq }));
  }

  _squareAt(x, y) {
    const r = this.boardSlot.getBoundingClientRect();
    if (!r.width || x < r.left || y < r.top || x >= r.right || y >= r.bottom) return null;
    const col = Math.min(7, Math.floor(((x - r.left) / r.width) * 8));
    const row = Math.min(7, Math.floor(((y - r.top) / r.height) * 8));
    return this.board.orientation === 'white' ? FILES[col] + (8 - row) : FILES[7 - col] + (row + 1);
  }

  _onBoardKey(e) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const cell = e.target?.closest?.('.gm-sq');
    const sq = cell?.dataset.square;
    if (!sq) return;
    const keys = ['Enter', ' ', 'Spacebar', 'Delete', 'Backspace', 'Escape'];
    if (!keys.includes(e.key)) return;
    if (e.key === 'Escape') {
      if (!this._kbdPick) return;
      e.preventDefault(); e.stopPropagation();
      this._kbdPick = null;
      this._update({ animate: false });
      announce(t('a11y.board.cancelled'));
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Delete' || e.key === 'Backspace') { this._remove(sq); return; }
    if (this.tool !== 'move') { this._applyTool(sq); return; }
    const code = this.st.arr[sqIndex(sq)];
    if (!this._kbdPick) {
      if (!code) { announce(t('editor.announce.emptyHint', { square: sq })); return; }
      this._kbdPick = sq;
      this._update({ animate: false });
      announce(t('editor.announce.pickedUp', { piece: coloredPiece(code), square: sq }));
      return;
    }
    const from = this._kbdPick;
    this._kbdPick = null;
    if (from !== sq) {
      const moving = this.st.arr[sqIndex(from)];
      this.st.arr[sqIndex(from)] = null;
      if (moving) this._put(sq, moving);
      this._update({ animate: true });
      if (moving) announce(t('editor.announce.placed', { piece: coloredPiece(moving), square: sq }));
    } else {
      this._update({ animate: false });
      announce(t('a11y.board.cancelled'));
    }
  }

  _onBoardDown(e) {
    if (this._destroyed) return;
    e.stopPropagation(); // the view-only Board must not see editor pointers
    if (e.cancelable) e.preventDefault();
    const ae = document.activeElement;
    if (ae && ae !== document.body && !this.boardSlot.contains(ae) && typeof ae.blur === 'function') ae.blur();
    const sq = this._squareAt(e.clientX, e.clientY);
    if (!sq) return;
    if (e.button === 2) { this._remove(sq); return; }
    if (e.button !== 0 || e.isPrimary === false) return;
    this._kbdPick = null;
    const code = this.st.arr[sqIndex(sq)];
    if (!code) { this._applyTool(sq); return; }
    // Pieces can always be dragged; a plain click/tap applies the selected tool.
    this._startDrag(e, code, sq, () => this._applyTool(sq));
    if (e.pointerType !== 'mouse') {
      const d = this._drag;
      d.longPress = this._later(() => {
        if (this._drag !== d || d.moved) return;
        this._cancelDrag(false);
        this._remove(sq);
      }, LONG_PRESS_MS);
    }
  }

  _onPaletteDown(e) {
    if (e.button !== 0 || e.isPrimary === false) return;
    const b = e.target.closest('.pe-tool');
    if (!b) return;
    const tool = b.dataset.tool;
    if (tool === 'move' || tool === 'erase') return; // plain buttons (click handler)
    e.preventDefault();
    this._startDrag(e, tool, null, null);
  }

  /**
   * Start a (potential) drag of `code` from board square `from` (null = from the palette).
   * Below a few pixels of movement it stays a click: `onClick` runs on release.
   */
  _startDrag(e, code, from, onClick) {
    this._cancelDrag(true);
    const size = this.boardSlot.getBoundingClientRect().width / 8 || 48;
    const touch = e.pointerType !== 'mouse';
    const ghost = h('img', { class: 'pe-ghost', src: pieceUrl(code), alt: '', draggable: 'false' });
    const scale = touch ? 1.5 : 1;
    ghost.style.width = `${size * scale}px`;
    ghost.style.height = `${size * scale}px`;
    const d = {
      code, from, onClick, ghost, moved: false, pointerId: e.pointerId,
      sx: e.clientX, sy: e.clientY, threshold: touch ? 6 : 3, half: (size * scale) / 2, touch, longPress: null,
    };
    const place = (x, y) => { ghost.style.transform = `translate(${x - d.half}px, ${y - d.half - (touch ? size * 0.4 : 0)}px)`; };
    const move = (ev) => {
      if (ev.pointerId !== d.pointerId) return;
      if (!d.moved) {
        if (Math.hypot(ev.clientX - d.sx, ev.clientY - d.sy) < d.threshold) return;
        d.moved = true;
        if (d.longPress) { clearTimeout(d.longPress); this._timers.delete(d.longPress); }
        document.body.appendChild(ghost);
        this.root.classList.add('pe-dragging');
        if (from) { this.st.arr[sqIndex(from)] = null; this.board.setPosition(this._displayFen(), { animate: false, sound: false }); }
      }
      if (ev.cancelable) ev.preventDefault();
      place(ev.clientX, ev.clientY);
    };
    const up = (ev) => {
      if (ev.pointerId !== d.pointerId) return;
      this._endDrag();
      if (!d.moved) { if (d.from === null) this._suppressClick = false; d.onClick?.(); return; }
      if (d.from === null) this._suppressClick = true;
      const to = this._squareAt(ev.clientX, ev.clientY);
      if (to) {
        this._put(to, d.code);
        this._update({ animate: false });
        announce(t('editor.announce.placed', { piece: coloredPiece(d.code), square: to }));
      } else {
        this._update({ animate: false });
        if (d.from) announce(t('editor.announce.removed', { piece: coloredPiece(d.code), square: d.from }));
      }
      // The click that follows a palette drag lands elsewhere; reset the guard shortly after.
      this._later(() => { this._suppressClick = false; }, 0);
    };
    const cancel = (ev) => { if (ev.pointerId === d.pointerId) this._cancelDrag(true); };
    window.addEventListener('pointermove', move, { passive: false });
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    d.off = () => {
      window.removeEventListener('pointermove', move, { passive: false });
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel);
      if (d.longPress) { clearTimeout(d.longPress); this._timers.delete(d.longPress); }
      ghost.remove();
      this.root.classList.remove('pe-dragging');
    };
    this._drag = d;
  }

  _endDrag() {
    const d = this._drag;
    this._drag = null;
    if (d) d.off();
    return d;
  }

  /** Abort a drag; `restore` puts a lifted board piece back on its square. */
  _cancelDrag(restore) {
    const d = this._endDrag();
    if (!d) return;
    if (restore && d.moved && d.from) {
      this.st.arr[sqIndex(d.from)] = d.code;
      this._update({ animate: false });
    }
  }

  destroy() {
    if (this._destroyed) return;
    this._cancelDrag(false);
    this._destroyed = true;
    for (const id of this._timers) clearTimeout(id);
    this._timers.clear();
    for (const off of this._offs.splice(0)) { try { off(); } catch { /* ignore */ } }
    this.board.destroy();
    this.board = null;
    this.root.remove();
    this.opts.onChange = null;
  }
}

export default PositionEditor;
