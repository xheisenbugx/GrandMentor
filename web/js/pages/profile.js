// Profile page (#/profile): editable name/avatar, stats from /api/stats + /api/profile,
// W/D/L donut, puzzle-rating history and accuracy-trend line charts (inline SVG, with hover),
// per-bot record table and client-side achievements.

import { api, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, emptyState, skeleton, toast, formatDate, formatRelative } from '../ui.js';
import { ensureHubCss, gameOutcome, userAccuracy } from './library.js';

export const title = 'Profile';

const AVATARS = ['♟️', '♞', '👑', '🦁', '🐯', '🦊', '🐼', '🐨', '🐸', '🐙', '🦄', '🐲', '🤖', '🧙', '🦸', '🥷', '🧑‍🎓', '🧑‍🚀', '🎩', '🌟', '🔥', '⚡', '🍀', '🎯'];
const NAME_MAX = 40;

export async function mount(root) {
  await ensureHubCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;

  const content = h('div', { class: 'stack-lg' }, skeleton('card', 4), skeleton('text', 6));
  const page = h('div', { class: 'page hub-page hub-profile' },
    pageHeader({ title: 'Profile', subtitle: 'Your progress, stats and achievements', icon: 'profile',
      actions: [h('a', { class: 'btn btn-ghost', href: '#/settings', html: icon('settings') + '<span>Settings</span>' })] }),
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
      content.replaceChildren(h('div', { class: 'card' }, emptyState({ icon: 'wifi-off', title: 'We couldn’t load your profile', text: pR.e?.message || 'Is the GrandMentor server running?', action: { label: 'Try again', icon: 'refresh', onClick: load } })));
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
      .map((g) => ({ label: formatDate(g.created_at), y: userAccuracy(g) }));
    const avgAcc = typeof stats.avg_accuracy === 'number' ? stats.avg_accuracy
      : accPoints.length ? accPoints.reduce((s, p) => s + p.y, 0) / accPoints.length : null;
    const ratingPoints = (Array.isArray(stats.rating_history) ? stats.rating_history : [])
      .filter((p) => p && Number.isFinite(Number(p.rating)))
      .slice(-120)
      .map((p) => ({ label: formatDate(p.at), y: Number(p.rating) }));

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

    content.replaceChildren(
      renderIdentity(profile, ctx),
      h('div', { class: 'grid-auto grid-auto-sm hub-stat-grid' },
        statTile('Games played', played, 'board'),
        statTile('Win rate', played ? `${Math.round((won / played) * 100)}%` : '—', 'trophy'),
        statTile('Avg. accuracy', avgAcc != null ? avgAcc.toFixed(1) : '—', 'target'),
        statTile('Puzzle rating', Math.round(ctx.rating), 'puzzle'),
        statTile('Puzzles solved', solved, 'check-circle', solved + failed ? `${Math.round((solved / (solved + failed)) * 100)}% success` : null),
        statTile('Rush best', ctx.rush, 'bolt'),
        statTile('Lessons done', totalLessons ? `${lessonsDone}/${totalLessons}` : lessonsDone, 'learn'),
        statTile('Day streak', ctx.streak, 'fire')),
      h('div', { class: 'hub-profile-charts' },
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('chart') + '<span>Results</span>' })),
          donut({ won, drawn, lost })),
        h('section', { class: 'card hub-chart-card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('puzzle') + '<span>Puzzle rating</span>' }),
            ratingPoints.length ? h('span', { class: 'badge badge-info' }, String(Math.round(ratingPoints[ratingPoints.length - 1].y))) : null),
          ratingPoints.length >= 2
            ? lineChart(ratingPoints, { name: 'Puzzle rating', format: (v) => String(Math.round(v)), table: true })
            : emptyState({ icon: 'puzzle', title: 'No rating history yet', text: 'Solve a few puzzles and your rating graph will appear here.', action: { label: 'Solve puzzles', href: '#/puzzles', kind: 'secondary' } }))),
      h('section', { class: 'card hub-chart-card' },
        h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('target') + '<span>Accuracy trend</span>' }),
          avgAcc != null ? h('span', { class: 'muted text-sm' }, `Average ${avgAcc.toFixed(1)}%`) : null),
        accPoints.length >= 2
          ? lineChart(accPoints, { name: 'Accuracy', yMin: 0, yMax: 100, format: (v) => `${v.toFixed(1)}%`, avg: avgAcc })
          : emptyState({ icon: 'sparkles', title: 'Review games to track accuracy', text: 'Run Game Review on two or more games to see how your accuracy changes over time.', action: { label: 'Open library', href: '#/library', kind: 'secondary' } })),
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
          h('h2', { class: 'hub-identity-name' }, profile.name?.trim() || 'Chess friend'),
          h('div', { class: 'row-sm row-wrap muted text-sm' },
            h('span', { class: 'badge badge-gold', html: icon('trophy', { size: 14 }) + `<span>${Math.round(ctx.rating)} puzzles</span>` }),
            ctx.streak ? h('span', { class: 'badge badge-warning', html: icon('fire', { size: 14 }) + `<span>${ctx.streak}-day streak</span>` }) : null,
            profile.last_active ? h('span', null, `Last active ${formatActiveDay(profile.last_active)}`) : null)),
        h('button', { type: 'button', class: 'btn btn-secondary', html: icon('edit') + '<span>Edit profile</span>', onClick: () => { editing = true; draftAvatar = profile.avatar || '♟️'; edit(); } }));
    };
    const edit = () => {
      const nameInput = h('input', { class: 'input', value: profile.name || '', maxlength: String(NAME_MAX), placeholder: 'Your name', 'aria-label': 'Display name' });
      const preview = h('div', { class: 'avatar avatar-xl avatar-round hub-identity-avatar', 'aria-hidden': 'true' }, draftAvatar);
      const grid = h('div', { class: 'hub-avatar-grid', role: 'radiogroup', 'aria-label': 'Choose an avatar' },
        AVATARS.map((a) => h('button', { type: 'button', class: ['hub-avatar-opt', a === draftAvatar && 'active'], role: 'radio', 'aria-checked': String(a === draftAvatar), dataset: { a } }, a)));
      grid.addEventListener('click', (e) => {
        const b = e.target.closest('button[data-a]');
        if (!b) return;
        draftAvatar = b.dataset.a;
        preview.textContent = draftAvatar;
        for (const x of grid.children) { const on = x === b; x.classList.toggle('active', on); x.setAttribute('aria-checked', String(on)); }
      });
      const saveBtn = h('button', { type: 'submit', class: 'btn btn-primary', html: icon('check') + '<span>Save</span>' });
      const form = h('form', { class: 'hub-identity-form stack' },
        h('div', { class: 'field' }, h('label', { class: 'label' }, 'Display name'), nameInput),
        h('div', { class: 'field' }, h('div', { class: 'label' }, 'Avatar'), grid),
        h('div', { class: 'row-sm' }, saveBtn,
          h('button', { type: 'button', class: 'btn btn-ghost', onClick: () => { editing = false; show(); } }, 'Cancel')));
      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const name = nameInput.value.trim().slice(0, NAME_MAX);
        saveBtn.classList.add('loading');
        try {
          const updated = await api.put('/api/profile', { name, avatar: draftAvatar }, { signal });
          Object.assign(profile, updated && typeof updated === 'object' ? updated : { name, avatar: draftAvatar });
          editing = false;
          show();
          toast('Profile updated', 'success');
        } catch (err) {
          if (isAbort(err)) return;
          saveBtn.classList.remove('loading');
          toast(err?.message || 'Could not save your profile', 'error');
        }
      });
      card.replaceChildren(preview, form);
      requestAnimationFrame(() => { if (editing && nameInput.isConnected) nameInput.focus(); });
    };
    show();
    return card;
  }

  // ---- Line chart (SVG) with crosshair tooltip -------------------------------------
  function lineChart(points, { name, yMin, yMax, format = String, avg = null, table = false }) {
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
      grid.push(h('text', { class: 'hub-axis', x: P.l - 8, y: y(v) + 4, 'text-anchor': 'end' }, format(v).replace(/\.0%$/, '%')));
    }
    const d = points.map((p, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)} ${y(p.y).toFixed(1)}`).join('');
    const area = `${d}L${x(n - 1).toFixed(1)} ${H - P.b}L${x(0).toFixed(1)} ${H - P.b}Z`;
    const cross = h('line', { class: 'hub-cross', x1: 0, x2: 0, y1: P.t, y2: H - P.b, visibility: 'hidden' });
    const dot = h('circle', { class: 'hub-dotmark', r: 5, cx: 0, cy: 0, visibility: 'hidden' });
    const svg = h('svg', { class: 'hub-line-svg', viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': `${name}: ${points.length} points, from ${format(points[0].y)} to ${format(points[n - 1].y)}` },
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
      h('details', { class: 'hub-table-toggle' }, h('summary', { class: 'muted text-sm' }, 'Show as table'),
        h('table', { class: 'hub-table' },
          h('thead', null, h('tr', null, h('th', null, 'Date'), h('th', { class: 'num' }, name))),
          h('tbody', null, rows.map((p) => h('tr', null, h('td', null, p.label), h('td', { class: 'num tabular' }, format(p.y))))))));
  }

  function renderBotTable(stats, botsById) {
    const rows = (Array.isArray(stats.per_bot) ? stats.per_bot : []).filter((r) => r && num(r.played) > 0)
      .sort((a, b) => num(botsById.get(a.bot_id)?.elo) - num(botsById.get(b.bot_id)?.elo));
    const card = h('section', { class: 'card card-flush hub-bot-table' },
      h('div', { class: 'card-header p-4' }, h('div', { class: 'card-title', html: icon('robot') + '<span>Record vs bots</span>' })));
    if (!rows.length) {
      card.appendChild(emptyState({ icon: 'robot', title: 'No bot games yet', text: 'Challenge a bot and track your record against each one.', action: { label: 'Play a bot', href: '#/play', icon: 'play' } }));
      return card;
    }
    card.appendChild(h('div', { class: 'hub-table-scroll' }, h('table', { class: 'hub-table' },
      h('thead', null, h('tr', null, h('th', null, 'Opponent'), h('th', { class: 'num' }, 'Games'), h('th', { class: 'num' }, 'W'), h('th', { class: 'num' }, 'D'), h('th', { class: 'num' }, 'L'), h('th', { class: 'hide-mobile' }, 'Score'), h('th', null, ''))),
      h('tbody', null, rows.map((r) => {
        const b = botsById.get(r.bot_id);
        const played = num(r.played) || 1;
        const pct = (k) => `${(num(r[k]) / played) * 100}%`;
        return h('tr', null,
          h('td', null, h('div', { class: 'row-sm' }, h('span', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, b?.avatar || '🤖'),
            h('div', null, h('div', { class: 'semibold' }, b?.name || r.bot_id), b ? h('div', { class: 'subtle text-xs' }, `Rated ${b.elo}`) : null))),
          h('td', { class: 'num tabular' }, String(num(r.played))),
          h('td', { class: 'num tabular text-primary' }, String(num(r.won))),
          h('td', { class: 'num tabular muted' }, String(num(r.drawn))),
          h('td', { class: 'num tabular text-danger' }, String(num(r.lost))),
          h('td', { class: 'hide-mobile' }, h('div', { class: 'hub-wdl', title: `${r.won} won, ${r.drawn} drawn, ${r.lost} lost` },
            h('span', { class: 'w', style: { width: pct('won') } }), h('span', { class: 'd', style: { width: pct('drawn') } }), h('span', { class: 'l', style: { width: pct('lost') } }))),
          h('td', { class: 'num' }, h('a', { class: 'btn btn-ghost btn-sm', href: `#/play/${encodeURIComponent(r.bot_id)}` }, 'Rematch')));
      })))));
    return card;
  }
}

// ---- Achievements --------------------------------------------------------------
const ACHIEVEMENTS = [
  { id: 'first-game', emoji: '♟️', title: 'First Move', desc: 'Play your first game', get: (c) => [c.played, 1] },
  { id: 'first-win', emoji: '🏆', title: 'First Victory', desc: 'Win a game', get: (c) => [c.won, 1] },
  { id: 'games-10', emoji: '🎲', title: 'Regular', desc: 'Play 10 games', get: (c) => [c.played, 10] },
  { id: 'games-50', emoji: '🏟️', title: 'Seasoned', desc: 'Play 50 games', get: (c) => [c.played, 50] },
  { id: 'wins-25', emoji: '🥇', title: 'Winner', desc: 'Win 25 games', get: (c) => [c.won, 25] },
  { id: 'black-win', emoji: '🌑', title: 'Dark Side', desc: 'Win a game with Black', get: (c) => [c.wonAsBlack, 1] },
  { id: 'draw', emoji: '🤝', title: 'Peacemaker', desc: 'Draw a game', get: (c) => [c.drawn, 1] },
  { id: 'beat-1000', emoji: '🥉', title: 'Club Player', desc: 'Beat a bot rated 1000+', get: (c) => [c.maxBeatElo >= 1000 ? 1 : 0, 1] },
  { id: 'beat-1500', emoji: '🥈', title: 'Tournament Ready', desc: 'Beat a bot rated 1500+', get: (c) => [c.maxBeatElo >= 1500 ? 1 : 0, 1] },
  { id: 'beat-2000', emoji: '👑', title: 'Expert Slayer', desc: 'Beat a bot rated 2000+', get: (c) => [c.maxBeatElo >= 2000 ? 1 : 0, 1] },
  { id: 'bots-5', emoji: '🤖', title: 'Bot Collector', desc: 'Beat 5 different bots', get: (c) => [c.botsBeaten, 5] },
  { id: 'acc-80', emoji: '🎯', title: 'Sharp', desc: 'Play a game with 80%+ accuracy', get: (c) => [Math.min(c.bestAcc, 80), 80] },
  { id: 'acc-90', emoji: '💎', title: 'Precision', desc: 'Play a game with 90%+ accuracy', get: (c) => [Math.min(c.bestAcc, 90), 90] },
  { id: 'puz-10', emoji: '🧩', title: 'Puzzler', desc: 'Solve 10 puzzles', get: (c) => [c.solved, 10] },
  { id: 'puz-100', emoji: '🧠', title: 'Tactician', desc: 'Solve 100 puzzles', get: (c) => [c.solved, 100] },
  { id: 'puz-500', emoji: '⚔️', title: 'Tactics Machine', desc: 'Solve 500 puzzles', get: (c) => [c.solved, 500] },
  { id: 'rating-1500', emoji: '📈', title: 'Rising Star', desc: 'Reach a 1500 puzzle rating', get: (c) => [Math.min(c.rating, 1500), 1500] },
  { id: 'rating-2000', emoji: '🚀', title: 'Puzzle Master', desc: 'Reach a 2000 puzzle rating', get: (c) => [Math.min(c.rating, 2000), 2000] },
  { id: 'rush-15', emoji: '⚡', title: 'Speedy', desc: 'Score 15 in Puzzle Rush', get: (c) => [c.rush, 15] },
  { id: 'rush-30', emoji: '🌪️', title: 'Lightning', desc: 'Score 30 in Puzzle Rush', get: (c) => [c.rush, 30] },
  { id: 'streak-3', emoji: '🔥', title: 'On Fire', desc: 'Practice 3 days in a row', get: (c) => [c.streak, 3] },
  { id: 'streak-7', emoji: '📅', title: 'Weekly Habit', desc: 'Practice 7 days in a row', get: (c) => [c.streak, 7] },
  { id: 'streak-30', emoji: '🗓️', title: 'Dedicated', desc: 'Practice 30 days in a row', get: (c) => [c.streak, 30] },
  { id: 'lesson-1', emoji: '📘', title: 'Student', desc: 'Complete a lesson', get: (c) => [c.lessonsDone, 1] },
  { id: 'lesson-10', emoji: '🎓', title: 'Scholar', desc: 'Complete 10 lessons', get: (c) => [c.lessonsDone, 10] },
  { id: 'course-1', emoji: '🏅', title: 'Graduate', desc: 'Finish a whole course', get: (c) => [c.coursesDone, 1] },
  { id: 'fav', emoji: '⭐', title: 'Curator', desc: 'Star a game in your library', get: (c) => [c.favorites, 1] },
];

function renderAchievements(ctx) {
  const items = ACHIEVEMENTS.map((a) => {
    const [cur, target] = a.get(ctx);
    const value = Math.max(0, Number(cur) || 0);
    return { ...a, value, target, earned: value >= target };
  }).sort((a, b) => Number(b.earned) - Number(a.earned) || (b.value / b.target) - (a.value / a.target));
  const earned = items.filter((i) => i.earned).length;
  return h('section', { class: 'card' },
    h('div', { class: 'card-header' },
      h('div', { class: 'card-title', html: icon('medal') + '<span>Achievements</span>' }),
      h('span', { class: 'badge badge-gold' }, `${earned} / ${items.length} unlocked`)),
    h('div', { class: 'hub-badges' }, items.map((a) => h('div', {
      class: ['hub-badge', a.earned ? 'earned' : 'locked'],
      title: a.earned ? `${a.title} — unlocked!` : `${a.title}: ${a.desc}`,
    },
    h('div', { class: 'hub-badge-icon', 'aria-hidden': 'true' }, a.emoji),
    h('div', { class: 'hub-badge-title' }, a.title),
    h('div', { class: 'hub-badge-desc' }, a.desc),
    a.earned
      ? h('div', { class: 'hub-badge-done', html: icon('check', { size: 14 }) + '<span>Unlocked</span>' })
      : h('div', { class: 'progress progress-sm', role: 'progressbar', 'aria-label': `${a.title} progress`, 'aria-valuenow': String(Math.round(a.value)), 'aria-valuemax': String(a.target) },
        h('div', { class: 'progress-bar', style: { width: `${Math.min(100, (a.value / a.target) * 100)}%` } }))))));
}

// ---- Small pieces ------------------------------------------------------------------
function num(v) { const n = Number(v); return Number.isFinite(n) ? n : 0; }

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
    return emptyState({ icon: 'chart', title: 'No finished games yet', text: 'Your wins, draws and losses will be charted here.', action: { label: 'Play a game', href: '#/play', icon: 'play' } });
  }
  const R = 42; const C = 2 * Math.PI * R; const GAP = total > 1 ? 1.2 : 0;
  const segs = [['won', won, 'Won'], ['drawn', drawn, 'Drawn'], ['lost', lost, 'Lost']].filter((s) => s[1] > 0);
  let offset = 0;
  const circles = segs.map(([k, v, l]) => {
    const len = (v / total) * C;
    const dash = Math.max(0.01, len - (segs.length > 1 ? GAP : 0));
    const c = h('circle', { class: `hub-donut-seg ${k}`, r: R, cx: 50, cy: 50, 'stroke-dasharray': `${dash} ${C - dash}`, 'stroke-dashoffset': String(-offset) },
      h('title', null, `${l}: ${v} (${Math.round((v / total) * 100)}%)`));
    offset += len;
    return c;
  });
  const winRate = Math.round((won / total) * 100);
  return h('div', { class: 'hub-donut' },
    h('div', { class: 'hub-donut-fig' },
      h('svg', { viewBox: '0 0 100 100', role: 'img', 'aria-label': `${won} won, ${drawn} drawn, ${lost} lost` },
        h('circle', { class: 'hub-donut-track', r: R, cx: 50, cy: 50 }),
        h('g', { transform: 'rotate(-90 50 50)' }, circles)),
      h('div', { class: 'hub-donut-center' }, h('div', { class: 'hub-donut-num tabular' }, `${winRate}%`), h('div', { class: 'subtle text-xs' }, 'win rate'))),
    h('ul', { class: 'hub-legend' },
      [['won', won, 'Won'], ['drawn', drawn, 'Drawn'], ['lost', lost, 'Lost']].map(([k, v, l]) =>
        h('li', null, h('span', { class: `hub-swatch ${k}` }), h('span', null, l), h('span', { class: 'spacer' }), h('span', { class: 'semibold tabular' }, String(v)),
          h('span', { class: 'subtle tabular text-sm' }, `${Math.round((v / total) * 100)}%`)))));
}

/** `last_active` is a local calendar date ("YYYY-MM-DD"); compare by calendar day, not UTC instant. */
function formatActiveDay(value) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(value || ''));
  if (!m) return formatRelative(value);
  const day = new Date(+m[1], +m[2] - 1, +m[3]);
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today - day) / 86400000);
  if (days <= 0) return 'today';
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days} days ago`;
  return day.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: day.getFullYear() === now.getFullYear() ? undefined : 'numeric' });
}
