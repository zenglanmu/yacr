// Contract tests for the expanded command history: the typed record's order, the
// host echo's output attached to the command it belongs to, and the honest
// non-answers (no echo yet, unmatched echo, refusal row).
//
// These are plain-JavaScript bodies in a `.ts` file, like the rest of this suite:
// Node strips the types, `tsc` never includes `test/`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { commandWindowRows, sameCommandLine } from '../src/commands/command-window.ts';
import { setCommandAliases, setCommandCatalog } from '../src/commands/commands.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog } from './helpers.ts';

setCommandCatalog(['LINE', 'CIRCLE', 'ZOOM', 'MEASURE', 'AREA', 'DIST']);
setCommandAliases([{ alias: 'L', command: 'LINE' }]);

const en = await readCatalog('en');
const zh = await readCatalog('zh-CN');
const t = createTranslator({ en, 'zh-CN': zh }, 'en');

// Host wire shapes (`cad_status.commandLog`), oldest first. The result fixture is
// a polyline length so it renders exactly one value line.
function command(sequence, name) {
  return { sequence, kind: 'command', command: name, code: null, measurement: null };
}

function result(sequence, value) {
  return {
    sequence,
    kind: 'result',
    command: null,
    code: null,
    measurement: { algorithm: 'polyline', value, unit: null, precision: 'analytic', points: [] },
  };
}

function error(sequence, code) {
  return { sequence, kind: 'error', command: null, code, measurement: null };
}

function entry(kind, text) {
  return { kind, text };
}

test('one command carries its own echo and stays recallable', () => {
  const rows = commandWindowRows([entry('command', 'LINE')], [command(1, 'LINE'), result(2, 5)], t);
  assert.equal(rows.length, 1);
  assert.deepEqual(rows[0], {
    key: '0:command:LINE',
    kind: 'command',
    text: 'LINE',
    recall: 'LINE',
    code: null,
    lines: [{
      tone: 'value',
      label: en['command.log.length'],
      text: '5.0000',
      unit: en['units.unknown'],
      meta: null,
    }],
  });
});

test('output stops at the next host command and never leaks forward', () => {
  const rows = commandWindowRows(
    [entry('command', 'LINE'), entry('command', 'CIRCLE')],
    [command(1, 'LINE'), result(2, 5), command(3, 'CIRCLE'), result(4, 7)],
    t,
  );
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0].lines.map(line => line.text), ['5.0000']);
  assert.deepEqual(rows[1].lines.map(line => line.text), ['7.0000']);
});

test('a host error attaches as an error-tone line and surfaces its code', () => {
  const rows = commandWindowRows(
    [entry('command', 'LINE')],
    [command(1, 'LINE'), error(2, 'cancelled')],
    t,
  );
  assert.equal(rows[0].lines.length, 1);
  assert.equal(rows[0].lines[0].tone, 'error');
  assert.equal(rows[0].lines[0].text, en['command.log.error.cancelled']);
  assert.equal(rows[0].code, 'cancelled');
});

test('alias and argument forms name the same selector, a different name does not', () => {
  assert.equal(sameCommandLine('LINE', 'LINE'), true);
  assert.equal(sameCommandLine('line', 'LINE'), true);
  assert.equal(sameCommandLine('L', 'LINE'), true);
  assert.equal(sameCommandLine('LINE', 'L'), true, 'an echoed alias resolves to the typed canonical name');
  assert.equal(sameCommandLine('zoom 2', 'ZOOM'), true);
  assert.equal(sameCommandLine('MEASURE AREA', 'MEASURE AREA'), true);
  assert.equal(sameCommandLine('AREA OBJECT', 'AREA OBJECT'), true);
  assert.equal(sameCommandLine('LINE', 'CIRCLE'), false);
  assert.equal(sameCommandLine('', 'LINE'), false);
  assert.equal(sameCommandLine('LINE', ''), false);
  for (const value of [null, undefined, 7, {}, ['LINE']]) {
    assert.equal(sameCommandLine(value, 'LINE'), false);
    assert.equal(sameCommandLine('LINE', value), false);
  }

  const alias = commandWindowRows([entry('command', 'L')], [command(1, 'LINE'), result(2, 3)], t);
  assert.deepEqual(alias[0].lines.map(line => line.text), ['3.0000']);
  const withArgument = commandWindowRows(
    [entry('command', 'ZOOM 2')],
    [command(1, 'ZOOM'), result(2, 4)],
    t,
  );
  assert.deepEqual(withArgument[0].lines.map(line => line.text), ['4.0000']);
  const other = commandWindowRows([entry('command', 'CIRCLE')], [command(1, 'LINE'), result(2, 5)], t);
  assert.deepEqual(other[0].lines, []);
});

test('a refusal row is copy only and does not consume a host echo', () => {
  const rows = commandWindowRows(
    [
      entry('command', 'LINE'),
      entry('error', 'No drawing is open.'),
      entry('command', 'CIRCLE'),
    ],
    [command(1, 'LINE'), result(2, 5), command(3, 'CIRCLE'), result(4, 7)],
    t,
  );
  assert.equal(rows.length, 3);
  assert.deepEqual(rows[1], {
    key: '1:error:No drawing is open.',
    kind: 'error',
    text: 'No drawing is open.',
    recall: null,
    code: null,
    lines: [],
  });
  // The refusal sits between the two commands and changes nothing: CIRCLE still
  // gets its own segment.
  assert.deepEqual(rows[2].lines.map(line => line.text), ['7.0000']);
});

test('a host-only echo is never invented as its own row or attached elsewhere', () => {
  const rows = commandWindowRows(
    [entry('command', 'LINE')],
    [command(1, 'DIST'), result(2, 9), command(3, 'LINE'), result(4, 5)],
    t,
  );
  assert.equal(rows.length, 1, 'only the typed record makes rows');
  assert.deepEqual(rows[0].lines.map(line => line.text), ['5.0000']);
  assert.equal(rows[0].key, '0:command:LINE');
});

test('two identical commands consume two distinct echo segments', () => {
  const rows = commandWindowRows(
    [entry('command', 'LINE'), entry('command', 'LINE')],
    [command(1, 'LINE'), result(2, 5), command(3, 'LINE'), result(4, 7)],
    t,
  );
  assert.equal(rows.length, 2);
  assert.notEqual(rows[0].key, rows[1].key);
  assert.deepEqual(rows[0].lines.map(line => line.text), ['5.0000']);
  assert.deepEqual(rows[1].lines.map(line => line.text), ['7.0000']);
});

test('empty entries or a non-array log produce nothing to render', () => {
  assert.deepEqual(commandWindowRows([], [], t), []);
  assert.deepEqual(commandWindowRows([], null, t), []);
  assert.deepEqual(commandWindowRows([], 'not-a-log', t), []);
  assert.deepEqual(commandWindowRows(undefined, null, t), []);
  // With a typed row but no usable log the row still exists, simply without
  // output — the echo is missing, not the submitted line.
  const rows = commandWindowRows([entry('command', 'LINE')], { length: 0 }, t);
  assert.equal(rows.length, 1);
  assert.deepEqual(rows[0].lines, []);
  assert.equal(rows[0].recall, 'LINE');
});

test('a late echo starts attaching output on the next stateless run', () => {
  const entries = [entry('command', 'LINE'), entry('command', 'CIRCLE')];
  const before = commandWindowRows(entries, [], t);
  assert.deepEqual(before.map(row => row.lines), [[], []]);
  // Re-running the same inputs repeats the same answer: nothing is remembered.
  assert.deepEqual(commandWindowRows(entries, [], t), before);

  const after = commandWindowRows(entries, [command(1, 'LINE'), result(2, 5), command(3, 'CIRCLE'), result(4, 7)], t);
  assert.deepEqual(after[0].lines.map(line => line.text), ['5.0000']);
  assert.deepEqual(after[1].lines.map(line => line.text), ['7.0000']);
  // Order is the typed record's, never the echo's.
  assert.deepEqual(after.map(row => row.text), ['LINE', 'CIRCLE']);
});
