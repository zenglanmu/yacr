// N5 regression: the CAD pointer lane has two independent suspension reasons.
//
// `textFocus` (the command line owns the keyboard) and `popupOpen` (a host
// dialog is open) each own one bit; the lane is suspended while either holds.
// The command line's `onBlur` is unconditional and an outside press triggers it
// — with a modal popup open that must NOT resume the lane, or every forwarded
// (and host-refused) pointer event would surface as an error banner on the main
// window while the user is looking at the dialog.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createSuspendGate } from '../src/cad/input.ts';
import { readModule } from './helpers.ts';

/** A gate wired to a fake lane binding, the way `App` wires a real one. */
function harness() {
  const applied = [];
  const binding = { suspend: value => applied.push(value) };
  const gate = createSuspendGate(value => binding.suspend(value));
  gate.sync(); // App re-asserts onto every fresh binding
  return { applied, gate };
}

test('popup open + command-line blur still leaves the lane suspended', () => {
  const { applied, gate } = harness();
  assert.deepEqual(applied, [false], 'a fresh binding starts accepting');

  gate.set('textFocus', true);   // user clicks into the command line
  gate.set('popupOpen', true);   // host opens the modal About dialog
  gate.set('textFocus', false);  // an outside press blurs the command line

  assert.equal(gate.isSuspended(), true, 'the popup must survive the blur');
  assert.equal(applied.at(-1), true, 'the lane must not be resumed under the popup');
  assert.deepEqual(gate.reasons(), { textFocus: false, popupOpen: true });
});

test('closing the popup with no text focus resumes the lane', () => {
  const { applied, gate } = harness();
  gate.set('popupOpen', true);
  gate.set('popupOpen', false);
  assert.equal(gate.isSuspended(), false);
  assert.equal(applied.at(-1), false);
});

test('closing the popup while the command line is focused stays suspended', () => {
  const { applied, gate } = harness();
  gate.set('textFocus', true);
  gate.set('popupOpen', true);
  gate.set('popupOpen', false);
  assert.equal(gate.isSuspended(), true);
  assert.equal(applied.at(-1), true);
  assert.deepEqual(gate.reasons(), { textFocus: true, popupOpen: false });
});

test('each owner changes only its own reason, and non-booleans change nothing', () => {
  const { applied, gate } = harness();
  assert.equal(gate.set('pointerDown', true), false, 'an unknown reason is ignored');
  assert.equal(gate.set('textFocus', true), true);
  assert.equal(gate.set('textFocus', true), false, 'a repeated value applies nothing');
  assert.equal(gate.set('textFocus', 'yes'), false, 'a non-boolean is a caller bug, not a release');
  assert.equal(gate.set('popupOpen', 1), false);
  assert.deepEqual(applied, [false, true]);
  assert.deepEqual(gate.reasons(), { textFocus: true, popupOpen: false });
});

test('sync re-asserts onto a remounted binding instead of resuming it', () => {
  const applied = [];
  let current = { suspend: value => applied.push(value) };
  const gate = createSuspendGate(value => current.suspend(value));
  gate.sync();
  gate.set('popupOpen', true);
  // Remount: the effect creates a fresh binding and calls `sync` on it.
  current = { suspend: value => applied.push(value) };
  gate.sync();
  assert.deepEqual(applied, [false, true, true]);
  assert.equal(gate.isSuspended(), true);
});

test('App wires status.popup as its own reason and text focus as the other', async () => {
  const app = await readModule('App.tsx');
  assert.ok(app.includes('createSuspendGate(value => bindingRef.current?.suspend(value))'));
  assert.ok(app.includes('const popupOpen = status?.popup != null;'));
  // The non-null assertion on the ref is the only change: both reasons still go
  // through the one gate binding.
  assert.match(app, /suspendGate\.current!?\.set\('popupOpen', popupOpen\)/);
  assert.match(app, /suspendPointer: value => suspendGate\.current!?\.set\('textFocus', value\)/);
  assert.match(app, /suspendGate\.current!?\.sync\(\)/);

  // The command line keeps its unconditional focus/blur calls — it just no
  // longer owns the popup's reason.
  const commandLine = await readModule('components/CommandLine.tsx');
  assert.ok(commandLine.includes('shell.suspendPointer(true)'));
  assert.ok(commandLine.includes('shell.suspendPointer(false)'));
  assert.equal(commandLine.includes("'popupOpen'"), false);
});
