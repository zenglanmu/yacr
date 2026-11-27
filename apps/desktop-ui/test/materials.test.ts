// The Materials tray: the document read and the built-in palette.
//
// The document list comes from `cad_materials`; the palette is a frozen
// frontend constant. Both dispatch the ordinary `PAINT <name>` command, and a
// malformed row is dropped rather than guessed.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  BUILTIN_MATERIALS,
  MATERIALS_COMMAND,
  paintCommand,
  parseMaterialsAnswer,
} from '../src/domains/concept/materials.ts';

test('the document read normalises name, colour and transparency', () => {
  const answer = parseMaterialsAnswer({
    materials: [
      { name: 'Red', color: [178, 34, 34], transparency: 0 },
      { name: 'Glass', color: [170, 220, 240], transparency: 0.6 },
    ],
    total: 2,
    truncated: false,
  });
  assert.deepEqual(answer.materials, [
    { name: 'Red', color: '#b22222', transparency: 0 },
    { name: 'Glass', color: '#aadcf0', transparency: 0.6 },
  ]);
  assert.equal(answer.total, 2);
  assert.equal(MATERIALS_COMMAND, 'cad_materials');
});

test('malformed rows are dropped instead of fabricated', () => {
  const answer = parseMaterialsAnswer({
    materials: [
      { name: '', color: [1, 2, 3], transparency: 0 },
      { name: 'NoColor', transparency: 0 },
      { name: 'BadColor', color: [1, 2], transparency: 0 },
      { name: 'OutOfRange', color: [1, 2, 300], transparency: 0 },
      { name: 'BadAlpha', color: [1, 2, 3], transparency: 2 },
      { name: 'Red', color: [178, 34, 34], transparency: 0 },
      { name: 'Red', color: [1, 2, 3], transparency: 0 },
    ],
    total: 7,
    truncated: true,
  });
  assert.deepEqual(answer.materials, [
    { name: 'Red', color: '#b22222', transparency: 0 },
  ]);
  assert.equal(answer.total, 7);
  assert.equal(answer.truncated, true);
});

test('both lists dispatch the same PAINT selector', () => {
  assert.equal(paintCommand('Red'), 'PAINT Red');
  assert.equal(paintCommand('Glass'), 'PAINT Glass');
  assert.ok(BUILTIN_MATERIALS.length >= 12);
  assert.equal(BUILTIN_MATERIALS[0].name, 'Red');
});
