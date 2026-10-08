// GrandMentor app shell + hash router.
// Contract: docs/CONTRACT.md §5. Pages live in ./pages/*.js and export
//   mount(root, { params, query, path }) -> cleanup fn (may be async), and optional `title`.

import { h, icon, brandMark, toast, closeAllModals, escapeHtml } from './ui.js';
import { api } from './api.js';
import { getSettings, setSetting, onSettingsChange } from './settings.js';

// ---------------------------------------------------------------------------
// Navigation model
// ---------------------------------------------------------------------------
const NAV = [
  { key: 'home', label: 'Home', icon: 'home', href: '#/', primary: true },
  { key: 'play', label: 'Play', icon: 'play', href: '#/play', primary: true },
  { key: 'puzzles', label: 'Puzzles', icon: 'puzzle', href: '#/puzzles', primary: true },
  { key: 'learn', label: 'Learn', icon: 'learn', href: '#/learn', primary: true },
  { key: 'openings', label: 'Openings', icon: 'openings', href: '#/openings' },
  { key: 'endgames', label: 'Endgames', icon: 'endgames', href: '#/endgames' },
  { key: 'analysis', label: 'Analysis', icon: 'analysis', href: '#/analysis', primary: true },
  { key: 'library', label: 'Library', icon: 'library', href: '#/library' },
  { key: 'profile', label: 'Profile', icon: 'profile', href: '#/profile', section: 'you' },
  { key: 'settings', label: 'Settings', icon: 'settings', href: '#/settings', section: 'you' },
];

// Route table: first match wins. `nav` = sidebar item to highlight.
const ROUTES = [
  { pattern: '/', page: 'home', nav: 'home', title: 'Home' },
  { pattern: '/play', page: 'play', nav: 'play', title: 'Play' },
  { pattern: '/play/:botId', page: 'play', nav: 'play', title: 'Play' },
  { pattern: '/analysis', page: 'analysis', nav: 'analysis', title: 'Analysis' },
  { pattern: '/review/:gameId', page: 'review', nav: 'analysis', title: 'Game Review' },
  { pattern: '/puzzles/rush', page: 'puzzles', nav: 'puzzles', title: 'Puzzle Rush', params: { mode: 'rush' } },
  { pattern: '/puzzles/daily', page: 'puzzles', nav: 'puzzles', title: 'Daily Puzzle', params: { mode: 'daily' } },
  { pattern: '/puzzles', page: 'puzzles', nav: 'puzzles', title: 'Puzzles' },
  { pattern: '/learn', page: 'learn', nav: 'learn', title: 'Learn' },
  { pattern: '/learn/:courseId', page: 'learn', nav: 'learn', title: 'Learn' },
  { pattern: '/learn/:courseId/:lessonId', page: 'lesson', nav: 'learn', title: 'Lesson' },
  { pattern: '/openings', page: 'openings', nav: 'openings', title: 'Openings' },
  { pattern: '/openings/:id', page: 'openings', nav: 'openings', title: 'Openings' },
  { pattern: '/endgames', page: 'endgames', nav: 'endgames', title: 'Endgames' },
  { pattern: '/endgames/:id', page: 'endgames', nav: 'endgames', title: 'Endgames' },
  { pattern: '/library', page: 'library', nav: 'library', title: 'Library' },
  { pattern: '/profile', page: 'profile', nav: 'profile', title: 'Profile' },
  { pattern: '/settings', page: 'settings', nav: 'settings', title: 'Settings' },
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

function navLink(item, cls = 'nav-item') {
  return h('a', { class: cls, href: item.href, dataset: { nav: item.key }, 'aria-label': item.label },
    h('span', { html: icon(item.icon), style: 'display:contents' }),
    h('span', { class: 'nav-label' }, item.label));
}

function renderShell() {
  els.app = document.getElementById('app');
  els.sidebar = document.getElementById('sidebar');
  els.view = document.getElementById('view');
  els.bottombar = document.getElementById('bottombar');
  els.sheet = document.getElementById('more-sheet');
  els.progress = document.getElementById('route-progress');

  // Sidebar
  const brand = h('a', { class: 'brand', href: '#/', 'aria-label': `${APP_NAME} home`, html: brandMark(36) },
    h('span', { class: 'brand-name' }, 'Grand', h('span', null, 'Mentor')));
  const mainNav = h('nav', { class: 'nav' }, NAV.filter((n) => !n.section).map((n) => navLink(n)));
  const youNav = h('nav', { class: 'nav' },
    h('div', { class: 'nav-section-label' }, 'You'),
    NAV.filter((n) => n.section === 'you').map((n) => navLink(n)));

  els.statusDot = h('span', { class: 'status-dot' });
  els.statusText = h('span', { class: 'nav-label' }, 'Connecting…');
  const status = h('div', { class: 'engine-status', title: 'Engine status' }, els.statusDot, els.statusText);

  els.themeBtn = h('button', { class: 'nav-item', type: 'button', style: 'border:0;background:none;width:100%;text-align:left', onClick: toggleTheme });
  els.collapseBtn = h('button', { class: 'nav-item collapse-btn hide-mobile', type: 'button', style: 'border:0;background:none;width:100%;text-align:left', onClick: toggleSidebar },
    h('span', { html: icon('sidebar'), style: 'display:contents' }), h('span', { class: 'nav-label' }, 'Collapse'));
  const footer = h('div', { class: 'sidebar-footer' }, els.themeBtn, els.collapseBtn, status);

  els.sidebar.replaceChildren(brand, mainNav, youNav, footer);

  // Mobile bottom bar: primary items + "More"
  const tabs = NAV.filter((n) => n.primary).map((n) =>
    h('a', { class: 'tab-item', href: n.href, dataset: { nav: n.key } },
      h('span', { html: icon(n.icon), style: 'display:contents' }), h('span', null, n.label)));
  els.moreBtn = h('button', { class: 'tab-item', type: 'button', 'aria-haspopup': 'true', 'aria-expanded': 'false', onClick: () => toggleSheet() },
    h('span', { html: icon('more'), style: 'display:contents' }), h('span', null, 'More'));
  els.bottombar.replaceChildren(...tabs, els.moreBtn);

  // "More" sheet
  const sheetItems = NAV.filter((n) => !n.primary).map((n) => navLink(n));
  const sheetTheme = h('button', { class: 'nav-item', type: 'button', style: 'border:0', onClick: () => { toggleTheme(); } });
  els.sheetTheme = sheetTheme;
  const panel = h('div', { class: 'more-sheet-panel', role: 'menu' }, sheetItems, sheetTheme);
  els.sheet.replaceChildren(panel);
  els.sheet.addEventListener('click', (e) => {
    if (e.target === els.sheet || e.target.closest('a')) toggleSheet(false);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && els.sheet.classList.contains('open')) toggleSheet(false);
  });

  syncShellSettings(getSettings());
  onSettingsChange((s, key) => {
    if (key === 'theme' || key === 'sidebarCollapsed') syncShellSettings(s);
  });
}

function syncShellSettings(s) {
  els.app.classList.toggle('sidebar-collapsed', !!s.sidebarCollapsed);
  const isLight = s.theme === 'light';
  const themeContent = [h('span', { html: icon(isLight ? 'moon' : 'sun'), style: 'display:contents' }), h('span', { class: 'nav-label' }, isLight ? 'Dark mode' : 'Light mode')];
  els.themeBtn.replaceChildren(...themeContent);
  els.themeBtn.setAttribute('aria-label', isLight ? 'Switch to dark mode' : 'Switch to light mode');
  els.sheetTheme.replaceChildren(h('span', { html: icon(isLight ? 'moon' : 'sun'), style: 'display:contents' }), h('span', { class: 'nav-label' }, isLight ? 'Dark' : 'Light'));
  els.collapseBtn.querySelector('.nav-label').textContent = s.sidebarCollapsed ? 'Expand' : 'Collapse';
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
  els.moreBtn.setAttribute('aria-expanded', open ? 'true' : 'false');
  els.moreBtn.classList.toggle('active', open);
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
    const engines = hres && hres.engines ? ` · ${hres.engines} thread${hres.engines === 1 ? "" : "s"}` : '';
    els.statusText.textContent = `Engine ready${engines}`;
    els.statusDot.parentElement.title = hres && hres.llm_enabled ? 'Engine online · AI mentor enabled' : 'Engine online · Coach mentor (offline mode)';
  } catch {
    healthOk = false;
    els.statusDot.className = 'status-dot bad';
    els.statusText.textContent = 'Server offline';
    els.statusDot.parentElement.title = 'Cannot reach the GrandMentor server';
  } finally {
    healthBusy = false;
  }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
let currentCleanup = null;
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

function setTitle(t) {
  document.title = t ? `${t} · ${APP_NAME}` : `${APP_NAME} — Learn chess the friendly way`;
}

async function handleRoute() {
  const token = ++navToken;
  const { path, segments, query } = parseHash();
  const match = matchRoute(segments);

  toggleSheet(false);
  closeAllModals();
  await runCleanup();
  if (token !== navToken) return;

  if (healthOk === false) checkHealth();

  const host = h('div', { class: 'page-host page-enter' });
  els.view.replaceChildren(host);
  window.scrollTo(0, 0);

  if (!match) {
    setActiveNav(null);
    setTitle('Page not found');
    renderNotFound(host, path);
    return;
  }

  const { route, params } = match;
  setActiveNav(route.nav);
  setTitle(route.title);
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
      h('h2', { class: 'mb-2' }, 'Oops — this page tripped over a pawn'),
      h('p', { class: 'muted' }, `Something went wrong while opening ${route ? route.title : 'this page'}. You can try again, or head back home.`),
      h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' },
        h('button', { class: 'btn btn-primary', type: 'button', onClick: () => handleRoute(), html: icon('refresh') + '<span>Try again</span>' }),
        h('a', { class: 'btn btn-ghost', href: '#/', html: icon('home') + '<span>Home</span>' })),
      h('details', { class: 'mt-4', style: 'text-align:left' },
        h('summary', { class: 'subtle text-sm', style: 'cursor:pointer' }, 'Technical details'),
        h('pre', { class: 'error-details' }, details.slice(0, 2000)))));
  host.appendChild(card);
}

function renderNotFound(host, path) {
  host.appendChild(h('div', { class: 'page' },
    h('div', { class: 'card placeholder-card' },
      h('div', { class: 'empty-state-icon', html: icon('help') }),
      h('h2', { class: 'mb-2' }, 'Page not found'),
      h('p', { class: 'muted', html: `We couldn't find <code>${escapeHtml(path)}</code>. Maybe the knight jumped somewhere else?` }),
      h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' },
        h('a', { class: 'btn btn-primary', href: '#/', html: icon('home') + '<span>Go home</span>' })))));
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
  reportGlobal(r && r.message ? r.message : 'Something went wrong');
});
window.addEventListener('error', (e) => {
  if (!e.error) return; // resource load errors etc.
  console.error('[error]', e.error);
  reportGlobal('Something went wrong — try reloading the page');
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------
function boot() {
  renderShell();
  window.addEventListener('hashchange', handleRoute);
  window.addEventListener('online', checkHealth);
  if (!location.hash || location.hash === '#') history.replaceState(null, '', '#/');
  handleRoute();
  checkHealth();
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot, { once: true });
else boot();
