// AREA OBJECT / LAYERP user-facing surface (2026-10-27).
//
// Two additions, and one of them is **not** a plain "more of the same":
//
//   * `AREA OBJECT` is the selection-based area variant. It measures exactly one
//     closed object taken from the current selection (a circle, an ellipse or a
//     closed polyline with bulges) and echoes the exact area under its own
//     algorithm token (`area_object`). It is a **multi-word** selector, which is
//     the interesting part here: the host canonicalises the whole trimmed line
//     (`AREA OBJECT` is one name there, exactly like the `MEASURE AREA` alias),
//     while this chrome's `splitCommandText` reads the first word as the base
//     name and the rest as a typed argument. This suite therefore pins the
//     catalogue/completion half that works and the **local admission gap** that
//     does not, instead of pretending a space-separated line is admitted.
//   * `LAYERP` is no longer an alias of the "clear every override" command
//     (`LAYUNISO`/`LAYON` keep meaning that): it restores the *previous*
//     layer-override state, carries its own label (`layers.previous`) and its own
//     selection rule (none), and is dropped with the layers palette because that
//     palette is the only surface that names the state it restores.
//
// Every user-visible string comes from the shared catalogs; nothing here may claim
// interactive object picking (the command takes the existing selection).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  aliasTarget,
  commandById,
  filterCommandSuggestions,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  selectionRequirementMet,
  setCommandAliases,
  setCommandCatalog,
  splitCommandText,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import {
  ARGUMENT_COMMANDS,
  CAPABILITY_GATES,
  DOCUMENT_SCOPED_COMMANDS,
  GESTURE_ONLY_COMMANDS,
  LOCAL_CHROME_COMMANDS,
  SELECTION_REQUIREMENTS,
} from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { formatCommandLogEntry } from '../src/commands/command-log.ts';
import { PALETTE_COMMANDS, RIBBON_TABS, offeredItems, offeredPanels } from '../src/ribbon/ribbon-model.ts';
import { createTranslator } from '../src/i18n.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');
const tzh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');

// The router vocabulary that carries the two names. Admission — never a local
// flag — is what enables the affordances. `AREA OBJECT` is one entry: the host
// canonicalises the whole line, so the mirror must keep the space in one name.
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

function panel(tabId, panelId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const found = tab.panels.find(entry => entry.id === panelId);
  assert.ok(found, `${tabId} must keep its ${panelId} panel`);
  return found;
}

function iconNames(source) {
  const map = source.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('AREA OBJECT is catalogued as the selection-based area variant', async () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'AREA OBJECT');
  assert.deepEqual(
    { ...entry },
    { id: 'AREA OBJECT', labelKey: 'measure.kind.area_object', icon: 'area', group: 'measure' },
  );
  // It sits in the measurement run beside the picked-boundary AREA, and no shipped
  // row moved: ID keeps its "last measurement kind" slot.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf('AREA OBJECT'), order.indexOf('AREA') + 1);
  assert.ok(order.indexOf('AREA OBJECT') < order.indexOf('ID'));
  assert.ok(order.indexOf('AREA OBJECT') < order.indexOf('DIMLINEAR'));
  // The glyph is the measurement one and it really exists (an unknown name would
  // silently render the dot fallback).
  assert.ok(iconNames(await readModule('icons.tsx')).has(entry.icon));

  // A multi-word id is one *name* to the catalogue, the host vocabulary and the
  // lookup helpers — the space is not a separator anywhere in that path.
  assert.equal(normalizeCommandText('  area object '), 'AREA OBJECT');
  assert.equal(commandById('area object').id, 'AREA OBJECT');
  assert.equal(isSupportedCommand('area object'), true);
  // Nothing about it is chrome-local, gated by a host capability, a canvas
  // gesture, a typed argument or document-scoped.
  assert.equal(LOCAL_CHROME_COMMANDS.includes('AREA OBJECT'), false);
  assert.equal(CAPABILITY_GATES['AREA OBJECT'], undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes('AREA OBJECT'), false);
  assert.equal(ARGUMENT_COMMANDS['AREA OBJECT'], undefined);
  assert.equal(DOCUMENT_SCOPED_COMMANDS.includes('AREA OBJECT'), false);

  // The completion list really offers it, and only because the host reported it.
  const rows = filterCommandSuggestions('AREA OBJECT');
  const row = rows.find(suggestion => suggestion.id === 'AREA OBJECT');
  assert.ok(row, 'the multi-word id is offered as a completion row');
  assert.equal(row.supported, true);
  assert.equal(row.target, undefined, 'a canonical row, not an alias row');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('AREA OBJECT'), false);
  setCommandCatalog(HOST_NAMES);
});

test('a multi-word id is admitted whole before the argument grammar (host order)', () => {
  // The parser still reads the second word as a typed argument — that split is the
  // grammar for `ZOOM 2`/`SNAPUNIT 10` and must not change.
  assert.deepEqual(splitCommandText('AREA OBJECT'), {
    name: 'AREA',
    argument: 'OBJECT',
    text: 'AREA OBJECT',
  });
  assert.equal(ARGUMENT_COMMANDS.AREA, undefined, 'AREA takes no typed argument');
  // Admission resolves the **whole** line first, exactly like the host router, so a
  // canonical selector that contains a space is never refused as "AREA with a bad
  // argument". This used to be a real gap (it also hid `MEASURE AREA`), reported by
  // the UI lane and fixed as one shared rule rather than a per-id exception.
  assert.equal(admitCommandText('AREA OBJECT', { status: DOC }).kind, 'unavailable', 'no object selected');
  assert.equal(admitCommandText('AREA OBJECT', { status: selected(1) }).kind, 'admitted');
  // The rule is general, not taught to `AREA OBJECT`: a temporary vocabulary that
  // knows a multi-word name admits it the same way.
  setCommandCatalog([...HOST_NAMES, 'MEASURE AREA']);
  assert.deepEqual(splitCommandText('MEASURE AREA').name, 'MEASURE');
  assert.equal(admitCommandText('MEASURE AREA', { status: DOC }).kind, 'admitted');
  setCommandCatalog(HOST_NAMES);
  // A line whose whole text is not a command still goes through the argument
  // grammar: an argument-taking selector keeps working, and a stray second word on
  // a selector that takes none is still refused before IPC.
  assert.equal(admitCommandText('SNAPUNIT 10', { status: DOC }).kind, 'admitted');
  assert.deepEqual(admitCommandText('LINE 2', { status: DOC }), {
    kind: 'unsupported',
    command: 'LINE 2',
    reasonKey: 'command.unsupported',
  });
  // Nothing is admitted on a guess: with no loaded vocabulary the whole line is
  // unknown and the answer is the shared refusal.
  setCommandCatalog([]);
  assert.deepEqual(admitCommandText('AREA OBJECT', { status: selected(1) }), {
    kind: 'unsupported',
    command: 'AREA OBJECT',
    reasonKey: 'command.unsupported',
  });
  setCommandCatalog(HOST_NAMES);
  // `ZOOM` itself stays admissible with its argument, so nothing above weakened
  // the argument path.
  assert.equal(admitCommandText('ZOOM 2', { status: DOC }).kind, 'admitted');
});

test('the AREA button carries AREA OBJECT as a variant now that admission is whole-line', () => {
  const measure = panel('measure', 'measure');
  const area = measure.items.find(item => item.id === 'AREA');
  assert.ok(area, 'the Measure panel keeps its AREA button');
  // AutoCAD keeps Object mode inside the Area split button; now that admission
  // resolves the whole line first, the row is live instead of permanently disabled.
  const variants = (area.variants ?? []).map(variant => variant.command);
  assert.deepEqual(variants, ['AREA', 'AREA OBJECT']);
  // The panel still does not dispatch it behind the reader's back as its own button.
  assert.equal(
    measure.items.some(item => item.command === 'AREA OBJECT'),
    false,
    'no hidden AREA OBJECT button',
  );
  // Exactly one place in the whole ribbon offers it — the AREA popover above — so a
  // second copy can never drift out of sync with the enablement rule.
  const carriers = RIBBON_TABS.flatMap(tab => tab.panels)
    .flatMap(section => section.items)
    .reduce((count, item) => count
      + (item.command === 'AREA OBJECT' ? 1 : 0)
      + (item.variants ?? []).filter(variant => variant.command === 'AREA OBJECT').length, 0);
  assert.equal(carriers, 1, 'AREA OBJECT is offered exactly once, as the AREA variant');
});

test('AREA OBJECT needs one selected object and words the rule in its own hint', () => {
  // The table states a *minimum*; it cannot express "exactly one". Two selected
  // objects are therefore let through here and refused by the host **by name** —
  // the same division of labour every selection rule uses.
  assert.deepEqual({ ...SELECTION_REQUIREMENTS['AREA OBJECT'] }, {
    count: 1,
    reasonKey: 'measure.kind.area_object.hint',
  });
  assert.deepEqual({ ...selectionRequirement('area object') }, {
    count: 1,
    reasonKey: 'measure.kind.area_object.hint',
  });
  assert.equal(selectionRequirementMet('AREA OBJECT', DOC), false, 'nothing selected');
  assert.equal(selectionRequirementMet('AREA OBJECT', selected(0)), false);
  assert.equal(selectionRequirementMet('AREA OBJECT', selected(1)), true);
  assert.equal(selectionRequirementMet('AREA OBJECT', selected(2)), true, 'the upper bound is the host\'s');
  assert.equal(selectionRequirementMet('AREA OBJECT', NO_DOCUMENT), false, 'no drawing, no selection block');
  // The disabled-button hint resolves through the same table, so the sentence the
  // reader sees is the one-object rule and not "unsupported".
  assert.equal(unavailableReasonKey('AREA OBJECT', { status: DOC }), 'measure.kind.area_object.hint');
  assert.equal(unavailableReasonKey('AREA OBJECT', { status: NO_DOCUMENT }), 'plot.no_document');
});

test('LAYERP is its own selector, not chrome-local and not selection-gated', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'LAYERP');
  assert.deepEqual({ ...entry }, {
    id: 'LAYERP',
    labelKey: 'layers.previous',
    icon: 'eye',
    group: 'annotate',
  });
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf('LAYERP'), order.indexOf('LAYON') + 1, 'LAYERP closes the layer run');
  // Its own label, never the restore/`on` wording it used to borrow as an alias.
  assert.notEqual(entry.labelKey, 'layers.restore');
  assert.notEqual(entry.labelKey, 'draw.kind.layon');

  // A router command: the host vocabulary alone admits it.
  assert.equal(LOCAL_CHROME_COMMANDS.includes('LAYERP'), false);
  assert.equal(CAPABILITY_GATES['LAYERP'], undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes('LAYERP'), false);
  assert.equal(ARGUMENT_COMMANDS['LAYERP'], undefined);
  assert.equal(DOCUMENT_SCOPED_COMMANDS.includes('LAYERP'), false);
  assert.equal(isSupportedCommand('layerp'), true);

  // It restores a state, so no selection is ever the reason it cannot run; only a
  // missing drawing is (its `count: 0` entry is the "no document" answer only).
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.LAYERP }, { count: 0, reasonKey: 'draw.error.selection_required' });
  for (const status of [DOC, selected(0), selected(5)]) {
    assert.equal(admitCommandText('LAYERP', { status }).kind, 'admitted');
  }
  assert.equal(admitCommandText('LAYERP', { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
  assert.equal(unavailableReasonKey('LAYERP', { status: NO_DOCUMENT }), 'plot.no_document');
  // The empty host vocabulary is the one honest disable.
  setCommandCatalog([]);
  assert.equal(admitCommandText('LAYERP', { status: DOC }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('LAYERP is not an alias of the restore command in the UI vocabulary', () => {
  // The chrome owns no alias table of its own: `cad_command_aliases` is the host's
  // answer, and this build must not invent a `LAYERP → RESTORELAYERS` row.
  setCommandAliases([{ alias: 'L', command: 'LINE' }]);
  assert.equal(aliasTarget('LAYERP'), null, 'the UI invents no LAYERP alias row');
  assert.equal(aliasTarget('layerp'), null);
  setCommandAliases([]);
  assert.equal(aliasTarget('LAYERP'), null);
  // It is a canonical row of the vocabulary mirror, not a shorthand, and its
  // completion row therefore stands for itself (no `target`).
  assert.ok(HOST_NAMES.includes('LAYERP'), 'the router vocabulary carries LAYERP as its own name');
  const row = filterCommandSuggestions('LAYERP').find(suggestion => suggestion.id === 'LAYERP');
  assert.ok(row, 'LAYERP is suggested by its own name');
  assert.equal(row.target, undefined, 'not an alias row');
  assert.equal(row.labelKey, 'layers.previous');
  assert.equal(row.supported, true);
});

test('LAYERP sits beside LAYUNISO in the Layers panel and rides the palette filter', async () => {
  const layers = panel('home', 'layers');
  // Palette opener → isolate pair → previous state → off/on, appending nothing that
  // moves a shipped button beyond the two the run already ends with.
  assert.deepEqual(
    layers.items.map(item => item.id),
    ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON'],
  );
  const layerp = layers.items.find(item => item.id === 'LAYERP');
  assert.equal(layerp.command, 'LAYERP');
  assert.equal(layerp.labelKey, 'layers.previous');
  assert.equal(layerp.icon, 'eye');
  assert.equal(layerp.large, undefined, 'not a primary affordance');
  assert.equal(layerp.action, undefined, 'a router command, never a shell action');
  assert.equal(layerp.standardView, undefined, 'it carries no payload');
  assert.equal(layerp.variants, undefined, 'no options sub-prompt');
  // Exactly one dispatch token, the model's own union contract.
  assert.deepEqual(
    [layerp.command, layerp.standardView, layerp.action].filter(token => token !== undefined),
    ['LAYERP'],
  );
  assert.ok(iconNames(await readModule('icons.tsx')).has(layerp.icon));
  // One button only: no other panel dispatches it.
  for (const tab of RIBBON_TABS) {
    for (const section of tab.panels) {
      if (section === layers) continue;
      assert.equal(
        section.items.some(item => item.command === 'LAYERP'),
        false,
        `${tab.id}/${section.id} must not dispatch LAYERP`,
      );
    }
  }

  // The state it restores is only named in the palette, so the button is dropped
  // with it: no live affordance that changes layer visibility invisibly.
  assert.deepEqual([...PALETTE_COMMANDS], ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON']);
  assert.deepEqual(offeredItems(layers.items, false), []);
  assert.equal(offeredItems(layers.items, true), layers.items);
  assert.equal(
    offeredPanels(RIBBON_TABS.find(tab => tab.id === 'home').panels, false).some(each => each.id === 'layers'),
    false,
  );
});

test('the area_object token echoes as an area row, and unknown tokens still print raw', () => {
  const row = (algorithm, unit, precision = 'analytic') => formatCommandLogEntry({
    sequence: 4,
    kind: 'result',
    measurement: { algorithm, value: 12.5, unit, precision },
  }, t);
  // The selection variant is an **area**: same label, same squared unit — never a
  // bare number that could be read as a length.
  const objectArea = row('area_object', 'mm');
  assert.deepEqual(objectArea.lines[0], {
    tone: 'value',
    label: en['command.log.area'],
    text: '12.5000',
    unit: en['command.log.unit_squared'].replace('{unit}', 'mm'),
    meta: null,
  });
  // `unit: null` is the host's "drawing units" statement, squared like any area.
  const drawingUnits = formatCommandLogEntry({
    sequence: 5,
    kind: 'result',
    measurement: { algorithm: 'area_object', value: 3, unit: null, precision: 'analytic' },
  }, tzh);
  assert.equal(drawingUnits.lines[0].label, zh['command.log.area']);
  assert.equal(drawingUnits.lines[0].unit, zh['command.log.unit_squared'].replace('{unit}', zh['units.unknown']));
  assert.equal(drawingUnits.lines[0].unit, '图纸单位\u00B2');
  // A precision chip still rides along, exactly as on the picked-boundary row.
  assert.equal(row('area_object', 'mm', 'approximate').lines[0].meta, en['command.log.precision.approximate']);

  // The unknown-token rule is unchanged: anything this build does not know is
  // printed verbatim with the plain unit rule (never a squared one, never a label
  // it did not measure).
  const unknown = row('volume', 'mm');
  assert.equal(unknown.lines[0].label, 'volume');
  assert.equal(unknown.lines[0].unit, 'mm');
  assert.equal(row('volume', null).lines[0].unit, en['units.unknown']);
  // And it is one line, not a fabricated group.
  assert.equal(unknown.lines.length, 1);
});

test('the new labels come from the shared catalogs, in both languages', () => {
  const keys = ['measure.kind.area_object', 'measure.kind.area_object.hint', 'layers.previous'];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
  }
  // The area variant takes the current selection and says so: it is our stand-in
  // for AutoCAD's interactive Object pick, and no copy may claim that picking.
  assert.match(en['measure.kind.area_object.hint'], /selection/i);
  assert.match(en['measure.kind.area_object.hint'], /does not pick objects interactively/i);
  assert.match(en['measure.kind.area_object.hint'], /closed object/i);
  assert.match(zh['measure.kind.area_object.hint'], /选择/);
  assert.match(zh['measure.kind.area_object.hint'], /不做交互式对象选取/);
  assert.match(zh['measure.kind.area_object.hint'], /闭合对象/);
  // The layer label names the previous state, not "restore everything".
  assert.match(en['layers.previous'], /previous/i);
  assert.match(zh['layers.previous'], /上一/);
});

test('the renderer dispatches both names through the one command funnel', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  // No per-button local gate and no hardcoded selector: the model drives them.
  for (const id of ['AREA OBJECT', 'LAYERP']) {
    assert.equal(ribbon.includes(`'${id}'`), false, `${id} must be driven by the model, not by an id check`);
  }
});
