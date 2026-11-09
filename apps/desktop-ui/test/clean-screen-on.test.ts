// Contract tests for the chrome-local `CLEANSCREENON` selector (AutoCAD 2026).
//
// The typed word **sets** the clean-screen state the File tab's `cleanScreen`
// toggle already flips: it is not a second state, not a router command and never
// a toggle. `CLEANSCREENOFF` is its leaving half (own suite:
// `test/clean-screen-off.test.ts`), so the result kind carries a boolean value
// (`{ kind: 'cleanScreen', value: true }`) and the host exposes one setter
// (`setCleanScreen(value)`). These are pure-model and source assertions in the
// style of the other selector suites (no React DOM rendering).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  CLEANSCREENON_COMMAND,
  isCleanScreenOnCommand,
} from '../src/commands/chrome-commands.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { admitCommandText, isSupportedCommand } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the selector is the bare CLEANSCREENON word, case- and whitespace-insensitive', () => {
  assert.equal(CLEANSCREENON_COMMAND, 'CLEANSCREENON');
  // Append-only table: ABOUT first, then the ON row, then the OFF row, then the
  // later PROPERTIESCLOSE and LAYERCLOSE rows; the name list is derived.
  assert.deepEqual(
    [...CHROME_COMMAND_NAMES],
    ['ABOUT', 'CLEANSCREENON', 'CLEANSCREENOFF', 'PROPERTIESCLOSE', 'LAYERCLOSE', 'DSETTINGS', 'QUICKPROPERTIES', 'CLASSICLAYER', 'COMMANDLINE', 'COMMANDLINEHIDE'],
  );
  assert.equal(CHROME_COMMANDS.length, 10);
  for (const text of ['CLEANSCREENON', 'cleanscreenon', '  CleanScreenOn ', 'cLeAnScReEnOn']) {
    assert.equal(isCleanScreenOnCommand(text), true, text);
  }
  // Exact match only: an argument or a longer word is a different line.
  for (const text of ['CLEANSCREENON 1', 'CLEANSCREEN', 'CLEANSCREENOFF', '', '   ', null, undefined]) {
    assert.equal(isCleanScreenOnCommand(text), false, String(text));
  }
});

test('CLEANSCREENON resolves to the set-true result on every layout', () => {
  // It sets the state explicitly — `value: true`, never "toggle it".
  assert.deepEqual(localSelectorFor('CLEANSCREENON', 'desktop'), { kind: 'cleanScreen', value: true });
  assert.deepEqual(localSelectorFor('cleanscreenon', 'desktop'), { kind: 'cleanScreen', value: true });
  assert.deepEqual(localSelectorFor('  CleanScreenOn ', 'mobile'), { kind: 'cleanScreen', value: true });
  // Clean screen is chrome state, not a docked palette: the compact layout never
  // refuses it (unlike PROPERTIES/LAYER).
  assert.deepEqual(localSelectorFor('CLEANSCREENON', 'mobile'), { kind: 'cleanScreen', value: true });
  // A near miss (the OFF word with an argument) stays a router line; the bare
  // OFF word is its own selector (see `test/clean-screen-off.test.ts`).
  assert.equal(localSelectorFor('CLEANSCREENOFF 0', 'desktop'), null);
  assert.equal(localSelectorFor('CLEANSCREEN', 'desktop'), null);
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('CLEANSCREENON is admitted by the chrome, with no host vocabulary and no document', () => {
  assert.equal(isSupportedCommand('CLEANSCREENON'), true);
  assert.equal(isSupportedCommand('cleanscreenon'), true);
  assert.equal(admitCommandText('CLEANSCREENON', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('cleanscreenon', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('CLEANSCREENON', { status: null, pending: true }).kind, 'admitted');
  assert.equal(
    admitCommandText('CLEANSCREENON', { status: { documentLabel: null, loading: false } }).kind,
    'admitted',
    'the start page can still enter clean screen',
  );
});

test('the catalog exposes CLEANSCREENON exactly once, appended, with its own key', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'CLEANSCREENON');
  assert.equal(rows.length, 1);
  assert.deepEqual(
    { ...rows[0] },
    { id: 'CLEANSCREENON', labelKey: 'status.clean_screen_on', icon: 'cleanScreen', group: 'view' },
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

test('the File tab clean item offers a CLEANSCREENON variant and keeps its toggle action', () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const clean = utilities.items.find(item => item.id === 'clean');
  assert.ok(clean, 'the File tab keeps its clean-screen entry');
  // The main button is still the toggle, sharing the same state.
  assert.equal(clean.labelKey, 'status.clean_screen');
  assert.equal(clean.action, 'cleanScreen');
  assert.equal(clean.command, undefined, 'the main entry is not a router command');
  // The popover entry is the explicit set-on selector, and it stays first; the
  // later `clean-off` variant follows it (asserted in its own suite).
  const variants = (clean.variants ?? []).map(variant => ({ ...variant }));
  assert.deepEqual(variants[0], {
    id: 'clean-on', labelKey: 'status.clean_screen_on', icon: 'cleanScreen', command: 'CLEANSCREENON',
  });
  assert.equal(variants.length, 2, 'the toggle now offers the ON and OFF typed-word entries');
});

test('the local selector host sets the shared clean-screen state, never toggles it', async () => {
  const app = await readModule('App.tsx');
  const hostStart = app.indexOf('useLocalSelectors({');
  assert.ok(hostStart > 0, 'the local-selector host must exist');
  const hostBlock = app.slice(hostStart, app.indexOf('});', hostStart));
  assert.ok(hostBlock.includes('setCleanScreen,'), 'the existing React setter is passed to the local host');
  // The dispatch is a setter with the result's value — no `!` toggle and no
  // second state variable.
  const selectors = await readModule('commands/local-selectors.ts');
  assert.ok(
    selectors.includes("case 'cleanScreen': setCleanScreen(result.value); break;"),
    'the cleanScreen result sets the state from its boolean value',
  );
  assert.ok(selectors.includes("kind: 'cleanScreen', value: true"), 'CLEANSCREENON resolves to value true');
  assert.equal(selectors.includes('setCleanScreen(!'), false, 'the selector must not toggle');
});

test('status.clean_screen_on is declared in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['status.clean_screen_on'], 'string');
  assert.equal(typeof en['status.clean_screen_on'], 'string');
  assert.ok(zh['status.clean_screen_on'].trim().length > 0);
  assert.ok(en['status.clean_screen_on'].trim().length > 0);
});
