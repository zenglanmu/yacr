// Behaviour contract for the extracted shortcut runner.
//
// `layout-surfaces.ts` answers "what does this chord mean in this layout"; this
// file pins the second half: exactly which setter fires, and that `false` really
// means "no action" so the caller leaves the event alone. Plain function doubles
// make every branch testable without a DOM.
//
// There is no refusal branch to test any more: the properties inspector has a
// compact surface (its own bottom sheet), so the one chord that used to refuse on
// mobile now toggles. The typed-selector refusal for the *layers* palette lives
// in `mobile-actions.test.ts`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createShortcutRunner } from '../src/layout/shell-shortcuts.ts';
import type { ShortcutTargets } from '../src/layout/shell-shortcuts.ts';

function harness() {
  const calls: string[] = [];
  const targets: ShortcutTargets = {
    setCleanScreen: update => { calls.push(`clean:${update(false)}`); },
    setPropertiesOpen: update => { calls.push(`properties:${update(false)}`); },
    setCommandLineOpen: update => { calls.push(`commandLine:${update(false)}`); },
    setHistoryOpen: update => { calls.push(`history:${update(false)}`); },
  };
  return { calls, targets };
}

test('the desktop layout drives the four chrome surfaces', () => {
  const { calls, targets } = harness();
  const run = createShortcutRunner('desktop', false, targets);
  assert.equal(run('cleanScreen'), true);
  assert.equal(run('properties'), true);
  assert.equal(run('commandLine'), true);
  assert.equal(run('history'), true);
  assert.deepEqual(calls, ['clean:true', 'properties:true', 'commandLine:true', 'history:true']);
});

test('the compact layout toggles the properties sheet and claims nothing else', () => {
  const { calls, targets } = harness();
  const run = createShortcutRunner('mobile', false, targets);
  // The inspector exists on this layout too — as its bottom sheet — so the chord
  // toggles the one flag instead of putting a refusal on the error line.
  assert.equal(run('properties'), true, 'the sheet is a real surface');
  assert.equal(run('cleanScreen'), false, 'no clean screen exists: leave the event alone');
  assert.equal(run('commandLine'), false, 'the command bar is unconditional');
  assert.equal(run('history'), true, 'the command bar renders the history sheet');
  assert.deepEqual(calls, ['properties:true', 'history:true']);
});

test('a tool text step drops the compact history sheet instead of toggling nothing', () => {
  const { calls, targets } = harness();
  const run = createShortcutRunner('mobile', true, targets);
  assert.equal(run('history'), false);
  assert.deepEqual(calls, []);
  // The desktop band keeps the panel in both modes.
  const desktop = harness();
  assert.equal(createShortcutRunner('desktop', true, desktop.targets)('history'), true);
  assert.deepEqual(desktop.calls, ['history:true']);
});
