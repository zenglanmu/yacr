//! HATCHPOINT: create a HATCH from one **internal point** (project-specific
//! selector, like SELECTINBOX/GRIPSTRETCH).
//!
//! The command picks the **smallest visible closed boundary** containing the
//! picked point, includes every nested closed boundary as an island, and fills
//! with the existing HATCH parameters. Discovery runs over the drawing's
//! model-space entities; the current selection is **not** its input.
//!
//! # Boundary discovery (v1)
//!
//! Two kinds of boundary are discovered in the seed's `+Z` plane:
//!
//! * **whole visible closed entities** — `Circle`, full `Ellipse`, closed
//!   `Polyline`;
//! * **`LINE`/`ARC` edge faces** — the visible `Line`/`Arc` entities in that
//!   plane are chained into a planar half-edge graph at the absolute
//!   [`super::hatch_stitch::STITCH_TOLERANCE`] of `1e-6` world units, every face
//!   of the graph is walked, and the face holding the seed plus every face nested
//!   inside it becomes exact bulge loops (an `ARC` is `tan(sweep / 4)`, signed by
//!   the traversal; nothing is sampled). Dangling edges bound no face and are
//!   pruned, so an unrelated open chain never fails the pick. Branch nodes are
//!   handled by the face walk, never refused.
//!
//! The discovered loops are combined with the existing rule: the **smallest-area
//! containing** loop is the outer boundary and every loop inside it (all nesting
//! depths) is an island. The whole set goes through `HatchGeometry::new` as the
//! single creation-path validator; the even-odd fill/style rules decide holes.
//!
//! # Declared v1 limits (do not read more into this command)
//!
//! * edge faces cover only **`LINE`/`ARC`** boundaries: `SPLINE` and ellipse-arc
//!   edges are not stitched, and an `ARC` with `|sweep| > 2π` (or a degenerate
//!   sweep) is refused as an edge (skipped, never sampled or approximated);
//! * only boundaries in the seed's `+Z` plane (same elevation) are considered —
//!   other planes, hidden layers, model-space violations and unresolved refs are
//!   skipped, never projected or invented; block-instance edges are not followed;
//! * a seed on any candidate boundary, with no containing boundary, or with two
//!   equal-area containing boundaries is an explicit refusal, never a guess;
//! * a self-intersecting face is refused by the `HatchGeometry::new` validator
//!   (never repaired); the walk itself only refuses an inconsistent fan
//!   (`hatch.boundary_face_degenerate`), which no valid input can trigger;
//! * boundary entities are retained (no associativity), one transaction / one
//!   undo step on the active layer;
//! * the existing `HatchGeometry::new` creation-path validator measures the
//!   loop's **chords**, so its chord/bulge blind spot still applies here.
//!
//! The final pipeline is the same one [`super::hatch::create_hatch`] uses
//! (`HatchGeometry::new` → `commit_new_entity`), and the loops are built in the
//! seed's own plane frame, so both creation paths accept exactly the same loop
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
use super::hatch_stitch::{
    angle_within, walk_faces, EdgeCurve, FaceLoop, PlanarEdge, DEGENERATE_SWEEP, STITCH_TOLERANCE,
};

use crate::picking::ref_visibility;

/// A seed this close (2D world units) to a candidate ring lies "on the edge":
/// the pick is refused instead of resolving an arbitrary side.
const SEED_EDGE_TOLERANCE: f64 = 1e-6;

/// Relative area tie tolerance for the equal-area ambiguity refusal.
const AREA_TIE_TOLERANCE: f64 = 1e-9;

/// The upward `+Z` tolerance for a candidate plane normal.
const UPWARD_NORMAL_TOLERANCE: f64 = 1e-9;

/// One discovered boundary, in the seed plane's OCS 2D frame.
///
/// Shared with [`super::boundary::Application::create_boundary`] (BOUNDARY,
/// 2026-11-07): both commands consume the same discovery pipeline, the same
/// `boundary_on_edge` / `boundary_not_found` / `boundary_ambiguous` rules and the
/// same loop frame. HATCHPOINT uses both whole-entity loops and stitched
/// `LINE`/`ARC` faces; BOUNDARY only the whole-entity half.
pub(crate) struct CandidateBoundary {
    /// The boundary loop itself (bulges kept exact).
    pub(crate) loop_: HatchLoop,
    /// The loop's bulge-sampled ring, used for containment and area only.
    pub(crate) ring: Vec<[f64; 2]>,
    /// The absolute shoelace area of the sampled ring (world units squared).
    pub(crate) area: f64,
    /// Boundary edges the loop consumes (`0` for a whole closed entity).
    pub(crate) edge_count: usize,
}

/// Why the shared boundary pick could not resolve a loop set.
///
/// The two callers map this onto their own stable reason-code prefixes:
/// `hatch.boundary_*` for HATCHPOINT and `boundary.boundary_*` for BOUNDARY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundaryPickError {
    /// The seed lies on a candidate loop.
    OnEdge,
    /// No visible closed candidate contains the seed.
    NotFound,
    /// Two containing candidates have the same area.
    Ambiguous,
}

impl BoundaryPickError {
    /// The stable suffix shared by both commands (`on_edge`, `not_found`,
    /// `ambiguous`).
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::OnEdge => "on_edge",
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
        }
    }

    /// The human-readable detail, identical for both callers.
    pub(crate) fn detail(self) -> &'static str {
        match self {
            Self::OnEdge => {
                "the internal point lies on a candidate boundary; pick clearly inside or outside"
            }
            Self::NotFound => "no visible closed boundary contains the picked point",
            Self::Ambiguous => "two boundaries contain the point with the same area",
        }
    }
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
    /// * an inconsistent `LINE`/`ARC` half-edge fan
    ///   (`hatch.boundary_face_degenerate`);
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
        let plane = seed_plane(seed);
        let seed_2d = plane_projection(&plane)(seed);
        let (loops, outer_area, island_count, edge_count) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let database = document.drawing.as_ref();
            let scope = database.root_layer_scope();
            let candidates = discover_candidates(database, session, document_id, scope, seed)?;
            let (outer, islands) =
                choose_boundary(&candidates, seed_2d).map_err(hatch_pick_refusal)?;
            let outer_area = outer.area;
            let island_count = islands.len();
            let edge_count = std::iter::once(outer)
                .chain(islands.iter().copied())
                .map(|candidate| candidate.edge_count)
                .sum();
            let mut loops = Vec::with_capacity(island_count + 1);
            loops.push(outer.loop_.clone());
            loops.extend(islands.into_iter().map(|island| island.loop_.clone()));
            (loops, outer_area, island_count, edge_count)
        };

        let loop_count = loops.len();

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch =
            HatchGeometry::new(loops, plane, style, fill, vec![seed_2d], None, pattern_type)
                .map_err(hatch_creation_refusal)?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let mut outcome =
            self.commit_new_entity(session, command, geometry, "create hatch from point")?;
        if let Some(object) = outcome.objects.first().copied() {
            outcome.diagnostics.push(hatch_diagnostic(
                object,
                loop_count,
                edge_count,
                style,
                &parameters,
            ));
            outcome.diagnostics.push(Diagnostic {
                object: Some(object),
                code: "hatch.point_boundary".into(),
                message: format!(
                    "HATCHPOINT: seed ({}, {}, {}) selected the smallest visible closed boundary \
                     (outer area {outer_area}, {island_count} island(s), {edge_count} LINE/ARC \
                     edge(s)); whole visible closed entities plus stitched LINE/ARC edge faces in \
                     the seed's +Z plane, model space only",
                    seed.x, seed.y, seed.z
                ),
            });
        }
        Ok(outcome)
    }
}

/// Classify every visible boundary candidate: whole closed entities, then the
/// `LINE`/`ARC` faces.
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
    let mut candidates = discover_closed_candidates(database, session, document_id, scope, seed);
    candidates.extend(discover_edge_faces(
        database,
        session,
        document_id,
        scope,
        seed,
    )?);
    Ok(candidates)
}

/// The **whole visible closed entity** candidates only.
///
/// This is the half BOUNDARY (2026-11-07) shares: it deliberately excludes the
/// `LINE`/`ARC` edge faces `discover_candidates` adds for HATCHPOINT, because
/// BOUNDARY v1 documents the same whole-entity source set as HATCHPOINT's
/// declared limits (`Circle`, full `Ellipse`, closed `Polyline`).
pub(crate) fn discover_closed_candidates(
    database: &DrawingDatabase,
    session: &SessionState,
    document_id: DocumentId,
    scope: cad_db::LayerScope,
    seed: Point3,
) -> Vec<CandidateBoundary> {
    let plane = seed_plane(seed);
    let mut candidates = Vec::new();
    for entity in database.model_space() {
        let reference = selection_ref(document_id, entity.id);
        if ref_visibility(
            database,
            &session.layer_overrides,
            &session.hidden_entities,
            scope,
            &reference,
        ) != Some(true)
        {
            continue;
        }
        let Some(loop_) = closed_entity_loop(entity, &plane, seed) else {
            continue;
        };
        if let Some(candidate) = boundary_candidate(loop_, 0) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn selection_ref(document_id: DocumentId, entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: document_id,
        entity,
        instance: InstancePath(Vec::new()),
        sub_element: None,
    }
}

/// The single loop of a whole visible closed entity, or `None` when the entity is
/// not a candidate.
///
/// The declared v1 kinds are checked first, then the entity runs the same
/// classify/stitch/plane/loop pipeline HATCH uses. The loop is projected into the
/// **seed plane** (all candidates are already filtered to the seed's `+Z` plane
/// and elevation within [`plane_tolerance`]), so entity loops and edge faces share
/// one frame.
fn closed_entity_loop(entity: &DbEntity, plane: &HatchPlane, seed: Point3) -> Option<HatchLoop> {
    if !is_closed_candidate_geometry(&entity.geometry) {
        return None;
    }
    let shapes = boundary_shapes(std::slice::from_ref(entity)).ok()?;
    let stitched = stitch_boundary(&shapes).ok()?;
    let resolved = resolve_plane(&shapes, &stitched).ok()?;
    if resolved.normal.z < 1.0 - UPWARD_NORMAL_TOLERANCE {
        return None;
    }
    let elevation_tolerance = plane_tolerance(seed.z.abs().max(resolved.elevation.abs()));
    if (resolved.elevation - seed.z).abs() > elevation_tolerance {
        return None;
    }
    let loops = boundary_loops(&shapes, &stitched, plane).ok()?;
    let [loop_] = loops.as_slice() else {
        return None;
    };
    Some(loop_.clone())
}

/// The declared v1 whole-entity candidate kinds: a `Circle`, a full `Ellipse` or a
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

/// Every `LINE`/`ARC` face that may bound the seed's region.
///
/// The visible model-space `Line`/`Arc` entities in the seed's `+Z` plane are
/// handed to the planar face walk. Only the face the walk located around the seed
/// and the faces nested inside it are returned; a graph that holds the seed in no
/// bounded region contributes nothing (the pick then reports
/// `hatch.boundary_not_found`, exactly like a seed outside every closed entity).
fn discover_edge_faces(
    database: &DrawingDatabase,
    session: &SessionState,
    document_id: DocumentId,
    scope: cad_db::LayerScope,
    seed: Point3,
) -> CadResult<Vec<CandidateBoundary>> {
    let mut edges = Vec::new();
    for entity in database.model_space() {
        let reference = selection_ref(document_id, entity.id);
        if ref_visibility(
            database,
            &session.layer_overrides,
            &session.hidden_entities,
            scope,
            &reference,
        ) != Some(true)
        {
            continue;
        }
        if let Some(edge) = edge_from_geometry(&entity.geometry, seed) {
            edges.push(edge);
        }
    }
    if edges.len() < 2 {
        return Ok(Vec::new());
    }
    let plane = seed_plane(seed);
    let faces = walk_faces(&edges, &plane, seed)?;
    let Some(containing) = faces.containing else {
        return Ok(Vec::new());
    };
    let Some(outer) = face_boundary(&faces.faces[containing]) else {
        return Ok(Vec::new());
    };
    let outer_ring = outer.ring.clone();
    let mut candidates = vec![outer];
    for (index, face) in faces.faces.iter().enumerate() {
        if index == containing || face.signed_area <= 0.0 {
            continue;
        }
        let Some(candidate) = face_boundary(face) else {
            continue;
        };
        if cad_domain::hatch::ring_is_inside(&candidate.ring, &outer_ring) {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

/// One face as a hatch candidate, in the seed plane's 2D frame.
fn face_boundary(face: &FaceLoop) -> Option<CandidateBoundary> {
    let loop_ = HatchLoop::new(
        face.points.clone(),
        face.bulges.clone(),
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    );
    boundary_candidate(loop_, face.edge_count)
}

/// Wrap a loop as a candidate, rejecting rings that cannot be boundaries.
fn boundary_candidate(loop_: HatchLoop, edge_count: usize) -> Option<CandidateBoundary> {
    let ring = loop_.sampled_ring();
    if ring.len() < 3 {
        return None;
    }
    let area = ring_area(&ring).abs();
    if !area.is_finite() || area <= 0.0 {
        return None;
    }
    Some(CandidateBoundary {
        loop_,
        ring,
        area,
        edge_count,
    })
}

/// One visible `LINE`/`ARC` entity as a face-walk edge, or `None` when it is not a
/// usable edge in the seed's `+Z` plane.
///
/// Lines must have both endpoints at the seed elevation; arcs must sweep about an
/// upward normal at the seed elevation. A full-turn (or degenerate) arc is not an
/// edge: its endpoints coincide, so it is a self-loop the face walk cannot use —
/// skipped, never split or sampled here.
fn edge_from_geometry(geometry: &SemanticGeometry, seed: Point3) -> Option<PlanarEdge> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !is_finite_point(*start) || !is_finite_point(*end) {
                return None;
            }
            let tolerance = plane_tolerance(seed.z.abs().max(start.z.abs()).max(end.z.abs()));
            if (start.z - seed.z).abs() > tolerance || (end.z - seed.z).abs() > tolerance {
                return None;
            }
            if distance(*start, *end) <= STITCH_TOLERANCE {
                return None;
            }
            Some(PlanarEdge {
                start: *start,
                end: *end,
                curve: EdgeCurve::Line,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            if !is_finite_point(*center)
                || !is_finite_point(*normal)
                || !start.is_finite()
                || !sweep.is_finite()
            {
                return None;
            }
            if !radius.is_finite() || *radius <= 0.0 {
                return None;
            }
            let normal_length = normal.x.hypot(normal.y).hypot(normal.z);
            if !normal_length.is_finite() || normal_length <= 1e-12 {
                return None;
            }
            if normal.z / normal_length < 1.0 - UPWARD_NORMAL_TOLERANCE {
                return None;
            }
            let tolerance = plane_tolerance(seed.z.abs().max(center.z.abs()));
            if (center.z - seed.z).abs() > tolerance {
                return None;
            }
            if sweep.abs() <= DEGENERATE_SWEEP || sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                // A full turn (or an over-turn) has coincident endpoints and is not
                // a face-walk edge; see the doc above.
                return None;
            }
            // The endpoints use the same arbitrary-axis frame the tessellator and
            // the HATCH stitcher use, so the face walk clusters the drawn points.
            let (axis_x, axis_y, _) = cad_geometry::arbitrary_axis(*normal);
            let point_at = |angle: f64| Point3 {
                x: center.x + axis_x.x * radius * angle.cos() + axis_y.x * radius * angle.sin(),
                y: center.y + axis_x.y * radius * angle.cos() + axis_y.y * radius * angle.sin(),
                z: center.z + axis_x.z * radius * angle.cos() + axis_y.z * radius * angle.sin(),
            };
            let start_point = point_at(*start);
            let end_point = point_at(*start + *sweep);
            if distance(start_point, end_point) <= STITCH_TOLERANCE {
                return None;
            }
            Some(PlanarEdge {
                start: start_point,
                end: end_point,
                curve: EdgeCurve::Arc {
                    center: *center,
                    radius: *radius,
                    sweep: *sweep,
                },
            })
        }
        _ => None,
    }
}

/// Pick the containing boundary with the smallest area, then its islands.
///
/// First the on-edge refusal (applies to **every** candidate, with the exact bulge
/// geometry — never the sampled fill ring), then containment, then the smallest-
/// area choice with the equal-area ambiguity check, then every other candidate
/// whose ring lies inside the chosen outer ring (all nesting depths — the existing
/// even-odd fill/style rules decide holes).
pub(crate) fn choose_boundary(
    candidates: &[CandidateBoundary],
    seed_2d: [f64; 2],
) -> Result<(&CandidateBoundary, Vec<&CandidateBoundary>), BoundaryPickError> {
    for candidate in candidates {
        if loop_distance(seed_2d, &candidate.loop_) <= SEED_EDGE_TOLERANCE {
            return Err(BoundaryPickError::OnEdge);
        }
    }

    let mut containing: Vec<&CandidateBoundary> = candidates
        .iter()
        .filter(|candidate| cad_domain::hatch::point_in_ring(seed_2d, &candidate.ring))
        .collect();
    if containing.is_empty() {
        return Err(BoundaryPickError::NotFound);
    }
    containing.sort_by(|a, b| a.area.partial_cmp(&b.area).unwrap_or(Ordering::Equal));
    if let [first, second, ..] = containing.as_slice() {
        let largest = first.area.abs().max(second.area.abs());
        if (second.area - first.area).abs() <= AREA_TIE_TOLERANCE * largest {
            return Err(BoundaryPickError::Ambiguous);
        }
    }
    let outer = containing[0];
    let islands: Vec<&CandidateBoundary> = candidates
        .iter()
        .filter(|candidate| {
            !std::ptr::eq(*candidate, outer)
                && cad_domain::hatch::ring_is_inside(&candidate.ring, &outer.ring)
        })
        .collect();
    Ok((outer, islands))
}

/// Map the shared pick error onto HATCHPOINT's stable `hatch.boundary_*` codes.
fn hatch_pick_refusal(error: BoundaryPickError) -> CadError {
    CadError::InvalidInput(format!(
        "hatch.boundary_{}: {}",
        error.code(),
        error.detail()
    ))
}

/// The seed's own `+Z` hatch plane.
///
/// Every discovered candidate is already constrained to an upward normal at the
/// seed elevation within [`plane_tolerance`]; using the seed plane for the loop
/// frame keeps entity loops and `LINE`/`ARC` faces in one frame (for a `+Z` normal
/// the projection is simply `(x, y)`, so this is the frame the pipeline used).
pub(crate) fn seed_plane(seed: Point3) -> HatchPlane {
    HatchPlane::new(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        seed.z,
    )
}

/// The absolute 2D distance from `point` to a bulged loop's boundary.
///
/// A `LINE` segment uses the plain point-to-segment distance and a bulged segment
/// the exact circular arc (centre/radius recovered from the bulge, then the swept
/// angular range tested), so neither the on-edge rule nor the area rule depends on
/// the arc sampling the fill uses.
fn loop_distance(point: [f64; 2], loop_: &HatchLoop) -> f64 {
    let count = loop_.points.len();
    if count == 0 {
        return f64::INFINITY;
    }
    let mut best = f64::INFINITY;
    for index in 0..count {
        let a = loop_.points[index];
        let b = loop_.points[(index + 1) % count];
        let bulge = loop_.bulges.get(index).copied().unwrap_or(0.0);
        let distance = if bulge == 0.0 {
            distance_to_segment(point, a, b)
        } else {
            distance_to_arc(point, a, b, bulge)
        };
        best = best.min(distance);
    }
    best
}

/// The absolute 2D distance from `point` to the circular arc `a → b` of `bulge`.
fn distance_to_arc(point: [f64; 2], a: [f64; 2], b: [f64; 2], bulge: f64) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let chord = dx.hypot(dy);
    if !bulge.is_finite() || chord <= f64::MIN_POSITIVE {
        return distance_to_segment(point, a, b);
    }
    let sweep = 4.0 * bulge.atan();
    let half = sweep / 2.0;
    let (sin_half, cos_half) = half.sin_cos();
    if sin_half.abs() <= 1e-12 {
        return distance_to_segment(point, a, b);
    }
    // The same frame the domain sampler uses: a positive bulge puts the centre on
    // the left of `a → b`.
    let radius = chord / (2.0 * sin_half);
    let middle = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
    let left = [-dy / chord, dx / chord];
    let center = [
        middle[0] + left[0] * radius * cos_half,
        middle[1] + left[1] * radius * cos_half,
    ];
    let start_angle = (a[1] - center[1]).atan2(a[0] - center[0]);
    let theta = (point[1] - center[1]).atan2(point[0] - center[0]);
    if angle_within(theta, start_angle, sweep) {
        ((point[0] - center[0]).hypot(point[1] - center[1]) - radius.abs()).abs()
    } else {
        (point[0] - a[0])
            .hypot(point[1] - a[1])
            .min((point[0] - b[0]).hypot(point[1] - b[1]))
    }
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
