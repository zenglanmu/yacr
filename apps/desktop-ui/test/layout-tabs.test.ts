// Contract tests for the drawing-space strip: the `cad_status.layouts` /
// `activeSpace` wire projection, the exact space-switch commands (`MODEL` /
// `SPACE #<id>`), and the strict rejection of malformed additive payloads.
//
// These are model tests, not screenshots: the tab list comes from
// `layout/layout-tabs.ts`, and the component source is checked for wiring the
// same projection to the one command funnel.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import {
  layoutReasonCode, layoutSpaceCommand, layoutTabsView, layoutUnsupportedTitle, MODEL_SPACE_COMMAND,
} from '../src/layout/layout-tabs.ts';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { HOST_NAMES } from './helpers.ts';
import { readModule } from './helpers.ts';

/** The required fields `parseCadStatus` enforces (same shape as pointer.test.ts). */
const base = {
  documentLabel: null, entityCount: null, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false,
  snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false,
  backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

const layout = (overrides = {}) => ({
  id: 3,
  name: 'Sheet A',
  supported: true,
  reason: null,
  viewportCount: 2,
  ...overrides,
});

const status = (overrides = {}) => ({ ...base, ...overrides });

test('an older host with no layout blocks shows model space alone', () => {
  const tabs = layoutTabsView(status());
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0].id, null);
  assert.equal(tabs[0].active, true);
  assert.equal(tabs[0].command, null);
  // The parser still accepts the payload: omission is the documented default.
  const older = status();
  assert.equal(parseCadStatus(older), older);
});

test('zero layouts still renders the active model tab', () => {
  const tabs = layoutTabsView(status({ layouts: [], activeSpace: { kind: 'model' } }));
  assert.deepEqual(tabs.map(tab => tab.id), [null]);
  assert.equal(tabs[0].active, true);
});

test('one layout adds one paper tab in drawing order', () => {
  const tabs = layoutTabsView(status({ layouts: [layout()], activeSpace: { kind: 'model' } }));
  assert.deepEqual(tabs.map(tab => tab.id), [null, 3]);
  assert.equal(tabs[1].name, 'Sheet A');
  assert.equal(tabs[1].viewportCount, 2);
  assert.equal(tabs[1].supported, true);
  assert.equal(tabs[1].active, false);
  assert.equal(tabs[1].command, 'SPACE #3');
});

test('many layouts keep the host order and each keeps its own command', () => {
  const rows = [
    layout({ id: 3, name: 'Sheet A' }),
    layout({ id: 5, name: 'Sheet B', viewportCount: 0 }),
    layout({ id: 8, name: 'Sheet C', viewportCount: 4 }),
  ];
  const tabs = layoutTabsView(status({ layouts: rows, activeSpace: { kind: 'model' } }));
  assert.deepEqual(tabs.map(tab => tab.id), [null, 3, 5, 8]);
  assert.deepEqual(tabs.slice(1).map(tab => tab.command), ['SPACE #3', 'SPACE #5', 'SPACE #8']);
});

test('activeSpace drives the active tab, not a local click state', () => {
  const rows = [layout({ id: 3 }), layout({ id: 5, name: 'Sheet B' })];
  const onPaper = layoutTabsView(status({ layouts: rows, activeSpace: { kind: 'paper', id: 5 } }));
  assert.equal(onPaper[0].active, false);
  assert.equal(onPaper[0].command, MODEL_SPACE_COMMAND);
  assert.equal(onPaper[1].active, false);
  assert.equal(onPaper[2].active, true);
  assert.equal(onPaper[2].command, null, 'the active tab dispatches nothing');
  // Back to model: model is active and paper tabs are live again.
  const onModel = layoutTabsView(status({ layouts: rows, activeSpace: { kind: 'model' } }));
  assert.equal(onModel[0].active, true);
  assert.equal(onModel[0].command, null);
  assert.equal(onModel[1].command, 'SPACE #3');
});

test('an unsupported layout stays visible, disabled and carries only the stable code', () => {
  const tabs = layoutTabsView(status({
    layouts: [layout({ id: 4, supported: false, reason: 'viewport.twisted_transform: view is twisted' })],
    activeSpace: { kind: 'model' },
  }));
  const tab = tabs[1];
  assert.equal(tab.supported, false);
  assert.equal(tab.reasonCode, 'viewport.twisted_transform');
  assert.equal(tab.command, null, 'an unsupported tab never dispatches a switch');
  const t = key => key;
  assert.equal(layoutUnsupportedTitle(t, tab), 'layout.tab.unsupported_title', 'a code always uses the shaped sentence');
  // With a translator that honours placeholders, both the marker and the code
  // are present and the host's message is not.
  const shaped = (key, params) => (key === 'layout.tab.unsupported_title'
    ? `${params.marker}: ${params.reason}` : key);
  assert.equal(layoutUnsupportedTitle(shaped, tab), 'layout.unsupported_marker: viewport.twisted_transform');
  // A reason without a separator is its own code; an empty one is not a reason.
  assert.equal(layoutReasonCode('layout.missing'), 'layout.missing');
  assert.equal(layoutReasonCode(''), null);
  assert.equal(layoutReasonCode(null), null);
});

test('the tab click emits exactly one space-switch line through the shared funnel', async () => {
  assert.equal(layoutSpaceCommand(0), 'SPACE #0');
  assert.equal(layoutSpaceCommand(12), 'SPACE #12');
  assert.equal(MODEL_SPACE_COMMAND, 'MODEL');

  // The admitted line is the same text the component dispatches. Admission is
  // still the chrome's grammar; the host remains the authority for the id.
  setCommandCatalog(HOST_NAMES);
  const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
  const admitted = admitCommandText('SPACE #12', { status: loaded });
  assert.equal(admitted.kind, 'admitted');
  assert.equal(admitted.command, 'SPACE #12');
  assert.equal(admitCommandText('SPACE MODEL', { status: loaded }).kind, 'admitted');
  // The Model tab now dispatches the official payload-free `MODEL` selector
  // (AutoCAD 2026); the `SPACE MODEL` argument form above stays admitted.
  assert.equal(admitCommandText('MODEL', { status: loaded }).kind, 'admitted');
  // A display name is real host grammar (`SPACE Sheet One`), so the local
  // argument table must not reject it; the host names an unknown layout.
  assert.equal(admitCommandText('SPACE Sheet One', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('SPACE #nope', { status: loaded }).kind, 'admitted');

  const component = await readModule('components/LayoutTabs.tsx');
  assert.ok(component.includes('shell.runCommand(command)'), 'the click uses the shared command path');
  assert.ok(component.includes('const tabs = layoutTabsView(shell.status);'));
  assert.equal(component.includes("invoke('cad_command'"), false, 'the component does not hand-roll IPC');
});

test('malformed layout and activeSpace blocks are rejected, never coerced', () => {
  const bad = [
    { layouts: {} },
    { layouts: [{}] },
    { layouts: [layout({ id: -1 })] },
    { layouts: [layout({ id: 1.5 })] },
    { layouts: [layout({ name: 7 })] },
    { layouts: [layout({ supported: 'true' })] },
    { layouts: [layout({ reason: 3 })] },
    { layouts: [layout({ viewportCount: -2 })] },
    { activeSpace: null },
    { activeSpace: { kind: 'paper' } },
    { activeSpace: { kind: 'paper', id: '3' } },
    { activeSpace: { kind: 'block' } },
    { references: 'nope' },
  ];
  for (const block of bad) {
    assert.throws(() => parseCadStatus(status(block)), /CAD_STATUS_INVALID/, JSON.stringify(block));
  }
  // Well-formed blocks are accepted as-is (no normalisation).
  const good = status({
    layouts: [layout({ id: 0, name: '' })],
    activeSpace: { kind: 'paper', id: 0 },
  });
  assert.equal(parseCadStatus(good), good);
});

test('the desktop strip is horizontally scrollable and the active tab is clear', async () => {
  const { readCss } = await import('./helpers.ts');
  const css = await readCss();
  const list = css.match(/\.layouttabs__list\s*\{[^}]*\}/);
  assert.ok(list, 'the tab list keeps its own rule');
  assert.ok(/overflow-x:\s*auto/.test(list[0]), 'many layouts scroll instead of vanishing');
  assert.ok(/min-width:\s*0/.test(list[0]), 'the flex child may shrink below its content');
  assert.ok(/\.layout-tab\.is-active\s*\{[^}]*box-shadow:[^}]*var\(--accent\)/.test(css), 'the active tab is visually distinct');
});
