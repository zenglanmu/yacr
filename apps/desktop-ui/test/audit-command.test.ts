// `AUDIT` (AutoCAD 2026) contract: the **detect-only** drawing audit.
//
// The command reports every stored entity that fails the database's own
// validity check. AutoCAD's "corrects some errors" half is NOT IMPLEMENTED, so
// the UI must not promise a repair: the label is just "Audit" and the report
// text (produced in Rust) states that no repair was performed.
//
// This suite pins the catalog row, the File tab/utilities placement, admission,
// the start-page disable and both i18n catalogs. It makes no GUI or host claim:
// the loaded vocabulary is the `HOST_NAMES` mirror, as in the other chrome tests.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
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
};

function fileUtilitiesPanel() {
  const tab = RIBBON_TABS.find(candidate => candidate.id === 'file');
  assert.ok(tab, 'the File tab must exist');
  const panel = tab.panels.find(candidate => candidate.id === 'utilities');
  assert.ok(panel, 'the File tab must carry the utilities panel');
  return panel;
}

test('AUDIT is catalogued exactly once as a payload-free file command', () => {
  const rows = COMMAND_CATALOG.filter(command => command.id === 'AUDIT');
  assert.equal(rows.length, 1, 'AUDIT must appear exactly once in the catalog');
  assert.deepEqual(
    { ...rows[0] },
    { id: 'AUDIT', labelKey: 'audit.title', icon: 'diagnostics', group: 'file' },
    'the catalog row names the audit key and the diagnostics glyph',
  );
});

test('the File tab utilities panel puts AUDIT directly after Diagnostics', () => {
  const panel = fileUtilitiesPanel();
  const ids = panel.items.map(item => item.id);
  // The 2026-11-07 panel split added File info as the diagnostics opener; AUDIT
  // still follows the Diagnostics entry whose report it shares.
  assert.equal(ids[0], 'fileinfo', 'File info opens the utilities panel');
  const diagnosticsIndex = ids.indexOf('diagnostics');
  assert.equal(diagnosticsIndex, 1, 'Diagnostics stays directly after File info');
  assert.equal(ids[diagnosticsIndex + 1], 'audit', 'AUDIT sits directly after Diagnostics');
  assert.equal(ids.filter(id => id === 'audit').length, 1, 'AUDIT appears once');
  // The existing rows keep their order; nothing was reordered to make room. The
  // 2026-11-12 recovery report joins directly after AUDIT (the same read-only
  // report family); RECOVERALL/PURGEAECDATA stay catalog/command-line only.
  assert.deepEqual(ids, ['fileinfo', 'diagnostics', 'audit', 'recover', 'clean', 'cmdline', 'about', 'options', 'plugins']);
  // It is a real router command, not a shell action like the diagnostics panel.
  const audit = panel.items.find(item => item.id === 'audit');
  assert.deepEqual(
    { ...audit },
    { id: 'audit', labelKey: 'audit.title', icon: 'diagnostics', command: 'AUDIT' },
  );
  assert.equal(audit.action, undefined, 'AUDIT reaches the CAD router');
});

test('AUDIT is admitted by the loaded vocabulary and needs an open drawing', () => {
  assert.equal(HOST_NAMES.includes('AUDIT'), true, 'the vocabulary mirror carries AUDIT');
  assert.equal(isSupportedCommand('AUDIT'), true);
  assert.equal(isSupportedCommand(' audit '), true, 'admission is case-insensitive/trimmed');
  assert.equal(admitCommandText('AUDIT', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('AUDIT', { status: loaded }).command, 'AUDIT');
  // A drawing is its whole precondition: the start page disables it with the
  // shared "no drawing is open" reason rather than "unsupported".
  assert.equal(DOCUMENT_SCOPED_COMMANDS.includes('AUDIT'), true);
  const empty = {
    ...loaded,
    documentLabel: null,
    entityCount: null,
    canSave: false,
    canPlot: false,
    canFit: false,
  };
  assert.equal(admitCommandText('AUDIT', { status: empty }).kind, 'unavailable');
  assert.equal(admitCommandText('AUDIT', { status: null }).kind, 'unavailable');
});

test('both catalogs word the audit label', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(zh['audit.title'], '核查');
  assert.equal(en['audit.title'], 'Audit');
  // The key is not shared with the diagnostics panel's title.
  assert.notEqual(zh['audit.title'], zh['diagnostics.title']);
  assert.notEqual(en['audit.title'], en['diagnostics.title']);
});
