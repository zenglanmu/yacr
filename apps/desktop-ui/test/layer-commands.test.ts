// AutoCAD 2026 layer-selection commands (LAYCUR and the rest of the run).
//
// All are **pre-selection based** in this build: the fixed capture machine cannot
// ask AutoCAD's *Select objects* prompt, so the current session selection is the
// whole input, exactly like MATCHPROP/LAYISO. LAYTHW is the exception: it takes
// no selection at all. What this suite pins: the catalogue rows and their
// annotate group, the Layers-panel buttons (router `command`, never `action`),
// the selection requirements and their own hint keys, LAYTHW's document scope,
// the palette-carrier filtering, the loaded-vocabulary mirror and both i18n
// catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  selectionRequirementMet,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { DOCUMENT_SCOPED_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import {
  offeredItems,
  offeredPanels,
  PALETTE_COMMANDS,
  RIBBON_TABS,
} from '../src/ribbon/ribbon-model.ts';
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

/** The run's ids, in the panel's order. */
const IDS = ['LAYCUR', 'LAYMCUR', 'LAYFRZ', 'LAYTHW', 'LAYLCK', 'LAYULK', 'SETBYLAYER'];

/** Ids implemented so far in this lane (the file grows one commit at a time). */
const IMPLEMENTED = ['LAYCUR', 'LAYMCUR', 'LAYFRZ', 'LAYTHW', 'LAYLCK', 'LAYULK', 'SETBYLAYER'];

const SELECTION_COUNTS = {
  LAYCUR: 1,
  LAYMCUR: 1,
  LAYFRZ: 1,
  LAYTHW: 0,
  LAYLCK: 1,
  LAYULK: 1,
  SETBYLAYER: 1,
};

function layersPanel() {
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  assert.ok(home, 'the home tab must exist');
  const panel = home.panels.find(candidate => candidate.id === 'layers');
  assert.ok(panel, 'the home tab must keep its layers panel');
  return panel;
}

test('the layer-selection run is catalogued in the annotate group', () => {
  for (const id of IMPLEMENTED) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, `draw.kind.${id.toLowerCase()}`, id);
    assert.equal(entry.group, 'annotate', id);
    assert.equal(typeof entry.icon, 'string', id);
    // A canonical name of its own, admitted by the loaded host vocabulary.
    assert.equal(normalizeCommandText(`  ${id.toLowerCase()} `), id);
    assert.equal(isSupportedCommand(id), true, id);
  }
});

test('the Layers panel carries each implemented command as a router button', () => {
  const panel = layersPanel();
  for (const id of IMPLEMENTED) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item, `home/layers must carry ${id}`);
    assert.equal(item.command, id, id);
    assert.equal(item.action, undefined, `${id} must not be a shell action`);
    assert.equal(item.standardView, undefined, id);
    assert.equal(item.variants, undefined, `${id} has no options sub-prompt`);
    assert.equal(item.large, undefined, `${id} is a small layer button`);
    // Exactly one dispatch token.
    assert.deepEqual(
      [item.command, item.standardView, item.action].filter(token => token !== undefined),
      [id],
    );
  }
});

test('every implemented command states its selection precondition in its own hint', () => {
  for (const id of IMPLEMENTED) {
    const count = SELECTION_COUNTS[id];
    const reasonKey = `draw.kind.${id.toLowerCase()}.hint`;
    assert.deepEqual({ ...SELECTION_REQUIREMENTS[id] }, { count, reasonKey }, id);
    assert.deepEqual({ ...selectionRequirement(id.toLowerCase()) }, { count, reasonKey }, id);
    assert.equal(selectionRequirementMet(id, NO_DOCUMENT), false, `${id}: no document`);
    if (count === 0) {
      assert.equal(selectionRequirementMet(id, selected(0)), true, `${id}: empty selection is fine`);
    } else {
      assert.equal(selectionRequirementMet(id, selected(0)), false, `${id}: nothing selected`);
      assert.equal(selectionRequirementMet(id, selected(count)), true, id);
      assert.equal(selectionRequirementMet(id, selected(count + 1)), true, `${id}: the upper bound is the host's`);

      assert.equal(admitCommandText(id, { status: selected(count) }).kind, 'admitted', id);
      const refused = admitCommandText(id, { status: selected(0) });
      assert.equal(refused.kind, 'unavailable', id);
      assert.equal(refused.reasonKey, reasonKey, id);
      assert.equal(unavailableReasonKey(id, { status: selected(0) }), reasonKey, id);
    }
    // No drawing: "no drawing is open", never "select more objects".
    assert.equal(unavailableReasonKey(id, { status: NO_DOCUMENT }), 'plot.no_document', id);
  }
});

test('LAYTHW needs a drawing but no selection, and is document-scoped', () => {
  if (!IMPLEMENTED.includes('LAYTHW')) return;
  assert.equal(DOCUMENT_SCOPED_COMMANDS.includes('LAYTHW'), true);
  assert.equal(SELECTION_REQUIREMENTS.LAYTHW.count, 0);
  assert.equal(admitCommandText('LAYTHW', { status: selected(0) }).kind, 'admitted');
  assert.equal(unavailableReasonKey('LAYTHW', { status: NO_DOCUMENT }), 'plot.no_document');
  // The selection-gated commands are not document-scoped rows: their own
  // requirement already answers the no-document case.
  for (const id of IMPLEMENTED.filter(name => name !== 'LAYTHW')) {
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), false, id);
  }
});

test('the whole run is dropped with the palette on a palette-less carrier', () => {
  for (const id of IDS) {
    assert.ok(PALETTE_COMMANDS.includes(id), `${id} must be a palette command`);
  }
  const panel = layersPanel();
  assert.deepEqual(offeredItems(panel.items, false), [], 'nothing is offered without the palette');
  assert.deepEqual(offeredPanels([panel], false), [], 'the panel is dropped whole');
  for (const item of panel.items) {
    assert.ok(PALETTE_COMMANDS.includes(item.command), 'only palette commands live in this panel');
  }
});

test('each name is in the loaded-vocabulary mirror and both catalogs word the rule', () => {
  for (const id of IMPLEMENTED) {
    assert.ok(HOST_NAMES.includes(id), `${id} must be in HOST_NAMES`);
    const labelKey = `draw.kind.${id.toLowerCase()}`;
    const hintKey = `${labelKey}.hint`;
    for (const [locale, catalog] of [['en', en], ['zh-CN', zh]]) {
      assert.equal(typeof catalog[labelKey], 'string', `${locale} missing ${labelKey}`);
      assert.equal(typeof catalog[hintKey], 'string', `${locale} missing ${hintKey}`);
      assert.ok(catalog[labelKey].trim().length > 0, `${locale} empty ${labelKey}`);
      assert.ok(catalog[hintKey].trim().length > 0, `${locale} empty ${hintKey}`);
    }
  }
  // The hints state the pre-selection rule (or, for LAYTHW, that it takes no
  // selection), so the disabled button and the host refusal cannot disagree.
  for (const id of IMPLEMENTED) {
    const hint = `draw.kind.${id.toLowerCase()}.hint`;
    assert.match(en[hint], /select/i, `en ${hint} states the selection rule`);
    assert.match(zh[hint], /选择|图层|冻结/, `zh ${hint} states the rule`);
  }
});
