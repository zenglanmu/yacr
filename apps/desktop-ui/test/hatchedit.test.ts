// HATCHEDIT (AutoCAD 2026) affordance contracts: the typed v1 subset.
//
// `HATCHEDIT` is one canonical selector whose option is typed (`ANGLE`,
// `SCALE`, `SOLID`, `GRADIENT`), so the chrome must:
//   * carry one catalog row and one hatch-family split button on Annotate;
//   * admit the option forms through `ARGUMENT_COMMANDS.HATCHEDIT` (never a
//     second name list and never a fabricated `HATCHEDIT SOLID` vocabulary row);
//   * gate the command on exactly one selected object with its own hint, and let
//     a bare `HATCHEDIT` reach the host's explicit needs-option refusal;
//   * name the NOT IMPLEMENTED dialog fields in the hint copy.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

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

test('HATCHEDIT is one catalog row and one Annotate hatch split button', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'HATCHEDIT');
  assert.ok(entry, 'HATCHEDIT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.hatchedit');
  assert.equal(entry.icon, 'hatch');

  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate')
    .panels.find(panel => panel.id === 'annotation');
  assert.ok(annotate, 'Annotate keeps its annotation panel');
  const items = annotate.items.filter(item => item.id === 'HATCHEDIT');
  assert.equal(items.length, 1, 'exactly one HATCHEDIT button');
  const item = items[0];
  assert.equal(item.command, 'HATCHEDIT');
  assert.equal(item.action, undefined, 'HATCHEDIT is a router command');
  assert.ok(Array.isArray(item.variants), 'HATCHEDIT is a split button');
  assert.deepEqual(
    item.variants.map(variant => variant.id),
    ['HATCHEDIT SOLID', 'HATCHEDIT GRADIENT'],
  );
  assert.deepEqual(
    item.variants.map(variant => variant.command),
    ['HATCHEDIT SOLID', 'HATCHEDIT GRADIENT'],
  );
  // ANGLE/SCALE need a numeric value, so they are typed forms and get no
  // fabricated caret row; so do the unimplemented dialog fields.
  for (const variant of item.variants) {
    assert.equal(
      /ANGLE|SCALE|PATTERN|COLOR|PROPERTIES|DRAWORDER|ADD|REMOVE|ASSOCIATIVE/i.test(
        String(variant.command),
      ),
      false,
      `${variant.command} must not be offered as a one-click row`,
    );
  }
});

test('HATCHEDIT option forms are argument grammar, not extra command names', () => {
  assert.equal(HOST_NAMES.includes('HATCHEDIT'), true);
  for (const name of ['HATCHEDIT SOLID', 'HATCHEDIT GRADIENT', 'HATCHEDIT ANGLE', 'HATCHEDIT SCALE']) {
    assert.equal(HOST_NAMES.includes(name), false, `${name} must not be a published name`);
  }
  assert.equal(isSupportedCommand('HATCHEDIT'), true);

  for (const text of [
    'HATCHEDIT SOLID',
    'HATCHEDIT GRADIENT',
    'HATCHEDIT ANGLE 45',
    'HATCHEDIT ANGLE -22.5',
    'HATCHEDIT SCALE 2',
    'HATCHEDIT SCALE 0.5',
    'HATCHEDIT ADDBOUNDARIES',
    'HATCHEDIT REMOVEBOUNDARIES',
    'hatchedit angle 30',
  ]) {
    const admission = admitCommandText(text, { status: loaded });
    assert.equal(admission.kind, 'admitted', text);
    assert.equal(admission.reasonKey, null, text);
  }
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('SOLID'), true);
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('gradient'), true);
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('ANGLE 45'), true);
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('SCALE 2'), true);
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('ADDBOUNDARIES'), true);
  assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse('REMOVEBOUNDARIES'), true);
  for (const argument of [
    'PATTERN',
    'COLOR',
    'PROPERTIES',
    'DRAWORDER',
    'ADD',
    'REMOVE',
    'ASSOCIATIVE',
    'ANGLE',
    'SCALE',
    'ANGLE 1e3',
    'SCALE 0',
    'SCALE -1',
    'SOLID 1',
    '',
  ]) {
    assert.equal(ARGUMENT_COMMANDS.HATCHEDIT.parse(argument), false, argument);
  }
  for (const argument of ['PATTERN', 'SCALE 0', 'ANGLE 1e3']) {
    assert.equal(
      admitCommandText(`HATCHEDIT ${argument}`, { status: loaded }).kind,
      'unsupported',
      argument,
    );
  }

  // A bare HATCHEDIT is admitted by base name: the host answers with its
  // explicit `hatchedit.needs_option` refusal, never a defaulted option.
  const bare = admitCommandText('HATCHEDIT', { status: loaded });
  assert.equal(bare.kind, 'admitted');
  assert.equal(bare.command, 'HATCHEDIT');

  setCommandCatalog([]);
  assert.equal(isSupportedCommand('HATCHEDIT'), false);
  assert.equal(admitCommandText('HATCHEDIT SOLID', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('HATCHEDIT is gated on exactly one selected object with its own hint', () => {
  assert.equal(SELECTION_REQUIREMENTS.HATCHEDIT.count, 1);
  assert.equal(SELECTION_REQUIREMENTS.HATCHEDIT.reasonKey, 'draw.kind.hatchedit.hint');
  assert.equal(
    unavailableReasonKey('HATCHEDIT', { status: { ...loaded, selection: { count: 0 } } }),
    'draw.kind.hatchedit.hint',
  );
  assert.notEqual(
    unavailableReasonKey('HATCHEDIT', { status: { ...loaded, selection: { count: 1 } } }),
    'draw.kind.hatchedit.hint',
  );
  assert.equal(
    unavailableReasonKey('HATCHEDIT', {
      status: { ...loaded, documentLabel: null, selection: { count: 0 } },
    }),
    'plot.no_document',
  );
});

test('HATCHEDIT copy names the implemented forms and the unimplemented dialog fields', async () => {
  for (const locale of [await readCatalog('zh-CN'), await readCatalog('en')]) {
    for (const key of [
      'draw.kind.hatchedit',
      'draw.kind.hatchedit.hint',
      'draw.kind.hatchedit.solid',
      'draw.kind.hatchedit.gradient',
    ]) {
      assert.equal(typeof locale[key], 'string', key);
      assert.ok(locale[key].trim().length > 0, key);
    }
  }
  const en = await readCatalog('en');
  assert.match(en['draw.kind.hatchedit.hint'], /ANGLE/);
  assert.match(en['draw.kind.hatchedit.hint'], /SCALE/);
  assert.match(en['draw.kind.hatchedit.hint'], /SOLID/);
  assert.match(en['draw.kind.hatchedit.hint'], /GRADIENT/);
  assert.match(en['draw.kind.hatchedit.hint'], /Pattern/);
  assert.match(en['draw.kind.hatchedit.hint'], /Properties/);
  assert.match(en['draw.kind.hatchedit.hint'], /associativity/i);
  assert.match(en['draw.kind.hatchedit.hint'], /not implemented/i);
  assert.match(en['draw.kind.hatchedit.hint'], /\{command\}/);
});
