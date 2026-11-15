// HATCHEDIT typed-option admission: the chrome's local grammar must mirror the
// host parser (`crates/cad-ui-core/src/session/hatchedit.rs`) so no shipped
// option is refused before IPC, and no line the host would refuse is admitted as
// a different rule.
//
// What the chrome decides: the option **word** and the token **shape** (one
// plain signed decimal, a `u8`-shaped index, one keyword, the `PATTERNFILE`
// `<path> [<name>]` split). What the host decides and this suite pins as
// *forwarded*: the pattern-name table, the pattern-fill requirement, layer
// existence, `DRAWORDER BOUNDARY`'s missing boundary record, the three
// named-refusal forms, and reading a `PATTERNFILE` path (the chrome never touches
// a file).
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

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
  selection: { count: 1 },
};

/** One recorded `cad_command` call per invocation. */
function recorder() {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  return { calls, invoke };
}

// ---------------------------------------------------------------------------
// Every shipped option form is admitted locally.
// ---------------------------------------------------------------------------

test('every shipped HATCHEDIT option is admitted by the local grammar', () => {
  const admitted = [
    // Previously admitted (unchanged behaviour).
    'SOLID',
    'GRADIENT',
    'ANGLE 45',
    'ANGLE -22.5',
    'SCALE 2',
    'SCALE 0.5',
    'PATTERN ANSI31',
    'ADDBOUNDARIES',
    'REMOVEBOUNDARIES',
    // Newly mirrored from the host grammar.
    'STYLE IGNORE',
    'STYLE OUTER',
    'STYLE NORMAL',
    'PATTERNTYPE USER',
    'PATTERNTYPE PREDEFINED',
    'PATTERNTYPE CUSTOM',
    'COLOR 30',
    'COLOR +30',
    'COLOR 030',
    'COLOR 255',
    'COLOR BYLAYER',
    'TRANSPARENCY 0',
    'TRANSPARENCY 50',
    'TRANSPARENCY +90',
    'TRANSPARENCY BYLAYER',
    'LAYER Walls',
    'LAYER Walls exterior',
    'ORIGIN 1.5 -2',
    'ORIGIN 0 0',
    'ORIGIN +2.25 -0.5',
    'DRAWORDER BEHIND',
    'DRAWORDER FRONT',
    // The host owns this named refusal (`hatchedit.draw_order_boundary`).
    'DRAWORDER BOUNDARY',
    'RECREATEBOUNDARY',
    'SEPARATE',
    'PATTERNFILE /tmp/acad.pat',
    'PATTERNFILE /tmp/acadiso.pat MINE',
    'PATTERNFILE C:\\data\\acad.pat LADDER',
    // Named-refusal forms: forwarded so the **host** names the missing model.
    'ASSOCIATE',
    'DISASSOCIATE',
    'ANNOTATIVE',
  ];
  for (const argument of admitted) {
    assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse(argument), true, argument);
    const admission = admitCommandText(`HATCHEDIT ${argument}`, { status: loaded });
    assert.equal(admission.kind, 'admitted', argument);
    assert.equal(admission.reasonKey, null, argument);
  }
  // Case-insensitivity mirrors the host's `eq_ignore_ascii_case`.
  for (const argument of ['style outer', 'draworder behind', 'color bylayer', 'origin 1 2']) {
    assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse(argument), true, argument);
    assert.equal(
      admitCommandText(`hatchedit ${argument}`, { status: loaded }).kind,
      'admitted',
      argument,
    );
  }
});

// ---------------------------------------------------------------------------
// The host's refusals stay refusals (same stable reason key, nothing invented).
// ---------------------------------------------------------------------------

test('forms the host refuses are refused locally with the shared reason key', () => {
  const refused = [
    // Standalone words with a value, and valued words with none.
    'SOLID 1',
    'SEPARATE 1',
    'ASSOCIATE 1',
    'STYLE',
    'PATTERNTYPE',
    'DRAWORDER',
    'COLOR',
    'TRANSPARENCY',
    'LAYER',
    'ORIGIN',
    'PATTERNFILE',
    // Decimal shape: one plain signed decimal, no exponent, no fraction point only.
    'ANGLE 1e3',
    'ANGLE 1.',
    'ANGLE .5',
    'SCALE 0',
    'SCALE -1',
    'ORIGIN 1',
    'ORIGIN 1 2 3',
    'ORIGIN 1e3 2',
    'ORIGIN x 2',
    // `u8`-shaped bounds: ACI 1..=255, transparency 0..=90.
    'COLOR 0',
    'COLOR 256',
    'COLOR 1.0',
    'COLOR RED',
    'TRANSPARENCY 91',
    'TRANSPARENCY -1',
    'TRANSPARENCY 1.5',
    // Keyword sets are closed.
    'STYLE SIDEWAYS',
    'DRAWORDER SIDEWAYS',
    'PATTERNTYPE USER DEFINED',
    // A pattern name is one token; the name table itself is the host's.
    'PATTERN',
    'PATTERN ANSI 31',
    // `PATTERNFILE <path> [<name>]`: a spaced path is a path the grammar cannot
    // split from the name, which the host refuses as `pattern_file_path`.
    'PATTERNFILE /tmp/my patterns/acad.pat',
    'PATTERNFILE a.pat MINE EXTRA',
    // Not part of the grammar at all.
    'PROPERTIES',
    'ADD',
    'REMOVE',
    'ASSOCIATIVE',
  ];
  for (const argument of refused) {
    assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse(argument), false, argument);
    const admission = admitCommandText(`HATCHEDIT ${argument}`, { status: loaded });
    assert.equal(admission.kind, 'unsupported', argument);
    // The existing refusal style: a stable reason key, never free text of its own.
    assert.equal(admission.reasonKey, 'command.unsupported', argument);
  }
  // An empty argument is no option at all; the bare `HATCHEDIT` keeps its
  // name-only funnel (the host answers `hatchedit.needs_option`).
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse(''), false);
  assert.equal(admitCommandText('HATCHEDIT', { status: loaded }).kind, 'admitted');
});

test('an unknown built-in pattern name is still forwarded for the host to name', () => {
  // The name list lives in the host; the chrome admits the token shape and the
  // host answers `hatchedit.unknown_pattern` (no second name table here).
  const admission = admitCommandText('HATCHEDIT PATTERN NOSUCHNAME', { status: loaded });
  assert.equal(admission.kind, 'admitted');
  assert.equal(admission.command, 'HATCHEDIT PATTERN NOSUCHNAME');
});

// ---------------------------------------------------------------------------
// Admission dispatches exactly one IPC call — and a refusal dispatches none.
// ---------------------------------------------------------------------------

test('every admitted option form dispatches exactly one cad_command', async () => {
  const dispatched = [
    'HATCHEDIT SOLID',
    'HATCHEDIT GRADIENT',
    'HATCHEDIT ANGLE 45',
    'HATCHEDIT SCALE 2',
    'HATCHEDIT PATTERN ANSI31',
    'HATCHEDIT ADDBOUNDARIES',
    'HATCHEDIT REMOVEBOUNDARIES',
    'HATCHEDIT STYLE OUTER',
    'HATCHEDIT PATTERNTYPE CUSTOM',
    'HATCHEDIT COLOR 30',
    'HATCHEDIT COLOR BYLAYER',
    'HATCHEDIT TRANSPARENCY 40',
    'HATCHEDIT TRANSPARENCY BYLAYER',
    'HATCHEDIT LAYER WALLS',
    'HATCHEDIT ORIGIN 1.5 -2',
    'HATCHEDIT DRAWORDER BOUNDARY',
    'HATCHEDIT RECREATEBOUNDARY',
    'HATCHEDIT SEPARATE',
    'HATCHEDIT ASSOCIATE',
    'HATCHEDIT ANNOTATIVE',
    'HATCHEDIT PATTERNFILE /TMP/ACAD.PAT',
    'HATCHEDIT PATTERNFILE /TMP/ACADISO.PAT MINE',
  ];
  for (const text of dispatched) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, text, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: text }, text);
    assert.deepEqual(calls, [['cad_command', { command: text }]], text);
  }
});

test('a PATTERNFILE path is forwarded untouched and never read locally', async () => {
  // The chrome does not resolve, normalise, quote or stat the path: the whole
  // line reaches the host, whose resource strategy reads the file (a missing file
  // is the host's `hatchedit.pattern_file_missing`). The assertion also pins that
  // no extra IPC (a read/probe) happens.
  const { calls, invoke } = recorder();
  const text = 'HATCHEDIT PATTERNFILE /TMP/PATTERNS/ACADISO.PAT LADDER';
  const result = await queueCadCommand(invoke, text, loaded, false);
  assert.equal(result.kind, 'accepted');
  assert.deepEqual(calls, [['cad_command', { command: text }]]);
  assert.equal(calls.length, 1, 'one command call, no file probe');

  // A nonexistent-looking path is still admitted: existence is the host's answer.
  const second = recorder();
  const missing = 'HATCHEDIT PATTERNFILE /NONEXISTENT/ACAD.PAT';
  assert.equal((await queueCadCommand(second.invoke, missing, loaded, false)).kind, 'accepted');
  assert.deepEqual(second.calls, [['cad_command', { command: missing }]]);
});

// ---------------------------------------------------------------------------
// Case: a path tail keeps the spelling the user typed.
// ---------------------------------------------------------------------------

test('a PATTERNFILE path keeps its typed case; only the selector is upper-cased', async () => {
  // The whole point of the exception: the host reads the path from the
  // filesystem, so upper-casing it would be a wrong file on a case-sensitive
  // system. The **selector** is normalised like every other command line; the
  // argument tail keeps the spelling the user typed (the option word included —
  // the host matches option words case-insensitively, and the path must not be
  // rewritten).
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  const typed = 'hatchedit patternfile /tmp/My.Pat Custom';
  const admission = admitCommandText(typed, { status: loaded });
  assert.equal(admission.kind, 'admitted');
  assert.equal(admission.command, 'HATCHEDIT patternfile /tmp/My.Pat Custom');
  const result = await queueCadCommand(invoke, typed, loaded, false);
  assert.deepEqual(result, { kind: 'accepted', command: 'HATCHEDIT patternfile /tmp/My.Pat Custom' });
  assert.deepEqual(calls, [['cad_command', { command: 'HATCHEDIT patternfile /tmp/My.Pat Custom' }]]);

  // Already-upper-case lines are unchanged, the path is not rewritten in either
  // direction, and the line keeps its own separators (no re-spacing, no quoting).
  for (const [text, expected] of [
    ['HATCHEDIT PATTERNFILE /Tmp/MixedCase.PAT', 'HATCHEDIT PATTERNFILE /Tmp/MixedCase.PAT'],
    ['HATCHEDIT  PATTERNFILE  /tmp/Spaced.Pat  MINE', 'HATCHEDIT  PATTERNFILE  /tmp/Spaced.Pat  MINE'],
    ['Hatchedit PatternFile ..\\patterns\\Vendor.Pat Ladder', 'HATCHEDIT PatternFile ..\\patterns\\Vendor.Pat Ladder'],
  ]) {
    assert.equal(admitCommandText(text, { status: loaded }).command, expected, text);
  }
});

test('every other typed tail is still upper-cased exactly as before', async () => {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  // `HATCHEDIT ANGLE 45` keeps the old normal form — the exception is the path,
  // not the command.
  assert.equal(
    admitCommandText('hatchedit angle 45', { status: loaded }).command,
    'HATCHEDIT ANGLE 45',
  );
  assert.deepEqual(
    await queueCadCommand(invoke, 'hatchedit angle 45', loaded, false),
    { kind: 'accepted', command: 'HATCHEDIT ANGLE 45' },
  );
  assert.deepEqual(calls, [['cad_command', { command: 'HATCHEDIT ANGLE 45' }]]);
  // The other HATCHEDIT forms (keywords/names/numbers) are unchanged too.
  for (const [text, expected] of [
    ['hatchedit pattern ansi31', 'HATCHEDIT PATTERN ANSI31'],
    ['hatchedit layer Walls', 'HATCHEDIT LAYER WALLS'],
    ['hatchedit origin 1.5 -2', 'HATCHEDIT ORIGIN 1.5 -2'],
  ]) {
    assert.equal(admitCommandText(text, { status: loaded }).command, expected, text);
  }
});

test('IMAGEATTACH forwards its file path with the typed spelling and its numbers unchanged', () => {
  // The other already-shipped path tail: the first token is an opaque file name.
  const text = 'imageattach /tmp/Textures/My.Grain.Png 0 0 640 480 1';
  const admission = admitCommandText(text, { status: loaded });
  assert.equal(admission.kind, 'admitted');
  assert.equal(admission.command, 'IMAGEATTACH /tmp/Textures/My.Grain.Png 0 0 640 480 1');
  // The predicate still decides admission (a five-token line is refused, and its
  // echoed line keeps the typed spelling too).
  const refused = admitCommandText('imageattach /Tmp/My.png 0 0 640 480', { status: loaded });
  assert.equal(refused.kind, 'unsupported');
  assert.equal(refused.reasonKey, 'command.unsupported');
  assert.equal(refused.command, 'IMAGEATTACH /Tmp/My.png 0 0 640 480');
});

test('a refused form dispatches no IPC and keeps the shared reason key', async () => {
  const refused = [
    'HATCHEDIT PATTERNFILE',
    'HATCHEDIT PATTERNFILE /tmp/my patterns/acad.pat',
    'HATCHEDIT ORIGIN 1',
    'HATCHEDIT COLOR 256',
    'HATCHEDIT TRANSPARENCY 91',
    'HATCHEDIT DRAWORDER',
    'HATCHEDIT STYLE SIDEWAYS',
    'HATCHEDIT ANGLE 1e3',
    'HATCHEDIT SCALE 0',
  ];
  for (const text of refused) {
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, text, loaded, false);
    assert.equal(result.kind, 'unsupported', text);
    assert.equal(result.reasonKey, 'command.unsupported', text);
    assert.deepEqual(calls, [], text);
  }
});
