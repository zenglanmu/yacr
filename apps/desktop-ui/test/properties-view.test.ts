// Pure view + write contracts for the docked properties palette.
//
// The palette itself is a thin state machine over these two modules, so the
// checks that matter live here: which section a row lands in, which editor it
// may offer, what a commit would send, and how a refusal is worded. Nothing in
// this suite needs a DOM, a window or the host.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createTranslator } from '../src/i18n.ts';
import {
  PROPERTY_GROUP_ORDER,
  PROPERTY_NUMBER_RANGES,
  groupPropertyRows,
  propertiesView,
  propertyEditor,
  propertyRowView,
} from '../src/properties-view.ts';
import {
  PROPERTY_ERRORS,
  PROPERTY_WRITE_COMMAND,
  isPropertiesCommand,
  propertyCommit,
  propertyEditMessage,
  propertyErrorCode,
  setSelectedProperty,
  validatePropertyValue,
} from '../src/properties.ts';
import { readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');

/** One wire row as `cad_status.selection.rows` delivers it. */
function row(overrides = {}) {
  return {
    key: 'layer',
    value: '0',
    group: 'entity',
    mixed: false,
    editable: true,
    kind: 'enum',
    options: ['0', 'WALLS'],
    ...overrides,
  };
}

function selection(rows, overrides = {}) {
  return {
    count: 1,
    empty: false,
    rows,
    mixedKeys: [],
    text: null,
    text_multiline: false,
    ...overrides,
  };
}

// --- sections -----------------------------------------------------------

test('an absent or explicitly empty selection has no sections and no count line', () => {
  for (const value of [null, undefined, selection([], { count: 0, empty: true })]) {
    const view = propertiesView(value, t);
    assert.equal(view.kind, 'empty');
    assert.equal(view.count, 0);
    assert.equal(view.countLabel, null);
    assert.equal(view.mixedHelp, null);
    assert.deepEqual(view.sections, []);
  }
});

test('sections follow the fixed entity → geometry → text → display order, not the wire order', () => {
  const rows = [
    row({ key: 'color', group: 'display' }),
    row({ key: 'length', group: 'geometry', kind: 'text' }),
    row({ key: 'id', group: 'entity', kind: 'text', editable: false }),
    row({ key: 'text', group: 'text', kind: 'text' }),
    row({ key: 'height', group: 'text', kind: 'number' }),
  ];
  const view = propertiesView(selection(rows), t);
  assert.deepEqual(view.sections.map(section => section.group), [...PROPERTY_GROUP_ORDER]);
  assert.deepEqual(view.sections.map(section => section.label), [
    zh['properties.group.entity'],
    zh['properties.group.geometry'],
    zh['properties.group.text'],
    zh['properties.group.display'],
  ]);
  // Backend row order inside a section is preserved.
  assert.deepEqual(view.sections[2].rows.map(item => item.key), ['text', 'height']);
});

test('an unknown group is appended in first-seen order instead of dropped', () => {
  const view = propertiesView(
    selection([
      row({ key: 'future_a', group: 'future' }),
      row({ key: 'id', group: 'entity', kind: 'text', editable: false }),
      row({ key: 'future_b', group: 'future' }),
      row({ key: 'other', group: 'other_group' }),
    ]),
    t,
  );
  assert.deepEqual(view.sections.map(section => section.group), ['entity', 'future', 'other_group']);
  assert.equal(view.sections[1].label, 'future');
  assert.equal(view.sections[2].label, 'other_group');
  assert.deepEqual(view.sections[1].rows.map(item => item.key), ['future_a', 'future_b']);
});

test('groupPropertyRows tolerates a missing row list without inventing a section', () => {
  assert.deepEqual(groupPropertyRows(null), []);
  assert.deepEqual(groupPropertyRows([]), []);
});

// --- editors -----------------------------------------------------------

test('a read-only row always reads as a plain value, whatever its kind', () => {
  for (const kind of ['text', 'enum', 'number', 'bool', 'future_kind']) {
    assert.equal(propertyEditor(row({ editable: false, kind })), 'value', kind);
  }
  assert.equal(propertyEditor(null), 'value');
});

test('an editable enum uses a select when the host delivered options and text otherwise', () => {
  assert.equal(propertyEditor(row({ kind: 'enum', options: ['0'] })), 'select');
  assert.equal(propertyEditor(row({ kind: 'enum', options: [] })), 'text');
  assert.equal(propertyEditor(row({ kind: 'enum', options: undefined })), 'text');
});

test('numbers get a numeric field, booleans a two-choice select, text a text field', () => {
  assert.equal(propertyEditor(row({ kind: 'number' })), 'number');
  assert.equal(propertyEditor(row({ kind: 'bool' })), 'select');
  assert.equal(propertyEditor(row({ kind: 'text' })), 'text');
  // An unknown editing kind is still editable; it falls back to the safest
  // editor (text) rather than pretending to be read-only.
  assert.equal(propertyEditor(row({ kind: 'future_kind' })), 'text');
});

test('a numeric key carries its real range; other rows carry none', () => {
  const transparency = propertyRowView(row({ key: 'transparency', kind: 'number', value: '10' }), t);
  assert.deepEqual(transparency.range, { min: 0, max: 90 });
  assert.deepEqual({ ...PROPERTY_NUMBER_RANGES.transparency }, { min: 0, max: 90 });
  assert.equal(propertyRowView(row({ key: 'height', kind: 'number', value: '12' }), t).range, null);
  assert.equal(propertyRowView(row({ key: 'color', kind: 'text', value: '#ffffff' }), t).range, null);
});

test('a row view copies delivered options instead of aliasing the wire array', () => {
  const options = ['0', 'WALLS'];
  const view = propertyRowView(row({ options }), t);
  view.options.push('EXTRA');
  assert.deepEqual(options, ['0', 'WALLS']);
});

// --- mixed -------------------------------------------------------------

test('a mixed row displays the catalog placeholder naming its own property', () => {
  const view = propertyRowView(row({ key: 'layer', mixed: true, value: null, editable: false }), t);
  assert.equal(view.mixed, true);
  assert.equal(
    view.value,
    zh['properties.mixed'].replace('{keys}', zh['properties.row.layer']),
  );
  assert.equal(view.mixedText, view.value);
  assert.equal(view.rawValue, null);
  assert.equal(view.mixedHelp, null, 'no help when no help was passed');
});

test('mixed help is present only for a multi-selection and carries the count', () => {
  const rows = [row({ key: 'layer', mixed: true, value: null }), row({ key: 'color', group: 'display', kind: 'text' })];
  const multi = propertiesView(selection(rows, { count: 4 }), t);
  assert.equal(multi.kind, 'multi');
  const expected = zh['properties.edit.mixed_help'].replace('{count}', '4');
  assert.equal(multi.mixedHelp, expected);
  for (const section of multi.sections) {
    for (const item of section.rows) {
      // The top note covers the whole palette; an editor only repeats it when
      // its own write really reaches every object.
      assert.equal(item.mixedHelp, item.mixed ? expected : null, item.key);
    }
  }
  const single = propertiesView(selection(rows), t);
  assert.equal(single.kind, 'single');
  assert.equal(single.mixedHelp, null);
  for (const section of single.sections) {
    for (const item of section.rows) assert.equal(item.mixedHelp, null, item.key);
  }
});

test('the multi-selection count line is localized and absent for one object', () => {
  const view = propertiesView(selection([row()], { count: 3 }), tEn);
  assert.equal(view.countLabel, en['properties.selected_count'].replace('{count}', '3'));
  assert.equal(propertiesView(selection([row()]), tEn).countLabel, null);
});

test('an unknown row key keeps its raw machine name in both locales', () => {
  const view = propertyRowView(row({ key: 'future_key', kind: 'text' }), t);
  assert.equal(view.label, 'future_key');
  assert.equal(propertyRowView(row({ key: 'future_key', kind: 'text' }), tEn).label, 'future_key');
});

// --- value grammar mirror ----------------------------------------------

test('colour grammar accepts ByLayer/ByBlock, six-digit hex and aci:0..256', () => {
  for (const value of ['ByLayer', 'byblock', 'BYLAYER', '#ffffff', '#A0b1C2', 'aci:0', 'aci:256', 'ACI:7']) {
    assert.equal(validatePropertyValue('color', value), null, value);
  }
  for (const value of ['', 'ByLayerX', '#fff', '#gggggg', 'aci:257', 'aci:-1', 'aci:1.5', 'red', ' 1']) {
    assert.equal(validatePropertyValue('color', value), PROPERTY_ERRORS.invalidValue, value);
  }
});

test('layer and linetype names must be one of the delivered options when there are any', () => {
  const options = ['0', 'WALLS'];
  assert.equal(validatePropertyValue('layer', 'WALLS', options), null);
  assert.equal(validatePropertyValue('layer', 'walls', options), PROPERTY_ERRORS.unknownLayer);
  assert.equal(validatePropertyValue('layer', 'WALLS', []), null, 'no table means the host decides');
  assert.equal(validatePropertyValue('linetype', 'DASHED', ['DASHED']), null);
  assert.equal(validatePropertyValue('linetype', 'DASHED', ['SOLID']), PROPERTY_ERRORS.unknownLinetype);
  assert.equal(validatePropertyValue('linetype', 'ByLayer', ['SOLID']), null);
  assert.equal(validatePropertyValue('linetype', 'ByBlock', []), null);
});

test('lineweight accepts its keywords, delivered options or a millimetre shape', () => {
  for (const value of ['ByLayer', 'ByBlock', 'Default', '0.25', '2']) {
    assert.equal(validatePropertyValue('lineweight', value, []), null, value);
  }
  assert.equal(validatePropertyValue('lineweight', 'Heavy', []), PROPERTY_ERRORS.invalidValue);
  assert.equal(validatePropertyValue('lineweight', 'Heavy', ['Heavy']), null);
  assert.equal(validatePropertyValue('lineweight', 'Heavy', ['Thin']), PROPERTY_ERRORS.invalidValue);
});

test('transparency is digits only inside 0..=90 and is rejected, never clamped', () => {
  for (const value of ['0', '45', '90']) {
    assert.equal(validatePropertyValue('transparency', value), null, value);
  }
  for (const value of ['91', '100', '-1', '1.5', ' 5', '5%', '']) {
    assert.equal(validatePropertyValue('transparency', value), PROPERTY_ERRORS.invalidValue, value);
  }
});

test('a key without a client grammar is left to the host, but no key accepts an empty string', () => {
  assert.equal(validatePropertyValue('height', 'not-a-number'), null);
  assert.equal(validatePropertyValue('future_key', 'anything'), null);
  assert.equal(validatePropertyValue('future_key', ''), PROPERTY_ERRORS.invalidValue);
});

// --- commit decision ---------------------------------------------------

test('a read-only or untouched row never queues a write', () => {
  assert.deepEqual(propertyCommit(null, 'x'), { kind: 'skip', reason: 'read_only' });
  assert.deepEqual(propertyCommit(row({ editable: false }), 'x'), { kind: 'skip', reason: 'read_only' });
  assert.deepEqual(propertyCommit(row(), null), { kind: 'skip', reason: 'untouched' });
  assert.deepEqual(propertyCommit(row(), undefined), { kind: 'skip', reason: 'untouched' });
});

test('a single value equal to the host value is skipped, a mixed one is written', () => {
  const single = propertyRowView(row({ value: '0' }), t);
  assert.deepEqual(propertyCommit(single, '0'), { kind: 'skip', reason: 'unchanged' });
  const mixed = propertyRowView(row({ mixed: true, value: null }), t);
  assert.deepEqual(propertyCommit(mixed, '0'), { kind: 'send', key: 'layer', value: '0' });
});

test('a changed value is sent verbatim, with no trimming or normalising', () => {
  const layer = propertyRowView(row(), t);
  assert.deepEqual(propertyCommit(layer, 'WALLS'), { kind: 'send', key: 'layer', value: 'WALLS' });
  const color = propertyRowView(row({ key: 'color', kind: 'text', value: 'aci:1' }), t);
  assert.deepEqual(propertyCommit(color, 'ACI:7'), { kind: 'send', key: 'color', value: 'ACI:7' });
  const height = propertyRowView(row({ key: 'height', kind: 'number', value: '1' }), t);
  assert.deepEqual(propertyCommit(height, '2.50'), { kind: 'send', key: 'height', value: '2.50' });
  // Padding is not silently trimmed: the grammar rejects it, so the user sees
  // the refusal instead of a value they did not type.
  assert.deepEqual(propertyCommit(color, ' #ff0000'), { kind: 'invalid', code: PROPERTY_ERRORS.invalidValue });
});

test('an emptied field is a real draft and is refused, not ignored', () => {
  assert.deepEqual(propertyCommit(row(), ''), { kind: 'invalid', code: PROPERTY_ERRORS.invalidValue });
});

test('a draft with the wrong grammar is refused before any IPC is queued', () => {
  assert.deepEqual(
    propertyCommit(row({ key: 'transparency', kind: 'number', value: '0' }), '91'),
    { kind: 'invalid', code: PROPERTY_ERRORS.invalidValue },
  );
  assert.deepEqual(
    propertyCommit(row(), 'MISSING'),
    { kind: 'invalid', code: PROPERTY_ERRORS.unknownLayer },
  );
});

test('a non-string draft is refused rather than coerced', () => {
  assert.deepEqual(propertyCommit(row(), 1), { kind: 'invalid', code: PROPERTY_ERRORS.invalidValue });
});

// --- refusal vocabulary ------------------------------------------------

test('propertyErrorCode extracts the stable code from an IPC message', () => {
  assert.equal(propertyErrorCode(Error('property.unknown_layer')), 'property.unknown_layer');
  assert.equal(
    propertyErrorCode('property write refused: property.invalid_value (selection 3)'),
    'property.invalid_value',
  );
  assert.equal(propertyErrorCode('popup.modal_open'), 'popup.modal_open');
  assert.equal(propertyErrorCode('something else'), 'something else');
  assert.equal(propertyErrorCode(null), PROPERTY_ERRORS.writeFailed);
  assert.equal(propertyErrorCode(''), PROPERTY_ERRORS.writeFailed);
});

test('every known refusal maps to catalog copy and keeps the raw code for the title', () => {
  for (const code of Object.values(PROPERTY_ERRORS)) {
    const suffix = code.replace(/^(?:property|popup)\./, '');
    const key = `properties.edit.${suffix}`;
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    const zhMessage = propertyEditMessage(code, t);
    assert.equal(zhMessage.code, code);
    assert.equal(zhMessage.text, zh[key]);
    assert.equal(propertyEditMessage(code, tEn).text, en[key]);
  }
  assert.ok(!zh['properties.edit.failed'].includes('{code}'), 'the visible sentence never shows the raw code');
});

test('an unknown or host-only code falls back to the generic refusal with the code kept aside', () => {
  for (const code of ['property.some_future', 'popup.no_text_selection', 'not-a-code', '']) {
    const message = propertyEditMessage(code, t);
    assert.equal(message.code, code);
    assert.equal(message.text, zh['properties.edit.failed']);
  }
  assert.equal(propertyEditMessage(null, t).text, zh['properties.edit.failed']);
});

test('the two catalogs carry the same properties.edit.* and properties.value.* keys', () => {
  const keys = catalog => Object.keys(catalog)
    .filter(key => key.startsWith('properties.edit.') || key.startsWith('properties.value.'))
    .sort();
  assert.deepEqual(keys(zh), keys(en));
  assert.ok(keys(zh).length >= 18);
  for (const key of keys(zh)) {
    assert.ok(zh[key].trim().length > 0, `empty zh-CN copy at ${key}`);
    assert.ok(en[key].trim().length > 0, `empty en copy at ${key}`);
  }
});

// --- IPC ---------------------------------------------------------------

test('setSelectedProperty queues exactly the two-string payload on the one IPC', async () => {
  const calls = [];
  const invoke = async (command, payload) => {
    calls.push([command, payload]);
    return { accepted: true };
  };
  assert.deepEqual(await setSelectedProperty(invoke, 'layer', 'WALLS'), { kind: 'accepted' });
  assert.deepEqual(calls, [[PROPERTY_WRITE_COMMAND, { key: 'layer', value: 'WALLS' }]]);
  assert.equal(PROPERTY_WRITE_COMMAND, 'cad_set_selected_property');
});

test('a non-accepted answer or a thrown IPC is an explicit refusal', async () => {
  assert.deepEqual(
    await setSelectedProperty(async () => ({ accepted: false }), 'layer', '0'),
    { kind: 'refused', code: PROPERTY_ERRORS.invalidValue },
  );
  assert.deepEqual(
    await setSelectedProperty(async () => null, 'layer', '0'),
    { kind: 'refused', code: PROPERTY_ERRORS.invalidValue },
  );
  assert.deepEqual(
    await setSelectedProperty(async () => { throw Error('property.unknown_layer'); }, 'layer', 'x'),
    { kind: 'refused', code: PROPERTY_ERRORS.unknownLayer },
  );
  assert.deepEqual(
    await setSelectedProperty(async () => { throw Error('popup.modal_open'); }, 'layer', '0'),
    { kind: 'refused', code: 'popup.modal_open' },
  );
});

test('isPropertiesCommand recognizes the typed selector in any case and refuses lookalikes', () => {
  for (const text of ['PROPERTIES', 'properties', ' Properties ', 'propERTies']) {
    assert.equal(isPropertiesCommand(text), true, text);
  }
  for (const text of ['PROPERTY', 'PROPERTIES 1', 'PROPS', '', null, undefined, 0]) {
    assert.equal(isPropertiesCommand(text), false, String(text));
  }
});
