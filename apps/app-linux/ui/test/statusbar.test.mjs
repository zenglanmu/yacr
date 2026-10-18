// Status-bar contract: the idle hover-preview chip renders the host's real
// `status.hoverPreview` flag, dispatches the single HOVERPREVIEW command, and
// renders nothing when an older payload omits the field (never a guessed value).
import test from 'node:test';
import assert from 'node:assert/strict';
import { dispatchHoverPreview, HOVER_PREVIEW_COMMAND, hoverPreviewToggle } from '../src/cad.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('hover preview renders the real status flag and never fabricates one', () => {
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: true }, true), { on: true, disabled: false });
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: false }, true), { on: false, disabled: false });
  // The shell gates the command (pending / loading / unknown vocabulary).
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: true }, false), { on: true, disabled: true });
  assert.deepEqual(hoverPreviewToggle({ hoverPreview: false }, false), { on: false, disabled: true });
  // No field (older host), no status, or a non-boolean shows no chip at all.
  for (const status of [null, undefined, {}, { hoverPreview: null }, { hoverPreview: 'true' }, { hoverPreview: 1 }]) {
    assert.equal(hoverPreviewToggle(status, true), null);
  }
});

test('the hover-preview toggle dispatches the single HOVERPREVIEW command', () => {
  const calls = [];
  dispatchHoverPreview(command => calls.push(command));
  assert.deepEqual(calls, ['HOVERPREVIEW']);
  assert.equal(HOVER_PREVIEW_COMMAND, 'HOVERPREVIEW');
});

test('the status bar reads the chip state from status and uses normal command admission', async () => {
  const source = await readModule('components/StatusBar.jsx');
  assert.ok(source.includes("hoverPreviewToggle(shell.status, shell.canCommand('HOVERPREVIEW'))"));
  assert.ok(source.includes('on={hoverPreview.on}'));
  assert.ok(source.includes('disabled={hoverPreview.disabled}'));
  assert.ok(source.includes('dispatchHoverPreview(shell.runCommand)'));
  // The title explains the real on/off behaviour from the catalogs.
  assert.ok(source.includes("t(hoverPreview.on ? 'status.toggle.hover_preview_on' : 'status.toggle.hover_preview_off')"));
  assert.ok(source.includes("label={t('status.toggle.hover_preview')}"));
  // Missing field: the chip is simply absent, not rendered as off.
  assert.ok(source.includes('{hoverPreview ? ('));
  // The server is the only truth: no local mirror of the flag.
  assert.ok(!source.includes('useState'));
  assert.ok(!source.includes('setHoverPreview'));
  // The existing drafting placeholders keep their disabled/unsupported state.
  assert.ok(source.includes("t('status.toggle_unsupported', { name })"));
});

test('the hover-preview catalog keys exist in both locales with real copy', () => {
  for (const key of ['status.toggle.hover_preview', 'status.toggle.hover_preview_on', 'status.toggle.hover_preview_off']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
  }
  assert.match(zh['status.toggle.hover_preview_on'], /悬停/);
  assert.match(en['status.toggle.hover_preview_on'], /hover/i);
});
