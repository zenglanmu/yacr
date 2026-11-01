// XLINE / RAY ("construction line" / "ray") user-facing surface contract.
//
// Both commands are **two-point** Work-only captures — a base point and a
// through point — and both commit **one finite bounded segment** clipped to the
// model bounds. The entity keeps its construction type key (`AcDbXline` /
// `AcDbRay`), but there is no infinite geometry in this build, so every string
// asserted here has to say so: the labels name the type (so a reader can map the
// button onto the AutoCAD name), the hints say the drawn span is finite, and the
// ready copy says a **span** was captured. Neither command takes a parameter and
// neither is a pre-selection tool (the two picks are the input).
//
// The buttons are small, sit directly after ARC on both draw panels and keep the
// shared `line` family glyphs (`xline` / `ray`), so no shipped row moves. Every
// user-visible string comes from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const CONSTRUCTION = [
  {
    id: 'XLINE',
    labelKey: 'draw.kind.xline',
    hintKey: 'draw.kind.xline.hint',
    icon: 'xline',
    toolKey: 'draw.xline',
    toolLabelKey: 'tool.draw.xline',
    base: 'tool.prompt.xline.base',
    through: 'tool.prompt.xline.through',
    ready: 'tool.prompt.xline.ready',
  },
  {
    id: 'RAY',
    labelKey: 'draw.kind.ray',
    hintKey: 'draw.kind.ray.hint',
    icon: 'ray',
    toolKey: 'draw.ray',
    toolLabelKey: 'tool.draw.ray',
    base: 'tool.prompt.ray.base',
    through: 'tool.prompt.ray.through',
    ready: 'tool.prompt.ray.ready',
  },
];

const drawPanel = tabId => {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(entry => entry.id === 'draw');
  assert.ok(panel, `${tabId} must keep its draw panel`);
  return panel;
};

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('XLINE and RAY are catalogued draw commands beside ARC, admitted only by the router', async () => {
  const order = COMMAND_CATALOG.map(command => command.id);
  const glyphs = await iconNames();
  for (const entry of CONSTRUCTION) {
    assert.deepEqual(
      { ...COMMAND_CATALOG.find(command => command.id === entry.id) },
      { id: entry.id, labelKey: entry.labelKey, icon: entry.icon, group: 'draw' },
    );
    // A dedicated glyph really exists: an unknown name would silently render the
    // `dot` fallback, which would make the button unreadable.
    assert.ok(glyphs.has(entry.icon), `icons.tsx must define '${entry.icon}'`);
    // The pair stays inside the primitive run, so no shipped row moved.
    assert.ok(order.indexOf(entry.id) > order.indexOf('ARC'), `${entry.id} follows ARC`);
    assert.ok(order.indexOf(entry.id) < order.indexOf('PLINE'), `${entry.id} precedes PLINE`);
  }
  assert.equal(order.indexOf('XLINE'), order.indexOf('ARC') + 1);
  assert.equal(order.indexOf('RAY'), order.indexOf('XLINE') + 1);
  // The mirror stands in for the loaded host vocabulary in these suites; the Rust
  // lane adds the same selectors in parallel.
  for (const entry of CONSTRUCTION) assert.ok(HOST_NAMES.includes(entry.id), `HOST_NAMES must mirror ${entry.id}`);
});

test('both draw panels carry one small XLINE / RAY button directly after ARC', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    for (const entry of CONSTRUCTION) {
      const at = items.findIndex(item => item.id === entry.id);
      assert.ok(at !== -1, `${tabId}/draw must carry the ${entry.id} button`);
      assert.equal(items.filter(item => item.id === entry.id).length, 1, `${tabId}: one ${entry.id} button`);

      const item = items[at];
      assert.equal(item.command, entry.id);
      assert.equal(item.labelKey, entry.labelKey);
      assert.equal(item.icon, entry.icon);
      // The tooltip is the item's own honest limit: the span it draws is finite.
      assert.equal(item.hintKey, entry.hintKey, `${tabId}: ${entry.id} states the finite span`);
      // Small buttons like the other secondary draw tools: no caret (there is no
      // options sub-prompt and no parameter) and never a shell action.
      assert.equal(item.large, undefined, `${tabId}: ${entry.id} is not a primary draw button`);
      assert.equal(item.variants, undefined, `${tabId}: ${entry.id} has no variant popover`);
      assert.equal(item.action, undefined, `${tabId}: ${entry.id} must be a router command`);
    }
    // The pair reads ARC -> XLINE -> RAY, in that order, on both panels.
    const arc = items.findIndex(item => item.id === 'ARC');
    assert.deepEqual(items.slice(arc, arc + 3).map(item => item.id), ['ARC', 'XLINE', 'RAY'], tabId);
    // The draw tail that other suites pin is untouched.
    assert.deepEqual(items.slice(-2).map(item => item.id), ['HATCH', 'HATCHPOINT'], tabId);
  }
  // No other panel dispatches either selector.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      for (const entry of CONSTRUCTION) {
        assert.equal(panel.items.some(item => item.command === entry.id), false, `${tab.id}/${panel.id}/${entry.id}`);
      }
    }
  }
  const dispatched = new Set(ribbonCommandIds());
  for (const entry of CONSTRUCTION) assert.ok(dispatched.has(entry.id), `${entry.id} reaches the ribbon dispatcher`);
});

test('the chip names the base pick, the through pick, then the captured span', () => {
  for (const entry of CONSTRUCTION) {
    // Exactly two named picks: the capture is a two-point one, and the ready
    // prompt is not positional, so it lives in the parallel ready table.
    assert.deepEqual(TOOL_STEP_PROMPTS[entry.toolKey], [entry.base, entry.through]);
    assert.equal(TOOL_READY_PROMPTS[entry.toolKey], entry.ready);
    // It takes picks, so it is deliberately absent from the pre-selection table
    // (which is HATCH's zero-pick shape).
    assert.equal(PRESELECT_PROMPTS[entry.toolKey], undefined);

    // No point yet: ask for the base. The host's `ready` flag does not change
    // that — for a pick tool the count decides the step.
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 0 }), entry.base);
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 0, ready: true }), entry.base);
    // One point: the through pick carries the direction.
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 1 }), entry.through);
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 1, ready: false }), entry.through);
    // Both points: the span is captured and Enter creates it.
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 2 }), entry.ready);
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: 3 }), entry.ready);
    // A malformed count is read as "no points yet", never as a later step.
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: '1' }), entry.base);
    // It never borrows the dimension ready copy (nothing is dimensioned).
    assert.notEqual(entry.ready, 'tool.prompt.dim.ready');
    assert.notEqual(entry.ready, 'tool.prompt.dim.center_ready');
  }
});

test('neither construction capture takes a parameter', () => {
  for (const entry of CONSTRUCTION) {
    // No tool-parameter row means no parameter strip: two picks are the whole
    // input, exactly as the button's missing caret says.
    assert.equal(TOOL_PARAMETERS[entry.toolKey], undefined, entry.toolKey);
  }
});

test('both locales carry the construction copy and say the drawn span is finite', () => {
  for (const entry of CONSTRUCTION) {
    for (const key of [entry.labelKey, entry.hintKey, entry.toolLabelKey, entry.base, entry.through, entry.ready]) {
      for (const locale of [zh, en]) {
        assert.equal(typeof locale[key], 'string', `missing ${key}`);
        assert.ok(locale[key].trim().length > 0, `empty ${key}`);
      }
    }
  }

  // English: the label names the type (so the AutoCAD name is recognisable), the
  // tool chip is the short name, and the three prompts are the two picks plus the
  // captured span.
  assert.equal(en['draw.kind.xline'], 'Construction line (XLINE)');
  assert.equal(en['draw.kind.ray'], 'Ray (RAY)');
  assert.equal(en['tool.draw.xline'], 'Construction line');
  assert.equal(en['tool.draw.ray'], 'Ray');
  assert.equal(en['tool.prompt.xline.base'], 'Specify a point on the construction line');
  assert.equal(en['tool.prompt.xline.through'], 'Specify a second point defining its direction');
  assert.equal(en['tool.prompt.xline.ready'], 'Construction line span captured; press Enter to create');
  assert.equal(en['tool.prompt.ray.base'], 'Specify the ray origin');
  assert.equal(en['tool.prompt.ray.through'], 'Specify a point along the ray');
  assert.equal(en['tool.prompt.ray.ready'], 'Ray span captured; press Enter to create');

  // Chinese mirrors the same eight facts, with the machine name in the label.
  assert.equal(zh['draw.kind.xline'], '构造线（XLINE）');
  assert.equal(zh['draw.kind.ray'], '射线（RAY）');
  assert.equal(zh['tool.draw.xline'], '构造线');
  assert.equal(zh['tool.draw.ray'], '射线');
  assert.equal(zh['tool.prompt.xline.base'], '指定构造线上的点');
  assert.equal(zh['tool.prompt.xline.through'], '指定定义方向的第二点');
  assert.equal(zh['tool.prompt.xline.ready'], '已捕获构造线范围；按回车创建');
  assert.equal(zh['tool.prompt.ray.base'], '指定射线起点');
  assert.equal(zh['tool.prompt.ray.through'], '指定射线方向上的点');
  assert.equal(zh['tool.prompt.ray.ready'], '已捕获射线范围；按回车创建');

  // The honest boundary: neither tooltip may claim unbounded geometry, and each
  // says the segment is finite and clipped to the model bounds.
  for (const entry of CONSTRUCTION) {
    assert.match(en[entry.hintKey], /finite/i);
    assert.match(en[entry.hintKey], /clipped to the model bounds/i);
    assert.match(en[entry.hintKey], /no infinite geometry/i);
    assert.doesNotMatch(en[entry.hintKey], /infinite line|unbounded/i);
    assert.match(zh[entry.hintKey], /有限线段/);
    assert.match(zh[entry.hintKey], /裁剪到模型边界/);
    assert.match(zh[entry.hintKey], /不会产生无限长几何/);
  }
  // The ready copy names a captured **span** and Enter, never a defined entity
  // type the build does not draw.
  for (const entry of CONSTRUCTION) {
    assert.match(en[entry.ready], /span captured/i);
    assert.match(en[entry.ready], /Enter/);
    assert.match(zh[entry.ready], /已捕获/);
    assert.match(zh[entry.ready], /回车/);
  }
});
