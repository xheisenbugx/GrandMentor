/* GrandMentor service worker — installable app + offline puzzles and lessons.
 * Contract: docs/CONTRACT.md "Installable app (PWA)".
 *
 * Caches (all bounded, all prefixed "gm-"; anything else with that prefix is deleted on activate):
 *   gm-shell-<BUILD>  app shell precached from /precache-manifest.json (exact file list, versioned)
 *   gm-runtime-v1     same-origin static files missed by the precache + Google Fonts (max 120)
 *   gm-content-v1     read-only content, stale-while-revalidate, keyed per language (max 250)
 *   gm-api-v1         other GET /api responses, network-first with offline fallback (max 80)
 * Never cached: POST/PUT/DELETE, /api/health, the engine WebSocket, mentor, backups/exports.
 * Offline POSTs to idempotent-safe endpoints are queued in IndexedDB (max 200) and replayed.
 */
'use strict';

// Replaced by the server with a hash of the web assets; any frontend change ships a new worker.
const BUILD = '__GM_BUILD__';
const SHELL = `gm-shell-${BUILD}`;
const RUNTIME = 'gm-runtime-v1';
const CONTENT = 'gm-content-v1';
const API = 'gm-api-v1';
const KNOWN = new Set([SHELL, RUNTIME, CONTENT, API]);
const LIMITS = { [RUNTIME]: 120, [CONTENT]: 250, [API]: 80 };
const MAX_CACHE_BYTES = 2 * 1024 * 1024;
// Responses carry `Vary: origin, accept-encoding`; module scripts send Origin, precache fetches don't.
const MATCH = { ignoreVary: true };

const PACK_KEY = '/__gm/offline/puzzle-pack';
const PACK_SIZE = 200;
const MAX_COURSES = 80;
const PREFETCH_EVERY_MS = 12 * 3600 * 1000;
const QUEUE_MAX = 200;
const QUEUE_MAX_AGE_MS = 30 * 24 * 3600 * 1000;
const QUEUE_MAX_BODY = 8 * 1024;
const FONT_HOSTS = new Set(['fonts.googleapis.com', 'fonts.gstatic.com']);

// GET /api paths that must always hit the network (live data, streams, large downloads).
const API_BYPASS = [/^\/api\/health$/, /^\/api\/engine\//, /^\/api\/mentor\//, /^\/api\/backup/, /\/export/, /\/pgn$/];
// Read-only content: stale-while-revalidate.
const API_CONTENT = [
  /^\/api\/courses(\/[^/]+)?$/,
  /^\/api\/openings(\/[^/]+)?$/,
  /^\/api\/endgames(\/[^/]+)?$/,
  /^\/api\/classics(\/[^/]+)?$/,
  /^\/api\/puzzles\/themes$/,
  /^\/api\/bots$/,
];
const PUZZLE_BY_ID = /^\/api\/puzzles\/([^/]+)$/;
const PUZZLE_SPECIAL = new Set(['next', 'daily', 'themes', 'rush']);
// POSTs that are safe to replay later (each records one fact; order is preserved).
const QUEUEABLE = [/^\/api\/puzzles\/[^/]+\/attempt$/, /^\/api\/progress$/, /^\/api\/puzzles\/rush$/, /^\/api\/activity$/];

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------
self.addEventListener('install', (event) => {
  event.waitUntil(precache());
});

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(names.filter((n) => n.startsWith('gm-') && !KNOWN.has(n)).map((n) => caches.delete(n)));
    await self.clients.claim();
    flushQueue().catch(() => {});
  })());
});

async function precache() {
  const res = await fetch('/precache-manifest.json', { cache: 'no-store' });
  if (!res.ok) throw new Error(`precache manifest: HTTP ${res.status}`);
  const data = await res.json();
  const files = Array.isArray(data.files) ? data.files.filter((f) => typeof f === 'string' && f.startsWith('/')).slice(0, 1500) : [];
  if (!files.includes('/index.html')) files.push('/index.html');
  const cache = await caches.open(SHELL);
  // Small batches keep the server responsive; any failure aborts the install (atomic update).
  for (let i = 0; i < files.length; i += 8) {
    await Promise.all(files.slice(i, i + 8).map(async (path) => {
      const r = await fetch(path, { cache: 'no-cache' });
      if (!r.ok) throw new Error(`precache ${path}: HTTP ${r.status}`);
      await cache.put(path, r);
    }));
  }
}

self.addEventListener('message', (event) => {
  const msg = event.data || {};
  const reply = (value) => { if (event.ports && event.ports[0]) event.ports[0].postMessage(value); };
  if (msg.type === 'skipWaiting') {
    self.skipWaiting();
  } else if (msg.type === 'replay') {
    event.waitUntil(flushQueue().catch(() => {}));
  } else if (msg.type === 'prefetch') {
    event.waitUntil(prefetch(String(msg.lang || 'en'), !!msg.force).then(reply, () => reply({ ok: false })));
  } else if (msg.type === 'status') {
    event.waitUntil(queueCount().then((queued) => reply({ build: BUILD, queued }), () => reply({ build: BUILD, queued: 0 })));
  }
});

self.addEventListener('sync', (event) => {
  if (event.tag === 'gm-replay') event.waitUntil(flushQueue());
});

// ---------------------------------------------------------------------------
// Fetch routing
// ---------------------------------------------------------------------------
self.addEventListener('fetch', (event) => {
  const req = event.request;
  const url = new URL(req.url);
  const sameOrigin = url.origin === self.location.origin;

  if (req.method !== 'GET') {
    if (req.method === 'POST' && sameOrigin && QUEUEABLE.some((re) => re.test(url.pathname))) {
      event.respondWith(postOrQueue(req, url));
    }
    return; // never cache other methods
  }
  if (!sameOrigin) {
    if (FONT_HOSTS.has(url.hostname)) event.respondWith(staleWhileRevalidate(event, req, RUNTIME, req.url, true));
    return;
  }
  const path = url.pathname;
  if (path.startsWith('/api/')) {
    if (API_BYPASS.some((re) => re.test(path)) || req.headers.get('upgrade')) return;
    if (path === '/api/puzzles/next') { event.respondWith(networkOr(req, () => packNext(url))); return; }
    if (path === '/api/puzzles/rush') { event.respondWith(networkOr(req, () => packRush(url))); return; }
    const byId = PUZZLE_BY_ID.exec(path);
    if (byId && !PUZZLE_SPECIAL.has(byId[1])) {
      event.respondWith(staleWhileRevalidate(event, req, CONTENT, keyFor(req, url)).catch(() => packById(decodeURIComponent(byId[1]))));
      return;
    }
    if (API_CONTENT.some((re) => re.test(path))) { event.respondWith(staleWhileRevalidate(event, req, CONTENT, keyFor(req, url))); return; }
    event.respondWith(networkFirst(req, API, keyFor(req, url)));
    return;
  }
  if (path === '/sw.js' || path === '/precache-manifest.json') return;
  if (req.mode === 'navigate') { event.respondWith(navigation(req)); return; }
  event.respondWith(staticAsset(event, req));
});

/** Content is localized by Accept-Language, so the language is part of the cache key. */
function langOf(req) {
  const raw = (req.headers.get('accept-language') || 'en').split(',')[0].trim().slice(0, 2).toLowerCase();
  return /^[a-z]{2}$/.test(raw) ? raw : 'en';
}

function keyFor(req, url) {
  const u = new URL(url.href);
  u.searchParams.set('__lang', langOf(req));
  return u.pathname + u.search;
}

function cacheable(res, allowOpaque = false) {
  if (!res) return false;
  if (res.type === 'opaque') return allowOpaque;
  if (!res.ok) return false;
  const len = Number(res.headers.get('content-length') || 0);
  return !(len > MAX_CACHE_BYTES);
}

/** Put + LRU-ish trim: re-inserting moves a key to the end, the oldest keys are evicted first. */
async function put(cacheName, key, res) {
  const cache = await caches.open(cacheName);
  await cache.delete(key, MATCH);
  await cache.put(key, res);
  const max = LIMITS[cacheName];
  if (!max) return;
  const keys = await cache.keys();
  if (keys.length > max) await Promise.all(keys.slice(0, keys.length - max).map((k) => cache.delete(k)));
}

async function navigation(req) {
  try {
    return await fetch(req);
  } catch (e) {
    const cached = await caches.match('/index.html', { cacheName: SHELL, ignoreVary: true }) || await caches.match('/index.html', MATCH);
    if (cached) return cached;
    throw e;
  }
}

async function staticAsset(event, req) {
  const shell = await caches.open(SHELL);
  const hit = await shell.match(req, { ignoreSearch: true, ignoreVary: true }) || await (await caches.open(RUNTIME)).match(req, MATCH);
  if (hit) return hit;
  const res = await fetch(req);
  if (cacheable(res)) event.waitUntil(put(RUNTIME, req, res.clone()).catch(() => {}));
  return res;
}

async function staleWhileRevalidate(event, req, cacheName, key, allowOpaque = false) {
  const cached = await (await caches.open(cacheName)).match(key, MATCH);
  const network = fetch(req).then(async (res) => {
    if (cacheable(res, allowOpaque)) await put(cacheName, key, res.clone()).catch(() => {});
    return res;
  });
  if (cached) {
    event.waitUntil(network.catch(() => {}));
    return cached;
  }
  try {
    return await network;
  } catch (e) {
    const other = await matchOtherLanguage(cacheName, key);
    if (other) return other;
    throw e;
  }
}

async function networkFirst(req, cacheName, key) {
  try {
    const res = await fetch(req);
    const type = res.headers.get('content-type') || '';
    if (cacheable(res) && type.includes('application/json')) await put(cacheName, key, res.clone()).catch(() => {});
    return res;
  } catch (e) {
    const cached = await (await caches.open(cacheName)).match(key, MATCH) || await matchOtherLanguage(cacheName, key);
    if (cached) return cached;
    throw e;
  }
}

/** Offline and nothing cached in this language: the same content in another language beats nothing. */
async function matchOtherLanguage(cacheName, key) {
  if (!key.includes('__lang=')) return null;
  const want = new URL(key, self.location.origin);
  want.searchParams.delete('__lang');
  const cache = await caches.open(cacheName);
  for (const k of await cache.keys()) {
    const u = new URL(k.url);
    u.searchParams.delete('__lang');
    if (u.pathname === want.pathname && u.search === want.search) return cache.match(k, MATCH);
  }
  return null;
}

async function networkOr(req, fallback) {
  try {
    return await fetch(req);
  } catch (e) {
    const res = await fallback();
    if (res) return res;
    throw e;
  }
}

function json(data, extraHeaders = {}) {
  return new Response(JSON.stringify(data), {
    status: 200,
    headers: { 'Content-Type': 'application/json', 'X-GM-Offline': '1', ...extraHeaders },
  });
}

// ---------------------------------------------------------------------------
// Offline puzzle pack
// ---------------------------------------------------------------------------
const recentPack = [];

async function loadPack() {
  const res = await (await caches.open(CONTENT)).match(PACK_KEY, MATCH);
  if (!res) return [];
  try {
    const list = await res.json();
    return Array.isArray(list) ? list.filter((p) => p && typeof p.id === 'string' && typeof p.fen === 'string') : [];
  } catch {
    return [];
  }
}

async function packNext(url) {
  const pack = await loadPack();
  if (!pack.length) return null;
  const theme = url.searchParams.get('theme');
  let pool = theme ? pack.filter((p) => Array.isArray(p.themes) && p.themes.includes(theme)) : pack;
  if (!pool.length) pool = pack;
  const fresh = pool.filter((p) => !recentPack.includes(p.id));
  const pick = (fresh.length ? fresh : pool)[Math.floor(Math.random() * (fresh.length || pool.length))];
  recentPack.push(pick.id);
  if (recentPack.length > Math.min(100, Math.floor(pack.length / 2))) recentPack.shift();
  return json(pick);
}

async function packRush(url) {
  const pack = await loadPack();
  if (!pack.length) return null;
  const count = Math.max(1, Math.min(200, Number(url.searchParams.get('count')) || 40));
  const copy = pack.slice();
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return json(copy.slice(0, count).sort((a, b) => (a.rating || 0) - (b.rating || 0)));
}

async function packById(id) {
  const p = (await loadPack()).find((x) => x.id === id);
  if (p) return json(p);
  return Response.error();
}

// ---------------------------------------------------------------------------
// Prefetch (courses + lessons, a puzzle pack, reference lists) — throttled per language
// ---------------------------------------------------------------------------
let prefetching = null;

function prefetch(lang, force) {
  if (prefetching) return prefetching;
  prefetching = doPrefetch(lang, force).finally(() => { prefetching = null; });
  return prefetching;
}

async function doPrefetch(lang, force) {
  const code = /^[a-z]{2}$/.test(lang) ? lang : 'en';
  const metaKey = `prefetch:${code}`;
  const last = await metaGet(metaKey).catch(() => 0);
  if (!force && last && Date.now() - last < PREFETCH_EVERY_MS) return { ok: true, skipped: true };

  const headers = { Accept: 'application/json', 'Accept-Language': code };
  const grab = async (path, cacheName, key) => {
    const req = new Request(path, { headers });
    const res = await fetch(req);
    if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
    await put(cacheName, key || keyFor(req, new URL(req.url)), res.clone());
    return res;
  };

  // Lessons: the course list, then each course (which carries its lessons).
  const courses = await (await grab('/api/courses', CONTENT)).json();
  const ids = (Array.isArray(courses) ? courses : []).map((c) => c && c.id).filter((id) => typeof id === 'string').slice(0, MAX_COURSES);
  let lessons = 0;
  for (let i = 0; i < ids.length; i += 3) {
    await Promise.all(ids.slice(i, i + 3).map(async (id) => {
      try {
        const course = await (await grab(`/api/courses/${encodeURIComponent(id)}`, CONTENT)).json();
        lessons += Array.isArray(course && course.lessons) ? course.lessons.length : 0;
      } catch { /* keep going */ }
    }));
  }

  // Puzzles: one bounded pack shared by every language (puzzles have no text).
  let puzzles = 0;
  try {
    const res = await fetch(new Request(`/api/puzzles/rush?count=${PACK_SIZE}`, { headers }));
    if (res.ok) {
      const list = await res.json();
      if (Array.isArray(list) && list.length) {
        puzzles = list.length;
        await put(CONTENT, PACK_KEY, json(list.slice(0, PACK_SIZE)));
      }
    }
  } catch { /* keep the previous pack */ }

  // Small reference data used by the puzzles/learn pages.
  await Promise.all([
    grab('/api/puzzles/themes', CONTENT),
    grab('/api/openings', CONTENT),
    grab('/api/endgames', CONTENT),
    grab('/api/profile', API),
    grab('/api/progress', API),
  ].map((p) => p.catch(() => null)));

  await metaSet(metaKey, Date.now()).catch(() => {});
  const summary = { ok: true, courses: ids.length, lessons, puzzles };
  notify({ type: 'prefetched', ...summary });
  return summary;
}

// ---------------------------------------------------------------------------
// Offline write queue (IndexedDB)
// ---------------------------------------------------------------------------
let dbPromise = null;

function db() {
  if (!dbPromise) {
    dbPromise = new Promise((resolve, reject) => {
      const open = indexedDB.open('gm-pwa', 1);
      open.onupgradeneeded = () => {
        const d = open.result;
        if (!d.objectStoreNames.contains('queue')) d.createObjectStore('queue', { keyPath: 'id', autoIncrement: true });
        if (!d.objectStoreNames.contains('meta')) d.createObjectStore('meta');
      };
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => { dbPromise = null; reject(open.error); };
    });
  }
  return dbPromise;
}

async function tx(store, mode, fn) {
  const d = await db();
  return new Promise((resolve, reject) => {
    const t = d.transaction(store, mode);
    const req = fn(t.objectStore(store));
    t.oncomplete = () => resolve(req ? req.result : undefined);
    t.onerror = () => reject(t.error);
    t.onabort = () => reject(t.error);
  });
}

const metaGet = (key) => tx('meta', 'readonly', (s) => s.get(key));
const metaSet = (key, value) => tx('meta', 'readwrite', (s) => s.put(value, key));
const queueCount = () => tx('queue', 'readonly', (s) => s.count());

async function enqueue(entry) {
  const keys = await tx('queue', 'readonly', (s) => s.getAllKeys());
  const excess = keys.length - QUEUE_MAX + 1;
  await tx('queue', 'readwrite', (s) => {
    for (let i = 0; i < excess; i++) s.delete(keys[i]);
    return s.add(entry);
  });
}

async function postOrQueue(req, url) {
  const body = await req.clone().text();
  try {
    return await fetch(req);
  } catch (e) {
    if (body.length > QUEUE_MAX_BODY) throw e;
    await enqueue({ path: url.pathname + url.search, body, lang: req.headers.get('accept-language') || 'en', at: Date.now() });
    try { if (self.registration.sync) await self.registration.sync.register('gm-replay'); } catch { /* not supported */ }
    notify({ type: 'queued' });
    return synthesize(url.pathname, body, req);
  }
}

async function cachedJson(cacheName, path) {
  const cache = await caches.open(cacheName);
  const keys = await cache.keys();
  const key = keys.find((k) => new URL(k.url).pathname === path);
  if (!key) return null;
  try { return { key, data: await (await cache.match(key, MATCH)).json() }; } catch { return null; }
}

/** A plausible response for a queued write so the page keeps working offline. */
async function synthesize(path, body, req) {
  const queued = { 'X-GM-Queued': '1' };
  let payload = {};
  try { payload = JSON.parse(body || '{}') || {}; } catch { /* ignore */ }
  if (/^\/api\/puzzles\/[^/]+\/attempt$/.test(path)) {
    const prof = await cachedJson(API, '/api/profile');
    const rating = prof && Number.isFinite(prof.data.puzzle_rating) ? prof.data.puzzle_rating : 1200;
    return json({ rating, delta: 0, queued: true }, queued);
  }
  if (path === '/api/progress') {
    // Mark the lesson complete in the cached progress list so the course page shows it offline.
    const cached = await cachedJson(API, '/api/progress');
    const list = cached && Array.isArray(cached.data) ? cached.data.filter((p) => !(p.course_id === payload.course_id && p.lesson_id === payload.lesson_id)) : [];
    list.push({ course_id: String(payload.course_id || ''), lesson_id: String(payload.lesson_id || ''), completed: !!payload.completed, updated_at: new Date().toISOString() });
    const key = cached ? cached.key : keyFor(req, new URL('/api/progress', self.location.origin));
    await put(API, key, json(list)).catch(() => {});
    return json(list, queued);
  }
  if (path === '/api/puzzles/rush') {
    const prof = await cachedJson(API, '/api/profile');
    const best = Math.max(Number(payload.score) || 0, prof ? Number(prof.data.rush_best) || 0 : 0);
    return json({ best, queued: true }, queued);
  }
  return json({ queued: true }, queued);
}

let flushing = null;

function flushQueue() {
  if (!flushing) flushing = doFlush().finally(() => { flushing = null; });
  return flushing;
}

async function doFlush() {
  const entries = await tx('queue', 'readonly', (s) => s.getAll());
  if (!entries || !entries.length) return 0;
  let sent = 0;
  for (const entry of entries) {
    if (Date.now() - (entry.at || 0) > QUEUE_MAX_AGE_MS) {
      await tx('queue', 'readwrite', (s) => s.delete(entry.id));
      continue;
    }
    let res;
    try {
      res = await fetch(entry.path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json', 'Accept-Language': entry.lang || 'en' },
        body: entry.body,
      });
    } catch {
      break; // still offline: keep the rest, in order
    }
    if (res.status >= 500) break;
    // 2xx = done; 4xx = the server will never accept it (e.g. unknown puzzle): drop it.
    await tx('queue', 'readwrite', (s) => s.delete(entry.id));
    if (res.ok) sent++;
  }
  const remaining = await queueCount().catch(() => 0);
  if (sent) notify({ type: 'replayed', count: sent, remaining });
  return sent;
}

async function notify(msg) {
  const list = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  for (const c of list) c.postMessage({ source: 'gm-sw', ...msg });
}
