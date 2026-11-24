// ADR-0055 M3 UI wiring: the block-attribute commands are real panel entries.
//
// The Rust lane is the behaviour authority; this suite pins only the surface:
// Insert ▸ Block Definition carries ATTDEF/ATTSYNC/BATTMAN/BATTORDER and
// View ▸ Display carries ATTDISP, each command is catalogued once with real
// label/hint keys in both locales, and the typed line is admitted locally.
import test from 'node:test';
import assert from 'node:assert/strict';
import { RIBBON_TABS } from '../src/ribbon/tabs.ts';
import { ribbonCommandIds } from '../src/ribbon/tabs.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const BLOCK_PANEL = ['ATTDEF', 'ATTSYNC', 'BATTMAN', 'BATTORDER'];
const DISPLAY_PANEL = ['ATTDISP'];
const DOC = { documentLabel: 'drawing.dwg', loading: false };

function panel(tabId: string, panelId: string) {
  const tab = RIBBON_TABS.find(entry => entry.id === tabId);
  assert.ok(tab, `missing ${tabId}`);
  const found = tab.panels.find(entry => entry.id === panelId);
  assert.ok(found, `missing ${tabId}/${panelId}`);
  return found;
}

function labelSlug(id: string): string {
  return `draw.kind.${id.toLowerCase()}`;
}

test('Insert ▸ Block Definition carries the four attribute commands', () => {
  const block = panel('insert', 'block_definition');
  for (const id of BLOCK_PANEL) {
    const item = block.items.find(entry => entry.id === id);
    assert.ok(item, `block_definition must carry ${id}`);
    assert.equal(item.command, id);
    assert.equal(item.action, undefined, `${id} must use the command funnel`);
    assert.ok(ribbonCommandIds().includes(id), `${id} must reach the ribbon`);
  }
});

test('View ▸ Display carries ATTDISP as a session display setting', () => {
  const display = panel('view', 'display');
  const item = display.items.find(entry => entry.id === 'ATTDISP');
  assert.ok(item, 'display must carry ATTDISP');
  assert.equal(item.command, 'ATTDISP');
  assert.equal(item.hintKey, 'draw.kind.attdisp.hint');
});

test('each wired attribute command is catalogued once with both hint locales', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const id of [...BLOCK_PANEL, ...DISPLAY_PANEL]) {
    const rows = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(rows.length, 1, `${id} must be catalogued once`);
    assert.equal(rows[0].labelKey, labelSlug(id), `${id} label key`);
    assert.equal(typeof en[`${labelSlug(id)}.hint`], 'string', `en hint ${id}`);
    assert.equal(typeof zh[`${labelSlug(id)}.hint`], 'string', `zh hint ${id}`);
  }
});

test('the typed attribute lines are admitted locally', () => {
  const lines = [
    'ATTDEF TITLE ROOM A-101 POSITION 1 2 HEIGHT 2.5',
    '-ATTDEF TITLE ROOM A-101',
    'ATTREDEF TITLE FLOOR 1 CONSTANT',
    'ATTEDIT TITLE ROOM B',
    '-ATTEDIT TITLE ROOM B',
    'ATTIPEDIT TITLE ROOM B',
    'GATTE TITLE ROOM B',
    'EATTEDIT TITLE ROOM=B FLOOR=2 POSITION 1 2',
    'ATTSYNC TITLE',
    'BATTORDER TITLE ROOM,FLOOR',
    'ATTDISP NORMAL',
  ];
  for (const line of lines) {
    const admission = admitCommandText(line, { status: DOC });
    assert.equal(admission.kind, 'admitted', `${line}: ${JSON.stringify(admission)}`);
  }
  // A malformed definition line never reaches the router.
  assert.equal(admitCommandText('ATTDEF TITLE', { status: DOC }).kind, 'unsupported');
  assert.equal(admitCommandText('ATTDISP MAYBE', { status: DOC }).kind, 'unsupported');
});

test('attribute tags and values keep their typed spelling', () => {
  const admission = admitCommandText('ATTDEF TITLE Room "A 101"', { status: DOC });
  assert.equal(admission.kind, 'admitted');
  assert.match(admission.command, /Room "A 101"$/, admission.command);
  const edit = admitCommandText('ATTEDIT TITLE Room "Ground floor"', { status: DOC });
  assert.equal(edit.kind, 'admitted');
  assert.match(edit.command, /Room "Ground floor"$/, edit.command);
});
