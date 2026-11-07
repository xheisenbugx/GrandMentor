// Hash router in web/js/app.js: parseHash() and matchRoute(), plus the ROUTES table itself.
import './dom-shim.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// The shim keeps document.readyState at "loading", so importing app.js does not boot the app.
const { parseHash, matchRoute } = await import('../../web/js/app.js');
const WEB = fileURLToPath(new URL('../../web/', import.meta.url));

test('parseHash normalises the path and splits the query', () => {
  assert.deepEqual(parseHash(''), { path: '/', segments: [], query: {} });
  assert.deepEqual(parseHash('#'), { path: '/', segments: [], query: {} });
  assert.deepEqual(parseHash('#/'), { path: '/', segments: [], query: {} });
  assert.deepEqual(parseHash('#play'), { path: '/play', segments: ['play'], query: {} });
  assert.deepEqual(parseHash('#/play//pawnny/'), { path: '/play/pawnny', segments: ['play', 'pawnny'], query: {} });
  assert.deepEqual(parseHash('#/analysis?fen=8%2F8%2F8&flip=1'), { path: '/analysis', segments: ['analysis'], query: { fen: '8/8/8', flip: '1' } });
});

test('parseHash decodes segments and survives malformed escapes', () => {
  assert.deepEqual(parseHash('#/learn/my%20course/l1').segments, ['learn', 'my course', 'l1']);
  assert.deepEqual(parseHash('#/openings/%E0%A4%A').segments, ['openings', '%E0%A4%A']);
});

test('matchRoute fills :params and merges static route params', () => {
  const play = matchRoute(['play', 'pawnny']);
  assert.equal(play.route.page, 'play');
  assert.deepEqual(play.params, { botId: 'pawnny' });

  const lesson = matchRoute(['learn', 'basics', 'l2']);
  assert.equal(lesson.route.page, 'lesson');
  assert.deepEqual(lesson.params, { courseId: 'basics', lessonId: 'l2' });

  const rush = matchRoute(['puzzles', 'rush']);
  assert.equal(rush.route.page, 'puzzles');
  assert.deepEqual(rush.params, { mode: 'rush' });

  assert.equal(matchRoute([]).route.page, 'home');
});

test('matchRoute returns null for unknown paths', () => {
  assert.equal(matchRoute(['nope']), null);
  assert.equal(matchRoute(['play', 'a', 'b']), null);
  assert.equal(matchRoute(['learn', 'a', 'b', 'c']), null);
});

test('every route points at an existing page module and a real tab-title key', async () => {
  const src = readFileSync(new URL('app.js', new URL('../../web/js/', import.meta.url)), 'utf8');
  const table = src.slice(src.indexOf('const ROUTES'), src.indexOf('];', src.indexOf('const ROUTES')));
  const routes = [...table.matchAll(/\{\s*pattern:\s*'([^']+)',\s*page:\s*'([^']+)'[^}]*?titleKey:\s*'([^']+)'/g)].map((m) => ({ pattern: m[1], page: m[2], titleKey: m[3] }));
  assert.ok(routes.length >= 20, `parsed ${routes.length} routes from app.js`);
  const { default: en } = await import('../../web/locales/en/index.js');
  const lookup = (key) => key.split('.').reduce((o, k) => (o && typeof o === 'object' ? o[k] : undefined), en);
  for (const r of routes) {
    assert.ok(existsSync(`${WEB}js/pages/${r.page}.js`), `${r.pattern}: web/js/pages/${r.page}.js is missing`);
    assert.equal(typeof lookup(r.titleKey), 'string', `${r.pattern}: titleKey "${r.titleKey}" is not an English string`);
    // Each pattern must reach its own route: an earlier, broader pattern must not shadow it.
    const segs = r.pattern.split('/').filter(Boolean);
    const m = matchRoute(segs.map((s) => (s.startsWith(':') ? 'x' : s)));
    assert.equal(m?.route.pattern, r.pattern, `${r.pattern} is shadowed by ${m?.route.pattern}`);
  }
});
