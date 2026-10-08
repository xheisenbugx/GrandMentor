// Interactive lesson player (#/learn/:courseId/:lessonId).
// Step-by-step: coach text (markdown-lite) + board with arrows/highlights. Steps may carry a task
// (docs/CONTRACT.md "Interactive lessons"):
//   moves   – play the solution moves; scripted replies are auto-played
//   guess   – guess a master's moves; exact = 3 pts, engine-close alternatives earn partial credit
//   count   – who is ahead in material and by how much (answer buttons)
//   hanging – click every hanging piece (multi-select, then Check)
//   choice  – pick the best option; each option draws its arrows on the board
//   square  – coordinate quiz: click the named squares (optionally without board coordinates)
// Completion → POST /api/progress.

import { h, icon, disposables, mdLite, escapeHtml, emptyState, loadingBlock } from '../ui.js';
import { api, isAbort } from '../api.js';
import { Board } from '../components/board.js';
import { pieceUrl } from '../settings.js';
import {
  ensureLearnCss, START_FEN, chessAt, applyUci, isSameMove, sideToMove, playLine, timerSet,
  confetti, flashClass, sfx, setFeedback, breadcrumbs, rememberLesson, errorBlock, CATEGORIES,
} from './learn.js';
import { t, formatNumber } from '../i18n.js';

export const title = () => t('lesson.title');

const ARROW_COLORS = new Set(['green', 'red', 'blue', 'yellow']);
/** Option arrows are recoloured by position so the colour never gives the answer away. */
const OPTION_COLORS = ['blue', 'yellow', 'red', 'green'];
const KINDS = new Set(['moves', 'guess', 'count', 'hanging', 'choice', 'square']);
const SQUARE_RE = /^[a-h][1-8]$/;
const REPLY_DELAY_MS = 550;
const GUESS_REVEAL_MS = 650;
const GUESS_REPLY_MS = 1100;
const GUESS_EXACT = 3;
const PIECE_VALUES = { p: 1, n: 3, b: 3, r: 5, q: 9 };
const PIECE_ORDER = ['q', 'r', 'b', 'n', 'p'];

/** Normalize a step's task: known kind, sane fields. Returns null when unusable. */
function normalizeTask(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const kind = KINDS.has(raw.kind) ? raw.kind : (raw.kind ? null : 'moves');
  if (!kind) return null;
  const task = { ...raw, kind };
  const squares = (Array.isArray(raw.squares) ? raw.squares : []).map((s) => String(s).toLowerCase()).filter((s) => SQUARE_RE.test(s));
  switch (kind) {
    case 'moves': case 'guess':
      return Array.isArray(raw.solution) && raw.solution.length ? { ...task, notes: Array.isArray(raw.notes) ? raw.notes : [] } : null;
    case 'count': {
      const choices = (Array.isArray(raw.choices) ? raw.choices : []).filter(Number.isInteger);
      return Number.isInteger(raw.answer) && choices.includes(raw.answer) ? { ...task, choices } : null;
    }
    case 'hanging': case 'square':
      return squares.length ? { ...task, squares } : null;
    case 'choice': {
      const options = (Array.isArray(raw.options) ? raw.options : []).filter((o) => o && typeof o.text === 'string' && o.text);
      return options.length >= 2 && options.some((o) => o.correct) ? { ...task, options } : null;
    }
    default: return null;
  }
}

/** Resolve each step's starting FEN/orientation (steps without fen continue from the previous step's end). */
function prepareSteps(steps) {
  const out = [];
  let prevEnd = START_FEN;
  let prevOrient = 'white';
  for (const s of steps) {
    const fen = s && s.fen && chessAt(s.fen) ? chessAt(s.fen).fen() : prevEnd;
    const orientation = s && (s.orientation === 'black' || s.orientation === 'white') ? s.orientation : prevOrient;
    const task = normalizeTask(s && s.task);
    let end = fen;
    if (task && (task.kind === 'moves' || task.kind === 'guess')) {
      const line = playLine(fen, task.solution);
      end = line.fens[line.fens.length - 1];
    }
    out.push({ ...s, fen, orientation, task, endFen: end });
    prevEnd = end;
    prevOrient = orientation;
  }
  return out;
}

/** Score (white POV) → comparable centipawns. Mates dwarf any material score. */
function scoreCp(score) {
  if (!score || typeof score !== 'object') return null;
  if (Number.isFinite(score.cp)) return score.cp;
  if (Number.isFinite(score.mate)) {
    const n = score.mate;
    if (n === 0) return null;
    return Math.sign(n) * (100000 - Math.abs(n) * 100);
  }
  return null;
}

/** Material per side: { w: {q,r,b,n,p,total}, b: {...} }. */
function materialOf(fen) {
  const out = { w: { q: 0, r: 0, b: 0, n: 0, p: 0, total: 0 }, b: { q: 0, r: 0, b: 0, n: 0, p: 0, total: 0 } };
  const board = String(fen || '').split(' ')[0];
  for (const ch of board) {
    const lower = ch.toLowerCase();
    if (!PIECE_VALUES[lower]) continue;
    const side = ch === lower ? out.b : out.w;
    side[lower]++;
    side.total += PIECE_VALUES[lower];
  }
  return out;
}

function balanceLabel(n) {
  if (n > 0) return t('lesson.count.white', { n });
  if (n < 0) return t('lesson.count.black', { n: -n });
  return t('lesson.count.equal');
}

export async function mount(root, { params = {} } = {}) {
  ensureLearnCss();
  const bag = disposables();
  const timers = timerSet();
  bag.add(() => timers.clear());
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  let stepCtrl = null; // per-step requests (engine checks); aborted when the step changes
  bag.add(() => { if (stepCtrl) stepCtrl.abort(); });

  const courseId = params.courseId;
  const lessonId = params.lessonId;
  const courseHref = `#/learn/${encodeURIComponent(courseId)}`;

  root.appendChild(loadingBlock(t('lesson.loading')));

  let course;
  try {
    course = await api.get(`/api/courses/${encodeURIComponent(courseId)}`, { signal: ctrl.signal });
  } catch (e) {
    if (isAbort(e) || bag.disposed) return () => bag.dispose();
    root.replaceChildren(h('div', { class: 'page' }, e && e.status === 404
      ? emptyState({ emoji: '🔍', title: t('learn.course.notFound'), text: t('learn.course.notFoundText'), action: { label: t('learn.error.backToLearn'), href: '#/learn' } })
      : errorBlock(e && e.message)));
    return () => bag.dispose();
  }
  if (bag.disposed) return () => bag.dispose();

  const lessons = Array.isArray(course.lessons) ? course.lessons : [];
  const lessonIdx = lessons.findIndex((l) => l.id === lessonId);
  const lesson = lessons[lessonIdx];
  if (!lesson) {
    root.replaceChildren(h('div', { class: 'page' }, emptyState({ emoji: '🔍', title: t('lesson.notFound'), text: t('lesson.notFoundText'), action: { label: t('lesson.backToCourse'), href: courseHref } })));
    return () => bag.dispose();
  }
  const nextLesson = lessons[lessonIdx + 1] || null;
  const steps = prepareSteps(Array.isArray(lesson.steps) && lesson.steps.length ? lesson.steps : [{ text: lesson.summary || t('lesson.noSteps') }]);
  rememberLesson(course.id, lesson.id, 0);

  // -------------------------------------------------------------------------
  // Layout
  // -------------------------------------------------------------------------
  const cat = CATEGORIES.find((c) => c.key === course.category);
  const stepCount = h('span', { class: 'lrn-step-count' });
  const progressBar = h('div', { class: 'progress-bar', style: 'width:0%' });
  const closeBtn = h('a', { class: 'btn btn-ghost btn-icon', href: courseHref, 'aria-label': t('lesson.closeAria'), 'data-tooltip': t('lesson.backToCourse'), html: icon('close') });
  const top = h('div', { class: 'lrn-player-top' },
    breadcrumbs([{ label: t('learn.title'), href: '#/learn' }, { label: course.title, href: courseHref }, { label: lesson.title }]),
    stepCount,
    h('div', { class: 'progress progress-sm', role: 'progressbar', 'aria-label': t('lesson.progressAria') }, progressBar),
    closeBtn);

  const boardSlot = h('div', { class: 'board-slot' });
  const boardWrap = h('div', { class: 'board-row lrn-board-wrap' }, boardSlot);

  const textEl = h('div', { class: 'lrn-text md' });
  const taskEl = h('div', { class: 'lrn-task', hidden: true });
  const feedbackEl = h('div', { class: 'lrn-feedback', role: 'status', 'aria-live': 'polite' });
  const body = h('div', { class: 'panel-body' },
    h('div', { class: 'lrn-coach' }, h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'), textEl),
    taskEl, feedbackEl);

  const backBtn = h('button', { class: 'btn btn-secondary', type: 'button', html: icon('chevron-left') + `<span>${escapeHtml(t('lesson.back'))}</span>`, onClick: () => go(-1) });
  const hintBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('hint') + `<span>${escapeHtml(t('lesson.hint'))}</span>`, onClick: () => showHint() });
  const retryBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + `<span>${escapeHtml(t('lesson.retry'))}</span>`, onClick: () => enterStep(cur) });
  const nextBtn = h('button', { class: 'btn btn-primary', type: 'button', html: `<span>${escapeHtml(t('lesson.next'))}</span>` + icon('chevron-right'), onClick: () => go(1) });
  const footer = h('div', { class: 'panel-footer lrn-nav' }, backBtn, hintBtn, retryBtn, nextBtn);

  const panelHeader = h('div', { class: 'panel-header' },
    h('span', { 'aria-hidden': 'true' }, course.icon || (cat ? cat.emoji : '♟️')),
    h('span', { class: 'truncate' }, lesson.title));
  const panel = h('div', { class: 'panel grow' }, panelHeader, body, footer);

  const layout = h('div', { class: 'game-layout no-eval lrn-player' },
    h('div', { class: 'game-main' }, boardWrap),
    h('aside', { class: 'game-panel' }, panel));

  root.replaceChildren(top, layout);

  const board = new Board(boardSlot, {
    fen: steps[0].fen,
    orientation: steps[0].orientation,
    interactive: false,
    movableColor: null,
    onMove: (mv) => handleMove(mv),
    onSquareClick: (sq) => handleSquareClick(sq),
  });
  bag.add(() => board.destroy());

  // -------------------------------------------------------------------------
  // State
  // -------------------------------------------------------------------------
  let cur = 0;
  let curFen = steps[0].fen;
  let idx = 0;            // index into task.solution (moves / guess) or task.squares (square)
  let busy = false;       // reply animation / engine check in progress
  let solved = false;
  let hintStage = 0;
  let completed = false;
  let kindState = {};     // per-kind scratch state, reset on every enterStep
  /** Guess scores per step index: { score, max } (kept across steps for the finish card). */
  const guessScores = new Map();

  function stepArrows(step) {
    return (Array.isArray(step.arrows) ? step.arrows : [])
      .filter((a) => a && typeof a.from === 'string' && typeof a.to === 'string')
      .map((a) => ({ from: a.from, to: a.to, color: ARROW_COLORS.has(a.color) ? a.color : 'green' }));
  }
  function stepHighlights(step) {
    return (Array.isArray(step.highlights) ? step.highlights : [])
      .filter((s) => typeof s === 'string' && SQUARE_RE.test(s))
      .map((square) => ({ square, kind: 'hint' }));
  }

  const kindLabel = (kind) => t(`lesson.kinds.${kind}`);
  const kindIcon = { moves: 'target', guess: 'star', count: 'chart', hanging: 'eye', choice: 'list', square: 'grid' };

  function taskHeader(step, extra = '') {
    const side = sideToMove(step.fen);
    const turn = step.task.kind === 'moves' || step.task.kind === 'guess'
      ? `<span class="lrn-turn ${side}" aria-hidden="true"></span><span>${escapeHtml(t(side === 'white' ? 'lesson.whiteToMove' : 'lesson.blackToMove'))}</span>`
      : '';
    const label = step.task.kind === 'moves' ? t('lesson.yourTurn') : kindLabel(step.task.kind);
    return `<div class="lrn-task-label">${icon(kindIcon[step.task.kind] || 'target')}<span>${escapeHtml(label)}</span>${turn}${extra}</div>`;
  }

  function enterStep(i) {
    timers.clear();
    if (stepCtrl) stepCtrl.abort();
    stepCtrl = new AbortController();
    cur = Math.max(0, Math.min(steps.length - 1, i));
    const step = steps[cur];
    completed = false;
    idx = 0; busy = false; solved = false; hintStage = 0;
    kindState = {};
    curFen = step.fen;

    if (board.orientation !== step.orientation) board.setOrientation(step.orientation);
    board.setPosition(step.fen, { animate: true });
    board.setArrows(stepArrows(step));
    board.setHighlights(stepHighlights(step));
    board.clearBadges();
    boardSlot.classList.toggle('lrn-blind', !!(step.task && step.task.kind === 'square' && step.task.blind));

    textEl.innerHTML = mdLite(step.text || '');
    setFeedback(feedbackEl, null);
    taskEl.className = 'lrn-task';

    const task = step.task;
    if (task) {
      taskEl.hidden = false;
      taskEl.classList.add(`lrn-task-${task.kind}`);
      const interactiveBoard = task.kind === 'moves' || task.kind === 'guess';
      board.setInteractive(interactiveBoard, interactiveBoard ? sideToMove(step.fen) : null);
      ({ moves: renderMovesTask, guess: renderGuessTask, count: renderCountTask, hanging: renderHangingTask, choice: renderChoiceTask, square: renderSquareTask })[task.kind](step);
    } else {
      taskEl.hidden = true;
      taskEl.replaceChildren();
      board.setInteractive(false, null);
    }

    rememberLesson(course.id, lesson.id, cur);
    syncChrome();
    body.scrollTop = 0;
  }

  function syncChrome() {
    const step = steps[cur];
    const pct = ((cur + (step.task && !solved ? 0 : 1)) / steps.length) * 100;
    progressBar.style.width = `${pct.toFixed(1)}%`;
    stepCount.textContent = t('lesson.stepCount', { n: cur + 1, total: steps.length });
    backBtn.disabled = cur === 0;
    const needsSolve = !!step.task && !solved;
    const hasHint = !!step.task && (step.task.kind !== 'choice' || !!step.task.hint);
    hintBtn.hidden = !hasHint || solved;
    hintBtn.disabled = busy;
    retryBtn.hidden = !step.task || !solved;
    nextBtn.disabled = needsSolve;
    nextBtn.title = needsSolve ? t('lesson.solveToContinue') : '';
    const last = cur === steps.length - 1;
    nextBtn.innerHTML = last ? icon('check') + `<span>${escapeHtml(t('lesson.finish'))}</span>` : `<span>${escapeHtml(t('lesson.next'))}</span>` + icon('chevron-right');
    nextBtn.classList.toggle('ready', !!step.task && solved);
  }

  function go(delta) {
    if (completed) return;
    if (delta > 0) {
      const step = steps[cur];
      if (step.task && !solved) return;
      if (cur >= steps.length - 1) { finish(); return; }
      enterStep(cur + 1);
    } else if (cur > 0) {
      enterStep(cur - 1);
    }
  }

  function wrongFlash(square) {
    sfx('wrong');
    flashClass(boardSlot, 'shake', timers, 420);
    flashClass(boardWrap, 'flash-bad', timers, 650);
    if (square) board.setHighlights([...currentMarks(), { square, kind: 'bad' }]);
  }

  /** Highlights a kind wants to keep on the board (selection, found squares…). */
  function currentMarks() {
    const step = steps[cur];
    if (!step.task) return [];
    if (step.task.kind === 'hanging') return [...(kindState.selected || [])].map((square) => ({ square, kind: 'selected' }));
    return [];
  }

  function onSolved(html) {
    const step = steps[cur];
    solved = true;
    busy = false;
    board.setInteractive(false, null);
    setFeedback(feedbackEl, 'good', html || mdLite(step.task && step.task.success ? step.task.success : t('lesson.wellDone')));
    sfx('correct');
    flashClass(boardWrap, 'flash-good', timers, 900);
    confetti(boardSlot, timers, { count: 28 });
    syncChrome();
  }

  // -------------------------------------------------------------------------
  // moves: play the solution
  // -------------------------------------------------------------------------
  function renderMovesTask(step) {
    taskEl.innerHTML = taskHeader(step)
      + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || t('lesson.defaultPrompt'))}</div>`;
  }

  /** Did the user's board move reach the expected move (any mate is fine when the expected move mates)? */
  function matchesExpected(mv, expected, isLast) {
    let after = mv && mv.fen;
    if (!after && mv) {
      const c = chessAt(curFen);
      const m = c && applyUci(c, mv.uci || (mv.from + mv.to + (mv.promotion || '')));
      after = m ? c.fen() : null;
    }
    if (!after) return { after: null, ok: false };
    const mateOk = isLast && (() => {
      const e = chessAt(curFen); const em = e && applyUci(e, expected);
      if (!em || !e.isCheckmate()) return false;
      const a = chessAt(after); return !!a && a.isCheckmate();
    })();
    return { after, ok: mateOk || isSameMove(curFen, expected, after) };
  }

  function handleMove(mv) {
    const step = steps[cur];
    if (!step.task || solved || busy || completed) return false;
    if (step.task.kind === 'guess') return handleGuess(mv);
    if (step.task.kind !== 'moves') return false;
    const sol = step.task.solution;
    const expected = sol[idx];
    const { after, ok } = matchesExpected(mv, expected, idx === sol.length - 1);
    if (ok) {
      curFen = after;
      idx++;
      hintStage = 0;
      board.clearArrows();
      board.setHighlights(mv.to ? [{ square: mv.to, kind: 'good' }] : []);
      board.clearBadges();
      if (mv.to) board.setBadge(mv.to, 'best');
      if (idx >= sol.length) {
        board.clearArrows();
        onSolved();
      } else {
        busy = true;
        board.setInteractive(false, null);
        setFeedback(feedbackEl, 'good', escapeHtml(t('lesson.correctWatch')));
        sfx('correct');
        timers.later(playReply, REPLY_DELAY_MS);
      }
      return true;
    }
    wrongFlash(null);
    if (mv && mv.to) {
      board.setHighlights([{ square: mv.to, kind: 'bad' }]);
      timers.later(() => { if (!solved) board.setHighlights(idx === 0 ? stepHighlights(step) : []); }, 650);
    }
    setFeedback(feedbackEl, 'bad', t('lesson.wrong'));
    return false;
  }

  /** Auto-play the scripted reply at `idx`. Returns the chess.js move or null. */
  function autoReply() {
    const step = steps[cur];
    const c = chessAt(curFen);
    const m = c && applyUci(c, step.task.solution[idx]);
    if (!m) return null;
    curFen = c.fen();
    idx++;
    board.setPosition(curFen, { animate: true, lastMove: [m.from, m.to] });
    return m;
  }

  function playReply() {
    const step = steps[cur];
    if (!step.task || completed) return;
    const m = autoReply();
    if (!m) { board.clearArrows(); onSolved(); return; } // malformed data: don't trap the user
    board.setHighlights([]);
    board.clearBadges();
    busy = false;
    if (idx >= step.task.solution.length) { onSolved(); return; }
    board.setInteractive(true, sideToMove(curFen));
    setFeedback(feedbackEl, 'info', t(m.color === 'w' ? 'lesson.whiteReplied' : 'lesson.blackReplied', { san: escapeHtml(m.san) }));
  }

  // -------------------------------------------------------------------------
  // guess: guess the master's moves
  // -------------------------------------------------------------------------
  function renderGuessTask(step) {
    const learnerMoves = Math.ceil(step.task.solution.length / 2);
    kindState = { score: 0, max: learnerMoves * GUESS_EXACT, n: 0, total: learnerMoves };
    const scoreEl = h('span', { class: 'lrn-guess-score tabular' });
    const log = h('ol', { class: 'lrn-guess-log', 'aria-label': t('lesson.guess.logAria') });
    kindState.scoreEl = scoreEl;
    kindState.log = log;
    taskEl.innerHTML = taskHeader(step);
    if (step.task.game) {
      taskEl.appendChild(h('div', { class: 'lrn-guess-game', html: icon('book', { size: 14 }) + `<span>${escapeHtml(step.task.game)}</span>` }));
    }
    taskEl.appendChild(h('div', { class: 'lrn-task-prompt md', html: mdLite(step.task.prompt || t('lesson.guess.defaultPrompt')) }));
    taskEl.appendChild(h('div', { class: 'lrn-guess-bar' },
      h('span', { class: 'lrn-guess-progress' }), scoreEl));
    taskEl.appendChild(log);
    updateGuessBar();
  }

  function updateGuessBar() {
    const bar = taskEl.querySelector('.lrn-guess-progress');
    if (bar) bar.textContent = t('lesson.guess.progress', { n: Math.min(kindState.n + 1, kindState.total), total: kindState.total });
    if (kindState.scoreEl) kindState.scoreEl.innerHTML = icon('star', { size: 14 }) + `<span>${escapeHtml(t('lesson.guess.score', { score: kindState.score, max: kindState.max }))}</span>`;
  }

  /** Evaluate the position after a move, from the mover's point of view (null = unknown). */
  async function evalAfter(fenAfter, moverWhite, signal) {
    const c = chessAt(fenAfter);
    if (!c) return null;
    if (c.isCheckmate()) return 100000;
    if (c.isDraw()) return 0;
    const info = await api.post('/api/engine/analyze', { fen: fenAfter, movetime_ms: 700, depth: 14 }, { signal });
    const line = info && Array.isArray(info.lines) ? info.lines[0] : null;
    const cp = line ? scoreCp(line.score) : null;
    if (cp === null) return null;
    return moverWhite ? cp : -cp;
  }

  function handleGuess(mv) {
    const step = steps[cur];
    const sol = step.task.solution;
    const expected = sol[idx];
    const { after, ok } = matchesExpected(mv, expected, idx === sol.length - 1);
    if (!after) return false;
    const before = curFen;
    const cap = hintStage >= 2 ? 1 : hintStage === 1 ? 2 : GUESS_EXACT;
    const note = step.task.notes[Math.floor(idx / 2)] || '';
    const guessSan = (() => { const c = chessAt(before); const m = c && applyUci(c, mv.uci || (mv.from + mv.to + (mv.promotion || ''))); return m ? m.san : ''; })();
    const master = (() => { const c = chessAt(before); const m = c && applyUci(c, expected); return m ? { san: m.san, from: m.from, to: m.to, fen: c.fen() } : null; })();
    if (!master) { onSolved(); return false; } // malformed data: don't trap the user
    busy = true;
    board.setInteractive(false, null);
    syncChrome();

    if (ok) {
      curFen = after;
      board.clearArrows();
      board.setHighlights([{ square: mv.to, kind: 'good' }]);
      board.clearBadges();
      board.setBadge(mv.to, 'best');
      sfx('correct');
      recordGuess({ guessSan, master, pts: cap, exact: true, note });
      return true;
    }

    // Not the master's move: keep the learner's move visible while the engine compares both.
    const moverWhite = sideToMove(before) === 'white';
    board.setArrows([{ from: mv.from, to: mv.to, color: 'blue' }]);
    setFeedback(feedbackEl, 'info', `<span class="lrn-spinner" aria-hidden="true"></span>${escapeHtml(t('lesson.guess.checking', { san: guessSan }))}`);
    const signal = stepCtrl.signal;
    const stepAt = cur;
    Promise.all([evalAfter(after, moverWhite, signal), evalAfter(master.fen, moverWhite, signal)])
      .then(([mine, theirs]) => {
        if (bag.disposed || cur !== stepAt || signal.aborted) return;
        let pts = 0;
        let quality = 'miss';
        if (mine === null || theirs === null) quality = 'unknown';
        else {
          const loss = Math.max(0, theirs - mine);
          if (loss <= 30) { pts = 2; quality = 'close'; }
          else if (loss <= 90) { pts = 1; quality = 'ok'; }
        }
        revealMaster({ guessSan, master, pts: Math.min(pts, cap), quality, note });
      })
      .catch((e) => {
        if (isAbort(e) || bag.disposed || cur !== stepAt) return;
        revealMaster({ guessSan, master, pts: 0, quality: 'unknown', note });
      });
    return false; // the board snaps the learner's piece back; the master's move is animated next
  }

  function revealMaster({ guessSan, master, pts, quality, note }) {
    if (pts > 0) sfx('correct'); else sfx('wrong');
    timers.later(() => {
      curFen = master.fen;
      board.setPosition(curFen, { animate: true, lastMove: [master.from, master.to] });
      board.setArrows([{ from: master.from, to: master.to, color: 'green' }]);
      board.setHighlights([]);
      recordGuess({ guessSan, master, pts, exact: false, quality, note });
    }, GUESS_REVEAL_MS);
  }

  function recordGuess({ guessSan, master, pts, exact, quality, note }) {
    const step = steps[cur];
    kindState.score += pts;
    kindState.n++;
    idx++;
    hintStage = 0;
    const moveNo = kindState.log.children.length + 1;
    const ptsLabel = t('lesson.guess.points', { count: pts });
    kindState.log.appendChild(h('li', { class: ['lrn-guess-row', exact ? 'exact' : pts > 0 ? 'partial' : 'miss'] },
      h('span', { class: 'lrn-guess-no' }, String(moveNo)),
      h('span', { class: 'lrn-guess-move' }, master.san),
      exact ? h('span', { class: 'lrn-guess-you', html: icon('check', { size: 14 }) })
        : h('span', { class: 'lrn-guess-you' }, t('lesson.guess.you', { san: guessSan })),
      h('span', { class: 'lrn-guess-pts' }, ptsLabel)));
    kindState.log.scrollTop = kindState.log.scrollHeight;
    updateGuessBar();

    const head = exact
      ? t('lesson.guess.exact', { san: escapeHtml(master.san), pts: escapeHtml(ptsLabel) })
      : t(`lesson.guess.${quality}`, { san: escapeHtml(guessSan), master: escapeHtml(master.san), pts: escapeHtml(ptsLabel) });
    const html = `<p>${head}</p>${note ? mdLite(note) : ''}`;
    const kind = exact ? 'good' : pts > 0 ? 'info' : 'warn';

    if (idx >= step.task.solution.length) { finishGuess(html); return; }
    setFeedback(feedbackEl, kind, html);
    // Opponent's reply, then the learner guesses again.
    timers.later(() => {
      if (completed) return;
      const m = autoReply();
      board.clearArrows();
      board.setHighlights([]);
      board.clearBadges();
      if (!m || idx >= step.task.solution.length) { finishGuess(html); return; }
      busy = false;
      board.setInteractive(true, sideToMove(curFen));
      setFeedback(feedbackEl, kind, html + `<p class="lrn-guess-next">${t(m.color === 'w' ? 'lesson.guess.whiteReplied' : 'lesson.guess.blackReplied', { san: escapeHtml(m.san) })}</p>`);
      updateGuessBar();
      syncChrome();
    }, exact ? GUESS_REPLY_MS - 400 : GUESS_REPLY_MS);
  }

  function finishGuess(lastHtml) {
    const step = steps[cur];
    guessScores.set(cur, { score: kindState.score, max: kindState.max });
    const pct = kindState.max ? kindState.score / kindState.max : 0;
    const verdict = pct >= 0.85 ? 'great' : pct >= 0.5 ? 'good' : 'keep';
    const summary = `<div class="lrn-guess-summary">${icon('trophy', { size: 18 })}<strong>${escapeHtml(t('lesson.guess.final', { score: kindState.score, max: kindState.max }))}</strong><span>${escapeHtml(t(`lesson.guess.verdict.${verdict}`))}</span></div>`;
    onSolved(lastHtml + summary + (step.task.success ? mdLite(step.task.success) : ''));
  }

  // -------------------------------------------------------------------------
  // count: material balance
  // -------------------------------------------------------------------------
  function renderCountTask(step) {
    const answers = [...step.task.choices].sort((a, b) => a - b);
    const grid = h('div', { class: 'lrn-answer-grid', role: 'group', 'aria-label': t('lesson.count.aria') },
      answers.map((n, i) => h('button', {
        class: 'btn btn-secondary lrn-answer', type: 'button', 'data-value': String(n), 'data-key': String(i + 1),
        onClick: (e) => answerCount(step, n, e.currentTarget),
      }, h('span', { class: 'lrn-answer-key', 'aria-hidden': 'true' }, String(i + 1)), balanceLabel(n))));
    taskEl.innerHTML = taskHeader(step) + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || t('lesson.count.defaultPrompt'))}</div>`;
    taskEl.appendChild(grid);
    kindState.grid = grid;
  }

  function materialTable(fen) {
    const mat = materialOf(fen);
    const row = (color) => {
      const side = mat[color];
      const pieces = PIECE_ORDER.filter((p) => side[p] > 0).map((p) => h('span', { class: 'lrn-mat-piece' },
        h('img', { src: pieceUrl(color + p.toUpperCase()), alt: '', width: 22, height: 22 }),
        h('span', { class: 'tabular' }, `×${side[p]}`)));
      return h('div', { class: 'lrn-mat-row' },
        h('span', { class: `lrn-turn ${color === 'w' ? 'white' : 'black'}`, 'aria-hidden': 'true' }),
        h('span', { class: 'lrn-mat-side' }, t(color === 'w' ? 'lesson.count.whiteSide' : 'lesson.count.blackSide')),
        h('span', { class: 'lrn-mat-pieces' }, pieces),
        h('strong', { class: 'lrn-mat-total tabular' }, t('lesson.count.points', { count: side.total })));
    };
    return h('div', { class: 'lrn-mat' }, row('w'), row('b'));
  }

  function answerCount(step, n, btn) {
    if (solved || completed) return;
    if (n === step.task.answer) {
      btn.classList.add('correct');
      for (const b of kindState.grid.querySelectorAll('button')) b.disabled = true;
      onSolved(mdLite(step.task.success || t('lesson.wellDone')));
      feedbackEl.querySelector('div')?.appendChild(materialTable(step.fen));
    } else {
      btn.classList.add('wrong');
      btn.disabled = true;
      wrongFlash(null);
      setFeedback(feedbackEl, 'bad', escapeHtml(t('lesson.count.wrong')));
    }
  }

  // -------------------------------------------------------------------------
  // hanging: click every hanging piece, then Check
  // -------------------------------------------------------------------------
  function renderHangingTask(step) {
    kindState = { selected: new Set(), found: new Set() };
    const counter = h('span', { class: 'lrn-hang-count muted text-sm' });
    const checkBtn = h('button', { class: 'btn btn-primary btn-sm', type: 'button', html: icon('check') + `<span>${escapeHtml(t('lesson.hanging.check'))}</span>`, onClick: () => checkHanging() });
    kindState.counter = counter;
    kindState.checkBtn = checkBtn;
    taskEl.innerHTML = taskHeader(step) + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || t('lesson.hanging.defaultPrompt'))}</div>`;
    taskEl.appendChild(h('div', { class: 'lrn-hang-bar' }, counter, checkBtn));
    syncHanging();
  }

  function syncHanging() {
    const n = kindState.selected.size;
    kindState.counter.textContent = t('lesson.hanging.selected', { count: n });
    kindState.checkBtn.disabled = n === 0 || solved;
    board.setHighlights([...currentMarks(), ...[...kindState.found].map((square) => ({ square, kind: 'good' }))]);
  }

  function hasPiece(sq) {
    const c = chessAt(curFen);
    return !!(c && c.get(sq));
  }

  function checkHanging() {
    const step = steps[cur];
    if (solved || busy) return;
    const want = new Set(step.task.squares);
    const picked = [...kindState.selected];
    const wrong = picked.filter((s) => !want.has(s));
    const right = picked.filter((s) => want.has(s));
    const missing = [...want].filter((s) => !kindState.selected.has(s));
    if (!wrong.length && !missing.length) {
      board.setHighlights(picked.map((square) => ({ square, kind: 'good' })));
      kindState.checkBtn.disabled = true;
      onSolved();
      return;
    }
    for (const s of right) kindState.found.add(s);
    const parts = [];
    if (wrong.length) parts.push(escapeHtml(t('lesson.hanging.wrongPicks', { count: wrong.length })));
    if (missing.length) parts.push(escapeHtml(t('lesson.hanging.missing', { count: missing.length })));
    setFeedback(feedbackEl, wrong.length ? 'bad' : 'warn', parts.join(' '));
    sfx('wrong');
    flashClass(boardWrap, 'flash-bad', timers, 650);
    board.setHighlights([...right.map((square) => ({ square, kind: 'good' })), ...wrong.map((square) => ({ square, kind: 'bad' }))]);
    busy = true;
    syncChrome();
    timers.later(() => {
      busy = false;
      for (const s of wrong) kindState.selected.delete(s);
      syncHanging();
      syncChrome();
    }, 900);
  }

  // -------------------------------------------------------------------------
  // square: coordinate quiz
  // -------------------------------------------------------------------------
  function renderSquareTask(step) {
    kindState = { started: 0, misses: 0 };
    taskEl.innerHTML = taskHeader(step, step.task.blind ? `<span class="badge lrn-blind-badge">${icon('eye-off', { size: 12 })}${escapeHtml(t('lesson.square.blind'))}</span>` : '')
      + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || t('lesson.square.defaultPrompt'))}</div>`;
    const target = h('div', { class: 'lrn-square-target', 'aria-live': 'polite' });
    const dots = h('div', { class: 'lrn-square-dots', 'aria-hidden': 'true' }, step.task.squares.map(() => h('i')));
    taskEl.appendChild(h('div', { class: 'lrn-square-row' }, target, dots));
    kindState.target = target;
    kindState.dots = dots;
    syncSquare();
  }

  function syncSquare() {
    const step = steps[cur];
    const sq = step.task.squares[idx];
    if (sq) kindState.target.innerHTML = `<span class="lrn-square-label">${escapeHtml(t('lesson.square.find'))}</span><span class="lrn-square-name">${escapeHtml(sq)}</span>`;
    [...kindState.dots.children].forEach((d, i) => { d.className = i < idx ? 'on' : i === idx ? 'cur' : ''; });
  }

  // -------------------------------------------------------------------------
  // Square clicks (hanging + square)
  // -------------------------------------------------------------------------
  function handleSquareClick(sq) {
    const step = steps[cur];
    if (!step || !step.task || solved || completed || busy) return;
    if (step.task.kind === 'hanging') {
      if (!hasPiece(sq) && !kindState.selected.has(sq)) return;
      if (kindState.selected.has(sq)) { kindState.selected.delete(sq); kindState.found.delete(sq); } else kindState.selected.add(sq);
      setFeedback(feedbackEl, null);
      syncHanging();
      return;
    }
    if (step.task.kind !== 'square') return;
    if (!kindState.started) kindState.started = performance.now();
    const target = step.task.squares[idx];
    if (sq === target) {
      idx++;
      hintStage = 0;
      board.setHighlights([{ square: sq, kind: 'good' }]);
      timers.later(() => { if (!solved && steps[cur] === step) board.setHighlights([]); }, 500);
      if (idx >= step.task.squares.length) {
        syncSquare();
        const secs = (performance.now() - kindState.started) / 1000;
        const stats = escapeHtml(t('lesson.square.done', { count: step.task.squares.length, secs: formatNumber(Math.max(0.1, secs), { maximumFractionDigits: 1 }) }));
        kindState.target.innerHTML = `<span class="lrn-square-label">${icon('check-circle', { size: 18 })}</span><span class="lrn-square-name">${stats}</span>`;
        onSolved(mdLite(step.task.success || t('lesson.wellDone')) + `<p class="text-sm muted">${stats}</p>`);
        return;
      }
      sfx('correct');
      setFeedback(feedbackEl, null);
      syncSquare();
    } else {
      kindState.misses++;
      wrongFlash(sq);
      timers.later(() => { if (!solved && steps[cur] === step) board.setHighlights([]); }, 650);
      setFeedback(feedbackEl, 'bad', t('lesson.square.wrong', { clicked: escapeHtml(sq), sq: escapeHtml(target) }));
    }
  }

  // -------------------------------------------------------------------------
  // choice: multiple choice with arrows
  // -------------------------------------------------------------------------
  function optionArrows(opt, i, colorOverride) {
    return (Array.isArray(opt.arrows) ? opt.arrows : [])
      .filter((a) => a && SQUARE_RE.test(a.from) && SQUARE_RE.test(a.to))
      .map((a) => ({ from: a.from, to: a.to, color: colorOverride || OPTION_COLORS[i % OPTION_COLORS.length] }));
  }

  function renderChoiceTask(step) {
    const opts = step.task.options;
    const allArrows = () => opts.flatMap((o, i) => optionArrows(o, i));
    kindState = { allArrows };
    board.setArrows([...stepArrows(step), ...allArrows()]);
    const list = h('div', { class: 'lrn-options', role: 'group', 'aria-label': t('lesson.choice.aria') },
      opts.map((o, i) => {
        const color = OPTION_COLORS[i % OPTION_COLORS.length];
        const btn = h('button', {
          class: 'lrn-option', type: 'button', 'data-key': String(i + 1),
          onClick: () => answerChoice(step, i, btn),
          onMouseenter: () => previewOption(i), onMouseleave: () => previewOption(-1),
          onFocus: () => previewOption(i), onBlur: () => previewOption(-1),
        },
        h('span', { class: `lrn-option-key arrow-${color}`, 'aria-hidden': 'true' }, String.fromCharCode(65 + i)),
        h('span', { class: 'lrn-option-text md', html: mdLite(o.text) }));
        return btn;
      }));
    taskEl.innerHTML = taskHeader(step) + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || t('lesson.choice.defaultPrompt'))}</div>`;
    taskEl.appendChild(list);
    taskEl.appendChild(h('div', { class: 'lrn-choice-tip muted text-sm' }, t('lesson.choice.tip')));
    kindState.list = list;
  }

  function previewOption(i) {
    const step = steps[cur];
    if (!step.task || step.task.kind !== 'choice' || solved) return;
    const opts = step.task.options;
    board.setArrows(i < 0 ? [...stepArrows(step), ...kindState.allArrows()] : optionArrows(opts[i], i));
  }

  function answerChoice(step, i, btn) {
    if (solved || completed) return;
    const opt = step.task.options[i];
    const explain = opt.explain ? mdLite(opt.explain) : '';
    if (opt.correct) {
      btn.classList.add('correct');
      for (const b of kindState.list.querySelectorAll('button')) b.disabled = true;
      board.setArrows(optionArrows(opt, i, 'green'));
      onSolved(explain + (step.task.success ? mdLite(step.task.success) : ''));
    } else {
      btn.classList.add('wrong');
      btn.disabled = true;
      board.setArrows(optionArrows(opt, i, 'red'));
      wrongFlash(null);
      setFeedback(feedbackEl, 'bad', explain + `<p>${escapeHtml(t('lesson.choice.tryAgain'))}</p>`);
    }
  }

  // -------------------------------------------------------------------------
  // Hints
  // -------------------------------------------------------------------------
  function showHint() {
    const step = steps[cur];
    if (!step.task || solved || busy) return;
    const task = step.task;
    const authored = task.hint && hintStage === 0 && (idx === 0 || task.kind !== 'moves') ? mdLite(task.hint) : null;
    switch (task.kind) {
      case 'moves': case 'guess': {
        const expected = task.solution[idx];
        if (!expected) return;
        const from = expected.slice(0, 2); const to = expected.slice(2, 4);
        if (hintStage === 0) {
          hintStage = 1;
          board.setHighlights([{ square: from, kind: 'hint' }]);
          setFeedback(feedbackEl, 'warn', (authored || escapeHtml(t('lesson.hintPiece'))) + (task.kind === 'guess' ? `<p class="text-sm">${escapeHtml(t('lesson.guess.hintCost'))}</p>` : ''));
        } else {
          hintStage = 2;
          board.setHighlights([{ square: from, kind: 'hint' }]);
          board.setArrows([{ from, to, color: 'green' }]);
          setFeedback(feedbackEl, 'warn', escapeHtml(t('lesson.hintArrow')));
        }
        return;
      }
      case 'count':
        hintStage = 1;
        setFeedback(feedbackEl, 'warn', authored || escapeHtml(t('lesson.count.hint')));
        return;
      case 'hanging': {
        if (hintStage === 0) {
          hintStage = 1;
          setFeedback(feedbackEl, 'warn', authored || escapeHtml(t('lesson.hanging.hint')));
        } else {
          const missing = task.squares.find((s) => !kindState.selected.has(s));
          if (!missing) return;
          board.setHighlights([...currentMarks(), { square: missing, kind: 'hint' }]);
          setFeedback(feedbackEl, 'warn', escapeHtml(t('lesson.hanging.hintSquare')));
        }
        return;
      }
      case 'square': {
        const target = task.squares[idx];
        if (!target) return;
        if (hintStage === 0) {
          hintStage = 1;
          setFeedback(feedbackEl, 'warn', authored || escapeHtml(t('lesson.square.hint', { file: target[0], rank: target[1] })));
        } else {
          hintStage = 2;
          board.setHighlights([{ square: target, kind: 'hint' }]);
          setFeedback(feedbackEl, 'warn', escapeHtml(t('lesson.square.hintShown')));
        }
        return;
      }
      case 'choice':
        if (task.hint) setFeedback(feedbackEl, 'warn', mdLite(task.hint));
        return;
      default:
    }
  }

  // -------------------------------------------------------------------------
  // Finish
  // -------------------------------------------------------------------------
  async function finish() {
    if (completed) return;
    completed = true;
    timers.clear();
    if (stepCtrl) stepCtrl.abort();
    board.setInteractive(false, null);
    progressBar.style.width = '100%';
    rememberLesson(course.id, lesson.id, 0);

    let score = 0; let max = 0;
    for (const v of guessScores.values()) { score += v.score; max += v.max; }

    const nextHref = nextLesson ? `#/learn/${encodeURIComponent(course.id)}/${encodeURIComponent(nextLesson.id)}` : courseHref;
    const status = h('p', { class: 'text-sm subtle' }, t('lesson.saving'));
    const card = h('div', { class: 'lrn-complete' },
      h('div', { class: ['lrn-complete-badge', max && score / max >= 0.85 && 'gold'], html: icon(max ? 'trophy' : 'check') }),
      h('h2', null, t('lesson.complete')),
      max ? h('div', { class: 'lrn-guess-total', html: icon('star', { size: 16 }) + `<span>${escapeHtml(t('lesson.guess.lessonScore', { score, max }))}</span>` }) : null,
      h('p', null, nextLesson ? t('lesson.upNext', { title: nextLesson.title }) : t('lesson.finishedCourse', { course: course.title })),
      status,
      h('div', { class: 'lrn-nav' },
        h('a', { class: 'btn btn-primary btn-lg', href: nextHref, html: nextLesson ? `<span>${escapeHtml(t('lesson.nextLesson'))}</span>` + icon('chevron-right') : icon('trophy') + `<span>${escapeHtml(t('lesson.backToCourse'))}</span>` }),
        nextLesson ? h('a', { class: 'btn btn-secondary', href: courseHref, html: icon('list') + `<span>${escapeHtml(t('lesson.backToCourse'))}</span>` }) : h('a', { class: 'btn btn-secondary', href: '#/learn', html: icon('learn') + `<span>${escapeHtml(t('lesson.moreCourses'))}</span>` }),
        h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + `<span>${escapeHtml(t('lesson.replay'))}</span>`, onClick: () => { completed = false; guessScores.clear(); restoreBody(); enterStep(0); } })));

    body.replaceChildren(card);
    footer.hidden = true;
    stepCount.textContent = t('lesson.completed');
    sfx('gameEnd');
    confetti(boardSlot, timers, { count: 48 });

    try {
      await api.post('/api/progress', { course_id: course.id, lesson_id: lesson.id, completed: true }, { signal: ctrl.signal });
      if (!bag.disposed) status.textContent = t('lesson.saved');
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      status.textContent = t('lesson.saveFailed');
      status.classList.add('text-danger');
    }
  }

  function restoreBody() {
    body.replaceChildren(h('div', { class: 'lrn-coach' }, h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'), textEl), taskEl, feedbackEl);
    footer.hidden = false;
  }

  // Keyboard: ← / → step through the lesson (Enter = next when ready); 1–9 pick an answer.
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const tgt = e.target;
    if (tgt && (tgt.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tgt.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'ArrowRight') { e.preventDefault(); go(1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); go(-1); }
    else if ((e.key === 'h' || e.key === 'H') && !completed) showHint();
    else if (/^[1-9]$/.test(e.key) && !completed && !solved) {
      const btn = taskEl.querySelector(`button[data-key="${e.key}"]`);
      if (btn && !btn.disabled) { e.preventDefault(); btn.click(); }
    }
  });

  enterStep(0);
  return () => bag.dispose();
}
