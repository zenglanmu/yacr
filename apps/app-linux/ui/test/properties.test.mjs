// Properties palette contract: the pure selection view (empty / single / multi),
// the machine-key label set, and the rendered structure of the palette.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { propertyRowLabel, selectionView } from '../src/cad.mjs';
import { createTranslator } from '../src/i18n.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');

test('selectionView is empty for a missing block and for an explicit empty selection', () => {
  for (const selection of [null, undefined, { count: 0, empty: true, rows: [], mixedKeys: [] }]) {
    const view = selectionView(selection, t);
    assert.equal(view.kind, 'empty');
    assert.equal(view.count, 0);
    assert.deepEqual(view.rows, []);
  }
});

test('selectionView maps a single object row set to catalog label/value pairs', () => {
  const view = selectionView(
    { count: 1, empty: false, rows: [{ key: 'id', value: '1' }, { key: 'length', value: '5.000000' }], mixedKeys: [] },
    t,
  );
  assert.equal(view.kind, 'single');
  assert.equal(view.count, 1);
  assert.deepEqual(view.rows, [
    { key: 'id', label: zh['properties.row.id'], value: '1' },
    { key: 'length', label: zh['properties.row.length'], value: '5.000000' },
  ]);
});

test('selectionView shows the count and labelled mixed keys for a multi-selection', () => {
  const view = selectionView({ count: 3, empty: false, rows: [], mixedKeys: ['id', 'type'] }, t);
  assert.equal(view.kind, 'multi');
  assert.equal(view.count, 3);
  assert.deepEqual(view.rows, []);
  assert.deepEqual(view.mixedKeys, [zh['properties.row.id'], zh['properties.row.type']]);
  assert.equal(view.countLabel, zh['properties.selected_count'].replace('{count}', '3'));
  assert.equal(view.mixedLabel, zh['properties.mixed'].replace('{keys}', `${zh['properties.row.id']}, ${zh['properties.row.type']}`));
});

test('a multi-selection with no differing keys carries no mixed label instead of an empty one', () => {
  const view = selectionView({ count: 2, empty: false, rows: [], mixedKeys: [] }, t);
  assert.equal(view.kind, 'multi');
  assert.equal(view.mixedLabel, null);
});

test('an unknown machine key is shown by its raw key, never a fabricated label', () => {
  assert.equal(propertyRowLabel(t, 'future_key'), 'future_key');
  const view = selectionView(
    { count: 1, empty: false, rows: [{ key: 'future_key', value: 'raw' }], mixedKeys: [] },
    t,
  );
  assert.deepEqual(view.rows, [{ key: 'future_key', label: 'future_key', value: 'raw' }]);
});

test('every machine key emitted by cad-app::selection has a label in both catalogs', async () => {
  // Read the Rust source of truth so a new row key cannot silently ship unlabeled.
  const source = await readFile(new URL('../../../../crates/cad-app/src/selection.rs', import.meta.url), 'utf8');
  const keys = new Set([...source.matchAll(/PropertyRow::new\(\s*"([a-z0-9_]+)"/g)].map(match => match[1]));
  assert.ok(keys.size > 30, `expected the selection module to emit many row keys, saw ${keys.size}`);
  for (const key of keys) {
    assert.equal(typeof zh[`properties.row.${key}`], 'string', `zh-CN missing properties.row.${key}`);
    assert.equal(typeof en[`properties.row.${key}`], 'string', `en missing properties.row.${key}`);
    // The switch must map the key to that catalog entry, not fall through to raw.
    assert.equal(propertyRowLabel(t, key), zh[`properties.row.${key}`], `no label mapped for ${key}`);
    assert.equal(propertyRowLabel(createTranslator({ 'zh-CN': zh, en }, 'en'), key), en[`properties.row.${key}`], `no en label mapped for ${key}`);
  }
});

test('the properties palette renders the three states from the real status block only', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  assert.ok(source.includes('selectionView(shell.status?.selection ?? null, t)'));
  assert.ok(source.includes("t('properties.panel')"));
  assert.ok(source.includes("t('properties.empty')"));
  assert.ok(source.includes("t('properties.close')"));
  assert.ok(source.includes("view.kind === 'single'"));
  assert.ok(source.includes("view.kind === 'multi'"));
  assert.ok(source.includes('row.label'));
  // Read-only: selection is a host operation, so no editing or clear affordance.
  assert.ok(!source.includes('properties.clear_selection'));
  assert.ok(!source.includes('<input'));
  assert.ok(!source.includes('<textarea'));
});

test('the properties palette is docked in the workspace and toggled from the status bar', async () => {
  const workspace = await readModule('components/Workspace.jsx');
  const statusbar = await readModule('components/StatusBar.jsx');
  assert.ok(workspace.includes('PropertiesPalette'));
  assert.ok(workspace.includes('shell.propertiesOpen'));
  assert.ok(statusbar.includes('shell.setPropertiesOpen'));
  // The native CAD partition stays a childless node whose CSS we do not change.
  assert.ok(workspace.includes('className="native-input"'));
});
