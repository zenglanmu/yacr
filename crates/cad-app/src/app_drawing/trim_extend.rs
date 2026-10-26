use super::trim_boundary::{collect_boundaries, BoundarySet};
use super::trim_curves::{cut_tolerance, plan_extend, plan_trim, TargetCurve, TrimPlan};
use super::*;

/// The per-insertion options of the shared create funnel
/// ([`Application::insert_new_entities`]).
///
/// Bundled instead of six positional parameters so the stored `type_key` override
/// fits without a second, near-identical funnel. `type_key: None` is exactly the
/// pre-existing behaviour ([`class_name`]'s answer).
struct NewEntityInsert<'a> {
    layer: LayerId,
    space: SpaceId,
    /// A captured display state (render attributes + dynamic-visibility
    /// membership); single-entity OFFSET only.
    display: Option<cad_db::EntityDisplayState>,
    /// The stored `type_key` override; `None` = [`class_name`].
    type_key: Option<&'a str>,
    /// The transaction / undo label (also used in refusal messages).
    label: &'a str,
}

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

    /// Trim one LINE/ARC/CIRCLE target against a supported boundary set.
    ///
    /// Supported subset: the target may be a LINE, an ARC or a CIRCLE, and every
    /// boundary may be a LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE (straight or
    /// bulged) or SPLINE, in any mix. The target is cut at the intersections with
    /// the boundary and the piece containing the **projection** of `pick_point` is
    /// kept — the pick is projected onto the target ([`TargetCurve::project_param`])
    /// because the host resolves the target with its own pickbox, so a click a few
    /// pixels off the curve is still a pick of this entity. A LINE keeps the
    /// sub-segment, an ARC keeps a shorter arc with the same centre/normal/radius
    /// and the same sweep sign, a CIRCLE becomes the one ARC between the two cuts
    /// that bracket the pick. A LINE with no cut that lies wholly inside a closed
    /// boundary is deleted.
    ///
    /// A SPLINE boundary is **sampled**, not analytic: every cut it contributes
    /// adds a `drawing.trim` diagnostic saying the cut point is approximate. It is
    /// also excluded from the "wholly inside" delete rule (a spline has no closed
    /// flag and no boundary plane), so a target wholly inside a closed spline is
    /// refused explicitly (`Unsupported`) rather than deleted.
    ///
    /// SPLINE/INSERT/Opaque/Mesh **targets**, INSERT/Opaque/Mesh boundaries,
    /// degenerate input, a pick whose projection falls outside the target's span,
    /// a pick with no radial direction on an ARC/CIRCLE (it lies on the target's
    /// axis) and intersection-free cases are explicit refusals that change
    /// nothing; the whole command is one transaction and one undo step.
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
        let policy = TolerancePolicy::default();
        let tol = cut_tolerance();
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
        let curve = TargetCurve::new(&target_entity.geometry, "TRIM", "a LINE, ARC or CIRCLE")?;
        let mut boundaries = BoundarySet::default();
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
            collect_boundaries(&entity.geometry, &mut boundaries, &policy, "TRIM")?;
        }
        if boundaries.is_empty() {
            return Err(CadError::Unsupported(
                "TRIM boundary has no supported geometry to cut against".into(),
            ));
        }

        let resolution = match plan_trim(&curve, *pick_point, &boundaries, &policy, tol) {
            Ok(resolution) => resolution,
            // A plan-level refusal (degenerate target, no intersection, a pick
            // whose projection falls outside the target's span) is invalid
            // *input*, not an unsupported geometry, so it maps to the same "no
            // intersection" surface as EXTEND. The one capability gap the plan
            // reports (closed-SPLINE containment) stays `Unsupported`.
            Err(reason) => return Err(reason.into_cad_error()),
        };
        // Cuts the engine could only approximate are reported, never hidden.
        let notes = resolution.notes;
        let plan = resolution.plan;
        // Capture the display state a full delete will prune, so undo restores it.
        let target_state = document.drawing.entity_display_state(target_entity.id);

        let transaction = next_transaction_id();
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("trim entity", transaction)?;
            match &plan {
                TrimPlan::Delete => staged.delete_entity(target_entity.id)?,
                TrimPlan::Replace(geometry) => {
                    let mut after = target_entity.clone();
                    after.geometry = geometry.clone();
                    staged.update_entity(after)?;
                }
            }
            staged.commit()?
        };

        let target_id = target_entity.id;
        let patch = match &plan {
            TrimPlan::Delete => drawing_patch_deleted(target_id, target_entity, target_state),
            TrimPlan::Replace(_) => {
                // Read the committed entity back so the patch is the exact stored
                // value, never a re-derivation.
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

        let mut diagnostics = vec![Diagnostic {
            object: Some(ObjectId(target_id.0)),
            code: "drawing.trim".into(),
            message: match &plan {
                TrimPlan::Delete => "TRIM：目标完全位于边界内，整段删除".to_string(),
                TrimPlan::Replace(_) => {
                    format!("TRIM: kept the picked piece ({} target)", curve.label())
                }
            },
        }];
        diagnostics.extend(notes.into_iter().map(|message| Diagnostic {
            object: Some(ObjectId(target_id.0)),
            code: "drawing.trim".into(),
            message,
        }));
        Ok(CommandOutcome {
            objects: vec![ObjectId(target_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Extend one LINE/ARC target to the nearest boundary crossing ahead.
    ///
    /// The supported subset mirrors TRIM: the target may be a LINE or an ARC and
    /// every boundary may be a LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE (straight or
    /// bulged) or SPLINE, in any mix. A SPLINE boundary is sampled, so a cut it
    /// contributes always adds an approximation note (`drawing.extend`). The pick
    /// is projected onto the target and that projection chooses the end to grow. A
    /// LINE's picked endpoint moves along its own infinite line; an ARC's picked
    /// end moves along its own circle and keeps the sweep's sign, so both only ever
    /// get longer. A CIRCLE target has no end and is refused, as are unsupported
    /// targets/boundaries, a pick whose projection falls outside the target's span,
    /// a target that already touches or reaches the boundary, and a boundary that
    /// never lies ahead.
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
        let policy = TolerancePolicy::default();
        let tol = cut_tolerance();
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
        let curve = TargetCurve::new(&target_entity.geometry, "EXTEND", "a LINE or ARC")?;
        if curve.is_periodic() {
            return Err(CadError::Unsupported(format!(
                "EXTEND target {} is closed and has no endpoint; only LINE and ARC are supported",
                geometry_kind(&target_entity.geometry)
            )));
        }
        let mut boundaries = BoundarySet::default();
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
            collect_boundaries(&entity.geometry, &mut boundaries, &policy, "EXTEND")?;
        }
        if boundaries.is_empty() {
            return Err(CadError::Unsupported(
                "EXTEND boundary has no supported geometry to extend to".into(),
            ));
        }

        let resolution = match plan_extend(&curve, *pick_point, &boundaries, &policy, tol) {
            Ok(resolution) => resolution,
            Err(reason) => return Err(CadError::InvalidInput(reason)),
        };
        // Cuts the engine could only approximate are reported, never hidden.
        let notes = resolution.notes;
        let geometry = resolution.geometry;

        let transaction = next_transaction_id();
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let mut after = target_entity.clone();
        after.geometry = geometry;
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("extend entity", transaction)?;
            staged.update_entity(after)?;
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

        let mut diagnostics = vec![Diagnostic {
            object: Some(ObjectId(target_id.0)),
            code: "drawing.extend".into(),
            message: format!(
                "EXTEND: {} extended to the nearest boundary ahead",
                curve.label()
            ),
        }];
        diagnostics.extend(notes.into_iter().map(|message| Diagnostic {
            object: Some(ObjectId(target_id.0)),
            code: "drawing.extend".into(),
            message,
        }));
        Ok(CommandOutcome {
            objects: vec![ObjectId(target_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
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

    /// The same insert path with an explicit **stored type key** override.
    ///
    /// Every geometry gets its `type_key` from [`class_name`], which is right for
    /// the analytic entities but cannot express "this `Text` geometry came from
    /// MTEXT" — both would be `AcDbText`. MTEXT creation passes `AcDbMText` here
    /// so the newline policy, the export path and `TEXTEDIT` agree on the same
    /// entity. Everything else (active layer, model space, one transaction, one
    /// undo step, all-or-nothing) is the shared funnel.
    pub(super) fn commit_new_entity_typed(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometry: SemanticGeometry,
        type_key: &str,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.insert_new_entities(
            command,
            vec![geometry],
            NewEntityInsert {
                layer: session.active_layer,
                space: SpaceId::Model,
                display: None,
                type_key: Some(type_key),
                label,
            },
        )
    }

    /// Shared insert path for a **batch** of new entities on the active layer in
    /// model space (DIMCENTER inserts several plain LINEs in one transaction /
    /// one undo step).
    ///
    /// Same single funnel as [`Self::commit_new_entity`]: the layer must exist (a
    /// missing layer is an explicit error), every entity lands in one transaction
    /// and one undo step, and the whole batch is all-or-nothing. An empty batch is
    /// an explicit refusal, never a silent empty success.
    pub(super) fn commit_new_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometries: Vec<SemanticGeometry>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.commit_new_entities_in(
            command,
            geometries,
            session.active_layer,
            SpaceId::Model,
            None,
            label,
        )
    }

    /// Insert a new entity into `layer`/`space`, optionally carrying a captured
    /// display state (render attributes + dynamic-visibility membership).
    ///
    /// A thin wrapper over [`Self::commit_new_entities_in`] for the single-entity
    /// callers; the display state (used by OFFSET) applies to that one entity.
    fn commit_new_entity_in(
        &mut self,
        command: &Command,
        geometry: SemanticGeometry,
        layer: LayerId,
        space: SpaceId,
        display: Option<cad_db::EntityDisplayState>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.commit_new_entities_in(command, vec![geometry], layer, space, display, label)
    }

    /// Insert a batch of new entities into `layer`/`space` in one transaction.
    ///
    /// The layer must exist: a missing layer is an explicit error, never a silent
    /// fallback. A captured display state (single-entity OFFSET only) is restored
    /// on the **first** inserted entity after commit and recorded on its patch's
    /// `after_state` so a redo re-applies it; a batch (DIMCENTER) passes `None`.
    ///
    /// Every entity's stored `type_key` is [`class_name`]'s answer; the
    /// type-key-carrying variant [`Self::commit_new_entity_typed`] is what MTEXT
    /// creation uses and every caller here keeps exactly that behaviour.
    fn commit_new_entities_in(
        &mut self,
        command: &Command,
        geometries: Vec<SemanticGeometry>,
        layer: LayerId,
        space: SpaceId,
        display: Option<cad_db::EntityDisplayState>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.insert_new_entities(
            command,
            geometries,
            NewEntityInsert {
                layer,
                space,
                display,
                type_key: None,
                label,
            },
        )
    }

    /// The shared create funnel: one transaction, one undo step, all-or-nothing.
    ///
    /// `insert.type_key` is the **only** variation between an ordinary creation and
    /// a typed one: `None` stamps [`class_name`] (every pre-existing caller), and
    /// `Some(key)` stamps that key instead. The override is refused for a batch —
    /// sharing one key across entities it was not meant for would mis-label them —
    /// so it can only describe a single created entity.
    fn insert_new_entities(
        &mut self,
        command: &Command,
        geometries: Vec<SemanticGeometry>,
        insert: NewEntityInsert<'_>,
    ) -> CadResult<CommandOutcome> {
        let NewEntityInsert {
            layer,
            space,
            display,
            type_key,
            label,
        } = insert;
        if geometries.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{label} needs at least one entity to insert"
            )));
        }
        if type_key.is_some() && geometries.len() != 1 {
            return Err(CadError::InvalidInput(format!(
                "{label} can stamp an explicit type key on exactly one entity, got {}",
                geometries.len()
            )));
        }
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
        let base_order = drawing
            .model_space()
            .iter()
            .map(|e| e.draw_order)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0);
        let mut entities: Vec<DbEntity> = Vec::with_capacity(geometries.len());
        for (index, geometry) in geometries.into_iter().enumerate() {
            let entity_id = drawing.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: type_key
                        .map(str::to_string)
                        .unwrap_or_else(|| class_name(&geometry).to_string()),
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
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for entity in &entities {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };
        if let Some(display) = &display {
            if let Some(entity) = entities.first() {
                drawing.restore_entity_display_state(entity.id, display);
            }
        }
        let patches: Vec<DrawingPatch> = entities
            .iter()
            .enumerate()
            .map(|(index, entity)| DrawingPatch {
                id: entity.id,
                before: None,
                after: Some(entity.clone()),
                before_state: None,
                // The captured display state applies only to the first (single)
                // entity; a batch records none.
                after_state: if index == 0 { display.clone() } else { None },
            })
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches,
                merge_key: None,
            },
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
