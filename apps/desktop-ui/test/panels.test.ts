// The Match Photo / Overlays tray wires: the capability statement is kept
// verbatim, overlay rows are the host's own channel list, and both panels are
// real bodies (not pending).

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

import {
  MATCH_PHOTO_COMMAND,
  OVERLAYS_COMMAND,
  parseMatchPhotoAnswer,
  parseOverlaysAnswer,
  readMatchPhoto,
  readOverlays,
} from '../src/domains/concept/panels.ts';
import { CONCEPT_TRAY } from '../src/domains/concept/tray.ts';

test('the match photo answer keeps the capability statement', () => {
  const answer = parseMatchPhotoAnswer({
    modeled: false,
    reason: 'matchphoto.not_implemented: no photo model',
  });
  assert.deepEqual(answer, {
    modeled: false,
    reason: 'matchphoto.not_implemented: no photo model',
  });
  assert.throws(() => parseMatchPhotoAnswer(null), /matchphoto\.invalid_answer/);
});

test('the overlays answer keeps the host rows and drops malformed ones', () => {
  const answer = parseOverlaysAnswer({
    rows: [
      { key: 'axes', active: true },
      { key: 'grid', active: false },
      { key: '', active: true },
      { key: 'axes', active: false },
      { active: true },
    ],
  });
  assert.deepEqual(answer.rows, [
    { key: 'axes', active: true },
    { key: 'grid', active: false },
  ]);
  assert.throws(() => parseOverlaysAnswer(42), /overlays\.invalid_answer/);
});

test('both panel reads use their one frozen command', async () => {
  const seen: string[] = [];
  const match = await readMatchPhoto(async (command: string) => {
    seen.push(command);
    return { modeled: true, reason: '' };
  });
  const overlays = await readOverlays(async (command: string) => {
    seen.push(command);
    return { rows: [{ key: 'axes', active: true }] };
  });
  assert.deepEqual(seen, [MATCH_PHOTO_COMMAND, OVERLAYS_COMMAND]);
  assert.equal(match.modeled, true);
  assert.equal(overlays.rows.length, 1);
});

test('the concept tray carries real Match Photo and Overlays bodies', () => {
  for (const id of ['concept.match_photo', 'concept.overlays']) {
    const panel = CONCEPT_TRAY.find(entry => entry.id === id);
    assert.ok(panel, id);
    assert.notEqual(panel.kind, 'pending', id);
    assert.equal(panel.pendingKey, undefined, id);
  }
  assert.equal(CONCEPT_TRAY.find(entry => entry.id === 'concept.match_photo')?.kind, 'matchPhoto');
  assert.equal(CONCEPT_TRAY.find(entry => entry.id === 'concept.overlays')?.kind, 'overlays');
});
