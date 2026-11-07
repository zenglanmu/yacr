import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ARGUMENT_COMMANDS,
  DOCUMENT_SCOPED_COMMANDS,
} from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

test('the layout/viewport batch is catalogued, admitted and document-scoped', () => {
  for (const id of [
    'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP',
    'VPSCALE', 'LAYVPI',
  ]) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(COMMAND_CATALOG.some(command => command.id === id), true, id);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, id);
  }
  assert.equal(isSupportedCommand('-VPORTS'), true, 'the alias is admitted');
});

test('the typed layout/viewport grammars mirror the host boundary', () => {
  const layout = ARGUMENT_COMMANDS.LAYOUT.parse;
  assert.equal(layout('LIST'), true);
  assert.equal(layout('NEW Sheet One'), true);
  assert.equal(layout('RENAME Old New'), true);
  assert.equal(layout('RENAME Old'), false);
  assert.equal(layout('BOGUS'), false);

  assert.equal(ARGUMENT_COMMANDS.LAYOUTMERGE.parse('One Two'), true);
  assert.equal(ARGUMENT_COMMANDS.LAYOUTMERGE.parse('One'), false);
  assert.equal(ARGUMENT_COMMANDS.LAYOUTWIZARD.parse('Sheet3'), true);
  assert.equal(ARGUMENT_COMMANDS.LAYOUTWIZARD.parse('Sheet3 A4 210 297'), true);
  assert.equal(ARGUMENT_COMMANDS.LAYOUTWIZARD.parse('Sheet3 A4 0 297'), false);

  const vports = ARGUMENT_COMMANDS.VPORTS.parse;
  assert.equal(vports('2 3'), true);
  assert.equal(vports('0 3'), false);
  assert.equal(vports('2'), false);
  assert.equal(ARGUMENT_COMMANDS['-VPORTS'].parse('2 3'), true, 'the alias shares the grammar');

  const vpclip = ARGUMENT_COMMANDS.VPCLIP.parse;
  assert.equal(vpclip('RECT 0 0 100 50'), true);
  // A non-RECT boundary is admitted here so the host's own
  // `vpclip.polygonal_not_implemented` refusal can speak.
  assert.equal(vpclip('POLY 0 0 100 50'), true);
  assert.equal(vpclip('RECT 0 0'), false);

  assert.equal(ARGUMENT_COMMANDS.VPSCALE.parse('1:100'), true);
  assert.equal(ARGUMENT_COMMANDS.VPSCALE.parse('NOPE'), true);
  assert.equal(ARGUMENT_COMMANDS.VPSCALE.parse('1 2'), false);

  const layvpi = ARGUMENT_COMMANDS.LAYVPI.parse;
  assert.equal(layvpi('WALLS,HIDDEN'), true);
  assert.equal(layvpi('WALLS,'), false);
});

test('every new catalog label resolves in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of [
    'draw.kind.layout', 'draw.kind.layoutmerge', 'draw.kind.layoutwizard',
    'draw.kind.vports', 'draw.kind.vpmax', 'draw.kind.vpmin', 'draw.kind.vpclip',
    'draw.kind.vpscale', 'draw.kind.layvpi',
  ]) {
    assert.equal(typeof zh[key], 'string', `${key} (zh-CN)`);
    assert.equal(typeof en[key], 'string', `${key} (en)`);
  }
});
