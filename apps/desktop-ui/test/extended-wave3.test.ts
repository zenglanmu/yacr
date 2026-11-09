// The 2026-11-09 Wave 3 batch of payload-free command-line names.
//
// Thirty names are wired into the chrome so the command line can complete them:
// eight **reports/session** commands (MSPACE, PSPACE, STYLE, -STYLE,
// MLEADERSTYLE, BLOCK?, DIMREASSOC, DIMREASSOCIATE) and twenty-two **named
// refusals** (DIMJOGGED through CONSTRAINTSETTINGS). The Rust lane is the
// behaviour authority; this suite pins only the user-facing surface: one catalog
// row each, real label and hint keys in both catalogs, membership in the host
// vocabulary mirror, and the "needs an open drawing" rule with no capability
// gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import {
  admitCommandText, isSupportedCommand, setCommandCatalog,
} from '../src/commands/commands.ts';
import { CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const BATCH = [
  // Reports / session.
  'MSPACE', 'PSPACE', 'STYLE', '-STYLE', 'MLEADERSTYLE', 'BLOCK?',
  'DIMREASSOC', 'DIMREASSOCIATE',
  // Named refusals.
  'DIMJOGGED', 'DIMINSPECT', '-DIMINSPECT', 'ANNORESET', 'ANNOUPDATE',
  'OBJECTSCALE', '-OBJECTSCALE', 'QLATTACH', 'QLATTACHSET', 'QLDETACHSET',
  'ARRAYCLOSE', 'ARRAYEDIT', 'BEDIT', '-BEDIT', 'BCLOSE', 'BCYCLEORDER',
  'BGRIPSET', 'BESETTINGS', 'BTESTBLOCK', 'LAYERPALETTE', 'CONSTRAINTBAR',
  'CONSTRAINTSETTINGS',
];

// `?` is not a legal i18n key character, so BLOCK? owns the explicit
// `block_list` slug instead of a mechanical lowercase.
const SLUG_OVERRIDES = { 'BLOCK?': 'block_list' };

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
  if (SLUG_OVERRIDES[id]) return `draw.kind.${SLUG_OVERRIDES[id]}`;
  return `draw.kind.${id.startsWith('-') ? `dash${id.slice(1)}` : id}`.toLowerCase();
}

test('every Wave 3 command is catalogued exactly once with real label and hint keys', async () => {
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
    // No file I/O is involved, so no capability boolean may gate these names.
    assert.equal(CAPABILITY_GATES[id], undefined, `${id} must not carry a capability gate`);
  }
});

test('the Wave 3 hints state the honest boundary and never promise an effect', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const id of BATCH) {
    const key = `${labelSlug(id)}.hint`;
    for (const [locale, catalog] of [['en', en], ['zh-CN', zh]]) {
      const hint = catalog[key];
      assert.equal(typeof hint, 'string', `${locale} ${key}`);
      assert.ok(hint.length > 20, `${locale} ${key} must say something specific`);
    }
    const english = en[key];
    assert.match(
      english,
      /read-only|session token|never|named refusal|NOT IMPLEMENTED/i,
      `${id} hint must state its boundary: ${english}`,
    );
  }
});

test('the Wave 3 names are advertised by the host vocabulary and admitted with a drawing', () => {
  for (const id of BATCH) {
    assert.equal(isSupportedCommand(id), true, `${id} must be in the loaded vocabulary`);
    const admission = admitCommandText(id, { status: DOC });
    assert.deepEqual(admission, { kind: 'admitted', command: id, reasonKey: null }, id);
  }
});

test('the Wave 3 names are disabled on the start page with the no-document sentence', () => {
  for (const id of BATCH) {
    const admission = admitCommandText(id, { status: NO_DOC });
    assert.equal(admission.kind, 'unavailable', id);
    assert.equal(admission.reasonKey, 'plot.no_document', `${id} says a drawing must be open`);
  }
});
