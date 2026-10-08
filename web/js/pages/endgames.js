// Endgame theory: drills grouped by theme (#/endgames) and a drill page (#/endgames/:id) with a short
// "key idea" lesson followed by practice against the engine at full strength.
//
// Practice: the opponent is the engine (POST /api/engine/analyze, 500 ms; falls back to the
// strongest bot). Each attempt starts from the drill's main position or one of its variants. The
// drill's `success` decides when an attempt is solved (mate / safe promotion / bare king / holding
// the draw for N moves); failures (lost or drawn evaluation, draw rule, move limit) explain what
// went wrong. Attempts are recorded with POST /api/training/:id/attempt; 3 successes in a row
// master a drill. Attempts that used help (best-move hint or takeback) are not recorded.

import { h, icon, disposables, pageHeader, emptyState, skeleton, mdLite, escapeHtml, formatSan, loadingBlock } from '../ui.js';
import { api, isAbort } from '../api.js';
import { getSetting } from '../settings.js';
import { Board } from '../components/board.js';
import {
  ensureLearnCss, chessAt, applyUci, moveUci, sideToMove, fenKey, miniBoardSvg, levelPill, timerSet,
  confetti, flashClass, sfx, setFeedback, breadcrumbs, errorBlock,
} from './learn.js';
import { t } from '../i18n.js';

export const title = (params) => (params && params.id ? t('endgames.drillTitle') : t('endgames.title'));

const ENGINE_MOVETIME_MS = 500;
const BASELINE_MOVETIME_MS = 400;
const HINT_MOVETIME_MS = 700;
const MIN_THINK_MS = 350;
const LOST_CP = -500;          // user-POV eval at or below which a drill counts as lost…
const LOST_MARGIN_CP = 600;    // …or this much worse than the start position (draw drills start "worse")
const DRAWN_CP = 25;           // |eval| at or below this (no mate) for…
const DRAWN_STREAK = 2;        // …this many engine replies in a row = the win slipped into a draw
const DEFAULT_MASTERY = 3;
const ARROW_COLORS = new Set(['green', 'red', 'blue', 'yellow']);

const CATEGORY_ORDER = ['basic', 'pawn', 'rook', 'queen', 'minor'];
const CATEGORY_EMOJI = { basic: '👑', pawn: '♟️', rook: '🏰', queen: '👸', minor: '🐴' };
const catLabel = (c) => (CATEGORY_ORDER.includes(c) ? t(`endgames.categories.${c}.label`) : c);
const catBlurb = (c) => (CATEGORY_ORDER.includes(c) ? t(`endgames.categories.${c}.blurb`) : '');
const catRank = (c) => { const i = CATEGORY_ORDER.indexOf(c); return i < 0 ? 99 : i; };

function ensureEndgamesCss() {
  if (document.querySelector('link[data-gm-css="endgames"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/endgames.css';
  link.dataset.gmCss = 'endgames';
  document.head.appendChild(link);
}

function goalOf(d) { return d && d.goal === 'draw' ? 'draw' : 'win'; }
function goalBadge(goal) {
  return h('span', { class: `lrn-goal ${goal}` }, goal === 'draw' ? t('endgames.goal.draw') : t('endgames.goal.win'));
}
function sortDrills(list) {
  const lv = { beginner: 0, intermediate: 1, advanced: 2, master: 3 };
  return [...list].sort((a, b) => catRank(a.category) - catRank(b.category)
    || (lv[a.level] ?? 9) - (lv[b.level] ?? 9) || String(a.title).localeCompare(String(b.title)));
}
function validDrills(raw) {
  return sortDrills((Array.isArray(raw) ? raw : []).filter((d) => d && d.id && chessAt(d.fen)));
}
function drillFens(d) {
  const vs = (Array.isArray(d.variants) ? d.variants : []).filter((f) => typeof f === 'string' && chessAt(f));
  return [d.fen, ...vs];
}
function successOf(d) {
  const s = d && d.success && typeof d.success === 'object' ? d.success : {};
  const kinds = ['mate', 'promote', 'bare_king', 'hold'];
  const kind = kinds.includes(s.kind) ? s.kind : (goalOf(d) === 'draw' ? 'hold' : 'mate');
  const moves = Number.isFinite(s.moves) && s.moves > 0 ? Math.min(100, s.moves) : (kind === 'hold' ? 30 : 50);
  return { kind, moves };
}
/** replaceChildren that skips null/false (native replaceChildren would print "null"). */
function put(el, ...kids) { el.replaceChildren(...kids.filter((k) => k != null && k !== false && k !== '')); }
function stripP(html) { return html.replace(/^<p>|<\/p>$/g, ''); }

/** Progress dots ●●○ towards mastery (+ medal when mastered). */
function masteryDots(p, need) {
  const streak = p ? Math.min(need, p.streak || 0) : 0;
  const mastered = !!(p && p.mastered);
  const filled = mastered ? need : streak;
  const dots = h('span', { class: ['eg-dots', mastered && 'mastered'], role: 'img', 'aria-label': mastered ? t('endgames.masteredAria') : t('endgames.dotsAria', { count: streak, need }) },
    Array.from({ length: need }, (_, i) => h('i', { class: i < filled ? 'on' : null })));
  return dots;
}
function masteryBadge() {
  return h('span', { class: 'eg-mastered', html: icon('medal', { size: 14 }) + `<span>${escapeHtml(t('endgames.mastered'))}</span>` });
}

async function loadProgress(signal) {
  try {
    const res = await api.get('/api/training', { signal });
    const map = new Map();
    for (const p of (res && Array.isArray(res.drills) ? res.drills : [])) if (p && p.drill_id) map.set(p.drill_id, p);
    const need = Number(res && res.mastery_streak) || DEFAULT_MASTERY;
    return { map, need, ok: true };
  } catch (e) {
    if (isAbort(e)) throw e;
    return { map: new Map(), need: DEFAULT_MASTERY, ok: false };
  }
}

export async function mount(root, { params = {} } = {}) {
  ensureLearnCss();
  ensureEndgamesCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  try {
    if (params.id) await mountDrill(root, params.id, bag, ctrl.signal);
    else await mountList(root, bag, ctrl.signal);
  } catch (e) {
    if (!isAbort(e) && !bag.disposed) root.replaceChildren(h('div', { class: 'page' }, errorBlock(e && e.message)));
  }
  return () => bag.dispose();
}

// ===========================================================================
// List
// ===========================================================================
async function mountList(root, bag, signal) {
  const page = h('div', { class: 'page' });
  root.appendChild(page);
  const header = pageHeader({ title: t('endgames.listTitle'), icon: 'endgames', subtitle: t('endgames.subtitle') });
  page.replaceChildren(header, h('div', { class: 'eg-grid mt-6' }, skeleton('card', 6)));

  const [raw, prog] = await Promise.all([api.get('/api/endgames', { signal }), loadProgress(signal)]);
  if (bag.disposed) return;
  const drills = validDrills(raw);
  if (!drills.length) {
    page.replaceChildren(header, emptyState({ emoji: '🏁', title: t('endgames.noDrills.title'), text: t('endgames.noDrills.text') }));
    return;
  }
  const { map, need } = prog;
  const isMastered = (d) => !!(map.get(d.id) && map.get(d.id).mastered);
  const doneCount = drills.filter(isMastered).length;
  const pct = Math.round((doneCount / drills.length) * 100);

  // Next: a drill in progress (attempted, not mastered) first, then the first untried one.
  const next = drills.find((d) => !isMastered(d) && map.get(d.id) && map.get(d.id).attempts > 0)
    || drills.find((d) => !isMastered(d));
  const hero = h('section', { class: 'lrn-hero mt-4' },
    h('div', { class: 'lrn-hero-emoji', 'aria-hidden': 'true' }, next ? '🏁' : '🏆'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'lrn-hero-kicker' }, next ? (map.get(next.id) && map.get(next.id).attempts ? t('endgames.keepPractising') : t('endgames.nextDrill')) : t('endgames.allMastered')),
      h('div', { class: 'lrn-hero-title' }, next ? next.title : t('endgames.masteredAll')),
      h('p', { class: 'lrn-hero-sub' }, t('endgames.masteredCount', { done: doneCount, total: drills.length })),
      h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: `width:${pct}%` }))),
    next ? h('a', { class: 'btn btn-primary btn-lg', href: `#/endgames/${encodeURIComponent(next.id)}`, html: icon('play') + `<span>${escapeHtml(t('endgames.startDrill'))}</span>` }) : null);

  const cats = [...new Set(drills.map((d) => d.category))].sort((a, b) => catRank(a) - catRank(b));
  let active = 'all';
  const chips = h('div', { class: 'chip-row mt-6' });
  const sections = h('div');
  const renderChips = () => chips.replaceChildren(
    ...[['all', t('endgames.all')], ...cats.map((c) => [c, `${CATEGORY_EMOJI[c] || '♟️'} ${catLabel(c)}`])].map(([k, label]) =>
      h('button', { type: 'button', class: ['chip', active === k && 'active'], 'aria-pressed': active === k ? 'true' : 'false', onClick: () => { active = k; renderChips(); renderSections(); } }, label)));
  const renderSections = () => sections.replaceChildren(...cats.filter((c) => active === 'all' || c === active).map((c) => {
    const list = drills.filter((d) => d.category === c);
    const done = list.filter(isMastered).length;
    const gpct = Math.round((done / list.length) * 100);
    return h('section', { class: 'lrn-section' },
      h('div', { class: 'lrn-section-head eg-group-head' },
        h('span', { class: 'lrn-section-emoji', 'aria-hidden': 'true' }, CATEGORY_EMOJI[c] || '♟️'),
        h('div', { class: 'eg-group-text' },
          h('h2', null, catLabel(c)),
          catBlurb(c) ? h('div', { class: 'muted text-sm' }, catBlurb(c)) : null),
        h('div', { class: 'eg-group-progress' },
          h('span', { class: 'text-sm' }, t('endgames.groupMastered', { done, total: list.length })),
          h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: `width:${gpct}%` })))),
      h('div', { class: 'eg-grid' }, list.map((d) => drillCard(d, map.get(d.id), need))));
  }));
  renderChips();
  renderSections();
  const offline = prog.ok ? null : h('div', { class: 'eg-note mt-4', role: 'status' }, t('endgames.progressOffline'));
  put(page, header, offline, hero, chips, sections);
}

function drillCard(d, p, need) {
  const mastered = !!(p && p.mastered);
  const attempts = p ? p.attempts || 0 : 0;
  return h('a', { class: ['card card-link eg-card', mastered && 'is-mastered'], href: `#/endgames/${encodeURIComponent(d.id)}`, title: d.title },
    mastered ? h('span', { class: 'lrn-check eg-card-medal', html: icon('medal'), 'aria-label': t('endgames.masteredAria') }) : null,
    h('div', { html: miniBoardSvg(d.fen, sideToMove(d.fen)) }),
    h('h3', { class: 'eg-card-title' }, d.title),
    d.description ? h('p', { class: 'eg-card-desc' }, d.description) : null,
    h('div', { class: 'eg-card-meta' }, goalBadge(goalOf(d)), levelPill(d.level)),
    h('div', { class: 'eg-card-progress' },
      masteryDots(p, need),
      h('span', { class: 'text-xs subtle' }, attempts ? t('endgames.attempts', { count: attempts }) : t('endgames.notTried'))));
}

// ===========================================================================
// Drill: lesson + practice
// ===========================================================================
async function mountDrill(root, id, bag, signal) {
  root.appendChild(loadingBlock(t('endgames.loading')));
  const [raw, prog] = await Promise.all([api.get('/api/endgames', { signal }), loadProgress(signal)]);
  if (bag.disposed) return;
  const drills = validDrills(raw);
  const drill = drills.find((d) => d.id === id);
  if (!drill) {
    root.replaceChildren(h('div', { class: 'page' }, emptyState({ emoji: '🔍', title: t('endgames.notFound.title'), text: t('endgames.notFound.text'), action: { label: t('endgames.allDrills'), href: '#/endgames' } })));
    return;
  }
  const need = prog.need;
  let progress = prog.map.get(drill.id) || null;
  const nextDrill = drills[drills.indexOf(drill) + 1] || null;
  const goal = goalOf(drill);
  const success = successOf(drill);
  const fens = drillFens(drill).map((f) => chessAt(f).fen());
  const userColor = sideToMove(fens[0]);
  const engineColor = userColor === 'white' ? 'black' : 'white';
  const userChar = userColor === 'white' ? 'w' : 'b';
  const oppChar = userChar === 'w' ? 'b' : 'w';
  const lessonSteps = (Array.isArray(drill.lesson) ? drill.lesson : []).filter((s) => s && typeof s.text === 'string');
  const technique = Array.isArray(drill.technique) ? drill.technique.filter(Boolean) : [];

  const timers = timerSet();
  bag.add(() => timers.clear());
  let engineCtrl = null;
  let hintCtrl = null;
  let baseCtrl = null;
  let saveCtrl = null;
  bag.add(() => { for (const c of [engineCtrl, hintCtrl, baseCtrl, saveCtrl]) if (c) c.abort(); });

  // ------------------------------------------------------------------ state
  let phase = 'lesson';      // lesson | practice
  let lessonIdx = 0;
  let variantIdx = 0;
  let startFen = fens[0];
  let game = chessAt(startFen);
  let sans = [];
  let status = 'play';       // play | won | lost
  let busy = false;
  let gen = 0;
  let hintStage = 0;
  let assisted = false;      // best-move hint or takeback used in this attempt
  let recorded = false;      // this attempt was already sent to the server
  let drawnStreak = 0;
  let baseline = null;       // user-POV cp of the start position (null until known)
  let attemptNo = 0;
  let oppHadMaterial = false; // the engine's side had more than a king at the start
  const hintCache = new Map();

  // ------------------------------------------------------------------ layout
  const boardSlot = h('div', { class: 'board-slot' });
  const boardWrap = h('div', { class: 'board-row lrn-board-wrap' }, boardSlot);
  const thinking = h('span', { class: 'eg-thinking', hidden: true, 'aria-label': t('endgames.thinkingAria') }, h('i'), h('i'), h('i'));
  const topBar = h('div', { class: 'player-bar' },
    h('div', { class: 'avatar', 'aria-hidden': 'true' }, '🤖'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'player-name' }, t('endgames.engine'), ' ', h('span', { class: 'player-rating' }, t('endgames.fullStrength'))),
      h('div', { class: 'player-captures' }, engineColor === 'white' ? t('endgames.white') : t('endgames.black'))),
    h('div', { class: 'clock-slot' }, thinking));
  const bottomBar = h('div', { class: 'player-bar' },
    h('div', { class: 'avatar', 'aria-hidden': 'true' }, '🙂'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'player-name' }, t('endgames.you')),
      h('div', { class: 'player-captures' }, userColor === 'white' ? t('endgames.white') : t('endgames.black'))));

  const goalText = t(`endgames.banner.${userColor}${goal === 'win' ? 'Win' : 'Draw'}`);
  const banner = h('div', { class: `eg-banner ${goal}` },
    goalBadge(goal),
    h('div', null, h('div', { class: 'eg-banner-title' }, goalText), h('div', { class: 'eg-banner-sub' }, t(`endgames.successKind.${success.kind}`, { count: success.moves }))));

  const masteryBox = h('div', { class: 'eg-mastery' });
  const renderMastery = () => masteryBox.replaceChildren(masteryDots(progress, need),
    progress && progress.mastered ? masteryBadge() : h('span', { class: 'text-xs subtle' }, progress && progress.attempts ? t('endgames.attempts', { count: progress.attempts }) : t('endgames.notTried')));
  renderMastery();

  const panelHeader = h('div', { class: 'panel-header' },
    h('span', { 'aria-hidden': 'true' }, CATEGORY_EMOJI[drill.category] || '🏁'),
    h('span', { class: 'truncate' }, drill.title), h('span', { class: 'spacer' }), levelPill(drill.level));
  const body = h('div', { class: 'panel-body' });
  const footer = h('div', { class: 'panel-footer' });
  const panel = h('div', { class: 'panel grow' }, panelHeader, body, footer);
  const head = breadcrumbs([{ label: t('endgames.title'), href: '#/endgames' }, { label: catLabel(drill.category) }, { label: drill.title }]);
  const layout = h('div', { class: 'game-layout no-eval eg-drill' },
    h('div', { class: 'game-main' }, topBar, boardWrap, bottomBar),
    h('aside', { class: 'game-panel' }, head, panel));
  root.replaceChildren(layout);

  const board = new Board(boardSlot, {
    fen: startFen, orientation: userColor, interactive: false, movableColor: null,
    onMove: (mv) => onUserMove(mv),
  });
  bag.add(() => board.destroy());

  const sleep = (ms) => new Promise((resolve) => timers.later(resolve, ms));
  const sanOf = (s) => formatSan(s, getSetting('moveNotation'));
  const userMoveCount = () => Math.ceil(sans.length / 2); // the user always moves first
  const btn = (cls, iconName, label, onClick, extra = {}) => h('button', { class: `btn ${cls}`, type: 'button', html: (iconName ? icon(iconName) : '') + `<span>${escapeHtml(label)}</span>`, onClick, ...extra });
  const playLink = (fen) => h('a', { class: 'btn btn-ghost', href: `#/play?fen=${encodeURIComponent(fen)}&color=${userChar}`, html: icon('robot') + `<span>${escapeHtml(t('endgames.playBot'))}</span>` });

  // ================================================================== lesson
  function showLesson(i = 0) {
    phase = 'lesson';
    cancelEngine();
    lessonIdx = Math.max(0, Math.min(lessonSteps.length - 1, i));
    const step = lessonSteps[lessonIdx];
    if (!step) { startPractice(true); return; }
    const fen = step.fen && chessAt(step.fen) ? step.fen : fens[0];
    board.setInteractive(false, null);
    board.setPosition(fen, { animate: true, sound: false });
    board.setArrows((Array.isArray(step.arrows) ? step.arrows : [])
      .filter((a) => a && /^[a-h][1-8]$/.test(a.from) && /^[a-h][1-8]$/.test(a.to))
      .map((a) => ({ from: a.from, to: a.to, color: ARROW_COLORS.has(a.color) ? a.color : 'green' })));
    board.setHighlights((Array.isArray(step.highlights) ? step.highlights : [])
      .filter((s) => typeof s === 'string' && /^[a-h][1-8]$/.test(s)).map((square) => ({ square, kind: 'hint' })));
    thinking.hidden = true;

    const last = lessonIdx === lessonSteps.length - 1;
    put(body,
      banner,
      h('div', { class: 'eg-phase' },
        h('span', { class: 'eg-phase-kicker' }, t('endgames.lesson.kicker')),
        h('span', { class: 'text-xs subtle' }, t('endgames.lesson.step', { n: lessonIdx + 1, total: lessonSteps.length }))),
      h('div', { class: 'eg-steps', 'aria-hidden': 'true' }, lessonSteps.map((_, k) => h('i', { class: k <= lessonIdx ? 'on' : null }))),
      h('div', { class: 'lrn-coach' },
        h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'),
        h('div', { class: 'lrn-text md', html: mdLite(step.text) })),
      lessonIdx === 0 && drill.description ? h('p', { class: 'muted text-sm eg-desc', html: stripP(mdLite(drill.description)) }) : null,
      masteryBox);
    put(footer,
      btn('btn-ghost', 'chevron-left', t('endgames.lesson.back'), () => showLesson(lessonIdx - 1), { disabled: lessonIdx === 0 }),
      h('span', { class: 'spacer' }),
      last ? null : btn('btn-ghost', null, t('endgames.lesson.skip'), () => startPractice(true)),
      last
        ? btn('btn-primary', 'play', t('endgames.lesson.start'), () => startPractice(true))
        : btn('btn-primary', 'chevron-right', t('endgames.lesson.next'), () => showLesson(lessonIdx + 1)));
  }

  // ================================================================== practice
  const feedback = h('div', { class: 'lrn-feedback', role: 'status', 'aria-live': 'polite' });
  const resultBox = h('div');
  const movesEl = h('div', { class: 'eg-movelist', 'aria-label': t('endgames.movesAria') });
  const counterEl = h('span', { class: 'text-xs subtle' });
  const positionEl = h('span', { class: 'text-xs subtle' });
  const hintBtn = btn('btn-ghost', 'hint', t('endgames.hint'), () => showHint());
  const iconBtn = (iconName, label, onClick) => h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(iconName), onClick });
  const undoBtn = iconBtn('undo', t('endgames.undo'), () => undo());
  const resetBtn = iconBtn('refresh', t('endgames.reset'), () => newAttempt(false));
  const lessonBtn = lessonSteps.length ? iconBtn('book', t('endgames.lesson.review'), () => showLesson(0)) : null;
  const flipBtn = iconBtn('flip', t('endgames.flip'), () => board.flip());
  const botLinkSlot = h('div', { class: 'eg-links' });

  function renderPracticePanel() {
    put(body,
      banner,
      h('div', { class: 'eg-phase' },
        h('span', { class: 'eg-phase-kicker' }, t('endgames.practice.kicker')),
        positionEl, h('span', { class: 'spacer' }), counterEl),
      masteryBox,
      feedback,
      resultBox,
      technique.length ? h('div', null,
        h('div', { class: 'op-block-title', html: icon('hint') + `<span>${escapeHtml(t('endgames.technique'))}</span>` }),
        h('ol', { class: 'eg-technique' }, technique.map((s) => h('li', { html: `<span>${stripP(mdLite(s))}</span>` })))) : null,
      h('div', { class: 'op-block-title', html: icon('list') + `<span>${escapeHtml(t('endgames.moves'))}</span>` }),
      movesEl,
      botLinkSlot);
    (lessonBtn || flipBtn).classList.add('eg-push');
    put(footer, hintBtn, undoBtn, resetBtn, lessonBtn, flipBtn);
  }

  function pickVariant(fresh) {
    if (!fresh || fens.length === 1) return variantIdx;
    // First attempt ever: the main (lesson) position. Afterwards: a random different one.
    if (!progress || !progress.attempts) return 0;
    let k = Math.floor(Math.random() * (fens.length - 1));
    if (k >= variantIdx) k++;
    return k;
  }

  function startPractice(fresh) {
    phase = 'practice';
    renderPracticePanel();
    newAttempt(fresh);
  }

  function newAttempt(fresh) {
    cancelEngine();
    attemptNo++;
    variantIdx = pickVariant(fresh);
    startFen = fens[variantIdx];
    game = chessAt(startFen);
    oppHadMaterial = piecesOf(oppChar).some((p) => p.type !== 'k');
    sans = [];
    status = 'play';
    hintStage = 0;
    assisted = false;
    recorded = false;
    drawnStreak = 0;
    hintCache.clear();
    board.setPosition(startFen, { animate: true, sound: false });
    board.clearArrows(); board.setHighlights([]);
    resultBox.replaceChildren();
    setFeedback(feedback, null);
    botLinkSlot.replaceChildren(playLink(startFen));
    positionEl.textContent = fens.length > 1 ? t('endgames.practice.position', { n: variantIdx + 1, total: fens.length }) : '';
    renderMoves();
    syncControls();
    fetchBaseline(startFen);
  }

  async function fetchBaseline(fen) {
    baseline = null;
    if (baseCtrl) baseCtrl.abort();
    baseCtrl = new AbortController();
    const myAttempt = attemptNo;
    try {
      const info = await api.post('/api/engine/analyze', { fen, movetime_ms: BASELINE_MOVETIME_MS, multipv: 1 }, { signal: baseCtrl.signal, timeout: 15000 });
      const line = info && Array.isArray(info.lines) ? info.lines[0] : null;
      if (myAttempt === attemptNo && !bag.disposed && line && line.score) baseline = userPov(line.score);
    } catch (e) { /* best effort: fall back to the fixed threshold */ }
  }

  function renderMoves() {
    const startNo = Number(startFen.split(' ')[5]) || 1;
    const blackFirst = sideToMove(startFen) === 'black';
    const nodes = [];
    let no = startNo;
    sans.forEach((s, i) => {
      const isWhite = blackFirst ? i % 2 === 1 : i % 2 === 0;
      if (isWhite) nodes.push(h('span', { class: 'n' }, `${no}.`));
      else if (i === 0) nodes.push(h('span', { class: 'n' }, `${no}…`));
      nodes.push(h('span', { class: ['m', i === sans.length - 1 && 'last'] }, sanOf(s)));
      if (!isWhite) no++;
    });
    if (!nodes.length) nodes.push(h('span', { class: 'subtle' }, t('endgames.goodLuck')));
    movesEl.replaceChildren(...nodes);
    const n = Math.min(userMoveCount(), success.moves);
    counterEl.textContent = success.kind === 'hold'
      ? t('endgames.practice.movesHeld', { count: n, total: success.moves })
      : t('endgames.practice.movesUsed', { count: n, total: success.moves });
  }

  function syncControls() {
    if (phase !== 'practice') return;
    undoBtn.disabled = sans.length === 0;
    resetBtn.disabled = sans.length === 0 && status === 'play';
    hintBtn.disabled = status !== 'play' || busy;
    thinking.hidden = !busy;
    const myTurn = status === 'play' && !busy && game.turn() === userChar;
    board.setInteractive(myTurn, myTurn ? userColor : null);
  }

  // ------------------------------------------------------------------ outcome
  const piecesOf = (color) => game.board().flat().filter((p) => p && p.color === color);
  function userPov(score) {
    if (!score || typeof score !== 'object') return null;
    const sign = userColor === 'white' ? 1 : -1;
    if (typeof score.mate === 'number') return { mate: score.mate * sign };
    if (typeof score.cp === 'number') return { cp: score.cp * sign };
    return null;
  }

  /** Result decided by the position itself (after any move). */
  function outcome(lastMove) {
    if (game.isCheckmate()) {
      return game.turn() === userChar
        ? { result: 'lost', reason: t('endgames.reason.mated') }
        : { result: 'won', reason: goal === 'win' ? t('endgames.reason.mateWin') : t('endgames.reason.mateDraw') };
    }
    let drawReason = null;
    if (game.isStalemate()) drawReason = t('endgames.reason.stalemate');
    else if (game.isInsufficientMaterial()) drawReason = t('endgames.reason.insufficient');
    else if (game.isThreefoldRepetition()) drawReason = t('endgames.reason.threefold');
    else if (game.isDrawByFiftyMoves && game.isDrawByFiftyMoves()) drawReason = t('endgames.reason.fifty');
    else if (game.isDraw()) drawReason = t('endgames.reason.drawn');
    if (drawReason) {
      return goal === 'draw'
        ? { result: 'won', reason: `${drawReason} ${t('endgames.reason.heldDraw')}` }
        : { result: 'lost', reason: `${drawReason} ${t('endgames.reason.neededWin')}` };
    }
    if (goal === 'win') {
      const mine = piecesOf(userChar);
      if (mine.every((p) => p.type === 'k')) return { result: 'lost', reason: t('endgames.reason.noPieces') };
      const theirs = piecesOf(oppChar);
      if ((success.kind === 'bare_king' || success.kind === 'promote') && oppHadMaterial && theirs.every((p) => p.type === 'k')) {
        return { result: 'won', reason: t('endgames.reason.bareKing') };
      }
      if (success.kind === 'promote' && lastMove && lastMove.color === userChar && lastMove.promotion) {
        const safe = !game.moves({ verbose: true }).some((m) => m.to === lastMove.to);
        if (safe) return { result: 'won', reason: t('endgames.reason.promoted') };
      }
    } else {
      // The engine can't win with a bare king or a lone minor piece (and no pawns).
      const theirs = piecesOf(oppChar).filter((p) => p.type !== 'k');
      if (theirs.length <= 1 && theirs.every((p) => p.type === 'b' || p.type === 'n')) {
        return { result: 'won', reason: t('endgames.reason.cantWin') };
      }
    }
    return null;
  }

  /** Result decided after the engine's reply (move limits and the engine's evaluation). */
  function afterEngine(score) {
    const s = userPov(score);
    if (s && typeof s.mate === 'number' && s.mate < 0) {
      return { result: 'lost', reason: goal === 'win' ? t('endgames.reason.slipped') : t('endgames.reason.engineMate') };
    }
    if (s && typeof s.cp === 'number') {
      // Draw drills often start "worse" for the defender (a pawn or a piece down): until the start
      // position's evaluation is known, only a forced mate counts as lost.
      const base = baseline && typeof baseline.cp === 'number' ? baseline.cp : null;
      const limit = base !== null ? Math.min(LOST_CP, base - LOST_MARGIN_CP) : (goal === 'win' ? LOST_CP : -Infinity);
      if (s.cp <= limit) return { result: 'lost', reason: goal === 'win' ? t('endgames.reason.slipped') : t('endgames.reason.engineWinning') };
      if (goal === 'win') {
        drawnStreak = Math.abs(s.cp) <= DRAWN_CP ? drawnStreak + 1 : 0;
        if (drawnStreak >= DRAWN_STREAK) return { result: 'lost', reason: t('endgames.reason.drawnNow') };
      }
    }
    if (userMoveCount() >= success.moves) {
      return success.kind === 'hold'
        ? { result: 'won', reason: t('endgames.reason.survived', { count: success.moves }) }
        : { result: 'lost', reason: t('endgames.reason.tooSlow', { count: success.moves }) };
    }
    return null;
  }

  async function record(won) {
    if (recorded || assisted) return null;
    recorded = true;
    if (saveCtrl) saveCtrl.abort();
    saveCtrl = new AbortController();
    try {
      const res = await api.post(`/api/training/${encodeURIComponent(drill.id)}/attempt`, { success: won, moves: Math.min(500, userMoveCount()) }, { signal: saveCtrl.signal, timeout: 10000 });
      if (bag.disposed) return null;
      if (res && res.drill_id) { progress = res; renderMastery(); }
      return res;
    } catch (e) {
      if (!isAbort(e) && !bag.disposed) setFeedback(feedback, 'bad', escapeHtml(t('endgames.saveFailed')));
      return null;
    }
  }

  async function finish({ result, reason }) {
    status = result;
    busy = false;
    syncControls();
    setFeedback(feedback, null);
    const won = result === 'won';
    const moves = userMoveCount();
    const fenNow = startFen;
    const actions = h('div', { class: 'lrn-nav' });
    const box = h('div', { class: 'lrn-complete' },
      h('div', { class: ['lrn-complete-badge', !won && 'fail'], html: icon(won ? 'trophy' : 'x') }),
      h('h2', null, won ? t('endgames.complete') : t('endgames.notQuite')),
      h('p', null, won ? `${reason} ${t('endgames.summary', { count: moves })}` : reason),
      won ? null : (drill.pitfall ? h('div', { class: 'eg-pitfall' },
        h('div', { class: 'eg-pitfall-title', html: icon('alert', { size: 16 }) + `<span>${escapeHtml(t('endgames.whatWentWrong'))}</span>` }),
        h('p', { html: stripP(mdLite(drill.pitfall)) })) : null),
      assisted ? h('p', { class: 'text-sm subtle' }, t('endgames.practice.assisted')) : null,
      actions);
    resultBox.replaceChildren(box);
    botLinkSlot.replaceChildren(); // the result box has its own link
    if (won) {
      sfx('gameEnd');
      confetti(boardSlot, timers, { count: 48 });
      flashClass(boardWrap, 'flash-good', timers, 1000);
    } else {
      sfx('wrong');
      flashClass(boardSlot, 'shake', timers, 420);
    }
    const tryBtn = btn('btn-primary btn-lg', 'refresh', fens.length > 1 ? t('endgames.newPosition') : t('endgames.tryAgain'), () => newAttempt(true));
    const sameBtn = fens.length > 1 ? btn('btn-secondary', 'undo', t('endgames.samePosition'), () => newAttempt(false)) : null;
    put(actions, tryBtn, sameBtn,
      !won && sans.length ? btn('btn-ghost', 'undo', t('endgames.undoLast'), () => undo()) : null,
      playLink(fenNow),
      h('a', { class: 'btn btn-ghost', href: `#/analysis?fen=${encodeURIComponent(fenNow)}`, html: icon('analysis') + `<span>${escapeHtml(t('endgames.study'))}</span>` }));
    resultBox.scrollIntoView({ block: 'nearest', behavior: 'smooth' });

    const res = await record(won);
    if (!res || bag.disposed || resultBox.firstChild !== box) return;
    if (won && res.just_mastered) {
      box.querySelector('h2').textContent = t('endgames.justMastered');
      box.insertBefore(h('p', { class: 'eg-streak-note' }, masteryBadge(), ' ', t('endgames.masteredText')), actions);
      confetti(boardSlot, timers, { count: 72 });
    } else if (won && !res.mastered) {
      const streak = Math.min(need, res.streak || 0);
      box.insertBefore(h('p', { class: 'eg-streak-note' }, masteryDots(res, need), ' ', t('endgames.streakText', { count: streak, left: Math.max(0, need - streak) })), actions);
    }
    if (won && nextDrill && res.mastered) {
      actions.insertBefore(h('a', { class: 'btn btn-secondary', href: `#/endgames/${encodeURIComponent(nextDrill.id)}`, html: `<span>${escapeHtml(t('endgames.nextDrill'))}</span>` + icon('chevron-right') }), actions.children[1] || null);
    }
  }

  // ------------------------------------------------------------------ moves
  function onUserMove(mv) {
    if (phase !== 'practice' || status !== 'play' || busy || game.turn() !== userChar) return false;
    const m = applyUci(game, mv.uci || (mv.from + mv.to + (mv.promotion || '')));
    if (!m) return false;
    sans.push(m.san);
    hintStage = 0;
    board.clearArrows();
    board.setHighlights([]);
    setFeedback(feedback, null);
    renderMoves();
    const out = outcome(m);
    if (out) { finish(out); return true; }
    engineTurn();
    return true;
  }

  let botIdPromise = null;
  function strongestBotId(sig) {
    if (!botIdPromise) {
      botIdPromise = api.get('/api/bots', { signal: sig }).then((bots) => {
        const list = (Array.isArray(bots) ? bots : []).filter((b) => b && b.id && b.category !== 'coach');
        list.sort((a, b) => (b.elo || 0) - (a.elo || 0));
        if (!list.length) throw new Error(t('endgames.errors.noBots'));
        return list[0].id;
      }).catch((e) => { botIdPromise = null; throw e; });
    }
    return botIdPromise;
  }

  async function fetchEngineMove(fen, sig) {
    try {
      const info = await api.post('/api/engine/analyze', { fen, movetime_ms: ENGINE_MOVETIME_MS, multipv: 1 }, { signal: sig, timeout: 15000 });
      const line = info && Array.isArray(info.lines) ? info.lines[0] : null;
      if (line && Array.isArray(line.moves) && line.moves.length) return { uci: line.moves[0], score: line.score || null };
    } catch (e) {
      if (isAbort(e)) throw e;
    }
    const botId = await strongestBotId(sig);
    const historyUcis = game.history({ verbose: true }).map(moveUci);
    const bm = await api.post('/api/bot/move', { bot_id: botId, start_fen: startFen, moves: historyUcis }, { signal: sig, timeout: 15000 });
    if (!bm || !bm.uci) throw new Error(t('endgames.errors.noMove'));
    return { uci: bm.uci, score: null };
  }

  async function engineTurn() {
    const myGen = ++gen;
    busy = true;
    syncControls();
    if (engineCtrl) engineCtrl.abort();
    engineCtrl = new AbortController();
    const started = performance.now();
    let res;
    try {
      res = await fetchEngineMove(game.fen(), engineCtrl.signal);
    } catch (e) {
      if (isAbort(e) || myGen !== gen || bag.disposed) return;
      busy = false;
      syncControls();
      board.setInteractive(false, null);
      setFeedback(feedback, 'bad', t('endgames.engineError', { error: escapeHtml(e && e.message ? e.message : t('endgames.unknownError')) }));
      feedback.appendChild(h('button', { class: 'btn btn-sm btn-secondary', type: 'button', style: 'margin-left:auto', onClick: () => { setFeedback(feedback, null); engineTurn(); } }, t('endgames.retry')));
      return;
    }
    const elapsed = performance.now() - started;
    if (elapsed < MIN_THINK_MS) await sleep(MIN_THINK_MS - elapsed);
    if (myGen !== gen || bag.disposed || status !== 'play') return;

    const m = applyUci(game, res.uci);
    if (!m) {
      busy = false; syncControls();
      setFeedback(feedback, 'bad', escapeHtml(t('endgames.illegal')));
      return;
    }
    sans.push(m.san);
    board.setPosition(game.fen(), { animate: true, lastMove: [m.from, m.to] });
    renderMoves();
    busy = false;

    const out = outcome(m) || afterEngine(res.score);
    if (out) { finish(out); return; }
    if (game.inCheck()) setFeedback(feedback, 'warn', escapeHtml(t('endgames.check')));
    syncControls();
  }

  // ------------------------------------------------------------------ controls
  async function showHint() {
    if (phase !== 'practice' || status !== 'play' || busy || game.turn() !== userChar) return;
    if (hintStage === 0) {
      hintStage = 1;
      // The written hint names moves from the main position; on variants show the first technique.
      const text = variantIdx === 0 ? drill.hint : (technique[0] || drill.hint);
      setFeedback(feedback, 'warn', stripP(mdLite(text || t('endgames.defaultHint'))) + ` <span class="subtle">${escapeHtml(t('endgames.hintAgain'))}</span>`);
      return;
    }
    const fen = game.fen();
    const key = fenKey(fen);
    let best = hintCache.get(key);
    if (!best) {
      hintBtn.classList.add('loading');
      if (hintCtrl) hintCtrl.abort();
      hintCtrl = new AbortController();
      const myGen = gen;
      try {
        const info = await api.post('/api/engine/analyze', { fen, movetime_ms: HINT_MOVETIME_MS, multipv: 1 }, { signal: hintCtrl.signal, timeout: 15000 });
        const line = info && info.lines && info.lines[0];
        const uci = line && line.moves && line.moves[0];
        if (!uci) throw new Error(t('endgames.hintFailed'));
        const mv = applyUci(chessAt(fen), uci);
        best = { uci, san: mv ? mv.san : uci };
        if (hintCache.size > 64) hintCache.clear();
        hintCache.set(key, best);
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        setFeedback(feedback, 'bad', escapeHtml(t('endgames.hintFailed')));
        return;
      } finally {
        if (!bag.disposed) hintBtn.classList.remove('loading');
      }
      if (myGen !== gen || fenKey(game.fen()) !== key) return;
    }
    hintStage = 2;
    assisted = true;
    board.setArrows([{ from: best.uci.slice(0, 2), to: best.uci.slice(2, 4), color: 'green' }]);
    board.setHighlights([{ square: best.uci.slice(0, 2), kind: 'hint' }]);
    setFeedback(feedback, 'warn', `${t('endgames.engineSuggests', { san: escapeHtml(sanOf(best.san)) })} <span class="subtle">${escapeHtml(t('endgames.practice.assisted'))}</span>`);
  }

  function cancelEngine() {
    gen++;
    if (engineCtrl) { engineCtrl.abort(); engineCtrl = null; }
    if (hintCtrl) { hintCtrl.abort(); hintCtrl = null; }
    timers.clear();
    busy = false;
  }

  function undo() {
    if (!sans.length) return;
    cancelEngine();
    while (sans.length) {
      const m = game.undo();
      if (!m) break;
      sans.pop();
      if (m.color === userChar) break;
    }
    // A takeback during an attempt is help; after a recorded result it is just exploring.
    if (!recorded) assisted = true;
    status = 'play';
    hintStage = 0;
    drawnStreak = 0;
    const hist = game.history({ verbose: true });
    const last = hist.length ? hist[hist.length - 1] : null;
    board.setPosition(game.fen(), { animate: true, lastMove: last ? [last.from, last.to] : null, sound: false });
    board.clearArrows(); board.setHighlights([]);
    resultBox.replaceChildren();
    botLinkSlot.replaceChildren(playLink(startFen));
    setFeedback(feedback, 'info', `${escapeHtml(t('endgames.takenBack'))}${recorded ? '' : ` <span class="subtle">${escapeHtml(t('endgames.practice.assisted'))}</span>`}`);
    renderMoves();
    syncControls();
  }

  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const tgt = e.target;
    if (tgt && (tgt.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tgt.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'f' || e.key === 'F') board.flip();
    else if (phase === 'lesson') {
      if (e.key === 'ArrowRight') { e.preventDefault(); if (lessonIdx < lessonSteps.length - 1) showLesson(lessonIdx + 1); else startPractice(true); }
      else if (e.key === 'ArrowLeft' && lessonIdx > 0) { e.preventDefault(); showLesson(lessonIdx - 1); }
    } else if (e.key === 'h' || e.key === 'H') showHint();
    else if (e.key === 'ArrowLeft' || ((e.key === 'z' || e.key === 'Z') && !e.shiftKey)) { if (sans.length) { e.preventDefault(); undo(); } }
  });

  // First visit: the lesson. Once the drill has been practised: straight to practice.
  if (lessonSteps.length && !(progress && progress.attempts)) showLesson(0);
  else startPractice(true);
}
