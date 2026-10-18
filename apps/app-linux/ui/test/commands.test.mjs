import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  COMMAND_CATALOG,
  commandCatalog,
  defaultSuggestionIndex,
  excludedCommandNames,
  filterCommandSuggestions,
  HOST_EXCLUDED_COMMANDS,
  historyText,
  isSupportedCommand,
  normalizeCommandText,
  pushHistory,
  setCommandCatalog,
  stepHistoryCursor,
  toolCommandForKey,
  toolLabelKey,
} from '../src/commands.mjs';
import { RIBBON_TABS, ribbonCommandIds, ribbonTab } from '../src/ribbon-model.mjs';

// Mirrors `cad_ui_core::session::admissible_names()` (canonical selectors plus
// aliases) closely enough for admission tests. NEW/SAVE/PLOT are present because
// the **router** accepts them; the chrome excludes them explicitly.
const HOST_NAMES = [
  'OPEN', 'NEW', 'PLOT', 'SAVE', 'CANCEL', 'PAN', 'ZOOM', 'FIT', 'RESETVIEW', 'TOGGLELAYER',
  'SETLAYERS', 'RESTORELAYERS', 'SPACE', 'SELECT', 'SELECTALL', 'MEASURE', 'CONFIRM',
  'CANCELMEASURE', 'UNDO', 'REDO', 'RESOURCES', 'DIAGNOSTICS', 'SWITCHBACKEND', 'SWITCH2D3D',
  'STANDARDVIEW', 'ORBIT', 'PROJECTION', 'MODE', 'LINE', 'CIRCLE', 'ARC', 'MOVE', 'TRIM',
  'EXTEND', 'ERASE', 'COPY', 'ROTATE', 'SCALE', 'MIRROR', 'COPYCLIP', 'CUTCLIP', 'PASTECLIP',
  'OFFSET', 'SETACTIVELAYER', 'EXPORT', 'PRINT', 'SAVEAS', 'QSAVE', 'ZOOM EXTENTS', 'ENTER',
];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canFit: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canFit: false };

test('admission is the loaded host catalog: empty before load, exclusions stay explicit', () => {
  // Before the host answers, nothing is supported (explicitly, not by default).
  setCommandCatalog([]);
  assert.deepEqual(commandCatalog(), []);
  assert.equal(isSupportedCommand('OPEN'), false);
  assert.equal(isSupportedCommand('LINE'), false);
  assert.equal(admitCommandText('OPEN', { status: loaded }).kind, 'unsupported');

  setCommandCatalog(HOST_NAMES);
  assert.equal(isSupportedCommand('open'), true);
  assert.equal(isSupportedCommand(' fit '), true);
  assert.equal(isSupportedCommand('LINE'), true);
  assert.equal(isSupportedCommand('MOVE'), true);
  assert.equal(isSupportedCommand('UNDO'), true);
  assert.equal(isSupportedCommand('CONFIRM'), true);
  // Host-owned names the React host cannot service are excluded explicitly.
  assert.deepEqual(excludedCommandNames(), ['NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT']);
  assert.deepEqual([...HOST_EXCLUDED_COMMANDS], ['NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT']);
  for (const name of HOST_EXCLUDED_COMMANDS) assert.equal(isSupportedCommand(name), false, name);
  // Unknown names are never admitted.
  assert.equal(isSupportedCommand('NOT_A_COMMAND'), false);
  assert.equal(isSupportedCommand(''), false);
  assert.equal(normalizeCommandText('  fit '), 'FIT');
  // The setter normalizes and dedupes; then restore the canonical list.
  assert.deepEqual(setCommandCatalog(['line', 'LINE', ' circle ', '']), ['LINE', 'CIRCLE']);
  setCommandCatalog(HOST_NAMES);
});

test('autocomplete prefers prefix matches, then usage, then alphabetical', () => {
  const all = filterCommandSuggestions('');
  assert.ok(all.length > 0);
  const prefix = filterCommandSuggestions('co');
  assert.equal(prefix[0].id, 'COPY');
  // Mid-string search still finds commands that do not start with the needle.
  assert.ok(filterCommandSuggestions('ine').some(suggestion => suggestion.id === 'LINE'));
  const used = filterCommandSuggestions('c', { query: 'c', history: ['CIRCLE', 'CIRCLE'] });
  assert.equal(used[0].id, 'CIRCLE');
  const limited = filterCommandSuggestions('', { limit: 3 });
  assert.equal(limited.length, 3);
  assert.equal('score' in limited[0], false);
  assert.equal('usage' in limited[0], false);
});

test('autocomplete marks supported commands and never injects unknown ids', () => {
  const line = filterCommandSuggestions('LINE');
  assert.equal(line[0].id, 'LINE');
  assert.equal(line.find(suggestion => suggestion.id === 'LINE').supported, true);
  // `PLINE` is a chrome display alias the router does not know: it stays unsupported.
  assert.equal(line.find(suggestion => suggestion.id === 'PLINE').supported, false);
  const open = filterCommandSuggestions('open');
  assert.deepEqual(open.map(suggestion => suggestion.id), ['OPEN']);
  assert.equal(open[0].supported, true);
  assert.deepEqual(filterCommandSuggestions('zzz'), []);
  // Before the host answers, every suggestion is unsupported — never "allowed".
  setCommandCatalog([]);
  assert.ok(filterCommandSuggestions('LINE').every(suggestion => suggestion.supported === false));
  setCommandCatalog(HOST_NAMES);
});

test('default suggestion index highlights an exact match, else the first row', () => {
  const suggestions = [{ id: 'FIT' }, { id: 'FITNESS' }];
  assert.equal(defaultSuggestionIndex(suggestions, 'FITNESS'), 1);
  assert.equal(defaultSuggestionIndex(suggestions, 'FIT'), 0);
  assert.equal(defaultSuggestionIndex(suggestions, ''), 0);
  assert.equal(defaultSuggestionIndex([], ''), -1);
});

test('history cursor walks older entries then returns to the draft', () => {
  const history = ['OPEN', 'FIT', 'LINE'];
  let index = null;
  index = stepHistoryCursor(history, index, -1);
  assert.equal(index, 2);
  assert.equal(historyText(history, index, ''), 'LINE');
  index = stepHistoryCursor(history, index, -1);
  assert.equal(index, 1);
  index = stepHistoryCursor(history, index, 1);
  assert.equal(index, 2);
  index = stepHistoryCursor(history, index, 1);
  assert.equal(index, null);
  assert.equal(historyText(history, index, 'draft'), 'draft');
  assert.equal(stepHistoryCursor([], null, -1), null);
  index = stepHistoryCursor(history, 0, -1);
  assert.equal(index, 0);
});

test('command history is bounded and drops adjacent duplicates', () => {
  assert.deepEqual(pushHistory([], 'OPEN'), ['OPEN']);
  assert.deepEqual(pushHistory(['OPEN'], 'OPEN'), ['OPEN']);
  assert.deepEqual(pushHistory(['OPEN'], '  '), ['OPEN']);
  let history = [];
  for (const id of ['A', 'B', 'C', 'D']) history = pushHistory(history, id, 2);
  assert.deepEqual(history, ['C', 'D']);
});

test('admission mirrors cad_command: unsupported never reaches IPC, capability gates apply', () => {
  assert.equal(admitCommandText('', { status: loaded }).kind, 'empty');
  // Interactive tools the router accepts are admitted; excluded/unknown names are not.
  assert.equal(admitCommandText('LINE', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('CONFIRM', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('NEW', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('SAVE', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('PLOT', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('NOT_A_COMMAND', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('OPEN', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('open', { pending: true, status: loaded }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, loading: true } }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, canOpen: false } }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('FIT', { status: empty }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: null }).kind, 'unavailable');
});

test('Enter and Escape drive the active tool, but never while typing or without a tool', () => {
  assert.equal(toolCommandForKey('Enter', { toolActive: true }), 'CONFIRM');
  assert.equal(toolCommandForKey('Escape', { toolActive: true }), 'CANCEL');
  // A field owns its keys (the command line handles its own Enter).
  assert.equal(toolCommandForKey('Enter', { toolActive: true, typing: true }), null);
  assert.equal(toolCommandForKey('Escape', { toolActive: true, typing: true }), null);
  // IME composition and auto-repeat are not confirm/cancel.
  assert.equal(toolCommandForKey('Enter', { toolActive: true, composing: true }), null);
  assert.equal(toolCommandForKey('Enter', { toolActive: true, repeat: true }), null);
  // No active tool: Escape must not send CANCEL (it keeps closing panels).
  assert.equal(toolCommandForKey('Escape', { toolActive: false }), null);
  assert.equal(toolCommandForKey('Enter', { toolActive: false }), null);
  // Unrelated keys are never intercepted.
  assert.equal(toolCommandForKey('F2', { toolActive: true }), null);
  assert.equal(toolCommandForKey('a', { toolActive: true }), null);
});

test('a host tool selector maps to its catalog key', () => {
  assert.equal(toolLabelKey('draw.line'), 'tool.draw.line');
  assert.equal(toolLabelKey('measure.distance'), 'tool.measure.distance');
  assert.equal(toolLabelKey('  draw.arc '), 'tool.draw.arc');
  assert.equal(toolLabelKey(''), null);
  assert.equal(toolLabelKey(null), null);
  assert.equal(toolLabelKey(undefined), null);
});

test('every ribbon command id and label key exists in the shared catalog', async () => {
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const ids = ribbonCommandIds();
  assert.ok(ids.length >= 15, `ribbon should reference many commands, saw ${ids.length}`);
  const known = new Set(COMMAND_CATALOG.map(command => command.id));
  for (const id of ids) assert.ok(known.has(id), `ribbon command ${id} is not in the catalog`);
  for (const tab of RIBBON_TABS) {
    assert.equal(typeof zh[tab.labelKey], 'string', tab.labelKey);
    for (const panel of tab.panels) {
      assert.equal(typeof zh[panel.titleKey], 'string', panel.titleKey);
      for (const item of panel.items) {
        assert.equal(typeof zh[item.labelKey], 'string', item.labelKey);
        assert.equal(typeof en[item.labelKey], 'string', item.labelKey);
        for (const variant of item.variants ?? []) {
          assert.equal(typeof zh[variant.labelKey], 'string', variant.labelKey);
        }
      }
    }
  }
  assert.equal(ribbonTab('nope').id, 'home');
});
