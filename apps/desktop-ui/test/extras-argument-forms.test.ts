// The four typed forms this wave closed between "the Rust session parses it" and
// "the React command line admits it": `LINETYPE`/`-LINETYPE CREATE|DELETE`,
// the multi-vertex `MLINE`, `LAYOUT COPY` and `TABLEEXPORT <path.csv>`.
//
// Each predicate is the **token shape** only, mirroring its host parser
// (`session/commands/extras.rs::parse_linetype_argument`,
// `commands/draw2.rs::parse_mline_argument`,
// `commands/layout.rs::parse_layout_argument`,
// `session/table_export.rs::parse_tableexport_argument`). The host keeps every
// named refusal — `linetype.exists` / `linetype.bad_pattern`,
// `mline.point_count`, `layout.template_not_implemented`, `tableexport.path` /
// `.no_selection` / `.empty` / `unsupported-runtime: table-export` — so an
// admitted shape must reach IPC and a malformed one keeps the chrome's generic
// `command.unsupported` with no second reason vocabulary.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// A drawing is open: all four selectors are document-scoped, so the only thing
// admission can turn on is the typed grammar itself.
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

const UNSUPPORTED_KEY = 'command.unsupported';

function recorder() {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  return { calls, invoke };
}

/** One accepted line: the typed text and the exact `cad_command` payload. */
const DISPATCHED = [
  // CREATE without a description keeps the shared normal form.
  ['LINETYPE CREATE A 2,-1', 'LINETYPE CREATE A 2,-1'],
  ['linetype create "A B" 2,-1 DESCRIPTION dashed',
    'LINETYPE create "A B" 2,-1 DESCRIPTION dashed'],
  // DELETE takes the whole remainder as the name, exactly like the host.
  ['LINETYPE DELETE A', 'LINETYPE DELETE A'],
  ['linetype set a b', 'LINETYPE SET A B'],
  // The dash spelling shares the grammar; only the selector differs.
  ['-LINETYPE CREATE A 3,-1,1,-3', '-LINETYPE CREATE A 3,-1,1,-3'],
  // Multi-segment baselines: two or more vertices, unbounded locally.
  ['MLINE 1 0 0,0 10,0 10,10', 'MLINE 1 0 0,0 10,0 10,10'],
  ['MLINE 2 0.5 -1.5,2 3,4 5,6 7,8', 'MLINE 2 0.5 -1.5,2 3,4 5,6 7,8'],
  // COPY: the host duplicates a layout of the current drawing.
  ['LAYOUT COPY Sheet1 Sheet2', 'LAYOUT COPY SHEET1 SHEET2'],
  // TEMPLATE is admitted so the host answers `layout.template_not_implemented`.
  ['LAYOUT TEMPLATE x', 'LAYOUT TEMPLATE X'],
  // The destination path is a path tail: its typed case is preserved.
  ['TABLEEXPORT /tmp/My.Csv', 'TABLEEXPORT /tmp/My.Csv'],
  ['tableexport /tmp/Quarterly Report.CSV', 'TABLEEXPORT /tmp/Quarterly Report.CSV'],
] as const;

/** One malformed line per form: outside the grammar, so no IPC. */
const REFUSED = [
  // CREATE needs a pattern token, a decimal pattern and no stray trailer.
  'LINETYPE CREATE A',
  'LINETYPE CREATE A 2,x',
  'LINETYPE CREATE A 2,-1 TOTAL',
  'LINETYPE DELETE',
  // MLINE needs at least two vertices and whole `x,y` pairs.
  'MLINE 1 0 0,0',
  'MLINE 1 0 0,0 10',
  'MLINE 1 0 0,0 10;10',
  // COPY needs two names; an unknown option is outside the grammar.
  'LAYOUT COPY Sheet1',
  'LAYOUT WIPE Sheet1',
] as const;

test('every newly mirrored form dispatches exactly one cad_command', async () => {
  for (const [typed, expected] of DISPATCHED) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.command, expected, typed);
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.deepEqual(result, { kind: 'accepted', command: expected }, typed);
    assert.deepEqual(calls, [['cad_command', { command: expected }]], typed);
  }
});

test('a malformed argument is refused before IPC with the shared key', async () => {
  for (const typed of REFUSED) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'unsupported', typed);
    assert.equal(admission.reasonKey, UNSUPPORTED_KEY, typed);
    const { calls, invoke } = recorder();
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.equal(result.kind, 'unsupported', typed);
    assert.equal(result.reasonKey, UNSUPPORTED_KEY, typed);
    assert.deepEqual(calls, [], `${typed} must not reach cad_command`);
  }
});

test('TABLEEXPORT has no local path refusal: the host answers `tableexport.path`', async () => {
  // The extension check is a **named host refusal**, so admission must not
  // pre-empt it. A non-CSV destination therefore still reaches `cad_command`
  // (the host writes nothing and reports the reason), exactly like the empty
  // destination `TABLEEXPORT <path>` would if it could be typed.
  for (const typed of ['TABLEEXPORT /tmp/My.Dwg', 'TABLEEXPORT export.txt']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    const { calls, invoke } = recorder();
    await queueCadCommand(invoke, typed, loaded, false);
    assert.deepEqual(calls, [['cad_command', { command: typed }]], typed);
  }
});

test('LINETYPE and -LINETYPE share the one CREATE/SET/DELETE grammar', () => {
  assert.equal(ARGUMENT_COMMANDS['-LINETYPE'].parse, ARGUMENT_COMMANDS.LINETYPE.parse);
});

test('only a LINETYPE CREATE with DESCRIPTION preserves the typed case', () => {
  // The description is stored content (like a TOLERANCE frame or a path), but a
  // name in `SET`/`DELETE` is matched by the host and keeps the normal form.
  assert.equal(
    admitCommandText('linetype create A 2,-1 DESCRIPTION Dashed', { status: loaded }).command,
    'LINETYPE create A 2,-1 DESCRIPTION Dashed',
  );
  assert.equal(
    admitCommandText('linetype set My Type', { status: loaded }).command,
    'LINETYPE SET MY TYPE',
  );
});
