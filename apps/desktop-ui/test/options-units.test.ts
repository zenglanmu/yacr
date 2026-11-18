// The units / recovery rows of the Options table (ADR-0037 D9/D12).
//
// Split out of `options.test.ts` so both files stay inside the file-size gate;
// what is pinned here is the `files` tab block itself, the cross-field rule the
// host also enforces, and the copy both catalogs must carry.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createTranslator } from '../src/i18n.ts';
import {
  SETTINGS_ERROR,
  UNIT_TOKENS,
  applyPayload,
  draftFromSettings,
  fieldById,
  fieldsForTab,
  validateDraft,
} from '../src/options/options.ts';
import { readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');

function snapshot(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    drawingUnit: 'drawing',
    displayUnit: 'drawing',
    unitPrecision: 3,
    recoveryInterval: 60,
    ...overrides,
  };
}

test('the files tab holds one units block and the recovery interval', () => {
  // Declaration order is the rendering order: the four rows read as one block
  // under the tab heading, with the read-only fonts folder last.
  assert.deepEqual(
    fieldsForTab('files').map(field => [field.id, field.kind, field.wire, field.key]),
    [
      ['drawingUnit', 'select', 'drawingUnit', 'drawingUnit'],
      ['displayUnit', 'select', 'displayUnit', 'displayUnit'],
      ['unitPrecision', 'number', 'unitPrecision', 'unitPrecision'],
      ['recoveryInterval', 'number', 'recoveryInterval', 'recoveryInterval'],
      ['fontsDir', 'readonly', 'fontsDir', undefined],
    ],
  );
  // Both unit selects carry the schema's closed lowercase token set, default
  // first, and their own copy (the wire spelling is never shown raw).
  for (const id of ['drawingUnit', 'displayUnit']) {
    const field = fieldById(id);
    assert.deepEqual(field.choices, ['drawing', 'mm', 'cm', 'm', 'in', 'ft']);
    assert.deepEqual(field.choices, UNIT_TOKENS);
    assert.equal(field.choiceKeyPrefix, 'options.unitToken');
    assert.equal(field.tab, 'files');
  }
  assert.deepEqual(UNIT_TOKENS, ['drawing', 'mm', 'cm', 'm', 'in', 'ft']);
  // The numeric rows carry the host's own bounds and a unit suffix.
  assert.equal(fieldById('unitPrecision').min, 0);
  assert.equal(fieldById('unitPrecision').max, 8);
  assert.equal(fieldById('unitPrecision').unitKey, 'options.unit.decimal_places');
  assert.equal(fieldById('recoveryInterval').min, 0);
  assert.equal(fieldById('recoveryInterval').max, 3600);
  assert.equal(fieldById('recoveryInterval').unitKey, 'options.unit.seconds');
});

test('a typed display over untyped coordinates is refused before Apply', () => {
  // The one cross-field rule: the host refuses it too, naming
  // `ui.units.displayUnit`.
  assert.equal(validateDraft({ drawingUnit: 'mm', displayUnit: 'cm' }).displayUnit, undefined);
  assert.equal(
    validateDraft({ drawingUnit: 'drawing', displayUnit: 'drawing' }).displayUnit,
    undefined,
  );
  assert.equal(
    validateDraft({ drawingUnit: 'drawing', displayUnit: 'm' }).displayUnit,
    SETTINGS_ERROR.invalidValue,
  );
});

test('the units block seeds, diffs and writes under the shared camelCase keys', () => {
  const host = snapshot();
  const draft = draftFromSettings(host);
  assert.equal(draft.drawingUnit, 'drawing');
  assert.equal(draft.unitPrecision, '3');
  assert.equal(draft.recoveryInterval, '60');
  draft.drawingUnit = 'mm';
  draft.displayUnit = 'cm';
  draft.unitPrecision = '2';
  draft.recoveryInterval = '0';
  assert.deepEqual(
    applyPayload(draft, host).map(entry => [entry.key, entry.value]),
    [
      ['drawingUnit', 'mm'],
      ['displayUnit', 'cm'],
      ['unitPrecision', 2],
      ['recoveryInterval', 0],
    ],
  );
  // An out-of-range value is refused, never clamped.
  assert.equal(validateDraft({ unitPrecision: '9' }).unitPrecision, SETTINGS_ERROR.invalidValue);
  assert.equal(
    validateDraft({ recoveryInterval: '3601' }).recoveryInterval,
    SETTINGS_ERROR.invalidValue,
  );
});

test('the files tab and every unit token have real copy in both catalogs', () => {
  // The tab now holds units/recovery as well as the fonts folder, and its label
  // says so.
  assert.equal(zh['options.tab.files'], '单位与文件');
  assert.equal(en['options.tab.files'], 'Units & Files');
  for (const choice of UNIT_TOKENS) {
    const key = `options.unitToken.${choice}`;
    assert.notEqual(tEn(key), key, 'the raw key must never be rendered');
    assert.ok(zh[key].trim().length > 0, `zh-CN is missing ${key}`);
  }
  for (const field of fieldsForTab('files')) {
    for (const key of [field.labelKey, field.helpKey, 'unitKey' in field ? field.unitKey : null]) {
      if (!key) continue;
      assert.ok(en[key].trim().length > 0, `en is missing ${key}`);
      assert.ok(zh[key].trim().length > 0, `zh-CN is missing ${key}`);
    }
  }
});
