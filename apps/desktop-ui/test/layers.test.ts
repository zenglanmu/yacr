// Layers palette contract (spec F03), UI half.
//
// Pure-function tests for the wire normalisation, the projections and the
// single-flight mutation store, plus source assertions for the parts that only
// exist as markup (the `LAYER` selector's interception, the docked mount, the
// catalog-only copy). No jsdom: the store is a plain object, so its behaviour is
// exercised with a fake `invoke` instead of a rendered tree.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ackAdmitted,
  createLayer,
  CREATE_LAYER_COMMAND,
  createLayersStore,
  deleteLayer,
  DELETE_LAYER_COMMAND,
  EMPTY_LAYERS_ANSWER,
  EMPTY_LAYERS_STATE,
  errorCode,
  filterLayers,
  isLayersCommand,
  isWaitingError,
  LAYERS_COMMAND,
  LAYER_READ_DELAY_MS,
  layerNameHash,
  layerTone,
  nextFocusIndex,
  overrideCount,
  parseLayersAnswer,
  refreshLayers,
  renameLayer,
  RENAME_LAYER_COMMAND,
  RESTORE_LAYERS_COMMAND,
  restoreLayers,
  SET_ACTIVE_LAYER_COMMAND,
  setActiveLayer,
  SET_LAYER_VISIBLE_COMMAND,
  setLayerVisible,
  visibleLayers,
  WAITING_CODE,
} from '../src/layers/layers.ts';
import { DEFAULT_LAYER_NAME, deletableLayer, layerErrorKey } from '../src/layers/layer-labels.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/commands.ts';
import { offeredItems, PALETTE_COMMANDS } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readCss, readModule } from './helpers.ts';

/**
 * One layer row as the host sends it.
 *
 * Every field is present, exactly like the Rust `CadLayer` (the attributes are
 * display/edit inputs, never behaviour — see `layers-wire.test.ts` for their
 * normalisation contracts).
 */
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

/** One `cad_layers` answer. */
function answer(layers, extra = {}) {
  return { layers, active: null, overridden: 0, ...extra };
}

/** A promise plus its settle handle, for the in-flight tests. */
function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

// --- the selector -------------------------------------------------------

test('LAYER is a chrome-local selector, recognised like every other one', () => {
  assert.equal(isLayersCommand('LAYER'), true);
  assert.equal(isLayersCommand(' layer '), true);
  assert.equal(isLayersCommand('layers'), false);
  assert.equal(isLayersCommand('TOGGLELAYER'), false);
  assert.equal(isLayersCommand(''), false);
  assert.equal(isLayersCommand(null), false);
  assert.equal(isLayersCommand(undefined), false);
  // It is in the shared local-selector set, next to the inspector.
  assert.ok(LOCAL_CHROME_COMMANDS.includes('LAYER'));
  assert.ok(LOCAL_CHROME_COMMANDS.includes('PROPERTIES'));
});

// --- normalisation ------------------------------------------------------

test('the empty answer is an empty list, never an invented row', () => {
  assert.deepEqual(parseLayersAnswer(null), { ...EMPTY_LAYERS_ANSWER });
  assert.deepEqual(parseLayersAnswer(undefined), { ...EMPTY_LAYERS_ANSWER });
  assert.deepEqual(parseLayersAnswer({}), { ...EMPTY_LAYERS_ANSWER });
  assert.deepEqual(parseLayersAnswer({ layers: [] }), { ...EMPTY_LAYERS_ANSWER });
  // A non-array `layers` field is not coerced into something.
  assert.deepEqual(parseLayersAnswer({ layers: 'WALLS' }), { ...EMPTY_LAYERS_ANSWER });
});

test('the answer is normalised row by row, dropping only unaddressable rows', () => {
  const parsed = parseLayersAnswer(answer([
    row('0', { current: true }),
    row('WALLS'),
    // No name: it can neither be shown nor addressed, so it is not a row.
    { visible: true },
    null,
    'WALLS',
    // A repeated name is the same layer as the first occurrence.
    row('WALLS', { visible: false }),
    row('DIMS', { databaseVisible: false, overridden: true }),
  ], { active: 'WALLS', overridden: 1 }));
  assert.deepEqual(parsed.layers.map(layer => layer.name), ['0', 'WALLS', 'DIMS']);
  assert.equal(parsed.layers[0].current, true);
  assert.equal(parsed.layers[1].visible, true, 'the first occurrence wins');
  assert.deepEqual(parsed.layers[2], {
    name: 'DIMS', visible: true, databaseVisible: false, overridden: true, current: false,
    color: { aci: 7, rgb: '#ffffff' }, linetype: 'Continuous', lineweight: 'ByLayer',
    transparency: '0', locked: false, frozen: false, off: false, plottable: true,
  });
  assert.equal(parsed.active, 'WALLS');
  assert.equal(parsed.overridden, 1);
});

test('missing or non-boolean flags read as false instead of a guessed state', () => {
  const parsed = parseLayersAnswer(answer([{ name: '0', visible: 1, databaseVisible: 'yes', overridden: 'no' }]));
  // The attribute fields are defaults too: no colour it can paint (`rgb: ''`),
  // no linetype/lineweight/transparency text, every flag false.
  assert.deepEqual(parsed.layers[0], {
    name: '0', visible: false, databaseVisible: false, overridden: false, current: false,
    color: { aci: null, rgb: '' }, linetype: '', lineweight: '', transparency: '',
    locked: false, frozen: false, off: false, plottable: false,
  });
  // A non-string/empty active layer is "no current layer", not the string "null".
  assert.equal(parseLayersAnswer(answer([row('0')], { active: 42 })).active, null);
  assert.equal(parseLayersAnswer(answer([row('0')], { active: '' })).active, null);
});

test('the override counter is the host number, or counted from the same rows', () => {
  const rows = [row('0', { overridden: true }), row('WALLS')];
  assert.equal(parseLayersAnswer(answer(rows, { overridden: 1 })).overridden, 1);
  // No counter field at all: counted from the rows of the same answer, never
  // estimated from anything else.
  assert.equal(parseLayersAnswer({ layers: rows }).overridden, 1);
  // A negative or non-integer counter is unusable, so it is counted instead.
  assert.equal(parseLayersAnswer(answer(rows, { overridden: -2 })).overridden, 1);
  assert.equal(parseLayersAnswer(answer(rows, { overridden: '1' })).overridden, 1);
  // A usable host number wins (including a real zero).
  assert.equal(parseLayersAnswer(answer(rows, { overridden: 0 })).overridden, 0, 'the host number wins');
});

// --- projections --------------------------------------------------------

test('search is a trimmed case-insensitive substring match', () => {
  const rows = parseLayersAnswer(answer([row('0'), row('WALLS'), row('inner wall'), row('DIMS')])).layers;
  assert.deepEqual(filterLayers(rows, 'wall').map(layer => layer.name), ['WALLS', 'inner wall']);
  assert.deepEqual(filterLayers(rows, '  WALL ').map(layer => layer.name), ['WALLS', 'inner wall']);
  assert.deepEqual(filterLayers(rows, 'inner').map(layer => layer.name), ['inner wall']);
  assert.deepEqual(filterLayers(rows, 'nothing'), []);
  // An empty (or whitespace) needle is "no filter": the very same array, so a
  // cleared search never re-renders the list for nothing.
  assert.equal(filterLayers(rows, ''), rows);
  assert.equal(filterLayers(rows, '   '), rows);
  assert.equal(filterLayers(rows, null), rows);
  assert.deepEqual(filterLayers(null, 'x'), []);
});

test('visibleLayers and overrideCount are projections over the rows', () => {
  const rows = parseLayersAnswer(answer([
    row('0'),
    row('WALLS', { visible: false }),
    row('DIMS', { overridden: true, visible: false }),
  ])).layers;
  assert.deepEqual(visibleLayers(rows).map(layer => layer.name), ['0']);
  assert.equal(overrideCount(rows), 1);
  assert.equal(overrideCount([]), 0);
  assert.equal(overrideCount(null), 0);
  assert.deepEqual(visibleLayers(null), []);
});

test('the identity dot is name-derived, deterministic and deliberately muted', () => {
  // The same name answers the same tone, whether it arrives as a row or a string.
  assert.deepEqual(layerTone('WALLS'), layerTone({ name: 'WALLS' }));
  assert.deepEqual(layerTone('WALLS'), layerTone('WALLS'));
  assert.notDeepEqual(layerTone('WALLS'), layerTone('DIMS'));
  assert.ok(Number.isInteger(layerNameHash('WALLS')));
  assert.equal(layerNameHash(''), layerNameHash(''), 'stable for the empty name too');
  // A hash, not the drawing's colour: the stored colour is on the wire
  // (`row.color`), but this projection stays a muted name-derived marker, so a
  // swatch never claims to be the user's layer colour.
  const names = ['0', 'WALLS', 'DIMS', 'HATCH', 'TEXT', 'CENTER', 'HIDDEN'];
  const tones = names.map(name => layerTone(name));
  for (const [index, tone] of tones.entries()) {
    assert.match(tone.color, /^#[0-9a-f]{6}$/);
    const channels = [0, 2, 4].map(offset => Number.parseInt(tone.color.slice(1 + offset, 3 + offset), 16));
    for (const channel of channels) {
      assert.ok(channel >= 0x80 && channel <= 0xbe, `${names[index]} channel ${channel} stays muted`);
    }
  }
  // Different names really do separate.
  assert.ok(new Set(tones.map(tone => tone.color)).size >= 6);
});

test('the focus cursor steps and wraps, and Home/End land on the ends', () => {
  assert.equal(nextFocusIndex(0, 1, 3), 1);
  assert.equal(nextFocusIndex(2, 1, 3), 0, 'ArrowDown wraps');
  assert.equal(nextFocusIndex(0, -1, 3), 2, 'ArrowUp wraps');
  // No cursor yet: the first step lands on the first row, the last on the last.
  assert.equal(nextFocusIndex(-1, 1, 3), 0, 'Home');
  assert.equal(nextFocusIndex(-1, -1, 3), 2, 'End');
  assert.equal(nextFocusIndex(99, 1, 3), 0);
  assert.equal(nextFocusIndex(1, 10, 3), 2);
  assert.equal(nextFocusIndex(1, 0, 3), 1);
  // An empty list has no position at all.
  assert.equal(nextFocusIndex(0, 1, 0), -1);
  assert.equal(nextFocusIndex(-1, -1, 0), -1);
});

// --- error classification ----------------------------------------------

test('the host "no session" code is recognised and never read as an empty list', () => {
  assert.equal(WAITING_CODE, 'cad_waiting');
  assert.equal(isWaitingError('cad_waiting'), true);
  assert.equal(isWaitingError('cad_waiting: no session'), true);
  assert.equal(isWaitingError('layer.unknown: WALLS'), false);
  assert.equal(isWaitingError(''), false);
  assert.equal(errorCode('layer.unknown: WALLS'), 'layer.unknown');
  assert.equal(errorCode('unsupported_on_surface_mode'), 'unsupported_on_surface_mode');
  assert.equal(errorCode(''), '');
});

test('an ack admits unless it explicitly refuses', () => {
  assert.equal(ackAdmitted({ accepted: true }), true);
  assert.equal(ackAdmitted({ admitted: true, command: 'LAYER' }), true);
  assert.equal(ackAdmitted(undefined), true, 'a resolved call is an admission');
  assert.equal(ackAdmitted({ accepted: false }), false);
  assert.equal(ackAdmitted({ admitted: false }), false);
});

// --- the IPC wrappers ---------------------------------------------------

test('each wrapper sends its own command and payload, and maps a throw to a reason', async () => {
  const calls = [];
  const ok = async (command, args) => { calls.push([command, args]); return { accepted: true }; };
  assert.deepEqual(await setLayerVisible(ok, 'WALLS', false), { kind: 'ok' });
  assert.deepEqual(await restoreLayers(ok), { kind: 'ok' });
  assert.deepEqual(await setActiveLayer(ok, 'WALLS'), { kind: 'ok' });
  assert.deepEqual(calls, [
    [SET_LAYER_VISIBLE_COMMAND, { name: 'WALLS', visible: false }],
    [RESTORE_LAYERS_COMMAND, undefined],
    [SET_ACTIVE_LAYER_COMMAND, { name: 'WALLS' }],
  ]);
  // The visibility payload is always a boolean.
  calls.length = 0;
  await setLayerVisible(ok, 'WALLS', 'yes');
  assert.deepEqual(calls[0][1], { name: 'WALLS', visible: false });
  // A refusal throws; the raw reason travels untouched.
  const refusing = async () => { throw new Error('layer.unknown: WALLS'); };
  assert.deepEqual(await setLayerVisible(refusing, 'WALLS', true), {
    kind: 'error', reason: 'layer.unknown: WALLS',
  });
  assert.deepEqual(await restoreLayers(refusing), { kind: 'error', reason: 'layer.unknown: WALLS' });
  // An addressless name never reaches the host.
  const unreachable = async () => { throw Error('name reached IPC'); };
  assert.deepEqual(await setLayerVisible(unreachable, '', true), { kind: 'invalid' });
  assert.deepEqual(await setActiveLayer(unreachable, '', true), { kind: 'invalid' });
});

test('refreshLayers normalises the answer and never throws', async () => {
  const ok = await refreshLayers(async command => {
    assert.equal(command, LAYERS_COMMAND);
    return answer([row('WALLS', { current: true })], { active: 'WALLS', overridden: 0 });
  });
  assert.equal(ok.kind, 'ok');
  assert.deepEqual(ok.layers.map(layer => layer.name), ['WALLS']);
  assert.equal(ok.active, 'WALLS');
  const failed = await refreshLayers(async () => { throw new Error('cad_waiting'); });
  assert.deepEqual(failed, { kind: 'error', reason: 'cad_waiting' });
});

// --- the store ---------------------------------------------------------

test('the first read fills the rows, and an empty answer is its own state', async () => {
  const store = createLayersStore(async () => answer([row('0', { current: true }), row('WALLS')]));
  assert.equal(store.getState().phase, 'idle');
  const loading = store.load();
  assert.equal(store.getState().phase, 'loading', 'the first read reports its pending state');
  await loading;
  const state = store.getState();
  assert.equal(state.phase, 'ready');
  assert.deepEqual(state.layers.map(layer => layer.name), ['0', 'WALLS']);
  assert.equal(state.error, null);
  assert.equal(state.busy, false);

  const emptyStore = createLayersStore(async () => ({ layers: [], active: null, overridden: 0 }));
  await emptyStore.load();
  assert.equal(emptyStore.getState().phase, 'empty');
  assert.deepEqual(emptyStore.getState().layers, []);
});

test('cad_waiting is a not-ready state, not an empty drawing and not an error', async () => {
  const store = createLayersStore(async () => { throw new Error('cad_waiting: no session'); },
    { wait: () => Promise.resolve() });
  await store.load();
  const state = store.getState();
  assert.equal(state.phase, 'waiting');
  assert.deepEqual(state.layers, []);
  assert.equal(state.error, null, 'nothing is wrong that the user could fix');
});

test('the not-ready window self-heals with one bounded follow-up read', async () => {
  const calls = [];
  const store = createLayersStore(async (command) => {
    calls.push(command);
    // The first read lands in the frame before the host's first mirror.
    if (calls.length === 1) throw new Error('cad_waiting');
    return answer([row('0', { current: true })]);
  }, { wait: () => Promise.resolve() });
  await store.load();
  assert.deepEqual(calls, [LAYERS_COMMAND, LAYERS_COMMAND]);
  assert.equal(store.getState().phase, 'ready');

  // A host that stays not-ready is asked exactly twice, never in a loop.
  const stuck = [];
  const cold = createLayersStore(async () => {
    stuck.push(LAYERS_COMMAND);
    throw new Error('cad_waiting');
  }, { wait: () => Promise.resolve() });
  await cold.load();
  assert.equal(stuck.length, 2);
  assert.equal(cold.getState().phase, 'waiting');
});

test('a failed read keeps the rows it already has, and says why', async () => {
  let fail = false;
  const store = createLayersStore(async () => {
    if (fail) throw new Error('cad_layers: host gone');
    return answer([row('0'), row('WALLS')]);
  });
  await store.load();
  fail = true;
  await store.load();
  const state = store.getState();
  assert.equal(state.phase, 'ready', 'the list stays on screen');
  assert.equal(state.layers.length, 2);
  assert.deepEqual(state.error, { kind: 'read', reason: 'cad_layers: host gone' });

  // With nothing to keep, the failure is the whole panel state.
  const cold = createLayersStore(async () => { throw new Error('cad_layers: host gone'); });
  await cold.load();
  assert.equal(cold.getState().phase, 'error');
  assert.deepEqual(cold.getState().error, { kind: 'read', reason: 'cad_layers: host gone' });
});

test('a mutation is single flight: a second call is refused locally, never queued', async () => {
  const gate = deferred();
  const calls = [];
  const store = createLayersStore(async (command, args) => {
    calls.push(command);
    if (command === SET_LAYER_VISIBLE_COMMAND) return gate.promise;
    if (command === LAYERS_COMMAND) return answer([row('WALLS')]);
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();
  const first = store.setVisible('WALLS', false);
  assert.equal(store.getState().busy, true);
  const second = await store.setVisible('WALLS', true);
  assert.deepEqual(second, { kind: 'busy' });
  assert.equal(calls.filter(command => command === SET_LAYER_VISIBLE_COMMAND).length, 1,
    'the second click never reaches the host');
  gate.resolve({ accepted: true });
  assert.deepEqual(await first, { kind: 'ok' });
  assert.equal(store.getState().busy, false);
});

test('an admitted mutation reads the host back after the host tick, never optimistically', async () => {
  const log = [];
  let visible = true;
  let applyOnHost = true;
  const store = createLayersStore(async (command, args) => {
    log.push(command);
    if (command === SET_LAYER_VISIBLE_COMMAND) {
      if (applyOnHost) visible = args.visible;
      return { accepted: true };
    }
    if (command === LAYERS_COMMAND) return answer([row('WALLS', { visible })]);
    throw new Error(`unexpected ${command}`);
  }, { wait: async () => { log.push('tick'); } });
  await store.load();
  log.length = 0;
  await store.setVisible('WALLS', false);
  assert.deepEqual(log, [SET_LAYER_VISIBLE_COMMAND, 'tick', LAYERS_COMMAND],
    'the ack is a queue, so the read waits for the host tick');
  assert.equal(store.getState().layers[0].visible, false, 'the host answer is what the row shows');

  // The read is the only source of the rows: a host that still reports the old
  // state keeps the old row instead of the value the click asked for.
  applyOnHost = false;
  log.length = 0;
  await store.setVisible('WALLS', true);
  assert.equal(store.getState().layers[0].visible, false);
  assert.deepEqual(log, [SET_LAYER_VISIBLE_COMMAND, 'tick', LAYERS_COMMAND, 'tick', LAYERS_COMMAND],
    'a projection that does not show the write yet gets exactly one follow-up read');
});

test('the read-back is bounded: it never turns into a poll loop', async () => {
  const calls = [];
  const store = createLayersStore(async (command) => {
    calls.push(command);
    if (command === SET_LAYER_VISIBLE_COMMAND) return { accepted: true };
    if (command === LAYERS_COMMAND) return answer([row('WALLS', { visible: true })]);
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();
  calls.length = 0;
  await store.setVisible('WALLS', false);
  // The host never applies it in this test: after the follow-up read the panel
  // reports the host's own answer and stops asking.
  assert.deepEqual(calls, [SET_LAYER_VISIBLE_COMMAND, LAYERS_COMMAND, LAYERS_COMMAND]);
  assert.equal(store.getState().busy, false);
  assert.equal(store.getState().layers[0].visible, true);
  // The delay covers at least one host frame (`LAYER_READ_DELAY_MS`), otherwise
  // the read would always land before the write was applied.
  assert.ok(LAYER_READ_DELAY_MS >= 16, `the read-back must outlast one host frame, saw ${LAYER_READ_DELAY_MS}`);
});

test('a refused mutation keeps the previous list and reports the real reason', async () => {
  const calls = [];
  const store = createLayersStore(async (command) => {
    calls.push(command);
    if (command === SET_LAYER_VISIBLE_COMMAND) throw new Error('layer.unknown: WALLS');
    if (command === LAYERS_COMMAND) return answer([row('0'), row('WALLS')], { overridden: 0 });
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();
  calls.length = 0;
  const result = await store.setVisible('WALLS', false);
  assert.deepEqual(result, { kind: 'error', reason: 'layer.unknown: WALLS' });
  const state = store.getState();
  assert.equal(state.layers.length, 2, 'no row is dropped on a refusal');
  assert.equal(state.busy, false);
  assert.deepEqual(state.error, { kind: 'mutation', reason: 'layer.unknown: WALLS' });
  assert.deepEqual(calls, [SET_LAYER_VISIBLE_COMMAND], 'a refused call is not read back');
});

test('restore and set-current ride the same single-flight path, and the read owns the count', async () => {
  const calls = [];
  let overridden = 0;
  let active = null;
  const store = createLayersStore(async (command, args) => {
    calls.push([command, args]);
    if (command === RESTORE_LAYERS_COMMAND) {
      overridden = 0;
      return { accepted: true };
    }
    if (command === SET_ACTIVE_LAYER_COMMAND) {
      active = args.name;
      return { accepted: true };
    }
    if (command === LAYERS_COMMAND) {
      return answer(
        [row('0', { current: active === '0' }), row('WALLS', { current: active === 'WALLS' })],
        { active, overridden },
      );
    }
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();
  assert.equal(store.getState().overridden, 0);

  overridden = 2;
  await store.load();
  assert.equal(store.getState().overridden, 2);
  await store.restore();
  assert.equal(store.getState().overridden, 0, 'the count comes from the read-back');
  assert.deepEqual(calls.at(-2), [RESTORE_LAYERS_COMMAND, undefined]);
  assert.equal(calls.at(-1)[0], LAYERS_COMMAND);

  await store.setActive('WALLS');
  assert.equal(store.getState().active, 'WALLS');
  assert.equal(store.getState().layers.find(layer => layer.name === 'WALLS').current, true);
  assert.deepEqual(calls.at(-2), [SET_ACTIVE_LAYER_COMMAND, { name: 'WALLS' }]);
});

test('reads are serialised, so a document change during a read still gets its own read', async () => {
  const gate = deferred();
  const calls = [];
  const store = createLayersStore(async (command) => {
    calls.push(command);
    if (calls.length === 1) return gate.promise;
    return answer([row('WALLS')]);
  });
  const first = store.load();
  const second = store.load();
  assert.equal(calls.length, 1, 'the second read waits for the first');
  gate.resolve(answer([row('0')]));
  await Promise.all([first, second]);
  assert.equal(calls.length, 2);
  assert.deepEqual(store.getState().layers.map(layer => layer.name), ['WALLS'],
    'the newest answer wins');
});

test('the empty state is the documented shape', () => {
  assert.deepEqual({ ...EMPTY_LAYERS_STATE }, {
    phase: 'idle', layers: [], active: null, overridden: 0, busy: false, error: null,
  });
});

// --- the structural layer writes (create / rename / delete) -------------

test('the three table writes send their own command and payload', async () => {
  assert.equal(CREATE_LAYER_COMMAND, 'cad_create_layer');
  assert.equal(RENAME_LAYER_COMMAND, 'cad_rename_layer');
  assert.equal(DELETE_LAYER_COMMAND, 'cad_delete_layer');

  const calls = [];
  const ok = async (command, args) => { calls.push([command, args]); return { accepted: true }; };
  assert.deepEqual(await createLayer(ok, 'DOORS'), { kind: 'ok' });
  assert.deepEqual(await renameLayer(ok, 'DOORS', 'PARTITIONS'), { kind: 'ok' });
  assert.deepEqual(await deleteLayer(ok, 'PARTITIONS'), { kind: 'ok' });
  assert.deepEqual(calls, [
    [CREATE_LAYER_COMMAND, { name: 'DOORS' }],
    [RENAME_LAYER_COMMAND, { layer: 'DOORS', name: 'PARTITIONS' }],
    [DELETE_LAYER_COMMAND, { layer: 'PARTITIONS' }],
  ]);

  // An unaddressable row or a nameless create never reaches the host.
  const unreachable = async () => { throw Error('reached IPC'); };
  assert.deepEqual(await createLayer(unreachable, ''), { kind: 'invalid' });
  assert.deepEqual(await renameLayer(unreachable, '', 'X'), { kind: 'invalid' });
  assert.deepEqual(await renameLayer(unreachable, 'X', ''), { kind: 'invalid' });
  assert.deepEqual(await deleteLayer(unreachable, ''), { kind: 'invalid' });

  // The database's own refusals travel untouched.
  const refusing = async () => { throw new Error('layer.duplicate_name: this drawing already has a layer named \'WALLS\''); };
  const refused = await createLayer(refusing, 'walls');
  assert.equal(refused.kind, 'error');
  assert.equal(errorCode(refused.reason), 'layer.duplicate_name');
});

test('create, rename and delete re-read the host instead of inventing a row', async () => {
  const log = [];
  let rows = [row('0', { current: true }), row('WALLS', { overridden: true })];
  let applyOnHost = true;
  const store = createLayersStore(async (command, args) => {
    log.push([command, args]);
    if (command === CREATE_LAYER_COMMAND) {
      if (applyOnHost) rows = [...rows, row(args.name, { databaseVisible: true })];
      return { accepted: true };
    }
    if (command === RENAME_LAYER_COMMAND) {
      if (applyOnHost) {
        rows = rows.map(layer => layer.name === args.layer
          ? { ...layer, name: args.name }
          : layer);
      }
      return { accepted: true };
    }
    if (command === DELETE_LAYER_COMMAND) {
      if (applyOnHost) rows = rows.filter(layer => layer.name !== args.layer);
      return { accepted: true };
    }
    if (command === LAYERS_COMMAND) {
      return answer(rows, { active: '0', overridden: overrideCount(rows) });
    }
    throw new Error(`unexpected ${command}`);
  }, { wait: async () => { log.push('tick'); } });
  await store.load();
  log.length = 0;

  assert.deepEqual(await store.createLayer('DOORS'), { kind: 'ok' });
  assert.deepEqual(log, [[CREATE_LAYER_COMMAND, { name: 'DOORS' }], 'tick', [LAYERS_COMMAND, undefined]],
    'the ack is a queue, so the row arrives with the read-back');
  assert.deepEqual(store.getState().layers.map(layer => layer.name), ['0', 'WALLS', 'DOORS']);

  log.length = 0;
  await store.renameLayer('WALLS', 'PARTITIONS');
  assert.deepEqual(store.getState().layers.map(layer => layer.name), ['0', 'PARTITIONS', 'DOORS']);
  assert.equal(store.getState().layers.find(layer => layer.name === 'WALLS'), undefined,
    'the old name is gone');
  // Renaming never clears the session's temporary override.
  assert.equal(store.getState().layers.find(layer => layer.name === 'PARTITIONS').overridden, true);
  assert.equal(store.getState().overridden, 1);

  log.length = 0;
  await store.deleteLayer('DOORS');
  assert.deepEqual(store.getState().layers.map(layer => layer.name), ['0', 'PARTITIONS']);
  assert.equal(store.getState().active, '0', 'the active layer comes from the read-back');

  // A host that has not applied the write yet gets exactly one follow-up read,
  // and the panel then shows the host's own answer — never the requested one.
  applyOnHost = false;
  log.length = 0;
  await store.createLayer('MISSING');
  assert.deepEqual(log.map(entry => (Array.isArray(entry) ? entry[0] : entry)),
    [CREATE_LAYER_COMMAND, 'tick', LAYERS_COMMAND, 'tick', LAYERS_COMMAND]);
  assert.equal(store.getState().layers.some(layer => layer.name === 'MISSING'), false);
});

test('a refused delete keeps every row and reports the stable reason', async () => {
  const calls = [];
  let refusal = 'layer.in_use: layer \'WALLS\' (id 2) is still referenced by 4 entities; not deleted';
  const store = createLayersStore(async (command) => {
    calls.push(command);
    if (command === DELETE_LAYER_COMMAND) throw new Error(refusal);
    if (command === LAYERS_COMMAND) return answer([row('0', { current: true }), row('WALLS')]);
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();
  calls.length = 0;

  const refused = await store.deleteLayer('WALLS');
  assert.equal(refused.kind, 'error');
  assert.equal(errorCode(refused.reason), 'layer.in_use');
  assert.deepEqual(calls, [DELETE_LAYER_COMMAND], 'a refused write is not read back');
  assert.equal(store.getState().layers.length, 2, 'no row is dropped on a refusal');
  assert.deepEqual(store.getState().error, { kind: 'mutation', reason: refused.reason });
  assert.equal(store.getState().busy, false);

  // Deleting the default layer is refused for its own reason.
  refusal = 'layer.default_undeletable: layer \'0\' is the drawing\'s default layer and cannot be deleted';
  const zero = await store.deleteLayer('0');
  assert.equal(errorCode(zero.reason), 'layer.default_undeletable');
  assert.equal(store.getState().layers.length, 2);
});

test('the structural writes are single flight and never send a nameless request', async () => {
  const gate = deferred();
  const calls = [];
  const store = createLayersStore(async (command) => {
    calls.push(command);
    if (command === CREATE_LAYER_COMMAND) return gate.promise;
    if (command === LAYERS_COMMAND) return answer([row('0', { current: true })]);
    throw new Error(`unexpected ${command}`);
  }, { wait: () => Promise.resolve() });
  await store.load();

  const first = store.createLayer('DOORS');
  assert.deepEqual(await store.renameLayer('0', 'ROOT'), { kind: 'busy' },
    'the window covers the queued write and its read-back');
  gate.resolve({ accepted: true });
  assert.deepEqual(await first, { kind: 'ok' });
  assert.equal(calls.filter(command => command === CREATE_LAYER_COMMAND).length, 1);
  assert.deepEqual(await store.createLayer(''), { kind: 'invalid' });
  assert.deepEqual(await store.deleteLayer(''), { kind: 'invalid' });
});

// --- source / catalog assertions ---------------------------------------

test('the palette is docked next to the other palettes and opened by a shell action', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("import { LayersPalette } from './LayersPalette.tsx';"));
  assert.ok(workspace.includes('const showLayers = palettesAvailable && Boolean(shell.layersOpen);'));
  assert.ok(workspace.includes('{showLayers ? <LayersPalette /> : null}'));
  // It contributes to the dock being open, and never paints over the partition.
  assert.ok(workspace.includes('const leftOpen = showStart || showProperties || showDiagnostics || showLayers;'));

  const app = await readModule('App.tsx');
  assert.ok(app.includes("import { isLayersCommand } from './layers/layers.ts';"));
  assert.ok(app.includes('const [layersOpen, setLayersOpen] = useState(false);'),
    'closed by default, so the start page keeps its geometry');
  assert.ok(app.includes('const [layersFocus, setLayersFocus] = useState(0);'));
  assert.ok(app.includes('setLayersFocus(value => value + 1)'), 'a repeat LAYER re-focuses and re-reads');
  assert.ok(app.includes('layersOpen,'));
  assert.ok(app.includes('toggleLayers: () => setLayersOpen(value => !value)'));
  // The selector is intercepted before the in-flight guard, like OPTIONS/PROPERTIES.
  const intercept = app.indexOf('isLayersCommand(text)');
  assert.ok(intercept > 0);
  assert.ok(intercept < app.indexOf('submitting.current = true'));

  const palette = await readModule('components/LayersPalette.tsx');
  assert.ok(palette.includes('useSyncExternalStore'), 'the panel subscribes to the store, not to polls');
  assert.ok(palette.includes('shell.setLayersOpen(false)'), 'it closes through the same plumbing');
  assert.ok(palette.includes('shell.layersFocus'), 'a repeat LAYER focuses and re-reads it');
  assert.ok(palette.includes('shell.documentLabel'), 'a new document re-reads the table');
  // No overlay, no backdrop: the CAD partition keeps its pointer input.
  assert.equal(palette.includes('position: fixed'), false);
  assert.equal(palette.includes('role="dialog"'), false);
  assert.equal(palette.includes('aria-modal'), false);
});

test('LAYER is not offered as a ribbon affordance on a carrier without palettes', () => {
  // The layer-state commands ride the same carrier filter: a carrier that never
  // renders the layers palette has no surface for the isolation or off/on state.
  assert.deepEqual([...PALETTE_COMMANDS], ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYOFF', 'LAYON']);
  const layer = { id: 'LAYER', labelKey: 'layers.panel', command: 'LAYER' };
  const properties = { id: 'PROPERTIES', labelKey: 'properties.panel', action: 'properties' };
  const items = [layer, properties, { id: 'OPEN', command: 'OPEN' }];
  assert.deepEqual(offeredItems(items, false).map(item => item.id), ['OPEN']);
  assert.equal(offeredItems(items, true), items);
});

test('every sentence the palette shows comes from the catalog, in both languages', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const keys = [
    'layers.panel',
    'layers.restore',
    'layers.override_count',
    'layers.overridden_marker',
    'layers.empty',
    'layers.search_placeholder',
    'layers.clear_search',
    'layers.no_matches',
    'layers.current',
    'layers.set_current',
    'layers.visibility_on',
    'layers.visibility_off',
    'layers.database_hidden',
    'layers.write_failed',
    'layers.read_failed',
    'layers.close',
    // The three table-write forms. The palette renders every one of them: the two
    // header/row entry points, the row's two actions, the create field with its
    // rules, and the confirm step's note.
    'layers.new',
    'layers.rename',
    'layers.delete',
    'layers.name_label',
    'layers.new_hint',
    'layers.delete_confirm',
    'layers.delete_in_use_hint',
    // Reused, not invented: the Cancel verb every tool prompt already uses.
    'tool.cancel',
    // Reused, not invented: the not-ready sentence is the catalog's existing
    // wording for the same fact.
    'host.poll_not_ready',
  ];
  const palette = await readModule('components/LayersPalette.tsx');
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', key);
    assert.equal(typeof en[key], 'string', key);
    assert.ok(palette.includes(`'${key}'`), `the palette renders ${key}`);
  }
  // The attribute surface (color, the four flags with their honest hints, the
  // read-only secondaries). The marks' labels/hints are copy-keyed in
  // `layer-attributes.ts` (pure data) and the details line is its own component
  // (`LayerDetails.tsx`, so the palette keeps a single row of markup), so a key may
  // be literal in any of the three; `layers-attributes.test.ts` covers the
  // behavior.
  const attributes = await readModule('layers/layer-attributes.ts');
  const details = await readModule('components/LayerDetails.tsx');
  const renders = (key: string) =>
    palette.includes(`'${key}'`) || attributes.includes(`'${key}'`) || details.includes(`'${key}'`);
  const attributeKeys = [
    'layers.off', 'layers.frozen', 'layers.locked', 'layers.plottable', 'layers.no_plot',
    'layers.off.hint', 'layers.frozen.hint', 'layers.locked.hint',
    'layers.plottable.hint', 'layers.no_plot.hint', 'layers.marks_more', 'layers.marks_more.hint',
    'layers.attributes', 'layers.attributes.close', 'layers.attributes.hint',
    'layers.color', 'layers.color.aci', 'layers.color.hint',
    'layers.color.marker', 'layers.color.reported',
    'layers.linetype', 'layers.lineweight', 'layers.transparency',
  ];
  for (const key of attributeKeys) {
    assert.equal(typeof zh[key], 'string', key);
    assert.equal(typeof en[key], 'string', key);
    assert.ok(renders(key), `the palette renders ${key}`);
  }
  // The mutation failure sentence is neutral across visibility/create/rename/
  // delete (it used to name visibility only) and carries the host's real reason.
  assert.ok(palette.includes("t('layers.write_failed', { reason: state.error.reason })"));
  // The delete confirm names the layer it is about to remove.
  assert.ok(palette.includes("t('layers.delete_confirm', { name: layer.name })"));
  // The visibility toggle's accessible name is the action it performs.
  assert.ok(palette.includes("t('layers.visibility_off') : t('layers.visibility_on')"));
  // A forced-on layer says so: the drawing's own flag is not overwritten.
  assert.ok(palette.includes('layer.visible && !layer.databaseVisible'));
  assert.ok(palette.includes("t('layers.database_hidden')"));
  // The one refusal this panel makes itself (a blank name, refused before any
  // IPC) reuses the catalog's own sentence instead of carrying copy of its own.
  assert.ok(palette.includes("setDraftError('layers.error.layer.name_empty')"));
  assert.equal(/[\u3400-\u9fff]/.test(palette), false, 'and it is never written in the source');
});

test('every layer refusal code maps to catalog copy, in both languages', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  // The host refuses a layer-table write with a stable code, and the catalog keys
  // one sentence per code (`layers.error.<wire code>`, the same
  // `options.error.<wire code>` convention the options dialog uses). The mapper
  // (`layer-labels.ts`) is the only place the two are joined.
  const refusals = [
    'layer.name_empty',
    'layer.name_illegal_char',
    'layer.duplicate_name',
    'layer.default_undeletable',
    'layer.in_use',
    // The two attribute refusals the palette's own controls can provoke.
    'layer.property.unsupported',
    'layer.property.value',
  ];
  const labels = await readModule('layers/layer-labels.ts');
  for (const code of refusals) {
    const key = `layers.error.${code}`;
    assert.equal(typeof zh[key], 'string', `zh-CN ${key}`);
    assert.equal(typeof en[key], 'string', `en ${key}`);
    assert.notEqual(zh[key].trim(), '', `zh-CN ${key} is not blank`);
    assert.notEqual(en[key].trim(), '', `en ${key} is not blank`);
    // No placeholder in either copy: these sentences name no measurement.
    assert.equal(/\{[a-z]+\}/i.test(zh[key]) || /\{[a-z]+\}/i.test(en[key]), false, key);
    // The mapper names the catalog key **literally**, so neither half of the pair
    // can be dropped silently: removing the copy fails the catalog checks above,
    // removing the mapping fails this one. The behavioural `layerErrorKey` call is
    // the same pair, not a second, looser spelling of it.
    assert.ok(labels.includes(`case '${code}': return '${key}';`), `${code} maps to ${key}`);
    assert.equal(layerErrorKey(code), key);
  }
  // The host appends its own prose after the code (`layer.in_use: layer 'WALLS'
  // (id 2) is still referenced by 4 entities; not deleted`); only the code is
  // matched, so the sentence can change without breaking the lookup.
  assert.equal(
    layerErrorKey(errorCode("layer.in_use: layer 'WALLS' (id 2) is still referenced by 4 entities; not deleted")),
    'layers.error.layer.in_use',
  );
  // A code this build has no sentence for is never guessed at: the palette falls
  // back to its generic refusal sentence plus the host's own words.
  for (const unmapped of ['layer.unknown', 'cad_waiting', '', null, undefined, 42, {}]) {
    assert.equal(layerErrorKey(unmapped), null, `${String(unmapped)} is not mapped`);
  }
  // And the palette really routes a refused write through it.
  const palette = await readModule('components/LayersPalette.tsx');
  assert.ok(
    palette.includes('layerErrorKey(errorCode(state.error.reason))'),
    'a refused write is named by its code, never by the raw reason alone',
  );
  assert.ok(
    palette.indexOf('layerErrorKey(errorCode(state.error.reason))') < palette.indexOf('t(key) : t(\'layers.write_failed\''),
    'the mapped copy is what wins; the raw reason is the fallback',
  );
  // The model declares the three IPC names and the three store actions.
  const model = await readModule('layers/layers.ts');
  for (const name of ['cad_create_layer', 'cad_rename_layer', 'cad_delete_layer']) {
    assert.ok(model.includes(`'${name}'`), `${name} is the wire name the model sends`);
  }
  for (const action of ['createLayer(name: string)', 'renameLayer(layer: string, name: string)', 'deleteLayer(layer: string)']) {
    assert.ok(model.includes(action), `the store exposes ${action}`);
  }
});

test('the default layer is never offered for deletion', () => {
  // The host refuses it (`layer.default_undeletable`); a destructive affordance
  // that can only ever be refused must not be shown at all.
  assert.equal(DEFAULT_LAYER_NAME, '0');
  assert.equal(deletableLayer('0'), false);
  assert.equal(deletableLayer(DEFAULT_LAYER_NAME), false);
  assert.equal(deletableLayer('WALLS'), true);
  assert.equal(deletableLayer('inner wall'), true);
  // A row with no addressable name is not a layer, so nothing is offered for it.
  assert.equal(deletableLayer(''), false);
  assert.equal(deletableLayer(null), false);
  assert.equal(deletableLayer(undefined), false);
  assert.equal(deletableLayer(0), false, 'the number is not the name');
});

test('create, rename and delete are one inline form each, keyboard and focus included', async () => {
  const palette = await readModule('components/LayersPalette.tsx');
  const occurrences = (needle: string) => palette.split(needle).length - 1;

  // --- entry points: real buttons, named by catalog copy --------------------
  for (const label of ['t(\'layers.new\')', 't(\'layers.rename\')', 't(\'layers.delete\')']) {
    assert.ok(palette.includes(`label={${label}}`), `${label} names a real button`);
  }
  // Only one form at a time: a single piece of state decides which.
  assert.equal(occurrences('setForm({ kind:'), 3, 'create, rename and delete each open the one form');
  assert.ok(palette.includes("form?.kind === 'create'"), 'the create trigger and the create form agree');
  // A create is only offered for a layer table the host answered, and the open
  // form always keeps its own trigger mounted so Escape has a focus target.
  assert.ok(palette.includes("const canCreate = state.phase === 'ready' || state.phase === 'empty';"));
  assert.ok(palette.includes("canCreate || form?.kind === 'create'"));
  // Both rename paths are the same call: the double-click shortcut and the pencil.
  assert.equal(occurrences('openRename(layer)'), 2, 'double-click and the explicit pencil both rename');
  assert.ok(palette.includes('onDoubleClick={() => openRename(layer)}'));
  // Delete is gated on the row's name, not on a guess.
  assert.ok(palette.includes('deletableLayer(layer.name)'), 'the default layer offers no delete');

  // --- one key handler for both fields: Enter submits, Escape cancels -------
  assert.equal(occurrences('onFieldKeyDown(event'), 2, 'the create field and the rename field share one handler');
  assert.ok(palette.includes("if (event.key === 'Escape')"), 'Escape cancels');
  assert.ok(palette.includes("if (event.key === 'Enter')"), 'Enter submits');
  assert.ok(palette.includes('event.isComposing || event.keyCode === 229'), 'an IME keeps its own Enter');
  // The row itself closes on Escape too (the confirm has no field to catch it).
  assert.ok(palette.includes("event.key !== 'Escape' || event.defaultPrevented"));

  // --- a blank name is refused here, before any IPC ------------------------
  assert.ok(palette.includes('const name = draft.trim();'));
  assert.equal(occurrences("setDraftError('layers.error.layer.name_empty')"), 2,
    'create and rename both refuse a blank name locally');
  // An unchanged rename is not written either: no round trip, just a close.
  assert.ok(palette.includes('if (name === layer.name) {'));
  assert.ok(palette.indexOf('if (name === layer.name) {') < palette.indexOf('store.renameLayer('),
    'the unchanged-name guard runs before the store call');

  // --- blur cancels, and it is the only blur path -------------------------
  assert.equal(occurrences('onBlur='), 1, 'one blur path, on the editing row');
  assert.ok(palette.includes('if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;'),
    'focus moving inside the row is not a blur out of the edit');
  assert.ok(palette.indexOf('onBlur=') < palette.indexOf('cancelForm();', palette.indexOf('onBlur=')),
    'blur cancels — it never commits');

  // --- focus goes in, and it comes back ------------------------------------
  assert.ok(palette.includes('newFieldRef.current?.focus?.()'), 'the create field takes focus on open');
  assert.ok(palette.includes('renameFieldRef.current?.focus?.()'), 'the rename field takes focus on open');
  assert.ok(palette.includes('restoreFocus.current = () => newTriggerRef.current?.focus?.();'),
    'the header trigger takes focus back');
  assert.ok(palette.includes('if (form === null) {'), 'the restore runs after the closing commit');

  // --- feedback is the palette's own error affordance ----------------------
  assert.ok(palette.includes('className="layers__error" role="alert"'), 'refusals land on the palette line');
  assert.equal(/\b(?:window\.)?(?:alert|confirm|prompt)\s*\(/.test(palette), false, 'no browser dialog, ever');
  // Nothing takes the pointer away from a control in the translucent dock.
  assert.equal(palette.includes('pointer-events'), false, 'the controls stay clickable');
});

test('the panel and the model carry no hardcoded CJK', async () => {
  const sources = [
    await readModule('layers/layers.ts'),
    await readModule('components/LayersPalette.tsx'),
    await readModule('App.tsx'),
    await readModule('components/Workspace.tsx'),
  ];
  for (const source of sources) {
    assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(source), false, 'no hardcoded CJK');
  }
});

test('the palette styles reuse the chrome tokens and never float over the drawing', async () => {
  const css = await readCss();
  const start = css.indexOf('/* Layers palette');
  const end = css.indexOf('/* Navigation rail');
  assert.ok(start > 0 && end > start, 'the layers block exists between the docks');
  const block = css.slice(start, end);
  for (const token of ['var(--hover)', 'var(--field)', 'var(--line-soft)', 'var(--focus)', 'var(--accent)']) {
    assert.ok(block.includes(token), `the layers block uses ${token}`);
  }
  assert.ok(block.includes('.layers__row'));
  assert.ok(block.includes('.layers__eye'));
  assert.ok(block.includes('.layers__search:focus-within'));
  // The inline forms reuse the same tokens and the same field ring, and the
  // reserved actions column is what keeps a row from moving when a form opens.
  assert.ok(block.includes('.layers__field:focus-within'), 'the create field focuses into the shared ring');
  assert.ok(block.includes('.layers__rename:focus-visible'), 'so does the inline rename field');
  assert.ok(block.includes('.layers__actions'), 'the row actions keep their reserved space');
  assert.ok(block.includes('.layers__confirm'), 'the delete confirm stays inside the row');
  assert.ok(block.includes('var(--danger)'), 'the destructive press uses the chrome danger token');
  // The dock sits in a translucent overlay: nothing here may switch the pointer
  // off for a control (the rows rely on the default `auto`).
  assert.equal(block.includes('pointer-events'), false, 'no control is made pointer-transparent');
  assert.equal(block.includes('position: fixed'), false, 'docked, never floating over the CAD surface');
});
