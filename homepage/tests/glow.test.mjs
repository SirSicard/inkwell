// The page's copy of Glow's tokens (src/data/glow.ts) against the app's own: design/tokens.json and
// the Mac shell's Glow.swift, one and two levels up. Fails until a change there is copied here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { MODES, PRESETS, FIT, ORB, WANDER_MAC, REST_TINT } from '../src/data/glow.ts';

const tokens = JSON.parse(readFileSync(new URL('../../design/tokens.json', import.meta.url), 'utf8'));
const glowSwift = readFileSync(new URL('../../mac/Sources/Inkwell/Glow/Glow.swift', import.meta.url), 'utf8');

test('mode colours match design/tokens.json', () => {
  for (const mode of ['light', 'dark']) {
    for (const [key, hex] of Object.entries(MODES[mode])) {
      assert.equal(hex.toUpperCase(), tokens.modes[mode].colors[key].toUpperCase(), `${mode}.${key}`);
    }
  }
});

test('dot presets, fit and orb placement match design/tokens.json', () => {
  assert.deepEqual(PRESETS.map((p) => ({ ...p })), tokens.presets);
  assert.deepEqual({ ...FIT, luma: [...FIT.luma] }, tokens.fit);
  assert.deepEqual(JSON.parse(JSON.stringify(ORB)), tokens.orb);
});

test('the wander region and the rest tint match Glow.swift', () => {
  const m = /static let wander = OrbWander\.Bounds\(x: ([\d.]+)\.\.\.([\d.]+), y: ([\d.]+)\.\.\.([\d.]+)\)/.exec(glowSwift);
  assert.ok(m, 'Glow.Orb.wander not found');
  assert.deepEqual([WANDER_MAC.x, WANDER_MAC.y].flat(), m.slice(1).map(Number));
  assert.equal(REST_TINT, Number(/static let restTint = ([\d.]+)/.exec(glowSwift)[1]));
});
