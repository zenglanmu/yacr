// Contract tests for the chrome-local `ABOUT` selector.
//
// The typed word only opens the host-owned About dialog that already exists: it
// sends no router command, carries no payload and adds no second dialog. These
// are pure-model and source assertions in the style of the other selector suites
// (no React DOM rendering).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ABOUT_COMMAND,
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  isAboutCommand,
} from '../src/commands/chrome-commands.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { admitCommandText, isSupportedCommand } from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the selector is the bare ABOUT word, case- and whitespace-insensitive', () => {
  assert.equal(ABOUT_COMMAND, 'ABOUT');
  // The append-only table opens with this row (later selectors append after it),
  // and the name list is derived from it (never a second list).
  assert.equal(CHROME_COMMANDS[0].name, 'ABOUT');
  assert.equal(CHROME_COMMAND_NAMES[0], 'ABOUT');
  assert.equal(CHROME_COMMAND_NAMES.length, CHROME_COMMANDS.length);
  for (const text of ['ABOUT', 'about', '  About ', 'AbOuT']) {
    assert.equal(isAboutCommand(text), true, text);
  }
  // Exact match only: an argument or a longer word is a different line.
  for (const text of ['ABOUT 1', 'ABOUT ME', 'ABOUTS', '', '   ', null, undefined]) {
    assert.equal(isAboutCommand(text), false, String(text));
  }
});

test('ABOUT resolves to its own local result on every layout', () => {
  assert.deepEqual(localSelectorFor('ABOUT', 'desktop'), { kind: 'about' });
  assert.deepEqual(localSelectorFor('about', 'desktop'), { kind: 'about' });
  assert.deepEqual(localSelectorFor('  About ', 'mobile'), { kind: 'about' });
  // It opens a dialog, not a docked palette, so the compact layout never refuses
  // it (the FONTS/RESOURCES rule, unlike PROPERTIES/LAYER).
  assert.deepEqual(localSelectorFor('ABOUT', 'mobile'), { kind: 'about' });
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('ABOUT is admitted by the chrome, with no host vocabulary and no document', () => {
  assert.equal(isSupportedCommand('ABOUT'), true);
  assert.equal(isSupportedCommand('about'), true);
  assert.equal(admitCommandText('ABOUT', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('about', { status: null }).kind, 'admitted');
  assert.equal(admitCommandText('ABOUT', { status: null, pending: true }).kind, 'admitted');
  assert.equal(
    admitCommandText('ABOUT', { status: { documentLabel: null, loading: false } }).kind,
    'admitted',
    'the start page can still show the About dialog',
  );
});

test('the catalog exposes ABOUT exactly once, appended, with the existing key', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'ABOUT');
  assert.equal(rows.length, 1);
  assert.deepEqual(
    { ...rows[0] },
    { id: 'ABOUT', labelKey: 'file.about', icon: 'app', group: 'file' },
  );
  // Append-only: later rows (CLEANSCREENON today) may follow, and no shipped row
  // may move across this one.
  const catalogIds = COMMAND_CATALOG.map(command => command.id);
  assert.ok(catalogIds.indexOf('ABOUT') > 0);
  assert.equal(catalogIds[catalogIds.indexOf('ABOUT') - 1], 'PDSIZE');
});

test('the typed command and the File tab share requestOpenPopup(about)', async () => {
  const app = await readModule('App.tsx');
  // The local selector is wired to the one existing dialog opener.
  assert.ok(
    app.includes("openAbout: () => requestOpenPopup('about')"),
    'the local selector must open the existing About dialog',
  );
  // The File tab's action token keeps its own path to that same opener.
  assert.ok(app.includes("if (action === 'about')"), 'the ribbon action path stays wired');
  assert.ok(app.includes("requestOpenPopup('about');"), 'both paths call the one opener');
  // The ribbon token is unchanged, so its active-state logic (`action === 'about'`)
  // keeps matching the dialog it opens.
  const ribbon = await readModule('ribbon/ribbon-model.ts');
  assert.ok(
    ribbon.includes("{ id: 'about', labelKey: POPUP_KEYS.ribbonEntry, icon: 'app', action: 'about' }"),
    'the File tab keeps its About affordance and action token',
  );
});

test('file.about is declared in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['file.about'], 'string');
  assert.equal(typeof en['file.about'], 'string');
});
