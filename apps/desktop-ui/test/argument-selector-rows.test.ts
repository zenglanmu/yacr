// The nine typed arguments the host already parsed while the React command line
// had no row for them (2026-11-15 audit of `ARGUMENT_SELECTORS` against
// `ARGUMENT_COMMANDS`): `ADDSELECTED`, `ALIASEDIT`, `MLEDIT`, `-MLEDIT`,
// `MLEADERALIGN`, `MLEADEREDIT`, `MVSETUP`, `TOLERANCE` and `SSX`.
//
// Each predicate is the **token shape** only, mirroring its host parser
// (`commands/{mleader,filters,shape_block,paper_arguments}.rs`,
// `session/command_overrides.rs`). Every value/selection refusal stays the host's
// named one — `addselected.refusal`, `aliasedit.malformed`, `mledit.*`,
// `mleaderalign.*`, `mleaderedit.*`, `mvsetup.invalid_argument`, `tolerance.*`,
// `ssx.*` — so nothing here may be pre-empted with the chrome's generic
// `command.unsupported`. A shape outside the grammar keeps exactly that generic
// refusal, with no new reason vocabulary and no fabricated argument.
//
// One tail is **free text**: `TOLERANCE <x> <y> <text…>` becomes TEXT entities,
// so its spelling is content. Admission forwards it with the typed case
// (`commands.ts`'s case-preserving tails, the same mechanism as `IMAGEATTACH`'s
// path); the two coordinate tokens are numbers, so preserving them is a no-op.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog, splitCommandText } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { HOST_NAMES } from './helpers.ts';

// Every selector in this batch is a real host selector the shared mirror lists
// (`COMMAND_TABLE`/`COMMAND_ALIASES`); `-MLEDIT` and `-MLEDIT` are distinct
// canonical selectors, never alias spellings.
setCommandCatalog(HOST_NAMES);

// The frozen `cad_status` shape: a drawing is open (all nine selectors are
// document-scoped) and `SSX`'s one-object minimum is met, so the only thing an
// admission can turn on is the command's own typed grammar.
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
  selection: { count: 2 },
};

const UNSUPPORTED_KEY = 'command.unsupported';

/** One recorded `cad_command` call per invocation. */
function recorder() {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  return { calls, invoke };
}

/**
 * Per selector: the typed lines that dispatch (with the exact wire text), and the
 * lines whose shape is outside the grammar.
 *
 * The wire text is the shared normal form (trimmed, upper-cased) — except
 * `TOLERANCE`'s text tail, which keeps the typed case.
 */
const CASES = [
  {
    selector: 'ADDSELECTED',
    dispatched: [
      ['ADDSELECTED 1 2', 'ADDSELECTED 1 2'],
      ['addselected -0.5 +2.5', 'ADDSELECTED -0.5 +2.5'],
    ],
    refused: ['ADDSELECTED 1', 'ADDSELECTED 1 2 3', 'ADDSELECTED a b', 'ADDSELECTED 1,2 3'],
  },
  {
    selector: 'ALIASEDIT',
    dispatched: [
      ['ALIASEDIT LIST', 'ALIASEDIT LIST'],
      ['aliasedit add l2 line', 'ALIASEDIT ADD L2 LINE'],
      ['ALIASEDIT REMOVE L2', 'ALIASEDIT REMOVE L2'],
    ],
    refused: ['ALIASEDIT LIST X', 'ALIASEDIT ADD L2', 'ALIASEDIT REMOVE A B', 'ALIASEDIT WIPE A'],
  },
  {
    selector: 'MLEDIT',
    dispatched: [
      ['MLEDIT MERGE 1 2', 'MLEDIT MERGE 1 2'],
      ['mledit break 0 0 4 4', 'MLEDIT BREAK 0 0 4 4'],
      ['MLEDIT OPEN -1.5 2', 'MLEDIT OPEN -1.5 2'],
    ],
    refused: [
      'MLEDIT MERGE 1', 'MLEDIT MERGE 1 2 3 4', 'MLEDIT BREAK 1 2',
      'MLEDIT BREAK 1 2 3 4 5', 'MLEDIT JOIN 1 2',
    ],
  },
  {
    selector: '-MLEDIT',
    dispatched: [
      ['-MLEDIT TEE 1 2', '-MLEDIT TEE 1 2'],
      ['-mledit closed 0.5 -0.5', '-MLEDIT CLOSED 0.5 -0.5'],
    ],
    refused: ['-MLEDIT TEE 1', '-MLEDIT BREAK 1 2', '-MLEDIT OPEN a b', '-MLEDIT CROSS 1 2 3 4 5'],
  },
  {
    selector: 'MLEADERALIGN',
    dispatched: [
      ['MLEADERALIGN LEFT', 'MLEADERALIGN LEFT'],
      ['mleaderalign right 2.5', 'MLEADERALIGN RIGHT 2.5'],
      ['MLEADERALIGN C', 'MLEADERALIGN C'],
    ],
    refused: [
      'MLEADERALIGN CENTER 0', 'MLEADERALIGN LEFT +2', 'MLEADERALIGN TOP',
      'MLEADERALIGN LEFT 2 3', 'MLEADERALIGN LEFT 1e2',
    ],
  },
  {
    selector: 'MLEADEREDIT',
    dispatched: [
      ['MLEADEREDIT ADD 1 2', 'MLEADEREDIT ADD 1 2'],
      ['mleaderedit remove 0', 'MLEADEREDIT REMOVE 0'],
      ['MLEADEREDIT REMOVE 12', 'MLEADEREDIT REMOVE 12'],
    ],
    refused: [
      'MLEADEREDIT ADD 1', 'MLEADEREDIT ADD a b', 'MLEADEREDIT REMOVE -1',
      'MLEADEREDIT REMOVE 1.5', 'MLEADEREDIT MOVE 1 2',
    ],
  },
  {
    selector: 'MVSETUP',
    dispatched: [
      ['MVSETUP 2 2 1:100', 'MVSETUP 2 2 1:100'],
      ['mvsetup 2 3 50', 'MVSETUP 2 3 50'],
      ['MVSETUP 4 4 25', 'MVSETUP 4 4 25'],
    ],
    refused: ['MVSETUP 2 2', 'MVSETUP 0 2 1:100', 'MVSETUP a 2 1:100', 'MVSETUP 2 2 1:100 X'],
  },
  {
    selector: 'TOLERANCE',
    dispatched: [
      // An empty tail is inside the grammar: the host answers `tolerance.text`.
      ['TOLERANCE 1 2', 'TOLERANCE 1 2'],
      ['TOLERANCE 1 2 |+0.1|-0.1|A', 'TOLERANCE 1 2 |+0.1|-0.1|A'],
      // The frame text keeps the spelling it was typed with.
      ['tolerance -10.5 4.25 Wall A |+0.1/-0.1|', 'TOLERANCE -10.5 4.25 Wall A |+0.1/-0.1|'],
    ],
    refused: ['TOLERANCE 1', 'TOLERANCE a b text', 'TOLERANCE 1..2 4 text', 'TOLERANCE 1 2.3.4 text'],
  },
  {
    selector: 'SSX',
    dispatched: [
      ['SSX LAYER Walls', 'SSX LAYER WALLS'],
      ['ssx pick', 'SSX PICK'],
      // The three unmodelled criteria are admitted so the **host** names them.
      ['SSX FLAG', 'SSX FLAG'],
      ['SSX THICKNESS 1e3', 'SSX THICKNESS 1E3'],
      ['SSX VECTOR 1 2 3', 'SSX VECTOR 1 2 3'],
      ['SSX CLEAR LTYPE', 'SSX CLEAR LTYPE'],
    ],
    refused: [
      'SSX LAYER', 'SSX CLEAR', 'SSX CLEAR NOPE', 'SSX FLAG 1',
      'SSX VECTOR 1 2', 'SSX VECTOR 1 2 3 4', 'SSX THICKNESS abc', 'SSX NOPE 1',
    ],
  },
];

// ---------------------------------------------------------------------------
// Admission dispatches exactly one IPC call with the exact line.
// ---------------------------------------------------------------------------

for (const { selector, dispatched } of CASES) {
  test(`${selector} dispatches exactly one cad_command for the host's grammar`, async () => {
    for (const [typed, expected] of dispatched) {
      const admission = admitCommandText(typed, { status: loaded });
      assert.equal(admission.kind, 'admitted', typed);
      assert.equal(admission.command, expected, typed);
      const { calls, invoke } = recorder();
      const result = await queueCadCommand(invoke, typed, loaded, false);
      assert.deepEqual(result, { kind: 'accepted', command: expected }, typed);
      assert.deepEqual(calls, [['cad_command', { command: expected }]], typed);
    }
  });
}

// ---------------------------------------------------------------------------
// A shape outside the grammar dispatches nothing and keeps the stable key.
// ---------------------------------------------------------------------------

for (const { selector, refused } of CASES) {
  test(`${selector} refuses a malformed argument before IPC`, async () => {
    for (const typed of refused) {
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
}

// ---------------------------------------------------------------------------
// The one shared predicate, and the tails whose spelling is load-bearing.
// ---------------------------------------------------------------------------

test('MLEDIT and -MLEDIT share the one grammar', () => {
  // The host dispatches both spellings to `parse_mledit_argument` and only the
  // recorded selector differs, so a second copy of the shapes would be a second
  // grammar by construction.
  assert.equal(ARGUMENT_COMMANDS['-MLEDIT'].parse, ARGUMENT_COMMANDS.MLEDIT.parse);
});

test('TOLERANCE keeps the frame text exactly as typed, the selector upper-cased', () => {
  // Free text stored verbatim (it becomes TEXT entities): upper-casing it would
  // rewrite the drawing's content, exactly like upper-casing a path would rename
  // the file. The two coordinate tokens are numbers, so their case is a no-op.
  assert.deepEqual(
    splitCommandText('tolerance 1 2 Mixed Case Text'),
    { name: 'TOLERANCE', argument: '1 2 MIXED CASE TEXT', text: 'TOLERANCE 1 2 MIXED CASE TEXT' },
  );
  assert.equal(
    admitCommandText('tolerance 1 2 Mixed Case Text', { status: loaded }).command,
    'TOLERANCE 1 2 Mixed Case Text',
  );
});

test('a bare selector in this batch stays name-only', () => {
  // Each base name keeps its existing name-only funnel (the host answers with its
  // own needs-argument refusal); no row may turn it into an admitted argument.
  for (const bare of ['ADDSELECTED', 'ALIASEDIT', 'MLEDIT', '-MLEDIT', 'MLEADERALIGN', 'MLEADEREDIT', 'MVSETUP', 'TOLERANCE', 'SSX']) {
    assert.deepEqual(splitCommandText(`${bare}  `), { name: bare, argument: null, text: bare }, bare);
    const admission = admitCommandText(bare, { status: loaded });
    assert.equal(admission.kind, 'admitted', bare);
    assert.equal(admission.command, bare, bare);
  }
});
