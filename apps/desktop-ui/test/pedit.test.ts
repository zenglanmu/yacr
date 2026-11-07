// PEDIT (AutoCAD 2026) affordance contracts: the declared v1 option subset.
//
// `PEDIT` is one canonical selector whose option is typed (`PEDIT CLOSE|OPEN|
// REVERSE`), so the chrome must:
//   * carry one catalog row and one split button on both Modify panels;
//   * admit the option words through `ARGUMENT_COMMANDS.PEDIT` (never a second
//     name list and never a fabricated `PEDIT CLOSE` vocabulary entry);
//   * gate the command on exactly one selected object with the command's own
//     hint, and let a bare `PEDIT` reach the host's explicit needs-option
//     refusal instead of defaulting to an option locally;
//   * name the NOT IMPLEMENTED AutoCAD options in the hint copy.
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

function modifyPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab`);
  const panel = tab.panels.find(candidate => candidate.id === 'modify');
  assert.ok(panel, `${tabId}/modify panel`);
  return panel;
}

test('PEDIT is one catalog row and a split button on both Modify panels', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'PEDIT');
  assert.ok(entry, 'PEDIT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.pedit');
  assert.equal(entry.icon, 'polyline');
  assert.equal(entry.group, 'modify');

  for (const tabId of ['home', 'draw']) {
    const panel = modifyPanel(tabId);
    const item = panel.items.find(candidate => candidate.id === 'PEDIT');
    assert.ok(item, `${tabId}/modify must carry the PEDIT affordance`);
    assert.equal(item.command, 'PEDIT');
    assert.equal(item.action, undefined, 'PEDIT is a router command, not a shell action');
    assert.ok(Array.isArray(item.variants), `${tabId}: PEDIT must be a split button`);
    assert.deepEqual(
      item.variants.map(variant => variant.id),
      ['PEDIT CLOSE', 'PEDIT OPEN', 'PEDIT REVERSE'],
      tabId,
    );
    assert.deepEqual(
      item.variants.map(variant => variant.command),
      ['PEDIT CLOSE', 'PEDIT OPEN', 'PEDIT REVERSE'],
      tabId,
    );
    // Exactly one PEDIT button per Modify panel.
    assert.equal(panel.items.filter(candidate => candidate.id === 'PEDIT').length, 1, tabId);
    // No unimplemented AutoCAD option has a caret row: a row would promise it.
    for (const variant of item.variants) {
      assert.equal(
        /JOIN|WIDTH|FIT|SPLINE|DECURVE|LTYPE|UNDO|MULTIPLE|VERTEX/i.test(String(variant.command)),
        false,
        `${variant.command} must not be offered`,
      );
    }
  }
});

test('PEDIT option words are argument grammar, not extra command names', () => {
  // Only the base selector is a published name; the three options are grammar.
  assert.equal(HOST_NAMES.includes('PEDIT'), true);
  assert.equal(HOST_NAMES.includes('PEDIT CLOSE'), false);
  assert.equal(isSupportedCommand('PEDIT'), true);

  for (const text of ['PEDIT CLOSE', 'PEDIT OPEN', 'PEDIT REVERSE', 'PEDIT C', 'PEDIT O', 'PEDIT R', 'pedit close']) {
    const admission = admitCommandText(text, { status: loaded });
    assert.equal(admission.kind, 'admitted', text);
    assert.equal(admission.reasonKey, null, text);
  }
  assert.equal(ARGUMENT_COMMANDS.PEDIT.parse('CLOSE'), true);
  assert.equal(ARGUMENT_COMMANDS.PEDIT.parse(' c '), true);
  assert.equal(ARGUMENT_COMMANDS.PEDIT.parse('REVERSE'), true);
  assert.equal(ARGUMENT_COMMANDS.PEDIT.parse('R'), true);
  for (const argument of ['JOIN', 'WIDTH', 'FIT', 'SPLINE', 'DECURVE', 'LTYPE', 'UNDO', 'MULTIPLE', 'VERTEX', 'C O', '']) {
    assert.equal(ARGUMENT_COMMANDS.PEDIT.parse(argument), false, argument);
  }
  for (const argument of ['JOIN', 'WIDTH', 'FIT', 'SPLINE', 'DECURVE', 'LTYPE', 'UNDO', 'MULTIPLE', 'VERTEX', 'C O']) {
    assert.equal(
      admitCommandText(`PEDIT ${argument}`, { status: loaded }).kind,
      'unsupported',
      argument,
    );
  }

  // A bare PEDIT is admitted by base name: the host answers with its explicit
  // `pedit.needs_option` refusal (which lists the implemented options), so the
  // chrome never invents a default option.
  const bare = admitCommandText('PEDIT', { status: loaded });
  assert.equal(bare.kind, 'admitted');
  assert.equal(bare.command, 'PEDIT');

  setCommandCatalog([]);
  assert.equal(isSupportedCommand('PEDIT'), false);
  assert.equal(admitCommandText('PEDIT CLOSE', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('PEDIT is gated on exactly one selected object with its own hint', () => {
  assert.equal(SELECTION_REQUIREMENTS.PEDIT.count, 1);
  assert.equal(SELECTION_REQUIREMENTS.PEDIT.reasonKey, 'draw.kind.pedit.hint');
  assert.equal(
    unavailableReasonKey('PEDIT', { status: { ...loaded, selection: { count: 0 } } }),
    'draw.kind.pedit.hint',
  );
  assert.notEqual(
    unavailableReasonKey('PEDIT', { status: { ...loaded, selection: { count: 1 } } }),
    'draw.kind.pedit.hint',
  );
  // No drawing open is its own sentence, not the selection hint.
  assert.equal(
    unavailableReasonKey('PEDIT', { status: { ...loaded, documentLabel: null, selection: { count: 0 } } }),
    'plot.no_document',
  );
});

test('PEDIT copy names the three implemented options and the unimplemented ones', async () => {
  for (const locale of [await readCatalog('zh-CN'), await readCatalog('en')]) {
    for (const key of [
      'draw.kind.pedit',
      'draw.kind.pedit.hint',
      'draw.kind.pedit.close',
      'draw.kind.pedit.open',
      'draw.kind.pedit.reverse',
    ]) {
      assert.equal(typeof locale[key], 'string', key);
      assert.ok(locale[key].trim().length > 0, key);
    }
  }
  const en = await readCatalog('en');
  assert.match(en['draw.kind.pedit.hint'], /CLOSE/);
  assert.match(en['draw.kind.pedit.hint'], /OPEN/);
  assert.match(en['draw.kind.pedit.hint'], /REVERSE/);
  assert.match(en['draw.kind.pedit.hint'], /Join/);
  assert.match(en['draw.kind.pedit.hint'], /Width/);
  assert.match(en['draw.kind.pedit.hint'], /Spline/);
  assert.match(en['draw.kind.pedit.hint'], /Edit vertex/);
  assert.match(en['draw.kind.pedit.hint'], /not implemented/i);
  assert.match(en['draw.kind.pedit.hint'], /\{command\}/);
});
