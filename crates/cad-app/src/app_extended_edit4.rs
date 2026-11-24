//! Wave 4 **real selection-based 2D edits** for the extended command batch.
//!
//! Twelve names that write entity rows (or create fresh entities) through the
//! existing planners and write paths — there is no second geometry engine here:
//!
//! * `CENTERMARK` / `CENTERLINE` create plain LINEs through the shared creation
//!   funnel (one transaction / one undo step per command).
//! * `NCOPY` copies the members of a selected block reference into model space
//!   with the placement transform baked in (the same walk EXPLODE uses, without
//!   deleting the source).
//! * `COPYTOLAYER` / `-COPYTOLAYER` copy the selection onto the first selected
//!   entity's layer as new ids; the originals are untouched.
//! * `CDORDER` rewrites stored draw order by colour index.
//! * `FLATTEN` rewrites the supported geometry's z to 0.
//! * `XPLODE` reuses the EXPLODE path for INSERTs and reports that it does not
//!   convert attribute values (`BURST` is the converting path).
//! * `BTRIM` / `BEXTEND` reuse `plan_trim` / `plan_extend` with the first
//!   selected INSERT's member geometry as the boundary.
//! * `CHANGE` / `CHPROP` apply a **typed property list** (layer, colour,
//!   linetype, lineweight, transparency) through the same `plan_entity` /
//!   `PropertyValue` surface `SETENTITYPROPERTY` uses; unsupported property names
//!   are refused by name. `property_ops` owns the handler.
//!
//! Every refusal happens **before** the transaction opens: nothing is written
//! and no history entry appears. A successful write is exactly one transaction
//! / one undo step.

use super::*;
use crate::app_drawing::{
    class_name, collect_boundaries, cut_tolerance, next_transaction_id, plan_entity, plan_extend,
    plan_trim, BoundarySet, CurveChange, PlanRefusal, TargetCurve, TrimPlan,
};
use crate::property_value::aci_for_rgb;
use cad_db::{transform_geometry, DbEntity, EntityColor};
use cad_geometry::{cross, dot, length, scale, sub};
use cad_history::{drawing_patch, DrawingPatch, DrawingUndoRecord};

mod block_ops;
mod property_ops;

const INSERT_SIMILARITY_TOLERANCE: f64 = 1e-9;
const PARALLEL_TOLERANCE: f64 = 1e-9;
const FLAT_TOLERANCE: f64 = 1e-9;

fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

/// One new top-level model-space entity a Wave 4 command will insert.
struct NewProduct {
    layer: LayerId,
    type_key: String,
    geometry: SemanticGeometry,
}

impl Application {
    /// Dispatch a Wave 4 real edit by its selector.
    pub(crate) fn extended_edit4(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::CenterMark => self.center_mark(session, command),
            ExtendedCommandId::CenterLine => self.center_line(session, command),
            ExtendedCommandId::NCopy => self.ncopy(session, command),
            ExtendedCommandId::CopyToLayer | ExtendedCommandId::DashCopyToLayer => {
                self.copy_to_layer(session, command, ext.selector())
            }
            ExtendedCommandId::CDOrder => self.cdorder(session, command),
            ExtendedCommandId::Flatten => self.flatten(session, command),
            ExtendedCommandId::XPlode => self.xplode(session, command),
            ExtendedCommandId::BTrim | ExtendedCommandId::BExtend => {
                self.block_trim_extend(session, command, ext)
            }
            ExtendedCommandId::Change | ExtendedCommandId::ChProp => {
                self.change_properties(session, command, ext)
            }
            other => Err(invalid(
                "extended4",
                "dispatch",
                format!("{} is not a Wave 4 edit", other.selector()),
            )),
        }
    }

    /// `CENTERMARK`: two crossing LINEs at each selected CIRCLE/ARC centre.
    ///
    /// The arm defaults to **10% of the source radius** (the previously fixed
    /// rule). The typed `CENTERMARK ARM <distance>` / `CENTERMARK <factor>x`
    /// option overrides it with an absolute length or a radius factor.
    fn center_mark(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let options = match &command.payload {
            CommandPayload::None => crate::CenterMarkCommand { arm: None },
            CommandPayload::Center(center) => match center {
                crate::CenterEditPayload::Mark(options) => *options,
                crate::CenterEditPayload::Line(_) => {
                    return Err(invalid(
                        "centermark",
                        "payload",
                        "CENTERMARK received the CENTERLINE option payload",
                    ))
                }
            },
            _ => {
                return Err(invalid(
                    "centermark",
                    "payload",
                    "CENTERMARK takes no payload or its typed arm option",
                ))
            }
        };
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("centermark", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "centermark", true)?;

        let mut geometries = Vec::with_capacity(entities.len() * 2);
        for entity in &entities {
            let (center, radius) = match &entity.geometry {
                SemanticGeometry::Circle { center, radius, .. }
                | SemanticGeometry::Arc { center, radius, .. } => (*center, *radius),
                other => {
                    return Err(unsupported(
                        "centermark",
                        "unsupported",
                        format!(
                            "entity {} is {}; CENTERMARK supports CIRCLE and ARC only",
                            entity.id.0,
                            class_name(other)
                        ),
                    ))
                }
            };
            if !radius.is_finite() || radius <= 0.0 {
                return Err(invalid(
                    "centermark",
                    "degenerate",
                    format!("entity {} has a non-positive radius", entity.id.0),
                ));
            }
            let arm = match options.arm {
                None => radius * 0.10,
                Some(crate::CenterArm::Absolute(value)) => value,
                Some(crate::CenterArm::Factor(factor)) => radius * factor,
            };
            if !arm.is_finite() || arm <= 0.0 {
                return Err(invalid(
                    "centermark",
                    "invalid_arm",
                    format!(
                        "entity {} would get the degenerate centre-mark arm {arm}; \
                         CENTERMARK needs a positive finite arm",
                        entity.id.0
                    ),
                ));
            }
            geometries.push(SemanticGeometry::Line {
                start: Point3 {
                    x: center.x - arm,
                    y: center.y,
                    z: center.z,
                },
                end: Point3 {
                    x: center.x + arm,
                    y: center.y,
                    z: center.z,
                },
            });
            geometries.push(SemanticGeometry::Line {
                start: Point3 {
                    x: center.x,
                    y: center.y - arm,
                    z: center.z,
                },
                end: Point3 {
                    x: center.x,
                    y: center.y + arm,
                    z: center.z,
                },
            });
        }
        let mut outcome = self.commit_new_entities(session, command, geometries, "centermark")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "centermark.applied".into(),
            message: match options.arm {
                None => "CENTERMARK: arm length is 10% of the source radius (the declared \
                          default; pass `CENTERMARK ARM <distance>` or `CENTERMARK <factor>x` \
                          to override)"
                    .into(),
                Some(crate::CenterArm::Absolute(value)) => {
                    format!("CENTERMARK: arm length {value} drawing units")
                }
                Some(crate::CenterArm::Factor(factor)) => {
                    format!("CENTERMARK: arm length is {factor} x the source radius")
                }
            },
        });
        Ok(outcome)
    }

    /// `CENTERLINE`: one midway LINE between two parallel selected LINEs.
    ///
    /// The new line spans the overlap of the two lines' projections onto their
    /// common direction, extended beyond both ends by `EXTENSION` and inset from
    /// both ends by `GAP`. Non-parallel or mixed selections are refused by name.
    fn center_line(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let options = match &command.payload {
            CommandPayload::None => crate::CenterLineCommand {
                extension: 0.0,
                gap: 0.0,
            },
            CommandPayload::Center(center) => match center {
                crate::CenterEditPayload::Line(options) => *options,
                crate::CenterEditPayload::Mark(_) => {
                    return Err(invalid(
                        "centerline",
                        "payload",
                        "CENTERLINE received the CENTERMARK option payload",
                    ))
                }
            },
            _ => {
                return Err(invalid(
                    "centerline",
                    "payload",
                    "CENTERLINE takes no payload or its typed extension/gap options",
                ))
            }
        };
        if !options.extension.is_finite()
            || !options.gap.is_finite()
            || options.extension < 0.0
            || options.gap < 0.0
        {
            return Err(invalid(
                "centerline",
                "invalid_option",
                format!(
                    "CENTERLINE extension and gap must be finite and non-negative, got \
                     extension {} gap {}",
                    options.extension, options.gap
                ),
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("centerline", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "centerline", true)?;
        if entities.len() != 2 {
            return Err(invalid(
                "centerline",
                "selection",
                format!(
                    "CENTERLINE needs exactly two selected LINEs, got {}",
                    entities.len()
                ),
            ));
        }
        let (a, b) =
            match (&entities[0].geometry, &entities[1].geometry) {
                (
                    SemanticGeometry::Line { start: a0, end: a1 },
                    SemanticGeometry::Line { start: b0, end: b1 },
                ) => ((*a0, *a1), (*b0, *b1)),
                _ => return Err(unsupported(
                    "centerline",
                    "unsupported",
                    "CENTERLINE supports exactly two LINEs; mixed or non-LINE selection refused",
                )),
            };
        let mut line = midway_parallel_line(a, b)?;
        extend_centerline(&mut line, options.extension, options.gap)?;
        let mut outcome = self.commit_new_entities(session, command, vec![line], "centerline")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "centerline.applied".into(),
            message: format!(
                "CENTERLINE: one midway LINE across the projection overlap with extension {} \
                 and gap {}",
                options.extension, options.gap
            ),
        });
        Ok(outcome)
    }

    /// `COPYTOLAYER` / `-COPYTOLAYER`: copy the remaining selection onto the
    /// first selected entity's layer as new ids (originals untouched).
    fn copy_to_layer(
        &mut self,
        session: &SessionState,
        command: &Command,
        selector: &str,
    ) -> CadResult<CommandOutcome> {
        let prefix = if selector.starts_with('-') {
            "-copytolayer"
        } else {
            "copytolayer"
        };
        require_none_payload(command, selector)?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid(prefix, "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, prefix, true)?;
        let target_layer = entities[0].layer;
        if entities.len() == 1 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: format!("{prefix}.unchanged"),
                    message: format!(
                        "{selector}: only one entity is selected; there is nothing to copy onto \
                         the target layer"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut products = Vec::with_capacity(entities.len() - 1);
        for entity in &entities[1..] {
            products.push(NewProduct {
                layer: target_layer,
                type_key: entity.object.type_key.clone(),
                geometry: entity.geometry.clone(),
            });
        }
        let mut outcome = self.insert_products(command, products, prefix)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: format!("{prefix}.partial"),
            message: format!(
                "{selector}: {} copies placed on layer {}; there is no layer prompt and block \
                 members/paper space are refused",
                outcome.objects.len(),
                drawing
                    .layer(target_layer)
                    .map(|layer| layer.name.as_str())
                    .unwrap_or("<unknown>")
            ),
        });
        Ok(outcome)
    }

    /// `CDORDER`: rewrite the selection's (or all model-space) draw order by
    /// colour index ascending.
    fn cdorder(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "CDORDER")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("cdorder", "not_addressable", "document not open"))?
            .drawing
            .clone();

        let mut entities = if session.selection.refs().is_empty() {
            drawing
                .entities()
                .filter(|entity| entity.space == SpaceId::Model)
                .cloned()
                .collect()
        } else {
            resolve_direct(session, &drawing, command, "cdorder", true)?
        };
        if entities.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "cdorder.unchanged".into(),
                    message: "CDORDER: there is no model-space entity to reorder".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }

        // Keep the exact set of draw-order slots the candidates occupy, sorting
        // only *among* them: a non-selected entity can never end up underneath a
        // reordered one by accident.
        let mut slots: Vec<i64> = entities.iter().map(|entity| entity.draw_order).collect();
        slots.sort_unstable();
        entities.sort_by_key(|entity| {
            (
                color_index(&drawing, entity),
                entity.draw_order,
                entity.id.0,
            )
        });

        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for (entity, order) in entities.into_iter().zip(slots) {
            if entity.draw_order == order {
                continue;
            }
            let mut after = entity.clone();
            after.draw_order = order;
            planned.push((entity, after));
        }
        if planned.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "cdorder.unchanged".into(),
                    message: "CDORDER: every entity already has the colour-index order".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let mut candidate = drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("cdorder", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (before, _) in planned {
            let after = candidate.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant("CDORDER: entity vanished after commit".into())
            })?;
            objects.push(ObjectId(before.id.0));
            patches.push(drawing_patch(before.id, Some(before), Some(after)));
        }
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "cdorder".into(),
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
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "cdorder.applied".into(),
                message: format!(
                    "CDORDER: rewrote the draw order of {count} entity/entities by \
                                  colour index"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `XPLODE`: route the selected INSERTs through the existing EXPLODE path.
    fn xplode(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "XPLODE")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("xplode", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "xplode", true)?;
        for entity in &entities {
            if !matches!(entity.geometry, SemanticGeometry::Insert { .. }) {
                return Err(unsupported(
                    "xplode",
                    "unsupported",
                    format!(
                        "entity {} is {}; XPLODE supports selected INSERT references only",
                        entity.id.0,
                        class_name(&entity.geometry)
                    ),
                ));
            }
        }
        let mut outcome = self.explode_entities(session, command)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "xplode.attributes_not_converted".into(),
            message: "XPLODE: uses the EXPLODE path; the reference's own attribute values are NOT CONVERTED (use BURST), and attribute/property control is not implemented"
                .into(),
        });
        Ok(outcome)
    }

    /// Insert a batch of new model-space entities in one transaction / undo step.
    fn insert_products(
        &mut self,
        command: &Command,
        products: Vec<NewProduct>,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        if products.is_empty() {
            return Err(invalid(label, "empty", "nothing to insert"));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid(label, "not_addressable", "document not open"))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let base_order = drawing
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let mut entities = Vec::with_capacity(products.len());
        for (index, product) in products.into_iter().enumerate() {
            if drawing.layer(product.layer).is_none() {
                return Err(invalid(
                    label,
                    "layer_missing",
                    format!("layer {} does not exist", product.layer.0),
                ));
            }
            let new_id = drawing.allocate_entity_id();
            entities.push(DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key: product.type_key,
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: product.layer,
                space: SpaceId::Model,
                geometry: product.geometry,
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
        let patches: Vec<DrawingPatch> = entities
            .iter()
            .map(|entity| drawing_patch(entity.id, None, Some(entity.clone())))
            .collect();
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
}

/// Resolve the selection to directly addressable entities in this document.
///
/// A block-instance member or sub-element has no directly addressable identity,
/// so the whole command is refused by name instead of silently editing the
/// member's block definition. When `require_model_space` is set, paper-space
/// rows are refused too.
fn resolve_direct(
    session: &SessionState,
    drawing: &DrawingDatabase,
    command: &Command,
    prefix: &str,
    require_model_space: bool,
) -> CadResult<Vec<DbEntity>> {
    let references = session.selection.refs();
    if references.is_empty() {
        return Err(invalid(
            prefix,
            "selection",
            format!("{} needs at least one selected object", prefix),
        ));
    }
    let mut seen: Vec<EntityId> = Vec::new();
    let mut entities = Vec::with_capacity(references.len());
    for reference in references {
        if reference.document != command.document {
            return Err(invalid(
                prefix,
                "not_addressable",
                format!("entity {} belongs to another document", reference.entity.0),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                prefix,
                "not_addressable",
                format!(
                    "entity {} is referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            ));
        }
        if seen.contains(&reference.entity) {
            continue;
        }
        seen.push(reference.entity);
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                prefix,
                "not_addressable",
                format!("entity {} does not exist", reference.entity.0),
            )
        })?;
        if require_model_space && entity.space != SpaceId::Model {
            return Err(unsupported(
                prefix,
                "not_addressable",
                format!("entity {} is not in model space", entity.id.0),
            ));
        }
        if matches!(entity.space, SpaceId::Block(_)) {
            return Err(unsupported(
                prefix,
                "not_addressable",
                format!("entity {} is a block-definition member", entity.id.0),
            ));
        }
        entities.push(entity);
    }
    Ok(entities)
}

/// Reject a non-empty payload: these commands take the selection as the input.
fn require_none_payload(command: &Command, name: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(invalid(
            &name.to_ascii_lowercase().replace(' ', "_"),
            "payload",
            format!("{name} takes no payload; the current selection is the input"),
        ))
    }
}

/// The midpoint line between two parallel 3D lines.
fn midway_parallel_line(a: (Point3, Point3), b: (Point3, Point3)) -> CadResult<SemanticGeometry> {
    let (a0, a1) = a;
    let (b0, b1) = b;
    let da = sub(a1, a0);
    let db = sub(b1, b0);
    let la = length(da);
    let lb = length(db);
    if la <= PARALLEL_TOLERANCE || lb <= PARALLEL_TOLERANCE {
        return Err(invalid(
            "centerline",
            "degenerate",
            "a selected LINE has zero length",
        ));
    }
    let ua = scale(da, 1.0 / la);
    let mut ub = scale(db, 1.0 / lb);
    if dot(ua, ub) < 0.0 {
        ub = scale(ub, -1.0);
    }
    if length(cross(ua, ub)) > PARALLEL_TOLERANCE {
        return Err(unsupported(
            "centerline",
            "not_parallel",
            "the two selected LINEs are not parallel within tolerance",
        ));
    }
    // Offset from line A to line B, perpendicular to A, then halved.
    let to_b = sub(b0, a0);
    let along = dot(to_b, ua);
    let offset = sub(to_b, scale(ua, along));
    let half = scale(offset, 0.5);

    let ta0: f64 = 0.0;
    let ta1: f64 = la;
    let tb0 = along;
    let tb1 = along + dot(db, ua);
    let start_t = ta0.min(ta1).max(tb0.min(tb1));
    let end_t = ta0.max(ta1).min(tb0.max(tb1));
    if end_t <= start_t + PARALLEL_TOLERANCE {
        return Err(invalid(
            "centerline",
            "no_overlap",
            "the two selected LINEs have no overlapping projection",
        ));
    }
    Ok(SemanticGeometry::Line {
        start: point_add(scale(ua, start_t), point_add(a0, half)),
        end: point_add(scale(ua, end_t), point_add(a0, half)),
    })
}

fn point_add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Apply `CENTERLINE`'s `EXTENSION` (outward) and `GAP` (inward) to the midway
/// line: both ends move by `extension - gap` along the line direction, so the
/// span grows by `2 × (extension - gap)`.
fn extend_centerline(line: &mut SemanticGeometry, extension: f64, gap: f64) -> CadResult<()> {
    let SemanticGeometry::Line { start, end } = line else {
        return Err(CadError::Invariant(
            "CENTERLINE lost its midway LINE before the option adjustment".into(),
        ));
    };
    let direction = sub(*end, *start);
    let span = length(direction);
    if span <= PARALLEL_TOLERANCE {
        return Err(invalid(
            "centerline",
            "degenerate",
            "the midway centreline has zero length",
        ));
    }
    let net = extension - gap;
    if !net.is_finite() {
        return Err(invalid(
            "centerline",
            "invalid_option",
            "the CENTERLINE extension becomes non-finite",
        ));
    }
    let step = scale(scale(direction, 1.0 / span), net);
    let new_start = sub(*start, step);
    let new_end = point_add(*end, step);
    // Both ends move outward by `net`, so the signed span is `span + 2*net`.
    // A negative or zero result means the gap consumed (and would invert) the
    // line; refusing here avoids storing a backwards LINE.
    if span + 2.0 * net <= PARALLEL_TOLERANCE {
        return Err(invalid(
            "centerline",
            "degenerate",
            "the CENTERLINE gap consumes the whole maintained span",
        ));
    }
    *start = new_start;
    *end = new_end;
    Ok(())
}

/// The colour index used by `CDORDER` (0 = ByBlock, 256 = non-palette true
/// colour, 7 = the default when a layer carries no row).
fn color_index(drawing: &DrawingDatabase, entity: &DbEntity) -> u16 {
    match drawing.entity_render_attributes(entity.id).color {
        EntityColor::ByBlock => 0,
        EntityColor::Explicit(rgb) => aci_for_rgb(rgb).unwrap_or(256),
        EntityColor::ByLayer => drawing
            .layer_attributes_ref(entity.layer)
            .and_then(|attributes| {
                attributes
                    .color
                    .aci
                    .map(u16::from)
                    .or_else(|| aci_for_rgb(attributes.color.rgb))
            })
            .unwrap_or(7),
    }
}
