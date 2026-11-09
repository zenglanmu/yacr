//! JOIN edit command: weld the selected curves into one entity (AutoCAD 2026).
//!
//! AutoCAD's `JOIN` welds a selection into one object. The **first pick's type
//! decides the source-object mode** unless the typed line selected
//! `JOIN MULTIPLE`, and the result type follows the source:
//!
//! | source | accepted picks | rule | result |
//! | --- | --- | --- | --- |
//! | LINE | LINEs only | collinear; **gaps allowed**; spans union | LINE |
//! | ARC | ARCs only | same centre/normal/radius, same direction; **gaps allowed**; smallest covering sweep | ARC, or CIRCLE with `Close` when one full turn is covered |
//! | ELLIPSE (elliptical arc) | ELLIPSEs only | same centre/normal/major axis/ratio; **gaps allowed** | ELLIPSE; `Close` requires a full turn |
//! | POLYLINE | LINEs, ARCs, open POLYLINEs | contiguous end-to-end, coplanar (+Z plane) | POLYLINE |
//! | SPLINE | SPLINEs only | contiguous end-to-end, any plane (non-coplanar allowed), same degree, clamped | SPLINE (exact C0 concatenation) |
//!
//! `JOIN MULTIPLE` classifies the whole selection instead:
//!
//! * all collinear LINEs → LINE;
//! * all ARCs on one circle → ARC/CIRCLE;
//! * LINEs/ARCs/POLYLINEs (including the otherwise-invalid **LINE + ARC** mix)
//!   that are contiguous and coplanar → POLYLINE;
//! * pure SPLINE or pure ELLIPSE selections use the source rules;
//! * a spline/elliptical arc mixed with other kinds is
//!   `Unsupported` (`join.multiple_spline_mix`): the exact NURBS conversion of
//!   the mixed curves is not implemented.
//!
//! Named refusals write nothing (the whole plan is resolved before the first
//! write): `join.needs_two`, `join.mixed_kinds`, `join.not_collinear`,
//! `join.not_contiguous`, `join.layer_mismatch`, `join.space_mismatch`,
//! `join.sweep_overflow`, `join.degenerate`, `join.not_closed`,
//! `join.closed_object`, `join.construction_line`, `join.spline_degree`,
//! `join.spline_clamped`, `join.multiple_spline_mix`, `join.noncoplanar`.
//! A CIRCLE, a closed polyline/spline/ellipse and an XLINE/RAY can never be
//! joined. Non-coplanar POLYLINE results are refused (`join.noncoplanar`, a 3D
//! scope boundary); SPLINE joins keep their 3D coordinates.
//!
//! ## Transaction shape
//!
//! The whole plan is computed **before any write**; on success the sources are
//! deleted and the merged entity inserted in **one transaction with exactly one
//! undo step**, so an undo restores every source and removes the merged entity.

use super::*;
use crate::command_payload::JoinOptions;

#[path = "join_pieces.rs"]
mod pieces;
#[path = "join_plan.rs"]
mod plan;

pub(crate) use pieces::{chain_pieces, piece_from_geometry, PolylinePiece};
use plan::plan_join;

/// The world tolerance the JOIN predicates use (the same shape
/// [`super::trim_curves::cut_tolerance`] and the OFFSET helper use).
pub(super) fn join_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// The angular tolerance for continuity at one radius (`angle = arc/r`).
fn angle_tolerance(radius: f64) -> f64 {
    let radius = radius.abs().max(join_tolerance());
    (join_tolerance() / radius).max(1e-12)
}

/// A plan-level JOIN refusal: `InvalidInput` carrying its stable code.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A named capability-gap refusal (nothing is written).
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

impl Application {
    /// JOIN the current selection into one entity.
    ///
    /// The plan is resolved before any write, so every refusal leaves the
    /// drawing, the id allocator and history untouched.
    pub(crate) fn join_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let options = match &command.payload {
            CommandPayload::None => JoinOptions::default(),
            CommandPayload::Join(options) => *options,
            _ => {
                return Err(CadError::InvalidInput(
                    "JOIN takes no payload or a JoinOptions payload".into(),
                ));
            }
        };
        let refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(invalid(
                "join.needs_two",
                format!(
                    "JOIN needs at least two selected entities, got {}",
                    refs.len()
                ),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let (sources, mut diagnostics) =
            Self::resolve_selection(drawing, document_id, &refs, "join", "JOIN", "join", false)?;
        if sources.len() < 2 {
            return Err(invalid(
                "join.needs_two",
                "JOIN needs at least two distinct selected entities",
            ));
        }

        let joined = plan_join(&sources, options)?;

        // One layer and one space for the whole weld: a merge across either would
        // have to pick a winner, which is a silent loss.
        let layer = sources[0].layer;
        let space = sources[0].space.clone();
        for source in &sources[1..] {
            if source.layer != layer {
                return Err(invalid(
                    "join.layer_mismatch",
                    format!(
                        "entity {} is on layer {} but the first pick is on layer {}",
                        source.id.0, source.layer.0, layer.0
                    ),
                ));
            }
            if source.space != space {
                return Err(invalid(
                    "join.space_mismatch",
                    format!(
                        "entity {} is in another space than the first pick; a join never \
                         crosses model/paper space",
                        source.id.0
                    ),
                ));
            }
        }

        // Capture the display state each delete prunes, so undo restores it.
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| drawing.entity_display_state(source.id))
            .collect();

        let geometry = joined.geometry();
        let new_id = drawing.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(new_id.0),
                type_key: class_name(&geometry).to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: new_id,
            layer,
            space,
            geometry,
            // The weld replaces the group in place: it inherits the lowest draw
            // order of its sources instead of jumping above every entity.
            draw_order: sources
                .iter()
                .map(|source| source.draw_order)
                .min()
                .unwrap_or(0),
        };

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("join entities", transaction)?;
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            staged.insert_entity(entity.clone())?;
            staged.commit()?
        };

        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        patches.push(drawing_patch(new_id, None, Some(entity.clone())));

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "join entities".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        diagnostics.push(Diagnostic {
            object: Some(ObjectId(new_id.0)),
            code: "drawing.join".into(),
            message: format!(
                "JOIN: {} entities merged into {} {}",
                sources.len(),
                class_name(&entity.geometry),
                new_id.0
            ),
        });

        Ok(CommandOutcome {
            objects: vec![ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
