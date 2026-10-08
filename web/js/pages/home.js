// Home dashboard (#/): greeting + streak, big CTA cards, quick-play bots, puzzle rating,
// recent games, tip of the day and a mini daily-puzzle preview.

import { api, isAbort } from '../api.js';
import { h, icon, disposables, skeleton, emptyState, formatRelative } from '../ui.js';
import {
  ensureHubCss, fenBoardSvg, playUci, resultMarker, outcomeLabel, userAccuracy, accuracyPill,
  gameTitle, fullMoves,
} from './library.js';

export const title = 'Home';

const TIPS = [
  ['Control the center', 'Pawns and pieces on e4, d4, e5 and d5 control the most squares. Fight for the center early.'],
  ['Develop before you attack', 'Bring out your knights and bishops before moving the same piece twice or launching an attack.'],
  ['Castle early', 'Castling tucks your king away and connects your rooks. Most strong players castle in the first 10 moves.'],
  ['Check, captures, threats', 'Before every move, look for checks, captures and threats — for both you and your opponent.'],
  ['Don’t bring the queen out too early', 'An early queen gets chased around by enemy pieces, which lets your opponent develop with tempo.'],
  ['Knights on the rim are dim', 'A knight on the edge of the board controls only half as many squares as one in the center.'],
  ['Rooks love open files', 'Put your rooks on files with no pawns. From there they can invade your opponent’s position.'],
  ['Count before you capture', 'Count attackers and defenders on a square before you trade. Make sure you come out ahead.'],
  ['Every move has a reason', 'Ask “why did my opponent play that?” after each move. Most blunders come from skipping this question.'],
  ['Activate your king in the endgame', 'When the queens are off, the king becomes a strong piece. March it to the center!'],
  ['Passed pawns must be pushed', 'A pawn with no enemy pawns in front of it can become a queen. Support it and push it.'],
  ['Rooks belong behind passed pawns', 'Whether it is your passed pawn or your opponent’s, a rook behind it is usually best placed.'],
  ['Trade when you are ahead', 'If you are up material, trading pieces (not pawns) makes your extra material count even more.'],
  ['Look for forks', 'A fork attacks two things at once. Knights are especially good at forking king and queen.'],
  ['Pins paralyze pieces', 'A pinned piece can’t move without exposing something more valuable behind it. Pile up on it!'],
  ['Watch the back rank', 'If your king is stuck behind its own pawns, a rook or queen check on the back rank can be mate. Make some luft.'],
  ['Bishop pair is a small advantage', 'Two bishops work beautifully together in open positions. Avoid trading one cheaply.'],
  ['Improve your worst piece', 'When you don’t know what to do, find your least active piece and give it a better square.'],
  ['Don’t resign too early', 'Beginners and bots blunder too. Keep fighting and set problems for your opponent.'],
  ['Review every game', 'The fastest way to improve is to review your games and understand your mistakes. Try Game Review!'],
  ['Solve puzzles daily', 'Ten minutes of tactics a day trains your pattern recognition faster than anything else.'],
  ['Use your time', 'In longer games, take a moment on critical moves — captures, checks and big pawn moves.'],
  ['Opposition wins pawn endings', 'Kings facing each other with one square between: the side NOT to move has the opposition.'],
  ['Learn checkmate patterns', 'Back-rank mate, smothered mate, Anastasia’s mate… knowing patterns helps you spot them in games.'],
  ['Play the board, not the rating', 'Strong opponents make mistakes too. Focus on the position in front of you.'],
  ['Don’t grab every pawn', 'Pawn grabbing with your queen can cost you development time or even get the queen trapped.'],
  ['Doubled pawns aren’t always bad', 'They can open files for your rooks. Judge the position, not just the pawn shape.'],
  ['Make a plan', 'Look at pawn structure to choose a plan: attack where you have more space.'],
  ['Defend with pawns carefully', 'Pawns can’t move backwards. Every pawn move leaves weak squares behind.'],
  ['Have fun!', 'Chess is a game. Play openings you enjoy and celebrate your brilliant moves!'],
];

function dayOfYear(d = new Date()) {
  const start = new Date(d.getFullYear(), 0, 0);
  return Math.floor((d - start) / 86400000);
}

function greeting() {
  const hr = new Date().getHours();
  if (hr < 5) return 'Up late';
  if (hr < 12) return 'Good morning';
  if (hr < 18) return 'Good afternoon';
  return 'Good evening';
}

export async function mount(root) {
  await ensureHubCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  const opt = { signal };

  // Sections, filled independently as data arrives.
  const heroEl = h('section', { class: 'hub-hero card' }, h('div', { class: 'skeleton skeleton-title', style: 'width:40%' }), h('div', { class: 'skeleton skeleton-text', style: 'width:60%' }));
  const ctaEl = h('section', { class: 'hub-cta-grid', 'aria-label': 'Quick actions' }, skeletonCards(4));
  const botsEl = h('div', { class: 'hub-bot-row', role: 'list' }, Array.from({ length: 6 }, () => h('div', { class: 'skeleton hub-bot-skel' })));
  const recentEl = h('div', null, skeleton('list', 4));
  const puzzleEl = h('div', { class: 'card hub-daily' }, h('div', { class: 'skeleton skeleton-board' }));
  const ratingEl = h('div', { class: 'card hub-rating-card' }, skeleton('text', 3));
  const tipEl = h('div', { class: 'card hub-tip' });

  const page = h('div', { class: 'page hub-page hub-home' },
    heroEl,
    ctaEl,
    h('section', { class: 'hub-section' },
      h('h2', { class: 'section-title' }, 'Play a bot', h('a', { href: '#/play' }, 'See all')),
      botsEl),
    h('div', { class: 'hub-home-split' },
      h('div', { class: 'stack-lg' },
        h('section', null,
          h('h2', { class: 'section-title' }, 'Recent games', h('a', { href: '#/library' }, 'Library')),
          recentEl),
        tipEl),
      h('aside', { class: 'stack-lg' },
        h('section', null, h('h2', { class: 'section-title' }, 'Daily puzzle', h('a', { href: '#/puzzles' }, 'More puzzles')), puzzleEl),
        ratingEl)));
  root.appendChild(page);

  // ---- Tip of the day (local, instant) -------------------------------------
  let tipIndex = dayOfYear() % TIPS.length;
  const renderTip = () => {
    const [t, body] = TIPS[tipIndex];
    tipEl.replaceChildren(
      h('div', { class: 'hub-tip-icon', 'aria-hidden': 'true', html: icon('hint') }),
      h('div', { class: 'hub-tip-body' },
        h('div', { class: 'hub-eyebrow' }, 'Tip of the day'),
        h('div', { class: 'hub-tip-title' }, t),
        h('p', { class: 'muted' }, body)),
      h('button', { type: 'button', class: 'btn btn-ghost btn-sm hub-tip-next', 'aria-label': 'Next tip', html: icon('refresh', { size: 16 }) + '<span>Another</span>', onClick: () => { tipIndex = (tipIndex + 1) % TIPS.length; renderTip(); } }));
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
    renderCtas();
    renderBots();
    renderRecent();
    renderRating();
    await renderDaily();
  }

  // ---------------------------------------------------------------------------
  function renderHero() {
    const name = profile?.name?.trim() || 'friend';
    const streak = Number(profile?.streak_days) || 0;
    const played = games.length;
    heroEl.replaceChildren(
      h('div', { class: 'hub-hero-main' },
        h('div', { class: 'avatar avatar-lg avatar-round hub-hero-avatar', 'aria-hidden': 'true' }, profile?.avatar || '♟️'),
        h('div', { class: 'stack-sm', style: 'min-width:0' },
          h('div', { class: 'hub-eyebrow' }, new Date().toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' })),
          h('h1', { class: 'hub-hero-title' }, `${greeting()}, `, h('em', null, name), '!'),
          h('p', { class: 'muted hub-hero-sub' }, offline
            ? 'We can’t reach the GrandMentor server right now. Start it and refresh to load your progress.'
            : played ? 'Ready for another game? Every game makes you a little stronger.' : 'Welcome to GrandMentor! Play a friendly bot, solve a puzzle or start a lesson.'))),
      h('div', { class: 'hub-hero-stats' },
        h('div', { class: ['hub-streak', streak > 0 && 'on'], title: 'Days in a row you have practiced' },
          h('span', { class: 'hub-streak-flame', html: icon('fire') }),
          h('div', null,
            h('div', { class: 'hub-streak-num tabular' }, String(streak)),
            h('div', { class: 'subtle text-xs' }, 'day streak'))),
        profile ? h('div', { class: 'hub-streak', title: 'Your puzzle rating' },
          h('span', { class: 'hub-streak-flame hub-blue', html: icon('puzzle') }),
          h('div', null,
            h('div', { class: 'hub-streak-num tabular' }, String(Math.round(profile.puzzle_rating || 0))),
            h('div', { class: 'subtle text-xs' }, 'puzzle rating'))) : null));
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
      ctaCard({ href: '#/play', iconName: 'play', accent: 'var(--primary)', eyebrow: 'Play', titleText: 'Play a bot', text: 'From total beginner to grandmaster — pick an opponent your size.', cls: 'hub-cta-main' }),
      ctaCard({
        href: '#/puzzles/daily', iconName: 'calendar', accent: 'var(--info)', eyebrow: 'Daily puzzle',
        titleText: daily ? `Puzzle of the day` : 'Solve a puzzle',
        text: daily ? `Rated ${daily.rating}${Array.isArray(daily.themes) && daily.themes.length ? ' · ' + prettyTheme(daily.themes[0]) : ''}` : 'Sharpen your tactics with today’s puzzle.',
      }),
      nl
        ? ctaCard({
          href: `#/learn/${encodeURIComponent(nl.course.id)}/${encodeURIComponent(nl.lesson.id)}`, iconName: 'learn', accent: 'var(--gold)',
          eyebrow: nl.started ? 'Continue lesson' : 'Start learning', titleText: nl.lesson.title,
          text: `${nl.course.icon || '📘'} ${nl.course.title}`,
          extra: nl.total ? h('div', { class: 'progress progress-sm hub-cta-progress', role: 'progressbar', 'aria-valuenow': String(nl.completed), 'aria-valuemax': String(nl.total) },
            h('div', { class: 'progress-bar', style: { width: `${Math.round((nl.completed / nl.total) * 100)}%` } })) : null,
        })
        : ctaCard({ href: '#/learn', iconName: 'learn', accent: 'var(--gold)', eyebrow: 'Learn', titleText: 'Lessons', text: 'Openings, tactics, strategy and endgames — step by step.' }),
      last
        ? ctaCard({
          href: `#/review/${last.id}`, iconName: 'sparkles', accent: 'var(--cls-brilliant)', eyebrow: 'Game review',
          titleText: 'Review last game', text: `${gameTitle(last, botsById)} · ${outcomeLabel(last)}`,
        })
        : ctaCard({ href: '#/analysis', iconName: 'analysis', accent: 'var(--cls-brilliant)', eyebrow: 'Analysis', titleText: 'Analysis board', text: 'Explore any position with the engine and your mentor.' }),
    ];
    ctaEl.replaceChildren(...cards);
  }

  function renderBots() {
    if (!bots.length) {
      botsEl.replaceChildren(h('a', { class: 'card card-sm card-link hub-bot-empty', href: '#/play' }, 'Choose an opponent →'));
      return;
    }
    const sorted = [...bots].sort((a, b) => (a.elo || 0) - (b.elo || 0));
    botsEl.replaceChildren(...sorted.map((b) => h('a', {
      class: 'hub-bot', href: `#/play/${encodeURIComponent(b.id)}`, role: 'listitem',
      title: b.description || b.name, 'aria-label': `Play ${b.name}, rated ${b.elo}`,
    },
    h('div', { class: 'hub-bot-avatar', dataset: { cat: b.category || '' } }, b.avatar || '🤖'),
    h('div', { class: 'hub-bot-name truncate' }, b.name),
    h('div', { class: 'hub-bot-elo tabular' }, b.category === 'coach' ? 'Coach' : String(b.elo)))));
  }

  function renderRecent() {
    if (!gamesR.ok) {
      recentEl.replaceChildren(h('div', { class: 'card' }, emptyState({ icon: 'wifi-off', title: 'Games unavailable', text: 'We couldn’t load your recent games.' })));
      return;
    }
    if (!games.length) {
      recentEl.replaceChildren(h('div', { class: 'card' }, emptyState({ emoji: '♞', title: 'No games yet', text: 'Your games are saved automatically, so you can review them with your mentor afterwards.', action: { label: 'Play your first game', href: '#/play', icon: 'play' } })));
      return;
    }
    recentEl.replaceChildren(h('div', { class: 'card card-flush list' }, games.slice(0, 5).map((g) => {
      const bot = g.bot_id ? botsById.get(g.bot_id) : null;
      const meta = [g.opening_name, `${fullMoves(g)} moves`].filter(Boolean).join(' · ');
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
      ratingEl.replaceChildren(h('div', { class: 'card-title' }, 'Puzzle rating'), h('p', { class: 'muted' }, 'Solve puzzles to get a rating.'));
      return;
    }
    const rating = Math.round(profile.puzzle_rating || 0);
    const solved = Number(profile.puzzles_solved) || 0;
    const failed = Number(profile.puzzles_failed) || 0;
    const rate = solved + failed ? Math.round((solved / (solved + failed)) * 100) : 0;
    ratingEl.replaceChildren(
      h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('puzzle') + '<span>Puzzles</span>' }),
        h('a', { class: 'btn btn-ghost btn-sm', href: '#/puzzles/rush', html: icon('bolt', { size: 16 }) + '<span>Rush</span>' })),
      h('div', { class: 'hub-rating-row' },
        h('div', null,
          h('div', { class: 'hub-rating-num tabular' }, String(rating)),
          h('div', { class: 'subtle text-xs' }, profile.puzzle_rd ? `± ${Math.round(profile.puzzle_rd)} confidence` : 'Puzzle rating')),
        h('div', { class: 'progress-ring', style: { '--value': rate }, title: 'Success rate' }, `${rate}%`)),
      h('div', { class: 'hub-mini-stats' },
        miniStat('Solved', solved), miniStat('Missed', failed), miniStat('Rush best', Number(profile.rush_best) || 0)),
      h('a', { class: 'btn btn-secondary btn-block', href: '#/puzzles', html: icon('target') + '<span>Train tactics</span>' }));
  }

  async function renderDaily() {
    if (!daily || !daily.fen) {
      puzzleEl.replaceChildren(emptyState({ icon: 'puzzle', title: 'Daily puzzle', text: 'Today’s puzzle isn’t available right now.', action: { label: 'Try puzzles', href: '#/puzzles', kind: 'secondary' } }));
      return;
    }
    let pos = { fen: daily.fen, lastMove: null, turn: 'white' };
    try {
      const pre = Array.isArray(daily.moves) && daily.moves.length ? [daily.moves[0]] : [];
      pos = await playUci(daily.fen, pre);
    } catch { /* fallback to raw fen */ }
    if (signal.aborted) return;
    const slot = h('a', { class: 'hub-daily-board', href: '#/puzzles/daily', 'aria-label': 'Open the daily puzzle' });
    const toMove = pos.turn === 'white' ? 'White' : 'Black';
    puzzleEl.replaceChildren(
      slot,
      h('div', { class: 'hub-daily-info' },
        h('div', { class: 'row-sm' },
          h('span', { class: ['hub-turn', pos.turn === 'black' && 'black'] }),
          h('span', { class: 'semibold' }, `${toMove} to move`),
          h('span', { class: 'spacer' }),
          h('span', { class: 'badge badge-info' }, `Rated ${daily.rating ?? '?'}`)),
        Array.isArray(daily.themes) && daily.themes.length
          ? h('div', { class: 'chip-row hub-themes' }, daily.themes.slice(0, 3).map((t) => h('span', { class: 'badge' }, prettyTheme(t))))
          : null,
        h('a', { class: 'btn btn-primary btn-block', href: '#/puzzles/daily', html: icon('play') + '<span>Solve it</span>' })));
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
      slot.innerHTML = fenBoardSvg(pos.fen, { orientation: pos.turn, lastMove, label: 'Daily puzzle position' });
    }
  }
}

function miniStat(label, value) {
  return h('div', { class: 'hub-mini-stat' }, h('div', { class: 'hub-mini-stat-v tabular' }, String(value)), h('div', { class: 'subtle text-xs' }, label));
}

function skeletonCards(n) {
  return Array.from({ length: n }, () => h('div', { class: 'skeleton skeleton-card' }));
}

export function prettyTheme(t) {
  const s = String(t || '').replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[_-]+/g, ' ').trim();
  return s ? s[0].toUpperCase() + s.slice(1) : '';
}

