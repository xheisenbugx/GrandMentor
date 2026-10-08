// "Your data" section of the Settings page: download a backup, restore one (merge or replace),
// and sync with another GrandMentor on the same network. Self-contained:
//
//   const section = createBackupSection();
//   parent.appendChild(section.el);
//   ...
//   section.destroy();   // removes listeners, timers, aborts fetches, closes its modals
//
// API: docs/CONTRACT.md "Backup & sync". Browser preferences (localStorage keys starting with
// "grandmentor" / "gm.") travel inside the backup file under "browser".

import { h, icon, disposables, toast, modal, formatDate, formatRelative } from '../ui.js';
import { getSetting } from '../settings.js';
import { t } from '../i18n.js';

const MAX_FILE_BYTES = 200 * 1024 * 1024;
const REMIND_AFTER_DAYS = 30;
const SYNC_PREFS_KEY = 'grandmentor.sync.v1';
const BROWSER_KEY_RE = /^(grandmentor|gm[._-])/i;
const REQUEST_TIMEOUT_MS = 120000;
// Tables with a friendly label (others show their name, humanised).
const TABLE_LABELS = new Set(['games', 'profile', 'puzzle_attempts', 'rating_history', 'rush_scores', 'lesson_progress', 'activity']);

/** Friendly label for a database table. */
function tableLabel(name) {
  if (TABLE_LABELS.has(name)) return t(`settings.data.tables.${name}`);
  const s = String(name).replace(/[_-]+/g, ' ').trim();
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Browser preferences to carry in a backup. */
function collectBrowserSettings() {
  const out = {};
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && BROWSER_KEY_RE.test(k)) {
        const v = localStorage.getItem(k);
        if (typeof v === 'string' && v.length < 256 * 1024) out[k] = v;
      }
    }
  } catch { /* storage blocked */ }
  return out;
}

function restoreBrowserSettings(obj) {
  let n = 0;
  try {
    for (const [k, v] of Object.entries(obj || {})) {
      if (BROWSER_KEY_RE.test(k) && typeof v === 'string') { localStorage.setItem(k, v); n++; }
    }
  } catch { /* storage blocked */ }
  return n;
}

function readSyncPrefs() {
  try { return JSON.parse(localStorage.getItem(SYNC_PREFS_KEY) || '{}') || {}; } catch { return {}; }
}

function writeSyncPrefs(p) {
  try { localStorage.setItem(SYNC_PREFS_KEY, JSON.stringify(p)); } catch { /* ignore */ }
}

/** "192.168.1.20:8080" -> "http://192.168.1.20:8080" (no path, no trailing slash). */
function normalizeAddress(raw) {
  let s = String(raw || '').trim();
  if (!s) return null;
  if (!/^https?:\/\//i.test(s)) s = `http://${s}`;
  try {
    const u = new URL(s);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    return u.origin;
  } catch { return null; }
}

/**
 * fetch() with timeout + caller signal. Returns { status, data, text }; throws Error(message)
 * on network failure or non-2xx (message from the server's { error } when present).
 */
async function request(method, url, { body, headers = {}, signal, as = 'json', networkError } = {}) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), REQUEST_TIMEOUT_MS);
  const onAbort = () => ctrl.abort();
  signal?.addEventListener('abort', onAbort, { once: true });
  let res;
  try {
    res = await fetch(url, {
      method,
      body,
      headers: { 'Accept-Language': getSetting('language') || 'en', ...(body ? { 'Content-Type': 'application/json' } : {}), ...headers },
      signal: ctrl.signal,
    });
  } catch (e) {
    if (signal?.aborted) throw new DOMException('aborted', 'AbortError');
    throw new Error(ctrl.signal.aborted ? t('settings.data.errors.timeout') : (networkError || t('settings.data.errors.network')));
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', onAbort);
  }
  let text = '';
  try { text = await res.text(); } catch { /* ignore */ }
  let data = null;
  if (as === 'json' || !res.ok) { try { data = JSON.parse(text); } catch { data = null; } }
  if (!res.ok) {
    const msg = data && typeof data.error === 'string' ? data.error : t('settings.data.errors.status', { status: res.status });
    const err = new Error(msg);
    err.status = res.status;
    throw err;
  }
  return { status: res.status, data, text, headers: res.headers };
}

/** Inserts `"browser": {...}` into the server's backup JSON without re-parsing it. */
function withBrowserSettings(text) {
  const browser = JSON.stringify(collectBrowserSettings());
  const end = text.lastIndexOf('}');
  if (end < 0) return text;
  return `${text.slice(0, end)},"browser":${browser}}`;
}

function isAbortError(e) { return e?.name === 'AbortError'; }

/** replaceChildren() that skips null/false (h() children may be conditional). */
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat().filter((k) => k != null && k !== false));
}

export function createBackupSection() {
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  const modals = new Set();
  bag.add(() => { for (const m of modals) m.close(); modals.clear(); });

  // ---- Status --------------------------------------------------------------------
  const statusText = h('div', { class: 'setting-row-desc' }, t('common.loading'));
  const statusChips = h('div', { class: 'chip-row bk-chips' });
  const reminder = h('div', { class: 'callout callout-warning bk-reminder', hidden: true });

  const renderStatus = (st) => {
    const last = st?.last_backup_at;
    statusText.textContent = last
      ? t('settings.data.status.lastBackup', { when: formatRelative(last) })
      : t('settings.data.status.never');
    statusText.title = last ? formatDate(last) : '';
    const tables = Array.isArray(st?.tables) ? st.tables : [];
    const count = (n) => tables.find((x) => x.name === n)?.rows || 0;
    const chips = [
      [count('games'), 'settings.data.status.games'],
      [count('puzzle_attempts'), 'settings.data.status.puzzles'],
      [count('lesson_progress'), 'settings.data.status.lessons'],
    ].filter(([n]) => n > 0);
    statusChips.replaceChildren(...chips.map(([n, k]) => h('span', { class: 'badge badge-lg' }, t(k, { count: n }))));
    const hasData = tables.some((x) => x.name !== 'profile' && x.rows > 0);
    const ageDays = last ? (Date.now() - new Date(last).getTime()) / 86400000 : Infinity;
    const remind = hasData && ageDays > REMIND_AFTER_DAYS;
    reminder.hidden = !remind;
    if (remind) {
      reminder.replaceChildren(h('span', { class: 'bk-ci', html: icon('clock') }), h('div', null,
        h('strong', null, last ? t('settings.data.reminder.titleOld') : t('settings.data.reminder.titleNever')),
        h('div', { class: 'text-sm' }, t('settings.data.reminder.text'))));
    }
  };

  const loadStatus = async () => {
    try {
      const { data } = await request('GET', '/api/backup/status', { signal });
      if (!bag.disposed) renderStatus(data);
    } catch (e) {
      if (!isAbortError(e) && !bag.disposed) statusText.textContent = t('settings.data.status.unavailable');
    }
  };

  // ---- Download ------------------------------------------------------------------
  const downloadBtn = h('button', { type: 'button', class: 'btn btn-primary', html: icon('download') + `<span>${t('settings.data.download.button')}</span>` });
  bag.on(downloadBtn, 'click', async () => {
    downloadBtn.classList.add('loading');
    try {
      const { text, headers } = await request('GET', '/api/backup/export', { signal, as: 'text' });
      const cd = headers.get('content-disposition') || '';
      const name = (cd.match(/filename="([^"]+)"/) || [])[1] || `grandmentor-backup-${new Date().toISOString().slice(0, 10)}.json`;
      const blob = new Blob([withBrowserSettings(text)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = h('a', { href: url, download: name, style: 'display:none' });
      document.body.appendChild(a);
      a.click();
      a.remove();
      bag.timeout(() => URL.revokeObjectURL(url), 60000);
      toast(t('settings.data.download.done', { size: formatBytes(blob.size) }), 'success');
      loadStatus();
    } catch (e) {
      if (!isAbortError(e)) toast(e.message || t('settings.data.errors.generic'), 'error');
    } finally {
      downloadBtn.classList.remove('loading');
    }
  });

  // ---- Restore -------------------------------------------------------------------
  const fileInput = h('input', { type: 'file', accept: '.json,application/json', class: 'bk-file-input', tabindex: '-1', 'aria-hidden': 'true' });
  const chooseBtn = h('button', { type: 'button', class: 'btn btn-secondary btn-sm', html: icon('upload', { size: 16 }) + `<span>${t('settings.data.restore.choose')}</span>` });
  const drop = h('div', { class: 'bk-drop', role: 'button', tabindex: '0', 'aria-label': t('settings.data.restore.dropAria') },
    h('span', { class: 'bk-drop-icon', html: icon('upload') }),
    h('div', { class: 'bk-drop-title' }, t('settings.data.restore.dropTitle')),
    h('div', { class: 'setting-row-desc' }, t('settings.data.restore.dropHint')),
    chooseBtn, fileInput);
  const restorePanel = h('div', { class: 'bk-panel', hidden: true, 'aria-live': 'polite' });

  let current = null; // { file, preview }
  const openPicker = () => fileInput.click();
  bag.on(chooseBtn, 'click', (e) => { e.stopPropagation(); openPicker(); });
  bag.on(drop, 'click', openPicker);
  bag.on(drop, 'keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openPicker(); } });
  bag.on(fileInput, 'change', () => { const f = fileInput.files?.[0]; fileInput.value = ''; if (f) handleFile(f); });
  bag.on(drop, 'dragover', (e) => { e.preventDefault(); drop.classList.add('dragging'); });
  bag.on(drop, 'dragleave', () => drop.classList.remove('dragging'));
  bag.on(drop, 'drop', (e) => {
    e.preventDefault();
    drop.classList.remove('dragging');
    const f = e.dataTransfer?.files?.[0];
    if (f) handleFile(f);
  });

  const resetRestore = () => { current = null; restorePanel.hidden = true; restorePanel.replaceChildren(); drop.hidden = false; };

  const warningText = (w) => {
    switch (w.code) {
      case 'unknown_table': return t('settings.data.warnings.unknownTable', { table: tableLabel(w.table), count: w.count });
      case 'unknown_column': return t('settings.data.warnings.unknownColumn', { table: tableLabel(w.table), column: w.column });
      case 'rows_failed': return t('settings.data.warnings.rowsFailed', { table: tableLabel(w.table), count: w.count });
      case 'newer_schema': return t('settings.data.warnings.newerSchema');
      case 'browser_dropped': return t('settings.data.warnings.browserDropped');
      default: return w.code;
    }
  };
  const warningList = (warnings) => {
    // Group unknown columns per table to keep the list short.
    const list = Array.isArray(warnings) ? warnings.slice(0, 12) : [];
    if (!list.length) return null;
    return h('div', { class: 'callout callout-warning' }, h('span', { class: 'bk-ci', html: icon('alert') }),
      h('div', { class: 'stack-sm' },
        h('strong', null, t('settings.data.warnings.title')),
        h('ul', { class: 'bk-warnings' }, list.map((w) => h('li', null, warningText(w))))));
  };

  async function handleFile(file) {
    if (file.size > MAX_FILE_BYTES) { toast(t('settings.data.errors.tooBig'), 'error'); return; }
    drop.hidden = true;
    restorePanel.hidden = false;
    fill(restorePanel, h('div', { class: 'row-sm muted' }, h('span', { class: 'spinner' }), t('settings.data.restore.reading', { name: file.name })));
    try {
      const { data } = await request('POST', '/api/backup/preview', { body: file, signal });
      if (bag.disposed) return;
      current = { file, preview: data };
      renderPreview();
    } catch (e) {
      if (isAbortError(e) || bag.disposed) return;
      fill(restorePanel,
        h('div', { class: 'callout callout-danger' }, h('span', { class: 'bk-ci', html: icon('x-circle') }), h('div', null, h('strong', null, t('settings.data.restore.badFile')), h('div', { class: 'text-sm' }, e.message))),
        h('div', { class: 'bk-actions' }, h('button', { type: 'button', class: 'btn btn-ghost', onClick: resetRestore }, t('settings.data.restore.tryAnother'))));
    }
  }

  function renderPreview() {
    const { file, preview: p } = current;
    const known = (p.tables || []).filter((x) => x.known && x.rows > 0);
    const hasBrowser = p.browser && Object.keys(p.browser).length > 0;
    let mode = 'merge';
    const prefsBox = h('input', { type: 'checkbox', checked: hasBrowser });
    const modeCard = (value, iconName, titleKey, descKey) => h('label', { class: 'bk-mode' },
      h('input', { type: 'radio', name: 'bk-mode', value, checked: value === mode }),
      h('span', { class: 'bk-mode-icon', html: icon(iconName) }),
      h('span', { class: 'bk-mode-text' }, h('span', { class: 'bk-mode-title' }, t(titleKey)), h('span', { class: 'setting-row-desc' }, t(descKey))));
    const modes = h('div', { class: 'bk-modes', role: 'radiogroup', 'aria-label': t('settings.data.restore.howTitle') },
      modeCard('merge', 'plus', 'settings.data.restore.merge.title', 'settings.data.restore.merge.desc'),
      modeCard('replace', 'refresh', 'settings.data.restore.replace.title', 'settings.data.restore.replace.desc'));
    modes.addEventListener('change', (e) => { if (e.target.name === 'bk-mode') { mode = e.target.value; restoreBtn.className = `btn ${mode === 'replace' ? 'btn-danger' : 'btn-primary'}`; } });
    const restoreBtn = h('button', { type: 'button', class: 'btn btn-primary', html: icon('upload') + `<span>${t('settings.data.restore.go')}</span>` });
    restoreBtn.addEventListener('click', () => runImport(mode, hasBrowser && prefsBox.checked, restoreBtn));
    const meta = [
      p.created_at ? t('settings.data.restore.createdOn', { date: formatDate(p.created_at) }) : null,
      p.profile_name ? t('settings.data.restore.player', { name: p.profile_name }) : null,
      p.app_version ? t('settings.data.restore.version', { version: p.app_version }) : null,
      formatBytes(file.size),
    ].filter(Boolean).join(' · ');
    fill(restorePanel,
      h('div', { class: 'bk-file' },
        h('span', { class: 'bk-file-icon', html: icon('folder') }),
        h('div', { class: 'bk-file-text' }, h('div', { class: 'bk-file-name' }, file.name), h('div', { class: 'setting-row-desc' }, meta))),
      known.length
        ? h('div', { class: 'bk-table-list' }, known.map((x) => h('div', { class: 'bk-table-row' },
          h('span', null, tableLabel(x.name)),
          h('span', { class: 'tabular muted' }, t('settings.data.restore.rowsVs', { count: x.rows, here: x.current_rows ?? 0 })))))
        : h('p', { class: 'muted' }, t('settings.data.restore.empty')),
      warningList(p.warnings),
      h('div', { class: 'setting-row-title mt-2' }, t('settings.data.restore.howTitle')),
      modes,
      hasBrowser ? h('label', { class: 'bk-check' }, prefsBox, h('span', null, t('settings.data.restore.prefs'))) : null,
      h('div', { class: 'bk-actions' },
        h('button', { type: 'button', class: 'btn btn-ghost', onClick: resetRestore }, t('common.cancel')),
        restoreBtn));
  }

  function confirmReplace() {
    const word = t('settings.data.replaceConfirm.word');
    return new Promise((resolve) => {
      let ok = false;
      const input = h('input', { class: 'input', type: 'text', autocomplete: 'off', spellcheck: 'false', 'aria-label': t('settings.data.replaceConfirm.label', { word }) });
      const body = h('div', { class: 'stack-sm' },
        h('p', null, t('settings.data.replaceConfirm.message')),
        h('label', { class: 'field' }, h('span', { class: 'label' }, t('settings.data.replaceConfirm.label', { word })), input));
      const m = modal({
        title: t('settings.data.replaceConfirm.title'),
        body,
        actions: [
          { label: t('common.cancel'), kind: 'ghost' },
          { label: t('settings.data.replaceConfirm.button'), kind: 'danger', onClick: () => {
            if (input.value.trim().toLocaleUpperCase() !== word.toLocaleUpperCase()) { input.closest('.field')?.classList.add('error'); input.focus(); return false; }
            ok = true;
            return true;
          } },
        ],
        onClose: () => { modals.delete(m); resolve(ok); },
      });
      modals.add(m);
      input.addEventListener('keydown', (e) => { if (e.key === 'Enter') m.el.querySelector('.btn-danger')?.click(); });
    });
  }

  async function runImport(mode, restorePrefs, btn) {
    if (!current) return;
    if (mode === 'replace' && !(await confirmReplace())) return;
    if (bag.disposed) return;
    btn.classList.add('loading');
    try {
      const q = mode === 'replace' ? '?mode=replace&confirm=replace' : '?mode=merge';
      const { data: rep } = await request('POST', `/api/backup/import${q}`, { body: current.file, signal });
      if (bag.disposed) return;
      const restored = restorePrefs ? restoreBrowserSettings(rep.browser) : 0;
      renderImportDone(rep, restored > 0);
      loadStatus();
    } catch (e) {
      if (!isAbortError(e)) toast(e.message || t('settings.data.errors.generic'), 'error');
    } finally {
      btn.classList.remove('loading');
    }
  }

  const summaryText = (rep) => t('settings.data.result.summary', { added: rep.inserted || 0, updated: rep.updated || 0, kept: rep.skipped || 0 });

  function renderImportDone(rep, needsReload) {
    current = null;
    const reloadBtn = h('button', { type: 'button', class: 'btn btn-primary', html: icon('refresh') + `<span>${t('settings.data.result.reload')}</span>`, onClick: () => location.reload() });
    fill(restorePanel,
      h('div', { class: 'callout callout-success' }, h('span', { class: 'bk-ci', html: icon('check-circle') }),
        h('div', null, h('strong', null, rep.mode === 'replace' ? t('settings.data.result.replaced') : t('settings.data.result.merged')),
          h('div', { class: 'text-sm' }, summaryText(rep)),
          needsReload ? h('div', { class: 'text-sm' }, t('settings.data.result.prefsRestored')) : null)),
      warningList(rep.warnings),
      h('div', { class: 'bk-actions' },
        h('button', { type: 'button', class: 'btn btn-ghost', onClick: resetRestore }, t('settings.data.result.another')),
        reloadBtn));
  }

  // ---- Sync: share this device --------------------------------------------------------
  const shareBody = h('div', { class: 'stack-sm' });
  let countdown = 0;
  const stopCountdown = () => { if (countdown) { clearInterval(countdown); countdown = 0; } };
  bag.add(stopCountdown);

  const renderShareIdle = (info) => {
    stopCountdown();
    const btn = h('button', { type: 'button', class: 'btn btn-secondary', html: icon('link') + `<span>${t('settings.data.sync.share.button')}</span>` });
    btn.addEventListener('click', async () => {
      btn.classList.add('loading');
      try {
        const { data } = await request('POST', '/api/sync/pair', { signal });
        if (!bag.disposed) renderShareActive(data);
      } catch (e) {
        if (!isAbortError(e)) toast(e.message, 'error');
        btn.classList.remove('loading');
      }
    });
    fill(shareBody, h('p', { class: 'setting-row-desc' }, t('settings.data.sync.share.desc')), networkNote(info), h('div', null, btn));
  };

  const networkNote = (info) => (info && info.network_visible === false
    ? h('div', { class: 'callout bk-note' }, h('span', { class: 'bk-ci', html: icon('wifi-off') }),
      h('div', { class: 'text-sm' }, t('settings.data.sync.share.localOnly'), ' ', h('code', null, 'GM_HOST=0.0.0.0')))
    : null);

  const renderShareActive = (info) => {
    stopCountdown();
    let left = Math.max(0, Number(info.expires_in) || 0);
    const timer = h('span', { class: 'tabular' });
    const tick = () => {
      const m = Math.floor(left / 60);
      const s = String(left % 60).padStart(2, '0');
      timer.textContent = t('settings.data.sync.share.expires', { time: `${m}:${s}` });
      if (left <= 0) { renderShareIdle(info); return; }
      left -= 1;
    };
    const stopBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('x-circle', { size: 16 }) + `<span>${t('settings.data.sync.share.stop')}</span>` });
    stopBtn.addEventListener('click', async () => {
      try { await request('DELETE', '/api/sync/pair', { signal }); } catch { /* ignore */ }
      if (!bag.disposed) renderShareIdle(info);
    });
    const here = location.origin;
    fill(shareBody,
      h('p', { class: 'setting-row-desc' }, t('settings.data.sync.share.enterCode')),
      h('div', { class: 'bk-code', 'aria-label': t('settings.data.sync.share.codeAria') }, info.code),
      h('div', { class: 'row-sm muted text-sm bk-code-meta' }, h('span', { html: icon('timer', { size: 16 }) }), timer, stopBtn),
      h('div', { class: 'setting-row-desc' }, t('settings.data.sync.share.address'), ' ', h('code', null, here)),
      /localhost|127\.0\.0\.1|\[::1\]/.test(here) ? h('div', { class: 'setting-row-desc subtle text-xs' }, t('settings.data.sync.share.addressHint')) : null,
      networkNote(info));
    tick();
    countdown = setInterval(tick, 1000);
  };

  const loadPair = async () => {
    try {
      const { data } = await request('GET', '/api/sync/pair', { signal });
      if (bag.disposed) return;
      if (data?.active) renderShareActive(data); else renderShareIdle(data);
    } catch (e) {
      if (!isAbortError(e) && !bag.disposed) renderShareIdle(null);
    }
  };

  // ---- Sync: connect to another device ------------------------------------------------
  const prefs = readSyncPrefs();
  const addrInput = h('input', { class: 'input', type: 'url', inputmode: 'url', placeholder: 'http://192.168.1.20:8080', value: prefs.url || '', autocomplete: 'off', spellcheck: 'false' });
  const codeInput = h('input', { class: 'input bk-code-input', type: 'text', placeholder: 'ABC-234', maxlength: '9', autocomplete: 'off', spellcheck: 'false', autocapitalize: 'characters' });
  let direction = 'both';
  const dirSeg = h('div', { class: 'segmented block bk-dir', role: 'radiogroup', 'aria-label': t('settings.data.sync.connect.direction') },
    [['both', 'settings.data.sync.connect.both'], ['pull', 'settings.data.sync.connect.pull'], ['push', 'settings.data.sync.connect.push']]
      .map(([v, k]) => h('button', { type: 'button', role: 'radio', dataset: { v }, class: v === direction ? 'active' : null, 'aria-checked': String(v === direction) }, t(k))));
  bag.on(dirSeg, 'click', (e) => {
    const b = e.target.closest('button[data-v]');
    if (!b) return;
    direction = b.dataset.v;
    for (const c of dirSeg.children) { const on = c.dataset.v === direction; c.classList.toggle('active', on); c.setAttribute('aria-checked', String(on)); }
  });
  const syncBtn = h('button', { type: 'button', class: 'btn btn-primary', html: icon('refresh') + `<span>${t('settings.data.sync.connect.go')}</span>` });
  const syncLog = h('div', { class: 'bk-sync-log', 'aria-live': 'polite' });

  const step = (key, params) => {
    const body = h('span', { class: 'bk-step-text' }, h('span', null, t(key, params)));
    const li = h('div', { class: 'bk-step' }, h('span', { class: 'spinner bk-step-icon' }), body);
    syncLog.appendChild(li);
    return {
      done(text) { li.firstChild.replaceWith(h('span', { class: 'bk-step-icon ok', html: icon('check', { size: 16 }) })); if (text) body.appendChild(h('span', { class: 'muted text-xs' }, text)); },
      fail() { li.firstChild.replaceWith(h('span', { class: 'bk-step-icon bad', html: icon('x', { size: 16 }) })); },
    };
  };

  bag.on(syncBtn, 'click', async () => {
    const base = normalizeAddress(addrInput.value);
    const code = codeInput.value.trim();
    if (!base) { toast(t('settings.data.errors.badAddress'), 'warning'); addrInput.focus(); return; }
    if (base === location.origin) { toast(t('settings.data.errors.sameDevice'), 'warning'); return; }
    if (code.replace(/[^a-z0-9]/gi, '').length < 4) { toast(t('settings.data.errors.needCode'), 'warning'); codeInput.focus(); return; }
    writeSyncPrefs({ url: base });
    addrInput.value = base;
    syncBtn.classList.add('loading');
    syncLog.replaceChildren();
    const remote = { headers: { 'X-GM-Pair': code }, signal, networkError: t('settings.data.errors.remoteNetwork') };
    let current = null;
    try {
      if (direction !== 'push') {
        current = step('settings.data.sync.steps.download');
        const snap = await request('GET', `${base}/api/sync/snapshot`, { ...remote, as: 'text' });
        current.done(formatBytes(snap.text.length));
        current = step('settings.data.sync.steps.mergeHere');
        const { data: rep } = await request('POST', '/api/backup/import?mode=merge&source=sync', { body: snap.text, signal });
        current.done(summaryText(rep));
      }
      if (direction !== 'pull') {
        current = step('settings.data.sync.steps.upload');
        const mine = await request('GET', '/api/backup/export?mark=false', { signal, as: 'text' });
        const { data: rep } = await request('POST', `${base}/api/sync/merge`, { ...remote, body: mine.text });
        current.done(summaryText(rep));
      }
      current = null;
      if (bag.disposed) return;
      syncLog.appendChild(h('div', { class: 'callout callout-success mt-2' }, h('span', { class: 'bk-ci', html: icon('check-circle') }), h('div', null, h('strong', null, t('settings.data.sync.done')))));
      loadStatus();
    } catch (e) {
      if (isAbortError(e) || bag.disposed) return;
      current?.fail();
      syncLog.appendChild(h('div', { class: 'callout callout-danger mt-2' }, h('span', { class: 'bk-ci', html: icon('x-circle') }), h('div', { class: 'text-sm' }, e.message || t('settings.data.errors.generic'))));
    } finally {
      syncBtn.classList.remove('loading');
    }
  });

  // ---- Layout -------------------------------------------------------------------------
  const el = h('section', { class: 'card bk-section', id: 'your-data' },
    h('div', { class: 'card-header' },
      h('div', { class: 'card-title', html: icon('shield') + `<span>${t('settings.data.title')}</span>` })),
    h('p', { class: 'setting-row-desc bk-intro' }, t('settings.data.intro')),
    reminder,
    h('div', { class: 'setting-row' },
      h('div', { class: 'setting-row-text' },
        h('div', { class: 'setting-row-title' }, t('settings.data.download.title')),
        h('div', { class: 'setting-row-desc' }, t('settings.data.download.desc')),
        statusText,
        statusChips),
      downloadBtn),
    h('div', { class: 'hub-setting-block' },
      h('div', { class: 'setting-row-text' },
        h('div', { class: 'setting-row-title' }, t('settings.data.restore.title')),
        h('div', { class: 'setting-row-desc' }, t('settings.data.restore.desc'))),
      drop,
      restorePanel),
    h('div', { class: 'hub-setting-block' },
      h('div', { class: 'setting-row-text' },
        h('div', { class: 'setting-row-title' }, t('settings.data.sync.title')),
        h('div', { class: 'setting-row-desc' }, t('settings.data.sync.desc'))),
      h('div', { class: 'bk-sync-grid' },
        h('div', { class: 'bk-sync-card' },
          h('div', { class: 'bk-sync-head', html: icon('wifi') + `<span>${t('settings.data.sync.share.title')}</span>` }),
          shareBody),
        h('div', { class: 'bk-sync-card' },
          h('div', { class: 'bk-sync-head', html: icon('link') + `<span>${t('settings.data.sync.connect.title')}</span>` }),
          h('p', { class: 'setting-row-desc' }, t('settings.data.sync.connect.desc')),
          h('label', { class: 'field' }, h('span', { class: 'label' }, t('settings.data.sync.connect.address')), addrInput),
          h('label', { class: 'field' }, h('span', { class: 'label' }, t('settings.data.sync.connect.code')), codeInput),
          dirSeg,
          h('div', null, syncBtn),
          syncLog)),
      h('p', { class: 'subtle text-xs' }, t('settings.data.sync.privacy'))));

  loadStatus();
  loadPair();

  return { el, destroy: bag.dispose };
}
