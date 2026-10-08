// GrandMentor — two-column numbered move list.
// Contract: docs/CONTRACT.md §5 — new MoveList(el, {onSelect(ply)}), .setMoves([{san, classification?}]),
// .setCurrent(ply /* 0 = start */), .destroy(). Keyboard ←/→ is handled by the page.
//
// Extension: setMoves(moves, { startColor: 'black', startMoveNumber: 23 }) for positions that
// don't start with white to move at move 1 (e.g. puzzles / imported FENs).

import { classificationMeta, formatSan } from '../ui.js';
import { t } from '../i18n.js';
import { getSetting, onSettingsChange } from '../settings.js';

let cssInjected = false;
function ensureCss() {
  if (cssInjected || typeof document === 'undefined') return;
  cssInjected = true;
  const linked = Array.from(document.querySelectorAll('link[rel="stylesheet"]'))
    .some((l) => /\/board\.css(\?|$)/.test(l.getAttribute('href') || ''));
  if (linked) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = new URL('../../css/board.css', import.meta.url).href;
  document.head.appendChild(link);
}

// Known classifications (others are ignored); emoji symbols render as a plain colored dot.
const KNOWN = new Set(['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder', 'forced']);
const MAX_MOVES = 2000; // hard bound; real games never get close

export class MoveList {
  /**
   * @param {HTMLElement} el
   * @param {{onSelect?: (ply:number)=>void, emptyText?: string, showClassifications?: boolean}} [opts]
   */
  constructor(el, { onSelect = null, emptyText = t('ui.movelist.empty'), showClassifications = true } = {}) {
    if (!el) throw new Error('MoveList: container element required');
    ensureCss();
    this.el = el;
    this._onSelect = onSelect;
    this._emptyText = emptyText;
    this._showCls = showClassifications;
    this._moves = [];
    this._cells = [];   // ply-1 -> button
    this._current = 0;
    this._startBlack = false;
    this._startNum = 1;
    this._destroyed = false;
    this._raf = 0;
    this._notation = safeSetting('moveNotation') || 'san';

    const root = document.createElement('div');
    root.className = 'gm-movelist';
    root.setAttribute('role', 'list');
    root.setAttribute('aria-label', t('ui.movelist.label'));
    el.appendChild(root);
    this.root = root;

    this._onClick = (e) => {
      const btn = e.target.closest?.('.gm-ml-move');
      if (!btn || !root.contains(btn) || btn.classList.contains('empty')) return;
      const ply = Number(btn.dataset.ply);
      if (!Number.isInteger(ply) || ply < 1) return;
      if (typeof this._onSelect === 'function') {
        try { this._onSelect(ply); } catch (err) { console.error('[MoveList] onSelect error', err); }
      }
    };
    root.addEventListener('click', this._onClick);

    this._offSettings = onSettingsChange((s, key) => {
      if (key !== 'moveNotation') return;
      this._notation = s.moveNotation;
      for (let i = 0; i < this._cells.length; i++) {
        const sanEl = this._cells[i].querySelector('.san');
        if (sanEl) sanEl.textContent = formatSan(this._moves[i].san, this._notation);
      }
    });

    this._renderAll();
  }

  /**
   * @param {{san:string, classification?:string|null}[]} moves
   * @param {{startColor?: 'white'|'black', startMoveNumber?: number}} [o]
   */
  setMoves(moves, { startColor, startMoveNumber } = {}) {
    if (this._destroyed) return;
    const list = (Array.isArray(moves) ? moves : []).slice(0, MAX_MOVES).map((m) => ({
      san: String((m && m.san) ?? ''),
      classification: m && m.classification ? String(m.classification) : null,
    }));
    const startBlack = startColor !== undefined ? startColor === 'black' : this._startBlack;
    const startNum = Number.isInteger(startMoveNumber) && startMoveNumber > 0 ? startMoveNumber : (startColor !== undefined ? 1 : this._startNum);

    // Fast path: same layout and the old list is a prefix (live play appends one move at a time).
    const layoutSame = startBlack === this._startBlack && startNum === this._startNum;
    let prefix = 0;
    if (layoutSame) {
      const n = Math.min(list.length, this._moves.length);
      while (prefix < n && list[prefix].san === this._moves[prefix].san
        && list[prefix].classification === this._moves[prefix].classification) prefix++;
    }
    this._startBlack = startBlack;
    this._startNum = startNum;
    if (layoutSame && prefix === this._moves.length && this._moves.length > 0) {
      const old = this._moves.length;
      this._moves = list;
      for (let i = old; i < list.length; i++) this._appendCell(i);
    } else {
      this._moves = list;
      this._renderAll();
    }
    if (this._current > this._moves.length) this._current = this._moves.length;
    this._markCurrent(true);
  }

  /** Highlight a ply (0 = start position, nothing highlighted) and scroll it into view. */
  setCurrent(ply) {
    if (this._destroyed) return;
    const p = Math.max(0, Math.min(this._moves.length, Math.floor(Number(ply) || 0)));
    if (p === this._current) return;
    this._current = p;
    this._markCurrent(false);
  }

  get current() { return this._current; }
  get length() { return this._moves.length; }

  // ---------------------------------------------------------------- render

  _renderAll() {
    this.root.textContent = '';
    this._cells = [];
    this._rows = [];
    if (!this._moves.length) {
      const empty = document.createElement('div');
      empty.className = 'gm-ml-empty';
      empty.textContent = this._emptyText;
      this.root.appendChild(empty);
      return;
    }
    const frag = document.createDocumentFragment();
    const prev = this.root;
    this.root = frag; // build off-DOM
    try {
      for (let i = 0; i < this._moves.length; i++) this._appendCell(i);
    } finally {
      this.root = prev;
    }
    this.root.appendChild(frag);
  }

  /** index (0-based) -> [rowIndex, column 0|1] */
  _slot(i) {
    const k = i + (this._startBlack ? 1 : 0);
    return [Math.floor(k / 2), k % 2];
  }

  _row(r) {
    let row = this._rows[r];
    if (row) return row;
    if (r === 0) {
      const empty = this.root.querySelector?.('.gm-ml-empty');
      if (empty) empty.remove();
    }
    row = document.createElement('div');
    row.className = 'gm-ml-row';
    row.setAttribute('role', 'listitem');
    const num = document.createElement('span');
    num.className = 'gm-ml-num';
    num.textContent = `${this._startNum + r}.`;
    const w = document.createElement('span');
    const b = document.createElement('span');
    w.className = 'gm-ml-slot';
    b.className = 'gm-ml-slot';
    row.append(num, w, b);
    if (r === 0 && this._startBlack) {
      const filler = document.createElement('button');
      filler.className = 'gm-ml-move empty';
      filler.tabIndex = -1;
      filler.setAttribute('aria-hidden', 'true');
      filler.textContent = '…';
      w.appendChild(filler);
    }
    this._rows[r] = row;
    this.root.appendChild(row);
    return row;
  }

  _appendCell(i) {
    const m = this._moves[i];
    const [r, c] = this._slot(i);
    const row = this._row(r);
    const slot = row.children[1 + c];
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'gm-ml-move';
    btn.dataset.ply = String(i + 1);
    const san = document.createElement('span');
    san.className = 'san';
    san.textContent = formatSan(m.san, this._notation);
    if (this._showCls && m.classification && KNOWN.has(m.classification)) {
      const meta = classificationMeta(m.classification);
      btn.dataset.cls = meta.key;
      const dot = document.createElement('span');
      dot.className = 'gm-ml-dot';
      // Colour is always paired with the symbol (emoji symbols render without the coloured disc).
      dot.textContent = meta.symbol || '';
      dot.setAttribute('aria-hidden', 'true');
      if (/\p{Extended_Pictographic}/u.test(meta.symbol)) dot.classList.add('emoji');
      else if (!dot.textContent) dot.classList.add('small');
      dot.title = meta.label;
      btn.title = `${m.san} — ${meta.label}`;
      btn.setAttribute('aria-label', `${m.san}, ${meta.label}`);
      btn.append(dot, san);
    } else {
      btn.appendChild(san);
    }
    slot.textContent = '';
    slot.appendChild(btn);
    this._cells[i] = btn;
  }

  _markCurrent(instant) {
    const prev = this.root.querySelector('.gm-ml-move.current');
    if (prev) { prev.classList.remove('current'); prev.removeAttribute('aria-current'); }
    const cur = this._current > 0 ? this._cells[this._current - 1] : null;
    if (cur) {
      cur.classList.add('current');
      cur.setAttribute('aria-current', 'step');
    }
    if (this._raf) cancelAnimationFrame(this._raf);
    this._raf = requestAnimationFrame(() => {
      this._raf = 0;
      if (!this._destroyed) this._scrollTo(cur, instant);
    });
  }

  /** Scroll only the list (never the page) so the current move is visible. */
  _scrollTo(cell, instant) {
    const root = this.root;
    if (root.scrollHeight <= root.clientHeight) return;
    if (!cell) {
      if (this._current === 0) root.scrollTo({ top: 0, behavior: instant ? 'auto' : 'smooth' });
      return;
    }
    const row = cell.closest('.gm-ml-row') || cell;
    const y = row.getBoundingClientRect().top - root.getBoundingClientRect().top + root.scrollTop;
    const pad = row.offsetHeight;
    let target = null;
    if (y - pad < root.scrollTop) target = Math.max(0, y - pad);
    else if (y + row.offsetHeight + pad > root.scrollTop + root.clientHeight) {
      target = y + row.offsetHeight + pad - root.clientHeight;
    }
    if (target !== null) root.scrollTo({ top: target, behavior: instant ? 'auto' : 'smooth' });
  }

  destroy() {
    if (this._destroyed) return;
    this._destroyed = true;
    if (this._raf) cancelAnimationFrame(this._raf);
    this._raf = 0;
    this.root.removeEventListener('click', this._onClick);
    if (this._offSettings) this._offSettings();
    this._offSettings = null;
    this.root.remove();
    this._cells = [];
    this._rows = [];
    this._moves = [];
    this._onSelect = null;
  }
}

function safeSetting(key) {
  try { return getSetting(key); } catch { return undefined; }
}

export default MoveList;
