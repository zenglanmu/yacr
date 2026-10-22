// Properties palette contract: the wire parser, the machine-key label set, the
// grouped inspector's structure, and every entry point that opens it.
//
// The pure view/write model has its own suite (`properties-view.test.mjs`).
// This file keeps the host-facing half: what `cad_status.selection` may contain,
// that no emitted key ships unlabeled, and that the rendered palette edits
// locally-until-commit instead of writing per keystroke.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseSelection, propertyGroupLabel, propertyRowLabel, queueCadCommand } from '../src/cad.mjs';
import { admitCommandText, isSupportedCommand } from '../src/commands.mjs';
import { createTranslator } from '../src/i18n.mjs';
import { isPropertiesCommand } from '../src/properties.mjs';
import { RIBBON_TABS } from '../src/ribbon-model.mjs';
import { readCatalog, readCss, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');

// --- labels ------------------------------------------------------------

test('an unknown machine key is shown by its raw key, never a fabricated label', () => {
  assert.equal(propertyRowLabel(t, 'future_key'), 'future_key');
});

test('property groups have catalog labels and unknown groups stay raw', () => {
  assert.equal(propertyGroupLabel(t, 'entity'), zh['properties.group.entity']);
  assert.equal(propertyGroupLabel(t, 'geometry'), zh['properties.group.geometry']);
  assert.equal(propertyGroupLabel(t, 'text'), zh['properties.group.text']);
  assert.equal(propertyGroupLabel(t, 'display'), zh['properties.group.display']);
  assert.equal(propertyGroupLabel(tEn, 'display'), en['properties.group.display']);
  assert.equal(propertyGroupLabel(t, 'future_group'), 'future_group');
});

// --- wire --------------------------------------------------------------

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
  const source = await readFile(new URL('../../../crates/cad-app/src/selection.rs', import.meta.url), 'utf8');
  const keys = new Set([...source.matchAll(/PropertyRow::new\(\s*"([a-z0-9_]+)"/g)].map(match => match[1]));
  assert.ok(keys.size > 30, `expected the selection module to emit many row keys, saw ${keys.size}`);
  for (const key of keys) {
    assert.equal(typeof zh[`properties.row.${key}`], 'string', `zh-CN missing properties.row.${key}`);
    assert.equal(typeof en[`properties.row.${key}`], 'string', `en missing properties.row.${key}`);
    // The switch must map the key to that catalog entry, not fall through to raw.
    assert.equal(propertyRowLabel(t, key), zh[`properties.row.${key}`], `no label mapped for ${key}`);
    assert.equal(propertyRowLabel(tEn, key), en[`properties.row.${key}`], `no en label mapped for ${key}`);
  }
});

// --- rendered palette --------------------------------------------------

test('the properties palette renders the real status block through the pure view', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  assert.ok(source.includes('const selection = shell.status?.selection ?? null;'));
  assert.ok(source.includes('propertiesView(selection, t)'));
  assert.ok(source.includes("t('properties.panel')"));
  assert.ok(source.includes("t('properties.empty')"));
  assert.ok(source.includes("t('properties.close')"));
  assert.ok(source.includes("view.kind === 'empty'"));
  assert.ok(source.includes("view.kind === 'multi'"));
  assert.ok(source.includes('view.countLabel'));
  assert.ok(source.includes('view.mixedHelp'));
  // No fabricated rows: only the sections the view built are rendered.
  assert.ok(!source.includes('properties.clear_selection'));
  assert.ok(source.includes('section.label'));
});

test('a read-only row is a plain value, so no disabled input stands in for it', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  const readonly = source.slice(
    source.indexOf("if (row.editor === 'value')"),
    source.indexOf("if (row.editor === 'select')"),
  );
  assert.ok(readonly.includes('properties__readonly'), 'read-only rows carry the value style');
  assert.ok(readonly.includes('{row.value}'), 'read-only rows show the host value');
  assert.ok(!readonly.includes('<input'), 'no input is rendered for a read-only row');
  assert.ok(!readonly.includes('<select'), 'no select is rendered for a read-only row');
  assert.ok(!readonly.includes('disabled'), 'a read-only row is not a disabled editor');
  assert.ok(
    source.indexOf('<input') > source.indexOf("if (row.editor === 'value')"),
    'every input lives below the read-only early return',
  );
  assert.ok(
    source.indexOf('<select') > source.indexOf("if (row.editor === 'value')"),
    'every select lives below the read-only early return',
  );
});

test('editors stage a local draft and commit only on Enter, blur or select change', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  // Typing only writes the draft; the two field kinds use the same handler.
  const staged = 'onChange={event => onChange(row, event.target.value)}';
  assert.equal(source.split(staged).length - 1, 2, 'number and text fields stage only');
  // A blur and Enter commit; Escape drops the draft instead of blur-committing.
  assert.equal(source.split('onBlur={() => onCommit(row)}').length - 1, 2);
  assert.ok(source.includes("if (event.key === 'Enter')"));
  assert.ok(source.includes("if (event.key === 'Escape')"));
  assert.ok(source.includes('event.stopPropagation()'), 'Escape must not reach the global blur handler');
  // A select commits its own change, and is never left as a silent local edit.
  assert.ok(source.includes('onChange(row, value);'));
  assert.ok(source.includes('onCommit(row);'));
  // The row disables itself while its write is queued, but nothing else does.
  assert.ok(source.includes('const saving = entry.sent !== null;'));
  assert.equal(source.split('disabled={saving}').length - 1, 1, 'the select disables itself while queued');
  assert.ok(source.includes('disabled: saving,'), 'both text fields share the queued-row flag');
});

test('the palette writes through the shell, never a private IPC', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  assert.ok(source.includes('shell.applySelectedProperty(check.key, check.value)'));
  assert.ok(!source.includes('invoke('), 'the palette never calls Tauri directly');
  assert.ok(!source.includes('cad_set_selected_property'), 'the IPC name lives in properties.mjs only');
  // The text editor stays a separate mounted component in every state.
  assert.ok(source.includes('<SelectionTextEditor />'));
});

test('a refusal is catalog copy with the raw code kept in the title', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  assert.ok(source.includes('propertyEditMessage(code, t)'));
  assert.ok(source.includes('title={message.code}'));
  assert.ok(source.includes('role="alert"'));
});

test('a multi-selection says above the sections that a change applies to all objects', async () => {
  const source = await readModule('components/PropertiesPalette.jsx');
  assert.ok(source.includes('view.mixedHelp ? <p className="properties__hint">{view.mixedHelp}</p> : null'));
  // Each mixed editor repeats the same sentence as its tooltip.
  assert.ok(source.includes('const mixedTitle = row.mixed ? row.mixedHelp : null;'));
  assert.ok(source.includes('title={mixedTitle ?? undefined}'));
  // A mixed text/number row shows the same placeholder as its value until the
  // user types a real one (the select uses a placeholder option instead).
  assert.ok(source.includes('placeholder: row.mixed && entry.draft === null ? row.mixedText : undefined'));
});

test('the palette styles match the chrome tokens and keep the docked width', async () => {
  const css = await readCss();
  const block = css.slice(css.indexOf('/* Properties palette'), css.indexOf('.start-panel'));
  for (const token of ['var(--hover)', 'var(--line-soft)', 'var(--focus)', 'var(--field)']) {
    assert.ok(block.includes(token), `properties block uses ${token}`);
  }
  assert.ok(block.includes('.properties__input:focus-visible'));
  assert.ok(block.includes('.properties__select:focus-visible'));
  assert.ok(block.includes('.properties__row'));
  assert.ok(!block.includes('position: fixed'), 'the inspector stays docked, never floating over the CAD surface');
});

// --- entry points ------------------------------------------------------

test('the typed PROPERTIES selector is chrome-local and never reaches the router', async () => {
  assert.equal(isPropertiesCommand('properties'), true);
  assert.equal(isSupportedCommand('PROPERTIES'), true, 'the chrome answers it, so autocomplete may offer it');
  assert.equal(admitCommandText('PROPERTIES', { status: null, pending: true }).kind, 'admitted');
  // LAYER is untouched: still a router name, not a local selector.
  assert.equal(isPropertiesCommand('LAYER'), false);
  const calls = [];
  const result = await queueCadCommand(text => calls.push(text), 'PROPERTIES', null, false);
  assert.deepEqual(result, { kind: 'local', command: 'PROPERTIES' });
  assert.deepEqual(calls, [], 'no cad_command is sent for a chrome-local selector');
});

test('Ctrl+1 toggles the palette after the defaultPrevented guard', async () => {
  const app = await readModule('App.jsx');
  assert.ok(app.includes("event.key === '1'"));
  assert.ok(app.includes('setPropertiesOpen(value => !value)'));
  assert.ok(
    app.indexOf("event.key === '1'") > app.indexOf('event.defaultPrevented'),
    'a webview-default chord must win before Ctrl+1 is claimed',
  );
});

test('typed PROPERTIES is intercepted before the in-flight command guard', async () => {
  const app = await readModule('App.jsx');
  const intercept = app.indexOf('isPropertiesCommand(text)');
  assert.ok(intercept > 0);
  assert.ok(intercept < app.indexOf('submitting.current = true'), 'a second Enter must still open the palette');
  assert.ok(app.includes('setPropertiesOpen(true)'));
});

test('the shell exposes the one property write and the palette action', async () => {
  const app = await readModule('App.jsx');
  assert.ok(app.includes('applySelectedProperty: (key, value) => setSelectedProperty(invoke, key, value)'));
  assert.ok(app.includes("action === 'properties'"));
  assert.ok(app.includes('setPropertiesOpen(true)'));
});

test('both ribbon PROPERTIES entries dispatch the shell action, and LAYER stays a command', () => {
  const items = RIBBON_TABS.flatMap(tab => tab.panels.flatMap(panel => panel.items))
    .filter(item => item.id === 'PROPERTIES');
  assert.equal(items.length, 2, 'Home/Annotate and the large entry both exist');
  for (const item of items) {
    assert.equal(item.action, 'properties');
    assert.equal(item.command, undefined, 'the ribbon must not send cad_command for PROPERTIES');
  }
  const layers = RIBBON_TABS.flatMap(tab => tab.panels.flatMap(panel => panel.items))
    .filter(item => item.id === 'LAYER');
  for (const item of layers) {
    assert.equal(item.command, 'LAYER', 'LAYER is untouched');
    assert.equal(item.action, undefined);
  }
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
