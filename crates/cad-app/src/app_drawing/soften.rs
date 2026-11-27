//! `SOFTENEDGES`: SketchUp's Soften Edges on the selected bodies.
//!
//! The command writes **display-only** state: each selected mesh/body entity
//! gets `render_attributes.soften = Some(angle)` (degrees, `0..=180`), and the
//! representation suppresses display edges whose two adjacent triangles meet at
//! or below that angle. Geometry, type, layer and the exact B-rep are untouched.
//!
//! The write is one `update_entity_display` batch, one revision and one undo
//! step; the history patches carry the whole before/after display state, so undo
//! restores the previous angle exactly. A selection that already holds the angle
//! writes nothing (`soften.unchanged`).
//!
//! Refusals: `soften.angle_invalid` (non-finite or outside `0..=180`),
//! `soften.selection_required`, `soften.not_addressable` (a block member or
//! sub-element), `soften.not_a_body` (a selected row that is not a 3D body),
//! `soften.target_missing`, `soften.unchanged`.
//!
//! A body with **no** stored display edge (a plain triangle mesh whose
//! wireframe is its surface boundary) has nothing to soften; the angle is still
//! stored and takes effect when such a body gains stored edges (for example a
//! SKP import). Smooth-normal shading is **not** implemented: this command hides
//! edges, it does not blend face gradients.

use super::*;
use cad_db::EntityRenderAttributes;

/// The angle a bare `SOFTENEDGES` applies (SketchUp's default).
pub const DEFAULT_SOFTEN_ANGLE: f64 = 20.0;

impl Application {
    /// `SOFTENEDGES [<angle>]`: soften the selected bodies' coplanar edges.
    pub(crate) fn soften_selected_edges(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let angle = match &command.payload {
            CommandPayload::None => DEFAULT_SOFTEN_ANGLE,
            CommandPayload::SoftenEdges(angle) => *angle,
            _ => {
                return Err(CadError::InvalidInput(
                    "softenedges.requires_payload: SOFTENEDGES takes an optional angle in \
                     degrees"
                        .into(),
                ))
            }
        };
        if !angle.is_finite() || !(0.0..=180.0).contains(&angle) {
            return Err(CadError::InvalidInput(format!(
                "soften.angle_invalid: the angle must be finite and in 0..=180 degrees, got \
                 {angle}"
            )));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(CadError::InvalidInput(
                "soften.selection_required: select at least one 3D body to soften".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // Preflight every ref: one bad ref refuses the whole command.
        for reference in &references {
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "soften.not_addressable: block-instance members and sub-elements have no \
                     directly addressable body identity (entity {})",
                    reference.entity.0
                )));
            }
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "soften.target_missing: entity {} does not exist in this drawing",
                    reference.entity.0
                ))
            })?;
            if !matches!(entity.geometry, SemanticGeometry::Body3d(_)) {
                return Err(CadError::InvalidInput(format!(
                    "soften.not_a_body: entity {} is {}, not a 3D body",
                    reference.entity.0, entity.object.type_key
                )));
            }
        }

        // Only entities whose stored angle actually changes are staged.
        let mut changed: Vec<EntityId> = Vec::new();
        for reference in &references {
            if changed.contains(&reference.entity) {
                continue;
            }
            let attributes = drawing.entity_render_attributes(reference.entity);
            if attributes.soften != Some(angle) {
                changed.push(reference.entity);
            }
        }
        if changed.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "soften.unchanged".into(),
                    message: format!(
                        "SOFTENEDGES: the selected bodies already soften at {angle} degrees"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let next: Vec<(EntityId, EntityRenderAttributes)> = changed
            .iter()
            .map(|id| {
                let mut attributes = candidate.entity_render_attributes(*id);
                attributes.soften = Some(angle);
                (*id, attributes)
            })
            .collect();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("soften edges", transaction)?;
            for (id, attributes) in next {
                staged.update_entity_display(id, attributes)?;
            }
            staged.commit()?
        };

        let patches: Vec<DrawingPatch> = changed
            .iter()
            .map(|id| DrawingPatch {
                id: *id,
                before: drawing.entity(*id).cloned(),
                after: candidate.entity(*id).cloned(),
                before_state: Some(drawing.entity_display_state(*id)),
                after_state: Some(candidate.entity_display_state(*id)),
            })
            .collect();
        let objects: Vec<ObjectId> = changed.iter().map(|id| ObjectId(id.0)).collect();
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: format!("soften edges {angle} degrees"),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "soften.applied".into(),
                message: format!(
                    "SOFTENEDGES: {count} body/bodies soften edges at {angle} degrees"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
