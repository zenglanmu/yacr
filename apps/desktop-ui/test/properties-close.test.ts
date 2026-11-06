// Contract tests for the chrome-local `PROPERTIESCLOSE` selector (AutoCAD 2026).
//
// The typed word closes the properties window the palette's own close button
// closes. It is a chrome action like `ABOUT`/`CLEANSCREENON`/`CLEANSCREENOFF`: it
// sends nothing to the router, needs no drawing and is never gated by layout —
// closing a window that is already closed is an idempotent success, not a
// refusal. These are pure-model and source assertions in the style of the other
// selector suites (no React DOM rendering).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  PROPERTIESCLOSE_COMMAND,
  isPropertiesCloseCommand,
} from '../src/commands/chrome-commands.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { admitCommandText, isSupportedCommand } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the selector is the bare PROPERTIESCLOSE word, case- and whitespace-insensitive', () => {
  assert.equal(PROPERTIESCLOSE_COMMAND, 'PROPERTIESCLOSE');
  // Append-only table: the new row is appended after the clean-screen pair and
  // the later `LAYERCLOSE` row; the name list is derived from the one table.
  assert.deepEqual(
    [...CHROME_COMMAND_NAMES],
    ['ABOUT', 'CLEANSCREENON', 'CLEANSCREENOFF', 'PROPERTIESCLOSE', 'LAYERCLOSE'],
  );
  assert.equal(CHROME_COMMANDS.length, 5);
  assert.equal(CHROME_COMMANDS[3].name, 'PROPERTIESCLOSE');
  for (const text of ['PROPERTIESCLOSE', 'propertiesclose', '  PropertiesClose ', 'pRoPeRtIeScLoSe']) {
    assert.equal(isPropertiesCloseCommand(text), true, text);
  }
  // Exact match only: the `PROPERTIES` word, an argument or a longer word is a
  // different line. Closing is never a prefix match of opening.
  for (const text of [
    'PROPERTIES', 'PROPERTIESCLOSE 1', 'PROPERTIESCLOSE OFF', 'PROPERTIESCLOSED',
    '', '   ', null, undefined,
  ]) {
    assert.equal(isPropertiesCloseCommand(text), false, String(text));
  }
  assert.equal(isPropertiesCloseCommand('PROPERTIES'), false, 'the open word never matches close');
});

test('PROPERTIESCLOSE resolves to propertiesClose on every layout', () => {
  assert.deepEqual(localSelectorFor('PROPERTIESCLOSE', 'desktop'), { kind: 'propertiesClose' });
  assert.deepEqual(localSelectorFor('propertiesclose', 'desktop'), { kind: 'propertiesClose' });
  assert.deepEqual(localSelectorFor('  PropertiesClose ', 'desktop'), { kind: 'propertiesClose' });
  // Closing is chrome state, not a docked palette: the compact layout never
  // refuses it (unlike `PROPERTIES`/`LAYER`, which cannot open there).
  assert.deepEqual(localSelectorFor('PROPERTIESCLOSE', 'mobile'), { kind: 'propertiesClose' });
  assert.deepEqual(localSelectorFor('propertiesclose', 'mobile'), { kind: 'propertiesClose' });
  // The open word still resolves to its own kind and is not shadowed.
  assert.deepEqual(localSelectorFor('PROPERTIES', 'desktop'), { kind: 'properties' });
  // Near misses stay router lines.
  assert.equal(localSelectorFor('PROPERTIESCLOSE 1', 'desktop'), null);
  assert.equal(localSelectorFor('PROPERTIESCLOSED', 'desktop'), null);
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('PROPERTIESCLOSE is admitted by the chrome, with no host vocabulary and no document', () => {
  assert.equal(isSupportedCommand('PROPERTIESCLOSE'), true);
  assert.equal(isSupportedCommand('propertiesclose'), true);
  assert.equal(LOCAL_CHROME_COMMANDS.includes('PROPERTIESCLOSE'), true);
  assert.equal(admitCommandText('PROPERTIESCLOSE', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('propertiesclose', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('PROPERTIESCLOSE', { status: null, pending: true }).kind, 'admitted');
  assert.equal(
    admitCommandText('PROPERTIESCLOSE', { status: { documentLabel: null, loading: false } }).kind,
    'admitted',
    'the start page can still close the properties window',
  );
  // An argument on a selector that takes none is an explicit refusal, never a
  // locally parsed value.
  assert.equal(admitCommandText('PROPERTIESCLOSE 1', { status: null }).kind, 'unsupported');
});

test('the catalog exposes PROPERTIESCLOSE exactly once, appended, with properties.close', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'PROPERTIESCLOSE');
  assert.equal(rows.length, 1);
  assert.deepEqual(
    { ...rows[0] },
    { id: 'PROPERTIESCLOSE', labelKey: 'properties.close', icon: 'close', group: 'annotate' },
  );
  // Append-only: the new row is still the last one, so no shipped row moved.
  // (`LAYERCLOSE`, added after it, is the only newer row.)
  assert.equal(COMMAND_CATALOG[COMMAND_CATALOG.length - 2].id, 'PROPERTIESCLOSE');
  assert.equal(COMMAND_CATALOG[COMMAND_CATALOG.length - 3].id, 'CLEANSCREENOFF');
});

test('both properties palette sources close through the one command', async () => {
  const palette = await readModule('components/PropertiesPalette.tsx');
  const dock = await readModule('components/PropertiesDock.tsx');
  for (const [name, source] of [
    ['components/PropertiesPalette.tsx', palette],
    ['components/PropertiesDock.tsx', dock],
  ]) {
    assert.ok(
      source.includes("shell.runCommand('PROPERTIESCLOSE')"),
      `${name} dispatches the close command`,
    );
    assert.equal(
      source.includes('setPropertiesOpen(false)'),
      false,
      `${name} must not close the window behind the one command`,
    );
  }
  // The palette keeps its existing icon and localized label on the same control.
  assert.ok(palette.includes('icon="close"'), 'the close control keeps its icon');
  assert.ok(palette.includes("label={t('properties.close')}"), 'the close control keeps its label');
});

test('the local selector host dispatches closeProperties and App supplies it', async () => {
  const selectors = await readModule('commands/local-selectors.ts');
  assert.ok(
    selectors.includes('readonly closeProperties: () => void;'),
    'the host declares the close callback',
  );
  assert.ok(
    selectors.includes("case 'propertiesClose': closeProperties(); break;"),
    'the propertiesClose result dispatches the close callback',
  );
  assert.ok(
    selectors.includes("if (isPropertiesCloseCommand(text)) return { kind: 'propertiesClose' };"),
    'the selector resolves after the clean-screen selectors',
  );
  const app = await readModule('App.tsx');
  const hostStart = app.indexOf('useLocalSelectors({');
  assert.ok(hostStart > 0, 'the local-selector host must exist');
  const hostBlock = app.slice(hostStart, app.indexOf('});', hostStart));
  assert.ok(
    hostBlock.includes('closeProperties: () => setPropertiesOpen(false),'),
    'App closes the one properties state through the host',
  );
});

test('properties.close is declared in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['properties.close'], 'string');
  assert.equal(typeof en['properties.close'], 'string');
  assert.ok(zh['properties.close'].trim().length > 0);
  assert.ok(en['properties.close'].trim().length > 0);
});
