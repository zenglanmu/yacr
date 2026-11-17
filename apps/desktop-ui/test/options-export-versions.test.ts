// The two Save-As export-version rows: the DXF container version and the ACIS
// payload version, staged through the same `cad_set_setting` path as every
// other control.
//
// They get their own file because the contract is narrow and easy to get wrong
// in a way the generic table test would not catch: both are **save-time**
// choices (the host persists the token and the exporter reads it when it builds
// the file), both accept a closed token list with no clamping, and both must
// read with the copy the export lane already published
// (`options.setting.{dxf,acis}_version.{label,help}`) rather than a second label.
//
// The wire objects below are deliberately minimal: `draftFromSettings`,
// `dirtyFields`, `applyPayload` and `entryVisible` only touch the rows' own
// `wire` keys, so a two-field snapshot is the honest fixture here.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ACIS_VERSIONS,
  DXF_VERSIONS,
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

test('the export version selectors are save-time rows, written through cad_set_setting', async () => {
  const dialog = await readModule('components/OptionsDialog.tsx');
  // Both rows live on the same page as the plot format, container before payload,
  // and after the existing row (this change adds rows, it does not reorder the page).
  const plot = fieldsForTab('plot').map(field => field.id);
  assert.equal(plot.indexOf('dxfVersion'), plot.indexOf('plotFormat') + 1);
  assert.equal(plot.indexOf('acisVersion'), plot.indexOf('dxfVersion') + 1);

  const dxf = fieldById('dxfVersion');
  assert.equal(dxf.kind, 'select');
  assert.equal(dxf.tab, 'plot');
  // The snapshot field, the write key and the persisted leaf share one spelling.
  assert.equal(dxf.wire, 'dxfVersion');
  assert.equal(dxf.key, 'dxfVersion');
  // The host's closed token list, default first; no clamping to the default.
  assert.deepEqual(dxf.choices, ['2010', '2013', '2018']);
  assert.deepEqual(DXF_VERSIONS, ['2010', '2013', '2018']);
  assert.equal(dxf.labelKey, 'options.setting.dxf_version.label');
  assert.equal(dxf.helpKey, 'options.setting.dxf_version.help');
  // A token outside the list is the host's generic invalid value, not the plot
  // format refusal.
  assert.equal(dxf.errorCode, undefined);

  const acis = fieldById('acisVersion');
  assert.equal(acis.kind, 'select');
  assert.equal(acis.tab, 'plot');
  assert.equal(acis.wire, 'acisVersion');
  assert.equal(acis.key, 'acisVersion');
  assert.deepEqual(acis.choices, ['7.0', '21.0']);
  assert.deepEqual(ACIS_VERSIONS, ['7.0', '21.0']);
  assert.equal(acis.labelKey, 'options.setting.acis_version.label');
  assert.equal(acis.helpKey, 'options.setting.acis_version.help');
  assert.equal(acis.errorCode, undefined);

  // Every listed token is accepted and every neighbour is refused by name, so a
  // hand-edited value can never be folded onto the default.
  for (const value of DXF_VERSIONS) assert.equal(validateField('dxfVersion', value), null, value);
  for (const value of ACIS_VERSIONS) assert.equal(validateField('acisVersion', value), null, value);
  for (const value of ['2007', 'r2018', '2018.0', '', 2018]) {
    assert.equal(validateField('dxfVersion', value), SETTINGS_ERROR.invalidValue, String(value));
  }
  for (const value of ['2016', '7', '21', '', 21]) {
    assert.equal(validateField('acisVersion', value), SETTINGS_ERROR.invalidValue, String(value));
  }

  // A host that does not report them leaves the rows absent, never dirty and
  // never written: the dialog does not invent the `2010` / `7.0` defaults.
  assert.equal(draftFromSettings({}).dxfVersion, null);
  assert.equal(draftFromSettings({}).acisVersion, null);
  assert.deepEqual(dirtyFields(draftFromSettings({}), {}), []);
  assert.deepEqual(applyPayload(draftFromSettings({}), {}), []);

  // The same snapshot value is not a change, in both directions.
  const wire = { dxfVersion: '2010', acisVersion: '7.0' };
  assert.deepEqual(dirtyFields(draftFromSettings(wire), wire), []);
  assert.deepEqual(applyPayload(draftFromSettings(wire), wire), []);

  // A staged change writes exactly one typed string per row, in table order
  // (container before payload), through the same `cad_set_setting` queue.
  const draft = { ...draftFromSettings(wire), dxfVersion: '2018', acisVersion: '21.0' };
  assert.deepEqual(applyPayload(draft, wire), [
    { field: 'dxfVersion', key: 'dxfVersion', value: '2018' },
    { field: 'acisVersion', key: 'acisVersion', value: '21.0' },
  ]);
  const granted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(granted.invoke, applyPayload(draft, wire)), { kind: 'accepted' });
  assert.deepEqual(granted.calls, [
    { command: 'cad_set_setting', args: { key: 'dxfVersion', value: '2018' } },
    { command: 'cad_set_setting', args: { key: 'acisVersion', value: '21.0' } },
  ]);
  // A refusal keeps the host's stable code and names the row it came from.
  const refused = recordingInvoke(() => { throw 'settings.invalid_value: dxfVersion expects 2010, 2013 or 2018'; });
  assert.deepEqual(await writeAll(refused.invoke, applyPayload(draft, wire)), {
    kind: 'failed', field: 'dxfVersion', key: 'dxfVersion', code: 'settings.invalid_value',
  });
  // The re-read check recognises the written token, so settle cannot claim a
  // version the host does not have.
  const entry = { field: 'dxfVersion', key: 'dxfVersion', value: '2018' };
  assert.equal(entryVisible({ dxfVersion: '2018' }, entry), true);
  assert.equal(entryVisible({ dxfVersion: '2010' }, entry), false);

  // The copy is the export lane's already published pair: the visible text says
  // it applies to a save, so the row cannot read as "convert the open drawing".
  assert.equal(zh[dxf.labelKey], 'DXF 版本');
  assert.equal(en[dxf.labelKey], 'DXF version');
  assert.match(zh[dxf.helpKey], /保存 DXF/);
  assert.match(en[dxf.helpKey], /saved DXF/);
  assert.equal(zh[acis.labelKey], 'ACIS 载荷版本');
  assert.equal(en[acis.labelKey], 'ACIS payload version');
  assert.match(zh[acis.helpKey], /保存 3DSOLID/);
  assert.match(en[acis.helpKey], /saved 3DSOLID/);

  // Rendered by the shared select control: no per-field branch in the dialog,
  // and a bare token select keeps its own spelling (a version number needs no
  // catalog key).
  assert.ok(!dialog.includes('dxfVersion'), 'no special case for this field');
  assert.ok(!dialog.includes('acisVersion'), 'no special case for this field');
  assert.ok(dialog.includes("case 'select':"));
  assert.ok(dialog.includes(": choice => choice.toUpperCase();"));
});
