// The Soften Edges panel: a real tray body with one validated angle.
//
// All semantics live in the `SOFTENEDGES` command; the panel only refuses an
// out-of-range input locally (so a click is never meaningless) and dispatches
// the typed line through the ordinary command path.

import test from 'node:test';
import assert from 'node:assert/strict';
import { CONCEPT_TRAY } from '../src/domains/concept/tray.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { readModule } from './helpers.ts';

test('the concept tray carries a real Soften Edges panel, not a pending one', () => {
  const panel = CONCEPT_TRAY.find(entry => entry.id === 'concept.soften_edges');
  assert.ok(panel);
  assert.equal(panel.kind, 'soften');
  assert.equal(panel.pendingKey, undefined);
});

test('the catalog names SOFTENEDGES once with a real label', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'SOFTENEDGES');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].labelKey, 'draw.kind.softenedges');
});

test('the panel validates 0..=180 and dispatches the typed SCENE-like line', async () => {
  const source = await readModule('domains/ui/DomainTray.tsx');
  assert.match(source, /shell\.canCommand\('SOFTENEDGES'\)/);
  assert.match(source, /parsed >= 0 && parsed <= 180/);
  assert.match(source, /SOFTENEDGES \$\{parsed\}/);
});
