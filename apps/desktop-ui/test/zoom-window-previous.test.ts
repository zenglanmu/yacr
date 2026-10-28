// `ZOOM WINDOW` / `ZOOM PREVIOUS`: the two **argument forms** of the `ZOOM`
// selector (2026-10-28).
//
// The contract these lock, in the chrome's own terms:
//
//   * they are **not** commands: the host adds no `CommandId` and advertises no
//     `ZOOM WINDOW` / `ZOOM PREVIOUS` name, so the words live in exactly one
//     place — the `ARGUMENT_COMMANDS.ZOOM` grammar, beside `ALL`/`EXTENTS`;
//   * they dispatch the **full line** on `cad_command` (never a fabricated
//     `ZOOM`, never a second IPC), so what the test asserts is the exact string
//     the host receives;
//   * the button is enabled by that same admission — enabled iff `ZOOM` is, with
//     no local enable path and no display-catalog row (a row would be the second
//     name list the command line must not need);
//   * the tooltips state the honest limits instead of promising AutoCAD parity:
//     two corners and 2D only (a 3D view refuses the form by name), and one
//     discrete view step back, with pan/wheel/orbit deliberately not recorded.
import test from 'node:test';
import assert from 'node:assert/strict';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';
import {
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  splitCommandText,
} from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { offeredItems, RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';

setCommandCatalog(HOST_NAMES);

// The formed selectors, spelled once: every assertion below reads this pair, so a
// re-spelling in the model or a stray lowercase variant fails here.
const ZOOM_FORMS = ['ZOOM WINDOW', 'ZOOM PREVIOUS'];

// A drawing is open and every navigation capability is available, so the only
// reason an admission could fail here is the command line itself.
const loaded = {
  documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true,
  gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false,
  canSave: true, canPlot: true, canFit: true,
};

function viewNavigatePanel() {
  const tab = RIBBON_TABS.find(entry => entry.id === 'view');
  const panel = tab.panels.find(entry => entry.id === 'navigate');
  assert.ok(panel, 'the View tab must keep its navigate panel');
  return panel;
}

function iconNames(source) {
  const map = source.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('the two zoom forms are grammar on ZOOM, not names the host advertises', () => {
  for (const line of ZOOM_FORMS) {
    const split = splitCommandText(line);
    // The base name is `ZOOM` and the word is its argument: that is the whole
    // reason no second IPC and no second vocabulary entry is needed.
    assert.equal(split.name, 'ZOOM', line);
    assert.notEqual(split.argument, null, line);
    assert.equal(ARGUMENT_COMMANDS.ZOOM.parse(split.argument), true, line);
    // The host publishes the base selector only — the mirror stays honest.
    assert.equal(HOST_NAMES.includes(line), false, `${line} must not enter the vocabulary mirror`);
    assert.equal(COMMAND_CATALOG.some(entry => entry.id === line), false,
      `${line} gets no display-catalog row: the argument table is the one list`);
  }
  // The argument-taking selectors are unchanged: this feature adds words to the
  // existing `ZOOM` grammar, never a fifth selector row.
  assert.deepEqual(Object.keys(ARGUMENT_COMMANDS), ['ZOOM', 'SNAPUNIT', 'PDMODE', 'PDSIZE']);
  // The numeric/`ALL`/`EXTENTS` grammar still answers exactly as it did, and a
  // *different* word is still refused (the match is the word, not a prefix).
  for (const admitted of ['2', '0.5', 'ALL', 'EXTENTS', 'WINDOW', 'PREVIOUS']) {
    assert.equal(ARGUMENT_COMMANDS.ZOOM.parse(admitted), true, admitted);
  }
  for (const refused of ['0', '2 3', 'WINDOWS', 'PREVIOUSLY', 'PREV', 'WIN']) {
    assert.equal(ARGUMENT_COMMANDS.ZOOM.parse(refused), false, refused);
  }
});

test('the form is enabled exactly when ZOOM is admitted, and disabled otherwise', () => {
  for (const line of ZOOM_FORMS) {
    const admission = admitCommandText(line, { status: loaded });
    assert.equal(admission.kind, 'admitted', line);
    assert.equal(admission.reasonKey, null, line);
    // The line, not the base name: the host receives the word and validates it.
    assert.equal(admission.command, splitCommandText(line).text, line);
    // Typing is normalized like every other command line.
    assert.equal(admitCommandText(`  ${line.toLowerCase()}  `, { status: loaded }).kind, 'admitted', line);
  }
  // Same gates as `ZOOM 2`: a command already in flight blocks the form with the
  // busy sentence, not a permanent-limitation one.
  assert.equal(admitCommandText('ZOOM WINDOW', { status: loaded, pending: true }).kind, 'unavailable');
  assert.equal(admitCommandText('ZOOM PREVIOUS', { status: null }).kind, 'unavailable');

  // Without `ZOOM` in the host vocabulary the form is refused as unsupported —
  // the button is disabled because the router never advertised the base name,
  // never because a local rule says so.
  setCommandCatalog(HOST_NAMES.filter(name => name !== 'ZOOM'));
  for (const line of ZOOM_FORMS) {
    assert.equal(isSupportedCommand(line), false, line);
    const admission = admitCommandText(line, { status: loaded });
    assert.equal(admission.kind, 'unsupported', line);
    assert.equal(admission.reasonKey, 'command.unsupported', line);
  }
  setCommandCatalog(HOST_NAMES);
  for (const line of ZOOM_FORMS) assert.equal(admitCommandText(line, { status: loaded }).kind, 'admitted', line);
});

test('ZOOM WINDOW / ZOOM PREVIOUS reach cad_command as exactly those lines', async () => {
  for (const line of ZOOM_FORMS) {
    const calls = [];
    const result = await queueCadCommand(
      async (...args) => { calls.push(args); return { accepted: true }; },
      line, loaded, false,
    );
    assert.equal(result.kind, 'accepted', line);
    assert.equal(result.command, line, line);
    // One `cad_command` call with the whole line: no `ZOOM` rewrite and no
    // second IPC carrying the word somewhere else.
    assert.deepEqual(calls, [['cad_command', { command: line }]], line);
  }
  // A word outside the grammar never reaches IPC at all.
  const refused = await queueCadCommand(
    () => assert.fail('an out-of-grammar ZOOM word reached IPC'),
    'ZOOM WINDOWS', loaded, false,
  );
  assert.equal(refused.kind, 'unsupported');
});

test('both entries sit in the View navigate cluster and name themselves honestly', async () => {
  const panel = viewNavigatePanel();
  const byId = Object.fromEntries(panel.items.map(item => [item.id, item]));
  assert.deepEqual(
    panel.items.map(item => item.id),
    ['ZOOMIN', 'ZOOMOUT', 'ZOOM WINDOW', 'PAN', 'FIT', 'ZOOM PREVIOUS', 'RESETVIEW'],
  );
  const iconNames_ = iconNames(await readModule('icons.tsx'));
  const expected = {
    'ZOOM WINDOW': { labelKey: 'toolbar.zoom_window', icon: 'zoomWindow', hintKey: 'toolbar.zoom_window.hint' },
    'ZOOM PREVIOUS': { labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', hintKey: 'toolbar.zoom_previous.hint' },
  };
  for (const line of ZOOM_FORMS) {
    const item = byId[line];
    assert.ok(item, `${line} must be offered by the View tab`);
    // The dispatch token is the whole line and the item id names it, so the
    // button can never look like one command while sending another.
    assert.equal(item.command, line, line);
    assert.equal(item.action, undefined, line);
    assert.equal(item.standardView, undefined, line);
    assert.equal(item.variants, undefined, `${line} has no options sub-prompt`);
    assert.deepEqual(
      { labelKey: item.labelKey, icon: item.icon, hintKey: item.hintKey },
      expected[line],
      line,
    );
    // A glyph name that does not exist would render the dot fallback silently.
    assert.equal(iconNames_.has(item.icon), true, `${line}: icons.tsx must define '${item.icon}'`);
    // Carrier-independent: zooming names no palette state, so the palette-less
    // carrier (`dom-canvas`) keeps the button.
    assert.equal(offeredItems(panel.items, false).some(entry => entry.id === line), true, line);
    assert.equal(ribbonCommandIds().includes(line), true, line);
  }
  // `shell.canCommand('ZOOM WINDOW')` is the only gate: the renderer dispatches
  // through the shared funnel, never through a local branch on the word.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  assert.equal(ribbon.includes('ZOOM WINDOW'), false, 'the component never spells the argument form itself');
});

test('the tooltips state the limits: two corners and 2D only; discrete steps only', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of ['toolbar.zoom_previous', 'toolbar.zoom_window.hint', 'toolbar.zoom_previous.hint']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN ${key} is empty`);
    assert.ok(en[key].trim().length > 0, `en ${key} is empty`);
  }
  // The window form: two corners, 2D only, a 3D view refuses it.
  assert.match(en['toolbar.zoom_window.hint'], /two corners/i);
  assert.match(en['toolbar.zoom_window.hint'], /2D only/i);
  assert.match(en['toolbar.zoom_window.hint'], /3D/);
  assert.match(zh['toolbar.zoom_window.hint'], /两个角点/);
  assert.match(zh['toolbar.zoom_window.hint'], /二维/);
  assert.match(zh['toolbar.zoom_window.hint'], /三维/);
  // The previous form: one discrete view back, and what is **not** a step. No
  // parity promise: pan/wheel/orbit are named as unrecorded.
  assert.match(en['toolbar.zoom_previous.hint'], /last recorded view/i);
  assert.match(en['toolbar.zoom_previous.hint'], /Pan/);
  assert.match(en['toolbar.zoom_previous.hint'], /orbit/i);
  assert.match(en['toolbar.zoom_previous.hint'], /not recorded/i);
  assert.match(zh['toolbar.zoom_previous.hint'], /上一个/);
  assert.match(zh['toolbar.zoom_previous.hint'], /平移/);
  assert.match(zh['toolbar.zoom_previous.hint'], /环绕/);
  assert.match(zh['toolbar.zoom_previous.hint'], /不记/);
  // Two different facts are worded twice, not once: the window limit is not the
  // previous limit.
  assert.notEqual(en['toolbar.zoom_window.hint'], en['toolbar.zoom_previous.hint']);
  assert.notEqual(zh['toolbar.zoom_window.hint'], zh['toolbar.zoom_previous.hint']);
  // The model points at those keys, and the renderer shows them as the enabled
  // tooltip (`hint`), so the copy is reachable — not a dead catalog row.
  for (const line of ZOOM_FORMS) {
    const item = viewNavigatePanel().items.find(entry => entry.id === line);
    assert.ok(Object.prototype.hasOwnProperty.call(en, item.hintKey), `${line}: ${item.hintKey}`);
  }
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('hint={item.hintKey ? t(item.hintKey) : undefined}'),
    'the enabled button must show the item hint');
  // And it is *not* the refusal sentence: the disabled branch keeps its own
  // wording (`unavailable`), so a live button never claims a reason it lacks.
  assert.ok(ribbon.includes('disabled hint={unavailable(t, item, shell)}'),
    'the refusal sentence stays on the disabled path');
});

test('the command line keeps suggesting the base name and keeps the typed word', () => {
  // Discoverability without a second list: the needle is the base name, so a
  // half-typed form still suggests the zoom family...
  const rows = filterCommandSuggestions('ZOOM W');
  assert.ok(rows.some(row => row.id === 'ZOOMIN'), 'the zoom family keeps matching');
  assert.ok(rows.some(row => row.id === 'ZOOMOUT'));
  // ...and the words themselves are **not** catalog rows: typing them is the
  // discoverability path, and the argument table is their only list.
  assert.equal(rows.some(row => ZOOM_FORMS.includes(row.id)), false);
  // Enter keeps the typed argument instead of replacing the line with `ZOOM`
  // (`CommandLine.effectiveText`), so the word reaches the host.
  assert.equal(splitCommandText('ZOOM W').argument, 'W');
  // A complete form is admitted as typed and keeps its own spelling.
  assert.equal(admitCommandText('ZOOM WINDOW', { status: loaded }).command, 'ZOOM WINDOW');
});
