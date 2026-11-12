//! `PEDIT WIDTH <w>`: the uniform polyline-width write (one transaction).
//!
//! Split out of `pedit.rs` to keep that module inside the code-size limit. The
//! method is a private helper of `PEDIT`, not a second option surface; the
//! typed option dispatch and every other option stay in the parent module.
//!
//! `w` is the entity model's **uniform** width, centred on the vertices
//! (AutoCAD's constant LWPOLYLINE width, DXF group 43): `0` is the hairline
//! draw and a positive value is a real wide stroke. The write preserves the
//! points, bulges, closed flag, id, layer, space, draw order and type key of
//! every selected polyline, and stages all the `update_entity` calls in one
//! transaction / one undo step. Per-vertex start/end **taper** is a different
//! model and stays `pedit.vertex_width_unsupported` (see `pedit_vertex`); a
//! uniform `WIDTH` never invents a taper.

use super::*;

impl Application {
    /// `PEDIT WIDTH <w>`: write the **uniform** polyline width on every selected
    /// polyline, in one transaction / one undo step.
    ///
    /// `w` must be finite and `>= 0` (`0` resets to the hairline draw); a bad
    /// value is `pedit.width_invalid`, and every selected entity must be a
    /// directly addressable model-space polyline (or the single selection is
    /// converted with `PEDITACCEPT`). The stored points, bulges, closed flag,
    /// id, layer, space, draw order and type key are preserved exactly: only
    /// `SemanticGeometry::Polyline::width` changes, and a run whose polylines
    /// already carry the target width opens no transaction
    /// (`pedit.width_unchanged`).
    ///
    /// Per-segment taper is a different model and stays a named refusal
    /// (`pedit.vertex_width_unsupported`, see [`super::pedit_vertex`]).
    #[allow(clippy::too_many_lines)]
    pub(super) fn pedit_width(
        &mut self,
        command: &Command,
        references: &[SelectionRef],
        width: f64,
        convert: bool,
    ) -> CadResult<CommandOutcome> {
        if !width.is_finite() || width < 0.0 {
            return Err(invalid(
                "width_invalid",
                "PEDIT WIDTH needs a finite non-negative width",
            ));
        }
        if references.is_empty() {
            return Err(invalid(
                "selection",
                "PEDIT WIDTH needs at least one selected polyline, got none",
            ));
        }
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let drawing = &mut candidate;

        let mut pairs: Vec<(DbEntity, DbEntity)> = Vec::new();
        for reference in references {
            if reference.document != document_id {
                return Err(invalid(
                    "not_addressable",
                    format!(
                        "the selected ref for entity {} belongs to another document",
                        reference.entity.0
                    ),
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(unsupported(
                    "not_addressable",
                    format!(
                        "PEDIT WIDTH needs directly addressable model-space polylines; entity \
                         {} is referenced through a block instance or a sub-element",
                        reference.entity.0
                    ),
                ));
            }
            if pairs
                .iter()
                .any(|(before, _)| before.id == reference.entity)
            {
                continue;
            }
            let before = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "not_addressable",
                    format!(
                        "entity {} does not exist in this drawing",
                        reference.entity.0
                    ),
                )
            })?;
            if before.space != SpaceId::Model {
                return Err(unsupported(
                    "not_addressable",
                    format!(
                        "PEDIT edits model-space polylines only; entity {} is in another space",
                        before.id.0
                    ),
                ));
            }
            let mut after = before.clone();
            match &before.geometry {
                SemanticGeometry::Polyline { .. } => {
                    let SemanticGeometry::Polyline { width: current, .. } = &mut after.geometry
                    else {
                        unreachable!("the variant was just matched");
                    };
                    *current = width;
                }
                other => {
                    if !(convert && references.len() == 1) {
                        return Err(unsupported(
                            "not_polyline",
                            format!(
                                "PEDIT WIDTH needs POLYLINE selections; {} is not supported (set \
                                 `PEDITACCEPT` / pass `convert = true` to turn it into one)",
                                class_name(other)
                            ),
                        ));
                    }
                    let mut converted = convert_to_polyline(other)?;
                    let SemanticGeometry::Polyline { width: current, .. } = &mut converted else {
                        return Err(CadError::Invariant(
                            "PEDIT WIDTH: the conversion did not produce a polyline".into(),
                        ));
                    };
                    *current = width;
                    after.geometry = converted;
                    after.object.type_key = class_name(&after.geometry).to_string();
                }
            }
            pairs.push((before, after));
        }

        let mut changed = Vec::with_capacity(pairs.len());
        for (before, after) in pairs {
            if before.geometry == after.geometry {
                continue;
            }
            changed.push((before, after));
        }
        if changed.is_empty() {
            return Ok(unchanged(
                "pedit.width_unchanged",
                format!(
                    "PEDIT WIDTH: the selected polyline(s) already have width {width}; nothing \
                     was written"
                ),
            ));
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("pedit", transaction)?;
            for (_, after) in &changed {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        // Read every committed entity back so the undo patch's `after` is the
        // exact stored value, never a re-derivation.
        let mut patches = Vec::with_capacity(changed.len());
        let mut objects = Vec::with_capacity(changed.len());
        let mut diagnostics = Vec::with_capacity(changed.len());
        for (before, after) in changed {
            let stored = drawing.entity(after.id).cloned().ok_or_else(|| {
                CadError::Invariant("PEDIT WIDTH: a polyline vanished after commit".into())
            })?;
            patches.push(drawing_patch(after.id, Some(before), Some(stored)));
            objects.push(ObjectId(after.id.0));
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(after.id.0)),
                code: "pedit.applied".into(),
                message: format!(
                    "PEDIT WIDTH: set the uniform polyline width to {width} on entity {} in one \
                     transaction",
                    after.id.0
                ),
            });
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "pedit".to_string(),
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
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
