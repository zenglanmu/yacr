// ID / JOIN / LAYISO / LAYUNISO / LAYOFF / LAYON user-facing surface (2026-10-26).
//
// Four additions, each with the same contract as its AutoCAD neighbour:
//
//   * `ID` is the **fifth measurement kind**: one point pick, echoed as X/Y/Z in
//     the command window (`command.log.point`). It sits in the Measure panel with
//     Distance/Angle/Area, the way AutoCAD's Measure group keeps the coordinate
//     readout beside the distance tool.
//   * `JOIN` is a **modify** command that welds the current selection into one
//     polyline, so it is a selection precondition of at least two objects and it
//     lives in the Modify panel (AutoCAD's own slot, last in the panel).
//   * `LAYISO`/`LAYUNISO` are **router** layer commands (not chrome-local
//     selectors like `LAYER`): isolate needs something selected, unisolate needs
//     nothing, and both need a drawing open.
//   * `LAYOFF`/`LAYON` (2026-10-26) close that layer run with the same contract:
//     `LAYOFF` turns off every layer the selection sits on (one object minimum,
//     the current layer excepted), `LAYON` turns every layer back on (no
//     selection). Both are router commands, and both are dropped with the layers
//     palette on a carrier that never renders it.
//
// Nothing here may imply an unimplemented option: ID has no options sub-prompt,
// JOIN has no settings prompt and the layer commands have no settings dialog, so
// none of them carries a variant popover. Every user-visible string comes from the
// shared catalogs; the item ids, the dispatched selectors and the catalog entries
// are asserted together so a ribbon button and a catalogue row cannot drift apart.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ARGUMENT_COMMANDS,
  CAPABILITY_GATES,
  COMMAND_CATALOG,
  DOCUMENT_SCOPED_COMMANDS,
  GESTURE_ONLY_COMMANDS,
  LOCAL_CHROME_COMMANDS,
  SELECTION_REQUIREMENTS,
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import {
  PALETTE_COMMANDS,
  RIBBON_TABS,
  offeredItems,
  offeredPanels,
  ribbonCommandIds,
} from '../src/ribbon/ribbon-model.ts';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { readAppSource, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router vocabulary that carries the new selectors. Admission — not a local
// flag — is what enables the buttons.
const HOST_NAMES = ['ID', 'JOIN', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON'];
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

test('ID is the fifth Measure button, a router command, and a catalogue row', async () => {
  const measure = panel('measure', 'measure');
  // Appended after the four shipped kinds, so no existing button moved, and it
  // keeps their conventions: a large entry with no variant popover.
  assert.deepEqual(measure.items.map(item => item.id), ['DIST', 'PLINE', 'ANGLE', 'AREA', 'ID']);
  const id = measure.items.at(-1);
  assert.equal(id.command, 'ID');
  assert.equal(id.labelKey, 'measure.kind.id');
  assert.equal(id.icon, 'point');
  assert.equal(id.large, true, 'the Measure panel has no small-entry row');
  assert.equal(id.variants, undefined, 'ID has no options sub-prompt');
  assert.equal(id.action, undefined, 'ID must dispatch through the router');
  assert.equal(id.standardView, undefined, 'ID carries no payload');

  // The catalogue mirrors it, and the measure group stays contiguous.
  const entry = COMMAND_CATALOG.find(command => command.id === 'ID');
  assert.deepEqual({ ...entry }, { id: 'ID', labelKey: 'measure.kind.id', icon: 'point', group: 'measure' });
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.ok(order.indexOf('ID') > order.indexOf('AREA'));
  assert.ok(order.indexOf('ID') < order.indexOf('DIMLINEAR'), 'ID stays in the measurement run');
  assert.ok(new Set(ribbonCommandIds()).has('ID'));

  // Router admission is the single gate — never a chrome-local selector.
  assert.equal(LOCAL_CHROME_COMMANDS.includes('ID'), false);
  assert.equal(CAPABILITY_GATES.ID, undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes('ID'), false);
  assert.equal(ARGUMENT_COMMANDS.ID, undefined, 'ID takes no typed argument');
  assert.equal(isSupportedCommand('id'), true);
  assert.equal(admitCommandText('ID', { status: DOC }).kind, 'admitted');
  assert.equal(admitCommandText('ID', { status: null }).kind, 'unavailable');
  setCommandCatalog([]);
  assert.equal(admitCommandText('ID', { status: DOC }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);

  // The glyph exists: an unknown name would silently render the dot fallback.
  assert.ok(iconNames(await readModule('icons.tsx')).has(id.icon));
});

test('the ID tool chip names its one pick and never claims a dimension', () => {
  // The host keys the tool `measure.id`, which the chip resolves to the catalogue.
  assert.equal(toolLabelKey('measure.id'), 'tool.measure.id');
  // A single-point capture that commits itself: exactly one named step.
  assert.deepEqual(TOOL_STEP_PROMPTS['measure.id'], ['tool.prompt.point']);
  assert.equal(toolStepPromptKey({ key: 'measure.id', points: 0, ready: false }), 'tool.prompt.point');
  // Once the pick is counted the capture is complete. The shared dimension ready
  // copy ("Dimension defined") would be false, so the override is the generic
  // "Enter to confirm" sentence and never the dimension one.
  assert.equal(toolStepPromptKey({ key: 'measure.id', points: 1, ready: true }), 'tool.prompt.confirm');
  assert.notEqual(TOOL_READY_PROMPTS['measure.id'], 'tool.prompt.dim.ready');
  // Not a pre-selection tool: the boundary it reports is the pick itself.
  assert.equal(PRESELECT_PROMPTS['measure.id'], undefined);
  // No prompt the tool shows names a dimension, an open-ended capture or a finish
  // verb it does not have.
  const prompts = [TOOL_STEP_PROMPTS['measure.id'][0], TOOL_READY_PROMPTS['measure.id']];
  for (const key of prompts) {
    assert.equal(typeof key, 'string');
    assert.equal(/dimension/i.test(en[key]), false, `${key} must not name a dimension`);
  }
});

test('JOIN closes the Modify panel on both tabs and is a router command', async () => {
  for (const tabId of ['home', 'draw']) {
    const modify = panel(tabId, 'modify');
    // AutoCAD's Modify panel ends with Join; appending keeps every shipped button
    // where it was.
    const join = modify.items.at(-1);
    assert.equal(join.id, 'JOIN', tabId);
    assert.equal(join.command, 'JOIN');
    assert.equal(join.labelKey, 'draw.kind.join');
    assert.equal(join.icon, 'polyline');
    assert.equal(join.large, undefined, `${tabId}: JOIN is not a primary edit`);
    assert.equal(join.variants, undefined, 'JOIN has no settings prompt');
    assert.equal(join.action, undefined, 'JOIN must dispatch through the router');
    assert.equal(join.standardView, undefined, 'JOIN carries no payload');
  }
  const entry = COMMAND_CATALOG.find(command => command.id === 'JOIN');
  assert.deepEqual({ ...entry }, { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', group: 'modify' });
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.ok(order.indexOf('JOIN') > order.indexOf('MATCHPROP'), 'JOIN closes the modify run');
  assert.ok(order.indexOf('JOIN') < order.indexOf('DIST'), 'and does not leak into the measure run');
  assert.ok(new Set(ribbonCommandIds()).has('JOIN'));
  assert.ok(iconNames(await readModule('icons.tsx')).has(entry.icon));
  // No other panel dispatches it.
  for (const tab of RIBBON_TABS) {
    for (const each of tab.panels) {
      if (each.id === 'modify') continue;
      assert.equal(each.items.some(item => item.command === 'JOIN'), false, `${tab.id}/${each.id}`);
    }
  }
});

test('JOIN needs at least two selected objects, and says so before the host refuses', () => {
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.JOIN }, { count: 2, reasonKey: 'draw.kind.join.hint' });
  assert.equal(admitCommandText('JOIN', { status: selected(2) }).kind, 'admitted');
  for (const count of [0, 1]) {
    assert.deepEqual(admitCommandText('JOIN', { status: selected(count) }), {
      kind: 'unavailable',
      command: 'JOIN',
      reasonKey: 'draw.kind.join.hint',
    });
  }
  // With no drawing open the honest sentence is the missing drawing, not "select
  // more objects" — there is no selection block to count.
  assert.equal(admitCommandText('JOIN', { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
  // The ribbon's disabled hint resolves through the very same table.
  assert.equal(unavailableReasonKey('JOIN', { status: selected(1) }), 'draw.kind.join.hint');
  assert.equal(unavailableReasonKey('JOIN', { status: NO_DOCUMENT }), 'plot.no_document');
  // It is not document-scoped: the selection requirement already answers that.
  assert.equal(admitCommandText('JOIN', { status: { ...DOC, loading: true } }).reasonKey, 'linux.busy');
});

test('LAYISO and LAYUNISO sit beside LAYER and ride the palette carrier filter', async () => {
  const layers = panel('home', 'layers');
  // LAYOFF/LAYON (2026-10-26) close the same run; they have their own test below.
  assert.deepEqual(layers.items.map(item => item.id), ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON']);
  const [layer, isolate, unisolate] = layers.items;
  assert.equal(layer.large, true, 'the palette opener keeps the large slot');
  for (const item of [isolate, unisolate]) {
    assert.equal(item.large, undefined, `${item.id} is not a primary affordance`);
    assert.equal(item.action, undefined, `${item.id} is a router command, never a shell action`);
    assert.equal(item.command, item.id, `${item.id} dispatches its own selector`);
    assert.equal(item.standardView, undefined, `${item.id} carries no payload`);
    assert.equal(item.variants, undefined, `${item.id} has no options sub-prompt`);
  }
  assert.equal(isolate.labelKey, 'layers.isolate');
  assert.equal(unisolate.labelKey, 'layers.unisolate');
  // The glyph pair is the palette's own visibility vocabulary: isolating hides the
  // other layers, unisolating brings them back.
  assert.equal(isolate.icon, 'eyeOff');
  assert.equal(unisolate.icon, 'eye');
  const icons = iconNames(await readModule('icons.tsx'));
  assert.ok(icons.has(isolate.icon) && icons.has(unisolate.icon));

  // Catalogue mirror, in the layer run next to LAYER.
  for (const [id, labelKey, icon] of [
    ['LAYISO', 'layers.isolate', 'eyeOff'],
    ['LAYUNISO', 'layers.unisolate', 'eye'],
  ]) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.deepEqual({ ...entry }, { id, labelKey, icon, group: 'annotate' }, id);
    assert.ok(new Set(ribbonCommandIds()).has(id), id);
  }
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf('LAYISO'), order.indexOf('LAYER') + 1);
  assert.equal(order.indexOf('LAYUNISO'), order.indexOf('LAYER') + 2);

  // A carrier that never renders the layers palette has no surface for the
  // isolation state, so the whole panel (toggle + isolate pair + the off/on pair) is
  // dropped — never left behind as live buttons that hide geometry invisibly.
  assert.deepEqual([...PALETTE_COMMANDS], ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON']);
  assert.deepEqual(offeredItems(layers.items, false), []);
  assert.equal(offeredPanels(RIBBON_TABS.find(tab => tab.id === 'home').panels, false)
    .some(each => each.id === 'layers'), false);
  // A carrier with the palettes renders exactly the same array.
  assert.equal(offeredItems(layers.items, true), layers.items);
});

test('isolate needs a selection, unisolate needs none, both need a drawing', () => {
  // Router commands, not chrome-local selectors: `LAYER` answers itself, these do
  // not.
  for (const id of ['LAYISO', 'LAYUNISO']) {
    assert.equal(LOCAL_CHROME_COMMANDS.includes(id), false, id);
    assert.equal(CAPABILITY_GATES[id], undefined, id);
    assert.equal(GESTURE_ONLY_COMMANDS.includes(id), false, id);
    assert.equal(ARGUMENT_COMMANDS[id], undefined, `${id} takes no typed argument`);
    assert.equal(isSupportedCommand(id.toLowerCase()), true, id);
  }
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.LAYISO }, { count: 1, reasonKey: 'draw.error.selection_required' });
  assert.equal(admitCommandText('LAYISO', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('LAYISO', { status: selected(3) }).kind, 'admitted');
  // Isolating nothing would hide every layer with no way back from the canvas.
  assert.deepEqual(admitCommandText('LAYISO', { status: DOC }), {
    kind: 'unavailable',
    command: 'LAYISO',
    reasonKey: 'draw.error.selection_required',
  });
  assert.equal(unavailableReasonKey('LAYISO', { status: DOC }), 'draw.error.selection_required');

  // Unisolate restores the drawing, so no selection is ever the reason it cannot
  // run; only a missing drawing is.
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.LAYUNISO }, { count: 0, reasonKey: 'draw.error.selection_required' });
  for (const status of [DOC, selected(0), selected(5)]) {
    assert.equal(admitCommandText('LAYUNISO', { status }).kind, 'admitted');
  }
  assert.equal(admitCommandText('LAYUNISO', { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
  assert.equal(unavailableReasonKey('LAYUNISO', { status: NO_DOCUMENT }), 'plot.no_document');

  // Neither is admitted on an empty host vocabulary, and neither is document-
  // scoped by the chrome (`SELECTION_REQUIREMENTS` already answers that case).
  setCommandCatalog([]);
  for (const id of ['LAYISO', 'LAYUNISO']) {
    assert.equal(admitCommandText(id, { status: selected(2) }).kind, 'unsupported', id);
  }
  setCommandCatalog(HOST_NAMES);
});

test('every new label comes from the shared catalogs, in both languages', () => {
  // The keys the Rust lane owns; this test is the UI-side proof that the surface
  // never invents copy of its own.
  const keys = [
    'measure.kind.id',
    'tool.measure.id',
    'draw.kind.join',
    'draw.kind.join.hint',
    'layers.isolate',
    'layers.unisolate',
  ];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
  }
});

test('the renderer dispatches every new selector through the one command funnel', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  // No per-button local gate and no hardcoded selector for the new items.
  for (const id of ['ID', 'JOIN', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON']) {
    assert.equal(ribbon.includes(`'${id}'`), false, `${id} must be driven by the model, not by an id check`);
  }
});

test('LAYOFF and LAYON close the Layers panel and ride the same carrier filter', async () => {
  const layers = panel('home', 'layers');
  // Directly after the isolate pair, so the run reads palette opener → isolate →
  // restore → off → on. Appending keeps every shipped button where it was.
  assert.deepEqual(layers.items.map(item => item.id), ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON']);
  const [off, on] = layers.items.slice(-2);
  assert.equal(off.id, 'LAYOFF');
  assert.equal(on.id, 'LAYON');
  for (const item of [off, on]) {
    assert.equal(item.large, undefined, `${item.id} is not a primary affordance`);
    assert.equal(item.command, item.id, `${item.id} dispatches its own selector`);
    assert.equal(item.action, undefined, `${item.id} is a router command, never a shell action`);
    assert.equal(item.standardView, undefined, `${item.id} carries no payload`);
    assert.equal(item.variants, undefined, `${item.id} has no options sub-prompt`);
    // Exactly one dispatch token, the model's own union contract: a button with
    // two lanes would have no defined order.
    assert.deepEqual(
      [item.command, item.standardView, item.action].filter(token => token !== undefined),
      [item.id],
      `${item.id} carries exactly one dispatch token`,
    );
  }
  // The labels are the Rust lane's catalogue keys, never UI-invented copy.
  assert.equal(off.labelKey, 'draw.kind.layoff');
  assert.equal(on.labelKey, 'draw.kind.layon');
  // Same visibility vocabulary as the isolate pair: turning a layer off hides its
  // geometry (dimmed eye), turning every layer back on shows it again (plain eye).
  assert.equal(off.icon, 'eyeOff');
  assert.equal(on.icon, 'eye');
  const icons = iconNames(await readModule('icons.tsx'));
  assert.ok(icons.has(off.icon) && icons.has(on.icon), 'both glyphs must exist in icons.tsx');

  // Catalogue mirror: the same group the isolate pair uses, and the ribbon really
  // references both.
  for (const [id, labelKey, icon] of [
    ['LAYOFF', 'draw.kind.layoff', 'eyeOff'],
    ['LAYON', 'draw.kind.layon', 'eye'],
  ]) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.deepEqual({ ...entry }, { id, labelKey, icon, group: 'annotate' }, id);
    assert.ok(new Set(ribbonCommandIds()).has(id), id);
  }
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf('LAYOFF'), order.indexOf('LAYUNISO') + 1);
  assert.equal(order.indexOf('LAYON'), order.indexOf('LAYUNISO') + 2);

  // One button each: no other panel dispatches them.
  for (const tab of RIBBON_TABS) {
    for (const each of tab.panels) {
      if (each === layers) continue;
      for (const id of ['LAYOFF', 'LAYON']) {
        assert.equal(each.items.some(item => item.command === id), false, `${tab.id}/${each.id} must not dispatch ${id}`);
      }
    }
  }

  // The carrier rule is "is its effect only visible in the layers palette": turning
  // the selection's layers off hides geometry and the palette is the only place
  // either state is named or reversed, so both go with the palette.
  assert.ok(PALETTE_COMMANDS.includes('LAYOFF') && PALETTE_COMMANDS.includes('LAYON'));
  assert.deepEqual(offeredItems(layers.items, false), []);
  assert.equal(offeredItems(layers.items, true), layers.items);
});

test('LAYOFF needs a selection, LAYON needs none, both need a drawing', () => {
  // Router commands, not chrome-local selectors: `LAYER` answers itself, these do
  // not, and neither has a capability gate or a gesture-only refusal.
  for (const id of ['LAYOFF', 'LAYON']) {
    assert.equal(LOCAL_CHROME_COMMANDS.includes(id), false, id);
    assert.equal(CAPABILITY_GATES[id], undefined, id);
    assert.equal(GESTURE_ONLY_COMMANDS.includes(id), false, id);
    assert.equal(ARGUMENT_COMMANDS[id], undefined, `${id} takes no typed argument`);
    // The selection requirement already answers the no-document case.
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), false, id);
    assert.equal(isSupportedCommand(id.toLowerCase()), true, id);
  }
  // Turning a layer off means turning off the layers the selection sits on: with
  // nothing selected there is no layer to act on, and the host refuses that
  // outright. The sentence is the catalogue's own "Select objects first", exactly
  // like LAYISO — no second copy of the same fact is invented.
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.LAYOFF }, { count: 1, reasonKey: 'draw.error.selection_required' });
  assert.equal(admitCommandText('LAYOFF', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('LAYOFF', { status: selected(3) }).kind, 'admitted');
  assert.deepEqual(admitCommandText('LAYOFF', { status: DOC }), {
    kind: 'unavailable',
    command: 'LAYOFF',
    reasonKey: 'draw.error.selection_required',
  });
  assert.equal(unavailableReasonKey('LAYOFF', { status: DOC }), 'draw.error.selection_required');
  // With no drawing open the honest sentence is the missing drawing, not "select
  // objects first" — there is no selection block to count.
  assert.equal(admitCommandText('LAYOFF', { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
  assert.equal(unavailableReasonKey('LAYOFF', { status: NO_DOCUMENT }), 'plot.no_document');

  // LAYON restores every layer, so no selection is ever the reason it cannot run;
  // only a missing drawing is.
  assert.deepEqual({ ...SELECTION_REQUIREMENTS.LAYON }, { count: 0, reasonKey: 'draw.error.selection_required' });
  for (const status of [DOC, selected(0), selected(5)]) {
    assert.equal(admitCommandText('LAYON', { status }).kind, 'admitted');
  }
  assert.equal(admitCommandText('LAYON', { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
  assert.equal(unavailableReasonKey('LAYON', { status: NO_DOCUMENT }), 'plot.no_document');

  // Neither is admitted on an empty host vocabulary: the router's own answer is the
  // gate, never a local enable.
  setCommandCatalog([]);
  for (const id of ['LAYOFF', 'LAYON']) {
    assert.equal(admitCommandText(id, { status: selected(2) }).kind, 'unsupported', id);
  }
  setCommandCatalog(HOST_NAMES);
});

test('every LAYOFF/LAYON label comes from the shared catalogs, in both languages', () => {
  // The keys the Rust lane owns. This test is deliberately the UI-side marker that
  // the surface never invents copy of its own: while a key is missing the UI
  // references a label it cannot render, and the honest state is a failing check,
  // never a literal in the React tree.
  const keys = ['draw.kind.layoff', 'draw.kind.layon'];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(String(en[key] ?? '').trim().length > 0, `en empty ${key}`);
    assert.ok(String(zh[key] ?? '').trim().length > 0, `zh-CN empty ${key}`);
  }
  // The wording names the layer operation (not the selection): "Turn layer off" /
  // "Turn all layers on".
  assert.match(String(en['draw.kind.layoff'] ?? ''), /layer/i);
  assert.match(String(en['draw.kind.layon'] ?? ''), /layer/i);
});

test('no rendered copy still claims the retired OFFSET/MIRROR entity limits', async () => {
  // The shipped engine went past the old limits (2026-10-26: OFFSET also takes
  // straight-segment polylines, MIRROR also takes Arc and Ellipse). Any *rendered*
  // sentence that still says otherwise would be a lie in the affordance, so the
  // claim shapes are swept over every catalog string this tree can display: the
  // keys the components pass to `t(...)` plus every catalogue `labelKey` the
  // command/ribbon models carry.
  //
  // The scan is on the **rendered text**, not on module comments: prose in a source
  // comment explains history and is never shown to the user.
  const source = await readAppSource();
  const keys = new Set();
  for (const match of source.matchAll(/\bt\(\s*'([^']+)'/g)) keys.add(match[1]);
  for (const match of source.matchAll(/labelKey:\s*'([^']+)'/g)) keys.add(match[1]);
  assert.ok(keys.size > 20, `expected the tree to render many catalogue keys, saw ${keys.size}`);
  const retired = [
    // "OFFSET supports LINE and CIRCLE only" / "only a line or circle can be offset"
    /\bonly\b[^.]{0,60}\b(?:LINE|CIRCLE)\b/i,
    /\b(?:LINE|CIRCLE)\b[^.]{0,40}\bonly\b/i,
    // "MIRROR cannot mirror an Arc/Ellipse" / "refuses an arc or ellipse"
    /cannot\s+mirror/i,
    /(?:refuses?|not\s+supported\s+for)\s+(?:an?\s+)?(?:ARC|ELLIPSE)/i,
    // "an Arc/Ellipse cannot be offset/mirrored"
    /(?:ARC|ELLIPSE)[^.]{0,40}\b(?:cannot|is\s+not|are\s+not)\b[^.]{0,30}\b(?:offset|mirrored|mirror)\b/i,
  ];
  // The Chinese sentences would word the same claims as "仅/只…直线…" and
  // "镜像/偏移…不支持"; they are swept too, so a locale-only regression cannot hide
  // behind the English patterns. (Only the *sentence* shapes above are probed — a
  // claim in wording these patterns do not anticipate is not covered, and this is
  // not a general capability audit.)
  const retiredZh = [
    /(?:仅|只)\s*(?:支持)?\s*(?:直线|圆)/,
    /(?:镜像|偏移)[^。]{0,20}(?:不支持|无法|不能)/,
  ];
  for (const key of keys) {
    for (const [locale, catalog] of [['en', en], ['zh-CN', zh]]) {
      const text = catalog[key];
      if (typeof text !== 'string') continue;
      const claims = locale === 'en' ? retired : retiredZh;
      for (const claim of claims) {
        assert.equal(claim.test(text), false, `${locale}/${key} still claims a retired entity limit: ${text}`);
      }
    }
  }
});

