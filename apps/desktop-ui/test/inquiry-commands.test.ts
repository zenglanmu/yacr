// Contract tests for the read-only inquiry commands LIST, STATUS and FS.
//
// These tests pin the desktop chrome's half: the catalog rows and their groups,
// the ribbon placement (LIST with Properties, STATUS with the View info panel,
// no FS button because AutoCAD only has the Express command-line form), the
// selection/document gates, and the bilingual labels/hints.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { DOCUMENT_SCOPED_COMMANDS, SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

function status({ documentLabel = 'fixture.dwg', selectionCount = 0 } = {}) {
  return {
    documentLabel,
    entityCount: 3,
    loading: false,
    canOpen: true,
    canNew: true,
    canSave: true,
    canPlot: true,
    canFit: true,
    selection: { count: selectionCount },
  };
}

test('LIST, STATUS and FS are host vocabulary names with catalog rows', () => {
  for (const name of ['LIST', 'STATUS', 'FS']) {
    assert.ok(HOST_NAMES.includes(name), `${name} must be in the host vocabulary mirror`);
    assert.equal(isSupportedCommand(name), true, `${name} must be admitted`);
  }

  const byId = new Map(COMMAND_CATALOG.map(entry => [entry.id, entry]));
  assert.equal(byId.get('LIST').group, 'annotate');
  assert.equal(byId.get('LIST').icon, 'properties');
  assert.equal(byId.get('STATUS').group, 'view');
  assert.equal(byId.get('STATUS').icon, 'diagnostics');
  // FS is an Express command-line tool: it is catalog/command-line only.
  assert.equal(byId.get('FS').group, 'modify');
  assert.equal(byId.get('FS').icon, 'selectAll');
});

test('LIST needs one selected object and a document; STATUS is document-scoped; FS needs one seed', () => {
  // No drawing: all three answer with the shared "no drawing" sentence.
  const noDocument = status({ documentLabel: null });
  for (const name of ['LIST', 'STATUS', 'FS']) {
    const result = admitCommandText(name, { status: noDocument });
    assert.equal(result.kind, 'unavailable', name);
    assert.equal(result.reasonKey, 'plot.no_document', name);
  }

  assert.ok(DOCUMENT_SCOPED_COMMANDS.includes('STATUS'));
  assert.equal(SELECTION_REQUIREMENTS.LIST.count, 1);
  assert.equal(SELECTION_REQUIREMENTS.FS.count, 1);

  // With a drawing but nothing selected, the two selection-based commands are
  // disabled with their own hint; STATUS stays available.
  const emptySelection = status({ selectionCount: 0 });
  assert.equal(admitCommandText('LIST', { status: emptySelection }).kind, 'unavailable');
  assert.equal(
    unavailableReasonKey('LIST', { status: emptySelection }),
    'draw.kind.list.hint',
  );
  assert.equal(admitCommandText('FS', { status: emptySelection }).kind, 'unavailable');
  assert.equal(unavailableReasonKey('FS', { status: emptySelection }), 'draw.kind.fs.hint');
  assert.equal(admitCommandText('STATUS', { status: emptySelection }).kind, 'admitted');

  // One selected object satisfies both minimums; a second object is still
  // admitted locally for FS, and the host refuses that seed count by name
  // (`fs.selection`) — the table expresses a minimum, not an upper bound.
  const one = status({ selectionCount: 1 });
  assert.equal(admitCommandText('LIST', { status: one }).kind, 'admitted');
  assert.equal(admitCommandText('FS', { status: one }).kind, 'admitted');
  assert.equal(admitCommandText('FS', { status: status({ selectionCount: 2 }) }).kind, 'admitted');
});

test('ribbon: LIST sits with Properties, STATUS with the View info panel, FS has no button', () => {
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  const properties = home.panels.find(panel => panel.id === 'properties');
  assert.deepEqual(properties.items.map(item => item.id), ['PROPERTIES', 'LIST', 'MASSPROP']);
  assert.equal(properties.items.find(item => item.id === 'LIST').command, 'LIST');

  const view = RIBBON_TABS.find(tab => tab.id === 'view');
  const utilities = view.panels.find(panel => panel.id === 'palettes');
  assert.ok(
    utilities.items.some(item => item.id === 'STATUS' && item.command === 'STATUS'),
    'STATUS must be reachable from the View info panel',
  );

  const commands = ribbonCommandIds();
  assert.ok(commands.includes('LIST'));
  assert.ok(commands.includes('STATUS'));
  assert.ok(!commands.includes('FS'), 'AutoCAD has no FS ribbon button; this build matches that');
});

test('both locales carry LIST/STATUS/FS labels and hints', async () => {
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of [
      'draw.kind.list',
      'draw.kind.list.hint',
      'draw.kind.status',
      'draw.kind.status.hint',
      'draw.kind.fs',
      'draw.kind.fs.hint',
    ]) {
      assert.equal(typeof catalog[key], 'string', `${locale}:${key}`);
      assert.ok(catalog[key].length > 0, `${locale}:${key} must not be empty`);
    }
  }
});
