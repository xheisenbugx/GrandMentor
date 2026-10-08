// Endgame drills: list (#/endgames) and drill vs the engine (#/endgames/:id).
// The opponent is the engine at full strength (POST /api/engine/analyze, movetime 500ms; falls back
// to POST /api/bot/move with the strongest bot). Success = goal reached (mate for "win"; draw by rule
// or surviving long enough for "draw"). Local progress is stored per drill in localStorage.

import { h, icon, disposables, pageHeader, emptyState, skeleton, mdLite, escapeHtml, formatSan, loadingBlock } from '../ui.js';
import { api, isAbort } from '../api.js';
import { getSetting } from '../settings.js';
import { Board } from '../components/board.js';
import {
  ensureLearnCss, chessAt, applyUci, moveUci, sideToMove, fenKey, miniBoardSvg, levelPill, timerSet,
  confetti, flashClass, sfx, setFeedback, breadcrumbs, errorBlock, readStore, writeStore,
} from './learn.js';

export const title = (params) => (params && params.id ? 'Endgame drill' : 'Endgames');

const PROGRESS_KEY = 'gm.endgames.v1';
const ENGINE_MOVETIME_MS = 500;
const HINT_MOVETIME_MS = 700;
const MIN_THINK_MS = 350;
const DRAW_SURVIVE_MOVES = 30;     // "draw" goal: hold this many of your own moves
const LOST_CP = -800;              // user-POV eval below which a drill counts as lost

const CATEGORY_META = {
  basic: { label: 'Basic checkmates', emoji: '👑', blurb: 'Every player must know these mates.' },
  pawn: { label: 'Pawn endings', emoji: '♟️', blurb: 'Opposition, key squares and the race to promote.' },
  rook: { label: 'Rook endings', emoji: '🏰', blurb: 'The most common endings in real games.' },
  minor: { label: 'Minor pieces', emoji: '🐴', blurb: 'Bishops and knights in the endgame.' },
  queen: { label: 'Queen endings', emoji: '👸', blurb: 'Queen versus pawns and more.' },
};
const CATEGORY_ORDER = ['basic', 'pawn', 'rook', 'minor', 'queen'];

function progressDb() { return readStore(PROGRESS_KEY, {}); }
function markDone(id, moves, validIds) {
  const db = progressDb();
  if (validIds) for (const k of Object.keys(db)) if (!validIds.has(k)) delete db[k];
  const prev = db[id] || {};
  db[id] = { done: true, best: prev.best ? Math.min(prev.best, moves) : moves, at: Date.now() };
  writeStore(PROGRESS_KEY, db);
  return db[id];
}

function goalOf(d) { return d && d.goal === 'draw' ? 'draw' : 'win'; }
function goalBadge(goal) {
  return h('span', { class: `lrn-goal ${goal}` }, goal === 'draw' ? 'Draw' : 'Win');
}
function sortDrills(list) {
  const lv = { beginner: 0, intermediate: 1, advanced: 2, master: 3 };
  return [...list].sort((a, b) => {
    const ca = CATEGORY_ORDER.indexOf(a.category); const cb = CATEGORY_ORDER.indexOf(b.category);
    return (ca < 0 ? 99 : ca) - (cb < 0 ? 99 : cb) || (lv[a.level] ?? 9) - (lv[b.level] ?? 9) || String(a.title).localeCompare(String(b.title));
  });
}

export async function mount(root, { params = {} } = {}) {
  ensureLearnCss();
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
  const header = pageHeader({
    title: 'Endgame drills', icon: 'endgames',
    subtitle: 'Practise must-know endgames against the engine at full strength. Reach the goal to complete a drill.',
  });
  page.replaceChildren(header, h('div', { class: 'eg-grid mt-6' }, skeleton('card', 6)));

  const raw = await api.get('/api/endgames', { signal });
  if (bag.disposed) return;
  const drills = sortDrills((Array.isArray(raw) ? raw : []).filter((d) => d && d.id && chessAt(d.fen)));
  if (!drills.length) {
    page.replaceChildren(header, emptyState({ emoji: '🏁', title: 'No drills yet', text: 'Endgame drills appear here once data/endgames.json has entries.' }));
    return;
  }
  const db = progressDb();
  const doneCount = drills.filter((d) => db[d.id] && db[d.id].done).length;
  const pct = Math.round((doneCount / drills.length) * 100);

  const firstOpen = drills.find((d) => !(db[d.id] && db[d.id].done));
  const hero = h('section', { class: 'lrn-hero mt-4' },
    h('div', { class: 'lrn-hero-emoji', 'aria-hidden': 'true' }, doneCount === drills.length ? '🏆' : '🏁'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'lrn-hero-kicker' }, firstOpen ? 'Next drill' : 'All drills complete'),
      h('div', { class: 'lrn-hero-title' }, firstOpen ? firstOpen.title : 'You mastered every drill!'),
      h('p', { class: 'lrn-hero-sub' }, `${doneCount} of ${drills.length} drills completed`),
      h('div', { class: 'progress progress-sm' }, h('div', { class: 'progress-bar', style: `width:${pct}%` }))),
    firstOpen ? h('a', { class: 'btn btn-primary btn-lg', href: `#/endgames/${encodeURIComponent(firstOpen.id)}`, html: icon('play') + '<span>Start drill</span>' }) : null);

  const cats = [...new Set(drills.map((d) => d.category))].sort((a, b) => {
    const ia = CATEGORY_ORDER.indexOf(a); const ib = CATEGORY_ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
  let active = 'all';
  const chips = h('div', { class: 'chip-row mt-6' });
  const sections = h('div');
  const renderChips = () => chips.replaceChildren(
    ...[['all', 'All'], ...cats.map((c) => [c, `${(CATEGORY_META[c] || {}).emoji || '♟️'} ${(CATEGORY_META[c] || {}).label || c}`])].map(([k, label]) =>
      h('button', { type: 'button', class: ['chip', active === k && 'active'], 'aria-pressed': active === k ? 'true' : 'false', onClick: () => { active = k; renderChips(); renderSections(); } }, label)));
  const renderSections = () => sections.replaceChildren(...cats.filter((c) => active === 'all' || c === active).map((c) => {
    const meta = CATEGORY_META[c] || { label: c, emoji: '♟️', blurb: '' };
    const list = drills.filter((d) => d.category === c);
    const done = list.filter((d) => db[d.id] && db[d.id].done).length;
    return h('section', { class: 'lrn-section' },
      h('div', { class: 'lrn-section-head' },
        h('span', { class: 'lrn-section-emoji', 'aria-hidden': 'true' }, meta.emoji),
        h('div', null,
          h('h2', null, meta.label, ' ', h('span', { class: 'lrn-count' }, `· ${done}/${list.length} done`)),
          meta.blurb ? h('div', { class: 'muted text-sm' }, meta.blurb) : null)),
      h('div', { class: 'eg-grid' }, list.map((d) => drillCard(d, db[d.id]))));
  }));
  renderChips();
  renderSections();
  page.replaceChildren(header, hero, chips, sections);
}

function drillCard(d, rec) {
  const done = rec && rec.done;
  return h('a', { class: 'card card-link eg-card', href: `#/endgames/${encodeURIComponent(d.id)}`, title: d.title },
    done ? h('span', { class: 'lrn-check', html: icon('check'), 'aria-label': 'Completed' }) : null,
    h('div', { html: miniBoardSvg(d.fen, sideToMove(d.fen)) }),
    h('h3', { class: 'eg-card-title' }, d.title),
    d.description ? h('p', { class: 'eg-card-desc' }, d.description) : null,
    h('div', { class: 'eg-card-meta' }, goalBadge(goalOf(d)), levelPill(d.level),
      done && rec.best ? h('span', { class: 'text-xs subtle' }, `Best: ${rec.best} moves`) : null));
}

// ===========================================================================
// Drill
// ===========================================================================
async function mountDrill(root, id, bag, signal) {
  root.appendChild(loadingBlock('Setting up the drill…'));
  const raw = await api.get('/api/endgames', { signal });
  if (bag.disposed) return;
  const drills = sortDrills((Array.isArray(raw) ? raw : []).filter((d) => d && d.id && chessAt(d.fen)));
  const drill = drills.find((d) => d.id === id);
  if (!drill) {
    root.replaceChildren(h('div', { class: 'page' }, emptyState({ emoji: '🔍', title: 'Drill not found', text: 'We could not find this endgame drill.', action: { label: 'All drills', href: '#/endgames' } })));
    return;
  }
  const validIds = new Set(drills.map((d) => d.id));
  const idx = drills.indexOf(drill);
  const nextDrill = drills[idx + 1] || null;
  const goal = goalOf(drill);
  const startFen = chessAt(drill.fen).fen();
  const userColor = sideToMove(startFen);
  const engineColor = userColor === 'white' ? 'black' : 'white';
  const userChar = userColor === 'white' ? 'w' : 'b';

  const timers = timerSet();
  bag.add(() => timers.clear());
  let engineCtrl = null;
  let hintCtrl = null;
  bag.add(() => { if (engineCtrl) engineCtrl.abort(); if (hintCtrl) hintCtrl.abort(); });

  // ------------------------------------------------------------------ state
  let game = chessAt(startFen);
  let sans = [];
  let status = 'play';     // play | won | lost
  let busy = false;
  let gen = 0;
  let hintStage = 0;
  let hintsUsed = 0;
  const hintCache = new Map(); // fenKey -> {uci, san} (bounded below)

  // ------------------------------------------------------------------ layout
  const boardSlot = h('div', { class: 'board-slot' });
  const boardWrap = h('div', { class: 'board-row lrn-board-wrap' }, boardSlot);
  const thinking = h('span', { class: 'eg-thinking', hidden: true, 'aria-label': 'Engine is thinking' }, h('i'), h('i'), h('i'));
  const topBar = h('div', { class: 'player-bar' },
    h('div', { class: 'avatar', 'aria-hidden': 'true' }, '🤖'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'player-name' }, 'Engine ', h('span', { class: 'player-rating' }, '(full strength)')),
      h('div', { class: 'player-captures' }, engineColor === 'white' ? 'White' : 'Black')),
    h('div', { class: 'clock-slot' }, thinking));
  const bottomBar = h('div', { class: 'player-bar' },
    h('div', { class: 'avatar', 'aria-hidden': 'true' }, '🙂'),
    h('div', { style: 'min-width:0' },
      h('div', { class: 'player-name' }, 'You'),
      h('div', { class: 'player-captures' }, userColor === 'white' ? 'White' : 'Black')));

  const goalText = goal === 'win' ? `${userColor === 'white' ? 'White' : 'Black'} to play and win` : `${userColor === 'white' ? 'White' : 'Black'} to play and draw`;
  const goalSub = goal === 'win' ? 'Deliver checkmate against best defence.' : `Reach a draw — or survive ${DRAW_SURVIVE_MOVES} moves without losing.`;
  const banner = h('div', { class: `eg-banner ${goal}` },
    goalBadge(goal),
    h('div', null, h('div', { class: 'eg-banner-title' }, goalText), h('div', { class: 'eg-banner-sub' }, goalSub)));

  const feedback = h('div', { class: 'lrn-feedback', role: 'status', 'aria-live': 'polite' });
  const resultBox = h('div');
  const movesEl = h('div', { class: 'eg-movelist', 'aria-label': 'Moves' });
  const technique = Array.isArray(drill.technique) ? drill.technique.filter(Boolean) : [];

  const body = h('div', { class: 'panel-body' },
    banner,
    drill.description ? h('div', { class: 'lrn-coach mt-4' }, h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'), h('div', { class: 'lrn-text md', html: mdLite(drill.description) })) : null,
    feedback,
    resultBox,
    technique.length ? h('div', null,
      h('div', { class: 'op-block-title', html: icon('hint') + '<span>Technique</span>' }),
      h('ol', { class: 'eg-technique' }, technique.map((t) => h('li', { html: `<span>${mdLite(t).replace(/^<p>|<\/p>$/g, '')}</span>` })))) : null,
    h('div', { class: 'op-block-title', html: icon('list') + '<span>Moves</span>' }),
    movesEl);

  const hintBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('hint') + '<span>Hint</span>', onClick: () => showHint() });
  const undoBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('undo') + '<span>Undo</span>', onClick: () => undo() });
  const resetBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + '<span>Reset</span>', onClick: () => reset() });
  const flipBtn = h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': 'Flip board', 'data-tooltip': 'Flip board', html: icon('flip'), onClick: () => board.flip() });
  const footer = h('div', { class: 'panel-footer' }, hintBtn, undoBtn, resetBtn, flipBtn);

  const panelHeader = h('div', { class: 'panel-header' }, h('span', { 'aria-hidden': 'true' }, (CATEGORY_META[drill.category] || {}).emoji || '🏁'), h('span', { class: 'truncate' }, drill.title), h('span', { class: 'spacer' }), levelPill(drill.level));
  const panel = h('div', { class: 'panel grow' }, panelHeader, body, footer);

  const head = breadcrumbs([{ label: 'Endgames', href: '#/endgames' }, { label: (CATEGORY_META[drill.category] || {}).label || drill.category }, { label: drill.title }]);

  const layout = h('div', { class: 'game-layout no-eval eg-drill' },
    h('div', { class: 'game-main' }, topBar, boardWrap, bottomBar),
    h('aside', { class: 'game-panel' }, head, panel));
  root.replaceChildren(layout);

  const board = new Board(boardSlot, {
    fen: startFen,
    orientation: userColor,
    interactive: true,
    movableColor: userColor,
    onMove: (mv) => onUserMove(mv),
  });
  bag.add(() => board.destroy());

  // ------------------------------------------------------------------ helpers
  const sleep = (ms) => new Promise((resolve) => timers.later(resolve, ms));
  const sanOf = (s) => formatSan(s, getSetting('moveNotation'));
  // The user is always to move in the drill's start position, so user moves = ceil(plies / 2).
  const userMoveCount = () => Math.ceil(sans.length / 2);

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
    if (!nodes.length) nodes.push(h('span', { class: 'subtle' }, 'Your move — good luck!'));
    movesEl.replaceChildren(...nodes);
  }

  function syncControls() {
    const canUndo = sans.length > 0;
    undoBtn.disabled = !canUndo;
    resetBtn.disabled = sans.length === 0 && status === 'play';
    hintBtn.disabled = status !== 'play' || busy;
    thinking.hidden = !busy;
    const myTurn = status === 'play' && !busy && game.turn() === userChar;
    board.setInteractive(myTurn, myTurn ? userColor : null);
  }

  // ------------------------------------------------------------------ outcome
  function outcome() {
    if (game.isCheckmate()) {
      const loser = game.turn(); // side to move is mated
      return loser === userChar
        ? { result: 'lost', reason: 'You were checkmated.' }
        : { result: 'won', reason: goal === 'win' ? 'Checkmate! You converted the endgame.' : 'Checkmate — even better than a draw!' };
    }
    let drawReason = null;
    if (game.isStalemate()) drawReason = 'Stalemate — the king had no legal moves.';
    else if (game.isInsufficientMaterial()) drawReason = 'Draw by insufficient material.';
    else if (game.isThreefoldRepetition()) drawReason = 'Draw by threefold repetition.';
    else if (game.isDrawByFiftyMoves && game.isDrawByFiftyMoves()) drawReason = 'Draw by the 50-move rule.';
    else if (game.isDraw()) drawReason = 'The game is drawn.';
    if (drawReason) {
      return goal === 'draw'
        ? { result: 'won', reason: `${drawReason} You held the draw!` }
        : { result: 'lost', reason: `${drawReason} You needed to win this one.` };
    }
    if (goal === 'win') {
      // The user's side has only a king left → no way to win.
      const onlyKing = game.board().flat().filter((p) => p && p.color === userChar).every((p) => p.type === 'k');
      if (onlyKing) return { result: 'lost', reason: 'You have no pieces left to checkmate with.' };
    }
    if (goal === 'draw' && game.turn() === userChar && userMoveCount() >= DRAW_SURVIVE_MOVES) {
      return { result: 'won', reason: `You survived ${DRAW_SURVIVE_MOVES} moves against the engine — that's a draw!` };
    }
    return null;
  }

  function finish({ result, reason }) {
    status = result;
    busy = false;
    syncControls();
    if (result === 'won') {
      const moves = userMoveCount();
      const rec = markDone(drill.id, moves, validIds);
      sfx('gameEnd');
      confetti(boardSlot, timers, { count: 48 });
      flashClass(boardWrap, 'flash-good', timers, 1000);
      setFeedback(feedback, null);
      resultBox.replaceChildren(h('div', { class: 'lrn-complete' },
        h('div', { class: 'lrn-complete-badge', html: icon('trophy') }),
        h('h2', null, 'Drill complete!'),
        h('p', null, `${reason} ${moves} move${moves === 1 ? '' : 's'}${hintsUsed ? `, ${hintsUsed} hint${hintsUsed === 1 ? '' : 's'}` : ''}. Best: ${rec.best}.`),
        h('div', { class: 'lrn-nav' },
          nextDrill ? h('a', { class: 'btn btn-primary btn-lg', href: `#/endgames/${encodeURIComponent(nextDrill.id)}`, html: '<span>Next drill</span>' + icon('chevron-right') }) : h('a', { class: 'btn btn-primary btn-lg', href: '#/endgames', html: icon('grid') + '<span>All drills</span>' }),
          h('button', { class: 'btn btn-secondary', type: 'button', html: icon('refresh') + '<span>Play again</span>', onClick: () => reset() }))));
    } else {
      sfx('wrong');
      flashClass(boardSlot, 'shake', timers, 420);
      setFeedback(feedback, null);
      resultBox.replaceChildren(h('div', { class: 'lrn-complete' },
        h('div', { class: 'lrn-complete-badge fail', html: icon('x') }),
        h('h2', null, 'Not quite'),
        h('p', null, reason),
        h('div', { class: 'lrn-nav' },
          h('button', { class: 'btn btn-primary btn-lg', type: 'button', html: icon('refresh') + '<span>Try again</span>', onClick: () => reset() }),
          sans.length ? h('button', { class: 'btn btn-secondary', type: 'button', html: icon('undo') + '<span>Undo last move</span>', onClick: () => undo() }) : null,
          h('a', { class: 'btn btn-ghost', href: `#/analysis?fen=${encodeURIComponent(startFen)}`, html: icon('analysis') + '<span>Study in analysis</span>' }))));
    }
    resultBox.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  // ------------------------------------------------------------------ moves
  function onUserMove(mv) {
    if (status !== 'play' || busy || game.turn() !== userChar) return false;
    const m = applyUci(game, mv.uci || (mv.from + mv.to + (mv.promotion || '')));
    if (!m) return false;
    sans.push(m.san);
    hintStage = 0;
    board.clearArrows();
    board.setHighlights([]);
    setFeedback(feedback, null);
    renderMoves();
    const out = outcome();
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
        if (!list.length) throw new Error('No bots available');
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
    // Fallback: strongest bot
    const botId = await strongestBotId(sig);
    const historyUcis = game.history({ verbose: true }).map(moveUci);
    const bm = await api.post('/api/bot/move', { bot_id: botId, start_fen: startFen, moves: historyUcis }, { signal: sig, timeout: 15000 });
    if (!bm || !bm.uci) throw new Error('The engine did not return a move');
    return { uci: bm.uci, score: null };
  }

  function userPovLost(score) {
    if (!score || typeof score !== 'object') return false;
    const sign = userColor === 'white' ? 1 : -1;
    if (typeof score.mate === 'number') return score.mate * sign < 0;
    if (typeof score.cp === 'number') return score.cp * sign <= LOST_CP;
    return false;
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
      setFeedback(feedback, 'bad', `The engine could not move: ${escapeHtml(e && e.message ? e.message : 'unknown error')}.`);
      feedback.appendChild(h('button', { class: 'btn btn-sm btn-secondary', type: 'button', style: 'margin-left:auto', onClick: () => { setFeedback(feedback, null); engineTurn(); } }, 'Retry'));
      return;
    }
    const elapsed = performance.now() - started;
    if (elapsed < MIN_THINK_MS) await sleep(MIN_THINK_MS - elapsed);
    if (myGen !== gen || bag.disposed || status !== 'play') return;

    const m = applyUci(game, res.uci);
    if (!m) {
      busy = false; syncControls();
      setFeedback(feedback, 'bad', 'The engine suggested an illegal move. Try Undo or Reset.');
      return;
    }
    sans.push(m.san);
    board.setPosition(game.fen(), { animate: true, lastMove: [m.from, m.to] });
    renderMoves();
    busy = false;

    const out = outcome();
    if (out) { finish(out); return; }
    if (userPovLost(res.score)) {
      finish({ result: 'lost', reason: goal === 'win' ? 'The advantage slipped away — the engine is now winning.' : 'The engine has a winning position now.' });
      return;
    }
    if (game.inCheck()) setFeedback(feedback, 'warn', 'Check! Get your king to safety.');
    syncControls();
  }

  // ------------------------------------------------------------------ controls
  async function showHint() {
    if (status !== 'play' || busy || game.turn() !== userChar) return;
    if (hintStage === 0) {
      hintStage = 1;
      hintsUsed++;
      setFeedback(feedback, 'warn', mdLite(drill.hint || 'Think about the technique list below.').replace(/^<p>|<\/p>$/g, '') + ' <span class="subtle">(Press Hint again for the best move.)</span>');
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
        if (!uci) throw new Error('No hint available');
        const c = chessAt(fen); const mv = applyUci(c, uci);
        best = { uci, san: mv ? mv.san : uci };
        if (hintCache.size > 64) hintCache.clear();
        hintCache.set(key, best);
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        setFeedback(feedback, 'bad', 'Could not get a hint from the engine right now.');
        return;
      } finally {
        if (!bag.disposed) hintBtn.classList.remove('loading');
      }
      if (myGen !== gen || fenKey(game.fen()) !== key) return;
    }
    hintStage = 2;
    board.setArrows([{ from: best.uci.slice(0, 2), to: best.uci.slice(2, 4), color: 'green' }]);
    board.setHighlights([{ square: best.uci.slice(0, 2), kind: 'hint' }]);
    setFeedback(feedback, 'warn', `The engine suggests <strong>${escapeHtml(sanOf(best.san))}</strong>.`);
  }

  function cancelEngine() {
    gen++;
    if (engineCtrl) { engineCtrl.abort(); engineCtrl = null; }
    timers.clear();
    busy = false;
  }

  function undo() {
    if (!sans.length) return;
    cancelEngine();
    // Remove moves until it's the user's turn again and at least one user move was taken back.
    while (sans.length) {
      const m = game.undo();
      if (!m) break;
      sans.pop();
      if (m.color === userChar) break;
    }
    status = 'play';
    hintStage = 0;
    const hist = game.history({ verbose: true });
    const last = hist.length ? hist[hist.length - 1] : null;
    board.setPosition(game.fen(), { animate: true, lastMove: last ? [last.from, last.to] : null, sound: false });
    board.clearArrows(); board.setHighlights([]);
    resultBox.replaceChildren();
    setFeedback(feedback, 'info', 'Move taken back. Your turn.');
    renderMoves();
    syncControls();
  }

  function reset() {
    cancelEngine();
    game = chessAt(startFen);
    sans = [];
    status = 'play';
    hintStage = 0;
    hintsUsed = 0;
    board.setPosition(startFen, { animate: true, sound: false });
    board.clearArrows(); board.setHighlights([]);
    resultBox.replaceChildren();
    setFeedback(feedback, null);
    renderMoves();
    syncControls();
  }

  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'f' || e.key === 'F') board.flip();
    else if (e.key === 'h' || e.key === 'H') showHint();
    else if (e.key === 'ArrowLeft' || ((e.key === 'z' || e.key === 'Z') && !e.shiftKey)) { if (sans.length) { e.preventDefault(); undo(); } }
  });

  renderMoves();
  syncControls();
}
