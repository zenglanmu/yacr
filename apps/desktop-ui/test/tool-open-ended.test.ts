// Open-ended tool chip contract (ADR 0015 decision B): while the host reports
// `status.tool.openEnded`, the status-bar tool chip shows the honest "more points
// accepted, Enter finishes" copy instead of a numeric count that implies a fixed
// complement; a fixed-arity tool keeps the numeric count. The flag is read from
// the status, never inferred from the captured point count.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('the tool chip renders the open-ended copy from status.tool.openEnded', async () => {
  const statusbar = await readModule('components/StatusBar.tsx');
  // The flag is read from the status only, never computed from the point count.
  assert.ok(statusbar.includes('const toolOpenEnded = tool?.openEnded === true;'));
  assert.ok(
    statusbar.includes("toolOpenEnded ? t('tool.open_ended') : t('tool.points', { count: toolPoints })"),
  );
  // No local mirror of the flag and no arity re-derivation in the view component.
  assert.ok(!statusbar.includes('useState'));
});

test('the open-ended tool chip catalog key exists in both locales with real copy', () => {
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['tool.open_ended'], 'string');
    assert.ok(locale['tool.open_ended'].trim().length > 0);
  }
  // The copy names the honest finish gesture, not a fixed remaining count.
  assert.match(en['tool.open_ended'], /Enter/i);
  assert.match(zh['tool.open_ended'], /回车/);
});
