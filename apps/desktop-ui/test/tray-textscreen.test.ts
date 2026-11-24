// Contract tests for the two surfaces this lane gave a real consumer:
//
//   * the **text screen** (`TEXTSCR`/`GRAPHSCR`), whose visibility is the host's
//     `cad_status.textScreen` session flag and whose body is the *same* folded
//     command history the command line renders (no second history model), and
//   * the **Tray Settings** surface (`TRAYSETTINGS`), whose one switch is the
//     host's `cad_status.traySettings` flag, and the status bar whose tray that
//     flag hides.
//
// No React DOM is rendered: the stores/helpers are pure and the component
// sources are checked for the wiring they must keep.
import test from 'node:test';
import assert from 'node:assert/strict';
import { textScreenOpen, traySettingsOn, TRAY_SETTINGS_COMMAND } from '../src/cad/cad.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { isTraySettingsCommand, TRAYSETTINGS_COMMAND } from '../src/commands/chrome-commands.ts';
import {
  chromeSurfaceSnapshot,
  closeTraySettings,
  openTraySettings,
  resetChromeSurfaces,
} from '../src/chrome/surfaces.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('the text screen follows the host flag and an absent field means closed', () => {
  assert.equal(textScreenOpen({ textScreen: true }), true);
  assert.equal(textScreenOpen({ textScreen: false }), false);
  for (const status of [null, undefined, {}, { textScreen: null }, { textScreen: 'true' }, { textScreen: 1 }]) {
    assert.equal(textScreenOpen(status), false, `no fabricated open window for ${JSON.stringify(status)}`);
  }
});

test('the tray uses the host flag and an absent field keeps the historical tray', () => {
  assert.equal(traySettingsOn({ traySettings: true }), true);
  assert.equal(traySettingsOn({ traySettings: false }), false);
  for (const status of [null, undefined, {}, { traySettings: null }, { traySettings: 'false' }]) {
    assert.equal(traySettingsOn(status), true, `older hosts keep the tray for ${JSON.stringify(status)}`);
  }
});

test('TRAYSETTINGS typed opens the surface on desktop and is refused by name on mobile', () => {
  assert.equal(TRAYSETTINGS_COMMAND, 'TRAYSETTINGS');
  assert.equal(TRAY_SETTINGS_COMMAND, 'TRAYSETTINGS');
  assert.equal(isTraySettingsCommand('tra ysettings'), false);
  assert.equal(isTraySettingsCommand('  traysettings  '), true);
  assert.deepEqual(localSelectorFor('TRAYSETTINGS', 'desktop'), { kind: 'traySettings' });
  assert.deepEqual(localSelectorFor('traysettings', 'desktop'), { kind: 'traySettings' });
  // The compact layout has no status bar/tray, so the word must answer a refusal
  // instead of opening a surface with nothing to configure.
  assert.deepEqual(localSelectorFor('TRAYSETTINGS', 'mobile'), { kind: 'traySettingsUnavailable' });
});

test('the tray settings surface opens, closes and resets through one store', () => {
  resetChromeSurfaces();
  assert.equal(chromeSurfaceSnapshot().traySettings, false);
  openTraySettings();
  assert.equal(chromeSurfaceSnapshot().traySettings, true);
  // Opening another float closes it, like every other chrome surface.
  openTraySettings();
  assert.equal(chromeSurfaceSnapshot().traySettings, true);
  closeTraySettings();
  assert.equal(chromeSurfaceSnapshot().traySettings, false);
  // A second close is an idempotent success.
  closeTraySettings();
  assert.equal(chromeSurfaceSnapshot().traySettings, false);
  resetChromeSurfaces();
});

test('the text screen renders the existing history model, never a second log', async () => {
  const source = await readModule('components/TextScreen.tsx');
  assert.ok(source.includes("from '../commands/command-window.ts'"), 'it reuses the command-window fold');
  assert.ok(source.includes('commandWindowRows('), 'the rows come from the one fold');
  assert.ok(source.includes("from '../commands/history-copy.ts'"), 'it reuses the history serializer');
  assert.ok(source.includes('commandWindowText('), 'the body is the visible history text');
  assert.ok(source.includes('textScreenOpen(shell.status)'), 'visibility is the host flag');
  assert.ok(!source.includes('useState'), 'no local history or visibility mirror');
  assert.ok(source.includes("runRef.current(GRAPH_SCREEN_COMMAND)"), 'closing sends GRAPHSCR');
  assert.ok(source.includes("'textscreen.empty'"), 'an empty session is named');
  assert.ok(source.includes("'textscreen.unavailable'"), 'an absent echo source is named, not faked');
  const surfaces = await readModule('components/ChromeSurfaces.tsx');
  assert.ok(surfaces.includes('<TextScreen />'), 'ChromeSurfaces is the one mount point');
  assert.ok(surfaces.includes('<TraySettingsSurface />'), 'the tray surface is mounted there too');
});

test('the tray surface reads the host flag and dispatches the one router command', async () => {
  const source = await readModule('components/TraySettings.tsx');
  assert.ok(source.includes('traySettingsOn(shell.status)'), 'the checkbox reads the host flag');
  assert.ok(source.includes('shell.runCommand(TRAY_SETTINGS_COMMAND)'), 'the switch is the router command');
  assert.ok(source.includes('shell.canCommand(TRAY_SETTINGS_COMMAND)'), 'admission is the shared rule');
  assert.ok(!source.includes('useState'), 'no local mirror of the flag');
  assert.ok(source.includes("'tray.contents'"), 'the surface names what the tray really holds');
});

test('the status bar hides its tray cluster when the flag is off', async () => {
  const source = await readModule('components/StatusBar.tsx');
  assert.ok(source.includes('traySettingsOn(shell.status)'), 'the bar reads the same flag');
  assert.ok(source.includes('{trayVisible ? ('), 'the tray cluster is conditional');
  assert.ok(source.includes('openTraySettings'), 'the always-visible tray settings entry stays');
  assert.ok(source.includes("t('tray.title')"), 'the entry has a real accessible name');
});

test('both surfaces have bilingual copy, including the mobile refusal', () => {
  const keys = [
    'tray.title', 'tray.close', 'tray.hint', 'tray.toggle', 'tray.toggle_hint_on',
    'tray.toggle_hint_off', 'tray.contents', 'traysettings.refusal.no_status_bar',
    'textscreen.title', 'textscreen.close', 'textscreen.hint', 'textscreen.empty',
    'textscreen.unavailable',
  ];
  for (const key of keys) {
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[key], 'string', `${locale} missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} empty ${key}`);
    }
  }
  assert.match(en['textscreen.unavailable'], /no fabricated/i);
  assert.match(en['traysettings.refusal.no_status_bar'], /no status bar or tray/i);
  assert.match(zh['traysettings.refusal.no_status_bar'], /状态栏/);
});
