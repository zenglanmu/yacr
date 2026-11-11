//! `TEXTTOFRONT` (AutoCAD 2026): bring the **selected** text, leaders and
//! dimensions in front of every other model-space object.
//!
//! The current selection is the input (a declared divergence from AutoCAD's
//! interactive `Select objects` prompt). Eligible entities are directly
//! addressable model-space `SemanticGeometry::Text`, `MultiLeader` and
//! `Dimension` rows; every other selected object and every non-addressable ref is
//! reported as a per-entity `texttofront.skipped` diagnostic and skipped without
//! failing the command.
//!
//! `base` is the current maximum `draw_order` over all model-space **non-eligible**
//! entities (`0` when there is none). Eligible entities are sorted by
//! `(draw_order, entity id)` and receive the contiguous orders `base + 1 ..=` in
//! that order, preserving their relative order. When every eligible is already
//! above `base` the command is `texttofront.unchanged`: no transaction, no
//! revision bump and no undo entry. Otherwise only changed rows are staged with
//! one `update_entity` each in **one transaction / one undo step**, so an undo
//! restores every exact prior order.
//!
//! Declared limits: model space only (paper/block draw order is not reordered),
//! selected non-text objects are not moved, and AutoCAD's all-text-in-drawing form
//! and Draw Order dialog are **NOT IMPLEMENTED**. Full contract:
//! [`crate::command_id::texttofront`].

use std::collections::BTreeSet;

use super::*;

impl Application {
    /// TEXTTOFRONT: move the selected text/leader/dimension draw order above all
    /// non-eligible model-space rows.
    pub(crate) fn text_to_front(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "texttofront.payload",
                "TEXTTOFRONT takes no payload; the current selection is the input",
            ));
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(refusal(
                "texttofront.selection",
                "TEXTTOFRONT needs at least one selected object",
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal("texttofront.not_addressable", "document not open"))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // Every model-space row is a possible order authority; only the
        // non-eligible rows define the "in front of everything else" baseline.
        let base = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .filter(|entity| !is_eligible_text_object(&entity.geometry))
            .map(|entity| entity.draw_order)
            .max()
            .unwrap_or(0);

        let mut skipped = Vec::new();
        let mut seen = BTreeSet::new();
        let mut eligible = Vec::new();
        for reference in refs {
            if reference.document != document_id {
                skipped.push(skipped_diagnostic(
                    reference.entity,
                    "it belongs to another document",
                ));
                continue;
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                skipped.push(skipped_diagnostic(
                    reference.entity,
                    "it is referenced through a block instance or a sub-element",
                ));
                continue;
            }
            let Some(source) = drawing.entity(reference.entity).cloned() else {
                skipped.push(skipped_diagnostic(
                    reference.entity,
                    "it does not exist in this drawing",
                ));
                continue;
            };
            if source.space != SpaceId::Model {
                skipped.push(skipped_diagnostic(
                    reference.entity,
                    "it is not in model space (paper-space/block draw order is not reordered)",
                ));
                continue;
            }
            if !is_eligible_text_object(&source.geometry) {
                skipped.push(skipped_diagnostic(
                    reference.entity,
                    "it is not text, a multileader or a dimension",
                ));
                continue;
            }
            if seen.insert(source.id) {
                eligible.push(source);
            }
        }

        if eligible.is_empty() {
            return Ok(unchanged_outcome(
                skipped,
                "the selection holds no text, multileader or dimension in model space",
            ));
        }
        // `base` is a strict lower bound: every eligible order must be above it.
        if eligible.iter().all(|entity| entity.draw_order > base) {
            return Ok(unchanged_outcome(
                skipped,
                "every eligible object is already in front of every other object",
            ));
        }

        eligible.sort_by_key(|entity| (entity.draw_order, entity.id.0));
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(eligible.len());
        for (index, before) in eligible.into_iter().enumerate() {
            let target = base
                .checked_add(i64::try_from(index).map_err(|_| order_exhausted())? + 1)
                .ok_or_else(order_exhausted)?;
            if before.draw_order == target {
                continue;
            }
            let mut after = before.clone();
            after.draw_order = target;
            planned.push((before, after));
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                skipped,
                "every eligible object already has its target order",
            ));
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("text to front", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let ids: Vec<ObjectId> = planned
            .iter()
            .map(|(before, _)| ObjectId(before.id.0))
            .collect();
        let moved = ids.len();
        let patches: Vec<DrawingPatch> = planned
            .into_iter()
            .map(|(before, after)| drawing_patch(before.id, Some(before), Some(after)))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "text to front".into(),
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

        let mut diagnostics = skipped;
        diagnostics.push(Diagnostic {
            object: None,
            code: "texttofront.applied".into(),
            message: format!(
                "TEXTTOFRONT: moved {moved} text/leader/dimension object(s) above every \
                 non-eligible model-space object"
            ),
        });
        Ok(CommandOutcome {
            objects: ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Whether a geometry is eligible for TEXTTOFRONT (`TEXT`/`MTEXT`, `MLEADER`,
/// `DIMENSION`).
fn is_eligible_text_object(geometry: &SemanticGeometry) -> bool {
    matches!(
        geometry,
        SemanticGeometry::Text { .. }
            | SemanticGeometry::MultiLeader(_)
            | SemanticGeometry::Dimension(_)
    )
}

fn skipped_diagnostic(entity: EntityId, detail: &str) -> Diagnostic {
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: "texttofront.skipped".into(),
        message: format!("TEXTTOFRONT skipped entity {}: {detail}", entity.0),
    }
}

fn unchanged_outcome(skipped: Vec<Diagnostic>, detail: &str) -> CommandOutcome {
    let mut diagnostics = skipped;
    diagnostics.push(Diagnostic {
        object: None,
        code: "texttofront.unchanged".into(),
        message: format!("TEXTTOFRONT: {detail}; nothing was written"),
    });
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

fn order_exhausted() -> CadError {
    refusal(
        "texttofront.order_exhausted",
        "the target draw orders would overflow i64",
    )
}

/// A command refusal carrying its stable reason code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
