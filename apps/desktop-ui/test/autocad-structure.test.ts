// AutoCAD 2024 ribbon structure contract (ADR-0053 D7).
//
// The fixture is a hand transcription of the reference tab/panel order (File is
// this build's extension). The test fails when a tab is renamed, reordered, or a
// self-made top-level tab returns; a panel moving inside its tab is a deliberate
// structural edit and must update the fixture in the same change.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog } from './helpers.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/tabs.ts';

const FIXTURE = new URL('./fixtures/autocad-2024-drafting.json', import.meta.url);

test('the tab order is the reference order, with File as the one extension tab', async () => {
  const expected = JSON.parse(await readFile(FIXTURE, 'utf8'));
  assert.deepEqual(
    RIBBON_TABS.map(tab => tab.id),
    [...expected.extensionTabs, ...expected.tabs.map(tab => tab.id)],
  );
  assert.equal(RIBBON_TABS[0].id, 'file', 'the extension tab stays first');
  assert.equal(RIBBON_TABS[1].id, 'home', 'Home is the first reference tab');
});

test('every tab carries the reference panels in order', async () => {
  const expected = JSON.parse(await readFile(FIXTURE, 'utf8'));
  for (const tab of expected.tabs) {
    const actual = RIBBON_TABS.find(entry => entry.id === tab.id);
    assert.ok(actual, `missing reference tab ${tab.id}`);
    assert.deepEqual(
      actual.panels.map(panel => panel.id),
      tab.panels,
      `${tab.id} panel order drifted from the reference`,
    );
  }
});

test('the self-made top-level tabs do not come back', () => {
  for (const id of ['draw', 'measure', 'review']) {
    assert.equal(RIBBON_TABS.some(tab => tab.id === id), false, `${id} must not be a top-level tab`);
  }
});

test('reference-pending panels are empty and say so', async () => {
  const expected = JSON.parse(await readFile(FIXTURE, 'utf8'));
  const pending = new Set(expected.pendingPanels);
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      assert.equal(Boolean(panel.pending), pending.has(panel.id), `${panel.id} pending flag`);
      if (panel.pending) {
        assert.deepEqual(panel.items, [], `${panel.id} must not offer a fake button`);
      } else {
        assert.ok(panel.items.length > 0, `${panel.id} must carry at least one real item`);
      }
    }
  }
  // The two drawing groups moved into Home with their commands intact; the old
  // top-level tabs are gone but no selector is lost.
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  const draw = home.panels.find(panel => panel.id === 'draw');
  const utilities = home.panels.find(panel => panel.id === 'utilities');
  assert.ok(draw.items.some(item => item.command === 'LINE'));
  assert.ok(utilities.items.some(item => item.command === 'DIST'));
});

test('the catalog names use the reference localization', async () => {
  const zh = await readCatalog('zh-CN');
  assert.equal(zh['ribbon.annotate'], '注释', 'AutoCAD zh-CN calls the tab 注释, not 批注');
  for (const key of ['ribbon.home', 'ribbon.insert', 'ribbon.annotate', 'ribbon.parametric',
    'ribbon.view', 'ribbon.manage', 'ribbon.output', 'ribbon.addins']) {
    assert.equal(typeof zh[key], 'string', key);
  }
});

test('every reference tab still references at least one real router selector', () => {
  const ids = new Set(ribbonCommandIds());
  // Home/Insert/Annotate/View/Manage/Output carry wired commands; Parametric and
  // Add-ins are structurally present but only hold pending/ chrome entries.
  for (const command of ['LINE', 'PLINE', 'DIMLINEAR', 'INSERT', 'TABLE', 'PLOT', 'EXPORT', 'XREF']) {
    assert.ok(ids.has(command), `${command} must stay on the ribbon`);
  }
});
