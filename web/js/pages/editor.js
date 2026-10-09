// Board editor (#/editor[?fen=<FEN>][&orientation=black])
// Set up any position with the PositionEditor component, then analyse it, play it against a bot
// or a friend, copy its FEN or share a link to it. Contract: docs/CONTRACT.md "Position editor".

import { h, icon, toast, copyText, disposables } from '../ui.js';
import { t } from '../i18n.js';
import { PositionEditor, START_FEN } from '../components/editor.js';

export function title() { return t('editor.title'); }

const URL_SYNC_MS = 400;

export async function mount(root, { query = {} } = {}) {
  const bag = disposables();
  const host = h('div', { class: 'ed-page' });
  root.appendChild(host);
  bag.add(() => host.remove());

  const linkFen = typeof query.fen === 'string' && query.fen.trim() ? query.fen : null;
  let botColor = null; // null = follow the side to move

  // ---- Actions (rendered into the editor's side column) --------------------
  const spanHtml = (text) => h('span', null, text).outerHTML;
  const actionBtn = (ic, label, kind = 'secondary') => h('button', { class: `btn btn-${kind}`, type: 'button', html: icon(ic) + spanHtml(label) });
  const bAnalyze = actionBtn('analysis', t('editor.actions.analyze'), 'primary');
  const bBot = actionBtn('robot', t('editor.actions.playBot'));
  const bFriend = actionBtn('users', t('editor.actions.playFriend'));
  const bCopy = actionBtn('copy', t('editor.actions.copyFen'), 'ghost');
  const bShare = actionBtn('link', t('editor.actions.share'), 'ghost');

  const colorW = h('button', { type: 'button', role: 'radio' }, t('editor.actions.asWhite'));
  const colorB = h('button', { type: 'button', role: 'radio' }, t('editor.actions.asBlack'));
  const colorGroup = h('div', { class: 'segmented block', role: 'radiogroup', 'aria-label': t('editor.actions.chooseColor') }, colorW, colorB);
  const blockedMsg = h('p', { class: 'help ed-blocked', role: 'status' });

  const actions = h('div', { class: 'card ed-actions' },
    h('div', { class: 'label' }, t('editor.actions.title')),
    bAnalyze,
    h('div', { class: 'ed-bot' }, colorGroup, bBot),
    bFriend,
    blockedMsg,
    h('div', { class: 'ed-row' }, bCopy, bShare));

  const heading = h('div', { class: 'stack-sm' },
    h('h1', { class: 'ed-title' }, t('editor.heading')),
    h('p', { class: 'subtle text-sm ed-sub' }, t('editor.subtitle')));

  // ---- Editor ---------------------------------------------------------------
  let urlTimer = null;
  const syncUrl = (fen) => {
    if (urlTimer) clearTimeout(urlTimer);
    urlTimer = setTimeout(() => {
      urlTimer = null;
      if (bag.disposed || !location.hash.startsWith('#/editor')) return;
      const orient = editor?.orientation === 'black' ? '&orientation=black' : '';
      try { history.replaceState(history.state, '', `#/editor?fen=${encodeURIComponent(fen)}${orient}`); } catch { /* ignore */ }
    }, URL_SYNC_MS);
  };
  bag.add(() => { if (urlTimer) clearTimeout(urlTimer); });

  let editor = null;
  const refresh = () => {
    if (!editor) return;
    const fen = editor.getFen();
    const turn = fen.split(' ')[1];
    const color = botColor || turn;
    colorW.classList.toggle('active', color === 'w');
    colorB.classList.toggle('active', color === 'b');
    colorW.setAttribute('aria-checked', String(color === 'w'));
    colorB.setAttribute('aria-checked', String(color === 'b'));
    const valid = editor.valid;
    const over = valid && editor.gameOver;
    for (const b of [bAnalyze, bBot, bFriend]) b.setAttribute('aria-disabled', 'false');
    if (!valid) for (const b of [bAnalyze, bBot, bFriend]) b.setAttribute('aria-disabled', 'true');
    else if (over) for (const b of [bBot, bFriend]) b.setAttribute('aria-disabled', 'true');
    blockedMsg.textContent = !valid ? t('editor.actions.fixFirst') : over ? t('editor.actions.overNoPlay') : '';
    blockedMsg.hidden = !blockedMsg.textContent;
  };

  editor = new PositionEditor(host, {
    fen: START_FEN,
    orientation: query.orientation === 'black' ? 'black' : 'white',
    onChange: (fen) => { refresh(); syncUrl(fen); },
  });
  bag.add(() => editor.destroy());
  editor.sideEl.append(actions);
  editor.root.querySelector('.pe-side')?.prepend(heading);
  if (linkFen && !editor.setFen(linkFen)) toast(t('editor.linkInvalid'), 'warning');
  refresh();

  // ---- Wiring ---------------------------------------------------------------
  const guard = (btn, fn) => bag.on(btn, 'click', () => {
    if (btn.getAttribute('aria-disabled') === 'true') {
      toast(blockedMsg.textContent || t('editor.actions.fixFirst'), 'warning');
      return;
    }
    fn();
  });
  const enc = () => encodeURIComponent(editor.getFen());
  guard(bAnalyze, () => {
    const orient = editor.orientation === 'black' ? '&orientation=black' : '';
    location.hash = `#/analysis?fen=${enc()}${orient}`;
  });
  guard(bBot, () => {
    const color = botColor || editor.getFen().split(' ')[1];
    location.hash = `#/play?fen=${enc()}&color=${color}`;
  });
  guard(bFriend, () => { location.hash = `#/local?fen=${enc()}`; });
  bag.on(colorW, 'click', () => { botColor = 'w'; refresh(); });
  bag.on(colorB, 'click', () => { botColor = 'b'; refresh(); });
  bag.on(bCopy, 'click', () => { copyText(editor.getFen(), t('editor.actions.fenCopied')); });
  bag.on(bShare, 'click', () => {
    const orient = editor.orientation === 'black' ? '&orientation=black' : '';
    const url = `${location.origin}${location.pathname}#/editor?fen=${enc()}${orient}`;
    copyText(url, t('editor.actions.linkCopied'));
  });

  return bag.dispose;
}
