//! Drawing creation and edit commands for [`Application`] (spec §4.7, F-EDIT).
//!
//! These are the real, transactional implementations behind
//! [`CommandId::CreateLine`], [`CommandId::CreateCircle`],
//! [`CommandId::MoveEntities`] and [`CommandId::TrimEntity`]. The contract is
//! pinned in `docs/drawing-edit.md` §2/§3; this module implements exactly that:
//!
//! * every command is Work-only (the command layer refuses Viewer mode);
//! * every command commits through the drawing database's validated write path
//!   and records exactly one undo step with full before/after entity patches;
//! * a failure changes no state (the database transaction is all-or-nothing)
//!   and records no history;
//! * TRIM covers the documented LINE-vs-LINE subset only and reports anything
//!   else as an explicit `Unsupported`/diagnostic without touching the drawing.
//!
//! Nothing here builds UI strings or dispatches tools; that is workstream C.

use super::*;
use cad_db::DbEntity;
use cad_history::{drawing_patch, DrawingPatch, DrawingUndoRecord};

impl Application {
    /// Dispatch the four drawing/edit commands.
    pub(crate) fn drawing_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::CreateLine => self.create_line(session, command),
            CommandId::CreateCircle => self.create_circle(session, command),
            CommandId::MoveEntities => self.move_entities(command),
            CommandId::TrimEntity => self.trim_entity(command),
            CommandId::SetActiveLayer => self.set_active_layer(session, command),
            _ => Err(CadError::InvalidInput(
                "not a drawing/edit command".to_string(),
            )),
        }
    }

    /// Set the session's active drawing layer (session state only).
    ///
    /// The layer need not exist yet: the *create* commands are the point where a
    /// missing layer becomes an explicit `InvalidInput`, so a UI may set a
    /// preference before the table is fully known. This command performs no
    /// database write and records no history.
    pub(crate) fn set_active_layer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ActiveLayer(layer) = command.payload else {
            return Err(CadError::InvalidInput(
                "SetActiveLayer needs an ActiveLayer(LayerId) payload".into(),
            ));
        };
        session.active_layer = layer;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.active_layer".into(),
                message: format!("活动图层：{}", layer.0),
            }],
            measurement: None,
        })
    }

    /// Insert a LINE into model space on the session's active layer.
    ///
    /// Accepts either `Points([start, end])` (the UI path) or a fully formed
    /// `Geometry(SemanticGeometry::Line { .. })` (the programmatic path). The
    /// geometry is validated by the database before it is committed, so a
    /// non-finite or degenerate line is refused with no state change.
    pub(crate) fn create_line(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let start = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs a start point".into())
                })?;
                let end = *points.get(1).ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs an end point".into())
                })?;
                SemanticGeometry::Line { start, end }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Line { start, end } => SemanticGeometry::Line {
                    start: *start,
                    end: *end,
                },
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateLine needs a Line geometry, got {}",
                        geometry_kind(other)
                    )))
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateLine needs a Points([start, end]) or Geometry payload".into(),
                ))
            }
        };
        self.commit_new_entity(session, command, geometry, "create line")
    }

    /// Insert a CIRCLE with radius = |edge - center| into model space.
    ///
    /// A zero (or non-finite) radius is an explicit `InvalidInput` and leaves
    /// the database revision unchanged, never a degenerate circle.
    pub(crate) fn create_circle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircle needs a Points([center, edge]) payload".into(),
            ));
        };
        let center = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs a center point".into()))?;
        let edge = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs an edge point".into()))?;
        let radius = distance(center, edge);
        if !radius.is_finite() || radius <= 0.0 {
            return Err(CadError::InvalidInput(
                "CreateCircle radius must be a positive finite distance".into(),
            ));
        }
        let geometry = SemanticGeometry::Circle {
            center,
            // The zero-radius guard above, plus the finite-radius check, means
            // the normal is only a documented plane hint; a flat circle uses the
            // work-plane +Z normal, never a fabricated tilt.
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        };
        self.commit_new_entity(session, command, geometry, "create circle")
    }

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
        let drawing = Arc::make_mut(&mut document.drawing);

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
        self.record_drawing(
            document_id,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "move entities".into(),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(change_set),
            diagnostics,
            measurement: None,
        })
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
                )))
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
            collect_boundary_segments(&entity.geometry, &mut segments, &mut closed_loops)?;
        }
        if segments.is_empty() {
            return Err(CadError::Unsupported(
                "TRIM boundary has no straight segments to cut against".into(),
            ));
        }

        let plan = match plan_trim(a, b, *pick_point, &segments, &closed_loops) {
            Ok(plan) => plan,
            Err(reason) => return Err(CadError::Unsupported(reason)),
        };

        let transaction = next_transaction_id();
        let drawing = Arc::make_mut(&mut document.drawing);
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
            TrimPlan::Delete => drawing_patch(target_id, Some(target_entity), None),
            TrimPlan::Keep { .. } => {
                // `after` was recomputed above; rebuild it from the committed
                // geometry so the patch is the exact stored value.
                let after = document.drawing.entity(target_id).cloned().ok_or_else(|| {
                    CadError::Invariant("trimmed entity vanished after commit".into())
                })?;
                drawing_patch(target_id, Some(target_entity.clone()), Some(after))
            }
        };
        self.record_drawing(
            document_id,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "trim entity".into(),
                patches: vec![patch],
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(target_id.0)],
            changes: Some(change_set),
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

    /// Shared insert path for LINE/CIRCLE creation.
    fn commit_new_entity(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometry: SemanticGeometry,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = Arc::make_mut(&mut document.drawing);

        // The active layer must exist: a missing layer is an explicit error,
        // never a silent fallback onto layer 0.
        if drawing.layer(session.active_layer).is_none() {
            return Err(CadError::InvalidInput(format!(
                "active layer {} does not exist",
                session.active_layer.0
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
            layer: session.active_layer,
            space: SpaceId::Model,
            geometry,
            draw_order,
        };

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            staged.insert_entity(entity.clone())?;
            staged.commit()?
        };
        self.record_drawing(
            document_id,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches: vec![drawing_patch(entity_id, None, Some(entity))],
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(entity_id.0)],
            changes: Some(change_set),
            diagnostics: Vec::new(),
            measurement: None,
        })
    }

    /// Append one drawing undo record to the document's history.
    fn record_drawing(
        &mut self,
        document_id: DocumentId,
        record: DrawingUndoRecord,
    ) -> CadResult<()> {
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing(record)
    }
}

/// A monotonic transaction id for drawing commits.
///
/// Mirrors the layer counter style but lives in the app layer so a
/// command never depends on a host. The counter is process-global and only needs
/// to be unique within a session's history.
fn next_transaction_id() -> TransactionId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    TransactionId(COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}

/// The stored class name for a semantic geometry, matching the importer's keys.
pub(crate) fn class_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "AcDbLine",
        SemanticGeometry::Circle { .. } => "AcDbCircle",
        SemanticGeometry::Polyline { .. } => "AcDbPolyline",
        SemanticGeometry::Arc { .. } => "AcDbArc",
        SemanticGeometry::Ellipse { .. } => "AcDbEllipse",
        SemanticGeometry::Spline { .. } => "AcDbSpline",
        SemanticGeometry::Mesh(_) => "AcDbSubDMesh",
        SemanticGeometry::Insert { .. } => "AcDbBlockReference",
        SemanticGeometry::Text { .. } => "AcDbText",
        SemanticGeometry::Shape { .. } => "AcDbShape",
        SemanticGeometry::Point(_) => "AcDbPoint",
        SemanticGeometry::Opaque { .. } => "AcDbUnknown",
        SemanticGeometry::Compound(_) => "AcDbCompound",
    }
}

/// A short, stable kind label for diagnostics (never a UI string).
fn geometry_kind(geometry: &SemanticGeometry) -> &'static str {
    class_name(geometry)
}

fn distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// The resolved boundary input for a trim: straight segments plus closed loops.
fn collect_boundary_segments(
    geometry: &SemanticGeometry,
    segments: &mut Vec<(Point3, Point3)>,
    closed_loops: &mut Vec<Vec<Point3>>,
) -> CadResult<()> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            segments.push((*start, *end));
            Ok(())
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.len() < 2 {
                return Ok(());
            }
            let straight = |i: usize| bulges.get(i).copied().unwrap_or(0.0).abs() <= 1e-12;
            for i in 0..points.len().saturating_sub(1) {
                if !straight(i) {
                    return Err(CadError::Unsupported(
                        "TRIM boundary contains a polyline arc segment; only straight LWPOLYLINE \
                         segments are supported"
                            .into(),
                    ));
                }
                segments.push((points[i], points[i + 1]));
            }
            if *closed {
                let last = points[points.len() - 1];
                if !straight(points.len() - 1) {
                    return Err(CadError::Unsupported(
                        "TRIM boundary contains a closing arc segment; only straight LWPOLYLINE \
                         segments are supported"
                            .into(),
                    ));
                }
                segments.push((last, points[0]));
                closed_loops.push(points.clone());
            }
            Ok(())
        }
        other => Err(CadError::Unsupported(format!(
            "TRIM boundary {} is not supported; only LINE and straight LWPOLYLINE are",
            geometry_kind(other)
        ))),
    }
}

/// What a resolved trim will do to the target.
#[derive(Clone, Copy)]
enum TrimPlan {
    /// The target is fully inside a closed boundary (or reduced to nothing).
    Delete,
    /// Keep the sub-segment from `start` to `end`.
    Keep { start: Point3, end: Point3 },
}

/// Resolve the trim cuts for a LINE target against straight boundary segments.
///
/// Returns `Err(reason)` for the documented degenerate/no-intersection cases so
/// the caller can raise `Unsupported` without touching the database.
fn plan_trim(
    a: Point3,
    b: Point3,
    pick: Point3,
    segments: &[(Point3, Point3)],
    closed_loops: &[Vec<Point3>],
) -> Result<TrimPlan, String> {
    let policy = TolerancePolicy::default();
    let tol = policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world);
    let ab = Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: 0.0,
    };
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let len = len2.sqrt();
    if !len.is_finite() || len <= tol {
        return Err("TRIM target is degenerate (zero length)".into());
    }
    let param_tol = (tol / len).max(1e-12);
    let t_pick = ((pick.x - a.x) * ab.x + (pick.y - a.y) * ab.y) / len2;
    if !t_pick.is_finite() || t_pick < -param_tol || t_pick > 1.0 + param_tol {
        return Err("TRIM pick point does not lie on the target line".into());
    }
    let t_pick = t_pick.clamp(0.0, 1.0);

    // Collect interior intersection parameters, de-duplicated within tolerance.
    let mut cuts: Vec<f64> = Vec::new();
    for (c, d) in segments {
        if let Some(t) = segment_intersection_param(a, b, *c, *d, tol) {
            if t > param_tol
                && t < 1.0 - param_tol
                && !cuts
                    .iter()
                    .any(|existing| (existing - t).abs() <= param_tol * 2.0)
            {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));

    if cuts.is_empty() {
        // No cut: the line is untouched unless it lies wholly inside a closed
        // boundary, in which case TRIM removes it entirely.
        let midpoint = Point3 {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
            z: 0.0,
        };
        if closed_loops
            .iter()
            .any(|loop_pts| point_in_polygon(midpoint, loop_pts))
        {
            return Ok(TrimPlan::Delete);
        }
        return Err("TRIM found no intersection between the target and the boundary".into());
    }

    // Breakpoints partition [0, 1]; keep the piece containing the pick.
    let mut breaks: Vec<f64> = Vec::with_capacity(cuts.len() + 2);
    breaks.push(0.0);
    breaks.extend(cuts.iter().copied());
    breaks.push(1.0);
    let mut keep = None;
    for window in breaks.windows(2) {
        let (t0, t1) = (window[0], window[1]);
        if t_pick + param_tol >= t0 && t_pick - param_tol <= t1 {
            keep = Some((t0, t1));
            break;
        }
    }
    let Some((t0, t1)) = keep else {
        return Err("TRIM could not determine the retained segment".into());
    };
    // A full-span window means the boundary did not actually cut the target.
    if t0 <= param_tol && t1 >= 1.0 - param_tol {
        return Err("TRIM boundary does not cut the target".into());
    }
    let start = point_at(a, b, t0);
    let end = point_at(a, b, t1);
    if distance(start, end) <= tol {
        return Ok(TrimPlan::Delete);
    }
    Ok(TrimPlan::Keep { start, end })
}

fn point_at(a: Point3, b: Point3, t: f64) -> Point3 {
    Point3 {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
    }
}

/// 2D parameter of the boundary segment `c→d` cut along `a→b`, or `None`.
///
/// Parallel/collinear segments return `None` (they are reported as an explicit
/// no-intersection by [`plan_trim`]) rather than fabricating an overlap cut.
fn segment_intersection_param(a: Point3, b: Point3, c: Point3, d: Point3, tol: f64) -> Option<f64> {
    let r = (b.x - a.x, b.y - a.y);
    let s = (d.x - c.x, d.y - c.y);
    let denom = r.0 * s.1 - r.1 * s.0;
    let scale = (len2(r).sqrt() * len2(s).sqrt()).max(f64::MIN_POSITIVE);
    if denom.abs() / scale <= tol.max(1e-12) {
        // Parallel or collinear: no unique crossing point.
        return None;
    }
    let ca = (c.x - a.x, c.y - a.y);
    let t = (ca.0 * s.1 - ca.1 * s.0) / denom;
    let u = (ca.0 * r.1 - ca.1 * r.0) / denom;
    let param_tol = (tol / len2(r).sqrt().max(f64::MIN_POSITIVE)).max(1e-12);
    if t >= -param_tol && t <= 1.0 + param_tol && (-1e-9..=1.0 + 1e-9).contains(&u) {
        Some(t.clamp(0.0, 1.0))
    } else {
        None
    }
}

fn len2(v: (f64, f64)) -> f64 {
    v.0 * v.0 + v.1 * v.1
}

/// 2D point-in-polygon (ray casting) on the XY plane.
fn point_in_polygon(p: Point3, polygon: &[Point3]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = polygon.len();
    let mut j = n - 1;
    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];
        let crosses = (pi.y > p.y) != (pj.y > p.y);
        if crosses {
            let x_at = (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x;
            if p.x < x_at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Apply a list of drawing patches to a database (used by undo/redo).
///
/// `forward` selects the `after` side (redo); otherwise the `before` side is
/// applied (undo). This goes through the database's single validated write path
/// so a patch that no longer matches the store is refused, never silently
/// force-written.
pub(crate) fn apply_drawing_patches(
    drawing: &mut DrawingDatabase,
    reason: &str,
    transaction: TransactionId,
    patches: &[DrawingPatch],
    forward: bool,
) -> CadResult<ChangeSet> {
    let changes: Vec<(EntityId, Option<DbEntity>)> = patches
        .iter()
        .map(|p| {
            (
                p.id,
                if forward {
                    p.after.clone()
                } else {
                    p.before.clone()
                },
            )
        })
        .collect();
    drawing.apply_drawing_changes(reason, transaction, changes)
}
