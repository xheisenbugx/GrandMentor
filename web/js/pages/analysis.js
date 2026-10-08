// Analysis board (#/analysis?fen=..|?game=:id|?pgn=..[&ply=n])
// Free board with a variation-aware move tree, live multi-PV engine, eval bar, opening banner,
// book moves, load/setup/copy/save tools and the Mentor chat.
// Every listener, timer, socket and component is released by the returned cleanup.

import { Chess, validateFen } from '../../vendor/chess.js';
import { api, qs, isAbort, EngineClient } from '../api.js';
import {
  h, icon, toast, modal, formatScore, copyText, disposables, debounce, formatSan,
} from '../ui.js';
import { getSettings, onSettingsChange, pieceUrl } from '../settings.js';
import { Board } from '../components/board.js';
import { EvalBar } from '../components/evalbar.js';
import { MentorPanel } from '../components/mentor.js';
import { ensureAnalysisCss } from '../components/evalgraph.js';

export const title = 'Analysis';

export const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
const MAX_NODES = 4000;
const MAX_PGN_CHARS = 1_000_000;
const ENGINE_MOVETIME = 20000;
const OPENING_CACHE_MAX = 300;
const STORE_KEY = 'gm.analysis.v1';
const ENGINE_KEY = 'gm.analysis.engine';

// ---------------------------------------------------------------------------
// Helpers shared with review.js
// ---------------------------------------------------------------------------
export function uciSquares(uci) {
  if (typeof uci !== 'string' || uci.length < 4) return null;
  return [uci.slice(0, 2), uci.slice(2, 4)];
}

/** Absolute half-move index of a FEN (0 = white to move on move 1). */
export function fenPly(fen) {
  const parts = String(fen || '').split(/\s+/);
  const full = Math.max(1, parseInt(parts[5], 10) || 1);
  return (full - 1) * 2 + (parts[1] === 'b' ? 1 : 0);
}

/** Play a UCI move on a chess.js instance. Returns the verbose move or null. */
export function playUci(chess, uci) {
  const sq = uciSquares(uci);
  if (!sq) return null;
  try {
    return chess.move({ from: sq[0], to: sq[1], promotion: uci.length > 4 ? uci[4] : undefined });
  } catch {
    return null;
  }
}

function moveUci(m) {
  return m.from + m.to + (m.promotion || '');
}

function safeGet(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}
function safeSet(key, val) {
  try { localStorage.setItem(key, val); } catch { /* storage blocked */ }
}

/** Format a SAN line with move numbers, starting at absolute ply `ply0` (before the first move). */
export function numberedLine(sans, ply0, notation = 'san', max = 12) {
  const out = [];
  for (let i = 0; i < sans.length && i < max; i++) {
    const p = ply0 + i;
    const no = Math.floor(p / 2) + 1;
    if (p % 2 === 0) out.push(`${no}.`);
    else if (i === 0) out.push(`${no}…`);
    out.push(formatSan(sans[i], notation));
  }
  if (sans.length > max) out.push('…');
  return out.join(' ');
}

// ---------------------------------------------------------------------------
// Move tree
// ---------------------------------------------------------------------------
class MoveTree {
  constructor(fen) { this.reset(fen); }

  reset(fen = START_FEN) {
    this._next = 1;
    this.root = { id: 0, parent: null, children: [], fen, san: null, uci: null, ply: fenPly(fen) };
    this.nodes = new Map([[0, this.root]]);
  }

  get size() { return this.nodes.size; }

  /** Add (or reuse) a child of parent. Returns { node, created } or null when full. */
  add(parent, { san, uci, fen }) {
    const existing = parent.children.find((c) => c.uci === uci);
    if (existing) return { node: existing, created: false };
    if (this.nodes.size >= MAX_NODES) return null;
    const node = { id: this._next++, parent, children: [], fen, san, uci, ply: parent.ply + 1 };
    parent.children.push(node);
    this.nodes.set(node.id, node);
    return { node, created: true };
  }

  path(node) {
    const out = [];
    for (let n = node; n && n.parent; n = n.parent) out.push(n);
    return out.reverse();
  }

  mainline() {
    const out = [];
    for (let n = this.root.children[0]; n; n = n.children[0]) out.push(n);
    return out;
  }

  isMainline(node) {
    for (let n = node; n && n.parent; n = n.parent) if (n.parent.children[0] !== n) return false;
    return true;
  }

  /** Move the variation containing node one level up (towards the mainline). */
  promote(node) {
    for (let n = node; n && n.parent; n = n.parent) {
      const sibs = n.parent.children;
      const i = sibs.indexOf(n);
      if (i > 0) { sibs.splice(i, 1); sibs.unshift(n); return true; }
    }
    return false;
  }

  /** Delete node and its subtree. Returns the parent. */
  remove(node) {
    if (!node.parent) return node;
    const parent = node.parent;
    parent.children = parent.children.filter((c) => c !== node);
    const stack = [node];
    while (stack.length) {
      const n = stack.pop();
      this.nodes.delete(n.id);
      stack.push(...n.children);
      n.children = [];
      n.parent = null;
    }
    return parent;
  }

  /** Compact serialisation: [uci, [children...]] */
  serialize() {
    const enc = (n) => [n.uci, n.children.map(enc)];
    return { fen: this.root.fen, tree: this.root.children.map(enc) };
  }

  static deserialize(data) {
    const t = new MoveTree(data.fen);
    const walk = (parent, list) => {
      if (!Array.isArray(list)) return;
      for (const item of list) {
        if (!Array.isArray(item) || typeof item[0] !== 'string') continue;
        const c = new Chess(parent.fen);
        const m = playUci(c, item[0]);
        if (!m) continue;
        const r = t.add(parent, { san: m.san, uci: moveUci(m), fen: c.fen() });
        if (!r) return;
        walk(r.node, item[1]);
      }
    };
    walk(t.root, data.tree);
    return t;
  }
}

/** Validate a FEN for analysis. Returns error string or null. */
export function fenError(fen) {
  const f = String(fen || '').trim();
  if (!f) return 'Please enter a FEN.';
  const v = validateFen(f);
  if (!v.ok) { const m = v.error.replace(/^Invalid FEN: /, ''); return m.charAt(0).toUpperCase() + m.slice(1) + '.'; }
  const rows = f.split(/\s+/)[0].split('/');
  if (/[pP]/.test(rows[0]) || /[pP]/.test(rows[7])) return 'Pawns cannot stand on the first or last rank.';
  const count = (re) => (f.split(/\s+/)[0].match(re) || []).length;
  if (count(/K/g) !== 1 || count(/k/g) !== 1) return 'Each side needs exactly one king.';
  if (count(/[PNBRQK]/g) > 16 || count(/[pnbrqk]/g) > 16) return 'Too many pieces for one side (max 16).';
  if (count(/P/g) > 8 || count(/p/g) > 8) return 'Too many pawns (max 8 per side).';
  try {
    const parts = f.split(/\s+/);
    parts[1] = parts[1] === 'w' ? 'b' : 'w';
    parts[3] = '-';
    if (new Chess(parts.join(' ')).inCheck()) return 'The side that is not to move is in check — that cannot happen.';
    new Chess(f);
  } catch (e) {
    return String(e?.message || e).replace(/^Invalid FEN: /, '');
  }
  return null;
}

function normalizeFen(fen) {
  const parts = String(fen || '').trim().split(/\s+/);
  if (parts.length === 4) parts.push('0', '1');
  if (parts.length === 5) parts.push('1');
  return parts.join(' ');
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export async function mount(root, { query = {} } = {}) {
  ensureAnalysisCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const settings = getSettings();

  const state = {
    tree: new MoveTree(START_FEN),
    cur: null,
    engineOn: safeGet(ENGINE_KEY) !== 'off',
    showBest: true,
    lines: [],
    linesFen: null,
    depth: 0,
    nps: 0,
    hoverLine: -1,
    headers: {},
    gameId: null,
    tab: 'analysis',
  };
  state.cur = state.tree.root;

  // ---- DOM skeleton -----------------------------------------------------
  const openingName = h('div', { class: 'an-opening-name' }, 'Starting position');
  const openingEco = h('span', { class: 'an-opening-eco' });
  const openingBar = h('div', { class: 'player-bar an-opening' },
    h('div', { class: 'an-opening-icon', html: icon('book') }),
    h('div', { class: 'an-opening-text' }, openingEco, openingName));

  const turnDot = h('span', { class: 'an-turn-dot' });
  const turnText = h('span', { class: 'an-turn-text' }, 'White to move');
  const fenInput = h('input', { class: 'input input-sm mono an-fen-input', spellcheck: 'false', 'aria-label': 'FEN of the current position', title: 'FEN — paste one and press Enter to load it' });
  const statusBar = h('div', { class: 'player-bar an-status' }, h('div', { class: 'an-turn' }, turnDot, turnText), fenInput);

  const evalSlot = h('div', { class: 'evalbar-slot' });
  const boardSlot = h('div', { class: 'board-slot' });
  const main = h('div', { class: 'game-main' }, openingBar, h('div', { class: 'board-row' }, evalSlot, boardSlot), statusBar);

  // Engine card
  const engineSwitch = h('input', { type: 'checkbox', checked: state.engineOn, 'aria-label': 'Engine on/off' });
  const engineDepth = h('span', { class: 'badge an-depth' }, '–');
  const engineScore = h('span', { class: 'an-score' }, '0.0');
  const engineNps = h('span', { class: 'subtle text-xs an-nps' });
  const bestToggle = h('button', { class: 'btn btn-ghost btn-icon btn-sm', type: 'button', 'data-tooltip': 'Best-move arrow', 'aria-label': 'Toggle best-move arrow', 'aria-pressed': 'true', html: icon('eye') });
  const linesBox = h('div', { class: 'an-lines' });
  const engineCard = h('div', { class: 'an-engine' },
    h('div', { class: 'an-engine-head' },
      h('label', { class: 'switch' }, engineSwitch, h('span', { class: 'switch-track' })),
      engineScore,
      h('div', { class: 'an-engine-meta' }, h('span', { class: 'semibold text-sm' }, 'Engine'), engineNps),
      h('span', { class: 'spacer' }),
      engineDepth, bestToggle),
    linesBox);

  const bookBox = h('div', { class: 'an-book', hidden: true });
  const treeBox = h('div', { class: 'move-tree', role: 'list', 'aria-label': 'Moves and variations' });
  const varActions = h('div', { class: 'an-var-actions', hidden: true });

  const analysisTab = h('div', { class: 'an-tab-body' }, engineCard, bookBox, h('div', { class: 'an-tree-wrap' }, treeBox), varActions);
  const mentorHost = h('div', { class: 'an-mentor-host' });
  const ideasBtn = h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon('hint') + '<span>Ideas for this position</span>' });
  const mentorTab = h('div', { class: 'an-tab-body an-mentor-tab', hidden: true }, h('div', { class: 'row-sm an-mentor-actions' }, ideasBtn), mentorHost);

  const tabA = h('button', { class: 'tab active', role: 'tab', 'aria-selected': 'true', type: 'button', html: icon('analysis') + '<span>Analysis</span>' });
  const tabM = h('button', { class: 'tab', role: 'tab', 'aria-selected': 'false', type: 'button', html: icon('mentor') + '<span>Mentor</span>' });
  const tabs = h('div', { class: 'tabs an-tabs', role: 'tablist' }, tabA, tabM);

  const actBtn = (ic, label, tip) => h('button', { class: 'btn btn-ghost btn-sm an-act', type: 'button', 'data-tooltip': tip, 'aria-label': tip, html: icon(ic) + `<span>${label}</span>` });
  const bNew = actBtn('refresh', 'New', 'Start a new analysis');
  const bLoad = actBtn('upload', 'Load', 'Load FEN, PGN or a saved game');
  const bSetup = actBtn('edit', 'Setup', 'Set up a position');
  const bCopy = actBtn('copy', 'Copy', 'Copy FEN or PGN');
  const bSave = actBtn('save', 'Save', 'Save to your library');
  const reviewLink = h('a', { class: 'btn btn-ghost btn-sm an-act', hidden: true, html: icon('chart') + '<span>Review</span>' });
  const actions = h('div', { class: 'an-actions' }, bNew, bLoad, bSetup, bCopy, bSave, reviewLink);

  const nav = (ic, label) => h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(ic) });
  const bFirst = nav('first', 'First move (Home)');
  const bPrev = nav('chevron-left', 'Previous (←)');
  const bNext = nav('chevron-right', 'Next (→)');
  const bLast = nav('last', 'Last move (End)');
  const bFlip = nav('flip', 'Flip board (F)');
  const toolbar = h('div', { class: 'toolbar' }, bFirst, bPrev, bNext, bLast, bFlip);

  const panel = h('div', { class: 'panel grow an-panel' }, actions, tabs, analysisTab, mentorTab);
  const aside = h('aside', { class: 'game-panel' }, panel, toolbar);
  const layout = h('div', { class: 'game-layout analysis-page' + (settings.showEvalBar === false ? ' no-eval' : ''), style: { '--panel-w': '400px' } }, main, aside);
  root.appendChild(layout);
  bag.add(() => layout.remove());

  // ---- Components -------------------------------------------------------
  const board = new Board(boardSlot, {
    fen: START_FEN,
    orientation: query.orientation === 'black' ? 'black' : 'white',
    interactive: true,
    movableColor: 'both',
    showCoords: settings.showCoords,
    showLegal: settings.showLegal,
    animationMs: settings.animationMs,
    sounds: settings.sounds,
    onMove: (m) => onBoardMove(m),
  });
  bag.add(() => board.destroy());

  const evalBar = new EvalBar(evalSlot, { orientation: board.orientation || 'white' });
  bag.add(() => evalBar.destroy());

  const engine = new EngineClient();
  bag.add(() => engine.close());

  const mentor = new MentorPanel(mentorHost, {
    getContext: () => ({
      fen: state.cur.fen,
      moves_san: state.tree.path(state.cur).map((n) => n.san),
      engine_lines: state.linesFen === state.cur.fen ? state.lines.map((l) => `${formatScore(l.score)}: ${(l.san || []).slice(0, 8).join(' ')}`) : [],
    }),
    greeting: "Hi! I'm your coach. Make moves on the board and ask me anything — **plans**, **threats**, or why a move is good or bad. Tap *Ideas for this position* for a quick summary.",
  });
  bag.add(() => mentor.destroy());

  bag.add(onSettingsChange((s, key) => {
    if (key === 'showEvalBar') layout.classList.toggle('no-eval', s.showEvalBar === false);
    if (key === 'moveNotation') { renderTree(); renderLines(); }
  }));

  // ---- Tree rendering ---------------------------------------------------
  const nodeEls = new Map();

  function renderTree() {
    nodeEls.clear();
    const notation = getSettings().moveNotation;
    const frag = document.createDocumentFragment();
    const first = state.tree.root.children[0];
    if (!first) {
      frag.appendChild(h('div', { class: 'move-tree-empty' },
        h('div', { class: 'move-tree-empty-icon', html: icon('knight') }),
        h('div', null, 'Make a move on the board to start analysing.'),
        h('div', { class: 'subtle text-xs' }, 'Playing a different move from an earlier position creates a variation.')));
    } else {
      renderLine(frag, first, 0, notation);
    }
    treeBox.replaceChildren(frag);
    markCurrent();
  }

  function renderLine(container, start, depth, notation) {
    let flow = h('div', { class: depth ? 'mt-flow' : 'mt-flow mt-main' });
    container.appendChild(flow);
    let needNum = true;
    for (let node = start; node; node = node.children[0]) {
      const before = node.ply - 1;
      const no = Math.floor(before / 2) + 1;
      if (before % 2 === 0) flow.appendChild(h('span', { class: 'mt-num' }, `${no}.`));
      else if (needNum) flow.appendChild(h('span', { class: 'mt-num' }, `${no}…`));
      const btn = h('button', { class: 'mt-move', type: 'button', role: 'listitem', dataset: { id: node.id } }, formatSan(node.san, notation));
      nodeEls.set(node.id, btn);
      flow.appendChild(btn);
      needNum = false;
      const p = node.parent;
      if (p.children[0] === node && p.children.length > 1) {
        for (let i = 1; i < p.children.length; i++) {
          const vb = h('div', { class: 'mt-var', style: { '--depth': Math.min(depth + 1, 6) } });
          container.appendChild(vb);
          renderLine(vb, p.children[i], depth + 1, notation);
        }
        flow = h('div', { class: depth ? 'mt-flow' : 'mt-flow mt-main' });
        container.appendChild(flow);
        needNum = true;
      }
    }
    if (!flow.childElementCount) flow.remove();
  }

  let prevCurEl = null;
  function markCurrent() {
    prevCurEl?.classList.remove('current');
    prevCurEl?.removeAttribute('aria-current');
    const el = nodeEls.get(state.cur.id) || null;
    prevCurEl = el;
    if (el) {
      el.classList.add('current');
      el.setAttribute('aria-current', 'true');
      const box = treeBox.parentElement;
      const r = el.getBoundingClientRect();
      const b = box.getBoundingClientRect();
      if (r.top < b.top || r.bottom > b.bottom) box.scrollTop += r.top - b.top - b.height / 2;
    } else if (state.cur === state.tree.root) {
      treeBox.parentElement.scrollTop = 0;
    }
  }

  bag.on(treeBox, 'click', (e) => {
    const b = e.target.closest('.mt-move');
    if (!b) return;
    const node = state.tree.nodes.get(Number(b.dataset.id));
    if (node) goTo(node);
  });

  function renderVarActions() {
    const inVar = state.cur.parent && !state.tree.isMainline(state.cur);
    varActions.hidden = !state.cur.parent;
    varActions.replaceChildren(...[
      inVar ? h('button', { class: 'btn btn-ghost btn-sm', type: 'button', onClick: promoteCur, html: icon('chevron-up') + '<span>Promote variation</span>' }) : null,
      h('button', { class: 'btn btn-ghost btn-sm', type: 'button', onClick: deleteCur, html: icon('trash') + '<span>Delete from here</span>' }),
    ].filter(Boolean));
  }

  function promoteCur() {
    if (state.tree.promote(state.cur)) { renderTree(); renderVarActions(); persist(); }
  }
  function deleteCur() {
    const parent = state.tree.remove(state.cur);
    goTo(parent, { animate: false, rebuild: true });
    persist();
  }

  // ---- Navigation -------------------------------------------------------
  function goTo(node, { animate = true, rebuild = false, skipBoard = false } = {}) {
    if (!node) return;
    state.cur = node;
    state.hoverLine = -1;
    if (!skipBoard) board.setPosition(node.fen, { animate, lastMove: node.uci ? uciSquares(node.uci) : null });
    if (rebuild) renderTree(); else markCurrent();
    renderVarActions();
    renderStatus();
    updateOpening(node);
    scheduleEngine();
    renderLines();
    refreshArrows();
  }

  const step = {
    prev: () => state.cur.parent && goTo(state.cur.parent),
    next: () => state.cur.children[0] && goTo(state.cur.children[0]),
    first: () => goTo(state.tree.root),
    last: () => { let n = state.cur; while (n.children[0]) n = n.children[0]; goTo(n); },
  };

  function onBoardMove(m) {
    const uci = m.uci || (m.from + m.to + (m.promotion || ''));
    if (addMove(uci, { fromBoard: true }) === false) return false;
    return true;
  }

  /** Play uci from the current node (reuses existing children). Returns false if illegal. */
  function addMove(uci, { fromBoard = false } = {}) {
    const c = new Chess(state.cur.fen);
    const m = playUci(c, uci);
    if (!m) return false;
    const r = state.tree.add(state.cur, { san: m.san, uci: moveUci(m), fen: c.fen() });
    if (!r) {
      toast('This analysis is very large — delete some variations to add more moves.', 'warning');
      return false;
    }
    // A board move is already shown (and animated) by the Board itself.
    goTo(r.node, { animate: true, rebuild: r.created, skipBoard: fromBoard });
    if (r.created) persist();
    return true;
  }

  // ---- Status bar -------------------------------------------------------
  function gameOverText(fen) {
    let c;
    try { c = new Chess(fen); } catch { return null; }
    if (c.isCheckmate()) return c.turn() === 'w' ? 'Checkmate — Black wins' : 'Checkmate — White wins';
    if (c.isStalemate()) return 'Stalemate — draw';
    if (c.isInsufficientMaterial()) return 'Draw — insufficient material';
    if (c.isDrawByFiftyMoves()) return 'Draw — 50-move rule';
    return null;
  }

  function renderStatus() {
    const fen = state.cur.fen;
    const white = fen.split(' ')[1] !== 'b';
    const over = gameOverText(fen);
    turnDot.classList.toggle('black', !white);
    turnText.textContent = over || `${white ? 'White' : 'Black'} to move`;
    if (document.activeElement !== fenInput) fenInput.value = fen;
  }

  bag.on(fenInput, 'keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { e.preventDefault(); loadFen(fenInput.value); fenInput.blur(); }
    if (e.key === 'Escape') { fenInput.value = state.cur.fen; fenInput.blur(); }
  });
  bag.on(fenInput, 'blur', () => { fenInput.value = state.cur.fen; });
  bag.on(fenInput, 'focus', () => fenInput.select());

  // ---- Opening banner & book moves -------------------------------------
  const openingCache = new Map(); // key: first 4 FEN fields -> OpeningMatch|null
  let openingCtrl = null;
  const isStart = (fen) => fen.split(' ').slice(0, 4).join(' ') === START_FEN.split(' ').slice(0, 4).join(' ');

  async function fetchOpening(fen) {
    const key = fen.split(' ').slice(0, 4).join(' ');
    if (openingCache.has(key)) {
      const v = openingCache.get(key);
      openingCache.delete(key); openingCache.set(key, v); // LRU touch
      return v;
    }
    openingCtrl?.abort();
    openingCtrl = new AbortController();
    const signal = openingCtrl.signal;
    const onPageAbort = () => openingCtrl?.abort();
    ctrl.signal.addEventListener('abort', onPageAbort, { once: true });
    try {
      const res = await api.get('/api/openings/lookup' + qs({ fen }), { signal, timeout: 8000 });
      openingCache.set(key, res || null);
      while (openingCache.size > OPENING_CACHE_MAX) openingCache.delete(openingCache.keys().next().value);
      return res || null;
    } finally {
      ctrl.signal.removeEventListener('abort', onPageAbort);
    }
  }

  let openingSeq = 0;
  async function updateOpening(node) {
    const seq = ++openingSeq;
    // Deepest named opening along the path (walk back until a lookup hits).
    let named = null;
    let match = null;
    try {
      match = await fetchOpening(node.fen);
      if (seq !== openingSeq) return;
      if (match?.opening) named = match.opening;
      else {
        for (let n = node.parent, hops = 0; n && hops < 16 && !named; n = n.parent, hops++) {
          const anc = await fetchOpening(n.fen);
          if (seq !== openingSeq) return;
          if (anc?.opening) named = anc.opening;
        }
      }
    } catch (e) {
      if (isAbort(e) || seq !== openingSeq) return;
    }
    if (named) {
      openingEco.textContent = named.eco || '';
      openingEco.hidden = !named.eco;
      openingName.textContent = named.name;
      openingBar.classList.add('known');
    } else {
      openingEco.hidden = true;
      openingName.textContent = isStart(state.tree.root.fen) && !state.tree.root.children.length ? 'Starting position'
        : node === state.tree.root ? (isStart(node.fen) ? 'Starting position' : 'Custom position')
          : (isStart(state.tree.root.fen) ? 'Out of the opening book' : 'Custom position');
      openingBar.classList.remove('known');
    }
    renderBook(match?.continuations || []);
  }

  function renderBook(conts) {
    const list = conts.slice().sort((a, b) => (b.weight || 0) - (a.weight || 0)).slice(0, 8);
    bookBox.hidden = !list.length;
    if (!list.length) { bookBox.replaceChildren(); return; }
    const notation = getSettings().moveNotation;
    bookBox.replaceChildren(
      h('div', { class: 'an-book-title', html: icon('book') + '<span>Popular book moves</span>' }),
      h('div', { class: 'chip-row' }, list.map((b) => h('button', {
        class: 'chip an-book-move', type: 'button', title: b.name || '', dataset: { uci: b.uci },
      }, h('strong', null, formatSan(b.san, notation)), b.name ? h('span', { class: 'an-book-name' }, b.name.replace(/^.*?:\s*/, '')) : null))),
    );
  }
  bag.on(bookBox, 'click', (e) => {
    const b = e.target.closest('[data-uci]');
    if (b) addMove(b.dataset.uci);
  });

  // ---- Engine -----------------------------------------------------------
  const scheduleEngine = debounce(runEngine, 140);
  bag.add(() => scheduleEngine.cancel());
  // Throttle DOM updates from engine info (setTimeout, so it also works in background tabs).
  let pendingRender = 0;
  bag.add(() => clearTimeout(pendingRender));

  function runEngine() {
    if (bag.disposed) return;
    const fen = state.cur.fen;
    engineCard.classList.toggle('off', !state.engineOn);
    if (!state.engineOn) {
      engine.stop();
      state.lines = []; state.linesFen = null;
      renderLines();
      refreshArrows();
      return;
    }
    const over = gameOverText(fen);
    if (over) {
      engine.stop();
      state.lines = []; state.linesFen = fen; state.depth = 0;
      const mated = over.startsWith('Checkmate');
      const score = mated ? { mate: 0 } : { cp: 0 };
      evalBar.set(score, { fen });
      engineScore.textContent = mated ? '#' : '½';
      renderLines(over);
      refreshArrows();
      return;
    }
    engineCard.classList.add('thinking');
    engine.analyze(fen, { multipv: 3, movetime_ms: ENGINE_MOVETIME }, (info, done) => {
      if (bag.disposed || state.cur.fen !== fen) return;
      if (Array.isArray(info.lines) && info.lines.length) {
        state.lines = info.lines.slice(0, 3);
        state.linesFen = fen;
      }
      state.depth = info.depth || state.depth;
      state.nps = info.nps || 0;
      if (done) engineCard.classList.remove('thinking');
      if (!pendingRender) {
        pendingRender = setTimeout(() => {
          pendingRender = 0;
          if (bag.disposed) return;
          renderLines();
          refreshArrows();
        }, 60);
      }
    }, (err) => {
      if (bag.disposed) return;
      engineCard.classList.remove('thinking');
      renderLines(`Engine unavailable: ${err}`);
    });
  }

  function renderLines(message) {
    const notation = getSettings().moveNotation;
    if (!state.engineOn) {
      engineDepth.textContent = 'Off';
      engineNps.textContent = '';
      engineScore.textContent = '–';
      linesBox.replaceChildren(h('div', { class: 'an-lines-msg' }, 'Engine is off. Turn it on to see the best moves and evaluation.'));
      return;
    }
    if (message) {
      engineDepth.textContent = '–';
      engineNps.textContent = '';
      linesBox.replaceChildren(h('div', { class: 'an-lines-msg' }, message));
      return;
    }
    const fresh = state.linesFen === state.cur.fen;
    if (!fresh || !state.lines.length) {
      engineDepth.textContent = '…';
      engineNps.textContent = 'Thinking…';
      linesBox.replaceChildren(...[0, 1, 2].map(() => h('div', { class: 'engine-line skeleton-line' }, h('span', { class: 'skeleton skeleton-text' }))));
      return;
    }
    const top = state.lines[0];
    evalBar.set(top.score);
    engineScore.textContent = formatScore(top.score);
    engineScore.classList.toggle('neg', isNeg(top.score));
    engineDepth.textContent = `Depth ${state.depth}`;
    engineNps.textContent = state.nps ? `${formatNps(state.nps)} positions/s` : '';
    const ply0 = state.cur.ply;
    linesBox.replaceChildren(...state.lines.map((l, i) => h('div', {
      class: 'engine-line' + (i === state.hoverLine ? ' hover' : ''), role: 'button', tabindex: 0,
      dataset: { i }, title: 'Click to play this move',
    },
    h('span', { class: 'engine-score' + (isNeg(l.score) ? ' neg' : '') }, formatScore(l.score)),
    h('span', { class: 'engine-moves' }, numberedLine(l.san || [], ply0, notation, 14)))));
  }

  const isNeg = (s) => (typeof s?.mate === 'number' ? s.mate < 0 : (s?.cp || 0) < 0);
  const formatNps = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n));

  bag.on(linesBox, 'click', (e) => {
    const row = e.target.closest('.engine-line[data-i]');
    const l = row && state.lines[Number(row.dataset.i)];
    if (l?.moves?.[0] && state.linesFen === state.cur.fen) addMove(l.moves[0]);
  });
  bag.on(linesBox, 'keydown', (e) => {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    const row = e.target.closest('.engine-line[data-i]');
    const l = row && state.lines[Number(row.dataset.i)];
    if (l?.moves?.[0]) { e.preventDefault(); addMove(l.moves[0]); }
  });
  bag.on(linesBox, 'pointerover', (e) => {
    const row = e.target.closest('.engine-line[data-i]');
    const i = row ? Number(row.dataset.i) : -1;
    if (i !== state.hoverLine) { state.hoverLine = i; refreshArrows(); }
  });
  bag.on(linesBox, 'pointerleave', () => { state.hoverLine = -1; refreshArrows(); });

  let lastArrowKey = '';
  function refreshArrows() {
    const arrows = [];
    const fresh = state.linesFen === state.cur.fen && state.engineOn;
    if (fresh && state.hoverLine >= 0 && state.lines[state.hoverLine]) {
      const mv = state.lines[state.hoverLine].moves || [];
      mv.slice(0, 3).forEach((u, i) => {
        const sq = uciSquares(u);
        if (sq) arrows.push({ from: sq[0], to: sq[1], color: i % 2 === 0 ? 'blue' : 'yellow' });
      });
    } else if (fresh && state.showBest && state.lines[0]?.moves?.[0]) {
      const sq = uciSquares(state.lines[0].moves[0]);
      if (sq) arrows.push({ from: sq[0], to: sq[1], color: 'green' });
    }
    const key = JSON.stringify(arrows);
    if (key === lastArrowKey) return;
    lastArrowKey = key;
    if (arrows.length) board.setArrows(arrows); else board.clearArrows();
  }

  bag.on(engineSwitch, 'change', () => {
    state.engineOn = engineSwitch.checked;
    safeSet(ENGINE_KEY, state.engineOn ? 'on' : 'off');
    runEngine();
  });
  bag.on(bestToggle, 'click', () => {
    state.showBest = !state.showBest;
    bestToggle.innerHTML = icon(state.showBest ? 'eye' : 'eye-off');
    bestToggle.setAttribute('aria-pressed', String(state.showBest));
    refreshArrows();
  });

  // ---- Tabs ---------------------------------------------------------------
  function setTab(t) {
    state.tab = t;
    tabA.classList.toggle('active', t === 'analysis');
    tabM.classList.toggle('active', t === 'mentor');
    tabA.setAttribute('aria-selected', String(t === 'analysis'));
    tabM.setAttribute('aria-selected', String(t === 'mentor'));
    analysisTab.hidden = t !== 'analysis';
    mentorTab.hidden = t !== 'mentor';
    if (t === 'analysis') markCurrent();
  }
  bag.on(tabA, 'click', () => setTab('analysis'));
  bag.on(tabM, 'click', () => setTab('mentor'));

  let ideasCtrl = null;
  bag.add(() => ideasCtrl?.abort());
  bag.on(ideasBtn, 'click', async () => {
    ideasCtrl?.abort();
    ideasCtrl = new AbortController();
    ideasBtn.classList.add('loading');
    const fen = state.cur.fen;
    try {
      const res = await api.get('/api/mentor/position' + qs({ fen }), { signal: ideasCtrl.signal, timeout: 30000 });
      const ideas = Array.isArray(res?.ideas) && res.ideas.length ? res.ideas : ['This position is balanced — develop your pieces and keep your king safe.'];
      mentor.say(ideas, { title: `Ideas (eval ${formatScore(res?.eval)})`, kind: 'idea' });
      if (Array.isArray(res?.best_line_san) && res.best_line_san.length) {
        mentor.say(`**Best line:** ${numberedLine(res.best_line_san, fenPly(fen), getSettings().moveNotation, 10)}`);
      }
    } catch (e) {
      if (!isAbort(e)) mentor.say(`I couldn't look at this position right now (${e.message}).`, { kind: 'error' });
    } finally {
      ideasBtn.classList.remove('loading');
    }
  });

  // ---- Persistence (per-viewer convenience) ----------------------------
  const persist = debounce(() => {
    if (state.gameId) return; // saved games live in the library
    try { safeSet(STORE_KEY, JSON.stringify(state.tree.serialize())); } catch { /* ignore */ }
  }, 400);
  bag.add(() => persist.cancel());

  // ---- Loading ----------------------------------------------------------
  function setTree(tree, { headers = {}, gameId = null, ply = null } = {}) {
    state.tree = tree;
    state.headers = headers;
    state.gameId = gameId;
    reviewLink.hidden = !gameId;
    if (gameId) reviewLink.href = `#/review/${encodeURIComponent(gameId)}`;
    // Keep the URL honest so a reload doesn't bring back an older position (no hashchange fired).
    try {
      const want = gameId ? `#/analysis?game=${encodeURIComponent(gameId)}` : '#/analysis';
      if (location.hash.startsWith('#/analysis') && location.hash !== want && (location.hash.includes('?') || gameId)) {
        if (!(gameId && location.hash.startsWith(want))) history.replaceState(null, '', want);
      }
    } catch { /* ignore */ }
    let target = tree.root;
    if (ply === 'end') { while (target.children[0]) target = target.children[0]; }
    else if (Number.isFinite(ply) && ply > 0) {
      for (let i = 0; i < ply && target.children[0]; i++) target = target.children[0];
    }
    lastArrowKey = '';
    goTo(target, { animate: false, rebuild: true });
  }

  function loadFen(raw) {
    const fen = normalizeFen(raw);
    const err = fenError(fen);
    if (err) { toast(`Invalid FEN: ${err}`, 'error'); return false; }
    setTree(new MoveTree(new Chess(fen).fen()));
    persist();
    return true;
  }

  function treeFromMoves(startFen, ucis) {
    const t = new MoveTree(startFen);
    const c = new Chess(startFen);
    let node = t.root;
    for (const u of ucis.slice(0, MAX_NODES - 1)) {
      const m = playUci(c, u);
      if (!m) break;
      node = t.add(node, { san: m.san, uci: moveUci(m), fen: c.fen() }).node;
    }
    return t;
  }

  function loadPgn(text) {
    const raw = String(text || '').slice(0, MAX_PGN_CHARS).trim();
    if (!raw) { toast('Paste a PGN first.', 'warning'); return false; }
    // Only the first game of a multi-game PGN.
    const first = raw.split(/\n\s*\n(?=\s*\[Event\s)/)[0];
    const c = new Chess();
    try {
      c.loadPgn(first);
    } catch (e) {
      toast(`Could not read that PGN: ${String(e?.message || e).slice(0, 160)}`, 'error');
      return false;
    }
    const headers = (typeof c.getHeaders === 'function' ? c.getHeaders() : c.header()) || {};
    const hist = c.history({ verbose: true });
    const startFen = headers.FEN ? normalizeFen(headers.FEN) : (hist[0]?.before || START_FEN);
    setTree(treeFromMoves(startFen, hist.map(moveUci)), { headers, ply: 'end' });
    persist();
    return true;
  }

  async function loadGame(id, ply) {
    try {
      const g = await api.get(`/api/games/${encodeURIComponent(id)}`, { signal: ctrl.signal });
      if (bag.disposed) return;
      const start = g.start_fen && g.start_fen !== 'start' ? g.start_fen : START_FEN;
      if (fenError(start)) throw new Error('The saved game has an invalid start position');
      const headers = { White: g.white, Black: g.black, Result: g.result, Event: 'GrandMentor game' };
      if (g.user_color === 'black') { board.setOrientation('black'); evalBar.setOrientation('black'); }
      setTree(treeFromMoves(start, Array.isArray(g.moves) ? g.moves : []), { headers, gameId: g.id, ply: ply ?? 'end' });
      toast(`Loaded ${g.white} vs ${g.black}`, 'success', { duration: 2000 });
    } catch (e) {
      if (!isAbort(e)) toast(`Could not load that game: ${e.message}`, 'error');
    }
  }

  // ---- PGN export ------------------------------------------------------
  function toPgn() {
    const t = state.tree;
    const hd = state.headers || {};
    const today = new Date();
    const date = `${today.getFullYear()}.${String(today.getMonth() + 1).padStart(2, '0')}.${String(today.getDate()).padStart(2, '0')}`;
    const tags = [
      ['Event', hd.Event || 'Analysis'], ['Site', 'GrandMentor'], ['Date', hd.Date || date],
      ['White', hd.White || '?'], ['Black', hd.Black || '?'], ['Result', hd.Result || '*'],
    ];
    if (t.root.fen !== START_FEN) tags.push(['SetUp', '1'], ['FEN', t.root.fen]);
    const esc = (s) => String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    const line = (start) => {
      const toks = [];
      let need = true;
      for (let n = start; n; n = n.children[0]) {
        const before = n.ply - 1;
        const no = Math.floor(before / 2) + 1;
        if (before % 2 === 0) toks.push(`${no}.`); else if (need) toks.push(`${no}...`);
        toks.push(n.san);
        need = false;
        const p = n.parent;
        if (p.children[0] === n && p.children.length > 1) {
          for (let i = 1; i < p.children.length; i++) toks.push(`(${line(p.children[i]).join(' ')})`);
          need = true;
        }
      }
      return toks;
    };
    const toks = t.root.children[0] ? line(t.root.children[0]) : [];
    toks.push(hd.Result || '*');
    let out = ''; let cur = '';
    for (const tok of toks.join(' ').split(' ')) {
      if (cur.length + tok.length + 1 > 80) { out += cur + '\n'; cur = tok; } else cur = cur ? `${cur} ${tok}` : tok;
    }
    out += cur;
    return tags.map(([k, v]) => `[${k} "${esc(v)}"]`).join('\n') + '\n\n' + out + '\n';
  }

  // ---- Modals -----------------------------------------------------------
  // At most one modal of ours is tracked; it is closed on unmount.
  let closeModal = null;
  const trackModal = (fn) => { closeModal = fn; };
  bag.add(() => { closeModal?.(); closeModal = null; });

  function openLoadModal() {
    const ta = h('textarea', { class: 'textarea mono an-load-text', rows: 8, placeholder: 'Paste a FEN or a PGN here…', spellcheck: 'false' });
    const gamesBox = h('div', { class: 'an-load-games' }, h('div', { class: 'subtle text-sm' }, 'Loading your games…'));
    const tPaste = h('button', { class: 'active', type: 'button' }, 'FEN / PGN');
    const tGames = h('button', { type: 'button' }, 'My games');
    const seg = h('div', { class: 'segmented block' }, tPaste, tGames);
    const pastePane = h('div', { class: 'stack-sm' }, ta, h('div', { class: 'help' }, 'We detect the format automatically. PGN variations are ignored; only the main line is loaded.'));
    const gamesPane = h('div', { hidden: true }, gamesBox);
    const body = h('div', { class: 'stack' }, seg, pastePane, gamesPane);
    let gamesLoaded = false;
    const lctrl = new AbortController();
    const showTab = (games) => {
      tPaste.classList.toggle('active', !games); tGames.classList.toggle('active', games);
      pastePane.hidden = games; gamesPane.hidden = !games;
      if (games && !gamesLoaded) {
        gamesLoaded = true;
        api.get('/api/games' + qs({ limit: 30 }), { signal: lctrl.signal }).then((list) => {
          if (!Array.isArray(list) || !list.length) {
            gamesBox.replaceChildren(h('div', { class: 'subtle text-sm' }, 'No saved games yet. Play a bot and your games appear here.'));
            return;
          }
          gamesBox.replaceChildren(h('div', { class: 'card card-flush list' }, list.map((g) => h('button', {
            class: 'list-row clickable', type: 'button',
            onClick: () => { m.close(); location.hash = `#/analysis?game=${encodeURIComponent(g.id)}`; },
          },
          h('span', { class: `result ${resultClass(g)}` }, resultLetter(g)),
          h('div', { class: 'list-row-main' },
            h('div', { class: 'list-row-title' }, `${g.white} vs ${g.black}`),
            h('div', { class: 'list-row-sub' }, [g.opening_name, `${Math.ceil((g.move_count || 0) / 2)} moves`, g.result].filter(Boolean).join(' · '))),
          h('span', { html: icon('chevron-right') })))));
        }).catch((e) => {
          if (!isAbort(e)) gamesBox.replaceChildren(h('div', { class: 'callout callout-danger' }, e.message));
        });
      }
    };
    tPaste.addEventListener('click', () => showTab(false));
    tGames.addEventListener('click', () => showTab(true));
    const m = modal({
      title: 'Load a position or game',
      body,
      onClose: () => lctrl.abort(),
      actions: [
        { label: 'Cancel', kind: 'ghost' },
        {
          label: 'Load', kind: 'primary', onClick: () => {
            const text = ta.value.trim();
            if (!text) { toast('Paste a FEN or PGN first.', 'warning'); return false; }
            const looksFen = /^[pnbrqkPNBRQK1-8]+(\/[pnbrqkPNBRQK1-8]+){7}(\s|$)/.test(text);
            return looksFen ? loadFen(text) : loadPgn(text);
          },
        },
      ],
    });
    trackModal(() => m.close());
  }

  function resultClass(g) {
    if (!g.user_color || g.result === '*') return 'result-draw';
    if (g.result === '1/2-1/2') return 'result-draw';
    const whiteWon = g.result === '1-0';
    return (whiteWon === (g.user_color === 'white')) ? 'result-win' : 'result-loss';
  }
  function resultLetter(g) {
    const c = resultClass(g);
    return c === 'result-win' ? 'W' : c === 'result-loss' ? 'L' : (g.result === '*' ? '–' : '½');
  }

  function openCopyMenu() {
    const m = modal({
      title: 'Copy position',
      body: h('div', { class: 'stack' },
        h('div', { class: 'field' }, h('label', { class: 'label' }, 'FEN (current position)'), h('div', { class: 'fen an-copy-fen' }, state.cur.fen)),
        h('div', { class: 'field' }, h('label', { class: 'label' }, 'PGN (with variations)'), h('pre', { class: 'an-copy-pgn mono' }, toPgn()))),
      actions: [
        { label: 'Copy FEN', kind: 'ghost', icon: 'copy', onClick: () => { copyText(state.cur.fen, 'FEN copied'); } },
        { label: 'Copy PGN', kind: 'primary', icon: 'copy', onClick: () => { copyText(toPgn(), 'PGN copied'); } },
      ],
    });
    trackModal(() => m.close());
  }

  function openSaveModal() {
    const ml = state.tree.mainline();
    const hd = state.headers || {};
    const white = h('input', { class: 'input', value: hd.White && hd.White !== '?' ? hd.White : 'White', maxlength: 60 });
    const black = h('input', { class: 'input', value: hd.Black && hd.Black !== '?' ? hd.Black : 'Black', maxlength: 60 });
    const result = h('select', { class: 'select' }, ['*', '1-0', '0-1', '1/2-1/2'].map((r) => h('option', { value: r, selected: (hd.Result || '*') === r }, r === '*' ? 'Unfinished / analysis' : r)));
    const notes = h('textarea', { class: 'textarea', rows: 3, maxlength: 2000, placeholder: 'What did you learn here? (optional)' });
    const m = modal({
      title: 'Save to your library',
      body: h('div', { class: 'stack' },
        h('div', { class: 'form-row' },
          h('div', { class: 'field' }, h('label', { class: 'label' }, 'White'), white),
          h('div', { class: 'field' }, h('label', { class: 'label' }, 'Black'), black)),
        h('div', { class: 'field' }, h('label', { class: 'label' }, 'Result'), result),
        h('div', { class: 'field' }, h('label', { class: 'label' }, 'Notes'), notes),
        h('div', { class: 'help' }, `Saves the main line (${ml.length} half-moves). Variations stay here on the analysis board.`)),
      actions: [
        { label: 'Cancel', kind: 'ghost' },
        {
          label: 'Save game', kind: 'primary', icon: 'save', onClick: async () => {
            try {
              const g = await api.post('/api/games', {
                white: white.value.trim() || 'White', black: black.value.trim() || 'Black',
                result: result.value, termination: result.value === '*' ? 'analysis' : '',
                start_fen: state.tree.root.fen, moves: ml.map((n) => n.uci),
                bot_id: null, user_color: null, time_control: null,
                opening_name: openingBar.classList.contains('known') ? openingName.textContent : null,
                notes: notes.value.trim(), tags: ['analysis'],
              }, { signal: ctrl.signal });
              if (bag.disposed) return;
              state.gameId = g.id;
              reviewLink.hidden = false;
              reviewLink.href = `#/review/${encodeURIComponent(g.id)}`;
              toast('Saved to your library', 'success');
              return true;
            } catch (e) {
              if (!isAbort(e)) toast(`Could not save: ${e.message}`, 'error');
              return false;
            }
          },
        },
      ],
    });
    trackModal(() => m.close());
  }

  bag.on(bNew, 'click', () => {
    setTree(new MoveTree(START_FEN));
    if (board.orientation === 'black') { board.setOrientation('white'); evalBar.setOrientation('white'); }
    persist();
  });
  bag.on(bLoad, 'click', openLoadModal);
  bag.on(bCopy, 'click', openCopyMenu);
  bag.on(bSave, 'click', () => {
    if (!state.tree.root.children.length) { toast('Make some moves first — then save them to your library.', 'info'); return; }
    openSaveModal();
  });
  bag.on(bSetup, 'click', () => {
    trackModal(openSetupEditor(state.cur.fen, board.orientation || 'white', (fen) => { setTree(new MoveTree(fen)); persist(); }));
  });

  // ---- Toolbar & keyboard ----------------------------------------------
  const flip = () => { board.flip(); evalBar.setOrientation(board.orientation); };
  bag.on(bFirst, 'click', step.first);
  bag.on(bPrev, 'click', step.prev);
  bag.on(bNext, 'click', step.next);
  bag.on(bLast, 'click', step.last);
  bag.on(bFlip, 'click', flip);
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    const map = { ArrowLeft: step.prev, ArrowRight: step.next, Home: step.first, End: step.last };
    if (map[e.key]) { e.preventDefault(); map[e.key](); return; }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      // Cycle between sibling variations at the current move.
      const p = state.cur.parent;
      if (p && p.children.length > 1) {
        e.preventDefault();
        const i = p.children.indexOf(state.cur);
        const n = p.children.length;
        goTo(p.children[(i + (e.key === 'ArrowDown' ? 1 : n - 1)) % n]);
      }
      return;
    }
    if (e.key === 'f' || e.key === 'F') flip();
    else if (e.key === 'e' || e.key === 'E') { engineSwitch.checked = !engineSwitch.checked; engineSwitch.dispatchEvent(new Event('change')); }
  });

  // ---- Initial load -----------------------------------------------------
  renderLines();
  const plyQ = query.ply != null && query.ply !== '' ? parseInt(query.ply, 10) : null;
  if (query.game) {
    goTo(state.tree.root, { animate: false, rebuild: true });
    await loadGame(query.game, Number.isFinite(plyQ) ? plyQ : null);
  } else if (query.pgn) {
    goTo(state.tree.root, { animate: false, rebuild: true });
    loadPgn(query.pgn);
  } else if (query.fen) {
    if (!loadFen(query.fen)) setTree(new MoveTree(START_FEN));
  } else {
    let restored = null;
    try {
      const raw = safeGet(STORE_KEY);
      if (raw && raw.length < 2_000_000) {
        const data = JSON.parse(raw);
        if (data && typeof data.fen === 'string' && !fenError(data.fen)) restored = MoveTree.deserialize(data);
      }
    } catch { restored = null; }
    setTree(restored || new MoveTree(START_FEN), { ply: restored ? 'end' : null });
    if (restored && restored.root.children.length) {
      toast('Restored your last analysis. Press "New" to start fresh.', 'info', { duration: 2500 });
    }
  }

  return bag.dispose;
}

// ---------------------------------------------------------------------------
// Setup position editor (modal). Returns a close() function.
// ---------------------------------------------------------------------------
const PALETTE = ['wK', 'wQ', 'wR', 'wB', 'wN', 'wP', 'bK', 'bQ', 'bR', 'bB', 'bN', 'bP'];
const FEN_CHAR = { wK: 'K', wQ: 'Q', wR: 'R', wB: 'B', wN: 'N', wP: 'P', bK: 'k', bQ: 'q', bR: 'r', bB: 'b', bN: 'n', bP: 'p' };
const CHAR_CODE = Object.fromEntries(Object.entries(FEN_CHAR).map(([k, v]) => [v, k]));
const PIECE_NAMES = { K: 'king', Q: 'queen', R: 'rook', B: 'bishop', N: 'knight', P: 'pawn' };

function placementToArray(placement) {
  const arr = new Array(64).fill(null);
  const rows = String(placement || '').split('/');
  for (let r = 0; r < 8 && r < rows.length; r++) {
    let f = 0;
    for (const ch of rows[r]) {
      if (f > 7) break;
      if (/[1-8]/.test(ch)) f += Number(ch);
      else if (CHAR_CODE[ch]) { arr[r * 8 + f] = CHAR_CODE[ch]; f++; }
    }
  }
  return arr;
}

function arrayToPlacement(arr) {
  const rows = [];
  for (let r = 0; r < 8; r++) {
    let s = ''; let empty = 0;
    for (let f = 0; f < 8; f++) {
      const p = arr[r * 8 + f];
      if (!p) { empty++; continue; }
      if (empty) { s += empty; empty = 0; }
      s += FEN_CHAR[p];
    }
    if (empty) s += empty;
    rows.push(s);
  }
  return rows.join('/');
}

const sqName = (idx) => 'abcdefgh'[idx % 8] + (8 - Math.floor(idx / 8));

export function openSetupEditor(fen, orientation, onApply) {
  const parts = String(fen || START_FEN).split(/\s+/);
  const st = {
    arr: placementToArray(parts[0]),
    turn: parts[1] === 'b' ? 'b' : 'w',
    castling: new Set((parts[2] || '').replace('-', '').split('').filter((c) => 'KQkq'.includes(c))),
    tool: 'move',
    flipped: orientation === 'black',
  };
  let dragCleanup = null;

  const boardEl = h('div', { class: 'se-board', role: 'grid', 'aria-label': 'Position editor board' });
  const paletteW = h('div', { class: 'se-palette' });
  const paletteB = h('div', { class: 'se-palette' });
  const errorBox = h('div', { class: 'callout callout-danger se-error', hidden: true });
  const fenField = h('input', { class: 'input input-sm mono', spellcheck: 'false', 'aria-label': 'FEN' });
  const turnW = h('button', { type: 'button' }, 'White to move');
  const turnB = h('button', { type: 'button' }, 'Black to move');
  const castleBoxes = ['K', 'Q', 'k', 'q'].map((c) => {
    const input = h('input', { type: 'checkbox' });
    input.addEventListener('change', () => { if (input.checked) st.castling.add(c); else st.castling.delete(c); update(); });
    const label = { K: 'White O-O', Q: 'White O-O-O', k: 'Black O-O', q: 'Black O-O-O' }[c];
    return { c, input, el: h('label', { class: 'checkbox' }, input, ` ${label}`) };
  });

  const toolBtn = (code, label, html) => h('button', { class: 'se-tool', type: 'button', dataset: { tool: code }, 'aria-label': label, title: label, html });
  const moveTool = toolBtn('move', 'Move pieces (drag)', icon('grid'));
  const eraseTool = toolBtn('erase', 'Eraser', icon('trash'));
  for (const code of PALETTE) {
    const b = toolBtn(code, `${code[0] === 'w' ? 'White' : 'Black'} ${PIECE_NAMES[code[1]]}`, `<img src="${pieceUrl(code)}" alt="" draggable="false">`);
    (code[0] === 'w' ? paletteW : paletteB).appendChild(b);
  }
  paletteW.prepend(moveTool);
  paletteB.prepend(eraseTool);

  const castleRights = () => {
    const a = st.arr;
    const ok = {
      K: a[60] === 'wK' && a[63] === 'wR', Q: a[60] === 'wK' && a[56] === 'wR',
      k: a[4] === 'bK' && a[7] === 'bR', q: a[4] === 'bK' && a[0] === 'bR',
    };
    return ok;
  };

  const currentFen = () => {
    const ok = castleRights();
    const c = ['K', 'Q', 'k', 'q'].filter((x) => st.castling.has(x) && ok[x]).join('') || '-';
    return `${arrayToPlacement(st.arr)} ${st.turn} ${c} - 0 1`;
  };

  function renderBoard() {
    const frag = document.createDocumentFragment();
    for (let i = 0; i < 64; i++) {
      const idx = st.flipped ? 63 - i : i;
      const light = (Math.floor(idx / 8) + (idx % 8)) % 2 === 0;
      const p = st.arr[idx];
      const sq = h('div', { class: `se-sq ${light ? 'light' : 'dark'}`, dataset: { idx }, role: 'gridcell', 'aria-label': `${sqName(idx)}${p ? ' ' + p : ''}` });
      if (p) sq.appendChild(h('img', { src: pieceUrl(p), alt: '', draggable: 'false' }));
      if (i % 8 === 0) sq.appendChild(h('span', { class: 'se-coord rank' }, String(8 - Math.floor(idx / 8))));
      if (i >= 56) sq.appendChild(h('span', { class: 'se-coord file' }, 'abcdefgh'[idx % 8]));
      frag.appendChild(sq);
    }
    boardEl.replaceChildren(frag);
  }

  function update() {
    renderBoard();
    turnW.classList.toggle('active', st.turn === 'w');
    turnB.classList.toggle('active', st.turn === 'b');
    const ok = castleRights();
    for (const cb of castleBoxes) {
      cb.input.disabled = !ok[cb.c];
      cb.input.checked = ok[cb.c] && st.castling.has(cb.c);
    }
    for (const b of [moveTool, eraseTool, ...paletteW.children, ...paletteB.children]) b.classList?.toggle('active', b.dataset?.tool === st.tool);
    const f = currentFen();
    if (document.activeElement !== fenField) fenField.value = f;
    const err = fenError(f);
    errorBox.hidden = !err;
    errorBox.textContent = err || '';
    return err;
  }

  function applyTool(idx) {
    if (st.tool === 'erase') st.arr[idx] = null;
    else if (st.tool !== 'move') st.arr[idx] = st.arr[idx] === st.tool ? null : st.tool;
    update();
  }

  function startDrag(e, code, fromIdx, onClick) {
    e.preventDefault();
    dragCleanup?.();
    const size = boardEl.getBoundingClientRect().width / 8 || 48;
    const ghost = h('img', { class: 'se-ghost', src: pieceUrl(code), alt: '', style: { width: `${size}px`, height: `${size}px` } });
    document.body.appendChild(ghost);
    const sx = e.clientX; const sy = e.clientY;
    let moved = false;
    const place = (x, y) => { ghost.style.transform = `translate(${x - size / 2}px, ${y - size / 2}px)`; };
    place(sx, sy);
    ghost.style.opacity = '0';
    const onMove = (ev) => {
      if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) > 4) {
        moved = true;
        ghost.style.opacity = '1';
        if (fromIdx != null) { st.arr[fromIdx] = null; renderBoard(); }
      }
      if (moved) place(ev.clientX, ev.clientY);
    };
    const finish = (ev, cancelled) => {
      cleanup();
      if (!moved) { if (!cancelled) onClick?.(); return; }
      const target = cancelled ? null : document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.('.se-sq');
      if (target && boardEl.contains(target)) st.arr[Number(target.dataset.idx)] = code;
      update();
    };
    const onUp = (ev) => finish(ev, false);
    const onCancel = (ev) => {
      if (fromIdx != null && moved) st.arr[fromIdx] = code; // put it back
      finish(ev, true);
    };
    function cleanup() {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      ghost.remove();
      dragCleanup = null;
    }
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    dragCleanup = cleanup;
  }

  boardEl.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const sq = e.target.closest('.se-sq');
    if (!sq) return;
    const idx = Number(sq.dataset.idx);
    const p = st.arr[idx];
    if (p && st.tool !== 'erase') startDrag(e, p, idx, () => applyTool(idx));
    else { e.preventDefault(); applyTool(idx); }
  });
  boardEl.addEventListener('contextmenu', (e) => {
    const sq = e.target.closest('.se-sq');
    if (!sq) return;
    e.preventDefault();
    st.arr[Number(sq.dataset.idx)] = null;
    update();
  });
  const onPalette = (e) => {
    if (e.button !== 0) return;
    const b = e.target.closest('.se-tool');
    if (!b) return;
    const tool = b.dataset.tool;
    const select = () => { st.tool = tool; update(); };
    if (tool === 'move' || tool === 'erase') { e.preventDefault(); select(); return; }
    startDrag(e, tool, null, select);
  };
  paletteW.addEventListener('pointerdown', onPalette);
  paletteB.addEventListener('pointerdown', onPalette);
  turnW.addEventListener('click', () => { st.turn = 'w'; update(); });
  turnB.addEventListener('click', () => { st.turn = 'b'; update(); });
  fenField.addEventListener('keydown', (e) => e.stopPropagation());
  fenField.addEventListener('change', () => {
    const p = fenField.value.trim().split(/\s+/);
    if (!p[0] || p[0].split('/').length !== 8) { errorBox.hidden = false; errorBox.textContent = 'That FEN does not have 8 ranks.'; return; }
    st.arr = placementToArray(p[0]);
    st.turn = p[1] === 'b' ? 'b' : 'w';
    st.castling = new Set((p[2] || '').replace('-', '').split('').filter((c) => 'KQkq'.includes(c)));
    update();
  });

  const smallBtn = (ic, label, fn) => {
    const b = h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon(ic) + `<span>${label}</span>` });
    b.addEventListener('click', fn);
    return b;
  };
  const tools = h('div', { class: 'row-wrap se-quick' },
    smallBtn('refresh', 'Starting position', () => {
      const p = START_FEN.split(' ');
      st.arr = placementToArray(p[0]); st.turn = 'w'; st.castling = new Set(['K', 'Q', 'k', 'q']); update();
    }),
    smallBtn('trash', 'Clear board', () => {
      st.arr = new Array(64).fill(null); st.castling.clear(); update();
    }),
    smallBtn('flip', 'Flip', () => { st.flipped = !st.flipped; update(); }));

  const body = h('div', { class: 'setup-editor' },
    h('div', { class: 'se-left' }, paletteB, boardEl, paletteW),
    h('div', { class: 'se-right stack' },
      h('p', { class: 'muted text-sm' }, 'Drag pieces from the palette onto the board, or pick a piece and tap squares. Drag a piece off the board (or right-click it) to remove it.'),
      tools,
      h('div', { class: 'field' }, h('label', { class: 'label' }, 'Side to move'), h('div', { class: 'segmented block' }, turnW, turnB)),
      h('div', { class: 'field' }, h('label', { class: 'label' }, 'Castling rights'), h('div', { class: 'se-castling' }, castleBoxes.map((c) => c.el))),
      h('div', { class: 'field' }, h('label', { class: 'label' }, 'FEN'), fenField),
      errorBox));

  update();
  const m = modal({
    title: 'Set up a position',
    size: 'lg',
    body,
    className: 'setup-modal',
    onClose: () => { dragCleanup?.(); },
    actions: [
      { label: 'Cancel', kind: 'ghost' },
      {
        label: 'Analyse this position', kind: 'primary', icon: 'analysis', onClick: () => {
          const err = update();
          if (err) {
            errorBox.classList.remove('shake'); void errorBox.offsetWidth; errorBox.classList.add('shake');
            return false;
          }
          onApply(currentFen());
          return true;
        },
      },
    ],
  });
  return () => { dragCleanup?.(); m.close(); };
}
