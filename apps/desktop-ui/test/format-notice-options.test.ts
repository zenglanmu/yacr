// The Options row for the open-time format notice (ADR-0041 D5).
//
// The card's own contract is `format-provenance-notice.test.ts`; this suite pins
// the one control that owns the same preference from the other side:
//
//   * the row is an ordinary `bool` on the `user` page - not a bespoke control -
//     reading and writing the same root-level `formatNoticeEnabled` spelling the
//     card's button writes;
//   * it is seeded from the host, never from a local idea of the default, so a
//     host that reports nothing shows "not reported" instead of a checkmark it
//     never sent;
//   * off and on are ordinary staged boolean writes: one switch for DWG, DGN,
//     SKP and RVT at once, with no per-format record anywhere;
//   * both catalogs carry its label and help copy.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { FORMAT_NOTICE_OFF, FORMAT_NOTICE_SETTING } from '../src/cad/format-notice-setting.ts';
import {
  SETTINGS_ERROR,
  applyPayload,
  draftFromSettings,
  fieldById,
  validateField,
} from '../src/options/options.ts';
import { readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('the row is a generic user-page boolean for the same key the card writes', () => {
  const field = fieldById('formatNoticeEnabled');
  assert.ok(field, 'the Options table owns the row');
  assert.equal(field.id, 'formatNoticeEnabled');
  assert.equal(field.tab, 'user');
  assert.equal(field.kind, 'bool');
  // Read and written with the same camelCase spelling, so the Options row, the
  // card's own button and the host config leaf cannot drift apart.
  assert.equal(field.wire, 'formatNoticeEnabled');
  assert.equal(field.key, 'formatNoticeEnabled');
  assert.equal(field.labelKey, 'options.field.formatNotice');
  assert.equal(field.helpKey, 'options.help.formatNotice');
  // One key, one write: the card's "Don't show again" is this row turned off.
  assert.equal(FORMAT_NOTICE_SETTING, 'formatNoticeEnabled');
  assert.equal(field.wire, FORMAT_NOTICE_SETTING);
  assert.equal(field.id, FORMAT_NOTICE_OFF.field);
  assert.equal(field.key, FORMAT_NOTICE_OFF.key);
  assert.equal(FORMAT_NOTICE_OFF.value, false);
});

test('the row is seeded from the host, never from a local default', () => {
  assert.equal(draftFromSettings({ formatNoticeEnabled: true }).formatNoticeEnabled, true);
  assert.equal(draftFromSettings({ formatNoticeEnabled: false }).formatNoticeEnabled, false);
  // An older host reports no such key: the row stays explicitly absent (the
  // dialog renders "not reported"), never a `true` invented here.
  assert.equal(draftFromSettings({}).formatNoticeEnabled, null);
  assert.equal(draftFromSettings({ formatNoticeEnabled: 'false' }).formatNoticeEnabled, null);
  // A boolean is the only legal value, exactly like every other bool row.
  assert.equal(validateField('formatNoticeEnabled', true), null);
  assert.equal(validateField('formatNoticeEnabled', false), null);
  for (const bad of ['false', 0, 1, null, undefined]) {
    assert.equal(validateField('formatNoticeEnabled', bad), SETTINGS_ERROR.invalidValue,
      `${JSON.stringify(bad)} is not a bool`);
  }
});

test('off and on are ordinary staged boolean writes, with no per-format record', () => {
  // The host reports the row on: unchecking it stages exactly one write.
  const enabled = { formatNoticeEnabled: true };
  const off = { ...draftFromSettings(enabled, 'en'), formatNoticeEnabled: false };
  assert.deepEqual(applyPayload(off, enabled), [
    { field: 'formatNoticeEnabled', key: 'formatNoticeEnabled', value: false },
  ]);
  // A silenced install: checking it again stages the same key with `true`, which
  // restores the notice for every format.
  const silenced = { formatNoticeEnabled: false };
  const on = { ...draftFromSettings(silenced, 'en'), formatNoticeEnabled: true };
  assert.deepEqual(applyPayload(on, silenced), [
    { field: 'formatNoticeEnabled', key: 'formatNoticeEnabled', value: true },
  ]);
  // A host that never reported the row still accepts the user's explicit choice
  // (the draft is the only side that knows a value).
  assert.deepEqual(applyPayload(off, {}), [
    { field: 'formatNoticeEnabled', key: 'formatNoticeEnabled', value: false },
  ]);
});

test('both catalogs carry the row copy, with no placeholder to drift', () => {
  const field = fieldById('formatNoticeEnabled');
  for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
    for (const key of [field.labelKey, field.helpKey]) {
      assert.equal(typeof catalog[key], 'string', `${locale} is missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} has empty copy for ${key}`);
      assert.doesNotMatch(catalog[key], /\{[A-Za-z_]/, `${key} takes no placeholder`);
    }
  }
  assert.equal(en[field.labelKey], 'Show this notice for these formats');
  assert.equal(zh[field.labelKey], '打开此类格式时显示提示');
});
