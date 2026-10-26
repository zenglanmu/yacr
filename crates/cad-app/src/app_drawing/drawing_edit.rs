use super::*;

impl Application {
    /// Translate every selected entity by `delta` in one transaction.
    ///
    /// Duplicate entity ids are collapsed (two sub-elements of one entity move
    /// the entity once) and sub-element/instance refs carry an explicit
    /// diagnostic that the whole entity was moved. The INSERT case composes the
    /// translation into the insert's own transform so every referenced instance
    /// moves, matching [`DrawingTransaction::transform_entity`].
    pub(crate) fn move_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Move { refs, delta } = &command.payload else {
            return Err(CadError::InvalidInput(
                "MoveEntities needs a Move { refs, delta } payload".into(),
            ));
        };
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "MoveEntities needs at least one selected reference".into(),
            ));
        }
        if !is_finite_point(*delta) {
            return Err(CadError::InvalidInput(
                "MoveEntities delta must be finite".into(),
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

        // Collapse to unique entities, keeping first-seen order, and capture the
        // before state. Reading is done before the transaction opens so a missing
        // entity is refused before any write.
        let mut seen: Vec<EntityId> = Vec::new();
        let mut before_states: Vec<DbEntity> = Vec::new();
        let mut diagnostics = Vec::new();
        for reference in refs {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(
                    "MoveEntities reference belongs to another document".into(),
                ));
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
                    code: "drawing.move.sub_element".into(),
                    message: format!(
                        "子图元 {} 按整实体移动（子图元级编辑未支持）",
                        reference.entity.0
                    ),
                });
            }
            if !reference.instance.0.is_empty() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.move.instance".into(),
                    message: format!(
                        "实例路径引用的实体 {} 移动其块定义，影响所有实例",
                        reference.entity.0
                    ),
                });
            }
            before_states.push(before);
        }

        let translation = Transform3::translation(*delta);
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("move entities", transaction)?;
            for before in &before_states {
                // Stage through the documented transform entry point so the
                // database owns geometry replacement (including INSERT
                // transform composition).
                staged.transform_entity(before.id, &translation)?;
            }
            staged.commit()?
        };

        // Read each committed entity back so the undo patch's `after` is the
        // exact stored value for every geometry kind, not a re-derivation.
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .map(|before| {
                let after = drawing.entity(before.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "moved entity {} vanished after commit",
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
                label: "move entities".into(),
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

    /// Delete every entity in the current selection in one transaction.
    ///
    /// The selection is the session's; an empty selection is an explicit refusal
    /// that changes nothing. Duplicate refs to one entity are collapsed (one
    /// delete), every selected entity must exist before any write, and the whole
    /// erase is one undo step.
    pub(crate) fn erase_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "EraseEntities takes no payload".into(),
            ));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "ERASE needs a non-empty selection".into(),
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

        let mut seen: Vec<EntityId> = Vec::new();
        let mut before_states: Vec<DbEntity> = Vec::new();
        let mut display_states: Vec<_> = Vec::new();
        let mut diagnostics = Vec::new();
        for reference in &refs {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(
                    "EraseEntities reference belongs to another document".into(),
                ));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let before = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!("entity {} does not exist", reference.entity.0))
            })?;
            // Mirror MOVE's contract: a sub-element/instance ref is accepted as a
            // whole-entity erase, reported rather than silently over-applied.
            if reference.sub_element.is_some() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.erase.sub_element".into(),
                    message: format!(
                        "子图元 {} 按整实体删除（子图元级编辑未支持）",
                        reference.entity.0
                    ),
                });
            }
            if !reference.instance.0.is_empty() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.erase.instance".into(),
                    message: format!(
                        "实例路径引用的实体 {} 删除其块定义，影响所有实例",
                        reference.entity.0
                    ),
                });
            }
            // Capture the display state the delete will prune, so undo restores it.
            display_states.push(drawing.entity_display_state(reference.entity));
            before_states.push(before);
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("erase entities", transaction)?;
            for before in &before_states {
                staged.delete_entity(before.id)?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .zip(display_states)
            .map(|(before, state)| drawing_patch_deleted(before.id, before, state))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "erase entities".into(),
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

    /// Reconcile the session selection with the authoritative document after an
    /// edit: drop exactly the refs whose entity no longer exists.
    ///
    /// An edit can remove entities (ERASE, a TRIM that deletes the whole target,
    /// CUT, and undo/redo of a deleting edit). A session selection that still
    /// holds a removed entity is a dangling reference: the status projection
    /// reports it as an `unresolved` row and the render overlay surfaces a
    /// `highlight.unresolved` "… is not present in the document" error that the
    /// user cannot act on. This is the **single** reconciliation point, run after
    /// a successful edit command; it is session state only (no database write,
    /// no history entry, no ChangeSet), so an edit's one transaction and one undo
    /// step are untouched.
    ///
    /// The rule is **drop only the removed refs, keep the rest**: a ref survives
    /// iff it belongs to the open document (`reference.document ==
    /// session.document`) **and** `DrawingDatabase::entity(ref.entity)` still
    /// resolves (which covers model space, paper space and block children).
    /// Sub-element/instance refs whose owning entity survives are kept. When
    /// there is no open document there is nothing to reconcile against, so the
    /// selection is left untouched (never cleared behind the user's back).
    ///
    /// Keying on `(document, entity)` rather than `entity` alone closes a latent
    /// wrong-retention: an entity-only prune would keep a **foreign-document**
    /// ref whenever its id merely coincided with an entity of the open drawing,
    /// attributing it here as though it belonged. The session selection is
    /// single-document (rebuilt per document, and `SelectionSet` dedups without
    /// the document), so no legitimate flow places a foreign-document ref in it;
    /// this only ever drops a ref that was already wrong. Behaviour change: such
    /// a foreign ref is now dropped instead of silently retained.
    ///
    /// Replacement-ids decision: none of the edit commands replace an existing
    /// entity with a new id — MOVE/ROTATE/SCALE and a keeping TRIM mutate the
    /// same entity in place, while COPY/MIRROR/OFFSET/PASTE insert *new* entities
    /// (`objects` in the outcome) and leave the sources intact. So the selection
    /// is never redirected onto a copy; it keeps the original refs, which still
    /// resolve. Following this project's honest convention, a ref that stops
    /// resolving is dropped rather than silently retargeted onto a fresh id.
    pub(crate) fn reconcile_selection(&self, session: &mut SessionState) {
        let Some(document) = self.workspace.documents.get(&session.document) else {
            return;
        };
        let drawing = &document.drawing;
        session.selection.retain(|reference| {
            reference.document == session.document && drawing.entity(reference.entity).is_some()
        });
    }

    /// Clone every selected entity with a fresh id, translating its geometry.
    ///
    /// Mirrors [`Application::move_entities`] but inserts new entities instead of
    /// transforming the originals: layer and space are preserved per source
    /// entity, and any geometry that cannot be transformed refuses the whole
    /// command before a write (never dropping a selected entity silently). One
    /// transaction and one undo step cover every copy.
    pub(crate) fn copy_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Copy { refs, delta } = &command.payload else {
            return Err(CadError::InvalidInput(
                "CopyEntities needs a Copy { refs, delta } payload".into(),
            ));
        };
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "CopyEntities needs at least one selected reference".into(),
            ));
        }
        if !is_finite_point(*delta) {
            return Err(CadError::InvalidInput(
                "CopyEntities delta must be finite".into(),
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

        let translation = Transform3::translation(*delta);
        let mut seen: Vec<EntityId> = Vec::new();
        let mut sources: Vec<DbEntity> = Vec::new();
        let mut diagnostics = Vec::new();
        for reference in refs {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(
                    "CopyEntities reference belongs to another document".into(),
                ));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let source = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!("entity {} does not exist", reference.entity.0))
            })?;
            // Refuse the whole command if any selected geometry cannot be
            // transformed, rather than dropping that entity from the copy.
            transform_geometry(&source.geometry, &translation).map_err(|error| {
                CadError::Unsupported(format!(
                    "COPY cannot transform {}: {error}",
                    geometry_kind(&source.geometry)
                ))
            })?;
            // Mirror MOVE's contract: a sub-element/instance ref is accepted as a
            // whole-entity copy, reported rather than silently over-applied.
            if reference.sub_element.is_some() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.copy.sub_element".into(),
                    message: format!(
                        "子图元 {} 按整实体复制（子图元级编辑未支持）",
                        reference.entity.0
                    ),
                });
            }
            if !reference.instance.0.is_empty() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.copy.instance".into(),
                    message: format!(
                        "实例路径引用的实体 {} 复制其块定义，影响所有实例",
                        reference.entity.0
                    ),
                });
            }
            // A per-entity display override cannot be carried by the current
            // write path; report the honest fallback instead of a silent change.
            let attributes = drawing.entity_render_attributes(reference.entity);
            if attributes != cad_db::EntityRenderAttributes::default() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "drawing.copy.style".into(),
                    message: format!(
                        "实体 {} 的按对象颜色/线型/线宽覆盖未复制，副本继承图层样式",
                        reference.entity.0
                    ),
                });
            }
            sources.push(source);
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
            let geometry = transform_geometry(&source.geometry, &translation)?;
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
            let mut staged = drawing.begin_drawing_transaction("copy entities", transaction)?;
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
                label: "copy entities".into(),
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

    /// Rotate the current selection about `base` by `angle` radians (XY, +Z).
    ///
    /// The transform is a proper rotation (positive determinant), so circles,
    /// arcs and ellipses keep their exact representation. A geometry that cannot
    /// be transformed is refused for the whole command; one transaction and one
    /// undo step cover the selection.
    pub(crate) fn rotate_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Rotate { refs, base, angle } = &command.payload else {
            return Err(CadError::InvalidInput(
                "RotateEntities needs a Rotate { refs, base, angle } payload".into(),
            ));
        };
        if !is_finite_point(*base) || !angle.is_finite() {
            return Err(CadError::InvalidInput(
                "RotateEntities base and angle must be finite".into(),
            ));
        }
        let transform = crate::draw_tool::rotation_transform(*base, *angle);
        self.transform_selection(
            command,
            refs,
            &transform,
            "rotate entities",
            "rotate",
            "RotateEntities",
            "ROTATE",
            "旋转",
        )
    }

    /// Scale the current selection uniformly about `base` by `factor`.
    ///
    /// A uniform scale is a positive-determinant similarity, so circles, arcs and
    /// ellipses stay exact. A non-positive or non-finite factor, or any geometry
    /// that cannot be transformed, refuses the whole command.
    ///
    /// Note: `transform_geometry` rejects a transform whose determinant is below
    /// `1e-12`; since a uniform scale has `det = factor³`, a legitimate but tiny
    /// factor (roughly below `1e-4`) is refused as a collapsing transform rather
    /// than applied. That is a deliberate guard against degenerate scaling, not a
    /// silent no-op.
    pub(crate) fn scale_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Scale { refs, base, factor } = &command.payload else {
            return Err(CadError::InvalidInput(
                "ScaleEntities needs a Scale { refs, base, factor } payload".into(),
            ));
        };
        if !is_finite_point(*base) || !factor.is_finite() || *factor <= 0.0 {
            return Err(CadError::InvalidInput(
                "ScaleEntities factor must be positive and finite".into(),
            ));
        }
        let transform = crate::draw_tool::scale_transform(*base, *factor);
        self.transform_selection(
            command,
            refs,
            &transform,
            "scale entities",
            "scale",
            "ScaleEntities",
            "SCALE",
            "缩放",
        )
    }

    /// Mirror the current selection across the axis `axis_start`→`axis_end`.
    ///
    /// Mirrors MOVE/COPY's contract but **keeps the originals** (AutoCAD default):
    /// mirrored copies are inserted with fresh ids in one transaction and one undo
    /// step, leaving the sources untouched. A reflection is orientation-reversing,
    /// so an Arc/Ellipse (whose parameterisation would reverse) is an explicit
    /// refusal for the whole command, never a silent drop.
    pub(crate) fn mirror_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Mirror {
            refs,
            axis_start,
            axis_end,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "MirrorEntities needs a Mirror { refs, axis_start, axis_end } payload".into(),
            ));
        };
        if !is_finite_point(*axis_start) || !is_finite_point(*axis_end) {
            return Err(CadError::InvalidInput(
                "MirrorEntities axis points must be finite".into(),
            ));
        }
        let transform = crate::draw_tool::mirror_transform(*axis_start, *axis_end)?;
        self.mirror_selection(command, refs, &transform)
    }
}
