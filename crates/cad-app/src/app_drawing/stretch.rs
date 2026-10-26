//! STRETCH edit command (ADR 0016 decision E).
//!
//! The interaction is four points: the two opposite corners of a **crossing
//! window**, then a base point and a displacement point. The window is **always
//! treated as CROSSING** (AutoCAD requires a crossing window; cpolygon/lasso
//! selection is NOT IMPLEMENTED), and the enclosure classification is
//! recomputed from the rectangle and the geometry — it is never stored in the
//! selection (single source of truth).
//!
//! Behaviour (Autodesk's two-way rule, with the per-kind refinements below):
//!
//! * **fully enclosed ⇒ move**: the whole entity is translated by the
//!   displacement (`transform_geometry`, exactly what MOVE does);
//! * **partially enclosed ⇒ stretch**: only the vertices/endpoints inside the
//!   window move; every other vertex is unchanged. The per-vertex kinds are
//!   `LINE` (start/end), `POLYLINE`/`LWPOLYLINE` (every stored vertex, bulges
//!   preserved) and `SPLINE` (every stored **control point**); an `ARC` is
//!   stretchable per endpoint under the declared rule below;
//! * `CIRCLE`/`ELLIPSE` cannot be stretched: fully enclosed ⇒ move, partially
//!   enclosed ⇒ unchanged plus an explicit diagnostic; `INSERT` follows the
//!   declared insertion-point rule below (it is no longer in the move-only list);
//! * `TEXT` fails the **whole** command. This is a deliberate, tested choice
//!   that is **unchanged** by ADR 0019 decision B: text now accepts a
//!   similarity transform, but STRETCH's per-vertex semantics have no meaning for
//!   a text run (Autodesk does not document its behaviour), so the command still
//!   refuses it rather than silently resolving it into a translation. The reason
//!   is the restricted similarity-only transform surface, **not** "text cannot be
//!   transformed";
//! * an entity that crosses the window with no vertex inside is unchanged plus
//!   an explicit diagnostic (never silent).
//!
//! **SPLINE partial stretch.** A spline's addressable vertices are its stored
//! **control points** (`SemanticGeometry::Spline::control_points`), and a control
//! point is generally **not on the curve** — only a clamped spline's first and
//! last control point are; that is the definition, not a defect. Every in-window
//! control point is translated by the displacement, every out-of-window control
//! point is copied bit-identically, and `degree`/`knots`/`weights` are copied
//! verbatim. The result is therefore a **different NURBS curve with the same
//! parameterisation**: a control point shapes the curve over a support of
//! `degree + 1` knot spans rather than at one point, so a partial stretch
//! reshapes the **whole** curve instead of "moving one piece of it". That is
//! inherent to NURBS control-point editing, not a bug, and it is the same
//! property the spline control-point grip has. Copying the vectors verbatim keeps
//! the definition indexable, so the stored spline still satisfies `cad-db`'s
//! invariants (`knots.len() == control_points.len() + degree + 1`,
//! `weights.len() == control_points.len()`, non-decreasing knots) — the contract
//! test asserts the committed entity through `cad_db::validate_geometry`. A spline
//! whose stored definition is **already** inconsistent (either relation violated:
//! nothing can index it, so nothing can edit it faithfully) is left unchanged with
//! `drawing.stretch.spline_inconsistent` rather than writing vectors no consumer
//! can evaluate. All of that is **our choice**: Autodesk does not document
//! STRETCH on a spline.
//!
//! A **fully enclosed** spline is translated as a whole like every other kind,
//! through `cad-db`'s `transform_geometry`. That arm used to be missing: `cad-db`
//! refused to bake a spline at all, so this command kept a private
//! translation-only control-point shift for the fully enclosed case. `cad-db` now
//! bakes a spline under **any** finite affine transform (a NURBS is affinely
//! invariant: mapping the control points with `degree`/`knots`/`weights` copied
//! verbatim is exactly the affine image of the curve), so the app-side exception
//! is **gone** and a fully enclosed STRETCH and a MOVE cannot disagree about a
//! spline. Only a stored definition that is already unindexable is refused, by
//! the same `transform_geometry`/`validate_geometry` rules every other kind obeys.
//!
//! **ARC partial stretch (our declared rule).** An arc has exactly two addressable
//! vertices: its stored start point and end point,
//! `centre + radius·(cos θ·ax + sin θ·ay)` at `θ = start` and `θ = start + sweep`,
//! with `(ax, ay)` from `cad_geometry::arbitrary_axis` — the frame the tessellator
//! and the arc endpoint grips measure angles in. For a partially enclosed arc,
//! **each endpoint whose point lies inside the window moves**: its new angle is
//! the in-plane direction of `point + delta` seen from the centre, so the arc
//! stays on the **same circle** (`center`/`radius`/`normal` are copied
//! bit-identically) and the out-of-plane part of the displacement is discarded
//! (an arc endpoint cannot leave its own plane). The endpoint outside the window
//! keeps its stored angle, and `sweep` is recomputed **in the stored direction**
//! (`sweep < 0`, the clockwise/reflected arc, stays clockwise). Three outcomes are
//! refused per entity with a named diagnostic and nothing written — never clamped,
//! never a partial write: an in-window endpoint whose translated point has **no
//! in-plane direction** (it sits on the centre or on the normal through it) is
//! `drawing.stretch.arc_no_direction`; a recomputed sweep of ~0 or ~2π (`|sweep| <=
//! 1e-9` or `2π − |sweep| <= 1e-9`, the same slack the tessellator and the arc grip
//! model use) is `drawing.stretch.arc_degenerate_sweep`; and a rule that leaves
//! both stored angles untouched (a displacement with no tangential in-plane
//! component, e.g. purely radial or purely out-of-plane) is
//! `drawing.stretch.arc_unchanged`. An arc the window merely crosses, with
//! **neither** endpoint inside, is unchanged with `drawing.stretch.no_vertex`.
//! This endpoint rule is **our choice, not an AutoCAD assertion**; it is
//! deliberately the same rule the arc endpoint grip applies, so the grip and the
//! command never disagree about what "move an arc endpoint" means.
//!
//! **INSERT partial stretch (our declared rule).** A block reference has exactly one
//! addressable "vertex" through this path: its **insertion point**, i.e. the
//! block-space origin `(0, 0, 0)` mapped through the instance placement
//! (`Transform3::apply_point`), which is the placement's translation column. A
//! partially enclosed INSERT therefore translates the **whole reference rigidly** —
//! one `transform_geometry` translation of the placement, the same call a fully
//! enclosed move makes, so the linear part (rotation / uniform or non-uniform scale
//! / mirror columns) is copied **bit-identically** and only the translation column
//! gains the displacement — **iff** that insertion point lies inside the crossing
//! window. Otherwise the reference is left **bit-identically** unchanged and the
//! reason is named: `drawing.stretch.insert_outside` (the window overlaps the
//! reference's displayed geometry, e.g. its members' conservative bounds, but not
//! its insertion point). A fully enclosed INSERT keeps the ordinary whole-move path,
//! and a reference outside the window is not touched at all (no staging, no write).
//!
//! Honest limits of that rule (all deliberate; none of them is "block support"):
//!
//! * block **members are not stretched individually** — an instance path into a
//!   block definition is not addressable through this path (the plan only ever
//!   writes top-level model-space owners, and there is no per-member vertex edit);
//! * a **rotated/scaled/mirrored** reference moves **rigidly** by the displacement
//!   and is **not** re-fitted to the window: the reference keeps its rotation and
//!   scale exactly, so "the part of the block inside the window stretches" does not
//!   happen;
//! * this is **not** AutoCAD's full block-reference STRETCH semantics. Autodesk's
//!   block-specific handling (definition-level edits, nested member stretching,
//!   attribute reflow) is **not modelled here and not claimed**; the rule above is
//!   our own, chosen so that a partial enclosure is never silent and never guesses.
//!
//! The insertion point of a non-finite placement is not special-cased: a
//! non-finite point is simply not inside any window, so such a reference reports
//! `insert_outside` and is left untouched.
//!
//! The per-vertex edits are built here as new geometry and staged as

//! `update_entity`, exactly like TRIM's keep branch; `cad-db` re-validates at
//! commit. The whole command is one transaction with one undo step, and an
//! in-window entity that cannot be handled fails the whole command and writes
//! nothing (the `copy_entities` whole-command-refusal precedent).
//!
//! **UNVERIFIED**: Autodesk documents only that STRETCH does not modify
//! width/tangency/curve-fit information and does not document what happens to a
//! bulge when a bulge-segment endpoint is dragged, so our declared rule is to
//! **keep the stored bulge value unchanged** (move the vertex only). The
//! contract tests pin the stored bulge numerically.

use super::*;

use cad_spatial::pick_world_bounds;

use crate::select_box::{geometry_hits_rect, Rect};

/// The enclosure classification of one entity against the crossing window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enclosure {
    /// The geometry neither intersects nor is contained by the window.
    Outside,
    /// The geometry hits the window but is not fully enclosed.
    Partial,
    /// The geometry is fully enclosed by the window.
    Full,
}

/// The planned action for one in-window entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StretchAction {
    /// Translate the whole entity by the displacement: a fully enclosed entity, or a
    /// partially enclosed `INSERT` whose insertion point is inside the window (a
    /// block reference moves rigidly, see the module doc).
    Move,
    /// Partially enclosed `LINE`/`POLYLINE`/`SPLINE`/`ARC`: move only the
    /// in-window vertices, control points or arc endpoints.
    Stretch,
    /// Partially enclosed, not stretchable: unchanged plus a diagnostic.
    Unchanged,
}

impl Application {
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// The window corners are canvas-logical (`rect_logical`) and are mapped
    /// through the active viewport's orthographic plan view, exactly like
    /// `BoxSelect`; the effective translation is `delta − base`. Every input is
    /// validated and the enclosure classification is resolved for the whole
    /// drawing **before** any write, so a refusal changes nothing (the database
    /// transaction is all-or-nothing and no history is recorded).
    ///
    /// Refusals (nothing written): a non-finite window corner / base /
    /// displacement point; a degenerate window (zero width or height); a zero
    /// displacement (`delta == base`, nothing to do); a non-model space or
    /// non-plan view; a visible tree the pick builder cannot enumerate (opaque
    /// geometry, cyclic/depth-limited INSERTs, missing block members); and any
    /// in-window entity that cannot be handled (TEXT, or a kind with no
    /// per-vertex edit and no declared move-only rule).
    pub(crate) fn stretch_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Stretch {
            rect_logical,
            base,
            delta,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "StretchEntities needs a Stretch { rect_logical, base, delta } payload".into(),
            ));
        };
        let (rect_logical, base, delta) = (*rect_logical, *base, *delta);
        if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
            return Err(CadError::InvalidInput(
                "STRETCH window corners must be finite".into(),
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
        // The plan mapping is the same one picking/box-selection use. A
        // perspective/3D view has no plan mapping: 3D STRETCH is NOT IMPLEMENTED.
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
        let tol = stretch_tolerance();
        if !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() <= tol
            || rect.height() <= tol
        {
            return Err(CadError::InvalidInput(
                "STRETCH window must be non-degenerate (zero width or height)".into(),
            ));
        }
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
            let enclosure = classify_enclosure(&owner_entity.geometry, &owner_items, rect);
            if enclosure == Enclosure::Outside {
                continue;
            }
            // TEXT: the whole command still fails (ADR 0019 decision B keeps this
            // behaviour). The reason is NOT "text cannot be transformed" any more
            // — text accepts a similarity — but that STRETCH's per-vertex
            // semantics have no meaning for a text run, and Autodesk does not
            // document STRETCH's behaviour on text, so resolving it into a
            // translation would be a guess.
            if matches!(&owner_entity.geometry, SemanticGeometry::Text { .. }) {
                return Err(CadError::Unsupported(
                    "STRETCH cannot handle TEXT: its similarity-only transform (ADR 0019 B) has \
                     no per-vertex stretch semantics, and Autodesk does not document STRETCH's \
                     text behaviour, so the whole command is refused rather than guessed"
                        .into(),
                ));
            }
            // MULTILEADER: the whole command still fails, exactly as the pre-P7b
            // `Compound` did (both partial and full enclosure). It is refused
            // explicitly by type now that the variant has its own transform arm;
            // no `Unchanged` arm is added, so STRETCH does not fork by content
            // type (ADR 0019 decision G).
            if matches!(&owner_entity.geometry, SemanticGeometry::MultiLeader(_)) {
                return Err(CadError::Unsupported(
                    "STRETCH cannot handle MULTILEADER: its display combines leader paths, \
                     arrowheads, a dogleg and text, so a per-vertex stretch has no defined \
                     semantics; the whole command is refused (the pre-P7b compound behaviour is \
                     preserved)"
                        .into(),
                ));
            }
            match enclosure {
                Enclosure::Full => plan.push((owner, StretchAction::Move)),
                Enclosure::Partial => match &owner_entity.geometry {
                    // The per-vertex stretchable kinds: LINE and LWPOLYLINE
                    // (including bulge segments, whose stored bulges are kept),
                    // plus SPLINE (stored control points) and ARC (stored
                    // start/end points on the unchanged circle).
                    SemanticGeometry::Line { .. }
                    | SemanticGeometry::Polyline { .. }
                    | SemanticGeometry::Spline { .. }
                    | SemanticGeometry::Arc { .. } => {
                        plan.push((owner, StretchAction::Stretch));
                    }
                    // CIRCLE/ELLIPSE cannot be stretched. Both are move-only when
                    // fully enclosed and unchanged here.
                    SemanticGeometry::Circle { .. } | SemanticGeometry::Ellipse { .. } => {
                        plan.push((owner, StretchAction::Unchanged));
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
                        if rect.contains_point(insertion_point(transform)) {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, "insert_outside"));
                        }
                    }
                    // A dimension is not stretchable: a partial enclosure leaves
                    // it unchanged (like ARC) rather than failing the command,
                    // and a full enclosure moves it as a whole (the
                    // `Enclosure::Full` arm above; ADR 0017 A.8).
                    SemanticGeometry::Dimension(_) => {
                        diagnostics.push(stretch_unchanged_diagnostic(owner, "dimension"));
                        plan.push((owner, StretchAction::Unchanged));
                    }
                    // A hatch follows `compatibility.md`'s existing rule: a
                    // partial enclosure leaves it unchanged with a diagnostic,
                    // a full enclosure moves it as a whole (ADR 0018 E).
                    SemanticGeometry::Hatch(_) => {
                        diagnostics.push(stretch_unchanged_diagnostic(owner, "hatch"));
                        plan.push((owner, StretchAction::Unchanged));
                    }
                    // A kind with no per-vertex edit and no declared move-only
                    // rule fails the whole command (refusal granularity).
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
                    match stretched_geometry(&before.geometry, rect, translation) {
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
                StretchAction::Unchanged => {
                    diagnostics.push(stretch_unchanged_diagnostic(owner, "not_stretchable"));
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

/// The insertion point of an `INSERT`: the block-space origin `(0, 0, 0)` mapped
/// through the instance placement, i.e. the placement's translation column.
///
/// This is the **one** addressable vertex a block reference has on this path (see
/// the module doc): a partial enclosure moves the whole reference rigidly iff this
/// point is inside the crossing window. Mapping the origin (rather than reading the
/// column directly) keeps the definition honest for any affine placement, including a
/// reflected or sheared one, and a non-finite placement simply yields a non-finite
/// point that is inside no window.
fn insertion_point(transform: &Transform3) -> Point3 {
    transform.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

/// Classify one entity's geometry against the crossing window.
///
/// The "fully enclosed" rule is **two-way** (ADR 0016 decision E): for a straight
/// `LINE` and for a LWPOLYLINE whose segments are all straight, every vertex must
/// be inside the window (`Rect::contains_point`); for `CIRCLE`/`ELLIPSE`/`INSERT`,
/// for a polyline with bulge segments, and for `SPLINE`/`ARC` (and every other
/// kind), the **conservative world AABB** must be fully inside
/// (`Rect::contains_aabb`) — a bulge arc or a spline can bow out of the window
/// while all its vertices are inside, and this boundary case is our declared
/// choice. The classification is deliberately independent of the per-kind edit:
/// `SPLINE`/`ARC` are classified here by bounds and then **stretched** per vertex
/// or endpoint when the answer is `Partial`, which is why the window may contain
/// every control point / both endpoints and still produce a per-vertex edit.
/// "Partial" is then a hit on the window (the shared crossing hit test) that is
/// not fully enclosed.
fn classify_enclosure(
    geometry: &SemanticGeometry,
    items: &[&crate::PickItem],
    rect: Rect,
) -> Enclosure {
    let full = match geometry {
        SemanticGeometry::Line { start, end } => {
            rect.contains_point(*start) && rect.contains_point(*end)
        }
        SemanticGeometry::Polyline { points, bulges, .. }
            if !points.is_empty() && bulges.iter().all(|bulge| bulge.abs() <= 1e-12) =>
        {
            points.iter().all(|point| rect.contains_point(*point))
        }
        // CIRCLE/ELLIPSE/INSERT, bulge polylines and every other kind use the
        // conservative bounds fully-inside path.
        _ => match union_bounds(items) {
            Some((min, max)) => rect.contains_aabb(min, max),
            // No computable box: never fabricate an inclusion.
            None => return Enclosure::Outside,
        },
    };
    if full {
        return Enclosure::Full;
    }
    let hits = items
        .iter()
        .any(|item| geometry_hits_rect(&item.geometry, &item.transform, rect));
    if hits {
        Enclosure::Partial
    } else {
        Enclosure::Outside
    }
}

/// The union of the conservative world AABBs of the visible pick items owned by
/// one top-level entity, or `None` when no item contributes a computable box.
fn union_bounds(items: &[&crate::PickItem]) -> Option<(Point3, Point3)> {
    let mut acc: Option<(Point3, Point3)> = None;
    for item in items {
        let Some((min, max)) = pick_world_bounds(&item.geometry, &item.transform) else {
            continue;
        };
        acc = Some(match acc {
            Some((acc_min, acc_max)) => (
                Point3 {
                    x: acc_min.x.min(min.x),
                    y: acc_min.y.min(min.y),
                    z: acc_min.z.min(min.z),
                },
                Point3 {
                    x: acc_max.x.max(max.x),
                    y: acc_max.y.max(max.y),
                    z: acc_max.z.max(max.z),
                },
            ),
            None => (min, max),
        });
    }
    acc
}

/// The whole-entity translation used by `Enclosure::Full` — exactly MOVE's
/// result.
///
/// Every kind goes through `cad-db`'s [`transform_geometry`], the same call MOVE
/// makes, so a fully enclosed STRETCH and a MOVE cannot disagree. That includes
/// `SPLINE`: a NURBS is affinely invariant under a control-point map with
/// `degree`/`knots`/`weights` copied verbatim, so `transform_geometry` bakes a
/// spline under any finite affine transform and this command needs no per-kind
/// exception (the translation-only control-point shift that used to live here for
/// the fully enclosed case was removed once that arm existed).
///
/// A stored spline definition that cannot be indexed (knot or weight count not
/// matching the control points) is refused by `cad-db` (the transform's result is
/// re-validated on commit) rather than staged: a fully enclosed entity that
/// cannot be written fails the whole command, which is the `Move` arm's rule.
fn moved_geometry(geometry: &SemanticGeometry, translation: Point3) -> CadResult<SemanticGeometry> {
    transform_geometry(geometry, &Transform3::translation(translation))
}

/// Build the per-vertex stretched geometry for a `LINE`, `POLYLINE`, `SPLINE` or
/// `ARC`, moving only the vertices inside the window by `delta`.
///
/// `Ok(geometry)` means the entity moved (the geometry always differs from the
/// stored one in at least one bit-identical vertex / angle); `Err(reason)` means
/// nothing moved and carries the stable diagnostic reason the command reports, so
/// an in-window entity is never silent.
///
/// **Bulge preservation is UNVERIFIED**: Autodesk documents only that STRETCH
/// does not modify width/tangency/curve-fit information and does not document what
/// happens to a bulge when a bulge-segment endpoint is dragged, so our declared
/// rule is to keep the stored bulge value unchanged (move the vertex only). The
/// contract tests pin the stored bulge numerically.
///
/// **SPLINE**: the moved vertices are the stored control points (see the module
/// doc: a control point is not a curve point, so a partial stretch reshapes the
/// curve globally), and `degree`/`knots`/`weights` are copied verbatim so the
/// definition stays indexable. A stored definition that is **already**
/// inconsistent is refused by name (`spline_inconsistent`) rather than staged for
/// the database to reject.
///
/// **ARC**: see the module doc for the declared endpoint rule. The helpers that
/// express the angle convention (`arc_plane_axes`, `arc_point`, `in_plane_angle`,
/// `sweep_in_direction`) are local to this module for the same reason
/// `select_box`/`trim_boundary` keep their own: `cad-app`'s grip helpers are
/// module-private, and this command must not reach into the grip model. They
/// evaluate the tessellator's expression term for term, and the degeneracy slack is
/// imported from the grip model ([`crate::grips::ARC_SWEEP_EPSILON`]) so the two
/// cannot disagree about which sweeps are unstorable.
fn stretched_geometry(
    geometry: &SemanticGeometry,
    rect: Rect,
    delta: Point3,
) -> Result<SemanticGeometry, &'static str> {
    let shift = |point: Point3| Point3 {
        x: point.x + delta.x,
        y: point.y + delta.y,
        z: point.z + delta.z,
    };
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let new_start = if rect.contains_point(*start) {
                shift(*start)
            } else {
                *start
            };
            let new_end = if rect.contains_point(*end) {
                shift(*end)
            } else {
                *end
            };
            if new_start == *start && new_end == *end {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Line {
                start: new_start,
                end: new_end,
            })
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            let new_points: Vec<Point3> = points
                .iter()
                .map(|point| {
                    if rect.contains_point(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *points {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Polyline {
                points: new_points,
                // UNVERIFIED: the stored bulge values are preserved as-is.
                bulges: bulges.clone(),
                closed: *closed,
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            // A stored definition that is not indexable cannot be edited
            // faithfully: refuse by name instead of staging vectors that no
            // consumer (`cad-geometry::NurbsCurve`, the control-point grips) can
            // evaluate. This is the same relation `cad-db` enforces on write, with
            // the same two empty-vector shorthands (`knots: []` = clamped uniform,
            // `weights: []` = all ones) exempt in their own respect only.
            let expected_knots = control_points.len() + *degree as usize + 1;
            let weights_indexable = weights.is_empty() || weights.len() == control_points.len();
            let knots_indexable = knots.is_empty() || knots.len() == expected_knots;
            if !weights_indexable || !knots_indexable {
                return Err("spline_inconsistent");
            }
            let new_points: Vec<Point3> = control_points
                .iter()
                .map(|point| {
                    if rect.contains_point(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *control_points {
                return Err("no_vertex");
            }
            // `degree`, `knots` and `weights` are copied verbatim: the moved
            // points are a different NURBS with the same parameterisation, so the
            // structural invariants above still hold.
            Ok(SemanticGeometry::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: new_points,
                weights: weights.clone(),
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let axes = arc_plane_axes(*normal);
            let end_angle = start + sweep;
            let start_point = arc_point(*center, *radius, axes, *start);
            let end_point = arc_point(*center, *radius, axes, end_angle);
            let start_inside = rect.contains_point(start_point);
            let end_inside = rect.contains_point(end_point);
            if !start_inside && !end_inside {
                return Err("no_vertex");
            }
            // Each in-window endpoint moves to the in-plane direction of its
            // translated position; the other one keeps its stored angle.
            let mut new_start = *start;
            let mut new_end = end_angle;
            if start_inside {
                new_start =
                    in_plane_angle(*center, axes, shift(start_point)).ok_or("arc_no_direction")?;
            }
            if end_inside {
                new_end =
                    in_plane_angle(*center, axes, shift(end_point)).ok_or("arc_no_direction")?;
            }
            let direction = if *sweep < 0.0 { -1.0 } else { 1.0 };
            let new_sweep = sweep_in_direction(new_start, new_end, direction);
            let extent = new_sweep.abs();
            if !new_sweep.is_finite()
                || extent <= crate::grips::ARC_SWEEP_EPSILON
                || std::f64::consts::TAU - extent <= crate::grips::ARC_SWEEP_EPSILON
            {
                return Err("arc_degenerate_sweep");
            }
            if new_start == *start && new_sweep == *sweep {
                // The in-plane rule left the arc identical: the displacement has
                // no tangential component in the arc's own plane (purely radial or
                // purely out-of-plane), so "the endpoints move" is a no-op here.
                return Err("arc_unchanged");
            }
            Ok(SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: new_start,
                sweep: new_sweep,
            })
        }
        // The plan only ever routes the four kinds above into `Stretch`; anything
        // else here would be a planning defect, reported rather than guessed.
        _ => Err("not_stretchable"),
    }
}

/// The explicit "unchanged" diagnostic for an in-window entity that is not
/// stretched, never a silent drop.
///
/// `reason` is the stable suffix of the diagnostic code
/// (`drawing.stretch.<reason>`); the ones in use are `no_vertex` (a per-vertex
/// kind with no vertex inside), `not_stretchable` (a move-only kind),
/// `dimension`/`hatch` (a kind unchanged by its own documented rule),
/// `insert_outside` (a block reference whose insertion point is outside the
/// window, so its rigid move does not apply),
/// `spline_inconsistent` (a spline definition that cannot be indexed),
/// `arc_no_direction`, `arc_degenerate_sweep` and `arc_unchanged` (the three ARC
/// outcomes of the declared endpoint rule).
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
        _ => move_only,
    };
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: format!("drawing.stretch.{reason}"),
        message,
    }
}

/// The arc's plane frame `(ax, ay)`: the AutoCAD arbitrary-axis frame, i.e. the
/// same axes the tessellator, the representation provider and the arc endpoint
/// grips measure `start`/`sweep` in. Sharing the frame is what makes an angle here
/// denote the point the display draws for the same stored values.
///
/// Duplicating the two-line frame locally (as `select_box`/`trim_boundary` also
/// do) is deliberate: the grip helpers are module-private and STRETCH must not
/// reach into the grip model.
fn arc_plane_axes(normal: Point3) -> (Point3, Point3) {
    let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
    (ax, ay)
}

/// One point on the arc: `centre + radius·(cos θ·ax + sin θ·ay)`, the
/// tessellator's expression term for term.
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    cad_geometry::add(
        center,
        cad_geometry::add(
            cad_geometry::scale(axes.0, angle.cos() * radius),
            cad_geometry::scale(axes.1, angle.sin() * radius),
        ),
    )
}

/// The in-plane angle of `to` as seen from `center`, or `None` when it has no
/// in-plane direction at all (it sits on the centre, on the normal through it, or
/// the projection is non-finite). The out-of-plane offset is discarded by
/// construction, so stretching an arc endpoint can never tilt the arc out of its
/// own plane.
fn in_plane_angle(center: Point3, axes: (Point3, Point3), to: Point3) -> Option<f64> {
    let offset = cad_geometry::sub(to, center);
    let u = cad_geometry::dot(offset, axes.0);
    let v = cad_geometry::dot(offset, axes.1);
    if !u.is_finite() || !v.is_finite() || (u == 0.0 && v == 0.0) {
        return None;
    }
    Some(v.atan2(u))
}

/// Wrap an angle into `(0, 2π]` — the positive-direction extent between two
/// angles, the same normalization the importer's `normalize_sweep` performs.
fn wrap_positive(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped <= 0.0 {
        wrapped += tau;
    }
    wrapped
}

/// The signed sweep from `from` to `to` **in the stored direction**.
///
/// `direction` is `+1.0` for a counter-clockwise (positive-sweep) arc and `-1.0`
/// for a clockwise/reflected one, so a stretch never flips the arc's side. The
/// caller rejects a value of 0 or a full turn.
fn sweep_in_direction(from: f64, to: f64, direction: f64) -> f64 {
    if direction < 0.0 {
        -wrap_positive(from - to)
    } else {
        wrap_positive(to - from)
    }
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
fn stretch_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}
