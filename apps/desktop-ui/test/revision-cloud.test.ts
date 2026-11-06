// REVCLOUD user-facing surface (AutoCAD 2026, selection-based Object mode v1).
//
// The command takes `CommandPayload::None`: the one preselected closed
// straight-segment polyline is the whole input, and every edge becomes one
// outward arc bulge. There is **no** interactive rectangular/polygonal draw mode,
// no `Arc length` option and no object picking in v1, so the surface is an
// ordinary selection-gated router command — never a draw tool with a capture.
//
// What this suite pins: the catalogue row, both draw-panel buttons, the
// one-object selection requirement and its hint key, admission from the loaded
// host vocabulary plus a selection, and both i18n keys.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  commandById,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  selectionRequirementMet,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const DOC = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};
const NO_DOCUMENT = { ...DOC, documentLabel: null, entityCount: null };
const selected = count => ({ ...DOC, selection: { count } });

const ID = 'REVCLOUD';
const LABEL_KEY = 'draw.kind.revcloud';
const HINT_KEY = 'draw.kind.revcloud.hint';

function drawPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `the ${tabId} tab must exist`);
  const panel = tab.panels.find(candidate => candidate.id === 'draw');
  assert.ok(panel, `the ${tabId} tab must keep its draw panel`);
  return panel;
}

test('REVCLOUD is catalogued as its own draw selector', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === ID);
  assert.ok(entry, 'REVCLOUD must be in the command catalog');
  assert.deepEqual(
    { ...entry },
    { id: ID, labelKey: LABEL_KEY, icon: 'polyline', group: 'draw' },
  );
  // A canonical name of its own, admitted by the loaded host vocabulary.
  assert.equal(normalizeCommandText('  revcloud '), ID);
  assert.equal(commandById(ID.toLowerCase()).id, ID);
  assert.equal(isSupportedCommand(ID.toLowerCase()), true);
});

test('both draw panels carry the small REVCLOUD router button', () => {
  for (const tabId of ['home', 'draw']) {
  const panel = drawPanel(tabId);
  const item = panel.items.find(candidate => candidate.id === ID);
  assert.ok(item, `${tabId}/draw must carry REVCLOUD`);
  assert.equal(item.command, ID);
  assert.equal(item.labelKey, LABEL_KEY);
  assert.equal(item.icon, 'polyline');
  // A small router button, not a draw capture: no payload, no popover, no action.
  assert.equal(item.large, undefined, 'not a primary affordance');
  assert.equal(item.variants, undefined, 'no options sub-prompt');
  assert.equal(item.action, undefined, 'a router command, not a shell action');
  assert.equal(item.standardView, undefined, 'it carries no payload');
  // Exactly one dispatch token, the model's own union contract.
  assert.deepEqual(
    [item.command, item.standardView, item.action].filter(token => token !== undefined),
    [ID],
  );
  }
  // It appears once on each draw panel (Home and Draw); there is no second
  // capture to confuse it with.
  const carriers = RIBBON_TABS.flatMap(tab => tab.panels).flatMap(section => section.items)
    .filter(candidate => candidate.id === ID);
  assert.equal(carriers.length, 2, 'one carrier per draw panel');
  const variantCarriers = RIBBON_TABS.flatMap(tab => tab.panels)
    .flatMap(section => section.items)
    .flatMap(entry => entry.variants ?? [])
    .filter(variant => variant.command === ID);
  assert.deepEqual(variantCarriers, [], 'no hidden variant carrier');
});

test('REVCLOUD needs one selected object and words it in its own hint', () => {
  assert.deepEqual({ ...SELECTION_REQUIREMENTS[ID] }, { count: 1, reasonKey: HINT_KEY });
  assert.deepEqual({ ...selectionRequirement(ID.toLowerCase()) }, {
    count: 1,
    reasonKey: HINT_KEY,
  });
  assert.equal(selectionRequirementMet(ID, DOC), false, 'nothing selected');
  assert.equal(selectionRequirementMet(ID, selected(0)), false);
  assert.equal(selectionRequirementMet(ID, selected(1)), true);
  assert.equal(selectionRequirementMet(ID, selected(2)), true, 'the upper bound is the host\'s');
  assert.equal(selectionRequirementMet(ID, NO_DOCUMENT), false, 'no drawing, no selection block');

  // One object is admitted; none is refused before IPC with this hint — the same
  // key the disabled ribbon button shows.
  assert.equal(admitCommandText(ID, { status: selected(1) }).kind, 'admitted');
  const refused = admitCommandText(ID, { status: selected(0) });
  assert.equal(refused.kind, 'unavailable');
  assert.equal(refused.reasonKey, HINT_KEY);
  assert.equal(unavailableReasonKey(ID, { status: selected(0) }), HINT_KEY);
  // No drawing: "no drawing is open", never "select more objects".
  assert.equal(unavailableReasonKey(ID, { status: NO_DOCUMENT }), 'plot.no_document');
  // The empty host vocabulary is the one honest disable.
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(ID), false);
  assert.equal(admitCommandText(ID, { status: selected(1) }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('both catalogs carry the label and the Object-mode v1 hint', () => {
  for (const key of [LABEL_KEY, HINT_KEY]) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
  }
  assert.equal(en[LABEL_KEY], 'Revision Cloud');
  assert.equal(zh[LABEL_KEY], '修订云线');
  // The hint states the v1 boundary plainly and promises no interactive mode.
  assert.match(en[HINT_KEY], /closed/i);
  assert.match(en[HINT_KEY], /straight-segment polyline/i);
  assert.match(en[HINT_KEY], /outward arc/i);
  assert.match(en[HINT_KEY], /Object mode v1/);
  assert.match(zh[HINT_KEY], /闭合/);
  assert.match(zh[HINT_KEY], /直线段多段线/);
  assert.match(zh[HINT_KEY], /向外弧/);
  assert.match(zh[HINT_KEY], /v1 对象模式/);
  for (const hint of [en[HINT_KEY], zh[HINT_KEY]]) {
    assert.equal(/rectangular|polygonal|矩形|多边形/i.test(hint), false, 'no draw-mode claim');
    assert.equal(/arc length|弧长/i.test(hint), false, 'no Arc length option');
  }
});
