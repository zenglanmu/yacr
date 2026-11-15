// AutoCAD's `COLOR <aci>|BYLAYER|BYBLOCK` (and the `-COLOR` prompt spelling) on
// the command line: local admission, the one `cad_command` dispatch, and the
// refusals.
//
// The chrome decides the **token shape** only: one integer or one of the three
// symbolic words, mirroring `crates/cad-ui-core/src/session/commands/extras.rs`
// `parse_color_argument` token for token. The closed ACI domain and every
// semantic answer stay the host's, so an out-of-domain integer (`COLOR 257`) is
// forwarded for the router to name (`color.invalid_value`) instead of being
// pre-empted with the chrome's `command.unsupported` — the same split
// `IMAGEADJUST` uses for its percentages.
//
// A shape outside the grammar is the existing stable refusal: the chrome's
// `command.unsupported` reason key, never a new reason vocabulary and never a
// fabricated `COLOR BYLAYER`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog, splitCommandText } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// The frozen `cad_status` shape: a drawing is open, so `COLOR` (a document-scoped
// selector) is available and the only thing an admission can turn on is the
// command/argument itself.
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
// The local grammar mirrors the host's token shapes.
// ---------------------------------------------------------------------------

test('the COLOR grammar is a whole ACI or one symbolic word', () => {
  const { parse } = ARGUMENT_COMMANDS.COLOR;
  for (const admitted of ['0', '1', '7', '256', '030', '+030', '-0', '-1', '257', '9999999999999999999999']) {
    assert.equal(parse(admitted), true, admitted);
  }
  for (const admitted of ['BYLAYER', 'bylayer', 'ByLayer', 'BYNAME', 'byname', 'BYBLOCK', 'byblock']) {
    assert.equal(parse(admitted), true, admitted);
  }
  for (const refused of ['', '   ', '1.0', '1e3', '0x1', 'aci:1', '1 2', 'RED', 'red', 'BYLAYER 1', '+', '-', '1,']) {
    assert.equal(parse(refused), false, refused);
  }
  // Both spellings share the **one** predicate: `-COLOR` is the prompt form of
  // the same session token, so a second grammar is impossible by construction.
  assert.equal(ARGUMENT_COMMANDS['-COLOR'].parse, ARGUMENT_COMMANDS.COLOR.parse);
});

test('an out-of-domain ACI is the host\'s refusal, not a local one', () => {
  // `257` and `-1` are integers, so the chrome forwards them and the router names
  // them (`color.invalid_value`); refusing them here would turn a real, named
  // refusal into a generic "unsupported".
  for (const line of ['COLOR 257', 'COLOR -1']) {
    const admission = admitCommandText(line, { status: loaded });
    assert.equal(admission.kind, 'admitted', line);
    assert.equal(admission.command, line, line);
  }
});

// ---------------------------------------------------------------------------
// Admission dispatches exactly one IPC call — and a refusal dispatches none.
// ---------------------------------------------------------------------------

test('every admitted COLOR form dispatches exactly one cad_command with the line', async () => {
  // The argument tail keeps the shared normal form (trimmed, upper-cased), which
  // the host grammar reads case-insensitively on both sides.
  const dispatched = [
    ['COLOR 1', 'COLOR 1'],
    ['COLOR 0', 'COLOR 0'],
    ['COLOR 256', 'COLOR 256'],
    ['color bylayer', 'COLOR BYLAYER'],
    ['COLOR BYBLOCK', 'COLOR BYBLOCK'],
    ['-COLOR 5', '-COLOR 5'],
    ['COLOR +030', 'COLOR +030'],
  ];
  for (const [typed, expected] of dispatched) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: expected }, typed);
    assert.deepEqual(calls, [['cad_command', { command: expected }]], typed);
  }
});

test('a malformed COLOR argument dispatches nothing and keeps the stable reason key', async () => {
  for (const typed of ['COLOR 1.0', 'COLOR RED', 'COLOR 1 2', 'COLOR 1e3', 'COLOR aci:1', '-COLOR 1_0']) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.equal(result.kind, 'unsupported', typed);
    assert.equal(result.reasonKey, UNSUPPORTED_KEY, typed);
    assert.deepEqual(calls, [], `${typed} must not reach cad_command`);
  }
});

// ---------------------------------------------------------------------------
// The bare selectors keep their existing path.
// ---------------------------------------------------------------------------

test('bare COLOR and -COLOR stay name-only and unchanged', async () => {
  for (const bare of ['COLOR', '-COLOR']) {
    // A trailing space is still a name-only line: the splitter never produces an
    // empty argument, so the bare report cannot be mistaken for a refused value.
    assert.deepEqual(splitCommandText(`${bare}   `), { name: bare, argument: null, text: bare });
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, bare, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: bare }, bare);
    assert.deepEqual(calls, [['cad_command', { command: bare }]], bare);
  }
});
