// The anti-aliasing (MSAA) row: a plain `select` over the host's closed token
// set (`"off" | "2x" | "4x"`), staged through the same `cad_set_setting` path as
// every other control.
//
// It gets its own file because the row carries two easy-to-"fix" mistakes:
//   * the UI must never invent the host's default (`4x` is what the host
//     reports; a missing snapshot field stays `null` and is never written), and
//   * the row must never gate or disable itself per carrier — the renderer
//     downgrades an unsupported level and says so in diagnostics, so all three
//     tokens are always offered and always writable.
//
// The wire object below is deliberately minimal: `draftFromSettings`,
// `dirtyFields`, `applyPayload` and `entryVisible` only touch the row's own
// `wire` key, so a one-field snapshot is the honest fixture here.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  MSAA_MODES,
  SETTINGS_ERROR,
  applyPayload,
  dirtyFields,
  draftFromSettings,
  fieldById,
  fieldsForTab,
  validateField,
} from '../src/options/options.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('the MSAA level is a generic select row, written through cad_set_setting', async () => {
  const field = fieldById('msaaMode');
  assert.ok(field, 'the msaaMode row exists');
  // A `select`, not a bespoke toggle: the three levels are one closed token set.
  assert.equal(field.kind, 'select');
  assert.equal(field.tab, 'display', 'it sits with the other graphics settings');
  // The snapshot field and the write key are the same camelCase spelling.
  assert.equal(field.wire, 'msaaMode');
  assert.equal(field.key, 'msaaMode');
  // The contract order is exactly `off`, `2x`, `4x`; the default is the host's
  // (`4x`) and is reported by the host, never baked into the list order here.
  assert.deepEqual(field.choices, ['off', '2x', '4x']);
  assert.deepEqual(MSAA_MODES, ['off', '2x', '4x']);
  assert.equal(field.choiceKeyPrefix, 'options.msaaMode');
  assert.equal(field.labelKey, 'options.field.msaaMode');
  assert.equal(field.helpKey, 'options.help.msaaMode');

  // It sits immediately after the GPU budget: both are graphics-cost rows.
  const display = fieldsForTab('display').map(entry => entry.id);
  assert.equal(display.indexOf('msaaMode'), display.indexOf('gpuBudgetPercent') + 1);

  // The host's reported level seeds the draft as an editable, clean value.
  const wire = { msaaMode: '2x' };
  assert.equal(draftFromSettings(wire).msaaMode, '2x');
  assert.deepEqual(dirtyFields(draftFromSettings(wire), wire), []);
  assert.deepEqual(applyPayload(draftFromSettings(wire), wire), []);

  // Exactly the three tokens are legal; everything else is the generic refusal.
  for (const value of ['4x', '2x', 'off']) {
    assert.equal(validateField('msaaMode', value), null, `${value} must be legal`);
  }
  for (const value of ['8x', '', '4', 'OFF', 'msaa']) {
    assert.equal(validateField('msaaMode', value), SETTINGS_ERROR.invalidValue, `${value} must be refused`);
  }

  // Changing it stages exactly one string write under the same key.
  const draft = { ...draftFromSettings({ msaaMode: '4x' }), msaaMode: '2x' };
  assert.deepEqual(applyPayload(draft, { msaaMode: '4x' }), [
    { field: 'msaaMode', key: 'msaaMode', value: '2x' },
  ]);

  // A host that does not report it stays absent: nothing is invented, and
  // nothing is dirty or written (the default is the host's to report).
  const absent = draftFromSettings({});
  assert.equal(absent.msaaMode, null);
  assert.deepEqual(dirtyFields(absent, {}), []);
  assert.deepEqual(applyPayload(absent, {}), []);
  // A value outside the token set is not seeded either: it reads as unreported
  // instead of being folded onto a default.
  assert.equal(draftFromSettings({ msaaMode: '8x' }).msaaMode, null);
  assert.equal(draftFromSettings({ msaaMode: 4 }).msaaMode, null);

  // Both catalogs carry the row's copy and every choice label.
  assert.equal(zh[field.labelKey], '抗锯齿 (MSAA)');
  assert.equal(en[field.labelKey], 'Anti-aliasing (MSAA)');
  assert.equal(
    zh[field.helpKey],
    '三维模型视图使用多重采样抗锯齿。关闭可降低显存与 GPU 占用；设备不支持所选档位时会自动降级并在诊断中标注。',
  );
  assert.equal(
    en[field.helpKey],
    'Use multisample anti-aliasing for 3D model views. Off reduces VRAM and GPU cost; a level the device cannot support is downgraded and reported in diagnostics.',
  );
  const choices = { off: ['关闭', 'Off'], '2x': ['2x', '2x'], '4x': ['4x', '4x'] };
  for (const [choice, [zhLabel, enLabel]] of Object.entries(choices)) {
    const key = `${field.choiceKeyPrefix}.${choice}`;
    assert.equal(zh[key], zhLabel, `zh ${key}`);
    assert.equal(en[key], enLabel, `en ${key}`);
  }

  // Rendered by the shared select editor: no per-field branch in the dialog and
  // no carrier gate/disable on this row.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('msaaMode'), 'no special case for this field');
  assert.ok(dialog.includes("case 'select':"), 'the generic select editor renders this row');
  assert.ok(dialog.includes('field.choiceKeyPrefix'));
});
