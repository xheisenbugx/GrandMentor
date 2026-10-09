// Guided first week: shared pieces used by the #/start page, the Home card and Settings.
// Server: GET /api/first-week, POST /api/first-week/{start,restart,dismiss,step} (docs/CONTRACT.md).
//
//   ensureFirstWeekCss()            -> Promise (injects /css/firstweek.css once)
//   dotPath(state, { compact })     -> the 7-dot progress path (Node)
//   celebrate(state, { bag })       -> toasts (+ confetti unless reduced motion) for newly finished steps/days/week
//   new FirstWeekCard(container)    -> Home card; .load(), .destroy()
//   createFirstWeekSection()        -> Settings card { el, destroy }

import { api, isAbort } from '../api.js';
import { h, icon, disposables, toast, confirmDialog } from '../ui.js';
import { t, formatDateIntl } from '../i18n.js';
import { reducedMotion } from '../settings.js';

let cssPromise = null;
export function ensureFirstWeekCss() {
  if (cssPromise) return cssPromise;
  cssPromise = new Promise((resolve) => {
    if (document.querySelector('link[data-page-css="firstweek"]')) { resolve(); return; }
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = '/css/firstweek.css';
    link.dataset.pageCss = 'firstweek';
    link.onload = () => resolve();
    link.onerror = () => resolve();
    document.head.appendChild(link);
  });
  return cssPromise;
}

/** Status of a day for the UI: done | today | open | locked. */
export function dayStatus(state, day) {
  if (day.done) return 'done';
  if (!day.unlocked) return 'locked';
  return state.current_day === day.day ? 'today' : 'open';
}

/** "Opens tomorrow" / "Opens Friday, 10 Oct". */
export function opensLabel(state, day) {
  if (!state.started || !day.unlock_on) return t('firstweek.status.locked');
  const diff = Math.round((Date.parse(`${day.unlock_on}T00:00:00Z`) - Date.parse(`${state.today}T00:00:00Z`)) / 86400000);
  if (diff === 1) return t('firstweek.status.tomorrow');
  return t('firstweek.status.opensOn', { date: formatDateIntl(new Date(`${day.unlock_on}T12:00:00Z`), { weekday: 'long', month: 'short', day: 'numeric', timeZone: 'UTC' }) });
}

/** The 7-dot path. */
export function dotPath(state, { compact = false } = {}) {
  const days = Array.isArray(state?.days) ? state.days : [];
  const done = days.filter((d) => d.done).length;
  return h('ol', { class: ['fw-path', compact && 'compact'], 'aria-label': t('firstweek.dotsLabel', { done }) },
    days.map((d) => {
      const st = state.started ? dayStatus(state, d) : (d.day === 1 ? 'today' : 'locked');
      const label = t(`firstweek.dot.${st}`, { day: d.day });
      return h('li', { class: ['fw-dot', st], title: label },
        h('span', { class: 'fw-dot-mark', 'aria-hidden': 'true' }, st === 'done' ? h('span', { html: icon('check', { size: compact ? 14 : 18 }), style: 'display:contents' }) : String(d.day)),
        compact ? h('span', { class: 'sr-only' }, label) : h('span', { class: 'fw-dot-label' }, h('span', { class: 'sr-only' }, label), h('span', { 'aria-hidden': 'true' }, t('firstweek.dayShort', { day: d.day }))));
    }));
}

/** Step title for a step id. */
export const stepTitle = (id) => t(`firstweek.steps.${id}.title`);

// ---------------------------------------------------------------------------
// Celebration
// ---------------------------------------------------------------------------
const CONFETTI_COLORS = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--cls-brilliant)', 'var(--cls-mistake)'];

/** Confetti burst (skipped under reduced motion). Returns a cancel function. */
export function confetti({ pieces = 70 } = {}) {
  let motionOff = false;
  try { motionOff = reducedMotion(); } catch { motionOff = false; }
  if (motionOff) return () => {};
  const layer = h('div', { class: 'fw-confetti', 'aria-hidden': 'true' });
  for (let i = 0; i < pieces; i++) {
    const x = Math.random() * 100;
    const drift = (Math.random() - 0.5) * 40;
    const delay = Math.random() * 400;
    const dur = 1400 + Math.random() * 1200;
    layer.appendChild(h('i', {
      style: {
        left: `${x}vw`, background: CONFETTI_COLORS[i % CONFETTI_COLORS.length],
        '--fw-drift': `${drift}vw`, '--fw-spin': `${Math.round(Math.random() * 720 - 360)}deg`,
        animationDelay: `${delay}ms`, animationDuration: `${dur}ms`,
      },
    }));
  }
  document.body.appendChild(layer);
  const timer = setTimeout(() => layer.remove(), 3200);
  return () => { clearTimeout(timer); layer.remove(); };
}

/** Toasts (and confetti) for what this response newly finished. Registers cleanup on `bag`. */
export function celebrate(state, { bag, quietSteps = false } = {}) {
  if (!state || !state.started) return;
  const days = Array.isArray(state.newly_completed_days) ? state.newly_completed_days : [];
  if (state.week_just_completed) {
    toast(t('firstweek.celebrate.week'), 'success', { duration: 6000 });
    const stop = confetti({ pieces: 120 });
    if (bag) bag.add(stop);
  } else if (days.length) {
    toast(t('firstweek.celebrate.day', { day: days[days.length - 1] }), 'success', { duration: 5000 });
    const stop = confetti();
    if (bag) bag.add(stop);
  } else if (!quietSteps && Array.isArray(state.newly_done) && state.newly_done.length) {
    toast(t('firstweek.celebrate.step', { step: stepTitle(state.newly_done[state.newly_done.length - 1]) }), 'success');
  }
}

/** Next step to do: { day, step } on the first open, unfinished day. */
export function nextStep(state) {
  const day = (state.days || []).find((d) => d.day === state.current_day);
  if (!day) return null;
  const step = day.steps.find((s) => !s.done);
  return step ? { day, step } : null;
}

// ---------------------------------------------------------------------------
// Home card
// ---------------------------------------------------------------------------
export class FirstWeekCard {
  constructor(container) {
    this.bag = disposables();
    this.ctrl = new AbortController();
    this.bag.add(() => this.ctrl.abort());
    this.container = container;
    container.hidden = true;
    this.el = h('div', { class: 'fw-home' });
    container.appendChild(this.el);
  }

  destroy() {
    this.bag.dispose();
    this.el.remove();
  }

  async load() {
    await ensureFirstWeekCss();
    let state;
    try {
      state = await api.get('/api/first-week', { signal: this.ctrl.signal });
    } catch (e) {
      if (!isAbort(e)) console.warn('[first-week]', e);
      return;
    }
    if (this.bag.disposed) return;
    this.render(state);
    celebrate(state, { bag: this.bag, quietSteps: true });
  }

  render(state) {
    if (!state || !state.eligible) { this.container.hidden = true; this.el.replaceChildren(); return; }
    const next = state.started ? nextStep(state) : null;
    const doneDays = Number(state.completed_days) || 0;
    let text;
    if (!state.started) text = t('firstweek.card.startText');
    else if (next) text = t('firstweek.card.nextUp', { step: stepTitle(next.step.id) });
    else text = t('firstweek.card.allDoneToday');
    // Nothing open right now: show the day that opens next.
    const day = next ? next.day : (state.started ? (state.days || []).find((d) => !d.unlocked) || null : null);
    const primary = !state.started
      ? h('a', { class: 'btn btn-primary', href: '#/start', html: icon('play') + `<span>${t('firstweek.card.startBtn')}</span>` })
      : h('a', { class: 'btn btn-primary', href: next ? next.step.href : '#/start', html: icon(next ? 'play' : 'calendar') + `<span>${next ? t('firstweek.card.continueBtn') : t('firstweek.card.open')}</span>` });
    const dismissBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm fw-dismiss', onClick: () => this.dismiss() }, t('firstweek.dismiss'));
    this.container.hidden = false;
    this.el.replaceChildren(h('section', { class: 'card fw-card', 'aria-labelledby': 'fw-card-title' },
      h('div', { class: 'fw-card-emoji', 'aria-hidden': 'true' }, day ? day.emoji : '🌱'),
      h('div', { class: 'fw-card-main' },
        h('div', { class: 'hub-eyebrow' }, state.started && day ? t('firstweek.dayOf', { day: day.day }) : t('firstweek.eyebrow')),
        h('h2', { class: 'fw-card-title', id: 'fw-card-title' },
          h('a', { href: '#/start' }, day ? t(`firstweek.days.${day.id}.title`) : t('firstweek.title'))),
        h('p', { class: 'muted fw-card-text' }, text),
        dotPath(state, { compact: true }),
        h('div', { class: 'fw-card-actions' },
          primary,
          state.started && next ? h('a', { class: 'btn btn-secondary', href: '#/start' }, t('firstweek.card.seePath', { done: doneDays })) : null,
          h('span', { class: 'spacer' }),
          dismissBtn))));
  }

  async dismiss() {
    try {
      const state = await api.post('/api/first-week/dismiss', { dismissed: true }, { signal: this.ctrl.signal });
      if (this.bag.disposed) return;
      this.render(state);
      toast(t('firstweek.dismissed'), 'info', { duration: 5000 });
    } catch (e) {
      if (!isAbort(e)) toast(t('firstweek.error'), 'error');
    }
  }
}

// ---------------------------------------------------------------------------
// Settings section
// ---------------------------------------------------------------------------
export function createFirstWeekSection() {
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const status = h('div', { class: 'setting-row-desc subtle text-xs' });
  const showBtn = h('button', { type: 'button', class: 'btn btn-secondary', hidden: true }, t('firstweek.settings.show'));
  const restartBtn = h('button', { type: 'button', class: 'btn btn-secondary', html: icon('refresh') + `<span>${t('firstweek.restart')}</span>` });
  const el = h('section', { class: 'card' },
    h('div', { class: 'card-header' }, h('h2', { class: 'card-title', html: icon('flag') + `<span>${t('firstweek.settings.section')}</span>` })),
    h('div', { class: 'setting-row' },
      h('div', { class: 'setting-row-text' },
        h('div', { class: 'setting-row-title' }, h('a', { href: '#/start' }, t('firstweek.title'))),
        h('div', { class: 'setting-row-desc' }, t('firstweek.settings.desc')),
        status),
      h('div', { class: 'row-sm', style: 'flex-wrap:wrap;justify-content:flex-end' }, showBtn, restartBtn)));

  const render = (st) => {
    if (!st) return;
    showBtn.hidden = !st.dismissed;
    status.textContent = !st.started
      ? t('firstweek.settings.notStarted')
      : st.week_complete ? t('firstweek.settings.finished') : t('firstweek.progress', { done: st.completed_days || 0 });
  };
  api.get('/api/first-week', { signal: ctrl.signal }).then(render).catch((e) => { if (!isAbort(e)) status.textContent = ''; });

  bag.on(showBtn, 'click', async () => {
    try {
      render(await api.post('/api/first-week/dismiss', { dismissed: false }, { signal: ctrl.signal }));
      toast(t('firstweek.settings.shown'), 'success');
    } catch (e) { if (!isAbort(e)) toast(t('firstweek.error'), 'error'); }
  });
  bag.on(restartBtn, 'click', async () => {
    const ok = await confirmDialog({ title: t('firstweek.restartConfirm.title'), message: t('firstweek.restartConfirm.message'), confirmLabel: t('firstweek.restartConfirm.button') });
    if (!ok || bag.disposed) return;
    try {
      render(await api.post('/api/first-week/restart', {}, { signal: ctrl.signal }));
      toast(t('firstweek.restarted'), 'success');
    } catch (e) { if (!isAbort(e)) toast(t('firstweek.error'), 'error'); }
  });
  return { el, destroy: () => { bag.dispose(); el.remove(); } };
}
