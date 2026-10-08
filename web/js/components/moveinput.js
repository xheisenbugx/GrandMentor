// GrandMentor — typed move input (keyboard / screen-reader friendly way to make a move).
// Contract: docs/CONTRACT.md §5 (`createMoveInput`, `parseMoveText`).
//
//   const mi = createMoveInput({ board });          // or board: () => currentBoard
//   panel.appendChild(mi.el);  ...  mi.destroy();
//
// Accepts SAN ("e4", "Nf3", "exd5", "O-O", "0-0-0", "e8=Q", "e8Q", lower-case "nf3") and UCI
// ("e2e4", "e7e8q"). The move is played through board.playUserMove(), so it goes through the page's
// normal onMove handler exactly like a drag or a click (puzzle checking, engine replies, etc.).
// Errors are shown under the input and announced; the board announces the move itself.

import { Chess } from '../../vendor/chess.js';
import { h, icon } from '../ui.js';
import { t } from '../i18n.js';
import { announce } from './announcer.js';

const UCI_RE = /^([a-h][1-8])-?([a-h][1-8])([qrbn])?$/i;

/** Looks like move notation at all (so "not legal" can be told apart from "not understood"). */
export function looksLikeMove(s) {
  const v = String(s || '').trim();
  return UCI_RE.test(v)
    || /^([KQRBNkqrbn]?[a-h]?[1-8]?x?-?[a-h][1-8](=?[QRBNqrbn])?|[O0o]-?[O0o](-?[O0o])?)[+#]?[!?]*$/.test(v);
}

/**
 * Parse a typed move in `chess`'s position (a chess.js instance or a FEN).
 * @returns {{from:string,to:string,promotion?:string,san:string}|null} null when not legal / not understood
 */
export function parseMoveText(chess, text) {
  const raw = String(text || '').trim().replace(/[!?]+$/, '');
  if (!raw || raw.length > 12) return null;
  let fen;
  try { fen = typeof chess === 'string' ? chess : chess.fen(); } catch { return null; }
  const tryMove = (spec) => {
    try {
      const probe = new Chess(fen);
      const mv = probe.move(spec);
      return mv ? { from: mv.from, to: mv.to, promotion: mv.promotion || undefined, san: mv.san } : null;
    } catch { return null; }
  };
  const uci = raw.match(UCI_RE);
  if (uci) {
    const from = uci[1].toLowerCase();
    const to = uci[2].toLowerCase();
    const promo = uci[3] ? uci[3].toLowerCase() : undefined;
    const r = tryMove({ from, to, promotion: promo }) || (promo ? null : tryMove({ from, to, promotion: 'q' }));
    if (r) return r;
  }
  const variants = [raw];
  const castle = raw.replace(/0/g, 'O').replace(/o/g, 'O');
  if (/^O-?O(-?O)?[+#]?$/.test(castle)) variants.push(castle.replace(/^OOO/, 'O-O-O').replace(/^O-OO/, 'O-O-O').replace(/^OO/, 'O-O'));
  if (/^[nrqk]/.test(raw)) variants.push(raw[0].toUpperCase() + raw.slice(1));
  if (/^b[a-h]?[1-8]?x?[a-h][1-8]/.test(raw) && !/^b[1-8]/.test(raw)) variants.push('B' + raw.slice(1));
  variants.push(raw.replace(/=?([qrbn])([+#]?)$/i, (_, p, c) => '=' + p.toUpperCase() + c));
  for (const v of variants) {
    const r = tryMove(v);
    if (r) return r;
  }
  // A pawn move to the last rank without a piece ("e8"): promote to a queen.
  if (/^[a-h](x[a-h])?[18]$/.test(raw)) return tryMove(raw + '=Q');
  return null;
}

let seq = 0;

/**
 * Create a typed-move form.
 * @param {object} o
 * @param {object|(() => object)} o.board   Board instance (or a getter, for pages that rebuild boards)
 * @param {(mv:{from,to,promotion,san}) => any} [o.onMove]  custom handler instead of board.playUserMove;
 *        return false to report the move as rejected
 * @param {() => (string|null)} [o.blocked] return a message when moves are not possible right now
 * @param {string} [o.label]       accessible label (default "Type a move")
 * @param {string} [o.placeholder] (default "e4, Nf3, O-O…")
 * @param {string} [o.className]
 * @returns {{el: HTMLFormElement, input: HTMLInputElement, focus(): void, clear(): void, setDisabled(on: boolean): void, destroy(): void}}
 */
export function createMoveInput({ board, onMove, blocked, label, placeholder, className } = {}) {
  const id = `gm-move-input-${++seq}`;
  const helpId = `${id}-help`;
  const fbId = `${id}-fb`;
  const getBoard = typeof board === 'function' ? board : () => board;
  const input = h('input', {
    id, type: 'text', class: 'input gm-move-input-field', autocomplete: 'off', autocapitalize: 'off', spellcheck: 'false',
    enterkeyhint: 'send', maxlength: '12', placeholder: placeholder || t('a11y.moveInput.placeholder'),
    'aria-describedby': `${helpId} ${fbId}`,
  });
  const submit = h('button', { type: 'submit', class: 'btn btn-secondary btn-sm btn-icon gm-move-input-go', 'aria-label': t('a11y.moveInput.submit'), title: t('a11y.moveInput.submit'), html: icon('send', { size: 16 }) });
  const feedback = h('div', { id: fbId, class: 'gm-move-input-fb text-xs' });
  const help = h('span', { id: helpId, class: 'sr-only' }, t('a11y.moveInput.help'));
  const lbl = h('label', { class: 'gm-move-input-label', for: id, html: icon('keyboard', { size: 16 }) + '<span class="sr-only"></span>' });
  lbl.querySelector('.sr-only').textContent = label || t('a11y.moveInput.label');
  lbl.title = label || t('a11y.moveInput.label');
  const form = h('form', { class: ['gm-move-input', className], role: 'group', 'aria-label': label || t('a11y.moveInput.label') },
    h('div', { class: 'gm-move-input-row' }, lbl, input, submit), help, feedback);

  const setFeedback = (text, kind) => {
    feedback.textContent = text || '';
    feedback.className = `gm-move-input-fb text-xs${kind ? ' is-' + kind : ''}`;
    input.classList.toggle('input-error', kind === 'error');
    if (kind === 'error') input.setAttribute('aria-invalid', 'true'); else input.removeAttribute('aria-invalid');
  };
  const fail = (text) => { setFeedback(text, 'error'); announce(text, { assertive: true, dedupe: false }); };

  const onSubmit = (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    const b = getBoard();
    const why = typeof blocked === 'function' ? blocked() : null;
    if (why) { fail(why); return; }
    if (!b || !b.chess) { fail(t('a11y.moveInput.notNow')); return; }
    const mv = parseMoveText(b.chess, text);
    if (!mv) { fail(looksLikeMove(text) ? t('a11y.moveInput.illegal', { move: text }) : t('a11y.moveInput.unknown', { move: text })); return; }
    let ok;
    if (typeof onMove === 'function') {
      try { ok = onMove(mv); } catch (err) { console.error('[moveinput] onMove failed', err); ok = false; }
      ok = ok !== false;
    } else {
      if (typeof b.canUserMove === 'function' && !b.canUserMove(mv.from)) {
        fail(t('a11y.moveInput.notYourTurn'));
        return;
      }
      ok = !!b.playUserMove(mv);
    }
    if (!ok) { fail(t('a11y.moveInput.rejected', { move: mv.san })); return; }
    input.value = '';
    setFeedback(t('a11y.moveInput.played', { san: mv.san }), 'ok');
  };
  const onInput = () => { if (input.classList.contains('input-error')) setFeedback('', null); };
  form.addEventListener('submit', onSubmit);
  input.addEventListener('input', onInput);

  return {
    el: form,
    input,
    focus() { try { input.focus(); } catch { /* ignore */ } },
    clear() { input.value = ''; setFeedback('', null); },
    setDisabled(on) { input.disabled = !!on; submit.disabled = !!on; },
    destroy() {
      form.removeEventListener('submit', onSubmit);
      input.removeEventListener('input', onInput);
      form.remove();
    },
  };
}

export default createMoveInput;
