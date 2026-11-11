//! `MASSPROP`'s two non-curve exact-analytic regions: a **closed** SPLINE
//! (sampled, explicitly labelled) and a **HATCH** boundary loop set (net area).
//!
//! Split out of `app_massprop.rs` to keep both modules inside the code-size
//! limit; every item here is used by `Application::mass_region` only.
//!
//! ## Closed SPLINE: sampled, and it says so
//!
//! A B-spline region has no closed-form area in this build (a rational integral
//! of the NURBS basis would be a new numeric subsystem), so the spline is
//! sampled with the **existing** adaptive NURBS discretiser
//! ([`cad_geometry::NurbsCurve::discretize`], the same routine the display and
//! the tessellator use) and the sample ring is measured with the exact
//! bulge-aware ring area (zero bulges). The result is therefore the **sampled
//! polygon's** area/perimeter/bbox/centroid, not the curve's; every run says so
//! with `massprop.approx_spline_region` and marks the perimeter `~`.
//!
//! * closed ⇒ the evaluated start and end points coincide within the session
//!   computation tolerance; an open spline is `massprop.open_spline`. A spline
//!   whose control net/knot vector cannot be evaluated is
//!   `massprop.spline_invalid` — never a silently defaulted curve (the display
//!   tessellator's clamped-uniform *fallback* is deliberately not reused here).
//! * the chord tolerance handed to the discretiser is
//!   `max(computation tolerance, control-net span · 1e-4)` and the sample count
//!   is capped, so a pathological spline cannot produce an unbounded ring. Both
//!   numbers are printed in the note.
//!
//! ## HATCH: net loop area, exact where the loops are exact
//!
//! The hatch's stored loops are the authority (the display fill is derived and
//! never measured):
//!
//! * every loop's **signed** area is its exact bulge-aware ring area
//!   ([`cad_geometry::measure_bulged_ring_area`]) signed by **nesting parity** —
//!   a loop inside an odd number of other loops is a hole — so islands subtract
//!   even when the source wrote them with the outer loop's own winding;
//! * the reported area is `|Σ signed loop area|`, the perimeter is the sum of
//!   the loops carrying the DXF `external` flag bit (`92 & 1`), the bounding box
//!   is the union of the exact bulged loop boxes and the centroid is the signed
//!   area-weighted loop centroid (the same analytic straight-edge +
//!   circular-segment moments the closed polyline uses);
//! * the import path samples every boundary edge into the loop's points and
//!   stores zero bulges (see `cad-import-acadrust`'s `hatch_geometry`), so a loop
//!   whose `source_edges` carries anything other than `Line` is a **sampled**
//!   boundary: that hatch is reported as approximate
//!   (`massprop.approx_hatch_boundary`). A loop with no source edges is a
//!   polyline path, and its points/bulges are exact.
//!
//! Nothing here writes: both functions only return a [`super::MassRegion`].

use super::*;
use cad_geometry::{
    arbitrary_axis, measure_bulged_ring_area, polyline_length, AreaError, NurbsCurve,
    TessellationParams,
};

/// Hard cap on the sample count of one closed spline's measurement ring.
const SPLINE_SAMPLE_CAP: usize = 1024;
/// Lower bound on that ring, so a nearly straight closed spline still has a
/// usable polygon.
const SPLINE_MIN_SAMPLES: usize = 16;
/// Sample chord tolerance as a fraction of the control net's diagonal. This is a
/// measurement-quality floor (not a display tolerance): 1e-4 of the extent keeps
/// the sampled area error far below the printed six decimals while bounding the
/// sample count.
const SPLINE_SAMPLE_EXTENT_FRACTION: f64 = 1e-4;

/// Measure one closed SPLINE as a **sampled** region.
pub(super) fn mass_spline_region(
    entity: &cad_db::DbEntity,
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
    tolerance: f64,
) -> CadResult<MassRegion> {
    if control_points
        .iter()
        .any(|point| !camera::is_finite_point(*point))
        || knots.iter().any(|knot| !knot.is_finite())
        || weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        return Err(mass_degenerate(
            entity,
            "the spline has a non-finite control point, knot or weight",
        ));
    }
    let tolerance = tolerance.max(1e-9);
    let curve = NurbsCurve::new(
        degree as usize,
        control_points.to_vec(),
        knots.to_vec(),
        weights.to_vec(),
    )
    .map_err(|error| {
        CadError::InvalidInput(format!(
            "massprop.spline_invalid: entity {}'s control net/knot vector cannot be evaluated \
             ({error}); MASSPROP does not guess a curve",
            entity.id.0
        ))
    })?;

    let (t_start, t_end) = curve.domain();
    let start = curve.evaluate(t_start);
    let end = curve.evaluate(t_end);
    let closure = camera::length3(camera::sub(end, start));
    if !closure.is_finite() {
        return Err(mass_degenerate(
            entity,
            "the spline's end points are not finite",
        ));
    }
    if closure > tolerance {
        return Err(CadError::InvalidInput(format!(
            "massprop.open_spline: entity {} is not closed (its end points differ by {closure}); \
             MASSPROP needs a closed region",
            entity.id.0
        )));
    }

    let span = control_net_span(control_points);
    let sample_tolerance = tolerance
        .max(span * SPLINE_SAMPLE_EXTENT_FRACTION)
        .max(1e-12);
    let params = TessellationParams {
        tolerance: sample_tolerance,
        max_segments: SPLINE_SAMPLE_CAP,
        min_segments: SPLINE_MIN_SAMPLES,
    };
    let mut samples = curve.discretize(sample_tolerance, &params);
    if samples.len() >= 2
        && camera::length3(camera::sub(*samples.last().unwrap(), samples[0])) <= tolerance
    {
        // The sampled ring is implicitly closed; a duplicated closing vertex
        // would add a zero-length chord.
        samples.pop();
    }
    if samples.len() < 3 {
        return Err(mass_degenerate(
            entity,
            "the sampled spline ring has fewer than three distinct vertices",
        ));
    }
    let bulges = vec![0.0; samples.len()];
    let ring = measure_bulged_ring_area(&samples, &bulges, tolerance)
        .map_err(|error| spline_refusal(entity, error))?;
    let normal = camera::normalize3(camera::cross(ring.plane.u, ring.plane.v))
        .ok_or_else(|| mass_degenerate(entity, "the sampled ring's plane normal is not finite"))?;
    let flat = flatten_to_plane(&samples, ring.plane);
    let centroid = bulged_ring_centroid(ring.plane, &flat, &bulges).ok_or_else(|| {
        mass_degenerate(
            entity,
            "the sampled spline ring's centroid could not be derived",
        )
    })?;
    let (min, max) = bulged_ring_bounds(&samples, &bulges, ring.plane);
    let mut closed = samples.clone();
    closed.push(samples[0]);
    let perimeter = polyline_length(&closed);
    if !perimeter.is_finite() || perimeter <= 0.0 {
        return Err(mass_degenerate(
            entity,
            "the sampled spline ring has no finite perimeter",
        ));
    }
    Ok(MassRegion {
        kind: "closed spline (sampled)",
        area: ring.area,
        // The sampled polyline's length is not the curve's length.
        perimeter: (perimeter, true),
        min,
        max,
        centroid,
        normal,
        plane_point: ring.plane.origin,
        notes: vec![(
            "massprop.approx_spline_region",
            format!(
                "MASSPROP: entity {} is measured from an adaptive {} vertex sample ring (chord \
                 tolerance {sample_tolerance:.6e}); its area, perimeter, bounding box and centroid \
                 are the sample ring's values, not closed-form B-spline integrals",
                entity.id.0,
                samples.len()
            ),
        )],
    })
}

/// Measure one HATCH's boundary loop set as a region: net signed loop area.
pub(super) fn mass_hatch_region(
    entity: &cad_db::DbEntity,
    hatch: &HatchGeometry,
    tolerance: f64,
) -> CadResult<MassRegion> {
    let loops = hatch.loops();
    if loops.is_empty() {
        return Err(mass_degenerate(entity, "the hatch has no boundary loops"));
    }
    let tolerance = tolerance.max(1e-9);
    let plane = hatch.plane();
    let (u, v, normal) = arbitrary_axis(plane.normal);
    let normal = camera::normalize3(normal)
        .ok_or_else(|| mass_degenerate(entity, "the hatch plane normal has no direction"))?;
    let work_plane = WorkPlane {
        // `HatchPlane::to_world` places points from this same origin/basis, so the
        // loop-local 2D coordinates and this plane cannot disagree.
        origin: plane.world_origin(),
        u,
        v,
    };

    let mut measured = Vec::with_capacity(loops.len());
    for loop_ in loops {
        let count = loop_.points.len();
        if count < 3 {
            return Err(mass_degenerate(
                entity,
                &format!("a hatch boundary loop has only {count} vertex/vertices"),
            ));
        }
        if loop_
            .points
            .iter()
            .any(|point| !point[0].is_finite() || !point[1].is_finite())
        {
            return Err(mass_degenerate(
                entity,
                "a hatch boundary loop has a non-finite vertex",
            ));
        }
        let bulges = if loop_.bulges.is_empty() {
            vec![0.0; count]
        } else if loop_.bulges.len() == count {
            loop_.bulges.clone()
        } else {
            return Err(mass_degenerate(
                entity,
                &format!(
                    "a hatch boundary loop has {count} vertices but {} bulges",
                    loop_.bulges.len()
                ),
            ));
        };
        let points: Vec<Point3> = loop_
            .points
            .iter()
            .map(|point| plane.to_world(*point))
            .collect();
        let ring = measure_bulged_ring_area(&points, &bulges, tolerance)
            .map_err(|error| hatch_refusal(entity, error))?;
        let perimeter = bulged_ring_perimeter(&points, &bulges)
            .ok_or_else(|| mass_degenerate(entity, "a hatch loop has no finite perimeter"))?;
        let centroid =
            bulged_ring_centroid(work_plane, &loop_.points, &bulges).ok_or_else(|| {
                mass_degenerate(entity, "a hatch loop's centroid could not be derived")
            })?;
        let (min, max) = bulged_ring_bounds(&points, &bulges, work_plane);
        measured.push(HatchLoopMetrics {
            area: ring.area,
            signed_area: 0.0,
            // The sampled ring (bulges included) is the containment polygon; the
            // loop's own points would miss an arc that reaches over a vertex.
            ring: loop_.sampled_ring(),
            perimeter,
            centroid,
            min,
            max,
            external: (loop_.flags.bits() & cad_domain::HatchLoopFlags::EXTERNAL) != 0,
            sampled: loop_.source_edges.as_ref().is_some_and(|edges| {
                edges
                    .iter()
                    .any(|edge| !matches!(edge, BoundaryEdge::Line { .. }))
            }),
        });
    }

    // Nesting parity decides the sign, **not** the stored winding: a source is free
    // to write an island with the same orientation as its outer loop, and reading
    // the winding would then add it instead of subtracting it. A loop contained by
    // an odd number of other loops is a hole; even depth is a filled region.
    for index in 0..measured.len() {
        let probe = measured[index].ring[0];
        let depth = measured
            .iter()
            .enumerate()
            .filter(|(other, candidate)| {
                *other != index && ring_contains_point(&candidate.ring, probe)
            })
            .count();
        measured[index].signed_area = if depth % 2 == 0 {
            measured[index].area
        } else {
            -measured[index].area
        };
    }

    let net_area: f64 = measured.iter().map(|loop_| loop_.signed_area).sum();
    if !net_area.is_finite() {
        return Err(mass_degenerate(
            entity,
            "the hatch's summed loop area is not finite",
        ));
    }
    let area = net_area.abs();
    if area <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "massprop.hatch_degenerate: entity {}'s hatch loops enclose a zero net area",
            entity.id.0
        )));
    }

    // The outer boundary is the loop set the source flagged as external. A hatch
    // built from scratch carries no flag bits at all; then the largest-|area| loop
    // is the outer boundary and the report names that rule instead of guessing.
    let flagged = measured.iter().any(|loop_| loop_.external);
    let mut notes = Vec::new();
    let perimeter = if flagged {
        measured
            .iter()
            .filter(|loop_| loop_.external)
            .map(|loop_| loop_.perimeter)
            .sum::<f64>()
    } else {
        let outer = measured
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.signed_area
                    .abs()
                    .partial_cmp(&b.signed_area.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        notes.push((
            "massprop.hatch_perimeter_rule",
            format!(
                "MASSPROP: entity {}'s hatch loops carry no external flag bit, so the outer \
                 boundary is taken as the largest-area loop for the reported perimeter",
                entity.id.0
            ),
        ));
        measured[outer].perimeter
    };
    if !perimeter.is_finite() || perimeter <= 0.0 {
        return Err(mass_degenerate(
            entity,
            "the hatch's outer boundary has no finite perimeter",
        ));
    }
    if measured.iter().any(|loop_| loop_.sampled) {
        notes.push((
            "massprop.approx_hatch_boundary",
            format!(
                "MASSPROP: entity {}'s hatch boundary carries non-linear source edges that the \
                 importer sampled into its loop vertices, so the reported area, perimeter, \
                 bounding box and centroid are the sampled boundary's values, not exact \
                 arc/spline integrals",
                entity.id.0
            ),
        ));
    }

    let mut min = measured[0].min;
    let mut max = measured[0].max;
    for loop_ in &measured[1..] {
        min = min_point(min, loop_.min);
        max = max_point(max, loop_.max);
    }
    let mut centroid_sums = (0.0, 0.0, 0.0);
    for loop_ in &measured {
        centroid_sums.0 += loop_.centroid.x * loop_.signed_area;
        centroid_sums.1 += loop_.centroid.y * loop_.signed_area;
        centroid_sums.2 += loop_.centroid.z * loop_.signed_area;
    }
    let centroid = Point3 {
        x: centroid_sums.0 / net_area,
        y: centroid_sums.1 / net_area,
        z: centroid_sums.2 / net_area,
    };
    if !camera::is_finite_point(centroid) {
        return Err(mass_degenerate(
            entity,
            "the hatch's area-weighted centroid is not finite",
        ));
    }
    Ok(MassRegion {
        kind: "hatch region",
        area,
        perimeter: (perimeter, false),
        min,
        max,
        centroid,
        normal,
        plane_point: work_plane.origin,
        notes,
    })
}

/// One hatch loop's measurement, kept together so the outer-boundary rule can be
/// applied after every loop is known.
struct HatchLoopMetrics {
    /// The loop's exact bulge-aware area magnitude.
    area: f64,
    /// `area` with the nesting-parity sign (negative inside an odd number of
    /// other loops), filled in after every loop is measured.
    signed_area: f64,
    /// The loop's sampled 2D ring (bulges included), used for containment only.
    ring: Vec<[f64; 2]>,
    perimeter: f64,
    centroid: Point3,
    min: Point3,
    max: Point3,
    external: bool,
    sampled: bool,
}

/// Even-odd point-in-polygon test on a closed 2D ring (no repeated last vertex).
///
/// This answers "is this loop nested inside that one" for the hatch's parity rule;
/// a point exactly on an edge is ambiguous by construction, which is why the
/// creation path rejects touching loops and the import path keeps the source's
/// own loop rows instead of re-deriving them.
fn ring_contains_point(ring: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    let mut previous = ring.len().wrapping_sub(1);
    for current in 0..ring.len() {
        let a = ring[current];
        let b = ring[previous];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            if point[0] < a[0] + t * (b[0] - a[0]) {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Project world points into a ring plane's own 2D frame.
fn flatten_to_plane(points: &[Point3], plane: WorkPlane) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| {
            let delta = camera::sub(*point, plane.origin);
            [camera::dot3(delta, plane.u), camera::dot3(delta, plane.v)]
        })
        .collect()
}

/// The diagonal of the control net's axis-aligned box (a size for the sampling
/// floor; `0.0` when every control point coincides).
fn control_net_span(points: &[Point3]) -> f64 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    let mut min = *first;
    let mut max = *first;
    for point in points {
        min = min_point(min, *point);
        max = max_point(max, *point);
    }
    camera::length3(camera::sub(max, min))
}

fn spline_refusal(entity: &cad_db::DbEntity, error: AreaError) -> CadError {
    let id = entity.id.0;
    match error {
        AreaError::Degenerate => CadError::InvalidInput(format!(
            "massprop.degenerate: entity {id}'s sampled spline ring has zero area"
        )),
        AreaError::TooFewPoints(count) => CadError::InvalidInput(format!(
            "massprop.degenerate: entity {id}'s sampled spline ring has {count} distinct vertices"
        )),
        AreaError::UndeterminedPlane => CadError::InvalidInput(format!(
            "massprop.degenerate: entity {id}'s sampled spline ring does not determine a plane"
        )),
        AreaError::NonPlanar => CadError::InvalidInput(format!(
            "massprop.non_planar: entity {id}'s spline is not planar within the computation \
             tolerance"
        )),
        AreaError::SelfIntersecting => CadError::InvalidInput(format!(
            "massprop.self_intersecting: entity {id}'s spline crosses itself; area is undefined"
        )),
        AreaError::BulgesMismatch { points, bulges } => CadError::InvalidInput(format!(
            "massprop.degenerate: entity {id}'s sampled ring is inconsistent ({points} vertices, \
             {bulges} bulges)"
        )),
    }
}

fn hatch_refusal(entity: &cad_db::DbEntity, error: AreaError) -> CadError {
    let id = entity.id.0;
    match error {
        AreaError::Degenerate => CadError::InvalidInput(format!(
            "massprop.hatch_degenerate: entity {id} has a boundary loop with zero area"
        )),
        AreaError::TooFewPoints(count) => CadError::InvalidInput(format!(
            "massprop.hatch_degenerate: entity {id} has a boundary loop with {count} distinct \
             vertices"
        )),
        AreaError::UndeterminedPlane => CadError::InvalidInput(format!(
            "massprop.hatch_degenerate: entity {id} has a boundary loop that does not determine a \
             plane"
        )),
        AreaError::NonPlanar => CadError::InvalidInput(format!(
            "massprop.hatch_non_planar: entity {id} has a boundary loop that leaves the hatch plane"
        )),
        AreaError::SelfIntersecting => CadError::InvalidInput(format!(
            "massprop.hatch_self_intersecting: entity {id} has a self-intersecting boundary loop; \
             area is undefined"
        )),
        AreaError::BulgesMismatch { points, bulges } => CadError::InvalidInput(format!(
            "massprop.hatch_degenerate: entity {id} has a corrupt boundary loop ({points} vertices, \
             {bulges} bulges)"
        )),
    }
}
