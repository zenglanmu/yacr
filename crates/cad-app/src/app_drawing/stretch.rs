//! STRETCH edit command (ADR 0016 decision E).
//!
//! The **rectangle** interaction is four points: the two opposite corners of a
//! **crossing window**, then a base point and a displacement point. The
//! **polygon/lasso** interaction is the free-form boundary vertices, then the
//! same base and displacement points. Both are CROSSING windows and the
//! enclosure classification is recomputed from the window and the geometry, never
//! stored in the selection. A lasso is the same containment test over a
//! free-form polygon, so there is exactly one polygon rule (`stretch_window`).
//!
//! Behaviour (Autodesk's two-way rule):
//!
//! * **fully enclosed ⇒ move** the whole entity (`transform_geometry`, exactly
//!   what MOVE does);
//! * **partially enclosed ⇒ stretch** only the vertices/endpoints inside the
//!   window. Per-vertex kinds: `LINE` (start/end), `POLYLINE`/`LWPOLYLINE` (every
//!   stored vertex; bulges kept; no width/tangent/curve-fit field is touched),
//!   `SPLINE` (stored control points; a control point is not on the curve, so
//!   this reshapes the whole curve) and `ARC`
//!   (the two stored endpoints move on their own circle; our declared endpoint
//!   rule is the arc grip's rule);
//! * `CIRCLE` cannot be stretched and moves **only when its centre** is enclosed;
//! * `TEXT`/`MTEXT` move **only when their insertion point** is enclosed (the
//!   documented MTEXT rule, declared for TEXT too);
//! * `INSERT` moves rigidly only when its insertion point is enclosed; block
//!   members are never stretched individually;
//! * `DIMENSION` definition points inside the window move and the display is
//!   re-synthesized (Autodesk documents dimensions as stretchable);
//! * `ELLIPSE`, `HATCH`, `IMAGE`, `MASK` and `TABLE` are move-only: a partial
//!   enclosure leaves them unchanged with a named diagnostic (HATCH is not
//!   documented in STRETCH, so it is not invented here; an image/mask/table has
//!   no per-vertex model in this build; an ellipse's axes cannot be re-derived
//!   from one in-window point);
//! * `POINT`/`SHAPE` are whole-entity kinds in practice: this build's pick box
//!   for each is the anchor point itself, so the enclosure classifier can only
//!   return `Full` (the anchor is inside one window ⇒ the entity moves) or
//!   `Outside`; neither ever reaches a partial branch, and no per-vertex rule is
//!   invented for them.
//!
//! `MESH`, `OPAQUE`, `COMPOUND` and `MULTILEADER` still fail the whole command: a
//! 3D face has no modeled 2D vertex rule, proxy geometry has no semantic form at
//! all, a compound has no single owner-level edit, and MULTILEADER's
//! whole-command refusal is the contract this build pins in
//! `tests/drawing/multileader.rs` (its per-point definition *is* modeled, so the
//! per-vertex rule is a feasible follow-up, not a missing model). An in-window
//! entity that cannot be handled fails the whole command and writes nothing; one
//! transaction / one undo step.
//!
//! **UNVERIFIED**: Autodesk documents only that STRETCH does not modify
//! width/tangency/curve-fit information and does not document a bulge when a
//! bulge-segment endpoint is dragged; our declared rule keeps the stored bulge
//! value unchanged (the tests pin it numerically).

use super::*;

use super::stretch_vertices::{moved_geometry, stretched_geometry};
use super::stretch_window::{classify_enclosure, Enclosure, StretchWindow};

use crate::select_box::Rect;

/// The planned action for one in-window entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StretchAction {
    /// Translate the whole entity by the displacement: a fully enclosed entity, or a
    /// partially enclosed `INSERT` whose insertion point is inside the window (a
    /// block reference moves rigidly, see the module doc).
    Move,
    /// Partially enclosed `LINE`/`POLYLINE`/`SPLINE`/`ARC`/`DIMENSION`: move only
    /// the in-window vertices, control points, endpoints or definition points.
    Stretch,
    /// Partially enclosed, not stretchable: unchanged plus the named diagnostic
    /// (the stable `drawing.stretch.<reason>` suffix).
    Unchanged(&'static str),
}

impl Application {
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// The window is either the two canvas-logical corners (`rect_logical`) or
    /// the canvas-logical free-form polygon/lasso vertex list
    /// (`polygon_logical`), mapped through the active viewport's orthographic
    /// plan view exactly like `BoxSelect`; the effective translation is
    /// `delta − base`. Every input is validated and the enclosure classification
    /// is resolved for the whole drawing **before** any write, so a refusal
    /// changes nothing (the database transaction is all-or-nothing and no history
    /// is recorded).
    ///
    /// Refusals (nothing written): a non-finite window corner/vertex / base /
    /// displacement point; a degenerate window (zero width/height or a polygon
    /// with fewer than three vertices or zero area); a zero displacement
    /// (`delta == base`, nothing to do); a non-model space or non-plan view; a
    /// visible tree the pick builder cannot enumerate (opaque geometry,
    /// cyclic/depth-limited INSERTs, missing block members); and any in-window
    /// entity that cannot be handled (TEXT, or a kind with no per-vertex edit and
    /// no declared move-only rule).
    pub(crate) fn stretch_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (rect_logical, polygon_logical) = match &command.payload {
            CommandPayload::Stretch { rect_logical, .. } => (Some(*rect_logical), None),
            CommandPayload::StretchPolygon(polygon) => {
                (None, Some(polygon.polygon_logical.as_slice()))
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "StretchEntities needs a Stretch { rect_logical, base, delta } or a \
                     StretchPolygon { polygon_logical, base, delta } payload"
                        .into(),
                ));
            }
        };
        let (base, delta) = match &command.payload {
            CommandPayload::Stretch { base, delta, .. } => (*base, *delta),
            CommandPayload::StretchPolygon(polygon) => (polygon.base, polygon.delta),
            _ => unreachable!("the payload was matched above"),
        };
        if let Some(rect_logical) = rect_logical {
            if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
                return Err(CadError::InvalidInput(
                    "STRETCH window corners must be finite".into(),
                ));
            }
        }
        if polygon_logical
            .is_some_and(|polygon| polygon.iter().flatten().any(|value| !value.is_finite()))
        {
            return Err(CadError::InvalidInput(
                "STRETCH crossing polygon vertices must be finite".into(),
            ));
        }
        if !is_finite_point(base) || !is_finite_point(delta) {
            return Err(CadError::InvalidInput(
                "STRETCH base and displacement points must be finite".into(),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != document_id {
            return Err(CadError::StaleResult);
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "STRETCH currently supports model space only".into(),
            ));
        }
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let size = viewport.logical_size;
        if !size.iter().all(|value| value.is_finite() && *value > 0.0) {
            return Err(CadError::InvalidInput(
                "STRETCH viewport size must be finite and positive".into(),
            ));
        }
        let tol = stretch_tolerance();
        // The plan mapping is the same one picking/box-selection use. A
        // perspective/3D view has no plan mapping: 3D STRETCH is NOT IMPLEMENTED.
        let window = if let Some(rect_logical) = rect_logical {
            let first = viewport
                .screen_to_world(rect_logical[0], size)
                .ok_or_else(|| {
                    CadError::Unsupported("STRETCH requires the orthographic plan view".into())
                })?;
            let second = viewport
                .screen_to_world(rect_logical[1], size)
                .ok_or_else(|| {
                    CadError::Unsupported("STRETCH requires the orthographic plan view".into())
                })?;
            let rect = Rect::from_corners(first, second);
            if !rect.width().is_finite()
                || !rect.height().is_finite()
                || rect.width() <= tol
                || rect.height() <= tol
            {
                return Err(CadError::InvalidInput(
                    "STRETCH window must be non-degenerate (zero width or height)".into(),
                ));
            }
            StretchWindow::Rect(rect)
        } else {
            let polygon_logical = polygon_logical.expect("a payload variant was matched");
            let mut vertices: Vec<(f64, f64)> = Vec::with_capacity(polygon_logical.len());
            for logical in polygon_logical {
                let world = viewport.screen_to_world(*logical, size).ok_or_else(|| {
                    CadError::Unsupported("STRETCH requires the orthographic plan view".into())
                })?;
                vertices.push((world.x, world.y));
            }
            polygon_window(vertices)?
        };
        let translation = Point3 {
            x: delta.x - base.x,
            y: delta.y - base.y,
            z: delta.z - base.z,
        };
        let displacement_length = (translation.x * translation.x
            + translation.y * translation.y
            + translation.z * translation.z)
            .sqrt();
        if !is_finite_point(translation)
            || !displacement_length.is_finite()
            || displacement_length <= tol
        {
            return Err(CadError::InvalidInput(
                "STRETCH displacement must be non-zero and finite (nothing to do)".into(),
            ));
        }

        self.stretch_windows(session, command, vec![window], translation)
    }

    /// The shared STRETCH planner over one or more crossing windows.
    ///
    /// `stretch_entities` calls it with one window (rectangle or polygon);
    /// `MSTRETCH` calls it with the typed rectangle list. An entity is fully
    /// enclosed when one window encloses it, is partial when any window hits it,
    /// and its vertices/endpoints move when they lie inside **any** window. The
    /// geometry edit itself is the existing [`stretched_geometry`] /
    /// [`moved_geometry`] planner, never a second copy.
    pub(super) fn stretch_windows(
        &mut self,
        session: &SessionState,
        command: &Command,
        windows: Vec<StretchWindow>,
        translation: Point3,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // The pick builder silently skips cycles/depth limits; preflight the same
        // visible tree `select_box` refuses on before trusting its catalogue.
        let scope = drawing.root_layer_scope();
        for entity in drawing.model_space() {
            crate::select_all::validate_branch(drawing, session, entity, scope, &mut Vec::new())
                .map_err(stretch_tree_refusal)?;
        }

        // Group the visible pick items by their top-level model-space owner: a
        // bare entity owns itself, an INSERT owns its expanded children (whose
        // `InstancePath` starts at the insert entity).
        let items = drawing_pick_items(drawing, document_id);
        let mut groups: Vec<(EntityId, Vec<usize>)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let visible = crate::picking::ref_visibility(
                drawing,
                &session.layer_overrides,
                &session.hidden_entities,
                scope,
                &item.source,
            )
            .ok_or(CadError::StaleResult)?;
            if !visible {
                continue;
            }
            let owner = item
                .source
                .instance
                .0
                .first()
                .copied()
                .unwrap_or(item.source.entity);
            match groups.iter_mut().find(|(id, _)| *id == owner) {
                Some((_, indices)) => indices.push(index),
                None => groups.push((owner, vec![index])),
            }
        }

        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut plan: Vec<(EntityId, StretchAction)> = Vec::new();
        for (owner, indices) in groups {
            let owner_entity = drawing
                .entity(owner)
                .cloned()
                .ok_or(CadError::StaleResult)?;
            let owner_items: Vec<&crate::PickItem> =
                indices.iter().map(|index| &items[*index]).collect();
            let enclosure = classify_enclosure(&owner_entity.geometry, &owner_items, &windows);
            if enclosure == Enclosure::Outside {
                continue;
            }
            match enclosure {
                Enclosure::Full => plan.push((owner, StretchAction::Move)),
                Enclosure::Partial => match &owner_entity.geometry {
                    // The per-vertex stretchable kinds: LINE and LWPOLYLINE
                    // (including bulge segments, whose stored bulges are kept),
                    // plus SPLINE (stored control points), ARC (stored
                    // start/end points on the unchanged circle) and DIMENSION
                    // (its definition points).
                    SemanticGeometry::Line { .. }
                    | SemanticGeometry::Polyline { .. }
                    | SemanticGeometry::Spline { .. }
                    | SemanticGeometry::Arc { .. }
                    | SemanticGeometry::Dimension(_) => {
                        plan.push((owner, StretchAction::Stretch));
                    }
                    // CIRCLE cannot be stretched at all. It moves as a whole only
                    // when its **centre** is enclosed (AutoCAD's documented rule);
                    // a circle whose centre is outside the crossing window is left
                    // unchanged with the named reason `circle_center_outside`.
                    SemanticGeometry::Circle { center, .. } => {
                        if windows.iter().any(|window| window.contains_point(*center)) {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics
                                .push(stretch_unchanged_diagnostic(owner, "circle_center_outside"));
                        }
                    }
                    // ELLIPSE cannot be stretched: move-only when fully enclosed,
                    // unchanged here. One in-window point does not determine new
                    // axes/ratio, so no per-vertex rule is invented.
                    SemanticGeometry::Ellipse { .. } => {
                        plan.push((owner, StretchAction::Unchanged("ellipse")));
                    }
                    // TEXT/MTEXT follow the **insertion-point** rule: move the whole
                    // run iff its stored anchor (`position`) is inside the crossing
                    // window; otherwise leave it unchanged with the named reason
                    // `text_outside`. Autodesk documents this rule for MTEXT and is
                    // silent for TEXT; using the same rule for both is our declared
                    // choice (and is the only rule this flat-text domain can state
                    // honestly, since it stores an anchor and no per-glyph vertices).
                    SemanticGeometry::Text { position, .. } => {
                        if windows
                            .iter()
                            .any(|window| window.contains_point(*position))
                        {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, "text_outside"));
                        }
                    }
                    // INSERT: the reference has no per-vertex edit, so the declared
                    // rule is rigid: translate the whole reference by the
                    // displacement **iff** its insertion point (the transformed
                    // block-space origin) is inside the crossing window, otherwise
                    // leave it bit-identically unchanged and report the reason by
                    // name (`drawing.stretch.insert_outside`). Both arms are
                    // pre-existing paths — `Move` is the ordinary whole-entity
                    // `transform_geometry` translation (so the linear part of the
                    // placement is preserved bit for bit) and the diagnostic is an
                    // ordinary per-entity "unchanged" report, so nothing is staged.
                    SemanticGeometry::Insert { transform, .. } => {
                        if windows
                            .iter()
                            .any(|window| window.contains_point(insertion_point(transform)))
                        {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, "insert_outside"));
                        }
                    }
                    // A dimension's **definition points** stretch: every point
                    // inside the crossing window moves by the displacement and the
                    // derived display is re-synthesized. Autodesk documents
                    // dimensions as stretchable; see `stretched_dimension`.
                    // (Handled by the per-vertex arm above.)
                    // A hatch follows `compatibility.md`'s existing rule: a
                    // partial enclosure leaves it unchanged with exactly one
                    // diagnostic, and a full enclosure moves it as a whole
                    // (ADR 0018 E). A hatch is never stretched and never moves on a
                    // partial enclosure, so it cannot drift silently.
                    SemanticGeometry::Hatch(_) => {
                        plan.push((owner, StretchAction::Unchanged("hatch")));
                    }
                    // Raster images, masks and tables have no per-vertex edit in
                    // this build: move-only, with their own named reason. (Their
                    // display is derived, so re-deriving it from partly moved
                    // children would be a fabricated edit.)
                    SemanticGeometry::Image { .. } => {
                        plan.push((owner, StretchAction::Unchanged("image")));
                    }
                    SemanticGeometry::Mask { .. } => {
                        plan.push((owner, StretchAction::Unchanged("mask")));
                    }
                    SemanticGeometry::Table(_) => {
                        plan.push((owner, StretchAction::Unchanged("table")));
                    }
                    // MULTILEADER still fails the whole command: the per-point
                    // definition (leader vertices, landing points, text location)
                    // *is* present in the domain variant, so this is a **contract**
                    // decision rather than a missing model — the refused-whole
                    // behaviour is pinned by
                    // `tests/drawing/multileader.rs::stretch_refuses_a_partially_enclosed_multileader_as_a_whole_command`,
                    // which lives outside this lane's file fence. Implementing the
                    // per-point rule is a separate, test-visible change.
                    SemanticGeometry::MultiLeader(_) => {
                        return Err(CadError::Unsupported(
                            "STRETCH cannot handle MULTILEADER: its display combines leader \
                             paths, arrowheads, a dogleg and text, and the whole-command refusal \
                             is the contract this build pins; the whole command is refused and \
                             nothing is written"
                                .into(),
                        ));
                    }
                    // MESH (a 3D face), OPAQUE (no semantic form) and COMPOUND (no
                    // single owner-level edit): no per-vertex rule exists, so the
                    // whole command fails rather than guessing (refusal
                    // granularity).
                    other => {
                        return Err(CadError::Unsupported(format!(
                            "STRETCH cannot handle {} inside the window",
                            geometry_kind(other)
                        )));
                    }
                },
                Enclosure::Outside => unreachable!("Outside was filtered above"),
            }
        }

        // Build the updates (pure). A fully enclosed entity that cannot be
        // translated fails the whole command (never a partial success).
        let mut before_states: Vec<DbEntity> = Vec::new();
        let mut updates: Vec<DbEntity> = Vec::new();
        for (owner, action) in plan {
            let before = drawing
                .entity(owner)
                .cloned()
                .ok_or(CadError::StaleResult)?;
            match action {
                StretchAction::Move => {
                    let geometry = moved_geometry(&before.geometry, translation).map_err(
                        |error| match error {
                            CadError::Unsupported(reason) => CadError::Unsupported(format!(
                                "STRETCH cannot move {}: {reason}",
                                geometry_kind(&before.geometry)
                            )),
                            other => other,
                        },
                    )?;
                    let mut after = before.clone();
                    after.geometry = geometry;
                    before_states.push(before);
                    updates.push(after);
                }
                StretchAction::Stretch => {
                    // One containment rule for both window kinds, and one
                    // displacement: the per-vertex planner is shared.
                    let inside =
                        |point: Point3| windows.iter().any(|window| window.contains_point(point));
                    let shift = |point: Point3| Point3 {
                        x: point.x + translation.x,
                        y: point.y + translation.y,
                        z: point.z + translation.z,
                    };
                    match stretched_geometry(&before.geometry, inside, shift) {
                        Ok(geometry) => {
                            let mut after = before.clone();
                            after.geometry = geometry;
                            before_states.push(before);
                            updates.push(after);
                        }
                        // The window crosses the entity but nothing moved: the
                        // geometry is unchanged plus an explicit diagnostic
                        // (never silent, never a fabricated edit).
                        Err(reason) => {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, reason))
                        }
                    }
                }
                StretchAction::Unchanged(reason) => {
                    diagnostics.push(stretch_unchanged_diagnostic(owner, reason));
                }
            }
        }

        if updates.is_empty() {
            // Nothing in the window changed: 0 is a result, reported explicitly,
            // and no transaction/history is recorded.
            diagnostics.push(Diagnostic {
                object: None,
                code: "drawing.stretch".into(),
                message: "STRETCH: nothing changed in the window".into(),
            });
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
                point_report: None,
            });
        }

        let changed_count = updates.len();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("stretch entities", transaction)?;
            for after in &updates {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        // Read each committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .map(|before| {
                let after = drawing.entity(before.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "stretched entity {} vanished after commit",
                        before.id.0
                    ))
                })?;
                Ok(drawing_patch(before.id, Some(before), Some(after)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        let changed_ids: Vec<ObjectId> = updates.iter().map(|after| ObjectId(after.id.0)).collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "stretch entities".into(),
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

        diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.stretch".into(),
            message: format!("STRETCH: {changed_count} entity(ies) changed (crossing window)"),
        });
        Ok(CommandOutcome {
            objects: changed_ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The insertion point of an `INSERT`: the block-space origin mapped through the
/// placement (the translation column). Mapping the origin stays honest for any
/// affine placement; a non-finite placement lies in no window.
fn insertion_point(transform: &Transform3) -> Point3 {
    transform.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

/// The explicit "unchanged" diagnostic for an in-window entity that is not
/// stretched, never a silent drop. `reason` is the stable suffix of
/// `drawing.stretch.<reason>`; see the match arms below for the full set.
fn stretch_unchanged_diagnostic(entity: EntityId, reason: &str) -> Diagnostic {
    let move_only = format!(
        "STRETCH: entity {} crosses the window but cannot be stretched (move-only kind); \
         left unchanged",
        entity.0
    );
    let message = match reason {
        "no_vertex" => format!(
            "STRETCH: entity {} crosses the window but has no vertex inside; left unchanged",
            entity.0
        ),
        "spline_inconsistent" => format!(
            "STRETCH: entity {} crosses the window but its stored spline definition is \
             inconsistent (knot or weight count does not match its control points), so it cannot \
             be edited faithfully; left unchanged",
            entity.0
        ),
        "arc_no_direction" => format!(
            "STRETCH: entity {} crosses the window but an in-window arc endpoint has no in-plane \
             direction from the arc centre (it lies on the centre or on the normal through it), so \
             no endpoint angle exists; left unchanged",
            entity.0
        ),
        "arc_degenerate_sweep" => format!(
            "STRETCH: entity {} crosses the window but moving its in-window endpoint(s) on the same \
             circle in the stored direction would give a sweep of 0 or a full turn, which is not a \
             storable arc; left unchanged",
            entity.0
        ),
        "arc_unchanged" => format!(
            "STRETCH: entity {} crosses the window but its in-window endpoint(s) keep their stored \
             angles on the arc's circle (the displacement has no tangential in-plane component); \
             left unchanged",
            entity.0
        ),
        // Our declared INSERT rule: a block reference only moves rigidly, and only
        // when its insertion point is in the window. The message names the
        // precondition so a partially enclosed reference is never silently dropped.
        "insert_outside" => format!(
            "STRETCH: block reference {} crosses the window but its insertion point (the \
             transformed block-space origin) is outside it, and STRETCH moves a block reference \
             only rigidly by its insertion point (block members are not stretched individually); \
             left unchanged",
            entity.0
        ),
        // Documented AutoCAD circle rule: a circle cannot be stretched, and it
        // only moves as a whole when its centre is in the crossing window.
        "circle_center_outside" => format!(
            "STRETCH: circle {} crosses the window but its centre is outside it; a circle cannot \
             be stretched and only moves (as a whole) when its centre is enclosed, so it is left \
             unchanged",
            entity.0
        ),
        // Our declared TEXT/MTEXT rule (Autodesk documents it for MTEXT): the run
        // moves only by its insertion point.
        "text_outside" => format!(
            "STRETCH: text {} crosses the window but its insertion point is outside it; TEXT and \
             MTEXT move only when their insertion point is enclosed, so it is left unchanged",
            entity.0
        ),
        // Move-only kinds: a partial enclosure never edits them, so they cannot
        // drift or be silently half-moved.
        "ellipse" => format!(
            "STRETCH: ellipse {} crosses the window but an ellipse cannot be stretched (one \
             in-window point does not determine new axes or a ratio); it moves only when fully \
             enclosed, so it is left unchanged",
            entity.0
        ),
        "hatch" => format!(
            "STRETCH: hatch {} crosses the window but a HATCH is move-only here (its display is \
             derived from the boundary loops and the fill); a partial enclosure never moves it, so \
             it is left unchanged",
            entity.0
        ),
        "image" => format!(
            "STRETCH: image {} crosses the window but a raster image is move-only (no per-vertex \
             model); a partial enclosure never moves it, so it is left unchanged",
            entity.0
        ),
        "mask" => format!(
            "STRETCH: mask {} crosses the window but a mask is move-only (no per-vertex model); a \
             partial enclosure never moves it, so it is left unchanged",
            entity.0
        ),
        "table" => format!(
            "STRETCH: table {} crosses the window but a TABLE is move-only (its grid and cells \
             have no per-vertex stretch rule); a partial enclosure never moves it, so it is left \
             unchanged",
            entity.0
        ),
        "dimension_unchanged" => format!(
            "STRETCH: dimension {} crosses the window but no definition point is inside it (or \
             the stretched definition cannot be synthesized); left unchanged",
            entity.0
        ),
        _ => move_only,
    };
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: format!("drawing.stretch.{reason}"),
        message,
    }
}

/// Validate a crossing polygon/lasso and wrap it as a window.
///
/// The vertex list is the free-form boundary, so a repeated closing vertex (a
/// capture that stores its first point again) is harmless for even-odd
/// containment and is kept as a zero-length edge. At least three vertices and a
/// non-zero XY area are required: a degenerate polygon encloses nothing and is
/// refused instead of silently selecting nothing (a refusal changes no state).
fn polygon_window(vertices: Vec<(f64, f64)>) -> CadResult<StretchWindow> {
    if vertices.len() < 3 {
        return Err(CadError::InvalidInput(
            "STRETCH crossing polygon needs at least three vertices".into(),
        ));
    }
    let count = vertices.len();
    let mut area = 0.0;
    let mut previous = count - 1;
    for index in 0..count {
        let (x0, y0) = vertices[previous];
        let (x1, y1) = vertices[index];
        area += x0 * y1 - x1 * y0;
        previous = index;
    }
    let tol = stretch_tolerance();
    if !area.is_finite() || area.abs() <= tol * tol {
        return Err(CadError::InvalidInput(
            "STRETCH crossing polygon must be non-degenerate (zero area)".into(),
        ));
    }
    Ok(StretchWindow::Polygon(vertices))
}

/// Relabel the shared tree-preflight refusal with the STRETCH command context.
fn stretch_tree_refusal(error: CadError) -> CadError {
    match error {
        CadError::Unsupported(_) => CadError::Unsupported(
            "STRETCH cannot enumerate the visible entity tree (opaque geometry, cyclic or \
             depth-limited INSERTs, or missing block members)"
                .into(),
        ),
        other => other,
    }
}

/// The world tolerance shared by the STRETCH degeneracy checks (window area and
/// displacement magnitude).
pub(super) fn stretch_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}
