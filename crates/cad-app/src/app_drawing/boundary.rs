//! `BOUNDARY` (AutoCAD 2026): create closed polylines from one **internal point**
//! (project-specific selector, like HATCHPOINT).
//!
//! The command reuses HATCHPOINT's internal-point discovery
//! ([`super::hatch_internal_point`]): the smallest visible closed boundary that
//! contains the seed is the outer loop and every nested closed boundary is an
//! island. It consumes only the **whole-visible-closed-entity** half of that
//! discovery, which is exactly the source set its sibling documents: `Circle`,
//! full `Ellipse` and closed `Polyline` in the seed's `+Z` plane. `LINE`/`ARC`
//! edge chains, block-instance edges, hidden geometry, other planes and unresolved
//! refs are skipped, never projected or invented.
//!
//! Every discovered loop becomes one closed `AcDbPolyline` on the **active
//! layer** in model space:
//!
//! * the loop's hatch-plane 2D points and bulges are mapped to world `+Z`
//!   coordinates with the same plane frame the fill uses;
//! * bulges are preserved and a repeated last point (with its degenerate closing
//!   bulge) is dropped, because a closed polyline implies the closing segment;
//! * straight rings get one zero bulge per vertex.
//!
//! The two `-BOUNDARY` options ride the payload
//! ([`crate::command_id::boundary::BoundaryOptions`]):
//!
//! * **Boundary set** — `Some(refs)` replaces the whole-drawing auto-discovery with
//!   exactly those entities; any ref that is not a visible, directly addressable,
//!   closed candidate in the seed plane is a named refusal, never silently dropped
//!   and never widened back to the whole drawing;
//! * **Island detection** — `false` writes only the outer loop and names every
//!   skipped nested loop; `true` (the default) writes the outer loop plus every
//!   nested loop as **separate** closed polylines and says so. A closed polyline
//!   carries no hole list, so the outer/island relation exists only in the named
//!   diagnostic — never silently dropped.
//!
//! The whole set is inserted through the shared create funnel in **one
//! transaction / one undo step**; the source boundary entities are retained
//! (**no associativity**).
//!
//! Full contract and refusal list: [`crate::command_id::boundary`].

use super::*;

use super::hatch_boundary::plane_projection;
use super::hatch_internal_point::{
    boundary_candidate, choose_boundary, closed_entity_loop, discover_closed_candidates,
    seed_plane, BoundaryPickError, CandidateBoundary,
};
use super::hatch_utilities::loop_world_ring;

use crate::command_id::boundary::BoundaryOptions;
use crate::picking::ref_visibility;

impl Application {
    /// BOUNDARY: one closed polyline per boundary loop discovered around `seed`.
    ///
    /// See the module docs and [`crate::command_id::boundary`] for the exact
    /// source set, options, output shape and refusals. Every refusal happens
    /// before `commit_new_entities`, so nothing is written and no transaction
    /// exists.
    pub(crate) fn create_boundary(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::BoundaryPoint(input) = &command.payload else {
            return Err(refusal(
                "boundary.payload",
                "BOUNDARY needs a BoundaryPoint(BoundaryInput { seed, options }) payload",
            ));
        };
        let seed = input.seed;
        let options = &input.options;
        if !is_finite_point(seed) {
            return Err(refusal(
                "boundary.not_planar",
                "the internal point must be finite and lie in the +Z model plane",
            ));
        }
        if session.active_space != SpaceId::Model {
            return Err(refusal(
                "boundary.not_planar",
                "BOUNDARY currently supports model space only",
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal("boundary.not_addressable", "document not open"))?;
        let drawing = document.drawing.as_ref();
        if drawing.layer(session.active_layer).is_none() {
            return Err(refusal(
                "boundary.not_addressable",
                format!("active layer {} does not exist", session.active_layer.0),
            ));
        }

        let plane = seed_plane(seed);
        let seed_2d = plane_projection(&plane)(seed);
        let scope = drawing.root_layer_scope();
        let candidates = match &options.boundary_set {
            Some(refs) => {
                boundary_set_candidates(drawing, session, document_id, scope, seed, refs)?
            }
            None => discover_closed_candidates(drawing, session, document_id, scope, seed),
        };
        let (outer, islands) =
            choose_boundary(&candidates, seed_2d).map_err(boundary_pick_refusal)?;

        let island_count = islands.len();
        let loops: Vec<&CandidateBoundary> = if options.island_detection {
            std::iter::once(outer)
                .chain(islands.iter().copied())
                .collect()
        } else {
            vec![outer]
        };
        let mut geometries = Vec::with_capacity(loops.len());
        for candidate in loops {
            let (points, bulges) = loop_world_ring(&candidate.loop_, plane)
                .map_err(|detail| refusal("boundary.not_addressable", detail))?;
            geometries.push(SemanticGeometry::Polyline {
                points,
                bulges,
                closed: true,
                width: 0.0,
            });
        }
        if geometries.is_empty() {
            // Defensive: discovery always yields at least the chosen outer loop.
            return Err(refusal(
                "boundary.boundary_not_found",
                "no boundary loop was discovered around the picked point",
            ));
        }
        let loop_count = geometries.len();
        let source = boundary_set_label(options);
        let island_mode = if options.island_detection {
            "ON"
        } else {
            "OFF"
        };

        let mut outcome =
            self.commit_new_entities(session, command, geometries, "create boundary from point")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "boundary.created".into(),
            message: format!(
                "BOUNDARY: created {loop_count} non-associative closed polyline(s) from the \
                 smallest visible boundary around ({}, {}, {}) of {source}; island detection \
                 {island_mode}; source entities retained",
                seed.x, seed.y, seed.z
            ),
        });
        if options.island_detection {
            if island_count > 0 {
                outcome.diagnostics.push(Diagnostic {
                    object: outcome.objects.get(1).copied(),
                    code: "boundary.islands".into(),
                    message: format!(
                        "BOUNDARY island detection ON: {island_count} nested loop(s) written as \
                         separate closed polylines; a closed polyline carries no hole list, so \
                         the outer/hole relation is this diagnostic only"
                    ),
                });
            }
        } else {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "boundary.island_detection_off".into(),
                message: format!(
                    "BOUNDARY island detection OFF (-BOUNDARY option): {island_count} nested \
                     loop(s) inside the outer boundary were NOT created; no hole is ever implied \
                     by the single outer polyline"
                ),
            });
        }
        Ok(outcome)
    }
}

/// The `-BOUNDARY` **boundary set**: the candidate subset the caller named.
///
/// An empty set is refused (never widened to the whole drawing) and every ref must
/// resolve to a visible, directly addressable, model-space whole closed candidate
/// in the seed's `+Z` plane — the same source kinds the automatic discovery uses.
/// A violation is a named refusal before any write: the set narrows the search, it
/// never guesses an entity the caller did not name.
fn boundary_set_candidates(
    drawing: &DrawingDatabase,
    session: &SessionState,
    document_id: DocumentId,
    scope: cad_db::LayerScope,
    seed: Point3,
    refs: &[SelectionRef],
) -> CadResult<Vec<CandidateBoundary>> {
    if refs.is_empty() {
        return Err(refusal(
            "boundary.set_empty",
            "the -BOUNDARY boundary set is empty; name at least one entity or leave Boundary \
             set unset (every visible closed entity)",
        ));
    }
    let plane = seed_plane(seed);
    let mut candidates = Vec::with_capacity(refs.len());
    let mut seen: Vec<EntityId> = Vec::with_capacity(refs.len());
    for reference in refs {
        if reference.document != document_id {
            return Err(refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} is a block-instance member or sub-element; the \
                     boundary set takes directly addressable entities",
                    reference.entity.0
                ),
            ));
        }
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} is not present in the drawing",
                    reference.entity.0
                ),
            )
        })?;
        if entity.space != SpaceId::Model {
            return Err(refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} is not a model-space entity",
                    reference.entity.0
                ),
            ));
        }
        if ref_visibility(
            drawing,
            &session.layer_overrides,
            &session.hidden_entities,
            scope,
            reference,
        ) != Some(true)
        {
            return Err(refusal(
                "boundary.set_not_candidate",
                format!("boundary-set entity {} is hidden", reference.entity.0),
            ));
        }
        let loop_ = closed_entity_loop(entity, &plane, seed).ok_or_else(|| {
            refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} is not a visible closed circle, full ellipse or \
                     closed polyline in the seed's +Z plane",
                    reference.entity.0
                ),
            )
        })?;
        let candidate = boundary_candidate(loop_, 0).ok_or_else(|| {
            refusal(
                "boundary.set_not_candidate",
                format!(
                    "boundary-set entity {} has no non-degenerate boundary loop",
                    reference.entity.0
                ),
            )
        })?;
        if seen.contains(&entity.id) {
            continue;
        }
        seen.push(entity.id);
        candidates.push(candidate);
    }
    Ok(candidates)
}

/// The boundary-set label used by the success diagnostic.
fn boundary_set_label(options: &BoundaryOptions) -> String {
    match &options.boundary_set {
        Some(refs) => format!(
            "the named {}-entity boundary set",
            refs.iter()
                .map(|reference| reference.entity)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        ),
        None => "every visible closed entity".to_string(),
    }
}

/// Map the shared pick error onto BOUNDARY's stable `boundary.boundary_*` codes.
fn boundary_pick_refusal(error: BoundaryPickError) -> CadError {
    refusal(
        &format!("boundary.boundary_{}", error.code()),
        error.detail(),
    )
}

/// A command refusal carrying its stable reason code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
