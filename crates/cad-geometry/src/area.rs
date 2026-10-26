//! Planar area measurement with explicit self-intersection handling (§3.3).
//!
//! Area is only defined for a coplanar, non-self-intersecting ring; anything
//! else is refused rather than returning a meaningless number.

use cad_domain::{Point3, WorkPlane};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AreaError {
    #[error("a polygon needs at least three distinct points, got {0}")]
    TooFewPoints(usize),
    #[error("the polygon is self-intersecting; area is undefined for this input")]
    SelfIntersecting,
    #[error("the polygon has zero area")]
    Degenerate,
    #[error("the polygon is not coplanar within the given tolerance")]
    NonPlanar,
    #[error(
        "the ring has {points} vertices but {bulges} bulges; one bulge per vertex is required"
    )]
    BulgesMismatch { points: usize, bulges: usize },
    #[error("the ring's plane cannot be determined from its vertices")]
    UndeterminedPlane,
}

/// Signed area via the shoelace formula on the XY projection (positive = CCW).
pub fn signed_area(pts: &[Point3]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    // Evaluate translation-invariant differences without modifying the source
    // coordinates. Products of absolute coordinates lose small local areas.
    let origin = pts[0];
    let mut acc = 0.0;
    for i in 0..n {
        let a = sub(pts[i], origin);
        let b = sub(pts[(i + 1) % n], origin);
        acc += a.x * b.y - b.x * a.y;
    }
    acc * 0.5
}

/// Measure the area of a user-selected closed polygon.
///
/// Rejects non-coplanar rings, self-intersections and non-finite input;
/// `planarity_tolerance` is a world-space value supplied by the caller's
/// [`cad_domain::TolerancePolicy`].
pub fn measure_polygon_area(pts: &[Point3], planarity_tolerance: f64) -> Result<f64, AreaError> {
    // A non-finite coordinate would make the shoelace sum NaN and could still
    // compare as "not degenerate"; refuse it up front (audit B24).
    if pts
        .iter()
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite())
    {
        return Err(AreaError::Degenerate);
    }
    let mut clean: Vec<Point3> = Vec::with_capacity(pts.len());
    for &p in pts {
        if clean.last().map(|l| dist2(*l, p) > 1e-24).unwrap_or(true) {
            clean.push(p);
        }
    }
    if clean.len() >= 2 {
        let first = clean[0];
        if dist2(*clean.last().unwrap(), first) <= 1e-24 {
            clean.pop();
        }
    }
    if clean.len() < 3 {
        return Err(AreaError::TooFewPoints(clean.len()));
    }
    // Coplanarity: all points must lie in one plane, checked against the first
    // three non-collinear points.
    if !is_coplanar(&clean, planarity_tolerance.max(1e-9)) {
        return Err(AreaError::NonPlanar);
    }
    if self_intersects(&clean) {
        return Err(AreaError::SelfIntersecting);
    }
    let a = signed_area(&clean).abs();
    if !a.is_finite() || a <= 1e-15 {
        return Err(AreaError::Degenerate);
    }
    Ok(a)
}

/// The exact area of a closed, bulged polyline ring plus the plane it was
/// measured in.
///
/// `plane.origin` is the ring's first vertex, `plane.u` the first non-degenerate
/// chord direction and `plane.v = normalize(normal × u)`, so `u × v` is exactly
/// the ring normal the area was measured against (a `u`/`v` pair in the ring's
/// own plane, never a projection onto world XY).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BulgedRingArea {
    /// The area in the ring plane's own units, always non-negative (± winding is
    /// not reported: area is a magnitude).
    pub area: f64,
    /// The ring's own plane (see the struct docs for the basis convention).
    pub plane: WorkPlane,
}

/// A ring normal is undetermined when its Newell vector is at most this fraction
/// of the ring's extent squared.
///
/// The Newell vector has units of area, so the test is **relative** to the ring's
/// own size: an absolute threshold would refuse a tiny but valid ring and accept a
/// huge collinear one (the same rule `cad-app`'s hatch boundary uses).
const RING_PLANE_RELATIVE_AREA_TOLERANCE: f64 = 1e-12;

/// A measured ring area at or below this fraction of its own extent squared is
/// reported as zero area rather than as a very small real area.
const RING_ZERO_AREA_RELATIVE_TOLERANCE: f64 = 1e-12;

/// Below this **signed included angle** (|θ| < 1e-3 rad) the circular-segment
/// area is evaluated by its series expansion: `θ − sin θ` loses every significant
/// digit to cancellation there, while the series stays exact to double precision.
const BULGE_SEGMENT_SERIES_RADIANS: f64 = 1e-3;

/// A chord direction whose cross product with the ring normal is at most this
/// length (|sin φ|, both unit vectors) is treated as parallel to the normal and
/// is not used as the `u` basis direction.
const RING_BASIS_PARALLEL_TOLERANCE: f64 = 1e-9;

/// Measure the **exact** area of a closed, bulged planar ring in its own plane.
///
/// This is the bulge-aware sibling of [`measure_polygon_area`], which is XY-only
/// and treats every segment as a straight chord. Here each segment's bulge is an
/// exact circular arc (`bulge = tan(θ/4)`, the DXF/LWPOLYLINE convention), so the
/// area is the chord shoelace **plus** one circular-segment area per segment:
/// `A_seg = c² (θ − sin θ) / (8 sin²(θ/2))` with the sign of `bulge`. No polygon
/// approximation and no chord sampling is involved.
///
/// `points` is the closed ring **without** a duplicated closing vertex and
/// `bulges[i]` is the bulge of segment `i → i+1` (the last one closes the ring),
/// exactly the `SemanticGeometry::Polyline` convention.
///
/// The ring plane comes from the vertices (Newell's method), with the same
/// `+Z`-for-XY-parallel rule the display tessellator uses
/// ([`crate::is_xy_parallel`]), so a bulge's sign means the same thing here as on
/// screen. Refusals (never a silently flattened or approximated number):
///
/// * [`AreaError::BulgesMismatch`] — one bulge per vertex is required;
/// * [`AreaError::Degenerate`] — a non-finite coordinate/bulge, or a non-finite
///   or zero area result;
/// * [`AreaError::UndeterminedPlane`] — fewer than three vertices, or vertices
///   that do not determine a winding (collinear points, or a self-cancelling one
///   such as a symmetric bowtie): the ring's plane — and therefore the sign frame
///   of every bulge — cannot be derived, so it is not guessed;
/// * [`AreaError::NonPlanar`] — a vertex off the ring plane by more than
///   `planarity_tolerance` (a world-space distance, supplied by the caller's
///   [`cad_domain::TolerancePolicy`]);
/// * [`AreaError::SelfIntersecting`] — the **chord** ring crosses itself. This is
///   a chord-level test: an arc that crosses a distant segment's arc is **not**
///   detected (the exact test needs arc/arc intersection, which is not
///   implemented here) — a recorded limitation, not a silent pass.
pub fn measure_bulged_ring_area(
    points: &[Point3],
    bulges: &[f64],
    planarity_tolerance: f64,
) -> Result<BulgedRingArea, AreaError> {
    if points.len() != bulges.len() {
        return Err(AreaError::BulgesMismatch {
            points: points.len(),
            bulges: bulges.len(),
        });
    }
    if points.iter().any(|p| !is_finite(*p)) || bulges.iter().any(|b| !b.is_finite()) {
        return Err(AreaError::Degenerate);
    }
    if points.len() < 3 {
        // Two vertices (for example a circle written as two semicircular bulge
        // segments) carry no plane: the sign frame of the bulges would have to be
        // invented. Refused, never assumed.
        return Err(AreaError::UndeterminedPlane);
    }
    let normal = ring_normal(points).ok_or(AreaError::UndeterminedPlane)?;
    // Coplanarity: a real world-space distance from the plane through `points[0]`.
    if points
        .iter()
        .any(|p| dot(sub(*p, points[0]), normal).abs() > planarity_tolerance)
    {
        return Err(AreaError::NonPlanar);
    }
    let (u, v) = ring_basis(points, normal).ok_or(AreaError::UndeterminedPlane)?;
    let flat: Vec<Point3> = points
        .iter()
        .map(|p| {
            let d = sub(*p, points[0]);
            Point3 {
                x: dot(d, u),
                y: dot(d, v),
                z: 0.0,
            }
        })
        .collect();
    if self_intersects(&flat) {
        return Err(AreaError::SelfIntersecting);
    }
    let mut signed = signed_area(&flat);
    let mut extent = 0.0_f64;
    for i in 0..flat.len() {
        let a = flat[i];
        let b = flat[(i + 1) % flat.len()];
        signed += bulge_segment_area(a, b, bulges[i]);
        extent = extent.max((b.x - a.x).abs()).max((b.y - a.y).abs());
    }
    let magnitude = signed.abs();
    if !magnitude.is_finite() {
        return Err(AreaError::Degenerate);
    }
    if magnitude <= RING_ZERO_AREA_RELATIVE_TOLERANCE * extent * extent {
        return Err(AreaError::Degenerate);
    }
    Ok(BulgedRingArea {
        area: magnitude,
        plane: WorkPlane {
            origin: points[0],
            u,
            v,
        },
    })
}

/// The ring's plane normal from its vertices (Newell's method).
///
/// `None` when the Newell vector is degenerate relative to the ring extent, i.e.
/// when the vertices do not determine a plane. A ring parallel to world XY uses
/// `+Z` (the DXF OCS sense the display and the fill sampler share), never the
/// winding sign.
fn ring_normal(points: &[Point3]) -> Option<Point3> {
    let mut acc = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        acc.x += (a.y - b.y) * (a.z + b.z);
        acc.y += (a.z - b.z) * (a.x + b.x);
        acc.z += (a.x - b.x) * (a.y + b.y);
    }
    let mut min = points[0];
    let mut max = points[0];
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    let span = (max.x - min.x).hypot(max.y - min.y).hypot(max.z - min.z);
    let length = len(acc);
    if !span.is_finite() || !length.is_finite() {
        return None;
    }
    if length <= RING_PLANE_RELATIVE_AREA_TOLERANCE * span * span {
        return None;
    }
    if crate::is_xy_parallel(points) {
        return Some(Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        });
    }
    Some(normalize(acc))
}

/// An in-plane orthonormal `(u, v)` basis with `u × v` equal to `normal`.
///
/// `u` is the first chord direction that is long enough to define it; `None` when
/// every chord is degenerate or when `u` is parallel to the normal (which a real
/// planar ring cannot be — it means the plane is not one).
fn ring_basis(points: &[Point3], normal: Point3) -> Option<(Point3, Point3)> {
    let mut u = None;
    for i in 0..points.len() {
        let chord = sub(points[(i + 1) % points.len()], points[i]);
        if len(chord) <= RING_PLANE_RELATIVE_AREA_TOLERANCE {
            continue;
        }
        let candidate = normalize(chord);
        // `u` must be **in** the ring plane: a chord parallel to the normal would
        // leave `v` undefined, so it is not used as the basis direction.
        if len(cross(normal, candidate)) <= RING_BASIS_PARALLEL_TOLERANCE {
            continue;
        }
        u = Some(candidate);
        break;
    }
    let u = u?;
    let v = normalize(cross(normal, u));
    Some((u, v))
}

/// The **signed** area between a chord and its bulge arc.
///
/// Positive for a positive bulge (the arc bulges to the right of the chord, i.e.
/// away from a counter-clockwise ring's interior). Exact: `A = c²(θ − sin θ) /
/// (8 sin²(θ/2))` with `θ = 4·atan(bulge)`, evaluated by its series for very small
/// angles (see [`BULGE_SEGMENT_SERIES_RADIANS`]).
fn bulge_segment_area(a: Point3, b: Point3, bulge: f64) -> f64 {
    let chord = (b.x - a.x).hypot(b.y - a.y);
    if chord <= 0.0 {
        return 0.0;
    }
    let theta = 4.0 * bulge.atan();
    if theta.abs() < BULGE_SEGMENT_SERIES_RADIANS {
        // A = c²θ/12·(1 + θ²/30) is the cancellation-free form of the limit
        // c²·bulge/3.
        return chord * chord * theta / 12.0 * (1.0 + theta * theta / 30.0);
    }
    let half_sin = (theta * 0.5).sin();
    chord * chord * (theta - theta.sin()) / (8.0 * half_sin * half_sin)
}

fn is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

fn is_coplanar(pts: &[Point3], tol: f64) -> bool {
    if pts.len() <= 3 {
        return true;
    }
    // Find a normal from the first non-degenerate triple.
    let mut normal = None;
    for i in 1..pts.len() - 1 {
        let n = cross(sub(pts[i], pts[0]), sub(pts[i + 1], pts[0]));
        if len(n) > tol {
            normal = Some(normalize(n));
            break;
        }
    }
    let Some(n) = normal else { return true };
    pts.iter()
        .all(|p| dot(sub(*p, pts[0]), n).abs() <= tol * 1000.0)
}

fn self_intersects(pts: &[Point3]) -> bool {
    let n = pts.len();
    for i in 0..n {
        let a1 = pts[i];
        let a2 = pts[(i + 1) % n];
        for j in 0..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let b1 = pts[j];
            let b2 = pts[(j + 1) % n];
            if properly_cross(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}

fn properly_cross(a1: Point3, a2: Point3, b1: Point3, b2: Point3) -> bool {
    let d1 = orient(b1, b2, a1);
    let d2 = orient(b1, b2, a2);
    let d3 = orient(a1, a2, b1);
    let d4 = orient(a1, a2, b2);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn orient(a: Point3, b: Point3, c: Point3) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn len(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: Point3) -> Point3 {
    let l = len(a);
    if l < 1e-24 {
        a
    } else {
        Point3 {
            x: a.x / l,
            y: a.y / l,
            z: a.z / l,
        }
    }
}

fn dist2(a: Point3, b: Point3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn unit_square_area() {
        let sq = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        assert!((measure_polygon_area(&sq, 1e-6).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bowtie_is_rejected() {
        let bowtie = [p(0.0, 0.0), p(1.0, 1.0), p(1.0, 0.0), p(0.0, 1.0)];
        assert_eq!(
            measure_polygon_area(&bowtie, 1e-6),
            Err(AreaError::SelfIntersecting)
        );
    }

    #[test]
    fn non_planar_ring_is_rejected() {
        let ring = [
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 5.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        assert_eq!(measure_polygon_area(&ring, 1e-6), Err(AreaError::NonPlanar));
    }

    // -----------------------------------------------------------------------
    // Bulge-aware ring area (`AREA OBJECT`'s polyline path)
    // -----------------------------------------------------------------------

    const PLANARITY: f64 = 1e-9;

    fn closed(ring: &[Point3], bulges: &[f64]) -> Result<BulgedRingArea, AreaError> {
        measure_bulged_ring_area(ring, bulges, PLANARITY)
    }

    #[test]
    fn flat_bulged_rectangle_equals_its_shoelace_area() {
        // 3 x 4 rectangle, counter-clockwise, no bulges.
        let ring = [p(0.0, 0.0), p(3.0, 0.0), p(3.0, 4.0), p(0.0, 4.0)];
        let measured = closed(&ring, &[0.0; 4]).expect("a flat closed ring");
        assert!((measured.area - 12.0).abs() < 1e-12, "{measured:?}");
        // The reported plane is the ring's own: origin at the first vertex and a
        // right-handed in-plane basis, so u x v is the ring normal (+Z here).
        assert_eq!(measured.plane.origin, p(0.0, 0.0));
        assert!((measured.plane.u.x - 1.0).abs() < 1e-12);
        assert!((measured.plane.v.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_clockwise_ring_reports_the_same_magnitude() {
        let ccw = [p(0.0, 0.0), p(3.0, 0.0), p(3.0, 4.0), p(0.0, 4.0)];
        let cw = [p(0.0, 0.0), p(0.0, 4.0), p(3.0, 4.0), p(3.0, 0.0)];
        let a = closed(&ccw, &[0.0; 4]).unwrap().area;
        let b = closed(&cw, &[0.0; 4]).unwrap().area;
        // The winding must not change a magnitude, and the XY-parallel rule keeps
        // the bulge frame at +Z for both.
        assert!(
            (a - 12.0).abs() < 1e-12 && (b - 12.0).abs() < 1e-12,
            "{a} {b}"
        );
    }

    #[test]
    fn a_semicircular_bulge_cap_adds_the_exact_segment_area() {
        // 3 x 4 rectangle whose first edge (0,0)->(3,0) is a semicircle bulging
        // **outside** the ring's interior: bulge = 1 gives theta = pi over a chord
        // of 3, so r = 1.5 and the segment area is (r^2/2)(pi - sin pi) = pi*2.25/2.
        let ring = [p(0.0, 0.0), p(3.0, 0.0), p(3.0, 4.0), p(0.0, 4.0)];
        let measured = closed(&ring, &[1.0, 0.0, 0.0, 0.0]).expect("a bulged ring");
        let expected = 12.0 + std::f64::consts::PI * 2.25 / 2.0;
        assert!(
            (measured.area - expected).abs() < 1e-12,
            "{} vs {expected}",
            measured.area
        );
        // A negative bulge on the same edge cuts the same area out of the ring.
        let inward = closed(&ring, &[-1.0, 0.0, 0.0, 0.0]).expect("a bulged ring");
        assert!(
            (inward.area - (12.0 - std::f64::consts::PI * 2.25 / 2.0)).abs() < 1e-12,
            "{}",
            inward.area
        );
    }

    #[test]
    fn a_two_segment_circle_ring_is_refused_as_an_undetermined_plane() {
        // Two vertices with bulge 1 each is a circle on screen, but the vertices
        // alone define no plane (and therefore no bulge sign frame).
        assert_eq!(
            closed(&[p(0.0, 0.0), p(2.0, 0.0)], &[1.0, 1.0]),
            Err(AreaError::UndeterminedPlane)
        );
        // Collinear vertices do not determine a plane either.
        assert_eq!(
            closed(&[p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)], &[0.0; 3]),
            Err(AreaError::UndeterminedPlane)
        );
    }

    #[test]
    fn a_tilted_ring_measures_in_its_own_plane() {
        // The 3 x 4 rectangle in the plane x = 5 (normal +X) has the same area.
        let ring = [
            Point3 {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 5.0,
                y: 3.0,
                z: 0.0,
            },
            Point3 {
                x: 5.0,
                y: 3.0,
                z: 4.0,
            },
            Point3 {
                x: 5.0,
                y: 0.0,
                z: 4.0,
            },
        ];
        let measured = closed(&ring, &[0.0; 4]).expect("a tilted planar ring");
        assert!((measured.area - 12.0).abs() < 1e-12, "{measured:?}");
        // The plane is the ring's own, not world XY: the normal is +/-X.
        let normal = cross(measured.plane.u, measured.plane.v);
        assert!(
            normal.y.abs() < 1e-12 && normal.z.abs() < 1e-12,
            "{normal:?}"
        );
        assert!((normal.x.abs() - 1.0).abs() < 1e-12, "{normal:?}");
    }

    #[test]
    fn a_non_planar_ring_and_a_bulge_count_mismatch_are_refused() {
        let off_plane = [
            p(0.0, 0.0),
            p(3.0, 0.0),
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.5,
            },
            p(0.0, 4.0),
        ];
        assert_eq!(closed(&off_plane, &[0.0; 4]), Err(AreaError::NonPlanar));
        let flat = [p(0.0, 0.0), p(3.0, 0.0), p(3.0, 4.0), p(0.0, 4.0)];
        assert_eq!(
            closed(&flat, &[0.0; 3]),
            Err(AreaError::BulgesMismatch {
                points: 4,
                bulges: 3
            })
        );
        // A ring that crosses itself **and** still has a defined plane (a
        // symmetric bowtie has a zero Newell vector, so it is refused earlier as
        // `UndeterminedPlane` — also honest, since it has no winding to speak of).
        let crossed = [p(0.0, 0.0), p(4.0, 0.0), p(0.0, 3.0), p(3.0, 3.0)];
        assert_eq!(
            closed(&crossed, &[0.0; 4]),
            Err(AreaError::SelfIntersecting)
        );
        let bowtie = [p(0.0, 0.0), p(1.0, 1.0), p(1.0, 0.0), p(0.0, 1.0)];
        assert_eq!(
            closed(&bowtie, &[0.0; 4]),
            Err(AreaError::UndeterminedPlane)
        );
    }

    #[test]
    fn a_tiny_bulge_keeps_its_area_through_the_series_branch() {
        // bulge = 1e-5 is far inside the series threshold; the analytic limit for
        // a small bulge is c^2 * b / 3, so a unit chord adds ~3.3e-6.
        let ring = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let measured = closed(&ring, &[1e-5, 0.0, 0.0, 0.0]).unwrap();
        let expected = 1.0 + 1e-5 / 3.0;
        assert!(
            (measured.area - expected).abs() < 1e-15,
            "{} vs {expected}",
            measured.area
        );
    }
}
