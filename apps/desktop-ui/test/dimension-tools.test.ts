// Dimension-tool surface contract (ADR 0017 P5c): DIMLINEAR/DIMALIGNED are
// three-pick captures (two extension-line origins, then the dimension-line
// location). The status-bar tool chip names the next expected pick, derived from
// the host's real `tool.key` + captured count — never a local mirror and never a
// fabricated step. Every user-visible string comes from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad.ts';
import { readCatalog, readCss, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const DIMENSION_TOOLS = ['draw.dimlinear', 'draw.dimaligned'];
const SHARED_KEYS = ['tool.prompt.dim.origin_first', 'tool.prompt.dim.origin_second'];

test('the step prompt names each dimension pick from the captured count', () => {
  // Two extension-line origins, then the dimension-line location.
  for (const key of DIMENSION_TOOLS) {
    assert.equal(toolStepPromptKey({ key, points: 0 }), 'tool.prompt.dim.origin_first');
    assert.equal(toolStepPromptKey({ key, points: 1 }), 'tool.prompt.dim.origin_second');
    assert.equal(toolStepPromptKey({ key, points: 2 }), TOOL_STEP_PROMPTS[key][2]);
  }
  // DIMLINEAR's location pick also decides horizontal vs vertical; DIMALIGNED's
  // location does not, so the two location prompts are deliberately different.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimlinear'][2], 'tool.prompt.dim.location_linear');
  assert.equal(TOOL_STEP_PROMPTS['draw.dimaligned'][2], 'tool.prompt.dim.location_aligned');
  assert.notEqual(TOOL_STEP_PROMPTS['draw.dimlinear'][2], TOOL_STEP_PROMPTS['draw.dimaligned'][2]);
  // The two origins are shared copy: both tools ask for the same first/second.
  for (const key of DIMENSION_TOOLS) {
    assert.deepEqual(TOOL_STEP_PROMPTS[key].slice(0, 2), SHARED_KEYS);
  }
});

test('a ready dimension capture names the confirm step, never a stale pick', () => {
  for (const key of DIMENSION_TOOLS) {
    assert.equal(toolStepPromptKey({ key, points: 3 }), 'tool.prompt.dim.ready');
    // An out-of-range count can only mean a ready capture (exact arity of three).
    assert.equal(toolStepPromptKey({ key, points: 9 }), 'tool.prompt.dim.ready');
  }
});

test('tools with no named steps render no prompt at all', () => {
  for (const key of ['draw.line', 'draw.polygon', 'draw.move', 'draw.spline', 'measure.distance']) {
    assert.equal(toolStepPromptKey({ key, points: 1 }), null, key);
  }
  // No tool, a non-object, or a missing key yields nothing (never a guess).
  for (const tool of [null, undefined, {}, { key: 7 }, 'draw.dimlinear']) {
    assert.equal(toolStepPromptKey(tool), null);
  }
  // A missing or malformed count is read as "no points yet", not a later step.
  assert.equal(toolStepPromptKey({ key: 'draw.dimlinear' }), 'tool.prompt.dim.origin_first');
  assert.equal(toolStepPromptKey({ key: 'draw.dimlinear', points: '2' }), 'tool.prompt.dim.origin_first');
  assert.equal(toolStepPromptKey({ key: 'draw.dimlinear', points: -1 }), 'tool.prompt.dim.origin_first');
});

test('every step-prompt key exists in both catalogs with real copy', () => {
  const keys = [
    ...SHARED_KEYS,
    'tool.prompt.dim.location_linear',
    'tool.prompt.dim.location_aligned',
    'tool.prompt.dim.ready',
  ];
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The linear location prompt tells the user the pick sets the direction; the
  // aligned one does not make that claim.
  assert.match(en['tool.prompt.dim.location_linear'], /horizontal or vertical/i);
  assert.doesNotMatch(en['tool.prompt.dim.location_aligned'], /horizontal or vertical/i);
  assert.match(zh['tool.prompt.dim.location_linear'], /水平/);
});

test('the status bar renders the step prompt from the status and localizes it', async () => {
  const statusbar = await readModule('components/StatusBar.tsx');
  // Derived from the real active tool, never a local mirror.
  assert.ok(statusbar.includes('const toolStepKey = tool ? toolStepPromptKey(tool) : null;'));
  // Rendered only when a step exists: no placeholder element for other tools.
  assert.ok(statusbar.includes('{toolStepKey ? ('));
  // Localized through the catalog, never a hardcoded string.
  assert.ok(statusbar.includes('{t(toolStepKey)}'));
  assert.ok(statusbar.includes('className="statusbar__tool-step"'));
  assert.ok(!statusbar.includes('useState'));
});

test('the step prompt is bounded so a long prompt cannot reflow the status bar', async () => {
  const css = await readCss();
  const rule = css.match(/\.statusbar__tool-step\s*\{([^}]*)\}/);
  assert.ok(rule, 'expected a .statusbar__tool-step rule');
  assert.match(rule[1], /text-overflow:\s*ellipsis/);
  assert.match(rule[1], /white-space:\s*nowrap/);
  assert.match(rule[1], /max-width:/);
});
