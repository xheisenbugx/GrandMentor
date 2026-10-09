// GrandMentor app shell + hash router.
// Contract: docs/CONTRACT.md §5. Pages live in ./pages/*.js and export
//   mount(root, { params, query, path }) -> cleanup fn (may be async), and optional `title`.

import { h, icon, brandMark, toast, closeAllModals, escapeHtml, tOr } from './ui.js';
import { announce, clearAnnouncements } from './components/announcer.js';
import { api } from './api.js';
import { getSettings, setSetting, onSettingsChange } from './settings.js';
import { initI18n, onLanguageChange, t } from './i18n.js';
import { initPwa } from './pwa.js';

// ---------------------------------------------------------------------------
// Navigation model (labels are i18n keys nav.<key>, resolved at render time)
// ---------------------------------------------------------------------------
const NAV = [
  { key: 'home', icon: 'home', href: '#/', primary: true },
  { key: 'play', icon: 'play', href: '#/play', primary: true },
  { key: 'puzzles', icon: 'puzzle', href: '#/puzzles', primary: true },
  { key: 'learn', icon: 'learn', href: '#/learn', primary: true },
  { key: 'openings', icon: 'openings', href: '#/openings' },
  { key: 'repertoire', icon: 'book', href: '#/repertoire' },
  { key: 'endgames', icon: 'endgames', href: '#/endgames' },
  { key: 'analysis', icon: 'analysis', href: '#/analysis', primary: true },
  { key: 'library', icon: 'library', href: '#/library' },
  { key: 'insights', icon: 'chart', href: '#/insights', section: 'you' },
  { key: 'profile', icon: 'profile', href: '#/profile', section: 'you' },
  { key: 'settings', icon: 'settings', href: '#/settings', section: 'you' },
];

// Route table: first match wins. `nav` = sidebar item to highlight. `titleKey` = i18n key of the tab title.
const ROUTES = [
  { pattern: '/', page: 'home', nav: 'home', titleKey: 'nav.routes.home' },
  { pattern: '/play', page: 'play', nav: 'play', titleKey: 'nav.routes.play' },
  { pattern: '/play/:botId', page: 'play', nav: 'play', titleKey: 'nav.routes.play' },
  { pattern: '/analysis', page: 'analysis', nav: 'analysis', titleKey: 'nav.routes.analysis' },
  // Game Review is engine analysis of a finished game, so it lights up "Analysis" on purpose, whether it was
  // opened from Play, Home or the Library: one stable highlight instead of one that depends on history.
  { pattern: '/review/:gameId', page: 'review', nav: 'analysis', titleKey: 'nav.routes.review' },
  { pattern: '/puzzles/rush', page: 'puzzles', nav: 'puzzles', titleKey: 'nav.routes.puzzleRush', params: { mode: 'rush' } },
  { pattern: '/puzzles/daily', page: 'puzzles', nav: 'puzzles', titleKey: 'nav.routes.dailyPuzzle', params: { mode: 'daily' } },
  { pattern: '/puzzles/mistakes', page: 'puzzles', nav: 'puzzles', titleKey: 'nav.routes.mistakes', params: { mode: 'mistakes' } },
  { pattern: '/puzzles', page: 'puzzles', nav: 'puzzles', titleKey: 'nav.routes.puzzles' },
  { pattern: '/local', page: 'local', nav: 'play', titleKey: 'nav.routes.local' },
  { pattern: '/drills', page: 'drills', nav: 'learn', titleKey: 'nav.routes.drills' },
  { pattern: '/drills/:drillId', page: 'drills', nav: 'learn', titleKey: 'nav.routes.drills' },
  { pattern: '/classics', page: 'classics', nav: 'learn', titleKey: 'nav.routes.classics' },
  { pattern: '/classics/:classicId', page: 'classics', nav: 'learn', titleKey: 'nav.routes.classics' },
  { pattern: '/repertoire', page: 'repertoire', nav: 'repertoire', titleKey: 'nav.routes.repertoire' },
  { pattern: '/repertoire/:side', page: 'repertoire', nav: 'repertoire', titleKey: 'nav.routes.repertoire' },
  { pattern: '/insights', page: 'insights', nav: 'insights', titleKey: 'nav.routes.insights' },
  { pattern: '/learn', page: 'learn', nav: 'learn', titleKey: 'nav.routes.learn' },
  { pattern: '/learn/:courseId', page: 'learn', nav: 'learn', titleKey: 'nav.routes.learn' },
  { pattern: '/learn/:courseId/:lessonId', page: 'lesson', nav: 'learn', titleKey: 'nav.routes.lesson' },
  { pattern: '/openings', page: 'openings', nav: 'openings', titleKey: 'nav.routes.openings' },
  { pattern: '/openings/:id', page: 'openings', nav: 'openings', titleKey: 'nav.routes.openings' },
  { pattern: '/endgames', page: 'endgames', nav: 'endgames', titleKey: 'nav.routes.endgames' },
  { pattern: '/endgames/:id', page: 'endgames', nav: 'endgames', titleKey: 'nav.routes.endgames' },
  { pattern: '/library', page: 'library', nav: 'library', titleKey: 'nav.routes.library' },
  { pattern: '/profile', page: 'profile', nav: 'profile', titleKey: 'nav.routes.profile' },
  { pattern: '/settings', page: 'settings', nav: 'settings', titleKey: 'nav.routes.settings' },
].map((r) => ({ ...r, segments: r.pattern.split('/').filter(Boolean) }));

const APP_NAME = 'GrandMentor';

/** Parse location.hash into { path, segments, query }. */
export function parseHash(hash = location.hash) {
  let raw = String(hash || '').replace(/^#/, '');
  if (!raw.startsWith('/')) raw = '/' + raw;
  const qi = raw.indexOf('?');
  const pathPart = qi >= 0 ? raw.slice(0, qi) : raw;
  const queryPart = qi >= 0 ? raw.slice(qi + 1) : '';
  const segments = pathPart.split('/').filter(Boolean).map(safeDecode);
  const query = {};
  for (const [k, v] of new URLSearchParams(queryPart)) query[k] = v;
  return { path: '/' + segments.join('/'), segments, query };
}

function safeDecode(s) {
  try { return decodeURIComponent(s); } catch { return s; }
}

/** Match parsed segments against the route table. */
export function matchRoute(segments) {
  for (const r of ROUTES) {
    if (r.segments.length !== segments.length) continue;
    const params = {};
    let ok = true;
    for (let i = 0; i < r.segments.length; i++) {
      const p = r.segments[i];
      if (p.startsWith(':')) params[p.slice(1)] = segments[i];
      else if (p !== segments[i]) { ok = false; break; }
    }
    if (ok) return { route: r, params: { ...(r.params || {}), ...params } };
  }
  return null;
}

/** Navigate programmatically: navigate('/play/martin') or navigate('#/library'). */
export function navigate(path, { replace = false } = {}) {
  const target = path.startsWith('#') ? path : '#' + (path.startsWith('/') ? path : '/' + path);
  if (replace) {
    history.replaceState(null, '', target);
    handleRoute();
  } else if (location.hash === target) {
    handleRoute();
  } else {
    location.hash = target;
  }
}

// ---------------------------------------------------------------------------
// Shell rendering
// ---------------------------------------------------------------------------
const els = {};

const navLabel = (item) => t(`nav.${item.key}`);

function navLink(item, cls = 'nav-item') {
  return h('a', { class: cls, href: item.href, dataset: { nav: item.key }, 'aria-label': navLabel(item) },
    h('span', { html: icon(item.icon), style: 'display:contents' }),
    h('span', { class: 'nav-label' }, navLabel(item)));
}

/** Localize the static landmarks in index.html (they live outside the re-rendered shell). */
function localizeStatic() {
  els.sidebar.setAttribute('aria-label', t('nav.mainNavigation'));
  els.bottombar.setAttribute('aria-label', t('nav.mainNavigation'));
  const skip = document.querySelector('a.skip-link, a.sr-only[href="#view"]');
  if (skip) skip.textContent = t('nav.skipToContent');
  const desc = document.querySelector('meta[name="description"]');
  if (desc) desc.setAttribute('content', t('nav.metaDescription'));
}

function renderShell() {
  els.app = document.getElementById('app');
  els.sidebar = document.getElementById('sidebar');
  els.view = document.getElementById('view');
  els.bottombar = document.getElementById('bottombar');
  els.sheet = document.getElementById('more-sheet');
  els.progress = document.getElementById('route-progress');
  localizeStatic();

  // Sidebar
  const brand = h('a', { class: 'brand', href: '#/', 'aria-label': t('nav.brandHome', { app: APP_NAME }), html: brandMark(36) },
    h('span', { class: 'brand-name' }, 'Grand', h('span', null, 'Mentor')));
  const mainNav = h('nav', { class: 'nav' }, NAV.filter((n) => !n.section).map((n) => navLink(n)));
  const youNav = h('nav', { class: 'nav' },
    h('div', { class: 'nav-section-label' }, t('nav.you')),
    NAV.filter((n) => n.section === 'you').map((n) => navLink(n)));

  els.statusDot = h('span', { class: 'status-dot' });
  els.statusText = h('span', { class: 'nav-label' }, t('nav.status.connecting'));
  const status = h('div', { class: 'engine-status', title: t('nav.status.title') }, els.statusDot, els.statusText);

  els.themeBtn = h('button', { class: 'nav-item', type: 'button', style: 'border:0;background:none;width:100%;text-align:left', onClick: toggleTheme });
  els.collapseBtn = h('button', { class: 'nav-item collapse-btn hide-mobile', type: 'button', style: 'border:0;background:none;width:100%;text-align:left', onClick: toggleSidebar },
    h('span', { html: icon('sidebar'), style: 'display:contents' }), h('span', { class: 'nav-label' }, t('nav.collapse')));
  const footer = h('div', { class: 'sidebar-footer' }, els.themeBtn, els.collapseBtn, status);

  els.sidebar.replaceChildren(brand, mainNav, youNav, footer);

  // Mobile bottom bar: primary items + "More"
  const tabs = NAV.filter((n) => n.primary).map((n) =>
    h('a', { class: 'tab-item', href: n.href, dataset: { nav: n.key } },
      h('span', { html: icon(n.icon), style: 'display:contents' }), h('span', null, navLabel(n))));
  els.moreBtn = h('button', { class: 'tab-item', type: 'button', 'aria-haspopup': 'true', 'aria-expanded': 'false', onClick: () => toggleSheet() },
    h('span', { html: icon('more'), style: 'display:contents' }), h('span', null, t('nav.more')));
  els.bottombar.replaceChildren(...tabs, els.moreBtn);

  // "More" sheet
  const sheetItems = NAV.filter((n) => !n.primary).map((n) => navLink(n));
  const sheetTheme = h('button', { class: 'nav-item', type: 'button', style: 'border:0', onClick: () => { toggleTheme(); } });
  els.sheetTheme = sheetTheme;
  const panel = h('nav', { class: 'more-sheet-panel', id: 'more-sheet-panel', 'aria-label': t('nav.more') }, sheetItems, sheetTheme);
  els.sheet.replaceChildren(panel);
  els.moreBtn.setAttribute('aria-controls', 'more-sheet-panel');
  els.sheet.inert = !els.sheet.classList.contains('open');
  syncShellSettings(getSettings());

  // renderShell() runs again on every language switch: install global listeners only once.
  if (shellListenersInstalled) return;
  shellListenersInstalled = true;
  els.sheet.addEventListener('click', (e) => {
    if (e.target === els.sheet || e.target.closest('a')) toggleSheet(false);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && els.sheet.classList.contains('open')) { toggleSheet(false); els.moreBtn?.focus(); }
  });
  // Skip link: the hash router owns location.hash, so move focus instead of navigating to "#view".
  const skip = document.querySelector('a.skip-link, a.sr-only[href="#view"]');
  if (skip) {
    skip.addEventListener('click', (e) => {
      e.preventDefault();
      const target = els.view.querySelector('h1') || els.view;
      if (target !== els.view && !target.hasAttribute('tabindex')) target.setAttribute('tabindex', '-1');
      try { target.focus(); } catch { /* ignore */ }
    });
  }
  onSettingsChange((s, key) => {
    if (key === 'theme' || key === 'sidebarCollapsed') syncShellSettings(s);
  });
}
let shellListenersInstalled = false;

function syncShellSettings(s) {
  els.app.classList.toggle('sidebar-collapsed', !!s.sidebarCollapsed);
  const isLight = s.theme === 'light';
  const themeContent = [h('span', { html: icon(isLight ? 'moon' : 'sun'), style: 'display:contents' }), h('span', { class: 'nav-label' }, isLight ? t('nav.theme.darkMode') : t('nav.theme.lightMode'))];
  els.themeBtn.replaceChildren(...themeContent);
  els.themeBtn.setAttribute('aria-label', isLight ? t('nav.theme.switchToDark') : t('nav.theme.switchToLight'));
  els.sheetTheme.replaceChildren(h('span', { html: icon(isLight ? 'moon' : 'sun'), style: 'display:contents' }), h('span', { class: 'nav-label' }, isLight ? t('nav.theme.dark') : t('nav.theme.light')));
  els.collapseBtn.querySelector('.nav-label').textContent = s.sidebarCollapsed ? t('nav.expand') : t('nav.collapse');
}

function toggleTheme() {
  setSetting('theme', getSettings().theme === 'light' ? 'dark' : 'light');
}

function toggleSidebar() {
  setSetting('sidebarCollapsed', !getSettings().sidebarCollapsed);
  // Board layouts depend on the sidebar width; let components re-measure.
  requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
}

function toggleSheet(force) {
  const open = typeof force === 'boolean' ? force : !els.sheet.classList.contains('open');
  els.sheet.classList.toggle('open', open);
  els.sheet.setAttribute('aria-hidden', open ? 'false' : 'true');
  els.sheet.inert = !open; // closed sheet links must not be reachable with Tab
  els.moreBtn.setAttribute('aria-expanded', open ? 'true' : 'false');
  els.moreBtn.classList.toggle('active', open);
  if (open) {
    const first = els.sheet.querySelector('a, button');
    if (first) requestAnimationFrame(() => { try { first.focus({ preventScroll: true }); } catch { /* ignore */ } });
  }
}

function setActiveNav(key) {
  for (const a of document.querySelectorAll('[data-nav]')) {
    const on = a.dataset.nav === key;
    a.classList.toggle('active', on);
    if (on) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current');
  }
  // "More" tab lights up when a secondary page is active.
  const secondary = NAV.find((n) => n.key === key && !n.primary);
  if (els.moreBtn) els.moreBtn.classList.toggle('active', !!secondary);
}

// Engine/server status indicator
let healthBusy = false;
let healthOk = null;
async function checkHealth() {
  if (healthBusy) return;
  healthBusy = true;
  try {
    const hres = await api.get('/api/health', { timeout: 5000 });
    healthOk = true;
    els.statusDot.className = 'status-dot ok';
    els.statusText.textContent = hres && hres.engines ? t('nav.status.readyThreads', { count: hres.engines }) : t('nav.status.ready');
    els.statusDot.parentElement.title = hres && hres.llm_enabled ? t('nav.status.onlineAi') : t('nav.status.onlineCoach');
  } catch {
    healthOk = false;
    els.statusDot.className = 'status-dot bad';
    els.statusText.textContent = t('nav.status.offline');
    els.statusDot.parentElement.title = t('nav.status.offlineTitle', { app: APP_NAME });
  } finally {
    healthBusy = false;
  }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
let currentCleanup = null;
let routedOnce = false;
let navToken = 0;
let progressTimer = null;

async function runCleanup() {
  const fn = currentCleanup;
  currentCleanup = null;
  if (typeof fn === 'function') {
    try { await fn(); } catch (e) { console.error('[router] page cleanup failed', e); }
  }
}

function showProgress(on) {
  if (progressTimer) { clearTimeout(progressTimer); progressTimer = null; }
  if (on) progressTimer = setTimeout(() => els.progress.classList.add('active'), 120);
  else els.progress.classList.remove('active');
}

function setTitle(text) {
  document.title = text ? `${text} · ${APP_NAME}` : t('nav.defaultTitle', { app: APP_NAME });
}

async function handleRoute() {
  const token = ++navToken;
  const { path, segments, query } = parseHash();
  const match = matchRoute(segments);

  toggleSheet(false);
  closeAllModals();
  clearAnnouncements();
  await runCleanup();
  if (token !== navToken) return;

  if (healthOk === false) checkHealth();

  const host = h('div', { class: 'page-host page-enter' });
  els.view.replaceChildren(host);
  window.scrollTo(0, 0);

  if (!match) {
    setActiveNav(null);
    setTitle(t('nav.notFound.title'));
    renderNotFound(host, path);
    return;
  }

  const { route, params } = match;
  setActiveNav(route.nav);
  setTitle(t(route.titleKey));
  // Tell screen-reader users that the page changed (the hash router doesn't reload the document).
  if (routedOnce) announce(t(route.titleKey));
  routedOnce = true;
  showProgress(true);

  try {
    const mod = await import(`./pages/${route.page}.js`);
    if (token !== navToken) return;
    if (typeof mod.mount !== 'function') throw new Error(`Page "${route.page}" has no mount() export`);
    if (mod.title) setTitle(typeof mod.title === 'function' ? mod.title(params, query) : mod.title);

    const cleanup = await mod.mount(host, { params, query, path });
    if (token !== navToken) {
      // A newer navigation started while mounting: discard this page immediately.
      if (typeof cleanup === 'function') { try { await cleanup(); } catch (e) { console.error(e); } }
      return;
    }
    currentCleanup = typeof cleanup === 'function' ? cleanup : null;
  } catch (err) {
    if (token !== navToken) return;
    console.error(`[router] failed to load ${route.page}`, err);
    // Best effort: the failing page may have partially rendered.
    host.replaceChildren();
    renderError(host, err, route);
  } finally {
    if (token === navToken) showProgress(false);
  }
}

function renderError(host, err, route) {
  const details = err && (err.stack || err.message) ? String(err.stack || err.message) : String(err);
  const card = h('div', { class: 'page' },
    h('div', { class: 'card placeholder-card error-card' },
      h('div', { class: 'empty-state-icon', html: icon('alert') }),
      h('h2', { class: 'mb-2' }, t('nav.error.title')),
      h('p', { class: 'muted' }, route ? t('nav.error.textPage', { page: t(route.titleKey) }) : t('nav.error.text')),
      h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' },
        h('button', { class: 'btn btn-primary', type: 'button', onClick: () => handleRoute(), html: icon('refresh') + `<span>${escapeHtml(t('common.tryAgain'))}</span>` }),
        h('a', { class: 'btn btn-ghost', href: '#/', html: icon('home') + `<span>${escapeHtml(t('nav.home'))}</span>` })),
      h('details', { class: 'mt-4', style: 'text-align:left' },
        h('summary', { class: 'subtle text-sm', style: 'cursor:pointer' }, t('nav.error.details')),
        h('pre', { class: 'error-details' }, details.slice(0, 2000)))));
  host.appendChild(card);
}

function renderNotFound(host, path) {
  host.appendChild(h('div', { class: 'page' },
    h('div', { class: 'card placeholder-card' },
      h('div', { class: 'empty-state-icon', html: icon('help') }),
      h('h2', { class: 'mb-2' }, t('nav.notFound.title')),
      h('p', { class: 'muted', html: escapeHtml(t('nav.notFound.text', { path: '\u0000' })).replace('\u0000', `<code>${escapeHtml(path)}</code>`) }),
      h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' },
        h('a', { class: 'btn btn-primary', href: '#/', html: icon('home') + `<span>${escapeHtml(t('nav.goHome'))}</span>` })))));
}

// ---------------------------------------------------------------------------
// Global error reporting (friendly, throttled)
// ---------------------------------------------------------------------------
let lastGlobalToast = 0;
function reportGlobal(message) {
  const now = Date.now();
  if (now - lastGlobalToast < 4000) return;
  lastGlobalToast = now;
  toast(message, 'error');
}
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason;
  if (r && r.name === 'AbortError') { e.preventDefault(); return; }
  console.error('[unhandled]', r);
  reportGlobal(r && r.message ? r.message : tOr('common.somethingWentWrong', 'Something went wrong'));
});
window.addEventListener('error', (e) => {
  if (!e.error) return; // resource load errors etc.
  console.error('[error]', e.error);
  // i18n may not have loaded yet (an error during boot): fall back to English, never a raw key.
  reportGlobal(tOr('nav.globalError', 'Something went wrong — try reloading the page'));
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------
async function boot() {
  try { await initI18n(); } catch (e) { console.error('[i18n] init failed', e); }
  renderShell();
  initPwa();
  setTitle(null);
  // A language switch re-renders the shell and remounts the current page in the new language.
  onLanguageChange(() => {
    renderShell();
    checkHealth();
    handleRoute();
  });
  window.addEventListener('hashchange', handleRoute);
  window.addEventListener('online', checkHealth);
  // pwa.js found the server down / back up: refresh the engine status dot right away.
  window.addEventListener('gm:connection', checkHealth);
  if (!location.hash || location.hash === '#') history.replaceState(null, '', '#/');
  handleRoute();
  checkHealth();
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot, { once: true });
else boot();
