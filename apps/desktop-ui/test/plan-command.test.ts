// `PLAN` (AutoCAD 2026) contract: "displays an orthographic view of the XY
// plane".
//
// v1 is the **top/plan 2D view only**. This build has no UCS model, so
// AutoCAD's `PLAN` orientation options (UCS / World / Current) are NOT
// IMPLEMENTED — the command takes no argument, and the catalog/ribbon must not
// imply a choice it does not have. The command travels on the payload-free
// `cad_command` funnel (never the typed `cad_standard_view` lane), and the
// existing Top standard-view button keeps that lane unchanged.
//
// This suite pins the three UI surfaces (catalog row, View-tab standard panel,
// admission/gating) and both i18n catalogs. It makes no GUI or host claim: the
// loaded vocabulary is the `HOST_NAMES` mirror, as in the other chrome tests.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { STANDARD_VIEWS } from '../src/cad/view3d.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  gridEnabled: true,
  snapGridEnabled: false,
  snapSpacing: 10,
  orthoEnabled: false,
  canSave: true,
  canPlot: true,
  canFit: true,
};

function standardPanel() {
  const tab = RIBBON_TABS.find(candidate => candidate.id === 'view');
  assert.ok(tab, 'the View tab must exist');
  const panel = tab.panels.find(candidate => candidate.id === 'standard');
  assert.ok(panel, 'the View tab must carry the standard-view panel');
  return panel;
}

test('PLAN is catalogued exactly once as a payload-free view command', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'PLAN');
  assert.equal(rows.length, 1, 'PLAN must appear exactly once in the catalog');
  assert.deepEqual(
    { ...rows[0] },
    { id: 'PLAN', labelKey: 'view.plan', icon: 'view', group: 'view' },
    'the catalog row names the shared catalog key and the view glyph',
  );
});

test('the View tab standard panel appends PLAN after the seven standard views', () => {
  const panel = standardPanel();
  const ids = panel.items.map(item => item.id);
  // The seven standard views keep their order first: PLAN must never be spliced
  // between them.
  assert.deepEqual(ids.slice(0, STANDARD_VIEWS.length), STANDARD_VIEWS.map(entry => entry.id));
  assert.equal(ids.length, STANDARD_VIEWS.length + 1, 'PLAN is the only appended row');
  assert.equal(ids[STANDARD_VIEWS.length], 'PLAN');
  assert.equal(ids.filter(id => id === 'PLAN').length, 1);
  // The row is a `cad_command` token, not a typed standard view: the Top button
  // stays on `cad_standard_view`.
  const plan = panel.items.find(item => item.id === 'PLAN');
  assert.deepEqual({ ...plan }, { id: 'PLAN', labelKey: 'view.plan', icon: 'view', command: 'PLAN' });
  assert.equal(plan.standardView, undefined, 'PLAN is payload-free');
  assert.equal(plan.action, undefined, 'PLAN is a router command, not a shell action');
  // ...and the Top button is untouched and still payload-carrying.
  const top = panel.items.find(item => item.id === 'viewTop');
  assert.equal(top.standardView, 'top');
  assert.equal(top.command, undefined);
});

test('PLAN is admitted by the loaded vocabulary and needs an open drawing', () => {
  assert.equal(HOST_NAMES.includes('PLAN'), true, 'the vocabulary mirror carries PLAN');
  assert.equal(isSupportedCommand('PLAN'), true);
  assert.equal(isSupportedCommand(' plan '), true, 'admission is case-insensitive/trimmed');
  assert.equal(admitCommandText('PLAN', { status: loaded }).kind, 'admitted');
  // A document is its whole precondition: the start page disables it with the
  // shared "no drawing is open" reason rather than "unsupported".
  assert.equal(DOCUMENT_SCOPED_COMMANDS.includes('PLAN'), true);
  const empty = { ...loaded, documentLabel: null, entityCount: null, canSave: false, canPlot: false, canFit: false };
  assert.equal(admitCommandText('PLAN', { status: empty }).kind, 'unavailable');
  assert.equal(admitCommandText('PLAN', { status: null }).kind, 'unavailable');
  // It is not a gesture: a click carries the complete command.
  assert.equal(admitCommandText('PLAN', { status: loaded }).command, 'PLAN');
});

test('both catalogs word the PLAN label', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(zh['view.plan'], '平面视图');
  assert.equal(en['view.plan'], 'Plan view');
  // The key is not shared with an existing row, so the plan row cannot borrow
  // another view's name.
  assert.notEqual(zh['view.plan'], zh['view.standard.top']);
  assert.notEqual(en['view.plan'], en['view.standard.top']);
});
