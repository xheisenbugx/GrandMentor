// GrandMentor chess clock.
// Contract (docs/CONTRACT.md §5):
//   new ChessClock(el, { initialMs, incrementMs, onFlag(color) })
//   .start(color)  .press()  .pause()  .destroy()
// Extensions (backward compatible):
//   opts.slots      { white: HTMLElement, black: HTMLElement } — render each face into its own slot
//                   (e.g. the player bars). Without it, both faces render into `el`.
//   opts.initialMs  number, or { white, black } to resume a game with different remaining times.
//   opts.onLowTime(color)  called once per side when it drops under the low-time threshold.
//   opts.lowTimeMs  override the low-time threshold (default 10s, 20s for controls ≥ 10 min).
//   .getTimes() → { white, black } remaining ms   .setTimes({white, black})
//   .running → 'white' | 'black' | null           .flagged → 'white' | 'black' | null
//   .resume()  restarts the side that was running before pause().
// Colours may be given as 'white'|'black' or 'w'|'b'; callbacks always receive 'white'|'black'.
//
// Timing: one setTimeout scheduled to the next visible change (≤ 100 ms granularity under 10 s,
// otherwise aligned to the next whole second), using performance.now() so the clock never drifts.
// No timers survive pause(), flag or destroy().

import { formatClock } from '../ui.js';

const STYLE_ID = 'gm-clock-style';
const CSS = `
.gm-clock { display: inline-flex; align-items: center; gap: 6px; min-width: 96px; justify-content: flex-end;
  padding: 6px 12px; border-radius: var(--r-sm, 6px); background: var(--surface-2); color: var(--text-muted);
  font-family: var(--font-mono, ui-monospace, monospace); font-size: var(--fs-xl, 20px); font-weight: var(--fw-bold, 700);
  font-variant-numeric: tabular-nums; line-height: 1.1; letter-spacing: 0.02em; user-select: none;
  border: 1px solid var(--divider); transition: background var(--dur, 200ms) var(--ease, ease), color var(--dur, 200ms) var(--ease, ease); }
.gm-clock svg { width: 16px; height: 16px; opacity: 0; transition: opacity var(--dur, 200ms); }
.gm-clock.active { background: var(--text); color: var(--bg); border-color: transparent; box-shadow: var(--shadow-sm); }
.gm-clock.active svg { opacity: 0.8; animation: gm-clock-spin 2s linear infinite; }
.gm-clock.low { color: var(--danger); }
.gm-clock.low.active { background: var(--danger); color: #fff; }
.gm-clock.flagged { background: var(--danger-soft); color: var(--danger); }
.gm-clock-pair { display: inline-flex; flex-direction: column; gap: 6px; }
@keyframes gm-clock-spin { to { transform: rotate(360deg); } }
@media (max-width: 560px) { .gm-clock { min-width: 80px; font-size: var(--fs-lg, 17px); padding: 4px 10px; } }
@media (prefers-reduced-motion: reduce) { .gm-clock.active svg { animation: none; } }
`;

const HOURGLASS = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M9 2h6"/></svg>';

function ensureStyle() {
  if (typeof document === 'undefined' || document.getElementById(STYLE_ID)) return;
  const s = document.createElement('style');
  s.id = STYLE_ID;
  s.textContent = CSS;
  document.head.appendChild(s);
}

function norm(color) {
  if (color === 'w' || color === 'white') return 'white';
  if (color === 'b' || color === 'black') return 'black';
  return null;
}

function nonNeg(n, fallback = 0) {
  const v = Number(n);
  return Number.isFinite(v) && v >= 0 ? v : fallback;
}

export class ChessClock {
  constructor(el, { initialMs = 300000, incrementMs = 0, onFlag, onLowTime, lowTimeMs, slots } = {}) {
    ensureStyle();
    this._el = el;
    const init = typeof initialMs === 'object' && initialMs
      ? { white: nonNeg(initialMs.white, 300000), black: nonNeg(initialMs.black, 300000) }
      : { white: nonNeg(initialMs, 300000), black: nonNeg(initialMs, 300000) };
    this._remaining = init;
    this._increment = nonNeg(incrementMs);
    const base = Math.max(init.white, init.black);
    this._lowMs = lowTimeMs != null ? nonNeg(lowTimeMs) : (base >= 600000 ? 20000 : 10000);
    this._onFlag = typeof onFlag === 'function' ? onFlag : null;
    this._onLowTime = typeof onLowTime === 'function' ? onLowTime : null;
    this._lowFired = { white: init.white <= this._lowMs, black: init.black <= this._lowMs };
    this._running = null;
    this._lastRunning = null;
    this._since = 0;
    this._timer = null;
    this._flagged = null;
    this._destroyed = false;

    this._faces = {};
    this._ownWrapper = null;
    for (const c of ['white', 'black']) {
      const face = document.createElement('div');
      face.className = `gm-clock gm-clock-${c}`;
      face.setAttribute('role', 'timer');
      face.setAttribute('aria-label', `${c === 'white' ? 'White' : 'Black'} clock`);
      face.innerHTML = HOURGLASS;
      const txt = document.createElement('span');
      face.appendChild(txt);
      this._faces[c] = { face, txt, last: '' };
    }
    if (slots && slots.white && slots.black) {
      slots.white.appendChild(this._faces.white.face);
      slots.black.appendChild(this._faces.black.face);
    } else if (el) {
      this._ownWrapper = document.createElement('div');
      this._ownWrapper.className = 'gm-clock-pair';
      this._ownWrapper.append(this._faces.black.face, this._faces.white.face);
      el.appendChild(this._ownWrapper);
    }
    this._render();
  }

  get running() { return this._running; }
  get flagged() { return this._flagged; }

  /** Remaining ms for a colour, accounting for the running side. */
  _now(color) {
    const r = this._remaining[color];
    if (this._running !== color) return r;
    return Math.max(0, r - (performance.now() - this._since));
  }

  getTimes() {
    return { white: Math.round(this._now('white')), black: Math.round(this._now('black')) };
  }

  setTimes({ white, black } = {}) {
    if (this._destroyed) return;
    this._commit();
    if (white != null) this._remaining.white = nonNeg(white, this._remaining.white);
    if (black != null) this._remaining.black = nonNeg(black, this._remaining.black);
    for (const c of ['white', 'black']) this._lowFired[c] = this._remaining[c] <= this._lowMs;
    this._render();
    this._schedule();
  }

  /** Start (or switch to) the clock of `color`. */
  start(color) {
    const c = norm(color);
    if (!c || this._destroyed || this._flagged) return;
    this._commit();
    this._running = c;
    this._lastRunning = c;
    this._since = performance.now();
    this._render();
    this._schedule();
  }

  /** The running side finished its move: add its increment and start the opponent. */
  press() {
    if (this._destroyed || this._flagged || !this._running) return;
    const mover = this._running;
    this._commit();
    this._remaining[mover] += this._increment;
    if (this._remaining[mover] > this._lowMs) this._lowFired[mover] = false;
    this.start(mover === 'white' ? 'black' : 'white');
  }

  pause() {
    if (this._destroyed) return;
    this._commit();
    this._running = null;
    this._clearTimer();
    this._render();
  }

  resume() {
    if (this._lastRunning && !this._running) this.start(this._lastRunning);
  }

  destroy() {
    if (this._destroyed) return;
    this._destroyed = true;
    this._clearTimer();
    this._running = null;
    this._onFlag = null;
    this._onLowTime = null;
    if (this._ownWrapper) this._ownWrapper.remove();
    else for (const c of ['white', 'black']) this._faces[c].face.remove();
    this._faces = { white: { face: null }, black: { face: null } };
  }

  // -- internals -----------------------------------------------------------
  _commit() {
    if (!this._running) return;
    const t = performance.now();
    this._remaining[this._running] = Math.max(0, this._remaining[this._running] - (t - this._since));
    this._since = t;
  }

  _clearTimer() {
    if (this._timer) { clearTimeout(this._timer); this._timer = null; }
  }

  _schedule() {
    this._clearTimer();
    if (!this._running || this._destroyed) return;
    const left = this._now(this._running);
    // Next visible change: tenths under 10 s, otherwise the next whole second.
    let wait = left < 10000 ? (left % 100) || 100 : (left % 1000) || 1000;
    wait = Math.max(16, Math.min(1000, wait + 1));
    this._timer = setTimeout(() => this._tick(), wait);
  }

  _tick() {
    this._timer = null;
    if (this._destroyed || !this._running) return;
    const c = this._running;
    const left = this._now(c);
    if (!this._lowFired[c] && left <= this._lowMs) {
      this._lowFired[c] = true;
      if (this._onLowTime) { try { this._onLowTime(c); } catch (e) { console.error(e); } }
    }
    if (left <= 0) {
      this._commit();
      this._remaining[c] = 0;
      this._running = null;
      this._flagged = c;
      this._render();
      const cb = this._onFlag;
      if (cb) { try { cb(c); } catch (e) { console.error(e); } }
      return;
    }
    this._render();
    this._schedule();
  }

  _render() {
    if (this._destroyed) return;
    for (const c of ['white', 'black']) {
      const f = this._faces[c];
      if (!f.face) continue;
      const ms = this._now(c);
      const text = formatClock(ms);
      if (text !== f.last) { f.txt.textContent = text; f.last = text; }
      f.face.classList.toggle('active', this._running === c);
      f.face.classList.toggle('low', ms <= this._lowMs);
      f.face.classList.toggle('flagged', this._flagged === c);
    }
  }
}
