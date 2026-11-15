// STRETCH's crossing-window capture mode (`stretchWindow`): the tool option that
// lets the STRETCH tool capture a polygon (successive vertices) or a lasso
// (freehand drag) window instead of the two-corner rectangle.
//
// The contract pinned here is the UI half only: the chooser offers exactly the
// three implemented codes and names them, the tool's step prompts follow the live
// mode (and an open ring names its own repeatable vertex/drag step), a missing or
// malformed value falls back to the rectangle this tool always captured, and both
// catalogues carry every key. The capture state machine and the command are the
// Rust contracts' (`cad-app`/`cad-host`).
import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOL_PARAMETERS, TOOL_PARAMETER_SPECS } from '../src/tools/tool-parameter-specs.ts';
import { setToolParameterNumber, toolParameterViews } from '../src/tools/tool-parameters.ts';
import { toolStepPromptKey } from '../src/cad/cad.ts';
import { readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const admitted = () => true;
const stretch = (params, points = 0, extra = {}) => ({
  key: 'draw.stretch',
  points,
  ready: false,
  params,
  ...extra,
});

test('the STRETCH tool owns one crossing-window capture-mode chooser', () => {
  assert.deepEqual(TOOL_PARAMETERS['draw.stretch'], {
    command: 'STRETCH',
    keys: ['stretchWindow'],
  });
  const spec = TOOL_PARAMETER_SPECS.stretchWindow;
  assert.equal(spec.kind, 'number');
  assert.equal(spec.statusKey, 'stretchWindow');
  // The closed set is exactly the three implemented capture shapes: the chooser
  // can never offer a code the host would refuse.
  assert.deepEqual([...spec.choices], [0, 1, 2]);
  assert.deepEqual(Object.keys(spec.choiceLabelKeys).map(Number).sort(), [0, 1, 2]);

  // The host projects the value only while a STRETCH capture runs, so the row
  // renders as a named segmented chooser there.
  const views = toolParameterViews({ tool: stretch({ stretchWindow: 1 }) }, admitted);
  assert.equal(views.length, 1);
  const view = views[0];
  assert.equal(view.kind, 'number');
  assert.equal(view.key, 'stretchWindow');
  assert.equal(view.value, 1);
  assert.equal(view.choiceLayout, 'segments');
  assert.deepEqual(
    view.choices.map(choice => [choice.value, choice.labelKey, choice.selected]),
    [
      [0, 'status.tool_parameter.stretch_window.rect', false],
      [1, 'status.tool_parameter.stretch_window.polygon', true],
      [2, 'status.tool_parameter.stretch_window.lasso', false],
    ],
  );
  // A command admission refusal disables the pick without inventing a value.
  const gated = toolParameterViews({ tool: stretch({ stretchWindow: 0 }) }, () => false)[0];
  assert.equal(gated.disabled, true);
  assert.ok(gated.choices.every(choice => choice.disabled));

  // Another tool never gets this row, and an absent/out-of-set value projects
  // nothing (the tool renders no control rather than a fabricated rectangle).
  assert.deepEqual(toolParameterViews({ tool: { key: 'draw.line', points: 0, ready: false, params: {} } }, admitted), []);
  assert.deepEqual(toolParameterViews({ tool: stretch({}) }, admitted), []);
  assert.deepEqual(toolParameterViews({ tool: stretch({ stretchWindow: 3 }) }, admitted), []);
});

test('picking a capture mode dispatches exactly one code through the one IPC', async () => {
  const calls = [];
  const invoke = async (...args) => { calls.push(args); return { accepted: true }; };
  for (const code of [0, 1, 2]) {
    assert.deepEqual(await setToolParameterNumber(invoke, 'stretchWindow', code), { kind: 'accepted' });
  }
  assert.deepEqual(calls, [
    ['cad_set_tool_parameter', { parameter: 'stretchWindow', number: 0 }],
    ['cad_set_tool_parameter', { parameter: 'stretchWindow', number: 1 }],
    ['cad_set_tool_parameter', { parameter: 'stretchWindow', number: 2 }],
  ]);
  // A fourth "shape" is refused before IPC: the host's closed set has three.
  for (const code of [3, -1, 0.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal((await setToolParameterNumber(() => assert.fail('offered'), 'stretchWindow', code)).kind, 'refused');
  }
});

test('the step prompts follow the live capture mode', () => {
  // Rectangle (the default, and the shape a missing/malformed value falls back
  // to): two window corners, then the base and displacement points.
  for (const params of [{ stretchWindow: 0 }, {}, { stretchWindow: 3 }, { stretchWindow: 'polygon' }]) {
    assert.deepEqual(
      [0, 1, 2, 3, 4].map(points => toolStepPromptKey(stretch(params, points))),
      [
        'tool.prompt.stretch.window_first',
        'tool.prompt.stretch.window_second',
        'tool.prompt.stretch.base',
        'tool.prompt.stretch.delta',
        'tool.prompt.stretch.ready',
      ],
      JSON.stringify(params),
    );
  }

  // Polygon: while the ring is open the tool reports itself open-ended and the
  // vertex sentence repeats; once closed, the two displacement picks follow.
  //
  // The ring's vertices are the window step, not captured *points*, so the open
  // phase always reports `points: 0` — the repeated sentence is therefore the same
  // for every vertex count.
  const polygonOpen = stretch({ stretchWindow: 1 }, 0, { openEnded: true });
  assert.equal(toolStepPromptKey(polygonOpen), 'tool.prompt.stretch.polygon_vertex');
  assert.deepEqual(
    [0, 1, 2].map(points => toolStepPromptKey(stretch({ stretchWindow: 1 }, points, { openEnded: false }))),
    ['tool.prompt.stretch.base', 'tool.prompt.stretch.delta', 'tool.prompt.stretch.ready'],
  );

  // Lasso: the ring step is a drag, not a click, and the displacement picks are
  // the same two the polygon form uses (one window planner for both).
  assert.equal(
    toolStepPromptKey(stretch({ stretchWindow: 2 }, 0, { openEnded: true })),
    'tool.prompt.stretch.lasso_trace',
  );
  assert.deepEqual(
    [0, 1, 2].map(points => toolStepPromptKey(stretch({ stretchWindow: 2 }, points, { openEnded: false }))),
    ['tool.prompt.stretch.base', 'tool.prompt.stretch.delta', 'tool.prompt.stretch.ready'],
  );

  // A ready capture is the crossing window itself, never a dimension.
  assert.doesNotMatch(en['tool.prompt.stretch.ready'], /dimension/i);
  assert.doesNotMatch(zh['tool.prompt.stretch.ready'], /标注/);
});

test('both catalogues carry every stretch-window key with real copy', () => {
  const keys = [
    'status.tool_parameter.stretch_window',
    'status.tool_parameter.stretch_window.hint',
    'status.tool_parameter.stretch_window.rect',
    'status.tool_parameter.stretch_window.polygon',
    'status.tool_parameter.stretch_window.lasso',
    'tool.prompt.stretch.window_first',
    'tool.prompt.stretch.window_second',
    'tool.prompt.stretch.polygon_vertex',
    'tool.prompt.stretch.lasso_trace',
    'tool.prompt.stretch.base',
    'tool.prompt.stretch.delta',
    'tool.prompt.stretch.ready',
  ];
  for (const key of keys) {
    for (const catalog of [zh, en]) {
      assert.equal(typeof catalog[key], 'string', key);
      assert.ok(catalog[key].trim().length > 0, `${key} is empty`);
    }
  }
  // The three capture modes are three distinct names, never one string thrice.
  const modes = ['rect', 'polygon', 'lasso'].map(mode => `status.tool_parameter.stretch_window.${mode}`);
  assert.equal(new Set(modes.map(key => en[key])).size, 3);
  assert.equal(new Set(modes.map(key => zh[key])).size, 3);
});
