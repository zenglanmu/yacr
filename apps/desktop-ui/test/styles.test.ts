// The Styles panel exposes exactly the two real VSCURRENT tokens.
//
// Every other SketchUp style spelling is refused by the host with
// `vscurrent.style_not_implemented`; the panel must never offer one.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  STYLE_TOKENS,
  isStyleToken,
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
