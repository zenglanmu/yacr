// The three Express-tool text commands (2026-11-07): TORIENT, TCASE,
// TXT2MTXT.
//
// All three are **selection-based** router commands (`CommandPayload::None`;
// the current selection is the whole input), so their buttons are ordinary
// annotate-tab entries gated by `SELECTION_REQUIREMENTS`. TORIENT and TCASE also
// accept one typed argument (`TORIENT <angle>` / `TCASE <mode>`), admitted by
// `ARGUMENT_COMMANDS`; TCASE exposes its five modes as popover rows.
//
// These tests pin the UI surface only. The Rust command paths and their write
// contracts live in `crates/cad-app/src/tests/text_commands.rs`; `HOST_NAMES` is
// the local mirror of the router vocabulary and never proves the host advertises
// a name.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { ARGUMENT_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const annotationPanel = () => RIBBON_TABS
  .find(tab => tab.id === 'annotate')
  .panels.find(panel => panel.id === 'annotation');

const selected = count => ({
  documentLabel: 'drawing.dwg',
  entityCount: 12,
  loading: false,
  selection: { count },
  canNew: true,
  canSave: true,
  canPlot: true,
});

// --- catalogue ---------------------------------------------------------------

test('the three text commands are annotate-group catalogue rows', () => {
  for (const entry of [
    { id: 'TORIENT', labelKey: 'draw.kind.torient', icon: 'rotate', group: 'annotate' },
    { id: 'TCASE', labelKey: 'draw.kind.tcase', icon: 'pencil', group: 'annotate' },
    { id: 'TXT2MTXT', labelKey: 'draw.kind.txt2mtxt', icon: 'copy', group: 'annotate' },
  ]) {
    assert.deepEqual({ ...COMMAND_CATALOG.find(command => command.id === entry.id) }, entry);
    assert.equal(
      COMMAND_CATALOG.filter(command => command.id === entry.id).length,
      1,
      `${entry.id}: exactly one row`,
    );
  }
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf('TORIENT'), order.indexOf('DIMCONTINUE') + 3);
  assert.equal(order.indexOf('TCASE'), order.indexOf('TORIENT') + 1);
  assert.equal(order.indexOf('TXT2MTXT'), order.indexOf('TCASE') + 1);
});

// --- ribbon ------------------------------------------------------------------

test('the annotate panel carries the three buttons and TCASE five mode rows', () => {
  const items = annotationPanel().items;
  const ids = items.map(item => item.id);
  assert.equal(ids.indexOf('TORIENT'), ids.indexOf('DIMCONTINUE') + 3);

  const torient = items.find(item => item.id === 'TORIENT');
  assert.equal(torient.command, 'TORIENT');
  assert.equal(torient.labelKey, 'draw.kind.torient');
  assert.equal(torient.icon, 'rotate');
  assert.equal(torient.large, undefined);
  assert.equal(torient.variants, undefined, 'the absolute form is the typed argument, not a popover');

  const tcase = items.find(item => item.id === 'TCASE');
  assert.equal(tcase.command, 'TCASE', 'the base click is the bare Title form');
  assert.equal(tcase.labelKey, 'draw.kind.tcase');
  assert.equal(tcase.icon, 'pencil');
  assert.equal(tcase.large, undefined);
  assert.deepEqual(
    tcase.variants.map(variant => variant.command),
    ['TCASE T', 'TCASE U', 'TCASE L', 'TCASE S', 'TCASE G'],
    'the popover sends the documented single-letter modes',
  );
  assert.deepEqual(
    tcase.variants.map(variant => variant.labelKey),
    [
      'draw.kind.tcase.title',
      'draw.kind.tcase.upper',
      'draw.kind.tcase.lower',
      'draw.kind.tcase.sentence',
      'draw.kind.tcase.toggle',
    ],
  );
  for (const variant of tcase.variants) {
    assert.equal(variant.icon, 'pencil');
    assert.equal(variant.command.startsWith('TCASE '), true);
  }

  const combined = items.find(item => item.id === 'TXT2MTXT');
  assert.equal(combined.command, 'TXT2MTXT');
  assert.equal(combined.labelKey, 'draw.kind.txt2mtxt');
  assert.equal(combined.icon, 'copy');
  assert.equal(combined.variants, undefined, 'v1 has no settings dialog to offer');

  // They are annotate affordances only, and the ribbon reaches every base
  // selector plus the five argument rows.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'annotation') continue;
      for (const id of ['TORIENT', 'TCASE', 'TXT2MTXT']) {
        assert.equal(
          panel.items.some(item => item.command === id),
          false,
          `${tab.id}/${panel.id} must not dispatch ${id}`,
        );
      }
    }
  }
  const reached = new Set(ribbonCommandIds());
  for (const id of ['TORIENT', 'TCASE', 'TXT2MTXT', 'TCASE T', 'TCASE U', 'TCASE L', 'TCASE S', 'TCASE G']) {
    assert.equal(reached.has(id), true, `the ribbon reaches ${id}`);
  }
});

// --- selection gate ----------------------------------------------------------

test('each command requires at least one selected object and words its own rule', () => {
  assert.deepEqual(SELECTION_REQUIREMENTS.TORIENT, {
    count: 1,
    reasonKey: 'draw.kind.torient.hint',
  });
  assert.deepEqual(SELECTION_REQUIREMENTS.TCASE, {
    count: 1,
    reasonKey: 'draw.kind.tcase.hint',
  });
  assert.deepEqual(SELECTION_REQUIREMENTS.TXT2MTXT, {
    count: 1,
    reasonKey: 'draw.kind.txt2mtxt.hint',
  });

  for (const [id, hint] of [
    ['TORIENT', 'draw.kind.torient.hint'],
    ['TCASE', 'draw.kind.tcase.hint'],
    ['TXT2MTXT', 'draw.kind.txt2mtxt.hint'],
  ]) {
    assert.equal(admitCommandText(id, { status: selected(1) }).kind, 'admitted', id);
    const empty = admitCommandText(id, { status: selected(0) });
    assert.equal(empty.kind, 'unavailable', `${id}: empty selection`);
    assert.equal(empty.reasonKey, hint, `${id}: the hint says what is missing`);
    const noDocument = admitCommandText(id, {
      status: { ...selected(0), documentLabel: null, entityCount: null },
    });
    assert.equal(noDocument.reasonKey, 'plot.no_document', `${id}: no drawing`);
  }
});

// --- typed arguments ---------------------------------------------------------

test('TORIENT admits one signed decimal angle and TCASE its five modes', () => {
  const torient = ARGUMENT_COMMANDS.TORIENT.parse;
  for (const value of ['45', '22.5', '0', '0.0', '-90', '+30', ' 45 ']) {
    assert.equal(torient(value), true, value);
  }
  for (const value of ['', 'abc', '1e3', '.5', '5.', '45 30', 'NaN', 'inf', '45deg']) {
    assert.equal(torient(value), false, value);
  }

  const tcase = ARGUMENT_COMMANDS.TCASE.parse;
  for (const value of ['UPPER', 'upper', 'U', 'u', 'LOWER', 'L', 'TITLE', 'T', 'SENTENCE', 'S', 'TOGGLE', 'G', ' Title ']) {
    assert.equal(tcase(value), true, value);
  }
  for (const value of ['', 'TITL', 'X', 'title case', 'UPPER TITLE', 'Sentence2']) {
    assert.equal(tcase(value), false, value);
  }
  // TXT2MTXT is name-only: it must not grow an argument grammar by accident.
  assert.equal(ARGUMENT_COMMANDS.TXT2MTXT, undefined);

  assert.equal(admitCommandText('TORIENT 45', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('TCASE U', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('TCASE UPPER', { status: selected(1) }).kind, 'admitted');
  assert.equal(admitCommandText('TORIENT abc', { status: selected(1) }).kind, 'unsupported');
  // The selection precondition applies to the argument form too.
  const empty = admitCommandText('TCASE U', { status: selected(0) });
  assert.equal(empty.kind, 'unavailable');
  assert.equal(empty.reasonKey, 'draw.kind.tcase.hint');
});

// --- vocabulary mirror -------------------------------------------------------

test('the host vocabulary mirror carries all three base selectors', () => {
  for (const id of ['TORIENT', 'TCASE', 'TXT2MTXT']) {
    assert.equal(isSupportedCommand(id), true, id);
  }
  setCommandCatalog([]);
  for (const id of ['TORIENT', 'TCASE', 'TXT2MTXT']) {
    assert.equal(isSupportedCommand(id), false, `${id}: an empty host admits nothing`);
  }
  setCommandCatalog(HOST_NAMES);
});

// --- copy --------------------------------------------------------------------

test('the labels and hints exist in both locales and state the v1 limits', () => {
  const keys = [
    'draw.kind.torient',
    'draw.kind.torient.hint',
    'draw.kind.tcase',
    'draw.kind.tcase.hint',
    'draw.kind.tcase.upper',
    'draw.kind.tcase.lower',
    'draw.kind.tcase.title',
    'draw.kind.tcase.sentence',
    'draw.kind.tcase.toggle',
    'draw.kind.txt2mtxt',
    'draw.kind.txt2mtxt.hint',
  ];
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The TXT2MTXT hint must not promise the settings dialog it does not have.
  assert.match(en['draw.kind.txt2mtxt.hint'], /not implemented/i);
  assert.match(zh['draw.kind.txt2mtxt.hint'], /未实现/);
  // TORIENT's hint names the two real input forms.
  assert.match(en['draw.kind.torient.hint'], /TORIENT <angle>/);
  assert.match(en['draw.kind.torient.hint'], /most readable/i);
  // TCASE's hint documents the Title default.
  assert.match(en['draw.kind.tcase.hint'], /Title case by default/);
});
