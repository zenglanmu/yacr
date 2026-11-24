// The typed command lines that became reachable when the host added them:
// `PYRAMID [INSCRIBED|CIRCUMSCRIBED] <sides> <cx> <cy> <radius> <height>`,
// `LAYERSTATESAVE [<name>|?]` and the extended `QLEADER`/`LEADER` grammars
// (`CONTENT …`, and `QLEADER`'s `ARROW`/`MAXPOINTS`).
//
// The chrome predicates are **token shapes only**; the host owns every semantic
// refusal (`pyramid.sides`, `pyramid.radius`, `mleader.option_max_points`,
// `qleader.invalid_max_points`, the layer-state store's rules) and this test
// only pins that a legal line travels while a shape outside the grammar is the
// shared `command.unsupported` answer.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
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
  selection: { count: 0 },
};

function parses(selector: string, argument: string): boolean {
  const spec = ARGUMENT_COMMANDS[selector];
  assert.ok(spec, `${selector} must have a local row`);
  return spec.parse(argument);
}

test('pyramid accepts the optional ring choice and exactly five numbers', () => {
  for (const line of [
    '6 0 0 5 10',
    'INSCRIBED 6 0 0 5 10',
    'I 4 -1 2.5 3 -4',
    'CIRCUMSCRIBED 5 1 2 3 4',
    'C 3 0 0 1 1',
  ]) {
    assert.equal(parses('PYRAMID', line), true, line);
  }
  for (const line of [
    '',
    '6 0 0 5',
    '6 0 0 5 10 11',
    '4.0 0 0 5 10',
    '-4 0 0 5 10',
    '6 nan 0 5 10',
    'SIDES 6 0 0 5 10',
    '6 0 0 5 10 extra',
  ]) {
    assert.equal(parses('PYRAMID', line), false, line);
  }
});

test('layerstatesave accepts a name or the list spelling', () => {
  for (const line of ['Plan A', 'SAVE', '?', 'RESTORE x']) {
    assert.equal(parses('LAYERSTATESAVE', line), true, line);
  }
  for (const line of ['', '? extra', '?name']) {
    assert.equal(parses('LAYERSTATESAVE', line), false, line);
  }
});

test('qleader and leader share the content prefix; only qleader owns arrow/maxpoints', () => {
  for (const line of [
    '1,2 3,4',
    '1,2 3,4 Note',
    'CONTENT MTEXT 1,2 3,4 Note',
    'CONTENT BLOCK Door 1,2 3,4',
    'CONTENT TOLERANCE 1,2 3,4 |a|b|c',
  ]) {
    assert.equal(parses('QLEADER', line), true, `QLEADER ${line}`);
    assert.equal(parses('LEADER', line), true, `LEADER ${line}`);
  }
  assert.equal(parses('QLEADER', 'ARROW OFF 1,2 3,4 Note'), true);
  assert.equal(parses('QLEADER', 'MAXPOINTS 2 1,2 3,4 Note'), true);
  assert.equal(parses('LEADER', 'ARROW OFF 1,2 3,4 Note'), false);
  assert.equal(parses('LEADER', 'MAXPOINTS 2 1,2 3,4 Note'), false);
  for (const line of [
    '1,2',
    'CONTENT 1,2 3,4',
    'CONTENT BLOCK 1,2 3,4',
    'CONTENT NOPE 1,2 3,4 Note',
    'ARROW MAYBE 1,2 3,4 Note',
    'MAXPOINTS x 1,2 3,4 Note',
  ]) {
    assert.equal(parses('QLEADER', line), false, line);
  }
});

test('the three typed lines are admitted and forwarded whole', () => {
  for (const [typed, wire] of [
    ['pyramid 6 0 0 5 10', 'PYRAMID 6 0 0 5 10'],
    ['pyramid circumscribed 4 1 1 2 3', 'PYRAMID CIRCUMSCRIBED 4 1 1 2 3'],
    ['layerstatesave plan a', 'LAYERSTATESAVE PLAN A'],
    ['layerstatesave ?', 'LAYERSTATESAVE ?'],
    ['qleader content block door 1,2 3,4', 'QLEADER CONTENT BLOCK DOOR 1,2 3,4'],
    ['qleader arrow off 1,2 3,4 note', 'QLEADER ARROW OFF 1,2 3,4 NOTE'],
  ]) {
    const admission = admitCommandText(typed, { status: loaded });
    assert.equal(admission.kind, 'admitted', typed);
    assert.equal(admission.command, wire, typed);
  }
});
