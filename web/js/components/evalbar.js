// GrandMentor — vertical evaluation bar (who is winning?).
// Contract: docs/CONTRACT.md §5 — new EvalBar(el, {orientation}), .set(score), .setOrientation(c), .destroy().
// Score is white-POV: {cp: 35} | {mate: 3} | {mate: -2} | null (unknown).

import { winPercent } from '../ui.js';
import { t } from '../i18n.js';

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

/** Short label for the bar: "1.3", "10", "M3", "1-0". Always unsigned (side is shown by placement). */
function shortLabel(score) {
  if (!score || typeof score !== 'object') return '';
  if (typeof score.mate === 'number') {
    if (score.mate === 0) return '#';
    return `M${Math.abs(score.mate)}`;
  }
  if (typeof score.cp === 'number' && Number.isFinite(score.cp)) {
    const p = Math.abs(score.cp) / 100;
    if (p >= 10) return p >= 100 ? '99+' : p.toFixed(0);
    return p.toFixed(1);
  }
  return '';
}

function describe(score) {
  if (!score || typeof score !== 'object') return t('ui.evalbar.unavailable');
  if (typeof score.mate === 'number') {
    if (score.mate === 0) return t('ui.evalbar.checkmate');
    const side = score.mate > 0 ? 'white' : 'black';
    return t(`ui.evalbar.${side}.mate`, { count: Math.abs(score.mate) });
  }
  const cp = Number(score.cp) || 0;
  const p = Math.abs(cp) / 100;
  const side = cp > 0 ? 'white' : 'black';
  if (p < 0.3) return t('ui.evalbar.equal', { eval: `${cp >= 0 ? '+' : '-'}${p.toFixed(1)}` });
  const how = p < 1 ? 'slightlyBetter' : p < 2.5 ? 'better' : 'winning';
  return t(`ui.evalbar.${side}.${how}`, { eval: `${cp > 0 ? '+' : '-'}${p.toFixed(1)}` });
}

export class EvalBar {
  /**
   * @param {HTMLElement} el container (e.g. `.evalbar-slot`)
   * @param {{orientation?: 'white'|'black'}} [opts]
   */
  constructor(el, { orientation = 'white' } = {}) {
    if (!el) throw new Error('EvalBar: container element required');
    ensureCss();
    this.el = el;
    this._orientation = orientation === 'black' ? 'black' : 'white';
    this._score = null;
    this._destroyed = false;

    const root = document.createElement('div');
    root.className = 'gm-evalbar empty';
    root.setAttribute('role', 'meter');
    root.setAttribute('aria-valuemin', '0');
    root.setAttribute('aria-valuemax', '100');
    root.setAttribute('aria-label', t('ui.evalbar.label'));
    const fill = document.createElement('div');
    fill.className = 'gm-evalbar-fill';
    const mid = document.createElement('div');
    mid.className = 'gm-evalbar-mid';
    const label = document.createElement('div');
    label.className = 'gm-evalbar-label';
    root.append(fill, mid, label);
    el.appendChild(root);
    this.root = root;
    this._fill = fill;
    this._label = label;
    this._render();
  }

  /**
   * Update with a white-POV score; null/undefined shows an even bar with no label.
   * A finished checkmate ({mate: 0}) carries no sign, so pass `{mated: 'white'|'black'}`
   * (or a FEN via `{fen}`: the side to move is the mated side); otherwise the bar keeps
   * the side that was already ahead.
   */
  set(score, opts = {}) {
    if (this._destroyed) return;
    if (score && typeof score === 'object' && (typeof score.cp === 'number' || typeof score.mate === 'number')) {
      const prevWhiteAhead = this._whiteAhead();
      this._score = typeof score.mate === 'number' ? { mate: score.mate } : { cp: score.cp };
      if (this._score.mate === 0) {
        let mated = opts && (opts.mated === 'white' || opts.mated === 'black') ? opts.mated : null;
        if (!mated && opts && typeof opts.fen === 'string') {
          const stm = opts.fen.split(' ')[1];
          if (stm === 'w') mated = 'white'; else if (stm === 'b') mated = 'black';
        }
        this._mateWinnerWhite = mated ? mated === 'black' : prevWhiteAhead;
      }
    } else {
      this._score = null;
    }
    this._render();
  }

  get score() { return this._score; }

  _whiteAhead() {
    const s = this._score;
    if (!s) return true;
    if (typeof s.mate === 'number') return s.mate === 0 ? this._mateWinnerWhite !== false : s.mate > 0;
    return s.cp >= 0;
  }

  setOrientation(color) {
    if (this._destroyed) return;
    this._orientation = color === 'black' ? 'black' : 'white';
    this._render();
  }

  get orientation() { return this._orientation; }

  _render() {
    const s = this._score;
    let pct; // white share, 0..100
    if (!s) pct = 50;
    else if (typeof s.mate === 'number') {
      // mate 0 = finished checkmate; the winner was resolved in set().
      pct = s.mate > 0 ? 100 : s.mate < 0 ? 0 : (this._whiteAhead() ? 100 : 0);
    } else {
      pct = winPercent(s);
    }
    // Keep a sliver of each color visible unless it is a forced mate.
    const isMate = s && typeof s.mate === 'number';
    if (!isMate) pct = Math.max(4, Math.min(96, pct));

    // White fill is an inset:0 layer translated away by the black share.
    const offset = 100 - pct;
    const whiteAtBottom = this._orientation === 'white';
    this._fill.style.transform = `translateY(${whiteAtBottom ? offset : -offset}%)`;

    const whiteBetter = this._whiteAhead();
    const labelAtBottom = whiteBetter === whiteAtBottom;
    this._label.textContent = shortLabel(s);
    this._label.className = `gm-evalbar-label ${labelAtBottom ? 'bottom' : 'top'} ${whiteBetter ? 'white' : 'black'}`;
    this.root.classList.toggle('empty', !s);
    this.root.setAttribute('aria-valuenow', String(Math.round(pct)));
    const text = describe(s);
    this.root.setAttribute('aria-valuetext', text);
    this.root.title = text;
  }

  destroy() {
    if (this._destroyed) return;
    this._destroyed = true;
    this.root.remove();
    this._fill = null;
    this._label = null;
  }
}

export default EvalBar;
