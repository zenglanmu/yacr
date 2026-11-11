// Contract tests for the chrome-local `LAYERCLOSE` selector (AutoCAD 2026).
//
// The typed word closes the layers palette the palette's own close control
// closes. It is a chrome action like `PROPERTIESCLOSE`/`ABOUT`: it sends nothing
// to the router, needs no drawing and is never gated by layout — closing a
// palette that is already closed is an idempotent success, not a refusal. These
// are pure-model and source assertions in the style of the other selector suites
// (no React DOM rendering).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  LAYERCLOSE_COMMAND,
  isLayerCloseCommand,
} from '../src/commands/chrome-commands.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { admitCommandText, isSupportedCommand } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the selector is the bare LAYERCLOSE word, case- and whitespace-insensitive', () => {
  assert.equal(LAYERCLOSE_COMMAND, 'LAYERCLOSE');
  // Append-only table: the row lands after `PROPERTIESCLOSE`; the name list is
  // derived from the one table.
  assert.deepEqual(
    [...CHROME_COMMAND_NAMES],
    ['ABOUT', 'CLEANSCREENON', 'CLEANSCREENOFF', 'PROPERTIESCLOSE', 'LAYERCLOSE', 'DSETTINGS', 'QUICKPROPERTIES', 'CLASSICLAYER', 'COMMANDLINE', 'COMMANDLINEHIDE', 'RIBBON', 'RIBBONCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'FILETAB', 'FILETABCLOSE', 'QVDRAWING', 'QVDRAWINGCLOSE', 'QVLAYOUT', 'QVLAYOUTCLOSE', 'QUICKCALC', 'WORKSPACE', 'WSSAVE', 'WSSETTINGS', 'QCCLOSE', 'UPDATETHUMBSNOW', 'TOOLBAR', '-TOOLBAR'],
  );
  assert.equal(CHROME_COMMANDS.length, 28);
  assert.equal(CHROME_COMMANDS[4].name, 'LAYERCLOSE');
  for (const text of ['LAYERCLOSE', 'layerclose', '  LayerClose ', 'lAyErClOsE']) {
    assert.equal(isLayerCloseCommand(text), true, text);
  }
  // Exact match only: the `LAYER` word, an argument or a longer word is a
  // different line. Closing is never a prefix match of opening.
  for (const text of [
    'LAYER', 'LAYERCLOSE 1', 'LAYERCLOSE OFF', 'LAYERCLOSED', 'LAYERS',
    '', '   ', null, undefined,
  ]) {
    assert.equal(isLayerCloseCommand(text), false, String(text));
  }
  assert.equal(isLayerCloseCommand('LAYER'), false, 'the open word never matches close');
});

test('LAYERCLOSE resolves to layersClose on every layout', () => {
  assert.deepEqual(localSelectorFor('LAYERCLOSE', 'desktop'), { kind: 'layersClose' });
  assert.deepEqual(localSelectorFor('layerclose', 'desktop'), { kind: 'layersClose' });
  assert.deepEqual(localSelectorFor('  LayerClose ', 'desktop'), { kind: 'layersClose' });
  // Closing is chrome state, not a docked palette: the compact layout never
  // refuses it (unlike `LAYER`, which cannot open there).
  assert.deepEqual(localSelectorFor('LAYERCLOSE', 'mobile'), { kind: 'layersClose' });
  assert.deepEqual(localSelectorFor('layerclose', 'mobile'), { kind: 'layersClose' });
  // The open word still resolves to its own kind (and its layout refusal where
  // the palette cannot open); close never shadows it.
  assert.deepEqual(localSelectorFor('LAYER', 'desktop'), { kind: 'layers' });
  // Near misses stay router lines.
  assert.equal(localSelectorFor('LAYERCLOSE 1', 'desktop'), null);
  assert.equal(localSelectorFor('LAYERCLOSED', 'desktop'), null);
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('LAYERCLOSE is admitted by the chrome, with no host vocabulary and no document', () => {
  assert.equal(isSupportedCommand('LAYERCLOSE'), true);
  assert.equal(isSupportedCommand('layerclose'), true);
  assert.equal(LOCAL_CHROME_COMMANDS.includes('LAYERCLOSE'), true);
  assert.equal(admitCommandText('LAYERCLOSE', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('layerclose', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('LAYERCLOSE', { status: null, pending: true }).kind, 'admitted');
  assert.equal(
    admitCommandText('LAYERCLOSE', { status: { documentLabel: null, loading: false } }).kind,
    'admitted',
    'the start page can still close the layers palette',
  );
  // An argument on a selector that takes none is an explicit refusal, never a
  // locally parsed value.
  assert.equal(admitCommandText('LAYERCLOSE 1', { status: null }).kind, 'unsupported');
});

test('the catalog exposes LAYERCLOSE exactly once, appended, with layers.close', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'LAYERCLOSE');
  assert.equal(rows.length, 1);
  assert.deepEqual(
    { ...rows[0] },
    { id: 'LAYERCLOSE', labelKey: 'layers.close', icon: 'close', group: 'annotate' },
  );
  // Append-only: LAYERCLOSE keeps its place directly after PROPERTIESCLOSE and
  // after the last shipped pre-2026-11-07 row (`PDSIZE`); later router rows may
  // follow the run.
  const order = COMMAND_CATALOG.map(command => command.id);
  const at = (id: string) => order.indexOf(id);
  assert.ok(at('PDSIZE') < at('ABOUT'), 'the chrome run is appended after shipped rows');
  assert.equal(at('PROPERTIESCLOSE') + 1, at('LAYERCLOSE'));
});

test('the layers palette closes through the one command', async () => {
  const palette = await readModule('components/LayersPalette.tsx');
  assert.ok(
    palette.includes("shell.runCommand('LAYERCLOSE')"),
    'the palette dispatches the close command',
  );
  assert.equal(
    palette.includes('setLayersOpen(false)'),
    false,
    'the palette must not close its window behind the one command',
  );
  // The palette keeps its existing icon and localized label on the same control.
  assert.ok(palette.includes('icon="close"'), 'the close control keeps its icon');
  assert.ok(palette.includes("label={t('layers.close')}"), 'the close control keeps its label');
});

test('the local selector host dispatches closeLayers and App supplies it', async () => {
  const selectors = await readModule('commands/local-selectors.ts');
  assert.ok(
    selectors.includes('readonly closeLayers: () => void;'),
    'the host declares the close callback',
  );
  assert.ok(
    selectors.includes("case 'layersClose': closeLayers(); break;"),
    'the layersClose result dispatches the close callback',
  );
  assert.ok(
    selectors.includes("if (isLayerCloseCommand(text)) return { kind: 'layersClose' };"),
    'the selector resolves after the properties close selector',
  );
  const app = await readModule('App.tsx');
  const hostStart = app.indexOf('useLocalSelectors({');
  assert.ok(hostStart > 0, 'the local-selector host must exist');
  const hostBlock = app.slice(hostStart, app.indexOf('});', hostStart));
  assert.ok(
    hostBlock.includes('closeLayers: () => setLayersOpen(false),'),
    'App closes the one layers state through the host',
  );
});

test('layers.close is declared in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['layers.close'], 'string');
  assert.equal(typeof en['layers.close'], 'string');
  assert.ok(zh['layers.close'].trim().length > 0);
  assert.ok(en['layers.close'].trim().length > 0);
});
