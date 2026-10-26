use super::*;

impl Application {
    /// Transform the selected entities in place (ROTATE/SCALE).
    ///
    /// `command_label` is the shell-style command name used in refusal messages
    /// (matching COPY's "COPY cannot transform …" style); `operation` is the
    /// PascalCase id used in reference-resolution errors.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn transform_selection(
        &mut self,
        command: &Command,
        refs: &[SelectionRef],
        transform: &Transform3,
        label: &str,
        code: &str,
        operation: &str,
        command_label: &str,
        action: &str,
    ) -> CadResult<CommandOutcome> {
        if refs.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{operation} needs at least one selected reference"
            )));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let (before_states, diagnostics) =
            Self::resolve_selection(drawing, document_id, refs, code, operation, action, false)?;
        // Refuse the whole command if any selected geometry cannot be transformed
        // exactly, rather than dropping it from the transform. An unsupported kind
        // is relabelled with the operation context; a genuine `InvalidInput`
        // (e.g. a collapsing transform) keeps its original kind.
        for before in &before_states {
            transform_geometry(&before.geometry, transform).map_err(|error| match error {
                CadError::Unsupported(reason) => CadError::Unsupported(format!(
                    "{command_label} cannot transform {}: {reason}",
                    geometry_kind(&before.geometry)
                )),
                other => other,
            })?;
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for before in &before_states {
                staged.transform_entity(before.id, transform)?;
            }
            staged.commit()?
        };

        // Read each committed entity back so the undo patch's `after` is the exact
        // stored value for every geometry kind.
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .map(|before| {
                let after = drawing.entity(before.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "transformed entity {} vanished after commit",
                        before.id.0
                    ))
                })?;
                Ok(drawing_patch(before.id, Some(before), Some(after)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Mirror the selected entities into fresh copies, keeping the originals.
    pub(super) fn mirror_selection(
        &mut self,
        command: &Command,
        refs: &[SelectionRef],
        transform: &Transform3,
    ) -> CadResult<CommandOutcome> {
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "MirrorEntities needs at least one selected reference".into(),
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

        let (sources, mut diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            refs,
            "mirror",
            "MirrorEntities",
            "镜像",
            false,
        )?;
        // Refuse the whole command if any selected geometry cannot be mirrored,
        // never drop it. Arc/Ellipse are mirrored by this crate's explicit
        // reflection arms (`mirror_geometry`); every other kind keeps the generic
        // transform surface's behaviour and refusals (Text/Shape/Opaque).
        for source in &sources {
            mirror_geometry(&source.geometry, transform).map_err(|error| match error {
                CadError::Unsupported(reason) => CadError::Unsupported(format!(
                    "MIRROR cannot mirror {}: {reason}",
                    geometry_kind(&source.geometry)
                )),
                other => other,
            })?;
            // A per-entity display override cannot be carried by the current write
            // path; report the honest fallback instead of a silent change.
            if drawing.entity_render_attributes(source.id)
                != cad_db::EntityRenderAttributes::default()
            {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(source.id.0)),
                    code: "drawing.mirror.style".into(),
                    message: format!(
                        "实体 {} 的按对象颜色/线型/线宽覆盖未复制，副本继承图层样式",
                        source.id.0
                    ),
                });
            }
        }

        let base_order = drawing
            .model_space()
            .iter()
            .map(|e| e.draw_order)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0);
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(sources.len());
        for (index, source) in sources.iter().enumerate() {
            let new_id = drawing.allocate_entity_id();
            let geometry = mirror_geometry(&source.geometry, transform)?;
            let copy = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key: source.object.type_key.clone(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: source.layer,
                space: source.space.clone(),
                geometry,
                draw_order: base_order + index as i64,
            };
            copies.push((new_id, copy));
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("mirror entities", transaction)?;
            for (_, copy) in &copies {
                staged.insert_entity(copy.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = copies
            .iter()
            .map(|(id, copy)| drawing_patch(*id, None, Some(copy.clone())))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "mirror entities".into(),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Resolve selection refs to the current entities, collapsing duplicates and
    /// reporting sub-element/instance refs exactly like MOVE/COPY.
    ///
    /// `code` is the diagnostic code stem (e.g. `rotate`), `operation` the English
    /// name used in errors, and `action` the verb used in the diagnostics.
    /// `read_only_capture` selects the capture wording for an instance ref, which
    /// affects no other instance (a transform, by contrast, edits the shared block
    /// definition).
    pub(crate) fn resolve_selection(
        drawing: &DrawingDatabase,
        document_id: DocumentId,
        refs: &[SelectionRef],
        code: &str,
        operation: &str,
        action: &str,
        read_only_capture: bool,
    ) -> CadResult<(Vec<DbEntity>, Vec<Diagnostic>)> {
        let mut seen: Vec<EntityId> = Vec::new();
        let mut before_states: Vec<DbEntity> = Vec::new();
        let mut diagnostics = Vec::new();
        for reference in refs {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(format!(
                    "{operation} reference belongs to another document"
                )));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let before = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!("entity {} does not exist", reference.entity.0))
            })?;
            if reference.sub_element.is_some() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: format!("drawing.{code}.sub_element"),
                    message: format!(
                        "子图元 {} 按整实体{action}（子图元级编辑未支持）",
                        reference.entity.0
                    ),
                });
            }
            if !reference.instance.0.is_empty() {
                let message = if read_only_capture {
                    format!(
                        "实例路径引用的实体 {} 已按整实体捕获（只读，不影响实例）",
                        reference.entity.0
                    )
                } else {
                    format!(
                        "实例路径引用的实体 {} {action}其块定义，影响所有实例",
                        reference.entity.0
                    )
                };
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: format!("drawing.{code}.instance"),
                    message,
                });
            }
            before_states.push(before);
        }
        if before_states.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{operation} needs at least one selected reference"
            )));
        }
        Ok((before_states, diagnostics))
    }
}
