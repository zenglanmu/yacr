// Contract tests for the three classic panels' data: the additive
// `blocks`/`groups`/`images` status blocks, their pure row projections, and the
// chrome-local selectors that open the panels.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import { blocksOf, groupsOf, imagesOf } from '../src/cad/cad-status.ts';
import { blockRows, groupRows, imageRows } from '../src/classic/classic-rows.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { readCatalog } from './helpers.ts';

const base = {
  documentLabel: 'plan.dwg', entityCount: 4, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false,
  snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true,
  backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

const blocks = [
  { name: 'Widget', memberCount: 2, basePoint: [1, 2, 3] },
  { name: null, memberCount: 0, basePoint: null },
];

const groups = [
  {
    name: 'Walls',
    memberCount: 2,
    members: [
      { entityId: '20', instancePath: [] },
      { entityId: '21', instancePath: ['7'] },
    ],
  },
];

const images = [
  {
    id: '20',
    space: 'model',
    spaceId: null,
    file: 'logo.png',
    pixels: [640, 480],
    visible: true,
    clip: { vertexCount: 2, inside: true },
  },
  {
    id: '21',
    space: 'block',
    spaceId: 2,
    file: null,
    pixels: [10, 5],
    visible: false,
    clip: null,
  },
];

const payload = (overrides: Record<string, unknown> = {}) => ({
  ...base, blocks, groups, images, ...overrides,
});

test('the three blocks are parsed and projected without inventing rows', () => {
  const status = parseCadStatus(payload());
  assert.deepEqual(blocksOf(status), blocks);
  assert.deepEqual(groupsOf(status), groups);
  assert.deepEqual(imagesOf(status), images);

  const projected = blockRows(status);
  assert.equal(projected[0].insertable, true);
  assert.equal(projected[1].insertable, false, 'missing metadata is not insertable');
  assert.equal(groupRows(status)[0].members[1].instancePath[0], '7');
  assert.equal(imageRows(status)[1].space, 'block');
});

test('a malformed classic row is rejected, never coerced', () => {
  const rejects = (value: unknown, message: string): void => {
    assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/, message);
  };
  rejects(payload({ blocks: [{ name: 'B', memberCount: -1, basePoint: null }] }), 'negative member count');
  rejects(payload({ blocks: [{ name: 'B', memberCount: 0, basePoint: [0, 0] }] }), 'short base point');
  rejects(payload({
    groups: [{ name: 'G', memberCount: 2, members: [{ entityId: '1', instancePath: [] }] }],
  }), 'member count disagreeing with the members array');
  rejects(payload({
    images: [{ ...images[0], space: 'viewport' }],
  }), 'unknown space key');
  rejects(payload({
    images: [{ ...images[0], clip: { vertexCount: -1, inside: true } }],
  }), 'negative clip vertex count');
  // An older host that omits the additive blocks still parses; the readers are empty.
  const older = parseCadStatus({ ...base });
  assert.deepEqual(blocksOf(older), []);
  assert.deepEqual(groupsOf(older), []);
  assert.deepEqual(imagesOf(older), []);
});

test('the classic words are chrome-local selectors and their keys exist', async () => {
  for (const [word, panel] of [
    ['CLASSICINSERT', 'insert'],
    ['CLASSICGROUP', 'group'],
    ['CLASSICIMAGE', 'image'],
  ] as const) {
    assert.deepEqual(localSelectorFor(word, 'desktop'), { kind: 'classicPanel', panel });
    assert.deepEqual(localSelectorFor(`  ${word.toLowerCase()} `, 'mobile'), { kind: 'classicPanel', panel });
  }

  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const keys = [
    'classicinsert.title', 'classicinsert.subtitle', 'classicinsert.action.insert',
    'classicinsert.refusal.no_metadata', 'classicinsert.refusal.no_document',
    'classicgroup.title', 'classicgroup.subtitle', 'classicgroup.showMembers',
    'classicgroup.refusal.select',
    'classicimage.title', 'classicimage.subtitle', 'classicimage.space.paper',
    'classicimage.clip',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
  }
});
