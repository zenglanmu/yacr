// The 2026-11-09 Wave 13 batch: page-setup/conversion reports, table/field
// surfaces and named refusals.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const BATCH = [
  'PAGESETUP', '-PAGESETUP', 'PSETUPIN', '-PSETUPIN', 'PLOTSTYLE', '-PLOTSTYLE',
  'CONVERTPSTYLES', 'CONVERTCTB', 'CONVERTPOLY', 'CONVERT', 'TABLE', '-TABLE',
  'TABLEDIT', 'TABLEEXPORT', 'TABLESTYLE', 'TINSERT', 'MATCHCELL', 'FIELD',
  'RTEXT', 'RTEDIT', 'SPELL', 'SUPERHATCH', 'ALIGNSPACE', 'VPSYNC', 'LAYTRANS',
];
const DOC = { documentLabel: 'drawing.dwg', loading: false };
const NO_DOC = { documentLabel: null, loading: false };
setCommandCatalog(HOST_NAMES);

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map);
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}
function labelSlug(id) { return `draw.kind.${id.startsWith('-') ? `dash${id.slice(1)}` : id}`.toLowerCase(); }

test('every Wave 13 command is catalogued once with real keys', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const glyphs = await iconNames();
  for (const id of BATCH) {
    const rows = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(rows.length, 1, id);
    const [row] = rows;
    assert.equal(row.labelKey, labelSlug(id), id);
    assert.ok(glyphs.has(row.icon), `${id} icon`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[row.labelKey], 'string', `${locale} ${row.labelKey}`);
      assert.equal(typeof catalog[`${row.labelKey}.hint`], 'string', `${locale} ${row.labelKey}.hint`);
    }
    assert.equal(HOST_NAMES.includes(id), true, id);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, id);
    assert.equal(CAPABILITY_GATES[id], undefined, id);
  }
});

test('reports, writes and refusals state their boundaries', async () => {
  const en = await readCatalog('en');
  assert.match(en['draw.kind.convertpoly.hint'], /Read-only report/i);
  for (const id of ['PAGESETUP', '-PAGESETUP']) {
    assert.match(en[`${labelSlug(id)}.hint`], /one transaction\/one undo step/i, id);
  }
  for (const id of ['PSETUPIN', '-PSETUPIN']) {
    assert.match(en[`${labelSlug(id)}.hint`], /same drawing|same-drawing/i, id);
  }
  for (const id of ['PLOTSTYLE', 'FIELD', 'SPELL', 'SUPERHATCH', 'ALIGNSPACE', 'VPSYNC', 'LAYTRANS']) {
    assert.match(en[`${labelSlug(id)}.hint`], /Named refusal/i, id);
  }
  // TABLE / -TABLE / TABLEDIT are implemented writes now: their hints describe
  // the real one-transaction behaviour, not a refusal.
  for (const id of ['TABLE', '-TABLE']) {
    assert.match(en[`${labelSlug(id)}.hint`], /one transaction\s*\/\s*one undo step/i, id);
    assert.doesNotMatch(en[`${labelSlug(id)}.hint`], /Named refusal/i, id);
  }
  assert.match(en['draw.kind.tabledit.hint'], /1-based/i);
  assert.match(en['draw.kind.tabledit.hint'], /one transaction\s*\/\s*one undo step/i);
  assert.doesNotMatch(en['draw.kind.tabledit.hint'], /Named refusal/i);
});

test('the Wave 13 names are admitted with a drawing and refused without one', () => {
  for (const id of BATCH) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.deepEqual(admitCommandText(id, { status: DOC }), { kind: 'admitted', command: id, reasonKey: null }, id);
    const noDoc = admitCommandText(id, { status: NO_DOC });
    assert.equal(noDoc.kind, 'unavailable', id);
    assert.equal(noDoc.reasonKey, 'plot.no_document', id);
  }
});
