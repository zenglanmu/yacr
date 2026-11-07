// MASSPROP / MEASUREGEOM chrome contracts (AutoCAD 2026 inquiry, 2026-11-07).
//
// * MASSPROP is the selection-based, read-only mass-properties report. Its only
//   local gate is the one-object minimum; the host refuses non-coplanar/unsupported
//   kinds by name. The chrome must never claim inertia, volume or `.mpr` output.
// * MEASUREGEOM is a **router**, not a second measurement engine. Every ribbon
//   variant sends the full `MEASUREGEOM <OPTION>` line, and the local grammar
//   admits exactly the six full words the host implements. `VOLUME` is listed so
//   the host can answer with its named not-implemented code.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  splitCommandText,
} from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { createTranslator } from '../src/i18n.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');

setCommandCatalog(HOST_NAMES);

const LOADED = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};
const selected = count => ({ ...LOADED, selection: { count } });

function tab(id) {
  const found = RIBBON_TABS.find(candidate => candidate.id === id);
  assert.ok(found, `${id} tab must exist`);
  return found;
}

function panel(tabId, panelId) {
  const found = tab(tabId).panels.find(candidate => candidate.id === panelId);
  assert.ok(found, `${tabId}/${panelId} panel must exist`);
  return found;
}

test('MASSPROP is catalogued and ribbon-placed in the inquiry area', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'MASSPROP');
  assert.ok(entry, 'MASSPROP must be catalogued');
  assert.equal(entry.labelKey, 'draw.kind.massprop');
  assert.equal(entry.group, 'annotate');
  assert.equal(entry.icon, 'properties');

  // AutoCAD keeps mass properties with the inquiry/report commands; the Home
  // Properties panel is this chrome's inquiry slot (next to LIST).
  const properties = panel('home', 'properties');
  const item = properties.items.find(candidate => candidate.id === 'MASSPROP');
  assert.ok(item, 'Home/Properties must carry MASSPROP');
  assert.equal(item.command, 'MASSPROP');
  assert.equal(item.hintKey, 'draw.kind.massprop.hint');
  assert.equal(item.action, undefined);
  assert.equal(item.variants, undefined, 'MASSPROP has no option sub-prompt');
});

test('MASSPROP is gated by one selected object and admits in Viewer-like status', () => {
  assert.equal(SELECTION_REQUIREMENTS.MASSPROP.count, 1);
  assert.equal(SELECTION_REQUIREMENTS.MASSPROP.reasonKey, 'draw.kind.massprop.hint');
  assert.equal(isSupportedCommand('MASSPROP'), true);
  assert.equal(admitCommandText('MASSPROP', { status: selected(0) }).kind, 'unavailable');
  assert.equal(admitCommandText('MASSPROP', { status: selected(0) }).reasonKey, 'draw.kind.massprop.hint');
  assert.equal(admitCommandText('MASSPROP', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('MASSPROP', { status: LOADED }).reasonKey, 'draw.kind.massprop.hint');
  // It takes no typed argument.
  assert.equal(ARGUMENT_COMMANDS.MASSPROP, undefined);
  assert.equal(splitCommandText('MASSPROP 1').argument, '1');
});

test('MEASUREGEOM is catalogued as the inquiry router', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'MEASUREGEOM');
  assert.ok(entry, 'MEASUREGEOM must be catalogued');
  assert.equal(entry.labelKey, 'measure.kind.measuregeom');
  assert.equal(entry.group, 'measure');
  // It has no pre-gate: its tool options capture their own points, and only the
  // RADIUS option reads the selection (the host refuses an empty one by name).
  assert.equal(SELECTION_REQUIREMENTS.MEASUREGEOM, undefined);
});

test('the Measure ribbon split button sends the full MEASUREGEOM option line', () => {
  const measure = panel('measure', 'measure');
  const item = measure.items.find(candidate => candidate.id === 'MEASUREGEOM');
  assert.ok(item, 'Measure panel must carry MEASUREGEOM');
  assert.equal(item.command, 'MEASUREGEOM DISTANCE');
  assert.equal(item.large, true);
  const variants = item.variants ?? [];
  assert.deepEqual(
    variants.map(variant => variant.command),
    [
      'MEASUREGEOM DISTANCE',
      'MEASUREGEOM ANGLE',
      'MEASUREGEOM AREA',
      'MEASUREGEOM POLYLINE',
      'MEASUREGEOM RADIUS',
      'MEASUREGEOM VOLUME',
    ],
  );
  // Every row is a router command (never a local action) and dispatches the whole
  // line, which is what the host's typed-argument grammar reads.
  for (const variant of variants) {
    assert.equal(variant.action, undefined, variant.id);
    assert.match(variant.command, /^MEASUREGEOM [A-Z]+$/);
    assert.equal(splitCommandText(variant.command).name, 'MEASUREGEOM');
  }
});

test('the MEASUREGEOM argument grammar admits the six full words only', () => {
  const parse = ARGUMENT_COMMANDS.MEASUREGEOM.parse;
  assert.ok(parse, 'MEASUREGEOM has a local argument grammar');
  for (const argument of ['DISTANCE', 'distance', 'Angle', ' AREA ', 'POLYLINE', 'RADIUS', 'VOLUME']) {
    assert.equal(parse(argument), true, argument);
  }
  for (const refused of ['', 'D', 'DIST', 'A', 'R', 'V', 'POLY', 'RADIUS2', 'RADIUS VOLUME', 'DISTANCE 2']) {
    assert.equal(parse(refused), false, refused);
  }
  // Full lines are admitted through the base selector; the host receives the
  // whole normalized text (never only the option word).
  for (const line of [
    'MEASUREGEOM DISTANCE',
    'measuregeom radius',
    'MEASUREGEOM VOLUME',
  ]) {
    const admission = admitCommandText(line, { status: LOADED });
    assert.equal(admission.kind, 'admitted', line);
    assert.equal(admission.command, line.trim().toUpperCase());
  }
  // An unknown option and a short form never reach IPC.
  for (const line of ['MEASUREGEOM RAD', 'MEASUREGEOM D', 'MEASUREGEOM DIST']) {
    assert.equal(admitCommandText(line, { status: LOADED }).kind, 'unsupported', line);
  }
});

test('both commands have complete labels and hints in both catalogs', () => {
  for (const catalog of [zh, en]) {
    for (const key of [
      'draw.kind.massprop',
      'draw.kind.massprop.hint',
      'measure.kind.measuregeom',
      'measure.kind.measuregeom.hint',
      'measure.kind.radius',
      'measure.kind.volume',
      'measure.kind.measuregeom_volume.hint',
    ]) {
      assert.equal(typeof catalog[key], 'string', key);
      assert.ok(catalog[key].length > 0, key);
    }
    // The hint must say what is NOT implemented instead of implying it works.
    assert.match(catalog['draw.kind.massprop.hint'], /惯性矩|moments of inertia/i);
    assert.match(catalog['measure.kind.measuregeom_volume.hint'], /未实现|not implemented/i);
  }
  assert.equal(t('draw.kind.massprop'), 'Mass properties');
  assert.equal(t('measure.kind.measuregeom'), 'Measure geometry');
});
