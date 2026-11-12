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
//! The whole set is inserted through the shared create funnel in **one
//! transaction / one undo step**; the source boundary entities are retained
//! (**no associativity**). The current selection is not the input.
//!
//! Full contract and refusal list: [`crate::command_id::boundary`]. `-BOUNDARY`
//! options (boundary set, island detection off, region output, object type) are
//! **NOT IMPLEMENTED**.

use super::*;

use super::hatch_boundary::plane_projection;
use super::hatch_internal_point::{
    choose_boundary, discover_closed_candidates, seed_plane, BoundaryPickError,
};
use super::hatch_utilities::loop_world_ring;

impl Application {
    /// BOUNDARY: one closed polyline per boundary loop discovered around `seed`.
    ///
    /// See the module docs and [`crate::command_id::boundary`] for the exact
    /// source set, output shape and refusals. Every refusal happens before
    /// `commit_new_entities`, so nothing is written and no transaction exists.
    pub(crate) fn create_boundary(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::BoundaryPoint { seed } = &command.payload else {
            return Err(refusal(
                "boundary.payload",
                "BOUNDARY needs a BoundaryPoint { seed } payload",
            ));
        };
        let seed = *seed;
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
        let candidates = discover_closed_candidates(drawing, session, document_id, scope, seed);
        let (outer, islands) =
            choose_boundary(&candidates, seed_2d).map_err(boundary_pick_refusal)?;

        let mut geometries = Vec::with_capacity(islands.len() + 1);
        for candidate in std::iter::once(outer).chain(islands) {
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

        let mut outcome =
            self.commit_new_entities(session, command, geometries, "create boundary from point")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "boundary.created".into(),
            message: format!(
                "BOUNDARY: created {loop_count} non-associative closed polyline(s) from the \
                 smallest visible boundary around ({}, {}, {}) and its nested islands; source \
                 entities retained; -BOUNDARY options are not implemented",
                seed.x, seed.y, seed.z
            ),
        });
        Ok(outcome)
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
