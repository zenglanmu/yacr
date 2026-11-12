//! `TCIRCLE` (estimated-bounds boundary), `TEXTMASK` (offset grouped wipeout)
//! and `TEXTUNMASK` (group-aware mask deletion).

use super::shared::*;
use super::*;
use crate::command_payload::{TextMaskKind, TextMaskOptions, UtilityPayload};

/// The group-name prefix `TEXTMASK` uses for its text/mask pairs.
const TEXTMASK_GROUP_PREFIX: &str = "*TEXTMASK-";

/// The `*TEXTMASK-*` groups that contain `reference`.
fn textmask_groups_for<'a>(
    session: &'a SessionState,
    reference: &SelectionRef,
) -> Vec<&'a crate::EntityGroup> {
    session
        .groups
        .groups()
        .filter(|group| {
            group.name.starts_with(TEXTMASK_GROUP_PREFIX) && group.members.contains(reference)
        })
        .collect()
}

/// Allocate `count` fresh `*TEXTMASK-<n>` names, avoiding retained groups.
fn allocate_textmask_names(
    session: &SessionState,
    old_groups: &std::collections::BTreeSet<String>,
    count: usize,
) -> Vec<String> {
    let removed: std::collections::BTreeSet<String> = old_groups
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut names: Vec<String> = Vec::with_capacity(count);
    let mut index = 1usize;
    while names.len() < count {
        let name = format!("{TEXTMASK_GROUP_PREFIX}{index}");
        index += 1;
        if removed.contains(&name.to_ascii_lowercase())
            || session.groups.contains(&name)
            || names
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        names.push(name);
    }
    names
}

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
                        width: 0.0,
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

    /// `TEXTMASK`: insert an `AcDbWipeout` mask behind each selected text.
    ///
    /// The documented offset factor (default **0.35**) grows the estimated text
    /// box by `factor × height` on every side. Only the `Wipeout` mask type has a
    /// modelled entity; `3dface` and `Solid` are named `Unsupported` refusals.
    /// The text and its new mask are grouped in the existing session group store
    /// (`GROUP`), so a group-aware move/copy/erase sees them together; the DWG
    /// itself still has no association record. Re-running TEXTMASK deletes the
    /// previous `*TEXTMASK-*` group mask for each selected text and creates the
    /// new one in the same transaction.
    pub(crate) fn text_mask(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let options = match utility_payload(command) {
            Some(UtilityPayload::TextMask(options)) => *options,
            Some(_) => {
                return Err(refusal(
                    "textmask.invalid_payload",
                    "TEXTMASK does not accept another utility command's payload",
                ));
            }
            None => match command.payload {
                CommandPayload::None => TextMaskOptions::default(),
                _ => {
                    return Err(refusal(
                        "textmask.invalid_payload",
                        "TEXTMASK takes no payload or a TextMask utility payload; the current \
                         selection is the input",
                    ));
                }
            },
        };
        if options.kind != TextMaskKind::Wipeout {
            let kind = match options.kind {
                TextMaskKind::Wipeout => "wipeout",
                TextMaskKind::Face3d => "3dface",
                TextMaskKind::Solid => "solid",
            };
            return Err(CadError::Unsupported(format!(
                "textmask.mask_type: the `{kind}` mask type has no entity model in this build \
                 (there is no 3DFACE or 2D SOLID entity); only `wipeout` (`AcDbWipeout`) is \
                 supported"
            )));
        }
        if !options.offset_factor.is_finite() || options.offset_factor < 0.0 {
            return Err(refusal(
                "textmask.invalid_offset",
                format!(
                    "the mask offset factor must be finite and non-negative, got {}",
                    options.offset_factor
                ),
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
        let mut sources: Vec<(SelectionRef, DbEntity)> = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            let entity = resolve_model_entity(&candidate, document_id, reference, "textmask")?;
            require_text("textmask", &entity)?;
            if seen.insert(entity.id) {
                sources.push((reference.clone(), entity));
            }
        }
        // Previous TEXTMASK groups/masks belonging to these texts only.
        let mut old_groups = std::collections::BTreeSet::new();
        let mut old_masks: Vec<DbEntity> = Vec::new();
        let mut old_seen = std::collections::BTreeSet::new();
        for (reference, _) in &sources {
            for group in textmask_groups_for(session, reference) {
                old_groups.insert(group.name.clone());
                for member in &group.members {
                    if member.entity == reference.entity
                        || member.document != document_id
                        || !member.instance.0.is_empty()
                        || member.sub_element.is_some()
                    {
                        continue;
                    }
                    if let Some(entity) = candidate.entity(member.entity).cloned() {
                        if matches!(entity.geometry, SemanticGeometry::Mask { .. })
                            && old_seen.insert(entity.id)
                        {
                            old_masks.push(entity);
                        }
                    }
                }
            }
        }
        let remaining_groups = session.groups.len().saturating_sub(old_groups.len());
        if remaining_groups + sources.len() > session.groups.cap() {
            return Err(CadError::Unsupported(format!(
                "textmask.group_cap: grouping {} text/mask pair(s) would exceed the session group \
                 capacity of {} (currently {remaining_groups} other group(s) retained)",
                sources.len(),
                session.groups.cap()
            )));
        }
        let names = allocate_textmask_names(session, &old_groups, sources.len());
        let mut entities = Vec::with_capacity(sources.len());
        for (_, source) in &sources {
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
            let pad = options.offset_factor
                * match &source.geometry {
                    SemanticGeometry::Text { height, .. } => *height,
                    _ => 0.0,
                };
            if !pad.is_finite() {
                return Err(refusal(
                    "textmask.degenerate",
                    format!("the offset for text {} is not finite", source.id.0),
                ));
            }
            let expanded = EstimatedTextBounds {
                min_x: bounds.min_x - pad,
                min_y: bounds.min_y - pad,
                max_x: bounds.max_x + pad,
                max_y: bounds.max_y + pad,
                z: bounds.z,
            };
            if ![
                expanded.min_x,
                expanded.min_y,
                expanded.max_x,
                expanded.max_y,
            ]
            .iter()
            .all(|value| value.is_finite())
            {
                return Err(refusal(
                    "textmask.degenerate",
                    format!("the offset bounds of text {} are not finite", source.id.0),
                ));
            }
            if expanded.width() <= 0.0 || expanded.height() <= 0.0 {
                return Err(refusal(
                    "textmask.degenerate",
                    format!("the offset bounds of text {} enclose no area", source.id.0),
                ));
            }
            let geometry = SemanticGeometry::Mask {
                boundary: expanded.corners(),
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
        let delete_states: Vec<_> = old_masks
            .iter()
            .map(|mask| candidate.entity_display_state(mask.id))
            .collect();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("text mask", transaction)?;
            for mask in &old_masks {
                staged.delete_entity(mask.id)?;
            }
            for entity in &entities {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };
        let mut patches: Vec<DrawingPatch> = old_masks
            .iter()
            .zip(delete_states)
            .map(|(mask, state)| drawing_patch_deleted(mask.id, mask.clone(), state))
            .collect();
        let mut objects = Vec::with_capacity(entities.len());
        for entity in &entities {
            let stored = candidate.entity(entity.id).cloned().ok_or_else(|| {
                CadError::Invariant("TEXTMASK: an inserted mask vanished after commit".into())
            })?;
            patches.push(drawing_patch(entity.id, None, Some(stored)));
            objects.push(ObjectId(entity.id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "text mask".into(),
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
        for name in &old_groups {
            session.groups.remove(name);
        }
        for (((reference, _), mask_id), name) in sources.iter().zip(&objects).zip(&names) {
            session.groups.create(
                name,
                vec![
                    reference.clone(),
                    SelectionRef {
                        document: document_id,
                        entity: EntityId(mask_id.0),
                        instance: InstancePath::default(),
                        sub_element: None,
                    },
                ],
            );
        }
        let count = objects.len();
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "drawing.textmask".into(),
                    message: format!(
                        "TEXTMASK: inserted {count} AcDbWipeout mask(s) behind the selected text \
                         with offset factor {} and created {count} session group(s)",
                        options.offset_factor
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "textmask.estimated_bounds".into(),
                    message: "TEXTMASK: the mask boundary is the declared estimate (height x \
                              char_count and height, rotation ignored) expanded by the offset \
                              factor; the text and mask are grouped in the session group store \
                              (`*TEXTMASK-*`), but the DWG stores no association record"
                        .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `TEXTUNMASK`: delete masks belonging to the selected text.
    ///
    /// A selected text's `*TEXTMASK-*` group names the exact mask to remove, even
    /// when the mask itself is not in the selection. Selected masks continue to
    /// use the declared estimated-bounds heuristic. No unselected, ungrouped mask
    /// is ever scanned. A `*TEXTMASK-*` group of a selected text whose mask row
    /// no longer exists is dropped as a stale association
    /// (`textunmask.stale_group`), so no orphan group survives the command; that
    /// cleanup is session state only and opens no drawing transaction.
    pub(crate) fn text_unmask(
        &mut self,
        session: &mut SessionState,
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
        let mut text_refs = Vec::new();
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
                    text_refs.push(reference.clone());
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
        // Group membership is the association authority: only groups created by
        // TEXTMASK are consulted, and only for the selected text refs. A
        // TEXTMASK group always pairs one text with one mask, so a group with no
        // live mask member is a stale association (its mask was erased
        // elsewhere); it is dropped here instead of surviving as an orphan.
        let mut group_names = std::collections::BTreeSet::new();
        let mut stale_groups = std::collections::BTreeSet::new();
        for reference in &text_refs {
            for group in textmask_groups_for(session, reference) {
                group_names.insert(group.name.clone());
                let has_live_mask = group.members.iter().any(|member| {
                    member.document == document_id
                        && candidate.entity(member.entity).is_some_and(|entity| {
                            matches!(entity.geometry, SemanticGeometry::Mask { .. })
                        })
                });
                if !has_live_mask {
                    stale_groups.insert(group.name.clone());
                }
                for member in &group.members {
                    if member.entity == reference.entity
                        || member.document != document_id
                        || !member.instance.0.is_empty()
                        || member.sub_element.is_some()
                    {
                        continue;
                    }
                    if let Some(entity) = candidate.entity(member.entity).cloned() {
                        if matches!(entity.geometry, SemanticGeometry::Mask { .. })
                            && seen.insert(entity.id)
                        {
                            masks.push(entity);
                        }
                    }
                }
            }
        }
        for name in &stale_groups {
            session.groups.remove(name);
        }
        if !stale_groups.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "textunmask.stale_group".into(),
                message: format!(
                    "TEXTUNMASK: dropped {} stale *TEXTMASK-* group(s) whose mask no longer \
                     exists ({})",
                    stale_groups.len(),
                    stale_groups.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            });
        }
        if masks.is_empty() {
            return Ok(unchanged_outcome(
                "textunmask.unchanged",
                "TEXTUNMASK: no grouped or selected mask was found; nothing was written".into(),
                diagnostics,
            ));
        }
        // With text selected, selected masks must lie inside the text bounds; a
        // group member is already the exact mask for its text and bypasses that
        // filter. With no text, every selected mask is deleted.
        let doomed: Vec<DbEntity> = if text_bounds.is_empty() {
            masks
        } else {
            masks
                .into_iter()
                .filter(|mask| {
                    group_names.iter().any(|name| {
                        session.groups.resolve(name).is_some_and(|group| {
                            group.members.iter().any(|member| {
                                member.entity == mask.id && member.document == document_id
                            })
                        })
                    }) || mask_lies_within_any(mask, &text_bounds)
                })
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
        let doomed_ids: std::collections::BTreeSet<EntityId> =
            doomed.iter().map(|mask| mask.id).collect();
        let mut groups_to_remove = group_names.clone();
        groups_to_remove.extend(stale_groups.iter().cloned());
        for group in session.groups.groups() {
            if group.name.starts_with(TEXTMASK_GROUP_PREFIX)
                && group
                    .members
                    .iter()
                    .any(|member| doomed_ids.contains(&member.entity))
            {
                groups_to_remove.insert(group.name.clone());
            }
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
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        for name in &groups_to_remove {
            session.groups.remove(name);
        }
        let removed = objects.len();
        let mut all = diagnostics;
        all.push(Diagnostic {
            object: None,
            code: "drawing.textunmask".into(),
            message: format!(
                "TEXTUNMASK: deleted {removed} mask entity(ies) belonging to the selected text \
                 (session groups) and/or selected masks covered by the estimated text bounds"
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
