// Contract tests for the ViewCube's projection, its picking and its painter.
//
// None of these are pixel tests: the assertions are geometric (which faces are
// visible, where their corners sit, which face a point selects) and structural
// (the upright text frame, the palette reader's honesty). The cube is a real
// projection of the host basis — a face the camera cannot see is never a pick
// target and is never painted.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  drawCube,
  MAX_CUBE_EXTENT,
  PERSPECTIVE_STRENGTH,
  pickFace,
  projectCube,
  readCubePalette,
} from '../src/cad/viewcube-canvas.ts';
import { viewCubeBasis } from '../src/cad/viewcube.ts';

const SIZE = 48;
const HALF = SIZE / 2;
const palette = {
  front: '#front', back: '#back', left: '#left', right: '#right',
  top: '#top', bottom: '#bottom', edge: '#edge', text: '#text',
  textDisabled: '#text-disabled', accent: '#accent', hover: '#hover',
};

const ortho = axes => ({ axes, projection: 'orthographic' });
// The three basis triples `viewCubeBasis` produces for the canonical views (its
// own suite proves the link); here they are inputs so the projection is tested
// without the host camera again.
const FRONT = ortho([[1, 0, 0], [0, 1, 0], [0, 0, 1]]);
const TOP = ortho([[1, 0, 0], [0, 0, -1], [0, 1, 0]]);
const RIGHT = ortho([[0, 0, 1], [0, 1, 0], [-1, 0, 0]]);
const camera = overrides => ({ mode: '3d', projection: 'orthographic', eye: [0, 0, 10], target: [0, 0, 0], up: [0, 1, 0], ...overrides });
const ids = quads => quads.map(quad => quad.id);

/** A recording 2D context: matrices, fills, dashes and the painted labels. */
function recorder() {
  const labels = [];
  const fills = [];
  const strokes = [];
  const stack = [];
  let matrix = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
  let dash = [];
  let fill = null;
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
  const ctx = {
    save() { stack.push({ ...matrix }); },
    restore() { matrix = stack.pop() ?? { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; },
    translate(x, y) { multiply({ a: 1, b: 0, c: 0, d: 1, e: x, f: y }); },
    transform(a, b, c, d, e, f) { multiply({ a, b, c, d, e, f }); },
    beginPath() {}, moveTo() {}, lineTo() {}, closePath() {},
    setLineDash(next) { dash = next; },
    setTransform() {}, clearRect() {},
    fill() { fills.push(fill); },
    stroke() { strokes.push([...dash]); },
    fillText(text) { labels.push({ text, matrix: { ...matrix }, fill }); },
    set fillStyle(value) { fill = value; },
    set strokeStyle(value) {},
    set lineWidth(value) {},
    set font(value) {},
    set textAlign(value) {},
    set textBaseline(value) {},
  };
  return { ctx, labels, fills, strokes };
}

test('the canonical front basis projects one face-on square at the centre', () => {
  const quads = projectCube(FRONT, SIZE);
  assert.deepEqual(ids(quads), ['front']);
  const [face] = quads;
  assert.equal(face.view, 'front');
  assert.equal(face.depth, 1, 'the front face points straight at the viewer');
  assert.ok(Math.abs(face.centroid.x - HALF) < 1e-9);
  assert.ok(Math.abs(face.centroid.y - HALF) < 1e-9);
  const xs = face.points.map(point => point.x);
  const ys = face.points.map(point => point.y);
  const width = Math.max(...xs) - Math.min(...xs);
  const height = Math.max(...ys) - Math.min(...ys);
  assert.ok(Math.abs(width - height) < 1e-9, 'a face-on face is a square');
  // The fitted radius is the worst-case (isometric) one, so a face-on cube is
  // genuinely smaller than its box — the silhouette grows as the cube turns.
  const expected = 2 * ((SIZE / 2) * 0.98) / MAX_CUBE_EXTENT;
  assert.ok(Math.abs(width - expected) < 1e-9, `${width} vs ${expected}`);
});

test('every projected point stays inside the sizePx box, in both lenses', () => {
  const bases = [
    FRONT, TOP, RIGHT,
    viewCubeBasis(camera({ eye: [1, -1, 1], up: [0, 0, 1] })),
    viewCubeBasis(camera({ eye: [2.6, -3.1, 1.7], up: [0, 1, 0] })),
    viewCubeBasis(camera({ eye: [1, -1, 1], up: [0, 0, 1], projection: 'perspective' })),
  ];
  for (const basis of bases) {
    for (const quad of projectCube(basis, SIZE)) {
      for (const point of quad.points) {
        assert.ok(point.x >= 0 && point.x <= SIZE, `${quad.id} x ${point.x}`);
        assert.ok(point.y >= 0 && point.y <= SIZE, `${quad.id} y ${point.y}`);
      }
    }
  }
  // A zero/NaN box has no coordinates to give, and must not invent NaN points.
  assert.deepEqual(projectCube(FRONT, 0), []);
  assert.deepEqual(projectCube(FRONT, Number.NaN), []);
});

test('the top and right bases show only their own face (backface culling)', () => {
  assert.deepEqual(ids(projectCube(TOP, SIZE)), ['top']);
  assert.deepEqual(ids(projectCube(RIGHT, SIZE)), ['right']);
  // The canonical front basis hides the five faces pointing away from the eye.
  const front = ids(projectCube(FRONT, SIZE));
  for (const hidden of ['back', 'top', 'bottom', 'left', 'right']) {
    assert.equal(front.includes(hidden), false, `${hidden} is culled`);
  }
});

test('visible faces are ordered back-to-front by depth', () => {
  const skew = viewCubeBasis(camera({ eye: [2, -3, 1], up: [0, 1, 0] }));
  const quads = projectCube(skew, SIZE);
  assert.deepEqual(ids(quads), ['top', 'right', 'front'], 'three faces face this camera');
  assert.deepEqual(quads.map(quad => Number(quad.depth.toFixed(3))), [0.267, 0.535, 0.802]);
  for (let index = 1; index < quads.length; index += 1) {
    assert.ok(quads[index].depth >= quads[index - 1].depth, 'nearer faces paint later');
  }
});

test('picking selects a visible face and rejects a hidden one', () => {
  const quads = projectCube(FRONT, SIZE);
  assert.equal(pickFace(quads, { x: HALF, y: HALF }), 'front');
  // The back face projects onto the same square but is not in the visible list,
  // so a click there can only ever be the front face.
  assert.equal(pickFace(quads, { x: HALF, y: HALF }), 'front');
  // Outside the silhouette (the box corner) is nothing.
  assert.equal(pickFace(quads, { x: 1, y: 1 }), null);
  assert.equal(pickFace(quads, { x: SIZE + 10, y: HALF }), null);
  assert.equal(pickFace([], { x: HALF, y: HALF }), null);
  // In a top view the front face is edge-on and culled: the centre is the top.
  const top = projectCube(TOP, SIZE);
  assert.equal(pickFace(top, { x: HALF, y: HALF }), 'top');
  // Every visible face is selectable at its own centroid.
  const skew = projectCube(viewCubeBasis(camera({ eye: [2, -3, 1], up: [0, 1, 0] })), SIZE);
  for (const quad of skew) {
    assert.equal(pickFace(skew, quad.centroid), quad.id, quad.id);
  }
});

test('a perspective host gets one mild, deterministic lens', () => {
  assert.equal(MAX_CUBE_EXTENT, Math.sqrt(3));
  assert.ok(PERSPECTIVE_STRENGTH > 0 && PERSPECTIVE_STRENGTH < 0.3);
  const flat = projectCube(FRONT, SIZE);
  const lens = projectCube({ axes: FRONT.axes, projection: 'perspective' }, SIZE);
  assert.deepEqual(ids(lens), ['front']);
  const widthOf = quads => Math.max(...quads[0].points.map(point => point.x))
    - Math.min(...quads[0].points.map(point => point.x));
  assert.ok(widthOf(lens) < widthOf(flat), 'a perspective host shrinks the whole cube slightly');
  // The documented lens: near depth grows, and a face at depth `d` is scaled by
  // `(1 + k·d/√3) / (1 + k)`. The face-on face sits at depth 1.
  const expected = (1 + PERSPECTIVE_STRENGTH / MAX_CUBE_EXTENT) / (1 + PERSPECTIVE_STRENGTH);
  assert.ok(Math.abs(widthOf(lens) / widthOf(flat) - expected) < 1e-12, 'the lens is exactly the documented factor');
  // Depth does not depend on the lens: the projection carries the host's depth.
  assert.equal(lens[0].depth, flat[0].depth);
});

test('the palette is read from tokens, and a missing token is not guessed', () => {
  const tokens = {
    '--cube-front': ' #404b5a ', '--cube-back': '#3a4452', '--cube-left': '#303a46',
    '--cube-right': '#37414e', '--cube-top': '#4c5766', '--cube-bottom': '#2b333f',
    '--line-soft': '#4b5563', '--text': '#e9edf3', '--text-disabled': '#98a3b4',
    '--accent': '#4a9eff', '--hover': '#4c5766',
  };
  const read = readCubePalette(token => tokens[token] ?? '');
  assert.equal(read.front, '#404b5a', 'token values are trimmed');
  assert.equal(read.top, '#4c5766');
  assert.equal(read.edge, '#4b5563');
  assert.equal(read.textDisabled, '#98a3b4');
  assert.equal(read.accent, '#4a9eff');
  assert.equal(readCubePalette(token => (token === '--hover' ? '' : '#000')), null);
});

test('the painter labels every readable face upright and skips a sliver', () => {
  const front = recorder();
  drawCube(front.ctx, projectCube(FRONT, SIZE), palette, { sizePx: SIZE });
  assert.deepEqual(front.labels.map(label => label.text), ['FRT']);
  const text = front.labels[0].matrix;
  // Local text x → right, local text y (down) → down the screen: d > 0 means the
  // glyph is upright, and a positive determinant means it is never mirrored.
  assert.ok(text.d > 0, 'the label is never upside-down');
  assert.ok(text.a * text.d - text.b * text.c > 0, 'the label is never mirrored');
  assert.ok(Math.abs(text.e - HALF) < 1e-9 && Math.abs(text.f - HALF) < 1e-9);
  assert.equal(front.labels[0].fill, palette.text);

  // A face whose projected short edge is a sliver gets no label at all.
  const sliver = {
    id: 'front', view: 'front', depth: 1, centroid: { x: 20, y: 24.5 },
    points: [{ x: 0, y: 24 }, { x: 40, y: 24 }, { x: 40, y: 25 }, { x: 0, y: 25 }],
  };
  const thin = recorder();
  drawCube(thin.ctx, [sliver], palette, { sizePx: SIZE });
  assert.deepEqual(thin.labels, []);
});

test('hover and disabled styling are visible on the canvas', () => {
  const quads = projectCube(FRONT, SIZE);
  const hovered = recorder();
  drawCube(hovered.ctx, quads, palette, { sizePx: SIZE, hover: 'front' });
  assert.deepEqual(hovered.fills, [palette.hover], 'the hovered face takes the hover fill');

  const disabled = recorder();
  drawCube(disabled.ctx, quads, palette, { sizePx: SIZE, disabled: ['front'] });
  assert.equal(disabled.fills[0], palette.front, 'a disabled face keeps its own fill');
  assert.deepEqual(disabled.strokes[0], [2, 2], 'a disabled face gets a dashed edge');
  assert.equal(disabled.labels[0].fill, palette.textDisabled, 'and disabled ink');
});
