//! Neutral B-rep data model: placement, analytic surfaces, curves, loops,
//! faces, shells and the per-body container.
//!
//! Every type here is plain data crossing the kernel seam: no ACIS handle, no
//! SAT/SAB parser and no CAD-package type appears. It lives in `cad-domain`
//! (rather than in `cad-kernel-adapter`) because a body's exact geometry is
//! domain data, not a kernel-implementation detail; the adapter owns only the
//! evaluation and tessellation seams that consume it.

use crate::{Point3, Transform3};

/// Unit vector in the direction of `p`; a vanished vector normalises to zero
/// rather than to NaN.
fn normalize(p: Point3) -> Point3 {
    let length = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
    if length > 1e-12 {
        Point3 {
            x: p.x / length,
            y: p.y / length,
            z: p.z / length,
        }
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// A body placement in the ACIS convention `world = scale·(p·M) + t`.
///
/// `matrix` is row-major exactly as `SatDocument::placement` returns it, so the
/// same numbers survive the seam unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrepPlacement {
    pub matrix: [[f64; 3]; 3],
    pub translation: Point3,
    pub scale: f64,
}

impl BrepPlacement {
    pub fn identity() -> Self {
        BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: 1.0,
        }
    }

    /// Whether this placement is the identity (no transform work needed).
    pub fn is_identity(&self) -> bool {
        self.scale == 1.0
            && self.translation.x == 0.0
            && self.translation.y == 0.0
            && self.translation.z == 0.0
            && self.matrix == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    /// Apply the placement to a point (ACIS row-vector convention).
    pub fn apply(&self, p: Point3) -> Point3 {
        let m = &self.matrix;
        let t = self.translation;
        let s = self.scale;
        Point3 {
            x: s * (p.x * m[0][0] + p.y * m[1][0] + p.z * m[2][0]) + t.x,
            y: s * (p.x * m[0][1] + p.y * m[1][1] + p.z * m[2][1]) + t.y,
            z: s * (p.x * m[0][2] + p.y * m[1][2] + p.z * m[2][2]) + t.z,
        }
    }

    /// Rotate a direction by the placement's linear part and renormalise.
    pub fn apply_direction(&self, d: Point3) -> Point3 {
        let m = &self.matrix;
        normalize(Point3 {
            x: d.x * m[0][0] + d.y * m[1][0] + d.z * m[2][0],
            y: d.x * m[0][1] + d.y * m[1][1] + d.z * m[2][1],
            z: d.x * m[0][2] + d.y * m[1][2] + d.z * m[2][2],
        })
    }
}

/// The analytic supporting surface of a face.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepSurface {
    /// A plane through `origin` with `normal`; `u_dir` fixes the in-plane frame.
    Plane {
        origin: Point3,
        normal: Point3,
        u_dir: Point3,
    },
    /// A sphere; loop-less faces cover the full parameter domain.
    Sphere {
        center: Point3,
        radius: f64,
        u_dir: Point3,
        pole: Point3,
    },
    /// A torus whose axis is `normal`; loop-less faces cover the full domain.
    Torus {
        center: Point3,
        normal: Point3,
        major_radius: f64,
        minor_radius: f64,
        u_dir: Point3,
    },
    /// A circular cylinder. Side faces are bounded by two full circles.
    Cylinder {
        origin: Point3,
        axis: Point3,
        ref_dir: Point3,
        radius: f64,
    },
    /// A circular cone (or truncated cone). `radius` is the reference
    /// cross-section radius at `origin`; the cross-section radius grows with
    /// axial distance at `slope = sin_half_angle / cos_half_angle`. A side face
    /// is bounded by one full circle plus a degenerate apex loop (full cone) or
    /// by two full circles (frustum).
    Cone {
        origin: Point3,
        axis: Point3,
        ref_dir: Point3,
        radius: f64,
        sin_half_angle: f64,
        cos_half_angle: f64,
    },
    /// A **form** surface: a tensor-product rational B-spline (NURBS), carried as
    /// the record states it.
    ///
    /// `points[i][j]` is the homogeneous control coefficient at `(u_i, v_j)`,
    /// written `[x·w, y·w, z·w, w]`; the control point is `(x, y, z)` and `w` its
    /// weight. Knots are the producer's own, unmodified, and the **degree is
    /// implicit**: `u_knots.len() - points.len() - 1` in `u`,
    /// `v_knots.len() - points[0].len() - 1` in `v`.
    ///
    /// This is a *form*, not an analytic label: it says "here is the net", never
    /// "this is really a cylinder". A face that has an analytic name keeps the
    /// analytic variant above (that is what lets display and export stay exact);
    /// no consumer may read a `Nurbs` back as a known surface type.
    ///
    /// Every weight must be finite and strictly positive — a non-positive weight
    /// is not a point of a surface. The record's structural limits on the net
    /// (2…4096 control points per direction, degree 1…16,
    /// `knots.len() == count + degree + 1`, non-decreasing knots, a non-empty
    /// parameter span, no ragged rows) are **not** re-asserted by this type: they
    /// are validated where a neutral body is written back out (the `solidcore`
    /// record lift), which is the only consumer that can tell a malformed net
    /// from an unusual one.
    ///
    /// There is deliberately **no periodicity flag**. Closure is a property of the
    /// control net and its knots (the first and last row/column coincide), never
    /// something this model declares; a consumer that needs to know whether a
    /// direction wraps must look at the geometry.
    Nurbs {
        u_knots: Vec<f64>,
        v_knots: Vec<f64>,
        points: Vec<Vec<[f64; 4]>>,
    },
    /// A surface this build cannot evaluate. Reported as a missing face.
    Unsupported { type_key: String },
}

/// A curve segment of a loop, already oriented in loop direction.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepCurve {
    Line {
        start: Point3,
        end: Point3,
    },
    /// A full circle. `u_dir` is the radius direction at parameter zero.
    Circle {
        center: Point3,
        normal: Point3,
        u_dir: Point3,
        radius: f64,
    },
    /// A circular arc: a proper sub-span of the circle through `start` and `end`.
    ///
    /// `(u_dir, normal)` define the plane frame (`u_dir` lies in the plane,
    /// `normal = u_dir × v_dir` is its right-handed normal, and `v_dir` is the
    /// in-plane direction a quarter turn counter-clockwise from `u_dir`). The
    /// pair is **oriented**: the traversal from `start` to `end` runs
    /// **counter-clockwise about `normal`**, so the sweep angle is in the open
    /// interval `(0, 2π)` and is never ambiguous.
    ///
    /// The same two points on the same circle describe two different arcs, and
    /// which one this is depends on the orientation exactly as follows (sweep
    /// `σ` is the counter-clockwise turn `start → end` about `normal`):
    ///
    /// | endpoints | `normal` | arc |
    /// |---|---|---|
    /// | `start → end` | `+n` | the arc of sweep `σ` |
    /// | `end → start` | `+n` | the **complementary** arc, sweep `2π − σ` |
    /// | `start → end` | `−n` | the **complementary** arc, sweep `2π − σ` |
    /// | `end → start` | `−n` | *this* arc, traversed the other way (sweep `σ`) |
    ///
    /// So reversing **either** the endpoints **or** the normal selects the
    /// complementary arc, while reversing **both** keeps the same point set and
    /// only turns the traversal around — that is what a neighbouring face does
    /// when it shares the edge in the opposite loop direction.
    /// `BrepCurve::Circle` is the closed case and is not an arc; this variant is
    /// a **proper** sub-span, so a producer must never encode a zero-length or
    /// full-turn "arc".
    ///
    /// `start` and `end` are points on the circle (not angles) and satisfy
    /// `|start - center| = |end - center| = radius`; `radius` is finite and
    /// strictly positive. They are the authoritative chain points, so adjacent
    /// curves in a loop share the exact same coordinates.
    Arc {
        center: Point3,
        normal: Point3,
        u_dir: Point3,
        radius: f64,
        start: Point3,
        end: Point3,
    },
    /// A curve this build cannot evaluate; the owning face is unsupported.
    Unsupported {
        type_key: String,
    },
}

/// A connected, closed loop of curves in loop order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepLoop {
    pub edges: Vec<BrepCurve>,
}

/// A face: a supporting surface plus its boundary loops.
#[derive(Debug, Clone, PartialEq)]
pub struct BrepFace {
    /// Source face record index (or a stable sequential id). Reported on failure.
    pub id: u32,
    pub surface: BrepSurface,
    /// `true` when the face sense is reversed relative to the surface normal.
    pub reversed: bool,
    /// Boundary loops; outer vs. inner is derived geometrically at tessellation.
    pub loops: Vec<BrepLoop>,
    /// The producer's `(u, v)` box for this face — `[[u0, u1], [v0, v1]]` **in the
    /// face's own surface parameters** — when it states one.
    ///
    /// `None` means "unspecified": a writer may derive the box from the boundary
    /// (that is what the record writer does for a plane). A
    /// [`BrepSurface::Nurbs`] face must carry `Some`: the neutral model never
    /// reparameterises a form surface, so a writer cannot invent a box for one and
    /// must refuse it by name instead.
    ///
    /// This is a **parameter** rectangle, not a length: a similarity placement of
    /// the geometry (`record::bake`, `transform`) leaves it untouched.
    pub domain: Option<[[f64; 2]; 2]>,
    /// The producer's own tolerance for this face, in **model** units (a length).
    ///
    /// `None` means **unspecified**: the producer did not state one, and a
    /// consumer must not invent a number for it. When `Some`, the value is
    /// finite and strictly positive.
    ///
    /// Model units are the units of the shell coordinates, i.e. *before* the
    /// body's [`BrepPlacement`] maps them to world. A transform that only
    /// composes a placement therefore leaves it alone; baking a placement into
    /// the geometry (the record lift) scales it by the placement's uniform
    /// factor, exactly like a coordinate.
    pub tolerance: Option<f64>,
}

/// A connected shell of faces.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepShell {
    pub id: u32,
    pub faces: Vec<BrepFace>,
}

/// Neutral B-rep for one body.
///
/// `tolerance` is the producer's own modelling/intersection tolerance, in **model**
/// units (a length; the units of the shell coordinates, before `placement` maps
/// them to world), or `None` when the source stated none. A transform that only
/// composes a placement leaves it alone; baking the placement into geometry
/// scales it by the placement's uniform factor. It is **not** a display tolerance
/// and is never fabricated: `Default` leaves it `None`, which is the deliberate
/// "unspecified" state rather than a made-up default value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepData {
    pub shells: Vec<BrepShell>,
    pub placement: Option<BrepPlacement>,
    pub tolerance: Option<f64>,
}

impl BrepData {
    /// Total face count across all shells.
    pub fn face_count(&self) -> usize {
        self.shells.iter().map(|s| s.faces.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.face_count() == 0
    }
}

impl BrepPlacement {
    /// Compose this placement with `transform`: a model point is mapped by
    /// `self` and then by `transform` (`world' = transform(world)`).
    ///
    /// The result is only geometrically meaningful when `transform` is a
    /// similarity (uniform scale, rotation and/or reflection, translation);
    /// callers must check [`Transform3::is_uniform_scale`] first. A
    /// non-similarity would fold a shear into `matrix`, which this placement
    /// does not represent.
    pub fn composed_with(&self, transform: &Transform3) -> BrepPlacement {
        let m = &transform.matrix;
        let column = |i: usize| (m[0][i] * m[0][i] + m[1][i] * m[1][i] + m[2][i] * m[2][i]).sqrt();
        // Mean column length; only read after `is_uniform_scale` accepted the
        // matrix, so the three columns agree.
        let scale = ((column(0) + column(1) + column(2)) / 3.0).max(1e-12);
        let mut matrix = [[0.0f64; 3]; 3];
        for (row, out_row) in matrix.iter_mut().enumerate() {
            for (out_column, cell) in out_row.iter_mut().enumerate() {
                // `R = L / scale`, so `M'[row][col] = Σ M[row][k]·R[col][k]`.
                *cell = (0..3)
                    .map(|k| self.matrix[row][k] * (m[out_column][k] / scale))
                    .sum();
            }
        }
        let rotated = transform.apply_vector(self.translation);
        BrepPlacement {
            matrix,
            translation: Point3 {
                x: rotated.x + m[0][3],
                y: rotated.y + m[1][3],
                z: rotated.z + m[2][3],
            },
            scale: self.scale * scale,
        }
    }
}

#[cfg(test)]
mod arc_orientation_tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn axis_z() -> Point3 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    }

    fn negate(a: Point3) -> Point3 {
        Point3 {
            x: -a.x,
            y: -a.y,
            z: -a.z,
        }
    }

    fn dot(a: Point3, b: Point3) -> f64 {
        a.x * b.x + a.y * b.y + a.z * b.z
    }

    fn cross(a: Point3, b: Point3) -> Point3 {
        Point3 {
            x: a.y * b.z - a.z * b.y,
            y: a.z * b.x - a.x * b.z,
            z: a.x * b.y - a.y * b.x,
        }
    }

    fn close(a: Point3, b: Point3) -> bool {
        (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12 && (a.z - b.z).abs() < 1e-12
    }

    /// The `Arc` contract read literally: the counter-clockwise turn `start → end`
    /// about `normal`, measured in the `(u_dir, normal × u_dir)` frame.
    fn sweep(start: Point3, end: Point3, normal: Point3, u_dir: Point3) -> f64 {
        let v_dir = cross(normal, u_dir);
        let angle = |q: Point3| dot(q, v_dir).atan2(dot(q, u_dir));
        let delta = angle(end) - angle(start);
        if delta > 0.0 {
            delta
        } else {
            delta + std::f64::consts::TAU
        }
    }

    /// The arc's midpoint (half the sweep from `start`), on the unit circle about
    /// the origin. It is the same geometric point whichever way the arc is walked,
    /// which is what tells "the same arc reversed" from "the complementary arc".
    fn midpoint(start: Point3, end: Point3, normal: Point3, u_dir: Point3) -> Point3 {
        let v_dir = cross(normal, u_dir);
        let angle = |q: Point3| dot(q, v_dir).atan2(dot(q, u_dir));
        let mid = angle(start) + 0.5 * sweep(start, end, normal, u_dir);
        // A point of the frame, not of the world xy axes: `v_dir` flips with the
        // normal, which is the whole point of the test.
        p(
            mid.cos() * u_dir.x + mid.sin() * v_dir.x,
            mid.cos() * u_dir.y + mid.sin() * v_dir.y,
        )
    }

    /// The four orientation combinations the `Arc` doc pins, on a quarter arc
    /// (whose sweep distinguishes the two arcs) and on the reviewer's half arc
    /// (whose sweep is `π` either way, so only the midpoint distinguishes them).
    #[test]
    fn reversing_either_the_endpoints_or_the_normal_gives_the_complementary_arc() {
        let z = axis_z();
        let x_dir = p(1.0, 0.0);
        let quarter = std::f64::consts::FRAC_PI_2;
        let start = p(1.0, 0.0);
        let end = p(0.0, 1.0);

        // (1) as written: a quarter turn from +x to +y about +z.
        assert!((sweep(start, end, z, x_dir) - quarter).abs() < 1e-12);
        let upper = midpoint(start, end, z, x_dir);
        assert!(
            close(
                upper,
                p(
                    std::f64::consts::FRAC_1_SQRT_2,
                    std::f64::consts::FRAC_1_SQRT_2
                )
            ),
            "{upper:?}"
        );

        // (2) endpoints reversed, normal unchanged: the complementary arc.
        let complement = sweep(end, start, z, x_dir);
        assert!((complement - 3.0 * quarter).abs() < 1e-12, "{complement}");
        let by_endpoints = midpoint(end, start, z, x_dir);
        assert!(!close(by_endpoints, upper), "a different point set");

        // (3) normal reversed, endpoints unchanged: the *same* complementary arc.
        let by_normal = sweep(start, end, negate(z), x_dir);
        assert!((by_normal - 3.0 * quarter).abs() < 1e-12, "{by_normal}");
        assert!(
            close(midpoint(start, end, negate(z), x_dir), by_endpoints),
            "flipping either the endpoints or the normal names the same complementary arc"
        );

        // (4) both reversed: this arc, traversed the other way (same point set).
        let same = sweep(end, start, negate(z), x_dir);
        assert!((same - quarter).abs() < 1e-12, "{same}");
        assert!(
            close(midpoint(end, start, negate(z), x_dir), upper),
            "reversing both keeps the point set and only turns the traversal around"
        );
    }

    #[test]
    fn a_half_arc_is_the_upper_or_lower_half_by_its_midpoint() {
        let z = axis_z();
        let x_dir = p(1.0, 0.0);
        let start = p(1.0, 0.0);
        let end = p(-1.0, 0.0);
        // Every combination sweeps π, so the midpoint says which half it is.
        for (s, e, n, want) in [
            (start, end, z, p(0.0, 1.0)),
            (end, start, z, p(0.0, -1.0)),
            (start, end, negate(z), p(0.0, -1.0)),
            (end, start, negate(z), p(0.0, 1.0)),
        ] {
            assert!((sweep(s, e, n, x_dir) - std::f64::consts::PI).abs() < 1e-12);
            let mid = midpoint(s, e, n, x_dir);
            assert!(close(mid, want), "{mid:?} vs {want:?}");
        }
    }
}
