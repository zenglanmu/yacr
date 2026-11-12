use super::*;

use super::helpers::offset_tolerance;
use crate::command_payload::{
    MirrorOptions, OffsetGapType, OffsetLayer, OffsetMode, OffsetOptions,
};

/// One deleted row plus the display state the delete pruned (the `OOPS` snapshot
/// entry shape).
pub(crate) type DeletedEntityState = (DbEntity, cad_db::EntityDisplayState);

/// The `delete_entity_refs_with_state` result: the change set, the diagnostics
/// and the deleted rows (with display state) captured for `OOPS`.
pub(crate) type DeleteWithStateResult = (
    Option<Arc<ChangeSet>>,
    Vec<Diagnostic>,
    Vec<DeletedEntityState>,
);

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
            DrawingUndoRecord::entity_only(change_set.transaction, "move entities", patches),
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
        session: &mut SessionState,
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
        let (changes, diagnostics, states) =
            self.delete_entity_refs_with_state(command.document, &refs, "ERASE")?;
        // OOPS state is recorded **only after** the delete transaction committed
        // (the helper returned successfully) and only by ERASE — the two cut
        // commands use `delete_entity_refs`, which discards this state.
        session.set_last_erase(Some(crate::session_state::EraseSnapshot {
            document: command.document,
            entries: states
                .into_iter()
                .map(|(entity, display)| crate::session_state::EraseEntry { entity, display })
                .collect(),
        }));
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Delete exactly `refs` in one transaction and one undo step.
    ///
    /// This is the **single** delete body: ERASE passes the session selection,
    /// `CUTCLIP` passes the same snapshot it captured, and `CUTBASE` passes the
    /// payload refs (which need not match the session selection any more). Every
    /// ref is validated **before** the transaction opens — a foreign-document ref,
    /// an entity id that does not exist, or an empty list refuses the whole batch
    /// and writes nothing — and the batch is committed once, so a cut can never
    /// leave a partially deleted selection.
    ///
    /// Sub-element/instance refs are accepted as whole-entity deletes with a
    /// diagnostic (`drawing.erase.sub_element` / `drawing.erase.instance`), exactly
    /// like MOVE/COPY report their whole-entity fallback. The display state pruned
    /// by the delete is captured and recorded on the undo patch, so undo restores
    /// the render attributes and dynamic-visibility membership.
    pub(crate) fn delete_entity_refs(
        &mut self,
        document_id: DocumentId,
        refs: &[SelectionRef],
        operation: &str,
    ) -> CadResult<(Option<Arc<ChangeSet>>, Vec<Diagnostic>)> {
        let (changes, diagnostics, _) =
            self.delete_entity_refs_with_state(document_id, refs, operation)?;
        Ok((changes, diagnostics))
    }

    /// The single delete body, additionally returning the exact deleted rows and
    /// their display state so ERASE can record its `OOPS` snapshot.
    pub(crate) fn delete_entity_refs_with_state(
        &mut self,
        document_id: DocumentId,
        refs: &[SelectionRef],
        operation: &str,
    ) -> CadResult<DeleteWithStateResult> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // The shared resolution collapses duplicates, refuses any unaddressable
        // ref and refuses an empty batch — all before a write.
        let (before_states, diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            refs,
            "erase",
            operation,
            "删除",
            false,
        )?;
        // Capture the display state the delete will prune, so undo restores it.
        let display_states: Vec<_> = before_states
            .iter()
            .map(|before| drawing.entity_display_state(before.id))
            .collect();

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("erase entities", transaction)?;
            for before in &before_states {
                staged.delete_entity(before.id)?;
            }
            staged.commit()?
        };
        let deleted: Vec<DeletedEntityState> = before_states
            .iter()
            .cloned()
            .zip(display_states.iter().cloned())
            .collect();
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .zip(display_states)
            .map(|(before, state)| drawing_patch_deleted(before.id, before, state))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord::entity_only(change_set.transaction, "erase entities", patches),
        )?;

        Ok((Some(Arc::new(change_set)), diagnostics, deleted))
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
            DrawingUndoRecord::entity_only(change_set.transaction, "copy entities", patches),
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

    /// Mirror the current selection across the axis `axis_start`→`axis_end`
    /// (the bare payload form).
    ///
    /// This is exactly AutoCAD's default option set: **keep the originals**
    /// (`erase_source = false`) and **readable text** (`MIRRTEXT = 0`). It
    /// delegates to [`Self::mirror_entities_with_options`], the full option
    /// entry point, so the two can never disagree.
    pub(crate) fn mirror_entities(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let options = match &command.payload {
            CommandPayload::Mirror {
                refs,
                axis_start,
                axis_end,
            } => MirrorOptions {
                refs: refs.clone(),
                axis_start: *axis_start,
                axis_end: *axis_end,
                mirrortext: false,
                erase_source: false,
            },
            CommandPayload::MirrorOptions(options) => {
                return self.mirror_entities_with_options(command, options);
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "MirrorEntities needs a Mirror { refs, axis_start, axis_end } or \
                     MirrorOptions payload"
                        .into(),
                ));
            }
        };
        self.mirror_entities_with_options(command, &options)
    }

    /// The full MIRROR option set: `MIRRTEXT` and `Erase source objects`.
    ///
    /// `MIRRTEXT = 0` (readable) is the default and the implemented behaviour:
    /// text/MTEXT positions mirror and the run stays readable (ATTDEF would
    /// follow the same rule, but this build has no attribute model), and a
    /// mirrored block reference reflects its placement, so member glyphs
    /// (including constant-attribute text) are reversed with it **regardless** of
    /// `MIRRTEXT`; that is a global note, not a per-attribute rule. `MIRRTEXT = 1`
    /// is a named refusal (`mirror.mirrortext_glyphs`) because the flat-text
    /// domain cannot mirror glyphs.
    ///
    /// Dimensions are mirrored as a **documented choice**: definition points are
    /// reflected and the display is re-synthesized (Autodesk does not document
    /// MIRROR on dimensions). `erase_source` deletes every source in the same
    /// transaction as the insert, so one undo step restores them.
    ///
    /// Wiring: [`CommandPayload::MirrorOptions`] routes here directly; the bare
    /// `Mirror` payload keeps the defaults above.
    pub(crate) fn mirror_entities_with_options(
        &mut self,
        command: &Command,
        options: &MirrorOptions,
    ) -> CadResult<CommandOutcome> {
        if !is_finite_point(options.axis_start) || !is_finite_point(options.axis_end) {
            return Err(CadError::InvalidInput(
                "MirrorEntities axis points must be finite".into(),
            ));
        }
        let transform = crate::draw_tool::mirror_transform(options.axis_start, options.axis_end)?;
        self.mirror_selection_with_options(
            command,
            &options.refs,
            &transform,
            options.mirrortext,
            options.erase_source,
        )
    }
}

impl Application {
    /// The full OFFSET option set: distance / `Through`, `Erase`, `Layer`,
    /// `Multiple` (one side/through point per repetition) and the
    /// `OFFSETGAPTYPE` join.
    ///
    /// Supported sources: LINE, CIRCLE, ARC, ELLIPSE (full or elliptical arc),
    /// LWPOLYLINE (straight or bulged) and SPLINE. An ellipse or elliptical arc
    /// is offset into a **clamped approximating B-spline** through sampled
    /// offset points — never an ellipse — with the sampling tolerance documented
    /// on [`super::offset_polyline::offset_ellipse_as_spline`]. A spline source
    /// is likewise offset through samples; a non-planar spline is refused
    /// (`offset.non_planar`).
    ///
    /// A polyline is offset segment by segment with the configured
    /// `OFFSETGAPTYPE` join: `Sharp` (0) uses the analytic miter for straight
    /// vertices and the exact line/circle and circle/circle intersections of the
    /// arc arm; `Fillet` (1) fills a real gap with the tangent arc of radius equal
    /// to the offset distance centred on the source vertex; `Chamfer` (2) joins
    /// the two offset segment ends with a straight segment. Where the offset
    /// segments already cross (a concave corner, a trim not a gap) every gap type
    /// keeps the miter. A **closed** offset ring that crosses itself is refused by
    /// name (`offset.self_intersection`): Autodesk does not document that case, so
    /// this build neither trims nor fills it.
    ///
    /// `Erase` deletes the source in the **same** transaction as the insert(s)
    /// (one undo step restores it). `Layer::Current` stores the caller-resolved
    /// current layer; `Layer::Source` keeps the source layer. Every offset
    /// carries the source's display state. A refusal changes nothing and records
    /// no history.
    pub(crate) fn offset_entities_with_options(
        &mut self,
        command: &Command,
        options: &OffsetOptions,
    ) -> CadResult<CommandOutcome> {
        if options.refs.is_empty() {
            return Err(CadError::InvalidInput(
                "OffsetEntity needs at least one selected reference".into(),
            ));
        }
        if options.side_points.is_empty() {
            return Err(CadError::InvalidInput(
                "OFFSET needs at least one side/through point".into(),
            ));
        }
        if !options
            .side_points
            .iter()
            .all(|point| is_finite_point(*point))
        {
            return Err(CadError::InvalidInput(
                "OffsetEntity side point must be finite".into(),
            ));
        }
        let fixed_distance = match options.mode {
            OffsetMode::Distance(distance) => {
                if !distance.is_finite() || distance <= offset_tolerance() {
                    return Err(CadError::InvalidInput(
                        "OFFSET distance must be finite and positive".into(),
                    ));
                }
                Some(distance)
            }
            OffsetMode::Through => None,
        };
        let document_id = command.document;
        let (source, layer, display, planned, diagnostics) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = &document.drawing;
            let (sources, diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                &options.refs,
                "offset",
                "OffsetEntity",
                "偏移",
                false,
            )?;
            if sources.len() != 1 {
                return Err(CadError::InvalidInput(format!(
                    "OffsetEntity requires exactly one selected entity, got {}",
                    sources.len()
                )));
            }
            let source = sources[0].clone();
            let layer = match options.layer {
                OffsetLayer::Source => source.layer,
                OffsetLayer::Current(layer) => layer,
            };
            if drawing.layer(layer).is_none() {
                return Err(CadError::InvalidInput(format!(
                    "OFFSET target layer {} does not exist",
                    layer.0
                )));
            }
            let display = drawing.entity_display_state(source.id);
            let mut planned = Vec::with_capacity(options.side_points.len());
            for side_point in &options.side_points {
                planned.push(plan_offset(
                    &source.geometry,
                    *side_point,
                    fixed_distance,
                    options.gap,
                )?);
            }
            (source, layer, display, planned, diagnostics)
        };

        let geometries: Vec<SemanticGeometry> = planned
            .iter()
            .map(|(geometry, _, _)| geometry.clone())
            .collect();
        let erase = options
            .erase_source
            .then(|| (source.clone(), display.clone()));
        let mut outcome = self.commit_offset_batch(
            command,
            geometries,
            layer,
            source.space.clone(),
            Some(display),
            erase,
        )?;
        let mut merged = Vec::new();
        for (index, (_, applied, message)) in planned.iter().enumerate() {
            let object = outcome.objects.get(index).copied();
            merged.push(Diagnostic {
                object,
                code: "drawing.offset".into(),
                message: message.replace("{distance}", &format!("{applied:.4}")),
            });
        }
        merged.extend(diagnostics);
        outcome.diagnostics = merged;
        Ok(outcome)
    }

    /// Insert every offset in one transaction, optionally erasing the source in
    /// the same transaction (one undo step).
    fn commit_offset_batch(
        &mut self,
        command: &Command,
        geometries: Vec<SemanticGeometry>,
        layer: LayerId,
        space: SpaceId,
        display: Option<cad_db::EntityDisplayState>,
        erase: Option<(DbEntity, cad_db::EntityDisplayState)>,
    ) -> CadResult<CommandOutcome> {
        if geometries.is_empty() {
            return Err(CadError::InvalidInput(
                "OFFSET needs at least one offset to insert".into(),
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
        if drawing.layer(layer).is_none() {
            return Err(CadError::InvalidInput(format!(
                "OFFSET target layer {} does not exist",
                layer.0
            )));
        }
        let base_order = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let mut entities: Vec<DbEntity> = Vec::with_capacity(geometries.len());
        for (index, geometry) in geometries.into_iter().enumerate() {
            let entity_id = drawing.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: class_name(&geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: entity_id,
                layer,
                space: space.clone(),
                geometry,
                draw_order: base_order + index as i64,
            });
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("offset entity", transaction)?;
            for entity in &entities {
                staged.insert_entity(entity.clone())?;
            }
            if let Some(state) = &display {
                for entity in &entities {
                    staged.update_entity_display_state(entity.id, state)?;
                }
            }
            if let Some((source, _)) = &erase {
                staged.delete_entity(source.id)?;
            }
            staged.commit()?
        };
        let mut patches: Vec<DrawingPatch> = entities
            .iter()
            .map(|entity| DrawingPatch {
                id: entity.id,
                before: None,
                after: Some(entity.clone()),
                before_state: None,
                after_state: display.clone(),
            })
            .collect();
        if let Some((source, state)) = erase {
            patches.push(drawing_patch_deleted(source.id, source, state));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord::entity_only(change_set.transaction, "offset entity", patches),
        )?;

        Ok(CommandOutcome {
            objects: entities
                .iter()
                .map(|entity| ObjectId(entity.id.0))
                .collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: Vec::new(),
            measurement: None,
            point_report: None,
        })
    }
}

/// Plan one offset for the given source geometry and side/through point.
///
/// `distance = Some(d)` is the fixed-distance prompt (the point only picks the
/// side); `None` is `Through` (the pick also fixes the distance). The returned
/// message contains a `{distance}` placeholder the caller replaces with the
/// applied distance, so the command reports one number per created entity.
fn plan_offset(
    geometry: &SemanticGeometry,
    side_point: Point3,
    distance: Option<f64>,
    gap: OffsetGapType,
) -> CadResult<(SemanticGeometry, f64, String)> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let (geometry, applied) =
                super::offset_fixed::offset_line_with_distance(*start, *end, side_point, distance)?;
            Ok((
                geometry,
                applied,
                "OFFSET: line offset by {distance} on the picked side".into(),
            ))
        }
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (geometry, applied) = super::offset_fixed::offset_circle_with_distance(
                *center, *normal, *radius, side_point, distance,
            )?;
            Ok((
                geometry,
                applied,
                "OFFSET: circle offset, new radius {distance}".into(),
            ))
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (geometry, applied) = super::offset_fixed::offset_arc_with_distance(
                *center, *normal, *radius, *start, *sweep, side_point, distance,
            )?;
            Ok((
                geometry,
                applied,
                "OFFSET: arc offset, new radius {distance}".into(),
            ))
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            let (geometry, applied) = match distance {
                Some(_) => offset_polyline_with_distance(
                    points, bulges, *closed, side_point, distance, gap,
                )?,
                None => super::offset_polyline::offset_polyline(
                    points, bulges, *closed, side_point, gap,
                )?,
            };
            let join = match gap {
                OffsetGapType::Sharp => "sharp",
                OffsetGapType::Fillet => "fillet",
                OffsetGapType::Chamfer => "chamfer",
            };
            Ok((
                geometry,
                applied,
                format!("OFFSET: polyline offset by {{distance}} ({join} joins)"),
            ))
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let (geometry, applied) = super::offset_polyline::offset_ellipse_as_spline(
                *center,
                *normal,
                *major_axis,
                *ratio,
                *start,
                *sweep,
                side_point,
                distance,
            )?;
            Ok((
                geometry,
                applied,
                "OFFSET: ellipse offset approximated by a spline, distance {distance}".into(),
            ))
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            let (geometry, applied) = super::offset_polyline::offset_spline_as_spline(
                *degree,
                knots,
                control_points,
                weights,
                side_point,
                distance,
            )?;
            Ok((
                geometry,
                applied,
                "OFFSET: spline offset approximated by a spline, distance {distance}".into(),
            ))
        }
        other => Err(CadError::Unsupported(format!(
            "OFFSET supports LINE, CIRCLE, ARC, ELLIPSE, LWPOLYLINE and SPLINE, got {}",
            geometry_kind(other)
        ))),
    }
}
