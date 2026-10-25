// Status-bar contract: the idle hover-preview chip renders the host's real
// `status.hoverPreview` flag, dispatches the single HOVERPREVIEW command, and
// renders nothing when an older payload omits the field (never a guessed value).
import test from 'node:test';
import assert from 'node:assert/strict';
import { dispatchGrid, dispatchGridSnap, dispatchHoverPreview, dispatchObjectSnap, dispatchOrtho, draftingAcceleratorCommand, GRID_COMMAND, gridSnapToggle, gridToggle, GRID_SNAP_COMMAND, HOVER_PREVIEW_COMMAND, hoverPreviewToggle, OBJECT_SNAP_COMMAND, objectSnapToggle, ORTHO_COMMAND, orthoToggle, snapKindLabelKey } from '../src/cad.mjs';
import { readAppSource, readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('hover preview renders the real status flag and never fabricates one', () => {
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: true }, true), { on: true, disabled: false });
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: false }, true), { on: false, disabled: false });
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: true }, false), { on: true, disabled: true });
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: false }, false), { on: false, disabled: true });
  // No field (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { hoverPreview: null }, { hoverPreview: 'true' }, { hoverPreview: 1 }]) {
    assert.equal(hoverPreviewToggle(status, true), null);
  }
});

test('the hover-preview toggle dispatches the single HOVERPREVIEW command', () => {
  const calls = [];
  dispatchHoverPreview(command => calls.push(command));
  assert.deepEqual(calls, ['HOVERPREVIEW']);
  assert.equal(HOVER_PREVIEW_COMMAND, 'HOVERPREVIEW');
});

test('the status bar reads the chip state from status and uses normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("hoverPreviewToggle(shell.status, shell.canCommand('HOVERPREVIEW'))"));
  assert.ok(source.includes('on={hoverPreview.on}'));
  assert.ok(source.includes('disabled={hoverPreview.disabled}'));
  assert.ok(source.includes('dispatchHoverPreview(shell.runCommand)'));
  // The title explains the real on/off behaviour from the catalogs.
  assert.ok(source.includes("t(hoverPreview.on ? 'status.toggle.hover_preview_on' : 'status.toggle.hover_preview_off')"));
  assert.ok(source.includes("label={t('status.toggle.hover_preview')}"));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{hoverPreview ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('useState'));
  assert.ok(!source.includes('setHoverPreview'));
  // There is no disabled drafting placeholder left: every aid this build has is a
  // real projected toggle (the unsupported chip helper is gone from the bar).
  assert.ok(!source.includes('status.toggle_unsupported'));
  assert.ok(!source.includes('const DRAFTING'));
});

test('the hover-preview catalog keys exist in both locales with real copy', () => {
  for (const key of ['status.toggle.hover_preview', 'status.toggle.hover_preview_on', 'status.toggle.hover_preview_off']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
  }
  assert.match(zh['status.toggle.hover_preview_on'], /悬停/);
  assert.match(en['status.toggle.hover_preview_on'], /hover/i);
});

test('object snap renders the real status flag and never fabricates one', () => {
  assert.deepEqual(objectSnapToggle({ snapEnabled: true }, true), { on: true, disabled: false });
  assert.deepEqual(objectSnapToggle({ snapEnabled: false }, true), { on: false, disabled: false });
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(objectSnapToggle({ snapEnabled: true }, false), { on: true, disabled: true });
  assert.deepEqual(objectSnapToggle({ snapEnabled: false }, false), { on: false, disabled: true });
  // No field (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { snapEnabled: null }, { snapEnabled: 'true' }, { snapEnabled: 1 }]) {
    assert.equal(objectSnapToggle(status, true), null);
  }
});

test('the object-snap toggle dispatches the single OSNAP command', () => {
  const calls = [];
  dispatchObjectSnap(command => calls.push(command));
  assert.deepEqual(calls, ['OSNAP']);
  assert.equal(OBJECT_SNAP_COMMAND, 'OSNAP');
});

test('the status bar wires OSNAP from status with normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("objectSnapToggle(shell.status, shell.canCommand('OSNAP'))"));
  assert.ok(source.includes('on={objectSnap.on}'));
  assert.ok(source.includes('disabled={objectSnap.disabled}'));
  assert.ok(source.includes('dispatchObjectSnap(shell.runCommand)'));
  assert.ok(source.includes("label={t('status.toggle.osnap')}"));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{objectSnap ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('setObjectSnap'));
  // SNAP, GRID and ORTHO are all real toggles now; the chip vocabulary is shared.
  assert.ok(source.includes("t('status.toggle.grid')"));
  assert.ok(source.includes("t('status.toggle.ortho')"));
  assert.ok(source.includes("t('status.toggle.snap')"));
});

test('the OSNAP catalog key exists in both locales with real copy', () => {
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['status.toggle.osnap'], 'string');
    assert.ok(locale['status.toggle.osnap'].trim().length > 0);
  }
});

test('F3/F7/F8/F9 dispatch their drafting toggles through the command funnel with preventDefault', async () => {
  const source = await readAppSource();
  // One mapping table, one call site: the accelerators cannot drift from the
  // chips because both resolve through `draftingAcceleratorCommand`.
  assert.ok(
    source.includes('const draftingCommand = draftingAcceleratorCommand(event.key);'),
    'the drafting keys must resolve through the shared mapping table',
  );
  assert.ok(
    source.includes('if (draftingCommand) { event.preventDefault(); void submitCommand(draftingCommand); return; }'),
    'the resolved command must run through the same submit path as every other affordance',
  );
  assert.equal(draftingAcceleratorCommand('F3'), 'OSNAP');
  assert.equal(draftingAcceleratorCommand('F7'), 'GRID');
  assert.equal(draftingAcceleratorCommand('F8'), 'ORTHO');
  assert.equal(draftingAcceleratorCommand('F9'), 'SNAP');
});

test('the display grid renders the real status flag and never fabricates one', () => {
  assert.deepEqual(gridToggle({ gridEnabled: true }, true), { on: true, disabled: false });
  assert.deepEqual(gridToggle({ gridEnabled: false }, true), { on: false, disabled: false });
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(gridToggle({ gridEnabled: true }, false), { on: true, disabled: true });
  assert.deepEqual(gridToggle({ gridEnabled: false }, false), { on: false, disabled: true });
  // No field (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { gridEnabled: null }, { gridEnabled: 'true' }, { gridEnabled: 1 }]) {
    assert.equal(gridToggle(status, true), null);
  }
});

test('the display-grid toggle dispatches the single GRID command', () => {
  const calls = [];
  dispatchGrid(command => calls.push(command));
  assert.deepEqual(calls, ['GRID']);
  assert.equal(GRID_COMMAND, 'GRID');
});

test('the status bar wires GRID from status with normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("gridToggle(shell.status, shell.canCommand('GRID'))"));
  assert.ok(source.includes('on={grid.on}'));
  assert.ok(source.includes('disabled={grid.disabled}'));
  assert.ok(source.includes('dispatchGrid(shell.runCommand)'));
  assert.ok(source.includes("label={t('status.toggle.grid')}"));
  // The chip advertises AutoCAD's own key for the display grid.
  assert.ok(source.includes('(F7)'));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{grid ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('setGridEnabled'));
});

test('the display-grid catalog key exists in both locales with real copy', () => {
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['status.toggle.grid'], 'string');
    assert.ok(locale['status.toggle.grid'].trim().length > 0);
  }
  assert.match(zh['status.toggle.grid'], /栅格/);
  assert.equal(en['status.toggle.grid'], 'Grid');
});

test('F7 is the display-grid accelerator, resolved by the same one mapping table as F3', () => {
  // One table, not a second key handler: the shell's single `keydown` funnel asks
  // it which router command a drafting-aid function key means.
  assert.equal(draftingAcceleratorCommand('F7'), 'GRID');
  assert.equal(draftingAcceleratorCommand('F3'), 'OSNAP');
  // Every other key (and a non-string) resolves to nothing: no hijack.
  for (const key of ['F1', 'F2', 'F5', 'F12', 'a', 'Enter', '', null, undefined, 7]) {
    assert.equal(draftingAcceleratorCommand(key), null, String(key));
  }
  // F7 and F3 dispatch the *same* toggle their chips dispatch.
  assert.equal(draftingAcceleratorCommand('F7'), GRID_COMMAND);
  assert.equal(draftingAcceleratorCommand('F3'), OBJECT_SNAP_COMMAND);
});

test('ORTHO renders the real status flag and never fabricates one', () => {
  assert.deepEqual(orthoToggle({ orthoEnabled: true }, true), { on: true, disabled: false });
  assert.deepEqual(orthoToggle({ orthoEnabled: false }, true), { on: false, disabled: false });
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(orthoToggle({ orthoEnabled: true }, false), { on: true, disabled: true });
  assert.deepEqual(orthoToggle({ orthoEnabled: false }, false), { on: false, disabled: true });
  // No field (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { orthoEnabled: null }, { orthoEnabled: 'true' }, { orthoEnabled: 1 }]) {
    assert.equal(orthoToggle(status, true), null);
  }
});

test('the ORTHO toggle dispatches the single ORTHO command', () => {
  const calls = [];
  dispatchOrtho(command => calls.push(command));
  assert.deepEqual(calls, ['ORTHO']);
  assert.equal(ORTHO_COMMAND, 'ORTHO');
});

test('the status bar wires ORTHO from status with normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("orthoToggle(shell.status, shell.canCommand('ORTHO'))"));
  assert.ok(source.includes('on={ortho.on}'));
  assert.ok(source.includes('disabled={ortho.disabled}'));
  assert.ok(source.includes('dispatchOrtho(shell.runCommand)'));
  assert.ok(source.includes("label={t('status.toggle.ortho')}"));
  // The chip advertises AutoCAD's own key for the orthographic constraint.
  assert.ok(source.includes('(F8)'));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{ortho ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('setOrthoEnabled'));
  // SNAP is a real toggle too now (grid snap, F9), not a disabled placeholder.
  assert.ok(source.includes('gridSnapToggle(shell.status'));
});

test('the ORTHO catalog key exists in both locales with real copy', () => {
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['status.toggle.ortho'], 'string');
    assert.ok(locale['status.toggle.ortho'].trim().length > 0);
  }
  assert.match(zh['status.toggle.ortho'], /正交/);
  assert.equal(en['status.toggle.ortho'], 'Ortho');
});

test('F8 is the ORTHO accelerator, resolved by the same one mapping table as F3/F7', () => {
  assert.equal(draftingAcceleratorCommand('F8'), 'ORTHO');
  // It dispatches the *same* toggle the chip dispatches.
  assert.equal(draftingAcceleratorCommand('F8'), ORTHO_COMMAND);
  // F9 is the grid-snap key, a different aid from ORTHO (F8) and OSNAP (F3).
  assert.equal(draftingAcceleratorCommand('F9'), GRID_SNAP_COMMAND);
  assert.notEqual(GRID_SNAP_COMMAND, ORTHO_COMMAND);
  assert.notEqual(GRID_SNAP_COMMAND, OBJECT_SNAP_COMMAND);
});

test('the current snap candidate kind maps to real catalog labels and never guesses', () => {
  // Every real `cad_measure::SnapKind` machine key maps to its own label key.
  const kinds = {
    endpoint: 'snap.kind.endpoint',
    midpoint: 'snap.kind.midpoint',
    center: 'snap.kind.center',
    quadrant: 'snap.kind.quadrant',
    perpendicular: 'snap.kind.perpendicular',
    intersection: 'snap.kind.intersection',
  };
  for (const [kind, key] of Object.entries(kinds)) {
    assert.equal(snapKindLabelKey({ snapKind: kind }), key);
  }
  // No candidate (null), an older host (absent field), a non-string or an
  // unknown kind renders nothing rather than a guessed label.
  for (const status of [null, undefined, {}, { snapKind: null }, { snapKind: 1 }, { snapKind: 'future_osnap' }]) {
    assert.equal(snapKindLabelKey(status), null);
  }
});

test('the status bar shows the current snap kind next to OSNAP and nothing without one', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes('snapKindLabelKey(shell.status)'));
  // Rendered only when a candidate kind is present: no placeholder chip.
  assert.ok(source.includes('{snapKindKey ? ('));
  // The label is localized through the catalog, never a hardcoded string.
  assert.ok(source.includes('{t(snapKindKey)}'));
});

test('the snap-kind catalog keys exist in both locales with real copy', () => {
  const keys = [
    'snap.kind.endpoint',
    'snap.kind.midpoint',
    'snap.kind.center',
    'snap.kind.quadrant',
    'snap.kind.perpendicular',
    'snap.kind.intersection',
  ];
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  assert.match(zh['snap.kind.endpoint'], /端点/);
  assert.match(en['snap.kind.endpoint'], /endpoint/i);
});

test('grid snap renders the real status flag and its spacing, and never fabricates one', () => {
  assert.deepEqual(
    gridSnapToggle({ snapGridEnabled: true, snapSpacing: 10 }, true),
    { on: true, spacing: 10, disabled: false },
  );
  assert.deepEqual(
    gridSnapToggle({ snapGridEnabled: false, snapSpacing: 2.5 }, true),
    { on: false, spacing: 2.5, disabled: false },
  );
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(
    gridSnapToggle({ snapGridEnabled: true, snapSpacing: 10 }, false),
    { on: true, spacing: 10, disabled: true },
  );
  // No flag (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { snapGridEnabled: null }, { snapGridEnabled: 'true' }, { snapGridEnabled: 1 }]) {
    assert.equal(gridSnapToggle(status, true), null);
  }
  // A missing or unusable spacing keeps the toggle but reports no spacing: the
  // chip never advertises a number the host did not send.
  for (const snapSpacing of [undefined, null, 0, -5, '10', Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.deepEqual(
      gridSnapToggle({ snapGridEnabled: true, snapSpacing }, true),
      { on: true, spacing: null, disabled: false },
      String(snapSpacing),
    );
  }
});

test('the grid-snap toggle dispatches the single SNAP command', () => {
  const calls = [];
  dispatchGridSnap(command => calls.push(command));
  assert.deepEqual(calls, ['SNAP']);
  assert.equal(GRID_SNAP_COMMAND, 'SNAP');
  // It is a different selector from object snap and from the display grid.
  assert.notEqual(GRID_SNAP_COMMAND, OBJECT_SNAP_COMMAND);
  assert.notEqual(GRID_SNAP_COMMAND, GRID_COMMAND);
});

test('the status bar wires SNAP from status with normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("gridSnapToggle(shell.status, shell.canCommand('SNAP'))"));
  assert.ok(source.includes('on={snapGrid.on}'));
  assert.ok(source.includes('disabled={snapGrid.disabled}'));
  assert.ok(source.includes('dispatchGridSnap(shell.runCommand)'));
  assert.ok(source.includes("label={t('status.toggle.snap')}"));
  // The chip advertises AutoCAD's own key for grid snap and reports the spacing.
  assert.ok(source.includes('(F9)'));
  assert.ok(source.includes('snapGrid.spacing'));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{snapGrid ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('setSnapGridEnabled'));
});

test('the grid-snap catalog key exists in both locales with real copy', () => {
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['status.toggle.snap'], 'string');
    assert.ok(locale['status.toggle.snap'].trim().length > 0);
  }
  assert.match(zh['status.toggle.snap'], /捕捉/);
  assert.equal(en['status.toggle.snap'], 'Snap');
});

test('F9 is the grid-snap accelerator, a distinct switch from F3/F7/F8', () => {
  assert.equal(draftingAcceleratorCommand('F9'), 'SNAP');
  assert.equal(draftingAcceleratorCommand('F9'), GRID_SNAP_COMMAND);
  assert.notEqual(draftingAcceleratorCommand('F9'), draftingAcceleratorCommand('F3'));
  assert.notEqual(draftingAcceleratorCommand('F9'), draftingAcceleratorCommand('F7'));
  assert.notEqual(draftingAcceleratorCommand('F9'), draftingAcceleratorCommand('F8'));
});
