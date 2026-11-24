//! `HATCHGENERATEBOUNDARY` and `HATCHTOBACK` (AutoCAD 2026 hatch utilities).
//!
//! Two Work-only commands that act on stored hatch entities without changing the
//! hatches' geometry:
//!
//! * [`crate::CommandId::HatchGenerateBoundary`] — create one **non-associative**
//!   closed polyline per hatch loop around the selected hatches, on each hatch's
//!   own layer and space, in one transaction / one undo step. The source hatches
//!   are retained.
//! * [`crate::CommandId::HatchToBack`] — move every hatch behind every non-hatch
//!   object **of its own draw-order scope** (model space, one paper layout, or
//!   one block definition) by rewriting only the stored `draw_order`, in one
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
    /// be a directly addressable `SemanticGeometry::Hatch` in **model space or a
    /// paper layout**; a block-definition member, a sub-element, a missing ref or
    /// another document's ref is refused by name before a write. Each loop is
    /// mapped from the hatch's own OCS 2D frame through [`HatchPlane::to_world`]
    /// — the same arbitrary-axis projection the fill/display uses — and stored as
    /// an `AcDbPolyline` with the loop's bulges and the closed flag. A repeated
    /// last point is dropped (with the degenerate closing bulge when the source
    /// carried one) before the world mapping.
    ///
    /// The boundary polylines inherit the source hatch's layer **and space** (a
    /// paper-layout hatch produces paper-layout boundaries through the same
    /// `insert_entity` path layouts already use), and the draw order is the next
    /// one in that space's own sequence. They are a snapshot, not an associative
    /// boundary: editing the hatch afterwards does not update them, and
    /// associativity remains a global (run-wide) declaration rather than a
    /// per-object record.
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
            match &source.space {
                SpaceId::Model | SpaceId::Paper(_) => {}
                SpaceId::Block(_) => {
                    return Err(not_addressable(
                        reference.entity,
                        "it is inside a block definition; the boundary is written for model \
                         space or a paper layout only",
                    ))
                }
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

        // 2. One draw-order run after the highest row **of the same space**,
        //    exactly like the shared create funnel groups model/layout/block
        //    sequences (a new paper boundary does not inherit a model order).
        let mut next_orders: std::collections::BTreeMap<DrawOrderScope, i64> =
            std::collections::BTreeMap::new();
        for entity in drawing.entities() {
            let scope = DrawOrderScope::of(&entity.space);
            let entry = next_orders.entry(scope).or_insert(0);
            *entry = (*entry).max(entity.draw_order.saturating_add(1));
        }
        let mut entities = Vec::with_capacity(planned.len());
        for (layer, space, geometry) in planned.into_iter() {
            let entity_id = drawing.allocate_entity_id();
            let scope = DrawOrderScope::of(&space);
            let order = next_orders.entry(scope).or_insert(0);
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
                draw_order: *order,
            });
            *order = order.saturating_add(1);
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

    /// `HATCHTOBACK`: put every hatch behind all other objects **of its own
    /// draw-order scope** by rewriting their stored `draw_order` in one
    /// transaction / one undo step.
    ///
    /// A scope is everything that shares one draw-order sequence: model space,
    /// one paper layout, or one block definition (`SpaceId`, the same field every
    /// consumer of `DbEntity::draw_order` already groups by — no second ordering
    /// mechanism). Each scope is planned independently: the hatches in it keep
    /// their relative order and receive the contiguous block
    /// `min_non_hatch_order - n ..= min_non_hatch_order - 1`. A scope with no
    /// hatch, or whose hatches are already strictly below its lowest non-hatch
    /// order, is reported and left alone (a no-op, never an error). The
    /// arithmetic is checked per scope, so an `i64` underflow is the explicit
    /// `hatchtoback.order_exhausted` refusal for the whole run — every scope is
    /// planned before anything is staged, so a refusal writes nothing.
    ///
    /// Nothing at all is staged when no scope has a hatch (`hatchtoback.none`) or
    /// when every hatch is already behind (`hatchtoback.unchanged`); the
    /// diagnostic lists what happened per space/block either way.
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

        // 1. Group every row by the draw-order sequence it belongs to, then plan
        //    every scope before staging anything (a refusal writes nothing).
        let mut scopes: std::collections::BTreeMap<DrawOrderScope, Vec<DbEntity>> =
            std::collections::BTreeMap::new();
        for entity in drawing.entities() {
            scopes
                .entry(DrawOrderScope::of(&entity.space))
                .or_default()
                .push(entity.clone());
        }
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        let mut reports: Vec<ScopeReport> = Vec::new();
        for (scope, rows) in scopes {
            let plan = plan_scope_behind(&rows)?;
            let hatches = rows.iter().filter(|row| is_hatch(row)).count();
            reports.push(ScopeReport {
                label: scope.label(drawing),
                hatches,
                moved: plan.len(),
            });
            planned.extend(plan);
        }
        if reports.iter().all(|report| report.hatches == 0) {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "hatchtoback.none".into(),
                    message: "HATCHTOBACK: no hatches in the drawing".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(&reports));
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
                message: format!(
                    "HATCHTOBACK: moved {moved} hatch(es) behind every other object — {}",
                    describe(&reports)
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// One draw-order scope: a `SpaceId` without its `Ord`/`Hash` (the domain enum
/// has neither), so the scopes iterate in a stable model → layout → block order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DrawOrderScope {
    Model,
    Paper(LayoutId),
    Block(BlockId),
}

impl DrawOrderScope {
    fn of(space: &SpaceId) -> Self {
        match space {
            SpaceId::Model => DrawOrderScope::Model,
            SpaceId::Paper(layout) => DrawOrderScope::Paper(*layout),
            SpaceId::Block(block) => DrawOrderScope::Block(*block),
        }
    }

    /// The report label: the space's own name where the drawing has one.
    fn label(self, drawing: &DrawingDatabase) -> String {
        match self {
            DrawOrderScope::Model => "model space".into(),
            DrawOrderScope::Paper(layout) => match drawing.layout(layout) {
                Some(row) if !row.name.trim().is_empty() => format!("layout '{}'", row.name),
                _ => format!("layout #{}", layout.0),
            },
            DrawOrderScope::Block(block) => match drawing.block_name(block) {
                Some(name) if !name.trim().is_empty() => format!("block '{name}'"),
                _ => format!("block #{}", block.0),
            },
        }
    }
}

/// What one scope did, for the run's single diagnostic.
struct ScopeReport {
    label: String,
    hatches: usize,
    moved: usize,
}

fn describe(reports: &[ScopeReport]) -> String {
    let parts: Vec<String> = reports
        .iter()
        .map(|report| {
            if report.hatches == 0 {
                format!("{}: no hatch", report.label)
            } else if report.moved == 0 {
                format!(
                    "{}: {} hatch(es) already behind",
                    report.label, report.hatches
                )
            } else {
                format!("{}: {} moved", report.label, report.moved)
            }
        })
        .collect();
    parts.join("; ")
}

fn is_hatch(entity: &DbEntity) -> bool {
    matches!(entity.geometry, SemanticGeometry::Hatch(_))
}

/// The reorder plan for one draw-order scope: its hatches run contiguous and in
/// relative order, immediately below its lowest non-hatch row.
///
/// Returns the changed rows only (an already-correct hatch is skipped), so an
/// empty plan means "nothing to do in this scope". The `i64` arithmetic is
/// checked: `count` and `min_order - count` are the two refusals.
fn plan_scope_behind(rows: &[DbEntity]) -> CadResult<Vec<(DbEntity, DbEntity)>> {
    let mut hatches: Vec<&DbEntity> = rows.iter().filter(|row| is_hatch(row)).collect();
    if hatches.is_empty() {
        return Ok(Vec::new());
    }
    let min_order = rows
        .iter()
        .filter(|row| !is_hatch(row))
        .map(|row| row.draw_order)
        .min();
    let Some(min_order) = min_order else {
        // Nothing but hatches: each hatch is already behind every non-hatch.
        return Ok(Vec::new());
    };
    if hatches.iter().all(|row| row.draw_order < min_order) {
        return Ok(Vec::new());
    }
    let count = i64::try_from(hatches.len()).map_err(|_| order_exhausted())?;
    let first = min_order.checked_sub(count).ok_or_else(order_exhausted)?;
    // `first` is the lowest target and `first + (n-1) = min_order - 1` is the
    // highest; both are representable once `min_order - n` is.
    hatches.sort_by_key(|row| (row.draw_order, row.id.0));
    let mut plan = Vec::with_capacity(hatches.len());
    for (index, before) in hatches.into_iter().enumerate() {
        let target = first + index as i64;
        if before.draw_order == target {
            continue;
        }
        let mut after = before.clone();
        after.draw_order = target;
        plan.push((before.clone(), after));
    }
    Ok(plan)
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
        "the target draw orders would underflow i64 in at least one space or block",
    )
}

fn unchanged_outcome(reports: &[ScopeReport]) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "hatchtoback.unchanged".into(),
            message: format!(
                "HATCHTOBACK: every hatch is already behind every other object of its own space \
                 — {}",
                describe(reports)
            ),
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
