#!/usr/bin/env node
// GrandMentor UI sweep: visits every route × language × viewport in headless Chrome and checks for
// console errors, failed API calls, horizontal scroll, raw i18n keys, leaked "null"/"undefined" text, English leaking into other
// languages, elements sticking out of the viewport and overlapping layers/buttons.
// Plain Node 22, no dependencies; drives Chrome through tools/screenshots/cdp.mjs.
//
//   cargo build --release -p gm-server
//   node tools/qa/sweep.mjs                    # starts its own server on a throwaway database
//   BASE=http://localhost:8080 node tools/qa/sweep.mjs   # use a running server (it seeds a game if none exist!)
//
// Environment (all optional):
//   BASE           server to test; when unset the sweep starts target/release/grandmentor itself
//   QA_PORT        port for the server the sweep starts (default 8199)
//   GM_BIN         server binary (default target/release/grandmentor)
//   CDP_PORT       Chrome DevTools port (default 9555)
//   CHROME         Chrome/Chromium executable (auto-detected); CHROME_ARGS extra flags
//   QA_OUT         output directory (default target/qa-sweep): report.json, report.md, shots/
//   QA_LANGS       comma-separated language codes (default: every language in web/js/languages.js)
//   QA_VIEWPORTS   comma-separated: desktop (1440×900), mobile (390×844)  (default both)
//   QA_ROUTES      regex; only route patterns matching it are visited
//   QA_SAMPLE      N > 1: non-English languages visit only every N-th route (rotating), to save CI time
//   QA_SHOTS       failures (default) | all | none
//   QA_SETTLE_MS   extra wait after the network goes idle (default 500)
//   QA_SETTINGS    JSON merged into the app settings, e.g. '{"theme":"light"}' or '{"highContrast":true}'
//   QA_KEYBOARD    0 skips the keyboard smoke test (Tab through QA_KBD_STEPS focus stops, default 20)
//   QA_INJECT      JS injected into every page before it loads (to test the sweep itself, e.g.
//                  QA_INJECT="console.error('boom'); fetch('/api/nope')" must make every page fail)
// Exit code: 0 clean, 1 failures found, 2 the sweep itself could not run.
import { spawn, execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync, rmSync, mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const env = process.env;
const OUT = path.resolve(ROOT, env.QA_OUT || 'target/qa-sweep');
const SHOTS = env.QA_SHOTS || 'failures';
const SETTLE = Number(env.QA_SETTLE_MS) || 500;
const SAMPLE = Math.max(1, Number(env.QA_SAMPLE) || 1);
const VIEWPORTS = {
  desktop: { width: 1440, height: 900, mobile: false },
  mobile: { width: 390, height: 844, mobile: true },
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/** QA_SETTINGS: JSON merged into the app settings, e.g. '{"theme":"light"}' or '{"highContrast":true}'. */
let EXTRA_SETTINGS = {};
try { EXTRA_SETTINGS = env.QA_SETTINGS ? JSON.parse(env.QA_SETTINGS) : {}; } catch { console.error('sweep: QA_SETTINGS is not valid JSON'); process.exit(2); }
const log = (...a) => console.log(...a);

// ---------------------------------------------------------------------------
// Chrome detection (before importing cdp.mjs, which reads CHROME / CHROME_ARGS at import time)
// ---------------------------------------------------------------------------
function findChrome() {
  if (env.CHROME) return env.CHROME;
  const mac = ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/Applications/Chromium.app/Contents/MacOS/Chromium'];
  for (const p of mac) if (existsSync(p)) return p;
  for (const name of ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'chrome']) {
    try { const p = execFileSync('which', [name], { encoding: 'utf8' }).trim(); if (p) return p; } catch { /* not found */ }
  }
  return null;
}
const chrome = findChrome();
if (!chrome) { console.error('sweep: no Chrome found; set CHROME=/path/to/chrome'); process.exit(2); }
env.CHROME = chrome;
if (env.CHROME_ARGS === undefined && process.platform === 'linux') env.CHROME_ARGS = '--no-sandbox --disable-dev-shm-usage';
env.CDP_PORT = env.CDP_PORT || '9555';
const { launch } = await import(pathToFileURL(path.join(ROOT, 'tools/screenshots/cdp.mjs')).href);

// ---------------------------------------------------------------------------
// Inputs: routes, languages, catalogs, allowlist
// ---------------------------------------------------------------------------
/** Route patterns parsed from the ROUTES table in web/js/app.js (so new pages are picked up). */
function readRoutes() {
  const src = readFileSync(path.join(ROOT, 'web/js/app.js'), 'utf8');
  const start = src.indexOf('const ROUTES');
  const end = src.indexOf('];', start);
  if (start < 0 || end < 0) throw new Error('cannot find the ROUTES table in web/js/app.js');
  const table = src.slice(start, end);
  return [...table.matchAll(/pattern:\s*(['"])([^'"]+)\1/g)].map((m) => m[2]);
}

const PLURAL = new Set(['zero', 'one', 'two', 'few', 'many', 'other']);
const isPlural = (v) => v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length > 0 && Object.keys(v).every((k) => PLURAL.has(k));
function flatten(obj, prefix = '', out = new Map()) {
  for (const [k, v] of Object.entries(obj || {})) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === 'object' && !Array.isArray(v) && !isPlural(v)) flatten(v, key, out);
    else out.set(key, v);
  }
  return out;
}
const msgStrings = (v) => (typeof v === 'string' ? [v] : Array.isArray(v) ? v : v && typeof v === 'object' ? Object.values(v) : []).filter((s) => typeof s === 'string');
async function catalog(lang) {
  const mod = await import(pathToFileURL(path.join(ROOT, 'web/locales', lang, 'index.js')).href);
  return flatten(mod.default);
}

function loadAllowlist() {
  const file = path.join(ROOT, 'tools/qa/allowlist.json');
  if (!existsSync(file)) return { visibleEnglish: [], sweep: [] };
  const a = JSON.parse(readFileSync(file, 'utf8'));
  return { visibleEnglish: (a.visibleEnglish || []).map((e) => (typeof e === 'string' ? e : e.text)), sweep: a.sweep || [] };
}

/** English UI phrases that should not appear verbatim on a page in `lang`. */
function englishPhrases(en, other, allowed) {
  const otherBlob = [...other.values()].flatMap(msgStrings).join('\n');
  const phrases = new Set();
  for (const [key, v] of en) {
    const same = msgStrings(other.get(key));
    for (const s of msgStrings(v)) {
      if (same.includes(s)) continue; // intentionally identical (check-i18n reports these separately)
      for (const frag of s.replace(/\*\*?|__/g, '').split(/\{\w+\}|\n/)) {
        const f = frag.trim();
        if (f.length < 14 || !/[A-Za-z]{3,}[\s,'’-]+[A-Za-z]{3,}/.test(f)) continue;
        if (otherBlob.includes(f) || allowed.some((a) => f.includes(a) || a.includes(f))) continue;
        phrases.add(f);
      }
    }
  }
  return [...phrases].sort((a, b) => b.length - a.length);
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------
let server = null;
let tmpDir = null;
let serverLog = null;
async function waitHealthy(base, ms) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try { if ((await fetch(base + '/api/health')).ok) return true; } catch { /* not up yet */ }
    await sleep(250);
  }
  return false;
}
async function startServer() {
  if (env.BASE) return env.BASE.replace(/\/$/, '');
  const bin = path.resolve(ROOT, env.GM_BIN || 'target/release/grandmentor');
  if (!existsSync(bin)) throw new Error(`${path.relative(ROOT, bin)} not found; run: cargo build --release -p gm-server`);
  const port = env.QA_PORT || '8199';
  tmpDir = mkdtempSync(path.join(os.tmpdir(), 'gm-qa-'));
  const childEnv = { ...env, NO_COLOR: '1', GM_PORT: port, GM_HOST: '127.0.0.1', GM_DB: path.join(tmpDir, 'qa.sqlite'), GM_WEB_DIR: path.join(ROOT, 'web') };
  delete childEnv.ANTHROPIC_API_KEY; // never call a paid LLM from the sweep
  server = spawn(bin, [], { cwd: ROOT, env: childEnv, stdio: ['ignore', 'pipe', 'pipe'] });
  const chunks = [];
  let size = 0;
  const keep = (d) => { if (size < 4 << 20) { chunks.push(d); size += d.length; } }; // bounded
  server.stdout.on('data', keep);
  server.stderr.on('data', keep);
  serverLog = () => Buffer.concat(chunks);
  const base = `http://127.0.0.1:${port}`;
  if (!(await waitHealthy(base, 60000))) throw new Error(`server did not become healthy on ${base}`);
  return base;
}
function cleanup() {
  if (serverLog) { try { writeFileSync(path.join(OUT, 'server.log'), serverLog()); } catch { /* ignore */ } }
  if (server && server.exitCode === null) server.kill();
  if (tmpDir) rmSync(tmpDir, { recursive: true, force: true });
}
process.on('SIGINT', () => { cleanup(); process.exit(130); });
process.on('SIGTERM', () => { cleanup(); process.exit(143); });

// ---------------------------------------------------------------------------
// Route parameters from real data
// ---------------------------------------------------------------------------
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';
async function api(base, p, body) {
  const r = await fetch(base + p, body === undefined ? {} : { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(`${body === undefined ? 'GET' : 'POST'} ${p} → ${r.status}`);
  return r.json();
}
const firstList = (v) => (Array.isArray(v) ? v : v && typeof v === 'object' ? Object.values(v).find(Array.isArray) || [] : []);
const idOf = (x) => (x && typeof x === 'object' ? x.id ?? x.slug ?? x.key : x);

async function resolveParams(base, routes) {
  const cache = new Map();
  const list = async (p) => {
    if (!cache.has(p)) cache.set(p, api(base, p).then(firstList).catch(() => []));
    return cache.get(p);
  };
  const resolvers = {
    botId: async () => idOf((await list('/api/bots'))[0]),
    gameId: async () => {
      let g = (await list('/api/games?limit=1'))[0];
      if (!g) {
        const bot = idOf((await list('/api/bots'))[0]) ?? null;
        g = await api(base, '/api/games', { white: 'QA', black: 'Bot', result: '0-1', termination: 'checkmate', start_fen: START,
          moves: ['f2f3', 'e7e5', 'g2g4', 'd8h4'], bot_id: bot, user_color: 'white', time_control: '10+0', opening_name: null, notes: '', tags: [] });
        await api(base, '/api/review', { game_id: g.id, depth: 8 }).catch(() => null);
      }
      return idOf(g);
    },
    courseId: async () => idOf((await list('/api/courses'))[0]),
    lessonId: async (vals) => {
      const course = (await list('/api/courses')).find((c) => idOf(c) === vals.courseId);
      return idOf(course?.lessons?.[0]);
    },
    side: async () => 'white',
  };
  const resolved = new Map();
  for (const pattern of routes) {
    const segs = pattern.split('/').filter(Boolean);
    const vals = {};
    let skip = null;
    for (const seg of segs.filter((s) => s.startsWith(':'))) {
      const name = seg.slice(1);
      let v;
      try {
        if (resolvers[name]) v = await resolvers[name](vals);
        else v = idOf((await list(`/api/${segs[0]}`))[0]); // generic: /x/:id → first item of GET /api/x
      } catch (e) { skip = `${name}: ${e.message}`; }
      if (v === undefined || v === null || v === '') { skip = skip || `no data for :${name}`; break; }
      vals[name] = String(v);
    }
    const p = '/' + segs.map((s) => (s.startsWith(':') ? encodeURIComponent(vals[s.slice(1)] ?? '') : s)).join('/');
    resolved.set(pattern, skip ? { skip } : { path: p === '/' ? '/' : p });
  }
  return resolved;
}

// ---------------------------------------------------------------------------
// Per-page collection
// ---------------------------------------------------------------------------
function makeCollector(b, base) {
  const origin = new URL(base).origin;
  let issues = [];
  let inflight = new Map(); // requestId -> url, until the response headers arrive
  let urls = new Map();
  let lastActivity = Date.now();
  let loaded = false;
  b.on((method, p) => {
    switch (method) {
      case 'Page.loadEventFired': loaded = true; break;
      case 'Runtime.exceptionThrown': {
        const d = p.exceptionDetails;
        issues.push({ check: 'exception', detail: (d.exception?.description || d.text || 'uncaught exception').split('\n').slice(0, 3).join(' | ') });
        break;
      }
      case 'Runtime.consoleAPICalled': {
        const text = (p.args || []).map((a) => a.value ?? a.description ?? '').join(' ');
        if (p.type === 'error' || p.type === 'assert') issues.push({ check: 'console', detail: text.slice(0, 300) });
        else if (/^\[i18n\] unknown key/.test(text)) issues.push({ check: 'i18n-key', detail: text });
        else if (/^\[i18n\] missing/.test(text)) issues.push({ check: 'i18n-missing', detail: text });
        break;
      }
      case 'Log.entryAdded':
        if (p.entry.level === 'error' && p.entry.source !== 'network') issues.push({ check: 'console', detail: `${p.entry.source}: ${p.entry.text}`.slice(0, 300) });
        break;
      case 'Network.requestWillBeSent':
        inflight.set(p.requestId, p.request.url); urls.set(p.requestId, p.request.url); lastActivity = Date.now(); break;
      case 'Network.responseReceived': {
        const { url, status } = p.response;
        inflight.delete(p.requestId); lastActivity = Date.now();
        if (url.startsWith(origin) && status >= 400) issues.push({ check: 'http', detail: `${status} ${url.slice(origin.length)}` });
        break;
      }
      case 'Network.loadingFinished':
        inflight.delete(p.requestId); lastActivity = Date.now(); break;
      case 'Network.loadingFailed': {
        const url = urls.get(p.requestId);
        inflight.delete(p.requestId); lastActivity = Date.now();
        if (!p.canceled && url && url.startsWith(origin)) issues.push({ check: 'http', detail: `${p.errorText} ${url.slice(origin.length)}` });
        break;
      }
      default:
    }
  });
  return {
    reset() { issues = []; inflight = new Map(); urls = new Map(); lastActivity = Date.now(); loaded = false; },
    take() { const i = issues; issues = []; return i; },
    pending() { return [...new Set(inflight.values())].map((u) => (u.startsWith(origin) ? u.slice(origin.length) : u)).slice(0, 5); },
    async settle(timeout = 12000) {
      const t0 = Date.now();
      while (Date.now() - t0 < timeout) {
        if (loaded && inflight.size === 0 && Date.now() - lastActivity > 400) return true;
        await sleep(100);
      }
      return false;
    },
  };
}

function allowed(issue, page, allow) {
  return allow.sweep.find((a) => (!a.check || a.check === issue.check)
    && (!a.route || a.route === page.route)
    && (!a.lang || a.lang === page.lang)
    && (!a.viewport || a.viewport === page.viewport)
    && (!a.match || issue.detail.includes(a.match)));
}

function issuesFromChecks(r) {
  const out = [];
  if (r.hscroll) out.push({ check: 'hscroll', detail: `page scrolls horizontally: scrollWidth ${r.scrollWidth}px > ${r.innerWidth}px` });
  for (const o of r.overflow) out.push({ check: 'overflow', detail: `${o.el} spans ${o.left}..${o.right}px (viewport ${r.innerWidth}px)` });
  for (const k of r.rawKeys) out.push({ check: 'raw-key', detail: `"${k.key}" in ${k.where}` });
  for (const j of r.junk || []) out.push({ check: 'junk-text', detail: `"${j.text}" shown in ${j.where}` });
  for (const e of r.english) out.push({ check: 'english', detail: `"${e}"` });
  for (const o of r.layerOverlaps) out.push({ check: 'layer-over-board', detail: `${o.layer} covers ${o.board} (${o.overlap})` });
  for (const o of r.buttonOverlaps) out.push({ check: 'button-overlap', detail: `${o.a} overlaps ${o.b} (${o.overlap})` });
  const a = r.a11y || {};
  for (const x of a.names || []) out.push({ check: 'a11y-name', detail: `${x} has no accessible name` });
  for (const x of a.alt || []) out.push({ check: 'a11y-alt', detail: `${x} has no text alternative` });
  for (const x of a.hiddenFocus || []) out.push({ check: 'a11y-hidden-focus', detail: `${x} is focusable but hidden from assistive tech` });
  for (const x of a.dupIds || []) out.push({ check: 'a11y-dup-id', detail: `duplicate id ${x}` });
  for (const c of a.contrast || []) out.push({ check: 'contrast', detail: `${c.el} ${c.ratio}:1 < ${c.need}:1 (${c.fg} on ${c.bg})` });
  for (const x of a.touch || []) out.push({ check: 'touch-target', detail: `${x} is smaller than 40×40px` });
  return out;
}

// Keyboard smoke test: press Tab KBD_STEPS times from the top of the page. Every stop must move
// focus to a visible element (not inside aria-hidden) that shows a focus indicator.
const KBD_STEPS = Number(env.QA_KBD_STEPS) || 20;
const KBD_PROBE = `(() => {
  const el = document.activeElement;
  if (!el || el === document.body || el === document.documentElement) return { body: true };
  let s = el.tagName.toLowerCase(); if (el.id) s += '#' + el.id;
  const cls = typeof el.className === 'string' ? el.className.trim().split(/\\s+/).filter(Boolean).slice(0, 2) : [];
  if (cls.length) s += '.' + cls.join('.');
  const label = (el.getAttribute('aria-label') || el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 30);
  if (label) s += ' "' + label + '"';
  const r = el.getBoundingClientRect();
  const hiddenAT = !!el.closest('[aria-hidden="true"], [inert]');
  const cs = getComputedStyle(el);
  const ring = (st) => (st.outlineStyle !== 'none' && parseFloat(st.outlineWidth) > 0) || (st.boxShadow && st.boxShadow !== 'none');
  let indicator = ring(cs);
  if (!indicator && el.matches('.switch input')) indicator = !!el.nextElementSibling && ring(getComputedStyle(el.nextElementSibling));
  if (!indicator && el.matches('.gm-sq')) { const c = el.closest('.gm-board')?.querySelector('.gm-kbd-cursor'); indicator = !!c && getComputedStyle(c).display !== 'none'; }
  if (!indicator && el.matches('input, textarea, select')) indicator = cs.borderColor !== '' && cs.boxShadow !== 'none';
  const tiny = (r.width < 2 || r.height < 2) && !el.matches('.switch input');
  // Is anything tabbable after it? (Tab from the last stop leaves the document: not a trap.)
  const sel = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
  const after = [...document.querySelectorAll(sel)].some((x) => x !== el && (el.compareDocumentPosition(x) & Node.DOCUMENT_POSITION_FOLLOWING)
    && !x.closest('[inert], [hidden]') && x.getClientRects().length > 0 && getComputedStyle(x).visibility !== 'hidden');
  if (!el.dataset.qaKbd) el.dataset.qaKbd = String((window.__qaKbdSeq = (window.__qaKbdSeq || 0) + 1));
  return { id: el.dataset.qaKbd, desc: s, hiddenAT, tiny, indicator, last: !after };
})()`;
async function keyboardSmoke(b) {
  const issues = [];
  await b.eval(`(() => { const a = document.activeElement; if (a && a.blur) a.blur(); window.scrollTo(0, 0); })()`);
  let last = null;
  let bodyStops = 0;
  for (let i = 0; i < KBD_STEPS; i++) {
    await b.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 });
    await b.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 });
    await sleep(40);
    const r = await b.eval(KBD_PROBE);
    if (r.body) { if (++bodyStops > 1) break; continue; } // wrapped around the page
    if (r.id === last) { if (!r.last) issues.push({ check: 'keyboard', detail: `focus is stuck on ${r.desc}` }); break; }
    last = r.id;
    if (r.hiddenAT) issues.push({ check: 'keyboard', detail: `Tab reached ${r.desc}, which is hidden from assistive tech` });
    else if (r.tiny) issues.push({ check: 'keyboard', detail: `Tab reached ${r.desc}, which is not visible` });
    else if (!r.indicator) issues.push({ check: 'keyboard', detail: `${r.desc} shows no focus indicator` });
    if (issues.length >= 5) break;
  }
  await b.eval(`(() => { const a = document.activeElement; if (a && a.blur) a.blur(); window.scrollTo(0, 0); })()`);
  return issues;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
  rmSync(path.join(OUT, 'shots'), { recursive: true, force: true });
  mkdirSync(path.join(OUT, 'shots'), { recursive: true });
  const { LANGUAGES } = await import(pathToFileURL(path.join(ROOT, 'web/js/languages.js')).href);
  const langs = (env.QA_LANGS ? env.QA_LANGS.split(',').map((s) => s.trim()) : Object.keys(LANGUAGES)).filter((l) => {
    if (LANGUAGES[l]) return true;
    log(`sweep: skipping unknown language "${l}"`); return false;
  });
  const vps = (env.QA_VIEWPORTS ? env.QA_VIEWPORTS.split(',').map((s) => s.trim()) : Object.keys(VIEWPORTS)).filter((v) => VIEWPORTS[v]);
  const routeFilter = env.QA_ROUTES ? new RegExp(env.QA_ROUTES) : null;
  const routes = readRoutes().filter((r) => !routeFilter || routeFilter.test(r));
  const allow = loadAllowlist();
  const en = await catalog('en');
  const pageCheck = readFileSync(path.join(ROOT, 'tools/qa/page-checks.js'), 'utf8');

  const base = await startServer();
  log(`sweep: ${base} — ${routes.length} routes × ${langs.join(',')} × ${vps.join(',')} (chrome: ${chrome})`);
  const params = await resolveParams(base, routes);

  const profileDir = mkdtempSync(path.join(os.tmpdir(), 'gm-qa-chrome-'));
  const b = await launch(profileDir, 1440, 900);
  const results = [];
  const skipped = [];
  try {
    const col = makeCollector(b, base);
    await b.send('Network.enable');
    await b.send('Log.enable');
    let nav = 0;
    for (const lang of langs) {
      const other = lang === 'en' ? null : await catalog(lang);
      const qaData = { keys: [...en.keys()], english: other ? englishPhrases(en, other, allow.visibleEnglish) : [] };
      const inject = env.QA_INJECT ? `\naddEventListener('DOMContentLoaded', () => { ${env.QA_INJECT} });` : '';
      const { identifier } = await b.send('Page.addScriptToEvaluateOnNewDocument', { source: `window.__qaData = ${JSON.stringify(qaData)};${inject}` });
      // Select the language through the app's own settings storage.
      await b.open(`${base}/?qa=lang#/`, 1500);
      await b.eval(`(() => { const k = 'grandmentor.settings.v1'; let s = {}; try { s = JSON.parse(localStorage.getItem(k)) || {}; } catch {} s.language = ${JSON.stringify(lang)}; Object.assign(s, ${JSON.stringify(EXTRA_SETTINGS)}); localStorage.setItem(k, JSON.stringify(s)); })()`);
      for (const [vi, vpName] of vps.entries()) {
        const vp = VIEWPORTS[vpName];
        await b.size(vp.width, vp.height, vp.mobile);
        await b.send('Emulation.setTouchEmulationEnabled', { enabled: vp.mobile });
        for (const [ri, pattern] of routes.entries()) {
          if (lang !== 'en' && SAMPLE > 1 && (ri + vi + langs.indexOf(lang)) % SAMPLE !== 0) continue;
          const pr = params.get(pattern);
          if (pr.skip) { if (!skipped.some((s) => s.route === pattern)) skipped.push({ route: pattern, reason: pr.skip }); continue; }
          const page = { route: pattern, path: pr.path, lang, viewport: vpName };
          col.reset();
          const t0 = Date.now();
          let issues = [];
          try {
            await b.send('Page.navigate', { url: `${base}/?qa=${++nav}#${pr.path}` });
            const idle = await col.settle();
            await sleep(SETTLE);
            if (!idle) issues.push({ check: 'timeout', detail: `network did not go idle within 12s (pending: ${col.pending().join(', ') || 'page load event'})` });
            const htmlLang = await b.eval('document.documentElement.lang');
            if (htmlLang !== lang) issues.push({ check: 'language', detail: `<html lang="${htmlLang}">, expected "${lang}"` });
            const viewEmpty = await b.eval(`!document.querySelector('#view')?.children.length`);
            if (viewEmpty) issues.push({ check: 'empty', detail: '#view rendered nothing' });
            issues.push(...issuesFromChecks(await b.eval(pageCheck)));
            if (env.QA_KEYBOARD !== '0') issues.push(...await keyboardSmoke(b));
          } catch (e) {
            issues.push({ check: 'sweep-error', detail: String(e.message || e).slice(0, 300) });
          }
          issues.unshift(...col.take());
          const kept = [], allowedIssues = [];
          for (const i of issues) (allowed(i, page, allow) ? allowedIssues : kept).push(i);
          page.ms = Date.now() - t0;
          page.issues = kept;
          page.allowed = allowedIssues;
          if (SHOTS === 'all' || (SHOTS === 'failures' && kept.length)) {
            const file = `${lang}-${vpName}-${pr.path.replace(/[^a-z0-9]+/gi, '_').replace(/^_|_$/g, '') || 'home'}.png`;
            try { await b.shot(path.join(OUT, 'shots', file)); page.screenshot = `shots/${file}`; } catch { /* ignore */ }
          }
          results.push(page);
          log(`${kept.length ? 'FAIL' : ' ok '} ${lang} ${vpName.padEnd(7)} ${pr.path}${kept.length ? `  (${kept.map((i) => i.check).join(', ')})` : ''}`);
        }
      }
      await b.send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
    }
  } finally {
    b.close();
    await sleep(300);
    rmSync(profileDir, { recursive: true, force: true });
  }

  const failed = results.filter((r) => r.issues.length);
  const report = {
    ok: failed.length === 0, base, generated: new Date().toISOString(), languages: langs, viewports: vps,
    pages: results.length, failedPages: failed.length, skipped, results,
  };
  writeFileSync(path.join(OUT, 'report.json'), JSON.stringify(report, null, 2));
  writeFileSync(path.join(OUT, 'report.md'), markdown(report));
  log(`\nsweep: ${results.length} pages, ${failed.length} with issues, ${skipped.length} routes skipped — ${path.relative(ROOT, OUT)}/report.md`);
  return report.ok ? 0 : 1;
}

function markdown(r) {
  const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\n/g, ' ');
  const lines = [`# UI sweep report`, '', `${r.generated} · ${r.base} · languages: ${r.languages.join(', ')} · viewports: ${r.viewports.join(', ')}`, '',
    `**${r.ok ? 'PASS' : 'FAIL'}** — ${r.pages} pages visited, ${r.failedPages} with issues.`, ''];
  if (r.skipped.length) {
    lines.push('## Skipped routes', '', ...r.skipped.map((s) => `- \`${s.route}\`: ${s.reason}`), '');
  }
  const failed = r.results.filter((x) => x.issues.length);
  if (failed.length) {
    lines.push('## Issues', '', '| Route | Lang | Viewport | Check | Detail |', '|---|---|---|---|---|');
    for (const p of failed) for (const i of p.issues) lines.push(`| \`${p.path}\` | ${p.lang} | ${p.viewport} | ${i.check} | ${esc(i.detail)}${p.screenshot ? ` ([shot](${p.screenshot}))` : ''} |`);
    lines.push('');
  }
  const allowedCount = r.results.reduce((n, p) => n + p.allowed.length, 0);
  if (allowedCount) lines.push(`${allowedCount} issue(s) suppressed by tools/qa/allowlist.json.`, '');
  lines.push('## Pages', '', '| Route | Lang | Viewport | Result | ms |', '|---|---|---|---|---|');
  for (const p of r.results) lines.push(`| \`${p.path}\` | ${p.lang} | ${p.viewport} | ${p.issues.length ? `${p.issues.length} issue(s)` : 'ok'} | ${p.ms} |`);
  return lines.join('\n') + '\n';
}

let code = 2;
try { code = await main(); } catch (e) { console.error('sweep error:', e.stack || e); } finally { cleanup(); }
process.exit(code);
