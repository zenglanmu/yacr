// Contract tests for the optional ABI v1 registration extension:
// `decode` -> record cache -> `properties` panel / `command` interception.
//
// Runs with `node --test` (type stripping) against the plugin source, exactly
// like `vendor-plugin.test.ts`. The fixture is a synthetic base-6 `TCH_WALL`
// so the test stays dependency-free and fast.

import assert from 'node:assert/strict';
import test from 'node:test';

import { decode, register } from '../../../plugins/tch/src/index.ts';
import type {
  PluginCommandDecision,
  PluginCommandRequest,
  PluginPropertyPanel,
  PluginPropertyRow,
  PluginSelection,
  PluginSelectionRow,
  VendorPluginHost,
} from '../../../plugins/tch/src/index.ts';
import { buildRecordsFrame, makeRecord, wallWriter } from './vendor-plugin-fixtures.ts';

const WALL_HANDLE = 0x3e868n;
const WALL_HANDLE_TEXT = '3E868';

interface Captured {
  properties?: (selection: PluginSelection) => PluginPropertyPanel | null;
  command?: (request: PluginCommandRequest) => PluginCommandDecision;
}

function selectionRow(key: string, value: string | null, mixed = false): PluginSelectionRow {
  return {
    key,
    value,
    group: 'identity',
    mixed,
    editable: false,
    kind: 'text',
    options: [],
  };
}

function selection(handle: string | null, options: {
  count?: number;
  empty?: boolean;
  mixed?: boolean;
} = {}): PluginSelection {
  return {
    count: options.count ?? 1,
    empty: options.empty ?? false,
    rows: [
      selectionRow('type', 'TCH_WALL'),
      selectionRow('handle', handle, options.mixed ?? false),
      selectionRow('layer', 'WALLS (1)'),
    ],
  };
}

/** Decode one synthetic wall and register a fake host that captures handlers. */
function setup(): Captured {
  const wall = makeRecord('TCH_WALL', WALL_HANDLE, wallWriter({
    start: [0, 0, 0],
    end: [10, 0, 0],
    left: 1,
    right: 2,
    height: 3,
    joins: [1, 2, 3, 4],
  }));
  decode(buildRecordsFrame(71n, 71n, [wall]));

  const captured: Captured = {};
  const host = {
    on(event: string, handler: unknown): void {
      if (event === 'properties') {
        captured.properties = handler as Captured['properties'];
      } else if (event === 'command') {
        captured.command = handler as Captured['command'];
      }
    },
  } as unknown as VendorPluginHost;
  register(host);
  return captured;
}

function findRow(panel: PluginPropertyPanel, key: string): PluginPropertyRow | undefined {
  for (const group of panel.groups) {
    const found = group.rows.find(candidate => candidate.key === key);
    if (found) return found;
  }
  return undefined;
}

function groupKeys(panel: PluginPropertyPanel): string[] {
  return panel.groups.map(group => group.key);
}

test('a decoded TCH_WALL yields identity and geometry rows', () => {
  const captured = setup();
  assert.ok(captured.properties, 'register must install a properties handler');

  const panel = captured.properties(selection(WALL_HANDLE_TEXT));
  assert.ok(panel);
  assert.equal(panel.kind, 'TCH_WALL');
  assert.equal(panel.handle, WALL_HANDLE_TEXT);
  assert.deepEqual(groupKeys(panel), ['identity', 'geometry']);

  assert.equal(findRow(panel, 'identity.class')?.value, 'TCH_WALL');
  assert.equal(findRow(panel, 'identity.handle')?.value, WALL_HANDLE_TEXT);
  assert.equal(findRow(panel, 'identity.layer')?.value, 'WALLS (1)');
  assert.equal(findRow(panel, 'identity.source')?.value, 'unknown-entity');
  assert.equal(findRow(panel, 'identity.dwg_version')?.value, '1027');
  assert.ok(Number(findRow(panel, 'identity.record_bytes')?.value) > 0);
  assert.equal(findRow(panel, 'identity.references')?.value, '0');

  assert.equal(findRow(panel, 'wall.start')?.value, '0, 0, 0');
  assert.equal(findRow(panel, 'wall.end')?.value, '10, 0, 0');
  assert.equal(findRow(panel, 'wall.thickness_left')?.value, '1');
  assert.equal(findRow(panel, 'wall.thickness_right')?.value, '2');
  assert.equal(findRow(panel, 'wall.height')?.value, '3');
  assert.equal(findRow(panel, 'wall.length')?.value, '10');
  assert.equal(findRow(panel, 'wall.openings')?.value, '0');

  for (const key of ['wall.height', 'wall.thickness_left', 'wall.thickness_right', 'wall.length']) {
    const value = findRow(panel, key)?.value;
    assert.ok(value !== undefined, `${key} row must exist`);
    assert.ok(Number.isFinite(Number(value)), `${key} must be finite, got ${value}`);
  }
});

test('unknown handles and multi-selection pass through', () => {
  const captured = setup();
  assert.ok(captured.properties);

  assert.equal(captured.properties(selection('FFFF')), null, 'unknown handle is not handled');
  assert.equal(captured.properties(selection('none')), null, 'non-hex handle is not handled');
  assert.equal(
    captured.properties(selection(WALL_HANDLE_TEXT, { count: 2 })),
    null,
    'multi-selection is pass-through',
  );
  assert.equal(
    captured.properties(selection(WALL_HANDLE_TEXT, { mixed: true })),
    null,
    'mixed handle row is pass-through',
  );
});

test('edit commands on a TCH record are refused, everything else passes through', () => {
  const captured = setup();
  assert.ok(captured.command);

  assert.deepEqual(
    captured.command({ command: 'MOVE', selection: selection(WALL_HANDLE_TEXT) }),
    { handled: true, messageKey: 'vendor.editing.unsupported' },
  );

  assert.equal(
    captured.command({ command: 'MOVE', selection: selection('FFFF') }),
    null,
    'unknown handle keeps the normal command path',
  );
  assert.equal(
    captured.command({ command: 'LINE', selection: selection(WALL_HANDLE_TEXT) }),
    null,
    'draw commands are never intercepted',
  );
  assert.equal(
    captured.command({ command: 'MOVE', selection: selection(WALL_HANDLE_TEXT, { count: 2 }) }),
    null,
    'multi-selection is pass-through',
  );
});
