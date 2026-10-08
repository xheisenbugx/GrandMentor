// Openings library (#/openings) and opening detail (#/openings/:id).
// Detail has three modes: Learn (play through the mainline), Explore (book continuations from
// /api/openings/lookup as weighted bars, chess.com explorer style) and Train (play your repertoire
// side from memory against auto-replies, with a spaced-repetition-lite streak in localStorage).

import { h, icon, disposables, pageHeader, emptyState, skeleton, mdLite, escapeHtml, debounce, formatSan, loadingBlock } from '../ui.js';
import { api, qs, isAbort } from '../api.js';
import { getSetting } from '../settings.js';
import { Board } from '../components/board.js';
import {
  ensureLearnCss, START_FEN, chessAt, applyUci, isSameMove, sideToMove, playLine, sanLineToUci, fenKey,
  miniBoardSvg, levelPill, timerSet, confetti, flashClass, sfx, setFeedback, breadcrumbs, errorBlock,
  readStore, writeStore,
} from './learn.js';

export const title = (params) => (params && params.id ? 'Opening' : 'Openings');

// ---------------------------------------------------------------------------
// Training memory (spaced repetition lite)
// ---------------------------------------------------------------------------
const TRAIN_KEY = 'gm.openings.train.v1';
const DAY = 86400000;
const INTERVAL_DAYS = [0, 1, 3, 7, 14, 30];
const LEARNED_STREAK = 3;

function trainDb() { return readStore(TRAIN_KEY, {}); }
function trainRecord(id) {
  const r = trainDb()[id];
  return r && typeof r === 'object' ? r : null;
}
function saveTrainResult(id, perfect, scorePct, validIds) {
  const db = trainDb();
  // Bound the store to known openings.
  if (validIds) for (const k of Object.keys(db)) if (!validIds.has(k)) delete db[k];
  const prev = db[id] || { streak: 0, best: 0, runs: 0 };
  const streak = perfect ? (prev.streak || 0) + 1 : 0;
  const days = INTERVAL_DAYS[Math.min(streak, INTERVAL_DAYS.length - 1)];
  const now = Date.now();
  const rec = { streak, best: Math.max(prev.best || 0, scorePct), runs: (prev.runs || 0) + 1, last: now, due: now + days * DAY };
  db[id] = rec;
  writeStore(TRAIN_KEY, db);
  return rec;
}
function isDue(rec) { return !!rec && typeof rec.due === 'number' && rec.due <= Date.now(); }
function isLearned(rec) { return !!rec && (rec.streak || 0) >= LEARNED_STREAK; }
function dueLabel(rec) {
  if (!rec || typeof rec.due !== 'number') return '';
  const ms = rec.due - Date.now();
  if (ms <= 0) return 'Due now';
  const d = Math.ceil(ms / DAY);
  return d <= 1 ? 'Review tomorrow' : `Review in ${d} days`;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function sanOf(san) { return formatSan(san, getSetting('moveNotation')); }

/** "1. e4 e5 2. Nf3" from SAN list (startPly 0 = white to move first). */
function numberedLine(sans, startBlack = false) {
  const out = [];
  let moveNo = 1;
  sans.forEach((s, i) => {
    const whiteMove = startBlack ? i % 2 === 1 : i % 2 === 0;
    if (whiteMove) out.push(`${moveNo}.`);
    else if (i === 0) out.push(`${moveNo}…`);
    out.push(sanOf(s));
    if (!whiteMove) moveNo++;
  });
  return out.join(' ');
}

function openingLine(o) {
  const ucis = Array.isArray(o.uci) && o.uci.length ? o.uci : sanLineToUci(o.moves);
  return playLine(START_FEN, ucis);
}

function popularityDots(p) {
  const n = Math.max(1, Math.min(5, Math.round((Number(p) || 0) / 2)));
  return h('span', { class: 'lrn-dots', title: `Popularity ${p}/10`, 'aria-label': `Popularity ${n} of 5` },
    [1, 2, 3, 4, 5].map((i) => h('i', { class: i <= n ? 'on' : '' })));
}

function sidePill(side) {
  const s = side === 'black' ? 'black' : 'white';
  return h('span', { class: `lrn-side-pill ${s}` }, s === 'white' ? 'White' : 'Black');
}

// ===========================================================================
// Page entry
// ===========================================================================
export async function mount(root, { params = {}, query = {} } = {}) {
  ensureLearnCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  try {
    if (params.id) await mountDetail(root, params.id, query || {}, bag, ctrl.signal);
    else await mountLibrary(root, bag, ctrl.signal);
  } catch (e) {
    if (!isAbort(e) && !bag.disposed) {
      root.replaceChildren(h('div', { class: 'page' }, errorBlock(e && e.message)));
    }
  }
  return () => bag.dispose();
}

// ===========================================================================
// Library
// ===========================================================================
const PAGE_SIZE = 60;

async function mountLibrary(root, bag, signal) {
  const page = h('div', { class: 'page' });
  root.appendChild(page);
  const header = pageHeader({
    title: 'Openings', icon: 'openings',
    subtitle: 'Learn the most popular openings, explore book moves and train your repertoire from memory.',
    actions: [h('a', { class: 'btn btn-secondary', href: '#/analysis', html: icon('analysis') + '<span>Analysis board</span>' })],
  });
  page.replaceChildren(header, h('div', { class: 'op-grid mt-6' }, skeleton('card', 8)));

  const raw = await api.get('/api/openings', { signal });
  if (bag.disposed) return;
  const all = (Array.isArray(raw) ? raw : []).filter((o) => o && o.id && o.name);
  const validIds = new Set(all.map((o) => o.id));
  if (!all.length) {
    page.replaceChildren(header, emptyState({ emoji: '📖', title: 'No openings yet', text: 'Openings appear here once data/openings.json has entries.' }));
    return;
  }
  // Precompute search text + final position once.
  const items = all.map((o) => {
    let fen = o.fen;
    if (!fen || !chessAt(fen)) { const line = openingLine(o); fen = line.fens[line.fens.length - 1]; }
    return { o, fen, text: `${o.name} ${o.eco} ${o.family} ${o.moves}`.toLowerCase() };
  });
  const families = [...new Set(all.map((o) => o.family).filter(Boolean))].sort((a, b) => a.localeCompare(b));

  const state = { q: '', side: 'all', level: 'all', family: 'all', sort: 'popular', limit: PAGE_SIZE, dueOnly: false };

  // --- Controls
  const search = h('input', { class: 'input', type: 'search', placeholder: 'Search by name, ECO or moves (e.g. Sicilian, C50, Nf3)…', 'aria-label': 'Search openings' });
  const runSearch = debounce(() => { state.q = search.value.trim().toLowerCase(); state.limit = PAGE_SIZE; render(); }, 150);
  bag.add(() => runSearch.cancel());
  search.addEventListener('input', runSearch);

  const sideSeg = h('div', { class: 'segmented', role: 'group', 'aria-label': 'Repertoire side' });
  const renderSide = () => sideSeg.replaceChildren(...[['all', 'All'], ['white', 'White'], ['black', 'Black']].map(([k, label]) =>
    h('button', { type: 'button', class: state.side === k ? 'active' : '', 'aria-pressed': state.side === k ? 'true' : 'false', onClick: () => { state.side = k; state.limit = PAGE_SIZE; renderSide(); render(); } }, label)));
  renderSide();

  const levelSel = h('select', { class: 'select', 'aria-label': 'Level', onChange: (e) => { state.level = e.target.value; state.limit = PAGE_SIZE; render(); } },
    h('option', { value: 'all' }, 'All levels'), h('option', { value: 'beginner' }, 'Beginner'), h('option', { value: 'intermediate' }, 'Intermediate'), h('option', { value: 'advanced' }, 'Advanced'));
  const familySel = h('select', { class: 'select', 'aria-label': 'Opening family', onChange: (e) => { state.family = e.target.value; state.limit = PAGE_SIZE; render(); } },
    h('option', { value: 'all' }, 'All families'), families.map((f) => h('option', { value: f }, f)));
  const sortSel = h('select', { class: 'select', 'aria-label': 'Sort', onChange: (e) => { state.sort = e.target.value; render(); } },
    h('option', { value: 'popular' }, 'Most popular'), h('option', { value: 'family' }, 'Group by family'), h('option', { value: 'name' }, 'Name A–Z'), h('option', { value: 'eco' }, 'ECO code'));

  const toolbar = h('div', { class: 'lrn-toolbar' },
    h('div', { class: 'input-group', html: icon('search') }, search),
    sideSeg, levelSel, familySel, sortSel);

  // --- Training strip
  const trainingBox = h('div');
  const renderTraining = () => {
    const db = trainDb();
    const trained = items.filter((it) => db[it.o.id]);
    if (!trained.length) {
      trainingBox.replaceChildren(h('div', { class: 'callout mt-6', html: icon('info') + '<div><strong>Tip:</strong> open any opening and press <strong>Train</strong> to practise it from memory. We will remind you when it is time to review.</div>' }));
      return;
    }
    const due = trained.filter((it) => isDue(db[it.o.id]));
    const learned = trained.filter((it) => isLearned(db[it.o.id]));
    trainingBox.replaceChildren(h('section', { class: 'lrn-section' },
      h('div', { class: 'lrn-section-head' },
        h('span', { class: 'lrn-section-emoji', 'aria-hidden': 'true' }, '🧠'),
        h('div', null, h('h2', null, 'Your repertoire training ', h('span', { class: 'lrn-count' }, `· ${learned.length} learned · ${due.length} due`)),
          h('div', { class: 'muted text-sm' }, 'Perfect runs grow your streak; reviews space out as you remember more.'))),
      due.length
        ? h('div', { class: 'op-due-strip' }, due.slice(0, 20).map((it) => h('a', { class: 'op-due-item', href: `#/openings/${encodeURIComponent(it.o.id)}?mode=train` },
          h('span', { html: icon('clock'), style: 'display:contents' }),
          h('div', null, h('div', { class: 'semibold' }, it.o.name), h('div', { class: 'text-xs subtle' }, `Streak ${db[it.o.id].streak || 0} · Train now`)))))
        : h('div', { class: 'callout callout-success', html: icon('check-circle') + '<div>All caught up! No openings are due for review right now.</div>' })));
  };
  renderTraining();

  const results = h('div');
  const countEl = h('div', { class: 'muted text-sm mt-4' });

  function filtered() {
    let list = items.filter(({ o, text }) =>
      (state.side === 'all' || o.side === state.side)
      && (state.level === 'all' || o.level === state.level)
      && (state.family === 'all' || o.family === state.family)
      && (!state.q || state.q.split(/\s+/).every((t) => text.includes(t))));
    const byPop = (a, b) => (b.o.popularity || 0) - (a.o.popularity || 0) || a.o.name.localeCompare(b.o.name);
    if (state.sort === 'name') list.sort((a, b) => a.o.name.localeCompare(b.o.name));
    else if (state.sort === 'eco') list.sort((a, b) => String(a.o.eco).localeCompare(String(b.o.eco)) || a.o.name.localeCompare(b.o.name));
    else list.sort(byPop);
    return list;
  }

  function render() {
    const list = filtered();
    const db = trainDb();
    countEl.textContent = `${list.length} opening${list.length === 1 ? '' : 's'}`;
    if (!list.length) {
      results.replaceChildren(emptyState({ icon: 'search', title: 'No openings match', text: 'Try a different search or clear the filters.',
        action: { label: 'Clear filters', kind: 'secondary', onClick: () => { search.value = ''; state.q = ''; state.side = 'all'; state.level = 'all'; state.family = 'all'; levelSel.value = 'all'; familySel.value = 'all'; renderSide(); render(); } } }));
      return;
    }
    const shown = list.slice(0, state.limit);
    const more = list.length > shown.length
      ? h('div', { class: 'center mt-6' }, h('button', { class: 'btn btn-secondary', type: 'button', onClick: () => { state.limit += PAGE_SIZE; render(); } }, `Show more (${list.length - shown.length} left)`))
      : null;
    if (state.sort === 'family') {
      const groups = new Map();
      for (const it of shown) {
        const f = it.o.family || 'Other';
        if (!groups.has(f)) groups.set(f, []);
        groups.get(f).push(it);
      }
      const ordered = [...groups.entries()].sort((a, b) => Math.max(...b[1].map((x) => x.o.popularity || 0)) - Math.max(...a[1].map((x) => x.o.popularity || 0)) || a[0].localeCompare(b[0]));
      results.replaceChildren(...ordered.map(([fam, arr]) => h('section', { class: 'lrn-section' },
        h('div', { class: 'lrn-section-head' }, h('h2', null, fam, ' ', h('span', { class: 'lrn-count' }, `· ${arr.length}`))),
        h('div', { class: 'op-grid' }, arr.map((it) => openingCard(it, db))))), more || '');
    } else {
      results.replaceChildren(h('div', { class: 'op-grid mt-4' }, shown.map((it) => openingCard(it, db))), more || '');
    }
  }

  page.replaceChildren(header, toolbar, trainingBox, countEl, results);
  render();
}

function openingCard({ o, fen }, db) {
  const rec = db[o.id];
  const flag = isLearned(rec)
    ? h('span', { class: 'badge badge-gold op-card-flag', html: icon('star-filled') + '<span>Learned</span>' })
    : isDue(rec) ? h('span', { class: 'badge badge-warning op-card-flag', html: icon('clock') + '<span>Review</span>' }) : null;
  let sans = [];
  try { sans = String(o.moves || '').split(/\s+/).filter(Boolean); } catch { /* ignore */ }
  return h('a', { class: 'card card-link op-card', href: `#/openings/${encodeURIComponent(o.id)}`, title: o.name },
    flag,
    h('div', { html: miniBoardSvg(fen, o.side === 'black' ? 'black' : 'white') }),
    h('h3', { class: 'op-card-name' }, o.name),
    h('div', { class: 'op-card-moves' }, numberedLine(sans)),
    h('div', { class: 'op-card-meta' },
      o.eco ? h('span', { class: 'op-eco' }, o.eco) : null,
      sidePill(o.side), levelPill(o.level),
      h('span', { class: 'spacer' }), popularityDots(o.popularity)));
}

// ===========================================================================
// Detail
// ===========================================================================
const LOOKUP_CACHE_MAX = 300;
const lookupCache = new Map(); // fenKey -> OpeningMatch|null (module-level LRU, bounded)

function cacheGet(key) {
  if (!lookupCache.has(key)) return undefined;
  const v = lookupCache.get(key);
  lookupCache.delete(key); lookupCache.set(key, v); // refresh LRU order
  return v;
}
function cachePut(key, v) {
  lookupCache.set(key, v);
  while (lookupCache.size > LOOKUP_CACHE_MAX) lookupCache.delete(lookupCache.keys().next().value);
}

// Client-side book built from /api/openings: used when the server lookup has no match
// (e.g. the starting position, which has no named opening). Built once per session, size = content.
let localBookPromise = null;
function localBook() {
  if (!localBookPromise) {
    localBookPromise = api.get('/api/openings', { timeout: 15000 }).then((list) => {
      const byFen = new Map(); // fenKey -> { named: {id,eco,name,depth}, next: Map(uci -> {uci,san,name,weight}) }
      for (const o of Array.isArray(list) ? list : []) {
        if (!o || !o.id) continue;
        const line = openingLine(o);
        for (let i = 0; i < line.fens.length; i++) {
          const k = fenKey(line.fens[i]);
          let node = byFen.get(k);
          if (!node) { node = { named: null, next: new Map() }; byFen.set(k, node); }
          if (i === line.fens.length - 1 && (!node.named || (o.popularity || 0) > (node.named.popularity || 0))) {
            node.named = { id: o.id, eco: o.eco || '', name: o.name, popularity: o.popularity || 0 };
          }
          if (i < line.ucis.length) {
            const u = line.ucis[i];
            const prev = node.next.get(u);
            const w = Math.max(1, Number(o.popularity) || 1);
            if (prev) prev.weight += w;
            else node.next.set(u, { uci: u, san: line.sans[i], name: i === line.ucis.length - 1 ? o.name : null, weight: w });
          }
        }
      }
      return byFen;
    }).catch(() => { localBookPromise = null; return new Map(); });
  }
  return localBookPromise;
}

/** Merge a server OpeningMatch (may be null) with the local book for `fen`. */
async function lookupWithFallback(fen, serverRes) {
  const hasServerConts = serverRes && Array.isArray(serverRes.continuations) && serverRes.continuations.length;
  if (serverRes && hasServerConts) return serverRes;
  const book = await localBook();
  const node = book.get(fenKey(fen));
  if (!node && !serverRes) return null;
  const conts = node ? [...node.next.values()] : [];
  const opening = serverRes && serverRes.opening ? serverRes.opening : (node && node.named ? { id: node.named.id, eco: node.named.eco, name: node.named.name } : null);
  if (!opening && !conts.length) return null;
  return { opening, continuations: conts };
}

async function mountDetail(root, id, query, bag, signal) {
  root.appendChild(loadingBlock('Loading opening…'));
  let o;
  try {
    o = await api.get(`/api/openings/${encodeURIComponent(id)}`, { signal });
  } catch (e) {
    if (isAbort(e)) throw e;
    if (e && e.status === 404) {
      root.replaceChildren(h('div', { class: 'page' }, emptyState({ emoji: '🔍', title: 'Opening not found', text: 'We could not find this opening.', action: { label: 'All openings', href: '#/openings' } })));
      return;
    }
    throw e;
  }
  if (bag.disposed || !o) return;

  const timers = timerSet();
  bag.add(() => timers.clear());
  let lookupCtrl = null;
  bag.add(() => { if (lookupCtrl) lookupCtrl.abort(); });

  const main = openingLine(o);              // { fens, sans, ucis }
  const userSide = o.side === 'black' ? 'black' : 'white';

  // ------------------------------------------------------------------ state
  let mode = 'learn';            // learn | explore | train
  let line = { ...main };        // current line for learn/explore
  let ply = main.ucis.length;    // cursor (0 = start)
  let playing = false;
  let lookupToken = 0;
  let train = null;

  // ------------------------------------------------------------------ layout
  const boardSlot = h('div', { class: 'board-slot' });
  const boardWrap = h('div', { class: 'board-row lrn-board-wrap' }, boardSlot);
  const nameLine = h('div', { class: 'op-opening-name', 'aria-live': 'polite' });

  const btnFirst = toolBtn('first', 'Start (Home)', () => { stopPlay(); setPly(0); });
  const btnPrev = toolBtn('chevron-left', 'Previous (←)', () => { stopPlay(); setPly(ply - 1); });
  const btnPlay = toolBtn('play-circle', 'Play through (Space)', () => togglePlay());
  const btnNext = toolBtn('chevron-right', 'Next (→)', () => { stopPlay(); setPly(ply + 1); });
  const btnLast = toolBtn('last', 'End (End)', () => { stopPlay(); setPly(line.ucis.length); });
  const btnFlip = toolBtn('flip', 'Flip board (F)', () => board.flip());
  const toolbar = h('div', { class: 'toolbar' }, btnFirst, btnPrev, btnPlay, btnNext, btnLast, btnFlip);

  const tabs = h('div', { class: 'tabs', role: 'tablist', style: 'padding:0 var(--sp-3)' });
  const body = h('div', { class: 'panel-body' });
  const analysisLink = h('a', { class: 'btn btn-secondary btn-block', html: icon('analysis') + '<span>Analyze this position</span>' });
  const footer = h('div', { class: 'panel-footer' }, analysisLink);
  const panel = h('div', { class: 'panel grow' }, tabs, body, footer);

  const head = h('div', { class: 'stack-sm' },
    breadcrumbs([{ label: 'Openings', href: '#/openings' }, ...(o.family && o.family !== o.name ? [{ label: o.family }] : []), { label: o.name }]),
    h('div', { class: 'op-head' },
      h('h1', null, o.name),
      o.eco ? h('span', { class: 'op-eco' }, o.eco) : null,
      sidePill(userSide), levelPill(o.level), popularityDots(o.popularity)));

  const layout = h('div', { class: 'game-layout no-eval op-detail' },
    h('div', { class: 'game-main' }, nameLine, boardWrap, toolbar),
    h('aside', { class: 'game-panel' }, head, panel));
  root.replaceChildren(layout);

  const board = new Board(boardSlot, {
    fen: main.fens[ply] || START_FEN,
    orientation: userSide,
    interactive: true,
    movableColor: 'both',
    onMove: (mv) => onBoardMove(mv),
  });
  bag.add(() => board.destroy());

  function toolBtn(ic, label, onClick) {
    return h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(ic), onClick });
  }

  // ------------------------------------------------------------------ tabs
  const TABS = [['learn', 'Learn', 'book'], ['explore', 'Explore', 'search'], ['train', 'Train', 'target']];
  function renderTabs() {
    tabs.replaceChildren(...TABS.map(([k, label, ic]) => h('button', {
      class: ['tab', mode === k && 'active'], role: 'tab', type: 'button', 'aria-selected': mode === k ? 'true' : 'false',
      html: icon(ic) + `<span>${label}</span>`, onClick: () => setMode(k),
    })));
  }

  function setMode(m) {
    if (m === mode && m !== 'train') return;
    stopPlay();
    timers.clear();
    train = null;
    mode = m;
    renderTabs();
    board.clearArrows(); board.setHighlights([]); board.clearBadges();
    if (m === 'learn') {
      line = { ...main };
      setPly(Math.min(ply, line.ucis.length), { force: true });
    } else if (m === 'explore') {
      // Explore starts from wherever the cursor is on the current line.
      line = { fens: line.fens.slice(0, ply + 1), sans: line.sans.slice(0, ply), ucis: line.ucis.slice(0, ply) };
      setPly(ply, { force: true });
    } else {
      startTrain();
    }
  }

  // ------------------------------------------------------------------ navigation (learn/explore)
  function currentFen() { return mode === 'train' && train ? train.fen : (line.fens[ply] || START_FEN); }

  function setPly(p, { force = false, animate = true } = {}) {
    if (mode === 'train') return;
    const np = Math.max(0, Math.min(line.ucis.length, p));
    if (np === ply && !force) return;
    const forward = np === ply + 1;
    ply = np;
    const last = ply > 0 ? [line.ucis[ply - 1].slice(0, 2), line.ucis[ply - 1].slice(2, 4)] : null;
    board.setPosition(line.fens[ply], { animate, lastMove: last, sound: forward });
    board.setInteractive(true, 'both');
    board.clearArrows();
    syncNav();
    renderBody();
    refreshLookup();
  }

  function syncNav() {
    const inTrain = mode === 'train';
    btnFirst.disabled = inTrain || ply === 0;
    btnPrev.disabled = inTrain || ply === 0;
    btnNext.disabled = inTrain || ply >= line.ucis.length;
    btnLast.disabled = inTrain || ply >= line.ucis.length;
    btnPlay.disabled = inTrain || mode !== 'learn';
    btnPlay.innerHTML = icon(playing ? 'pause' : 'play-circle');
    btnPlay.setAttribute('aria-label', playing ? 'Pause' : 'Play through (Space)');
    analysisLink.href = `#/analysis?fen=${encodeURIComponent(currentFen())}`;
  }

  function togglePlay() {
    if (mode !== 'learn') return;
    if (playing) { stopPlay(); return; }
    playing = true;
    if (ply >= line.ucis.length) setPly(0, { animate: false });
    syncNav();
    const tick = () => {
      if (!playing || mode !== 'learn') return;
      if (ply >= line.ucis.length) { stopPlay(); return; }
      setPly(ply + 1);
      if (ply >= line.ucis.length) stopPlay();
      else timers.later(tick, 800);
    };
    timers.later(tick, 450);
  }
  function stopPlay() {
    if (!playing) return;
    playing = false;
    timers.clear();
    syncNav();
  }

  // ------------------------------------------------------------------ board moves
  function onBoardMove(mv) {
    if (mode === 'train') return onTrainMove(mv);
    stopPlay();
    const fenAfter = mv.fen;
    // Following the existing line? Just advance.
    if (ply < line.ucis.length && isSameMove(line.fens[ply], line.ucis[ply], fenAfter)) {
      ply++;
      syncNav(); renderBody(); refreshLookup();
      return true;
    }
    // Otherwise branch: switch to explore with the new move appended.
    if (mode === 'learn') { mode = 'explore'; renderTabs(); }
    line = {
      fens: [...line.fens.slice(0, ply + 1), fenAfter],
      sans: [...line.sans.slice(0, ply), mv.san],
      ucis: [...line.ucis.slice(0, ply), mv.uci],
    };
    ply = line.ucis.length;
    board.clearArrows();
    syncNav(); renderBody(); refreshLookup();
    return true;
  }

  function playBookMove(uci) {
    const c = chessAt(line.fens[ply]);
    const m = c && applyUci(c, uci);
    if (!m) return;
    stopPlay();
    if (mode === 'learn') { mode = 'explore'; renderTabs(); }
    line = {
      fens: [...line.fens.slice(0, ply + 1), c.fen()],
      sans: [...line.sans.slice(0, ply), m.san],
      ucis: [...line.ucis.slice(0, ply), m.from + m.to + (m.promotion || '')],
    };
    ply = line.ucis.length;
    board.setPosition(c.fen(), { animate: true, lastMove: [m.from, m.to] });
    board.clearArrows();
    syncNav(); renderBody(); refreshLookup();
  }

  // ------------------------------------------------------------------ lookup
  let lookup = undefined;      // undefined = loading, null = out of book
  async function refreshLookup() {
    const fen = currentFen();
    const key = fenKey(fen);
    const token = ++lookupToken;
    const cached = cacheGet(key);
    if (cached !== undefined) { lookup = cached; renderLookupBits(); return; }
    lookup = undefined;
    renderLookupBits();
    if (lookupCtrl) lookupCtrl.abort();
    lookupCtrl = new AbortController();
    try {
      let res = null;
      try {
        res = await api.get('/api/openings/lookup' + qs({ fen }), { signal: lookupCtrl.signal, timeout: 10000 });
      } catch (e) { if (isAbort(e)) throw e; }
      const v = await lookupWithFallback(fen, res && typeof res === 'object' && res.opening ? res : null);
      cachePut(key, v);
      if (token !== lookupToken || bag.disposed) return;
      lookup = v;
    } catch (e) {
      if (isAbort(e) || token !== lookupToken || bag.disposed) return;
      lookup = null;
    }
    renderLookupBits();
  }

  function renderLookupBits() {
    // Name line above the board
    const nm = lookup && lookup.opening ? lookup.opening : null;
    if (ply === 0 && mode !== 'train' && !nm) nameLine.innerHTML = icon('book') + '<span>Starting position</span>';
    else if (nm) nameLine.innerHTML = icon('book') + `<span>${nm.eco ? `<span class="op-eco">${escapeHtml(nm.eco)}</span> ` : ''}${escapeHtml(nm.name)}</span>`;
    else if (lookup === undefined) nameLine.innerHTML = '<span class="subtle">…</span>';
    else if (lookup && lookup.continuations && lookup.continuations.length) nameLine.innerHTML = icon('book') + '<span class="subtle">Book position</span>';
    else nameLine.innerHTML = '<span class="subtle">Out of book</span>';
    if (mode === 'explore') renderExplorerList();
  }

  // ------------------------------------------------------------------ panel bodies
  let explorerList = null;

  function renderBody() {
    if (mode === 'learn') renderLearnBody();
    else if (mode === 'explore') renderExploreBody();
    syncNav();
  }

  function moveButtons(target, sans, activePly, onPick) {
    const nodes = [];
    sans.forEach((s, i) => {
      if (i % 2 === 0) nodes.push(h('span', { class: 'op-move-num' }, `${i / 2 + 1}.`));
      nodes.push(h('button', { type: 'button', class: ['op-move', activePly === i + 1 && 'active'], onClick: () => onPick(i + 1) }, sanOf(s)));
    });
    target.replaceChildren(...nodes);
    return target;
  }

  function renderLearnBody() {
    const moves = moveButtons(h('div', { class: 'op-moves' }), main.sans, ply, (p) => { stopPlay(); setPly(p); });
    const ideas = Array.isArray(o.ideas) ? o.ideas.filter(Boolean) : [];
    const traps = Array.isArray(o.traps) ? o.traps.filter(Boolean) : [];
    const rec = trainRecord(o.id);
    body.replaceChildren(
      h('div', { class: 'stack-sm' },
        h('div', { class: 'text-xs subtle semibold', style: 'text-transform:uppercase;letter-spacing:.06em' }, 'Main line'),
        moves,
        h('div', { class: 'text-xs subtle' }, 'Tip: press ▶ to watch the line, or make your own move on the board to explore.')),
      o.description ? h('div', { class: 'lrn-text md mt-4', html: mdLite(o.description) }) : null,
      ideas.length ? h('div', null,
        h('div', { class: 'op-block-title', html: icon('hint') + '<span>Key ideas</span>' }),
        h('ul', { class: 'op-ideas' }, ideas.map((t) => h('li', { html: `<span>${mdLite(t).replace(/^<p>|<\/p>$/g, '')}</span>` })))) : null,
      traps.length ? h('div', null,
        h('div', { class: 'op-block-title', html: icon('alert') + '<span>Traps to know</span>' }),
        traps.map((t) => h('div', { class: 'op-trap', html: icon('alert') + `<div>${mdLite(t).replace(/^<p>|<\/p>$/g, '')}</div>` }))) : null,
      h('div', { class: 'callout mt-6', html: icon('target') + `<div><strong>Ready to test yourself?</strong> Train mode asks you to play the ${userSide} moves from memory.${rec ? ` Current streak: <strong>${rec.streak || 0}</strong> · ${escapeHtml(dueLabel(rec))}.` : ''}</div>` }),
      h('button', { class: 'btn btn-primary btn-block mt-3', type: 'button', html: icon('target') + '<span>Train this opening</span>', onClick: () => setMode('train') }));
  }

  function renderExploreBody() {
    const lineBox = h('div', { class: 'op-explorer-line' });
    if (line.sans.length) moveButtons(lineBox, line.sans, ply, (p) => setPly(p));
    else lineBox.appendChild(h('span', { class: 'subtle' }, 'Starting position — pick a move below or play one on the board.'));
    explorerList = h('div', { class: 'stack-sm', role: 'list' });
    body.replaceChildren(
      h('div', { class: 'between row mb-2' },
        h('div', { class: 'text-xs subtle semibold', style: 'text-transform:uppercase;letter-spacing:.06em' }, 'Your line'),
        h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon('refresh') + '<span>Back to main line</span>', onClick: () => { line = { ...main }; setPly(main.ucis.length, { force: true }); } })),
      lineBox,
      h('div', { class: 'text-xs subtle semibold mb-2', style: 'text-transform:uppercase;letter-spacing:.06em' }, 'Book moves'),
      explorerList);
    renderExplorerList();
  }

  function renderExplorerList() {
    if (!explorerList || mode !== 'explore') return;
    if (lookup === undefined) { explorerList.replaceChildren(h('div', { class: 'loading-center', style: 'padding:var(--sp-6)' }, h('div', { class: 'spinner' }))); return; }
    const conts = lookup && Array.isArray(lookup.continuations) ? lookup.continuations.filter((c) => c && c.uci) : [];
    if (!conts.length) {
      explorerList.replaceChildren(h('div', { class: 'callout callout-warning', html: icon('info') + '<div><strong>Out of book.</strong> No known opening moves from here. Use the analysis board to let the engine guide you.</div>' }));
      return;
    }
    const total = conts.reduce((s, c) => s + Math.max(0, Number(c.weight) || 0), 0) || conts.length;
    const sorted = [...conts].sort((a, b) => (Number(b.weight) || 0) - (Number(a.weight) || 0));
    explorerList.replaceChildren(...sorted.map((c) => {
      const pct = ((Math.max(0, Number(c.weight) || 0) || (total === conts.length ? 1 : 0)) / total) * 100;
      const row = h('button', {
        type: 'button', class: 'op-explorer-row', role: 'listitem',
        'aria-label': `${c.san}${c.name ? ', ' + c.name : ''}, ${pct.toFixed(0)}%`,
        onClick: () => playBookMove(c.uci),
        onMouseenter: () => board.setArrows([{ from: c.uci.slice(0, 2), to: c.uci.slice(2, 4), color: 'blue' }]),
        onMouseleave: () => board.clearArrows(),
        onFocus: () => board.setArrows([{ from: c.uci.slice(0, 2), to: c.uci.slice(2, 4), color: 'blue' }]),
        onBlur: () => board.clearArrows(),
      },
      h('span', { class: 'op-explorer-san' }, sanOf(c.san || c.uci)),
      h('span', { class: 'op-explorer-main' },
        h('span', { class: 'op-explorer-name' }, c.name || ''),
        h('span', { class: 'op-bar' }, h('span', { style: `width:${Math.max(2, pct).toFixed(1)}%` }))),
      h('span', { class: 'op-explorer-pct' }, `${pct.toFixed(0)}%`));
      return row;
    }));
  }

  // ------------------------------------------------------------------ train
  function startTrain() {
    timers.clear();
    const userPlies = main.ucis.map((_, i) => i).filter((i) => sideToMove(main.fens[i]) === userSide);
    train = { ply: 0, fen: main.fens[0], results: [], misses: 0, userPlies, busy: false, done: false };
    board.setOrientation(userSide);
    board.setPosition(main.fens[0], { animate: false });
    board.clearArrows(); board.setHighlights([]); board.clearBadges();
    board.setInteractive(false, null);
    syncNav();
    renderTrainBody();
    refreshLookup();
    if (!userPlies.length) {
      setFeedback(trainFeedback, 'info', 'This line has no moves for your side to train.');
      return;
    }
    trainAdvance();
  }

  let trainFeedback = null;
  let trainProgress = null;

  function renderTrainBody() {
    const rec = trainRecord(o.id);
    trainFeedback = h('div', { class: 'lrn-feedback', role: 'status', 'aria-live': 'polite' });
    trainProgress = h('div', { class: 'op-train-progress', 'aria-hidden': 'true' });
    body.replaceChildren(
      h('div', { class: 'op-score' },
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'Streak'), h('div', { class: 'stat-value' }, String(rec ? rec.streak || 0 : 0))),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'Best'), h('div', { class: 'stat-value' }, rec ? `${rec.best || 0}%` : '–')),
        h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, 'Next review'), h('div', { class: 'text-sm semibold' }, rec ? dueLabel(rec) : 'Not trained yet'))),
      h('div', { class: 'lrn-task' },
        h('div', { class: 'lrn-task-label', html: icon('target') + `<span>Play the ${userSide} moves</span><span class="lrn-turn ${userSide}"></span>` }),
        h('div', { class: 'lrn-task-prompt' }, `Play the ${o.name} from memory. Your opponent replies with the book moves.`),
        trainProgress),
      trainFeedback,
      h('div', { class: 'row mt-4', style: 'gap:var(--sp-2)' },
        h('button', { class: 'btn btn-ghost', type: 'button', html: icon('hint') + '<span>Show move</span>', onClick: () => trainHint() }),
        h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + '<span>Restart</span>', onClick: () => startTrain() })));
    renderTrainProgress();
  }

  function renderTrainProgress() {
    if (!train || !trainProgress) return;
    trainProgress.replaceChildren(...train.userPlies.map((p, i) => {
      const r = train.results[i];
      const cur = !r && i === train.results.length;
      return h('i', { class: r === 'ok' ? 'ok' : r === 'miss' ? 'miss' : cur ? 'cur' : '' });
    }));
  }

  function trainAdvance() {
    if (!train || train.done) return;
    if (train.ply >= main.ucis.length) { trainFinish(); return; }
    if (sideToMove(train.fen) === userSide) {
      train.busy = false;
      board.setInteractive(true, userSide);
      if (!train.results.length && train.ply === 0) setFeedback(trainFeedback, 'info', 'Your move!');
      return;
    }
    train.busy = true;
    board.setInteractive(false, null);
    timers.later(() => {
      if (!train || train.done) return;
      const c = chessAt(train.fen);
      const m = c && applyUci(c, main.ucis[train.ply]);
      if (!m) { trainFinish(); return; }
      train.fen = c.fen();
      train.ply++;
      board.setPosition(train.fen, { animate: true, lastMove: [m.from, m.to] });
      board.setHighlights([]); board.clearBadges();
      syncNav();
      refreshLookup();
      if (train.ply < main.ucis.length) setFeedback(trainFeedback, 'info', `Opponent played <strong>${escapeHtml(sanOf(m.san))}</strong>. Your move!`);
      trainAdvance();
    }, train.ply === 0 ? 600 : 450);
  }

  function onTrainMove(mv) {
    if (!train || train.busy || train.done) return false;
    const expected = main.ucis[train.ply];
    if (isSameMove(train.fen, expected, mv.fen)) {
      train.results.push(train.misses > 0 ? 'miss' : 'ok');
      train.misses = 0;
      train.fen = mv.fen;
      train.ply++;
      board.clearArrows();
      board.setHighlights([{ square: mv.to, kind: 'good' }]);
      board.clearBadges(); board.setBadge(mv.to, 'book');
      sfx('correct');
      setFeedback(trainFeedback, 'good', `<strong>${escapeHtml(sanOf(mv.san))}</strong> — that's the book move!`);
      renderTrainProgress();
      syncNav();
      refreshLookup();
      trainAdvance();
      return true;
    }
    train.misses++;
    sfx('wrong');
    flashClass(boardSlot, 'shake', timers, 420);
    flashClass(boardWrap, 'flash-bad', timers, 650);
    board.setHighlights([{ square: mv.to, kind: 'bad' }]);
    const bookSan = main.sans[train.ply];
    if (train.misses >= 2) {
      board.setArrows([{ from: expected.slice(0, 2), to: expected.slice(2, 4), color: 'green' }]);
      setFeedback(trainFeedback, 'bad', `That's not the main line. The book move is <strong>${escapeHtml(sanOf(bookSan))}</strong> — play it to continue.`);
    } else {
      setFeedback(trainFeedback, 'bad', "That's not the main line. <strong>Try again</strong> — or press <strong>Show move</strong>.");
    }
    return false;
  }

  function trainHint() {
    if (!train || train.done || train.busy) return;
    const expected = main.ucis[train.ply];
    if (!expected) return;
    train.misses = Math.max(train.misses, 1);
    board.setArrows([{ from: expected.slice(0, 2), to: expected.slice(2, 4), color: 'green' }]);
    setFeedback(trainFeedback, 'warn', `The book move is <strong>${escapeHtml(sanOf(main.sans[train.ply]))}</strong>. (This move won't count as perfect.)`);
  }

  function trainFinish() {
    if (!train || train.done) return;
    train.done = true;
    board.setInteractive(false, null);
    const total = train.results.length;
    const ok = train.results.filter((r) => r === 'ok').length;
    const pct = total ? Math.round((ok / total) * 100) : 100;
    const perfect = ok === total;
    const rec = saveTrainResult(o.id, perfect, pct);
    syncNav();
    if (perfect) { confetti(boardSlot, timers); sfx('gameEnd'); }
    body.replaceChildren(h('div', { class: 'lrn-complete' },
      h('div', { class: ['lrn-complete-badge', perfect ? '' : 'gold'], html: icon(perfect ? 'trophy' : 'target') }),
      h('h2', null, perfect ? 'Perfect run!' : `${pct}% correct`),
      h('p', null, perfect
        ? (isLearned(rec) ? `Opening learned! Streak ${rec.streak}. ${dueLabel(rec)}.` : `Streak ${rec.streak} — ${LEARNED_STREAK - rec.streak} more perfect run${LEARNED_STREAK - rec.streak === 1 ? '' : 's'} to mark it learned. ${dueLabel(rec)}.`)
        : `You found ${ok} of ${total} moves on the first try. Practise again to build your streak.`),
      h('div', { class: 'lrn-nav' },
        h('button', { class: 'btn btn-primary btn-lg', type: 'button', html: icon('refresh') + '<span>Train again</span>', onClick: () => startTrain() }),
        h('button', { class: 'btn btn-secondary', type: 'button', html: icon('search') + '<span>Explore from here</span>', onClick: () => { line = { ...main }; ply = main.ucis.length; setMode('explore'); } }))));
  }

  // ------------------------------------------------------------------ keyboard
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    if (e.key === ' ' && t && t.tagName === 'BUTTON') return; // let buttons handle Space
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'f' || e.key === 'F') { board.flip(); return; }
    if (mode === 'train') return;
    if (e.key === 'ArrowLeft') { e.preventDefault(); stopPlay(); setPly(ply - 1); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); stopPlay(); setPly(ply + 1); }
    else if (e.key === 'Home') { e.preventDefault(); stopPlay(); setPly(0); }
    else if (e.key === 'End') { e.preventDefault(); stopPlay(); setPly(line.ucis.length); }
    else if (e.key === ' ' && mode === 'learn') { e.preventDefault(); togglePlay(); }
  });

  // ------------------------------------------------------------------ init
  renderTabs();
  const initialMode = query.mode;
  if (initialMode === 'train' || initialMode === 'explore') setMode(initialMode);
  else { syncNav(); renderBody(); refreshLookup(); }
}
