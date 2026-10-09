// GrandMentor — "Why was that a mistake?" box for the game review walkthrough.
// Contract: docs/CONTRACT.md §5 (WhyPanel) — new WhyPanel(host, { move, ply0, board, onStart, onExit }),
// .stop(restore = true), .destroy().
//
// `move` is a reviewed move (CONTRACT §3 MoveReview) with a `reason` object. The panel shows the
// grounded reason, a "Show me" control that steps through the opponent's refutation on the board
// (red arrows, animated moves) and a control for the engine's better line (green). While a line
// is shown the board is read-only; ←/→ step, Space plays/pauses, Esc returns to the game
// position (via `onExit`). Every listener and timer is released by stop()/destroy().

import { Chess } from '../../vendor/chess.js';
import { h, icon, formatSan, escapeHtml } from '../ui.js';
import { getSettings } from '../settings.js';
import { t } from '../i18n.js';

const STEP_MS = 1000;
const MAX_LINE = 12;
const KINDS = new Set([
  'allows_mate', 'hangs_piece', 'allows_fork', 'allows_pin', 'allows_skewer', 'loses_material',
  'missed_mate', 'missed_fork', 'missed_material', 'positional',
]);

let cssInjected = false;
function ensureCss() {
  if (cssInjected || typeof document === 'undefined') return;
  cssInjected = true;
  if (document.getElementById('gm-whyline-css')) return;
  const link = document.createElement('link');
  link.id = 'gm-whyline-css';
  link.rel = 'stylesheet';
  link.href = new URL('../../css/whyline.css', import.meta.url).href;
  document.head.appendChild(link);
}

/** Replay a UCI line from `fen`; stops at the first illegal move. */
function buildLine(fen, ucis, sans) {
  const out = [];
  let c;
  try { c = new Chess(fen); } catch { return out; }
  const list = Array.isArray(ucis) ? ucis.slice(0, MAX_LINE) : [];
  for (let i = 0; i < list.length; i++) {
    const u = String(list[i] || '');
    if (!/^[a-h][1-8][a-h][1-8][qrbn]?$/.test(u)) break;
    let mv = null;
    try { mv = c.move({ from: u.slice(0, 2), to: u.slice(2, 4), promotion: u[4] || undefined }); } catch { mv = null; }
    if (!mv) break;
    out.push({ uci: u, from: mv.from, to: mv.to, san: (Array.isArray(sans) && sans[i]) || mv.san, fen: c.fen() });
  }
  return out;
}

/** True when a reviewed move has a reason worth showing. */
export function hasReason(move) {
  const r = move?.reason;
  return !!(r && typeof r === 'object' && KINDS.has(r.kind) && (r.text || (r.refutation_uci || []).length || (r.better_uci || []).length));
}

export class WhyPanel {
  /**
   * @param {HTMLElement} host
   * @param {{move: object, ply0?: number, board: object, onStart?: () => void, onExit?: () => void}} opts
   *   ply0 = absolute half-move index of the game's start position (for move numbers).
   */
  constructor(host, { move, ply0 = 0, board, onStart = null, onExit = null } = {}) {
    ensureCss();
    this.host = host;
    this.move = move;
    this.board = board;
    this.onStart = onStart;
    this.onExit = onExit;
    this.ply0 = Number(ply0) || 0;
    this.mode = null;   // 'refutation' | 'better' | null
    this.idx = 0;
    this.timer = 0;
    this.playing = false;
    this.destroyed = false;
    const r = move.reason || {};
    this.reason = r;
    this.lines = {
      refutation: buildLine(move.fen_after, r.refutation_uci, r.refutation_san),
      better: buildLine(move.fen_before, r.better_uci, r.better_san),
    };
    this._onKey = this._onKey.bind(this);
    this._onClick = this._onClick.bind(this);
    this.el = this._render();
    host.appendChild(this.el);
    this.el.addEventListener('click', this._onClick);
  }

  _render() {
    const r = this.reason;
    const kind = KINDS.has(r.kind) ? r.kind : 'positional';
    const showText = r.text && String(r.text).trim() !== String(this.move.explanation || '').trim();
    const notation = getSettings().moveNotation;
    const ref = this.lines.refutation;
    const bet = this.lines.better;
    this.player = h('div', { class: 'why-player', hidden: true, role: 'group', 'aria-label': t('why.playerLabel') });
    this.status = h('div', { class: 'sr-only', 'aria-live': 'polite' });
    return h('div', { class: 'why-box', dataset: { kind } },
      h('div', { class: 'why-head' },
        h('span', { class: 'why-icon', html: icon(kind.startsWith('missed') ? 'target' : 'alert') }),
        h('strong', null, t('why.title')),
        h('span', { class: 'why-tag' }, t(`why.kind.${kind}`))),
      showText ? h('p', { class: 'why-text' }, String(r.text)) : null,
      ref.length || bet.length ? h('div', { class: 'why-actions' },
        ref.length ? h('button', { class: 'btn btn-secondary btn-sm why-show', type: 'button', dataset: { why: 'refutation' }, title: t('why.showMeTip'),
          html: icon('eye') + `<span>${escapeHtml(t('why.showMe'))}</span>` }) : null,
        bet.length ? h('button', { class: 'btn btn-ghost btn-sm why-better', type: 'button', dataset: { why: 'better' }, title: t('why.betterTip'),
          html: icon('play-circle') + `<span>${escapeHtml(t('why.better', { move: formatSan(bet[0].san, notation) }))}</span>` }) : null) : null,
      this.player, this.status);
  }

  _onClick(e) {
    const b = e.target.closest('[data-why]');
    if (!b || !this.el.contains(b)) return;
    const a = b.dataset.why;
    if (a === 'refutation' || a === 'better') this.start(a);
    else if (a === 'prev') { this._pause(); this.go(this.idx - 1); }
    else if (a === 'next') { this._pause(); this.go(this.idx + 1); }
    else if (a === 'play') this._togglePlay();
    else if (a === 'exit') this.stop(true);
    else if (a === 'step') { this._pause(); this.go(Number(b.dataset.i)); }
  }

  /** Show `which` line on the board, starting from its base position and auto-playing it. */
  start(which) {
    if (this.destroyed) return;
    const line = this.lines[which];
    if (!line?.length) return;
    if (this.mode) this.stop(false);
    this.onStart?.();
    this.mode = which;
    this.idx = 0;
    this.board.setInteractive?.(false, null);
    this.board.clearBadges?.();
    this.board.clearHighlights?.();
    window.addEventListener('keydown', this._onKey, true);
    this._renderPlayer();
    this._markPressed();
    this.el.classList.add('active');
    this.go(0, false);
    this.playing = true;
    this._schedule();
    this.player.querySelector('[data-why="play"]')?.focus({ preventScroll: true });
    // On a phone the board is usually scrolled away by now: bring it back so the line is seen.
    const boardEl = this.board.root || this.board.el;
    if (boardEl?.getBoundingClientRect) {
      const r = boardEl.getBoundingClientRect();
      if (r.top < 0 || r.bottom > window.innerHeight) boardEl.scrollIntoView({ block: 'start', behavior: 'smooth' });
    }
  }

  _markPressed() {
    this.el.querySelectorAll('.why-actions [data-why]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.why === this.mode)));
  }

  _base() {
    return this.mode === 'better' ? this.move.fen_before : this.move.fen_after;
  }

  _startAbsPly() {
    // Absolute half-move index of the line's first move.
    return this.ply0 + Number(this.move.ply || 1) - 1 + (this.mode === 'better' ? 0 : 1);
  }

  _renderPlayer() {
    const line = this.lines[this.mode] || [];
    const notation = getSettings().moveNotation;
    const key = this.mode === 'better' ? this.reason.better_key : this.reason.refutation_key;
    const start = this._startAbsPly();
    const chips = [];
    line.forEach((m, i) => {
      const abs = start + i;
      if (abs % 2 === 0 || i === 0) chips.push(h('span', { class: 'why-num' }, `${Math.floor(abs / 2) + 1}${abs % 2 === 0 ? '.' : '…'}`));
      chips.push(h('button', {
        class: 'why-move' + (i === key ? ' key' : ''), type: 'button', dataset: { why: 'step', i: i + 1 },
        'aria-label': i === key ? `${m.san} — ${t('why.keyMove')}` : m.san,
      }, formatSan(m.san, notation)));
    });
    const ctl = (act, ic, label) => h('button', { class: 'btn btn-ghost btn-icon btn-sm', type: 'button', dataset: { why: act }, 'aria-label': label, title: label, html: icon(ic) });
    this.player.replaceChildren(
      h('div', { class: 'why-player-head' },
        h('span', { class: `why-line-label ${this.mode}` }, t(this.mode === 'better' ? 'why.betterLabel' : 'why.refutationLabel'))),
      h('div', { class: 'why-moves' }, chips),
      h('div', { class: 'why-controls' },
        ctl('prev', 'chevron-left', t('why.prev')),
        ctl('play', 'pause', t('why.pause')),
        ctl('next', 'chevron-right', t('why.next')),
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary btn-sm', type: 'button', dataset: { why: 'exit' }, html: icon('undo') + `<span>${escapeHtml(t('why.back'))}</span>` })));
    this.player.hidden = false;
  }

  /** Show the position after `i` moves of the current line (0 = its start). */
  go(i, animate = true) {
    const line = this.lines[this.mode];
    if (!line) return;
    this.idx = Math.max(0, Math.min(line.length, Number(i) || 0));
    const cur = this.idx > 0 ? line[this.idx - 1] : null;
    this.board.setPosition(cur ? cur.fen : this._base(), { animate, lastMove: cur ? [cur.from, cur.to] : null });
    const next = line[this.idx];
    const color = this.mode === 'better' ? 'green' : 'red';
    if (next) this.board.setArrows([{ from: next.from, to: next.to, color }]);
    else this.board.clearArrows();
    this.player.querySelectorAll('.why-move').forEach((el) => {
      const on = Number(el.dataset.i) === this.idx;
      el.classList.toggle('on', on);
      if (on) el.setAttribute('aria-current', 'step'); else el.removeAttribute('aria-current');
    });
    const prev = this.player.querySelector('[data-why="prev"]');
    const nxt = this.player.querySelector('[data-why="next"]');
    if (prev) prev.disabled = this.idx <= 0;
    if (nxt) nxt.disabled = this.idx >= line.length;
    this.status.textContent = cur ? t('why.step', { move: cur.san, n: this.idx, total: line.length }) : t('why.stepStart');
    if (this.idx >= line.length) this._pause();
  }

  _schedule() {
    clearTimeout(this.timer);
    this.timer = 0;
    if (!this.playing || this.destroyed) return;
    this.timer = setTimeout(() => {
      this.timer = 0;
      if (!this.playing || this.destroyed || !this.mode) return;
      const line = this.lines[this.mode];
      if (this.idx >= line.length) { this._pause(); return; }
      this.go(this.idx + 1);
      this._schedule();
    }, this.idx === 0 ? 500 : STEP_MS);
  }

  _setPlayIcon() {
    const b = this.player.querySelector('[data-why="play"]');
    if (!b) return;
    const label = t(this.playing ? 'why.pause' : 'why.play');
    b.innerHTML = icon(this.playing ? 'pause' : 'play');
    b.setAttribute('aria-label', label);
    b.title = label;
  }

  _pause() {
    this.playing = false;
    clearTimeout(this.timer);
    this.timer = 0;
    this._setPlayIcon();
  }

  _togglePlay() {
    if (this.playing) { this._pause(); return; }
    const line = this.lines[this.mode] || [];
    if (this.idx >= line.length) this.go(0, false);
    this.playing = true;
    this._setPlayIcon();
    this._schedule();
  }

  _onKey(e) {
    if (!this.mode || e.altKey || e.ctrlKey || e.metaKey) return;
    const tgt = e.target;
    if (tgt && (tgt.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tgt.tagName))) return;
    let handled = true;
    if (e.key === 'ArrowLeft') { this._pause(); this.go(this.idx - 1); }
    else if (e.key === 'ArrowRight') { this._pause(); this.go(this.idx + 1); }
    else if (e.key === 'Home') { this._pause(); this.go(0); }
    else if (e.key === 'End') { this._pause(); this.go(MAX_LINE); }
    else if (e.key === 'Escape') this.stop(true);
    else if (e.key === ' ' && !(tgt && tgt.tagName === 'BUTTON')) this._togglePlay();
    else handled = false;
    if (handled) { e.preventDefault(); e.stopPropagation(); }
  }

  /** Leave the line. `restore` asks the page (onExit) to show the game position again. */
  stop(restore = true) {
    if (!this.mode) return;
    const hadFocus = this.el.contains(document.activeElement);
    this.mode = null;
    this.playing = false;
    clearTimeout(this.timer);
    this.timer = 0;
    window.removeEventListener('keydown', this._onKey, true);
    this.player.hidden = true;
    this.player.replaceChildren();
    this.status.textContent = '';
    this.el.classList.remove('active');
    this._markPressed();
    if (restore) {
      this.onExit?.();
      if (hadFocus) this.el.querySelector('.why-show, .why-better')?.focus({ preventScroll: true });
    }
  }

  destroy() {
    if (this.destroyed) return;
    this.stop(false);
    this.destroyed = true;
    this.el.removeEventListener('click', this._onClick);
    this.el.remove();
  }
}

