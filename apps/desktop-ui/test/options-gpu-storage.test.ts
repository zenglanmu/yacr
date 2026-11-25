// The two GPU rows: the memory budget (a generic `number`, 10–80) and the
// accelerated-GPU-storage opt-out (a generic `bool`, default on) that gates it.
//
// Split out of `options.test.ts` so both files stay inside the code-file line
// limit. The dialog itself has no per-field branch: the shared number/boolean
// editors draw both rows and the shared visibility rule hides the budget while
// the accelerated copy is off.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applyPayload,
  dirtyFields,
  draftFromSettings,
  fieldById,
  fieldsForTab,
  fieldVisible,
  validateDraft,
} from '../src/options/options.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('the GPU budget is a generic numeric row, never a bespoke control', async () => {
  const field = fieldById('gpuBudgetPercent');
  assert.equal(field.kind, 'number');
  assert.equal(field.tab, 'display', 'it sits with the other graphics settings');
  assert.equal(field.wire, 'gpuBudgetPercent');
  assert.equal(field.key, 'gpuBudgetPercent');
  assert.equal(field.min, 10);
  assert.equal(field.max, 80);
  assert.equal(field.unitKey, 'options.unit.percent');
  assert.equal(field.labelKey, 'options.field.gpuBudgetPercent');
  assert.equal(field.helpKey, 'options.help.gpuBudgetPercent');
  assert.ok(fieldsForTab('display').includes(field));
  // The dialog renders every number field through the same row/editor path, so
  // this control needs (and has) no per-field branch of its own.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('gpuBudgetPercent'), 'no special case for this field');
  assert.ok(dialog.includes("case 'number':"));
  assert.ok(dialog.includes('t(field.unitKey)'));
  // Both catalogs carry the copy the descriptor points at.
  assert.equal(zh[field.labelKey], 'GPU 内存预算');
  assert.equal(en[field.labelKey], 'GPU memory budget');
  assert.ok(zh[field.helpKey].trim().length > 0);
  assert.ok(en[field.helpKey].trim().length > 0);
  // The help says it is a share of the card's memory and a budget, not the total.
  assert.match(en[field.helpKey], /graphics card's memory/i);
  assert.match(en[field.helpKey], /budget/i);
  assert.match(en[field.helpKey], /not the card's total/i);
  assert.match(en[field.helpKey], /10–80/);
});

test('the GPU storage switch is a generic boolean row that gates the budget row', async () => {
  const field = fieldById('gpuStorageDisabled');
  assert.equal(field.kind, 'bool');
  assert.equal(field.tab, 'display');
  assert.equal(field.wire, 'gpuStorageDisabled');
  assert.equal(field.key, 'gpuStorageDisabled');
  assert.equal(field.labelKey, 'options.field.gpuStorageDisabled');
  assert.equal(field.helpKey, 'options.help.gpuStorageDisabled');
  assert.ok(fieldsForTab('display').includes(field));
  assert.equal(zh[field.labelKey], '禁用 GPU 存储');
  assert.equal(en[field.labelKey], 'Disable GPU storage');
  assert.match(en[field.helpKey], /next time a window opens/i);

  // The dialog has no per-field branch: the shared boolean editor draws it and
  // the shared visibility rule (in the model) hides the budget row.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('gpuStorageDisabled'), 'no special case for this field');
  assert.ok(dialog.includes('fieldVisible('), 'rows are filtered by the model rule');

  const budget = fieldById('gpuBudgetPercent');
  // Enabled storage shows the budget; disabled (the host default) hides it; a
  // host that never reported the switch keeps the row visible.
  assert.equal(fieldVisible(budget, { gpuStorageDisabled: false }), true);
  assert.equal(fieldVisible(budget, { gpuStorageDisabled: true }), false);
  assert.equal(fieldVisible(budget, {}), true);
  assert.equal(fieldVisible(field, { gpuStorageDisabled: true }), true, 'the switch itself always renders');

  // Hidden means not staged: a budget edit made before the switch was turned on
  // is neither dirty nor sent, so Apply cannot write a value nobody can see.
  const wire = { gpuStorageDisabled: true, gpuBudgetPercent: 80 };
  const draft = { ...draftFromSettings(wire, 'en'), gpuBudgetPercent: '25' };
  assert.deepEqual(dirtyFields(draft, wire, 'en'), []);
  assert.deepEqual(applyPayload(draft, wire), []);
  assert.deepEqual(validateDraft({ gpuStorageDisabled: true, gpuBudgetPercent: '101' }), {});
  // Once storage is enabled the same draft is staged again, switch first.
  const enabled = { ...draft, gpuStorageDisabled: false };
  assert.deepEqual(dirtyFields(enabled, wire, 'en'), ['gpuStorageDisabled', 'gpuBudgetPercent']);
});
