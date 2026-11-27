// The Outliner tray wire: the host's row shape is kept verbatim, malformed rows
// are dropped instead of invented, and the collapse projection hides exactly a
// collapsed row's subtree. Selection is a separate dispatch (`cad_select_entities`).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  OUTLINER_COMMAND,
  groupRenameCommand,
  outlinerRowKey,
  parseOutlinerAnswer,
  readOutliner,
  visibleOutlinerRows,
} from '../src/domains/concept/outliner.ts';

const ROWS = [
  { kind: 'group', id: 1, label: 'walls', depth: 0, hasChildren: true, selectable: true, selectIds: [10, 11] },
  { kind: 'entity', id: 10, label: 'AcDbLine #10', depth: 1, hasChildren: false, selectable: true, selectIds: [10] },
  { kind: 'entity', id: 11, label: 'AcDbLine #11', depth: 1, hasChildren: false, selectable: true, selectIds: [11] },
  { kind: 'insert', id: 4, label: 'Widget #4', depth: 0, hasChildren: true, selectable: true, selectIds: [4] },
  { kind: 'member', id: 5, label: 'AcDbCircle #5', depth: 1, hasChildren: false, selectable: false, selectIds: [] },
];

test('a full answer keeps the host rows, order and select ids', () => {
  const answer = parseOutlinerAnswer({ revision: 7, rows: ROWS, total: 5, truncated: false });
  assert.equal(answer.revision, 7);
  assert.equal(answer.total, 5);
  assert.equal(answer.truncated, false);
  assert.deepEqual(answer.rows.map(row => row.label), ROWS.map(row => row.label));
  assert.deepEqual(answer.rows[0].selectIds, [10, 11]);
});

test('a malformed row is dropped, never invented', () => {
  const answer = parseOutlinerAnswer({
    rows: [
      null,
      { kind: '', id: 1, label: 'x' },
      { kind: 'entity', id: 1.5, label: 'x' },
      { kind: 'entity', id: 2, label: null },
      ROWS[1],
    ],
    total: 9,
  });
  assert.deepEqual(answer.rows.map(row => row.id), [10]);
  assert.equal(answer.total, 9, 'the host total is not recomputed');
});

test('a member row without select ids is not selectable', () => {
  const answer = parseOutlinerAnswer({ rows: [
    { kind: 'member', id: 5, label: 'm', depth: 1, hasChildren: false, selectable: true, selectIds: [] },
  ] });
  assert.equal(answer.rows[0].selectable, false);
});

test('a non-object answer is a named transport fault', () => {
  assert.throws(() => parseOutlinerAnswer(null), /outliner\.invalid_answer/);
});

test('collapsing a container hides exactly its contiguous subtree', () => {
  const rows = parseOutlinerAnswer({ rows: ROWS }).rows;
  assert.equal(visibleOutlinerRows(rows, new Set()).length, 5);
  const groupKey = outlinerRowKey({ kind: 'group', id: 1 });
  const collapsedGroup = visibleOutlinerRows(rows, new Set([groupKey]));
  assert.deepEqual(collapsedGroup.map(row => row.id), [1, 4, 5]);

  const insertKey = outlinerRowKey({ kind: 'insert', id: 4 });
  const collapsedInsert = visibleOutlinerRows(rows, new Set([insertKey]));
  assert.deepEqual(collapsedInsert.map(row => row.id), [1, 10, 11, 4]);

  const both = visibleOutlinerRows(rows, new Set([groupKey, insertKey]));
  assert.deepEqual(both.map(row => row.id), [1, 4]);
});

test('the read goes through the one frozen command', async () => {
  const seen: string[] = [];
  const answer = await readOutliner(async (command: string) => {
    seen.push(command);
    return { revision: 1, rows: ROWS, total: 5, truncated: false };
  });
  assert.deepEqual(seen, [OUTLINER_COMMAND]);
  assert.equal(answer.rows.length, 5);
});

test('a group rename uses the quoted GROUPEDIT grammar', () => {
  assert.equal(
    groupRenameCommand('walls', 'Walls 2'),
    'GROUPEDIT "walls" RENAME "Walls 2"',
  );
  assert.equal(
    groupRenameCommand('  walls  ', '  walls  '),
    'GROUPEDIT "walls" RENAME "walls"',
  );
});

test('a group name with a quote or line break cannot be addressed', () => {
  for (const value of ['a"b', 'a\nb', 'a\rb', '   ', null, 3]) {
    assert.equal(groupRenameCommand('walls', value), null, String(value));
    assert.equal(groupRenameCommand(value, 'walls'), null, String(value));
  }
});

test('the host hidden flag is preserved, never inferred', () => {
  const answer = parseOutlinerAnswer({
    rows: [
      { kind: 'entity', id: 1, label: 'a', hidden: true, selectIds: [1] },
      { kind: 'entity', id: 2, label: 'b', selectIds: [2] },
    ],
  });
  assert.equal(answer.rows[0].hidden, true);
  assert.equal(answer.rows[1].hidden, false);
});
