// Contract tests for the additive `cad_status.performance` block: the strict
// validation of a present block, and the explicit `null` of a host that
// publishes no metrics.
//
// These are wire-model tests, not screenshots. The parser is the same one the
// chrome runs on every poll, and an absent/unmeasured metric must never become a
// guessed number.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import type { CadStatus } from '../src/cad/cad-status.ts';

/** The required fields `parseCadStatus` enforces (same shape as layout-tabs.test.ts). */
const base = {
  documentLabel: null, entityCount: null, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false,
  snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false,
  backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

/** A fully measured performance block (every key present, every value real). */
const performance = {
  cpuPercent: 12.5,
  processRssBytes: 104_857_600,
  processVirtualBytes: 209_715_200,
  systemTotalBytes: 17_179_869_184,
  systemAvailableBytes: 8_589_934_592,
  gpuMemoryUsedBytes: 4_194_304,
  gpuMemoryPeakBytes: 8_388_608,
  gpuMemoryReservedBytes: 16_777_216,
  vramTotalBytes: 2_147_483_648,
};

const status = (overrides: Record<string, unknown> = {}) => ({ ...base, ...overrides });

const rejects = (value: unknown, message: string): void => {
  assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/, message);
};

test('a minimal valid status with performance: null parses', () => {
  const payload = status({ performance: null });
  assert.equal(parseCadStatus(payload), payload);
  assert.equal((parseCadStatus(payload) as CadStatus).performance, null);
});

test('a status that omits the performance block still parses (older host)', () => {
  const payload = status();
  assert.equal(parseCadStatus(payload), payload);
});

test('a fully populated performance block parses and keeps every value', () => {
  const payload = status({ performance });
  const parsed = parseCadStatus(payload) as CadStatus;
  assert.deepEqual(parsed.performance, performance);
});

test('a block whose metrics are all null parses (explicitly unmeasured)', () => {
  const measured = Object.fromEntries(
    Object.keys(performance).map((key) => [key, null]),
  );
  const payload = status({ performance: measured });
  assert.equal(parseCadStatus(payload), payload);
});

test('a non-object performance block is rejected', () => {
  for (const bad of [42, 'metrics', [], true]) {
    rejects(status({ performance: bad }), `non-object block: ${String(bad)}`);
  }
});

test('a missing required performance key is rejected', () => {
  const { vramTotalBytes: _omitted, ...missing } = performance;
  rejects(status({ performance: missing }), 'missing vramTotalBytes');
});

test('a NaN, negative or non-numeric metric is rejected', () => {
  rejects(
    status({ performance: { ...performance, cpuPercent: Number.NaN } }),
    'NaN cpuPercent',
  );
  rejects(
    status({ performance: { ...performance, gpuMemoryUsedBytes: -1 } }),
    'negative gpuMemoryUsedBytes',
  );
  rejects(
    status({ performance: { ...performance, systemTotalBytes: '1024' } }),
    'string systemTotalBytes',
  );
});
