use std::collections::HashMap;

use cad_domain::*;

use super::math::{add, dot3, is_finite_point, length3, scale3, sub};

pub(super) type PickBoundsCache = HashMap<*const SemanticGeometry, Option<(Point3, Point3)>>;

#[cfg(not(test))]
pub(super) fn bounds_culling_enabled() -> bool {
    true
}

#[cfg(test)]
pub(super) fn bounds_culling_enabled() -> bool {
    !super::BYPASS_PICK_BOUNDS.with(std::cell::Cell::get)
}

/// A deliberately conservative, low-cost local bound for families whose
/// geometry is known to remain within it. Returns `None` for uncertain data.
pub(super) fn local_pick_bounds(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
    let mut bounds: Option<(Point3, Point3)> = None;
    let mut radius = 0.0;
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !include_bound_point(&mut bounds, *start) || !include_bound_point(&mut bounds, *end)
            {
                return None;
            }
        }
        SemanticGeometry::Point(point) => {
            if !include_bound_point(&mut bounds, *point) {
                return None;
            }
        }
        SemanticGeometry::Shape { position, .. } => {
            if !include_bound_point(&mut bounds, *position) {
                return None;
            }
        }
        // The triangle predicate intentionally permits 1e-12 barycentric
        // slack; for very large triangles that can accept points outside even
        // a tolerance-expanded vertex bound. Preserve that established result.
        SemanticGeometry::Mesh(_) => return None,
        SemanticGeometry::Polyline {
            points: vertices,
            bulges,
            ..
        } if bulges.iter().all(|bulge| *bulge == 0.0) => {
            for point in vertices {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        SemanticGeometry::Mask { boundary, .. } => {
            for point in boundary {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        SemanticGeometry::Circle {
            center, radius: r, ..
        }
        | SemanticGeometry::Arc {
            center, radius: r, ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            radius = r.abs();
        }
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            radius = length3(*major_axis) * (1.0 + ratio.abs());
        }
        // A dimension's bound is the union of its display children, exactly
        // like a compound (ADR 0017 A.8): a child with no provable bound makes
        // the whole dimension unbounded, matching the compound rule.
        SemanticGeometry::Dimension(dimension) => {
            if dimension.display().is_empty() {
                return None;
            }
            for child in dimension.display() {
                let (child_min, child_max) = local_pick_bounds(child)?;
                bounds = Some(match bounds {
                    Some((min, max)) => (
                        Point3 {
                            x: min.x.min(child_min.x),
                            y: min.y.min(child_min.y),
                            z: min.z.min(child_min.z),
                        },
                        Point3 {
                            x: max.x.max(child_max.x),
                            y: max.y.max(child_max.y),
                            z: max.z.max(child_max.z),
                        },
                    ),
                    None => (child_min, child_max),
                });
            }
        }
        // A hatch keeps its precise boundary bound (ADR 0018 A.2): the union of
        // all loop vertices in the hatch plane. This is tighter than the
        // display-children union (whose fill mesh has no provable leaf bound)
        // and never falls back to the item AABB. A bulged loop returns `None`
        // rather than a non-conservative chord bound.
        SemanticGeometry::Hatch(hatch) => {
            let (min, max) = hatch.local_bounds()?;
            bounds = Some((min, max));
        }
        // Rational splines can leave the control-point hull for invalid or
        // non-positive weights; text bounds depend on shaping/font metrics.
        // Bulged polylines likewise need their true arc extrema.
        _ => return None,
    }
    let (min, max) = bounds?;
    if !radius.is_finite() {
        return None;
    }
    Some((
        Point3 {
            x: min.x - radius,
            y: min.y - radius,
            z: min.z - radius,
        },
        Point3 {
            x: max.x + radius,
            y: max.y + radius,
            z: max.z + radius,
        },
    ))
}

fn include_bound_point(bounds: &mut Option<(Point3, Point3)>, point: Point3) -> bool {
    if !is_finite_point(point) {
        return false;
    }
    match bounds {
        Some((min, max)) => {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
        None => *bounds = Some((point, point)),
    }
    true
}

pub(super) fn cached_local_pick_bounds(
    geometry: &SemanticGeometry,
    cache: &mut PickBoundsCache,
) -> Option<(Point3, Point3)> {
    // A hatch's precise bound is a single O(vertices) pass over its loops
    // (`HatchGeometry::local_bounds`), not an aggregate of display children, so
    // it is cached on its own key rather than through the child union below. It
    // is cached at all for parity with its peers: a hatch replaces the compound
    // it used to be (P6a-1), and a shared placement should not recompute it.
    if let SemanticGeometry::Hatch(hatch) = geometry {
        let key = geometry as *const SemanticGeometry;
        if let Some(bounds) = cache.get(&key) {
            return *bounds;
        }
        #[cfg(test)]
        super::PICK_BOUNDS_COMPUTED.with(|count| count.set(count.get() + 1));
        let bounds = hatch.local_bounds();
        cache.insert(key, bounds);
        return bounds;
    }
    // A dimension aggregates its display children like a compound, so it uses
    // the same cached-union path (ADR 0017 A.8).
    let children: &[SemanticGeometry] = match geometry {
        SemanticGeometry::Compound(children) => children,
        SemanticGeometry::Dimension(dimension) => dimension.display(),
        // Leaf bounds are cheap and common. Avoid hashing and inserting millions
        // of one-shot leaf pointers; only reusable aggregates (compound,
        // dimension, hatch) belong in the query cache.
        _ => return local_pick_bounds(geometry),
    };
    let key = geometry as *const SemanticGeometry;
    if let Some(bounds) = cache.get(&key) {
        return *bounds;
    }
    #[cfg(test)]
    super::PICK_BOUNDS_COMPUTED.with(|count| count.set(count.get() + 1));

    let mut union: Option<(Point3, Point3)> = None;
    let mut all_bounded = !children.is_empty();
    for child in children {
        let Some((child_min, child_max)) = cached_local_pick_bounds(child, cache) else {
            all_bounded = false;
            break;
        };
        union = Some(match union {
            Some((mut min, mut max)) => {
                min.x = min.x.min(child_min.x);
                min.y = min.y.min(child_min.y);
                min.z = min.z.min(child_min.z);
                max.x = max.x.max(child_max.x);
                max.y = max.y.max(child_max.y);
                max.z = max.z.max(child_max.z);
                (min, max)
            }
            None => (child_min, child_max),
        });
    }
    let bounds = if all_bounded { union } else { None };
    cache.insert(key, bounds);
    bounds
}

pub(super) fn conservative_world_bounds(
    local_bounds: Option<(Point3, Point3)>,
    transform: &Transform3,
) -> Option<(Point3, Point3)> {
    let (min, max) = local_bounds?;
    let mut world_min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut world_max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let point = transform.apply_point(Point3 { x, y, z });
                if !is_finite_point(point) {
                    return None;
                }
                world_min.x = world_min.x.min(point.x);
                world_min.y = world_min.y.min(point.y);
                world_min.z = world_min.z.min(point.z);
                world_max.x = world_max.x.max(point.x);
                world_max.y = world_max.y.max(point.y);
                world_max.z = world_max.z.max(point.z);
            }
        }
    }
    Some((world_min, world_max))
}

/// A conservative world-space AABB for a pick item, for the broad-phase index.
///
/// This is deliberately **not** [`local_pick_bounds`]: that predicate returns
/// `None` for every family whose precise test is not provably contained in a
/// cheap box (meshes, splines, bulged polylines, text), so a broad phase built
/// on it would fall back to a precise test for those families and never narrow
/// the scan. Here every family that *can* be bounded conservatively is, and
/// only genuinely unbounded families (opaque payloads, unexpanded inserts, a
/// compound with an unbounded child) return `None`.
///
/// # Why each bound is conservative
///
/// * line / point / mask / text: the pickable geometry is exactly the vertices
///   (text tessellates to the two placeholder-box corners, matching
///   `BoundsAccumulator`), so the vertex box contains it;
/// * polyline with bulges: each bulge arc stays within its chord box expanded
///   by the arc's sagitta `|bulge| * |chord| / 2`, accumulated per segment;
/// * circle / arc / ellipse / shape: a sphere/box about the centre that
///   contains the whole curve;
/// * spline: a NURBS curve with finite positive weights lies in the convex hull
///   of its control points (and `NurbsCurve::new` rejects any other weight), and
///   the fallback path returns the control points themselves, so the
///   control-point box contains the tessellation;
/// * mesh: the vertex box, plus a relative slack that covers the triangle
///   predicate's `1e-12` barycentric acceptance slack;
/// * compound: the union of its children's bounds, or `None` when any child is
///   unbounded (an opaque child must still be offered so its `skipped` entry is
///   not lost).
///
/// The transform is applied by [`conservative_world_bounds`], which hulls the
/// eight transformed corners, so a rotated/scaled instance still fits.
pub fn pick_world_bounds(
    geometry: &SemanticGeometry,
    transform: &Transform3,
) -> Option<(Point3, Point3)> {
    let local = index_local_bounds(geometry)?;
    let (min, max) = conservative_world_bounds(Some(local), transform)?;
    // A tiny relative slack covers the mesh triangle predicate's `1e-12`
    // barycentric slack and any last-bit float disagreement, so the broad phase
    // can never reject a hit the precise stage would accept. It only ever adds
    // candidates, so the `PickReport` is unchanged.
    let slack = 1e-9
        * (max.x - min.x)
            .abs()
            .max((max.y - min.y).abs())
            .max((max.z - min.z).abs());
    if !slack.is_finite() {
        return None;
    }
    Some((
        Point3 {
            x: min.x - slack,
            y: min.y - slack,
            z: min.z - slack,
        },
        Point3 {
            x: max.x + slack,
            y: max.y + slack,
            z: max.z + slack,
        },
    ))
}

/// The local (untransformed) conservative box behind [`pick_world_bounds`].
fn index_local_bounds(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
    let mut bounds: Option<(Point3, Point3)> = None;
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !include_bound_point(&mut bounds, *start) || !include_bound_point(&mut bounds, *end)
            {
                return None;
            }
        }
        SemanticGeometry::Point(point) => {
            if !include_bound_point(&mut bounds, *point) {
                return None;
            }
        }
        // The precise stage tests a SHAPE's anchor point only.
        SemanticGeometry::Shape { position, .. } => {
            if !include_bound_point(&mut bounds, *position) {
                return None;
            }
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.is_empty() {
                return None;
            }
            for point in points {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
            // A bulge replaces its chord with a circular arc. The arc is built
            // in the polyline's best-fit plane (matching the tessellator), so
            // for a non-planar 3D polyline the arc can leave the chord box by
            // far more than the sagitta; bound it by the ball around the arc's
            // centre instead, which contains the whole arc for any plane.
            let count = points.len();
            let last = if *closed {
                count
            } else {
                count.saturating_sub(1)
            };
            let plane_normal = polyline_plane_normal(points);
            for i in 0..last {
                let a = points[i];
                let b = points[(i + 1) % count];
                let bulge = bulges.get(i).copied().unwrap_or(0.0);
                if bulge == 0.0 {
                    continue;
                }
                let (min, max) = bulge_arc_bounds(a, b, bulge, plane_normal)?;
                include_bound_box(&mut bounds, min, max)?;
            }
        }
        SemanticGeometry::Mask { boundary, .. } => {
            for point in boundary {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        SemanticGeometry::Circle {
            center, radius: r, ..
        }
        | SemanticGeometry::Arc {
            center, radius: r, ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            let radius = r.abs();
            if !radius.is_finite() {
                return None;
            }
            let (min, max) = bounds?;
            bounds = Some(expand_box(min, max, radius));
        }
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            let radius = length3(*major_axis) * (1.0 + ratio.abs());
            if !radius.is_finite() {
                return None;
            }
            let (min, max) = bounds?;
            bounds = Some(expand_box(min, max, radius));
        }
        // A NURBS curve with finite positive weights lies in the convex hull of
        // its control points; the fallback path returns the control points.
        SemanticGeometry::Spline { control_points, .. } => {
            if control_points.is_empty() {
                return None;
            }
            for point in control_points {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        // A hatch keeps its precise boundary bound (ADR 0018 A.2), never the
        // coarse item AABB.
        SemanticGeometry::Hatch(hatch) => {
            let (min, max) = hatch.local_bounds()?;
            bounds = Some((min, max));
        }
        SemanticGeometry::Mesh(mesh) => {
            if mesh.vertices.is_empty() {
                return None;
            }
            for point in &mesh.vertices {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        // Text tessellates to the two placeholder-box corners, exactly the box
        // `BoundsAccumulator` uses, so this contains the pickable geometry.
        SemanticGeometry::Text {
            position,
            height,
            text,
            ..
        } => {
            let h = height.abs().max(1e-9);
            if !include_bound_point(&mut bounds, *position) {
                return None;
            }
            if !include_bound_point(
                &mut bounds,
                Point3 {
                    x: position.x + h * text.chars().count() as f64,
                    y: position.y + h,
                    z: position.z,
                },
            ) {
                return None;
            }
        }
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            ..
        } => {
            let corner = |du: f64, dv: f64| Point3 {
                x: origin.x + u.x * du + v.x * dv,
                y: origin.y + u.y * du + v.y * dv,
                z: origin.z + u.z * du + v.z * dv,
            };
            for (du, dv) in [
                (0.0, 0.0),
                (pixels[0], 0.0),
                (0.0, pixels[1]),
                (pixels[0], pixels[1]),
            ] {
                if !include_bound_point(&mut bounds, corner(du, dv)) {
                    return None;
                }
            }
        }
        SemanticGeometry::Compound(children) => {
            if children.is_empty() {
                return None;
            }
            for child in children {
                let child = index_local_bounds(child)?;
                bounds = Some(match bounds {
                    Some((min, max)) => (
                        Point3 {
                            x: min.x.min(child.0.x),
                            y: min.y.min(child.0.y),
                            z: min.z.min(child.0.z),
                        },
                        Point3 {
                            x: max.x.max(child.1.x),
                            y: max.y.max(child.1.y),
                            z: max.z.max(child.1.z),
                        },
                    ),
                    None => child,
                });
            }
        }
        // A dimension's conservative bound is the union of its display
        // children, matching the compound rule (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => {
            if dimension.display().is_empty() {
                return None;
            }
            for child in dimension.display() {
                let child = index_local_bounds(child)?;
                bounds = Some(match bounds {
                    Some((min, max)) => (
                        Point3 {
                            x: min.x.min(child.0.x),
                            y: min.y.min(child.0.y),
                            z: min.z.min(child.0.z),
                        },
                        Point3 {
                            x: max.x.max(child.1.x),
                            y: max.y.max(child.1.y),
                            z: max.z.max(child.1.z),
                        },
                    ),
                    None => child,
                });
            }
        }
        // A bare insert is expanded by the caller, never offered as an item; an
        // opaque payload cannot be bounded (and must stay a candidate so its
        // `skipped` entry survives).
        SemanticGeometry::Insert { .. } | SemanticGeometry::Opaque { .. } => return None,
    }
    bounds
}

fn expand_box(min: Point3, max: Point3, radius: f64) -> (Point3, Point3) {
    (
        Point3 {
            x: min.x - radius,
            y: min.y - radius,
            z: min.z - radius,
        },
        Point3 {
            x: max.x + radius,
            y: max.y + radius,
            z: max.z + radius,
        },
    )
}

fn include_bound_box(
    bounds: &mut Option<(Point3, Point3)>,
    min: Point3,
    max: Point3,
) -> Option<()> {
    if !is_finite_point(min) || !is_finite_point(max) {
        return None;
    }
    match bounds {
        Some((lo, hi)) => {
            lo.x = lo.x.min(min.x);
            lo.y = lo.y.min(min.y);
            lo.z = lo.z.min(min.z);
            hi.x = hi.x.max(max.x);
            hi.y = hi.y.max(max.y);
            hi.z = hi.z.max(max.z);
        }
        None => *bounds = Some((min, max)),
    }
    Some(())
}

/// The best-fit plane normal of a polyline, using Newell's method.
///
/// This mirrors the tessellator's `polyline_plane_normal` exactly (including
/// the closing edge and the world-Z fallback for degenerate input), so the arc
/// bound below is computed in the same plane the tessellator draws in.
fn polyline_plane_normal(points: &[Point3]) -> Point3 {
    let world_z = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let n = points.len();
    if n < 3 {
        return world_z;
    }
    let mut acc = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        acc.x += (a.y - b.y) * (a.z + b.z);
        acc.y += (a.z - b.z) * (a.x + b.x);
        acc.z += (a.x - b.x) * (a.y + b.y);
    }
    if length3(acc) < 1e-12 {
        world_z
    } else if cad_geometry::is_xy_parallel(points) {
        // 2D data: `+Z`, never the winding (see `cad_geometry::is_xy_parallel`).
        world_z
    } else {
        scale3(acc, 1.0 / length3(acc))
    }
}

/// A conservative local box for one bulge arc `a -> b`.
///
/// The arc lies on a circle of radius `radius_abs` about a 3D centre (matching
/// the tessellator's construction in `plane_normal`), so the ball about that
/// centre contains the whole arc. This is correct even when the chord is not in
/// the plane (a non-planar 3D polyline), where the arc can leave the chord box
/// by far more than the sagitta.
fn bulge_arc_bounds(
    a: Point3,
    b: Point3,
    bulge: f64,
    plane_normal: Point3,
) -> Option<(Point3, Point3)> {
    if !is_finite_point(a) || !is_finite_point(b) || !bulge.is_finite() {
        return None;
    }
    let chord = sub(b, a);
    let chord_len = length3(chord);
    if chord_len < 1e-12 {
        return Some((a, a));
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        return Some((min_point(a, b), max_point(a, b)));
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    if !radius_abs.is_finite() {
        return None;
    }
    let (ax, ay, _) = cad_geometry::arbitrary_axis(plane_normal);
    let d2 = [dot3(chord, ax), dot3(chord, ay)];
    let chord2_len = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2_len < 1e-12 {
        return Some((min_point(a, b), max_point(a, b)));
    }
    let dir2 = [d2[0] / chord2_len, d2[1] / chord2_len];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    // Same construction as `cad_geometry`'s tessellator: `+u2 * r * cos(theta/2)`.
    let center2 = [
        mid2[0] + u2[0] * radius_abs * half.cos(),
        mid2[1] + u2[1] * radius_abs * half.cos(),
    ];
    let center3 = add(a, add(scale3(ax, center2[0]), scale3(ay, center2[1])));
    if !is_finite_point(center3) {
        return None;
    }
    let (center_min, center_max) = expand_box(center3, center3, radius_abs);
    Some((
        min_point(min_point(a, b), center_min),
        max_point(max_point(a, b), center_max),
    ))
}

fn min_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        z: a.z.min(b.z),
    }
}

fn max_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.max(b.x),
        y: a.y.max(b.y),
        z: a.z.max(b.z),
    }
}

/// Slab test against an AABB expanded by the precise pick tolerance. Invalid
/// arithmetic returns `true` (exhaustive fallback), never a false miss.
pub(crate) fn ray_intersects_expanded_bounds(
    ray: &Ray3,
    min: Point3,
    max: Point3,
    tolerance: f64,
) -> bool {
    let mut near: f64 = 0.0;
    let mut far = f64::INFINITY;
    for (origin, direction, low, high) in [
        (
            ray.origin.x,
            ray.direction.x,
            min.x - tolerance,
            max.x + tolerance,
        ),
        (
            ray.origin.y,
            ray.direction.y,
            min.y - tolerance,
            max.y + tolerance,
        ),
        (
            ray.origin.z,
            ray.direction.z,
            min.z - tolerance,
            max.z + tolerance,
        ),
    ] {
        if ![origin, direction, low, high].iter().all(|v| v.is_finite()) {
            return true;
        }
        if direction == 0.0 {
            if origin < low || origin > high {
                return false;
            }
        } else {
            let a = (low - origin) / direction;
            let b = (high - origin) / direction;
            if !a.is_finite() || !b.is_finite() {
                return true;
            }
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    far >= 0.0
}
