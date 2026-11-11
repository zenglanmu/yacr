// The 2026-11-09 Wave 16 batch: image/PDF/DGN underlay commands.
//
// `IMAGE` / `-IMAGE` (read-only report), `IMAGEATTACH`, `IMAGECLIP` and
// `IMAGEADJUST` are real commands now; the remaining names stay named refusals.
// The Rust lane is the behaviour authority; this suite pins the user-facing
// surface: one catalog row each, real label/hint keys in both catalogs, the
// host vocabulary mirror, the "needs an open drawing" rule and no capability
// gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { admitCommandText, isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const IMPLEMENTED = ['IMAGE', '-IMAGE', 'IMAGEATTACH', 'IMAGECLIP', 'IMAGEADJUST', '-IMAGEADJUST'];
const REFUSALS = ['IMAGEQUALITY', 'IMAGEAPP', 'IMAGEASYNCWAIT', 'IMAGEEDIT', 'PDFATTACH', '-PDFATTACH', 'PDFCLIP', 'PDFLAYERS', 'PDFADJUST', 'PDFIMPORT', '-PDFIMPORT', 'PDFSHXTEXT', '-PDFSHXTEXT', 'DGNATTACH', '-DGNATTACH', 'DGNCLIP', 'DGNLAYERS', 'DGNADJUST'];
// The classic image panel is a chrome-local selector now; it still belongs to
// this batch's catalog surface but never reaches the router or needs a document.
const CHROME = ['CLASSICIMAGE'];
const BATCH = [...IMPLEMENTED, ...REFUSALS, ...CHROME];
const DOC = { documentLabel: 'drawing.dwg', loading: false };
const NO_DOC = { documentLabel: null, loading: false };
setCommandCatalog(HOST_NAMES);
async function iconNames() { const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/); assert.ok(map); return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(m => m[1])); }
const slug = id => `draw.kind.${id.startsWith('-') ? `dash${id.slice(1)}` : id}`.toLowerCase();

test('every Wave 16 command is catalogued once with real keys', async () => {
  const zh = await readCatalog('zh-CN'), en = await readCatalog('en'), glyphs = await iconNames();
  for (const id of BATCH) {
    const rows = COMMAND_CATALOG.filter(c => c.id === id); assert.equal(rows.length, 1, id); const [row] = rows;
    assert.equal(row.labelKey, slug(id), id); assert.ok(glyphs.has(row.icon), `${id} icon`);
    for (const [loc, cat] of [['zh-CN', zh], ['en', en]]) { assert.equal(typeof cat[row.labelKey], 'string'); assert.equal(typeof cat[`${row.labelKey}.hint`], 'string'); }
    assert.equal(HOST_NAMES.includes(id), true, id);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), CHROME.includes(id) ? false : true, id);
    assert.equal(CAPABILITY_GATES[id], undefined, id);
    if (REFUSALS.includes(id)) assert.match(en[`${slug(id)}.hint`], /Named refusal/i, id);
    else assert.doesNotMatch(en[`${slug(id)}.hint`], /Named refusal/i, id);
  }
});
test('the Wave 16 names are admitted with a drawing and refused without one', () => {
  for (const id of BATCH) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.deepEqual(admitCommandText(id, { status: DOC }), { kind: 'admitted', command: id, reasonKey: null }, id);
    if (CHROME.includes(id)) {
      assert.equal(admitCommandText(id, { status: NO_DOC }).kind, 'admitted', id);
      continue;
    }
    const noDoc = admitCommandText(id, { status: NO_DOC }); assert.equal(noDoc.kind, 'unavailable', id); assert.equal(noDoc.reasonKey, 'plot.no_document', id);
  }
});
