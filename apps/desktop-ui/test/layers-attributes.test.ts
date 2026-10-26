// Layer attributes in the palette, UI half.
//
// `layers.test.ts` owns the palette's model (selector, projections, identity dot,
// the structural table writes and the frozen `layers.*` key list);
// `layers-wire.test.ts` owns the attribute *wire* (normalisation of the new row
// fields, the sender, the store action). This file owns what the palette **does**
// with those fields: which color the row's dot paints, which state chips a row
// marks, what a press on one writes, and which strings the read-only secondaries
// show.
//
// The behaviour lives in `layer-attributes.ts` (pure data + predicates), so the
// interesting contracts are exercised directly instead of through a rendered
// tree; the markup contracts are read as source, the same way the other palette
// suites do it. No jsdom, no window.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  colorPayload,
  EMPTY_MARK_SUMMARY,
  flagPayload,
  layerAttributes,
  layerFlags,
  layerMarks,
  layerMarkSummary,
  layerSwatch,
  RESTING_MARK_LIMIT,
} from '../src/layers/layer-attributes.ts';
import {
  createLayersStore,
  LAYERS_COMMAND,
  layerTone,
  parseLayersAnswer,
  SET_LAYER_PROPERTY_COMMAND,
} from '../src/layers/layers.ts';
import { readCatalog, readModule } from './helpers.ts';

/** One layer row exactly as the Rust `CadLayer` serializes it. */
function row(name, overrides = {}) {
  return {
    name,
    visible: true,
    databaseVisible: true,
    overridden: false,
    current: false,
    color: { aci: 7, rgb: '#ffffff' },
    linetype: 'Continuous',
    lineweight: 'ByLayer',
    transparency: '0',
    locked: false,
    frozen: false,
    off: false,
    plottable: true,
    ...overrides,
  };
}

function answer(layers, extra = {}) {
  return { layers, active: null, overridden: 0, ...extra };
}

/** A promise plus its settle handle, for the in-flight test. */
function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

/**
 * The catalog keys this lane adds. Every one of them is a sentence (or a mark)
 * the palette shows, so each must exist in both catalogs and be referenced
 * literally somewhere in the two modules that render the attribute surface.
 */
const ATTRIBUTE_KEYS = [
  // The four flags, their marks and the honest behavior behind each one.
  'layers.off',
  'layers.frozen',
  'layers.locked',
  'layers.plottable',
  'layers.no_plot',
  'layers.off.hint',
  'layers.frozen.hint',
  'layers.locked.hint',
  'layers.plottable.hint',
  'layers.no_plot.hint',
  // The count a row shows when it holds more states than it can spell out.
  'layers.marks_more',
  'layers.marks_more.hint',
  // The details line itself.
  'layers.attributes',
  'layers.attributes.close',
  'layers.attributes.hint',
  // The color control and the two things the dot may say about itself.
  'layers.color',
  'layers.color.aci',
  'layers.color.hint',
  'layers.color.marker',
  'layers.color.reported',
  // The three read-only secondaries.
  'layers.linetype',
  'layers.lineweight',
  'layers.transparency',
];

// --- the swatch ---------------------------------------------------------

test('the dot paints the host color when it sent one, the name marker otherwise', () => {
  // The drawing's own color: `row.color.rgb`, as the host answered it.
  const real = layerSwatch({ name: 'WALLS', color: { aci: 1, rgb: '#ff0000' } });
  assert.deepEqual(real, { color: '#ff0000', fromColor: true });
  // An uppercase hex is the same color, normalised once for CSS and for the
  // accessible name.
  assert.deepEqual(layerSwatch({ name: 'WALLS', color: { aci: null, rgb: '#FF7F00' } }), {
    color: '#ff7f00', fromColor: true,
  });
  // No usable color: the muted, name-derived marker — and `fromColor` says so, so
  // the dot can never be announced as the layer's own color.
  for (const rgb of ['', 'red', '#fff', '#ff00000', 'rgb(255,0,0)']) {
    const fallback = layerSwatch({ name: 'WALLS', color: { aci: 1, rgb } });
    assert.equal(fallback.fromColor, false, `rgb: ${String(rgb)}`);
    assert.deepEqual(fallback, { color: layerTone('WALLS').color, fromColor: false });
  }
  // A row that reported no color at all (or no row at all) is the marker too.
  assert.deepEqual(layerSwatch({ name: 'WALLS' }), { color: layerTone('WALLS').color, fromColor: false });
  assert.deepEqual(layerSwatch(undefined), { color: layerTone('').color, fromColor: false });
  // The marker is the same dot the palette always had: two names separate, one
  // name is stable, and the value is always paintable CSS.
  assert.notDeepEqual(layerSwatch({ name: 'WALLS' }), layerSwatch({ name: 'DIMS' }));
  assert.match(layerSwatch({ name: 'WALLS' }).color, /^#[0-9a-f]{6}$/);
});

test('a color pick is sent as #RRGGBB, never as anything the grammar would refuse', () => {
  // The native `<input type="color">` hands back lowercase; the host's color
  // grammar takes `#RRGGBB`. Same color, one spelling on the wire.
  assert.equal(colorPayload('#ff7f00'), '#FF7F00');
  assert.equal(colorPayload('  #Aa00Ff  '), '#AA00FF');
  // Anything that is not a hex color produces no write at all (an empty string is
  // what the control answers before the first pick).
  for (const value of ['', '   ', 'red', '#fff', '#ff00000', 'rgb(1,2,3)', null, undefined, 42, {}]) {
    assert.equal(colorPayload(value), '', `value: ${String(value)}`);
  }
  // The flag grammar is the host's own two words.
  assert.equal(flagPayload(true), 'true');
  assert.equal(flagPayload(false), 'false');
  assert.equal(flagPayload('true'), 'false', 'only the boolean `true` is true');
  assert.equal(flagPayload(undefined), 'false');
});

// --- the flags and the marks --------------------------------------------

test('the four flags come from the row, and a press sends the opposite value', () => {
  const [walls] = parseLayersAnswer(answer([
    row('WALLS', { off: true, frozen: false, locked: true, plottable: false }),
  ])).layers;
  const flags = layerFlags(walls);
  // One entry per writable key, in the host's own grouping — and the values are
  // the row's fields, not a guess from `visible`.
  assert.deepEqual(flags.map(flag => flag.key), ['off', 'frozen', 'locked', 'plottable']);
  assert.deepEqual(flags.map(flag => flag.value), [true, false, true, false]);
  assert.deepEqual(flags.map(flag => flag.next), ['false', 'true', 'false', 'true']);
  assert.deepEqual(flags.map(flag => flag.labelKey), [
    'layers.off', 'layers.frozen', 'layers.locked', 'layers.plottable',
  ]);
  assert.deepEqual(flags.map(flag => flag.hintKey), [
    'layers.off.hint', 'layers.frozen.hint', 'layers.locked.hint', 'layers.plottable.hint',
  ]);

  // A row that carried none of the fields claims nothing: every flag reads false
  // (the same "not reported is not a state" rule the parser follows) and
  // `plottable` included — a missing plot flag is not a layer that plots.
  const [bare] = parseLayersAnswer(answer([{ name: '0' }])).layers;
  assert.deepEqual(layerFlags(bare).map(flag => flag.value), [false, false, false, false]);
  assert.deepEqual(layerFlags(undefined).map(flag => flag.value), [false, false, false, false]);
});

test('only the states that are not the default carry a mark, and each clears itself', () => {
  // A plain row: no chips at all, so the marks column is exactly what it was.
  const [plain] = parseLayersAnswer(answer([row('0')])).layers;
  assert.deepEqual(layerMarks(plain), []);

  const [marked] = parseLayersAnswer(answer([
    row('WALLS', { off: true, frozen: true, locked: true, plottable: false }),
  ])).layers;
  const marks = layerMarks(marked);
  assert.deepEqual(marks.map(mark => mark.key), ['off', 'frozen', 'locked', 'plottable']);
  // A press on a mark clears that very state: the one direction the mark promises.
  assert.deepEqual(marks.map(mark => mark.next), ['false', 'false', 'false', 'true']);
  assert.deepEqual(marks.map(mark => mark.labelKey), [
    'layers.off', 'layers.frozen', 'layers.locked', 'layers.no_plot',
  ]);
  assert.deepEqual(marks.map(mark => mark.hintKey), [
    'layers.off.hint', 'layers.frozen.hint', 'layers.locked.hint', 'layers.no_plot.hint',
  ]);

  // `plottable` is the one flag whose notable state is `false`: a layer that plots
  // carries no chip, a layer that does not carries the `no_plot` one — and a row
  // that never reported the field carries nothing (it is not marked as if it had).
  assert.deepEqual(layerMarks({ name: 'X', plottable: true }), []);
  assert.deepEqual(layerMarks({ name: 'X' }), []);
  assert.deepEqual(layerMarks({ name: 'X', plottable: false }).map(mark => mark.labelKey), ['layers.no_plot']);
  // And one flag at a time: an off layer alone marks nothing else.
  assert.deepEqual(layerMarks({ name: 'X', off: true }).map(mark => mark.key), ['off']);
  assert.deepEqual(layerMarks(undefined), []);
});

test('a resting row spells out two states and counts the rest, never dropping one', () => {
  // The dock's marks column fits two chips next to a name; more are counted.
  assert.equal(RESTING_MARK_LIMIT, 2);
  assert.deepEqual({ ...EMPTY_MARK_SUMMARY }, { marks: [], hidden: 0 });

  const [plain] = parseLayersAnswer(answer([row('0')])).layers;
  assert.deepEqual(layerMarkSummary(plain), { marks: [], hidden: 0 });

  const [one] = parseLayersAnswer(answer([row('A', { off: true })])).layers;
  assert.deepEqual(layerMarkSummary(one).marks.map(mark => mark.key), ['off']);
  assert.equal(layerMarkSummary(one).hidden, 0);

  // Three set states: the first two in the host's own order, and the third counted.
  const [three] = parseLayersAnswer(answer([
    row('B', { off: true, frozen: true, locked: true, plottable: false }),
  ])).layers;
  const summary = layerMarkSummary(three);
  assert.deepEqual(summary.marks.map(mark => mark.key), ['off', 'frozen']);
  assert.equal(summary.hidden, 2, 'locked and no-plot are counted, not lost');

  // The same row always spells out the same chips (no reordering between renders).
  assert.deepEqual(layerMarkSummary(three), summary);
  // And the count is never negative for a row with nothing set.
  assert.equal(layerMarkSummary(undefined).hidden, 0);
});

test('a mark press writes that key with the cleared value, one write per press', async () => {
  const layers = parseLayersAnswer(answer([
    row('WALLS', { off: true, frozen: true, locked: true, plottable: false }),
  ])).layers;
  const calls = [];
  const store = createLayersStore(async (command, args) => {
    calls.push({ command, args });
    if (command === SET_LAYER_PROPERTY_COMMAND) return { accepted: true };
    if (command === LAYERS_COMMAND) return answer([row('WALLS')]);
    throw new Error(`unexpected ${command}`);
  }, { wait: async () => {} });

  for (const mark of layerMarks(layers[0])) {
    assert.deepEqual(await store.setProperty('WALLS', mark.key, mark.next), { kind: 'ok' });
  }
  assert.deepEqual(
    calls.filter(call => call.command === SET_LAYER_PROPERTY_COMMAND).map(call => call.args),
    [
      { layer: 'WALLS', key: 'off', value: 'false' },
      { layer: 'WALLS', key: 'frozen', value: 'false' },
      { layer: 'WALLS', key: 'locked', value: 'false' },
      { layer: 'WALLS', key: 'plottable', value: 'true' },
    ],
  );
});

test('the details toggles send the flip in both directions, and a busy palette is refused', async () => {
  const [walls] = parseLayersAnswer(answer([
    row('WALLS', { off: false, frozen: true, locked: false, plottable: true }),
  ])).layers;
  const calls = [];
  const store = createLayersStore(async (command, args) => {
    calls.push({ command, args });
    if (command === SET_LAYER_PROPERTY_COMMAND) return { accepted: true };
    if (command === LAYERS_COMMAND) return answer([row('WALLS')]);
    throw new Error(`unexpected ${command}`);
  }, { wait: async () => {} });

  // Off is set (true), frozen is cleared (false), locked is set, plot is cleared.
  for (const flag of layerFlags(walls)) {
    await store.setProperty('WALLS', flag.key, flag.next);
  }
  assert.deepEqual(
    calls.filter(call => call.command === SET_LAYER_PROPERTY_COMMAND).map(call => call.args),
    [
      { layer: 'WALLS', key: 'off', value: 'true' },
      { layer: 'WALLS', key: 'frozen', value: 'false' },
      { layer: 'WALLS', key: 'locked', value: 'true' },
      { layer: 'WALLS', key: 'plottable', value: 'false' },
    ],
  );

  // The palette drops a press while a mutation is in flight (its `writeProperty`
  // guard), and the store refuses the same second call locally, so one press is
  // one write: no queue, no second request.
  const gate = deferred();
  const gated = [];
  const busyStore = createLayersStore(async (command, args) => {
    gated.push({ command, args });
    if (command === SET_LAYER_PROPERTY_COMMAND) return gate.promise;
    if (command === LAYERS_COMMAND) return answer([row('WALLS')]);
    throw new Error(`unexpected ${command}`);
  }, { wait: async () => {} });
  const first = busyStore.setProperty('WALLS', 'off', 'true');
  assert.deepEqual(await busyStore.setProperty('WALLS', 'frozen', 'true'), { kind: 'busy' });
  gate.resolve({ accepted: true });
  assert.deepEqual(await first, { kind: 'ok' });
  assert.equal(
    gated.filter(call => call.command === SET_LAYER_PROPERTY_COMMAND).length,
    1,
    'the refused press never reached the host',
  );
});

// --- the read-only secondaries ------------------------------------------

test('the secondary attributes are the wire strings, verbatim and in order', () => {
  const [walls] = parseLayersAnswer(answer([
    row('WALLS', { linetype: 'DASHED', lineweight: '35', transparency: '40' }),
  ])).layers;
  const attributes = layerAttributes(walls);
  assert.deepEqual(attributes.map(attribute => attribute.key), ['linetype', 'lineweight', 'transparency']);
  assert.deepEqual(attributes.map(attribute => attribute.labelKey), [
    'layers.linetype', 'layers.lineweight', 'layers.transparency',
  ]);
  // No conversion, no re-scaling, no unit appended: `35` stays 1/100 mm as the
  // host stores it, `40` stays percent transparent.
  assert.deepEqual(attributes.map(attribute => attribute.value), ['DASHED', '35', '40']);

  // The host's non-numeric lineweights pass through as their own words.
  assert.deepEqual(
    layerAttributes({ name: 'X', lineweight: 'Default' }).map(attribute => attribute.value),
    ['', 'Default', ''],
  );
  // A row that reported nothing shows nothing — it is never padded with a guess.
  assert.deepEqual(layerAttributes(undefined).map(attribute => attribute.value), ['', '', '']);
});

// --- the catalog --------------------------------------------------------

test('every attribute key exists in both catalogs and is rendered, never invented', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const palette = await readModule('components/LayersPalette.tsx');
  const attributes = await readModule('layers/layer-attributes.ts');
  const details = await readModule('components/LayerDetails.tsx');
  const renders = (key: string) =>
    palette.includes(`'${key}'`) || attributes.includes(`'${key}'`) || details.includes(`'${key}'`);
  for (const key of ATTRIBUTE_KEYS) {
    assert.equal(typeof zh[key], 'string', `zh-CN ${key}`);
    assert.equal(typeof en[key], 'string', `en ${key}`);
    assert.notEqual(zh[key].trim(), '', `zh-CN ${key} is not blank`);
    assert.notEqual(en[key].trim(), '', `en ${key} is not blank`);
    // The two languages must say the same thing structurally: the same
    // placeholders, or one language would print a raw `{color}`.
    const placeholders = source => [...String(source).matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)]
      .map(match => match[1]).sort();
    assert.deepEqual(placeholders(zh[key]), placeholders(en[key]), `${key} placeholders`);
    // The mark names come from `layer-attributes.ts` (pure data), the details line
    // from its own component and the rest from the palette, so the key has to be
    // literal in one of the three.
    assert.ok(renders(key), `${key} is referenced literally`);
  }
  // Nothing else may reach the catalogs under these two namespaces without copy:
  // the palette's attribute surface has no inline sentences.
  for (const key of ATTRIBUTE_KEYS) {
    assert.match(
      key,
      /^layers\.(off|frozen|locked|plottable|no_plot|marks_more|attributes|color|linetype|lineweight|transparency)(\.|$)/,
      key,
    );
  }
});

test('the copy states what off, frozen, locked and plottable actually do', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  // `off` and `frozen` both hide through the same visibility path today: both
  // hints say so, and neither claims an independent freeze rule.
  for (const key of ['layers.off.hint', 'layers.frozen.hint']) {
    assert.match(en[key], /same visibility path/i, `en ${key}`);
  }
  assert.match(en['layers.frozen.hint'], /not implemented/i, 'freezing rules are not claimed');
  // `locked` now refuses entity writes (one database choke point), while
  // `plottable` is still stored only. Each hint must state its own truth.
  assert.match(en['layers.locked.hint'], /cannot be created, changed or deleted/i);
  assert.match(en['layers.locked.hint'], /making a locked layer current is not refused yet/i);
  assert.match(en['layers.plottable.hint'], /nothing else changes yet/i);
  assert.match(en['layers.plottable.hint'], /does not read it yet/i);
  // The summary line repeats the same facts, so a reader who never opens a
  // tooltip still sees them.
  assert.match(en['layers.attributes.hint'], /off and frozen both hide/i);
  assert.match(en['layers.attributes.hint'], /locked refuses creating, changing or deleting/i);
  assert.match(en['layers.attributes.hint'], /plotting does not read it yet/i);
  // The colour cell says which identity it writes, and which ones a layer cannot
  // take: an index color from the palette, a true color from the value field, and
  // no by-layer/by-block because there is no layer above a layer.
  assert.match(en['layers.color.hint'], /#RRGGBB/);
  assert.match(en['layers.color.hint'], /index color/i);
  assert.match(en['layers.color.hint'], /aci:N/);
  assert.match(en['layers.color.hint'], /cannot be by-layer/i);
  // The copy does not claim the drawing is left alone: a ByLayer object does take
  // the layer's new colour on the next expansion (cad-db resolves it), so the hint
  // names exactly which objects follow and which keep their own colour.
  assert.match(en['layers.color.hint'], /drawn by layer show the new color/i);
  assert.match(en['layers.color.hint'], /explicit color keep it/i);
  assert.match(en['layers.color.marker'], /not reported/i);
  // And both languages carry the same facts (no bare "已冻结" that explains
  // nothing): each Chinese hint is at least as long as its English sibling's
  // information — a short check that neither was left as a bare label.
  for (const key of ATTRIBUTE_KEYS.filter(entry => entry.endsWith('.hint'))) {
    assert.ok(zh[key].length >= 12, `zh-CN ${key} explains itself`);
    assert.ok(en[key].length >= 24, `en ${key} explains itself`);
  }
});

// --- the markup ---------------------------------------------------------

test('the swatch, the marks and the details line are the palette\'s own rows', async () => {
  const palette = await readModule('components/LayersPalette.tsx');
  const details = await readModule('components/LayerDetails.tsx');
  const occurrences = needle => palette.split(needle).length - 1;

  // The dot: a real button, painted from `layerSwatch`, whose name is the honest
  // color state (reported hex, or "not reported, name marker").
  assert.ok(palette.includes('const swatch = layerSwatch(layer);'));
  assert.ok(palette.includes('<button'), 'the dot is a control');
  assert.ok(palette.includes('style={{ backgroundColor: swatch.color }}'));
  assert.ok(palette.includes("swatch.fromColor ? '' : 'is-tone'"), 'the fallback is visually distinguishable');
  assert.ok(palette.includes("t('layers.color.reported', { color: swatch.color })"));
  assert.ok(palette.includes("t('layers.color.marker')"));
  assert.equal(palette.includes('aria-hidden="true" />'), false, 'the dot is no longer decorative');
  assert.ok(palette.includes("title={t('layers.color')}"));
  assert.equal(palette.includes('layerTone'), false, 'the dot\'s color is decided in one place');

  // The marks: the row's own chips, in the existing marks column, and only while
  // the details line is closed (it repeats them as toggles). A row that holds more
  // states than fit spells out two and counts the rest with a chip that opens the
  // details line, so the row stays one line and no state is silently dropped.
  assert.ok(palette.includes('const summary = detailsOpen ? EMPTY_MARK_SUMMARY : layerMarkSummary(layer);'));
  assert.ok(palette.includes('{summary.marks.map(mark => ('));
  assert.ok(palette.includes('className="layers__chip layers__chip--action"'));
  assert.ok(palette.includes('aria-pressed={true}'));
  assert.ok(palette.includes("title={t(mark.hintKey)}"));
  assert.ok(palette.includes('{t(mark.labelKey)}'));
  assert.ok(palette.includes('{summary.hidden > 0 ? ('));
  assert.ok(palette.includes("t('layers.marks_more', { count: summary.hidden })"));
  assert.ok(palette.includes("title={t('layers.marks_more.hint')}"));
  assert.ok(palette.includes('onClick={() => openDetails(layer)}'), 'the count opens the details line');

  // The details line: one per row, on demand, in the row's existing second-line
  // slot — the resting row therefore keeps its height. The panel itself is
  // `LayerDetails`, so this file keeps a single row of markup.
  assert.ok(palette.includes('{detailsOpen ? ('));
  assert.ok(palette.includes('<LayerDetails'), 'the open line is the one component');
  const head = palette.slice(palette.indexOf('<LayerDetails'), palette.indexOf('/>', palette.indexOf('<LayerDetails')));
  for (const prop of ['layer={layer}', 'color={swatch.color}', 'fromColor={swatch.fromColor}',
    'hintId={colorHintId}', 'busy={controlsBusy}', 'onPickColor=', 'onToggleFlag=', 'onClose={closeDetails}']) {
    assert.ok(head.includes(prop), `the details line is handed ${prop}`);
  }
  // The panel renders what the row says and sends one write per press. The colour
  // cell is the shared in-DOM chooser: no webview colour dialog, no symbolic entry
  // the layer grammar would refuse, and the free-form entry routed by the layer
  // grammar's own `colorPayload`.
  const code = details.slice(details.indexOf('export function'));
  assert.ok(code.includes('className="layers__details"'));
  assert.ok(code.includes('role="group"'));
  assert.ok(details.includes("aria-label={t('layers.attributes')}"), 'the panel is named in catalog copy');
  assert.equal(code.split('type="color"').length - 1, 0, 'no webview colour dialog');
  assert.ok(details.includes('<ColorField'), 'one shared colour cell');
  assert.ok(details.includes('symbols={LAYER_COLOR_SYMBOLS}'), 'only entries the layer grammar takes');
  assert.ok(details.includes('custom={colorPayload}'), 'the free-form entry is the layer grammar');
  assert.ok(details.includes('onPick={onPickColor}'), 'one committed press is one write');
  assert.ok(details.includes("t('layers.color.aci', { index: layer.color.aci })"));
  assert.ok(details.includes('{layerFlags(layer).map(flag => ('));
  assert.ok(details.includes('aria-pressed={flag.value}'));
  assert.ok(details.includes('onClick={() => onToggleFlag(flag.key, flag.next)}'));
  assert.ok(details.includes('{layerAttributes(layer).map(attribute => ('));
  assert.ok(details.includes('{attribute.value}'), 'the wire string is rendered as it arrived');
  assert.ok(details.includes("t('layers.attributes.hint')"), 'the honest summary is on screen, not in a comment');
  assert.ok(details.includes("t('layers.attributes.close')"), 'the line closes with a named button');
  assert.ok(details.includes("if (event.key !== 'Escape' || event.defaultPrevented) return;"),
    'the panel catches its own Escape');
  assert.ok(details.includes('event.stopPropagation();'), 'so the row form never sees a second cancel');

  // Both rest triggers announce the same panel.
  assert.equal(occurrences('aria-expanded={detailsOpen}'), 2, 'the dot and the chevron both disclose it');
  assert.ok(palette.includes("icon={detailsOpen ? 'chevronUp' : 'chevronDown'}"));

  // A press writes the row's own key/value through one guarded path: busy drops
  // the press locally (the store would refuse it too), and a colour the grammar
  // cannot take is never sent.
  assert.ok(palette.includes([
    'const writeProperty = (layer: LayerRow, key: string, value: string) => {',
    '    if (controlsBusy) return;',
    '    void store.setProperty(layer.name, key, value);',
    '  };',
  ].join('\n')), 'one guarded write path for every attribute');
  assert.ok(palette.includes('onClick={() => toggleFlag(layer, mark.key, mark.next)}'));
  assert.ok(palette.includes('onToggleFlag={(key, value) => toggleFlag(layer, key, value)}'));
  assert.ok(palette.includes('const color = colorPayload(value);'));
  assert.ok(palette.indexOf('if (!color) return;') < palette.indexOf("writeProperty(layer, 'color', color);"),
    'an unparsable colour never reaches the wire');

  // Refusals use the palette's existing error line — no dialog, ever.
  assert.ok(palette.includes('className="layers__error" role="alert"'));
  for (const [name, source] of [['palette', palette], ['details', details]]) {
    assert.equal(/\b(?:window\.)?(?:alert|confirm|prompt)\s*\(/.test(source), false, `${name}: no browser dialog`);
    // And the attribute surface adds no second copy: every sentence is a catalog key.
    assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(source), false, `${name}: no hardcoded CJK`);
  }
  assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(await readModule('layers/layer-attributes.ts')), false,
    'no hardcoded CJK in the attribute model');
});

test('the attribute styles reuse the chrome tokens and keep the resting row 28px', async () => {
  const css = await readModule('style.css');
  const start = css.indexOf('/* Layers palette');
  const end = css.indexOf('/* Navigation rail');
  assert.ok(start > 0 && end > start, 'the layers block exists between the docks');
  const block = css.slice(start, end);

  // The resting row height is untouched; the details line is the only thing that
  // makes a row taller, and it only exists while it is open.
  assert.ok(block.includes('min-height: 28px'), 'the row keeps its resting height');
  assert.ok(block.includes('.layers__details {'));
  assert.ok(block.includes('grid-column: 1 / -1'), 'the details line spans the row grid');
  assert.ok(block.includes('.layers__chip--action'));
  assert.ok(block.includes('.layers__chip--action[aria-pressed=\'true\']'), 'pressed is a real state');
  // The dot keeps its 7px and gains a wider pointer target, not a wider column.
  assert.ok(block.includes('.layers__swatch::after'));
  assert.ok(block.includes('.layers__swatch.is-tone'));
  // The details line has no colour control of its own: it renders the shared
  // colour cell, styled once in that component's own block (see below).
  assert.equal(block.includes('.layers__color-input'), false, 'no native colour dialog is left');
  assert.ok(css.includes('.color-field__trigger'), 'the shared colour cell is the one control');
  // Same tokens as the rest of the chrome; nothing floats, nothing goes
  // pointer-transparent inside the translucent dock.
  for (const token of ['var(--field)', 'var(--line-soft)', 'var(--focus)', 'var(--pressed)', 'var(--text-muted)']) {
    assert.ok(block.includes(token), `the layers block uses ${token}`);
  }
  assert.equal(block.includes('pointer-events'), false, 'no control is made pointer-transparent');
  assert.equal(block.includes('position: fixed'), false, 'docked, never floating over the CAD surface');
});
