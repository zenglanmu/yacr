// The 2026-11-09 Wave 10 batch: layer/view/session reports, named refusals and
// five chrome-local selectors.
//
// The Rust lane owns the twenty router names; the five chrome-local names never
// reach the router. This suite pins the user-facing surface for both: one
// catalog row each, real label/hint keys, the right admission table, and the
// catalog's no-document rule only for router names.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import {
  admitCommandText, isSupportedCommand, setCommandCatalog,
} from '../src/commands/commands.ts';
import {
  CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS, LOCAL_CHROME_COMMANDS,
} from '../src/commands/gating.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const ROUTER = [
  '-UNITS', '-LAYER', 'LAYERSTATESAVE', 'NEWVIEW', '-SCALELISTEDIT',
  'LWEIGHT', '-LWEIGHT', 'COLOR', '-COLOR', 'HELP', 'CLASSICINSERT',
  'CLASSICGROUP', 'FILETAB', 'FILETABCLOSE', 'HIDEPALETTES', 'SHOWPALETTES',
  'RIBBONCLOSE', 'GOTOSTART', 'COPYHIST', 'TRANSPARENCY',
];

const CHROME = ['DSETTINGS', 'QUICKPROPERTIES', 'CLASSICLAYER', 'COMMANDLINE', 'COMMANDLINEHIDE'];

const BATCH = [...ROUTER, ...CHROME];

const DOC = { documentLabel: 'drawing.dwg', loading: false };
const NO_DOC = { documentLabel: null, loading: false };

setCommandCatalog(HOST_NAMES);

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

function labelSlug(id) {
  return `draw.kind.${id.startsWith('-') ? `dash${id.slice(1)}` : id}`.toLowerCase();
}

test('every Wave 10 command is catalogued once with real label and hint keys', async () => {
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
    assert.equal(CAPABILITY_GATES[id], undefined, `${id} must not carry a capability gate`);
  }
});

test('the router names are in the vocabulary and need an open drawing', () => {
  for (const id of ROUTER) {
    assert.equal(HOST_NAMES.includes(id), true, `${id} must be in the vocabulary mirror`);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, `${id} needs an open drawing`);
    assert.equal(isSupportedCommand(id), true, `${id} must be supported`);
    assert.deepEqual(admitCommandText(id, { status: DOC }), { kind: 'admitted', command: id, reasonKey: null }, id);
    const noDoc = admitCommandText(id, { status: NO_DOC });
    assert.equal(noDoc.kind, 'unavailable', id);
    assert.equal(noDoc.reasonKey, 'plot.no_document', id);
  }
});

test('the five chrome-local names bypass the router but are admitted', () => {
  for (const id of CHROME) {
    assert.equal(LOCAL_CHROME_COMMANDS.includes(id), true, `${id} must be chrome-local`);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), false, `${id} is not document-scoped`);
    assert.equal(isSupportedCommand(id), true, `${id} must be supported`);
    assert.equal(admitCommandText(id, { status: null }).kind, 'admitted', `${id} works on the start page`);
  }
});

test('the chrome-local selectors open the existing surfaces', () => {
  assert.deepEqual(localSelectorFor('DSETTINGS', 'desktop'), { kind: 'options' });
  assert.deepEqual(localSelectorFor('DSETTINGS', 'mobile'), { kind: 'options' });
  assert.deepEqual(localSelectorFor('QUICKPROPERTIES', 'desktop'), { kind: 'properties' });
  assert.equal(localSelectorFor('QUICKPROPERTIES', 'mobile').kind, 'refused');
  assert.deepEqual(localSelectorFor('CLASSICLAYER', 'desktop'), { kind: 'layers' });
  assert.equal(localSelectorFor('CLASSICLAYER', 'mobile').kind, 'refused');
  assert.deepEqual(localSelectorFor('COMMANDLINE', 'desktop'), { kind: 'commandLine', value: true });
  assert.deepEqual(localSelectorFor('COMMANDLINEHIDE', 'desktop'), { kind: 'commandLine', value: false });
});

test('the reports and refusals state their honest boundaries', async () => {
  const en = await readCatalog('en');
  for (const id of ['HELP', 'CLASSICINSERT', 'CLASSICGROUP', 'FILETAB', 'FILETABCLOSE',
    'HIDEPALETTES', 'SHOWPALETTES', 'RIBBONCLOSE', 'GOTOSTART', 'TRANSPARENCY']) {
    assert.match(en[`${labelSlug(id)}.hint`], /Named refusal/i, `${id} must say it is a refusal`);
  }
  for (const id of ['-LAYER', 'LAYERSTATESAVE', 'NEWVIEW', '-SCALELISTEDIT', 'COPYHIST',
    'LWEIGHT', 'COLOR', '-UNITS']) {
    assert.match(en[`${labelSlug(id)}.hint`], /Read-only|report/i, `${id} must say it is a report: ${en[`${labelSlug(id)}.hint`]}`);
  }
});
