//! Object snap engine (spec §3.3, F06).
//!
//! Snapping is computed from a **local** set of targets the caller has already
//! narrowed with the spatial index; the engine never iterates the drawing.
//! Candidates are filtered against a pick ray (`t >= 0` in front of the origin)
//! and a tolerance expressed in logical pixels, converted to world units with
//! `world_per_px`. Every candidate carries its [`SnapKind`], world point, source
//! entity/sub-element and [`Precision`] so the UI can show it and a measurement
//! can record its provenance.
//!
//! Available snaps: endpoint, midpoint, center, quadrant, perpendicular and
//! local intersection. Curved geometry is handled analytically (a bulge arc is
//! reconstructed from its bulge, an ellipse from its axes); nothing here is
//! derived from the display LOD, so a change of display density cannot change a
//! snap result.
//!
//! One opt-in exception exists for geometry that has no analytic form at all: a
//! [`SnapTarget`] may set
//! [`body_edge_snaps`](SnapTarget::body_edge_snaps) for a `Body3d` whose
//! [`display.edges`](cad_domain::BodyDisplay::edges) are **real stored document
//! edges** (the SketchUp preview carriers, ADR-0049). That path produces
//! endpoint and midpoint candidates only — never face, triangle, center,
//! quadrant, perpendicular or intersection gestures — and the caller owns the
//! claim that the edges are real. With the flag unset (the default) a body
//! contributes nothing, exactly like a mesh.

use cad_db::MeasurementRecord;
use cad_domain::*;
use cad_geometry::{DefaultGeometryEngine, GeometryEngine};

/// The kind of object snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    Quadrant,
    Perpendicular,
    LocalIntersection,
}

impl SnapKind {
    /// Stable identifier for diagnostics and UI labels.
    pub fn as_str(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Center => "center",
            SnapKind::Quadrant => "quadrant",
            SnapKind::Perpendicular => "perpendicular",
            SnapKind::LocalIntersection => "intersection",
        }
    }

    /// Tie-break order: earlier kinds win when distances are equal.
    fn priority(self) -> u8 {
        match self {
            SnapKind::Endpoint => 0,
            SnapKind::Midpoint => 1,
            SnapKind::Center => 2,
            SnapKind::Quadrant => 3,
            SnapKind::Perpendicular => 4,
            SnapKind::LocalIntersection => 5,
        }
    }
}

/// One resolved snap candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapCandidate {
    pub kind: SnapKind,
    pub point: Point3,
    /// Space the candidate's geometry lives in, used for space filtering.
    pub space: SpaceId,
    /// Source entity plus the sub-element the snap came from.
    pub source: SelectionRef,
    /// For an intersection, the second participating entity.
    pub secondary: Option<SelectionRef>,
    /// Precision of the geometry the candidate was derived from.
    pub precision: Precision,
    /// Where the geometry the candidate was derived from came from.
    pub geometry_source: GeometrySource,
    /// Perpendicular distance from the cursor, in logical pixels.
    pub logical_pixel_distance: f64,
}

/// A local snap target: entity geometry plus its source/precision context.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapTarget {
    pub geometry: SemanticGeometry,
    pub source: SelectionRef,
    pub space: SpaceId,
    pub precision: Precision,
    /// Provenance of the target geometry, copied onto every candidate it yields.
    pub geometry_source: GeometrySource,
    /// Whether a `Body3d`'s stored display edges may yield endpoint/midpoint
    /// candidates.
    ///
    /// Off by default: a body's display cache is a tessellation, so it only
    /// becomes a snap source when the caller has decided the stored edges are
    /// real (for example an importer that carries SketchUp's own edge records).
    pub body_edge_snaps: bool,
}

impl SnapTarget {
    pub fn new(
        geometry: SemanticGeometry,
        source: SelectionRef,
        space: SpaceId,
        precision: Precision,
    ) -> Self {
        SnapTarget {
            geometry,
            source,
            space,
            precision,
            geometry_source: GeometrySource::Analytic,
            body_edge_snaps: false,
        }
    }

    /// Enable endpoint/midpoint snap on the stored display edges of a `Body3d`.
    ///
    /// Only those two kinds are produced: the display cache carries no face,
    /// triangle, center, quadrant or intersection semantics, so this never
    /// widens the snap set beyond real stored edges.
    #[must_use]
    pub fn with_body_edge_snaps(mut self, geometry_source: GeometrySource) -> Self {
        self.geometry_source = geometry_source;
        self.body_edge_snaps = true;
        self
    }

    /// Build a target with explicit geometry provenance.
    ///
    /// Used for a body that is not edge-snappable (for example an ACIS body):
    /// the candidate still reports where its geometry came from, but the
    /// display-edge path stays disabled.
    #[must_use]
    pub fn with_provenance(mut self, geometry_source: GeometrySource) -> Self {
        self.geometry_source = geometry_source;
        self
    }
}

/// The provenance of one snapped input point, suitable for a measurement record.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapProvenance {
    pub kind: SnapKind,
    pub source: SelectionRef,
    pub secondary: Option<SelectionRef>,
    pub precision: Precision,
    pub geometry_source: GeometrySource,
    pub logical_pixel_distance: f64,
}

impl SnapProvenance {
    /// Build provenance from a resolved candidate.
    pub fn from_candidate(candidate: &SnapCandidate) -> Self {
        SnapProvenance {
            kind: candidate.kind,
            source: candidate.source.clone(),
            secondary: candidate.secondary.clone(),
            precision: candidate.precision.clone(),
            geometry_source: candidate.geometry_source.clone(),
            logical_pixel_distance: candidate.logical_pixel_distance,
        }
    }
}

/// A measurement together with the snap provenance of each input point.
///
/// [`MeasurementRecord`] itself is owned by the database layer and cannot gain
/// a snap field here; this wrapper carries the provenance beside it so a caller
/// can persist both without losing the record's contract.
#[derive(Debug, Clone, PartialEq)]
pub struct SnappedMeasurement {
    pub record: MeasurementRecord,
    pub snaps: Vec<SnapProvenance>,
}

/// Upper bound on the number of local targets for which pairwise intersection
/// snap is attempted. Point snaps are still produced above it; only the
/// quadratic intersection pass is skipped.
pub const MAX_INTERSECTION_TARGETS: usize = 64;

pub(crate) const TAU: f64 = std::f64::consts::TAU;
pub(crate) const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
pub(crate) const EPS: f64 = 1e-12;

/// One raw, unfiltered snap point produced by candidate generation.
pub(crate) struct Raw {
    pub(crate) kind: SnapKind,
    pub(crate) point: Point3,
    pub(crate) key: String,
}

/// Compute every snap candidate within tolerance of the pick ray.
///
/// `targets` must already be the **local** subset selected by the caller. The
/// query is rejected (`InvalidInput`) for a non-finite or degenerate ray, a
/// non-positive/non-finite `world_per_px`, or a non-positive tolerance. A
/// parallel ray that never meets `plane` yields no candidates.
pub fn collect_candidates(
    targets: &[SnapTarget],
    ray: &Ray3,
    plane: Option<&WorkPlane>,
    world_per_px: f64,
    space_filter: Option<&SpaceId>,
    policy: &TolerancePolicy,
) -> CadResult<Vec<SnapCandidate>> {
    let dir = unit_direction(ray)?;
    if !world_per_px.is_finite() || world_per_px <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap world_per_px must be finite and positive".to_string(),
        ));
    }
    let tolerance = policy.interaction_logical_pixels * world_per_px;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap tolerance must be finite and positive".to_string(),
        ));
    }

    let probe = match plane {
        Some(work) => match ray_plane_point(ray, &dir, work) {
            Some(point) => Some(point),
            // A plane was requested but the ray is parallel to it (or hits it
            // behind the origin): there is no cursor position to snap to.
            None => return Ok(Vec::new()),
        },
        None => None,
    };

    let mut out: Vec<SnapCandidate> = Vec::new();
    for target in targets {
        if let Some(filter) = space_filter {
            if target.space != *filter {
                continue;
            }
        }
        if !cad_geometry::geometry_is_finite(&target.geometry) {
            // A poisoned target cannot contribute a trustworthy snap.
            continue;
        }
        let mut raw: Vec<Raw> = Vec::new();
        match body_edge_source(target) {
            Some(edges) => generate_body_edges(edges, &mut raw),
            None => generate(&target.geometry, probe, &mut raw, ""),
        }
        for r in raw {
            if let Some(candidate) = accept(
                r,
                target,
                None,
                ray,
                &dir,
                probe,
                world_per_px,
                tolerance,
                policy,
            ) {
                out.push(candidate);
            }
        }
    }

    // Local intersections only: pairs within the supplied local set, so this is
    // never an all-pairs pass over the drawing.
    if targets.len() <= MAX_INTERSECTION_TARGETS {
        let engine = DefaultGeometryEngine;
        for i in 0..targets.len() {
            for j in (i + 1)..targets.len() {
                let (a, b) = (&targets[i], &targets[j]);
                if a.space != b.space || !engine_is_local(a, b) {
                    continue;
                }
                if let Some(filter) = space_filter {
                    if a.space != *filter {
                        continue;
                    }
                }
                if !cad_geometry::geometry_is_finite(&a.geometry)
                    || !cad_geometry::geometry_is_finite(&b.geometry)
                {
                    continue;
                }
                let points = engine.intersect_local(&a.geometry, &b.geometry, policy)?;
                for point in points {
                    let raw = Raw {
                        kind: SnapKind::LocalIntersection,
                        point,
                        key: "intersection".to_string(),
                    };
                    if let Some(candidate) = accept(
                        raw,
                        a,
                        Some(b.source.clone()),
                        ray,
                        &dir,
                        probe,
                        world_per_px,
                        tolerance,
                        policy,
                    ) {
                        out.push(candidate);
                    }
                }
            }
        }
    }

    dedup(&mut out);
    out.sort_by(|x, y| {
        x.logical_pixel_distance
            .partial_cmp(&y.logical_pixel_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.kind.priority().cmp(&y.kind.priority()))
            .then_with(|| {
                x.point
                    .x
                    .partial_cmp(&y.point.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                x.point
                    .y
                    .partial_cmp(&y.point.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(out)
}

/// Best candidate by logical pixel distance, if any.
pub fn best_candidate(
    targets: &[SnapTarget],
    ray: &Ray3,
    plane: Option<&WorkPlane>,
    world_per_px: f64,
    space_filter: Option<&SpaceId>,
    policy: &TolerancePolicy,
) -> CadResult<Option<SnapCandidate>> {
    Ok(
        collect_candidates(targets, ray, plane, world_per_px, space_filter, policy)?
            .into_iter()
            .next(),
    )
}

mod generate;
mod resolve;

use generate::{generate, generate_body_edges};
use resolve::{accept, dedup, engine_is_local, ray_plane_point, unit_direction};

/// The stored display edges of a body when the target allows edge snapping.
///
/// Returns `None` for every other target, so the ordinary analytic generation
/// path is untouched (and `generate` keeps ignoring `Body3d`).
fn body_edge_source(target: &SnapTarget) -> Option<&[Vec<Point3>]> {
    if !target.body_edge_snaps {
        return None;
    }
    match &target.geometry {
        SemanticGeometry::Body3d(body) if !body.display.edges.is_empty() => {
            Some(&body.display.edges)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
