// Library page (#/library): saved games with search, filters, sort, cheap static board
// thumbnails, favourites, notes/tags, PGN download/import and delete.
// Also exports small shared helpers used by home.js and profile.js (hub pages).

import { api, qs, isAbort } from '../api.js';
import {
  h, icon, pageHeader, emptyState, skeleton, disposables, debounce, modal, confirmDialog,
  toast, escapeHtml,
} from '../ui.js';
import { getSetting, pieceUrl } from '../settings.js';
import { t, getLocale, formatDateIntl } from '../i18n.js';

export const title = () => t('library.title');

/** Locale-aware "Mar 3, 2026". */
export function formatGameDate(date) {
  return formatDateIntl(date, { month: 'short', day: 'numeric', year: 'numeric' });
}

/** Locale-aware relative time ("5 minutes ago" / "hace 5 minutos"); older dates fall back to a short date. */
export function formatRelativeIntl(date) {
  const d = date instanceof Date ? date : new Date(date);
  if (Number.isNaN(d.getTime())) return '';
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 45) return t('library.time.justNow');
  let rtf;
  try { rtf = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' }); } catch { rtf = null; }
  const rel = (v, unit) => (rtf ? rtf.format(-v, unit) : formatGameDate(d));
  if (diff < 3600) return rel(Math.max(1, Math.round(diff / 60)), 'minute');
  if (diff < 86400) return rel(Math.round(diff / 3600), 'hour');
  if (diff < 604800) return rel(Math.max(1, Math.round(diff / 86400)), 'day');
  return formatDateIntl(d, { month: 'short', day: 'numeric', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' });
}

// ---------------------------------------------------------------------------
// Shared hub helpers
// ---------------------------------------------------------------------------

/** Load web/css/hub.css once (it is not linked from index.html). */
let hubCssPromise = null;
/** Load web/css/hub.css once (it is not linked from index.html). Resolves when loaded (or after 1.5s). */
export function ensureHubCss() {
  if (hubCssPromise) return hubCssPromise;
  const existing = document.querySelector('link[data-hub-css], link[href$="/css/hub.css"]');
  if (existing && existing.sheet) { hubCssPromise = Promise.resolve(); return hubCssPromise; }
  hubCssPromise = new Promise((resolve) => {
    const link = existing || document.createElement('link');
    let timer = 0;
    const done = () => { clearTimeout(timer); link.removeEventListener('load', done); link.removeEventListener('error', done); resolve(); };
    link.addEventListener('load', done);
    link.addEventListener('error', done);
    timer = setTimeout(done, 1500);
    if (!existing) {
      link.rel = 'stylesheet';
      link.href = '/css/hub.css';
      link.dataset.hubCss = '1';
      document.head.appendChild(link);
    }
  });
  return hubCssPromise;
}

const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';

// Dark squares of an 8x8 board in a single path (a8 = top-left is light).
const DARK_PATH = (() => {
  let d = '';
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) if ((x + y) % 2 === 1) d += `M${x} ${y}h1v1h-1z`;
  return d;
})();

const FILES = 'abcdefgh';
function squareXY(sq, flip) {
  if (typeof sq !== 'string' || sq.length !== 2) return null;
  const f = FILES.indexOf(sq[0]);
  const r = Number(sq[1]);
  if (f < 0 || !(r >= 1 && r <= 8)) return null;
  const x = flip ? 7 - f : f;
  const y = flip ? r - 1 : 8 - r;
  return { x, y };
}

/**
 * Lightweight static board as an SVG string (one element, no listeners).
 * fenBoardSvg(fen, { orientation: 'white'|'black', lastMove: 'e2e4' | ['e2','e4'], label })
 */
export function fenBoardSvg(fen, { orientation = 'white', lastMove = null, label, pieceSet } = {}) {
  const flip = orientation === 'black';
  const placement = String(fen || START_FEN).trim().split(/\s+/)[0] || '';
  const set = pieceSet || getSetting('pieceSet') || 'cburnett';
  let pieces = '';
  const rows = placement.split('/');
  if (rows.length === 8) {
    for (let ry = 0; ry < 8; ry++) {
      let fx = 0;
      for (const ch of rows[ry]) {
        if (fx > 7) break;
        if (ch >= '1' && ch <= '8') { fx += Number(ch); continue; }
        const lower = ch.toLowerCase();
        if (!'kqrbnp'.includes(lower)) { fx++; continue; }
        const code = (ch === lower ? 'b' : 'w') + lower.toUpperCase();
        const x = flip ? 7 - fx : fx;
        const y = flip ? 7 - ry : ry;
        pieces += `<image href="${pieceUrl(code, set)}" x="${x}" y="${y}" width="1" height="1"/>`;
        fx++;
      }
    }
  }
  let hl = '';
  if (lastMove) {
    const [from, to] = Array.isArray(lastMove) ? lastMove : [String(lastMove).slice(0, 2), String(lastMove).slice(2, 4)];
    for (const sq of [from, to]) {
      const p = squareXY(sq, flip);
      if (p) hl += `<rect class="hub-sq-last" x="${p.x}" y="${p.y}" width="1" height="1"/>`;
    }
  }
  return `<svg class="hub-mini-board" viewBox="0 0 8 8" role="img" aria-label="${escapeHtml(label ?? t('library.board.position'))}" shape-rendering="crispEdges">`
    + `<rect class="hub-sq-l" width="8" height="8"/><path class="hub-sq-d" d="${DARK_PATH}"/>${hl}`
    + `<g shape-rendering="auto">${pieces}</g></svg>`;
}

let chessPromise = null;
/** Lazily load the vendored chess.js (cached). */
export function loadChess() {
  if (!chessPromise) {
    chessPromise = import('/vendor/chess.js').then((m) => m.Chess).catch((e) => { chessPromise = null; throw e; });
  }
  return chessPromise;
}

/** Apply UCI moves to a FEN with chess.js; returns {fen, lastMove, turn} (stops at the first illegal move). */
export async function playUci(startFen, moves, maxMoves = 1200) {
  const Chess = await loadChess();
  let c;
  try { c = new Chess(startFen || START_FEN); } catch { c = new Chess(); }
  let last = null;
  const list = Array.isArray(moves) ? moves.slice(0, maxMoves) : [];
  for (const u of list) {
    if (typeof u !== 'string' || u.length < 4) break;
    try {
      c.move({ from: u.slice(0, 2), to: u.slice(2, 4), promotion: u[4] || undefined });
      last = u;
    } catch { break; }
  }
  return { fen: c.fen(), lastMove: last, turn: c.turn() === 'w' ? 'white' : 'black' };
}

// Bounded LRU cache of final positions for thumbnails: key "id:updated_at".
const FEN_CACHE_MAX = 300;
const fenCache = new Map();
function cacheGet(k) {
  if (!fenCache.has(k)) return undefined;
  const v = fenCache.get(k);
  fenCache.delete(k);
  fenCache.set(k, v);
  return v;
}
function cacheSet(k, v) {
  fenCache.delete(k);
  fenCache.set(k, v);
  while (fenCache.size > FEN_CACHE_MAX) fenCache.delete(fenCache.keys().next().value);
}

// Tiny concurrency limiter for thumbnail fetches (slot hand-off avoids over-subscription).
const MAX_FETCH = 4;
let activeFetch = 0;
const waiters = [];
async function withSlot(fn, signal) {
  if (activeFetch < MAX_FETCH) activeFetch++;
  else {
    await new Promise((resolve, reject) => {
      const w = { resolve };
      waiters.push(w);
      if (signal) {
        const onAbort = () => {
          const i = waiters.indexOf(w);
          if (i >= 0) { waiters.splice(i, 1); reject(new DOMException('Aborted', 'AbortError')); }
        };
        if (signal.aborted) onAbort(); else signal.addEventListener('abort', onAbort, { once: true });
        w.resolve = () => { signal.removeEventListener('abort', onAbort); resolve(); };
      }
    });
  }
  try { return await fn(); }
  finally {
    const next = waiters.shift();
    if (next) next.resolve(); else activeFetch--;
  }
}

/** Final position of a saved game (summary or full record). Cached; fetches the record if needed. */
export async function gameFinalPosition(game, { signal } = {}) {
  const key = `${game.id}:${game.updated_at || ''}`;
  const hit = cacheGet(key);
  if (hit) return hit;
  let moves = game.moves;
  let start = game.start_fen;
  if (!Array.isArray(moves)) {
    const rec = await withSlot(() => api.get(`/api/games/${encodeURIComponent(game.id)}`, { signal }), signal);
    moves = rec?.moves || [];
    start = rec?.start_fen || start;
  }
  const pos = await playUci(start, moves);
  cacheSet(key, pos);
  return pos;
}

/** Outcome of a game from the user's perspective. */
export function gameOutcome(g) {
  const r = g?.result;
  if (r === '1/2-1/2') return 'draw';
  if (r !== '1-0' && r !== '0-1') return 'ongoing';
  const uc = g.user_color;
  if (uc !== 'white' && uc !== 'black') return r === '1-0' ? 'white' : 'black';
  return (r === '1-0') === (uc === 'white') ? 'win' : 'loss';
}

// Labels resolve at render time via t('library.outcome.<key>') / t('library.outcomeShort.<key>').
const OUTCOME_CLS = {
  win: 'result-win', loss: 'result-loss', draw: 'result-draw', ongoing: 'result-draw', white: 'result-draw', black: 'result-draw',
};

/** Small result marker element (W / L / ½). */
export function resultMarker(g) {
  const o = gameOutcome(g);
  const label = t(`library.outcome.${o}`);
  return h('span', { class: `result ${OUTCOME_CLS[o]}`, title: label, 'aria-label': label }, t(`library.outcomeShort.${o}`));
}

export function outcomeLabel(g) { return t(`library.outcome.${gameOutcome(g)}`); }

/** User's accuracy in a game (or null). */
export function userAccuracy(g) {
  const v = g?.user_color === 'black' ? g.accuracy_black : g?.user_color === 'white' ? g.accuracy_white : null;
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

/** Accuracy pill element. */
export function accuracyPill(acc) {
  if (acc == null) return h('span', { class: 'hub-acc hub-acc-none', title: t('library.accuracy.notReviewed') }, '—');
  const tier = acc >= 85 ? 'hi' : acc >= 65 ? 'mid' : 'lo';
  return h('span', { class: `hub-acc hub-acc-${tier}`, title: t('library.accuracy.yours') }, acc.toFixed(1));
}

/** "You vs Martin" style title. */
export function gameTitle(g, botsById) {
  const uc = g.user_color;
  if (uc === 'white' || uc === 'black') {
    const opp = uc === 'white' ? g.black : g.white;
    const bot = g.bot_id && botsById ? botsById.get(g.bot_id) : null;
    return t('library.gameTitle.you', { opponent: bot?.name || opp || t('library.gameTitle.opponent') });
  }
  return t('library.gameTitle.players', { white: g.white || t('library.gameTitle.white'), black: g.black || t('library.gameTitle.black') });
}

export function fullMoves(g) {
  const n = Number(g?.move_count ?? (Array.isArray(g?.moves) ? g.moves.length : 0)) || 0;
  return Math.ceil(n / 2);
}

// ---------------------------------------------------------------------------
// Library page
// ---------------------------------------------------------------------------

const PAGE_SIZE = 100;
const MAX_LOADED = 2000;
const MAX_PGN_BYTES = 4 * 1024 * 1024;
const MAX_TAGS = 12;

// Remember filters while the app is open (small, bounded state).
const remembered = { search: '', outcome: 'all', botId: '', favorite: false, sort: 'newest' };

// Sort labels: t('library.sort.<key>') at render time.
const SORTS = {
  newest: { fn: (a, b) => cmpStr(b.created_at, a.created_at) || b.id - a.id },
  oldest: { fn: (a, b) => cmpStr(a.created_at, b.created_at) || a.id - b.id },
  longest: { fn: (a, b) => (b.move_count || 0) - (a.move_count || 0) },
  accuracy: { fn: (a, b) => (userAccuracy(b) ?? -1) - (userAccuracy(a) ?? -1) },
};
function cmpStr(a, b) { return String(a || '').localeCompare(String(b || '')); }

export async function mount(root) {
  await ensureHubCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;

  const state = {
    ...remembered,
    games: [],
    offset: 0,
    hasMore: false,
    loading: false,
    error: null,
    bots: [],
    botsById: new Map(),
  };
  let listReq = null; // AbortController for the current list request
  bag.add(() => listReq?.abort());
  let openModal = null;
  bag.add(() => openModal?.close());

  // ---- Thumbnails: lazy, observed only while visible -----------------------
  const io = 'IntersectionObserver' in window ? new IntersectionObserver(onIntersect, { rootMargin: '200px 0px' }) : null;
  bag.add(() => io?.disconnect());
  function onIntersect(entries) {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      io.unobserve(e.target);
      fillThumb(e.target);
    }
  }
  function fillThumb(el) {
    const id = Number(el.dataset.id);
    const g = state.games.find((x) => x.id === id);
    if (!g) return;
    gameFinalPosition(g, { signal }).then((pos) => {
      if (signal.aborted || !el.isConnected) return;
      el.innerHTML = fenBoardSvg(pos.fen, { orientation: g.user_color === 'black' ? 'black' : 'white', lastMove: pos.lastMove, label: t('library.board.finalPosition', { title: gameTitle(g, state.botsById) }) });
      el.classList.remove('loading');
    }).catch((e) => {
      if (isAbort(e) || !el.isConnected) return;
      el.classList.remove('loading');
      el.innerHTML = fenBoardSvg(g.start_fen || START_FEN, { orientation: g.user_color === 'black' ? 'black' : 'white' });
    });
  }

  // ---- Layout ----------------------------------------------------------------
  const importBtn = h('button', { class: 'btn btn-primary', type: 'button', html: icon('upload') + `<span>${escapeHtml(t('library.importPgn'))}</span>`, onClick: () => openImport() });
  const searchInput = h('input', { class: 'input', type: 'search', placeholder: t('library.search.placeholder'), value: state.search, 'aria-label': t('library.search.label'), maxlength: '100' });
  const outcomeSeg = h('div', { class: 'segmented', role: 'group', 'aria-label': t('library.filter.label') },
    [['all', t('library.filter.all')], ['win', t('library.filter.won')], ['loss', t('library.filter.lost')], ['draw', t('library.filter.drawn')]].map(([v, l]) =>
      h('button', { type: 'button', class: state.outcome === v ? 'active' : null, dataset: { v }, 'aria-pressed': String(state.outcome === v) }, l)));
  const botSelect = h('select', { class: 'select', 'aria-label': t('library.filter.opponentLabel') }, h('option', { value: '' }, t('library.filter.allOpponents')));
  const favChip = h('button', { type: 'button', class: ['chip', state.favorite && 'active'], 'aria-pressed': String(state.favorite), html: icon('star', { size: 16 }) + `<span>${escapeHtml(t('library.filter.favorites'))}</span>` });
  const sortSelect = h('select', { class: 'select', 'aria-label': t('library.sort.label') },
    Object.keys(SORTS).map((v) => h('option', { value: v, selected: v === state.sort }, t(`library.sort.${v}`))));
  const summaryEl = h('div', { class: 'hub-lib-summary', 'aria-live': 'polite' });
  const listEl = h('div', { class: 'hub-game-list' });
  const moreBtn = h('button', { class: 'btn btn-secondary', type: 'button', hidden: true }, t('library.loadMore'));
  const footer = h('div', { class: 'hub-lib-footer' }, moreBtn);

  const page = h('div', { class: 'page hub-page' },
    pageHeader({ title: t('library.title'), subtitle: t('library.subtitle'), icon: 'library', actions: [importBtn] }),
    h('div', { class: 'card hub-lib-filters' },
      h('div', { class: 'input-group hub-lib-search', html: icon('search') }, searchInput),
      outcomeSeg,
      h('div', { class: 'row-sm hub-lib-selects' }, botSelect, sortSelect, favChip)),
    summaryEl,
    listEl,
    footer);
  root.appendChild(page);

  // ---- Events (delegated) ------------------------------------------------------
  const onSearch = debounce(() => { state.search = searchInput.value.trim(); remembered.search = state.search; reload(); }, 280);
  bag.add(onSearch.cancel);
  bag.on(searchInput, 'input', onSearch);
  bag.on(outcomeSeg, 'click', (e) => {
    const b = e.target.closest('button[data-v]');
    if (!b) return;
    state.outcome = remembered.outcome = b.dataset.v;
    for (const x of outcomeSeg.children) { const on = x === b; x.classList.toggle('active', on); x.setAttribute('aria-pressed', String(on)); }
    renderList();
  });
  bag.on(botSelect, 'change', () => { state.botId = remembered.botId = botSelect.value; reload(); });
  bag.on(sortSelect, 'change', () => { state.sort = remembered.sort = sortSelect.value; renderList(); });
  bag.on(favChip, 'click', () => {
    state.favorite = remembered.favorite = !state.favorite;
    favChip.classList.toggle('active', state.favorite);
    favChip.setAttribute('aria-pressed', String(state.favorite));
    reload();
  });
  bag.on(moreBtn, 'click', () => loadPage(false));
  bag.on(listEl, 'click', onListClick);

  // ---- Data --------------------------------------------------------------------
  api.get('/api/bots', { signal }).then((bots) => {
    if (!Array.isArray(bots)) return;
    state.bots = bots;
    state.botsById = new Map(bots.map((b) => [b.id, b]));
    for (const b of [...bots].sort((a, b2) => (a.elo || 0) - (b2.elo || 0))) {
      botSelect.appendChild(h('option', { value: b.id, selected: b.id === state.botId }, `${b.avatar || ''} ${b.name} (${b.elo})`));
    }
    renderList();
  }).catch(() => { /* filter just stays minimal */ });

  function reload() { loadPage(true); }

  async function loadPage(reset) {
    listReq?.abort();
    const req = new AbortController();
    listReq = req;
    const onOuter = () => req.abort();
    signal.addEventListener('abort', onOuter, { once: true });
    if (reset) {
      state.offset = 0;
      state.games = [];
      io?.disconnect();
      listEl.replaceChildren(skeleton('list', 5));
      summaryEl.textContent = '';
    }
    state.loading = true;
    moreBtn.classList.add('loading');
    try {
      const rows = await api.get('/api/games' + qs({
        search: state.search, bot_id: state.botId, favorite: state.favorite ? 'true' : null,
        limit: PAGE_SIZE, offset: state.offset,
      }), { signal: req.signal });
      if (req.signal.aborted) return;
      const list = Array.isArray(rows) ? rows : [];
      const seen = new Set(state.games.map((g) => g.id));
      for (const g of list) if (g && typeof g.id === 'number' && !seen.has(g.id)) state.games.push(normalize(g));
      state.offset += list.length;
      state.hasMore = list.length === PAGE_SIZE && state.games.length < MAX_LOADED;
      state.error = null;
    } catch (e) {
      if (isAbort(e) || req.signal.aborted) return;
      state.error = e?.message || t('library.errors.load');
    } finally {
      signal.removeEventListener('abort', onOuter);
      if (listReq === req) { state.loading = false; moreBtn.classList.remove('loading'); listReq = null; }
    }
    renderList();
  }

  function normalize(g) {
    return { ...g, tags: Array.isArray(g.tags) ? g.tags.filter((tag) => typeof tag === 'string' && tag) : [], notes: typeof g.notes === 'string' ? g.notes : '' };
  }

  function visibleGames() {
    let list = state.games;
    if (state.outcome !== 'all') list = list.filter((g) => gameOutcome(g) === state.outcome);
    return [...list].sort(SORTS[state.sort]?.fn || SORTS.newest.fn);
  }

  function renderSummary(list) {
    const counts = { win: 0, loss: 0, draw: 0 };
    for (const g of state.games) { const o = gameOutcome(g); if (o in counts) counts[o]++; }
    const total = state.games.length;
    summaryEl.replaceChildren(
      h('span', { class: 'semibold' }, t('library.summary.games', { count: list.length })),
      total ? h('span', { class: 'hub-lib-counts' },
        h('span', { class: 'hub-dot hub-dot-win' }), t('library.summary.won', { count: counts.win }),
        h('span', { class: 'hub-dot hub-dot-draw' }), t('library.summary.drawn', { count: counts.draw }),
        h('span', { class: 'hub-dot hub-dot-loss' }), t('library.summary.lost', { count: counts.loss })) : null);
  }

  function renderList() {
    if (state.loading && !state.games.length) return;
    io?.disconnect();
    moreBtn.hidden = !state.hasMore;
    if (state.error && !state.games.length) {
      summaryEl.textContent = '';
      listEl.replaceChildren(emptyState({ icon: 'wifi-off', title: t('library.errors.loadTitle'), text: state.error, action: { label: t('library.errors.retry'), icon: 'refresh', onClick: reload } }));
      return;
    }
    const list = visibleGames();
    renderSummary(list);
    if (!list.length) {
      const filtered = state.search || state.botId || state.favorite || state.outcome !== 'all';
      listEl.replaceChildren(filtered
        ? emptyState({ icon: 'filter', title: t('library.empty.filteredTitle'), text: t('library.empty.filteredText'), action: { label: t('library.empty.clearFilters'), kind: 'secondary', onClick: clearFilters } })
        : emptyState({ emoji: '♟️', title: t('library.empty.title'), text: t('library.empty.text'), action: { label: t('library.empty.playBot'), href: '#/play', icon: 'play' } }));
      return;
    }
    const frag = document.createDocumentFragment();
    for (const g of list) frag.appendChild(renderRow(g));
    listEl.replaceChildren(frag);
    observeThumbs(listEl);
  }

  function observeThumbs(scope) {
    for (const el of scope.querySelectorAll('.hub-thumb.loading')) {
      if (io) io.observe(el); else fillThumb(el);
    }
  }

  function clearFilters() {
    Object.assign(state, { search: '', outcome: 'all', botId: '', favorite: false });
    Object.assign(remembered, { search: '', outcome: 'all', botId: '', favorite: false });
    searchInput.value = '';
    botSelect.value = '';
    favChip.classList.remove('active');
    favChip.setAttribute('aria-pressed', 'false');
    for (const x of outcomeSeg.children) { const on = x.dataset.v === 'all'; x.classList.toggle('active', on); x.setAttribute('aria-pressed', String(on)); }
    reload();
  }

  function renderRow(g) {
    const bot = g.bot_id ? state.botsById.get(g.bot_id) : null;
    const key = `${g.id}:${g.updated_at || ''}`;
    const cached = cacheGet(key);
    const orientation = g.user_color === 'black' ? 'black' : 'white';
    const thumb = h('a', {
      class: ['hub-thumb', !cached && 'loading'], href: `#/review/${g.id}`, dataset: { id: g.id },
      'aria-label': t('library.row.reviewAria', { title: gameTitle(g, state.botsById) }),
      html: cached ? fenBoardSvg(cached.fen, { orientation, lastMove: cached.lastMove }) : '',
    });
    const meta = [
      g.opening_name || null,
      t('library.row.moves', { count: fullMoves(g) }),
      g.time_control && g.time_control !== '-' ? g.time_control : null,
      formatRelativeIntl(g.created_at) || null,
    ].filter(Boolean);
    const tags = g.tags.length ? h('div', { class: 'hub-tags' }, g.tags.map((tag) => h('span', { class: 'badge hub-tag' }, `#${tag}`))) : null;
    const notes = g.notes ? h('p', { class: 'hub-game-notes', title: g.notes }, g.notes) : null;
    const acc = userAccuracy(g);
    const actBtn = (action, ic, label, extra = {}) => h('button', { type: 'button', class: 'btn btn-ghost btn-icon btn-sm', dataset: { action, id: g.id }, 'aria-label': label, 'data-tooltip': label, html: icon(ic), ...extra });
    return h('article', { class: ['hub-game', g.favorite && 'is-fav'], dataset: { id: g.id } },
      thumb,
      h('div', { class: 'hub-game-main' },
        h('div', { class: 'hub-game-title' },
          resultMarker(g),
          bot ? h('span', { class: 'avatar avatar-xs', 'aria-hidden': 'true' }, bot.avatar || '🤖') : null,
          h('a', { class: 'truncate', href: `#/review/${g.id}` }, gameTitle(g, state.botsById)),
          bot ? h('span', { class: 'muted text-sm nowrap' }, `(${bot.elo})`) : null),
        h('div', { class: 'hub-game-sub muted text-sm', title: formatGameDate(g.created_at) }, meta.join(' · ')),
        tags, notes),
      h('div', { class: 'hub-game-acc' }, h('span', { class: 'subtle text-xs' }, t('library.row.accuracy')), accuracyPill(acc)),
      h('div', { class: 'hub-game-actions' },
        h('button', {
          type: 'button', class: ['btn btn-ghost btn-icon btn-sm hub-star', g.favorite && 'on'], dataset: { action: 'fav', id: g.id },
          'aria-pressed': String(!!g.favorite), 'aria-label': g.favorite ? t('library.row.unfavorite') : t('library.row.favorite'),
          'data-tooltip': g.favorite ? t('library.row.unfavorite') : t('library.row.favorite'),
          html: icon(g.favorite ? 'star-filled' : 'star'),
        }),
        h('a', { class: 'btn btn-primary btn-sm', href: `#/review/${g.id}`, html: icon('sparkles', { size: 16 }) + `<span>${escapeHtml(t('library.row.review'))}</span>` }),
        h('a', { class: 'btn btn-ghost btn-icon btn-sm', href: `#/analysis?game=${g.id}`, 'aria-label': t('library.row.analyze'), 'data-tooltip': t('library.row.analyze'), html: icon('analysis') }),
        h('a', { class: 'btn btn-ghost btn-icon btn-sm', href: `/api/games/${g.id}/pgn`, download: `grandmentor-game-${g.id}.pgn`, 'aria-label': t('library.row.download'), 'data-tooltip': t('library.row.download'), html: icon('download') }),
        actBtn('edit', 'edit', t('library.row.notes')),
        actBtn('delete', 'trash', t('library.row.delete'))));
  }

  function replaceRow(g) {
    const old = listEl.querySelector(`.hub-game[data-id="${g.id}"]`);
    if (!old) return;
    if (state.outcome !== 'all' && gameOutcome(g) !== state.outcome) { renderList(); return; }
    const row = renderRow(g);
    old.replaceWith(row);
    observeThumbs(row);
  }

  function onListClick(e) {
    const btn = e.target.closest('button[data-action]');
    if (!btn) return;
    const id = Number(btn.dataset.id);
    const g = state.games.find((x) => x.id === id);
    if (!g) return;
    if (btn.dataset.action === 'fav') toggleFav(g);
    else if (btn.dataset.action === 'edit') openEdit(g);
    else if (btn.dataset.action === 'delete') deleteGame(g);
  }

  async function toggleFav(g) {
    const next = !g.favorite;
    g.favorite = next;
    replaceRow(g);
    try {
      const rec = await api.put(`/api/games/${g.id}`, { favorite: next }, { signal });
      if (rec && typeof rec === 'object') { g.updated_at = rec.updated_at || g.updated_at; }
      if (state.favorite && !next) { state.games = state.games.filter((x) => x.id !== g.id); renderList(); }
      toast(next ? t('library.toast.favAdded') : t('library.toast.favRemoved'), 'success', { duration: 1800 });
    } catch (e) {
      if (isAbort(e)) return;
      g.favorite = !next;
      replaceRow(g);
      toast(e?.message || t('library.toast.favError'), 'error');
    }
  }

  function openEdit(g) {
    const notesEl = h('textarea', { class: 'textarea', rows: '5', maxlength: '5000', placeholder: t('library.edit.notesPlaceholder') });
    notesEl.value = g.notes || '';
    const tagsEl = h('input', { class: 'input', placeholder: t('library.edit.tagsPlaceholder'), value: g.tags.join(', '), maxlength: '300' });
    const preview = h('div', { class: 'hub-tags' });
    const updatePreview = () => preview.replaceChildren(...parseTags(tagsEl.value).map((tag) => h('span', { class: 'badge hub-tag' }, `#${tag}`)));
    tagsEl.addEventListener('input', updatePreview); // freed with the modal DOM
    updatePreview();
    const body = h('div', { class: 'stack' },
      h('div', { class: 'field' }, h('label', { class: 'label' }, t('library.edit.notes')), notesEl),
      h('div', { class: 'field' }, h('label', { class: 'label' }, t('library.edit.tags')), tagsEl,
        h('div', { class: 'help' }, t('library.edit.tagsHelp')), preview));
    openModal = modal({
      title: t('library.edit.title', { title: gameTitle(g, state.botsById) }),
      body,
      actions: [
        { label: t('library.actions.cancel'), kind: 'ghost' },
        {
          label: t('library.actions.save'), kind: 'primary', icon: 'save',
          onClick: async () => {
            const patch = { notes: notesEl.value.slice(0, 5000), tags: parseTags(tagsEl.value) };
            const rec = await api.put(`/api/games/${g.id}`, patch, { signal });
            Object.assign(g, patch, rec && typeof rec === 'object' ? { updated_at: rec.updated_at || g.updated_at } : {});
            replaceRow(g);
            toast(t('library.edit.saved'), 'success');
          },
        },
      ],
      onClose: () => { openModal = null; },
    });
  }

  async function deleteGame(g) {
    const ok = await confirmDialog({
      title: t('library.delete.title'),
      message: t('library.delete.message', { title: gameTitle(g, state.botsById) }),
      confirmLabel: t('library.delete.confirm'),
      cancelLabel: t('library.actions.cancel'),
      danger: true,
    });
    if (!ok || signal.aborted) return;
    try {
      await api.del(`/api/games/${g.id}`, { signal });
      state.games = state.games.filter((x) => x.id !== g.id);
      state.offset = Math.max(0, state.offset - 1);
      renderList();
      toast(t('library.delete.done'), 'success');
    } catch (e) {
      if (!isAbort(e)) toast(e?.message || t('library.delete.error'), 'error');
    }
  }

  function openImport() {
    const ta = h('textarea', { class: 'textarea mono hub-pgn-input', rows: '10', placeholder: t('library.import.placeholder'), spellcheck: 'false' });
    const fileInput = h('input', { type: 'file', accept: '.pgn,.txt,application/x-chess-pgn,text/plain', class: 'sr-only' });
    const fileName = h('span', { class: 'muted text-sm truncate' }, t('library.import.noFile'));
    const pickBtn = h('button', { type: 'button', class: 'btn btn-secondary btn-sm', html: icon('folder') + `<span>${escapeHtml(t('library.import.chooseFile'))}</span>` });
    const drop = h('div', { class: 'hub-drop' }, ta, h('div', { class: 'hub-drop-hint subtle text-xs' }, t('library.import.dropHint')));
    const readFile = async (file) => {
      if (!file) return;
      if (file.size > MAX_PGN_BYTES) { toast(t('library.import.fileTooLarge'), 'warning'); return; }
      try {
        ta.value = await file.text();
        fileName.textContent = file.name;
      } catch { toast(t('library.import.readError'), 'error'); }
    };
    pickBtn.addEventListener('click', () => fileInput.click());
    fileInput.addEventListener('change', () => readFile(fileInput.files?.[0]));
    drop.addEventListener('dragover', (e) => { e.preventDefault(); drop.classList.add('over'); });
    drop.addEventListener('dragleave', () => drop.classList.remove('over'));
    drop.addEventListener('drop', (e) => { e.preventDefault(); drop.classList.remove('over'); readFile(e.dataTransfer?.files?.[0]); });
    const body = h('div', { class: 'stack' },
      h('p', { class: 'muted' }, t('library.import.intro')),
      drop,
      h('div', { class: 'row-sm' }, pickBtn, fileInput, fileName));
    openModal = modal({
      title: t('library.import.title'),
      size: 'lg',
      body,
      actions: [
        { label: t('library.actions.cancel'), kind: 'ghost' },
        {
          label: t('library.import.submit'), kind: 'primary', icon: 'upload',
          onClick: async () => {
            const pgn = ta.value.trim();
            if (!pgn) { toast(t('library.import.empty'), 'warning'); ta.focus(); return false; }
            if (pgn.length > MAX_PGN_BYTES) { toast(t('library.import.pgnTooLarge'), 'warning'); return false; }
            const res = await api.post('/api/games/import', { pgn }, { signal, timeout: 120000 });
            const n = Array.isArray(res) ? res.length : 0;
            if (!n) { toast(t('library.import.noneFound'), 'warning'); return false; }
            toast(t('library.import.imported', { count: n }), 'success');
            reload();
            return true;
          },
        },
      ],
      onClose: () => { openModal = null; },
    });
  }

  reload();
  return bag.dispose;
}

function parseTags(text) {
  const out = [];
  for (const raw of String(text || '').split(',')) {
    const tag = raw.trim().replace(/^#+/, '').replace(/\s+/g, '-').toLowerCase().slice(0, 24);
    if (tag && !out.includes(tag)) out.push(tag);
    if (out.length >= MAX_TAGS) break;
  }
  return out;
}
