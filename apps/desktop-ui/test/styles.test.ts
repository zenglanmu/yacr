// The Styles panel exposes exactly the two real VSCURRENT tokens.
//
// Every other SketchUp style spelling is refused by the host with
// `vscurrent.style_not_implemented`; the panel must never offer one.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  DEFAULT_GUIDE_COLOR,
  DEFAULT_GUIDE_DASHED,
  STYLE_TOKENS,
  guideStyleColorCommand,
  guideStyleDashCommand,
  guideStyleHex,
  isStyleToken,
  parseGuideStyleHex,
  styleCommand,
  styleLabelKey,
} from '../src/domains/concept/styles.ts';

test('the style tokens are the two real body display tokens', () => {
  assert.deepEqual([...STYLE_TOKENS], ['WIREFRAME', 'SHADED']);
  assert.equal(isStyleToken('WIREFRAME'), true);
  assert.equal(isStyleToken('SHADED'), true);
  for (const refused of ['REALISTIC', 'CONCEPTUAL', 'XRAY', '', null, 1]) {
    assert.equal(isStyleToken(refused), false, String(refused));
  }
});

test('each token dispatches the existing VSCURRENT selector', () => {
  assert.equal(styleCommand('WIREFRAME'), 'VSCURRENT WIREFRAME');
  assert.equal(styleCommand('SHADED'), 'VSCURRENT SHADED');
});

test('each token has its own label key', () => {
  assert.equal(styleLabelKey('WIREFRAME'), 'concept.styles.wireframe');
  assert.equal(styleLabelKey('SHADED'), 'concept.styles.shaded');
});

test('the guide style commands match the ADR-0101 grammar', () => {
  assert.equal(guideStyleColorCommand([10, 20, 30]), 'GUIDE STYLE COLOR 10 20 30');
  assert.equal(guideStyleColorCommand([-1, 300, Number.NaN]), 'GUIDE STYLE COLOR 0 255 0');
  assert.equal(guideStyleDashCommand(true), 'GUIDE STYLE DASH ON');
  assert.equal(guideStyleDashCommand(false), 'GUIDE STYLE DASH OFF');
});

test('the colour input round-trips through #rrggbb', () => {
  assert.equal(guideStyleHex(DEFAULT_GUIDE_COLOR), '#3373d9');
  assert.deepEqual(parseGuideStyleHex('#3373d9'), [51, 115, 217]);
  assert.deepEqual(parseGuideStyleHex('#000000'), [0, 0, 0]);
  assert.equal(parseGuideStyleHex('#fff'), null);
  assert.equal(parseGuideStyleHex('3373d9'), null);
  assert.equal(parseGuideStyleHex('#zzzzzz'), null);
});

test('the documented guide default is the old overlay constant', () => {
  assert.deepEqual([...DEFAULT_GUIDE_COLOR], [51, 115, 217]);
  assert.equal(DEFAULT_GUIDE_DASHED, true);
});
