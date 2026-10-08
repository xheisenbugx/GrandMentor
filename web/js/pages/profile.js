// Profile page (#/profile): editable name/avatar, stats from /api/stats + /api/profile,
// W/D/L donut, puzzle-rating history and accuracy-trend line charts (inline SVG, with hover),
// per-bot record table and client-side achievements.

import { api, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, emptyState, skeleton, toast, escapeHtml, displayName } from '../ui.js';
import { ensureHubCss, gameOutcome, userAccuracy, formatGameDate, formatRelativeIntl } from './library.js';
import { t, getLocale, formatNumber, formatDateIntl } from '../i18n.js';
import { topWeaknessesCard } from './insights.js';

export const title = () => t('profile.title');
const span = (key, params) => `<span>${escapeHtml(t(key, params))}</span>`;

const AVATARS = ['♟️', '♞', '👑', '🦁', '🐯', '🦊', '🐼', '🐨', '🐸', '🐙', '🦄', '🐲', '🤖', '🧙', '🦸', '🥷', '🧑‍🎓', '🧑‍🚀', '🎩', '🌟', '🔥', '⚡', '🍀', '🎯'];
const NAME_MAX = 40;

export async function mount(root) {
  await ensureHubCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  // Compact "Top weaknesses" card (Insights feature); re-created on every render.
  let weakCard = null;
  bag.add(() => weakCard?.destroy());

  const content = h('div', { class: 'stack-lg' }, skeleton('card', 4), skeleton('text', 6));
  const page = h('div', { class: 'page hub-page hub-profile' },
    pageHeader({ title: t('profile.title'), subtitle: t('profile.subtitle'), icon: 'profile',
      actions: [h('a', { class: 'btn btn-ghost', href: '#/settings', html: icon('settings') + span('profile.settings') })] }),
    content);
  root.appendChild(page);

  load();
  return bag.dispose;

  async function load() {
    content.replaceChildren(skeleton('card', 4), skeleton('text', 6));
    const settle = (p) => p.then((v) => ({ ok: true, v }), (e) => ({ ok: false, e }));
    const [pR, sR, bR, prR, cR, gR] = await Promise.all([
      settle(api.get('/api/profile', { signal })),
      settle(api.get('/api/stats', { signal })),
      settle(api.get('/api/bots', { signal })),
      settle(api.get('/api/progress', { signal })),
      settle(api.get('/api/courses', { signal })),
      settle(api.get('/api/games?limit=200', { signal })),
    ]);
    if (signal.aborted) return;
    if (!pR.ok && !sR.ok) {
      content.replaceChildren(h('div', { class: 'card' }, emptyState({ icon: 'wifi-off', title: t('profile.errors.loadTitle'), text: pR.e?.message || t('profile.errors.server'), action: { label: t('profile.errors.retry'), icon: 'refresh', onClick: load } })));
      return;
    }
    const profile = pR.ok && pR.v ? pR.v : { name: '', avatar: '♟️', puzzle_rating: 0, puzzles_solved: 0, puzzles_failed: 0, rush_best: 0, streak_days: 0 };
    const stats = sR.ok && sR.v ? sR.v : {};
    const bots = bR.ok && Array.isArray(bR.v) ? bR.v : [];
    const progress = prR.ok && Array.isArray(prR.v) ? prR.v : [];
    const courses = cR.ok && Array.isArray(cR.v) ? cR.v : [];
    const games = gR.ok && Array.isArray(gR.v) ? gR.v : [];
    render({ profile, stats, bots, progress, courses, games });
  }

  function render(d) {
    const { profile, stats, bots, progress, courses, games } = d;
    const botsById = new Map(bots.map((b) => [b.id, b]));
    const won = num(stats.games_won); const lost = num(stats.games_lost); const drawn = num(stats.games_drawn);
    const played = num(stats.games_played) || won + lost + drawn;
    const lessonsDone = Math.max(num(stats.lessons_completed), new Set(progress.filter((p) => p.completed).map((p) => `${p.course_id}/${p.lesson_id}`)).size);
    const totalLessons = courses.reduce((n, c) => n + (Array.isArray(c.lessons) ? c.lessons.length : num(c.lesson_count)), 0);
    const solved = num(profile.puzzles_solved); const failed = num(profile.puzzles_failed);

    // Accuracy trend: user's accuracy in reviewed games, oldest → newest (last 30).
    const accPoints = games
      .filter((g) => userAccuracy(g) != null)
      .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)))
      .slice(-30)
      .map((g) => ({ label: formatGameDate(g.created_at), y: userAccuracy(g) }));
    const avgAcc = typeof stats.avg_accuracy === 'number' ? stats.avg_accuracy
      : accPoints.length ? accPoints.reduce((s, p) => s + p.y, 0) / accPoints.length : null;
    const ratingPoints = (Array.isArray(stats.rating_history) ? stats.rating_history : [])
      .filter((p) => p && Number.isFinite(Number(p.rating)))
      .slice(-120)
      .map((p) => ({ label: formatGameDate(p.at), y: Number(p.rating) }));

    const ctx = {
      played, won, drawn, solved, lessonsDone,
      rating: num(profile.puzzle_rating), rush: num(profile.rush_best), streak: num(profile.streak_days),
      bestAcc: accPoints.reduce((m, p) => Math.max(m, p.y), 0),
      maxBeatElo: (Array.isArray(stats.per_bot) ? stats.per_bot : []).reduce((m, r) => (r.won > 0 ? Math.max(m, num(botsById.get(r.bot_id)?.elo)) : m), 0),
      botsBeaten: (Array.isArray(stats.per_bot) ? stats.per_bot : []).filter((r) => r.won > 0).length,
      favorites: games.filter((g) => g.favorite).length,
      wonAsBlack: games.filter((g) => g.user_color === 'black' && gameOutcome(g) === 'win').length,
      coursesDone: courses.filter((c) => Array.isArray(c.lessons) && c.lessons.length && c.lessons.every((l) => progress.some((p) => p.completed && p.course_id === c.id && p.lesson_id === l.id))).length,
    };

    weakCard?.destroy();
    weakCard = topWeaknessesCard();
    content.replaceChildren(
      renderIdentity(profile, ctx),
      h('div', { class: 'grid-auto grid-auto-sm hub-stat-grid' },
        statTile(t('profile.stats.played'), formatNumber(played), 'board'),
        statTile(t('profile.stats.winRate'), played ? pct(won / played) : '—', 'trophy'),
        statTile(t('profile.stats.avgAccuracy'), avgAcc != null ? fmt1(avgAcc) : '—', 'target'),
        statTile(t('profile.stats.puzzleRating'), formatNumber(Math.round(ctx.rating)), 'puzzle'),
        statTile(t('profile.stats.puzzlesSolved'), formatNumber(solved), 'check-circle', solved + failed ? t('profile.stats.success', { pct: pct(solved / (solved + failed)) }) : null),
        statTile(t('profile.stats.rushBest'), formatNumber(ctx.rush), 'bolt'),
        statTile(t('profile.stats.lessonsDone'), totalLessons ? `${lessonsDone}/${totalLessons}` : lessonsDone, 'learn'),
        statTile(t('profile.stats.streak'), formatNumber(ctx.streak), 'fire')),
      weakCard.el,
      h('div', { class: 'hub-profile-charts' },
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('chart') + span('profile.results.title') })),
          donut({ won, drawn, lost })),
        h('section', { class: 'card hub-chart-card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('puzzle') + span('profile.rating.title') }),
            ratingPoints.length ? h('span', { class: 'badge badge-info' }, String(Math.round(ratingPoints[ratingPoints.length - 1].y))) : null),
          ratingPoints.length >= 2
            ? lineChart(ratingPoints, { name: t('profile.rating.title'), format: (v) => String(Math.round(v)), table: true })
            : emptyState({ icon: 'puzzle', title: t('profile.rating.emptyTitle'), text: t('profile.rating.emptyText'), action: { label: t('profile.rating.emptyAction'), href: '#/puzzles', kind: 'secondary' } }))),
      h('section', { class: 'card hub-chart-card' },
        h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('target') + span('profile.accuracy.title') }),
          avgAcc != null ? h('span', { class: 'muted text-sm' }, t('profile.accuracy.average', { value: pctOf(avgAcc, 1) })) : null),
        accPoints.length >= 2
          ? lineChart(accPoints, { name: t('profile.accuracy.name'), yMin: 0, yMax: 100, format: (v) => pctOf(v, 1), axis: (v) => pctOf(v, 0), avg: avgAcc })
          : emptyState({ icon: 'sparkles', title: t('profile.accuracy.emptyTitle'), text: t('profile.accuracy.emptyText'), action: { label: t('profile.accuracy.emptyAction'), href: '#/library', kind: 'secondary' } })),
      renderBotTable(stats, botsById),
      renderAchievements(ctx));
  }

  // ---- Identity card with inline edit -----------------------------------------
  function renderIdentity(profile, ctx) {
    const card = h('section', { class: 'card hub-identity' });
    let editing = false;
    let draftAvatar = profile.avatar || '♟️';
    const show = () => {
      card.replaceChildren(
        h('div', { class: 'avatar avatar-xl avatar-round hub-identity-avatar', 'aria-hidden': 'true' }, profile.avatar || '♟️'),
        h('div', { class: 'stack-sm hub-identity-main' },
          h('h2', { class: 'hub-identity-name' }, displayName(profile.name) || t('profile.identity.defaultName')),
          h('div', { class: 'row-sm row-wrap muted text-sm' },
            h('span', { class: 'badge badge-gold', html: icon('trophy', { size: 14 }) + span('profile.identity.puzzleRating', { rating: Math.round(ctx.rating) }) }),
            ctx.streak ? h('span', { class: 'badge badge-warning', html: icon('fire', { size: 14 }) + span('profile.identity.streak', { count: ctx.streak }) }) : null,
            profile.last_active ? h('span', null, t('profile.identity.lastActive', { when: formatActiveDay(profile.last_active) })) : null)),
        h('button', { type: 'button', class: 'btn btn-secondary', html: icon('edit') + span('profile.identity.edit'), onClick: () => { editing = true; draftAvatar = profile.avatar || '♟️'; edit(); } }));
    };
    const edit = () => {
      const nameInput = h('input', { class: 'input', value: displayName(profile.name), maxlength: String(NAME_MAX), placeholder: t('profile.edit.namePlaceholder'), 'aria-label': t('profile.edit.name') });
      const preview = h('div', { class: 'avatar avatar-xl avatar-round hub-identity-avatar', 'aria-hidden': 'true' }, draftAvatar);
      const grid = h('div', { class: 'hub-avatar-grid', role: 'radiogroup', 'aria-label': t('profile.edit.chooseAvatar') },
        AVATARS.map((a) => h('button', { type: 'button', class: ['hub-avatar-opt', a === draftAvatar && 'active'], role: 'radio', 'aria-checked': String(a === draftAvatar), dataset: { a } }, a)));
      grid.addEventListener('click', (e) => {
        const b = e.target.closest('button[data-a]');
        if (!b) return;
        draftAvatar = b.dataset.a;
        preview.textContent = draftAvatar;
        for (const x of grid.children) { const on = x === b; x.classList.toggle('active', on); x.setAttribute('aria-checked', String(on)); }
      });
      const saveBtn = h('button', { type: 'submit', class: 'btn btn-primary', html: icon('check') + span('profile.edit.save') });
      const form = h('form', { class: 'hub-identity-form stack' },
        h('div', { class: 'field' }, h('label', { class: 'label' }, t('profile.edit.name')), nameInput),
        h('div', { class: 'field' }, h('div', { class: 'label' }, t('profile.edit.avatar')), grid),
        h('div', { class: 'row-sm' }, saveBtn,
          h('button', { type: 'button', class: 'btn btn-ghost', onClick: () => { editing = false; show(); } }, t('profile.edit.cancel'))));
      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const name = nameInput.value.trim().slice(0, NAME_MAX);
        saveBtn.classList.add('loading');
        try {
          const updated = await api.put('/api/profile', { name, avatar: draftAvatar }, { signal });
          Object.assign(profile, updated && typeof updated === 'object' ? updated : { name, avatar: draftAvatar });
          editing = false;
          show();
          toast(t('profile.edit.saved'), 'success');
        } catch (err) {
          if (isAbort(err)) return;
          saveBtn.classList.remove('loading');
          toast(err?.message || t('profile.edit.error'), 'error');
        }
      });
      card.replaceChildren(preview, form);
      requestAnimationFrame(() => { if (editing && nameInput.isConnected) nameInput.focus(); });
    };
    show();
    return card;
  }

  // ---- Line chart (SVG) with crosshair tooltip -------------------------------------
  function lineChart(points, { name, yMin, yMax, format = String, axis = null, avg = null, table = false }) {
    const W = 640; const H = 220; const P = { l: 44, r: 12, t: 14, b: 26 };
    const ys = points.map((p) => p.y);
    let lo = yMin ?? Math.min(...ys); let hi = yMax ?? Math.max(...ys);
    if (yMin == null || yMax == null) { const pad = Math.max(10, (hi - lo) * 0.12); lo = Math.floor((lo - pad) / 10) * 10; hi = Math.ceil((hi + pad) / 10) * 10; }
    if (hi <= lo) hi = lo + 1;
    const n = points.length;
    const x = (i) => P.l + (n === 1 ? 0 : (i / (n - 1)) * (W - P.l - P.r));
    const y = (v) => P.t + (1 - (v - lo) / (hi - lo)) * (H - P.t - P.b);
    const ticks = 4;
    const grid = [];
    for (let i = 0; i <= ticks; i++) {
      const v = lo + ((hi - lo) * i) / ticks;
      grid.push(h('line', { class: 'hub-grid', x1: P.l, x2: W - P.r, y1: y(v), y2: y(v) }));
      grid.push(h('text', { class: 'hub-axis', x: P.l - 8, y: y(v) + 4, 'text-anchor': 'end' }, (axis || format)(v)));
    }
    const d = points.map((p, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)} ${y(p.y).toFixed(1)}`).join('');
    const area = `${d}L${x(n - 1).toFixed(1)} ${H - P.b}L${x(0).toFixed(1)} ${H - P.b}Z`;
    const cross = h('line', { class: 'hub-cross', x1: 0, x2: 0, y1: P.t, y2: H - P.b, visibility: 'hidden' });
    const dot = h('circle', { class: 'hub-dotmark', r: 5, cx: 0, cy: 0, visibility: 'hidden' });
    const svg = h('svg', { class: 'hub-line-svg', viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': t('profile.chart.aria', { name, count: points.length, from: format(points[0].y), to: format(points[n - 1].y) }) },
      grid,
      h('path', { class: 'hub-area', d: area }),
      avg != null ? h('line', { class: 'hub-avg', x1: P.l, x2: W - P.r, y1: y(avg), y2: y(avg) }) : null,
      h('path', { class: 'hub-line', d }),
      h('circle', { class: 'hub-dotmark end', r: 4.5, cx: x(n - 1), cy: y(points[n - 1].y) }),
      h('text', { class: 'hub-axis', x: P.l, y: H - 6 }, points[0].label || ''),
      h('text', { class: 'hub-axis', x: W - P.r, y: H - 6, 'text-anchor': 'end' }, points[n - 1].label || ''),
      cross, dot);
    const tip = h('div', { class: 'hub-chart-tip', hidden: true });
    const wrap = h('div', { class: 'hub-line-wrap' }, svg, tip);
    let raf = 0;
    const onMove = (e) => {
      if (raf) return;
      const cx = e.clientX;
      raf = requestAnimationFrame(() => {
        raf = 0;
        const r = svg.getBoundingClientRect();
        if (!r.width) return;
        const sx = ((cx - r.left) / r.width) * W;
        const i = Math.max(0, Math.min(n - 1, Math.round(((sx - P.l) / (W - P.l - P.r)) * (n - 1))));
        const px = x(i); const py = y(points[i].y);
        cross.setAttribute('x1', px); cross.setAttribute('x2', px); cross.setAttribute('visibility', 'visible');
        dot.setAttribute('cx', px); dot.setAttribute('cy', py); dot.setAttribute('visibility', 'visible');
        tip.replaceChildren(h('div', { class: 'semibold tabular' }, format(points[i].y)), h('div', { class: 'subtle text-xs' }, points[i].label || ''));
        tip.hidden = false;
        const left = (px / W) * r.width;
        tip.style.left = `${Math.max(40, Math.min(r.width - 40, left))}px`;
        tip.style.top = `${(py / H) * r.height}px`;
      });
    };
    const onLeave = () => {
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
      cross.setAttribute('visibility', 'hidden'); dot.setAttribute('visibility', 'hidden'); tip.hidden = true;
    };
    bag.on(svg, 'pointermove', onMove);
    bag.on(svg, 'pointerleave', onLeave);
    bag.add(() => { if (raf) cancelAnimationFrame(raf); });
    if (!table) return wrap;
    const rows = points.slice(-15).reverse();
    return h('div', null, wrap,
      h('details', { class: 'hub-table-toggle' }, h('summary', { class: 'muted text-sm' }, t('profile.chart.showTable')),
        h('table', { class: 'hub-table' },
          h('thead', null, h('tr', null, h('th', null, t('profile.chart.date')), h('th', { class: 'num' }, name))),
          h('tbody', null, rows.map((p) => h('tr', null, h('td', null, p.label), h('td', { class: 'num tabular' }, format(p.y))))))));
  }

  function renderBotTable(stats, botsById) {
    const rows = (Array.isArray(stats.per_bot) ? stats.per_bot : []).filter((r) => r && num(r.played) > 0)
      .sort((a, b) => num(botsById.get(a.bot_id)?.elo) - num(botsById.get(b.bot_id)?.elo));
    const card = h('section', { class: 'card card-flush hub-bot-table' },
      h('div', { class: 'card-header p-4' }, h('div', { class: 'card-title', html: icon('robot') + span('profile.bots.title') })));
    if (!rows.length) {
      card.appendChild(emptyState({ icon: 'robot', title: t('profile.bots.emptyTitle'), text: t('profile.bots.emptyText'), action: { label: t('profile.bots.emptyAction'), href: '#/play', icon: 'play' } }));
      return card;
    }
    card.appendChild(h('div', { class: 'hub-table-scroll' }, h('table', { class: 'hub-table' },
      h('thead', null, h('tr', null, h('th', null, t('profile.bots.opponent')), h('th', { class: 'num' }, t('profile.bots.games')),
        h('th', { class: 'num', title: t('profile.results.won') }, t('profile.bots.w')), h('th', { class: 'num', title: t('profile.results.drawn') }, t('profile.bots.d')), h('th', { class: 'num', title: t('profile.results.lost') }, t('profile.bots.l')),
        h('th', { class: 'hide-mobile' }, t('profile.bots.score')), h('th', null, ''))),
      h('tbody', null, rows.map((r) => {
        const b = botsById.get(r.bot_id);
        const played = num(r.played) || 1;
        const share = (k) => `${(num(r[k]) / played) * 100}%`;
        return h('tr', null,
          h('td', null, h('div', { class: 'row-sm' }, h('span', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, b?.avatar || '🤖'),
            h('div', null, h('div', { class: 'semibold' }, b?.name || r.bot_id), b ? h('div', { class: 'subtle text-xs' }, t('profile.bots.rated', { elo: b.elo })) : null))),
          h('td', { class: 'num tabular' }, String(num(r.played))),
          h('td', { class: 'num tabular text-primary' }, String(num(r.won))),
          h('td', { class: 'num tabular muted' }, String(num(r.drawn))),
          h('td', { class: 'num tabular text-danger' }, String(num(r.lost))),
          h('td', { class: 'hide-mobile' }, h('div', { class: 'hub-wdl', title: t('profile.results.summary', { won: num(r.won), drawn: num(r.drawn), lost: num(r.lost) }) },
            h('span', { class: 'w', style: { width: share('won') } }), h('span', { class: 'd', style: { width: share('drawn') } }), h('span', { class: 'l', style: { width: share('lost') } }))),
          h('td', { class: 'num' }, h('a', { class: 'btn btn-ghost btn-sm', href: `#/play/${encodeURIComponent(r.bot_id)}` }, t('profile.bots.rematch'))));
      })))));
    return card;
  }
}

// ---- Achievements --------------------------------------------------------------
// Names and descriptions resolve at render time: t('profile.achievements.items.<id>.title|desc').
const ACHIEVEMENTS = [
  { id: 'first-game', emoji: '♟️', get: (c) => [c.played, 1] },
  { id: 'first-win', emoji: '🏆', get: (c) => [c.won, 1] },
  { id: 'games-10', emoji: '🎲', get: (c) => [c.played, 10] },
  { id: 'games-50', emoji: '🏟️', get: (c) => [c.played, 50] },
  { id: 'wins-25', emoji: '🥇', get: (c) => [c.won, 25] },
  { id: 'black-win', emoji: '🌑', get: (c) => [c.wonAsBlack, 1] },
  { id: 'draw', emoji: '🤝', get: (c) => [c.drawn, 1] },
  { id: 'beat-1000', emoji: '🥉', get: (c) => [c.maxBeatElo >= 1000 ? 1 : 0, 1] },
  { id: 'beat-1500', emoji: '🥈', get: (c) => [c.maxBeatElo >= 1500 ? 1 : 0, 1] },
  { id: 'beat-2000', emoji: '👑', get: (c) => [c.maxBeatElo >= 2000 ? 1 : 0, 1] },
  { id: 'bots-5', emoji: '🤖', get: (c) => [c.botsBeaten, 5] },
  { id: 'acc-80', emoji: '🎯', get: (c) => [Math.min(c.bestAcc, 80), 80] },
  { id: 'acc-90', emoji: '💎', get: (c) => [Math.min(c.bestAcc, 90), 90] },
  { id: 'puz-10', emoji: '🧩', get: (c) => [c.solved, 10] },
  { id: 'puz-100', emoji: '🧠', get: (c) => [c.solved, 100] },
  { id: 'puz-500', emoji: '⚔️', get: (c) => [c.solved, 500] },
  { id: 'rating-1500', emoji: '📈', get: (c) => [Math.min(c.rating, 1500), 1500] },
  { id: 'rating-2000', emoji: '🚀', get: (c) => [Math.min(c.rating, 2000), 2000] },
  { id: 'rush-15', emoji: '⚡', get: (c) => [c.rush, 15] },
  { id: 'rush-30', emoji: '🌪️', get: (c) => [c.rush, 30] },
  { id: 'streak-3', emoji: '🔥', get: (c) => [c.streak, 3] },
  { id: 'streak-7', emoji: '📅', get: (c) => [c.streak, 7] },
  { id: 'streak-30', emoji: '🗓️', get: (c) => [c.streak, 30] },
  { id: 'lesson-1', emoji: '📘', get: (c) => [c.lessonsDone, 1] },
  { id: 'lesson-10', emoji: '🎓', get: (c) => [c.lessonsDone, 10] },
  { id: 'course-1', emoji: '🏅', get: (c) => [c.coursesDone, 1] },
  { id: 'fav', emoji: '⭐', get: (c) => [c.favorites, 1] },
];

function renderAchievements(ctx) {
  const items = ACHIEVEMENTS.map((a) => {
    const [cur, target] = a.get(ctx);
    const value = Math.max(0, Number(cur) || 0);
    const title = t(`profile.achievements.items.${a.id}.title`);
    const desc = t(`profile.achievements.items.${a.id}.desc`);
    return { ...a, title, desc, value, target, earned: value >= target };
  }).sort((a, b) => Number(b.earned) - Number(a.earned) || (b.value / b.target) - (a.value / a.target));
  const earned = items.filter((i) => i.earned).length;
  return h('section', { class: 'card' },
    h('div', { class: 'card-header' },
      h('div', { class: 'card-title', html: icon('medal') + span('profile.achievements.title') }),
      h('span', { class: 'badge badge-gold' }, t('profile.achievements.unlockedCount', { earned, total: items.length }))),
    h('div', { class: 'hub-badges' }, items.map((a) => h('div', {
      class: ['hub-badge', a.earned ? 'earned' : 'locked'],
      title: a.earned ? t('profile.achievements.unlockedTip', { title: a.title }) : t('profile.achievements.lockedTip', { title: a.title, desc: a.desc }),
    },
    h('div', { class: 'hub-badge-icon', 'aria-hidden': 'true' }, a.emoji),
    h('div', { class: 'hub-badge-title' }, a.title),
    h('div', { class: 'hub-badge-desc' }, a.desc),
    a.earned
      ? h('div', { class: 'hub-badge-done', html: icon('check', { size: 14 }) + span('profile.achievements.unlocked') })
      : h('div', { class: 'progress progress-sm', role: 'progressbar', 'aria-label': t('profile.achievements.progress', { title: a.title }), 'aria-valuenow': String(Math.round(a.value)), 'aria-valuemax': String(a.target) },
        h('div', { class: 'progress-bar', style: { width: `${Math.min(100, (a.value / a.target) * 100)}%` } }))))));
}

// ---- Small pieces ------------------------------------------------------------------
function num(v) { const n = Number(v); return Number.isFinite(n) ? n : 0; }
/** Locale-aware percentage from a 0..1 ratio ("63%" / "63 %"). */
function pct(ratio) { return formatNumber(ratio, { style: 'percent', maximumFractionDigits: 0 }); }
/** Locale-aware number with one decimal ("87.4" / "87,4"). */
/** Locale-aware percentage from a 0..100 value with `digits` decimals. */
function pctOf(v, digits) { return formatNumber(v / 100, { style: 'percent', minimumFractionDigits: digits, maximumFractionDigits: digits }); }
function fmt1(v) { return formatNumber(v, { minimumFractionDigits: 1, maximumFractionDigits: 1 }); }

function statTile(label, value, iconName, delta) {
  return h('div', { class: 'stat hub-stat' },
    h('div', { class: 'hub-stat-icon', html: icon(iconName) }),
    h('div', { class: 'stat-label' }, label),
    h('div', { class: 'stat-value tabular' }, String(value)),
    delta ? h('div', { class: 'stat-delta' }, delta) : null);
}

/** W/D/L donut chart with legend; 2px gaps between segments. */
function donut({ won, drawn, lost }) {
  const total = won + drawn + lost;
  if (!total) {
    return emptyState({ icon: 'chart', title: t('profile.results.emptyTitle'), text: t('profile.results.emptyText'), action: { label: t('profile.results.emptyAction'), href: '#/play', icon: 'play' } });
  }
  const R = 42; const C = 2 * Math.PI * R; const GAP = total > 1 ? 1.2 : 0;
  const all = [['won', won, t('profile.results.won')], ['drawn', drawn, t('profile.results.drawn')], ['lost', lost, t('profile.results.lost')]];
  const segs = all.filter((s) => s[1] > 0);
  let offset = 0;
  const circles = segs.map(([k, v, l]) => {
    const len = (v / total) * C;
    const dash = Math.max(0.01, len - (segs.length > 1 ? GAP : 0));
    const c = h('circle', { class: `hub-donut-seg ${k}`, r: R, cx: 50, cy: 50, 'stroke-dasharray': `${dash} ${C - dash}`, 'stroke-dashoffset': String(-offset) },
      h('title', null, `${l}: ${v} (${pct(v / total)})`));
    offset += len;
    return c;
  });
  return h('div', { class: 'hub-donut' },
    h('div', { class: 'hub-donut-fig' },
      h('svg', { viewBox: '0 0 100 100', role: 'img', 'aria-label': t('profile.results.summary', { won, drawn, lost }) },
        h('circle', { class: 'hub-donut-track', r: R, cx: 50, cy: 50 }),
        h('g', { transform: 'rotate(-90 50 50)' }, circles)),
      h('div', { class: 'hub-donut-center' }, h('div', { class: 'hub-donut-num tabular' }, pct(won / total)), h('div', { class: 'subtle text-xs' }, t('profile.results.winRate')))),
    h('ul', { class: 'hub-legend' },
      all.map(([k, v, l]) =>
        h('li', null, h('span', { class: `hub-swatch ${k}` }), h('span', null, l), h('span', { class: 'spacer' }), h('span', { class: 'semibold tabular' }, String(v)),
          h('span', { class: 'subtle tabular text-sm' }, pct(v / total))))));
}

/** `last_active` is a local calendar date ("YYYY-MM-DD"); compare by calendar day, not UTC instant. */
function formatActiveDay(value) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(value || ''));
  if (!m) return formatRelativeIntl(value);
  const day = new Date(+m[1], +m[2] - 1, +m[3]);
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.max(0, Math.round((today - day) / 86400000));
  if (days < 7) {
    try { return new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' }).format(-days, 'day'); } catch { /* fall through */ }
  }
  return formatDateIntl(day, { month: 'short', day: 'numeric', year: day.getFullYear() === now.getFullYear() ? undefined : 'numeric' });
}
