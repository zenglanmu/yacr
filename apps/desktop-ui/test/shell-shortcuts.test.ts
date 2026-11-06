// Behaviour contract for the extracted shortcut runner.
//
// `layout-surfaces.ts` answers "what does this chord mean in this layout"; this
// file pins the second half: exactly which setter fires, that a refusal is
// visible, and that `false` really means "no action" so the caller leaves the
// event alone. Plain function doubles make every branch testable without a DOM.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createShortcutRunner } from '../src/layout/shell-shortcuts.ts';
import type { ShortcutTargets } from '../src/layout/shell-shortcuts.ts';
import type { Translator } from '../src/i18n.ts';

function harness() {
  const calls: string[] = [];
  const messages: string[] = [];
  const targets: ShortcutTargets = {
    setError: message => { calls.push('error'); messages.push(message); },
    setCleanScreen: update => { calls.push(`clean:${update(false)}`); },
    setPropertiesOpen: update => { calls.push(`properties:${update(false)}`); },
    setCommandLineOpen: update => { calls.push(`commandLine:${update(false)}`); },
    setHistoryOpen: update => { calls.push(`history:${update(false)}`); },
  };
  const t: Translator = (key, params) => {
    const panel = (params as { panel?: string } | null)?.panel;
    return panel ? `${key}|${panel}` : key;
  };
  return { calls, messages, targets, t };
}

test('the desktop layout drives the four chrome surfaces', () => {
  const { calls, targets, t } = harness();
  const run = createShortcutRunner('desktop', false, t, targets);
  assert.equal(run('cleanScreen'), true);
  assert.equal(run('properties'), true);
  assert.equal(run('commandLine'), true);
  assert.equal(run('history'), true);
  assert.deepEqual(calls, ['clean:true', 'properties:true', 'commandLine:true', 'history:true']);
});

test('the compact layout refuses the desktop palette in words and claims nothing else', () => {
  const { calls, messages, targets, t } = harness();
  const run = createShortcutRunner('mobile', false, t, targets);
  assert.equal(run('properties'), true, 'the refusal is a handled action');
  assert.equal(run('cleanScreen'), false, 'no clean screen exists: leave the event alone');
  assert.equal(run('commandLine'), false, 'the command bar is unconditional');
  assert.equal(run('history'), true, 'the command bar renders the history sheet');
  assert.deepEqual(calls, ['error', 'history:true']);
  assert.deepEqual(messages, ['mobile.refusal.docked_panel|properties.panel']);
});

test('a tool text step drops the compact history sheet instead of toggling nothing', () => {
  const { calls, targets, t } = harness();
  const run = createShortcutRunner('mobile', true, t, targets);
  assert.equal(run('history'), false);
  assert.deepEqual(calls, []);
  // The desktop band keeps the panel in both modes.
  const desktop = harness();
  assert.equal(createShortcutRunner('desktop', true, desktop.t, desktop.targets)('history'), true);
  assert.deepEqual(desktop.calls, ['history:true']);
});
