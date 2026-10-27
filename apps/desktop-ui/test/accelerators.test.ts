// The documented AutoCAD control accelerators (Ctrl+A/C/X/V/Z/Y/Home/O/N/S/P),
// the one pure mapping table behind them, and the one wiring point in the shell.
//
// The table is exercised directly (it takes plain records, no DOM), and the
// call site is asserted by source structure — the same contract style the other
// chrome suites use, because this app has no mounted-DOM test harness.
import test from 'node:test';
import assert from 'node:assert/strict';
import { acceleratorCommand, acceleratorDispatch } from '../src/commands/accelerators.ts';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { browserCommandForKey } from '../src/defaults.ts';
import { HOST_NAMES } from './helpers.ts';
import { readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// The host's capability shape for an open drawing (the booleans `canCommand`
// reads through `admitCommandText`).
const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};

// The same shell admission the ribbon, the QAT and the command line use.
const admitted = (command, status = loaded, pending = false) =>
  admitCommandText(command, { status, pending }).kind === 'admitted';
const dispatchWith = (event, context = {}, status = loaded) =>
  acceleratorDispatch(event, context, command => admitted(command, status));

const ctrl = (key, extra = {}) => ({ key, ctrlKey: true, ...extra });

test('every documented control accelerator resolves to its AutoCAD command', () => {
  const cases = [
    [ctrl('a'), 'SELECTALL'],
    [ctrl('A'), 'SELECTALL'],
    [ctrl('c'), 'COPYCLIP'],
    [ctrl('C'), 'COPYCLIP'],
    [ctrl('x'), 'CUTCLIP'],
    [ctrl('X'), 'CUTCLIP'],
    [ctrl('v'), 'PASTECLIP'],
    [ctrl('V'), 'PASTECLIP'],
    [ctrl('z'), 'UNDO'],
    [ctrl('Z'), 'UNDO'],
    [ctrl('y'), 'REDO'],
    [ctrl('Y'), 'REDO'],
    [ctrl('z', { shiftKey: true }), 'REDO'],
    [ctrl('o'), 'OPEN'],
    [ctrl('O'), 'OPEN'],
    [ctrl('n'), 'NEW'],
    [ctrl('N'), 'NEW'],
    [ctrl('s'), 'SAVE'],
    [ctrl('S'), 'SAVE'],
    [ctrl('s', { shiftKey: true }), 'SAVE'],
    [ctrl('p'), 'PLOT'],
    [ctrl('P'), 'PLOT'],
    // Ctrl+Home is AutoCAD's zoom-extents (`FIT`); the key is a named key, so it
    // keeps its exact spelling instead of being case-folded.
    [ctrl('Home'), 'FIT'],
  ];
  for (const [event, expected] of cases) {
    assert.equal(acceleratorCommand(event), expected, `${event.key}${event.shiftKey ? '+shift' : ''}`);
  }
});

test('Ctrl+Shift+Z is REDO while Ctrl+Z is UNDO, and Ctrl+Y is the same REDO', () => {
  assert.notEqual(acceleratorCommand(ctrl('z', { shiftKey: true })), acceleratorCommand(ctrl('z')));
  assert.equal(acceleratorCommand(ctrl('z')), 'UNDO');
  assert.equal(acceleratorCommand(ctrl('Z', { shiftKey: true })), 'REDO');
  assert.equal(acceleratorCommand(ctrl('y')), 'REDO');
  // Only the two documented shifted forms exist. Every other shifted letter is
  // declined, so Ctrl+Shift+C/X/V stay clear of AutoCAD's COPYBASE/CUTBASE/
  // PASTEBASE family (commands this build does not implement).
  assert.equal(acceleratorCommand(ctrl('s', { shiftKey: true })), 'SAVE');
  for (const key of ['a', 'c', 'x', 'v', 'o', 'n', 'p', 'y']) {
    assert.equal(acceleratorCommand(ctrl(key, { shiftKey: true })), null, `Ctrl+Shift+${key}`);
  }
  assert.equal(acceleratorCommand(ctrl('Home', { shiftKey: true })), null);
});

test('text entry, IME composition, an open popup, a tool and a grip drag all decline the chord', () => {
  // A focused text entry keeps native copy/paste/undo/select-all and its IME.
  assert.equal(acceleratorCommand(ctrl('a'), { typing: true }), null);
  assert.equal(acceleratorCommand(ctrl('v'), { typing: true }), null);
  assert.equal(acceleratorCommand(ctrl('z'), { typing: true }), null);
  // An IME composition owns the press, modern and legacy (`keyCode === 229`).
  assert.equal(acceleratorCommand({ key: 'a', ctrlKey: true, isComposing: true }), null);
  assert.equal(acceleratorCommand({ key: 'a', ctrlKey: true, keyCode: 229 }), null);
  // A host dialog is modal: its overlay owns the accelerators it defines.
  assert.equal(acceleratorCommand(ctrl('c'), { popupOpen: true }), null);
  // A CAD tool owns Enter/Escape; the chords must not run against a live capture.
  assert.equal(acceleratorCommand(ctrl('z'), { toolActive: true }), null);
  assert.equal(acceleratorCommand(ctrl('v'), { toolActive: true }), null);
  // The terminal text step owns the keyboard even when focus left the field.
  assert.equal(acceleratorCommand(ctrl('a'), { textStep: true }), null);
  // An armed grip drag is not a tool, but it owns the press the same way.
  assert.equal(acceleratorCommand(ctrl('z'), { gripDragActive: true }), null);
  // Every flag at once is still a decline, never a partial dispatch.
  assert.equal(acceleratorCommand(ctrl('c'), {
    typing: true, popupOpen: true, toolActive: true, textStep: true, gripDragActive: true,
  }), null);
});

test('Meta and Alt chords, plain keys and Ctrl chords outside the table are never intercepted', () => {
  for (const event of [
    { key: 'a', metaKey: true },
    { key: 'a', ctrlKey: true, metaKey: true },
    { key: 'a', ctrlKey: true, altKey: true },
    { key: 'z', ctrlKey: true, altKey: true, shiftKey: true },
    { key: 'c' },
    { key: 'Home' },
    { key: 'z' },
    // Chrome toggles and function keys keep their own branches.
    { key: '0', ctrlKey: true },
    { key: '1', ctrlKey: true },
    { key: '9', ctrlKey: true },
    { key: 'F2' },
    { key: 'F3', ctrlKey: true },
    // Ctrl chords this app deliberately leaves to the webview guard (`defaults.ts`)
    // or to native editing: reload, bold, find, quick-open, devtools.
    { key: 'r', ctrlKey: true, shiftKey: true },
    { key: 'b', ctrlKey: true },
    { key: 'f', ctrlKey: true },
    { key: 'q', ctrlKey: true },
    { key: 'i', ctrlKey: true, shiftKey: true },
    // Named keys that are not chords in the table.
    { key: 'End', ctrlKey: true },
    { key: 'ArrowHome', ctrlKey: true },
    { key: ' ' },
    { key: 'Dead' },
    { key: 'Unidentified', ctrlKey: true },
    {},
  ]) {
    assert.equal(acceleratorCommand(event), null, JSON.stringify(event));
  }
  // The webview guard still claims print and zoom for itself: these chords are
  // the app's now, and the guard never gets to run its browser action.
  assert.equal(browserCommandForKey({ key: 'p', ctrlKey: true }), 'print');
  assert.equal(acceleratorCommand(ctrl('p')), 'PLOT');
});

test('a chord whose command the shell does not admit is not dispatched', () => {
  // Every mapped chord is admitted for an open drawing, so the gate is the
  // command's own admission — not a second, private list here.
  for (const [event, expected] of [
    [ctrl('a'), 'SELECTALL'], [ctrl('c'), 'COPYCLIP'], [ctrl('x'), 'CUTCLIP'],
    [ctrl('v'), 'PASTECLIP'], [ctrl('z'), 'UNDO'], [ctrl('y'), 'REDO'],
    [ctrl('Home'), 'FIT'], [ctrl('o'), 'OPEN'], [ctrl('n'), 'NEW'],
    [ctrl('s'), 'SAVE'], [ctrl('p'), 'PLOT'],
  ]) {
    assert.equal(dispatchWith(event), expected, `${event.key} dispatches ${expected}`);
  }
  // Nothing is admitted → nothing is dispatched, and nothing is prevented (the
  // silence is the same `null` the caller turns into "leave the key alone").
  for (const event of [ctrl('a'), ctrl('c'), ctrl('z'), ctrl('Home'), ctrl('o'), ctrl('p')]) {
    assert.equal(acceleratorDispatch(event, {}, () => false), null, `${event.key} is refused`);
  }
  // A per-chord refusal only stops that chord.
  assert.equal(acceleratorDispatch(ctrl('p'), {}, command => command !== 'PLOT'), null);
  assert.equal(acceleratorDispatch(ctrl('c'), {}, command => command !== 'PLOT'), 'COPYCLIP');
  // The host capability booleans are the gates (`canCommand` → admission).
  assert.equal(dispatchWith(ctrl('o'), {}, { ...loaded, canOpen: false }), null);
  assert.equal(dispatchWith(ctrl('n'), {}, { ...loaded, canNew: false }), null);
  assert.equal(dispatchWith(ctrl('s'), {}, { ...loaded, canSave: false }), null);
  assert.equal(dispatchWith(ctrl('p'), {}, { ...loaded, canPlot: false }), null);
  assert.equal(dispatchWith(ctrl('Home'), {}, { ...loaded, canFit: false }), null);
  // A busy host refuses everything that is not chrome-local.
  for (const event of [ctrl('o'), ctrl('n'), ctrl('s'), ctrl('p'), ctrl('z')]) {
    assert.equal(acceleratorDispatch(event, {}, command => admitted(command, loaded, true)), null, `${event.key} waits`);
  }
  // No document: SAVE/PLOT/FIT are refused (their gates need a drawing), while
  // SELECTALL/UNDO keep exactly the admission the ribbon and command line give
  // them — the chord never invents a stricter rule than the explicit path.
  const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canNew: true, canSave: false, canPlot: false, canFit: false };
  assert.equal(dispatchWith(ctrl('s'), {}, empty), null);
  assert.equal(dispatchWith(ctrl('p'), {}, empty), null);
  assert.equal(dispatchWith(ctrl('Home'), {}, empty), null);
  assert.equal(dispatchWith(ctrl('a'), {}, empty), 'SELECTALL');
  assert.equal(dispatchWith(ctrl('z'), {}, empty), 'UNDO');
  // And a chord the table declines never even asks the admission function.
  let asked = 0;
  acceleratorDispatch({ key: 'q', ctrlKey: true }, { typing: true }, () => { asked += 1; return true; });
  assert.equal(asked, 0);
});

test('the shell wires one accelerator call site before the defaultPrevented guard and after Ctrl+0', async () => {
  const app = await readModule('App.tsx');
  assert.ok(app.includes("import { acceleratorDispatch } from './commands/accelerators.ts';"));
  assert.equal(app.split('acceleratorDispatch(').length - 1, 1, 'one call site, never a second table');
  assert.match(
    app,
    /acceleratorDispatch\(event, \{\s*typing: isTyping\(event\.target\) \|\| isTextEntryTarget\(event\.target\),\s*popupOpen,\s*toolActive,\s*textStep,\s*gripDragActive,\s*\}, canCommand\);/,
    'the table gets the real focus/session facts and the shell admission',
  );
  const callAt = app.indexOf('const accelerator = acceleratorDispatch(event, {');
  assert.ok(callAt !== -1);
  // Declined presses are never prevented: the check is "did we get a command".
  const dispatchAt = app.indexOf('if (accelerator) {', callAt);
  const preventAt = app.indexOf('event.preventDefault();', dispatchAt);
  const submitAt = app.indexOf('void submitCommand(accelerator);', dispatchAt);
  assert.ok(dispatchAt > callAt && preventAt > dispatchAt && submitAt > preventAt,
    'preventDefault sits inside the dispatch branch, immediately before the submit');
  // Order inside the handler: popup guard, then Ctrl+0, then the table, then the
  // defaultPrevented/modifier guard (the Ctrl+P listener-order exception).
  const popupAt = app.indexOf('if (popupOpen) return;');
  const zeroAt = app.indexOf("!event.altKey && event.key === '0'");
  const defaultPreventedAt = app.indexOf('if (event.defaultPrevented || event.metaKey || event.altKey) return;');
  const guardAt = app.indexOf('bindBrowserDefaults(window, {');
  assert.ok(popupAt !== -1 && zeroAt !== -1 && defaultPreventedAt !== -1 && guardAt !== -1);
  assert.ok(popupAt < zeroAt && zeroAt < callAt, 'a popup and Ctrl+0 still come first');
  assert.ok(callAt < defaultPreventedAt, 'Ctrl+P must not depend on listener order with the guard');
  assert.ok(callAt < guardAt, 'the shell handler is installed before the webview guard');
});

test('the shell dispatches the mapped name through submitCommand and prevents the default only with it', async () => {
  const app = await readModule('App.tsx');
  // The dispatch is the same funnel every affordance uses: no IPC of its own.
  assert.ok(app.includes('void submitCommand(accelerator);'));
  const callAt = app.indexOf('const accelerator = acceleratorDispatch(event, {');
  const block = app.slice(callAt, app.indexOf('if (event.defaultPrevented', callAt));
  assert.equal(block.includes('invoke('), false, 'the accelerators never call the router directly');
  assert.equal(block.includes('setCleanScreen'), false, 'no chrome toggle is run by a document chord');
  assert.equal(block.includes('setHistoryOpen'), false, 'no chrome toggle is run by a document chord');
  // It runs through the one admission the chrome uses everywhere.
  assert.ok(app.includes('[status, pending, catalogNames]'), 'canCommand stays the shared admission');
  assert.ok(app.includes('}, [popupOpen, submitCommand, toolActive, gripDragActive, textStep, canCommand]);'),
    'the listener re-installs when the accelerator inputs change');
  assert.ok(app.includes('const textStep = awaitingToolText(status);'));
});

test('the existing Ctrl+0/1/9, F2, drafting and Enter/Escape branches are unchanged', async () => {
  const app = await readModule('App.tsx');
  assert.ok(app.includes("if (event.ctrlKey && !event.metaKey && !event.altKey && event.key === '0')"));
  assert.ok(app.includes('setCleanScreen(value => !value);'));
  assert.ok(app.includes("if (event.ctrlKey && event.key === '9')"));
  assert.ok(app.includes('setCommandLineOpen(value => !value);'));
  assert.ok(app.includes("if (event.ctrlKey && event.key === '1')"));
  assert.ok(app.includes('setPropertiesOpen(value => !value);'));
  assert.ok(app.includes("if (event.key === 'F2')"));
  assert.ok(app.includes('setHistoryOpen(value => !value);'));
  assert.ok(app.includes('const draftingCommand = draftingAcceleratorCommand(event.key);'));
  assert.ok(app.includes('if (draftingCommand) { event.preventDefault(); void submitCommand(draftingCommand); return; }'));
  assert.ok(app.includes('toolCommandForKey(event.key, {'));
  // The tool/Escape branch keeps its own focus predicate: the accelerator
  // `typing` union is additive and must not quietly widen tool cancellation.
  assert.ok(app.includes('typing: isTyping(event.target)'));
  assert.ok(app.includes('if (isTyping(event.target)) event.target.blur();'));
  // The table never claims a chrome toggle or a function key: those branches are
  // the only owners, so behaviour cannot change under them.
  for (const key of ['0', '1', '9', 'F2', 'F3', 'F7', 'F8', 'F9', 'Enter', 'Escape']) {
    assert.equal(acceleratorCommand({ key, ctrlKey: key.length === 1 }), null, key);
  }
});

test('the accelerator table is pure: no React, no DOM, no dispatch of its own', async () => {
  const source = await readModule('commands/accelerators.ts');
  assert.equal(/from ['"]react/.test(source), false, 'no React');
  assert.equal(source.includes('HTMLElement'), false, 'no DOM types');
  assert.equal(source.includes('addEventListener'), false, 'no listeners');
  assert.equal(source.includes('preventDefault'), false, 'the caller owns preventDefault');
  assert.equal(source.includes('submitCommand'), false, 'the caller owns dispatch');
  assert.equal(source.includes('invoke'), false, 'no IPC');
  assert.equal(source.includes("t('"), false, 'no user-visible copy');
  assert.ok(source.includes('export function acceleratorCommand('));
  assert.ok(source.includes('export function acceleratorDispatch('));
  assert.equal(source.split('export function').length - 1, 2, 'exactly two exports-of-functions');
});
