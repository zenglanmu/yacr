//! `TCIRCLE` (estimated-bounds boundary), `TEXTMASK` (independent wipeout) and
//! `TEXTUNMASK` (selected-mask deletion heuristic).

use super::shared::*;
use super::*;

impl Application {
    /// `TCIRCLE CIRCLE|RECTANGLE`: draw an estimated-bounds boundary per text.
    pub(crate) fn text_circle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TextCircle(request) = &command.payload else {
            return Err(refusal(
                "tcircle.invalid_payload",
                "TCIRCLE needs a TextCircle { shape } payload \
                 (type `TCIRCLE CIRCLE|RECTANGLE`)",
            ));
        };
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "tcircle.needs_selection",
                "TCIRCLE needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut sources = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity = resolve_model_entity(&candidate, document_id, reference, "tcircle")?;
            require_text("tcircle", &entity)?;
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        // Build every boundary before opening the transaction.
        let mut base_order = next_model_draw_order(&candidate);
        let mut entities = Vec::with_capacity(sources.len());
        for source in &sources {
            let bounds = estimate_text_bounds(source, "tcircle")?;
            let geometry = match request.shape {
                TextBoundaryShape::Circle => {
                    let radius = 0.5 * bounds.width().hypot(bounds.height());
                    if !radius.is_finite() || radius <= 0.0 {
                        return Err(refusal(
                            "tcircle.degenerate",
                            format!(
                                "the estimated bounds of text {} have no positive radius",
                                source.id.0
                            ),
                        ));
                    }
                    let (cx, cy) = bounds.center();
                    SemanticGeometry::Circle {
                        center: Point3 {
                            x: cx,
                            y: cy,
                            z: bounds.z,
                        },
                        normal: Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 1.0,
                        },
                        radius,
                    }
                }
                TextBoundaryShape::Rectangle => {
                    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                        return Err(refusal(
                            "tcircle.degenerate",
                            format!(
                                "the estimated bounds of text {} enclose no area",
                                source.id.0
                            ),
                        ));
                    }
                    let points = bounds.corners();
                    SemanticGeometry::Polyline {
                        bulges: vec![0.0; points.len()],
                        points,
                        closed: true,
                    }
                }
            };
            let entity_id = candidate.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: class_name(&geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: entity_id,
                layer: source.layer,
                space: SpaceId::Model,
                geometry,
                draw_order: base_order,
            });
            base_order = base_order.saturating_add(1);
        }
        let (objects, change_set) = commit_new_rows(
            self,
            document_id,
            candidate,
            entities,
            "text circle",
            "text circle",
        )?;
        let count = objects.len();
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.tcircle".into(),
                message: format!(
                    "TCIRCLE {:?}: drew {} boundary entities on the source layers from the \
                     estimated text bounds; the text is retained",
                    request.shape, count
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `TEXTMASK`: insert one independent `AcDbWipeout` mask behind each text.
    pub(crate) fn text_mask(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "textmask.invalid_payload",
                "TEXTMASK takes no payload; the current selection is the input",
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "textmask.needs_selection",
                "TEXTMASK needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut sources = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity = resolve_model_entity(&candidate, document_id, reference, "textmask")?;
            require_text("textmask", &entity)?;
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        let mut entities = Vec::with_capacity(sources.len());
        for source in &sources {
            let bounds = estimate_text_bounds(source, "textmask")?;
            if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                return Err(refusal(
                    "textmask.degenerate",
                    format!(
                        "the estimated bounds of text {} enclose no area",
                        source.id.0
                    ),
                ));
            }
            let geometry = SemanticGeometry::Mask {
                boundary: bounds.corners(),
                inverted: false,
            };
            let entity_id = candidate.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: class_name(&geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: entity_id,
                layer: source.layer,
                space: SpaceId::Model,
                geometry,
                // Just below its own text. Ties with another row are resolved by
                // the renderer's existing order/id rule; unrelated rows are never
                // renumbered.
                draw_order: source.draw_order.saturating_sub(1),
            });
        }
        let (objects, change_set) = commit_new_rows(
            self,
            document_id,
            candidate,
            entities,
            "text mask",
            "text mask",
        )?;
        let count = objects.len();
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "drawing.textmask".into(),
                    message: format!(
                        "TEXTMASK: inserted {} AcDbWipeout masks behind the selected text from \
                         the estimated bounds",
                        count
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "textmask.estimated_bounds".into(),
                    message: "TEXTMASK: the mask boundary is the declared estimate \
                              (height × char_count and height, rotation ignored); no mask is \
                              associated with the text and no frame/display-not-plot option is \
                              implemented"
                        .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `TEXTUNMASK`: delete selected masks covered by the selected text.
    pub(crate) fn text_unmask(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "textunmask.invalid_payload",
                "TEXTUNMASK takes no payload; the current selection is the input",
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "textunmask.needs_selection",
                "TEXTUNMASK needs at least one selected Text/MText or mask entity",
            ));
        }
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut diagnostics = Vec::new();
        let mut text_bounds = Vec::new();
        let mut masks = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity =
                match resolve_model_entity(&candidate, document_id, reference, "textunmask") {
                    Ok(entity) => entity,
                    Err(error) => {
                        diagnostics.push(skipped(
                            "textunmask.skipped",
                            reference.entity,
                            &error_detail(error),
                        ));
                        continue;
                    }
                };
            match &entity.geometry {
                SemanticGeometry::Text { .. } => {
                    match estimate_text_bounds(&entity, "textunmask") {
                        Ok(bounds) => text_bounds.push(bounds),
                        Err(error) => diagnostics.push(skipped(
                            "textunmask.skipped",
                            entity.id,
                            &error_detail(error),
                        )),
                    }
                }
                SemanticGeometry::Mask { .. } => {
                    if seen.insert(entity.id) {
                        masks.push(entity);
                    }
                }
                other => diagnostics.push(skipped(
                    "textunmask.skipped",
                    entity.id,
                    &format!("{} is neither text nor a mask", kind_name(other)),
                )),
            }
        }
        if masks.is_empty() {
            return Ok(unchanged_outcome(
                "textunmask.unchanged",
                "TEXTUNMASK: no mask was selected; nothing was written".into(),
                diagnostics,
            ));
        }
        // With text selected, the text bounds filter the masks; with no text, every
        // selected mask is deleted. Only selected masks are ever removed.
        let doomed: Vec<DbEntity> = if text_bounds.is_empty() {
            masks
        } else {
            masks
                .into_iter()
                .filter(|mask| mask_lies_within_any(mask, &text_bounds))
                .collect()
        };
        if doomed.is_empty() {
            return Ok(unchanged_outcome(
                "textunmask.unchanged",
                "TEXTUNMASK: none of the selected masks lies inside the estimated bounds of the \
                 selected text; nothing was written"
                    .into(),
                diagnostics,
            ));
        }
        let display_states: Vec<_> = doomed
            .iter()
            .map(|mask| candidate.entity_display_state(mask.id))
            .collect();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("text unmask", transaction)?;
            for mask in &doomed {
                staged.delete_entity(mask.id)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(doomed.len());
        let mut objects = Vec::with_capacity(doomed.len());
        for (mask, state) in doomed.iter().zip(display_states) {
            patches.push(drawing_patch_deleted(mask.id, mask.clone(), state));
            objects.push(ObjectId(mask.id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "text unmask".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let removed = objects.len();
        let mut all = diagnostics;
        all.push(Diagnostic {
            object: None,
            code: "drawing.textunmask".into(),
            message: format!(
                "TEXTUNMASK: deleted {removed} selected mask entities covered by the estimated \
                 text bounds (the heuristic cannot prove a mask came from TEXTMASK)"
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: all,
            measurement: None,
            point_report: None,
        })
    }
}

/// Whether every boundary vertex of `mask` lies inside **any** of `boxes`.
pub(super) fn mask_lies_within_any(mask: &DbEntity, boxes: &[EstimatedTextBounds]) -> bool {
    let SemanticGeometry::Mask { boundary, .. } = &mask.geometry else {
        return false;
    };
    if boundary.is_empty() {
        return false;
    }
    boxes.iter().any(|box_| {
        // A small world tolerance so a boundary drawn from the same estimate
        // always matches its text (floating-point re-derivation is not exact).
        let tolerance = 1e-9 * (1.0 + box_.width().abs() + box_.height().abs());
        boundary.iter().all(|point| {
            point.x >= box_.min_x - tolerance
                && point.x <= box_.max_x + tolerance
                && point.y >= box_.min_y - tolerance
                && point.y <= box_.max_y + tolerance
        })
    })
}
