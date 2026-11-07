// HATCHGENERATEBOUNDARY (AutoCAD 2026 hatch boundary utility): catalogue row,
// ribbon placement, selection gating, command-line typing and both i18n
// catalogues.
//
// The backend semantics (loop→polyline geometry, bulges, undo, refusals) live in
// `crates/cad-app/src/tests/drawing/hatch_utilities.rs`.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  commandById,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const NO_DOCUMENT = {
  documentLabel: null,
  entityCount: null,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: false,
};
const DOCUMENT = {
  documentLabel: 'fixture.dwg',
  entityCount: 4,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};
const selected = count => ({ ...DOCUMENT, selection: { count } });

function panel(tabId, panelId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `the ${tabId} tab must exist`);
  const found = tab.panels.find(candidate => candidate.id === panelId);
  assert.ok(found, `${tabId} must keep its ${panelId} panel`);
  return found;
}

function assertCatalogued({ id, labelKey, icon, group }) {
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.deepEqual({ ...entry }, { id, labelKey, icon, group });
  // Command-line typing: the canonical name normalizes case-insensitively and
  // resolves back to itself, and the host vocabulary admits it.
  assert.equal(normalizeCommandText(`  ${id.toLowerCase()} `), id);
  assert.equal(commandById(id.toLowerCase()).id, id);
  assert.equal(isSupportedCommand(id), true);
  assert.equal(typeof en[labelKey], 'string');
  assert.equal(typeof zh[labelKey], 'string');
}

function assertRibbonCommand(tabId, panelId, { id, labelKey, icon }) {
  const item = panel(tabId, panelId).items.find(candidate => candidate.id === id);
  assert.ok(item, `${tabId}/${panelId} must carry ${id}`);
  assert.equal(item.command, id);
  assert.equal(item.labelKey, labelKey);
  assert.equal(item.icon, icon);
  assert.equal(item.variants, undefined, `${id} has no options popover`);
  assert.equal(item.action, undefined, `${id} dispatches through the router`);
  return item;
}

// --- HATCHGENERATEBOUNDARY: selected hatch → boundary polylines ---------------

const GENERATE_BOUNDARY = Object.freeze({
  id: 'HATCHGENERATEBOUNDARY',
  labelKey: 'draw.kind.hatchgenerateboundary',
  hintKey: 'draw.kind.hatchgenerateboundary.hint',
  icon: 'hatch',
});

test('HATCHGENERATEBOUNDARY is catalogued and typed as its own selector', () => {
  assertCatalogued({ ...GENERATE_BOUNDARY, group: 'annotate' });
  // Never an alias of HATCH/HATCHPOINT: those create a hatch, this reads one.
  assert.notEqual(GENERATE_BOUNDARY.id, 'HATCH');
  assert.notEqual(GENERATE_BOUNDARY.id, 'HATCHPOINT');
});

test('the Annotate tab carries the hatch-context boundary button only once', () => {
  const item = assertRibbonCommand('annotate', 'annotation', GENERATE_BOUNDARY);
  assert.equal(item.large, undefined, 'it is a secondary hatch action');
  for (const tab of RIBBON_TABS) {
    for (const each of tab.panels) {
      if (tab.id === 'annotate' && each.id === 'annotation') continue;
      assert.equal(
        each.items.some(candidate => candidate.command === GENERATE_BOUNDARY.id),
        false,
        `${tab.id}/${each.id} must not dispatch ${GENERATE_BOUNDARY.id}`,
      );
    }
  }
});

test('HATCHGENERATEBOUNDARY needs at least one selected hatch', () => {
  assert.deepEqual(
    { ...selectionRequirement(GENERATE_BOUNDARY.id) },
    { count: 1, reasonKey: GENERATE_BOUNDARY.hintKey },
  );
  assert.equal(typeof en[GENERATE_BOUNDARY.hintKey], 'string');
  assert.equal(typeof zh[GENERATE_BOUNDARY.hintKey], 'string');
  // The hint must not claim associativity or an interactive pick.
  assert.equal(/associat/i.test(en[GENERATE_BOUNDARY.hintKey]), true);
  assert.equal(/select at least one hatch/i.test(en[GENERATE_BOUNDARY.hintKey]), true);
  assert.equal(admitCommandText(GENERATE_BOUNDARY.id, { status: selected(1) }).kind, 'admitted');
  assert.deepEqual(admitCommandText(GENERATE_BOUNDARY.id, { status: selected(0) }), {
    kind: 'unavailable',
    command: GENERATE_BOUNDARY.id,
    reasonKey: GENERATE_BOUNDARY.hintKey,
  });
  // No drawing open answers with the missing document, not "select a hatch".
  assert.equal(
    admitCommandText(GENERATE_BOUNDARY.id, { status: NO_DOCUMENT }).reasonKey,
    'plot.no_document',
  );
});
