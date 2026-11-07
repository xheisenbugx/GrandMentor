// Pure formatting helpers from web/js/ui.js and web/js/api.js.
import './dom-shim.mjs';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const ui = await import('../../web/js/ui.js');
const { qs, isAbort } = await import('../../web/js/api.js');

test('formatClock: m:ss, h:mm:ss and tenths under 10 seconds', () => {
  assert.equal(ui.formatClock(0), '0:00.0');
  assert.equal(ui.formatClock(9_999), '0:09.9');
  assert.equal(ui.formatClock(10_000), '0:10');
  assert.equal(ui.formatClock(65_000), '1:05');
  assert.equal(ui.formatClock(3_600_000 + 61_000), '1:01:01');
  assert.equal(ui.formatClock(5_300, { tenths: false }), '0:05');
  assert.equal(ui.formatClock(-50), '0:00.0');
  assert.equal(ui.formatClock('junk'), '0:00.0');
});

test('formatScore: centipawns, mates, point of view and bad input', () => {
  assert.equal(ui.formatScore({ cp: 123 }), '+1.2');
  assert.equal(ui.formatScore({ cp: -40 }), '-0.4');
  assert.equal(ui.formatScore({ cp: 4 }), '0.0');
  assert.equal(ui.formatScore({ cp: 150 }, { pov: 'black' }), '-1.5');
  assert.equal(ui.formatScore({ cp: 123 }, { digits: 2 }), '+1.23');
  assert.equal(ui.formatScore({ mate: 3 }), 'M3');
  assert.equal(ui.formatScore({ mate: -2 }), '-M2');
  assert.equal(ui.formatScore({ mate: 2 }, { pov: 'black' }), '-M2');
  assert.equal(ui.formatScore({ mate: 0 }), '#');
  assert.equal(ui.formatScore(null), '–');
  assert.equal(ui.formatScore({}), '–');
});

test('scoreToNumber clamps and winPercent is a 0..100 curve centred on 50', () => {
  assert.equal(ui.scoreToNumber({ cp: 250 }), 2.5);
  assert.equal(ui.scoreToNumber({ cp: 5000 }), 10);
  assert.equal(ui.scoreToNumber({ mate: -1 }), -10);
  assert.equal(ui.scoreToNumber(null), 0);
  assert.equal(ui.winPercent({ cp: 0 }), 50);
  assert.equal(ui.winPercent(null), 50);
  assert.equal(ui.winPercent({ mate: 1 }), 100);
  assert.equal(ui.winPercent({ mate: -1 }), 0);
  const up = ui.winPercent({ cp: 300 });
  assert.ok(up > 50 && up < 100);
  assert.ok(Math.abs(up + ui.winPercent({ cp: -300 }) - 100) < 1e-9, 'symmetric around 50');
});

test('escapeHtml and mdLite never let markup through', () => {
  assert.equal(ui.escapeHtml(`<a href="x">'&'</a>`), '&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;');
  assert.equal(ui.escapeHtml(null), '');
  assert.equal(ui.mdLite('**Bold** and *italic* with `code`'), '<p><strong>Bold</strong> and <em>italic</em> with <code>code</code></p>');
  assert.equal(ui.mdLite('one\ntwo\n\nthree'), '<p>one<br>two</p><p>three</p>');
  assert.equal(ui.mdLite('<script>x</script>'), '<p>&lt;script&gt;x&lt;/script&gt;</p>');
});

test('formatSan converts to figurines only when asked', () => {
  assert.equal(ui.formatSan('Nf3'), 'Nf3');
  assert.equal(ui.formatSan('Qxe7+', 'figurine'), '♕xe7+');
  assert.equal(ui.formatSan('e8=Q', 'figurine'), 'e8=♕');
  assert.equal(ui.formatSan(null, 'figurine'), '');
});

test('displayName hides the placeholder name', () => {
  assert.equal(ui.displayName('  Alex '), 'Alex');
  assert.equal(ui.displayName('Player'), '');
  assert.equal(ui.displayName(undefined), '');
});

test('qs skips empty values and isAbort only matches AbortError', () => {
  assert.equal(qs({}), '');
  assert.equal(qs({ a: 1, b: null, c: undefined, d: '', e: 'x y' }), '?a=1&e=x+y');
  assert.equal(isAbort({ name: 'AbortError' }), true);
  assert.equal(isAbort(new Error('x')), false);
  assert.equal(isAbort(null), false);
});
