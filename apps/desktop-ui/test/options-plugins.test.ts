// Plugins tab contract: the tab registration, the row-state model, the file
// pre-check and the copy the tab is allowed to show.
//
// These are source/behaviour assertions, not a rendered-DOM test: the Vue/React
// tree cannot be mounted here (`test/*.ts` run under Node's type stripping and
// never enter `tsc`). What is worth pinning without a DOM is exactly what the
// logic lane can break silently: the tab must stay registered even though it
// owns no `OptionsField`, a built-in plugin must never grow an uninstall path,
// and an accepted change must never be described as live.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  OPTIONS_TABS,
  fieldsForTab,
  optionsTabs,
  requestOptionsTab,
  resetOptionsTabForTests,
  tabById,
  takeOptionsTab,
} from '../src/options/options.ts';
import {
  PLUGIN_FILE_EXTENSIONS,
  PLUGIN_MAX_BYTES,
  canDisable,
  canUninstall,
  pluginFileProblem,
  pluginFileProblemKey,
  pluginRowDomId,
  pluginState,
  pluginStateKey,
  scanDiagnostics,
  vendorSettledSummary,
} from '../src/options/plugins-view.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');

/** One complete `PluginRow`, in the shape `readPluginStatus` returns. */
function row(overrides = {}) {
  return {
    id: 'tch',
    name: 'TCH',
    version: '1.0.0',
    abi: 1,
    source: 'builtin',
    enabled: true,
    sessionActive: true,
    status: 'ok',
    ...overrides,
  };
}

// --- tab registration ------------------------------------------------------

test('the Plugins tab is registered last, self-rendered and owns no settings field', () => {
  assert.deepEqual(OPTIONS_TABS.map(tab => tab.id), ['display', 'drafting', 'plot', 'files', 'user', 'plugins']);
  const tab = tabById('plugins');
  assert.equal(tab.labelKey, 'options.tab.plugins');
  assert.equal(tab.icon, 'settings');
  assert.equal(tab.selfRendered, true);
  // The old "a tab with no field is dropped" rule would silently remove it.
  assert.deepEqual(fieldsForTab('plugins'), []);
  const rendered = optionsTabs();
  assert.equal(rendered.at(-1).id, 'plugins');
  assert.deepEqual(rendered.filter(entry => entry.selfRendered).map(entry => entry.id), ['plugins']);
});

test('the dialog renders the self-contained section for the Plugins tab only', async () => {
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes("from './OptionsPlugins.tsx'"));
  assert.ok(dialog.includes('<OptionsPlugins />'));
  assert.ok(dialog.includes("const pluginsTab = tab === 'plugins';"));
  // The Plugins page replaces the staged field list rather than being inserted
  // above or below it.
  assert.ok(dialog.includes('{pluginsTab ? ('));
  // Apply keeps its one meaning everywhere: write the staged settings draft.
  // It is never repurposed for plugin changes (those are already saved), and it
  // must keep working from any tab when another tab left edits staged.
  assert.match(dialog, /disabled=\{!ready \|\| busy \|\| dirtyCount === 0 \|\| hasLocalErrors\}/);
  assert.equal(dialog.includes('disabled={pluginsTab ||'), false);
  // OK/Cancel stay live so the dialog can always be closed; OK cannot depend on
  // the settings snapshot having loaded, because this tab does not need it.
  assert.match(dialog, /disabled=\{busy \|\| \(!pluginsTab && !ready\)\}/);
  // The footer says the one thing still pending on that tab.
  assert.ok(dialog.includes("t('plugins.restartShort')"));
  // OK stays a close on the Plugins tab even when the settings snapshot never
  // arrived; `commit` alone would return early and trap the dialog open.
  assert.ok(dialog.includes('if (pluginsTab && !ready)'));
});

test('File > Plugins hands the next Options mount its page exactly once', async () => {
  resetOptionsTabForTests();
  assert.equal(takeOptionsTab(), null, 'no request: the dialog keeps its first tab');

  requestOptionsTab('plugins');
  assert.equal(takeOptionsTab(), 'plugins');
  assert.equal(takeOptionsTab(), null, 'the handoff is consumed on the first mount');

  // A later plain Options open clears the slot instead of inheriting a stale page.
  requestOptionsTab('plugins');
  requestOptionsTab(null);
  assert.equal(takeOptionsTab(), null);
  // An empty id is no request at all; an unknown id is still returned so the
  // dialog (not this pure slot) decides the fallback.
  requestOptionsTab('');
  assert.equal(takeOptionsTab(), null);

  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('const requested = takeOptionsTab();'));
  assert.match(dialog, /tabs\.some\(entry => entry\.id === requested\)/);
  assert.ok(dialog.includes('takeOptionsTab,'), 'the page handoff is read from the model');

  const opener = await readModule('options/open-options.ts');
  assert.ok(opener.includes('requestOptionsTab(tab);'), 'the one opener owns the handoff');
  assert.ok(opener.includes("requestOpenPopup('options')"));

  const app = await readModule('App.tsx');
  // The handler sets the page and opens the one existing Options popup; it never
  // requests a second popup kind.
  const handler = app.indexOf("action === 'plugins'");
  assert.ok(handler > 0, 'runAction must answer the plugins token');
  const branch = app.slice(handler, handler + 400);
  assert.ok(branch.includes("openOptionsDialog(requestOpenPopup, 'plugins')"));
  assert.equal(app.includes("requestOpenPopup('plugins')"), false);

  const { RIBBON_TABS, SHELL_ACTIONS } = await import('../src/ribbon/ribbon-model.ts');
  assert.equal(SHELL_ACTIONS.plugins, 'plugins');
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const entry = file.panels.find(panel => panel.id === 'utilities').items.find(item => item.id === 'plugins');
  assert.equal(entry.action, 'plugins', 'the File entry dispatches the same token');
  assert.equal(entry.labelKey, 'file.plugins');

  // Leave no request behind for the rest of the suite.
  resetOptionsTabForTests();
});

// --- the row state model ---------------------------------------------------

test('a row reads as active / pending restart / disabled / invalid', () => {
  assert.equal(pluginState(row()), 'active');
  assert.equal(pluginState(row({ sessionActive: false })), 'pendingRestart');
  assert.equal(pluginState(row({ enabled: false, sessionActive: false })), 'disabled');
  // A saved-off decoder stays off even if the running session still had it: the
  // saved choice is what the next launch reads.
  assert.equal(pluginState(row({ enabled: false, sessionActive: true })), 'disabled');
  // Every non-`ok` manifest state is the same finding to this tab: the decoder
  // cannot load, so it must never be shown as active or merely disabled.
  for (const status of ['invalid', 'manifest_missing', 'manifest_invalid']) {
    assert.equal(pluginState(row({ status })), 'invalid', status);
    assert.equal(pluginState(row({ status, enabled: false })), 'invalid', status);
  }
  assert.equal(pluginState(row({ status: 'ok' })), 'active');

  assert.deepEqual(
    ['active', 'pendingRestart', 'disabled', 'invalid'].map(pluginStateKey),
    [
      'plugins.state.active', 'plugins.state.pendingRestart',
      'plugins.state.disabled', 'plugins.state.invalid',
    ],
  );
});

test('only a user plugin offers uninstall, and only a saved-on row offers disable', async () => {
  assert.equal(canUninstall(row({ source: 'builtin' })), false);
  assert.equal(canUninstall(row({ source: 'user' })), true);
  assert.equal(canDisable(row({ enabled: true })), true);
  assert.equal(canDisable(row({ enabled: false })), false);

  const component = await readModule('components/OptionsPlugins.tsx');
  // The uninstall button is behind the one predicate; a built-in row therefore
  // has no uninstall action at all (not a disabled one).
  assert.ok(component.includes('const userPlugin = canUninstall(row);'));
  assert.ok(component.includes('{userPlugin && confirmUninstall !== row.id ? ('));
  assert.equal(
    /\{userPlugin \? \(\s*<Button/.test(component),
    false,
    'the open confirmation must own the one Uninstall action',
  );
  // And it still shows the source badge, so the absence reads as a rule.
  assert.ok(component.includes("plugins.builtinBadge"));
  assert.ok(component.includes("plugins.userBadge"));
});

test('a decoder is one radio group, and arrows move focus instead of writing', async () => {
  const component = await readModule('components/OptionsPlugins.tsx');
  assert.ok(component.includes('const DECODER_GROUP ='));
  assert.ok(component.includes('type="radio"'));
  assert.ok(component.includes('name={DECODER_GROUP}'));
  assert.ok(component.includes('checked={row.enabled === true}'));
  // Every selection is a host write, so the group must not change on an arrow
  // press: the arrows are intercepted and only focus the next row.
  assert.ok(component.includes('RADIO_FOCUS_KEYS'));
  assert.ok(component.includes('event.preventDefault()'));
  assert.ok(component.includes('onKeyDown={event => moveRadioFocus(event, row.id)}'));
  assert.equal(/onKeyDown=\{[^}]*setEnabled/.test(component), false);
  // Space/click still commits through the radio's own change event.
  assert.ok(component.includes('onChange={() => setEnabled(row, true)}'));
});

// --- the picked file -------------------------------------------------------

test('the file pre-check accepts only .js/.mjs within the size limit', () => {
  assert.deepEqual(PLUGIN_FILE_EXTENSIONS, ['.js', '.mjs']);
  assert.equal(PLUGIN_MAX_BYTES, 4 * 1024 * 1024);
  assert.equal(pluginFileProblem({ name: 'tch-plugin.js', size: 2048 }), null);
  assert.equal(pluginFileProblem({ name: 'decoder.mjs', size: 0 }), null);
  assert.equal(pluginFileProblem({ name: 'DECODER.MJS', size: 1 }), null, 'the on-disk name is case-insensitive');
  assert.equal(pluginFileProblem({ name: 'plugin.ts', size: 10 }), 'fileType');
  assert.equal(pluginFileProblem({ name: 'plugin.js.map', size: 10 }), 'fileType');
  assert.equal(pluginFileProblem({ name: 'plugin', size: 10 }), 'fileType');
  assert.equal(pluginFileProblem(null), 'fileType');
  assert.equal(pluginFileProblem({ size: 10 }), 'fileType');
  // The limit is inclusive; one byte over is refused before it reaches the host.
  assert.equal(pluginFileProblem({ name: 'p.js', size: PLUGIN_MAX_BYTES }), null);
  assert.equal(pluginFileProblem({ name: 'p.js', size: PLUGIN_MAX_BYTES + 1 }), 'fileTooLarge');
  assert.equal(pluginFileProblemKey('fileType'), 'plugins.fileType');
  assert.equal(pluginFileProblemKey('fileTooLarge'), 'plugins.fileTooLarge');
});

test('a plugin id becomes a DOM-safe row id', () => {
  assert.equal(pluginRowDomId('tch'), 'tch');
  assert.equal(pluginRowDomId('my.plugin/v2'), 'my-plugin-v2');
  assert.equal(pluginRowDomId(''), 'plugin');
  assert.equal(pluginRowDomId(null), 'plugin');
});

// --- the last vendor pass --------------------------------------------------

test('the last-pass summary survives missing fields instead of inventing a count', () => {
  assert.equal(vendorSettledSummary(null), null);
  assert.equal(vendorSettledSummary(undefined), null);
  assert.equal(vendorSettledSummary('settled'), null);
  assert.equal(vendorSettledSummary([]), null);
  assert.deepEqual(vendorSettledSummary({ applied: 42, diagnostics: ['vendor.decode_failed: x'] }), {
    applied: 42,
    diagnostics: ['vendor.decode_failed: x'],
  });
  // Empty machine messages are dropped; non-strings are not rendered as `[object Object]`.
  assert.deepEqual(vendorSettledSummary({ applied: 0, diagnostics: ['ok', '', 7, null] }), {
    applied: 0,
    diagnostics: ['ok'],
  });
  // A record with no count still means "a pass happened": zero, not `null`.
  assert.deepEqual(vendorSettledSummary({ diagnostics: [] }), { applied: 0, diagnostics: [] });
  assert.deepEqual(vendorSettledSummary({ applied: Number.NaN }), { applied: 0, diagnostics: [] });
});

test('scan diagnostics are read defensively from the status frame', () => {
  assert.deepEqual(scanDiagnostics(null), []);
  assert.deepEqual(scanDiagnostics({}), []);
  assert.deepEqual(scanDiagnostics({ diagnostics: 'nope' }), []);
  assert.deepEqual(scanDiagnostics({ diagnostics: ['scan.a', 1, '', 'scan.b'] }), ['scan.a', 'scan.b']);
});

// --- copy and interaction rules -------------------------------------------

test('the tab subscribes to the last pass and never claims a live change', async () => {
  const component = await readModule('components/OptionsPlugins.tsx');
  assert.ok(component.includes('subscribeVendorSettled'));
  assert.ok(component.includes('lastVendorSettled'));
  assert.ok(component.includes("t('plugins.restart')"), 'the persistent banner copy is rendered');
  assert.ok(component.includes("t('plugins.appliedCount'"), 'the applied count is interpolated');

  // Host actions go through the shared manager, never a hand-built command name.
  for (const call of [
    'readPluginStatus(invoke)',
    'setPluginEnabled(invoke, row.id, enabled)',
    'uninstallPlugin(invoke, row.id)',
    'installPluginFile(invoke, file)',
  ]) {
    assert.ok(component.includes(call), `uses ${call}`);
  }

  // Nothing reloads the page or restarts the app: the copy is the whole story.
  for (const forbidden of ['location.reload', 'window.location', 'alert(', 'confirm(']) {
    assert.equal(component.includes(forbidden), false, `must not call ${forbidden}`);
  }
  assert.ok(!/@refresh|auto.?restart/i.test(component), 'no auto restart path');

  // An acceptance is `accepted: true`; a refusal keeps its reason visible and
  // keeps the file panel open. No branch renders "success".
  assert.ok(component.includes('result.accepted === true'));
  assert.ok(component.includes("t('plugins.errorReason')"));
  assert.ok(component.includes("t('plugins.errorLoad')"), 'a failed read has its own sentence');
  assert.ok(component.includes('refusal.reason ||'));
});

test('install is confirmed inline with the filename and the trust warning', async () => {
  const component = await readModule('components/OptionsPlugins.tsx');
  assert.ok(component.includes("type=\"file\""));
  assert.ok(component.includes('accept=".js,.mjs"'));
  assert.ok(component.includes('hidden'));
  assert.ok(component.includes("t('plugins.installWarning')"));
  assert.ok(component.includes("t('plugins.installConfirm')"));
  assert.ok(component.includes('{pendingFile.name}'), 'the picked filename is shown');
  // Cancel and the close affordance both leave the confirmation.
  assert.ok(component.includes("t('plugins.cancel')"));
  assert.ok(component.includes("t('plugins.close')"));
  // Uninstall asks in the row itself, not in a browser confirm.
  assert.ok(component.includes("t('plugins.uninstallConfirm')"));
  assert.ok(component.includes('confirmUninstall === row.id'));
});

test('the unsupported carrier is named instead of showing an empty list', async () => {
  const component = await readModule('components/OptionsPlugins.tsx');
  assert.ok(component.includes('frame.supported !== true'));
  assert.ok(component.includes('frame.carrier'));
  assert.ok(component.includes("t('plugins.unsupported')"));
  // The carrier's own refusal message is the reason it is unsupported.
  assert.match(component, /scan\.length > 0[\s\S]*?unsupported-/);
  // Install/uninstall only exist on the ready+supported path: this early return
  // is what keeps a carrier without plugin support from offering dead actions.
  assert.ok(component.indexOf('frame.supported !== true') < component.indexOf('plugins.install'));
});

test('the component carries no hardcoded CJK', async () => {
  const component = await readModule('components/OptionsPlugins.tsx');
  const view = await readModule('options/plugins-view.ts');
  for (const [name, source] of [['OptionsPlugins.tsx', component], ['plugins-view.ts', view]]) {
    assert.equal(/[\u3400-\u4dbf\u4e00-\u9fff\u3000-\u303f\uff00-\uffef]/.test(source), false, name);
  }
});

// --- catalog ---------------------------------------------------------------

test('every Plugins key exists in both catalogs with the same placeholders', () => {
  const keys = [
    'options.tab.plugins',
    'plugins.title', 'plugins.intro', 'plugins.decoders',
    'plugins.restart', 'plugins.restartShort',
    'plugins.install', 'plugins.installHint', 'plugins.installWarning', 'plugins.installConfirm',
    'plugins.cancel', 'plugins.uninstall', 'plugins.uninstallConfirm',
    'plugins.builtinBadge', 'plugins.userBadge', 'plugins.enable', 'plugins.disable',
    'plugins.state.active', 'plugins.state.pendingRestart', 'plugins.state.disabled', 'plugins.state.invalid',
    'plugins.diagnostics', 'plugins.scanDiagnostics', 'plugins.noDiagnostics', 'plugins.appliedCount',
    'plugins.empty', 'plugins.loading', 'plugins.refresh', 'plugins.unsupported',
    'plugins.errorGeneric', 'plugins.errorLoad', 'plugins.errorReason',
    'plugins.fileTooLarge', 'plugins.fileType',
    'plugins.version', 'plugins.id', 'plugins.source', 'plugins.close',
  ];
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of keys) {
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[key], 'string', `${locale} is missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} has empty copy for ${key}`);
    }
    assert.deepEqual(
      [...zh[key].matchAll(placeholder)].map(match => match[1]).sort(),
      [...en[key].matchAll(placeholder)].map(match => match[1]).sort(),
      `placeholders differ for ${key}`,
    );
  }
  // The one placeholder-bearing string really interpolates.
  assert.equal(tEn('plugins.appliedCount', { count: 7 }), 'Objects applied: 7');
  assert.equal(tEn('plugins.state.pendingRestart'), 'Restart required');
  // The two restart strings both say the change waits for the next launch; the
  // tab must never promise a live reload.
  for (const key of ['plugins.restart', 'plugins.restartShort']) {
    assert.match(en[key], /next (time the app starts|launch)/i, key);
    assert.doesNotMatch(en[key], /appl(y|ies) now|reload|restart now/i, key);
  }
  assert.match(en['plugins.installWarning'], /runs the plugin once/i);
  assert.match(en['plugins.installWarning'], /access/i);
});
