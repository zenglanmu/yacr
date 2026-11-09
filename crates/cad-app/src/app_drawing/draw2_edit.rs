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

    /// `MPEDIT CLOSE|OPEN|REVERSE` over every selected LWPOLYLINE.
    pub(crate) fn mpedit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::Mpedit(option) => option,
                _ => {
                    return Err(refusal(
                        "mpedit.payload",
                        "MPEDIT needs a Draw2::Mpedit(PeditOption) payload",
                    ));
                }
            },
            _ => {
                return Err(refusal(
                    "mpedit.payload",
                    "MPEDIT needs a Draw2::Mpedit(PeditOption) payload",
                ));
            }
        };
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
            let (points, bulges, closed) = match &entity.geometry {
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                } => (points.clone(), bulges.clone(), *closed),
                other => {
                    diagnostics.push(skipped(
                        "mpedit.skipped",
                        entity.id,
                        &format!("{} is not an LWPOLYLINE", class_name(other)),
                    ));
                    continue;
                }
            };
            let geometry = match option {
                PeditOption::Close => {
                    if closed {
                        diagnostics.push(skipped("mpedit.unchanged", entity.id, "already closed"));
                        continue;
                    }
                    if points.len() < 3 {
                        diagnostics.push(skipped(
                            "mpedit.skipped",
                            entity.id,
                            "fewer than three vertices cannot form a closed ring",
                        ));
                        continue;
                    }
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: true,
                    }
                }
                PeditOption::Open => {
                    if !closed {
                        diagnostics.push(skipped("mpedit.unchanged", entity.id, "already open"));
                        continue;
                    }
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: false,
                    }
                }
                PeditOption::Reverse => {
                    match super::modify_extra::reverse_geometry(&entity.geometry) {
                        Ok(geometry) => geometry,
                        Err(error) => {
                            diagnostics.push(skipped(
                                "mpedit.skipped",
                                entity.id,
                                &error.to_string(),
                            ));
                            continue;
                        }
                    }
                }
            };
            changes.push(CurveChange::Replace(entity, geometry));
        }
        self.commit_curve_changes(document_id, changes, diagnostics, "mpedit")
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
