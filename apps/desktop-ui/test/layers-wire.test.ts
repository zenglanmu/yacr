// Layer attributes on the wire, UI half.
//
// `layers.test.ts` owns the palette's model (selector, projections, identity dot,
// markup) and the three structural table writes. This file owns the **attribute**
// surface that arrived with `cad_set_layer_property`: the normalisation of the new
// row fields and the sender/store action behind a colour / flag / linetype /
// lineweight / transparency write.
//
// The attributes are display/edit **inputs, not behaviour**: `off`/`frozen` hide
// the layer through the host's existing `visible` flag, while `locked`,
// `plottable`, the colour, the linetype, the lineweight and the transparency are
// stored only. Nothing here invents a rendering effect.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ackAdmitted,
  CREATE_LAYER_COMMAND,
  createLayersStore,
  EMPTY_LAYERS_ANSWER,
  LAYERS_COMMAND,
  parseLayerColor,
  parseLayersAnswer,
  refreshLayers,
  renameLayer,
  RENAME_LAYER_COMMAND,
  SET_LAYER_PROPERTY_COMMAND,
  setLayerProperty,
} from '../src/layers.ts';

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

/** A fake `invoke` that records every call and answers from a script. */
function recorder(answers) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const answer = answers.shift();
    if (answer instanceof Error) throw answer;
    return answer;
  };
  return { calls, invoke };
}

/** A promise plus its settle handle. */
function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

// --- the new row fields -------------------------------------------------

test('the attribute fields are read verbatim, never inferred', () => {
  const parsed = parseLayersAnswer(answer([
    row('0'),
    row('WALLS', {
      visible: false,
      databaseVisible: false,
      color: { aci: null, rgb: '#FF7F00' },
      linetype: 'DASHED',
      lineweight: '35',
      transparency: '40',
      locked: true,
      frozen: true,
      off: false,
      plottable: false,
    }),
  ]));
  assert.deepEqual(parsed.layers[0], {
    name: '0', visible: true, databaseVisible: true, overridden: false, current: false,
    color: { aci: 7, rgb: '#ffffff' }, linetype: 'Continuous', lineweight: 'ByLayer',
    transparency: '0', locked: false, frozen: false, off: false, plottable: true,
  });
  const walls = parsed.layers[1];
  // A true colour keeps `aci: null` — the host's own fact, not a palette guess —
  // and the hex is normalised to lowercase for comparison and CSS.
  assert.deepEqual(walls.color, { aci: null, rgb: '#ff7f00' });
  // The host's own grammar strings pass through untouched: `35` is 1/100 mm and
  // `40` is percent transparent, never converted here.
  assert.equal(walls.linetype, 'DASHED');
  assert.equal(walls.lineweight, '35');
  assert.equal(walls.transparency, '40');
  assert.equal(walls.locked, true);
  assert.equal(walls.frozen, true);
  assert.equal(walls.off, false);
  assert.equal(walls.plottable, false);
});

test('a missing or malformed attribute reads as "not reported", never as a claim', () => {
  const parsed = parseLayersAnswer(answer([{ name: '0' }]));
  assert.deepEqual(parsed.layers[0], {
    name: '0', visible: false, databaseVisible: false, overridden: false, current: false,
    color: { aci: null, rgb: '' }, linetype: '', lineweight: '', transparency: '',
    locked: false, frozen: false, off: false, plottable: false,
  });
  // An empty `rgb` is the documented "no colour to paint" value: a consumer falls
  // back to the name-derived tone instead of painting an invented colour.
  assert.equal(parsed.layers[0].color.rgb, '');
});

test('the colour object keeps only an ACI index and a real #rrggbb', () => {
  // The host's contract shape.
  assert.deepEqual(parseLayerColor({ aci: 1, rgb: '#ff0000' }), { aci: 1, rgb: '#ff0000' });
  // A symbolic/absent ACI stays null; an out-of-range one is not plausible.
  assert.deepEqual(parseLayerColor({ aci: null, rgb: '#00ff00' }), { aci: null, rgb: '#00ff00' });
  assert.deepEqual(parseLayerColor({ aci: 0, rgb: '#00ff00' }), { aci: null, rgb: '#00ff00' });
  assert.deepEqual(parseLayerColor({ aci: 256, rgb: '#00ff00' }), { aci: null, rgb: '#00ff00' });
  assert.deepEqual(parseLayerColor({ aci: 12.5, rgb: '#00ff00' }), { aci: null, rgb: '#00ff00' });
  assert.deepEqual(parseLayerColor({ aci: '1', rgb: '#00ff00' }), { aci: null, rgb: '#00ff00' });
  // Anything that is not `#rrggbb` (a colour name, `rgb()`, a short hex) is not
  // paintable as the documented shape.
  for (const rgb of ['red', 'rgb(255,0,0)', '#fff', '#ff00000', 0xff0000, null]) {
    assert.equal(parseLayerColor({ aci: 1, rgb }).rgb, '', `rgb: ${String(rgb)}`);
  }
  assert.deepEqual(parseLayerColor(undefined), { aci: null, rgb: '' });
  assert.deepEqual(parseLayerColor('color'), { aci: null, rgb: '' });
});

// --- the sender ---------------------------------------------------------

test('setLayerProperty sends the one attribute command with three strings', async () => {
  assert.equal(SET_LAYER_PROPERTY_COMMAND, 'cad_set_layer_property');
  const { calls, invoke } = recorder([{ accepted: true }]);
  const ack = await setLayerProperty(invoke, 'WALLS', 'color', 'aci:1');
  assert.deepEqual(ack, { kind: 'ok' });
  assert.deepEqual(calls, [
    { command: 'cad_set_layer_property', args: { layer: 'WALLS', key: 'color', value: 'aci:1' } },
  ]);
});

test('setLayerProperty refuses a nameless or empty request without any IPC', async () => {
  const { calls, invoke } = recorder([]);
  const cases = [
    [null, 'color', 'aci:1'],
    ['', 'color', 'aci:1'],
    ['WALLS', '', 'aci:1'],
    ['WALLS', null, 'aci:1'],
    ['WALLS', 'color', ''],
    ['WALLS', 'color', '   '],
    ['WALLS', 'color', 1],
    ['WALLS', 'color', null],
  ];
  for (const [layer, key, value] of cases) {
    assert.deepEqual(
      await setLayerProperty(invoke, layer, key, value),
      { kind: 'invalid' },
      `${String(layer)}/${String(key)}/${String(value)}`,
    );
  }
  assert.deepEqual(calls, [], 'a request whose only answer is a refusal is not sent');
});

test('a refused attribute write reports the host reason unchanged', async () => {
  const { invoke } = recorder([new Error('layer.property.unsupported: plotstyle')]);
  const ack = await setLayerProperty(invoke, 'WALLS', 'plotstyle', 'Normal');
  assert.equal(ack.kind, 'error');
  assert.match(ack.reason, /layer\.property\.unsupported/);
});

test('the ack is "queued", not "applied"', async () => {
  // The one promise the sender makes.
  assert.equal(ackAdmitted({ accepted: true }), true);
  const { invoke } = recorder([{ accepted: true }]);
  assert.deepEqual(await setLayerProperty(invoke, 'WALLS', 'locked', 'true'), { kind: 'ok' });
});

// --- the store action ---------------------------------------------------

test('setProperty queues the write, then re-reads the host rows', async () => {
  const after = answer([row('WALLS', { color: { aci: 1, rgb: '#ff0000' } })]);
  const { calls, invoke } = recorder([{ accepted: true }, after]);
  const store = createLayersStore(invoke, { wait: async () => {} });

  const ack = await store.setProperty('WALLS', 'color', 'aci:1');

  assert.deepEqual(ack, { kind: 'ok' });
  assert.deepEqual(calls.map(call => call.command), [SET_LAYER_PROPERTY_COMMAND, LAYERS_COMMAND]);
  assert.deepEqual(store.getState().layers[0].color, { aci: 1, rgb: '#ff0000' });
  assert.equal(store.getState().busy, false);
  assert.equal(store.getState().error, null);
});

test('a property the projection has not caught up with costs exactly one more read', async () => {
  // First read: the pre-write rows. Second read: the row as written. The store may
  // read twice, never poll.
  const stale = answer([row('WALLS')]);
  const fresh = answer([row('WALLS', { locked: true })]);
  const { calls, invoke } = recorder([{ accepted: true }, stale, fresh]);
  const store = createLayersStore(invoke, { wait: async () => {} });

  assert.deepEqual(await store.setProperty('WALLS', 'locked', 'true'), { kind: 'ok' });
  assert.deepEqual(calls.map(call => call.command), [
    SET_LAYER_PROPERTY_COMMAND,
    LAYERS_COMMAND,
    LAYERS_COMMAND,
  ]);
  assert.equal(store.getState().layers[0].locked, true);
});

test('the read-back check understands the grammar spellings it claims to', async () => {
  // `aci:N`, `#RRGGBB`, `r,g,b`, the boolean spellings and the percent string all
  // count as "already shown"; anything else simply costs the second read.
  const settled = answer([
    row('A', { color: { aci: 1, rgb: '#ff0000' }, transparency: '0' }),
    row('B', { color: { aci: null, rgb: '#0a0b0c' }, locked: true }),
    row('C', { color: { aci: null, rgb: '#ff0000' } }),
  ]);
  // One `{accepted:true}` plus one read per write; the count assertion below is
  // what proves no write needed the extra follow-up read.
  const { calls, invoke } = recorder([{ accepted: true }, settled, { accepted: true }, settled, { accepted: true }, settled, { accepted: true }, settled, { accepted: true }, settled]);
  const store = createLayersStore(invoke, { wait: async () => {} });

  // One mutation per spelling; each answers from the same settled projection, so a
  // recognised spelling needs no follow-up read.
  await store.setProperty('A', 'color', 'aci:1');
  await store.setProperty('A', 'transparency', '0');
  await store.setProperty('B', 'color', 'rgb:#0A0B0C');
  await store.setProperty('B', 'locked', '1');
  await store.setProperty('C', 'color', '255,0,0');

  const reads = calls.filter(call => call.command === LAYERS_COMMAND).length;
  assert.equal(reads, 5, 'one read per admitted write: every spelling was recognised');
});

test('a refused attribute write keeps the rows and reports the reason', async () => {
  const rows = answer([row('WALLS')]);
  const { invoke } = recorder([rows, new Error('layer.linetype_missing: PHANTOM')]);
  const store = createLayersStore(invoke, { wait: async () => {} });
  await store.load();
  const before = store.getState().layers;

  const ack = await store.setProperty('WALLS', 'linetype', 'PHANTOM');

  assert.equal(ack.kind, 'error');
  assert.deepEqual(store.getState().layers, before, 'a refusal never edits the rows');
  assert.equal(store.getState().error.kind, 'mutation');
  assert.match(store.getState().error.reason, /layer\.linetype_missing/);
});

test('setProperty is single flight and never sends a blank request', async () => {
  const gate = deferred();
  const { calls, invoke } = recorder([{ accepted: true }, answer([row('WALLS')]), { accepted: true }, answer([row('WALLS')])]);
  const store = createLayersStore(
    async (command, args) => {
      if (command === SET_LAYER_PROPERTY_COMMAND) return gate.promise.then(() => invoke(command, args));
      return invoke(command, args);
    },
    { wait: async () => {} },
  );

  const first = store.setProperty('WALLS', 'off', 'true');
  // The window covers the queued write **and** its read-back.
  assert.deepEqual(await store.setProperty('WALLS', 'frozen', 'true'), { kind: 'busy' });
  gate.resolve();
  assert.deepEqual(await first, { kind: 'ok' });

  // A blank key/value never reaches the wire.
  assert.deepEqual(await store.setProperty('WALLS', '', 'true'), { kind: 'invalid' });
  assert.deepEqual(await store.setProperty('WALLS', 'off', ' '), { kind: 'invalid' });
  assert.equal(
    calls.filter(call => call.command === SET_LAYER_PROPERTY_COMMAND).length,
    1,
    'only the one admitted write was sent',
  );
});

test('the attribute sender and its siblings stay separate commands', async () => {
  // The attribute write is its own channel: it must never be smuggled through the
  // structural write or the visibility override.
  const commands = [CREATE_LAYER_COMMAND, RENAME_LAYER_COMMAND, SET_LAYER_PROPERTY_COMMAND];
  assert.equal(new Set(commands).size, commands.length);
  const { calls, invoke } = recorder([{ accepted: true }, { accepted: true }]);
  await renameLayer(invoke, 'WALLS', 'PARTITIONS');
  await setLayerProperty(invoke, 'PARTITIONS', 'color', 'aci:3');
  assert.deepEqual(calls.map(call => call.command), [RENAME_LAYER_COMMAND, SET_LAYER_PROPERTY_COMMAND]);
  // The read path is untouched by either.
  assert.equal(EMPTY_LAYERS_ANSWER.layers.length, 0);
  const read = recorder([answer([row('0')])]);
  assert.deepEqual(await refreshLayers(read.invoke), {
    kind: 'ok',
    layers: parseLayersAnswer(answer([row('0')])).layers,
    active: null,
    overridden: 0,
  });
});
