// The 2026-11-09 Wave 7 batch of command-line names.
//
// Thirty names are wired into the chrome so the command line can complete them:
// two **read-only reports** (BVSTATE, -BVSTATE) of each selected INSERT's active
// dynamic-block visibility state and twenty-eight **named refusals** (BACTION
// through DCLINEAR). The Rust lane is the behaviour authority; this suite pins
// only the user-facing surface: one catalog row each, real label and hint keys
// in both catalogs, membership in the host vocabulary mirror, and the "needs an
// open drawing" rule with no capability gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import {
  admitCommandText, isSupportedCommand, setCommandCatalog,
} from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const REPORTS = ['BVSTATE', '-BVSTATE'];

const REFUSALS = [
  'BACTION', 'BACTIONBAR', 'BACTIONSET', 'BACTIONTOOL', 'BASSOCIATE',
  'BAUTHORPALETTE', 'BAUTHORPALETTECLOSE', 'BCONSTRUCTION', 'BCPARAMETER',
  '-BCPARAMETER', 'BPARAMETER', 'BLOOKUPTABLE', 'BTABLE', 'BVHIDE', 'BVSHOW',
  'RESETBLOCK', 'AUTOCONSTRAIN', 'DELCONSTRAINT', 'DIMCONSTRAINT',
  'GEOMCONSTRAINT', 'DCALIGNED', 'DCANGULAR', 'DCCONVERT', 'DCDIAMETER',
  'DCDISPLAY', 'DCFORM', 'DCHORIZONTAL', 'DCLINEAR',
];

const BATCH = [...REPORTS, ...REFUSALS];

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

test('every Wave 7 command is catalogued exactly once with real label and hint keys', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const glyphs = await iconNames();
  for (const id of BATCH) {
    const rows = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(rows.length, 1, `${id} must appear exactly once`);
    const [row] = rows;
    assert.equal(row.labelKey, labelSlug(id), `${id} names its own label key`);
    // An unknown icon name would silently render the `dot` fallback.
    assert.ok(glyphs.has(row.icon), `icons.tsx must define '${row.icon}' for ${id}`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[row.labelKey], 'string', `${locale} missing ${row.labelKey}`);
      assert.equal(typeof catalog[`${row.labelKey}.hint`], 'string', `${locale} missing ${row.labelKey}.hint`);
    }
    assert.equal(HOST_NAMES.includes(id), true, `${id} must be in the vocabulary mirror`);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, `${id} needs an open drawing`);
    // The reports only read and the refusals only answer, so no capability
    // boolean may gate these names.
    assert.equal(CAPABILITY_GATES[id], undefined, `${id} must not carry a capability gate`);
  }
});

test('the two visibility reports state their read-only boundaries', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const id of REPORTS) {
    const key = `${labelSlug(id)}.hint`;
    assert.ok(en[key].length > 20, `en ${key} must say something specific`);
    assert.ok(zh[key].length > 10, `zh ${key} must say something specific`);
    assert.match(en[key], /read-only|session .*token|no-op/i, `${id} must state its boundary`);
    assert.match(en[key], /not modeled|no |not implemented/i, `${id} must state its honest gap`);
  }
});

test('the twenty-eight refusals say they are named refusals and promise no effect', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const id of REFUSALS) {
    const key = `${labelSlug(id)}.hint`;
    assert.match(en[key], /named refusal/i, `${id} hint must say it is a refusal: ${en[key]}`);
    assert.ok(zh[key].length > 10, `zh ${key}`);
  }
});

test('the Wave 7 names are advertised and admitted with a drawing', () => {
  for (const id of BATCH) {
    assert.equal(isSupportedCommand(id), true, `${id} must be in the loaded vocabulary`);
    const admission = admitCommandText(id, { status: DOC });
    assert.deepEqual(admission, { kind: 'admitted', command: id, reasonKey: null }, id);
  }
});

test('the Wave 7 names are disabled on the start page with the no-document sentence', () => {
  for (const id of BATCH) {
    const admission = admitCommandText(id, { status: NO_DOC });
    assert.equal(admission.kind, 'unavailable', id);
    assert.equal(admission.reasonKey, 'plot.no_document', `${id} says a drawing must be open`);
  }
});
