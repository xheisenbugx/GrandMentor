// Sound presets and the soundPicks validator (web/js/sound-catalog.js).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SOUND_EVENTS, SOUND_PRESETS, presetPicks, matchPreset, resolveStyle, isValidSoundPicks, stylesFor, defaultStyle } from '../../web/js/sound-catalog.js';

test('every preset round-trips through presetPicks → matchPreset', () => {
  for (const id of Object.keys(SOUND_PRESETS)) {
    const picks = presetPicks(id);
    assert.ok(isValidSoundPicks(picks), `${id} produces valid picks`);
    assert.equal(matchPreset(picks), id);
  }
  assert.deepEqual(presetPicks('classic'), {}, 'classic is the default sound: no overrides');
  assert.equal(presetPicks('nope'), null);
});

test('every event has a default style from its family', () => {
  for (const e of SOUND_EVENTS) {
    assert.ok(stylesFor(e.id).length > 0, `${e.id} has styles`);
    assert.equal(defaultStyle(e.id), stylesFor(e.id)[0]);
  }
  assert.deepEqual(stylesFor('not-an-event'), []);
});

test('isValidSoundPicks rejects unknown events, styles and shapes', () => {
  const ev = SOUND_EVENTS[0].id;
  assert.equal(isValidSoundPicks({}), true);
  assert.equal(isValidSoundPicks({ [ev]: stylesFor(ev).at(-1) }), true);
  assert.equal(isValidSoundPicks({ [ev]: 'kazoo' }), false);
  assert.equal(isValidSoundPicks({ 'not-an-event': 'wood' }), false);
  assert.equal(isValidSoundPicks([]), false);
  assert.equal(isValidSoundPicks(null), false);
});

test('resolveStyle falls back to the default for bad picks', () => {
  const ev = SOUND_EVENTS[0].id;
  assert.equal(resolveStyle(null, ev), defaultStyle(ev));
  assert.equal(resolveStyle({ [ev]: 'kazoo' }, ev), defaultStyle(ev));
  assert.equal(matchPreset({ [ev]: 'kazoo' }), 'classic');
});
