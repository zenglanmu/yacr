// The POINT display style sysvars on the command line: `PDMODE <mode>` and
// `PDSIZE <size>`.
//
// Both are command-line settings with exactly one numeric argument, modelled on
// `SNAPUNIT <spacing>`: the chrome admits by **base selector** and sends the full
// normalized line, so the value reaches `cad_command` untouched and the host stays
// the authority for the accepted range. A value outside the local one-number
// grammar is an explicit `unsupported` answer carrying the shared
// `command.unsupported` key — never a clamp and never a fabricated `PDMODE 3`.
//
// Neither is a button: `SNAPUNIT` has no ribbon item and these two have none
// either. They write the open drawing's point display style — a document value
// that the exporter writes and the next OPEN replaces — with no undo entry, so no
// copy here may call it session state or claim it is dropped on save.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ARGUMENT_COMMANDS,
  admitCommandText,
  COMMAND_CATALOG,
  commandById,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  splitCommandText,
} from '../src/commands/commands.ts';
import { queueCadCommand } from '../src/cad/cad-command.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readAppSource, readCatalog, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// The frozen `cad_status` shape: a drawing is open, so the only reason an
// admission could fail here is the command/argument itself.
const loaded = {
  documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true,
  gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true,
};

const UNSUPPORTED_KEY = 'command.unsupported';

const SYNVARS = [
  { id: 'PDMODE', labelKey: 'point.display.mode', hintKey: 'point.display.mode.hint', accepted: ['3', '0', '1'] },
  { id: 'PDSIZE', labelKey: 'point.display.size', hintKey: 'point.display.size.hint', accepted: ['2.5', '0', '-2.5'] },
];

test('the sysvars are catalogued command-line settings, not ribbon buttons', async () => {
  const expected = Object.fromEntries(SYNVARS.map(row => [row.id, row.labelKey]));
  for (const [id, labelKey] of Object.entries(expected)) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, labelKey, id);
    assert.equal(entry.group, 'view', `${id} sits with the drafting-aid/view settings`);
    // The glyph really exists, so the suggestion row cannot silently fall back
    // to the `dot` placeholder.
    assert.equal(typeof commandById(id).icon, 'string', id);
    // A dispatch token, never a local shell action: `commandById` is keyed by the
    // selector itself.
    assert.equal(commandById(id).id, id, id);
  }
  const icons = await readModuleIcons();
  for (const id of Object.keys(expected)) assert.ok(icons.has(commandById(id).icon), `${id}: unknown icon`);
  // The `SNAPUNIT` precedent: a typed setting gets no ribbon button, and neither
  // does this pair. Neither the id nor its dispatch token may appear in a panel.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      for (const item of panel.items) {
        assert.equal(Object.hasOwn(expected, item.id), false, `${tab.id}/${panel.id}: no ${item.id} button`);
        assert.equal(Object.hasOwn(expected, item.command ?? ''), false, `${tab.id}/${panel.id}: no ${item.command} dispatch`);
      }
    }
  }
});

test('both labels and both range hints exist in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const { id, labelKey, hintKey } of SYNVARS) {
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[labelKey], 'string', `${id}: ${labelKey} (${locale})`);
      assert.equal(typeof catalog[hintKey], 'string', `${id}: ${hintKey} (${locale})`);
      // A label names the variable it sets, so the reader can connect the copy
      // to the word they typed.
      assert.match(catalog[labelKey], new RegExp(id), `${id}: label names ${id} (${locale})`);
    }
    // The hint describes the accepted values. The copy contract is the honest
    // pair: the value lives in the drawing (so it survives saving and is replaced
    // by the next OPEN), and there is no undo entry for it.
    assert.match(en[hintKey], /number/i, `${id}: the hint says what the argument is`);
    assert.match(en[hintKey], /stored in the drawing/i, `${id}: a document value, not session state`);
    assert.match(en[hintKey], /survives saving/i, `${id}: the exporter writes it`);
    assert.match(en[hintKey], /cannot be undone/i, `${id}: not undoable`);
    // The two claims that used to be here are false and must not come back.
    assert.doesNotMatch(en[hintKey], /session only|session-scoped|does not write/i, `${id}: no stale non-persistence claim`);
    assert.match(zh[hintKey], /一个数值/, `${id}: the hint says what the argument is`);
    assert.match(zh[hintKey], /保存在图纸/, `${id}: a document value, not session state`);
    assert.match(zh[hintKey], /保存后仍然有效/, `${id}: the exporter writes it`);
    assert.match(zh[hintKey], /不能撤销/, `${id}: not undoable`);
    assert.doesNotMatch(zh[hintKey], /本图纸会话|不会写入/, `${id}: no stale non-persistence claim`);
  }
  // The two settings are presented as two different facts.
  assert.notEqual(zh['point.display.mode'], zh['point.display.size']);
  assert.notEqual(en['point.display.mode'], en['point.display.size']);
});

test('the argument table keeps one numeric grammar per selector', () => {
  assert.deepEqual(Object.keys(ARGUMENT_COMMANDS), ['ZOOM', 'SNAPUNIT', 'PDMODE', 'PDSIZE']);
  for (const { id, accepted } of SYNVARS) {
    const { parse } = ARGUMENT_COMMANDS[id];
    for (const admitted of accepted) assert.equal(parse(admitted), true, `${id} ${admitted}`);
    // One plain decimal: an empty argument, a non-number, an extra token, an
    // exponent, a bare fraction and a trailing dot are all refused.
    for (const refused of ['', ' ', 'abc', '2 3', 'ALL', 'EXTENTS', '.5', '5.', '1e3', 'NaN', '+5']) {
      assert.equal(parse(refused), false, `${id} ${JSON.stringify(refused)}`);
    }
  }
  // `PDSIZE 2.5` and `PDMODE 3` are the two shapes the copy documents. Zero and
  // a negative size are legal DWG values, so the local gate must not refuse them
  // (`PDMODE 0` is the dot default, `PDSIZE 0` the viewport-relative one, a
  // negative `PDSIZE` is a viewport percentage).
  assert.equal(ARGUMENT_COMMANDS.PDSIZE.parse('2.5'), true);
  assert.equal(ARGUMENT_COMMANDS.PDMODE.parse('3'), true);
  assert.equal(ARGUMENT_COMMANDS.PDSIZE.parse('0'), true);
  assert.equal(ARGUMENT_COMMANDS.PDSIZE.parse('-12.5'), true);
});

test('an admitted line keeps its value and dispatches by the selector itself', async () => {
  for (const typed of ['PDMODE 3', 'pdmode 0', '  PDMODE 2  ', 'PDSIZE 2.5', 'pdsize -12.5', '  PDSIZE 0  ']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.reasonKey, null, typed);
    // The line, not just the name: the host validates the value itself.
    assert.equal(admission.command, splitCommandText(typed).text, typed);
    // The dispatch token is the canonical selector, and the argument travels
    // beside it on the same `cad_command` line.
    const { name, argument } = splitCommandText(typed);
    assert.ok(name === 'PDMODE' || name === 'PDSIZE', typed);
    assert.equal(name, commandById(name).id, typed);
    assert.notEqual(argument, null, typed);
  }
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  for (const typed of ['PDMODE 3', ' pdsize 2.5 ']) {
    const result = await queueCadCommand(invoke, typed, loaded, false);
    assert.equal(result.kind, 'accepted', typed);
  }
  // One funnel, one IPC command: no dedicated sysvar channel was added.
  assert.deepEqual(calls, [
    ['cad_command', { command: 'PDMODE 3' }],
    ['cad_command', { command: 'PDSIZE 2.5' }],
  ]);
  const source = await readAppSource();
  for (const name of ['cad_pdmode', 'cad_pdsize', 'cad_set_point_display', 'cad_point_style']) {
    assert.equal(source.includes(name), false, `no dedicated IPC command (${name})`);
  }
});

test('an out-of-grammar argument is refused with the existing copy, before IPC', async () => {
  for (const typed of ['PDMODE abc', 'PDMODE 2 3', 'PDMODE ALL', 'PDSIZE abc', 'PDSIZE 2 3', 'PDSIZE 1e3']) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'unsupported', typed);
    // The shared refusal sentence, never a second copy for a sysvar value.
    assert.equal(admission.reasonKey, UNSUPPORTED_KEY, typed);
    // Nothing is clamped into a different line.
    assert.equal(admission.command, splitCommandText(typed).text, typed);
  }
  // A refused argument never reaches IPC at all.
  const refused = await queueCadCommand(
    () => assert.fail('an out-of-grammar sysvar line reached IPC'),
    'PDMODE abc', loaded, false,
  );
  assert.equal(refused.kind, 'unsupported');
  assert.equal(refused.reasonKey, UNSUPPORTED_KEY);
  // An argument on a selector that takes none stays refused, exactly like
  // `SNAP 5`: the sysvars did not widen the argument table to their neighbours.
  assert.equal(admitCommandText('POINT 3', { status: loaded }).kind, 'unsupported');
});

test('a bare selector keeps the payload-refusal path, exactly like SNAPUNIT', () => {
  // The chrome admits the name and sends it unchanged; the host owns the
  // missing-value answer (the same lane as bare `ZOOM`/`SNAPUNIT`). The chrome
  // never invents a value and never words a second refusal for it.
  for (const { id } of SYNVARS) {
    const bare = admitCommandText(id, { status: loaded });
    assert.equal(bare.kind, 'admitted', id);
    assert.equal(bare.command, id, `${id}: the name is not rewritten into a value`);
    assert.equal(splitCommandText(id).argument, null, id);
    // Trailing whitespace is normalization, not an empty argument.
    assert.equal(splitCommandText(`${id}  `).argument, null, id);
  }
});

test('admission is still the host vocabulary: nothing is enabled locally', () => {
  setCommandCatalog([]);
  for (const { id } of SYNVARS) {
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(`${id} 3`, { status: loaded }).kind, 'unsupported', id);
  }
  assert.ok(filterCommandSuggestions('PDMODE').every(row => row.supported === false));
  setCommandCatalog(HOST_NAMES);
  for (const { id } of SYNVARS) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(isSupportedCommand(id.toLowerCase()), true, id);
  }
});

test('the suggestion dropdown keeps matching on the base selector with a value typed', () => {
  const rows = filterCommandSuggestions('PDSIZE 2.5');
  assert.ok(rows.length > 0, 'the dropdown must not go blank on a typed argument');
  assert.equal(rows[0].id, 'PDSIZE');
  assert.equal(rows[0].labelKey, 'point.display.size');
  // The argument does not change which names are suggested.
  assert.deepEqual(
    filterCommandSuggestions('PDMODE 3').map(row => row.id),
    filterCommandSuggestions('PDMODE').map(row => row.id),
  );
});

// The icon set is a plain object literal in `icons.tsx`; this reads it as text
// (like `id-join-layiso.test.ts`) so an unknown glyph name fails here instead of
// silently drawing the `dot` fallback.
async function readModuleIcons() {
  const source = await readModule('icons.tsx');
  const map = source.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/^\s*([A-Za-z][A-Za-z0-9]*):/gm)].map(match => match[1]));
}
