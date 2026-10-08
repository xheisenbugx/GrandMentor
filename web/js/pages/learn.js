// Learn hub (#/learn) and course page (#/learn/:courseId).
// Also exports small shared helpers used by lesson.js, openings.js and endgames.js.
// Contract: docs/CONTRACT.md §4 (/api/courses, /api/progress) and §5 (page interface).

import { h, icon, pageHeader, disposables, emptyState, skeleton, escapeHtml } from '../ui.js';
import { api, isAbort } from '../api.js';
import { getSettings, pieceUrl } from '../settings.js';
import { Chess } from '/vendor/chess.js';
import { t } from '../i18n.js';

export const title = (params) => (params && params.courseId ? t('learn.courseTitle') : t('learn.title'));

// ===========================================================================
// Shared helpers (exported)
// ===========================================================================

export const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';

/** Inject /css/learn.css once (idempotent, kept for the session — it is tiny). */
export function ensureLearnCss() {
  if (document.querySelector('link[data-gm-css="learn"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/learn.css';
  link.dataset.gmCss = 'learn';
  document.head.appendChild(link);
}

/** localStorage helpers: never throw, always return a usable value. */
export function readStore(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const v = JSON.parse(raw);
    return v && typeof v === 'object' ? v : fallback;
  } catch { return fallback; }
}
export function writeStore(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* private mode / quota */ }
}

/** First four FEN fields (position identity, ignoring move counters). */
export function fenKey(fen) {
  return String(fen || '').trim().split(/\s+/).slice(0, 4).join(' ');
}

export function sideToMove(fen) {
  const parts = String(fen || '').trim().split(/\s+/);
  return parts[1] === 'b' ? 'black' : 'white';
}

/** New chess.js instance at `fen` (or null if the FEN is invalid). */
export function chessAt(fen) {
  try { return new Chess(!fen || fen === 'start' ? START_FEN : fen); } catch { return null; }
}

/** Apply a UCI move to a chess.js instance; returns the move object or null if illegal. */
export function applyUci(chess, uci) {
  if (!chess || typeof uci !== 'string' || uci.length < 4) return null;
  const from = uci.slice(0, 2).toLowerCase();
  const to = uci.slice(2, 4).toLowerCase();
  const promotion = uci.length > 4 ? uci[4].toLowerCase() : undefined;
  try {
    return chess.move({ from, to, promotion });
  } catch {
    // Castling written as king-takes-rook (e1h1) — try the standard square.
    const piece = chess.get(from);
    if (piece && piece.type === 'k') {
      const alt = { e1h1: 'g1', e1a1: 'c1', e8h8: 'g8', e8a8: 'c8' }[from + to];
      if (alt) { try { return chess.move({ from, to: alt }); } catch { /* fallthrough */ } }
    }
    return null;
  }
}

/** UCI string for a chess.js move object. */
export function moveUci(m) {
  return m ? m.from + m.to + (m.promotion || '') : '';
}

/** Play a list of UCI moves from a FEN. Returns { fens:[fen0..fenN], sans:[], ucis:[] } (stops at first illegal move). */
export function playLine(startFen, ucis) {
  const chess = chessAt(startFen);
  const out = { fens: [chess ? chess.fen() : START_FEN], sans: [], ucis: [] };
  if (!chess) return out;
  for (const u of ucis || []) {
    const m = applyUci(chess, u);
    if (!m) break;
    out.sans.push(m.san);
    out.ucis.push(moveUci(m));
    out.fens.push(chess.fen());
  }
  return out;
}

/** Convert space-separated SAN to UCI from `startFen`. */
export function sanLineToUci(san, startFen = START_FEN) {
  const chess = chessAt(startFen);
  const ucis = [];
  if (!chess) return ucis;
  for (const tok of String(san || '').split(/\s+/)) {
    const tk = tok.replace(/^\d+\.(\.\.)?/, '').trim();
    if (!tk) continue;
    try {
      const m = chess.move(tk);
      ucis.push(moveUci(m));
    } catch { break; }
  }
  return ucis;
}

/** Does playing `expectedUci` from `fenBefore` lead to `fenAfter`? (robust to castling/promo notation) */
export function isSameMove(fenBefore, expectedUci, fenAfter) {
  const c = chessAt(fenBefore);
  if (!c) return false;
  const m = applyUci(c, expectedUci);
  return !!m && fenKey(c.fen()) === fenKey(fenAfter);
}

/** Static board thumbnail as an SVG string (64 squares, piece images). */
export function miniBoardSvg(fen, orientation = 'white') {
  const board = String(fen && fen !== 'start' ? fen : START_FEN).split(' ')[0].split('/');
  const flip = orientation === 'black';
  let squares = '';
  let pieces = '';
  for (let r = 0; r < 8; r++) {
    for (let f = 0; f < 8; f++) {
      if ((r + f) % 2 === 1) {
        const x = flip ? 7 - f : f; const y = flip ? 7 - r : r;
        squares += `<rect class="d" x="${x}" y="${y}" width="1" height="1"/>`;
      }
    }
  }
  for (let r = 0; r < 8 && r < board.length; r++) {
    let f = 0;
    for (const ch of board[r]) {
      if (f > 7) break;
      if (/[1-8]/.test(ch)) { f += Number(ch); continue; }
      if (!/[prnbqkPRNBQK]/.test(ch)) { f++; continue; }
      const code = (ch === ch.toUpperCase() ? 'w' : 'b') + ch.toUpperCase();
      const x = flip ? 7 - f : f; const y = flip ? 7 - r : r;
      pieces += `<image href="${escapeHtml(pieceUrl(code))}" x="${x}" y="${y}" width="1" height="1"/>`;
      f++;
    }
  }
  return `<svg class="lrn-mini-board" viewBox="0 0 8 8" role="img" aria-label="${escapeHtml(t('learn.boardAria'))}" preserveAspectRatio="xMidYMid meet" shape-rendering="crispEdges"><rect class="l" width="8" height="8"/>${squares}${pieces}</svg>`;
}

/** Progress ring node with a centred label. */
export function progressRing(pct, { size = 52, label } = {}) {
  const v = Math.max(0, Math.min(100, Math.round(pct || 0)));
  const done = v >= 100;
  return h('div', { class: ['lrn-ring', done && 'done'], style: { '--value': v, '--size': size + 'px' }, role: 'img', 'aria-label': t('learn.ringAria', { pct: v }) },
    h('div', { class: 'lrn-ring-label', html: done ? icon('check') : escapeHtml(label ?? `${v}%`) }));
}

export function levelPill(level) {
  const l = String(level || 'beginner').toLowerCase();
  const cls = ['beginner', 'intermediate', 'advanced', 'master'].includes(l) ? l : 'beginner';
  return h('span', { class: `badge level-${cls}` }, cls === l ? t(`learn.levels.${cls}`) : l.charAt(0).toUpperCase() + l.slice(1));
}

/**
 * A small self-cleaning timer set: later(fn, ms) schedules; clear() cancels everything pending.
 * Fired timers remove themselves, so the set never grows unbounded.
 */
export function timerSet() {
  const ids = new Set();
  return {
    later(fn, ms) {
      const id = setTimeout(() => { ids.delete(id); fn(); }, ms);
      ids.add(id);
      return id;
    },
    cancel(id) { clearTimeout(id); ids.delete(id); },
    clear() { for (const id of ids) clearTimeout(id); ids.clear(); },
    get size() { return ids.size; },
  };
}

/** Confetti-lite burst over `host` (position:relative). Self-removes via `timers` (a timerSet). */
export function confetti(host, timers, { count = 36 } = {}) {
  if (!host) return;
  if (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  host.querySelectorAll(':scope > .lrn-confetti').forEach((n) => n.remove());
  const colors = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--cls-brilliant)', 'var(--cls-mistake)', 'var(--accent)'];
  const layer = h('div', { class: 'lrn-confetti', 'aria-hidden': 'true' });
  for (let i = 0; i < count; i++) {
    const angle = Math.random() * Math.PI * 2;
    const dist = 90 + Math.random() * 180;
    layer.appendChild(h('i', {
      style: {
        '--x': `${Math.cos(angle) * dist}px`,
        '--y': `${Math.sin(angle) * dist - 60}px`,
        '--r': `${Math.round(Math.random() * 720 - 360)}deg`,
        '--c': colors[i % colors.length],
        '--delay': `${Math.round(Math.random() * 120)}ms`,
        width: `${6 + Math.round(Math.random() * 5)}px`,
      },
    }));
  }
  host.appendChild(layer);
  timers.later(() => layer.remove(), 1700);
}

/** Restartable CSS class animation (e.g. 'shake', 'flash-bad'). */
export function flashClass(el, cls, timers, ms = 420) {
  if (!el) return;
  el.classList.remove(cls);
  void el.offsetWidth; // restart animation
  el.classList.add(cls);
  timers.later(() => el.classList.remove(cls), ms);
}

let soundMod = null;
let soundLoading = null;
/** Play a UI sound via components/sound.js (lazy, optional, honours the `sounds` setting). */
export function sfx(name) {
  if (!getSettings().sounds) return;
  if (soundMod) { try { soundMod.playSound(name); } catch { /* ignore */ } return; }
  if (!soundLoading) {
    soundLoading = import('../components/sound.js').then((m) => { if (typeof m.playSound === 'function') soundMod = m; }).catch(() => {});
  }
  soundLoading.then(() => { if (soundMod) { try { soundMod.playSound(name); } catch { /* ignore */ } } });
}

/** Feedback line helper: kind = good|bad|info|warn. `html` is trusted (escape inputs yourself / use mdLite). */
export function setFeedback(el, kind, html) {
  if (!el) return;
  if (!html) { el.className = 'lrn-feedback'; el.replaceChildren(); return; }
  const ic = { good: 'check-circle', bad: 'x-circle', info: 'info', warn: 'hint' }[kind] || 'info';
  el.className = `lrn-feedback ${kind}`;
  el.innerHTML = icon(ic) + `<div>${html}</div>`;
  // restart entrance animation
  el.style.animation = 'none'; void el.offsetWidth; el.style.animation = '';
}

/** Breadcrumb trail: [{label, href?}]. */
export function breadcrumbs(items) {
  return h('nav', { class: 'breadcrumbs', 'aria-label': t('learn.breadcrumbAria') },
    items.flatMap((b, i) => [
      i > 0 ? h('span', { html: icon('chevron-right'), style: 'display:contents' }) : null,
      b.href ? h('a', { href: b.href }, b.label) : h('span', null, b.label),
    ]));
}

/** A page-level loading / error block. */
export function errorBlock(message, retry) {
  return emptyState({
    icon: 'alert', title: t('learn.error.title'), text: message || t('learn.error.text'),
    action: retry ? { label: t('learn.error.tryAgain'), icon: 'refresh', onClick: retry } : { label: t('learn.error.backToLearn'), href: '#/learn' },
  });
}

// ===========================================================================
// Learn data
// ===========================================================================

// `label` / `blurb` are getters so they are translated at render time (never at import time).
const category = (key, emoji) => ({
  key, emoji,
  get label() { return t(`learn.categories.${key}.label`); },
  get blurb() { return t(`learn.categories.${key}.blurb`); },
});
export const CATEGORIES = [
  category('basics', '♟️'),
  category('openings', '📖'),
  category('middlegame', '⚔️'),
  category('tactics', '⚡'),
  category('strategy', '🧠'),
  category('endgame', '🏁'),
];
const LEVEL_ORDER = { beginner: 0, intermediate: 1, advanced: 2, master: 3 };
const LAST_KEY = 'gm.learn.last.v1';

/** Remember the lesson the user opened most recently (for "continue where you left off"). */
export function rememberLesson(courseId, lessonId, step = 0) {
  writeStore(LAST_KEY, { course_id: String(courseId), lesson_id: String(lessonId), step: Number(step) || 0, at: Date.now() });
}

function completedSet(progress) {
  const set = new Set();
  for (const p of Array.isArray(progress) ? progress : []) {
    if (p && p.completed) set.add(`${p.course_id}/${p.lesson_id}`);
  }
  return set;
}

function courseLessons(c) {
  return Array.isArray(c && c.lessons) ? c.lessons : [];
}

function courseStats(course, done) {
  const lessons = courseLessons(course);
  const total = lessons.length || Number(course.lesson_count) || 0;
  let completed = 0;
  let next = null;
  for (const l of lessons) {
    if (done.has(`${course.id}/${l.id}`)) completed++;
    else if (!next) next = l;
  }
  return { total, completed, pct: total ? (completed / total) * 100 : 0, next };
}

/** Pick the lesson to continue: last opened (if unfinished) → after most recent progress → first lesson overall. */
function pickContinue(courses, progress, done) {
  const last = readStore(LAST_KEY, null);
  if (last && last.course_id) {
    const c = courses.find((x) => x.id === last.course_id);
    const l = c && courseLessons(c).find((x) => x.id === last.lesson_id);
    if (c && l && !done.has(`${c.id}/${l.id}`)) return { course: c, lesson: l, kind: 'resume' };
  }
  const sorted = (Array.isArray(progress) ? progress : []).filter((p) => p && p.completed)
    .sort((a, b) => String(b.updated_at || '').localeCompare(String(a.updated_at || '')));
  if (sorted.length) {
    const startIdx = Math.max(0, courses.findIndex((c) => c.id === sorted[0].course_id));
    for (let i = 0; i < courses.length; i++) {
      const c = courses[(startIdx + i) % courses.length];
      const st = courseStats(c, done);
      if (st.next) return { course: c, lesson: st.next, kind: 'next' };
    }
    return null; // everything done
  }
  const first = courses.find((c) => courseLessons(c).length);
  return first ? { course: first, lesson: courseLessons(first)[0], kind: 'start' } : null;
}

function sortCourses(list) {
  const catIdx = (c) => { const i = CATEGORIES.findIndex((x) => x.key === c.category); return i < 0 ? 99 : i; };
  return [...list].sort((a, b) => catIdx(a) - catIdx(b)
    || (LEVEL_ORDER[a.level] ?? 9) - (LEVEL_ORDER[b.level] ?? 9)
    || String(a.title).localeCompare(String(b.title)));
}

// ===========================================================================
// Page
// ===========================================================================

export async function mount(root, { params = {} } = {}) {
  ensureLearnCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());

  const page = h('div', { class: 'page' });
  root.appendChild(page);

  const load = async () => {
    page.replaceChildren(skeletonHeader(), h('div', { class: 'lrn-course-grid mt-6' }, skeleton('card', 6)));
    try {
      if (params.courseId) await renderCourse(page, params.courseId, ctrl.signal);
      else await renderHub(page, ctrl.signal, bag);
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      page.replaceChildren(errorBlock(e && e.message, load));
    }
  };
  await load();

  return () => bag.dispose();
}

function skeletonHeader() {
  return h('div', { class: 'stack-sm' }, h('div', { class: 'skeleton skeleton-title' }), h('div', { class: 'skeleton skeleton-text', style: 'max-width:360px' }));
}

async function fetchProgress(signal) {
  try { return await api.get('/api/progress', { signal }); } catch (e) { if (isAbort(e)) throw e; return []; }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------
async function renderHub(page, signal, bag) {
  const [coursesRaw, progress] = await Promise.all([api.get('/api/courses', { signal }), fetchProgress(signal)]);
  const courses = sortCourses(Array.isArray(coursesRaw) ? coursesRaw : []);
  const done = completedSet(progress);

  const header = pageHeader({
    title: t('learn.title'), icon: 'learn',
    subtitle: t('learn.subtitle'),
    actions: [
      h('a', { class: 'btn btn-secondary', href: '#/openings', html: icon('openings') + `<span>${escapeHtml(t('learn.openings'))}</span>` }),
      h('a', { class: 'btn btn-secondary', href: '#/endgames', html: icon('endgames') + `<span>${escapeHtml(t('learn.endgameDrills'))}</span>` }),
    ],
  });

  if (!courses.length) {
    page.replaceChildren(header, emptyState({ emoji: '📚', title: t('learn.noCourses.title'), text: t('learn.noCourses.text') }));
    return;
  }

  // Totals
  let totalLessons = 0; let totalDone = 0; let coursesDone = 0;
  for (const c of courses) {
    const st = courseStats(c, done);
    totalLessons += st.total; totalDone += st.completed;
    if (st.total && st.completed >= st.total) coursesDone++;
  }

  // Continue card
  const cont = pickContinue(courses, progress, done);
  let hero;
  if (cont) {
    const st = courseStats(cont.course, done);
    const kicker = cont.kind === 'start' ? t('learn.hero.start') : t('learn.hero.resume');
    hero = h('section', { class: 'lrn-hero mt-4' },
      h('div', { class: 'lrn-hero-emoji', 'aria-hidden': 'true' }, cont.course.icon || '♟️'),
      h('div', { style: 'min-width:0' },
        h('div', { class: 'lrn-hero-kicker' }, kicker),
        h('div', { class: 'lrn-hero-title' }, cont.lesson.title),
        h('p', { class: 'lrn-hero-sub' }, `${cont.course.title} · ${cont.lesson.summary || t('learn.hero.nextLesson')}`),
        h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: `width:${st.pct.toFixed(0)}%` }))),
      h('a', {
        class: 'btn btn-primary btn-lg',
        href: `#/learn/${encodeURIComponent(cont.course.id)}/${encodeURIComponent(cont.lesson.id)}`,
        html: icon('play') + `<span>${escapeHtml(cont.kind === 'resume' ? t('learn.hero.btnResume') : cont.kind === 'start' ? t('learn.hero.btnStart') : t('learn.hero.btnContinue'))}</span>`,
      }));
  } else {
    hero = h('section', { class: 'lrn-hero mt-4' },
      h('div', { class: 'lrn-hero-emoji', 'aria-hidden': 'true' }, '🏆'),
      h('div', null,
        h('div', { class: 'lrn-hero-kicker' }, t('learn.hero.allDone')),
        h('div', { class: 'lrn-hero-title' }, t('learn.hero.allDoneTitle')),
        h('p', { class: 'lrn-hero-sub' }, t('learn.hero.allDoneSub'))),
      h('a', { class: 'btn btn-primary btn-lg', href: '#/puzzles', html: icon('puzzle') + `<span>${escapeHtml(t('learn.hero.solvePuzzles'))}</span>` }));
  }

  const stats = h('div', { class: 'lrn-stats' },
    statTile(t('learn.stats.lessonsCompleted'), `${totalDone}/${totalLessons}`),
    statTile(t('learn.stats.coursesFinished'), `${coursesDone}/${courses.length}`),
    statTile(t('learn.stats.overall'), `${totalLessons ? Math.round((totalDone / totalLessons) * 100) : 0}%`));

  // Category filter chips
  const present = CATEGORIES.filter((cat) => courses.some((c) => c.category === cat.key));
  const others = courses.filter((c) => !CATEGORIES.some((cat) => cat.key === c.category));
  let active = 'all';
  const chips = h('div', { class: 'chip-row mt-6', role: 'toolbar', 'aria-label': t('learn.filterAria') });
  const sections = h('div');

  const renderChips = () => {
    chips.replaceChildren(
      chip(t('learn.all'), 'all'),
      ...present.map((cat) => chip(`${cat.emoji} ${cat.label}`, cat.key)));
  };
  const chip = (label, key) => h('button', {
    class: ['chip', active === key && 'active'], type: 'button', 'aria-pressed': active === key ? 'true' : 'false',
    onClick: () => { active = key; renderChips(); renderSections(); },
  }, label);

  const renderSections = () => {
    const groups = [...present.map((cat) => ({ cat, list: courses.filter((c) => c.category === cat.key) }))];
    if (others.length) groups.push({ cat: { key: 'other', label: t('learn.moreCourses'), emoji: '✨', blurb: '' }, list: others });
    sections.replaceChildren(...groups
      .filter((g) => active === 'all' || g.cat.key === active)
      .map(({ cat, list }) => h('section', { class: 'lrn-section' },
        h('div', { class: 'lrn-section-head' },
          h('span', { class: 'lrn-section-emoji', 'aria-hidden': 'true' }, cat.emoji),
          h('div', null,
            h('h2', null, cat.label, ' ', h('span', { class: 'lrn-count' }, `· ${t('learn.courseCount', { count: list.length })}`)),
            cat.blurb ? h('div', { class: 'muted text-sm' }, cat.blurb) : null)),
        h('div', { class: 'lrn-course-grid' }, list.map((c) => courseCard(c, done))))));
  };

  renderChips();
  renderSections();
  page.replaceChildren(header, hero, stats, chips, sections);
  void bag;
}

function statTile(label, value) {
  return h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value' }, value));
}

function courseCard(c, done) {
  const st = courseStats(c, done);
  const lessonsLabel = t('learn.lessonCount', { count: st.total });
  const status = st.completed >= st.total && st.total
    ? h('span', { class: 'text-primary semibold' }, t('learn.completed'))
    : st.completed ? h('span', null, t('learn.doneOf', { done: st.completed, total: st.total })) : h('span', null, t('learn.notStarted'));
  return h('a', { class: `card card-link lrn-course-card lrn-cat-${escapeHtml(c.category || 'basics')}`, href: `#/learn/${encodeURIComponent(c.id)}` },
    h('div', { class: 'lrn-course-top' },
      h('div', { class: 'lrn-course-icon', 'aria-hidden': 'true' }, c.icon || '♟️'),
      h('div', { style: 'min-width:0' },
        h('h3', { class: 'lrn-course-title' }, c.title),
        levelPill(c.level)),
      progressRing(st.pct)),
    h('p', { class: 'lrn-course-desc' }, c.description || ''),
    h('div', { class: 'lrn-course-foot' }, h('span', { class: 'row-sm', html: icon('book', { size: 16 }) + `<span>${escapeHtml(lessonsLabel)}</span>` }), status));
}

// ---------------------------------------------------------------------------
// Course page
// ---------------------------------------------------------------------------
async function renderCourse(page, courseId, signal) {
  let course;
  try {
    [course] = await Promise.all([api.get(`/api/courses/${encodeURIComponent(courseId)}`, { signal })]);
  } catch (e) {
    if (isAbort(e)) throw e;
    if (e && e.status === 404) {
      page.replaceChildren(emptyState({ emoji: '🔍', title: t('learn.course.notFound'), text: t('learn.course.notFoundText'), action: { label: t('learn.error.backToLearn'), href: '#/learn', icon: 'learn' } }));
      return;
    }
    throw e;
  }
  const progress = await fetchProgress(signal);
  const done = completedSet(progress);
  const lessons = courseLessons(course);
  const st = courseStats(course, done);
  const cat = CATEGORIES.find((x) => x.key === course.category);
  const lessonHref = (l) => `#/learn/${encodeURIComponent(course.id)}/${encodeURIComponent(l.id)}`;

  const crumbs = breadcrumbs([{ label: t('learn.title'), href: '#/learn' }, { label: cat ? cat.label : t('learn.courseTitle'), href: '#/learn' }, { label: course.title }]);
  crumbs.classList.add('mb-4');

  const allDone = st.total > 0 && st.completed >= st.total;
  const cta = st.next
    ? h('a', { class: 'btn btn-primary btn-lg', href: lessonHref(st.next), html: icon('play') + `<span>${escapeHtml(st.completed ? t('learn.course.continue') : t('learn.course.startCourse'))}</span>` })
    : lessons.length ? h('a', { class: 'btn btn-secondary btn-lg', href: lessonHref(lessons[0]), html: icon('refresh') + `<span>${escapeHtml(t('learn.course.reviewFromStart'))}</span>` }) : null;

  const hero = h('section', { class: 'card lrn-course-hero' },
    h('div', { class: 'lrn-hero-emoji', 'aria-hidden': 'true' }, course.icon || '♟️'),
    h('div', { style: 'min-width:0' },
      h('h1', null, course.title),
      h('p', null, course.description || ''),
      h('div', { class: 'lrn-course-meta' },
        levelPill(course.level),
        cat ? h('span', { class: 'badge' }, `${cat.emoji} ${cat.label}`) : null,
        h('span', { class: 'muted text-sm' }, t('learn.lessonCount', { count: st.total })),
        allDone ? h('span', { class: 'badge badge-primary', html: icon('check') + `<span>${escapeHtml(t('learn.completed'))}</span>` }) : null),
      h('div', { class: 'lrn-course-actions' },
        cta,
        h('div', { class: 'stack-sm', style: 'flex:1 1 200px;max-width:360px' },
          h('div', { class: 'between row text-sm muted' }, h('span', null, t('learn.course.progress')), h('span', { class: 'tabular' }, `${st.completed}/${st.total}`)),
          h('div', { class: 'progress' }, h('div', { class: 'progress-bar', style: `width:${st.pct.toFixed(0)}%` }))))));

  const list = lessons.length
    ? h('div', { class: 'card card-flush lrn-lessons' }, lessons.map((l, i) => {
      const isDone = done.has(`${course.id}/${l.id}`);
      const isNext = st.next && st.next.id === l.id;
      return h('a', { class: ['lrn-lesson-row', isNext && 'next'], href: lessonHref(l) },
        isDone ? h('span', { class: 'lrn-check', html: icon('check'), 'aria-label': t('learn.completed') }) : h('span', { class: 'lrn-lesson-num' }, String(i + 1)),
        h('div', { class: 'lrn-lesson-main' },
          h('div', { class: 'lrn-lesson-title' }, l.title),
          l.summary ? h('div', { class: 'lrn-lesson-sub' }, l.summary) : null),
        isNext ? h('span', { class: 'btn btn-primary btn-sm' }, st.completed ? t('learn.course.continue') : t('learn.course.start'))
          : isDone ? h('span', { class: 'btn btn-ghost btn-sm' }, t('learn.course.review')) : null,
        h('span', { html: icon('chevron-right'), style: 'display:contents' }));
    }))
    : emptyState({ emoji: '🛠️', title: t('learn.course.noLessons'), text: t('learn.course.noLessonsText') });

  page.replaceChildren(crumbs, hero, h('h2', { class: 'section-title mt-6' }, t('learn.course.lessons')), list);
}
