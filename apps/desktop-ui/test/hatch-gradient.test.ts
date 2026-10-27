// HATCH gradient fill controls (pattern type `3`): the four rows the host
// exposes on `cad_set_tool_parameter` and the way the status bar shows them.
//
// Split from `hatch-tool.test.ts` (which covers the whole HATCH surface and is at
// the code-size ceiling): this file owns the gradient-specific contract only.
//
// What it pins, row by row:
//  * the **scope** — the four gradient rows apply while the pattern type is `3`
//    and the pattern rows (name, spacing, angle, double) apply while it is a
//    pattern kind; the reverse of each, never a disappearing control;
//  * the **kind** — a named chooser of exactly `LINEAR` and `SPHERICAL`
//    (the domain's `CYLINDER` spelling bakes as spherical, so it is not offered);
//  * the **angle** — degrees, the same 15° step as the pattern angle, no wrap;
//  * the **stops** — integer ACI indices `1..=255` on the wire (never a string, never
//    a hex value), ACI `0` never offered because the command refuses ByBlock;
//  * the copy — every label, hint and name in both catalogs, placeholders included.
//
// The assertions are pure-data and source-text checks: there is no mounted DOM,
// no pixel comparison and no real window behind them, so they prove the wiring
// and the wording, not the rendered appearance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { setToolParameterNumber, toolParameterViews } from '../src/tools/tool-parameters.ts';
import { TOOL_PARAMETER_SPECS } from '../src/tools/tool-parameter-specs.ts';
import { aciHex } from '../src/color/aci-palette.ts';
import { aciChoice, COLOR_GRID, selectedAciIndex } from '../src/color/color-value.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The ten values the host projects while HATCH runs, with its own defaults
// (`0` solid, `1` ANSI31, gradient kind `0` Linear, angle `0`, stops `1` red and
// `5` blue, spacing `1`, pattern angle `0`, double false, style `0` Normal).
const HATCH_PARAMS = Object.freeze({
  hatchPatternType: 0,
  hatchPattern: 1,
  hatchGradientKind: 0,
  hatchGradientAngle: 0,
  hatchGradientColorA: 1,
  hatchGradientColorB: 5,
  hatchSpacing: 1,
  hatchAngle: 0,
  hatchDouble: false,
  hatchStyle: 0,
});

const status = (params = {}, gate = () => true) => toolParameterViews(
  { tool: { key: 'draw.hatch', points: 0, ready: false, params: { ...HATCH_PARAMS, ...params } } },
  gate,
);
const viewOf = (params, key, gate = () => true) => status(params, gate).find(view => view.key === key);

// --- the gradient fill (pattern type 3) -------------------------------------
// Four rows, all reachable on the wire (`cad_set_tool_parameter`) and only in
// effect while the pattern type is 3: the kind (Linear/Spherical), the angle in
// degrees, and the two ACI stop colours. The stops are integers on the wire, so
// they are shown as swatches over the shared palette — never as free-form colour
// text, because a `#RRGGBB` has no numeric channel to travel on.

const GRADIENT_ROWS = ['hatchGradientKind', 'hatchGradientAngle', 'hatchGradientColorA', 'hatchGradientColorB'];
const PATTERN_ONLY_ROWS = ['hatchSpacing', 'hatchAngle', 'hatchDouble'];

test('type 3 turns the gradient rows on and the pattern rows off, and type 2 the reverse', () => {
  // Under a gradient the four gradient rows apply: the kind's segments are
  // pickable, the angle steps and both swatches are live.
  const gradient = status({ hatchPatternType: 3 });
  for (const key of GRADIENT_ROWS) {
    const row = gradient.find(entry => entry.key === key);
    assert.ok(row, `${key} must be visible under type 3`);
    assert.equal(row.inactive, false, `${key} applies under type 3`);
    assert.equal(row.disabled, false, `${key} is admitted`);
    if (row.kind === 'number' && row.choices) {
      for (const choice of row.choices) assert.equal(choice.disabled, false, `${key}/${choice.value}`);
    }
  }
  // …and every pattern-only row is inert, but still visible and still dispatchable
  // (scope, not capability: the host accepts the write and ignores it).
  for (const key of ['hatchPattern', ...PATTERN_ONLY_ROWS]) {
    const row = gradient.find(entry => entry.key === key);
    assert.ok(row, `${key} stays visible under type 3`);
    assert.equal(row.inactive, true, `${key} is inert under type 3`);
    assert.equal(row.disabled, false, `${key} is not a capability refusal`);
    if (row.kind === 'number' && row.choices) {
      assert.deepEqual(row.choices.map(choice => choice.disabled), row.choices.map(() => true), key);
    }
  }
  // The reverse: under the predefined type the pattern name and the pattern-only
  // rows apply and the whole gradient group is inert.
  const predefined = status({ hatchPatternType: 2 });
  assert.equal(predefined.find(entry => entry.key === 'hatchPattern').inactive, false);
  for (const key of PATTERN_ONLY_ROWS) {
    assert.equal(predefined.find(entry => entry.key === key).inactive, false, key);
  }
  for (const key of GRADIENT_ROWS) {
    const row = predefined.find(entry => entry.key === key);
    assert.ok(row, `${key} stays visible under type 2`);
    assert.equal(row.inactive, true, `${key} is inert under type 2`);
    if (row.kind === 'number' && row.choices) {
      assert.deepEqual(row.choices.map(choice => choice.disabled), row.choices.map(() => true), key);
    }
  }
  // A solid and a user-defined fill use neither group: the gradient rows are inert
  // and so is the pattern name.
  for (const type of [0, 1]) {
    const rows = status({ hatchPatternType: type });
    assert.equal(rows.find(entry => entry.key === 'hatchPattern').inactive, true, `type ${type}`);
    for (const key of GRADIENT_ROWS) assert.equal(rows.find(entry => entry.key === key).inactive, true, `type ${type}/${key}`);
  }
  // An absent or malformed sibling code never claims a scoped row applies — the
  // strip only asserts a scope it can prove from `cad_status`.
  for (const params of [{ hatchPatternType: null }, { hatchPatternType: '3' }, { hatchPatternType: 3.5 }]) {
    const rows = status(params);
    for (const key of [...GRADIENT_ROWS, ...PATTERN_ONLY_ROWS, 'hatchPattern']) {
      assert.equal(rows.find(entry => entry.key === key).inactive, true, `${JSON.stringify(params)}/${key}`);
    }
  }
  // Command admission is independent of scope: a pending command disables
  // everything without changing what is in effect.
  const deniedRows = status({ hatchPatternType: 3 }, () => false);
  for (const key of GRADIENT_ROWS) {
    const row = deniedRows.find(entry => entry.key === key);
    assert.equal(row.disabled, true, key);
    assert.equal(row.inactive, false, key);
    if (row.kind === 'number' && row.choices) {
      assert.deepEqual(row.choices.map(choice => choice.disabled), row.choices.map(() => true), key);
    }
  }
});

test('the gradient kind is a named chooser offering exactly Linear and Spherical', async () => {
  const spec = TOOL_PARAMETER_SPECS.hatchGradientKind;
  // The closed set the host accepts: `0` LINEAR and `1` SPHERICAL — never the
  // domain's third spelling `CYLINDER`, which bakes as spherical.
  assert.deepEqual([...spec.choices], [0, 1]);
  assert.deepEqual(Object.keys(spec.choiceLabelKeys).map(Number).sort(), [0, 1]);
  const row = status({ hatchPatternType: 3 }).find(entry => entry.key === 'hatchGradientKind');
  assert.equal(row.kind, 'number');
  assert.deepEqual(row.choices.map(choice => [choice.value, choice.labelKey]), [
    [0, 'status.tool_parameter.hatch_gradient_kind.linear'],
    [1, 'status.tool_parameter.hatch_gradient_kind.spherical'],
  ]);
  // Both names are on screen at once (no raw 0/1 stepper) and are real copy in both
  // catalogs — names a user reads, not the codes.
  for (const choice of row.choices) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[choice.labelKey], 'string', choice.labelKey);
      assert.ok(locale[choice.labelKey].trim().length > 0, choice.labelKey);
      assert.doesNotMatch(locale[choice.labelKey], /^[0-9]+$/, choice.labelKey);
    }
  }
  assert.equal(zh['status.tool_parameter.hatch_gradient_kind.linear'], '线性');
  assert.equal(zh['status.tool_parameter.hatch_gradient_kind.spherical'], '球面');
  for (const locale of [zh, en]) {
    for (const key of Object.values(spec.choiceLabelKeys)) {
      assert.doesNotMatch(locale[key], /CYLINDER|圆柱/i, `${key} must not offer CYLINDER`);
    }
  }
  // The chosen segment follows the host value; a pick writes the integer code.
  assert.deepEqual(
    status({ hatchPatternType: 3, hatchGradientKind: 1 }).find(entry => entry.key === 'hatchGradientKind')
      .choices.map(choice => choice.selected),
    [false, true],
  );
  for (const [choice, code] of [[row.choices[0], 0], [row.choices[1], 1]]) {
    assert.equal(Number.isSafeInteger(choice.value), true);
    const calls = [];
    assert.deepEqual(
      await setToolParameterNumber(async (...args) => { calls.push(args); return { accepted: true }; }, 'hatchGradientKind', code),
      { kind: 'accepted' },
    );
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'hatchGradientKind', number: code }]]);
  }
});

test('the gradient angle is a degrees control like the hatch angle, and writes the number as typed', async () => {
  const spec = TOOL_PARAMETER_SPECS.hatchGradientAngle;
  // The same increment and the same "any finite value" rule as the pattern angle:
  // the host converts degrees to radians exactly once and wraps nothing.
  assert.equal(spec.step, TOOL_PARAMETER_SPECS.hatchAngle.step);
  assert.equal(spec.step, 15);
  const row = status({ hatchPatternType: 3 }).find(entry => entry.key === 'hatchGradientAngle');
  assert.equal(row.kind, 'number');
  assert.equal(row.choices, null, 'an angle is a plain stepper');
  assert.equal(row.swatch, false);
  assert.equal(row.decrement, -15);
  assert.equal(row.increment, 15);
  // No artificial ceiling: 345 -> 360 is offered, and 1e6 is a value the host takes.
  assert.equal(status({ hatchPatternType: 3, hatchGradientAngle: 345 }).find(entry => entry.key === 'hatchGradientAngle').increment, 360);
  for (const number of [0, 45, -270, 1e6]) {
    const calls = [];
    assert.deepEqual(
      await setToolParameterNumber(async (...args) => { calls.push(args); return { accepted: true }; }, 'hatchGradientAngle', number),
      { kind: 'accepted' },
    );
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'hatchGradientAngle', number }]]);
  }
  // A non-finite projection renders nothing rather than an unexplained angle.
  assert.equal(viewOf({ hatchPatternType: 3, hatchGradientAngle: NaN }, 'hatchGradientAngle'), undefined);
});

test('a gradient colour swatch dispatches the integer ACI index and never a string', async () => {
  const rowA = status({ hatchPatternType: 3 }).find(entry => entry.key === 'hatchGradientColorA');
  const rowB = status({ hatchPatternType: 3 }).find(entry => entry.key === 'hatchGradientColorB');
  // Both stops are swatch rows on the same numeric channel, carrying the host's
  // own defaults (1 red / 5 blue) and no choices of their own.
  for (const [row, value] of [[rowA, 1], [rowB, 5]]) {
    assert.equal(row.kind, 'number');
    assert.equal(row.swatch, true);
    assert.equal(row.choices, null);
    assert.equal(row.value, value);
    assert.equal(Number.isSafeInteger(row.value), true);
  }
  // What the chooser hands back is the colour grammar (`aci:N`); the row's adapter
  // is the shared parser, and only the **index** reaches the wire.
  for (const index of [1, 5, 7, 42, 255]) {
    assert.equal(selectedAciIndex(aciChoice(index)), index, String(index));
    const calls = [];
    const result = await setToolParameterNumber(
      async (...args) => { calls.push(args); return { accepted: true }; },
      'hatchGradientColorA',
      selectedAciIndex(aciChoice(index)),
    );
    assert.deepEqual(result, { kind: 'accepted' });
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'hatchGradientColorA', number: index }]]);
    // The wire really carries a number: no `aci:` prefix, no `#rrggbb`.
    assert.equal(typeof calls[0][1].number, 'number');
    assert.equal(Number.isSafeInteger(calls[0][1].number), true);
  }
  // A string spelling can never reach the channel, whichever one it is.
  for (const value of ['aci:5', '#ff0000', '5', 'ByLayer', 'ByBlock']) {
    assert.equal((await setToolParameterNumber(() => assert.fail(`${value} reached IPC`), 'hatchGradientColorA', value)).kind, 'refused');
  }
  // The control is the shared chooser with no free-form entry and no symbolic
  // entries, so it can neither offer a hex value nor a ByBlock/ByLayer token.
  const controls = await readModule('components/ParameterControls.tsx');
  assert.ok(controls.includes("import { aciChoice, selectedAciIndex } from '../color/color-value.ts';"));
  assert.ok(controls.includes('const index = selectedAciIndex(next);'));
  assert.ok(controls.includes('onChoose(view.key, index);'));
  assert.ok(!controls.includes('onChoose(view.key, next)'));
  assert.ok(controls.includes('symbols={[]}'));
  assert.ok(!controls.includes('custom='));
  // …and it paints the value's own palette colour rather than an invented one.
  assert.equal(aciHex(1), '#ff0000');
  assert.equal(aciHex(5), '#0000ff');
  // The status bar routes a swatch row to its own control and keeps the one step
  // funnel for everything else.
  const statusbar = await readModule('components/StatusBar.tsx');
  assert.ok(statusbar.includes('if (view.swatch) return <ParameterSwatch'));
  assert.ok(statusbar.includes('onChoose={shell.stepToolParameter}'));
});

test('the gradient colour rows never offer ACI 0 (the ByBlock placeholder)', async () => {
  // The channel accepts `0..=255`, but index 0 has no sRGB triple and the command
  // refuses it **by name** — so the UI's own predicate starts at 1.
  for (const key of ['hatchGradientColorA', 'hatchGradientColorB']) {
    const spec = TOOL_PARAMETER_SPECS[key];
    assert.equal(spec.offered(0), false, `${key} must not offer ACI 0`);
    assert.equal(spec.offered(-1), false, key);
    assert.equal(spec.offered(1.5), false, key);
    assert.equal(spec.offered(256), false, key);
    assert.equal(spec.offered(1), true, key);
    assert.equal(spec.offered(9), true, key);
    assert.equal(spec.offered(255), true, key);
  }
  // A projected 0 is not renderable: the host contract would be broken, so no
  // swatch appears rather than a black square standing in for "no colour".
  assert.equal(viewOf({ hatchPatternType: 3, hatchGradientColorA: 0 }, 'hatchGradientColorA'), undefined);
  assert.equal(viewOf({ hatchPatternType: 3, hatchGradientColorB: 0 }, 'hatchGradientColorB'), undefined);
  // The shared palette the chooser paints starts at 1, and `aci:0` parses as the
  // ByBlock token rather than a colour — so the pick translation cannot produce 0.
  assert.equal(COLOR_GRID[0].index, 1);
  assert.equal(COLOR_GRID.some(cell => cell.index === 0), false);
  assert.equal(aciHex(0), null);
  assert.equal(selectedAciIndex('aci:0'), null);
  assert.equal(selectedAciIndex('ByBlock'), null);
  // …and 0 is refused before IPC, so no path can send it.
  for (const key of ['hatchGradientColorA', 'hatchGradientColorB']) {
    assert.equal((await setToolParameterNumber(() => assert.fail('ACI 0 reached IPC'), key, 0)).kind, 'refused', key);
  }
});

test('every gradient row label, hint and choice name exists in both catalogs', () => {
  const keys = [];
  for (const key of GRADIENT_ROWS) keys.push(TOOL_PARAMETER_SPECS[key].labelKey, TOOL_PARAMETER_SPECS[key].hintKey);
  keys.push(...Object.values(TOOL_PARAMETER_SPECS.hatchGradientKind.choiceLabelKeys));
  keys.push('status.tool_parameter.choose_color');
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  // The swatch trigger's name is parameterised by the row's own localized label and
  // by nothing else.
  for (const locale of [zh, en]) {
    assert.deepEqual(
      [...locale['status.tool_parameter.choose_color'].matchAll(placeholder)].map(match => match[1]),
      ['name'],
    );
  }
  // The four labels are four distinct names (no row borrows another's copy) and
  // each hint names its own pattern type in both languages.
  const labels = GRADIENT_ROWS.map(key => TOOL_PARAMETER_SPECS[key].labelKey);
  for (const locale of [zh, en]) {
    assert.equal(new Set(labels.map(key => locale[key])).size, labels.length);
  }
});
