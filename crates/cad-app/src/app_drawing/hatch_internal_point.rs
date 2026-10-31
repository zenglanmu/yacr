//! HATCHPOINT: create a HATCH from one **internal point** (project-specific
//! selector, like SELECTINBOX/GRIPSTRETCH).
//!
//! The command picks the **smallest visible closed boundary** containing the
//! picked point, includes every nested closed boundary as an island, and fills
//! with the existing HATCH parameters. Discovery runs over the drawing's
//! model-space entities; the current selection is **not** its input.
//!
//! # Declared v1 limits (do not read more into this command)
//!
//! * only **whole visible** `Circle`, full `Ellipse` and closed `Polyline`
//!   entities are candidates; `LINE`/`ARC` edge chains are **not**
//!   auto-discovered (that needs the stitched-loop discovery this v1 does not
//!   do), and the current selection is not consulted;
//! * only boundaries in the seed's `+Z` plane (same elevation) are considered —
//!   other planes, hidden layers and unresolved refs are skipped, never
//!   projected or invented;
//! * a seed on a candidate boundary, with no containing boundary, or with two
//!   equal-area containing boundaries is an explicit refusal, never a guess;
//! * boundary entities are retained (no associativity), one transaction / one
//!   undo step on the active layer;
//! * the existing `HatchGeometry::new` creation-path validator measures the
//!   loop's **chords**, so its chord/bulge blind spot still applies here.
//!
//! The pipeline is deliberately the same one [`super::hatch::create_hatch`]
//! uses (`boundary_shapes` → `stitch_boundary` → `resolve_plane` →
//! `boundary_loops`), so both creation paths accept exactly the same loop
//! construction.

use std::cmp::Ordering;

use super::*;

use super::hatch_boundary::{
    boundary_loops, boundary_shapes, plane_projection, plane_tolerance, resolve_plane,
    stitch_boundary,
};
use super::hatch_parameters::{
    hatch_creation_refusal, hatch_diagnostic, hatch_parameters, hatch_style,
};

use crate::picking::ref_visibility;

/// A seed this close (2D world units) to a candidate ring lies "on the edge":
/// the pick is refused instead of resolving an arbitrary side.
const SEED_EDGE_TOLERANCE: f64 = 1e-6;

/// Relative area tie tolerance for the equal-area ambiguity refusal.
const AREA_TIE_TOLERANCE: f64 = 1e-9;

/// The upward `+Z` tolerance for a candidate plane normal.
const UPWARD_NORMAL_TOLERANCE: f64 = 1e-9;

/// One whole visible closed entity that could be a boundary, in the seed plane.
struct CandidateBoundary {
    entity: DbEntity,
    plane: HatchPlane,
    /// The loop's bulge-sampled ring, in the candidate plane's OCS 2D frame.
    ring: Vec<[f64; 2]>,
    /// The absolute shoelace area of the ring (world units squared).
    area: f64,
}

impl Application {
    /// HATCHPOINT: fill the smallest visible closed boundary around `seed`.
    ///
    /// See the module docs for the declared v1 limits. One transaction / one
    /// undo step through the shared create funnel, on the active layer, with
    /// `type_key`/`class_name` = `AcDbHatch`; the discovered boundary entities
    /// are **retained** (no associativity).
    ///
    /// Refusals (nothing written, no transaction):
    ///
    /// * a non-finite seed (`hatch.boundary_invalid`);
    /// * a non-model active space;
    /// * a seed on a candidate boundary (`hatch.boundary_on_edge`);
    /// * no visible closed boundary contains the seed (`hatch.boundary_not_found`);
    /// * two containing boundaries of equal area (`hatch.boundary_ambiguous`);
    /// * every `HatchGeometry::new` creation-path rejection, mapped through
    ///   [`hatch_creation_refusal`].
    pub(crate) fn create_hatch_point(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::HatchPoint { seed } = &command.payload else {
            return Err(CadError::InvalidInput(
                "HATCHPOINT needs a HatchPoint { seed } payload".into(),
            ));
        };
        let seed = *seed;
        if !is_finite_point(seed) {
            return Err(CadError::InvalidInput(
                "hatch.boundary_invalid: the internal point must be finite".into(),
            ));
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "HATCHPOINT currently supports model space only".into(),
            ));
        }

        let parameters = hatch_parameters(session)?;
        let style = hatch_style(parameters.style)?;
        let (fill, pattern_type) = parameters.fill()?;

        let document_id = command.document;
        let (loops, plane, outer_area, island_count) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let database = document.drawing.as_ref();
            let scope = database.root_layer_scope();
            let candidates = discover_candidates(database, session, document_id, scope, seed)?;
            let (outer, islands) = choose_boundary(&candidates, seed)?;
            let outer_area = outer.area;
            let island_count = islands.len();
            let mut sources = Vec::with_capacity(island_count + 1);
            sources.push(outer.entity.clone());
            sources.extend(islands.into_iter().map(|island| island.entity.clone()));
            let (loops, plane) = build_loops(&sources, seed)?;
            (loops, plane, outer_area, island_count)
        };

        let loop_count = loops.len();
        let seed_2d = plane_projection(&plane)(seed);

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch =
            HatchGeometry::new(loops, plane, style, fill, vec![seed_2d], None, pattern_type)
                .map_err(hatch_creation_refusal)?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let mut outcome =
            self.commit_new_entity(session, command, geometry, "create hatch from point")?;
        if let Some(object) = outcome.objects.first().copied() {
            // The stitched-edge count is 0 by construction: v1 only discovers
            // whole closed entities, never LINE/ARC chains.
            outcome
                .diagnostics
                .push(hatch_diagnostic(object, loop_count, 0, style, &parameters));
            outcome.diagnostics.push(Diagnostic {
                object: Some(object),
                code: "hatch.point_boundary".into(),
                message: format!(
                    "HATCHPOINT: seed ({}, {}, {}) selected the smallest visible closed boundary \
                     (outer area {outer_area}, {island_count} island(s)); whole visible closed \
                     entities only, LINE/ARC chains and the selection are not used",
                    seed.x, seed.y, seed.z
                ),
            });
        }
        Ok(outcome)
    }
}

/// Classify every visible, closed, in-plane model-space entity as a candidate.
///
/// Hidden/unresolved refs and every non-candidate kind are **skipped** (never
/// refused): auto-discovery must not fail the command because the drawing holds
/// other geometry. The whole boundary pipeline runs per entity; a candidate whose
/// pipeline refuses is likewise skipped, because an invalid ring can never be a
/// boundary (and nothing is invented for it).
fn discover_candidates(
    database: &DrawingDatabase,
    session: &SessionState,
    document_id: DocumentId,
    scope: cad_db::LayerScope,
    seed: Point3,
) -> CadResult<Vec<CandidateBoundary>> {
    let mut candidates = Vec::new();
    for entity in database.model_space() {
        let reference = SelectionRef {
            document: document_id,
            entity: entity.id,
            instance: InstancePath(Vec::new()),
            sub_element: None,
        };
        if ref_visibility(database, &session.layer_overrides, scope, &reference) != Some(true) {
            continue;
        }
        if !is_closed_candidate_geometry(&entity.geometry) {
            continue;
        }

        let shapes = match boundary_shapes(std::slice::from_ref(entity)) {
            Ok(shapes) => shapes,
            Err(_) => continue,
        };
        let stitched = match stitch_boundary(&shapes) {
            Ok(stitched) => stitched,
            Err(_) => continue,
        };
        let plane = match resolve_plane(&shapes, &stitched) {
            Ok(plane) => plane,
            Err(_) => continue,
        };
        if plane.normal.z < 1.0 - UPWARD_NORMAL_TOLERANCE {
            continue;
        }
        let elevation_tolerance = plane_tolerance(seed.z.abs().max(plane.elevation.abs()));
        if (plane.elevation - seed.z).abs() > elevation_tolerance {
            continue;
        }
        let loops = match boundary_loops(&shapes, &stitched, &plane) {
            Ok(loops) => loops,
            Err(_) => continue,
        };
        let [loop_] = loops.as_slice() else {
            continue;
        };
        let ring = loop_.sampled_ring();
        if ring.len() < 3 {
            continue;
        }
        let area = ring_area(&ring).abs();
        if !area.is_finite() {
            continue;
        }
        candidates.push(CandidateBoundary {
            entity: entity.clone(),
            plane,
            ring,
            area,
        });
    }
    Ok(candidates)
}

/// The declared v1 candidate kinds: a whole `Circle`, a full `Ellipse` or a
/// closed `Polyline` with finite fields.
fn is_closed_candidate_geometry(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            is_finite_point(*center)
                && is_finite_point(*normal)
                && radius.is_finite()
                && *radius > 0.0
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            is_finite_point(*center)
                && is_finite_point(*normal)
                && is_finite_point(*major_axis)
                && ratio.is_finite()
                && start.is_finite()
                && sweep.is_finite()
                && sweep.abs() >= std::f64::consts::TAU - UPWARD_NORMAL_TOLERANCE
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            *closed
                && points.len() >= 3
                && points.iter().all(|point| is_finite_point(*point))
                && bulges.iter().all(|bulge| bulge.is_finite())
                && (bulges.is_empty() || bulges.len() == points.len())
        }
        _ => false,
    }
}

/// Pick the containing boundary with the smallest area, then its islands.
///
/// First the on-edge refusal (applies to **every** candidate), then containment,
/// then the smallest-area choice with the equal-area ambiguity check, then every
/// other candidate whose ring lies inside the chosen outer ring (all nesting
/// depths — the existing even-odd fill/style rules decide holes).
fn choose_boundary(
    candidates: &[CandidateBoundary],
    seed: Point3,
) -> CadResult<(&CandidateBoundary, Vec<&CandidateBoundary>)> {
    for candidate in candidates {
        let seed_2d = plane_projection(&candidate.plane)(seed);
        if distance_to_ring(seed_2d, &candidate.ring) <= SEED_EDGE_TOLERANCE {
            return Err(CadError::InvalidInput(
                "hatch.boundary_on_edge: the internal point lies on a candidate boundary; pick \
                 clearly inside or outside"
                    .into(),
            ));
        }
    }

    let mut containing: Vec<&CandidateBoundary> = candidates
        .iter()
        .filter(|candidate| {
            let seed_2d = plane_projection(&candidate.plane)(seed);
            cad_domain::hatch::point_in_ring(seed_2d, &candidate.ring)
        })
        .collect();
    if containing.is_empty() {
        return Err(CadError::InvalidInput(
            "hatch.boundary_not_found: no visible closed boundary contains the picked point".into(),
        ));
    }
    containing.sort_by(|a, b| a.area.partial_cmp(&b.area).unwrap_or(Ordering::Equal));
    if let [first, second, ..] = containing.as_slice() {
        let largest = first.area.abs().max(second.area.abs());
        if (second.area - first.area).abs() <= AREA_TIE_TOLERANCE * largest {
            return Err(CadError::InvalidInput(
                "hatch.boundary_ambiguous: two boundaries contain the point with the same area"
                    .into(),
            ));
        }
    }
    let outer = containing[0];
    let islands: Vec<&CandidateBoundary> = candidates
        .iter()
        .filter(|candidate| {
            candidate.entity.id != outer.entity.id
                && cad_domain::hatch::ring_is_inside(&candidate.ring, &outer.ring)
        })
        .collect();
    Ok((outer, islands))
}

/// Rebuild the final loop set (outer + islands) from the same entity pipeline.
///
/// The final plane is re-checked (`+Z`, seed elevation) so the loop frame the
/// fill and display use is the seed plane, never a candidate's near-coincident
/// plane.
fn build_loops(sources: &[DbEntity], seed: Point3) -> CadResult<(Vec<HatchLoop>, HatchPlane)> {
    let shapes = boundary_shapes(sources)?;
    let stitched = stitch_boundary(&shapes)?;
    let plane = resolve_plane(&shapes, &stitched)?;
    if plane.normal.z < 1.0 - UPWARD_NORMAL_TOLERANCE {
        return Err(CadError::InvalidInput(
            "hatch.boundary_invalid: the discovered boundary is not an upward +Z plane".into(),
        ));
    }
    let elevation_tolerance = plane_tolerance(seed.z.abs().max(plane.elevation.abs()));
    if (plane.elevation - seed.z).abs() > elevation_tolerance {
        return Err(CadError::InvalidInput(
            "hatch.boundary_invalid: the discovered boundary is not on the picked point's plane"
                .into(),
        ));
    }
    let loops = boundary_loops(&shapes, &stitched, &plane)?;
    Ok((loops, plane))
}

/// The absolute shoelace area of a 2D ring.
///
/// Implemented with [`cad_geometry::signed_area`] over `z = 0` points so the
/// area rule is the same one the rest of the geometry layer uses.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let points: Vec<Point3> = ring
        .iter()
        .map(|[x, y]| Point3 {
            x: *x,
            y: *y,
            z: 0.0,
        })
        .collect();
    cad_geometry::signed_area(&points)
}

/// The shortest 2D distance from `point` to the ring's closed segment set.
fn distance_to_ring(point: [f64; 2], ring: &[[f64; 2]]) -> f64 {
    let mut best = f64::INFINITY;
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        best = best.min(distance_to_segment(point, a, b));
    }
    best
}

/// The 2D point-to-segment distance (`segment` is `a → b`).
fn distance_to_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::MIN_POSITIVE {
        return (point[0] - a[0]).hypot(point[1] - a[1]);
    }
    let t = (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0);
    let nearest = [a[0] + t * dx, a[1] + t * dy];
    (point[0] - nearest[0]).hypot(point[1] - nearest[1])
}
