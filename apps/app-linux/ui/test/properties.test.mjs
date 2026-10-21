// Properties palette contract: the pure selection view (empty / single / multi),
// the machine-key label set, and the rendered structure of the palette.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseSelection, propertyGroupLabel, propertyRowLabel, selectionView } from '../src/cad.mjs';
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

test('property groups have catalog labels and unknown groups stay raw', () => {
  assert.equal(propertyGroupLabel(t, 'entity'), zh['properties.group.entity']);
  assert.equal(propertyGroupLabel(t, 'geometry'), zh['properties.group.geometry']);
  assert.equal(propertyGroupLabel(t, 'text'), zh['properties.group.text']);
  assert.equal(propertyGroupLabel(t, 'display'), zh['properties.group.display']);
  assert.equal(propertyGroupLabel(createTranslator({ 'zh-CN': zh, en }, 'en'), 'display'), en['properties.group.display']);
  assert.equal(propertyGroupLabel(t, 'future_group'), 'future_group');
});

// The wire block for two selected objects sharing a layer: `id` is mixed (no
// value), `layer` keeps its one real value and the real layer table as options.
const twoEntityWire = {
  count: 2,
  empty: false,
  rows: [
    { key: 'id', value: null, group: 'entity', mixed: true, editable: false, kind: 'text', options: [] },
    { key: 'layer', value: '0 (0)', group: 'entity', mixed: false, editable: true, kind: 'enum', options: ['0', 'WALLS'] },
  ],
  mixedKeys: ['id'],
  text: null,
  text_multiline: false,
};

test('parseSelection accepts a multi-selection wire sample and returns it unchanged', () => {
  assert.equal(parseSelection(twoEntityWire), twoEntityWire);
});

test('parseSelection accepts a single-selection row set only with the full metadata', () => {
  const single = {
    count: 1,
    empty: false,
    rows: [
      { key: 'id', value: '1', group: 'entity', mixed: false, editable: false, kind: 'text', options: [] },
      { key: 'layer', value: '0 (0)', group: 'entity', mixed: false, editable: true, kind: 'enum', options: ['0'] },
    ],
    mixedKeys: [],
    text: null,
    text_multiline: false,
  };
  assert.equal(parseSelection(single), single);
});

test('parseSelection rejects genuinely malformed selection payloads', () => {
  const clone = value => JSON.parse(JSON.stringify(value));
  const broken = [
    // A value and the mixed marker disagree.
    { ...clone(twoEntityWire), rows: [{ ...clone(twoEntityWire).rows[0], value: '1' }], mixedKeys: ['id'] },
    // `mixedKeys` drifts from the rows.
    { ...clone(twoEntityWire), mixedKeys: [] },
    // A mixed key on a single selection.
    { ...clone(twoEntityWire), count: 1 },
    // Rows without a selection.
    { ...clone(twoEntityWire), count: 0, empty: true },
    // Unknown editing kind.
    { ...clone(twoEntityWire), rows: [{ ...clone(twoEntityWire).rows[0], kind: 'color' }] },
    // Missing group.
    { ...clone(twoEntityWire), rows: [{ ...clone(twoEntityWire).rows[0], group: '' }] },
    // Non-string option.
    { ...clone(twoEntityWire), rows: [{ ...clone(twoEntityWire).rows[1], options: [0] }] },
  ];
  for (const value of broken) assert.throws(() => parseSelection(value), /CAD_STATUS_INVALID/);
  assert.throws(() => parseSelection(null), /CAD_STATUS_INVALID/);
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
  // The property rows are read-only: selection is a host operation, so there is
  // no editing or clear affordance for them. The one editable control is the
  // text-content editor, a separate component (ADR 0020 addendum B) — it is
  // mounted here, not inlined, so the row table stays a plain `<dl>`.
  assert.ok(!source.includes('properties.clear_selection'));
  assert.ok(!source.includes('<input'));
  assert.ok(!source.includes('<textarea'));
  assert.ok(source.includes('<SelectionTextEditor />'));
  assert.ok(!source.includes("invoke('cad_set_selected_text'"), 'the palette itself never sends the edit IPC');
});

test('the properties palette is docked in the workspace and toggled from the status bar', async () => {
  const workspace = await readModule('components/Workspace.jsx');
  const statusbar = await readModule('components/StatusBar.jsx');
  assert.ok(workspace.includes('PropertiesPalette'));
  assert.ok(workspace.includes('shell.propertiesOpen'));
  assert.ok(statusbar.includes('shell.setPropertiesOpen'));
  // The native CAD partition stays a childless node whose CSS we do not change.
  assert.ok(workspace.includes("'native-input'"));
});
