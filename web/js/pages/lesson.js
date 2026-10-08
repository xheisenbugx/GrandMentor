// Interactive lesson player (#/learn/:courseId/:lessonId).
// Step-by-step: coach text (markdown-lite) + board with arrows/highlights. Task steps require the user
// to play the solution moves; scripted replies are auto-played. Completion → POST /api/progress.

import { h, icon, disposables, mdLite, escapeHtml, emptyState, loadingBlock } from '../ui.js';
import { api, isAbort } from '../api.js';
import { Board } from '../components/board.js';
import {
  ensureLearnCss, START_FEN, chessAt, applyUci, isSameMove, sideToMove, playLine, timerSet,
  confetti, flashClass, sfx, setFeedback, breadcrumbs, rememberLesson, errorBlock, CATEGORIES,
} from './learn.js';

export const title = 'Lesson';

const ARROW_COLORS = new Set(['green', 'red', 'blue', 'yellow']);
const REPLY_DELAY_MS = 550;

/** Resolve each step's starting FEN/orientation (steps without fen continue from the previous step's end). */
function prepareSteps(steps) {
  const out = [];
  let prevEnd = START_FEN;
  let prevOrient = 'white';
  for (const s of steps) {
    const fen = s && s.fen && chessAt(s.fen) ? chessAt(s.fen).fen() : prevEnd;
    const orientation = s && (s.orientation === 'black' || s.orientation === 'white') ? s.orientation : prevOrient;
    const task = s && s.task && Array.isArray(s.task.solution) && s.task.solution.length ? s.task : null;
    let end = fen;
    if (task) {
      const line = playLine(fen, task.solution);
      end = line.fens[line.fens.length - 1];
    }
    out.push({ ...s, fen, orientation, task, endFen: end });
    prevEnd = end;
    prevOrient = orientation;
  }
  return out;
}

export async function mount(root, { params = {} } = {}) {
  ensureLearnCss();
  const bag = disposables();
  const timers = timerSet();
  bag.add(() => timers.clear());
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());

  const courseId = params.courseId;
  const lessonId = params.lessonId;
  const courseHref = `#/learn/${encodeURIComponent(courseId)}`;

  root.appendChild(loadingBlock('Opening your lesson…'));

  let course;
  try {
    course = await api.get(`/api/courses/${encodeURIComponent(courseId)}`, { signal: ctrl.signal });
  } catch (e) {
    if (isAbort(e) || bag.disposed) return () => bag.dispose();
    root.replaceChildren(h('div', { class: 'page' }, e && e.status === 404
      ? emptyState({ emoji: '🔍', title: 'Course not found', text: 'This course does not exist (anymore).', action: { label: 'Back to Learn', href: '#/learn' } })
      : errorBlock(e && e.message)));
    return () => bag.dispose();
  }
  if (bag.disposed) return () => bag.dispose();

  const lessons = Array.isArray(course.lessons) ? course.lessons : [];
  const lessonIdx = lessons.findIndex((l) => l.id === lessonId);
  const lesson = lessons[lessonIdx];
  if (!lesson) {
    root.replaceChildren(h('div', { class: 'page' }, emptyState({ emoji: '🔍', title: 'Lesson not found', text: 'We could not find this lesson in the course.', action: { label: 'Back to course', href: courseHref } })));
    return () => bag.dispose();
  }
  const nextLesson = lessons[lessonIdx + 1] || null;
  const steps = prepareSteps(Array.isArray(lesson.steps) && lesson.steps.length ? lesson.steps : [{ text: lesson.summary || 'This lesson has no steps yet.' }]);
  rememberLesson(course.id, lesson.id, 0);

  // -------------------------------------------------------------------------
  // Layout
  // -------------------------------------------------------------------------
  const cat = CATEGORIES.find((c) => c.key === course.category);
  const stepCount = h('span', { class: 'lrn-step-count' });
  const progressBar = h('div', { class: 'progress-bar', style: 'width:0%' });
  const closeBtn = h('a', { class: 'btn btn-ghost btn-icon', href: courseHref, 'aria-label': 'Close lesson', 'data-tooltip': 'Back to course', html: icon('close') });
  const top = h('div', { class: 'lrn-player-top' },
    breadcrumbs([{ label: 'Learn', href: '#/learn' }, { label: course.title, href: courseHref }, { label: lesson.title }]),
    stepCount,
    h('div', { class: 'progress progress-sm', role: 'progressbar', 'aria-label': 'Lesson progress' }, progressBar),
    closeBtn);

  const boardSlot = h('div', { class: 'board-slot' });
  const boardWrap = h('div', { class: 'board-row lrn-board-wrap' }, boardSlot);

  const textEl = h('div', { class: 'lrn-text md' });
  const taskEl = h('div', { class: 'lrn-task', hidden: true });
  const feedbackEl = h('div', { class: 'lrn-feedback', role: 'status', 'aria-live': 'polite' });
  const body = h('div', { class: 'panel-body' },
    h('div', { class: 'lrn-coach' }, h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'), textEl),
    taskEl, feedbackEl);

  const backBtn = h('button', { class: 'btn btn-secondary', type: 'button', html: icon('chevron-left') + '<span>Back</span>', onClick: () => go(-1) });
  const hintBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('hint') + '<span>Hint</span>', onClick: () => showHint() });
  const retryBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + '<span>Retry</span>', onClick: () => enterStep(cur) });
  const nextBtn = h('button', { class: 'btn btn-primary', type: 'button', html: '<span>Next</span>' + icon('chevron-right'), onClick: () => go(1) });
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
  });
  bag.add(() => board.destroy());

  // -------------------------------------------------------------------------
  // State
  // -------------------------------------------------------------------------
  let cur = 0;
  let curFen = steps[0].fen;
  let idx = 0;            // index into task.solution
  let busy = false;       // reply animation in progress
  let solved = false;
  let hintStage = 0;
  let completed = false;

  function stepArrows(step) {
    return (Array.isArray(step.arrows) ? step.arrows : [])
      .filter((a) => a && typeof a.from === 'string' && typeof a.to === 'string')
      .map((a) => ({ from: a.from, to: a.to, color: ARROW_COLORS.has(a.color) ? a.color : 'green' }));
  }
  function stepHighlights(step) {
    return (Array.isArray(step.highlights) ? step.highlights : [])
      .filter((s) => typeof s === 'string' && /^[a-h][1-8]$/.test(s))
      .map((square) => ({ square, kind: 'hint' }));
  }

  function enterStep(i) {
    timers.clear();
    cur = Math.max(0, Math.min(steps.length - 1, i));
    const step = steps[cur];
    completed = false;
    idx = 0; busy = false; solved = false; hintStage = 0;
    curFen = step.fen;

    if (board.orientation !== step.orientation) board.setOrientation(step.orientation);
    board.setPosition(step.fen, { animate: true });
    board.setArrows(stepArrows(step));
    board.setHighlights(stepHighlights(step));
    board.clearBadges();

    textEl.innerHTML = mdLite(step.text || '');
    setFeedback(feedbackEl, null);

    if (step.task) {
      const side = sideToMove(step.fen);
      taskEl.hidden = false;
      taskEl.innerHTML = `<div class="lrn-task-label">${icon('target')}<span>Your turn</span>`
        + `<span class="lrn-turn ${side}" aria-hidden="true"></span><span>${side === 'white' ? 'White' : 'Black'} to move</span></div>`
        + `<div class="lrn-task-prompt md">${mdLite(step.task.prompt || 'Find the best move.')}</div>`;
      board.setInteractive(true, side);
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
    stepCount.textContent = `Step ${cur + 1} of ${steps.length}`;
    backBtn.disabled = cur === 0;
    const needsSolve = !!step.task && !solved;
    hintBtn.hidden = !step.task || solved;
    retryBtn.hidden = !step.task || !solved;
    nextBtn.disabled = needsSolve;
    nextBtn.title = needsSolve ? 'Solve the task to continue' : '';
    const last = cur === steps.length - 1;
    nextBtn.innerHTML = last ? icon('check') + '<span>Finish</span>' : '<span>Next</span>' + icon('chevron-right');
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

  function handleMove(mv) {
    const step = steps[cur];
    if (!step.task || solved || busy || completed) return false;
    const sol = step.task.solution;
    const expected = sol[idx];
    let after = mv && mv.fen;
    if (!after && mv) {
      const c = chessAt(curFen);
      const m = c && applyUci(c, mv.uci || (mv.from + mv.to + (mv.promotion || '')));
      after = m ? c.fen() : null;
    }
    // Any checkmating move is accepted when the solution's final move is mate.
    const mateOk = after && idx === sol.length - 1 && (() => {
      const e = chessAt(curFen); const em = e && applyUci(e, expected);
      if (!em || !e.isCheckmate()) return false;
      const a = chessAt(after); return !!a && a.isCheckmate();
    })();
    if (after && (mateOk || isSameMove(curFen, expected, after))) {
      curFen = after;
      idx++;
      hintStage = 0;
      board.clearArrows();
      board.setHighlights(mv.to ? [{ square: mv.to, kind: 'good' }] : []);
      board.clearBadges();
      if (mv.to) board.setBadge(mv.to, 'best');
      if (idx >= sol.length) {
        onSolved();
      } else {
        busy = true;
        board.setInteractive(false, null);
        setFeedback(feedbackEl, 'good', 'Correct! Now watch the reply…');
        sfx('correct');
        timers.later(playReply, REPLY_DELAY_MS);
      }
      return true;
    }
    // Wrong move
    sfx('wrong');
    flashClass(boardSlot, 'shake', timers, 420);
    flashClass(boardWrap, 'flash-bad', timers, 650);
    if (mv && mv.to) {
      board.setHighlights([{ square: mv.to, kind: 'bad' }]);
      timers.later(() => { if (!solved) board.setHighlights(idx === 0 ? stepHighlights(step) : []); }, 650);
    }
    setFeedback(feedbackEl, 'bad', '<strong>Not quite — try again.</strong> Stuck? Press <strong>Hint</strong>.');
    return false;
  }

  function playReply() {
    const step = steps[cur];
    if (!step.task || completed) return;
    const sol = step.task.solution;
    const c = chessAt(curFen);
    const m = c && applyUci(c, sol[idx]);
    if (!m) { onSolved(); return; } // malformed data: don't trap the user
    curFen = c.fen();
    idx++;
    board.setPosition(curFen, { animate: true, lastMove: [m.from, m.to] });
    board.setHighlights([]);
    board.clearBadges();
    busy = false;
    if (idx >= sol.length) { onSolved(); return; }
    board.setInteractive(true, sideToMove(curFen));
    setFeedback(feedbackEl, 'info', `${m.color === 'w' ? 'White' : 'Black'} replied <strong>${escapeHtml(m.san)}</strong>. Your move!`);
  }

  function onSolved() {
    const step = steps[cur];
    solved = true;
    busy = false;
    board.setInteractive(false, null);
    board.clearArrows();
    setFeedback(feedbackEl, 'good', mdLite(step.task && step.task.success ? step.task.success : 'Well done!'));
    sfx('correct');
    flashClass(boardWrap, 'flash-good', timers, 900);
    confetti(boardSlot, timers, { count: 28 });
    syncChrome();
  }

  function showHint() {
    const step = steps[cur];
    if (!step.task || solved || busy) return;
    const expected = step.task.solution[idx];
    if (!expected) return;
    const from = expected.slice(0, 2); const to = expected.slice(2, 4);
    if (hintStage === 0) {
      hintStage = 1;
      board.setHighlights([{ square: from, kind: 'hint' }]);
      const text = idx === 0 && step.task.hint ? mdLite(step.task.hint) : 'Look at the highlighted piece — it wants to move.';
      setFeedback(feedbackEl, 'warn', text);
    } else {
      hintStage = 2;
      board.setHighlights([{ square: from, kind: 'hint' }]);
      board.setArrows([{ from, to, color: 'green' }]);
      setFeedback(feedbackEl, 'warn', 'Play the move shown by the green arrow.');
    }
  }

  async function finish() {
    if (completed) return;
    completed = true;
    timers.clear();
    board.setInteractive(false, null);
    progressBar.style.width = '100%';
    rememberLesson(course.id, lesson.id, 0);

    const nextHref = nextLesson ? `#/learn/${encodeURIComponent(course.id)}/${encodeURIComponent(nextLesson.id)}` : courseHref;
    const status = h('p', { class: 'text-sm subtle' }, 'Saving your progress…');
    const card = h('div', { class: 'lrn-complete' },
      h('div', { class: 'lrn-complete-badge', html: icon('check') }),
      h('h2', null, 'Lesson complete!'),
      h('p', null, nextLesson ? `Up next: ${nextLesson.title}` : `You finished the last lesson of “${course.title}”.`),
      status,
      h('div', { class: 'lrn-nav' },
        h('a', { class: 'btn btn-primary btn-lg', href: nextHref, html: nextLesson ? '<span>Next lesson</span>' + icon('chevron-right') : icon('trophy') + '<span>Back to course</span>' }),
        nextLesson ? h('a', { class: 'btn btn-secondary', href: courseHref, html: icon('list') + '<span>Back to course</span>' }) : h('a', { class: 'btn btn-secondary', href: '#/learn', html: icon('learn') + '<span>More courses</span>' }),
        h('button', { class: 'btn btn-ghost', type: 'button', html: icon('refresh') + '<span>Replay lesson</span>', onClick: () => { completed = false; restoreBody(); enterStep(0); } })));

    body.replaceChildren(card);
    footer.hidden = true;
    stepCount.textContent = 'Completed';
    sfx('gameEnd');
    confetti(boardSlot, timers, { count: 48 });

    try {
      await api.post('/api/progress', { course_id: course.id, lesson_id: lesson.id, completed: true }, { signal: ctrl.signal });
      if (!bag.disposed) status.textContent = 'Progress saved ✓';
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      status.textContent = 'Could not save progress — is the server running?';
      status.classList.add('text-danger');
    }
  }

  function restoreBody() {
    body.replaceChildren(h('div', { class: 'lrn-coach' }, h('div', { class: 'avatar avatar-sm', 'aria-hidden': 'true' }, '🎓'), textEl), taskEl, feedbackEl);
    footer.hidden = false;
  }

  // Keyboard: ← / → step through the lesson (Enter = next when ready).
  bag.on(window, 'keydown', (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'ArrowRight') { e.preventDefault(); go(1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); go(-1); }
    else if ((e.key === 'h' || e.key === 'H') && !completed) showHint();
  });

  enterStep(0);
  return () => bag.dispose();
}
