use super::*;

impl Application {
    /// Create an offset copy of a single selected LINE or CIRCLE.
    ///
    /// The picked `side_point` defines both the side and the distance: for a LINE
    /// it is the perpendicular distance to the infinite line (the new line lies
    /// on the side the point is on); for a CIRCLE the new radius is
    /// `|side_point − centre|` (a point outside the circle grows it, inside
    /// shrinks it). Any other geometry kind is an explicit `Unsupported` refusal;
    /// exactly one source entity is required. The new entity has a fresh id, the
    /// source's layer/space, and the source's display state; one transaction and
    /// one undo step.
    pub(crate) fn offset_entity(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Offset { refs, side_point } = &command.payload else {
            return Err(CadError::InvalidInput(
                "OffsetEntity needs an Offset { refs, side_point } payload".into(),
            ));
        };
        if !is_finite_point(*side_point) {
            return Err(CadError::InvalidInput(
                "OffsetEntity side point must be finite".into(),
            ));
        }
        let document_id = command.document;
        let (geometry, layer, space, display, message, diagnostics) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = &document.drawing;
            let (sources, diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                refs,
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
            let source = &sources[0];
            let (geometry, message) = match &source.geometry {
                SemanticGeometry::Line { start, end } => {
                    let (geometry, distance) = offset_line(*start, *end, *side_point)?;
                    (geometry, format!("OFFSET：直线沿拾取侧偏移 {distance:.4}"))
                }
                SemanticGeometry::Circle {
                    center,
                    normal,
                    radius,
                } => {
                    let (geometry, distance) =
                        offset_circle(*center, *normal, *radius, *side_point)?;
                    (geometry, format!("OFFSET：圆偏移，新半径 {distance:.4}"))
                }
                other => {
                    return Err(CadError::Unsupported(format!(
                        "OFFSET supports LINE and CIRCLE only, got {}",
                        geometry_kind(other)
                    )));
                }
            };
            let display = drawing.entity_display_state(source.id);
            (
                geometry,
                source.layer,
                source.space.clone(),
                display,
                message,
                diagnostics,
            )
        };
        let mut outcome = self.commit_new_entity_in(
            command,
            geometry,
            layer,
            space,
            Some(display),
            "offset entity",
        )?;
        let mut merged = vec![Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.offset".into(),
            message,
        }];
        merged.extend(diagnostics);
        outcome.diagnostics = merged;
        Ok(outcome)
    }

    /// Trim one LINE against LINE/LWPOLYLINE straight segments (spec §3).
    ///
    /// Supported subset only: the target must be a LINE and every boundary must
    /// be a LINE or an all-straight-segment LWPOLYLINE. The target is cut at the
    /// 2D intersections with the boundary and the piece containing `pick_point`
    /// is kept; a line fully inside a closed boundary is deleted. Curved
    /// boundaries, ARC/SPLINE/INSERT/Opaque targets, collinear/degenerate or
    /// intersection-free cases are explicit refusals that change nothing.
    pub(crate) fn trim_entity(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Trim {
            target,
            boundary,
            pick_point,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "TrimEntity needs a Trim { target, boundary, pick_point } payload".into(),
            ));
        };
        if target.document != command.document {
            return Err(CadError::InvalidInput(
                "TrimEntity target belongs to another document".into(),
            ));
        }
        if boundary.is_empty() {
            return Err(CadError::InvalidInput(
                "TrimEntity needs at least one boundary reference".into(),
            ));
        }
        if !is_finite_point(*pick_point) {
            return Err(CadError::InvalidInput(
                "TrimEntity pick point must be finite".into(),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // Resolve every input to plain data before mutating anything: an
        // unsupported boundary or target is a refusal with zero database change.
        let target_entity = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                CadError::InvalidInput(format!("trim target {} does not exist", target.entity.0))
            })?;
        let (a, b) = match &target_entity.geometry {
            SemanticGeometry::Line { start, end } => (*start, *end),
            other => {
                return Err(CadError::Unsupported(format!(
                    "TRIM target must be a LINE; {} is not supported",
                    geometry_kind(other)
                )));
            }
        };
        let mut segments: Vec<(Point3, Point3)> = Vec::new();
        let mut closed_loops: Vec<Vec<Point3>> = Vec::new();
        for reference in boundary {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(
                    "TrimEntity boundary belongs to another document".into(),
                ));
            }
            let entity = document.drawing.entity(reference.entity).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "trim boundary {} does not exist",
                    reference.entity.0
                ))
            })?;
            collect_boundary_segments(&entity.geometry, &mut segments, &mut closed_loops, "TRIM")?;
        }
        if segments.is_empty() {
            return Err(CadError::Unsupported(
                "TRIM boundary has no straight segments to cut against".into(),
            ));
        }

        let plan = match plan_trim(a, b, *pick_point, &segments, &closed_loops) {
            Ok(plan) => plan,
            // A plan-level refusal (degenerate target, no intersection, pick off
            // the target) is invalid *input*, not an unsupported geometry, so it
            // maps to the same "no intersection" surface as EXTEND.
            Err(reason) => return Err(CadError::InvalidInput(reason)),
        };
        // Capture the display state a full delete will prune, so undo restores it.
        let target_state = document.drawing.entity_display_state(target_entity.id);

        let transaction = next_transaction_id();
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let change_set = match plan {
            TrimPlan::Delete => {
                let mut staged = drawing.begin_drawing_transaction("trim entity", transaction)?;
                staged.delete_entity(target_entity.id)?;
                staged.commit()?
            }
            TrimPlan::Keep { start, end } => {
                let mut after = target_entity.clone();
                after.geometry = SemanticGeometry::Line { start, end };
                let mut staged = drawing.begin_drawing_transaction("trim entity", transaction)?;
                staged.update_entity(after.clone())?;
                staged.commit()?
            }
        };

        let target_id = target_entity.id;
        let patch = match plan {
            TrimPlan::Delete => drawing_patch_deleted(target_id, target_entity, target_state),
            TrimPlan::Keep { .. } => {
                // `after` was recomputed above; rebuild it from the committed
                // geometry so the patch is the exact stored value.
                let after = drawing.entity(target_id).cloned().ok_or_else(|| {
                    CadError::Invariant("trimmed entity vanished after commit".into())
                })?;
                drawing_patch(target_id, Some(target_entity.clone()), Some(after))
            }
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "trim entity".into(),
                patches: vec![patch],
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(target_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(target_id.0)),
                code: "drawing.trim".into(),
                message: match plan {
                    TrimPlan::Delete => "TRIM：目标完全位于边界内，整段删除".to_string(),
                    TrimPlan::Keep { .. } => {
                        "TRIM：保留拾取点一侧（LINE 对 LINE 子集）".to_string()
                    }
                },
            }],
            measurement: None,
        })
    }

    /// Extend one LINE to the nearest LINE/LWPOLYLINE boundary segment.
    ///
    /// The supported subset mirrors TRIM: the target must be a LINE and every
    /// boundary must be a LINE or an all-straight-segment LWPOLYLINE. The
    /// endpoint nearer `pick_point` is moved to the nearest boundary crossing
    /// beyond it. A non-LINE target, a curved/unsupported boundary, a target that
    /// already touches or crosses the boundary, and a boundary whose segment
    /// never meets the target's line are all explicit refusals that change
    /// nothing.
    pub(crate) fn extend_entity(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Extend {
            target,
            boundary,
            pick_point,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "ExtendEntity needs an Extend { target, boundary, pick_point } payload".into(),
            ));
        };
        if target.document != command.document {
            return Err(CadError::InvalidInput(
                "ExtendEntity target belongs to another document".into(),
            ));
        }
        if boundary.is_empty() {
            return Err(CadError::InvalidInput(
                "ExtendEntity needs at least one boundary reference".into(),
            ));
        }
        if !is_finite_point(*pick_point) {
            return Err(CadError::InvalidInput(
                "ExtendEntity pick point must be finite".into(),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // Resolve every input to plain data before mutating anything: an
        // unsupported boundary or target is a refusal with zero database change.
        let target_entity = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                CadError::InvalidInput(format!("extend target {} does not exist", target.entity.0))
            })?;
        let (a, b) = match &target_entity.geometry {
            SemanticGeometry::Line { start, end } => (*start, *end),
            other => {
                return Err(CadError::Unsupported(format!(
                    "EXTEND target must be a LINE; {} is not supported",
                    geometry_kind(other)
                )));
            }
        };
        let mut segments: Vec<(Point3, Point3)> = Vec::new();
        let mut closed_loops: Vec<Vec<Point3>> = Vec::new();
        for reference in boundary {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(
                    "ExtendEntity boundary belongs to another document".into(),
                ));
            }
            let entity = document.drawing.entity(reference.entity).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "extend boundary {} does not exist",
                    reference.entity.0
                ))
            })?;
            collect_boundary_segments(
                &entity.geometry,
                &mut segments,
                &mut closed_loops,
                "EXTEND",
            )?;
        }
        if segments.is_empty() {
            return Err(CadError::Unsupported(
                "EXTEND boundary has no straight segments to extend to".into(),
            ));
        }

        let (start, end) = match plan_extend(a, b, *pick_point, &segments) {
            Ok(range) => range,
            Err(reason) => return Err(CadError::InvalidInput(reason)),
        };

        let transaction = next_transaction_id();
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let mut after = target_entity.clone();
        after.geometry = SemanticGeometry::Line { start, end };
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("extend entity", transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        let target_id = target_entity.id;
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let stored = drawing
            .entity(target_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("extended entity vanished after commit".into()))?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "extend entity".into(),
                patches: vec![drawing_patch(target_id, Some(target_entity), Some(stored))],
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(target_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(target_id.0)),
                code: "drawing.extend".into(),
                message: "EXTEND：目标已延伸至最近的边界（LINE 对 LINE/LWPOLYLINE 子集）"
                    .to_string(),
            }],
            measurement: None,
        })
    }

    /// Shared insert path for LINE/CIRCLE/ARC creation (active layer, model space).
    pub(super) fn commit_new_entity(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometry: SemanticGeometry,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.commit_new_entity_in(
            command,
            geometry,
            session.active_layer,
            SpaceId::Model,
            None,
            label,
        )
    }

    /// Insert a new entity into `layer`/`space`, optionally carrying a captured
    /// display state (render attributes + dynamic-visibility membership).
    ///
    /// The layer must exist: a missing layer is an explicit error, never a silent
    /// fallback. A captured display state is restored after commit and recorded on
    /// the patch's `after_state` so a redo re-applies it.
    fn commit_new_entity_in(
        &mut self,
        command: &Command,
        geometry: SemanticGeometry,
        layer: LayerId,
        space: SpaceId,
        display: Option<cad_db::EntityDisplayState>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        if drawing.layer(layer).is_none() {
            return Err(CadError::InvalidInput(format!(
                "layer {} does not exist",
                layer.0
            )));
        }
        let draw_order = drawing
            .model_space()
            .iter()
            .map(|e| e.draw_order)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0);
        let entity_id = drawing.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(entity_id.0),
                type_key: class_name(&geometry).to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: entity_id,
            layer,
            space,
            geometry,
            draw_order,
        };

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            staged.insert_entity(entity.clone())?;
            staged.commit()?
        };
        if let Some(display) = &display {
            drawing.restore_entity_display_state(entity_id, display);
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches: vec![DrawingPatch {
                    id: entity_id,
                    before: None,
                    after: Some(entity),
                    before_state: None,
                    after_state: display,
                }],
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: Vec::new(),
            measurement: None,
        })
    }

    /// Record a committed candidate before publishing it as authoritative state.
    ///
    /// `record_drawing` validates before mutating history. Therefore a failure
    /// leaves both the current drawing (including its ID allocator) and history
    /// unchanged; after it succeeds, publishing the already-built candidate is
    /// infallible.
    pub(super) fn publish_candidate(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
        record: DrawingUndoRecord,
    ) -> CadResult<()> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing(record)?;
        document.drawing = Arc::new(candidate);
        Ok(())
    }
}
