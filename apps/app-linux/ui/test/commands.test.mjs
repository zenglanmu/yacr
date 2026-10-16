import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  COMMAND_CATALOG,
  defaultSuggestionIndex,
  filterCommandSuggestions,
  historyText,
  isSupportedCommand,
  normalizeCommandText,
  pushHistory,
  stepHistoryCursor,
  SUPPORTED_COMMANDS,
} from '../src/commands.mjs';
import { RIBBON_TABS, ribbonCommandIds, ribbonTab } from '../src/ribbon-model.mjs';

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canFit: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canFit: false };

test('only OPEN and FIT are supported, matching the host admission', () => {
  assert.deepEqual([...SUPPORTED_COMMANDS], ['OPEN', 'FIT']);
  assert.equal(isSupportedCommand('open'), true);
  assert.equal(isSupportedCommand(' fit '), true);
  assert.equal(isSupportedCommand('LINE'), false);
  assert.equal(normalizeCommandText('  fit '), 'FIT');
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
  assert.ok(line.every(suggestion => suggestion.supported === false));
  const open = filterCommandSuggestions('open');
  assert.deepEqual(open.map(suggestion => suggestion.id), ['OPEN']);
  assert.equal(open[0].supported, true);
  assert.deepEqual(filterCommandSuggestions('zzz'), []);
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
  assert.equal(admitCommandText('LINE', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('OPEN', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('open', { pending: true, status: loaded }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, loading: true } }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, canOpen: false } }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('FIT', { status: empty }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: null }).kind, 'unavailable');
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
