// GrandMentor PWA client: service-worker registration + update toast, offline banner,
// "Install app" entry (sidebar footer + mobile "More" sheet), idle prefetch of offline content
// and offline-queue notifications. Contract: docs/CONTRACT.md "Installable app (PWA)".
//
//   import { initPwa } from './pwa.js';
//   initPwa();          // once, after the shell is rendered (app.js boot)
//   destroyPwa();       // removes every listener/observer/timer (tests, hot reload)

import { h, icon, modal, mdLite, toast } from './ui.js';
import { t, onLanguageChange, getLanguage } from './i18n.js';

const HEALTH_URL = '/api/health';
const PROBE_TIMEOUT_MS = 4000;
const OFFLINE_PROBE_EVERY_MS = 15000;
const PREFETCH_DELAY_MS = 4000;
const QUEUE_TOAST_GAP_MS = 10000;

let ctx = null;

function isStandalone() {
  return window.matchMedia?.('(display-mode: standalone)').matches || navigator.standalone === true;
}

function isIos() {
  const ua = navigator.userAgent || '';
  return /iphone|ipad|ipod/i.test(ua) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
}

function swSupported() {
  if (!('serviceWorker' in navigator)) return false;
  return window.isSecureContext;
}

/** Initialise PWA features. Safe to call more than once. */
export function initPwa() {
  if (ctx) return;
  const ac = new AbortController();
  ctx = {
    ac,
    signal: ac.signal,
    observers: [],
    timers: new Set(),
    unsubs: [],
    deferredPrompt: null,
    installed: isStandalone(),
    offline: false,
    banner: document.getElementById('offline-banner'),
    probeTimer: null,
    updateEl: null,
    waiting: null,
    reloadRequested: false,
    lastQueueToast: 0,
    reg: null,
  };
  syncThemeColor();
  watchThemeColor();
  initOffline();
  initInstall();
  if (swSupported()) registerWorker();
  ctx.unsubs.push(onLanguageChange(() => {
    renderBanner();
    if (ctx.updateEl) renderUpdateToast();
    if (ctx.reg?.active) schedulePrefetch(false);
  }));
}

/** Tear everything down (listeners, observers, timers, injected nodes). */
export function destroyPwa() {
  if (!ctx) return;
  ctx.ac.abort();
  for (const o of ctx.observers) o.disconnect();
  for (const id of ctx.timers) { clearTimeout(id); clearInterval(id); }
  for (const u of ctx.unsubs) { try { u(); } catch { /* ignore */ } }
  if (ctx.probeTimer) clearInterval(ctx.probeTimer);
  document.querySelectorAll('.pwa-install-item').forEach((el) => el.remove());
  ctx.updateEl?.remove();
  if (ctx.banner) { ctx.banner.hidden = true; ctx.banner.replaceChildren(); }
  ctx = null;
}

function later(fn, ms) {
  const id = setTimeout(() => { ctx?.timers.delete(id); fn(); }, ms);
  ctx.timers.add(id);
  return id;
}

// ---------------------------------------------------------------------------
// Theme colour (browser chrome / installed app title bar follows the in-app theme)
// ---------------------------------------------------------------------------
function syncThemeColor() {
  const color = getComputedStyle(document.documentElement).getPropertyValue('--bg-elev').trim();
  if (!color) return;
  for (const meta of document.querySelectorAll('meta[name="theme-color"]')) meta.setAttribute('content', color);
}

function watchThemeColor() {
  const mo = new MutationObserver(() => syncThemeColor());
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  ctx.observers.push(mo);
}

// ---------------------------------------------------------------------------
// Connection status + banner
// ---------------------------------------------------------------------------
// Three states: 'ok', 'server' (this device is online but the GrandMentor server/engine doesn't
// answer — e.g. the service worker served the cached app while the server is stopped) and 'device'
// (navigator.onLine is false and the server doesn't answer). The truth comes from GET /api/health
// (never cached by the worker); api.js reports network failures of other calls as a hint
// (`gm:server-unreachable`), which triggers a probe instead of being trusted blindly. While not 'ok'
// we probe every OFFLINE_PROBE_EVERY_MS and the banner goes away by itself when the server answers.
// The banner sits in the normal flow at the top of the content (never over the board); while the
// server is up (normal use, the QA sweep) it is hidden and takes no space.
const HINT_PROBE_GAP_MS = 3000;

function initOffline() {
  const { signal } = ctx;
  ctx.status = 'ok';
  ctx.probing = false;
  ctx.lastHintProbe = 0;
  window.addEventListener('offline', () => probe(), { signal });
  window.addEventListener('online', () => probe(), { signal });
  // A failed API call: check now (throttled) rather than wait for the next scheduled probe.
  window.addEventListener('gm:server-unreachable', () => hintProbe(), { signal });
  // A successful API call while we think we're down: the server may be back (responses from the
  // worker's cache look successful too, so confirm with a health probe).
  window.addEventListener('gm:server-reachable', () => { if (ctx && ctx.status !== 'ok') hintProbe(); }, { signal });
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && ctx && ctx.status !== 'ok') probe();
  }, { signal });
  later(probe, navigator.onLine === false ? 0 : 1500);
}

function hintProbe() {
  if (!ctx) return;
  const now = Date.now();
  if (now - ctx.lastHintProbe < HINT_PROBE_GAP_MS) return;
  ctx.lastHintProbe = now;
  probe();
}

async function probe() {
  if (!ctx || ctx.probing) return;
  ctx.probing = true;
  let ok = false;
  const ac = new AbortController();
  const stop = () => ac.abort();
  const timer = setTimeout(stop, PROBE_TIMEOUT_MS);
  const { signal } = ctx;
  signal.addEventListener('abort', stop, { once: true });
  try {
    // Checked even when navigator.onLine is false: the server usually runs on this same
    // computer (localhost) and keeps working without internet.
    const res = await fetch(HEALTH_URL, { cache: 'no-store', signal: ac.signal });
    ok = res.ok;
  } catch {
    ok = false;
  } finally {
    clearTimeout(timer);
    signal.removeEventListener('abort', stop);
    if (ctx) ctx.probing = false;
  }
  if (!ctx || signal.aborted) return;
  setStatus(ok ? 'ok' : (navigator.onLine === false ? 'device' : 'server'));
}

/** Current connection status: 'ok' | 'server' | 'device' (docs/CONTRACT.md "Installable app"). */
export function connectionStatus() {
  return ctx ? ctx.status : 'ok';
}

function setStatus(status) {
  if (!ctx) return;
  const was = ctx.status;
  ctx.status = status;
  const offline = status !== 'ok';
  ctx.offline = offline;
  const root = document.documentElement;
  root.classList.toggle('is-offline', offline);
  if (offline) root.dataset.connection = status; else delete root.dataset.connection;
  if (offline && !ctx.probeTimer) {
    ctx.probeTimer = setInterval(probe, OFFLINE_PROBE_EVERY_MS);
  } else if (!offline && ctx.probeTimer) {
    clearInterval(ctx.probeTimer);
    ctx.probeTimer = null;
  }
  if (status === was) return;
  renderBanner();
  // Let the shell (engine status dot) and pages react.
  window.dispatchEvent(new CustomEvent('gm:connection', { detail: { status, previous: was } }));
  if (!offline) {
    postToWorker({ type: 'replay' });
    toast(t('pwa.offline.backOnline'), 'success', { duration: 2500 });
  }
}

function renderBanner() {
  const el = ctx?.banner;
  if (!el) return;
  if (ctx.status === 'ok') {
    el.hidden = true;
    el.replaceChildren();
    return;
  }
  const device = ctx.status === 'device';
  const key = device ? 'pwa.connection.device' : 'pwa.connection.server';
  el.className = `offline-banner ${device ? 'is-device' : 'is-server'}`;
  el.hidden = false;
  const retry = h('button', { class: 'btn btn-secondary btn-sm offline-banner-retry', type: 'button', 'aria-label': t('pwa.offline.retryLabel'), onClick: () => probe() }, t('pwa.offline.retry'));
  el.replaceChildren(
    h('span', { class: 'offline-banner-icon', html: icon(device ? 'wifi-off' : 'alert') }),
    h('div', { class: 'offline-banner-text' },
      h('div', { class: 'offline-banner-title' }, t(`${key}.title`)),
      h('div', { class: 'offline-banner-desc' }, t(`${key}.text`))),
    retry);
}

// ---------------------------------------------------------------------------
// Install entry
// ---------------------------------------------------------------------------
function initInstall() {
  const { signal } = ctx;
  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    ctx.deferredPrompt = e;
    refreshInstallEntries();
  }, { signal });
  window.addEventListener('appinstalled', () => {
    ctx.installed = true;
    ctx.deferredPrompt = null;
    refreshInstallEntries();
    navigator.storage?.persist?.().catch(() => {});
    schedulePrefetch(true);
  }, { signal });
  const mq = window.matchMedia?.('(display-mode: standalone)');
  mq?.addEventListener?.('change', () => { ctx.installed = isStandalone(); refreshInstallEntries(); }, { signal });

  // app.js re-renders the sidebar and the "More" sheet (e.g. on a language switch): re-inject.
  for (const id of ['sidebar', 'more-sheet']) {
    const host = document.getElementById(id);
    if (!host) continue;
    const mo = new MutationObserver(() => refreshInstallEntries());
    mo.observe(host, { childList: true });
    ctx.observers.push(mo);
  }
  refreshInstallEntries();
}

function canInstall() {
  if (!ctx || ctx.installed) return false;
  return !!ctx.deferredPrompt || isIos();
}

function installButton(extraClass) {
  return h('button', { class: `nav-item pwa-install-item ${extraClass}`, type: 'button', onClick: onInstallClick, 'aria-label': t('pwa.install.title') },
    h('span', { html: icon('download'), style: 'display:contents' }),
    h('span', { class: 'nav-label' }, t('pwa.install.button')));
}

function refreshInstallEntries() {
  if (!ctx) return;
  const show = canInstall();
  const footer = document.querySelector('#sidebar .sidebar-footer');
  const panel = document.querySelector('#more-sheet .more-sheet-panel');
  for (const [host, cls] of [[footer, 'pwa-install-side'], [panel, 'pwa-install-sheet']]) {
    if (!host) continue;
    const existing = host.querySelector('.pwa-install-item');
    if (!show) { existing?.remove(); continue; }
    if (existing) continue;
    // Sidebar: first footer row (above the theme toggle). Sheet: last tile.
    if (cls === 'pwa-install-side') host.prepend(installButton(cls));
    else host.append(installButton(cls));
  }
}

async function onInstallClick() {
  if (!ctx) return;
  const prompt = ctx.deferredPrompt;
  if (prompt) {
    ctx.deferredPrompt = null;
    try {
      await prompt.prompt();
      const choice = await prompt.userChoice;
      if (choice?.outcome === 'accepted') ctx.installed = true;
    } catch (e) {
      console.warn('[pwa] install prompt failed', e);
    }
    refreshInstallEntries();
    return;
  }
  if (isIos()) showIosHelp();
}

function showIosHelp() {
  const steps = [
    ['share', t('pwa.install.iosStep1')],
    ['plus', t('pwa.install.iosStep2')],
    ['check-circle', t('pwa.install.iosStep3')],
  ];
  const body = h('div', { class: 'pwa-ios-help' },
    h('p', { class: 'muted' }, t('pwa.install.iosIntro')),
    h('ol', { class: 'pwa-ios-steps' }, steps.map(([ic, text], i) =>
      h('li', null,
        h('span', { class: 'pwa-ios-num' }, String(i + 1)),
        h('span', { class: 'pwa-ios-icon', html: icon(ic) }),
        h('span', { class: 'pwa-ios-text', html: mdLite(text).replace(/^<p>|<\/p>$/g, '') })))));
  modal({ title: t('pwa.install.iosTitle'), body, actions: [{ label: t('pwa.install.gotIt'), kind: 'primary', autofocus: true }] });
}

// ---------------------------------------------------------------------------
// Service worker: registration, update flow, messages, prefetch
// ---------------------------------------------------------------------------
async function registerWorker() {
  const { signal } = ctx;
  const sw = navigator.serviceWorker;
  sw.addEventListener('message', onWorkerMessage, { signal });
  sw.addEventListener('controllerchange', () => {
    if (ctx?.reloadRequested) location.reload();
  }, { signal });

  let reg;
  try {
    reg = await sw.register('/sw.js', { scope: '/', updateViaCache: 'none' });
  } catch (e) {
    console.warn('[pwa] service worker registration failed', e);
    return;
  }
  if (!ctx || signal.aborted) return;
  ctx.reg = reg;

  const track = (worker) => {
    if (!worker) return;
    worker.addEventListener('statechange', () => {
      if (worker.state === 'installed' && navigator.serviceWorker.controller) showUpdate(worker);
    }, { signal });
  };
  if (reg.waiting && sw.controller) showUpdate(reg.waiting);
  track(reg.installing);
  reg.addEventListener('updatefound', () => track(reg.installing), { signal });

  // Look for updates when the tab comes back into view (bounded: at most every 30 min).
  let lastCheck = Date.now();
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible' || Date.now() - lastCheck < 30 * 60 * 1000) return;
    lastCheck = Date.now();
    reg.update().catch(() => {});
  }, { signal });

  await sw.ready;
  if (!ctx || signal.aborted) return;
  postToWorker({ type: 'replay' });
  schedulePrefetch(false);
}

function postToWorker(msg) {
  const target = navigator.serviceWorker?.controller || ctx?.reg?.active;
  try { target?.postMessage(msg); } catch { /* ignore */ }
}

/** Ask the worker to cache courses/lessons and a puzzle pack once the app is idle. */
function schedulePrefetch(force) {
  if (!ctx) return;
  const saveData = navigator.connection?.saveData;
  if (saveData && !force && !ctx.installed) return;
  later(() => {
    if (!ctx || ctx.offline) return;
    const run = () => { if (ctx && !ctx.offline) postToWorker({ type: 'prefetch', lang: getLanguage(), force: !!force }); };
    if ('requestIdleCallback' in window) {
      const id = requestIdleCallback(run, { timeout: 10000 });
      ctx.unsubs.push(() => cancelIdleCallback(id));
    } else run();
  }, PREFETCH_DELAY_MS);
}

function onWorkerMessage(event) {
  const msg = event.data;
  if (!ctx || !msg || msg.source !== 'gm-sw') return;
  if (msg.type === 'queued') {
    const now = Date.now();
    if (now - ctx.lastQueueToast < QUEUE_TOAST_GAP_MS) return;
    ctx.lastQueueToast = now;
    toast(t('pwa.queue.saved'), 'info');
  } else if (msg.type === 'replayed' && msg.count > 0) {
    toast(t('pwa.queue.synced', { count: msg.count }), 'success');
  } else if (msg.type === 'prefetched') {
    console.debug('[pwa] offline content ready', msg);
  }
}

function showUpdate(worker) {
  if (!ctx) return;
  ctx.waiting = worker;
  renderUpdateToast();
}

function renderUpdateToast() {
  if (!ctx || !ctx.waiting) return;
  const host = document.getElementById('toasts');
  if (!host) return;
  const el = ctx.updateEl || h('div', { class: 'toast toast-info pwa-update', role: 'status' });
  const reload = h('button', { class: 'btn btn-primary btn-sm', type: 'button', onClick: () => {
    if (!ctx?.waiting) return;
    ctx.reloadRequested = true;
    reload.disabled = true;
    ctx.waiting.postMessage({ type: 'skipWaiting' });
    // If the new worker never takes control (e.g. another tab holds it), reload anyway.
    later(() => location.reload(), 3000);
  } }, t('pwa.update.reload'));
  const close = h('button', { class: 'toast-close', type: 'button', 'aria-label': t('common.dismiss'), html: icon('close'), onClick: () => {
    el.remove();
    if (ctx) ctx.updateEl = null;
  } });
  el.innerHTML = icon('sparkles');
  el.append(
    h('div', { class: 'toast-msg' },
      h('div', { class: 'pwa-update-title' }, t('pwa.update.title')),
      h('div', { class: 'pwa-update-text' }, t('pwa.update.text')),
      h('div', { class: 'pwa-update-actions' }, reload)),
    close);
  if (!el.isConnected) host.appendChild(el);
  ctx.updateEl = el;
}
