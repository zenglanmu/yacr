// Contract for the one-shot GPU-fallback notice.
//
// The renderer lane reports a failed accelerated pulled copy by prefixing the
// ordinary `cad_status.error` string with `gpu.pulled.insufficient-vram`. That
// snapshot can arrive on the first status, after a gap recovery and again on
// every later snapshot, so the UI's hard part is not opening the modal — it is
// **not** opening it on every snapshot and **not** re-opening it after a
// dismissal while the same fallback lasts. These tests exercise the store
// directly; the source contracts at the end pin the two wiring points a rendered
// test could not reach without a live host.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  GPU_FALLBACK_TOKEN,
  dismiss,
  isGpuFallbackError,
  isVisible,
  observeStatusError,
  resetGpuFallbackNoticeForTests,
  showGpuFallbackNotice,
  statusBannerText,
  subscribe,
} from '../src/host/notice.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');
const tZh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');

const REPORT = `${GPU_FALLBACK_TOKEN}: required=2699194780 available=1200000000`;

test('the first token opens the notice; later snapshots keep it open without stacking', () => {
  resetGpuFallbackNoticeForTests();
  observeStatusError(null);
  assert.equal(isVisible(), false, 'an absent error never opens the notice');

  observeStatusError(REPORT);
  assert.equal(isVisible(), true, 'the first fallback report opens the notice');

  observeStatusError(`${GPU_FALLBACK_TOKEN}: required=1 available=0`);
  assert.equal(isVisible(), true, 'the notice stays open while the token persists');
});

test('a dismissed notice does not re-open while the same token persists', () => {
  resetGpuFallbackNoticeForTests();
  observeStatusError(REPORT);
  assert.equal(isVisible(), true);

  dismiss();
  assert.equal(isVisible(), false);

  // Every subsequent status snapshot still carries the token; none may reopen.
  observeStatusError(REPORT);
  observeStatusError(`${GPU_FALLBACK_TOKEN}: required=42 available=1`);
  assert.equal(isVisible(), false);
});

test('the latch re-arms after the token clears, so a later fallback warns again', () => {
  resetGpuFallbackNoticeForTests();
  observeStatusError(REPORT);
  dismiss();
  assert.equal(isVisible(), false);

  observeStatusError(null);
  observeStatusError(REPORT);
  assert.equal(isVisible(), true, 'a distinct fallback after recovery warns again');
});

test('recovery also closes a notice that is still on screen', () => {
  resetGpuFallbackNoticeForTests();
  observeStatusError(REPORT);
  assert.equal(isVisible(), true);

  observeStatusError('another host failure');
  assert.equal(isVisible(), false, 'the stale explanation must not outlive the fallback');

  observeStatusError(REPORT);
  assert.equal(isVisible(), true, 'and the latch is armed again for the new fallback');
});

test('dismiss and show are idempotent and notify only on a real change', () => {
  resetGpuFallbackNoticeForTests();
  let calls = 0;
  const off = subscribe(() => { calls += 1; });

  dismiss();
  assert.equal(calls, 0, 'dismissing a closed notice is a no-op');

  showGpuFallbackNotice();
  assert.equal(isVisible(), true);
  assert.equal(calls, 1);
  showGpuFallbackNotice();
  assert.equal(calls, 1, 'showing an open notice is a no-op');

  dismiss();
  assert.equal(isVisible(), false);
  assert.equal(calls, 2);
  dismiss();
  assert.equal(calls, 2);
  off();
});

test('the token test is a prefix test, not a whole-string match', () => {
  assert.equal(isGpuFallbackError(REPORT), true);
  assert.equal(isGpuFallbackError(GPU_FALLBACK_TOKEN), true);
  assert.equal(isGpuFallbackError(`${GPU_FALLBACK_TOKEN}-other`), true);
  assert.equal(isGpuFallbackError(`prefix ${GPU_FALLBACK_TOKEN}`), false);
  assert.equal(isGpuFallbackError(''), false);
  assert.equal(isGpuFallbackError(null), false);
  assert.equal(isGpuFallbackError(undefined), false);
});

test('the banner shows the short translated label only for the known token', () => {
  assert.equal(
    statusBannerText(REPORT, tEn),
    en['gpu.pulled.insufficient_vram.short'],
    'the raw byte-count string is replaced by the short label',
  );
  assert.equal(statusBannerText(REPORT, tZh), zh['gpu.pulled.insufficient_vram.short']);
  assert.equal(statusBannerText('unsupported-runtime: cad_standard_view', tEn), 'unsupported-runtime: cad_standard_view');
  assert.equal(statusBannerText('', tEn), '');
  // A catalog gap must not turn the banner into a key-shaped identifier.
  const tMissing = createTranslator({}, 'en');
  assert.equal(statusBannerText(REPORT, tMissing), REPORT);
});

test('both catalogs define the notice with a distinct title and all four keys', () => {
  const keys = [
    'gpu.pulled.insufficient_vram.title',
    'gpu.pulled.insufficient_vram.body',
    'gpu.pulled.insufficient_vram.short',
    'gpu.pulled.insufficient_vram.dismiss',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
  }
  assert.notEqual(
    zh['gpu.pulled.insufficient_vram.title'],
    zh['vendor.editing.title'],
    'the GPU warning must not reuse the vendor editing title',
  );
});

test('the store is wired to the status path and mounted once beside the plugin notice', async () => {
  const events = await readModule('cad/use-cad-events.ts');
  assert.ok(
    events.includes('observeStatusError(next.error)'),
    'every semantic status snapshot must feed the latch',
  );

  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("from './HostNoticeHost.tsx'"), 'the host notice host is imported');
  assert.ok(workspace.includes('<HostNoticeHost />'), 'the host notice is mounted with the chrome');
  assert.equal(
    workspace.split('<HostNoticeHost />').length - 1,
    1,
    'exactly one mount point, or the modal could stack',
  );
});
