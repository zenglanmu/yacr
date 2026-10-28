// AutoCAD's `ZOOM <argument>` on the command line: admission, submission and
// history.
//
// The chrome admits by **base name** and sends the full normalized line, so the
// value (factor, `ALL`, `EXTENTS`) reaches `cad_command` untouched and the host
// stays the authority for its semantics. A value outside the local syntax
// predicate is an explicit `unsupported` answer with the shared reason key —
// never a clamp, never a fabricated `ZOOM 1`.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  defaultSuggestionIndex,
  filterCommandSuggestions,
  setCommandAliases,
  setCommandCatalog,
  splitCommandText,
} from '../src/commands/commands.ts';
import { appendCommand, commandOnly } from '../src/commands/command-history.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);
// The host's real answer for the one spelling this feature adds: `ZOOM EXTENTS`
// is a pre-existing alias onto `FIT` (the `cad_command_aliases` IPC).
setCommandAliases([{ alias: 'ZOOM EXTENTS', command: 'FIT' }]);

// The frozen `cad_status` shape: a drawing is open and FIT is available, so the
// only reason an admission could fail here is the command/argument itself.
const loaded = {
  documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true,
  gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true,
};

const UNSUPPORTED_KEY = 'command.unsupported';

test('a typed argument is split off the base name, with one space/tab or many', () => {
  assert.deepEqual(splitCommandText('ZOOM 2'), { name: 'ZOOM', argument: '2', text: 'ZOOM 2' });
  assert.deepEqual(splitCommandText('zoom   2'), { name: 'ZOOM', argument: '2', text: 'ZOOM   2' });
  assert.deepEqual(splitCommandText('  ZOOM 2  '), { name: 'ZOOM', argument: '2', text: 'ZOOM 2' });
  assert.deepEqual(splitCommandText('ZOOM\t0.5'), { name: 'ZOOM', argument: '0.5', text: 'ZOOM\t0.5' });
  // The argument keeps its own internal text: `2 3` is one argument, refused by
  // the predicate rather than split a second time.
  assert.deepEqual(splitCommandText('ZOOM 2 3'), { name: 'ZOOM', argument: '2 3', text: 'ZOOM 2 3' });
  // A name-only line has no argument; an empty line has neither.
  assert.deepEqual(splitCommandText(' ZOOM '), { name: 'ZOOM', argument: null, text: 'ZOOM' });
  assert.deepEqual(splitCommandText('   '), { name: '', argument: null, text: '' });
  assert.deepEqual(splitCommandText(null), { name: '', argument: null, text: '' });
});

test('the local grammar mirrors the host: a positive decimal, ALL or EXTENTS', () => {
  const { parse } = ARGUMENT_COMMANDS.ZOOM;
  for (const admitted of ['2', '0.5', '1.25', '2.0', 'ALL', 'EXTENTS']) {
    assert.equal(parse(admitted), true, admitted);
  }
  for (const refused of ['0', '0.0', '-1', '+2', 'abc', '2 3', '.5', '2.', '1e3', 'NaN', '']) {
    assert.equal(parse(refused), false, refused);
  }
  // The table is explicit: `ZOOM`, `SNAPUNIT` and the two POINT display sysvars
  // are the argument-taking selectors, and each has its own grammar (`SNAPUNIT`
  // is a positive decimal spacing only — `ALL`/`EXTENTS` mean nothing there; the
  // sysvars take one plain decimal, zero/negative included, and are covered in
  // `point-display-sysvar.test.ts`).
  assert.deepEqual(Object.keys(ARGUMENT_COMMANDS), ['ZOOM', 'SNAPUNIT', 'PDMODE', 'PDSIZE']);
  const snapUnit = ARGUMENT_COMMANDS.SNAPUNIT.parse;
  for (const admitted of ['25', '0.5', '2.5', '10.0']) {
    assert.equal(snapUnit(admitted), true, admitted);
  }
  for (const refused of ['0', '0.0', '-5', '+5', 'abc', '2 3', '.5', '5.', '1e3', 'ALL', 'EXTENTS', 'NaN', '']) {
    assert.equal(snapUnit(refused), false, refused);
  }
});

test('ZOOM with a factor / ALL / EXTENTS is admitted and keeps the full line', () => {
  for (const typed of ['ZOOM 2', 'ZOOM 0.5', 'ZOOM ALL', 'ZOOM EXTENTS', 'zoom   2', '  ZOOM 2  ']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.reasonKey, null, typed);
    // The line, not just the name: the host validates the value itself.
    assert.equal(admission.command, splitCommandText(typed).text, typed);
  }
  // Trim and case normalization are the existing command-line normalization, so
  // `zoom   2` keeps its inner spacing but loses the surrounding whitespace.
  assert.equal(admitCommandText('  ZOOM 2  ', { status: loaded }).command, 'ZOOM 2');
  assert.equal(admitCommandText('zoom   2', { status: loaded }).command, 'ZOOM   2');
});

test('an out-of-grammar or name-only-argument line is explicitly unsupported', () => {
  for (const typed of ['ZOOM 0', 'ZOOM -1', 'ZOOM abc', 'ZOOM 2 3', 'LINE 2']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'unsupported', typed);
    assert.equal(admission.reasonKey, UNSUPPORTED_KEY, typed);
    // Nothing is clamped into a different command line.
    assert.equal(admission.command, splitCommandText(typed).text, typed);
  }
});

test('bare ZOOM stays the payload-refusal path, never a fabricated factor', async () => {
  const admission = admitCommandText('ZOOM', { status: loaded });
  assert.equal(admission.kind, 'admitted');
  assert.equal(admission.command, 'ZOOM', 'the name is not rewritten into a factor');
  assert.equal(splitCommandText('ZOOM').argument, null);

  const calls = [];
  const result = await queueCadCommand(
    async (...args) => { calls.push(args); return { accepted: true }; },
    'ZOOM', loaded, false,
  );
  assert.equal(result.kind, 'accepted');
  assert.deepEqual(calls, [['cad_command', { command: 'ZOOM' }]]);
});

test('the full line is what reaches cad_command and what history stores', async () => {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  let log = [];
  for (const typed of ['ZOOM 2', ' zoom 0.5 ', 'ZOOM ALL']) {
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.equal(result.kind, 'accepted', typed);
    log = appendCommand(log, result.command);
  }
  assert.deepEqual(calls, [
    ['cad_command', { command: 'ZOOM 2' }],
    ['cad_command', { command: 'ZOOM 0.5' }],
    ['cad_command', { command: 'ZOOM ALL' }],
  ]);
  // `App.tsx` stores `result.command` — the full line with its argument.
  assert.deepEqual(commandOnly(log), ['ZOOM 2', 'ZOOM 0.5', 'ZOOM ALL']);

  // A refused argument never reaches IPC at all.
  const refused = await queueCadCommand(
    () => assert.fail('an out-of-grammar ZOOM reached IPC'),
    'ZOOM 0', loaded, false,
  );
  assert.equal(refused.kind, 'unsupported');
  assert.equal(refused.reasonKey, UNSUPPORTED_KEY);
});

test('the suggestion dropdown still filters on the base name', () => {
  // The display catalog lists the two zoom buttons and the host publishes the
  // `ZOOM EXTENTS` alias; both must keep appearing while the factor is typed —
  // the raw needle `ZOOM 2` would match nothing at all.
  const rows = filterCommandSuggestions('ZOOM 2');
  assert.ok(rows.length > 0, 'the dropdown must not go blank on a typed argument');
  assert.ok(rows.some(row => row.id === 'ZOOMIN'));
  assert.ok(rows.some(row => row.id === 'ZOOMOUT'));
  assert.ok(rows.some(row => row.id === 'ZOOM EXTENTS'), 'the alias row keeps matching');
  // Highlighting stays on the zoom family, and a bare query is unchanged.
  assert.equal(rows[defaultSuggestionIndex(rows, 'ZOOM 2')].id, 'ZOOMIN');
  assert.deepEqual(
    filterCommandSuggestions('ZOOM 2').map(row => row.id),
    filterCommandSuggestions('ZOOM').map(row => row.id),
    'the argument does not change which names are suggested',
  );
  const extents = filterCommandSuggestions('ZOOM EXTENTS');
  assert.ok(extents.some(row => row.id === 'ZOOM EXTENTS'));
});

test('Enter on a typed argument keeps the line instead of the bare suggestion', async () => {
  // The rule moved with the rest of the command line's decisions into the shared
  // controller, so both the docked field and the mobile bar keep the argument.
  const model = await readModule('commands/command-line-model.ts');
  assert.ok(model.includes('export function commandLineSubmitText('));
  assert.ok(
    model.includes('splitCommandText(command).argument === null'),
    'the Enter line must keep a typed argument',
  );
  const app = await readModule('App.tsx');
  assert.ok(
    app.includes('commandHistory.appendCommand(result.command)'),
    'history stores the full admitted line',
  );
  assert.ok(
    app.includes('queueCadCommand(invoke, text, status, false)'),
    'the typed line reaches the host unchanged',
  );
});

test('SNAPUNIT is admitted by its base selector with the full typed line', async () => {
  for (const typed of ['SNAPUNIT 5', 'snapunit 0.5', '  SNAPUNIT 2.5  ', 'SNAPUNIT 25']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.reasonKey, null, typed);
    // The line, not just the name: the host validates the value itself.
    assert.equal(admission.command, splitCommandText(typed).text, typed);
  }
  // An out-of-grammar spacing never reaches IPC, and `SNAP 5` stays an argument
  // on a selector that takes none.
  for (const typed of ['SNAPUNIT 0', 'SNAPUNIT -5', 'SNAPUNIT abc', 'SNAPUNIT 2 3', 'SNAP 5']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'unsupported', typed);
    assert.equal(admission.reasonKey, UNSUPPORTED_KEY, typed);
    assert.equal(admission.command, splitCommandText(typed).text, typed);
  }
  // Bare SNAPUNIT keeps the payload-refusal path exactly like bare ZOOM.
  const bare = admitCommandText('SNAPUNIT', { status: loaded });
  assert.equal(bare.kind, 'admitted');
  assert.equal(bare.command, 'SNAPUNIT');
  const calls = [];
  const result = await queueCadCommand(
    async (...args) => { calls.push(args); return { accepted: true }; },
    'SNAPUNIT 5', loaded, false,
  );
  assert.equal(result.kind, 'accepted');
  assert.deepEqual(calls, [['cad_command', { command: 'SNAPUNIT 5' }]]);
});
