// The 3D orbit sensitivity row: a **real** percentage of the host's base drag
// rate, staged through the same `cad_set_setting` path as every other control.
//
// It gets its own file because the contract is narrow and easy to "simplify" the
// wrong way: the host's rule is a closed **f64** interval (`25.0..=200.0`, no
// integer requirement), so a hand-edited `60.5` is a legal setting that must
// seed as an editable value. With the integer editor it would read as "not
// reported" and the real host value could never be edited or overwritten.
//
// The wire object below is deliberately minimal: `draftFromSettings`,
// `dirtyFields`, `applyPayload` and `entryVisible` only touch the row's own
// `wire` key, so a one-field snapshot is the honest fixture here.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SETTINGS_ERROR,
  applyPayload,
  dirtyFields,
  draftFromSettings,
  entryVisible,
  fieldById,
  fieldsForTab,
  validateField,
  writeAll,
} from '../src/options/options.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

/** Record every IPC the dialog makes, in order. */
function recordingInvoke(handler) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push(args === undefined ? { command } : { command, args });
    const answer = typeof handler === 'function' ? handler(command, args, calls.length) : handler;
    if (answer instanceof Error) throw answer;
    return answer;
  };
  return { invoke, calls };
}

test('the orbit sensitivity is a generic real percentage row, written through cad_set_setting', async () => {
  const field = fieldById('orbitSensitivity');
  // A `real`, not a `number`: the host's rule is a closed f64 interval, so a
  // fractional value such as 60.5 must seed and stay editable.
  assert.equal(field.kind, 'real');
  assert.equal(field.tab, 'drafting');
  // It sits directly after the 2D input scheme: both are drag-behaviour rows.
  const drafting = fieldsForTab('drafting').map(entry => entry.id);
  assert.equal(drafting.indexOf('orbitSensitivity'), drafting.indexOf('leftDragPans') + 1);
  // The snapshot field and the write key are the same camelCase spelling; the
  // control's own id is the shorter label-facing name.
  assert.equal(field.wire, 'orbitSensitivityPercent');
  assert.equal(field.key, 'orbitSensitivityPercent');
  // The host's range: 25 % (a quarter of the base rate) to 200 % (twice it).
  assert.equal(field.min, 25);
  assert.equal(field.max, 200);
  assert.equal(field.unitKey, 'options.unit.percent');
  assert.equal(field.labelKey, 'options.field.orbitSensitivity');
  assert.equal(field.helpKey, 'options.help.orbitSensitivity');

  // A fractional host value is a legal setting: it seeds as editable text, is
  // valid, and is not dirty against the same snapshot.
  const wire = { orbitSensitivityPercent: 60.5 };
  assert.equal(draftFromSettings(wire).orbitSensitivity, '60.5');
  assert.deepEqual(dirtyFields(draftFromSettings(wire), wire), []);
  // The bounds are the host's closed f64 interval: a fraction is legal, and
  // nothing outside 25..=200 is accepted (never clamped).
  for (const value of ['25', '60.5', '70.5', '200']) {
    assert.equal(validateField('orbitSensitivity', value), null, `${value} must be legal`);
  }
  for (const value of ['24', '24.9', '200.1', '201', '0', '', 'abc']) {
    assert.equal(validateField('orbitSensitivity', value), SETTINGS_ERROR.invalidValue, `${value} must be refused`);
  }
  // A host that does not report it stays absent and is never dirty or written.
  assert.equal(draftFromSettings({}).orbitSensitivity, null);
  assert.deepEqual(dirtyFields(draftFromSettings({}), {}), []);
  assert.deepEqual(applyPayload(draftFromSettings({}), {}), []);

  // Changing it stages exactly one typed real write (a JSON number, never a
  // string the host would read as text and refuse).
  const draft = { ...draftFromSettings(wire), orbitSensitivity: '70.5' };
  assert.deepEqual(applyPayload(draft, wire), [
    { field: 'orbitSensitivity', key: 'orbitSensitivityPercent', value: 70.5 },
  ]);
  const granted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(granted.invoke, applyPayload(draft, wire)), { kind: 'accepted' });
  assert.deepEqual(granted.calls, [
    { command: 'cad_set_setting', args: { key: 'orbitSensitivityPercent', value: 70.5 } },
  ]);
  // A refusal keeps the host's code and names the field it came from.
  const refused = recordingInvoke(() => { throw 'settings.invalid_value: orbit sensitivity must be 25 to 200'; });
  assert.deepEqual(await writeAll(refused.invoke, applyPayload(draft, wire)), {
    kind: 'failed', field: 'orbitSensitivity', key: 'orbitSensitivityPercent', code: 'settings.invalid_value',
  });
  // The re-read check recognises the written real, so a fraction cannot look lost.
  const entry = { field: 'orbitSensitivity', key: 'orbitSensitivityPercent', value: 70.5 };
  assert.equal(entryVisible({ orbitSensitivityPercent: 70.5 }, entry), true);
  assert.equal(entryVisible({ orbitSensitivityPercent: 60.5 }, entry), false);

  // Both catalogs carry the row's copy, and the help says what the value means.
  assert.equal(zh[field.labelKey], '三维旋转灵敏度');
  assert.equal(en[field.labelKey], '3D rotation sensitivity');
  assert.equal(zh[field.unitKey], '%');
  assert.equal(en[field.unitKey], '%');
  assert.match(zh[field.helpKey], /100 为默认/);
  assert.match(en[field.helpKey], /100 is the default/i);
  // One setting for mouse and touch, and the help says so in both languages.
  assert.match(zh[field.helpKey], /鼠标与触控/);
  assert.match(en[field.helpKey], /mouse and touch/i);

  // Rendered by the shared real (decimal) editor, not a bespoke control: the
  // same row, unit suffix and `step="any"` the snap spacing uses.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('orbitSensitivity'), 'no special case for this field');
  assert.ok(dialog.includes("case 'real':"), 'the decimal editor renders this row');
  assert.ok(dialog.includes("step={decimal ? 'any' : undefined}"));
  assert.ok(dialog.includes('t(field.unitKey)'));
});
