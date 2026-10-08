// MentorPanel — chat with the coach about the current position.
// Contract (docs/CONTRACT.md §5):
//   const m = new MentorPanel(el, { getContext: () => ({ fen, moves_san, engine_lines }) });
//   m.say(text, { kind });  m.destroy();
// Extra (optional) options: { name, avatar, greeting, suggestions: [string], placeholder, compact }.
// Extra methods: ask(question), clear(), setSuggestions([..]).
// Bounded: at most MAX_MESSAGES bubbles kept in the DOM, last MAX_HISTORY turns sent to the server.
// destroy() aborts the in-flight request and removes every listener.

import { api, isAbort } from '../api.js';
import { h, icon, mdLite } from '../ui.js';
import { t } from '../i18n.js';
import { ensureAnalysisCss } from './evalgraph.js';

const MAX_MESSAGES = 80;
const MAX_HISTORY = 12;
const MAX_QUESTION = 600;

// Resolved at construction time so the panel follows the active language.
const defaultSuggestions = () => {
  const s = t('ui.mentor.suggestions');
  return Array.isArray(s) ? s : [];
};

export class MentorPanel {
  constructor(el, {
    getContext,
    name = t('ui.mentor.name'),
    avatar = '🎓',
    greeting = t('ui.mentor.greeting'),
    suggestions = defaultSuggestions(),
    placeholder = t('ui.mentor.placeholder'),
    compact = false,
  } = {}) {
    ensureAnalysisCss();
    this.el = el;
    this.getContext = typeof getContext === 'function' ? getContext : () => ({});
    this.name = name;
    this.avatar = avatar;
    this.history = [];
    this._ctrl = null;
    this._destroyed = false;
    this._busy = false;

    this.log = h('div', { class: 'mentor-log', role: 'log', 'aria-live': 'polite' });
    this.chips = h('div', { class: 'chip-row mentor-suggestions' });
    this.input = h('textarea', {
      class: 'textarea mentor-input', rows: 1, maxlength: MAX_QUESTION, placeholder,
      'aria-label': placeholder,
    });
    this.sendBtn = h('button', { class: 'btn btn-primary btn-icon mentor-send', type: 'button', 'aria-label': t('ui.mentor.send'), html: icon('send') });
    this.form = h('form', { class: 'mentor-form' }, this.input, this.sendBtn);
    this.root = h('div', { class: 'mentor-panel' + (compact ? ' compact' : '') }, this.log, this.chips, this.form);
    el.appendChild(this.root);

    this._onSubmit = (e) => { e.preventDefault(); this.ask(this.input.value); };
    this._onKeyDown = (e) => {
      e.stopPropagation(); // don't let page-level arrow-key navigation steal typing
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); this.ask(this.input.value); }
    };
    this._onInput = () => {
      this.input.style.height = 'auto';
      this.input.style.height = Math.min(120, this.input.scrollHeight) + 'px';
    };
    this._onChip = (e) => {
      const b = e.target.closest('button[data-q]');
      if (b) this.ask(b.dataset.q);
    };
    this.form.addEventListener('submit', this._onSubmit);
    this.input.addEventListener('keydown', this._onKeyDown);
    this.input.addEventListener('input', this._onInput);
    this.chips.addEventListener('click', this._onChip);

    this.setSuggestions(suggestions);
    if (greeting) this.say(greeting);
  }

  setSuggestions(list) {
    this.chips.replaceChildren(...(Array.isArray(list) ? list : []).slice(0, 6).map((q) =>
      h('button', { class: 'chip', type: 'button', dataset: { q } }, q)));
    this.chips.hidden = !this.chips.childElementCount;
  }

  /** Show a mentor message. kind: 'info' (default) | 'success' | 'warning' | 'error' | 'idea'. */
  say(text, { kind = 'info', source, title } = {}) {
    if (this._destroyed || !text) return null;
    const bubble = h('div', { class: `bubble bubble-mentor mentor-msg kind-${kind}` });
    if (title) bubble.appendChild(h('div', { class: 'mentor-msg-title' }, title));
    if (Array.isArray(text)) {
      bubble.appendChild(h('ul', { class: 'mentor-ideas' }, text.map((item) => h('li', { html: mdLite(String(item)) }))));
    } else {
      bubble.appendChild(h('div', { class: 'md', html: mdLite(String(text)) }));
    }
    if (source === 'llm') bubble.appendChild(h('div', { class: 'mentor-source' }, t('ui.mentor.aiCoach')));
    const row = h('div', { class: 'mentor-row pop-in' }, h('div', { class: 'avatar avatar-sm mentor-avatar' }, this.avatar), bubble);
    this._append(row);
    return row;
  }

  clear() {
    this.history = [];
    this.log.replaceChildren();
  }

  async ask(raw) {
    const question = String(raw || '').trim().slice(0, MAX_QUESTION);
    if (!question || this._busy || this._destroyed) return;
    this.input.value = '';
    this._onInput();
    this._append(h('div', { class: 'mentor-row mentor-row-user' }, h('div', { class: 'bubble bubble-user' }, question)));
    const typing = h('div', { class: 'mentor-row' },
      h('div', { class: 'avatar avatar-sm mentor-avatar' }, this.avatar),
      h('div', { class: 'bubble bubble-mentor mentor-typing', 'aria-label': t('ui.mentor.thinking', { name: this.name }) }, h('span'), h('span'), h('span')));
    this._append(typing);
    this._setBusy(true);

    let ctx = {};
    try { ctx = this.getContext() || {}; } catch (e) { console.warn('[mentor] getContext failed', e); }
    const body = {
      question,
      fen: String(ctx.fen || ''),
      moves_san: Array.isArray(ctx.moves_san) ? ctx.moves_san.slice(-200) : [],
      engine_lines: Array.isArray(ctx.engine_lines) ? ctx.engine_lines.slice(0, 5).map(String) : [],
      history: this.history.slice(-MAX_HISTORY),
    };
    this._ctrl?.abort();
    const ctrl = new AbortController();
    this._ctrl = ctrl;
    try {
      const res = await api.post('/api/mentor/chat', body, { signal: ctrl.signal, timeout: 90000 });
      if (this._destroyed) return;
      typing.remove();
      const answer = (res && res.answer) ? String(res.answer) : t('ui.mentor.noAnswer');
      this.say(answer, { source: res?.source });
      this.history.push({ role: 'user', text: question }, { role: 'mentor', text: answer });
      if (this.history.length > MAX_HISTORY * 2) this.history.splice(0, this.history.length - MAX_HISTORY * 2);
    } catch (e) {
      if (this._destroyed || isAbort(e)) return;
      typing.remove();
      this.say(e?.message ? t('ui.mentor.errorWithMessage', { message: e.message }) : t('ui.mentor.unreachable'), { kind: 'error' });
    } finally {
      if (this._ctrl === ctrl) this._ctrl = null;
      if (!this._destroyed) this._setBusy(false);
    }
  }

  destroy() {
    if (this._destroyed) return;
    this._destroyed = true;
    this._ctrl?.abort();
    this._ctrl = null;
    this.form.removeEventListener('submit', this._onSubmit);
    this.input.removeEventListener('keydown', this._onKeyDown);
    this.input.removeEventListener('input', this._onInput);
    this.chips.removeEventListener('click', this._onChip);
    this.root.remove();
    this.history = [];
    this.getContext = () => ({});
  }

  // -- internals -----------------------------------------------------------
  _setBusy(b) {
    this._busy = b;
    this.sendBtn.classList.toggle('loading', b);
    this.sendBtn.disabled = b;
  }

  _append(node) {
    this.log.appendChild(node);
    while (this.log.childElementCount > MAX_MESSAGES) this.log.firstElementChild.remove();
    this.log.scrollTop = this.log.scrollHeight;
  }
}
