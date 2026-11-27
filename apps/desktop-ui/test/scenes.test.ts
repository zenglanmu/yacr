// The Scenes tray: the `cad_scenes` read's normalisation and the panel's wiring.
//
// The panel is client-local chrome: it reads the host projection and dispatches
// ordinary commands, never mutating the document itself. These contracts pin
// that split (read vs `SCENE ...` dispatch) so a read can never move the camera
// and a row click can never be a local write.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SCENES_COMMAND,
  parseScenesAnswer,
  readScenes,
} from '../src/domains/concept/scenes.ts';
import { CONCEPT_TRAY } from '../src/domains/concept/tray.ts';
import { readModule } from './helpers.ts';

test('the scenes answer keeps usable rows and the host total', () => {
  const answer = parseScenesAnswer({
    scenes: [
      { name: 'Overview', is3d: false, space: 'Model', style: 'SHADED' },
      { name: 'Sheet', is3d: true, space: 'A1', style: 'WIREFRAME' },
      { name: '', is3d: false, space: 'Model', style: 'SHADED' },
      { name: 'Overview', is3d: true, space: 'Model', style: 'SHADED' },
      null,
      7,
    ],
    total: 9,
    truncated: true,
  });
  assert.deepEqual(answer.scenes.map(scene => scene.name), ['Overview', 'Sheet']);
  assert.equal(answer.scenes[0].style, 'SHADED');
  assert.equal(answer.scenes[1].is3d, true);
  assert.equal(answer.total, 9, 'the host total survives a short page');
  assert.equal(answer.truncated, true);
  // A missing payload is an empty answer, never a guessed list.
  assert.deepEqual(parseScenesAnswer(null), { scenes: [], total: 0, truncated: false });
});

test('the scenes read goes through the one IPC command', async () => {
  const calls: string[] = [];
  const answer = await readScenes(async (command: string) => {
    calls.push(command);
    return { scenes: [{ name: 'A', is3d: false, space: 'Model', style: 'SHADED' }], total: 1, truncated: false };
  });
  assert.deepEqual(calls, [SCENES_COMMAND]);
  assert.equal(calls[0], 'cad_scenes');
  assert.equal(answer.scenes.length, 1);
});

test('the concept tray carries a real Scenes panel, not a pending one', () => {
  const panel = CONCEPT_TRAY.find(entry => entry.id === 'concept.scenes');
  assert.ok(panel);
  assert.equal(panel.kind, 'scenes');
  assert.equal(panel.pendingKey, undefined);
});

test('the panel restores, saves and deletes through the SCENE command', async () => {
  const source = await readModule('domains/ui/DomainTray.tsx');
  assert.match(source, /readScenes\(invoke\)/);
  assert.match(source, /SCENE RESTORE \$\{scene\.name\}/);
  assert.match(source, /SCENE SAVE \$\{trimmed\}/);
  assert.match(source, /SCENE DELETE \$\{scene\.name\}/);
  // The panel never dispatches a bare SCENE (a name/option is required).
  assert.doesNotMatch(source, /runCommand\('SCENE'\)/);
});
