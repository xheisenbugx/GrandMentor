// Guided first week (#/start): a 7-day path for new players. Each day has 2–4 steps that link
// into existing features (lessons, drills, endgames, coach/beginner bots, game review, puzzles,
// daily goal). Steps are ticked off automatically from real activity (server-side detection) and
// can also be marked done by hand. Opening the page starts the path if it has not started yet.
// Server: GET /api/first-week, POST /api/first-week/{start,restart,dismiss,step}.

import { api, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, toast, confirmDialog, emptyState, skeleton } from '../ui.js';
import { t } from '../i18n.js';
import { ensureFirstWeekCss, dotPath, dayStatus, opensLabel, celebrate } from '../components/firstweek.js';

export const title = () => t('firstweek.title');

const KIND_ICON = {
  lesson: 'learn', drill: 'bolt', endgame: 'endgames', game: 'play', review: 'sparkles', puzzles: 'puzzle', goal: 'target',
};

export async function mount(root) {
  await ensureFirstWeekCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  let state = null;
  let busy = false;

  const heroEl = h('section', { class: 'card fw-hero' }, skeleton('text', 3));
  const daysEl = h('ol', { class: 'fw-days' });
  const footEl = h('div', { class: 'fw-foot' });
  const page = h('div', { class: 'page fw-page' },
    pageHeader({ title: t('firstweek.title'), subtitle: t('firstweek.subtitle'), icon: 'flag' }),
    heroEl, daysEl, footEl);
  root.appendChild(page);

  load().catch((e) => { if (!isAbort(e)) showError(); });
  return bag.dispose;

  async function load() {
    let st = await api.get('/api/first-week', { signal });
    if (!st.started) st = await api.post('/api/first-week/start', {}, { signal });
    if (signal.aborted) return;
    apply(st);
  }

  function apply(st) {
    state = st;
    render();
    celebrate(st, { bag });
  }

  function showError() {
    if (signal.aborted) return;
    heroEl.replaceChildren(emptyState({ icon: 'wifi-off', title: t('firstweek.offlineTitle'), text: t('firstweek.offline') }));
  }

  async function post(path, body) {
    if (busy) return;
    busy = true;
    try {
      const st = await api.post(path, body, { signal });
      if (!signal.aborted) apply(st);
    } catch (e) {
      if (!isAbort(e)) toast(t('firstweek.error'), 'error');
    } finally {
      busy = false;
    }
  }

  // ---------------------------------------------------------------------------
  function render() {
    renderHero();
    daysEl.replaceChildren(...state.days.map(renderDay));
    renderFoot();
  }

  function renderHero() {
    const done = Number(state.completed_days) || 0;
    const complete = !!state.week_complete;
    heroEl.classList.toggle('complete', complete);
    heroEl.replaceChildren(...[
      h('div', { class: 'fw-hero-top' },
        h('div', { class: 'fw-hero-emoji', 'aria-hidden': 'true' }, complete ? '🏆' : '🌱'),
        h('div', { class: 'stack-sm', style: 'min-width:0' },
          h('div', { class: 'hub-eyebrow' }, t('firstweek.progress', { done })),
          h('h2', { class: 'fw-hero-title' }, complete ? t('firstweek.complete.title') : t('firstweek.heroTitle')),
          h('p', { class: 'muted' }, complete ? t('firstweek.complete.text') : t('firstweek.heroText')))),
      dotPath(state),
      complete
        ? h('div', { class: 'fw-hero-actions' },
          h('a', { class: 'btn btn-primary', href: '#/play', html: icon('play') + `<span>${t('firstweek.complete.play')}</span>` }),
          h('a', { class: 'btn btn-secondary', href: '#/puzzles', html: icon('puzzle') + `<span>${t('firstweek.complete.puzzles')}</span>` }),
          h('a', { class: 'btn btn-secondary', href: '#/learn', html: icon('learn') + `<span>${t('firstweek.complete.learn')}</span>` }))
        : null,
    ].filter(Boolean));
  }

  function renderDay(day) {
    const st = dayStatus(state, day);
    const doneSteps = day.steps.filter((s) => s.done).length;
    const badge = st === 'done'
      ? h('span', { class: 'badge badge-success', html: icon('check', { size: 14 }) + `<span>${t('firstweek.status.done')}</span>` })
      : st === 'today' ? h('span', { class: 'badge badge-info' }, t('firstweek.status.today'))
        : st === 'open' ? h('span', { class: 'badge' }, t('firstweek.status.open'))
          : h('span', { class: 'badge fw-badge-locked', html: icon('lock', { size: 14 }) + `<span>${opensLabel(state, day)}</span>` });
    return h('li', { class: ['card fw-day', st], id: `fw-day-${day.day}` },
      h('div', { class: 'fw-day-head' },
        h('div', { class: 'fw-day-emoji', 'aria-hidden': 'true' }, day.emoji),
        h('div', { class: 'fw-day-titles' },
          h('div', { class: 'hub-eyebrow' }, t('firstweek.dayOf', { day: day.day })),
          h('h3', { class: 'fw-day-title' }, t(`firstweek.days.${day.id}.title`)),
          h('p', { class: 'muted text-sm fw-day-desc' }, t(`firstweek.days.${day.id}.desc`))),
        h('div', { class: 'fw-day-meta' }, badge,
          day.unlocked ? h('span', { class: 'subtle text-xs tabular' }, t('firstweek.stepsProgress', { done: doneSteps, total: day.steps.length })) : null)),
      h('ul', { class: 'fw-steps' }, day.steps.map((s) => renderStep(day, s))),
      !day.unlocked && day.day === (state.unlocked_days || 0) + 1 ? h('p', { class: 'subtle text-sm fw-day-note' }, t('firstweek.lockedHint')) : null);
  }

  function renderStep(day, s) {
    const stepTitle = t(`firstweek.steps.${s.id}.title`);
    const sub = s.progress && !s.done && day.unlocked
      ? t('firstweek.puzzleProgress', { have: s.progress.have, need: s.progress.need })
      : t(`firstweek.steps.${s.id}.desc`);
    const check = h('span', { class: 'fw-check', 'aria-hidden': 'true', html: s.done ? icon('check', { size: 16 }) : '' });
    const main = h('div', { class: 'fw-step-main' },
      h('div', { class: 'fw-step-title' }, stepTitle, s.done ? h('span', { class: 'sr-only' }, ` ${t('firstweek.doneSr')}`) : null),
      h('div', { class: 'fw-step-sub' }, sub));
    const kindIcon = h('span', { class: 'fw-step-icon', 'aria-hidden': 'true', html: icon(KIND_ICON[s.kind] || 'star', { size: 18 }) });
    if (!day.unlocked) {
      return h('li', { class: 'fw-step locked' }, check, kindIcon, main);
    }
    const actions = h('div', { class: 'fw-step-actions' });
    if (!s.done) {
      actions.append(
        h('a', { class: 'btn btn-primary btn-sm', href: s.href, 'aria-label': `${t('firstweek.go')}: ${stepTitle}` }, t('firstweek.go')),
        h('button', {
          type: 'button', class: 'btn btn-ghost btn-sm', title: t('firstweek.markDoneHint'), 'aria-label': `${t('firstweek.markDone')}: ${stepTitle}`,
          onClick: () => post('/api/first-week/step', { step_id: s.id, done: true }),
        }, t('firstweek.markDone')));
    } else if (s.manual) {
      actions.append(h('button', {
        type: 'button', class: 'btn btn-ghost btn-sm', 'aria-label': `${t('firstweek.undo')}: ${stepTitle}`,
        onClick: () => post('/api/first-week/step', { step_id: s.id, done: false }),
      }, t('firstweek.undo')));
    } else {
      actions.append(h('a', { class: 'btn btn-ghost btn-sm', href: s.href, 'aria-label': `${t('firstweek.again')}: ${stepTitle}` }, t('firstweek.again')));
    }
    return h('li', { class: ['fw-step', s.done && 'done'] }, check, kindIcon, main, actions);
  }

  function renderFoot() {
    const restart = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('refresh', { size: 16 }) + `<span>${t('firstweek.restart')}</span>` });
    restart.addEventListener('click', async () => {
      const ok = await confirmDialog({ title: t('firstweek.restartConfirm.title'), message: t('firstweek.restartConfirm.message'), confirmLabel: t('firstweek.restartConfirm.button') });
      if (!ok || signal.aborted) return;
      await post('/api/first-week/restart', {});
      if (!signal.aborted) toast(t('firstweek.restarted'), 'success');
    });
    const dismiss = state.dismissed
      ? h('button', { type: 'button', class: 'btn btn-ghost btn-sm', onClick: () => post('/api/first-week/dismiss', { dismissed: false }) }, t('firstweek.settings.show'))
      : h('button', {
        type: 'button', class: 'btn btn-ghost btn-sm',
        onClick: async () => {
          await post('/api/first-week/dismiss', { dismissed: true });
          if (signal.aborted || !state?.dismissed) return;
          toast(t('firstweek.dismissed'), 'info', { duration: 5000 });
          location.hash = '#/';
        },
      }, t('firstweek.dismiss'));
    footEl.replaceChildren(
      h('p', { class: 'subtle text-sm' }, state.dismissed ? t('firstweek.hiddenNotice') : t('firstweek.catchUp')),
      h('div', { class: 'row-sm', style: 'flex-wrap:wrap' }, dismiss, restart));
  }
}
