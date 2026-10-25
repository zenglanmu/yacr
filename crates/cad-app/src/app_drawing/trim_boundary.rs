//! Boundary resolution for TRIM/EXTEND: analytic primitives, closed-boundary
//! containment and cut precision.
//!
//! A boundary entity is reduced to analytic primitives (a bulged LWPOLYLINE is
//! exploded into LINE pieces and [`cad_geometry::bulge_arc_geometry`] ARCs) so a
//! cut can be an exact conic intersection. The closed-boundary "target wholly
//! inside" rule lives here too: it is a **plane-aware** inside test, because a
//! tilted outline projects to a real XY polygon and a flat XY ray cast would
//! delete targets that never touch it.
//!
//! This module also decides whether a cut pair has an analytic solution at all:
//! [`note_approximation`] reports the pairs the engine can only solve by its
//! sampled fallback, so the command layer never implies an exact cut it did not
//! get.

use super::*;
use cad_geometry::{
    cross, dot, length, normalize, scale, sub, DefaultGeometryEngine, GeometryEngine,
};

const TAU: f64 = std::f64::consts::TAU;

/// A closed LWPOLYLINE outline, sampled once and kept in the polyline's **own**
/// plane so the inside test cannot be fooled by a tilted outline.
struct ClosedOutline {
    origin: Point3,
    normal: Point3,
    ax: Point3,
    ay: Point3,
    /// The sampled ring in `(dot(v, ax), dot(v, ay))` coordinates.
    ring: Vec<(f64, f64)>,
}

impl ClosedOutline {
    fn new(points: Vec<Point3>, normal: Point3) -> Option<Self> {
        let origin = *points.first()?;
        let normal = normalize(normal);
        let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
        let ring: Vec<(f64, f64)> = points
            .iter()
            .map(|p| {
                let v = sub(*p, origin);
                (dot(v, ax), dot(v, ay))
            })
            .collect();
        if ring.len() < 3 {
            return None;
        }
        Some(Self {
            origin,
            normal,
            ax,
            ay,
            ring,
        })
    }

    /// Whether `p` lies inside by more than `tol`.
    ///
    /// The plane check comes first (a point off the polyline's plane is never
    /// inside, however its XY projection looks), then the ray cast runs in the
    /// polyline's own frame. Being within `tol` of the outline is **not** inside:
    /// the delete rule is destructive, so an ambiguous point refuses instead.
    fn contains(&self, p: Point3, tol: f64) -> bool {
        let v = sub(p, self.origin);
        if dot(v, self.normal).abs() > tol {
            return false;
        }
        let x = dot(v, self.ax);
        let y = dot(v, self.ay);
        if !point_in_polygon(x, y, &self.ring) {
            return false;
        }
        let count = self.ring.len();
        !self.ring.iter().enumerate().any(|(index, start)| {
            let end = self.ring[(index + 1) % count];
            distance_to_segment(x, y, *start, end) <= tol
        })
    }
}

/// A closed analytic boundary (a CIRCLE, or a full-sweep ARC/ELLIPSE).
struct ClosedCurve {
    center: Point3,
    normal: Point3,
    shape: ClosedShape,
}

enum ClosedShape {
    Circle { radius: f64 },
    Ellipse { major_axis: Point3, ratio: f64 },
}

impl ClosedCurve {
    /// A proper inside test on the boundary's own plane (an off-plane point is
    /// never inside).
    fn contains(&self, p: Point3, tol: f64) -> bool {
        let normal = normalize(self.normal);
        let v = sub(p, self.center);
        if dot(v, normal).abs() > tol {
            return false;
        }
        match &self.shape {
            ClosedShape::Circle { radius } => length(v) < radius - tol,
            ClosedShape::Ellipse { major_axis, ratio } => {
                let major_len = length(*major_axis);
                if major_len <= tol {
                    return false;
                }
                let major = scale(*major_axis, 1.0 / major_len);
                let minor = normalize(cross(normal, major));
                let u = dot(v, major) / major_len;
                let w = dot(v, minor) / (major_len * ratio).max(tol);
                u * u + w * w < 1.0
            }
        }
    }
}

/// The resolved boundary input of a TRIM/EXTEND.
#[derive(Default)]
pub(super) struct BoundarySet {
    /// One analytic primitive per boundary curve piece (LINE, ARC, CIRCLE or
    /// ELLIPSE).
    primitives: Vec<SemanticGeometry>,
    /// Closed outlines for the "target wholly inside the boundary" delete rule:
    /// a closed LWPOLYLINE is sampled once at the policy tolerance and projected
    /// into its own plane.
    closed_outlines: Vec<ClosedOutline>,
    /// Closed analytic boundaries for the same rule.
    closed_curves: Vec<ClosedCurve>,
}

impl BoundarySet {
    pub(super) fn primitives(&self) -> &[SemanticGeometry] {
        &self.primitives
    }

    pub(super) fn is_empty(&self) -> bool {
        self.primitives.is_empty()
            && self.closed_outlines.is_empty()
            && self.closed_curves.is_empty()
    }

    /// Whether `p` lies inside any closed boundary (an in-plane ray cast for a
    /// polyline outline, an analytic test for a circle/ellipse). A point off the
    /// boundary's own plane is never inside.
    pub(super) fn contains(&self, p: Point3, tol: f64) -> bool {
        self.closed_outlines
            .iter()
            .any(|outline| outline.contains(p, tol))
            || self
                .closed_curves
                .iter()
                .any(|curve| curve.contains(p, tol))
    }
}

/// Resolve one boundary entity into [`BoundarySet`].
///
/// Supported: LINE, ARC, CIRCLE, ELLIPSE and LWPOLYLINE (straight or bulged, in
/// any mix). Every other geometry kind is an explicit `Unsupported` refusal
/// naming the kind and the operation, before anything is mutated.
pub(super) fn collect_boundaries(
    geometry: &SemanticGeometry,
    set: &mut BoundarySet,
    policy: &TolerancePolicy,
    command: &str,
) -> CadResult<()> {
    match geometry {
        SemanticGeometry::Line { .. } => {
            set.primitives.push(geometry.clone());
            Ok(())
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            sweep,
            ..
        } => {
            set.primitives.push(geometry.clone());
            if sweep.abs() >= TAU - 1e-9 {
                set.closed_curves.push(ClosedCurve {
                    center: *center,
                    normal: *normal,
                    shape: ClosedShape::Circle {
                        radius: radius.abs(),
                    },
                });
            }
            Ok(())
        }
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            set.primitives.push(geometry.clone());
            set.closed_curves.push(ClosedCurve {
                center: *center,
                normal: *normal,
                shape: ClosedShape::Circle {
                    radius: radius.abs(),
                },
            });
            Ok(())
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            sweep,
            ..
        } => {
            set.primitives.push(geometry.clone());
            if sweep.abs() >= TAU - 1e-9 {
                set.closed_curves.push(ClosedCurve {
                    center: *center,
                    normal: *normal,
                    shape: ClosedShape::Ellipse {
                        major_axis: *major_axis,
                        ratio: ratio.abs(),
                    },
                });
            }
            Ok(())
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.len() < 2 {
                return Ok(());
            }
            let plane_normal = cad_geometry::polyline_plane_normal(points);
            let count = points.len();
            let last = if *closed { count } else { count - 1 };
            for index in 0..last {
                let a = points[index];
                let b = points[(index + 1) % count];
                if length(sub(b, a)) <= 1e-12 {
                    continue;
                }
                let bulge = bulges.get(index).copied().unwrap_or(0.0);
                if bulge.abs() < 1e-12 {
                    set.primitives
                        .push(SemanticGeometry::Line { start: a, end: b });
                    continue;
                }
                match cad_geometry::bulge_arc_geometry(a, b, bulge, plane_normal) {
                    Some(arc) => set.primitives.push(arc),
                    // A bulge too shallow to carry an arc is the chord.
                    None => set
                        .primitives
                        .push(SemanticGeometry::Line { start: a, end: b }),
                }
            }
            if *closed {
                let engine = DefaultGeometryEngine;
                let outline = engine.tessellate_curve(geometry, policy)?;
                if let Some(outline) = ClosedOutline::new(outline, plane_normal) {
                    set.closed_outlines.push(outline);
                }
            }
            Ok(())
        }
        other => Err(CadError::Unsupported(format!(
            "{command} boundary {} is not supported; only LINE, ARC, CIRCLE, ELLIPSE and \
             LWPOLYLINE are",
            geometry_kind(other)
        ))),
    }
}

/// Record that this boundary pair has no analytic solution and is cut only by the
/// engine's sampled fallback, so the reported cut point is an approximation.
pub(super) fn note_approximation(
    probe: &SemanticGeometry,
    primitive: &SemanticGeometry,
    policy: &TolerancePolicy,
    command: &str,
    notes: &mut Vec<String>,
) {
    if !pair_is_approximate(probe, primitive, policy.computation_world.max(1e-12)) {
        return;
    }
    let note = format!(
        "{command}: the cut against {} uses the sampled intersection approximation \
         ({} x {} has no analytic solution); the reported cut point is approximate",
        geometry_kind(primitive),
        geometry_kind(probe),
        geometry_kind(primitive),
    );
    if !notes.contains(&note) {
        notes.push(note);
    }
}

/// The conic kinds `cad_geometry` solves analytically, and therefore the only
/// ones a cut may claim to be exact for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConicKind {
    Segment,
    Circle,
    Arc,
    Ellipse,
}

fn conic_kind(geometry: &SemanticGeometry) -> Option<ConicKind> {
    match geometry {
        SemanticGeometry::Line { .. } => Some(ConicKind::Segment),
        SemanticGeometry::Circle { .. } => Some(ConicKind::Circle),
        SemanticGeometry::Arc { .. } => Some(ConicKind::Arc),
        SemanticGeometry::Ellipse { .. } => Some(ConicKind::Ellipse),
        _ => None,
    }
}

/// Whether `GeometryEngine::intersect_local` resolves this pair only by flattening
/// both curves and intersecting the chords (its documented fallback), so the cut
/// point is approximate. This mirrors the engine's own dispatch without changing
/// its public API:
///
/// * ARC×ELLIPSE and ELLIPSE×ELLIPSE have no analytic arm;
/// * the circle/arc family is analytic only when the two planes are parallel and
///   each centre lies in the other's plane (`circle_circle_points` returns `None`
///   otherwise, which sends the pair to the fallback).
fn pair_is_approximate(a: &SemanticGeometry, b: &SemanticGeometry, radial_tol: f64) -> bool {
    let (Some(kind_a), Some(kind_b)) = (conic_kind(a), conic_kind(b)) else {
        // Not a conic pair at all: always sampled.
        return true;
    };
    let circle_family = matches!(
        (kind_a, kind_b),
        (ConicKind::Circle, ConicKind::Circle)
            | (ConicKind::Circle, ConicKind::Arc)
            | (ConicKind::Arc, ConicKind::Circle)
            | (ConicKind::Arc, ConicKind::Arc)
    );
    if circle_family {
        let (Some((center_a, normal_a)), Some((center_b, normal_b))) =
            (circle_frame(a), circle_frame(b))
        else {
            return true;
        };
        let unit_a = normalize(normal_a);
        let unit_b = normalize(normal_b);
        if length(cross(unit_a, unit_b)) > 1e-9 {
            return true;
        }
        if dot(sub(center_b, center_a), unit_a).abs() > radial_tol {
            return true;
        }
        return false;
    }
    matches!(
        (kind_a, kind_b),
        (ConicKind::Arc, ConicKind::Ellipse)
            | (ConicKind::Ellipse, ConicKind::Arc)
            | (ConicKind::Ellipse, ConicKind::Ellipse)
    )
}

/// `(center, normal)` of a CIRCLE or ARC.
fn circle_frame(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
    match geometry {
        SemanticGeometry::Circle { center, normal, .. }
        | SemanticGeometry::Arc { center, normal, .. } => Some((*center, *normal)),
        _ => None,
    }
}

/// Point-in-polygon (ray casting) in a 2D frame; the ring is implicitly closed.
fn point_in_polygon(x: f64, y: f64, ring: &[(f64, f64)]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        let crosses = (yi > y) != (yj > y);
        if crosses {
            let x_at = (xj - xi) * (y - yi) / (yj - yi) + xi;
            if x < x_at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Distance from `(x, y)` to the 2D segment `a→b`.
fn distance_to_segment(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let px = a.0 + dx * t - x;
    let py = a.1 + dy * t - y;
    (px * px + py * py).sqrt()
}
