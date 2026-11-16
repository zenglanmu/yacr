//! TRIM of a HATCH **target** (AutoCAD's "trim a hatch object" case).
//!
//! A hatch stores no source curve — its display is derived from its boundary
//! loops and fill — but a loop **is** a polyline in the hatch plane
//! (`HatchLoop` points/bulges, implicitly closed). So TRIM resolves the loop with
//! the one shared polyline parameter model
//! ([`super::trim_polyline::plan_trim_polyline`]) and then rebuilds the hatch
//! from the kept window: no second cutter, no region boolean, no refit.
//!
//! # Why the kept window is re-closed by the cutting edge
//!
//! A hatch loop must be **closed** (a `HatchLoop` is implicitly closed and the
//! domain constructor rejects fewer than three vertices), while the TRIM contract
//! keeps only the window containing the pick's projection. The window is
//! therefore closed along the cutting boundary between its two cut points. That
//! closure is exactly the kept region only when the piece of boundary between the
//! two cuts is a **straight** segment: the chord of two points on one LINE is on
//! that LINE, so the rebuilt loop is the exact boundary of the trimmed region.
//!
//! Hence the one declared restriction: both window ends must lie on the same
//! straight LINE boundary primitive, whose segment must cover the whole closing
//! span (the midpoint and quarter points are checked too, so two collinear but
//! disjoint LINE boundaries cannot fake a single cutting edge). A cut that lands
//! on the loop's own vertex is still a cut (the shared polyline rule), so the
//! same straight-edge test decides that closure as well. A curved cutting
//! edge (ARC/CIRCLE/ELLIPSE/SPLINE) would need the boundary's own sub-curve in
//! the loop — an arc could be stored as a bulge, but a picked window between two
//! points of one circle has **two** candidate arcs and no model here decides
//! which one bounds the kept region, so it is refused by name
//! (`trim.hatch_cut_not_closable`) instead of guessing.
//!
//! # What else is refused by name (all write nothing)
//!
//! * `trim.hatch_nested_loops`: a hatch with more than one loop (an outer
//!   boundary plus islands). Cutting one loop changes which islands are still
//!   inside the region — that is a region boolean, not a loop trim.
//! * `trim.hatch_nonplanar_loop`: a **bulged** loop whose plane is not parallel
//!   to XY. The shared polyline model resolves a bulge as an arc in +Z, so the
//!   arc centres of a tilted bulged loop have no single in-plane frame.
//! * `trim.hatch_unrepresentable`: the rebuilt loop does not pass the hatch
//!   domain's own creation-path validation (self-intersection, fewer than three
//!   vertices, crossing rings) or the fill display cannot be rebuilt.
//!
//! A loop wholly inside a closed boundary is deleted exactly like every other
//! TRIM target (the shared planner's `TrimPlan::Delete`), and a plan-level
//! refusal (`trim.polyline_no_intersection`, `trim.polyline_pick_outside`,
//! `trim.polyline_degenerate`) is passed through unchanged: the loop is the
//! polyline target.
//!
//! The rebuilt hatch keeps its id, layer, space, draw order and display state
//! (the caller replaces the geometry in place), and the whole command is one
//! transaction / one undo step. The pattern phase (`pattern_origin`) is carried
//! over, so trimming never moves the pattern.

use super::trim_boundary::BoundarySet;
use super::trim_curves::{PlanRefusal, TrimPlan, TrimResolution};
use super::trim_polyline::plan_trim_polyline;
use super::*;
use cad_geometry::{arbitrary_axis, distance, dot, scale, sub};

/// Resolve a TRIM of a HATCH target, or refuse without touching the database.
///
/// See the module doc for the kept window, the exact re-closure rule and every
/// refusal code.
pub(crate) fn plan_trim_hatch(
    hatch: &HatchGeometry,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> Result<TrimResolution, PlanRefusal> {
    let loops = hatch.loops();
    if loops.len() != 1 {
        return Err(PlanRefusal::Unsupported(format!(
            "trim.hatch_nested_loops: the HATCH has {} boundary loops (an outer boundary and/or \
             islands); cutting one loop changes which islands stay inside the region, which is a \
             region boolean this build does not model. Only a single-loop HATCH is trimmable; \
             nothing was written",
            loops.len()
        )));
    }
    let loop_ = &loops[0];
    if loop_.points.len() < 3 {
        return Err(PlanRefusal::Unsupported(format!(
            "trim.hatch_degenerate_loop: the HATCH boundary loop has {} vertices; a trimmable \
             closed loop needs at least three. Nothing was written",
            loop_.points.len()
        )));
    }
    let plane = hatch.plane();
    let points: Vec<Point3> = loop_
        .points
        .iter()
        .map(|point| plane.to_world(*point))
        .collect();
    if points.iter().any(|point| !is_finite_point(*point)) {
        return Err(PlanRefusal::Unsupported(
            "trim.hatch_degenerate_loop: the HATCH boundary loop has a non-finite vertex in world \
             space; nothing was written"
                .into(),
        ));
    }
    let has_bulge = loop_.bulges.iter().any(|bulge| bulge.abs() > 1e-9);
    if has_bulge && !cad_geometry::is_xy_parallel(&points) {
        return Err(PlanRefusal::Unsupported(
            "trim.hatch_nonplanar_loop: a bulged HATCH boundary loop must be XY-parallel to resolve \
             its arc segments (the shared polyline model reads a bulge as an arc in +Z); this loop \
             is tilted, so no in-plane arc frame exists. Nothing was written"
                .into(),
        ));
    }

    // The loop is the polyline target: one cutter, one kept window.
    let resolution =
        plan_trim_polyline(&points, &loop_.bulges, true, pick, boundaries, policy, tol)?;
    let TrimPlan::Replace(geometry) = resolution.plan else {
        // Wholly inside a closed boundary: the same delete rule every TRIM target
        // uses. The caller deletes the entity (the hatch display goes with it).
        return Ok(TrimResolution {
            plan: TrimPlan::Delete,
            notes: resolution.notes,
        });
    };
    let SemanticGeometry::Polyline {
        points: window,
        bulges,
        ..
    } = geometry
    else {
        return Err(PlanRefusal::Unsupported(
            "trim.hatch_unrepresentable: the shared polyline planner did not return a polyline \
             window for the HATCH boundary loop; nothing was written"
                .into(),
        ));
    };
    if window.len() < 3 {
        return Err(PlanRefusal::Unsupported(format!(
            "trim.hatch_unrepresentable: the kept HATCH window has {} vertices; a boundary loop \
             needs at least three, so the trimmed region is a degenerate sliver. Nothing was \
             written",
            window.len()
        )));
    }
    let first = window[0];
    let last = *window.last().expect("checked non-empty");
    if !straight_cutting_edge(boundaries, first, last, tol) {
        return Err(PlanRefusal::Unsupported(format!(
            "trim.hatch_cut_not_closable: TRIM kept the window between the cuts at \
             ({:.9}, {:.9}, {:.9}) and ({:.9}, {:.9}, {:.9}), but no single straight LINE boundary \
             covers that whole closing span. A hatch loop must stay closed, and this build closes \
             it along the cutting edge only when that edge is straight (an arc/circle/ellipse/\
             spline edge between the same two cuts has two candidate sub-curves and no model here \
             chooses the one that bounds the kept region). Nothing was written",
            first.x, first.y, first.z, last.x, last.y, last.z
        )));
    }

    let mut rebuilt_points = Vec::with_capacity(window.len());
    for point in &window {
        rebuilt_points.push(plane_point(plane, *point));
    }
    // The window is an **open** polyline: one bulge per real segment. A
    // `HatchLoop` is implicitly closed, so the wrap segment is the closing chord
    // (bulge 0) — the straight cutting edge validated above.
    let mut rebuilt_bulges = bulges;
    rebuilt_bulges.truncate(rebuilt_points.len().saturating_sub(1));
    rebuilt_bulges.resize(rebuilt_points.len(), 0.0);
    // The loop no longer follows the source path, so the stored analytic edges are
    // dropped and only the authored flags that still describe it survive: it is
    // still an external (outermost) polyline loop, but it is no longer a derived
    // boundary, no longer a text box, and it is closed.
    let flags =
        HatchLoopFlags(loop_.flags.bits() & (HatchLoopFlags::EXTERNAL | HatchLoopFlags::POLYLINE));
    let rebuilt_loop = HatchLoop::new(rebuilt_points, rebuilt_bulges, flags);
    let rebuilt = HatchGeometry::new(
        vec![rebuilt_loop],
        plane,
        hatch.style(),
        hatch.fill().clone(),
        hatch.seeds().to_vec(),
        hatch.pixel_size(),
        hatch.pattern_type(),
    )
    .map_err(|error| {
        PlanRefusal::Unsupported(format!(
            "trim.hatch_unrepresentable: the trimmed boundary is not a valid HATCH loop ({error}); \
             nothing was written"
        ))
    })?
    // The creation path starts at a zero pattern origin; a trim must not move the
    // pattern phase, so the stored displacement is applied back (exactly what
    // HATCHEDIT's boundary rebuild does).
    .with_pattern_origin(hatch.pattern_origin());

    let mut notes = resolution.notes;
    notes.push(format!(
        "TRIM: the HATCH boundary loop was re-closed along the straight cutting edge between \
         ({:.6}, {:.6}, {:.6}) and ({:.6}, {:.6}, {:.6}); the fill was rebuilt from the kept loop",
        first.x, first.y, first.z, last.x, last.y, last.z
    ));
    Ok(TrimResolution {
        plan: TrimPlan::Replace(SemanticGeometry::Hatch(Box::new(rebuilt))),
        notes,
    })
}

/// Whether one **straight** boundary primitive covers the whole closing span
/// `first -> last`, so the window can be closed by its own chord.
///
/// The chord's endpoints are the two cut points, which lie on the cutting edge by
/// construction; the interior samples are what rule out two collinear but
/// disjoint LINE boundaries (or a LINE that merely passes near one end) faking a
/// single edge. Anything that is not a LINE primitive never qualifies: its
/// sub-curve between the same two points is not the chord.
fn straight_cutting_edge(boundaries: &BoundarySet, first: Point3, last: Point3, tol: f64) -> bool {
    boundaries
        .primitives()
        .iter()
        .filter_map(|primitive| match primitive {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .any(|(start, end)| {
            [0.0, 0.25, 0.5, 0.75, 1.0]
                .into_iter()
                .all(|t| distance_on_chord_to_segment(first, last, t, start, end) <= tol)
        })
}

/// The distance from the point at fraction `t` of the chord `first -> last` to the
/// boundary segment `start -> end`.
fn distance_on_chord_to_segment(
    first: Point3,
    last: Point3,
    t: f64,
    start: Point3,
    end: Point3,
) -> f64 {
    let along = sub(last, first);
    let point = cad_geometry::add(first, scale(along, t));
    distance_to_segment(point, start, end)
}

/// The 3D distance from `p` to the segment `a -> b`.
fn distance_to_segment(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if !len2.is_finite() || len2 <= 0.0 {
        return distance(p, a);
    }
    let ap = sub(p, a);
    let t = (dot(ap, ab) / len2).clamp(0.0, 1.0);
    distance(p, cad_geometry::add(a, scale(ab, t)))
}

/// The hatch-plane 2D coordinates of a world point.
///
/// The exact inverse of [`HatchPlane::to_world`], derived from the same
/// `cad_geometry::arbitrary_axis` frame and elevation origin, so a trimmed loop's
/// untouched vertices land back on their stored values (up to floating point).
fn plane_point(plane: HatchPlane, point: Point3) -> [f64; 2] {
    let (ax, ay, un) = arbitrary_axis(plane.normal);
    let origin = scale(un, plane.elevation);
    let offset = sub(point, origin);
    [dot(offset, ax), dot(offset, ay)]
}
