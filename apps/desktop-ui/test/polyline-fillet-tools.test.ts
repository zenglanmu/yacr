// FILLET POLYLINE / CHAMFER POLYLINE user-facing surface (2026-10-27).
//
// The two **Polyline modes** of the corner commands, exposed as selectors of
// their own (never aliases of `CHAMFER`/`FILLET`: those keep their two-pick
// capture, and an alias would route a polyline edit into it). Both take
// `CommandPayload::None` — the one preselected polyline is the whole input — so
// they are ordinary router commands whose only local precondition is a selection
// count, not draw tools with a capture.
//
// What this suite pins:
//   * the catalogue rows (beside the two-line pair, reusing its glyphs) and the
//     ribbon buttons that dispatch them;
//   * the one-object selection requirement and the hint key it reads;
//   * the copy: the v1 limits are stated plainly (one open polyline, straight
//     segments, the shared radius / distances, A on the incoming chamfer arm,
//     no automatic radius reduction), and nothing oversells what the host does.
//
// Every user-visible string comes from the shared catalogs; nothing here may
// claim interactive object picking or a TRIMMODE switch.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  commandById,
  filterCommandSuggestions,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  selectionRequirementMet,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import {
  ARGUMENT_COMMANDS,
  CAPABILITY_GATES,
  DOCUMENT_SCOPED_COMMANDS,
  GESTURE_ONLY_COMMANDS,
  LOCAL_CHROME_COMMANDS,
  SELECTION_REQUIREMENTS,
} from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router vocabulary that carries the two names; admission — never a local
// flag — is what enables the affordances. Both ids are one entry each: the host
// canonicalises the whole trimmed line, so the space is part of the name.
setCommandCatalog(HOST_NAMES);

const DOC = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};
const NO_DOCUMENT = { ...DOC, documentLabel: null, entityCount: null };
const selected = count => ({ ...DOC, selection: { count } });

// id → the catalog key pair the button and its hint read.
const MODES = [
  {
    id: 'CHAMFER POLYLINE',
    parent: 'CHAMFER',
    labelKey: 'draw.kind.chamfer_polyline',
    hintKey: 'draw.kind.chamfer_polyline.hint',
    icon: 'chamfer',
  },
  {
    id: 'FILLET POLYLINE',
    parent: 'FILLET',
    labelKey: 'draw.kind.fillet_polyline',
    hintKey: 'draw.kind.fillet_polyline.hint',
    icon: 'fillet',
  },
];

function panel(tabId, panelId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const found = tab.panels.find(entry => entry.id === panelId);
  assert.ok(found, `${tabId} must keep its ${panelId} panel`);
  return found;
}

async function iconNames() {
  const source = await readModule('icons.tsx');
  const map = source.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('both polyline modes are catalogued beside the two-line commands they refine', async () => {
  const icons = await iconNames();
  const order = COMMAND_CATALOG.map(command => command.id);
  for (const mode of MODES) {
    const entry = COMMAND_CATALOG.find(command => command.id === mode.id);
    assert.ok(entry, `${mode.id} must be in the command catalog`);
    assert.deepEqual(
      { ...entry },
      { id: mode.id, labelKey: mode.labelKey, icon: mode.icon, group: 'modify' },
    );
    // Directly after the two-pick pair, and no shipped row moved: ARRAYRECT still
    // opens the array run.
    assert.ok(order.indexOf(mode.id) > order.indexOf(mode.parent), `${mode.id} follows ${mode.parent}`);
    assert.ok(order.indexOf(mode.id) < order.indexOf('ARRAYRECT'), `${mode.id} precedes ARRAYRECT`);
    // The glyph is the parent's own (the operation is the same corner treatment)
    // and it really exists — an unknown name would render the dot fallback.
    assert.equal(entry.icon, COMMAND_CATALOG.find(command => command.id === mode.parent).icon);
    assert.ok(icons.has(entry.icon), `${entry.icon} must be a real glyph`);

    // A multi-word id is one *name* to the catalogue and the lookup helpers.
    assert.equal(normalizeCommandText(`  ${mode.id.toLowerCase()} `), mode.id);
    assert.equal(commandById(mode.id.toLowerCase()).id, mode.id);
    assert.equal(isSupportedCommand(mode.id.toLowerCase()), true);
    // Nothing about it is chrome-local, capability-gated, a canvas gesture, a
    // typed argument or document-scoped: it is an ordinary router command.
    assert.equal(LOCAL_CHROME_COMMANDS.includes(mode.id), false);
    assert.equal(CAPABILITY_GATES[mode.id], undefined);
    assert.equal(GESTURE_ONLY_COMMANDS.includes(mode.id), false);
    assert.equal(ARGUMENT_COMMANDS[mode.id], undefined);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(mode.id), false);
    // The completion list offers it, and only because the host reported it.
    const row = filterCommandSuggestions(mode.id).find(suggestion => suggestion.id === mode.id);
    assert.ok(row, `${mode.id} is offered as a completion row`);
    assert.equal(row.supported, true);
  }
});

test('the catalog and the ribbon offer the same two names, once per modify panel', () => {
  const ids = ribbonCommandIds();
  const known = new Set(COMMAND_CATALOG.map(command => command.id));
  for (const mode of MODES) {
    assert.ok(ids.includes(mode.id), `the ribbon must dispatch ${mode.id}`);
    assert.ok(known.has(mode.id), `${mode.id} must be catalogued`);
    // Both modify panels (Home and Draw) carry it, exactly beside the two-line
    // pair, and nowhere else in the ribbon.
    const carriers = [];
    for (const tabId of ['home', 'draw']) {
      const modify = panel(tabId, 'modify');
      const item = modify.items.find(candidate => candidate.id === mode.id);
      assert.ok(item, `${tabId}/modify must carry ${mode.id}`);
      assert.equal(item.command, mode.id);
      assert.equal(item.labelKey, mode.labelKey);
      assert.equal(item.icon, mode.icon);
      assert.equal(item.action, undefined, `${mode.id} is a router command, not a shell action`);
      assert.equal(item.standardView, undefined, 'it carries no payload');
      assert.equal(item.variants, undefined, 'no options sub-prompt');
      assert.equal(item.large, undefined, 'not a primary affordance');
      const at = modify.items.findIndex(candidate => candidate.id === mode.id);
      const isPair = modify.items.findIndex(candidate => candidate.id === mode.parent);
      assert.ok(isPair !== -1, `${tabId}/modify keeps ${mode.parent}`);
      assert.ok(at > isPair, `${tabId}: ${mode.id} follows ${mode.parent}`);
      carriers.push(`${tabId}:${at}`);
    }
    assert.equal(carriers.length, 2, `${mode.id} appears on exactly the two modify panels`);
    // Exactly one dispatch token, the model's own union contract.
    for (const tab of RIBBON_TABS) {
      for (const section of tab.panels) {
        for (const item of section.items) {
          if (item.id !== mode.id) continue;
          assert.deepEqual(
            [item.command, item.standardView, item.action].filter(token => token !== undefined),
            [mode.id],
          );
        }
      }
    }
  }
  // No third carrier hides in a variant popover.
  const variantCarriers = RIBBON_TABS.flatMap(tab => tab.panels)
    .flatMap(section => section.items)
    .flatMap(item => item.variants ?? [])
    .filter(variant => MODES.some(mode => mode.id === variant.command));
  assert.deepEqual(variantCarriers, []);
});

test('each polyline mode needs exactly one selected entity and words it in its own hint', () => {
  for (const mode of MODES) {
    // The table states a *minimum*; it cannot express "exactly one". A selection
    // of two is therefore let through here and refused by the host by name — the
    // same division of labour every selection rule uses.
    assert.deepEqual({ ...SELECTION_REQUIREMENTS[mode.id] }, { count: 1, reasonKey: mode.hintKey });
    assert.deepEqual({ ...selectionRequirement(mode.id.toLowerCase()) }, {
      count: 1,
      reasonKey: mode.hintKey,
    });
    assert.equal(selectionRequirementMet(mode.id, DOC), false, 'nothing selected');
    assert.equal(selectionRequirementMet(mode.id, selected(0)), false);
    assert.equal(selectionRequirementMet(mode.id, selected(1)), true);
    assert.equal(selectionRequirementMet(mode.id, selected(2)), true, "the upper bound is the host's");
    assert.equal(selectionRequirementMet(mode.id, NO_DOCUMENT), false, 'no drawing, no selection block');

    // One object is admitted, none is refused before IPC with this hint — the same
    // key the disabled ribbon button shows.
    assert.equal(admitCommandText(mode.id, { status: selected(1) }).kind, 'admitted');
    const refused = admitCommandText(mode.id, { status: selected(0) });
    assert.equal(refused.kind, 'unavailable');
    assert.equal(refused.reasonKey, mode.hintKey);
    assert.equal(unavailableReasonKey(mode.id, { status: selected(0) }), mode.hintKey);
    // No drawing: "no drawing is open", never "select more objects".
    assert.equal(unavailableReasonKey(mode.id, { status: NO_DOCUMENT }), 'plot.no_document');
    // The empty host vocabulary is the one honest disable.
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(mode.id), false);
    assert.equal(admitCommandText(mode.id, { status: selected(1) }).kind, 'unsupported');
    setCommandCatalog(HOST_NAMES);
  }
});

test('the hints state the v1 limits plainly, in both languages', () => {
  for (const mode of MODES) {
    for (const key of [mode.labelKey, mode.hintKey]) {
      assert.equal(typeof en[key], 'string', `en missing ${key}`);
      assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
      assert.ok(en[key].trim().length > 0, `en empty ${key}`);
      assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    }
  }
  const enFillet = en['draw.kind.fillet_polyline.hint'];
  const zhFillet = zh['draw.kind.fillet_polyline.hint'];
  // One selected open polyline, straight segments, the shared radius, +Z only.
  assert.match(enFillet, /exactly one selected open polyline/i);
  assert.match(enFillet, /straight/i);
  assert.match(enFillet, /fillet radius/i);
  assert.match(enFillet, /\+Z plane/);
  assert.match(enFillet, /closed polyline/i);
  assert.match(enFillet, /fewer than three vertices/i);
  assert.match(enFillet, /no real corner/i, 'a collinear vertex has no corner to round');
  // The two behaviours that differ from the two-line command are named, not implied.
  assert.match(enFillet, /greater than 0/i, 'zero radius is refused');
  assert.match(enFillet, /never reduced automatically/i, 'no AutoCAD-style auto reduction');
  assert.match(zhFillet, /恰好选中一条/);
  assert.match(zhFillet, /未闭合多段线/);
  assert.match(zhFillet, /直线/);
  assert.match(zhFillet, /圆角半径/);
  assert.match(zhFillet, /\+Z 平面/);
  assert.match(zhFillet, /少于三个顶点/);
  assert.match(zhFillet, /没有真实拐角/);
  assert.match(zhFillet, /大于 0/);
  assert.match(zhFillet, /自动缩小/);

  const enChamfer = en['draw.kind.chamfer_polyline.hint'];
  const zhChamfer = zh['draw.kind.chamfer_polyline.hint'];
  assert.match(enChamfer, /exactly one selected open polyline/i);
  assert.match(enChamfer, /straight/i);
  // The two distances are the shared tool parameters, and A/B are **our** rule.
  assert.match(enChamfer, /chamfer distances/i);
  assert.match(enChamfer, /incoming segment/i);
  assert.match(enChamfer, /outgoing/i);
  assert.match(enChamfer, /\+Z plane/);
  assert.match(enChamfer, /closed polyline/i);
  assert.match(enChamfer, /no real corner/i);
  // Only the corner-only form (A and B both 0) is refused; a single zero is a
  // legal one-sided chamfer, so the copy must not claim otherwise.
  assert.match(enChamfer, /both distances set to 0/i);
  assert.equal(/a distance of 0\b/i.test(enChamfer), false, 'a single 0 distance is legal');
  assert.match(zhChamfer, /恰好选中一条/);
  assert.match(zhChamfer, /未闭合多段线/);
  assert.match(zhChamfer, /倒角距离/);
  assert.match(zhChamfer, /进入的段/);
  assert.match(zhChamfer, /离开的段/);
  assert.match(zhChamfer, /\+Z 平面/);
  assert.match(zhChamfer, /没有真实拐角/);
  assert.match(zhChamfer, /同时为 0/);
  // Nothing may imply a TRIMMODE switch, a radius on the chamfer, or picking.
  for (const hint of [enFillet, enChamfer]) {
    assert.equal(/trim/i.test(hint), false, 'no TRIMMODE claim');
    assert.equal(/interactively/i.test(hint), false, 'no interactive picking claim');
    assert.equal(/preview/i.test(hint), false, 'no preview claim');
  }
  assert.equal(/radius/i.test(enChamfer), false, 'the chamfer takes distances, not a radius');
  // Short labels, not the whole rule.
  assert.equal(en['draw.kind.chamfer_polyline'], 'Chamfer polyline');
  assert.equal(en['draw.kind.fillet_polyline'], 'Fillet polyline');
});

test('the renderer dispatches both names through the one command funnel', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  // No per-button local gate and no hardcoded selector: the model drives them.
  for (const mode of MODES) {
    assert.equal(ribbon.includes(`'${mode.id}'`), false, `${mode.id} must be driven by the model`);
  }
});
