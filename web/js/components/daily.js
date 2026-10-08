// Daily plan, goal ring, streak strip and 12-week activity heatmap (shown on Home).
//
//   const panel = new DailyPanel(container, { getContext });
//   panel.load(ctx);     // ctx = { bots, courses, progress, games, dailyPuzzle } already fetched by Home
//   panel.destroy();
//
// Server: GET /api/daily (goal, progress, streak, last7), PUT /api/daily/goal, GET /api/activity?days=84.
// Optional (skipped when missing / 404): /api/mistakes/summary, /api/repertoire/summary,
// /api/adaptive/estimate, /api/endgames.
// Days are UTC calendar days, matching the server.

import { api, isAbort } from '../api.js';
import { h, icon, disposables, modal, toast } from '../ui.js';
import { t, formatDateIntl, formatNumber } from '../i18n.js';

const PLAN_KEY = 'grandmentor.daily.plan.v1';
const CELEBRATED_KEY = 'grandmentor.daily.celebrated.v1';
const HEATMAP_KEY = 'grandmentor.daily.heatmap.v1';
const HEATMAP_WEEKS = 12;
const PLAN_MAX_MINUTES = 15;
const PLAN_MIN_MINUTES = 12;
const PLAN_MIN_TASKS = 3;
const PLAN_MAX_TASKS = 5;

/** Goal presets shown in the picker. */
const PRESETS = [
  { id: 'casual', kind: 'minutes', target: 5 },
  { id: 'regular', kind: 'minutes', target: 10 },
  { id: 'serious', kind: 'minutes', target: 20 },
];

/** Task catalog: activity kinds that mark it done today, estimated minutes and icon. */
const TASKS = {
  puzzle: { kinds: ['puzzle'], minutes: 2, icon: 'puzzle', accent: 'var(--info)' },
  mistakes: { kinds: ['mistake_review'], minutes: 4, icon: 'target', accent: 'var(--cls-mistake)' },
  repertoire: { kinds: ['repertoire_review'], minutes: 4, icon: 'book', accent: 'var(--cls-book)' },
  lesson: { kinds: ['lesson'], minutes: 5, icon: 'learn', accent: 'var(--gold)' },
  drill: { kinds: ['drill'], minutes: 3, icon: 'bolt', accent: 'var(--cls-brilliant)' },
  endgame: { kinds: ['endgame'], minutes: 4, icon: 'king', accent: 'var(--cls-great)' },
  classic: { kinds: ['classic'], minutes: 5, icon: 'trophy', accent: 'var(--cls-book)' },
  game: { kinds: ['game', 'local_game'], minutes: 8, icon: 'play', accent: 'var(--primary)' },
};
const OPTIONAL = ['drill', 'endgame', 'classic', 'game'];

// ---------------------------------------------------------------------------
// Pure helpers (exported for reuse/testing)
// ---------------------------------------------------------------------------

/** Today as a UTC `YYYY-MM-DD` string. */
export function utcToday() { return new Date().toISOString().slice(0, 10); }

/** Day number (days since epoch) of a `YYYY-MM-DD` UTC day. */
export function dayNum(day) { return Math.floor(Date.parse(`${day}T00:00:00Z`) / 86400000); }
/** `YYYY-MM-DD` of a day number. */
export function dayStr(n) { return new Date(n * 86400000).toISOString().slice(0, 10); }

/** Small deterministic PRNG seeded by a string (FNV-1a + mulberry32). */
export function seededRandom(seed) {
  let a = 2166136261;
  for (let i = 0; i < seed.length; i++) { a ^= seed.charCodeAt(i); a = Math.imul(a, 16777619); }
  return () => {
    a |= 0; a = (a + 0x6D2B79F5) | 0;
    let x = Math.imul(a ^ (a >>> 15), 1 | a);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Choose today's tasks. Deterministic for the same inputs and date.
 * @param {{date:string, mistakesDue:number, repertoireDue:number, hasLesson:boolean, hasEndgame:boolean, hasGame:boolean}} o
 * @returns {string[]} task ids
 */
export function choosePlan({ date, mistakesDue = 0, repertoireDue = 0, hasLesson = false, hasEndgame = true, hasGame = true }) {
  const rnd = seededRandom(`plan:${date}`);
  const required = ['puzzle'];
  if (mistakesDue > 0) required.push('mistakes');
  if (repertoireDue > 0) required.push('repertoire');
  if (hasLesson) required.push('lesson');
  const pool = OPTIONAL.filter((id) => (id !== 'endgame' || hasEndgame) && (id !== 'game' || hasGame));
  for (let i = pool.length - 1; i > 0; i--) { const j = Math.floor(rnd() * (i + 1)); [pool[i], pool[j]] = [pool[j], pool[i]]; }

  const plan = [];
  let minutes = 0;
  const add = (id) => { plan.push(id); minutes += TASKS[id].minutes; };
  for (const id of required) {
    if (plan.length >= PLAN_MAX_TASKS) break;
    if (plan.length && minutes + TASKS[id].minutes > PLAN_MAX_MINUTES) continue;
    add(id);
  }
  for (const id of pool) {
    if (plan.length >= PLAN_MAX_TASKS) break;
    if (plan.length >= PLAN_MIN_TASKS && minutes >= PLAN_MIN_MINUTES) break;
    if (minutes + TASKS[id].minutes <= PLAN_MAX_MINUTES) add(id);
  }
  // Never fewer than 3 tasks: take the shortest leftovers.
  const rest = pool.filter((id) => !plan.includes(id)).sort((a, b) => TASKS[a].minutes - TASKS[b].minutes);
  while (plan.length < PLAN_MIN_TASKS && rest.length) add(rest.shift());
  return plan;
}

/** Whether a task counts as done from today's activity counts. */
export function taskDone(id, counts) {
  const spec = TASKS[id];
  return !!spec && spec.kinds.some((k) => Number(counts?.[k]) > 0);
}

/** Heat level 0..4 for a day total. */
export function heatLevel(total) {
  if (!total) return 0;
  if (total < 3) return 1;
  if (total < 6) return 2;
  if (total < 10) return 3;
  return 4;
}

function readJson(key) {
  try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) : null; } catch { return null; }
}
function writeJson(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* private mode / quota */ }
}

/** Load web/css/daily.css once. */
let cssPromise = null;
export function ensureDailyCss() {
  if (cssPromise) return cssPromise;
  const existing = document.querySelector('link[data-daily-css]');
  if (existing && existing.sheet) { cssPromise = Promise.resolve(); return cssPromise; }
  cssPromise = new Promise((resolve) => {
    const link = existing || document.createElement('link');
    let timer = 0;
    const done = () => { clearTimeout(timer); link.removeEventListener('load', done); link.removeEventListener('error', done); resolve(); };
    link.addEventListener('load', done);
    link.addEventListener('error', done);
    timer = setTimeout(done, 1500);
    if (!existing) {
      link.rel = 'stylesheet';
      link.href = '/css/daily.css';
      link.dataset.dailyCss = '1';
      document.head.appendChild(link);
    }
  });
  return cssPromise;
}

const prefersReducedMotion = () => {
  try { return window.matchMedia('(prefers-reduced-motion: reduce)').matches; } catch { return false; }
};

const utcDate = (day) => new Date(`${day}T12:00:00Z`);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export class DailyPanel {
  constructor(container) {
    this.bag = disposables();
    this.ctrl = new AbortController();
    this.bag.add(() => this.ctrl.abort());
    this.summary = null;
    this.ctx = {};
    this.extra = { mistakesDue: 0, repertoireDue: 0, estimate: null, endgames: [] };
    this.plan = null;

    this.planEl = h('div', { class: 'dp-plan' }, Array.from({ length: 4 }, () => h('div', { class: 'skeleton dp-task-skel' })));
    this.goalEl = h('div', { class: 'dp-goal' }, h('div', { class: 'skeleton dp-ring-skel' }));
    this.streakEl = h('div', { class: 'dp-streak' }, h('div', { class: 'skeleton skeleton-text' }));
    this.heatBody = h('div', { class: 'dp-heat-body' });
    const open = readJson(HEATMAP_KEY) === true;
    this.heatEl = h('details', { class: 'card dp-heat', open: open || null },
      h('summary', { class: 'dp-heat-summary' },
        h('span', { class: 'dp-heat-title', html: icon('calendar') + `<span>${t('daily.heatmap.title')}</span>` }),
        h('span', { class: 'dp-heat-meta subtle text-sm' }),
        h('span', { class: 'dp-heat-chev', 'aria-hidden': 'true', html: icon('chevron-down') })),
      this.heatBody);
    this.bag.on(this.heatEl, 'toggle', () => {
      writeJson(HEATMAP_KEY, this.heatEl.open);
      if (this.heatEl.open && !this.heatLoaded && this.summary) this.loadHeatmap();
    });

    this.el = h('section', { class: 'dp', 'aria-labelledby': 'dp-title' },
      h('div', { class: 'card dp-card' },
        h('div', { class: 'dp-main' },
          h('div', { class: 'dp-head' },
            h('div', null,
              h('div', { class: 'hub-eyebrow' }, t('daily.eyebrow')),
              h('h2', { class: 'dp-title', id: 'dp-title' }, t('daily.title'))),
            h('div', { class: 'dp-head-meta' })),
          this.planEl),
        h('aside', { class: 'dp-side' }, this.goalEl, this.streakEl)),
      this.heatEl);
    container.appendChild(this.el);
  }

  destroy() {
    this.bag.dispose();
    this.el.remove();
  }

  get signal() { return this.ctrl.signal; }

  /** ctx: { bots, courses, progress, games, dailyPuzzle, nextLesson } from Home. */
  async load(ctx) {
    this.ctx = ctx || {};
    const opt = { signal: this.signal };
    const settle = (p) => p.then((v) => ({ ok: true, v }), (e) => ({ ok: false, e }));
    const [dailyR, mistakesR, repR, estR, egR] = await Promise.all([
      settle(api.get('/api/daily', opt)),
      settle(api.get('/api/mistakes/summary', opt)),
      settle(api.get('/api/repertoire/summary', opt)),
      settle(api.get('/api/adaptive/estimate', opt)),
      settle(api.get('/api/endgames', opt)),
    ]);
    if (this.signal.aborted) return;
    const num = (r, k) => (r.ok && r.v && Number.isFinite(Number(r.v[k])) ? Math.max(0, Number(r.v[k])) : 0);
    this.extra = {
      mistakesDue: num(mistakesR, 'due'),
      repertoireDue: num(repR, 'due'),
      estimate: estR.ok && estR.v && Number.isFinite(Number(estR.v.rating)) && estR.v.rating !== null ? Number(estR.v.rating) : null,
      endgames: egR.ok && Array.isArray(egR.v) ? egR.v : [],
    };
    if (!dailyR.ok) {
      if (!isAbort(dailyR.e)) this.renderOffline();
      return;
    }
    this.summary = dailyR.v;
    this.plan = this.resolvePlan();
    this.render();
    if (this.heatEl.open && !this.heatLoaded) this.loadHeatmap();
  }

  renderOffline() {
    this.planEl.replaceChildren(h('p', { class: 'muted' }, t('daily.offline')));
    this.goalEl.replaceChildren();
    this.streakEl.replaceChildren();
  }

  // ---- plan ------------------------------------------------------------------

  /** Today's plan: reuse the stored one for this date (stable across reloads), else compose it. */
  resolvePlan() {
    const date = this.summary?.date || utcToday();
    const stored = readJson(PLAN_KEY);
    if (stored && stored.date === date && Array.isArray(stored.tasks) && stored.tasks.length
      && stored.tasks.every((x) => x && TASKS[x.id])) {
      return stored.tasks;
    }
    const { bots = [], games = [] } = this.ctx;
    const lesson = this.ctx.nextLesson || null;
    const rnd = seededRandom(`pick:${date}`);
    const endgames = this.extra.endgames;
    const beginnerEg = endgames.filter((e) => e.level === 'beginner');
    const egPool = beginnerEg.length ? beginnerEg : endgames;
    const endgame = egPool.length ? egPool[Math.floor(rnd() * egPool.length)] : null;
    const bot = pickBot(bots, this.extra.estimate, games);
    const ids = choosePlan({
      date,
      mistakesDue: this.extra.mistakesDue,
      repertoireDue: this.extra.repertoireDue,
      hasLesson: !!lesson,
      hasEndgame: true,
      hasGame: true,
    });
    const tasks = ids.map((id) => {
      const task = { id };
      if (id === 'mistakes') task.count = this.extra.mistakesDue;
      if (id === 'repertoire') task.count = this.extra.repertoireDue;
      if (id === 'lesson' && lesson) { task.courseId = lesson.course.id; task.lessonId = lesson.lesson.id; }
      if (id === 'endgame' && endgame) task.endgameId = endgame.id;
      if (id === 'game' && bot) task.botId = bot.id;
      return task;
    });
    writeJson(PLAN_KEY, { date, tasks });
    return tasks;
  }

  describe(task) {
    const { bots = [], courses = [], dailyPuzzle } = this.ctx;
    const spec = TASKS[task.id];
    const base = { minutes: spec.minutes, icon: spec.icon, accent: spec.accent };
    switch (task.id) {
      case 'puzzle':
        return { ...base, href: dailyPuzzle ? '#/puzzles/daily' : '#/puzzles',
          title: dailyPuzzle ? t('daily.task.puzzle') : t('daily.task.anyPuzzle'),
          sub: dailyPuzzle?.rating ? `${t('common.rated', { rating: dailyPuzzle.rating })} · ${t('daily.task.puzzleSub')}` : t('daily.task.puzzleSub') };
      case 'mistakes': {
        const n = this.extra.mistakesDue || task.count || 0;
        return { ...base, href: '#/puzzles/mistakes', title: t('daily.task.mistakes', { count: n }), sub: t('daily.task.mistakesSub') };
      }
      case 'repertoire': {
        const n = this.extra.repertoireDue || task.count || 0;
        return { ...base, href: '#/repertoire', title: t('daily.task.repertoire', { count: n }), sub: t('daily.task.repertoireSub') };
      }
      case 'lesson': {
        const course = courses.find((c) => c.id === task.courseId);
        const lesson = course?.lessons?.find((l) => l.id === task.lessonId);
        if (!course || !lesson) return { ...base, href: '#/learn', title: t('daily.task.anyLesson'), sub: t('daily.task.lessonSub') };
        return { ...base, href: `#/learn/${encodeURIComponent(course.id)}/${encodeURIComponent(lesson.id)}`,
          title: t('daily.task.lesson', { title: lesson.title }), sub: `${course.icon || '📘'} ${course.title}` };
      }
      case 'drill':
        return { ...base, href: '#/drills', title: t('daily.task.drill'), sub: t('daily.task.drillSub') };
      case 'endgame': {
        const eg = this.extra.endgames.find((e) => e.id === task.endgameId);
        return { ...base, href: eg ? `#/endgames/${encodeURIComponent(eg.id)}` : '#/endgames',
          title: eg ? t('daily.task.endgame', { title: eg.title }) : t('daily.task.anyEndgame'), sub: t('daily.task.endgameSub') };
      }
      case 'classic':
        return { ...base, href: '#/classics', title: t('daily.task.classic'), sub: t('daily.task.classicSub') };
      case 'game': {
        const bot = bots.find((b) => b.id === task.botId);
        if (!bot) return { ...base, href: '#/play', title: t('daily.task.anyGame'), sub: t('daily.task.gameSub') };
        return { ...base, href: `#/play/${encodeURIComponent(bot.id)}`, avatar: bot.avatar || '🤖',
          title: t('daily.task.game', { name: bot.name }),
          sub: this.extra.estimate !== null ? t('daily.task.gameNear', { elo: bot.elo }) : t('daily.task.gameElo', { elo: bot.elo }) };
      }
      default:
        return null;
    }
  }

  render() {
    const s = this.summary;
    const counts = s.today || {};
    const items = this.plan.map((task) => ({ task, d: this.describe(task), done: taskDone(task.id, counts) })).filter((x) => x.d);
    const doneCount = items.filter((x) => x.done).length;
    const totalMin = items.reduce((a, x) => a + x.d.minutes, 0);
    const allDone = items.length > 0 && doneCount === items.length;

    const head = this.el.querySelector('.dp-head-meta');
    head.replaceChildren(
      h('span', { class: ['badge', allDone ? 'badge-primary' : null], html: icon(allDone ? 'check-circle' : 'clock', { size: 14 }) + `<span>${allDone ? t('daily.allDone') : t('daily.aboutMinutes', { count: totalMin })}</span>` }));

    const list = h('ol', { class: 'dp-tasks' }, items.map(({ d, done }, i) => h('li', null,
      h('a', { class: ['dp-task', done && 'done'], href: d.href, style: { '--dp-accent': d.accent } },
        h('span', { class: 'dp-check', 'aria-hidden': 'true', html: done ? icon('check', { size: 16 }) : `<span class="dp-num">${i + 1}</span>` }),
        h('span', { class: 'dp-task-icon', 'aria-hidden': 'true', html: d.avatar ? '' : icon(d.icon, { size: 18 }) }, d.avatar || null),
        h('span', { class: 'dp-task-main' },
          h('span', { class: 'dp-task-title' }, d.title),
          h('span', { class: 'dp-task-sub' }, d.sub)),
        done
          ? h('span', { class: 'dp-task-done' }, t('daily.done'))
          : h('span', { class: 'dp-task-min tabular' }, t('daily.minutesShort', { count: d.minutes })),
        h('span', { class: 'sr-only' }, done ? t('daily.doneSr') : ''),
        h('span', { class: 'dp-go', 'aria-hidden': 'true', html: icon('chevron-right', { size: 18 }) })))));

    const pct = items.length ? Math.round((doneCount / items.length) * 100) : 0;
    this.planEl.replaceChildren(
      h('div', { class: 'dp-plan-progress' },
        h('div', { class: 'progress progress-sm', role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': String(items.length), 'aria-valuenow': String(doneCount), 'aria-label': t('daily.planProgress', { done: doneCount, total: items.length }) },
          h('div', { class: 'progress-bar', style: { width: `${pct}%` } })),
        h('span', { class: 'subtle text-xs tabular nowrap' }, t('daily.planProgress', { done: doneCount, total: items.length }))),
      list);

    this.renderGoal();
    this.renderStreak();
  }

  // ---- goal ------------------------------------------------------------------

  renderGoal() {
    const s = this.summary;
    const g = s.goal || { kind: 'minutes', target: 10 };
    const p = s.progress || { value: 0, target: g.target, ratio: 0, met: false };
    const R = 42;
    const C = 2 * Math.PI * R;
    const ratio = Math.max(0, Math.min(1, Number(p.ratio) || 0));
    const unit = g.kind === 'activities'
      ? t('daily.goal.activitiesUnit', { count: p.target })
      : t('daily.goal.minutesUnit', { count: p.target });
    const ring = h('div', { class: ['dp-ring', p.met && 'met'], role: 'img', 'aria-label': t('daily.goal.aria', { value: p.value, target: p.target, unit }) });
    ring.innerHTML = `<svg viewBox="0 0 100 100" aria-hidden="true" focusable="false">
      <circle class="dp-ring-track" cx="50" cy="50" r="${R}" />
      <circle class="dp-ring-fill" cx="50" cy="50" r="${R}" stroke-dasharray="${C.toFixed(2)}" stroke-dashoffset="${(C * (1 - ratio)).toFixed(2)}" />
    </svg>`;
    ring.appendChild(h('div', { class: 'dp-ring-center' },
      p.met
        ? h('span', { class: 'dp-ring-check', html: icon('check', { size: 26 }) })
        : h('span', { class: 'dp-ring-value tabular' }, formatNumber(p.value)),
      h('span', { class: 'dp-ring-unit' }, p.met ? t('daily.goal.metShort') : t('daily.goal.of', { target: p.target }))));

    const presetId = PRESETS.find((x) => x.kind === g.kind && x.target === g.target)?.id;
    const goalName = presetId ? t(`daily.goal.presets.${presetId}.name`) : t('daily.goal.custom');
    this.goalEl.replaceChildren(
      ring,
      h('div', { class: 'dp-goal-text' },
        h('div', { class: 'hub-eyebrow' }, t('daily.goal.title')),
        h('div', { class: 'dp-goal-name' }, `${goalName} · ${unit}`),
        h('p', { class: 'muted text-sm dp-goal-msg' }, p.met ? t('daily.goal.metText') : p.value > 0 ? t('daily.goal.progressText', { count: Math.max(0, p.target - p.value), unit: g.kind === 'activities' ? t('daily.goal.activitiesWord', { count: Math.max(0, p.target - p.value) }) : t('daily.goal.minutesWord', { count: Math.max(0, p.target - p.value) }) }) : t('daily.goal.startText')),
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm dp-goal-edit', html: icon('edit', { size: 16 }) + `<span>${t('daily.goal.change')}</span>`, onClick: () => this.openGoalPicker() })));

    if (p.met) this.maybeCelebrate(s.date);
  }

  maybeCelebrate(date) {
    if (readJson(CELEBRATED_KEY) === date) return;
    writeJson(CELEBRATED_KEY, date);
    toast(t('daily.goal.celebrate'), 'success');
    if (prefersReducedMotion()) return;
    const colors = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--cls-brilliant)', 'var(--cls-mistake)'];
    const rnd = seededRandom(`confetti:${date}`);
    const layer = h('div', { class: 'dp-confetti', 'aria-hidden': 'true' }, Array.from({ length: 36 }, (_, i) => h('i', {
      style: {
        '--x': `${Math.round((rnd() - 0.5) * 320)}px`,
        '--y': `${Math.round(-60 - rnd() * 160)}px`,
        '--r': `${Math.round(rnd() * 720 - 360)}deg`,
        '--d': `${Math.round(rnd() * 250)}ms`,
        '--c': colors[i % colors.length],
      },
    })));
    this.goalEl.appendChild(layer);
    this.bag.timeout(() => layer.remove(), 2200);
  }

  openGoalPicker() {
    const cur = this.summary?.goal || { kind: 'minutes', target: 10 };
    let choice = PRESETS.find((x) => x.kind === cur.kind && x.target === cur.target)?.id || (cur.kind === 'activities' ? 'activities' : 'regular');
    const actInput = h('input', { class: 'input input-sm dp-act-input', type: 'number', min: '1', max: '100', step: '1', inputmode: 'numeric',
      value: String(cur.kind === 'activities' ? cur.target : 3), 'aria-label': t('daily.goal.activitiesInput') });
    const options = [
      ...PRESETS.map((p) => ({ id: p.id, name: t(`daily.goal.presets.${p.id}.name`), desc: t(`daily.goal.presets.${p.id}.desc`), meta: t('daily.goal.minutesUnit', { count: p.target }) })),
      { id: 'activities', name: t('daily.goal.presets.activities.name'), desc: t('daily.goal.presets.activities.desc'), meta: actInput },
    ];
    const btns = options.map((o) => h('label', { class: ['dp-goal-opt', choice === o.id && 'active'] },
      h('input', { type: 'radio', name: 'dp-goal', value: o.id, checked: choice === o.id || null, class: 'sr-only' }),
      h('span', { class: 'dp-goal-opt-main' },
        h('span', { class: 'dp-goal-opt-name' }, o.name),
        h('span', { class: 'muted text-sm' }, o.desc)),
      h('span', { class: 'dp-goal-opt-meta' }, o.meta)));
    const body = h('div', { class: 'stack' }, h('p', { class: 'muted' }, t('daily.goal.pickerText')), h('div', { class: 'dp-goal-opts', role: 'radiogroup' }, btns));
    body.addEventListener('change', (e) => {
      if (e.target?.name !== 'dp-goal') return;
      choice = e.target.value;
      btns.forEach((b) => b.classList.toggle('active', b.querySelector('input[type=radio]')?.value === choice));
    });
    actInput.addEventListener('focus', () => {
      choice = 'activities';
      btns.forEach((b) => { const r = b.querySelector('input[type=radio]'); b.classList.toggle('active', r?.value === choice); if (r) r.checked = r.value === choice; });
    });
    const m = modal({
      title: t('daily.goal.pickerTitle'),
      body,
      actions: [
        { label: t('common.cancel'), kind: 'ghost' },
        {
          label: t('daily.goal.save'), kind: 'primary',
          onClick: async () => {
            let goal;
            if (choice === 'activities') {
              const n = Math.round(Number(actInput.value));
              if (!Number.isFinite(n) || n < 1 || n > 100) { actInput.focus(); toast(t('daily.goal.invalid'), 'warning'); return false; }
              goal = { kind: 'activities', target: n };
            } else {
              const p = PRESETS.find((x) => x.id === choice) || PRESETS[1];
              goal = { kind: p.kind, target: p.target };
            }
            const summary = await api.put('/api/daily/goal', goal, { signal: this.signal });
            if (this.bag.disposed) return true;
            this.summary = summary;
            this.render();
            toast(t('daily.goal.saved'), 'success');
            return true;
          },
        },
      ],
    });
    this.bag.add(() => m.close());
  }

  // ---- streak ----------------------------------------------------------------

  renderStreak() {
    const s = this.summary;
    const st = s.streak || { current: 0, best: 0, today_active: false };
    const last7 = Array.isArray(s.last7) ? s.last7 : [];
    const msg = st.current === 0
      ? t('daily.streak.start')
      : st.today_active ? t('daily.streak.today') : t('daily.streak.keep', { count: st.current });
    this.streakEl.replaceChildren(
      h('div', { class: 'dp-streak-head' },
        h('div', { class: ['dp-flame', st.current > 0 && 'on'], 'aria-hidden': 'true', html: icon('fire') }),
        h('div', { class: 'dp-streak-nums' },
          h('div', { class: 'dp-streak-cur' },
            h('span', { class: 'tabular' }, formatNumber(st.current)), ' ',
            h('span', { class: 'dp-streak-label' }, t('daily.streak.days', { count: st.current }))),
          h('div', { class: 'subtle text-xs' }, t('daily.streak.best', { count: st.best })))),
      h('ol', { class: 'dp-week', 'aria-label': t('daily.streak.weekAria') }, last7.map((d) => {
        const date = utcDate(d.day);
        const isToday = d.day === s.date;
        const label = formatDateIntl(date, { weekday: 'long', timeZone: 'UTC' });
        return h('li', { class: ['dp-day', d.active && 'on', isToday && 'today'], title: `${label}: ${d.active ? t('daily.streak.practised') : t('daily.streak.rest')}` },
          h('span', { class: 'dp-dot', 'aria-hidden': 'true', html: d.active ? icon('check', { size: 12 }) : '' }),
          h('span', { class: 'dp-day-name', 'aria-hidden': 'true' }, formatDateIntl(date, { weekday: 'narrow', timeZone: 'UTC' })),
          h('span', { class: 'sr-only' }, `${label}: ${d.active ? t('daily.streak.practised') : t('daily.streak.rest')}`));
      })),
      h('p', { class: 'muted text-sm dp-streak-msg' }, msg));
  }

  // ---- heatmap ---------------------------------------------------------------

  async loadHeatmap() {
    this.heatLoaded = true;
    this.heatBody.replaceChildren(h('div', { class: 'skeleton dp-heat-skel' }));
    let res;
    try {
      res = await api.get(`/api/activity?days=${HEATMAP_WEEKS * 7}`, { signal: this.signal });
    } catch (e) {
      if (isAbort(e) || this.bag.disposed) return;
      this.heatLoaded = false;
      this.heatBody.replaceChildren(h('p', { class: 'muted' }, t('daily.heatmap.error')));
      return;
    }
    if (this.bag.disposed) return;
    this.renderHeatmap(Array.isArray(res?.items) ? res.items : []);
  }

  renderHeatmap(items) {
    const byDay = new Map(items.map((d) => [d.day, d]));
    const today = dayNum(this.summary?.date || utcToday());
    const dow = (new Date(today * 86400000).getUTCDay() + 6) % 7; // Monday = 0
    const start = today - dow - (HEATMAP_WEEKS - 1) * 7;
    const grid = h('div', { class: 'dp-heat-grid', role: 'grid', 'aria-label': t('daily.heatmap.title') });
    let activeDays = 0; let total = 0;
    for (let w = 0; w < HEATMAP_WEEKS; w++) {
      const col = h('div', { class: 'dp-heat-col', role: 'row' });
      for (let d = 0; d < 7; d++) {
        const n = start + w * 7 + d;
        if (n > today) { col.appendChild(h('span', { class: 'dp-heat-cell future', role: 'gridcell', 'aria-hidden': 'true' })); continue; }
        const day = dayStr(n);
        const rec = byDay.get(day);
        const count = rec?.total || 0;
        if (count) { activeDays++; total += count; }
        const label = `${formatDateIntl(utcDate(day), { month: 'short', day: 'numeric', timeZone: 'UTC' })}: ${t('daily.heatmap.count', { count })}`;
        col.appendChild(h('span', { class: ['dp-heat-cell', `l${heatLevel(count)}`, n === today && 'today'], role: 'gridcell', title: label, 'aria-label': label }));
      }
      grid.appendChild(col);
    }
    const dayLabels = h('div', { class: 'dp-heat-days', 'aria-hidden': 'true' }, [0, 2, 4].map((i) => h('span', { style: { gridRow: String(i + 1) } },
      formatDateIntl(new Date((start + i) * 86400000), { weekday: 'short', timeZone: 'UTC' }))));
    const meta = this.heatEl.querySelector('.dp-heat-meta');
    if (meta) meta.textContent = t('daily.heatmap.summary', { count: activeDays });
    const best = Number(this.summary?.streak?.best) || 0;
    const stat = (label, value) => h('div', { class: 'stat dp-heat-stat' }, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value tabular' }, formatNumber(value)));
    this.heatBody.replaceChildren(h('div', { class: 'dp-heat-layout' },
      h('div', { class: 'dp-heat-left' },
        h('div', { class: 'dp-heat-wrap' }, dayLabels, grid),
        h('div', { class: 'dp-heat-foot' },
          h('span', { class: 'subtle text-xs' }, t('daily.heatmap.less')),
          [0, 1, 2, 3, 4].map((l) => h('span', { class: `dp-heat-cell l${l}`, 'aria-hidden': 'true' })),
          h('span', { class: 'subtle text-xs' }, t('daily.heatmap.more')))),
      h('div', { class: 'dp-heat-stats' },
        stat(t('daily.heatmap.activeDays'), activeDays),
        stat(t('daily.heatmap.activities'), total),
        stat(t('daily.heatmap.bestStreak'), best))));
  }
}

/** Opponent for today's game: closest to the estimated rating, else the last bot played, else the gentlest. */
function pickBot(bots, estimate, games) {
  const pool = (bots || []).filter((b) => b && b.id && b.category !== 'coach' && Number.isFinite(Number(b.elo)));
  if (!pool.length) return null;
  if (estimate !== null && estimate !== undefined) {
    return pool.reduce((best, b) => (Math.abs(b.elo - estimate) < Math.abs(best.elo - estimate) ? b : best), pool[0]);
  }
  const last = (games || []).find((g) => g && g.bot_id && pool.some((b) => b.id === g.bot_id));
  if (last) return pool.find((b) => b.id === last.bot_id);
  return [...pool].sort((a, b) => a.elo - b.elo)[0];
}
