// GRADIENT (AutoCAD 2026) affordance contracts.
//
// GRADIENT is its own canonical, payload-free router selector: the current
// selection is the closed boundary and the fill is forced to the session
// gradient. The chrome must:
//   * carry one catalog row and a ribbon entry in the draw/hatch runs;
//   * gate it on one selected object with its own hint;
//   * admit no argument (the boundary comes from the selection);
//   * name the v1 limits (no preview, no -GRADIENT prompts, no .pat).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
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
  selection: { count: 1 },
};

test('GRADIENT is one catalog row and a draw/annotate hatch-family entry', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'GRADIENT');
  assert.ok(entry, 'GRADIENT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.gradient');
  assert.equal(entry.icon, 'hatch');
  assert.equal(entry.group, 'draw');

  // Draw/Annotate: both draw panels and the Annotate hatch run expose it, never
  // twice in one panel.
  for (const tabId of ['home', 'draw']) {
    const panel = RIBBON_TABS.find(tab => tab.id === tabId)
      .panels.find(candidate => candidate.id === 'draw');
    assert.ok(panel, `${tabId}/draw panel`);
    const items = panel.items.filter(item => item.id === 'GRADIENT');
    assert.equal(items.length, 1, `${tabId}/draw must carry GRADIENT once`);
    assert.equal(items[0].command, 'GRADIENT');
    assert.equal(items[0].labelKey, 'draw.kind.gradient');
    assert.equal(items[0].action, undefined, 'GRADIENT is a router command');
    assert.equal(items[0].variants, undefined, 'GRADIENT has no option popover');
  }
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate')
    .panels.find(panel => panel.id === 'annotation');
  assert.ok(annotate);
  const annotateItem = annotate.items.find(item => item.id === 'GRADIENT');
  assert.ok(annotateItem, 'the Annotate hatch run carries GRADIENT');
  assert.equal(annotateItem.command, 'GRADIENT');
});

test('GRADIENT takes no argument and is admitted only by the loaded vocabulary', () => {
  assert.equal(HOST_NAMES.includes('GRADIENT'), true);
  assert.equal(isSupportedCommand('GRADIENT'), true);
  const admission = admitCommandText('GRADIENT', { status: loaded });
  assert.equal(admission.kind, 'admitted');
  assert.equal(admission.command, 'GRADIENT');
  assert.equal(admitCommandText('gradient', { status: loaded }).kind, 'admitted');

  // The boundary is the selection, so the name takes no typed argument.
  assert.equal(admitCommandText('GRADIENT 1', { status: loaded }).kind, 'unsupported');

  setCommandCatalog([]);
  assert.equal(isSupportedCommand('GRADIENT'), false);
  assert.equal(admitCommandText('GRADIENT', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('GRADIENT is gated on one selected object with its own hint', () => {
  assert.equal(SELECTION_REQUIREMENTS.GRADIENT.count, 1);
  assert.equal(SELECTION_REQUIREMENTS.GRADIENT.reasonKey, 'draw.kind.gradient.hint');
  assert.equal(
    unavailableReasonKey('GRADIENT', { status: { ...loaded, selection: { count: 0 } } }),
    'draw.kind.gradient.hint',
  );
  assert.notEqual(
    unavailableReasonKey('GRADIENT', { status: { ...loaded, selection: { count: 1 } } }),
    'draw.kind.gradient.hint',
  );
  assert.equal(
    unavailableReasonKey('GRADIENT', {
      status: { ...loaded, documentLabel: null, selection: { count: 0 } },
    }),
    'plot.no_document',
  );
});

test('GRADIENT copy names the v1 limits in both catalogs', async () => {
  for (const locale of [await readCatalog('zh-CN'), await readCatalog('en')]) {
    for (const key of ['draw.kind.gradient', 'draw.kind.gradient.hint']) {
      assert.equal(typeof locale[key], 'string', key);
      assert.ok(locale[key].trim().length > 0, key);
    }
  }
  const en = await readCatalog('en');
  assert.match(en['draw.kind.gradient.hint'], /\+Z/);
  assert.match(en['draw.kind.gradient.hint'], /gradient/i);
  assert.match(en['draw.kind.gradient.hint'], /preview/i);
  assert.match(en['draw.kind.gradient.hint'], /-GRADIENT/);
  assert.match(en['draw.kind.gradient.hint'], /\.pat/);
  assert.match(en['draw.kind.gradient.hint'], /\{command\}/);
});
