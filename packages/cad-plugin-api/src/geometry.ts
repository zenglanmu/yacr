// Geometry value types and their constructors.
//
// These are the only genuinely local operations in v1: no document, no wire, no
// carrier. Every function is pure and returns frozen-by-convention readonly
// values; nothing here allocates host state or claims a host answered.
//
// Conventions (documented because matrices are easy to get backwards):
//   * `Matrix3d.values` is 16 numbers, **row-major**, with **row-vector**
//     semantics: `p' = p * M`, so the translation row lives at indices 12..14.
//   * Angles are radians, counter-clockwise about +Z.
//   * A point is a location; a vector is a displacement. The add operation takes
//     one of each, so the naming carries the distinction AutoCAD makes.

import { invalidArgument } from './errors.ts';

/** A location in the XY plane. */
export interface Point2d {
  readonly x: number;
  readonly y: number;
}

/** A location in model space. */
export interface Point3d {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

/** A displacement in the XY plane. */
export interface Vector2d {
  readonly x: number;
  readonly y: number;
}

/** A displacement in model space. */
export interface Vector3d {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

/** A 4x4 affine transform; see the row-major/row-vector note above. */
export interface Matrix3d {
  readonly values: readonly number[];
}

/** An unbounded plane: an origin plus a (normalized) unit normal. */
export interface Plane {
  readonly origin: Point3d;
  readonly normal: Vector3d;
}

/** The geometry surface exposed on `CadApi.geometry`. */
export interface GeometryLibrary {
  readonly point2d: typeof point2d;
  readonly point3d: typeof point3d;
  readonly vector2d: typeof vector2d;
  readonly vector3d: typeof vector3d;
  readonly matrix3dIdentity: typeof matrix3dIdentity;
  readonly matrix3dTranslate: typeof matrix3dTranslate;
  readonly matrix3dScale: typeof matrix3dScale;
  readonly matrix3dRotateZ: typeof matrix3dRotateZ;
  readonly planeFromOriginNormal: typeof planeFromOriginNormal;
  readonly distance3d: typeof distance3d;
  readonly addPoints: typeof addPoints;
  readonly transformPoint: typeof transformPoint;
}

function requireFinite(value: unknown, field: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalidArgument(field);
  return value;
}

/** Validate a 16-element matrix and return its values. */
function requireMatrix(matrix: Matrix3d): readonly number[] {
  if (typeof matrix !== 'object' || matrix === null) throw invalidArgument('matrix');
  const values = matrix.values;
  if (!Array.isArray(values) || values.length !== 16) throw invalidArgument('matrix.values');
  for (let index = 0; index < 16; index += 1) {
    requireFinite(values[index], `matrix.values[${index}]`);
  }
  return values;
}

/** A 2D point. */
export function point2d(x: number, y: number): Point2d {
  return { x: requireFinite(x, 'x'), y: requireFinite(y, 'y') };
}

/** A 3D point. */
export function point3d(x: number, y: number, z: number): Point3d {
  return {
    x: requireFinite(x, 'x'),
    y: requireFinite(y, 'y'),
    z: requireFinite(z, 'z'),
  };
}

/** A 2D displacement. */
export function vector2d(x: number, y: number): Vector2d {
  return { x: requireFinite(x, 'x'), y: requireFinite(y, 'y') };
}

/** A 3D displacement. */
export function vector3d(x: number, y: number, z: number): Vector3d {
  return {
    x: requireFinite(x, 'x'),
    y: requireFinite(y, 'y'),
    z: requireFinite(z, 'z'),
  };
}

/** The identity transform. */
export function matrix3dIdentity(): Matrix3d {
  return {
    values: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
  };
}

/** A translation by `(x, y, z)`. */
export function matrix3dTranslate(x: number, y: number, z: number): Matrix3d {
  return {
    values: [
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 1, 0,
      requireFinite(x, 'x'), requireFinite(y, 'y'), requireFinite(z, 'z'), 1,
    ],
  };
}

/** A non-uniform scale about the origin. */
export function matrix3dScale(x: number, y: number, z: number): Matrix3d {
  return {
    values: [
      requireFinite(x, 'x'), 0, 0, 0,
      0, requireFinite(y, 'y'), 0, 0,
      0, 0, requireFinite(z, 'z'), 0,
      0, 0, 0, 1,
    ],
  };
}

/** A rotation by `angle` radians, counter-clockwise about +Z. */
export function matrix3dRotateZ(angle: number): Matrix3d {
  const radians = requireFinite(angle, 'angle');
  const cos = Math.cos(radians);
  const sin = Math.sin(radians);
  return {
    values: [cos, sin, 0, 0, -sin, cos, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
  };
}

/** A plane through `origin` with `normal` normalized; a zero normal is refused. */
export function planeFromOriginNormal(origin: Point3d, normal: Vector3d): Plane {
  if (typeof origin !== 'object' || origin === null) throw invalidArgument('origin');
  if (typeof normal !== 'object' || normal === null) throw invalidArgument('normal');
  const x = requireFinite(normal.x, 'normal.x');
  const y = requireFinite(normal.y, 'normal.y');
  const z = requireFinite(normal.z, 'normal.z');
  const length = Math.hypot(x, y, z);
  if (!(length > 0)) throw invalidArgument('normal');
  return {
    origin: point3d(origin.x, origin.y, origin.z),
    normal: vector3d(x / length, y / length, z / length),
  };
}

/** Euclidean distance between two points. */
export function distance3d(first: Point3d, second: Point3d): number {
  if (typeof first !== 'object' || first === null) throw invalidArgument('first');
  if (typeof second !== 'object' || second === null) throw invalidArgument('second');
  return Math.hypot(
    requireFinite(second.x, 'second.x') - requireFinite(first.x, 'first.x'),
    requireFinite(second.y, 'second.y') - requireFinite(first.y, 'first.y'),
    requireFinite(second.z, 'second.z') - requireFinite(first.z, 'first.z'),
  );
}

/** Translate a point by a displacement. */
export function addPoints(point: Point3d, displacement: Vector3d): Point3d {
  if (typeof point !== 'object' || point === null) throw invalidArgument('point');
  if (typeof displacement !== 'object' || displacement === null) {
    throw invalidArgument('displacement');
  }
  return point3d(
    point.x + displacement.x,
    point.y + displacement.y,
    point.z + displacement.z,
  );
}

/**
 * Apply an affine transform to a point (row-vector convention, `p' = p * M`).
 *
 * The homogeneous `w` is fixed at 1: this is an affine matrix, not a projective
 * one, so there is no perspective divide to perform.
 */
export function transformPoint(matrix: Matrix3d, point: Point3d): Point3d {
  const values = requireMatrix(matrix);
  if (typeof point !== 'object' || point === null) throw invalidArgument('point');
  const x = requireFinite(point.x, 'point.x');
  const y = requireFinite(point.y, 'point.y');
  const z = requireFinite(point.z, 'point.z');
  return point3d(
    x * values[0] + y * values[4] + z * values[8] + values[12],
    x * values[1] + y * values[5] + z * values[9] + values[13],
    x * values[2] + y * values[6] + z * values[10] + values[14],
  );
}

/** The pure geometry library; no host, no bridge, no cache. */
export function createGeometryLibrary(): GeometryLibrary {
  return {
    point2d,
    point3d,
    vector2d,
    vector3d,
    matrix3dIdentity,
    matrix3dTranslate,
    matrix3dScale,
    matrix3dRotateZ,
    planeFromOriginNormal,
    distance3d,
    addPoints,
    transformPoint,
  };
}
