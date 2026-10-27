// TEXT / MTEXT typed-parameter contract (2026-10-28): the two parameters the
// host landed beside the text height.
//
//   * `textRotation` — **degrees**, any finite value, default `0`, owned by both
//     TEXT and MTEXT (the host converts to radians once);
//   * `mtextAttachment` — AutoCAD's attachment code, the closed set `1..=9`,
//     default `1`, owned by **MTEXT only**.
//
// The rows are the host mirror (`cad_app::ToolParameter::validate`), so nothing
// here may widen a bound: the host refuses NaN/∞ rotation and a code outside
// `1..=9` **by name** and never clamps, so the strip offers only what it would
// accept. MTEXT's line spacing, columns, background fill and drawing direction
// are NOT implemented: no control exists for them, and a host that projected one
// anyway would still render nothing.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  NO_TOOL_PARAMETER_DRAFT,
  commitToolParameterDraft,
  setToolParameterNumber,
  stepToolParameterDraft,
  toolParameterViews,
  typeToolParameterDraft,
} from '../src/tools/tool-parameters.ts';
import { TOOL_PARAMETER_SPECS, TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
import { CHOICE_GRID_COLUMNS, choiceGridFocus } from '../src/tools/choice-grid.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const admitted = () => true;
const rows = (status, gate = admitted) => toolParameterViews(status, gate);
const row = (status, key, gate = admitted) => rows(status, gate).find(entry => entry.key === key);

const text = params => ({ tool: { key: 'draw.text', points: 1, ready: false, params } });
const mtext = params => ({ tool: { key: 'draw.mtext', points: 2, ready: false, params } });
const TEXT_VALUES = { textHeight: 2.5, textRotation: 0 };
const MTEXT_VALUES = { ...TEXT_VALUES, mtextAttachment: 1 };

// --- ownership ---------------------------------------------------------------

test('TEXT shows height and rotation but no attachment; MTEXT shows all three', () => {
  assert.deepEqual(rows(text(TEXT_VALUES)).map(entry => entry.key), ['textHeight', 'textRotation']);
  assert.deepEqual(
    rows(mtext(MTEXT_VALUES)).map(entry => entry.key),
    ['textHeight', 'textRotation', 'mtextAttachment'],
  );
  // TEXT has no attachment code at all — a host that projected one anyway renders
  // no row for it, because the tool map is the ownership authority.
  assert.deepEqual(
    rows(text({ ...TEXT_VALUES, mtextAttachment: 5 })).map(entry => entry.key),
    ['textHeight', 'textRotation'],
  );
  // An MTEXT payload without a projected attachment renders the two shared rows
  // and no invented default.
  assert.deepEqual(
    rows(mtext(TEXT_VALUES)).map(entry => entry.key),
    ['textHeight', 'textRotation'],
  );
  // …and a payload with no projected rotation loses that row only.
  assert.deepEqual(
    rows(mtext({ textHeight: 2.5, mtextAttachment: 1 })).map(entry => entry.key),
    ['textHeight', 'mtextAttachment'],
  );
  // The two shared rows are the same rows on both tools: one height, one rotation,
  // so TEXT and MTEXT can never disagree about them.
  for (const key of ['textHeight', 'textRotation']) {
    const onText = row(text(TEXT_VALUES), key);
    const onMtext = row(mtext(MTEXT_VALUES), key);
    assert.equal(onText.key, key);
    assert.equal(onMtext.key, key);
    assert.equal(onText.value, onMtext.value, key);
    assert.deepEqual(onText.choices, null, `${key} is a plain row`);
  }
  // The attachment is the only grid row; the shared rows are neither choosers nor
  // grids (the rotation is a typed angle, the height a typed length).
  assert.equal(row(mtext(MTEXT_VALUES), 'textRotation').typed, true);
  assert.equal(row(mtext(MTEXT_VALUES), 'textRotation').choiceLayout, null);
  assert.equal(row(mtext(MTEXT_VALUES), 'textHeight').choiceLayout, null);
  assert.equal(row(mtext(MTEXT_VALUES), 'mtextAttachment').choiceLayout, 'grid');
});

// --- the rotation (degrees) --------------------------------------------------

test('rotation is degrees: any finite value, typed or stepped, never wrapped', () => {
  const spec = TOOL_PARAMETER_SPECS.textRotation;
  assert.equal(spec.kind, 'number');
  assert.equal(spec.statusKey, 'textRotation');
  // The same angle convention as the other degree rows: a 15° step …
  assert.equal(spec.step, 15);
  assert.equal(spec.step, TOOL_PARAMETER_SPECS.hatchAngle.step);
  // … and the host's own predicate: finiteness, with no wrap and no turn limit.
  for (const degrees of [0, -30, 360, 720, -1e6, 0.5, 12, 22.5]) {
    assert.equal(spec.offered(degrees), true, String(degrees));
  }
  for (const refused of [NaN, Infinity, -Infinity, '0', null, undefined]) {
    assert.equal(spec.offered(refused), false, String(refused));
  }

  const zero = row(text(TEXT_VALUES), 'textRotation');
  assert.equal(zero.kind, 'number');
  assert.equal(zero.value, 0);
  assert.equal(zero.text, '0');
  assert.equal(zero.typed, true);
  assert.equal(zero.step, 15);
  assert.equal(zero.decrement, -15);
  assert.equal(zero.increment, 15);
  // No bound means both steps stay offered (0 is a valid angle).
  assert.equal(zero.canDecrement, true);
  assert.equal(zero.canIncrement, true);
  // A full turn is a real value, not folded back to 0, and the next step is 375 —
  // offered as-is, because the host never wraps.
  const full = row(text({ textHeight: 2.5, textRotation: 360 }), 'textRotation');
  assert.equal(full.value, 360);
  assert.equal(full.text, '360');
  assert.equal(full.decrement, 345);
  assert.equal(full.increment, 375);
  assert.equal(full.canIncrement, true);
  // A negative angle keeps its sign in the prose and in the targets.
  const negative = row(text({ textHeight: 2.5, textRotation: -30 }), 'textRotation');
  assert.equal(negative.value, -30);
  assert.equal(negative.text, '-30');
  assert.equal(negative.decrement, -45);
  assert.equal(negative.increment, -15);
  // A non-finite projection is a contract violation: no rotation control at all
  // (never a guessed 0), exactly as the host refuses it by name.
  for (const refused of [NaN, Infinity, -Infinity, null, '0', '30']) {
    assert.equal(row(text({ textHeight: 2.5, textRotation: refused }), 'textRotation'), undefined, String(refused));
  }
});

test('a typed rotation is written verbatim, in degrees, through the one IPC', async () => {
  // A rotation no 15° step reaches is typed, and it keeps every digit and sign.
  for (const [raw, degrees] of [['-33.75', -33.75], ['12', 12], ['22.5', 22.5], ['0', 0], ['360', 360], ['1e1', 10]]) {
    assert.deepEqual(
      commitToolParameterDraft('textRotation', typeToolParameterDraft(NO_TOOL_PARAMETER_DRAFT, raw)),
      { kind: 'write', number: degrees, draft: NO_TOOL_PARAMETER_DRAFT },
      raw,
    );
    const calls = [];
    const result = await setToolParameterNumber(
      async (...args) => { calls.push(args); return { accepted: true }; }, 'textRotation', degrees,
    );
    assert.deepEqual(result, { kind: 'accepted' });
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'textRotation', number: degrees }]]);
    assert.equal(typeof calls[0][1].number, 'number');
  }
  // A non-finite or non-numeric entry is refused before any write, and nothing is
  // rewritten to a nearby value.
  for (const raw of ['NaN', 'Infinity', '-Infinity', 'abc', '', '   ', '1..2', '30deg']) {
    const commit = commitToolParameterDraft('textRotation', typeToolParameterDraft(NO_TOOL_PARAMETER_DRAFT, raw));
    assert.equal(commit.kind, 'refused', raw);
    assert.equal(commit.draft.text, raw);
  }
  for (const number of [NaN, Infinity, -Infinity, '30', null, undefined]) {
    assert.equal(
      (await setToolParameterNumber(() => assert.fail('a refused rotation reached IPC'), 'textRotation', number)).kind,
      'refused',
      String(number),
    );
  }
  // The arrow keys step from the typed value, never off half-typed text, and a
  // step is a plain ±15° (no clamping into a turn).
  assert.equal(stepToolParameterDraft('textRotation', NO_TOOL_PARAMETER_DRAFT, 0, 1), 15);
  assert.equal(stepToolParameterDraft('textRotation', NO_TOOL_PARAMETER_DRAFT, 0, -1), -15);
  assert.equal(stepToolParameterDraft('textRotation', typeToolParameterDraft(null, '12'), 0, 1), 27);
  assert.equal(stepToolParameterDraft('textRotation', typeToolParameterDraft(null, 'abc'), 30, -1), 15);
});

// --- the MTEXT attachment (the nine codes) -----------------------------------

test('the nine attachment choices map to 1..=9 and dispatch the integer', async () => {
  const spec = TOOL_PARAMETER_SPECS.mtextAttachment;
  assert.equal(spec.kind, 'number');
  assert.equal(spec.statusKey, 'mtextAttachment');
  // The closed set is exactly AutoCAD's nine codes, in code order.
  assert.deepEqual([...spec.choices], [1, 2, 3, 4, 5, 6, 7, 8, 9]);
  assert.deepEqual(Object.keys(spec.choiceLabelKeys).map(Number), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
  // It is the one grid row: nine readable names in the 3×3 arrangement the codes
  // describe, not nine inline segments.
  assert.equal(spec.choiceLayout, 'grid');
  for (const code of [1, 2, 3, 4, 5, 6, 7, 8, 9]) assert.equal(spec.offered(code), true, String(code));
  for (const refused of [0, 10, -1, 5.5, 9.5, NaN, Infinity, '5', null]) {
    assert.equal(spec.offered(refused), false, String(refused));
  }

  // The row stands on the projected code: the default is top-left (`1`), the code
  // MTEXT creation stored before the parameter existed.
  const middle = row(mtext({ ...MTEXT_VALUES, mtextAttachment: 5 }), 'mtextAttachment');
  assert.equal(middle.value, 5);
  assert.equal(middle.text, '5');
  assert.equal(middle.step, 1);
  assert.equal(middle.typed, false);
  assert.equal(middle.choiceLayout, 'grid');
  assert.equal(middle.choices.length, 9);
  assert.deepEqual(middle.choices.map(choice => choice.value), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
  // Exactly one cell is marked, and it is the code the host holds.
  assert.deepEqual(
    middle.choices.map(choice => choice.selected),
    [false, false, false, false, true, false, false, false, false],
  );
  // Each cell carries the catalog key of its own name and no other key: the nine
  // names are distinct, so no two cells can read the same.
  const labelKeys = middle.choices.map(choice => choice.labelKey);
  assert.deepEqual(labelKeys, [1, 2, 3, 4, 5, 6, 7, 8, 9].map(code => spec.choiceLabelKeys[code]));
  assert.equal(new Set(labelKeys).size, 9);
  assert.deepEqual(middle.choices.map(choice => choice.disabled), new Array(9).fill(false));

  // A pick writes exactly the integer code through the one numeric channel: no
  // string, no second IPC, no name on the wire.
  for (const choice of middle.choices) {
    const calls = [];
    const result = await setToolParameterNumber(
      async (...args) => { calls.push(args); return { accepted: true }; }, 'mtextAttachment', choice.value,
    );
    assert.deepEqual(result, { kind: 'accepted' });
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'mtextAttachment', number: choice.value }]]);
    assert.equal(typeof calls[0][1].number, 'number');
    assert.equal(Number.isSafeInteger(calls[0][1].number), true);
  }
  // A pick outside the closed set never reaches IPC (the host refuses it by name).
  for (const code of [0, 10, 5.5, NaN, Infinity]) {
    const result = await setToolParameterNumber(
      () => assert.fail('an unoffered code reached IPC'), 'mtextAttachment', code,
    );
    assert.equal(result.kind, 'refused', String(code));
  }
  // The steps around the closed set are disabled, never clamped: at the top-left
  // code `-` would be 0, at the bottom-right `+` would be 10.
  const first = row(mtext(MTEXT_VALUES), 'mtextAttachment');
  assert.equal(first.decrement, 0);
  assert.equal(first.canDecrement, false);
  assert.equal(first.increment, 2);
  assert.equal(first.canIncrement, true);
  const last = row(mtext({ ...MTEXT_VALUES, mtextAttachment: 9 }), 'mtextAttachment');
  assert.equal(last.increment, 10);
  assert.equal(last.canIncrement, false);
  assert.equal(last.canDecrement, true);
  // Command admission disables every cell, independently of the value.
  const denied = row(mtext(MTEXT_VALUES), 'mtextAttachment', () => false);
  assert.equal(denied.disabled, true);
  assert.deepEqual(denied.choices.map(choice => choice.disabled), new Array(9).fill(true));
});

// --- copy ---------------------------------------------------------------------

test('every new key exists in both catalogs and the hints state the real limits', () => {
  const attachment = TOOL_PARAMETER_SPECS.mtextAttachment;
  const keys = [
    TOOL_PARAMETER_SPECS.textRotation.labelKey,
    TOOL_PARAMETER_SPECS.textRotation.hintKey,
    attachment.labelKey,
    attachment.hintKey,
    ...Object.values(attachment.choiceLabelKeys),
  ];
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
  // The rotation hint says the unit, the default, both owners and the no-wrap rule.
  for (const locale of [zh, en]) {
    const hint = locale[TOOL_PARAMETER_SPECS.textRotation.hintKey];
    assert.match(hint, /TEXT/);
    assert.match(hint, /MTEXT/);
    assert.match(hint, /0/);
  }
  assert.match(en[TOOL_PARAMETER_SPECS.textRotation.hintKey], /degrees?/i);
  assert.match(en[TOOL_PARAMETER_SPECS.textRotation.hintKey], /never wrapped/i);
  assert.match(zh[TOOL_PARAMETER_SPECS.textRotation.hintKey], /度/);
  assert.match(zh[TOOL_PARAMETER_SPECS.textRotation.hintKey], /折返/);
  // The attachment hint says MTEXT only, that TEXT stays single-line at its
  // insertion point, the default, and that the other MTEXT features do not exist.
  assert.match(en[attachment.hintKey], /MTEXT only/i);
  assert.match(en[attachment.hintKey], /TEXT/);
  assert.match(en[attachment.hintKey], /single-line/i);
  assert.match(en[attachment.hintKey], /not implemented/i);
  assert.match(en[attachment.hintKey], /top left/i);
  for (const feature of [/line spacing/i, /columns/i, /background/i, /drawing direction/i]) {
    assert.match(en[attachment.hintKey], feature);
  }
  assert.match(zh[attachment.hintKey], /MTEXT/);
  assert.match(zh[attachment.hintKey], /TEXT/);
  assert.match(zh[attachment.hintKey], /尚未实现/);
  assert.match(zh[attachment.hintKey], /左上/);
  for (const feature of [/行距/, /分栏/, /背景/, /方向/]) {
    assert.match(zh[attachment.hintKey], feature);
  }
  // The nine names are positional words, never digits, and they are translated
  // (the grid is read by name in either language).
  const names = Object.values(attachment.choiceLabelKeys);
  for (const locale of [zh, en]) {
    const values = names.map(key => locale[key]);
    assert.equal(new Set(values).size, 9, 'the nine names must be distinct');
    for (const value of values) assert.doesNotMatch(value, /[0-9]/, value);
  }
  assert.deepEqual(names.map(key => en[key]), [
    'Top left', 'Top centre', 'Top right',
    'Middle left', 'Middle centre', 'Middle right',
    'Bottom left', 'Bottom centre', 'Bottom right',
  ]);
  for (const key of names) assert.notEqual(zh[key], en[key], `${key} is translated`);
});

// --- no invented controls ------------------------------------------------------

test('no control is invented for the MTEXT features that are not implemented', () => {
  // MTEXT owns exactly these three parameters — the attachment did not smuggle a
  // fourth row (spacing, columns, background, direction) into the map.
  assert.deepEqual(TOOL_PARAMETERS['draw.mtext'].keys, ['textHeight', 'textRotation', 'mtextAttachment']);
  assert.deepEqual(TOOL_PARAMETERS['draw.text'].keys, ['textHeight', 'textRotation']);
  assert.equal(Object.keys(TOOL_PARAMETER_SPECS).length, 25);
  // No spec names an unimplemented MTEXT feature.
  for (const key of Object.keys(TOOL_PARAMETER_SPECS)) {
    assert.doesNotMatch(
      key,
      /lineSpacing|lineHeight|columnCount|mtextColumns|background|bgFill|drawingDirection|textDirection|flowDirection/i,
      key,
    );
  }
  // …and a host that projected one anyway still renders nothing for it: a control
  // appears only for a parameter the tool map names.
  const projected = mtext({
    ...MTEXT_VALUES,
    lineSpacing: 1.5,
    columns: 2,
    backgroundFill: 1,
    drawingDirection: 5,
  });
  assert.deepEqual(rows(projected).map(entry => entry.key), ['textHeight', 'textRotation', 'mtextAttachment']);
});

// --- the compact control ------------------------------------------------------

test("the grid chooser reads the view's own choices and moves focus in space", async () => {
  // Geometry of the nine cells: three columns, no wrap across a row edge.
  assert.equal(CHOICE_GRID_COLUMNS, 3);
  assert.equal(choiceGridFocus(4, 'ArrowLeft', 9), 3);
  assert.equal(choiceGridFocus(3, 'ArrowLeft', 9), 3);
  assert.equal(choiceGridFocus(4, 'ArrowRight', 9), 5);
  assert.equal(choiceGridFocus(5, 'ArrowRight', 9), 5);
  assert.equal(choiceGridFocus(1, 'ArrowUp', 9), 1);
  assert.equal(choiceGridFocus(4, 'ArrowUp', 9), 1);
  assert.equal(choiceGridFocus(1, 'ArrowDown', 9), 4);
  assert.equal(choiceGridFocus(7, 'ArrowDown', 9), 7);
  assert.equal(choiceGridFocus(5, 'Home', 9), 0);
  assert.equal(choiceGridFocus(1, 'End', 9), 8);
  // An unusable start begins at the first cell; an empty set has nothing to focus.
  assert.equal(choiceGridFocus(-1, 'ArrowDown', 9), 3);
  assert.equal(choiceGridFocus(4, 'ArrowDown', 0), -1);
  // A partial last row never steps past the count.
  assert.equal(choiceGridFocus(0, 'ArrowDown', 4), 3);
  assert.equal(choiceGridFocus(1, 'ArrowDown', 4), 1);

  // The rows are rendered from the map, and the grid is one component of its own:
  // the segmented chooser keeps the HATCH rows (the default layout).
  const statusbar = await readModule('components/StatusBar.tsx');
  assert.ok(statusbar.includes("if (view.choiceLayout === 'grid') return <ParameterGridChoices"));
  assert.ok(statusbar.includes('if (view.choices) return <ParameterChoices'));
  assert.equal(TOOL_PARAMETER_SPECS.hatchPattern.choiceLayout, undefined);
  assert.equal(TOOL_PARAMETER_SPECS.hatchGradientKind.choiceLayout, undefined);
  // The cells come from the view's own `choices` (never a hard-coded 1..9 table),
  // a pick is the same `onChoose(key, value)` funnel a segment uses, and the panel
  // never talks to the host itself.
  const controls = await readModule('components/ParameterControls.tsx');
  assert.ok(controls.includes('export function ParameterGridChoices'));
  assert.ok(controls.includes('view.choices ?? []'));
  assert.ok(controls.includes('onChoose(view.key, choice.value)'));
  assert.ok(controls.includes("aria-pressed={choice.selected}"));
  assert.ok(controls.includes('choiceGridFocus(focused, event.key, choices.length)'));
  // The panel is the shortcut menu's own in-DOM placement (fixed, no second
  // window) and its Escape is its own, so a tool is never cancelled by it.
  assert.ok(controls.includes('placeContextMenu('));
  assert.ok(controls.includes("if (event.key === 'Escape') {"));
  assert.ok(controls.includes('event.stopPropagation();'));
  assert.ok(!controls.includes('createPortal'));
  assert.ok(!controls.includes('invoke('));
});
