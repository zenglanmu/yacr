// Source contract for the in-app update banner.
//
// The store's own behaviour is covered by `test/update-store.test.ts`; this file
// pins the UI wiring that a node test cannot render without a host: `Workspace`
// mounts the host, the host starts the one check and defers to dialogs/other
// notices, the banner stays a polite non-modal status region (no focus trap, no
// key handler), and both catalogs carry the same `update.*` keys.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { readCatalog, readCss, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const UPDATE_KEYS = [
  'update.available.title',
  'update.available.body',
  'update.available.open',
  'update.available.skip',
  'update.available.later',
  'update.open.failed',
  'update.copy',
  'update.copy.copied',
  'update.copy.failed',
];

function placeholders(message) {
  return [...message.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map(match => match[1]).sort();
}

test('both catalogs carry every update key with the same placeholders', () => {
  for (const key of UPDATE_KEYS) {
    assert.equal(typeof en[key], 'string', `en catalog is missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN catalog is missing ${key}`);
    assert.deepEqual(placeholders(en[key]), placeholders(zh[key]), `${key} placeholders drift`);
  }
  assert.deepEqual(placeholders(en['update.available.title']), ['latest']);
  assert.deepEqual(placeholders(en['update.available.body']), ['current']);
});

test('Workspace mounts the update host beside the other notice hosts', async () => {
  const source = await readModule('components/Workspace.tsx');
  assert.match(source, /import \{ UpdateNoticeHost \} from '\.\/UpdateNoticeHost\.tsx';/);
  assert.match(source, /<UpdateNoticeHost \/>/);
});

test('the host starts one check and renders only for an available release', async () => {
  const source = await readModule('components/UpdateNoticeHost.tsx');
  assert.match(source, /startUpdateCheck\(\)/);
  assert.match(source, /useUpdateStatus\(\)/);
  assert.match(source, /status\.kind !== 'available'/, 'only `available` may render a banner');
  assert.match(source, /<UpdateNotice/);
});

test('the host defers to a live dialog and to the other shell notices', async () => {
  const source = await readModule('components/UpdateNoticeHost.tsx');
  assert.match(source, /useShell\(\)/);
  assert.match(source, /status\?\.popup/, 'a host dialog must suppress the banner');
  assert.match(source, /usePluginNotice\(\)/);
  assert.match(source, /isHostNoticeVisible/, 'the GPU-fallback notice must not stack with it');
});

test('the banner wires the three actions to the existing store API', async () => {
  const host = await readModule('components/UpdateNoticeHost.tsx');
  assert.match(host, /openReleasePage/);
  assert.match(host, /skipUpdateVersion/);
  assert.match(host, /dismissUpdate/);

  const notice = await readModule('components/UpdateNotice.tsx');
  assert.match(notice, /writeClipboardText\(RELEASE_PAGE_URL\)/, 'a failed open reveals the URL with a real copy path');
  assert.match(notice, /t\('update\.available\.open'\)/);
  assert.match(notice, /t\('update\.available\.skip'\)/);
  assert.match(notice, /t\('update\.available\.later'\)/);
  assert.match(notice, /t\('update\.copy\.copied'\)/);
  assert.match(notice, /t\('update\.copy\.failed'\)/);
});

test('the banner is a polite non-modal region that never takes the keyboard', async () => {
  const source = await readModule('components/UpdateNotice.tsx');
  assert.match(source, /role="status"/);
  assert.match(source, /aria-live="polite"/);
  assert.match(source, /aria-labelledby=/);
  assert.match(source, /aria-describedby=/);
  assert.doesNotMatch(source, /useNoticeFocus/, 'no focus trap');
  assert.doesNotMatch(source, /addEventListener\(['"]keydown['"]/, 'Escape must keep reaching CAD');
  assert.doesNotMatch(source, /\.focus\(\)/, 'focus is never stolen');
  assert.doesNotMatch(source, /[\u3400-\u4dbf\u4e00-\u9fff]/, 'no hardcoded CJK copy in the TSX');
});

test('the banner styles sit below the floating palette layer, above the carrier', async () => {
  const css = await readCss();
  assert.match(css, /\.update-notice-layer\s*\{[^}]*z-index:\s*9;/);
  assert.match(css, /\.palette-layer\s*\{[^}]*z-index:\s*10;/);
  assert.match(css, /:root\[data-layout="mobile"\] \.update-notice\s*\{/, 'compact layout gets its own inset');
  assert.match(css, /env\(safe-area-inset-left\)/);
});
