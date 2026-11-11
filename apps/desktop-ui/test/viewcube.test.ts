// Contract tests for the ViewCube's host-derived facts.
//
// Three contracts, none of them pixels:
//
//   * the AutoCAD defaults fill an absent `navigation` block, and the visibility
//     rule is the host's `NAVVCUBEDISPLAY` class bit **and** the matching
//     `DISPLAYVIEWCUBEIN2D/3D` gate (with a `null` view falling into the 2D
//     class, exactly like the host's own default);
//   * the CSS rotation is the host camera basis (identity for the canonical front
//     view, the known pose for a top/right view), and a degenerate basis returns
//     `null` — never a guessed orientation;
//   * the component reads the volatile store (so an orbit does not re-render the
//     shell), sends the six faces and Home through `runStandardView`, and the
//     faces are real buttons whose CSS is a `preserve-3d` cube.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  DEFAULT_NAVIGATION,
  navigationSettings,
  viewCubeClass,
  viewCubeMatrix,
  viewCubeVisible,
} from '../src/cad/viewcube.ts';

const camera = (overrides = {}) => ({
  mode: '3d',
  projection: 'orthographic',
  eye: [0, 0, 10],
  target: [0, 0, 0],
  up: [0, 1, 0],
  ...overrides,
});

test('an absent navigation block keeps AutoCAD defaults; a present block is honoured', () => {
  assert.deepEqual(navigationSettings(undefined), DEFAULT_NAVIGATION);
  assert.deepEqual(navigationSettings(null), DEFAULT_NAVIGATION);
  assert.equal(DEFAULT_NAVIGATION.navbarVisible, true);
  assert.equal(DEFAULT_NAVIGATION.viewCubeDisplay, 3);
  assert.equal(DEFAULT_NAVIGATION.viewCubeIn2d, true);
  assert.equal(DEFAULT_NAVIGATION.viewCubeIn3d, true);
  const custom = { navbarVisible: false, viewCubeDisplay: 2, viewCubeIn2d: false, viewCubeIn3d: true };
  assert.deepEqual(navigationSettings(custom), custom);
  // A partial/typed-wrong block never produces `undefined` fields: a later
  // reader (the rail, the stylesheet decision) always gets a real boolean/bit.
  const fallback = navigationSettings({ navbarVisible: 'yes', viewCubeDisplay: 9 });
  assert.equal(fallback.navbarVisible, true);
  assert.equal(fallback.viewCubeDisplay, 3);
});

test('the cube class and the display bit follow the live view mode', () => {
  assert.equal(viewCubeClass(null), 2);
  assert.equal(viewCubeClass(camera({ mode: '2d' })), 2);
  assert.equal(viewCubeClass(camera({ mode: '3d' })), 3);

  // 3D styles only: display bit 1, 3D gate.
  const threeOnly = navigationSettings({ navbarVisible: true, viewCubeDisplay: 1, viewCubeIn2d: true, viewCubeIn3d: true });
  assert.equal(viewCubeVisible(camera({ mode: '3d' }), threeOnly), true);
  assert.equal(viewCubeVisible(camera({ mode: '2d' }), threeOnly), false);
  // The gate is independent of the bit.
  const threeGateOff = navigationSettings({ navbarVisible: true, viewCubeDisplay: 3, viewCubeIn2d: true, viewCubeIn3d: false });
  assert.equal(viewCubeVisible(camera({ mode: '3d' }), threeGateOff), false);
  assert.equal(viewCubeVisible(camera({ mode: '2d' }), threeGateOff), true);

  // 2D styles only: display bit 2, 2D gate.
  const twoOnly = navigationSettings({ navbarVisible: true, viewCubeDisplay: 2, viewCubeIn2d: true, viewCubeIn3d: true });
  assert.equal(viewCubeVisible(camera({ mode: '2d' }), twoOnly), true);
  assert.equal(viewCubeVisible(camera({ mode: '3d' }), twoOnly), false);
  const twoGateOff = navigationSettings({ navbarVisible: true, viewCubeDisplay: 3, viewCubeIn2d: false, viewCubeIn3d: true });
  assert.equal(viewCubeVisible(camera({ mode: '2d' }), twoGateOff), false);
  assert.equal(viewCubeVisible(camera({ mode: '3d' }), twoGateOff), true);

  // Display 0 hides both classes; an absent block shows both.
  const none = navigationSettings({ navbarVisible: true, viewCubeDisplay: 0, viewCubeIn2d: true, viewCubeIn3d: true });
  assert.equal(viewCubeVisible(camera(), none), false);
  assert.equal(viewCubeVisible(camera({ mode: '2d' }), none), false);
  assert.equal(viewCubeVisible(camera(), undefined), true);

  // A host that reports null still shows the unavailable cube when the 2D gate
  // allows it: the null stays visible instead of silently removing the control.
  assert.equal(viewCubeVisible(null, undefined), true);
  assert.equal(viewCubeVisible(null, twoGateOff), false, 'the null view takes the 2D gate');
  assert.equal(viewCubeVisible(null, navigationSettings({ navbarVisible: true, viewCubeDisplay: 3, viewCubeIn2d: true, viewCubeIn3d: false })), true);
  assert.equal(viewCubeVisible(null, navigationSettings({ navbarVisible: true, viewCubeDisplay: 2, viewCubeIn2d: false, viewCubeIn3d: true })), false);
});

test('the canonical front view is the identity rotation', () => {
  // The host's FRONT is eye −Y with +Z up; the local front face (+z) faces the
  // reader, so the CSS cube needs no rotation.
  assert.equal(
    viewCubeMatrix(camera({ eye: [0, -10, 0], up: [0, 0, 1] })),
    'matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)',
  );
});

test('top and right views put the host camera basis on screen, with CSS y inverted', () => {
  // Host TOP (eye +Z, world +Y as the up hint): the top face (local −Y) reaches
  // the reader; the local +Z/front face projects to the top of the screen.
  assert.equal(
    viewCubeMatrix(camera({ eye: [0, 0, 10], up: [0, 1, 0] })),
    'matrix3d(1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1)',
  );
  // Looking from +x: the local +x face (right) faces the viewer.
  assert.equal(
    viewCubeMatrix(camera({ eye: [10, 0, 0], up: [0, 0, 1] })),
    'matrix3d(0, 0, 1, 0, 0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 0, 1)',
  );
});

test('a missing or degenerate basis never fabricates a rotation', () => {
  assert.equal(viewCubeMatrix(null), null);
  assert.equal(viewCubeMatrix(undefined), null);
  // eye == target: no direction.
  assert.equal(viewCubeMatrix(camera({ eye: [0, 0, 0], target: [0, 0, 0] })), null);
  // up parallel to z: no right vector.
  assert.equal(viewCubeMatrix(camera({ up: [0, 0, 1] })), null);
  // A non-finite value is refused (the status validator already rejects it; the
  // renderer must not turn NaN into a matrix if a test double slips through).
  assert.equal(viewCubeMatrix(camera({ eye: [Number.NaN, 0, 1] })), null);
});

test('the component subscribes to the volatile view and sends real standard views', async () => {
  const source = await readModule('components/ViewCube.tsx');
  assert.ok(source.includes('useSyncExternalStore('), 'the orbit lives in the leaf');
  assert.ok(source.includes('volatileStatusStore.subscribe'));
  assert.ok(source.includes('volatileStatusStore.getView'));
  // Six faces + Home are typed standard-view buttons, not invented camera writes.
  for (const view of ['top', 'bottom', 'front', 'back', 'left', 'right', 'isometric']) {
    assert.ok(source.includes(`'${view}'`), `${view} is one of the seven host keys`);
  }
  assert.ok(source.includes("shell.runStandardView(face.view)"));
  assert.ok(source.includes("shell.runStandardView('isometric')"));
  assert.ok(source.includes('standardViewAvailable(shell, face.view)'), 'shared admission, not a local guess');
  assert.ok(source.includes('standardViewUnavailableReasonKey'), 'a refusal keeps the shared reason key');
  assert.ok(source.includes("t('viewcube.standard_only')"), 'the edge/corner limit is stated');
  assert.ok(source.includes("t('viewcube.orientation_unavailable')"), 'a null view is explicit');
  assert.ok(source.includes("t(face.labelKey)"), 'faces read their catalog names from the face table');
  for (const face of ['front', 'back', 'left', 'right', 'top', 'bottom']) {
    assert.ok(source.includes(`'viewcube.face.${face}'`), `${face} has a catalog key`);
  }
});

test('the cube stylesheet is a preserve-3d cube with no floating overlay', async () => {
  const css = await readModule('components/viewcube.css');
  assert.ok(css.includes('transform-style: preserve-3d'));
  assert.ok(css.includes('backface-visibility: hidden'));
  assert.ok(css.includes('perspective:'), 'the stage carries the depth cue');
  assert.ok(css.includes('[data-projection="orthographic"]'), 'an orthographic host gets no lens');
  for (const face of ['front', 'back', 'left', 'right', 'top', 'bottom']) {
    assert.ok(css.includes(`.viewcube__face--${face} { transform:`), `${face} has a rest pose`);
  }
  assert.equal(/position:\s*fixed/.test(css), false, 'the cube is docked, never a floating overlay');
  // Cube styles stay in their own file: the shared sheet only owns the dock frame.
  const style = await readModule('style.css');
  assert.equal(style.includes('.viewcube__face'), false, 'no cube face rule in style.css');
  assert.ok(style.includes('.navdock'), 'the dock frame lives with the other regions');
});
