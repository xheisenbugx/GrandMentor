// Home dashboard (#/): greeting + streak, today's plan / goal / streak strip (components/daily.js), big CTA cards, quick-play bots, puzzle rating,
// recent games, tip of the day and a mini daily-puzzle preview.

import { api, isAbort } from '../api.js';
import { h, icon, disposables, skeleton, emptyState, formatRelative, displayName } from '../ui.js';
import { t, hasKey, formatDateIntl } from '../i18n.js';
import {
  ensureHubCss, fenBoardSvg, playUci, resultMarker, outcomeLabel, userAccuracy, accuracyPill,
  gameTitle, fullMoves,
} from './library.js';
import { DailyPanel, ensureDailyCss } from '../components/daily.js';

export const title = () => t('nav.routes.home');

// Tip of the day: ids of home.tips.<id>.{title,body} (resolved at render time).
const TIP_IDS = [
  'center',
  'develop',
  'castle',
  'cct',
  'queenEarly',
  'knightRim',
  'openFiles',
  'count',
  'reason',
  'kingActive',
  'passedPush',
  'rookBehind',
  'tradeAhead',
  'forks',
  'pins',
  'backRank',
  'bishopPair',
  'worstPiece',
  'resign',
  'review',
  'dailyPuzzles',
  'useTime',
  'opposition',
  'matePatterns',
  'playBoard',
  'pawnGrab',
  'doubled',
  'plan',
  'pawnDefense',
  'fun',
];

function dayOfYear(d = new Date()) {
  const start = new Date(d.getFullYear(), 0, 0);
  return Math.floor((d - start) / 86400000);
}

/** i18n key of the greeting sentence for the current hour (each contains a {name} placeholder). */
function greetingKey() {
  const hr = new Date().getHours();
  if (hr < 5) return 'home.greeting.late';
  if (hr < 12) return 'home.greeting.morning';
  if (hr < 18) return 'home.greeting.afternoon';
  return 'home.greeting.evening';
}

/** Render a translated sentence, wrapping the {name} placeholder in <em> (keeps each language's word order). */
function greetingNodes(key, name) {
  const MARK = '\u0000';
  const [before, after = ''] = t(key, { name: MARK }).split(MARK);
  return [before, h('em', null, name), after];
}

export async function mount(root) {
  await Promise.all([ensureHubCss(), ensureDailyCss()]);
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  const opt = { signal };

  // Sections, filled independently as data arrives.
  const heroEl = h('section', { class: 'hub-hero card' }, h('div', { class: 'skeleton skeleton-title', style: 'width:40%' }), h('div', { class: 'skeleton skeleton-text', style: 'width:60%' }));
  const ctaEl = h('section', { class: 'hub-cta-grid', 'aria-label': t('home.quickActions') }, skeletonCards(4));
  const botsEl = h('div', { class: 'hub-bot-row', role: 'list' }, Array.from({ length: 6 }, () => h('div', { class: 'skeleton hub-bot-skel' })));
  const recentEl = h('div', null, skeleton('list', 4));
  const puzzleEl = h('div', { class: 'card hub-daily' }, h('div', { class: 'skeleton skeleton-board' }));
  const ratingEl = h('div', { class: 'card hub-rating-card' }, skeleton('text', 3));
  const tipEl = h('div', { class: 'card hub-tip' });
  const dailyEl = h('div', { class: 'hub-daily-plan' });

  const page = h('div', { class: 'page hub-page hub-home' },
    heroEl,
    dailyEl,
    ctaEl,
    h('section', { class: 'hub-section' },
      h('h2', { class: 'section-title' }, t('home.playABot'), h('a', { href: '#/play' }, t('common.seeAll'))),
      botsEl),
    h('div', { class: 'hub-home-split' },
      h('div', { class: 'stack-lg' },
        h('section', null,
          h('h2', { class: 'section-title' }, t('home.recentGames'), h('a', { href: '#/library' }, t('nav.library'))),
          recentEl),
        tipEl),
      h('aside', { class: 'stack-lg' },
        h('section', null, h('h2', { class: 'section-title' }, t('home.dailyPuzzle'), h('a', { href: '#/puzzles' }, t('home.morePuzzles'))), puzzleEl),
        ratingEl)));
  root.appendChild(page);
  const dailyPanel = new DailyPanel(dailyEl);
  bag.add(() => dailyPanel.destroy());

  // ---- Tip of the day (local, instant) -------------------------------------
  let tipIndex = dayOfYear() % TIP_IDS.length;
  const renderTip = () => {
    const id = TIP_IDS[tipIndex];
    tipEl.replaceChildren(
      h('div', { class: 'hub-tip-icon', 'aria-hidden': 'true', html: icon('hint') }),
      h('div', { class: 'hub-tip-body' },
        h('div', { class: 'hub-eyebrow' }, t('home.tipOfTheDay')),
        h('div', { class: 'hub-tip-title' }, t(`home.tips.${id}.title`)),
        h('p', { class: 'muted' }, t(`home.tips.${id}.body`))),
      h('button', { type: 'button', class: 'btn btn-ghost btn-sm hub-tip-next', 'aria-label': t('home.nextTip'), html: icon('refresh', { size: 16 }) + `<span>${t('home.anotherTip')}</span>`, onClick: () => { tipIndex = (tipIndex + 1) % TIP_IDS.length; renderTip(); } }));
  };
  renderTip();

  // ---- Data (loaded in the background so mount returns its cleanup immediately) ----
  let profile = null; let games = []; let bots = []; let botsById = new Map();
  let progress = []; let courses = []; let daily = null; let offline = false; let gamesR = { ok: false };
  load().catch((e) => { if (!isAbort(e)) console.error('[home]', e); });
  return bag.dispose;

  async function load() {
    const settle = (p) => p.then((v) => ({ ok: true, v }), (e) => ({ ok: false, e }));
    const [profileR, gamesRes, botsR, progressR, coursesR, dailyR] = await Promise.all([
      settle(api.get('/api/profile', opt)),
      settle(api.get('/api/games?limit=6', opt)),
      settle(api.get('/api/bots', opt)),
      settle(api.get('/api/progress', opt)),
      settle(api.get('/api/courses', opt)),
      settle(api.get('/api/puzzles/daily', opt)),
    ]);
    if (signal.aborted) return;
    gamesR = gamesRes;
    profile = profileR.ok && profileR.v ? profileR.v : null;
    games = gamesR.ok && Array.isArray(gamesR.v) ? gamesR.v : [];
    bots = botsR.ok && Array.isArray(botsR.v) ? botsR.v : [];
    botsById = new Map(bots.map((b) => [b.id, b]));
    progress = progressR.ok && Array.isArray(progressR.v) ? progressR.v : [];
    courses = coursesR.ok && Array.isArray(coursesR.v) ? coursesR.v : [];
    daily = dailyR.ok && dailyR.v && typeof dailyR.v === 'object' ? dailyR.v : null;
    offline = !profileR.ok && !gamesR.ok && !botsR.ok;

    renderHero();
    dailyPanel.load({ bots, courses, progress, games, dailyPuzzle: daily, nextLesson: nextLesson() })
      .catch((e) => { if (!isAbort(e)) console.error('[home] daily', e); });
    renderCtas();
    renderBots();
    renderRecent();
    renderRating();
    await renderDaily();
  }

  // ---------------------------------------------------------------------------
  function renderHero() {
    const name = displayName(profile?.name) || t('home.friend');
    const streak = Number(profile?.streak_days) || 0;
    const played = games.length;
    heroEl.replaceChildren(
      h('div', { class: 'hub-hero-main' },
        h('div', { class: 'avatar avatar-lg avatar-round hub-hero-avatar', 'aria-hidden': 'true' }, profile?.avatar || '♟️'),
        h('div', { class: 'stack-sm', style: 'min-width:0' },
          h('div', { class: 'hub-eyebrow' }, formatDateIntl(new Date(), { weekday: 'long', month: 'long', day: 'numeric' })),
          h('h1', { class: 'hub-hero-title' }, greetingNodes(greetingKey(), name)),
          h('p', { class: 'muted hub-hero-sub' }, offline
            ? t('home.hero.offline')
            : played ? t('home.hero.returning') : t('home.hero.welcome')))),
      h('div', { class: 'hub-hero-stats' },
        h('div', { class: ['hub-streak', streak > 0 && 'on'], title: t('home.streakTitle') },
          h('span', { class: 'hub-streak-flame', html: icon('fire') }),
          h('div', null,
            h('div', { class: 'hub-streak-num tabular' }, String(streak)),
            h('div', { class: 'subtle text-xs' }, t('home.dayStreak', { count: streak })))),
        profile ? h('div', { class: 'hub-streak', title: t('home.yourPuzzleRating') },
          h('span', { class: 'hub-streak-flame hub-blue', html: icon('puzzle') }),
          h('div', null,
            h('div', { class: 'hub-streak-num tabular' }, String(Math.round(profile.puzzle_rating || 0))),
            h('div', { class: 'subtle text-xs' }, t('home.puzzleRatingLower')))) : null));
  }

  function nextLesson() {
    if (!courses.length) return null;
    const done = new Set(progress.filter((p) => p.completed).map((p) => `${p.course_id}/${p.lesson_id}`));
    const recent = [...progress].sort((a, b) => String(b.updated_at || '').localeCompare(String(a.updated_at || '')))[0];
    const order = [...courses];
    if (recent) {
      const i = order.findIndex((c) => c.id === recent.course_id);
      if (i > 0) order.unshift(...order.splice(i, 1));
    }
    for (const c of order) {
      const lessons = Array.isArray(c.lessons) ? c.lessons : [];
      const next = lessons.find((l) => !done.has(`${c.id}/${l.id}`));
      if (next) {
        const completed = lessons.filter((l) => done.has(`${c.id}/${l.id}`)).length;
        return { course: c, lesson: next, completed, total: lessons.length || c.lesson_count || 0, started: !!recent };
      }
    }
    return null;
  }

  function ctaCard({ href, iconName, accent, eyebrow, titleText, text, extra, cls }) {
    return h('a', { class: ['card card-link hub-cta', cls], href, style: { '--hub-accent': accent } },
      h('div', { class: 'hub-cta-icon', html: icon(iconName) }),
      h('div', { class: 'hub-cta-body' },
        h('div', { class: 'hub-eyebrow' }, eyebrow),
        h('div', { class: 'hub-cta-title' }, titleText),
        text ? h('p', { class: 'muted text-sm hub-cta-text' }, text) : null,
        extra || null),
      h('span', { class: 'hub-cta-go', 'aria-hidden': 'true', html: icon('chevron-right') }));
  }

  function renderCtas() {
    const last = games[0];
    const nl = nextLesson();
    const cards = [
      ctaCard({ href: '#/play', iconName: 'play', accent: 'var(--primary)', eyebrow: t('nav.play'), titleText: t('home.playABot'), text: t('home.cta.playText'), cls: 'hub-cta-main' }),
      ctaCard({
        href: '#/puzzles/daily', iconName: 'calendar', accent: 'var(--info)', eyebrow: t('home.dailyPuzzle'),
        titleText: daily ? t('home.cta.puzzleOfTheDay') : t('home.cta.solveAPuzzle'),
        text: daily ? `${t('common.rated', { rating: daily.rating })}${Array.isArray(daily.themes) && daily.themes.length ? ' · ' + prettyTheme(daily.themes[0]) : ''}` : t('home.cta.puzzleText'),
      }),
      nl
        ? ctaCard({
          href: `#/learn/${encodeURIComponent(nl.course.id)}/${encodeURIComponent(nl.lesson.id)}`, iconName: 'learn', accent: 'var(--gold)',
          eyebrow: nl.started ? t('home.cta.continueLesson') : t('home.cta.startLearning'), titleText: nl.lesson.title,
          text: `${nl.course.icon || '📘'} ${nl.course.title}`,
          extra: nl.total ? h('div', { class: 'progress progress-sm hub-cta-progress', role: 'progressbar', 'aria-valuenow': String(nl.completed), 'aria-valuemax': String(nl.total) },
            h('div', { class: 'progress-bar', style: { width: `${Math.round((nl.completed / nl.total) * 100)}%` } })) : null,
        })
        : ctaCard({ href: '#/learn', iconName: 'learn', accent: 'var(--gold)', eyebrow: t('nav.learn'), titleText: t('home.cta.lessons'), text: t('home.cta.lessonsText') }),
      last
        ? ctaCard({
          href: `#/review/${last.id}`, iconName: 'sparkles', accent: 'var(--cls-brilliant)', eyebrow: t('nav.routes.review'),
          titleText: t('home.cta.reviewLast'), text: `${gameTitle(last, botsById)} · ${outcomeLabel(last)}`,
        })
        : ctaCard({ href: '#/analysis', iconName: 'analysis', accent: 'var(--cls-brilliant)', eyebrow: t('nav.analysis'), titleText: t('home.cta.analysisBoard'), text: t('home.cta.analysisText') }),
    ];
    ctaEl.replaceChildren(...cards);
  }

  function renderBots() {
    if (!bots.length) {
      botsEl.replaceChildren(h('a', { class: 'card card-sm card-link hub-bot-empty', href: '#/play' }, t('home.chooseOpponent')));
      return;
    }
    const sorted = [...bots].sort((a, b) => (a.elo || 0) - (b.elo || 0));
    botsEl.replaceChildren(...sorted.map((b) => h('a', {
      class: 'hub-bot', href: `#/play/${encodeURIComponent(b.id)}`, role: 'listitem',
      title: b.description || b.name, 'aria-label': t('home.playBotAria', { name: b.name, elo: b.elo }),
    },
    h('div', { class: 'hub-bot-avatar', dataset: { cat: b.category || '' } }, b.avatar || '🤖'),
    h('div', { class: 'hub-bot-name truncate' }, b.name),
    h('div', { class: 'hub-bot-elo tabular' }, b.category === 'coach' ? t('home.coach') : String(b.elo)))));
  }

  function renderRecent() {
    if (!gamesR.ok) {
      recentEl.replaceChildren(h('div', { class: 'card' }, emptyState({ icon: 'wifi-off', title: t('home.recent.unavailableTitle'), text: t('home.recent.unavailableText') })));
      return;
    }
    if (!games.length) {
      recentEl.replaceChildren(h('div', { class: 'card' }, emptyState({ emoji: '♞', title: t('home.recent.emptyTitle'), text: t('home.recent.emptyText'), action: { label: t('home.recent.emptyAction'), href: '#/play', icon: 'play' } })));
      return;
    }
    recentEl.replaceChildren(h('div', { class: 'card card-flush list' }, games.slice(0, 5).map((g) => {
      const bot = g.bot_id ? botsById.get(g.bot_id) : null;
      const meta = [g.opening_name, t('common.moves', { count: fullMoves(g) })].filter(Boolean).join(' · ');
      return h('a', { class: 'list-row', href: `#/review/${g.id}` },
        resultMarker(g),
        bot ? h('span', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, bot.avatar || '🤖') : null,
        h('div', { class: 'list-row-main' },
          h('div', { class: 'list-row-title' }, gameTitle(g, botsById)),
          h('div', { class: 'list-row-sub' }, meta)),
        accuracyPill(userAccuracy(g)),
        h('div', { class: 'list-row-meta hide-mobile' }, formatRelative(g.created_at)),
        h('span', { class: 'subtle', html: icon('chevron-right') }));
    })));
  }

  function renderRating() {
    if (!profile) {
      ratingEl.replaceChildren(h('div', { class: 'card-title' }, t('home.puzzleRating')), h('p', { class: 'muted' }, t('home.rating.noRating')));
      return;
    }
    const rating = Math.round(profile.puzzle_rating || 0);
    const solved = Number(profile.puzzles_solved) || 0;
    const failed = Number(profile.puzzles_failed) || 0;
    const rate = solved + failed ? Math.round((solved / (solved + failed)) * 100) : 0;
    ratingEl.replaceChildren(
      h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('puzzle') + `<span>${t('nav.puzzles')}</span>` }),
        h('a', { class: 'btn btn-ghost btn-sm', href: '#/puzzles/rush', html: icon('bolt', { size: 16 }) + `<span>${t('home.rating.rush')}</span>` })),
      h('div', { class: 'hub-rating-row' },
        h('div', null,
          h('div', { class: 'hub-rating-num tabular' }, String(rating)),
          h('div', { class: 'subtle text-xs' }, profile.puzzle_rd ? t('home.rating.confidence', { rd: Math.round(profile.puzzle_rd) }) : t('home.puzzleRating'))),
        h('div', { class: 'progress-ring', style: { '--value': rate }, title: t('home.rating.successRate') }, `${rate}%`)),
      h('div', { class: 'hub-mini-stats' },
        miniStat(t('home.rating.solved'), solved), miniStat(t('home.rating.missed'), failed), miniStat(t('home.rating.rushBest'), Number(profile.rush_best) || 0)),
      h('a', { class: 'btn btn-secondary btn-block', href: '#/puzzles', html: icon('target') + `<span>${t('home.rating.train')}</span>` }));
  }

  async function renderDaily() {
    if (!daily || !daily.fen) {
      puzzleEl.replaceChildren(emptyState({ icon: 'puzzle', title: t('home.dailyPuzzle'), text: t('home.daily.unavailable'), action: { label: t('home.daily.tryPuzzles'), href: '#/puzzles', kind: 'secondary' } }));
      return;
    }
    let pos = { fen: daily.fen, lastMove: null, turn: 'white' };
    try {
      const pre = Array.isArray(daily.moves) && daily.moves.length ? [daily.moves[0]] : [];
      pos = await playUci(daily.fen, pre);
    } catch { /* fallback to raw fen */ }
    if (signal.aborted) return;
    const slot = h('a', { class: 'hub-daily-board', href: '#/puzzles/daily', 'aria-label': t('home.daily.open') });
    puzzleEl.replaceChildren(
      slot,
      h('div', { class: 'hub-daily-info' },
        h('div', { class: 'row-sm' },
          h('span', { class: ['hub-turn', pos.turn === 'black' && 'black'] }),
          h('span', { class: 'semibold' }, pos.turn === 'black' ? t('common.blackToMove') : t('common.whiteToMove')),
          h('span', { class: 'spacer' }),
          h('span', { class: 'badge badge-info' }, t('common.rated', { rating: daily.rating ?? '?' }))),
        Array.isArray(daily.themes) && daily.themes.length
          ? h('div', { class: 'chip-row hub-themes' }, daily.themes.slice(0, 3).map((th) => h('span', { class: 'badge' }, prettyTheme(th))))
          : null,
        h('a', { class: 'btn btn-primary btn-block', href: '#/puzzles/daily', html: icon('play') + `<span>${t('home.daily.solve')}</span>` })));
    // Prefer the real (non-interactive) Board; fall back to the static SVG thumbnail.
    const lastMove = pos.lastMove ? [pos.lastMove.slice(0, 2), pos.lastMove.slice(2, 4)] : null;
    try {
      const { Board } = await import('../components/board.js');
      if (signal.aborted || bag.disposed) return;
      const holder = h('div', { class: 'hub-daily-board-inner' });
      slot.appendChild(holder);
      const board = new Board(holder, { fen: pos.fen, orientation: pos.turn, interactive: false, movableColor: null, sounds: false });
      bag.add(() => board.destroy());
      if (lastMove) board.setPosition(pos.fen, { animate: false, lastMove });
    } catch {
      if (signal.aborted) return;
      slot.replaceChildren();
      slot.innerHTML = fenBoardSvg(pos.fen, { orientation: pos.turn, lastMove, label: t('home.daily.positionLabel') });
    }
  }
}

function miniStat(label, value) {
  return h('div', { class: 'hub-mini-stat' }, h('div', { class: 'hub-mini-stat-v tabular' }, String(value)), h('div', { class: 'subtle text-xs' }, label));
}

function skeletonCards(n) {
  return Array.from({ length: n }, () => h('div', { class: 'skeleton skeleton-card' }));
}

/** Display name of a puzzle theme id: translated via themes.<id> when known, else prettified. */
export function prettyTheme(theme) {
  const id = String(theme || '');
  if (id && hasKey(`themes.${id}`)) return t(`themes.${id}`);
  const s = id.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[_-]+/g, ' ').trim();
  return s ? s[0].toUpperCase() + s.slice(1) : '';
}

