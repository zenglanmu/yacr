// The 2026-11-09 Wave 9 batch of command-line names.
//
// Twenty-five names are wired into the chrome: eleven Work-only commands
// (`ARRAY`/`-ARRAY`/`ARRAYCLASSIC` routers, `BLEND`, `BREAKLINE` and the six
// command-line aliases) and fourteen Viewer-legal reports/refusals. The Rust
// lane is the behaviour authority; this suite pins only the user-facing
// surface: one catalog row each, real label and hint keys in both catalogs,
// vocabulary membership, the "needs an open drawing" rule, and no capability
// gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import {
  admitCommandText, isSupportedCommand, setCommandCatalog,
} from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const WORK = [
  'ARRAY', '-ARRAY', 'ARRAYCLASSIC', 'BLEND', 'BREAKLINE',
  '-BLOCK', '-BOUNDARY', '-INSERT', '-HATCH', '-HATCHEDIT', 'EXOFFSET',
];

const READ_ONLY = [
  'ARCTEXT', 'MLEADERALIGN', 'MLEADEREDIT', 'MLEADERCOLLECT', 'TOLERANCE',
  'DIM', 'DIMARC', 'CAL', 'DIMEX', 'DIMIM', 'SPACETRANS', 'TXTEXP',
  'UPDATEFIELD', 'UNITS',
];

const BATCH = [...WORK, ...READ_ONLY];

const DOC = { documentLabel: 'drawing.dwg', loading: false };
const NO_DOC = { documentLabel: null, loading: false };

setCommandCatalog(HOST_NAMES);

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

/** The mechanical label slug; the dash-prefixed selector gets a `dash` prefix. */
function labelSlug(id) {
  return `draw.kind.${id.startsWith('-') ? `dash${id.slice(1)}` : id}`.toLowerCase();
}

test('every Wave 9 command is catalogued exactly once with real label and hint keys', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const glyphs = await iconNames();
  for (const id of BATCH) {
    const rows = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(rows.length, 1, `${id} must appear exactly once`);
    const [row] = rows;
    assert.equal(row.labelKey, labelSlug(id), `${id} names its own label key`);
    assert.ok(glyphs.has(row.icon), `icons.tsx must define '${row.icon}' for ${id}`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[row.labelKey], 'string', `${locale} missing ${row.labelKey}`);
      assert.equal(typeof catalog[`${row.labelKey}.hint`], 'string', `${locale} missing ${row.labelKey}.hint`);
    }
    assert.equal(HOST_NAMES.includes(id), true, `${id} must be in the vocabulary mirror`);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, `${id} needs an open drawing`);
    assert.equal(CAPABILITY_GATES[id], undefined, `${id} must not carry a capability gate`);
  }
});

test('the real 2D edits state their declared boundaries', async () => {
  const en = await readCatalog('en');
  for (const id of ['BLEND', 'BREAKLINE']) {
    const key = `${labelSlug(id)}.hint`;
    assert.match(en[key], /Real 2D edit/i, `${id} must say it is a real edit`);
    assert.match(en[key], /declared rule/i, `${id} must name its declared rule`);
  }
  assert.match(en['draw.kind.array.hint'], /ARRAYRECT\/ARRAYPOLAR/);
  assert.match(en['draw.kind.exoffset.hint'], /OFFSET planner/);
});

test('the reports and refusals promise no drawing effect', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const id of READ_ONLY) {
    const key = `${labelSlug(id)}.hint`;
    assert.match(en[key], /Named refusal|Read-only report/i, `${id} hint must be honest: ${en[key]}`);
    assert.ok(zh[key].length > 8, `zh ${key}`);
  }
});

test('the Wave 9 names are advertised and admitted with a drawing', () => {
  for (const id of BATCH) {
    assert.equal(isSupportedCommand(id), true, `${id} must be in the loaded vocabulary`);
    const admission = admitCommandText(id, { status: DOC });
    assert.deepEqual(admission, { kind: 'admitted', command: id, reasonKey: null }, id);
  }
});

test('the Wave 9 names are disabled on the start page with the no-document sentence', () => {
  for (const id of BATCH) {
    const admission = admitCommandText(id, { status: NO_DOC });
    assert.equal(admission.kind, 'unavailable', id);
    assert.equal(admission.reasonKey, 'plot.no_document', `${id} says a drawing must be open`);
  }
});
