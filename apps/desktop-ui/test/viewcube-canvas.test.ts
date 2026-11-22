// Contract tests for the orientation ball's projection, picking and painter.
//
// None of these are pixel tests: the assertions are geometric (which caps are on
// the front, where their centres sit, which cap a point selects) and structural
// (upright labels, the palette reader's honesty, the front-only grid). The ball
// is a real projection of the host basis — a cap the camera cannot see is never a
// pick target and is never painted.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  drawSphere,
  FRONT_DEPTH_EPSILON,
  pickSphere,
  projectSphere,
  projectSphereGrid,
  readSpherePalette,
  SPHERE_DIRECTIONS,
  SPHERE_RADIUS_RATIO,
} from '../src/cad/viewcube-canvas.ts';
import { viewCubeBasis } from '../src/cad/viewcube.ts';

const SIZE = 112;
const HALF = SIZE / 2;
const RADIUS = SPHERE_RADIUS_RATIO * SIZE;
const palette = {
  surface: '#surface', highlight: '#highlight', rim: '#rim', grid: '#grid',
  cap: '#cap', capEdge: '#cap-edge', text: '#text', textDisabled: '#text-disabled',
  accent: '#accent', hover: '#hover',
};

const ortho = axes => ({ axes, projection: 'orthographic' });
// The three basis triples `viewCubeBasis` produces for the canonical views (its
// own suite proves the link); here they are inputs so the projection is tested
// without the host camera again.
const FRONT = ortho([[1, 0, 0], [0, 1, 0], [0, 0, 1]]);
const TOP = ortho([[1, 0, 0], [0, 0, -1], [0, 1, 0]]);
const RIGHT = ortho([[0, 0, 1], [0, 1, 0], [-1, 0, 0]]);
const camera = overrides => ({ mode: '3d', projection: 'orthographic', eye: [0, 0, 10], target: [0, 0, 0], up: [0, 1, 0], ...overrides });
const SKEW = viewCubeBasis(camera({ eye: [2, -3, 1], up: [0, 1, 0] }));

const capOf = (caps, id) => caps.find(cap => cap.id === id);
const visibleIds = caps => caps.filter(cap => cap.visible).map(cap => cap.id).sort();

/** A recording 2D context: matrices, gradients, fills, strokes and painted labels. */
function recorder() {
  const labels = [];
  const fills = [];
  const strokes = [];
  const stack = [];
  let matrix = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
  let dash = [];
  let fill = null;
  let stroke = null;
  let lineWidth = 1;
  const multiply = next => {
    matrix = {
      a: matrix.a * next.a + matrix.c * next.b,
      b: matrix.b * next.a + matrix.d * next.b,
      c: matrix.a * next.c + matrix.c * next.d,
      d: matrix.b * next.c + matrix.d * next.d,
      e: matrix.a * next.e + matrix.c * next.f + matrix.e,
      f: matrix.b * next.e + matrix.d * next.f + matrix.f,
    };
  };
  const gradient = () => ({ addColorStop() {} });
  const ctx = {
    save() { stack.push({ ...matrix }); },
    restore() { matrix = stack.pop() ?? { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; },
    translate(x, y) { multiply({ a: 1, b: 0, c: 0, d: 1, e: x, f: y }); },
    transform(a, b, c, d, e, f) { multiply({ a, b, c, d, e, f }); },
    beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, arc() {}, clip() {},
    createRadialGradient: gradient,
    setLineDash(next) { dash = next; },
    setTransform() {}, clearRect() {},
    fill() { fills.push(fill); },
    stroke() { strokes.push({ dash: [...dash], stroke, lineWidth }); },
    fillText(text, x, y) { labels.push({ text, x, y, matrix: { ...matrix }, fill }); },
    set fillStyle(value) { fill = value; },
    set strokeStyle(value) { stroke = value; },
    set lineWidth(value) { lineWidth = value; },
    set globalAlpha(value) {},
    set font(value) {},
    set textAlign(value) {},
    set textBaseline(value) {},
  };
  return { ctx, labels, fills, strokes };
}

test('the canonical front basis puts the front cap at the centre and the back is not a front cap', () => {
  const caps = projectSphere(FRONT, SIZE);
  assert.equal(caps.length, 6, 'all six directions are projected');
  const front = capOf(caps, 'front');
  assert.equal(front.view, 'front');
  assert.equal(front.depth, 1, 'the front direction points straight at the viewer');
  assert.equal(front.visible, true);
  assert.ok(Math.abs(front.center.x - HALF) < 1e-9);
  assert.ok(Math.abs(front.center.y - HALF) < 1e-9);
  // The back direction projects to the same point but faces away: it is omitted
  // from both painting and picking.
  const back = capOf(caps, 'back');
  assert.equal(back.visible, false);
  assert.equal(back.depth, -1);
  assert.deepEqual(visibleIds(caps), ['front']);
});

test('every cap centre stays inside the box, and an invalid box invents no points', () => {
  const bases = [
    FRONT, TOP, RIGHT, SKEW,
    viewCubeBasis(camera({ eye: [2.6, -3.1, 1.7], up: [0, 1, 0] })),
    viewCubeBasis(camera({ eye: [1, -1, 1], up: [0, 0, 1], projection: 'perspective' })),
  ];
  for (const basis of bases) {
    for (const cap of projectSphere(basis, SIZE)) {
      assert.ok(cap.center.x >= 0 && cap.center.x <= SIZE, `${cap.id} x ${cap.center.x}`);
      assert.ok(cap.center.y >= 0 && cap.center.y <= SIZE, `${cap.id} y ${cap.center.y}`);
      // A projected direction has screen magnitude <= 1, so its centre is never
      // farther from the middle than one radius.
      assert.ok(Math.hypot(cap.center.x - HALF, cap.center.y - HALF) <= RADIUS + 1e-9);
      assert.ok(cap.radius > 0);
    }
  }
  assert.deepEqual(projectSphere(FRONT, 0), []);
  assert.deepEqual(projectSphere(FRONT, Number.NaN), []);
  assert.deepEqual(projectSphereGrid(FRONT, 0), []);
});

test('the top and right bases show only their own cap (front culling)', () => {
  assert.deepEqual(visibleIds(projectSphere(TOP, SIZE)), ['top']);
  assert.deepEqual(visibleIds(projectSphere(RIGHT, SIZE)), ['right']);
  // The skew camera sees the front, right and top caps, all above the threshold.
  assert.deepEqual(visibleIds(projectSphere(SKEW, SIZE)), ['front', 'right', 'top']);
  // Every painted cap is strictly in front of the threshold.
  for (const cap of projectSphere(SKEW, SIZE)) {
    assert.equal(cap.visible, cap.depth > FRONT_DEPTH_EPSILON);
  }
});

test('the great-circle grid is sampled on the front hemisphere only', () => {
  for (const basis of [FRONT, TOP, RIGHT, SKEW]) {
    const lines = projectSphereGrid(basis, SIZE);
    assert.ok(lines.length >= 3, 'three coordinate planes produce front arcs');
    for (const line of lines) {
      assert.ok(line.length >= 2, 'a line has at least a segment');
      for (const point of line) {
        assert.ok(Number.isFinite(point.x) && Number.isFinite(point.y));
        // A front hemisphere point is a projected unit direction: inside the ball.
        assert.ok(Math.hypot(point.x - HALF, point.y - HALF) <= RADIUS + 1e-9);
      }
    }
  }
  // The arcs rotate with the camera: a face-on front basis and a skew basis
  // cannot produce identical geometry.
  const flat = JSON.stringify(projectSphereGrid(FRONT, SIZE));
  const skew = JSON.stringify(projectSphereGrid(SKEW, SIZE));
  assert.notEqual(flat, skew);
});

test('picking selects a front cap, never a back one, and is null outside the ball', () => {
  // The canonical front view: the centre is the front cap.
  assert.equal(pickSphere(FRONT, { x: HALF, y: HALF }, SIZE), 'front');
  // The box corner and outside the silhouette are nothing.
  assert.equal(pickSphere(FRONT, { x: 1, y: 1 }, SIZE), null);
  assert.equal(pickSphere(FRONT, { x: SIZE + 10, y: HALF }, SIZE), null);
  assert.equal(pickSphere(FRONT, { x: HALF, y: 0 }, SIZE), null, 'the rim gap is not a cap');
  assert.equal(pickSphere(FRONT, { x: HALF, y: HALF }, 0), null);
  // In a top view the centre is the top cap; the front cap is edge-on and culled.
  assert.equal(pickSphere(TOP, { x: HALF, y: HALF }, SIZE), 'top');

  // A back cap is never returned: its projected point either resolves to a front
  // cap or to nothing, never to the direction behind the sphere.
  const back = capOf(projectSphere(SKEW, SIZE), 'back');
  assert.notEqual(pickSphere(SKEW, back.center, SIZE), 'back');
  for (let row = 1; row < 10; row += 1) {
    for (let column = 1; column < 10; column += 1) {
      const id = pickSphere(FRONT, { x: (column / 10) * SIZE, y: (row / 10) * SIZE }, SIZE);
      assert.notEqual(id, 'back', 'the back hemisphere is opaque');
    }
  }
});

test('a click at the ball centre selects the cap nearest the viewer', () => {
  // The skew centre is between the three front caps; the nearest-to-viewer one
  // (the front cap, depth 0.802) is the only honest answer.
  assert.equal(pickSphere(SKEW, { x: HALF, y: HALF }, SIZE), 'front');
  // Each visible cap is selectable at its own disc centre.
  for (const cap of projectSphere(SKEW, SIZE)) {
    if (!cap.visible) continue;
    assert.equal(pickSphere(SKEW, cap.center, SIZE), cap.id, cap.id);
  }
  // The midpoint between two caps is honestly nothing, not a guess.
  const front = capOf(projectSphere(SKEW, SIZE), 'front');
  const right = capOf(projectSphere(SKEW, SIZE), 'right');
  const midpoint = { x: (front.center.x + right.center.x) / 2, y: (front.center.y + right.center.y) / 2 };
  assert.equal(pickSphere(SKEW, midpoint, SIZE), null);
});

test('the palette is read from tokens, and a missing token is not guessed', () => {
  const tokens = {
    '--ball-surface': ' #36414f ', '--ball-highlight': '#6b7d92', '--ball-rim': '#1b222b',
    '--ball-grid': '#93a4b8', '--ball-cap': '#232d38', '--line-soft': '#4b5563',
    '--text': '#e9edf3', '--text-disabled': '#98a3b4', '--accent': '#4a9eff', '--hover': '#4c5766',
  };
  const read = readSpherePalette(token => tokens[token] ?? '');
  assert.equal(read.surface, '#36414f', 'token values are trimmed');
  assert.equal(read.highlight, '#6b7d92');
  assert.equal(read.capEdge, '#4b5563');
  assert.equal(read.textDisabled, '#98a3b4');
  assert.equal(read.accent, '#4a9eff');
  assert.equal(readSpherePalette(token => (token === '--hover' ? '' : '#000')), null);
});

test('the painter labels every visible cap upright and omits the back', () => {
  const front = recorder();
  drawSphere(front.ctx, FRONT, SIZE, palette, {});
  assert.deepEqual(front.labels.map(label => label.text), ['FRT']);
  const text = front.labels[0].matrix;
  // No rotation is applied: local text x → screen right and local text y (down)
  // → screen down. `d > 0` means upright; determinant +1 means never mirrored.
  assert.ok(text.d > 0, 'the label is never upside-down');
  assert.ok(text.a > 0, 'the label is never mirrored');
  assert.ok(Math.abs(text.a - 1) < 1e-9 && Math.abs(text.b) < 1e-9);
  assert.ok(Math.abs(text.c) < 1e-9 && Math.abs(text.d - 1) < 1e-9);
  assert.ok(Math.abs(front.labels[0].x - HALF) < 1e-9 && Math.abs(front.labels[0].y - HALF) < 1e-9,
    'the label sits on the cap centre');
  assert.equal(front.labels[0].fill, palette.text);
  assert.ok(front.fills.some(fill => fill === palette.cap), 'the visible cap has its fill');

  // The back cap of the same basis is not painted at all.
  for (const label of front.labels) assert.notEqual(label.text, 'BCK');
});

test('hover and disabled styling are visible on the canvas', () => {
  const hovered = recorder();
  drawSphere(hovered.ctx, FRONT, SIZE, palette, { hover: 'front' });
  assert.ok(hovered.fills.includes(palette.hover), 'the hovered cap takes the hover fill');
  assert.ok(
    hovered.strokes.some(stroke => stroke.stroke === palette.accent),
    'the hovered cap gets the accent edge',
  );

  const disabled = recorder();
  drawSphere(disabled.ctx, FRONT, SIZE, palette, { disabled: ['front'] });
  assert.ok(
    disabled.strokes.some(stroke => stroke.dash.length === 2 && stroke.dash[0] === 2 && stroke.dash[1] === 2),
    'a disabled cap gets a dashed edge',
  );
  assert.equal(disabled.labels[0].fill, palette.textDisabled, 'and disabled ink');
});

test('the six standard directions keep their panel order and ASCII glyphs', () => {
  assert.deepEqual(SPHERE_DIRECTIONS.map(direction => direction.id), ['front', 'back', 'right', 'left', 'top', 'bottom']);
  assert.deepEqual(SPHERE_DIRECTIONS.map(direction => direction.short), ['FRT', 'BCK', 'RGT', 'LFT', 'TOP', 'BOT']);
  assert.deepEqual(SPHERE_DIRECTIONS.map(direction => direction.view), SPHERE_DIRECTIONS.map(direction => direction.id));
});
