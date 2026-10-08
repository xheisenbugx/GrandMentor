// EvalGraph — SVG area chart of the evaluation over a game (chess.com "game review" style).
// Contract (docs/CONTRACT.md §5):
//   const g = new EvalGraph(el, { onSelect(ply) });
//   g.setData(evals /* [Score], len = moves + 1 */, classifications /* [cls] for plies 1..n */, labels? /* [san] */);
//   g.setCurrent(ply); g.destroy();
// White area grows from the bottom; black fills the top. Dots mark notable moves.
// Owns: a ResizeObserver and a handful of DOM listeners — all removed in destroy().

import { winPercent, formatScore, classificationMeta } from '../ui.js';
import { t } from '../i18n.js';

const SVG_NS = 'http://www.w3.org/2000/svg';
const DOT_CLASSES = new Set(['brilliant', 'great', 'miss', 'mistake', 'blunder']);
const MAX_POINTS = 1200; // hard bound for absurdly long inputs

export function ensureAnalysisCss() {
  if (typeof document === 'undefined' || document.getElementById('gm-analysis-css')) return;
  const link = document.createElement('link');
  link.id = 'gm-analysis-css';
  link.rel = 'stylesheet';
  link.href = '/css/analysis.css';
  document.head.appendChild(link);
}

function svg(tag, attrs = {}) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null) el.setAttribute(k, String(v));
  return el;
}

export class EvalGraph {
  constructor(el, { onSelect, height = 120, compact = false } = {}) {
    ensureAnalysisCss();
    this.el = el;
    this.onSelect = typeof onSelect === 'function' ? onSelect : null;
    this.height = height;
    this.evals = [];
    this.cls = [];
    this.labels = [];
    this.current = -1;
    this.hover = -1;
    this.width = 0;
    this._destroyed = false;

    this.root = document.createElement('div');
    this.root.className = 'evalgraph' + (compact ? ' compact' : '');
    this.root.style.setProperty('--eg-h', `${height}px`);
    this.root.tabIndex = 0;
    this.root.setAttribute('role', 'slider');
    this.root.setAttribute('aria-label', t('ui.evalgraph.label'));
    this.root.setAttribute('aria-valuemin', '0');

    this.svg = svg('svg', { class: 'evalgraph-svg', height, 'aria-hidden': 'true' });
    this.gBg = svg('rect', { class: 'eg-black', x: 0, y: 0, height });
    this.pathWhite = svg('path', { class: 'eg-white' });
    this.mid = svg('line', { class: 'eg-mid' });
    this.hoverLine = svg('line', { class: 'eg-hover', y1: 0, y2: height });
    this.curLine = svg('line', { class: 'eg-current', y1: 0, y2: height });
    this.dots = svg('g', { class: 'eg-dots' });
    this.curDot = svg('circle', { class: 'eg-current-dot', r: 4.5 });
    this.svg.append(this.gBg, this.pathWhite, this.mid, this.hoverLine, this.curLine, this.dots, this.curDot);

    this.tip = document.createElement('div');
    this.tip.className = 'evalgraph-tip';
    this.root.append(this.svg, this.tip);
    el.appendChild(this.root);

    this._onMove = (e) => this._hoverAt(e.clientX);
    this._onLeave = () => { this.hover = -1; this._renderHover(); };
    this._onClick = (e) => {
      const ply = this._plyAt(e.clientX);
      if (ply >= 0 && this.onSelect) this.onSelect(ply);
    };
    this._onKey = (e) => {
      if (!this.onSelect || !this.evals.length) return;
      const last = this.evals.length - 1;
      let p = null;
      if (e.key === 'ArrowLeft') p = Math.max(0, this.current - 1);
      else if (e.key === 'ArrowRight') p = Math.min(last, this.current + 1);
      else if (e.key === 'Home') p = 0;
      else if (e.key === 'End') p = last;
      if (p != null) { e.preventDefault(); e.stopPropagation(); this.onSelect(p); }
    };
    this.root.addEventListener('pointermove', this._onMove);
    this.root.addEventListener('pointerleave', this._onLeave);
    this.root.addEventListener('click', this._onClick);
    this.root.addEventListener('keydown', this._onKey);

    this._ro = typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver((entries) => {
        const w = Math.round(entries[0]?.contentRect?.width || 0);
        if (w && w !== this.width) { this.width = w; this._render(); }
      })
      : null;
    this._ro?.observe(this.root);
    this.width = Math.round(this.root.clientWidth || 0);
  }

  setData(evals, classifications = [], labels = []) {
    this.evals = Array.isArray(evals) ? evals.slice(0, MAX_POINTS) : [];
    this.cls = Array.isArray(classifications) ? classifications.slice(0, MAX_POINTS) : [];
    this.labels = Array.isArray(labels) ? labels.slice(0, MAX_POINTS) : [];
    this.root.setAttribute('aria-valuemax', String(Math.max(0, this.evals.length - 1)));
    this._render();
  }

  setCurrent(ply) {
    this.current = Number.isFinite(ply) ? ply : -1;
    this.root.setAttribute('aria-valuenow', String(Math.max(0, this.current)));
    // Text alternative for screen readers: "12. Nf3, +0.4, Mistake".
    const p = this.current;
    if (p >= 0 && p < this.evals.length) {
      const moveNo = Math.ceil(p / 2);
      const parts = [p === 0 ? t('ui.evalgraph.start') : `${moveNo}${p % 2 === 1 ? '.' : '…'} ${this.labels[p - 1] || ''}`.trim(), formatScore(this.evals[p])];
      const c = p > 0 ? this.cls[p - 1] : null;
      if (c) { const meta = classificationMeta(c); if (meta.label) parts.push(meta.label); }
      this.root.setAttribute('aria-valuetext', parts.join(', '));
    } else {
      this.root.removeAttribute('aria-valuetext');
    }
    this._renderCurrent();
  }

  destroy() {
    if (this._destroyed) return;
    this._destroyed = true;
    this._ro?.disconnect();
    this._ro = null;
    this.root.removeEventListener('pointermove', this._onMove);
    this.root.removeEventListener('pointerleave', this._onLeave);
    this.root.removeEventListener('click', this._onClick);
    this.root.removeEventListener('keydown', this._onKey);
    this.root.remove();
    this.onSelect = null;
    this.evals = this.cls = this.labels = [];
  }

  // -- internals -----------------------------------------------------------
  _x(i) {
    const n = Math.max(1, this.evals.length - 1);
    const pad = 4;
    return pad + (i / n) * Math.max(1, this.width - pad * 2);
  }

  _y(score) {
    const pad = 3;
    const h = this.height - pad * 2;
    return pad + (1 - winPercent(score) / 100) * h;
  }

  _plyAt(clientX) {
    if (!this.evals.length) return -1;
    const r = this.root.getBoundingClientRect();
    const n = Math.max(1, this.evals.length - 1);
    const t = (clientX - r.left - 4) / Math.max(1, r.width - 8);
    return Math.max(0, Math.min(this.evals.length - 1, Math.round(t * n)));
  }

  _hoverAt(clientX) {
    this.hover = this._plyAt(clientX);
    this._renderHover();
  }

  _render() {
    if (this._destroyed) return;
    const w = this.width || Math.round(this.root.clientWidth || 0);
    if (!w) return;
    this.width = w;
    const h = this.height;
    this.svg.setAttribute('width', String(w));
    this.svg.setAttribute('viewBox', `0 0 ${w} ${h}`);
    this.gBg.setAttribute('width', String(w));
    this.mid.setAttribute('x1', '0');
    this.mid.setAttribute('x2', String(w));
    this.mid.setAttribute('y1', String(h / 2));
    this.mid.setAttribute('y2', String(h / 2));

    const n = this.evals.length;
    if (!n) {
      this.pathWhite.setAttribute('d', `M0 ${h / 2} H${w} V${h} H0 Z`);
      this.dots.replaceChildren();
      this._renderCurrent();
      return;
    }
    let d = `M${this._x(0).toFixed(1)} ${h} `;
    for (let i = 0; i < n; i++) d += `L${this._x(i).toFixed(1)} ${this._y(this.evals[i]).toFixed(1)} `;
    d += `L${this._x(n - 1).toFixed(1)} ${h} Z`;
    this.pathWhite.setAttribute('d', d);

    const frag = document.createDocumentFragment();
    for (let ply = 1; ply < n; ply++) {
      const c = this.cls[ply - 1];
      if (!c || !DOT_CLASSES.has(c)) continue;
      frag.appendChild(svg('circle', {
        class: 'eg-dot', cx: this._x(ply).toFixed(1), cy: this._y(this.evals[ply]).toFixed(1),
        r: 3.6, fill: `var(--cls-${c})`, 'data-ply': ply,
      }));
    }
    this.dots.replaceChildren(frag);
    this._renderCurrent();
    this._renderHover();
  }

  _renderCurrent() {
    const p = this.current;
    const on = p >= 0 && p < this.evals.length && this.width > 0;
    this.curLine.style.display = on ? '' : 'none';
    this.curDot.style.display = on ? '' : 'none';
    if (!on) return;
    const x = this._x(p).toFixed(1);
    this.curLine.setAttribute('x1', x);
    this.curLine.setAttribute('x2', x);
    this.curDot.setAttribute('cx', x);
    this.curDot.setAttribute('cy', this._y(this.evals[p]).toFixed(1));
    const c = p > 0 ? this.cls[p - 1] : null;
    this.curDot.setAttribute('fill', c ? `var(--cls-${c})` : 'var(--primary)');
  }

  _renderHover() {
    const p = this.hover;
    const on = p >= 0 && p < this.evals.length && this.width > 0;
    this.hoverLine.style.display = on ? '' : 'none';
    this.tip.classList.toggle('show', on);
    if (!on) return;
    const x = this._x(p);
    this.hoverLine.setAttribute('x1', x.toFixed(1));
    this.hoverLine.setAttribute('x2', x.toFixed(1));
    const moveNo = Math.ceil(p / 2);
    const label = p === 0 ? t('ui.evalgraph.start') : `${moveNo}${p % 2 === 1 ? '.' : '…'} ${this.labels[p - 1] || ''}`;
    const c = p > 0 ? this.cls[p - 1] : null;
    const meta = c ? classificationMeta(c) : null;
    this.tip.textContent = '';
    const strong = document.createElement('strong');
    strong.textContent = label.trim();
    const score = document.createElement('span');
    score.className = 'evalgraph-tip-score';
    score.textContent = formatScore(this.evals[p]);
    this.tip.append(strong, score);
    if (meta && meta.label) {
      const b = document.createElement('span');
      b.className = 'cls-text';
      b.dataset.cls = meta.key;
      b.textContent = meta.label;
      this.tip.append(b);
    }
    const tipW = this.tip.offsetWidth || 120;
    const left = Math.max(0, Math.min(this.width - tipW, x - tipW / 2));
    this.tip.style.transform = `translateX(${left.toFixed(0)}px)`;
  }
}
