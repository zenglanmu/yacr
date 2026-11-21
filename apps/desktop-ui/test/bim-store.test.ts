// BIM browse panel: the store's state machine.
//
// The fake clock and fake `invoke` keep these tests free of wall time and IPC:
// they pin the exact payloads the two commands receive, the 150 ms debounce, the
// generation-based supersession, and the states the panel must not confuse
// (unsupported / waiting / empty / filtered-empty / stale).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BIM_QUERY_COMMAND,
  BIM_SUMMARY_COMMAND,
  BIM_PAYLOAD_INVALID,
  emptyBimFilter,
} from '../src/bim/bim-query.ts';
import { createBimStore } from '../src/bim/bim-store.ts';
import { BIM_SELECT_COMMAND } from '../src/bim/selection.ts';

const EMPTY_WIRE_FILTER = {
  categories: [],
  levels: [],
  types: [],
  materials: [],
  property: null,
  evidence: null,
};

/** One wire row. */
function wireRow(overrides = {}) {
  return {
    object: { kind: 'source', connector: 'ifc', documentId: 'doc', id: 'g1' },
    entityIds: [7],
    category: 'IfcWall',
    name: 'Basic Wall',
    levelName: 'Level 1',
    typeName: 'Basic Wall',
    materialName: 'Concrete',
    source: { connector: 'ifc', documentId: 'doc', id: 'g1', nativeId: '22' },
    evidence: 'declared',
    propertySummary: [{ name: 'Pset_WallCommon', count: 3 }],
    ...overrides,
  };
}

/** One query page. */
function page(overrides = {}) {
  return {
    databaseId: 1,
    revision: 1,
    total: 1,
    offset: 0,
    limit: 100,
    stale: false,
    rows: [wireRow()],
    ...overrides,
  };
}

/** One summary answer. */
function summary(overrides = {}) {
  return {
    databaseId: 1,
    revision: 1,
    total: 12,
    summary: {
      total: 12,
      byCategory: [{ key: 'ifcwall', name: 'IfcWall', count: 12 }],
      byLevel: [],
      byType: [],
      byMaterial: [],
    },
    ...overrides,
  };
}

/** A recorded call plus a handler that answers it. */
function fakeInvoke(handler) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return handler(command, args, calls);
  };
  return { calls, invoke };
}

/** A manual clock: `schedule` records, `flush` runs one pending callback. */
function fakeClock() {
  const queue = [];
  return {
    schedule(run, ms) {
      const entry = { run, ms, cancelled: false };
      queue.push(entry);
      return entry;
    },
    cancel(handle) {
      handle.cancelled = true;
    },
    pending() {
      return queue.filter(entry => !entry.cancelled);
    },
    async flush() {
      // Skip timers a later request cancelled, so `flush` runs the newest one.
      for (;;) {
        const entry = queue.shift();
        if (!entry) return;
        if (entry.cancelled) continue;
        await entry.run();
        return;
      }
    },
  };
}

/** Let the store's promise chain settle (the fake transport resolves on a microtask). */
function pause() {
  return new Promise(resolve => setTimeout(resolve, 0));
}

/** A promise whose settlement the test controls. */
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

/** The last call for one command. */
function lastCall(calls, command) {
  return [...calls].reverse().find(call => call.command === command) ?? null;
}

test('the first load asks the summary without the filter and the page with it', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : page()));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  assert.equal(calls[0].command, BIM_SUMMARY_COMMAND);
  assert.deepEqual(calls[0].args.filter, EMPTY_WIRE_FILTER);
  // A summary is one logical row on the host; it ignores these, but sending the
  // documented pair keeps the request textual and honest.
  assert.deepEqual([calls[0].args.offset, calls[0].args.limit], [0, 1]);
  assert.equal(calls[1].command, BIM_QUERY_COMMAND);
  assert.deepEqual(calls[1].args.filter, EMPTY_WIRE_FILTER);
  assert.deepEqual([calls[1].args.offset, calls[1].args.limit], [0, 100]);

  const state = store.getState();
  assert.equal(state.phase, 'ready');
  assert.equal(state.busy, false);
  assert.equal(state.summary?.total, 12);
  assert.equal(state.rows.length, 1);
  assert.deepEqual(state.page, { total: 1, offset: 0, limit: 100, stale: false });
});

test('a filter press updates the chips immediately and sends one debounced query', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : page({ total: 12 })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  // The chip is on at once; the request waits for the pause.
  assert.deepEqual(store.getState().filter.categories, ['ifcwall']);
  // The page still belongs to the empty filter, so its count is not relabelled.
  assert.deepEqual(store.getState().queriedFilter, EMPTY_WIRE_FILTER);
  assert.equal(calls.filter(call => call.command === BIM_QUERY_COMMAND).length, 1);
  assert.equal(clock.pending().length, 1);

  await clock.flush();
  const query = lastCall(calls, BIM_QUERY_COMMAND);
  assert.deepEqual(query.args.filter.categories, ['ifcwall']);
  assert.equal(query.args.offset, 0);
  assert.deepEqual(store.getState().queriedFilter?.categories, ['ifcwall']);
  assert.equal(store.getState().busy, false);
});

test('a second filter within the debounce window replaces the first request', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : page({ total: 12 })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  const first = clock.pending()[0];
  store.setFilter({ ...emptyBimFilter(), categories: ['ifcslab'] });
  assert.equal(first.cancelled, true);
  assert.equal(clock.pending().length, 1);

  await clock.flush();
  const queries = calls.filter(call => call.command === BIM_QUERY_COMMAND);
  assert.equal(queries.length, 2);
  assert.deepEqual(queries[1].args.filter.categories, ['ifcslab']);
});

test('an older in-flight answer never overwrites a newer filter', async () => {
  const clock = fakeClock();
  const pending = [];
  const invoke = command => {
    if (command === BIM_SUMMARY_COMMAND) return Promise.resolve(summary());
    const reply = deferred();
    pending.push(reply);
    return reply.promise;
  };
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });

  const loading = store.load();
  await Promise.resolve();
  pending[0].resolve(page({ total: 12, rows: [wireRow({ name: 'first' })] }));
  await loading;

  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  const oldRun = clock.flush(); // request A
  await Promise.resolve();
  store.setFilter({ ...emptyBimFilter(), categories: ['ifcslab'] });
  const newRun = clock.flush(); // request B
  await Promise.resolve();

  assert.equal(pending.length, 3);
  pending[2].resolve(page({ total: 5, rows: [wireRow({ name: 'newest' })] }));
  await newRun;
  assert.equal(store.getState().rows[0].name, 'newest');

  pending[1].resolve(page({ total: 1, rows: [wireRow({ name: 'stale reply' })] }));
  await oldRun;
  assert.equal(store.getState().rows[0].name, 'newest');
  assert.equal(store.getState().summary?.total, 12);
});

test('unsupported-runtime: bimQuery is its own state, never an empty table', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(() => { throw new Error('unsupported-runtime: bimQuery'); });
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  const state = store.getState();
  assert.equal(state.phase, 'unsupported');
  assert.deepEqual(state.rows, []);
  assert.equal(state.summary, null);
  assert.equal(state.page, null);
  assert.equal(state.error?.reason, 'unsupported-runtime: bimQuery');
});

test('cad_waiting is a loading state, not an empty drawing', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(() => { throw new Error('cad_waiting'); });
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  assert.equal(store.getState().phase, 'waiting');
  assert.deepEqual(store.getState().rows, []);
  assert.equal(store.getState().error, null);
});

test('a real empty answer is "no BIM data"; a filtered page of zero is "no match"', async () => {
  const clock = fakeClock();
  let summaryTotal = 0;
  const { invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND
    ? summary({ total: summaryTotal, summary: { ...summary().summary, total: summaryTotal, byCategory: [] } })
    : page({ total: 0, rows: [] })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  assert.equal(store.getState().phase, 'empty');

  summaryTotal = 12;
  await store.refresh();
  assert.equal(store.getState().phase, 'empty');
  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  await clock.flush();
  assert.equal(store.getState().phase, 'filtered-empty');
});

test('the host stale flag is kept on the page', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 12, stale: true })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  assert.equal(store.getState().page?.stale, true);
});

test('a bim.stale refusal keeps the last page and turns on the stale banner', async () => {
  const clock = fakeClock();
  let stale = false;
  const { invoke } = fakeInvoke(command => {
    if (command === BIM_SUMMARY_COMMAND) return summary();
    if (stale) throw new Error('bim.stale: the query document is not the open document');
    return page({ total: 250 });
  });
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  const before = store.getState().rows;

  stale = true;
  store.setPage(100);
  await pause();
  const state = store.getState();
  assert.equal(state.rows, before);
  assert.equal(state.page?.stale, true);
  assert.equal(state.error, null);
  assert.equal(state.phase, 'ready');
});

test('paging sends the next offset immediately and keeps the filter', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke((command, args) => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 250, offset: args.offset })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  store.setPage(100);
  await pause();
  const query = lastCall(calls, BIM_QUERY_COMMAND);
  assert.equal(query.args.offset, 100);
  assert.equal(query.args.limit, 100);
  assert.equal(store.getState().page?.offset, 100);
  assert.equal(clock.pending().length, 0);
});

test('a refresh re-reads the summary without the filter and the page with it', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 12 })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  await clock.flush();
  await store.refresh();

  const summaryCalls = calls.filter(call => call.command === BIM_SUMMARY_COMMAND);
  assert.deepEqual(summaryCalls[1].args.filter, EMPTY_WIRE_FILTER);
  const query = lastCall(calls, BIM_QUERY_COMMAND);
  assert.deepEqual(query.args.filter.categories, ['ifcwall']);
});

test('a refresh during a pending filter change starts at page 1, not the old offset', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke((command, args) => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 250, offset: args.offset })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  store.setPage(100);
  await pause();
  assert.equal(lastCall(calls, BIM_QUERY_COMMAND).args.offset, 100);

  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  await store.refresh();
  const query = lastCall(calls, BIM_QUERY_COMMAND);
  assert.equal(query.args.offset, 0);
  assert.deepEqual(query.args.filter.categories, ['ifcwall']);
  assert.equal(clock.pending().length, 0);
});

test('a refresh keeps the page when the filter has not changed', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke((command, args) => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 250, offset: args.offset })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  store.setPage(200);
  await pause();
  await store.refresh();
  assert.equal(lastCall(calls, BIM_QUERY_COMMAND).args.offset, 200);
});

test('a different document resets the panel instead of reusing the old rows', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : page({ total: 12 })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  store.setFilter({ ...emptyBimFilter(), categories: ['ifcwall'] });
  await clock.flush();

  store.reset();
  const state = store.getState();
  assert.equal(state.phase, 'idle');
  assert.deepEqual(state.rows, []);
  assert.equal(state.summary, null);
  assert.equal(state.page, null);
  assert.equal(state.queriedFilter, null);
  assert.deepEqual(state.filter, EMPTY_WIRE_FILTER);
  assert.equal(state.busy, false);

  // The panel can read the new document from a clean slate.
  await store.load();
  assert.equal(store.getState().phase, 'ready');
});

test('clear-filter drops every group and queries at once', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND
    ? summary()
    : page({ total: 12 })));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  store.setFilter({
    ...emptyBimFilter(),
    categories: ['ifcwall'],
    levels: ['level-1'],
    property: { pset: 'P', property: 'Q', op: 'eq', value: { kind: 'integer', value: 1 } },
  });
  await clock.flush();

  store.clearFilter();
  assert.deepEqual(store.getState().filter, EMPTY_WIRE_FILTER);
  assert.equal(calls.filter(call => call.command === BIM_QUERY_COMMAND).length, 3);
  assert.deepEqual(lastCall(calls, BIM_QUERY_COMMAND).args.filter, EMPTY_WIRE_FILTER);
});

test('an unreadable page is an error with the stable payload marker', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : {}));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  assert.equal(store.getState().phase, 'error');
  assert.equal(store.getState().error?.reason, BIM_PAYLOAD_INVALID);
});

test('selection sends every geometry id and never claims a local selection', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => {
    if (command === BIM_SUMMARY_COMMAND) return summary();
    if (command === BIM_QUERY_COMMAND) return page();
    return { accepted: true };
  });
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  const result = await store.select([7, 8, 8]);
  assert.deepEqual(result, { kind: 'ok', count: 2 });
  const call = lastCall(calls, BIM_SELECT_COMMAND);
  assert.deepEqual(call.args, { entityIds: [7, 8] });
  assert.equal(store.getState().selectionError, null);
});

test('a refused selection is reported in the host words and can be dismissed', async () => {
  const clock = fakeClock();
  const { invoke } = fakeInvoke(command => {
    if (command === BIM_SUMMARY_COMMAND) return summary();
    if (command === BIM_QUERY_COMMAND) return page();
    return { accepted: false, error: 'selection.no_document' };
  });
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();

  const result = await store.select([7]);
  assert.deepEqual(result, { kind: 'error', reason: 'selection.no_document' });
  assert.equal(store.getState().selectionError, 'selection.no_document');
  store.clearSelectionError();
  assert.equal(store.getState().selectionError, null);
});

test('a selection request with no ids sends nothing', async () => {
  const clock = fakeClock();
  const { calls, invoke } = fakeInvoke(command => (command === BIM_SUMMARY_COMMAND ? summary() : page()));
  const store = createBimStore(invoke, { schedule: clock.schedule, cancel: clock.cancel });
  await store.load();
  const before = calls.length;
  assert.deepEqual(await store.select([]), { kind: 'invalid' });
  assert.equal(calls.length, before);
});
