// Contract tests for the chrome-local `CLEANSCREENOFF` selector (AutoCAD 2026).
//
// The typed word **clears** the clean-screen state the File tab's `cleanScreen`
// toggle flips and `CLEANSCREENON` sets: one shared state, one setter
// (`setCleanScreen(value)`), never a second state and never a toggle. These are
// pure-model and source assertions in the style of the other selector suites (no
// React DOM rendering).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  CLEANSCREENOFF_COMMAND,
  isCleanScreenOffCommand,
} from '../src/commands/chrome-commands.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { admitCommandText, isSupportedCommand } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the selector is the bare CLEANSCREENOFF word, case- and whitespace-insensitive', () => {
  assert.equal(CLEANSCREENOFF_COMMAND, 'CLEANSCREENOFF');
  // Append-only table: the OFF row is appended after ABOUT and CLEANSCREENON, and
  // PROPERTIESCLOSE/LAYERCLOSE after it; the name list is derived from the table.
  assert.deepEqual(
    [...CHROME_COMMAND_NAMES],
    ['ABOUT', 'CLEANSCREENON', 'CLEANSCREENOFF', 'PROPERTIESCLOSE', 'LAYERCLOSE', 'DSETTINGS', 'QUICKPROPERTIES', 'CLASSICLAYER', 'COMMANDLINE', 'COMMANDLINEHIDE', 'RIBBON', 'RIBBONCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'PMTOGGLE', 'CLASSICXREF', 'CLASSICINSERT', 'CLASSICGROUP', 'CLASSICIMAGE', 'FILETAB', 'FILETABCLOSE', 'QVDRAWING', 'QVDRAWINGCLOSE', 'QVLAYOUT', 'QVLAYOUTCLOSE', 'QUICKCALC', 'WORKSPACE', 'WSSAVE', 'WSSETTINGS', 'QCCLOSE', 'UPDATETHUMBSNOW', 'TOOLBAR', '-TOOLBAR', 'TOOLPALETTES', 'TOOLPALETTESCLOSE', 'TPNAVIGATE', 'PERFANALYZER', 'PERFANALYZERCLOSE'],
  );
  assert.equal(CHROME_COMMANDS.length, 38);
  for (const text of ['CLEANSCREENOFF', 'cleanscreenoff', '  CleanScreenOff ', 'cLeAnScReEnOfF']) {
    assert.equal(isCleanScreenOffCommand(text), true, text);
  }
  // Exact match only: the ON word, an argument or a longer word is a different
  // line. The two words are separate selectors, never prefixes of each other.
  for (const text of [
    'CLEANSCREENON', 'CLEANSCREENOFF 0', 'CLEANSCREENOFF 1', 'CLEANSCREEN', '',
    '   ', null, undefined,
  ]) {
    assert.equal(isCleanScreenOffCommand(text), false, String(text));
  }
  assert.equal(isCleanScreenOffCommand('CLEANSCREENON'), false, 'the ON word never matches OFF');
});

test('CLEANSCREENOFF resolves to the set-false result on every layout', () => {
  // It clears the state explicitly — `value: false`, never "toggle it".
  assert.deepEqual(localSelectorFor('CLEANSCREENOFF', 'desktop'), { kind: 'cleanScreen', value: false });
  assert.deepEqual(localSelectorFor('cleanscreenoff', 'desktop'), { kind: 'cleanScreen', value: false });
  assert.deepEqual(localSelectorFor('  CleanScreenOff ', 'mobile'), { kind: 'cleanScreen', value: false });
  // Clean screen is chrome state, not a docked palette: the compact layout never
  // refuses it (unlike PROPERTIES/LAYER).
  assert.deepEqual(localSelectorFor('CLEANSCREENOFF', 'mobile'), { kind: 'cleanScreen', value: false });
  // The ON word still resolves to its own true value and is not shadowed.
  assert.deepEqual(localSelectorFor('CLEANSCREENON', 'desktop'), { kind: 'cleanScreen', value: true });
  // Near misses stay router lines.
  assert.equal(localSelectorFor('CLEANSCREENOFF 0', 'desktop'), null);
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('CLEANSCREENOFF is admitted by the chrome, with no host vocabulary and no document', () => {
  assert.equal(isSupportedCommand('CLEANSCREENOFF'), true);
  assert.equal(isSupportedCommand('cleanscreenoff'), true);
  assert.equal(admitCommandText('CLEANSCREENOFF', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('cleanscreenoff', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('CLEANSCREENOFF', { status: null, pending: true }).kind, 'admitted');
  assert.equal(
    admitCommandText('CLEANSCREENOFF', { status: { documentLabel: null, loading: false } }).kind,
    'admitted',
    'the start page can still leave clean screen',
  );
});

test('the catalog exposes CLEANSCREENOFF exactly once, appended, with its own key', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'CLEANSCREENOFF');
  assert.equal(rows.length, 1);
  assert.deepEqual(
    { ...rows[0] },
    { id: 'CLEANSCREENOFF', labelKey: 'status.clean_screen_off', icon: 'cleanScreen', group: 'view' },
  );
  // Append-only: the chrome run keeps its internal order, and every row sits
  // after the last shipped pre-2026-11-07 row (`PDSIZE`). Later router rows
  // (MODEL/PLAN) may follow the run; they never move it.
  const order = COMMAND_CATALOG.map(command => command.id);
  const at = (id: string) => order.indexOf(id);
  assert.ok(at('PDSIZE') < at('ABOUT'), 'the chrome run is appended after shipped rows');
  assert.equal(at('CLEANSCREENON') + 1, at('CLEANSCREENOFF'));
  assert.equal(at('CLEANSCREENOFF') + 1, at('PROPERTIESCLOSE'));
  assert.equal(at('PROPERTIESCLOSE') + 1, at('LAYERCLOSE'));
});

test('the File tab clean item offers both typed-word variants in append order', () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const clean = utilities.items.find(item => item.id === 'clean');
  assert.ok(clean, 'the File tab keeps its clean-screen entry');
  // The main button stays the shared toggle; the variants are the explicit set
  // selectors, ON first and the later OFF second.
  assert.equal(clean.labelKey, 'status.clean_screen');
  assert.equal(clean.action, 'cleanScreen');
  assert.equal(clean.command, undefined, 'the main entry is not a router command');
  assert.deepEqual(
    (clean.variants ?? []).map(variant => ({ ...variant })),
    [
      { id: 'clean-on', labelKey: 'status.clean_screen_on', icon: 'cleanScreen', command: 'CLEANSCREENON' },
      { id: 'clean-off', labelKey: 'status.clean_screen_off', icon: 'cleanScreen', command: 'CLEANSCREENOFF' },
    ],
  );
});

test('the local selector host sets the shared clean-screen state, never toggles it', async () => {
  const app = await readModule('App.tsx');
  const hostStart = app.indexOf('useLocalSelectors({');
  assert.ok(hostStart > 0, 'the local-selector host must exist');
  const hostBlock = app.slice(hostStart, app.indexOf('});', hostStart));
  assert.ok(hostBlock.includes('setCleanScreen,'), 'the existing React setter is passed to the local host');
  // The dispatch is a setter with the result's value — no `!` toggle and no
  // second state variable for the OFF word.
  const selectors = await readModule('commands/local-selectors.ts');
  assert.ok(
    selectors.includes("case 'cleanScreen': setCleanScreen(result.value); break;"),
    'the cleanScreen result sets the state from its boolean value',
  );
  assert.ok(selectors.includes("kind: 'cleanScreen', value: false"), 'CLEANSCREENOFF resolves to value false');
  assert.equal(selectors.includes('setCleanScreen(!'), false, 'the selector must not toggle');
  assert.equal(selectors.includes('setCleanScreenOff'), false, 'no second callback/state for the OFF word');
});

test('status.clean_screen_off is declared in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['status.clean_screen_off'], 'string');
  assert.equal(typeof en['status.clean_screen_off'], 'string');
  assert.ok(zh['status.clean_screen_off'].trim().length > 0);
  assert.ok(en['status.clean_screen_off'].trim().length > 0);
});
