// Classic games library (#/classics) and narrated game player (#/classics/:classicId).
// Contract: docs/CONTRACT.md "Classic games" (GET /api/classics, GET /api/classics/:id,
// POST /api/classics/:id/progress).

import { h, icon, disposables, mdLite, escapeHtml, emptyState, loadingBlock, pageHeader, toast, formatSan } from '../ui.js';
import { api, isAbort, EngineClient } from '../api.js';
import { t, formatNumber } from '../i18n.js';
import { getSetting, pieceUrl } from '../settings.js';
import { Board } from '../components/board.js';
import { MoveList } from '../components/movelist.js';
import { EvalBar } from '../components/evalbar.js';
import { playSound } from '../components/sound.js';
import { Chess } from '/vendor/chess.js';

export const title = () => t('classics.title');

const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
const PREFS_KEY = 'grandmentor.classics.v1';
const LEVELS = ['beginner', 'intermediate', 'advanced'];
const ERAS = ['romantic', 'classical', 'modern', 'computer'];
const ARROW_COLORS = new Set(['green', 'red', 'blue', 'yellow']);
const SPEEDS = { slow: 3200, normal: 1900, fast: 1000 };
const NOTE_MS_PER_CHAR = 32;
const NOTE_MAX_EXTRA_MS = 7000;
const SAVE_DEBOUNCE_MS = 1200;
const ENGINE_MOVETIME = 700;
const MAX_SEARCH = 60;

function ensureCss() {
  if (document.querySelector('link[data-gm-css="classics"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/classics.css';
  link.dataset.gmCss = 'classics';
  document.head.appendChild(link);
}

function readPrefs() {
  try {
    const p = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}');
    return p && typeof p === 'object' ? p : {};
  } catch { return {}; }
}
function writePrefs(patch) {
  try { localStorage.setItem(PREFS_KEY, JSON.stringify({ ...readPrefs(), ...patch })); } catch { /* storage unavailable */ }
}

const themeLabel = (slug) => t(`classics.themes.${slug}`);
const levelLabel = (lvl) => (LEVELS.includes(lvl) ? t(`classics.levels.${lvl}`) : lvl);
const resultLabel = (r) => t(`classics.results.${r}`);

/** Static board thumbnail (SVG string) for the library cards. */
function miniBoard(fen, orientation, label) {
  const rows = String(fen || START_FEN).split(' ')[0].split('/');
  const flip = orientation === 'black';
  let squares = '';
  let pieces = '';
  for (let r = 0; r < 8; r++) {
    for (let f = 0; f < 8; f++) {
      if ((r + f) % 2 === 1) squares += `<rect class="d" x="${flip ? 7 - f : f}" y="${flip ? 7 - r : r}" width="1" height="1"/>`;
    }
  }
  for (let r = 0; r < 8 && r < rows.length; r++) {
    let f = 0;
    for (const ch of rows[r]) {
      if (f > 7) break;
      if (/[1-8]/.test(ch)) { f += Number(ch); continue; }
      if (!/[prnbqkPRNBQK]/.test(ch)) { f++; continue; }
      const code = (ch === ch.toUpperCase() ? 'w' : 'b') + ch.toUpperCase();
      pieces += `<image href="${escapeHtml(pieceUrl(code))}" x="${flip ? 7 - f : f}" y="${flip ? 7 - r : r}" width="1" height="1"/>`;
      f++;
    }
  }
  return `<svg class="cl-mini-board" viewBox="0 0 8 8" role="img" aria-label="${escapeHtml(label)}" preserveAspectRatio="xMidYMid meet" shape-rendering="crispEdges"><rect class="l" width="8" height="8"/>${squares}${pieces}</svg>`;
}

/** "11.Bxb5+" / "11...Nbd7" for a 1-based ply. */
function moveLabel(ply, san) {
  const n = Math.ceil(ply / 2);
  return `${n}${ply % 2 === 1 ? '.' : '...'}${formatSan(san, getSetting('moveNotation') || 'san')}`;
}

function errorState(message, retry) {
  return emptyState({
    icon: 'alert', title: t('classics.error.title'), text: message || t('classics.error.text'),
    action: retry ? { label: t('classics.error.retry'), icon: 'refresh', onClick: retry } : { label: t('classics.error.back'), href: '#/classics' },
  });
}

export async function mount(root, { params = {}, query = {} } = {}) {
  ensureCss();
  return params.classicId ? mountPlayer(root, params.classicId) : mountLibrary(root, query);
}

// =============================================================================================
// Library
// =============================================================================================

async function mountLibrary(root, query) {
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());

  const page = h('div', { class: 'page cl-page' });
  root.appendChild(page);
  const header = pageHeader({
    title: t('classics.title'), subtitle: t('classics.subtitle'), icon: 'book',
    breadcrumbs: [{ label: t('classics.learn'), href: '#/learn' }, { label: t('classics.title') }],
  });
  const body = h('div', { class: 'stack-lg' }, loadingBlock(t('classics.loadingList')));
  page.append(header, body);

  const state = {
    level: LEVELS.includes(query.level) ? query.level : '',
    theme: typeof query.theme === 'string' ? query.theme : '',
    era: ERAS.includes(query.era) ? query.era : '',
    q: '',
  };
  let games = [];

  async function load() {
    body.replaceChildren(loadingBlock(t('classics.loadingList')));
    try {
      games = await api.get('/api/classics', { signal: ctrl.signal });
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      body.replaceChildren(errorState(e && e.message, load));
      return;
    }
    if (bag.disposed) return;
    if (!Array.isArray(games) || !games.length) {
      body.replaceChildren(emptyState({ emoji: '📜', title: t('classics.empty.title'), text: t('classics.empty.text') }));
      return;
    }
    render();
  }

  const grid = h('div', { class: 'grid-auto cl-grid' });
  const filtersEl = h('div', { class: 'cl-filters card' });
  const statsEl = h('div', { class: 'cl-stats' });

  function statsBlock() {
    const read = games.filter((g) => g.progress && g.progress.completed).length;
    const totalQ = games.reduce((n, g) => n + (g.question_count || 0), 0);
    const answers = games.flatMap((g) => (g.progress && Array.isArray(g.progress.answers) ? g.progress.answers : []));
    const correct = answers.filter((a) => a.correct).length;
    const pct = games.length ? Math.round((read / games.length) * 100) : 0;
    const stat = (label, value, extra) => h('div', { class: 'stat cl-stat' },
      h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value tabular' }, value), extra || null);
    statsEl.replaceChildren(
      stat(t('classics.stats.read'), t('classics.stats.of', { done: formatNumber(read), total: formatNumber(games.length) }),
        h('div', { class: 'progress progress-sm mt-2', role: 'progressbar', 'aria-valuenow': pct, 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-label': t('classics.stats.read') },
          h('div', { class: 'progress-bar', style: `width:${pct}%` }))),
      stat(t('classics.stats.answered'), t('classics.stats.of', { done: formatNumber(answers.length), total: formatNumber(totalQ) })),
      stat(t('classics.stats.correct'), formatNumber(correct)));
  }

  function filterBlock() {
    const seg = h('div', { class: 'segmented cl-levels', role: 'group', 'aria-label': t('classics.filters.level') },
      ['', ...LEVELS].map((l) => h('button', {
        type: 'button', class: state.level === l ? 'active' : null, 'aria-pressed': String(state.level === l),
        onClick: () => { state.level = l; render(); },
      }, l ? levelLabel(l) : t('classics.filters.all'))));
    const eraSel = h('select', { class: 'select cl-era', 'aria-label': t('classics.filters.era') },
      h('option', { value: '' }, t('classics.filters.allEras')),
      ERAS.map((e) => h('option', { value: e, selected: state.era === e }, t(`classics.eras.${e}`))));
    eraSel.value = state.era;
    eraSel.addEventListener('change', () => { state.era = eraSel.value; render(); });
    const search = h('input', {
      class: 'input', type: 'search', placeholder: t('classics.filters.search'), 'aria-label': t('classics.filters.searchAria'),
      maxlength: MAX_SEARCH, value: state.q,
    });
    search.addEventListener('input', () => { state.q = search.value.slice(0, MAX_SEARCH); renderGrid(); });
    const themes = [...new Set(games.flatMap((g) => g.themes || []))].sort((a, b) => themeLabel(a).localeCompare(themeLabel(b)));
    const chips = h('div', { class: 'chip-row cl-theme-chips', role: 'group', 'aria-label': t('classics.filters.theme') },
      h('button', { type: 'button', class: ['chip', !state.theme && 'active'], 'aria-pressed': String(!state.theme), onClick: () => { state.theme = ''; render(); } }, t('classics.filters.allThemes')),
      themes.map((th) => h('button', {
        type: 'button', class: ['chip', state.theme === th && 'active'], 'aria-pressed': String(state.theme === th),
        onClick: () => { state.theme = state.theme === th ? '' : th; render(); },
      }, themeLabel(th))));
    filtersEl.replaceChildren(
      h('div', { class: 'cl-filter-row' }, seg, eraSel,
        h('div', { class: 'input-group cl-search', html: icon('search') }, search)),
      chips);
  }

  function matches(g) {
    if (state.level && g.level !== state.level) return false;
    if (state.era && g.era !== state.era) return false;
    if (state.theme && !(g.themes || []).includes(state.theme)) return false;
    const q = state.q.trim().toLowerCase();
    if (q) {
      const hay = [g.title, g.white, g.black, g.event, g.opening, String(g.year)].join(' ').toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  }

  function card(g) {
    const p = g.progress;
    const done = !!(p && p.completed);
    const started = !done && p && p.max_ply > 0;
    const fullMoves = Math.ceil((g.plies || 0) / 2);
    const boardBox = h('div', { class: 'cl-card-board', html: miniBoard(g.key_fen, g.orientation, t('classics.card.boardAria', { title: g.title })) });
    if (done) boardBox.appendChild(h('span', { class: 'badge badge-solid cl-read-badge', html: icon('check') + `<span>${escapeHtml(t('classics.card.read'))}</span>` }));
    return h('a', { class: ['card', 'card-link', 'cl-card', done && 'is-done'], href: `#/classics/${encodeURIComponent(g.id)}` },
      boardBox,
      h('div', { class: 'cl-card-body' },
        h('div', { class: 'cl-card-year' }, String(g.year), h('span', { class: 'dot-sep' }), g.event),
        h('div', { class: 'cl-card-title' }, g.title),
        h('div', { class: 'cl-card-players', 'aria-label': `${g.white} ${t('classics.vs')} ${g.black}` },
          h('div', { class: 'cl-card-player' }, h('span', { class: 'cl-dot white', 'aria-hidden': 'true' }), h('span', { class: 'truncate' }, g.white)),
          h('div', { class: 'cl-card-player' }, h('span', { class: 'cl-dot black', 'aria-hidden': 'true' }), h('span', { class: 'truncate' }, g.black))),
        h('div', { class: 'cl-card-tags' },
          h('span', { class: `badge level-${LEVELS.includes(g.level) ? g.level : 'beginner'}` }, levelLabel(g.level)),
          h('span', { class: 'badge' }, g.result.replace('1/2-1/2', '½-½')),
          (g.themes || []).slice(0, 2).map((th) => h('span', { class: 'badge cl-theme-badge' }, themeLabel(th)))),
        h('div', { class: 'cl-card-foot subtle' },
          h('span', null, t('classics.card.moves', { count: fullMoves })),
          h('span', { class: 'dot-sep' }),
          h('span', null, t('classics.card.questions', { count: g.question_count || 0 }))),
        started ? h('div', { class: 'cl-card-progress' },
          h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: `width:${Math.min(100, Math.round((p.max_ply / Math.max(1, g.plies)) * 100))}%` })),
          h('span', { class: 'text-xs muted' }, t('classics.card.inProgress', { move: Math.ceil(p.max_ply / 2), total: fullMoves }))) : null));
  }

  function renderGrid() {
    const list = games.filter(matches);
    if (!list.length) {
      grid.replaceChildren(h('div', { class: 'cl-grid-empty' }, emptyState({
        icon: 'filter', title: t('classics.noMatch.title'), text: t('classics.noMatch.text'),
        action: { label: t('classics.noMatch.clear'), kind: 'secondary', onClick: () => { Object.assign(state, { level: '', theme: '', era: '', q: '' }); render(); } },
      })));
      return;
    }
    grid.replaceChildren(...list.map(card));
  }

  function render() {
    statsBlock();
    filterBlock();
    renderGrid();
    if (!body.contains(grid)) body.replaceChildren(statsEl, filtersEl, grid);
  }

  await load();
  return () => bag.dispose();
}

// =============================================================================================
// Player
// =============================================================================================

async function mountPlayer(root, classicId) {
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const timers = new Set();
  const later = (fn, ms) => {
    const id = setTimeout(() => { timers.delete(id); if (!bag.disposed) fn(); }, ms);
    timers.add(id);
    return id;
  };
  const cancel = (id) => { if (id) { clearTimeout(id); timers.delete(id); } };
  bag.add(() => { for (const id of timers) clearTimeout(id); timers.clear(); });

  const page = h('div', { class: 'cl-player-page' }, loadingBlock(t('classics.loading')));
  root.appendChild(page);

  let game;
  try {
    game = await api.get(`/api/classics/${encodeURIComponent(classicId)}`, { signal: ctrl.signal });
  } catch (e) {
    if (isAbort(e) || bag.disposed) return () => bag.dispose();
    page.replaceChildren(h('div', { class: 'page' }, e && e.status === 404
      ? emptyState({ emoji: '🔍', title: t('classics.notFound.title'), text: t('classics.notFound.text'), action: { label: t('classics.error.back'), href: '#/classics' } })
      : errorState(e && e.message)));
    return () => bag.dispose();
  }
  if (bag.disposed) return () => bag.dispose();

  // ---- Derived data ------------------------------------------------------------------------
  const ucis = Array.isArray(game.uci) ? game.uci : [];
  const sans = [];
  const fens = [START_FEN];
  const chess = new Chess();
  for (const u of ucis) {
    let m = null;
    try { m = chess.move({ from: u.slice(0, 2), to: u.slice(2, 4), promotion: u[4] }); } catch { m = null; }
    if (!m) break;
    sans.push(m.san);
    fens.push(chess.fen());
  }
  const N = sans.length;
  const notes = new Map((game.annotations || []).filter((a) => a.ply <= N).map((a) => [a.ply, a]));
  const questions = new Map((game.questions || []).filter((q) => q.ply >= 1 && q.ply <= N).map((q) => [q.ply, q]));
  const noteBefore = (ply) => { for (let p = ply; p >= 0; p--) if (notes.has(p)) return notes.get(p); return null; };
  const progress0 = game.progress || null;
  const firstAnswers = new Map((progress0 && progress0.answers || []).map((a) => [a.ply, !!a.correct]));
  let completedBefore = !!(progress0 && progress0.completed);
  const resolved = new Set();   // question plies resolved in this session
  const prefs = readPrefs();
  let speed = SPEEDS[prefs.speed] ? prefs.speed : 'normal';
  let evalOn = prefs.eval === true;
  const orientation0 = game.orientation === 'black' ? 'black' : 'white';

  // ---- Layout ------------------------------------------------------------------------------
  const playPosLink = h('a', { class: 'btn btn-secondary btn-sm', href: '#/play', html: icon('robot') + `<span>${escapeHtml(t('classics.player.playPosition'))}</span>` });
  const analysisLink = h('a', { class: 'btn btn-ghost btn-sm', href: '#/analysis', html: icon('analysis') + `<span>${escapeHtml(t('classics.player.openAnalysis'))}</span>` });
  const top = h('div', { class: 'cl-top' },
    h('div', { class: 'cl-top-main' },
      h('nav', { class: 'breadcrumbs', 'aria-label': t('ui.breadcrumb') },
        h('a', { href: '#/learn' }, t('classics.learn')), h('span', { html: icon('chevron-right'), style: 'display:contents' }),
        h('a', { href: '#/classics' }, t('classics.title'))),
      h('h1', { class: 'cl-title' }, game.title),
      h('div', { class: 'cl-sub muted' },
        h('span', null, `${game.white} ${t('classics.vs')} ${game.black}`), h('span', { class: 'dot-sep' }),
        h('span', null, `${game.event}, ${game.year}`), h('span', { class: 'dot-sep' }),
        h('span', null, game.opening), h('span', { class: 'dot-sep' }),
        h('span', { class: 'semibold' }, resultLabel(game.result)))),
    h('div', { class: 'cl-top-actions' }, playPosLink, analysisLink));

  const playerBar = (color) => h('div', { class: 'player-bar cl-player-bar' },
    h('div', { class: `avatar cl-side ${color}`, 'aria-hidden': 'true' }, h('img', { src: pieceUrl(color === 'white' ? 'wK' : 'bK'), alt: '' })),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'player-name' }, color === 'white' ? game.white : game.black),
      h('div', { class: 'player-captures' }, t(`classics.player.${color}`))),
    h('div', { class: 'cl-bar-result' }, color === 'white' ? game.result.split('-')[0].replace('1/2', '½') : (game.result.split('-')[1] || '').replace('1/2', '½')));
  const topBarSlot = h('div', { class: 'cl-bar-slot' });
  const bottomBarSlot = h('div', { class: 'cl-bar-slot' });
  const evalSlot = h('div', { class: 'evalbar-slot' });
  const boardSlot = h('div', { class: 'board-slot' });
  const main = h('div', { class: 'game-main' }, topBarSlot, h('div', { class: 'board-row' }, evalSlot, boardSlot), bottomBarSlot);

  // Narration card
  const caption = h('span', { class: 'cl-caption' });
  const counter = h('span', { class: 'cl-counter tabular subtle' });
  const progressFill = h('div', { class: 'progress-bar' });
  const narrBody = h('div', { class: 'cl-narration-body', 'aria-live': 'polite' });
  const narration = h('section', { class: 'panel cl-narration', 'aria-label': t('classics.player.mentor') },
    h('div', { class: 'panel-header cl-narration-head' },
      h('div', { class: 'avatar avatar-sm mentor-avatar', 'aria-hidden': 'true' }, '🎓'),
      h('div', { class: 'cl-narration-title' }, h('span', null, t('classics.player.mentor')), caption),
      counter),
    h('div', { class: 'progress progress-sm cl-progress', role: 'progressbar', 'aria-label': t('classics.player.progressAria') }, progressFill),
    narrBody);

  // Key moments + move list
  const chipsRow = h('div', { class: 'chip-row cl-moments' });
  const listSlot = h('div', { class: 'cl-movelist' });
  const movesPanel = h('div', { class: 'panel grow cl-moves-panel' },
    h('div', { class: 'cl-moments-wrap' }, h('div', { class: 'cl-section-label' }, h('span', { class: 'cl-chip-icon', html: icon('star', { size: 14 }) }), t('classics.player.keyMoments')), chipsRow),
    h('div', { class: 'panel-body cl-moves-body' }, listSlot));

  // Controls
  const btn = (name, label, onClick, extra = '') => h('button', { class: `btn btn-ghost btn-icon ${extra}`, type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(name), onClick });
  const firstBtn = btn('first', t('classics.player.first'), () => userGo(0));
  const prevBtn = btn('chevron-left', t('classics.player.prev'), () => userGo(cur - 1));
  const playBtn = h('button', { class: 'btn btn-primary cl-play', type: 'button', onClick: () => togglePlay() });
  const nextBtn = btn('chevron-right', t('classics.player.next'), () => stepForward());
  const lastBtn = btn('last', t('classics.player.last'), () => userGo(N));
  const flipBtn = btn('flip', t('classics.player.flip'), () => flip());
  const toolbar = h('div', { class: 'toolbar cl-toolbar' }, firstBtn, prevBtn, playBtn, nextBtn, lastBtn, flipBtn);

  const speedSeg = h('div', { class: 'segmented cl-speed', role: 'group', 'aria-label': t('classics.player.speed') },
    Object.keys(SPEEDS).map((k) => h('button', { type: 'button', dataset: { speed: k }, onClick: () => setSpeed(k) }, t(`classics.player.speeds.${k}`))));
  const evalInput = h('input', { type: 'checkbox', checked: evalOn });
  const evalSwitch = h('label', { class: 'switch cl-eval-switch', title: t('classics.player.evalBarHelp') }, evalInput, h('span', { class: 'switch-track' }), h('span', null, t('classics.player.evalBar')));
  const options = h('div', { class: 'cl-options' }, speedSeg, evalSwitch);
  const controls = h('div', { class: 'cl-controls' }, toolbar, options);

  const aside = h('aside', { class: 'game-panel cl-panel' }, narration, movesPanel, controls);
  const layout = h('div', { class: ['game-layout', 'cl-layout', !evalOn && 'no-eval'], style: '--board-chrome: 250px' }, main, aside);
  const keysHelp = h('p', { class: 'cl-keys subtle text-xs hide-mobile' }, t('classics.player.keysHelp'));
  page.replaceChildren(top, layout, keysHelp);

  // ---- Components --------------------------------------------------------------------------
  const board = new Board(boardSlot, {
    fen: START_FEN, orientation: orientation0, interactive: false, movableColor: null,
    onMove: (mv) => onUserMove(mv),
  });
  bag.add(() => board.destroy());
  const evalBar = new EvalBar(evalSlot, { orientation: orientation0 });
  bag.add(() => evalBar.destroy());
  const moveList = new MoveList(listSlot, { onSelect: (ply) => userGo(ply) });
  bag.add(() => moveList.destroy());
  moveList.setMoves(sans.map((san) => ({ san })));
  let engine = null;
  bag.add(() => { if (engine) engine.close(); engine = null; });

  // ---- State -------------------------------------------------------------------------------
  let cur = 0;
  let playing = false;
  let playTimer = 0;
  let activeQ = null;   // { q, tries, revealed, done, resumePlay }
  let saveTimer = 0;
  let evalTimer = 0;
  let nextGameId = null;

  function renderPlayers() {
    const o = board.orientation;
    topBarSlot.replaceChildren(playerBar(o === 'white' ? 'black' : 'white'));
    bottomBarSlot.replaceChildren(playerBar(o));
  }

  function renderChips() {
    const items = [];
    for (const a of game.annotations || []) if (a.label && a.ply <= N) items.push({ ply: a.ply, label: a.label, kind: 'moment' });
    for (const q of questions.values()) items.push({ ply: q.ply - 1, label: t('classics.player.pauseChip'), kind: 'question', q });
    items.sort((a, b) => a.ply - b.ply || (a.kind === 'question' ? 1 : -1));
    chipsRow.replaceChildren(...items.map((it) => {
      const answered = it.kind === 'question' && (resolved.has(it.q.ply) || firstAnswers.has(it.q.ply));
      return h('button', {
        type: 'button', class: ['chip', 'cl-chip', `cl-chip-${it.kind}`, answered && 'answered'], dataset: { ply: it.ply },
        title: it.ply > 0 ? moveLabel(it.kind === 'question' ? it.q.ply : it.ply, sans[(it.kind === 'question' ? it.q.ply : it.ply) - 1]) : t('classics.player.intro'),
        onClick: () => (it.kind === 'question' ? openQuestion(it.q) : userGo(it.ply)),
      }, h('span', { class: 'cl-chip-icon', html: icon(it.kind === 'question' ? 'help' : 'star', { size: 14 }) }), it.label);
    }));
  }

  function syncChips() {
    for (const c of chipsRow.querySelectorAll('.cl-chip')) {
      const ply = Number(c.dataset.ply);
      const isQ = c.classList.contains('cl-chip-question');
      c.classList.toggle('active', isQ ? !!(activeQ && activeQ.q.ply - 1 === ply) : (!activeQ && ply === cur));
    }
  }

  function syncControls() {
    const blocked = !!(activeQ && !activeQ.done);
    firstBtn.disabled = cur === 0 && !activeQ;
    prevBtn.disabled = cur === 0;
    nextBtn.disabled = cur >= N || blocked;
    lastBtn.disabled = cur >= N || blocked;
    playBtn.disabled = blocked || (cur >= N && !playing);
    const label = playing ? t('classics.player.pause') : t('classics.player.play');
    playBtn.innerHTML = icon(playing ? 'pause' : 'play') + `<span>${escapeHtml(label)}</span>`;
    playBtn.setAttribute('aria-pressed', String(playing));
    counter.textContent = t('classics.player.moveOf', { move: Math.ceil(cur / 2), total: Math.ceil(N / 2) });
    progressFill.style.width = `${N ? ((cur / N) * 100).toFixed(1) : 0}%`;
    for (const b of speedSeg.querySelectorAll('button')) {
      const on = b.dataset.speed === speed;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', String(on));
    }
    const fen = fens[cur];
    playPosLink.href = `#/play?fen=${encodeURIComponent(fen)}&color=${fen.split(' ')[1] === 'b' ? 'b' : 'w'}`;
    analysisLink.href = `#/analysis?pgn=${encodeURIComponent(pgnText())}`;
  }

  function pgnText() {
    const tag = (k, v) => `[${k} "${String(v).replace(/["\\]/g, '')}"]`;
    const moves = sans.map((s, i) => (i % 2 === 0 ? `${i / 2 + 1}. ${s}` : s)).join(' ');
    return [tag('Event', game.event), tag('Date', `${game.year}.??.??`), tag('White', game.white), tag('Black', game.black), tag('Result', game.result), '', `${moves} ${game.result}`].join('\n');
  }

  function noteShapes(note) {
    board.setArrows((note && Array.isArray(note.arrows) ? note.arrows : [])
      .filter((a) => a && /^[a-h][1-8]$/.test(a.from) && /^[a-h][1-8]$/.test(a.to))
      .map((a) => ({ from: a.from, to: a.to, color: ARROW_COLORS.has(a.color) ? a.color : 'green' })));
    board.setHighlights((note && Array.isArray(note.highlights) ? note.highlights : [])
      .filter((s) => /^[a-h][1-8]$/.test(s)).map((square) => ({ square, kind: 'hint' })));
  }

  function bubble(textHtml, { kind = 'info', label, stale = false } = {}) {
    return h('div', { class: ['mentor-row', 'cl-bubble-row', !stale && 'pop-in', stale && 'stale'] },
      h('div', { class: `bubble bubble-mentor mentor-msg kind-${kind} cl-bubble` },
        label ? h('div', { class: 'cl-moment-label', html: icon('star', { size: 14 }) + `<span>${escapeHtml(label)}</span>` }) : null,
        h('div', { class: 'md', html: textHtml })));
  }

  function renderNarration() {
    if (activeQ) return renderQuestion();
    const exact = notes.get(cur);
    const note = exact || noteBefore(cur);
    caption.textContent = cur === 0 ? t('classics.player.intro') : t('classics.player.after', { move: moveLabel(cur, sans[cur - 1]) });
    const parts = [];
    if (note) parts.push(bubble(mdLite(note.text), { label: note.label || null, stale: !exact }));
    if (!exact && cur > 0) parts.push(h('p', { class: 'cl-hint-line subtle text-sm' }, t('classics.player.keepWatching')));
    if (cur === 0) {
      const resumePly = progress0 && !progress0.completed ? Math.min(N, progress0.last_ply || 0) : 0;
      if (resumePly > 0) {
        parts.push(h('div', { class: 'cl-resume' },
          h('span', { class: 'text-sm muted' }, t('classics.player.welcomeBack', { move: Math.ceil(resumePly / 2) })),
          h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon('play') + `<span>${escapeHtml(t('classics.player.resume', { move: Math.ceil(resumePly / 2) }))}</span>`, onClick: () => userGo(resumePly) })));
      } else {
        parts.push(h('p', { class: 'cl-hint-line subtle text-sm' }, t('classics.player.start')));
      }
    }
    if (cur === N && N > 0) parts.push(doneCard());
    narrBody.replaceChildren(...parts);
    narrBody.scrollTop = 0;
    noteShapes(exact || null);
  }

  function doneCard() {
    const total = questions.size;
    const correct = [...questions.keys()].filter((p) => firstAnswers.get(p) === true).length;
    return h('div', { class: 'cl-done pop-in' },
      h('div', { class: 'cl-done-title', html: icon('trophy') + `<span>${escapeHtml(t('classics.done.title'))}</span>` }),
      h('p', { class: 'text-sm' }, t('classics.done.text', { title: game.title })),
      total ? h('p', { class: 'text-sm muted' }, t('classics.done.score', { correct, total })) : null,
      h('div', { class: 'cl-done-actions' },
        nextGameId ? h('a', { class: 'btn btn-primary btn-sm', href: `#/classics/${encodeURIComponent(nextGameId)}`, html: `<span>${escapeHtml(t('classics.done.next'))}</span>` + icon('chevron-right') }) : null,
        h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon('refresh') + `<span>${escapeHtml(t('classics.done.replay'))}</span>`, onClick: () => userGo(0) }),
        h('a', { class: 'btn btn-ghost btn-sm', href: '#/classics' }, t('classics.done.library'))));
  }

  // ---- Navigation --------------------------------------------------------------------------
  function show(ply, { animate = true } = {}) {
    cur = Math.max(0, Math.min(N, ply));
    const last = cur > 0 ? [ucis[cur - 1].slice(0, 2), ucis[cur - 1].slice(2, 4)] : null;
    board.setInteractive(false, null);
    board.setPosition(fens[cur], { animate, lastMove: last });
    moveList.setCurrent(cur);
    renderNarration();
    syncChips();
    syncControls();
    scheduleEval();
    scheduleSave();
    if (cur === N && N > 0) markCompleted();
  }

  /** Jump requested by the user (move list, buttons, chips): leaves any open question. */
  function userGo(ply) {
    stopPlay();
    if (activeQ) closeQuestion();
    const target = Math.max(0, Math.min(N, ply));
    show(target, { animate: Math.abs(target - cur) === 1 });
  }

  /** One move forward; stops for a "pause and think" question first. */
  function stepForward() {
    if (activeQ && !activeQ.done) return false;
    if (activeQ && activeQ.done) { continueAfterQuestion(); return true; }
    if (cur >= N) return false;
    const q = questions.get(cur + 1);
    if (q && !resolved.has(q.ply)) { openQuestion(q, { resumePlay: playing }); return false; }
    show(cur + 1);
    return true;
  }

  function togglePlay() {
    if (playing) { stopPlay(); return; }
    if (cur >= N) return;
    playing = true;
    syncControls();
    if (stepForward()) scheduleNext(); else if (!activeQ) stopPlay();
  }

  function scheduleNext() {
    cancel(playTimer);
    if (!playing) return;
    if (cur >= N) { stopPlay(); return; }
    const note = notes.get(cur);
    const extra = note ? Math.min(NOTE_MAX_EXTRA_MS, (note.text || '').length * NOTE_MS_PER_CHAR) : 0;
    playTimer = later(() => {
      playTimer = 0;
      if (!playing) return;
      if (stepForward()) scheduleNext();
      else if (!activeQ) stopPlay();
    }, SPEEDS[speed] + extra);
  }

  function stopPlay() {
    cancel(playTimer);
    playTimer = 0;
    if (playing) { playing = false; syncControls(); }
  }

  function setSpeed(k) {
    if (!SPEEDS[k]) return;
    speed = k;
    writePrefs({ speed: k });
    syncControls();
    if (playing) scheduleNext();
  }

  function flip() {
    board.flip();
    evalBar.setOrientation(board.orientation);
    renderPlayers();
  }

  // ---- Pause and think ---------------------------------------------------------------------
  function openQuestion(q, { resumePlay = false } = {}) {
    cancel(playTimer);
    playTimer = 0;
    playing = false;
    activeQ = { q, tries: 0, revealed: false, done: false, resumePlay, feedback: null };
    cur = q.ply - 1;
    board.setPosition(fens[cur], { animate: true, lastMove: cur > 0 ? [ucis[cur - 1].slice(0, 2), ucis[cur - 1].slice(2, 4)] : null });
    board.clearArrows();
    board.clearHighlights();
    board.setInteractive(true, fens[cur].split(' ')[1] === 'b' ? 'black' : 'white');
    moveList.setCurrent(cur);
    renderNarration();
    syncChips();
    syncControls();
    scheduleEval();
    playSound('notify');
  }

  function closeQuestion() {
    activeQ = null;
    board.setInteractive(false, null);
  }

  function renderQuestion() {
    const { q, done, revealed, feedback } = activeQ;
    const side = fens[q.ply - 1].split(' ')[1] === 'b' ? 'black' : 'white';
    caption.textContent = t('classics.question.title');
    const fb = feedback ? h('div', { class: `cl-feedback ${feedback.kind}`, role: 'status', html: icon(feedback.kind === 'good' ? 'check-circle' : feedback.kind === 'bad' ? 'x-circle' : 'info') + `<div>${feedback.html}</div>` }) : null;
    const actions = done
      ? [h('button', { class: 'btn btn-primary btn-sm', type: 'button', html: `<span>${escapeHtml(t('classics.question.continue'))}</span>` + icon('chevron-right'), onClick: () => continueAfterQuestion() })]
      : [
        q.hint ? h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon('hint') + `<span>${escapeHtml(t('classics.question.hint'))}</span>`, onClick: () => showHint() }) : null,
        h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon('eye') + `<span>${escapeHtml(t('classics.question.showMe'))}</span>`, onClick: () => reveal() }),
      ];
    narrBody.replaceChildren(
      h('div', { class: ['cl-question', done && 'is-done'], 'aria-label': t('classics.question.answerAria') },
        h('div', { class: 'cl-question-head' },
          h('span', { class: 'cl-question-icon', html: icon('help') }),
          h('span', { class: 'bold' }, t('classics.question.title')),
          h('span', { class: 'spacer' }),
          h('span', { class: `cl-turn ${side}` }, h('span', { class: 'cl-turn-dot', 'aria-hidden': 'true' }), t(side === 'white' ? 'classics.question.whiteToPlay' : 'classics.question.blackToPlay'))),
        h('div', { class: 'md cl-question-prompt', html: mdLite(q.prompt || t('classics.question.prompt')) }),
        activeQ.hintShown && q.hint && !done ? h('div', { class: 'cl-question-hint text-sm', html: icon('hint', { size: 16 }) + `<span>${mdLite(q.hint)}</span>` }) : null,
        fb,
        done ? bubble(mdLite(q.explanation || ''), { kind: revealed ? 'idea' : 'success' }) : null,
        h('div', { class: 'cl-question-actions' }, actions)));
    if (!done) noteShapes(null);
  }

  function showHint() {
    if (!activeQ || activeQ.done) return;
    activeQ.hintShown = true;
    renderQuestion();
  }

  function recordAnswer(ply, correct) {
    if (firstAnswers.has(ply)) return;
    firstAnswers.set(ply, correct);
    api.post(`/api/classics/${encodeURIComponent(game.id)}/progress`, { answer: { ply, correct } }, { signal: ctrl.signal }).catch(() => {});
  }

  function onUserMove(mv) {
    if (!activeQ || activeQ.done) return false;
    const q = activeQ.q;
    const accept = Array.isArray(q.accept) && q.accept.length ? q.accept : [q.answer_uci];
    if (accept.includes(mv.uci)) {
      const exact = mv.uci === q.answer_uci;
      recordAnswer(q.ply, activeQ.tries === 0);
      activeQ.done = true;
      activeQ.feedback = { kind: 'good', html: escapeHtml(exact ? t('classics.question.correct') : t('classics.question.correctAlt', { move: moveLabel(q.ply, sans[q.ply - 1]) })) };
      resolved.add(q.ply);
      cur = q.ply;
      board.setInteractive(false, null);
      if (!exact) later(() => board.setPosition(fens[cur], { animate: true, lastMove: [ucis[cur - 1].slice(0, 2), ucis[cur - 1].slice(2, 4)] }), 650);
      playSound('correct');
      moveList.setCurrent(cur);
      renderQuestion();
      renderChips();
      syncChips();
      syncControls();
      scheduleEval();
      scheduleSave();
      return true;
    }
    activeQ.tries++;
    if (activeQ.tries === 1) recordAnswer(q.ply, false);
    activeQ.feedback = { kind: 'bad', html: escapeHtml(activeQ.tries >= 2 ? t('classics.question.wrongTwice') : t('classics.question.wrong')) };
    playSound('wrong');
    later(() => { if (activeQ && !activeQ.done) renderQuestion(); }, 0);
    return false;
  }

  function reveal() {
    if (!activeQ || activeQ.done) return;
    const q = activeQ.q;
    recordAnswer(q.ply, false);
    activeQ.done = true;
    activeQ.revealed = true;
    activeQ.feedback = { kind: 'info', html: escapeHtml(t('classics.question.revealed', { move: moveLabel(q.ply, sans[q.ply - 1]) })) };
    resolved.add(q.ply);
    cur = q.ply;
    board.setInteractive(false, null);
    board.setPosition(fens[cur], { animate: true, lastMove: [ucis[cur - 1].slice(0, 2), ucis[cur - 1].slice(2, 4)] });
    moveList.setCurrent(cur);
    renderQuestion();
    renderChips();
    syncChips();
    syncControls();
    scheduleEval();
    scheduleSave();
  }

  function continueAfterQuestion() {
    if (!activeQ) return;
    const resume = activeQ.resumePlay;
    closeQuestion();
    show(cur, { animate: false });
    if (resume && cur < N) { playing = true; syncControls(); scheduleNext(); }
  }

  // ---- Eval bar ----------------------------------------------------------------------------
  function setEval(on) {
    evalOn = on;
    writePrefs({ eval: on });
    layout.classList.toggle('no-eval', !on);
    window.dispatchEvent(new Event('resize'));
    if (on) scheduleEval();
    else if (engine) engine.stop();
  }

  function scheduleEval() {
    if (!evalOn) return;
    cancel(evalTimer);
    evalTimer = later(() => {
      evalTimer = 0;
      const fen = fens[cur];
      const c = new Chess(fen);
      if (c.isCheckmate()) { evalBar.set({ mate: 0 }, { fen }); return; }
      if (c.isDraw() || c.isStalemate()) { evalBar.set({ cp: 0 }); return; }
      if (!engine) engine = new EngineClient();
      engine.analyze(fen, { multipv: 1, movetime_ms: ENGINE_MOVETIME }, (info) => {
        if (bag.disposed || fens[cur] !== fen) return;
        const line = Array.isArray(info.lines) ? info.lines[0] : null;
        if (line && line.score) evalBar.set(line.score, { fen });
      }, () => {});
    }, 180);
  }

  // ---- Progress ----------------------------------------------------------------------------
  function scheduleSave() {
    cancel(saveTimer);
    saveTimer = later(() => {
      saveTimer = 0;
      api.post(`/api/classics/${encodeURIComponent(game.id)}/progress`, { ply: cur }, { signal: ctrl.signal }).catch(() => {});
    }, SAVE_DEBOUNCE_MS);
  }

  function markCompleted() {
    stopPlay();
    cancel(saveTimer);
    saveTimer = 0;
    api.post(`/api/classics/${encodeURIComponent(game.id)}/progress`, { ply: N, completed: true }, { signal: ctrl.signal })
      .then(() => {
        if (bag.disposed || completedBefore) return;
        completedBefore = true;
        playSound('gameEnd');
        toast(t('classics.done.toast'), 'success');
      })
      .catch(() => {});
  }

  // ---- Wire up -----------------------------------------------------------------------------
  evalInput.addEventListener('change', () => setEval(evalInput.checked));
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const tag = (e.target && e.target.tagName) || '';
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag) || (e.target && e.target.isContentEditable)) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'ArrowLeft') { e.preventDefault(); if (cur > 0) userGo(cur - 1); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); stopPlay(); stepForward(); }
    else if (e.key === 'Home') { e.preventDefault(); userGo(0); }
    else if (e.key === 'End') { e.preventDefault(); if (!(activeQ && !activeQ.done)) userGo(N); }
    else if (e.key === ' ' && !(e.target && e.target.closest && e.target.closest('button, a'))) { e.preventDefault(); if (!(activeQ && !activeQ.done)) togglePlay(); }
    else if (e.key === 'f' || e.key === 'F') flip();
  });

  renderPlayers();
  renderChips();
  show(0, { animate: false });
  if (evalOn) window.dispatchEvent(new Event('resize'));

  // Next game for the completion card (best effort).
  api.get('/api/classics', { signal: ctrl.signal }).then((list) => {
    if (bag.disposed || !Array.isArray(list)) return;
    const i = list.findIndex((g) => g.id === game.id);
    const rest = [...list.slice(i + 1), ...list.slice(0, Math.max(0, i))];
    const next = rest.find((g) => !(g.progress && g.progress.completed)) || rest[0];
    nextGameId = next ? next.id : null;
    if (cur === N && !activeQ) renderNarration();
  }).catch(() => {});

  return () => bag.dispose();
}
