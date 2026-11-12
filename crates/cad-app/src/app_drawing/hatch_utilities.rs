//! `HATCHGENERATEBOUNDARY` and `HATCHTOBACK` (AutoCAD 2026 hatch utilities).
//!
//! Two Work-only commands that act on stored hatch entities without changing the
//! hatches' geometry:
//!
//! * [`crate::CommandId::HatchGenerateBoundary`] — create one **non-associative**
//!   closed polyline per hatch loop around the selected hatches, on each hatch's
//!   own layer and space, in one transaction / one undo step. The source hatches
//!   are retained.
//! * [`crate::CommandId::HatchToBack`] — move every **model-space** hatch behind
//!   every non-hatch object by rewriting only the stored `draw_order`, in one
//!   transaction / one undo step.
//!
//! Both handlers validate their whole input before staging anything, so a refusal
//! leaves the database revision, the history depth and the ID allocator untouched.
//! The long-form contracts live in `crate::command_id::hatch_generate_boundary`
//! and `crate::command_id::hatch_to_back`.
//!
//! The AutoCAD `.pat` pattern library lives in [`super::hatch_pattern`].

use super::*;

impl Application {
    /// `HATCHGENERATEBOUNDARY`: one closed polyline per loop of every selected
    /// hatch, in one transaction.
    ///
    /// The selection is the whole input (`CommandPayload::None`). Every ref must
    /// be a directly addressable **model-space** `SemanticGeometry::Hatch`;
    /// anything else is refused by name before a write. Each loop is mapped from
    /// the hatch's own OCS 2D frame through [`HatchPlane::to_world`] — the same
    /// arbitrary-axis projection the fill/display uses — and stored as an
    /// `AcDbPolyline` with the loop's bulges and the closed flag. A repeated last
    /// point is dropped (with the degenerate closing bulge when the source
    /// carried one) before the world mapping.
    ///
    /// The boundary polylines inherit the source hatch's layer and space. They are
    /// a snapshot, not an associative boundary: editing the hatch afterwards does
    /// not update them.
    pub(crate) fn hatch_generate_boundary(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "hatchgenerateboundary.payload",
                "HATCHGENERATEBOUNDARY takes no payload; the selection is the input",
            ));
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(refusal(
                "hatchgenerateboundary.selection",
                "HATCHGENERATEBOUNDARY needs at least one selected hatch",
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // 1. Classify every ref (and every loop) before allocating an id.
        let mut planned: Vec<(LayerId, SpaceId, SemanticGeometry)> = Vec::new();
        let mut hatch_count = 0usize;
        for reference in refs {
            if reference.document != document_id {
                return Err(not_addressable(
                    reference.entity,
                    "it belongs to another document",
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(not_addressable(
                    reference.entity,
                    "it is referenced through a block instance or a sub-element",
                ));
            }
            let source = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                not_addressable(reference.entity, "it does not exist in this drawing")
            })?;
            if source.space != SpaceId::Model {
                return Err(not_addressable(
                    reference.entity,
                    "it is not in model space",
                ));
            }
            let SemanticGeometry::Hatch(hatch) = &source.geometry else {
                return Err(refusal(
                    "hatchgenerateboundary.not_hatch",
                    format!(
                        "entity {} is {}, not a hatch",
                        reference.entity.0,
                        geometry_kind(&source.geometry)
                    ),
                ));
            };
            hatch_count += 1;
            let plane = hatch.plane();
            for loop_ in hatch.loops() {
                let geometry = boundary_polyline(loop_, plane, reference.entity)?;
                planned.push((source.layer, source.space.clone(), geometry));
            }
        }
        if planned.is_empty() {
            return Err(refusal(
                "hatchgenerateboundary.empty",
                "the selected hatch(es) store no boundary loops",
            ));
        }
        let loop_count = planned.len();

        // 2. One draw-order run after the highest model-space row, exactly like the
        //    shared create funnel (`model_space()`'s filter, not a new ordering).
        let base_order = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let mut entities = Vec::with_capacity(planned.len());
        for (index, (layer, space, geometry)) in planned.into_iter().enumerate() {
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
                space,
                geometry,
                draw_order: base_order + index as i64,
            });
        }

        // 3. One transaction / one undo step; the hatches themselves are untouched.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged =
                drawing.begin_drawing_transaction("hatch generate boundary", transaction)?;
            for entity in &entities {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = entities
            .iter()
            .map(|entity| drawing_patch(entity.id, None, Some(entity.clone())))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "hatch generate boundary".into(),
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

        Ok(CommandOutcome {
            objects: entities
                .iter()
                .map(|entity| ObjectId(entity.id.0))
                .collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "hatchgenerateboundary.applied".into(),
                message: format!(
                    "HATCHGENERATEBOUNDARY: created {loop_count} boundary polyline(s) from \
                     {hatch_count} hatch(es)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `HATCHTOBACK`: put every model-space hatch behind all other model-space
    /// objects by rewriting their stored `draw_order` in one transaction.
    ///
    /// No payload and no selection. With no hatches (`hatchtoback.none`) or with
    /// every hatch already strictly below the lowest non-hatch order
    /// (`hatchtoback.unchanged`) nothing is staged. Otherwise the hatches keep
    /// their relative order and receive the contiguous block
    /// `min_order - n ..= min_order - 1`; the arithmetic is checked, so an
    /// `i64` underflow is the explicit `hatchtoback.order_exhausted` refusal.
    ///
    /// Only model-space rows are considered: this build has no paper-space draw
    /// order model, so layout hatches are left alone (declared, not silent).
    pub(crate) fn hatch_to_back(
        &mut self,
        _session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "hatchtoback.payload",
                "HATCHTOBACK takes no payload and no selection",
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let model: Vec<DbEntity> = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .cloned()
            .collect();
        let hatches: Vec<DbEntity> = model
            .iter()
            .filter(|entity| matches!(entity.geometry, SemanticGeometry::Hatch(_)))
            .cloned()
            .collect();
        if hatches.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "hatchtoback.none".into(),
                    message: "HATCHTOBACK: no hatches in model space".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let min_order = model
            .iter()
            .filter(|entity| !matches!(entity.geometry, SemanticGeometry::Hatch(_)))
            .map(|entity| entity.draw_order)
            .min();
        let Some(min_order) = min_order else {
            // Nothing but hatches: each hatch is already behind every non-hatch.
            return Ok(unchanged_outcome());
        };
        if hatches.iter().all(|entity| entity.draw_order < min_order) {
            return Ok(unchanged_outcome());
        }

        let count = i64::try_from(hatches.len()).map_err(|_| order_exhausted())?;
        let first = min_order.checked_sub(count).ok_or_else(order_exhausted)?;
        // `first` is the lowest target and `first + (n-1) = min_order - 1` is the
        // highest; both are representable once `min_order - n` is.
        let mut ordered = hatches;
        ordered.sort_by_key(|entity| (entity.draw_order, entity.id.0));
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(ordered.len());
        for (index, before) in ordered.into_iter().enumerate() {
            let target = first + index as i64;
            if before.draw_order == target {
                continue;
            }
            let mut after = before.clone();
            after.draw_order = target;
            planned.push((before, after));
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("hatch to back", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let ids: Vec<ObjectId> = planned
            .iter()
            .map(|(before, _)| ObjectId(before.id.0))
            .collect();
        let moved = ids.len();
        let patches: Vec<DrawingPatch> = planned
            .into_iter()
            .map(|(before, after)| drawing_patch(before.id, Some(before), Some(after)))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "hatch to back".into(),
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

        Ok(CommandOutcome {
            objects: ids,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "hatchtoback.applied".into(),
                message: format!("HATCHTOBACK: moved {moved} hatch(es) behind every other object"),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// A command refusal carrying its stable reason code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn not_addressable(entity: EntityId, detail: &str) -> CadError {
    refusal(
        "hatchgenerateboundary.not_addressable",
        format!(
            "entity {} is not a directly addressable model-space entity: {detail}",
            entity.0
        ),
    )
}

fn order_exhausted() -> CadError {
    refusal(
        "hatchtoback.order_exhausted",
        "the target draw orders would underflow i64",
    )
}

fn unchanged_outcome() -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "hatchtoback.unchanged".into(),
            message: "HATCHTOBACK: every hatch is already behind every other object".into(),
        }],
        measurement: None,
        point_report: None,
    }
}

/// One hatch loop as a **closed** world-space polyline.
///
/// The loop is stored in the hatch plane's 2D OCS frame; [`HatchPlane::to_world`]
/// is the single mapping back (the same arbitrary-axis basis the display uses).
/// A repeated last point is dropped, together with the last bulge when the source
/// stored one per vertex: that trailing segment is the degenerate closure, and a
/// closed polyline already implies it.
fn boundary_polyline(
    loop_: &HatchLoop,
    plane: HatchPlane,
    entity: EntityId,
) -> CadResult<SemanticGeometry> {
    let (points, bulges) = loop_world_ring(loop_, plane).map_err(|detail| {
        refusal(
            "hatchgenerateboundary.invalid_loop",
            format!("hatch {} {detail}", entity.0),
        )
    })?;
    Ok(SemanticGeometry::Polyline {
        points,
        bulges,
        closed: true,
        width: 0.0,
    })
}

/// Convert one hatch-plane loop into a closed world-space ring.
///
/// This is the shared loop-to-polyline conversion used by HATCHGENERATEBOUNDARY
/// and BOUNDARY (2026-11-07): [`HatchPlane::to_world`] is the single mapping back
/// (the same arbitrary-axis basis the display uses), a repeated last point is
/// dropped together with its degenerate closing bulge, and a straight ring gets
/// one zero bulge per vertex.
///
/// The error is a detail sentence (the caller prefixes its own entity/reason
/// code); it only describes a malformed loop the caller must refuse.
pub(crate) fn loop_world_ring(
    loop_: &HatchLoop,
    plane: HatchPlane,
) -> Result<(Vec<Point3>, Vec<f64>), String> {
    let mut points = loop_.points.clone();
    let mut bulges = loop_.bulges.clone();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
        if bulges.len() == loop_.points.len() {
            bulges.pop();
        }
    }
    if points.len() < 2 {
        return Err("has a boundary loop with fewer than two vertices".into());
    }
    if !bulges.is_empty() && bulges.len() != points.len() {
        return Err(format!(
            "has {} bulge(s) for {} boundary vertices",
            bulges.len(),
            points.len()
        ));
    }
    if bulges.is_empty() {
        bulges = vec![0.0; points.len()];
    }
    let points = points
        .into_iter()
        .map(|point| plane.to_world(point))
        .collect();
    Ok((points, bulges))
}
