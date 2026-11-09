// Source contract for the About dialog's manual update check.
//
// The store's forced check is covered by `test/update-store.test.ts`; this file
// pins the UI wiring a node test cannot render without a host: the footer
// action, the polite status line, the per-kind copy (only `up-to-date` claims
// the app is current), the availability action and its honest failure path, and
// the removed platform-acceptance sentence in both catalogs.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { POPUP_KEYS } from '../src/popup/popup.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const UPDATE_KEYS = [
  'dialog.about.check_update',
  'dialog.about.update_checking',
  'dialog.about.update_up_to_date',
  'dialog.about.update_available',
  'dialog.about.update_unavailable',
  'dialog.about.update_unsupported',
  'dialog.about.open_download',
];

function placeholders(message) {
  return [...message.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map(match => match[1]).sort();
}

test('both catalogs carry every About update key with the same placeholders', () => {
  for (const key of UPDATE_KEYS) {
    assert.equal(typeof en[key], 'string', `en catalog is missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN catalog is missing ${key}`);
    assert.deepEqual(placeholders(en[key]), placeholders(zh[key]), `${key} placeholders drift`);
  }
  assert.deepEqual(placeholders(en['dialog.about.update_available']), ['version']);
});

test('the About update keys are declared once in POPUP_KEYS', () => {
  assert.equal(POPUP_KEYS.updateCheck, 'dialog.about.check_update');
  assert.equal(POPUP_KEYS.updateChecking, 'dialog.about.update_checking');
  assert.equal(POPUP_KEYS.updateUpToDate, 'dialog.about.update_up_to_date');
  assert.equal(POPUP_KEYS.updateAvailable, 'dialog.about.update_available');
  assert.equal(POPUP_KEYS.updateUnavailable, 'dialog.about.update_unavailable');
  assert.equal(POPUP_KEYS.updateUnsupported, 'dialog.about.update_unsupported');
  assert.equal(POPUP_KEYS.openDownload, 'dialog.about.open_download');
});

test('AboutDialog calls the forced check and disables it only while checking', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  assert.match(source, /import \{[^}]*forceUpdateCheck[^}]*\} from '\.\.\/update\/store\.ts';/);
  assert.match(source, /useUpdateStatus\(\)/);
  assert.match(source, /forceUpdateCheck\(\);/);
  assert.match(source, /disabled=\{status\.kind === 'checking'\}/);
  assert.match(source, /onClick=\{handleCheck\}/);
  assert.match(source, /\{t\(POPUP_KEYS\.updateCheck\)\}/);
});

test('AboutDialog renders one polite status line and only the store decides its kind', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  assert.match(source, /role="status"/);
  assert.match(source, /aria-live="polite"/);
  assert.match(source, /let updateLine: string \| null = null;/);
  assert.match(
    source,
    /status\.kind === 'up-to-date'\) updateLine = t\(POPUP_KEYS\.updateUpToDate\)/,
    'only the up-to-date branch may claim the app is current',
  );
  assert.match(source, /status\.kind === 'checking'\) updateLine = t\(POPUP_KEYS\.updateChecking\)/);
  assert.match(source, /status\.kind === 'available'\) updateLine = t\(POPUP_KEYS\.updateAvailable/);
  assert.match(source, /status\.kind === 'unavailable'\) updateLine = t\(POPUP_KEYS\.updateUnavailable\)/);
  assert.match(source, /status\.kind === 'unsupported'\) updateLine = t\(POPUP_KEYS\.updateUnsupported\)/);
  assert.doesNotMatch(source, /status\.kind === 'idle'|status\.kind === 'skipped'/, 'idle/skipped stay silent');
});

test('the unavailable branch keeps the verbatim reason visible', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  assert.match(source, /status\.kind === 'unavailable'[\s\S]{0,200}<p style=\{UPDATE_REASON\}>\{status\.reason\}<\/p>/);
});

test('the available branch opens the release page and reports a refusal honestly', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  assert.match(source, /openReleasePage\(\)/);
  assert.match(source, /\{t\(POPUP_KEYS\.openDownload\)\}/);
  assert.match(source, /t\('update\.open\.failed'\)/);
  assert.match(source, /writeClipboardText\(RELEASE_PAGE_URL\)/);
  assert.match(source, /t\('update\.copy'\)/);
  assert.match(source, /t\('update\.copy\.copied'\)/);
  assert.match(source, /t\('update\.copy\.failed'\)/);
  assert.match(source, /useState<CopyState>\('idle'\)/);
});

test('AboutDialog carries no hardcoded CJK copy and no inline dialog keys', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  assert.doesNotMatch(source, /[\u3400-\u4dbf\u4e00-\u9fff]/);
  assert.doesNotMatch(source, /'(dialog\.|file\.about)/, 'dialog copy comes through POPUP_KEYS');
});

test('the platform-acceptance sentence is gone from both intros', () => {
  assert.doesNotMatch(en['dialog.about.intro'], /Linux\/Wayland is the primary development target/);
  assert.doesNotMatch(en['dialog.about.intro'], /Android APK has been sideloaded/);
  assert.match(en['dialog.about.intro'], /early development with no shippable release\.$/);

  assert.doesNotMatch(zh['dialog.about.intro'], /Linux\/Wayland 是主开发目标/);
  assert.doesNotMatch(zh['dialog.about.intro'], /Android APK 已由维护者侧载运行/);
  assert.match(zh['dialog.about.intro'], /早期开发，尚无可交付版本。$/);

  // The product description on both sides is untouched.
  assert.match(en['dialog.about.intro'], /Rust core[\s\S]*React \+ Tauri[\s\S]*wgpu/);
  assert.match(zh['dialog.about.intro'], /Rust 核心[\s\S]*React \+ Tauri[\s\S]*wgpu/);
});
