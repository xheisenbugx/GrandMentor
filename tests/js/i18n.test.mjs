// t(): lookup, {placeholder} interpolation, plural forms, English fallback and language switching.
import './dom-shim.mjs';
import { test, before } from 'node:test';
import assert from 'node:assert/strict';

const i18n = await import('../../web/js/i18n.js');
const { setSetting } = await import('../../web/js/settings.js');
const en = (await import('../../web/locales/en/index.js')).default;
const es = (await import('../../web/locales/es/index.js')).default;

/** Switch the UI language through settings, like the Settings page does. */
function switchTo(code) {
  return new Promise((resolve) => {
    const off = i18n.onLanguageChange((l) => { off(); resolve(l); });
    setSetting('language', code);
  });
}

before(async () => { await i18n.initI18n(); });

test('starts in English and resolves a plain key', () => {
  assert.equal(i18n.getLanguage(), 'en');
  assert.equal(i18n.getLocale(), 'en-US');
  assert.equal(i18n.t('lesson.hanging.check'), en.lesson.hanging.check);
  assert.equal(i18n.hasKey('lesson.hanging.check'), true);
});

test('unknown keys come back unchanged (and are not "undefined")', (t) => {
  t.mock.method(console, 'warn', () => {});
  assert.equal(i18n.t('nope.not.a.key'), 'nope.not.a.key');
  assert.equal(i18n.hasKey('nope.not.a.key'), false);
});

test('interpolates {placeholders} and keeps unknown ones', () => {
  assert.equal(i18n.t('home.greeting.morning', { name: 'Alex' }), 'Good morning, Alex!');
  // A missing or null value leaves the placeholder visible instead of printing "undefined".
  assert.equal(i18n.t('home.greeting.morning'), 'Good morning, {name}!');
  assert.equal(i18n.t('home.greeting.morning', { name: null }), 'Good morning, {name}!');
  assert.equal(i18n.t('home.greeting.morning', { name: 0 }), 'Good morning, 0!');
});

test('plural forms follow Intl.PluralRules, with an explicit zero form', () => {
  assert.equal(i18n.t('lesson.guess.points', { count: 0 }), '0 pts');
  assert.equal(i18n.t('lesson.guess.points', { count: 1 }), '+1 pt');
  assert.equal(i18n.t('lesson.guess.points', { count: 5 }), '+5 pts');
  // No zero form: 0 uses the language's rule ("other" in English).
  assert.equal(i18n.t('lesson.hanging.wrongPicks', { count: 0 }), '0 of your picks are safe.');
  assert.equal(i18n.t('lesson.hanging.wrongPicks', { count: 1 }), '1 of your picks is safe.');
  // A missing count counts as 0.
  assert.equal(i18n.t('lesson.guess.points'), '0 pts');
});

test('locale-aware formatting helpers', () => {
  assert.equal(i18n.formatNumber(1234567), '1,234,567');
  assert.equal(i18n.formatList(['a', 'b', 'c']), 'a, b, and c');
  assert.equal(i18n.formatDateIntl('not a date'), '');
});

test('switching to Spanish changes messages, plurals and <html lang>', async () => {
  assert.equal(await switchTo('es'), 'es');
  assert.equal(i18n.getLanguage(), 'es');
  assert.equal(document.documentElement.lang, 'es');
  assert.equal(i18n.t('lesson.hanging.check'), es.lesson.hanging.check);
  const one = es.lesson.guess.points.one.replace('{count}', '1');
  assert.equal(i18n.t('lesson.guess.points', { count: 1 }), one);
  assert.equal(i18n.formatNumber(1234567), new Intl.NumberFormat('es-ES').format(1234567));
});

test('an unsupported language is rejected and the current one stays', (t) => {
  t.mock.method(console, 'warn', () => {});
  assert.equal(setSetting('language', 'xx'), false);
  assert.equal(i18n.getLanguage(), 'es');
});

test('switching back to English', async () => {
  assert.equal(await switchTo('en'), 'en');
  assert.equal(i18n.t('lesson.hanging.check'), en.lesson.hanging.check);
  assert.equal(document.documentElement.lang, 'en');
});
