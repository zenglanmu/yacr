//! `TEXTTOFRONT` (AutoCAD 2026): bring the **selected** text, leaders and
//! dimensions in front of every other object of their own draw-order scope.
//!
//! The current selection is the input (a declared divergence from AutoCAD's
//! interactive `Select objects` prompt). Eligible entities are directly
//! addressable `SemanticGeometry::Text`, `MultiLeader` and `Dimension` rows; every
//! other selected object and every non-addressable ref is reported as a
//! per-entity `texttofront.skipped` diagnostic and skipped without failing the
//! command.
//!
//! # Scopes
//!
//! The reorder is planned **per [`SpaceId`]** — model space, each paper layout and
//! each block definition — exactly like `HATCHTOBACK` and `DRAWORDER`: a layout's
//! draw order is only meaningful against that layout's rows, and a block
//! definition's members only against each other. For every scope, `base` is the
//! maximum `draw_order` over that scope's **non-eligible** entities (`0` when
//! there is none); the eligible selected entities of that scope are sorted by
//! `(draw_order, entity id)` and receive the contiguous orders `base + 1 ..=`,
//! preserving their relative order.
//!
//! When every eligible object is already above its own scope's `base` the command
//! is `texttofront.unchanged`: no transaction, no revision bump and no undo entry.
//! Otherwise only changed rows are staged with one `update_entity` each in **one
//! transaction / one undo step**, so an undo restores every exact prior order.
//!
//! Declared limits: selected non-text objects are not moved, and AutoCAD's
//! all-text-in-drawing form and Draw Order dialog are **NOT IMPLEMENTED** (the
//! payload carries no "all text" option, so there is no channel to request it).
//! Full contract: [`crate::command_id::texttofront`].

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

        // Per-scope baselines: only the non-eligible rows of a scope define "in
        // front of everything else" *within that scope*.
        let mut scopes: Vec<SpaceId> = Vec::new();
        for entity in drawing.entities() {
            if !scopes.contains(&entity.space) {
                scopes.push(entity.space.clone());
            }
        }
        let mut base_by_scope: Vec<(SpaceId, i64)> = Vec::new();
        for space in scopes {
            let base = drawing
                .entities()
                .filter(|entity| entity.space == space)
                .filter(|entity| !is_eligible_text_object(&entity.geometry))
                .map(|entity| entity.draw_order)
                .max()
                .unwrap_or(0);
            base_by_scope.push((space, base));
        }

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
                "the selection holds no text, multileader or dimension",
            ));
        }

        // Plan every scope before writing: an overflow in one scope refuses the
        // whole run, so a partial reorder can never be published.
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(eligible.len());
        let mut changed_scopes = 0usize;
        let mut scope_spaces: Vec<SpaceId> = Vec::new();
        for entity in &eligible {
            if !scope_spaces.contains(&entity.space) {
                scope_spaces.push(entity.space.clone());
            }
        }
        for space in scope_spaces {
            let base = base_by_scope
                .iter()
                .find(|(scope, _)| *scope == space)
                .map(|(_, base)| *base)
                .unwrap_or(0);
            let mut scope_eligible: Vec<DbEntity> = eligible
                .iter()
                .filter(|entity| entity.space == space)
                .cloned()
                .collect();
            scope_eligible.sort_by_key(|entity| (entity.draw_order, entity.id.0));
            // Already above the scope's baseline: the exact orders are kept (the
            // command never compacts a sequence that is already in front).
            if scope_eligible.iter().all(|entity| entity.draw_order > base) {
                continue;
            }
            let mut scope_planned = Vec::new();
            for (index, before) in scope_eligible.into_iter().enumerate() {
                let target = base
                    .checked_add(i64::try_from(index).map_err(|_| order_exhausted())? + 1)
                    .ok_or_else(order_exhausted)?;
                if before.draw_order == target {
                    continue;
                }
                let mut after = before.clone();
                after.draw_order = target;
                scope_planned.push((before, after));
            }
            if !scope_planned.is_empty() {
                changed_scopes += 1;
            }
            planned.extend(scope_planned);
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                skipped,
                "every eligible object is already in front of every other object of its scope",
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
                 non-eligible object of their own scope in {changed_scopes} scope(s), in one \
                 transaction (one undo step)"
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
