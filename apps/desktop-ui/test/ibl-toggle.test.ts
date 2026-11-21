// Contract tests for the View tab's IBL (image-based lighting) toggle.
//
// The renderer gains image-based lighting for PBR meshes, driven by the real
// session command `IBL` (bare = toggle, `IBL ON|OFF|1|0` = explicit set). The
// checked state is the host's own `cad_status.iblEnabled`, read through the
// ribbon model's `activeKey` mechanism — never a local mirror.
//
// These are model/predicate tests plus a narrow source scan for the rendering
// wiring, the same style the other ribbon suites use. They do not prove the
// Rust session executes the command (that is the host lane's contract).
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import { ribbonItemActive, ribbonItemActiveKey } from '../src/ribbon/ribbon-active.ts';
import { ribbonTab } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// A complete status with a drawing open and IBL on: the command is advertised by
// the host vocabulary mirror, so the only variable under test is the line shape.
const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  importProgress: null,
  error: null,
  canOpen: true,
  canNew: true,
  gridEnabled: true,
  snapGridEnabled: false,
  snapSpacing: 10,
  orthoEnabled: false,
  iblEnabled: true,
  canSave: true,
  canPlot: true,
  canFit: true,
  backend: null,
  adapter: null,
  camera: null,
  importCount: 0,
  frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

/** The View tab's display panel, where the display-style toggles live. */
function displayPanel() {
  return ribbonTab('view').panels.find(panel => panel.id === 'display');
}

function iblItem() {
  return displayPanel()?.items.find(item => item.id === 'IBL');
}

test('the View tab display panel carries the IBL toggle with both i18n keys', () => {
  const panel = displayPanel();
  assert.ok(panel, 'the View tab must keep its display panel');
  const item = iblItem();
  assert.ok(item, 'the display panel must offer IBL');
  // The ribbon entry is a router command, never a shell action, and it reads the
  // host's own flag as its checked state.
  assert.equal(item.command, 'IBL');
  assert.equal(item.action, undefined);
  assert.equal(item.labelKey, 'ribbon.command.ibl');
  assert.equal(item.hintKey, 'ribbon.command.ibl.hint');
  assert.equal(item.activeKey, 'iblEnabled');
  // The icon set's lighting mark; a made-up glyph would render nothing.
  assert.equal(item.icon, 'sun');
  // It sits with the neighbouring display toggles, not in a new panel.
  assert.ok(panel.items.some(entry => entry.id === 'GRID'));
  assert.ok(panel.items.some(entry => entry.id === 'SNAP'));
  assert.ok(panel.items.some(entry => entry.id === 'ORTHO'));
});

test('IBL is a catalogued command-line affordance', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'IBL');
  assert.equal(rows.length, 1, 'exactly one catalog row');
  assert.deepEqual(
    { ...rows[0] },
    { id: 'IBL', labelKey: 'ribbon.command.ibl', icon: 'sun', group: 'view' },
  );
});

test('a bare IBL toggles through the one command funnel', async () => {
  assert.deepEqual(admitCommandText('IBL', { status: loaded }), {
    kind: 'admitted',
    command: 'IBL',
    reasonKey: null,
  });
  assert.equal(admitCommandText('ibl', { status: loaded }).command, 'IBL');
  const calls = [];
  const result = await queueCadCommand(
    async (...args) => { calls.push(args); return { accepted: true }; },
    ' ibl ',
    loaded,
    false,
  );
  assert.deepEqual(result, { kind: 'accepted', command: 'IBL' });
  assert.deepEqual(calls, [['cad_command', { command: 'IBL' }]]);
});

test('IBL accepts ON|OFF|1|0 case-insensitively and sends the explicit line', async () => {
  for (const [typed, expected] of [
    ['IBL ON', 'IBL ON'],
    ['ibl on', 'IBL ON'],
    ['IBL OFF', 'IBL OFF'],
    ['ibl off', 'IBL OFF'],
    ['IBL 1', 'IBL 1'],
    ['IBL 0', 'IBL 0'],
  ]) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.command, expected, typed);
    const calls = [];
    const result = await queueCadCommand(
      async (...args) => { calls.push(args); return { accepted: true }; },
      typed,
      loaded,
      false,
    );
    assert.deepEqual(result, { kind: 'accepted', command: expected }, typed);
    assert.deepEqual(calls, [['cad_command', { command: expected }]], typed);
  }
  // The shared predicate is the one grammar; a word outside it never reaches IPC.
  for (const word of ['ON', ' off ', '1', '0']) {
    assert.equal(ARGUMENT_COMMANDS.IBL.parse(word), true, word);
  }
  for (const word of ['', 'yes', 'true', '2', 'ON OFF']) {
    assert.equal(ARGUMENT_COMMANDS.IBL.parse(word), false, word);
  }
  assert.equal(admitCommandText('IBL MAYBE', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('IBL MAYBE', { status: loaded }).reasonKey, 'command.unsupported');
});

test('the pressed state follows the host flag, never a guess', () => {
  const item = iblItem();
  assert.equal(ribbonItemActiveKey(item), 'iblEnabled');
  assert.equal(ribbonItemActive({ iblEnabled: true }, item), true);
  assert.equal(ribbonItemActive({ iblEnabled: false }, item), false);
  // No status yet, or a host that omits the field, is off — not a checked guess.
  assert.equal(ribbonItemActive(null, item), false);
  assert.equal(ribbonItemActive(undefined, item), false);
  assert.equal(ribbonItemActive({}, item), false);
  // A one-shot command (no `activeKey`) is never a toggle.
  assert.equal(ribbonItemActiveKey({ id: 'FIT' }), null);
  assert.equal(ribbonItemActive({ iblEnabled: true }, { id: 'FIT' }), false);
});

test('iblEnabled is a required real boolean on the status', () => {
  assert.equal(parseCadStatus(loaded).iblEnabled, true);
  const off = { ...loaded, iblEnabled: false };
  assert.equal(parseCadStatus(off).iblEnabled, false);
  for (const iblEnabled of [undefined, null, 'true', 1]) {
    assert.throws(
      () => parseCadStatus({ ...loaded, iblEnabled }),
      /CAD_STATUS_INVALID/,
      `iblEnabled ${String(iblEnabled)}`,
    );
  }
});

test('the ribbon drives the pressed state generically and keeps About working', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  // The checked state is the model's helper reading `shell.status`, not a
  // per-item branch; the About dialog keeps its host-window rule.
  assert.ok(
    ribbon.includes('item.activeKey === undefined ? undefined : ribbonItemActive(shell.status, item)'),
    'the ribbon must read a toggle through the activeKey helper',
  );
  assert.ok(
    ribbon.includes("item.action === 'about' ? Boolean(shell.popupOpen)"),
    'About must keep reading the host popup as its only open signal',
  );
  assert.ok(ribbon.includes('active={active} pressed={pressed}'));
  // A toggle always reports `aria-pressed`; a one-shot button never does.
  const primitives = await readModule('components/primitives.tsx');
  assert.ok(primitives.includes("aria-pressed={toggle ? pressed : undefined}"));
  assert.ok(primitives.includes("active || (toggle && pressed) ? 'is-active' : ''"));
});
