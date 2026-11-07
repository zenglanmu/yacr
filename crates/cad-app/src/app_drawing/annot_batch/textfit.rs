//! `TEXTFIT <target width>`: fit the selected Text/MText to an **estimated**
//! width by changing only the glyph height.
//!
//! `cad-domain` stores no width-factor field, so this command uses the batch's
//! one declared estimate: `width = height × text.chars().count()`. Setting
//! `height = target / char_count` makes that estimate equal the target. True
//! glyph metrics and AutoCAD's width stretch (group 41) are **NOT IMPLEMENTED**.

use super::*;

impl Application {
    /// `TEXTFIT <target width>`: one transaction / one undo step over the
    /// selection.
    pub(crate) fn text_fit(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let target = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::TextFit(request) => request.target_width,
                _ => {
                    return Err(invalid(
                        "textfit.payload",
                        "TEXTFIT takes one target width (`TEXTFIT <target width>`)",
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    "textfit.needs_width",
                    "TEXTFIT needs one target width (`TEXTFIT <target width>`)",
                ))
            }
            _ => {
                return Err(invalid(
                    "textfit.payload",
                    "TEXTFIT takes one target width (`TEXTFIT <target width>`)",
                ))
            }
        };
        if !target.is_finite() || target <= 0.0 {
            return Err(invalid(
                "textfit.bad_width",
                format!("TEXTFIT needs a finite target width > 0, got {target}"),
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(invalid(
                "textfit.selection",
                "TEXTFIT needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut planned = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            if reference.document != document_id {
                return Err(invalid(
                    "textfit.not_addressable",
                    format!(
                        "the selected ref for entity {} belongs to another document",
                        reference.entity.0
                    ),
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "textfit.not_addressable",
                    format!(
                        "entity {} is a block-instance member or sub-element; TEXTFIT edits \
                         directly addressable model-space text only",
                        reference.entity.0
                    ),
                ));
            }
            if !seen.insert(reference.entity) {
                continue;
            }
            let before = candidate.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "textfit.target_missing",
                    format!("entity {} no longer exists", reference.entity.0),
                )
            })?;
            if before.space != SpaceId::Model {
                return Err(invalid(
                    "textfit.not_addressable",
                    format!(
                        "entity {} is not in model space; TEXTFIT edits model-space text only",
                        before.id.0
                    ),
                ));
            }
            let SemanticGeometry::Text {
                text,
                height,
                position,
                ..
            } = &before.geometry
            else {
                return Err(CadError::Unsupported(format!(
                    "textfit.not_text: TEXTFIT supports Text and MText only; entity {} is {}",
                    before.id.0,
                    class_name(&before.geometry)
                )));
            };
            if text.is_empty() {
                return Err(invalid(
                    "textfit.empty_text",
                    format!(
                        "entity {} stores an empty string, so it has no estimated width",
                        before.id.0
                    ),
                ));
            }
            if !position.x.is_finite() || !position.y.is_finite() || !position.z.is_finite() {
                return Err(invalid(
                    "textfit.degenerate",
                    format!("entity {} has a non-finite position", before.id.0),
                ));
            }
            let characters = text.chars().count() as f64;
            let fitted = target / characters;
            if !fitted.is_finite() || fitted <= 0.0 {
                return Err(invalid(
                    "textfit.bad_width",
                    format!(
                        "the fitted height for entity {} is not finite/positive",
                        before.id.0
                    ),
                ));
            }
            if (*height - fitted).abs() <= f64::EPSILON {
                continue;
            }
            let mut after = before.clone();
            if let SemanticGeometry::Text { height, .. } = &mut after.geometry {
                *height = fitted;
            }
            planned.push((before, after));
        }
        if planned.is_empty() {
            return Ok(unchanged(
                "textfit.unchanged",
                format!(
                    "TEXTFIT: every selected text already has the estimated width {target}; \
                     nothing was written"
                ),
            ));
        }
        let count = planned.len();
        let (objects, change_set) =
            commit_text_fit(self, document_id, candidate, &planned, "textfit")?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "textfit.applied".into(),
                message: format!(
                    "TEXTFIT: set the glyph height of {count} text entity/entities so the \
                     estimated width is {target} (height × char count; true width stretch is \
                     NOT IMPLEMENTED, one transaction)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Commit every planned text update in one transaction / one undo step.
fn commit_text_fit(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    planned: &[(DbEntity, DbEntity)],
    label: &str,
) -> CadResult<(Vec<ObjectId>, Arc<ChangeSet>)> {
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
        for (_, after) in planned {
            staged.update_entity(after.clone())?;
        }
        staged.commit()?
    };
    let mut patches = Vec::with_capacity(planned.len());
    let mut objects = Vec::with_capacity(planned.len());
    for (before, _) in planned {
        let stored = candidate.entity(before.id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("{label}: an updated text vanished after commit"))
        })?;
        patches.push(drawing_patch(before.id, Some(before.clone()), Some(stored)));
        objects.push(ObjectId(before.id.0));
    }
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.to_string(),
            patches,
            block_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok((objects, Arc::new(change_set)))
}
