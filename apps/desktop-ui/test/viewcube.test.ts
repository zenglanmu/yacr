// Contract tests for the ViewCube's host-derived facts and its canvas host.
//
// Three contracts, none of them pixels:
//
//   * the AutoCAD defaults fill an absent `navigation` block, and the visibility
//     rule is the host's `NAVVCUBEDISPLAY` class bit **and** the matching
//     `DISPLAYVIEWCUBEIN2D/3D` gate (with a `null` view falling into the 2D
//     class, exactly like the host's own default);
//   * the basis is the host camera in screen space (identity for the canonical
//     front view, the old `matrix3d` columns for top/right as axis triples), and
//     a degenerate/non-finite basis returns `null` — never a guessed orientation;
//   * the component is a 2D canvas plus six hidden real buttons and Home: one
//     volatile-store subscription (so an orbit does not re-render the shell),
//     standard views through `runStandardView`, the drag through `runOrbit`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  DEFAULT_NAVIGATION,
  navigationSettings,
  viewCubeBasis,
  viewCubeClass,
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

/** The basis axes as plain arrays, so a failure prints the numbers. */
const axesOf = basis => basis.axes.map(axis => [...axis]);

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

test('the canonical front view is the identity basis', () => {
  // The host's FRONT is eye −Y with +Z up; the cube's front face (+Z local) faces
  // the reader and no axis is rotated or turned away.
  const basis = viewCubeBasis(camera({ eye: [0, -10, 0], up: [0, 0, 1] }));
  assert.deepEqual(axesOf(basis), [[1, 0, 0], [0, 1, 0], [0, 0, 1]]);
  assert.equal(basis.projection, 'orthographic');
});

test('top and right views put the host camera basis on screen, with CSS y inverted', () => {
  // These are the columns the old CSS `matrix3d(...)` produced, now as the images
  // of the three cube-local axes. TOP: the top face reaches the reader and the
  // front face projects down the screen ([0, 1, 0]).
  const top = viewCubeBasis(camera({ eye: [0, 0, 10], up: [0, 1, 0] }));
  assert.deepEqual(axesOf(top), [[1, 0, 0], [0, 0, -1], [0, 1, 0]]);
  // RIGHT: looking from +x makes the right face point at the viewer (+depth).
  const right = viewCubeBasis(camera({ eye: [10, 0, 0], up: [0, 0, 1] }));
  assert.deepEqual(axesOf(right), [[0, 0, 1], [0, 1, 0], [-1, 0, 0]]);
  // The host's lens is carried through, not guessed.
  assert.equal(viewCubeBasis(camera({ projection: 'perspective' })).projection, 'perspective');
  assert.equal(viewCubeBasis(camera({ projection: 'sideways' })).projection, 'orthographic');
});

test('a missing or degenerate basis never fabricates an orientation', () => {
  assert.equal(viewCubeBasis(null), null);
  assert.equal(viewCubeBasis(undefined), null);
  // eye == target: no direction.
  assert.equal(viewCubeBasis(camera({ eye: [0, 0, 0], target: [0, 0, 0] })), null);
  // up parallel to z: no right vector.
  assert.equal(viewCubeBasis(camera({ up: [0, 0, 1] })), null);
  // A non-finite value is refused (the status validator already rejects it; the
  // renderer must not turn NaN into a basis if a test double slips through).
  assert.equal(viewCubeBasis(camera({ eye: [Number.NaN, 0, 1] })), null);
});

test('the component is a canvas with hidden real faces, Home and an orbit drag', async () => {
  const source = await readModule('components/ViewCube.tsx');
  assert.ok(source.includes('useSyncExternalStore('), 'the orbit lives in the leaf');
  assert.ok(source.includes('volatileStatusStore.subscribe'));
  assert.ok(source.includes('volatileStatusStore.getView'));
  // A 2D canvas, sized in CSS px and scaled by the device pixel ratio.
  assert.ok(source.includes('className="viewcube__canvas"'));
  assert.ok(source.includes('aria-hidden="true"'), 'the canvas is decoration');
  assert.ok(source.includes("getContext('2d')"), 'a real 2D context, with an explicit failure state');
  assert.ok(source.includes('setTransform(ratio, 0, 0, ratio, 0, 0)'), 'the backing store follows devicePixelRatio');
  assert.ok(source.includes('ResizeObserver'), 'a size change redraws');
  assert.ok(source.includes('MutationObserver') && source.includes('THEME_ATTRIBUTE'), 'a theme switch re-reads the tokens');
  assert.ok(source.includes('readCubePalette'), 'the palette comes from the CSS tokens');
  assert.ok(source.includes("getPropertyValue('--font-ui')"), 'the label font is the chrome token, not the UA default');
  assert.equal(source.includes('requestAnimationFrame'), false, 'no permanent animation loop in the component');
  // Six faces + Home stay real, focusable controls.
  assert.ok(source.includes('CUBE_FACES.map'), 'the face table is the single source');
  assert.ok(source.includes('className="sr-only"'), 'the faces are visually hidden, not removed');
  for (const face of ['front', 'back', 'left', 'right', 'top', 'bottom']) {
    assert.ok(source.includes(`'viewcube.face.${face}'`), `${face} has a catalog key`);
  }
  assert.ok(source.includes('shell.runStandardView(face.view)'), 'a face is a standard view, never a camera write');
  assert.ok(source.includes("shell.runStandardView('isometric')"));
  assert.ok(source.includes('standardViewAvailable(shell, face.view)'), 'shared admission, not a local guess');
  assert.ok(source.includes('standardViewUnavailableReasonKey'), 'a refusal keeps the shared reason key');
  // The pointer split: a click picks a face, a drag orbits through the coalescer.
  assert.ok(source.includes('pickFace('), 'a click hit-tests the visible faces');
  assert.ok(source.includes('new OrbitDrag('), 'a drag goes through the one coalescer');
  assert.ok(source.includes('orbitDelta('), 'the drag uses the host-mirroring delta');
  assert.ok(source.includes('orbitAvailable('), 'the carrier/view gate is shared, not local');
  assert.ok(source.includes('onLostPointerCapture') && source.includes('onPointerCancel'), 'a cancelled gesture ends without a camera move');
  // The honest states.
  assert.ok(source.includes("t('viewcube.orientation_unavailable')"), 'a null/degenerate view is explicit');
  assert.ok(source.includes("t('viewcube.render_unavailable')"), 'a canvas failure is explicit');
  assert.ok(source.includes("t('viewcube.standard_only')"), 'the edge/corner limit is stated');
  assert.ok(source.includes("t('viewcube.orbit_hint')"), 'the drag is described');
  assert.ok(source.includes("'viewcube.orbit_needs_3d'"), 'the 2D refusal has the cube wording');
});

test('the cube stylesheet sizes the canvas and is not a floating overlay', async () => {
  const css = await readModule('components/viewcube.css');
  assert.ok(css.includes('--viewcube-size: 48px'), 'the cube size is a token');
  const ruleFor = selector => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start >= 0, `${selector} has a rule`);
    return css.slice(start, css.indexOf('}', start));
  };
  const canvas = ruleFor('.viewcube__canvas');
  for (const declaration of ['width: 100%', 'height: auto', 'aspect-ratio: 1', 'display: block', 'touch-action: none']) {
    assert.ok(canvas.includes(declaration), `.viewcube__canvas keeps ${declaration}`);
  }
  // The size token owns the box: the stage is exactly one cube wide.
  assert.ok(ruleFor('.viewcube__stage').includes('width: var(--viewcube-size)'));
  assert.equal(/position:\s*fixed/.test(css), false, 'the cube is docked, never a floating overlay');
  assert.equal(css.includes('.viewcube__face'), false, 'the faces are canvas paint, not DOM faces');
  // Cube styles stay in their own file: the shared sheet only owns the dock frame.
  const style = await readModule('style.css');
  assert.equal(style.includes('.viewcube__canvas'), false, 'no cube canvas rule in style.css');
  assert.ok(style.includes('.navdock'), 'the dock frame lives with the other regions');
});
