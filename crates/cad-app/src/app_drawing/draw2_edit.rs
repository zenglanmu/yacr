//! The 2026-11-07 selection-edit batch: `TREX`, `EXTRIM`, `MPEDIT` and
//! `DRAWORDER`.
//!
//! Both trim commands reuse the existing planner
//! (`app_drawing::trim_curves::plan_trim` / `plan_extend`) and its boundary
//! collector — there is no second trim implementation. Every command resolves
//! and plans all of its work before a transaction opens, stages the whole plan in
//! one transaction and publishes one undo step.
//!
//! Full contracts: [`crate::command_id::draw2`].

use std::collections::BTreeSet;

use super::helpers::distance;
use super::trim_boundary::{collect_boundaries, BoundarySet};
use super::trim_curves::{
    cut_tolerance, plan_extend, plan_trim, PlanRefusal, TargetCurve, TrimPlan,
};
use super::*;

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

fn skipped(code: &str, entity: EntityId, detail: &str) -> Diagnostic {
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: code.into(),
        message: format!("{code}: entity {} skipped: {detail}", entity.0),
    }
}

/// The exact `PEDITACCEPT` conversion MPEDIT applies to a batch selection.
///
/// A LINE becomes its two endpoints; an ARC of the `+Z` plane becomes one bulge
/// segment (start point → end point). SPLINE tessellation and every other kind
/// stay PEDIT's single-object `convert` answer, never a silent approximation
/// here. The error text is the per-object skip reason.
fn convert_mpedit_object(geometry: &SemanticGeometry) -> Result<SemanticGeometry, String> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if distance(*start, *end) <= 1e-9 {
                return Err("the LINE has no length".into());
            }
            Ok(SemanticGeometry::Polyline {
                points: vec![*start, *end],
                bulges: vec![0.0],
                closed: false,
                width: 0.0,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let radius = radius.abs();
            if radius <= 1e-9 || !sweep.is_finite() || sweep.abs() <= 1e-12 {
                return Err("the ARC has no radius or sweep".into());
            }
            if normal.z < 1.0 - 1e-9 {
                return Err("an ARC outside the +Z plane cannot become a bulged polyline".into());
            }
            let Some(bulge) = cad_geometry::arc_sweep_to_bulge(*sweep) else {
                return Err(
                    "an ARC of one full turn or more cannot be one polyline bulge segment".into(),
                );
            };
            Ok(SemanticGeometry::Polyline {
                points: vec![
                    arc_endpoint(*center, *normal, radius, *start),
                    arc_endpoint(*center, *normal, radius, start + sweep),
                ],
                bulges: vec![bulge],
                closed: false,
                width: 0.0,
            })
        }
        other => Err(format!("{} is not an LWPOLYLINE", class_name(other))),
    }
}

/// The point at `angle` on an arc in its own plane.
fn arc_endpoint(center: Point3, normal: Point3, radius: f64, angle: f64) -> Point3 {
    let (axis_x, axis_y, _) = cad_geometry::arbitrary_axis(normal);
    Point3 {
        x: center.x + radius * (axis_x.x * angle.cos() + axis_y.x * angle.sin()),
        y: center.y + radius * (axis_x.y * angle.cos() + axis_y.y * angle.sin()),
        z: center.z + radius * (axis_x.z * angle.cos() + axis_y.z * angle.sin()),
    }
}

/// One staged trim change, built before the transaction opens.
pub(crate) enum CurveChange {
    Replace(DbEntity, SemanticGeometry),
    Delete(DbEntity, cad_db::EntityDisplayState),
}

impl Application {
    /// `TREX TRIM|EXTEND` over the current selection.
    pub(crate) fn trex(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let mode = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::Trex(mode) => mode,
                _ => {
                    return Err(refusal(
                        "trex.payload",
                        "TREX needs a Draw2::Trex(mode) payload",
                    ))
                }
            },
            _ => {
                return Err(refusal(
                    "trex.payload",
                    "TREX needs a Draw2::Trex(mode) payload",
                ))
            }
        };
        let mut refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(refusal(
                "trex.selection",
                format!(
                    "TREX needs at least two selected objects (targets and boundaries), got {} \
                     (there is no boundary prompt)",
                    refs.len()
                ),
            ));
        }
        let document_id = command.document;
        let (targets, diagnostics) = self.resolve_batch(document_id, &refs, "trex")?;
        refs.clear();
        if targets.len() < 2 {
            return Err(refusal(
                "trex.selection",
                "TREX needs at least two distinct selected objects",
            ));
        }
        let policy = TolerancePolicy::default();
        let tol = cut_tolerance();
        let changes = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| refusal("trex.not_addressable", "document not open"))?;
            let drawing = document.drawing.as_ref();
            let mut changes = Vec::new();
            let mut diagnostics = diagnostics;
            for target in &targets {
                let curve =
                    TargetCurve::new(&target.geometry, "TREX", "a LINE, ARC, CIRCLE or SPLINE")
                        .map_err(|error| match error {
                            CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                                refusal("trex.unsupported_target", message)
                            }
                            other => other,
                        })?;
                let mut boundaries = BoundarySet::default();
                for other in &targets {
                    if other.id == target.id {
                        continue;
                    }
                    collect_boundaries(&other.geometry, &mut boundaries, &policy, "TREX").map_err(
                        |error| match error {
                            CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                                refusal("trex.unsupported_boundary", message)
                            }
                            other => other,
                        },
                    )?;
                }
                if boundaries.is_empty() {
                    diagnostics.push(skipped(
                        "trex.skipped",
                        target.id,
                        "no other selected object can bound a cut",
                    ));
                    continue;
                }
                let plan = match mode {
                    TrexMode::Trim => {
                        match plan_trim(&curve, curve.point_at(0.5), &boundaries, &policy, tol) {
                            Ok(resolution) => Some(resolution.plan),
                            Err(reason) => {
                                diagnostics.push(skipped(
                                    "trex.skipped",
                                    target.id,
                                    &plan_reason(reason),
                                ));
                                None
                            }
                        }
                    }
                    TrexMode::Extend => {
                        match plan_extend(&curve, curve.point_at(1.0), &boundaries, &policy, tol) {
                            Ok(resolution) => Some(TrimPlan::Replace(resolution.geometry)),
                            Err(reason) => {
                                diagnostics.push(skipped("trex.skipped", target.id, &reason));
                                None
                            }
                        }
                    }
                };
                if let Some(plan) = plan {
                    match plan {
                        TrimPlan::Replace(geometry) => {
                            changes.push(CurveChange::Replace(target.clone(), geometry))
                        }
                        TrimPlan::Delete => changes.push(CurveChange::Delete(
                            target.clone(),
                            drawing.entity_display_state(target.id),
                        )),
                    }
                }
            }
            (changes, diagnostics)
        };
        let (changes, diagnostics) = changes;
        self.commit_curve_changes(document_id, changes, diagnostics, "trex")
    }

    /// `EXTRIM`: trim every crossing object at one boundary.
    pub(crate) fn extrim(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let (boundary, targets) = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::Extrim {
                    ref boundary,
                    ref targets,
                } => (boundary.clone(), targets.clone()),
                _ => {
                    return Err(refusal(
                        "extrim.payload",
                        "EXTRIM needs a Draw2::Extrim { boundary, targets } payload",
                    ));
                }
            },
            CommandPayload::None => {
                let refs = session.selection.refs();
                let Some((boundary, rest)) = refs.split_first() else {
                    return Err(refusal(
                        "extrim.selection",
                        "EXTRIM needs a boundary plus at least one target; the first selected \
                         object is the boundary",
                    ));
                };
                (boundary.clone(), rest.to_vec())
            }
            _ => {
                return Err(refusal(
                    "extrim.payload",
                    "EXTRIM takes a Draw2::Extrim payload or no payload (selection form)",
                ));
            }
        };
        if targets.is_empty() {
            return Err(refusal(
                "extrim.selection",
                "EXTRIM needs at least one crossing target",
            ));
        }
        let (target_entities, mut diagnostics) =
            self.resolve_batch(document_id, &targets, "extrim")?;
        if target_entities
            .iter()
            .any(|entity| entity.id == boundary.entity)
        {
            return Err(refusal(
                "extrim.boundary_is_target",
                "the EXTRIM boundary cannot also be a target",
            ));
        }
        let policy = TolerancePolicy::default();
        let tol = cut_tolerance();
        let changes = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| refusal("extrim.not_addressable", "document not open"))?;
            let drawing = document.drawing.as_ref();
            let boundary_entity = drawing.entity(boundary.entity).ok_or_else(|| {
                refusal(
                    "extrim.not_addressable",
                    format!("boundary {} does not exist", boundary.entity.0),
                )
            })?;
            if boundary_entity.space != SpaceId::Model {
                return Err(refusal(
                    "extrim.not_addressable",
                    format!("boundary {} is not in model space", boundary.entity.0),
                ));
            }
            let mut boundaries = BoundarySet::default();
            collect_boundaries(
                &boundary_entity.geometry,
                &mut boundaries,
                &policy,
                "EXTRIM",
            )
            .map_err(|error| match error {
                CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                    refusal("extrim.unsupported_boundary", message)
                }
                other => other,
            })?;
            if boundaries.is_empty() {
                return Err(unsupported(
                    "extrim.unsupported_boundary",
                    "the boundary geometry cannot bound a cut",
                ));
            }
            let mut changes = Vec::new();
            for target in &target_entities {
                let curve =
                    TargetCurve::new(&target.geometry, "EXTRIM", "a LINE, ARC, CIRCLE or SPLINE")
                        .map_err(|error| match error {
                        CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                            refusal("extrim.unsupported_target", message)
                        }
                        other => other,
                    })?;
                match plan_trim(&curve, curve.point_at(0.5), &boundaries, &policy, tol) {
                    Ok(resolution) => match resolution.plan {
                        TrimPlan::Replace(geometry) => {
                            changes.push(CurveChange::Replace(target.clone(), geometry))
                        }
                        TrimPlan::Delete => changes.push(CurveChange::Delete(
                            target.clone(),
                            drawing.entity_display_state(target.id),
                        )),
                    },
                    Err(reason) => {
                        diagnostics.push(skipped("extrim.skipped", target.id, &plan_reason(reason)))
                    }
                }
            }
            changes
        };
        self.commit_curve_changes(document_id, changes, diagnostics, "extrim")
    }

    /// `MPEDIT`: one typed polyline option over **every** selected object.
    ///
    /// `CLOSE` / `OPEN` / `REVERSE` are applied per selected polyline in one
    /// transaction / one undo step. `convert` (the `PEDITACCEPT` answer) first
    /// turns every selected LINE/ARC into an exact polyline — a LINE is its two
    /// endpoints, an ARC of the `+Z` plane is one bulge segment — so MPEDIT also
    /// performs AutoCAD's "Object selected is not a polyline. Do you want it to
    /// turn into one?" conversion for a batch.
    ///
    /// Every other PEDIT option (`JOIN`, `FIT`, `SPLINE`, `DECURVE`, `WIDTH`,
    /// `LTYPE GEN`, `UNDO`) delegates to [`Self::pedit`] and therefore keeps
    /// exactly PEDIT's single-object planners, selection rules and refusals: a
    /// heterogeneous batch has no defined meaning for them, so a mismatch is a
    /// named refusal from `pedit`, never a silent partial edit. Objects that need
    /// no change are reported one by one (`mpedit.unchanged`) and write nothing.
    pub(crate) fn mpedit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::Mpedit(option) => PeditEditCommand {
                    option: match option {
                        PeditOption::Close => PeditEditOption::Close,
                        PeditOption::Open => PeditEditOption::Open,
                        PeditOption::Reverse => PeditEditOption::Reverse,
                    },
                    convert: false,
                },
                _ => {
                    return Err(refusal(
                        "mpedit.payload",
                        "MPEDIT needs a Draw2::Mpedit(PeditOption) or PeditEdit payload",
                    ));
                }
            },
            CommandPayload::PeditEdit(request) => *request,
            _ => {
                return Err(refusal(
                    "mpedit.payload",
                    "MPEDIT needs a Draw2::Mpedit(PeditOption) or PeditEdit payload",
                ));
            }
        };
        if !matches!(
            request.option,
            PeditEditOption::Close | PeditEditOption::Open | PeditEditOption::Reverse
        ) {
            // `JOIN`/`FIT`/`SPLINE`/`DECURVE`/`WIDTH`/`LTYPE GEN`/`UNDO` are
            // PEDIT's own plans; reuse them verbatim (one transaction there).
            return self.pedit(session, command);
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(refusal(
                "mpedit.selection",
                "MPEDIT needs at least one selected polyline",
            ));
        }
        let document_id = command.document;
        let (mut entities, mut diagnostics) = self.resolve_batch(document_id, refs, "mpedit")?;
        let mut changes = Vec::new();
        entities.sort_by_key(|entity| (entity.draw_order, entity.id.0));
        for entity in entities {
            // `convert` expands a LINE/ARC into the exact polyline form; an
            // unconverted object that is not a polyline is skipped by name.
            let mut converted = false;
            let source = match &entity.geometry {
                SemanticGeometry::Polyline { .. } => entity.geometry.clone(),
                other if request.convert => match convert_mpedit_object(other) {
                    Ok(geometry) => {
                        converted = true;
                        geometry
                    }
                    Err(reason) => {
                        diagnostics.push(skipped("mpedit.skipped", entity.id, &reason));
                        continue;
                    }
                },
                other => {
                    diagnostics.push(skipped(
                        "mpedit.skipped",
                        entity.id,
                        &format!(
                            "{} is not an LWPOLYLINE (pass convert = true to turn it into one)",
                            class_name(other)
                        ),
                    ));
                    continue;
                }
            };
            let (points, bulges, closed) = match &source {
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                    ..
                } => (points.clone(), bulges.clone(), *closed),
                other => unreachable!("MPEDIT source is always a polyline, got {other:?}"),
            };
            let mut unchanged: Option<&str> = None;
            let geometry = match request.option {
                PeditEditOption::Close => {
                    if closed {
                        unchanged = Some("already closed");
                        source.clone()
                    } else if points.len() < 3 {
                        diagnostics.push(skipped(
                            "mpedit.skipped",
                            entity.id,
                            "fewer than three vertices cannot form a closed ring",
                        ));
                        continue;
                    } else {
                        SemanticGeometry::Polyline {
                            points,
                            bulges,
                            closed: true,
                            width: 0.0,
                        }
                    }
                }
                PeditEditOption::Open => {
                    if !closed {
                        unchanged = Some("already open");
                        source.clone()
                    } else {
                        SemanticGeometry::Polyline {
                            points,
                            bulges,
                            closed: false,
                            width: 0.0,
                        }
                    }
                }
                PeditEditOption::Reverse => match super::modify_extra::reverse_geometry(&source) {
                    Ok(geometry) => geometry,
                    Err(error) => {
                        diagnostics.push(skipped("mpedit.skipped", entity.id, &error.to_string()));
                        continue;
                    }
                },
                _ => unreachable!("only CLOSE/OPEN/REVERSE reach the batch path"),
            };
            if converted || geometry != entity.geometry {
                changes.push(CurveChange::Replace(entity, geometry));
            } else if let Some(reason) = unchanged {
                diagnostics.push(skipped("mpedit.unchanged", entity.id, reason));
            }
        }
        self.commit_curve_changes_inner(document_id, changes, diagnostics, "mpedit", true)
    }

    /// `DRAWORDER FRONT|BACK|ABOVE|BELOW` over the current selection.
    pub(crate) fn draw_order(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let mode = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::DrawOrder(mode) => mode,
                _ => {
                    return Err(refusal(
                        "draworder.payload",
                        "DRAWORDER needs a Draw2::DrawOrder(Front|Back|Above|Below) payload",
                    ));
                }
            },
            _ => {
                return Err(refusal(
                    "draworder.payload",
                    "DRAWORDER needs a Draw2::DrawOrder(Front|Back|Above|Below) payload",
                ));
            }
        };
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(refusal(
                "draworder.selection",
                "DRAWORDER needs at least one selected model-space object",
            ));
        }
        if matches!(mode, DrawOrderMode::Above | DrawOrderMode::Below) && refs.len() < 2 {
            return Err(refusal(
                "draworder.reference_required",
                "DRAWORDER ABOVE/BELOW uses the first selected object as the reference and \
                 needs at least one more selected object to move",
            ));
        }
        let document_id = command.document;
        let (entities, diagnostics) =
            self.resolve_batch_no_space(document_id, refs, "draworder")?;
        let mut selected: Vec<DbEntity> = entities;
        selected.sort_by_key(|entity| (entity.draw_order, entity.id.0));
        let selected_ids: BTreeSet<EntityId> = selected.iter().map(|entity| entity.id).collect();

        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal("draworder.not_addressable", "document not open"))?;
        let drawing = document.drawing.as_ref();
        let mut others: Vec<DbEntity> = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .filter(|entity| !selected_ids.contains(&entity.id))
            .cloned()
            .collect();
        others.sort_by_key(|entity| (entity.draw_order, entity.id.0));

        let mut planned: Vec<(DbEntity, i64)> = Vec::new();
        match mode {
            DrawOrderMode::Front | DrawOrderMode::Back => {
                if others.is_empty() {
                    return Ok(unchanged_order(
                        diagnostics,
                        "there is no other model-space object",
                    ));
                }
                let mut target = if matches!(mode, DrawOrderMode::Front) {
                    others
                        .iter()
                        .map(|entity| entity.draw_order)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or_else(order_exhausted)?
                } else {
                    others
                        .iter()
                        .map(|entity| entity.draw_order)
                        .min()
                        .unwrap_or(0)
                        .checked_sub(selected.len() as i64)
                        .ok_or_else(order_exhausted)?
                };
                for entity in &selected {
                    planned.push((entity.clone(), target));
                    target = target.checked_add(1).ok_or_else(order_exhausted)?;
                }
            }
            DrawOrderMode::Above | DrawOrderMode::Below => {
                let reference_id = refs[0].entity;
                let reference = selected
                    .iter()
                    .find(|entity| entity.id == reference_id)
                    .ok_or_else(|| {
                        refusal(
                            "draworder.reference_required",
                            "the first selected object is not a model-space entity",
                        )
                    })?
                    .clone();
                let moving: Vec<DbEntity> = selected
                    .iter()
                    .filter(|entity| entity.id != reference.id)
                    .cloned()
                    .collect();
                let n = moving.len() as i64;
                let reference_order = reference.draw_order;
                let above = matches!(mode, DrawOrderMode::Above);
                let side: Vec<DbEntity> = others
                    .iter()
                    .filter(|entity| {
                        if above {
                            entity.draw_order > reference_order
                        } else {
                            entity.draw_order < reference_order
                        }
                    })
                    .cloned()
                    .collect();
                let gap: i64 = if above {
                    side.iter()
                        .map(|entity| entity.draw_order)
                        .min()
                        .map(|next| next - reference_order - 1)
                        .unwrap_or(i64::MAX / 4)
                } else {
                    side.iter()
                        .map(|entity| entity.draw_order)
                        .max()
                        .map(|previous| reference_order - previous - 1)
                        .unwrap_or(i64::MAX / 4)
                };
                let shift = if gap >= n { 0 } else { n - gap };
                let base = if above {
                    reference_order.checked_add(1).ok_or_else(order_exhausted)?
                } else {
                    reference_order.checked_sub(n).ok_or_else(order_exhausted)?
                };
                for (index, entity) in moving.iter().enumerate() {
                    planned.push((
                        entity.clone(),
                        base.checked_add(index as i64).ok_or_else(order_exhausted)?,
                    ));
                }
                if shift > 0 {
                    for entity in side {
                        planned.push((
                            entity.clone(),
                            if above {
                                entity
                                    .draw_order
                                    .checked_add(shift)
                                    .ok_or_else(order_exhausted)?
                            } else {
                                entity
                                    .draw_order
                                    .checked_sub(shift)
                                    .ok_or_else(order_exhausted)?
                            },
                        ));
                    }
                }
            }
        }

        let mut changed: Vec<(DbEntity, i64)> = planned
            .into_iter()
            .filter(|(entity, order)| entity.draw_order != *order)
            .collect();
        changed.sort_by_key(|(entity, _)| (entity.draw_order, entity.id.0));
        if changed.is_empty() {
            return Ok(unchanged_order(
                diagnostics,
                "every target already has its draw order",
            ));
        }
        let mut candidate = drawing.clone();
        let drawing = &mut candidate;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("draw order", transaction)?;
            for (entity, order) in &changed {
                let mut after = entity.clone();
                after.draw_order = *order;
                staged.update_entity(after)?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = changed
            .iter()
            .map(|(entity, _)| {
                let after = drawing.entity(entity.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "DRAWORDER: entity {} vanished after commit",
                        entity.id.0
                    ))
                })?;
                Ok(drawing_patch(entity.id, Some(entity.clone()), Some(after)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        let ids: Vec<ObjectId> = changed
            .iter()
            .map(|(entity, _)| ObjectId(entity.id.0))
            .collect();
        let moved = ids.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "draw order".into(),
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
        let mut diagnostics = diagnostics;
        diagnostics.push(Diagnostic {
            object: None,
            code: "draworder.applied".into(),
            message: format!("DRAWORDER: rewrote the draw order of {moved} model-space object(s)"),
        });
        Ok(CommandOutcome {
            objects: ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Resolve refs to direct model-space entities, refusing anything else.
    ///
    /// `label` is the refusal prefix (`trex`, `extrim`, `mpedit`).
    fn resolve_batch(
        &self,
        document_id: DocumentId,
        refs: &[SelectionRef],
        label: &str,
    ) -> CadResult<(Vec<DbEntity>, Vec<Diagnostic>)> {
        let (entities, diagnostics) = self.resolve_batch_no_space(document_id, refs, label)?;
        if let Some(entity) = entities
            .iter()
            .find(|entity| entity.space != SpaceId::Model)
        {
            return Err(refusal(
                &format!("{label}.not_addressable"),
                format!(
                    "entity {} is not in model space; this command edits model space only",
                    entity.id.0
                ),
            ));
        }
        Ok((entities, diagnostics))
    }

    /// Resolve refs to direct entities in any space (DRAWORDER filters below).
    fn resolve_batch_no_space(
        &self,
        document_id: DocumentId,
        refs: &[SelectionRef],
        label: &str,
    ) -> CadResult<(Vec<DbEntity>, Vec<Diagnostic>)> {
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal(&format!("{label}.not_addressable"), "document not open"))?
            .drawing
            .as_ref();
        let mut seen = BTreeSet::new();
        let mut entities = Vec::new();
        let mut diagnostics = Vec::new();
        for reference in refs {
            if reference.document != document_id {
                return Err(refusal(
                    &format!("{label}.not_addressable"),
                    format!("entity {} belongs to another document", reference.entity.0),
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(refusal(
                    &format!("{label}.not_addressable"),
                    format!(
                        "entity {} is referenced through a block instance or a sub-element",
                        reference.entity.0
                    ),
                ));
            }
            if !seen.insert(reference.entity) {
                continue;
            }
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                refusal(
                    &format!("{label}.not_addressable"),
                    format!(
                        "entity {} does not exist in this drawing",
                        reference.entity.0
                    ),
                )
            })?;
            entities.push(entity);
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: format!("{label}.selection"),
            message: format!(
                "{label}: {} distinct directly addressable object(s)",
                entities.len()
            ),
        });
        Ok((entities, diagnostics))
    }

    /// Stage a planned set of curve replacements/deletes in one transaction.
    pub(crate) fn commit_curve_changes(
        &mut self,
        document_id: DocumentId,
        changes: Vec<CurveChange>,
        diagnostics: Vec<Diagnostic>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        self.commit_curve_changes_inner(document_id, changes, diagnostics, label, false)
    }

    /// The one batch-commit funnel, with an option to re-derive the entity's
    /// `type_key` from the replacement geometry (needed when a command converts
    /// a LINE/ARC into a POLYLINE, which is what MPEDIT's `convert` answer does).
    fn commit_curve_changes_inner(
        &mut self,
        document_id: DocumentId,
        changes: Vec<CurveChange>,
        diagnostics: Vec<Diagnostic>,
        label: &str,
        retype: bool,
    ) -> CadResult<CommandOutcome> {
        if changes.is_empty() {
            let mut diagnostics = diagnostics;
            diagnostics.push(Diagnostic {
                object: None,
                code: format!("{label}.nothing_changed"),
                message: format!("{label}: no object needed a change; nothing was written"),
            });
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
                point_report: None,
            });
        }
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal(&format!("{label}.not_addressable"), "document not open"))?
            .drawing
            .as_ref()
            .clone();
        let drawing = &mut candidate;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for change in &changes {
                match change {
                    CurveChange::Replace(before, geometry) => {
                        let mut after = before.clone();
                        after.geometry = geometry.clone();
                        if retype {
                            after.object.type_key = class_name(geometry).to_string();
                        }
                        staged.update_entity(after)?;
                    }
                    CurveChange::Delete(before, _) => staged.delete_entity(before.id)?,
                }
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(changes.len());
        let mut ids = Vec::with_capacity(changes.len());
        for change in changes {
            match change {
                CurveChange::Replace(before, _) => {
                    let after = drawing.entity(before.id).cloned().ok_or_else(|| {
                        CadError::Invariant(format!(
                            "{label}: entity {} vanished after commit",
                            before.id.0
                        ))
                    })?;
                    ids.push(ObjectId(before.id.0));
                    patches.push(drawing_patch(before.id, Some(before), Some(after)));
                }
                CurveChange::Delete(before, state) => {
                    ids.push(ObjectId(before.id.0));
                    patches.push(drawing_patch_deleted(before.id, before, state));
                }
            }
        }
        let count = ids.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
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
        let mut diagnostics = diagnostics;
        diagnostics.push(Diagnostic {
            object: None,
            code: format!("{label}.applied"),
            message: format!("{label}: changed {count} object(s) in one transaction"),
        });
        Ok(CommandOutcome {
            objects: ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

fn plan_reason(reason: PlanRefusal) -> String {
    match reason {
        PlanRefusal::Invalid(message) | PlanRefusal::Unsupported(message) => message,
    }
}

fn unchanged_order(diagnostics: Vec<Diagnostic>, detail: &str) -> CommandOutcome {
    let mut diagnostics = diagnostics;
    diagnostics.push(Diagnostic {
        object: None,
        code: "draworder.unchanged".into(),
        message: format!("DRAWORDER: {detail}; nothing was written"),
    });
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

fn order_exhausted() -> CadError {
    refusal(
        "draworder.order_exhausted",
        "a target draw order would overflow i64",
    )
}
