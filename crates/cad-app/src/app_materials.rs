//! `PAINT` and `MATERIALS`: SketchUp's Paint Bucket at the command layer.
//!
//! # Shapes
//!
//! * `PAINT <name>` applies one material to the **current selection** (whole,
//!   directly addressable entities; no block-instance path and no sub-element).
//!   The name resolves in the document table first and then in the built-in
//!   palette; painting a built-in creates the document row on first use, so a
//!   document only ever stores the materials it actually uses. One paint is one
//!   `tables/materials.json` write, one revision and one undo step; the first
//!   paint that changes nothing is refused as `paint.no_change`.
//! * `MATERIALS` is the read-only report (the command-level Materials Browser):
//!   the document's materials with their colours and painted-entity counts, plus
//!   the built-in palette names. It never writes.
//!
//! # Refusals
//!
//! `paint.requires_material` (no name), `paint.unknown_material` (neither the
//! document nor the palette has it), `paint.selection_required`,
//! `paint.not_addressable` (a block member or sub-element ref),
//! `paint.selection_changed` (the row is gone), `paint.insert_not_paintable` (an
//! INSERT is not an expanded row, so painting it would render nothing) and
//! `paint.no_change`. A refusal happens before the table write and records no
//! undo step.
//!
//! # Scope
//!
//! Paint is per **entity**: a mesh is one entity, so painting a mesh paints all
//! of it; face-level assignment is not modelled yet. Materials are flat colours
//! (no texture); the mesh texture/PBR channels importers fill are untouched.

use super::*;
use crate::app_extrude::invalid;
use cad_domain::material::{builtin_material, color_hex, markers, MaterialDocument, MATERIAL_CAP};
use cad_history::DrawingUndoRecord;

impl Application {
    /// `PAINT <name>`: assign one material to the current selection.
    pub(crate) fn paint_selection(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::PaintMaterial(name) = &command.payload else {
            return Err(invalid(
                "paint.requires_material",
                "PAINT takes the host-parsed material name (`PAINT <name>`)",
            ));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid(
                "paint.requires_material",
                "PAINT needs a material name",
            ));
        }
        if name.chars().count() > cad_domain::material::MAX_MATERIAL_NAME_CHARS {
            return Err(invalid(
                "paint.requires_material",
                format!(
                    "`{name}` is longer than {} characters",
                    cad_domain::material::MAX_MATERIAL_NAME_CHARS
                ),
            ));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(invalid(
                "paint.selection_required",
                "PAINT applies to the current selection; select at least one object",
            ));
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| invalid("paint.selection_changed", "document not open"))?;
        let drawing = document.drawing.as_ref();
        let painting = drawing.materials().clone();
        let before = painting.clone();

        // Resolve the material: the document row wins, then the built-in
        // palette. A built-in is copied into the document on first use so the
        // archive only stores what the drawing actually paints with.
        let mut next: MaterialDocument = painting;
        let seq = match next.resolve(name) {
            Some(row) => row.seq,
            None => {
                let (builtin_name, color, transparency) =
                    builtin_material(name).ok_or_else(|| {
                        invalid(
                            "paint.unknown_material",
                            format!(
                            "`{name}` is neither a document material nor a built-in palette name \
                             ({})",
                            builtin_names()
                        ),
                        )
                    })?;
                next.ensure(builtin_name, color, transparency)
                    .map_err(|error| CadError::InvalidInput(error.to_string()))?
            }
        };
        let material = next
            .get(seq)
            .cloned()
            .expect("the row was just resolved or created");

        let mut painted = 0_usize;
        for reference in &refs {
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "paint.not_addressable",
                    "PAINT reads whole top-level entities, never a block member or a sub-element",
                ));
            }
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                invalid(
                    "paint.selection_changed",
                    format!("entity {} does not exist any more", reference.entity.0),
                )
            })?;
            if matches!(entity.geometry, SemanticGeometry::Insert { .. }) {
                return Err(invalid(
                    "paint.insert_not_paintable",
                    format!(
                        "entity {} is a block reference; paint its members (an INSERT cascade is \
                         not modelled)",
                        reference.entity.0
                    ),
                ));
            }
            if next.assign(reference.entity, seq).is_err() {
                return Err(invalid(
                    markers::MATERIAL_UNKNOWN,
                    format!("material {} disappeared while painting", seq.0),
                ));
            }
            painted += 1;
        }
        if next == before {
            return Err(invalid(
                "paint.no_change",
                format!(
                    "every selected object already wears `{}`; nothing was written",
                    material.name
                ),
            ));
        }
        if next.len() > MATERIAL_CAP {
            return Err(invalid(
                markers::MATERIAL_CAP_EXCEEDED,
                format!("the material table holds at most {MATERIAL_CAP} rows"),
            ));
        }

        let label = format!("PAINT {}", material.name);
        let transaction = crate::app_drawing::next_transaction_id();
        let patch = cad_db::MaterialPatch {
            before: before.clone(),
            after: next.clone(),
        };
        {
            let document = self
                .workspace
                .documents
                .get_mut(&command.document)
                .ok_or_else(|| invalid("paint.selection_changed", "document not open"))?;
            let changes = std::sync::Arc::make_mut(&mut document.drawing).apply_material_document(
                next.clone(),
                transaction,
                &label,
            )?;
            debug_assert!(changes.scope.tables.contains(&cad_db::TableKind::Materials));
        }
        let record = DrawingUndoRecord::entity_only(transaction, label.clone(), Vec::new());
        if let Err(error) = self
            .history
            .entry(command.document)
            .or_default()
            .record_drawing_with_materials(record, vec![patch.clone()])
        {
            // The table write already landed; a refused history record must not
            // leave a paint the user cannot undo.
            let document = self
                .workspace
                .documents
                .get_mut(&command.document)
                .ok_or_else(|| invalid("paint.selection_changed", "document not open"))?;
            std::sync::Arc::make_mut(&mut document.drawing).apply_material_document(
                before,
                crate::app_drawing::next_transaction_id(),
                "PAINT rollback",
            )?;
            return Err(error);
        }
        let look = material.look();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "paint.applied".into(),
                message: format!(
                    "PAINT: `{}` ({}) on {painted} object(s)",
                    material.name,
                    color_hex(look.color)
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `MATERIALS`: the read-only material report.
    pub(crate) fn list_materials(&self, command: &Command) -> CadResult<CommandOutcome> {
        let rows = self
            .workspace
            .documents
            .get(&command.document)
            .map(|document| document.drawing.as_ref())
            .map(|drawing| {
                let materials = drawing.materials();
                let mut counts: std::collections::BTreeMap<u64, usize> =
                    std::collections::BTreeMap::new();
                for seq in materials.assignments().values() {
                    *counts.entry(seq.0).or_insert(0) += 1;
                }
                materials
                    .materials()
                    .iter()
                    .take(20)
                    .map(|material| {
                        format!(
                            "{} {} ({} painted)",
                            material.name,
                            color_hex(material.color),
                            counts.get(&material.seq.0).copied().unwrap_or(0)
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let document = if rows.is_empty() {
            "no material in this document".to_string()
        } else {
            format!("{} material(s): {}", rows.len(), rows.join(", "))
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "materials.listed".into(),
                message: format!(
                    "MATERIALS: {document}; built-in palette: {}",
                    builtin_names()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The built-in palette names, for the report and the unknown-material refusal.
fn builtin_names() -> String {
    cad_domain::material::BUILTIN_MATERIALS
        .iter()
        .map(|(name, _, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}
