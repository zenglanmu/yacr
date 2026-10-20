//! TEXTEDIT: replace the string of an existing single-line `Text` entity
//! (ADR 0020 amendment options A + B).
//!
//! Editing is deliberately **not** a second creation path: it re-reads the
//! addressed entity inside the command, changes only its `text` field and stages
//! one `update_entity` in one transaction / one undo step, exactly like
//! TRIM/EXTEND/STRETCH/CHAMFER. Every refusal is named and happens before the
//! transaction is opened, so a stale or wrong target writes nothing.
//!
//! The target must be an **entity**, not a sub-object: a ref carrying an
//! instance path (a block-instance member) or a sub-element is refused instead
//! of being reached into. Compound children and MLEADER content have no
//! addressable identity here (ADR 0020 postpones MLEADER editing to P7b).

use super::*;

impl Application {
    /// TEXTEDIT: replace the stored string of one directly addressable `Text`
    /// (ADR 0020 amendment A/B).
    ///
    /// The payload's `target` is the ref resolved at pick time (interactive
    /// `TEXTEDIT`) or the current single selection (the panel path). The entity
    /// is re-read here, so a target that was deleted, replaced or retyped since
    /// the pick is refused by name and opens no transaction.
    ///
    /// Only `text` changes: `style`/`height`/`rotation`/`position`/`font` and the
    /// two alignment fields are carried over byte-for-byte. The edit is one
    /// `update_entity` in one transaction / one undo step.
    pub(crate) fn edit_text(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::TextEdit { target, text } = &command.payload else {
            return Err(CadError::InvalidInput(
                "TEXTEDIT needs a TextEdit { target, text } payload".into(),
            ));
        };
        if target.document != command.document {
            return Err(CadError::InvalidInput(
                "TEXTEDIT target belongs to another document".into(),
            ));
        }
        // The same content refusals as the TEXT channel, re-validated here so a
        // programmatic caller cannot bypass the capture-time bound.
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: TEXTEDIT needs a non-empty, non-whitespace text".into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: TEXTEDIT allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }

        let document_id = command.document;
        let (drawing, before) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let before = resolve_text_edit_target(document.drawing.as_ref(), target)?;
            (document.drawing.clone(), before)
        };

        // Build the replacement from the stored entity, changing only the string.
        let mut after = before.clone();
        after.geometry = match &before.geometry {
            SemanticGeometry::Text {
                position,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
                ..
            } => SemanticGeometry::Text {
                text: text.clone(),
                position: *position,
                style: *style,
                height: *height,
                rotation: *rotation,
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
            },
            // `resolve_text_edit_target` guarantees this; keep an explicit
            // refusal instead of an unreachable panic.
            other => {
                return Err(CadError::Unsupported(format!(
                    "textedit.target_not_text: {} is not a TEXT entity",
                    geometry_kind(other)
                )))
            }
        };

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("edit text", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let after = candidate
            .entity(before.id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("edited text vanished after commit".into()))?;
        let object = ObjectId(before.id.0);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "edit text".into(),
                patches: vec![drawing_patch(before.id, Some(before), Some(after))],
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![object],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(object),
                code: "drawing.textedit".into(),
                message: "TEXTEDIT: replaced the text of one TEXT entity".into(),
            }],
            measurement: None,
        })
    }
}

/// Resolve the one `TEXTEDIT` target: an existing, **directly addressable**
/// single-line `Text` entity.
///
/// Named refusals (the caller opens no transaction on any of them):
///
/// * `textedit.target_missing` — the ref no longer resolves in this drawing;
/// * `textedit.target_not_text` — an instance path/sub-element (a block member
///   or sub-object, which has no entity identity of its own) or any other
///   geometry kind (line, hatch, dimension, compound, …).
///
/// It is shared by the interactive pick (which uses it to fill the prefill) and
/// the command (which re-validates at apply time), so the two can never drift.
pub(crate) fn resolve_text_edit_target(
    drawing: &DrawingDatabase,
    target: &SelectionRef,
) -> CadResult<DbEntity> {
    if !target.instance.0.is_empty() || target.sub_element.is_some() {
        return Err(CadError::Unsupported(
            "textedit.target_not_text: block-instance members and sub-elements are not \
             addressable TEXT entities"
                .into(),
        ));
    }
    let entity = drawing.entity(target.entity).cloned().ok_or_else(|| {
        CadError::InvalidInput(format!(
            "textedit.target_missing: TEXTEDIT target {} does not exist",
            target.entity.0
        ))
    })?;
    match &entity.geometry {
        SemanticGeometry::Text { .. } => Ok(entity),
        other => Err(CadError::Unsupported(format!(
            "textedit.target_not_text: {} is not a TEXT entity",
            geometry_kind(other)
        ))),
    }
}
