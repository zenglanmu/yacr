// Draw-variant affordance contracts: WIPEOUT and `POLYGON CIRCUMSCRIBED`.
//
// Both are real router selectors with their own machine keys and prompts, so the
// chrome must offer them exactly as the host advertises them — enabled by the
// loaded vocabulary alone, never locally invented — and their catalog copy must
// name what v1 really does (and does not do).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { toolStepPromptKey } from '../src/cad/cad.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

function panel(tabId, panelId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab`);
  const found = tab.panels.find(candidate => candidate.id === panelId);
  assert.ok(found, `${tabId}/${panelId} panel`);
  return found;
}

test('WIPEOUT is a catalog row and an Annotate affordance, admitted only by the router', async () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'WIPEOUT');
  assert.ok(entry, 'WIPEOUT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.wipeout');
  assert.equal(entry.group, 'draw');
  assert.equal(entry.icon, 'hatch');

  // AutoCAD 2026 exposes WIPEOUT on the Annotate tab; the draw panel does not
  // carry a second button for it.
  const annotation = panel('annotate', 'annotation');
  const item = annotation.items.find(candidate => candidate.id === 'WIPEOUT');
  assert.ok(item, 'annotate/annotation must carry the WIPEOUT affordance');
  assert.equal(item.command, 'WIPEOUT');
  assert.equal(item.action, undefined, 'WIPEOUT is a router command, not a shell action');
  assert.equal(item.variants, undefined, 'WIPEOUT has no variant popover');
  assert.equal(
    panel('home', 'draw').items.some(candidate => candidate.id === 'WIPEOUT'),
    false,
    'the Home draw panel must not duplicate the Annotate entry',
  );

  // Router admission is the single enable gate.
  assert.equal(isSupportedCommand('WIPEOUT'), true);
  assert.equal(admitCommandText('WIPEOUT', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('WIPEOUT'), false);
  assert.equal(admitCommandText('WIPEOUT', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);

  // The catalog copy names the v1 limits instead of implying a full AutoCAD
  // wipeout (frames, Polyline/Erase Polyline, self-intersection rejection).
  for (const locale of [await readCatalog('zh-CN'), await readCatalog('en')]) {
    assert.equal(typeof locale['draw.kind.wipeout'], 'string');
    assert.ok(locale['draw.kind.wipeout.hint'].trim().length > 0);
  }
  const en = await readCatalog('en');
  assert.match(en['draw.kind.wipeout.hint'], /Frames/);
  assert.match(en['draw.kind.wipeout.hint'], /Polyline/);
  assert.match(en['draw.kind.wipeout.hint'], /self-intersections/);
});

test('WIPEOUT is an open-ended capture with its own step and ready prompts', async () => {
  // The step prompts are read from the host's `tool.key` + captured count, exactly
  // like every other open-ended tool; `ready` is the host's own flag.
  assert.equal(
    toolStepPromptKey({ key: 'draw.wipeout', points: 0, ready: false }),
    'tool.prompt.wipeout.first',
  );
  assert.equal(
    toolStepPromptKey({ key: 'draw.wipeout', points: 1, ready: false }),
    'tool.prompt.wipeout.next',
  );
  assert.equal(
    toolStepPromptKey({ key: 'draw.wipeout', points: 2, ready: false }),
    'tool.prompt.wipeout.next',
  );
  // Three points make the ring confirmable, but more points stay accepted: the
  // ready sentence says so and never claims a dimension was defined.
  assert.equal(
    toolStepPromptKey({ key: 'draw.wipeout', points: 3, ready: true }),
    'tool.prompt.wipeout.ready',
  );
  assert.equal(
    toolStepPromptKey({ key: 'draw.wipeout', points: 4, ready: true }),
    'tool.prompt.wipeout.ready',
  );

  for (const locale of [await readCatalog('zh-CN'), await readCatalog('en')]) {
    for (const key of [
      'tool.prompt.wipeout.first',
      'tool.prompt.wipeout.next',
      'tool.prompt.wipeout.ready',
    ]) {
      assert.equal(typeof locale[key], 'string', key);
      assert.ok(locale[key].trim().length > 0, key);
    }
  }
});

test('the open-ended prompt table keeps WIPEOUT first-point copy free of the base-pick wording', async () => {
  const source = await readModule('tools/tool-open-ended.ts');
  assert.ok(source.includes("'draw.wipeout': Object.freeze({"));
  assert.ok(source.includes("select: 'tool.prompt.wipeout.first'"));
  assert.ok(source.includes("point: 'tool.prompt.wipeout.next'"));
  // The ready override lives in cad.ts's per-tool table, never in the step list.
  const cad = await readModule('cad/cad.ts');
  assert.ok(cad.includes("'draw.wipeout': 'tool.prompt.wipeout.ready'"));
});
