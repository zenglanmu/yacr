//! STRETCH edit command (ADR 0016 decision E).
//!
//! The interaction is four points: the two opposite corners of a **crossing
//! window**, then a base point and a displacement point. The window is **always
//! treated as CROSSING** (AutoCAD requires a crossing window; cpolygon/lasso
//! selection is NOT IMPLEMENTED), and the enclosure classification is
//! recomputed from the rectangle and the geometry — it is never stored in the
//! selection (single source of truth).
//!
//! Behaviour (strictly Autodesk's two-way rule):
//!
//! * **fully enclosed ⇒ move**: the whole entity is translated by the
//!   displacement (`transform_geometry`, exactly what MOVE does);
//! * **partially enclosed ⇒ stretch**: only the vertices/endpoints inside the
//!   window move; every other vertex is unchanged;
//! * `CIRCLE`/`ELLIPSE`/`INSERT`/`ARC` cannot be stretched (Autodesk documents
//!   the first three): fully enclosed ⇒ move, partially enclosed ⇒ unchanged
//!   plus an explicit diagnostic;
//! * `TEXT` fails the **whole** command (Autodesk does not document its
//!   behaviour and the transform surface already refuses text);
//! * an entity that crosses the window with no vertex inside is unchanged plus
//!   an explicit diagnostic (never silent).
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
    /// Fully enclosed: translate the whole entity by the displacement.
    Move,
    /// Partially enclosed LINE/LWPOLYLINE: move only the in-window vertices.
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
        for entity in drawing.model_space() {
            crate::select_all::validate_branch(drawing, session, entity, &mut Vec::new())
                .map_err(stretch_tree_refusal)?;
        }

        // Group the visible pick items by their top-level model-space owner: a
        // bare entity owns itself, an INSERT owns its expanded children (whose
        // `InstancePath` starts at the insert entity).
        let items = drawing_pick_items(drawing, document_id);
        let mut groups: Vec<(EntityId, Vec<usize>)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let mut visible = true;
            for id in item
                .source
                .instance
                .0
                .iter()
                .chain(std::iter::once(&item.source.entity))
            {
                let entity = drawing.entity(*id).ok_or(CadError::StaleResult)?;
                visible &= session.layer_overrides.is_entity_visible(drawing, entity);
            }
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
            // TEXT: Autodesk does not document STRETCH's behaviour on text and the
            // transform surface already refuses it — the whole command fails
            // rather than guessing.
            if matches!(&owner_entity.geometry, SemanticGeometry::Text { .. }) {
                return Err(CadError::Unsupported(
                    "STRETCH cannot handle TEXT (behaviour is undocumented and the transform \
                     surface refuses text)"
                        .into(),
                ));
            }
            match enclosure {
                Enclosure::Full => plan.push((owner, StretchAction::Move)),
                Enclosure::Partial => match &owner_entity.geometry {
                    // The two stretchable kinds (LINE and LWPOLYLINE, including
                    // bulge segments).
                    SemanticGeometry::Line { .. } | SemanticGeometry::Polyline { .. } => {
                        plan.push((owner, StretchAction::Stretch));
                    }
                    // CIRCLE/ELLIPSE/INSERT cannot be stretched (Autodesk for the
                    // first three); ARC is transformable but not stretchable. All
                    // are move-only when fully enclosed and unchanged here.
                    SemanticGeometry::Circle { .. }
                    | SemanticGeometry::Ellipse { .. }
                    | SemanticGeometry::Arc { .. }
                    | SemanticGeometry::Insert { .. } => {
                        plan.push((owner, StretchAction::Unchanged));
                    }
                    // A dimension is not stretchable: a partial enclosure leaves
                    // it unchanged (like ARC) rather than failing the command,
                    // and a full enclosure moves it as a whole (the
                    // `Enclosure::Full` arm above; ADR 0017 A.8).
                    SemanticGeometry::Dimension(_) => {
                        diagnostics.push(stretch_unchanged_diagnostic(owner, "dimension"));
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
                    let geometry =
                        transform_geometry(&before.geometry, &Transform3::translation(translation))
                            .map_err(|error| match error {
                                CadError::Unsupported(reason) => CadError::Unsupported(format!(
                                    "STRETCH cannot move {}: {reason}",
                                    geometry_kind(&before.geometry)
                                )),
                                other => other,
                            })?;
                    let mut after = before.clone();
                    after.geometry = geometry;
                    before_states.push(before);
                    updates.push(after);
                }
                StretchAction::Stretch => {
                    let (geometry, changed) =
                        stretched_geometry(&before.geometry, rect, translation);
                    if changed {
                        let mut after = before.clone();
                        after.geometry = geometry;
                        before_states.push(before);
                        updates.push(after);
                    } else {
                        // The window crosses the entity but no vertex is inside:
                        // geometry unchanged plus an explicit diagnostic (never
                        // silent).
                        diagnostics.push(stretch_unchanged_diagnostic(owner, "no_vertex"));
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
        })
    }
}

/// Classify one entity's geometry against the crossing window.
///
/// The "fully enclosed" rule is **two-way** (ADR 0016 decision E): for a straight
/// `LINE` and for a LWPOLYLINE whose segments are all straight, every vertex must
/// be inside the window (`Rect::contains_point`); for `CIRCLE`/`ELLIPSE`/`INSERT`
/// and for a polyline with bulge segments, the **conservative world AABB** must be
/// fully inside (`Rect::contains_aabb`) — a bulge arc can bow out of the window
/// while both endpoints are inside, and this boundary case is our declared choice.
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

/// Build the per-vertex stretched geometry for a LINE or LWPOLYLINE, moving only
/// the vertices inside the window by `delta`.
///
/// **Bulge preservation is UNVERIFIED**: Autodesk documents only that STRETCH
/// does not modify width/tangency/curve-fit information and does not document what
/// happens to a bulge when a bulge-segment endpoint is dragged, so our declared
/// rule is to keep the stored bulge value unchanged (move the vertex only). The
/// contract tests pin the stored bulge numerically.
///
/// Returns `(geometry, changed)`; `changed` is `false` when no vertex lies inside
/// the window (the entity crosses it with no endpoint inside).
fn stretched_geometry(
    geometry: &SemanticGeometry,
    rect: Rect,
    delta: Point3,
) -> (SemanticGeometry, bool) {
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
            let changed = new_start != *start || new_end != *end;
            (
                SemanticGeometry::Line {
                    start: new_start,
                    end: new_end,
                },
                changed,
            )
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
            let changed = new_points != *points;
            (
                SemanticGeometry::Polyline {
                    points: new_points,
                    // UNVERIFIED: the stored bulge values are preserved as-is.
                    bulges: bulges.clone(),
                    closed: *closed,
                },
                changed,
            )
        }
        other => (other.clone(), false),
    }
}

/// The explicit "unchanged" diagnostic for an in-window entity that is not
/// stretched, never a silent drop.
fn stretch_unchanged_diagnostic(entity: EntityId, reason: &str) -> Diagnostic {
    let message = match reason {
        "no_vertex" => format!(
            "STRETCH: entity {} crosses the window but has no vertex inside; left unchanged",
            entity.0
        ),
        _ => format!(
            "STRETCH: entity {} crosses the window but cannot be stretched (move-only kind); \
             left unchanged",
            entity.0
        ),
    };
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: format!("drawing.stretch.{reason}"),
        message,
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
