// The session-token and session-vocabulary typed forms on the command line:
// `WIPEOUTFRAME <0|1|2>`, `APERTURE <n>`, `PTYPE <mode> [<size>]`,
// `REVCLOUDPROPERTIES <length>`, `UNDEFINE <name>` and `REDEFINE <name>`.
//
// The chrome decides the **token shape** only; the closed domains and every
// keyword answer stay the host's, so an out-of-range value travels and is named
// there (`wipeoutframe.out_of_range`, `aperture.out_of_range`,
// `ptype.invalid_argument`). A shape outside the grammar keeps the existing
// stable refusal — `command.unsupported`, never a new reason vocabulary.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES } from './helpers.ts';

// `WIPEOUTFRAME`, `UNDEFINE` and `REDEFINE` are real host selectors the shared
// test mirror does not list; the host's `COMMAND_TABLE` carries all three, so the
// mirror is extended here instead of perturbing the fixture every test imports.
setCommandCatalog([...HOST_NAMES, 'WIPEOUTFRAME', 'UNDEFINE', 'REDEFINE']);

const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  gridEnabled: true,
  snapGridEnabled: false,
  snapSpacing: 10,
  orthoEnabled: false,
  canSave: true,
  canPlot: true,
  canFit: true,
};

const UNSUPPORTED_KEY = 'command.unsupported';

/** One recorded `cad_command` call per invocation. */
function recorder() {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  return { calls, invoke };
}

// ---------------------------------------------------------------------------
// The local grammars are token shapes, not value rules.
// ---------------------------------------------------------------------------

test('WIPEOUTFRAME admits one integer and leaves the range to the host', () => {
  const { parse } = ARGUMENT_COMMANDS.WIPEOUTFRAME;
  for (const admitted of ['0', '1', '2', '9', '-1', '030']) {
    assert.equal(parse(admitted), true, admitted);
  }
  for (const refused of ['', '   ', 'ON', 'off', '1 2', '1.0', '1e3', '+', '-']) {
    assert.equal(parse(refused), false, refused);
  }
});

test('the other session forms admit their shape and nothing more', () => {
  for (const code of ['0', '10', '999']) {
    assert.equal(ARGUMENT_COMMANDS.APERTURE.parse(code), true, code);
  }
  for (const refused of ['', '-1', '1.5', 'abc']) {
    assert.equal(ARGUMENT_COMMANDS.APERTURE.parse(refused), false, refused);
  }

  for (const line of ['3', '3 2.5', '-1', '+2 0', '40000']) {
    assert.equal(ARGUMENT_COMMANDS.PTYPE.parse(line), true, line);
  }
  for (const refused of ['', 'abc', '3.5.5', '0 0 0', 'ON']) {
    assert.equal(ARGUMENT_COMMANDS.PTYPE.parse(refused), false, refused);
  }

  assert.equal(ARGUMENT_COMMANDS.REVCLOUDPROPERTIES.parse('15'), true);
  assert.equal(ARGUMENT_COMMANDS.REVCLOUDPROPERTIES.parse('2.5'), true);
  for (const refused of ['', '0', '-1', 'abc', '1 2']) {
    assert.equal(ARGUMENT_COMMANDS.REVCLOUDPROPERTIES.parse(refused), false, refused);
  }

  // The two vocabulary controls take one command name; the host decides whether
  // the name exists.
  for (const name of ['ZOOM', 'some_command']) {
    assert.equal(ARGUMENT_COMMANDS.UNDEFINE.parse(name), true, name);
    assert.equal(ARGUMENT_COMMANDS.REDEFINE.parse(name), true, name);
  }
  for (const refused of ['', 'A B']) {
    assert.equal(ARGUMENT_COMMANDS.UNDEFINE.parse(refused), false, refused);
    assert.equal(ARGUMENT_COMMANDS.REDEFINE.parse(refused), false, refused);
  }
});

// ---------------------------------------------------------------------------
// Admission dispatches exactly one IPC call — and a refusal dispatches none.
// ---------------------------------------------------------------------------

test('every admitted session form dispatches exactly one cad_command with the line', async () => {
  const dispatched = [
    ['WIPEOUTFRAME 1', 'WIPEOUTFRAME 1'],
    ['WIPEOUTFRAME 2', 'WIPEOUTFRAME 2'],
    ['wipeoutframe 2', 'WIPEOUTFRAME 2'],
    ['APERTURE 10', 'APERTURE 10'],
    ['PTYPE 3', 'PTYPE 3'],
    ['PTYPE 3 2.5', 'PTYPE 3 2.5'],
    ['REVCLOUDPROPERTIES 15', 'REVCLOUDPROPERTIES 15'],
    ['UNDEFINE ZOOM', 'UNDEFINE ZOOM'],
    ['REDEFINE ZOOM', 'REDEFINE ZOOM'],
  ];
  for (const [typed, expected] of dispatched) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: expected }, typed);
    assert.deepEqual(calls, [['cad_command', { command: expected }]], typed);
  }
});

test('an out-of-range value reaches the host so it can name it', async () => {
  // The domain is the router's: `wipeoutframe.out_of_range` and
  // `aperture.out_of_range` are named refusals, so a local range test here would
  // mask them into the chrome's generic "unsupported".
  for (const typed of ['WIPEOUTFRAME 9', 'WIPEOUTFRAME -1', 'APERTURE 0', 'APERTURE 999']) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: typed }, typed);
    assert.deepEqual(calls, [['cad_command', { command: typed }]], typed);
  }
});

test('a malformed session argument dispatches nothing and keeps the stable reason key', async () => {
  for (const typed of [
    'WIPEOUTFRAME ON', 'WIPEOUTFRAME 1 2', 'WIPEOUTFRAME 1.0',
    'APERTURE -1', 'APERTURE 10PX',
    'PTYPE abc', 'PTYPE 1 2 3',
    'REVCLOUDPROPERTIES abc', 'REVCLOUDPROPERTIES 0',
    'UNDEFINE A B', 'REDEFINE A B',
  ]) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.equal(result.kind, 'unsupported', typed);
    assert.equal(result.reasonKey, UNSUPPORTED_KEY, typed);
    assert.deepEqual(calls, [], `${typed} must not reach cad_command`);
  }
});

// ---------------------------------------------------------------------------
// The bare selectors keep their existing name-only path.
// ---------------------------------------------------------------------------

test('bare WIPEOUTFRAME and the other session names stay name-only', async () => {
  for (const bare of ['WIPEOUTFRAME', 'APERTURE', 'PTYPE', 'REVCLOUDPROPERTIES', 'UNDEFINE', 'REDEFINE']) {
    const admission = admitCommandText(bare, { status: loaded });
    assert.equal(admission.kind, 'admitted', bare);
    assert.equal(admission.command, bare, bare);
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, bare, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: bare }, bare);
    assert.deepEqual(calls, [['cad_command', { command: bare }]], bare);
  }
});
