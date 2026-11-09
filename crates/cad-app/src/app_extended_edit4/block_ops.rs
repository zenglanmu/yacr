//! Block-member and flatten helpers for the Wave 4 extended edits.
//!
//! Split out of `app_extended_edit4.rs` to keep every file inside the 1000-line
//! code gate. The parent module routes NCOPY/FLATTEN/BTRIM/BEXTEND here; the
//! geometry math is still the shared `transform_geometry` surface and the
//! shared TRIM/EXTEND planner.

use super::*;
use cad_history::DrawingUndoRecord;

impl Application {
    /// `NCOPY`: copy the member geometry of selected INSERTs into model space.
    pub(super) fn ncopy(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "NCOPY")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("ncopy", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "ncopy", true)?;

        let mut products = Vec::new();
        for entity in &entities {
            let SemanticGeometry::Insert { block, transform } = &entity.geometry else {
                return Err(unsupported(
                    "ncopy",
                    "unsupported",
                    format!(
                        "entity {} is {}; NCOPY requires selected INSERT references",
                        entity.id.0,
                        class_name(&entity.geometry)
                    ),
                ));
            };
            for (layer, geometry) in insert_member_geometry(&drawing, *block, transform, "ncopy")? {
                products.push(NewProduct {
                    layer,
                    type_key: class_name(&geometry).to_string(),
                    geometry,
                });
            }
        }
        if products.is_empty() {
            return Err(invalid(
                "ncopy",
                "empty",
                "the selected block reference has no visible member geometry to copy",
            ));
        }
        let mut outcome = self.insert_products(command, products, "ncopy")?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "ncopy.partial".into(),
            message:
                "NCOPY: member geometry copied at its world transform; nested-selection UI is \
                      NOT IMPLEMENTED"
                    .into(),
        });
        Ok(outcome)
    }

    /// `FLATTEN`: set the supported selected geometry's z to 0.
    pub(super) fn flatten(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "FLATTEN")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("flatten", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "flatten", false)?;

        let mut planned: Vec<(DbEntity, SemanticGeometry)> = Vec::new();
        for entity in entities {
            let geometry = flatten_geometry(&entity.geometry)?;
            if geometry == entity.geometry {
                continue;
            }
            planned.push((entity, geometry));
        }
        if planned.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "flatten.unchanged".into(),
                    message: "FLATTEN: every selected entity is already at z = 0".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let mut candidate = drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("flatten", transaction)?;
            for (before, geometry) in &planned {
                let mut after = before.clone();
                after.geometry = geometry.clone();
                staged.update_entity(after)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (before, _) in planned {
            let after = candidate.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant("FLATTEN: entity vanished after commit".into())
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
                label: "flatten".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "flatten.applied".into(),
                message: format!(
                    "FLATTEN: set z = 0 on {count} entity/entities (Express-tool behaviour \
                     reduced to z = 0)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `BTRIM` / `BEXTEND`: trim/extend the selection against the first selected
    /// INSERT's member geometry.
    pub(super) fn block_trim_extend(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let (prefix, label) = if ext == ExtendedCommandId::BTrim {
            ("btrim", "BTRIM")
        } else {
            ("bextend", "BEXTEND")
        };
        require_none_payload(command, label)?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid(prefix, "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, prefix, true)?;
        if entities.len() < 2 {
            return Err(invalid(
                prefix,
                "selection",
                format!(
                    "{label} needs the first selected INSERT boundary plus at least one target, \
                     got {}",
                    entities.len()
                ),
            ));
        }
        let SemanticGeometry::Insert { block, transform } = &entities[0].geometry else {
            return Err(unsupported(
                prefix,
                "boundary",
                format!(
                    "the first selected entity is {}; {label} needs an INSERT boundary",
                    class_name(&entities[0].geometry)
                ),
            ));
        };

        let mut boundaries = BoundarySet::default();
        let policy = TolerancePolicy::default();
        for (_, geometry) in insert_member_geometry(&drawing, *block, transform, prefix)? {
            collect_boundaries(&geometry, &mut boundaries, &policy, label).map_err(|error| {
                match error {
                    CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                        unsupported(prefix, "boundary", message)
                    }
                    other => other,
                }
            })?;
        }
        if boundaries.is_empty() {
            return Err(unsupported(
                prefix,
                "boundary",
                "the selected INSERT has no member geometry that can bound a cut",
            ));
        }

        let tol = cut_tolerance();
        let mut changes = Vec::new();
        let diagnostics = Vec::new();
        for target in &entities[1..] {
            let curve = TargetCurve::new(&target.geometry, label, "a LINE, ARC, CIRCLE or SPLINE")
                .map_err(|error| match error {
                    CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                        unsupported(prefix, "target", message)
                    }
                    other => other,
                })?;
            let resolution = if ext == ExtendedCommandId::BTrim {
                plan_trim(&curve, curve.point_at(0.5), &boundaries, &policy, tol)
                    .map(|resolution| resolution.plan)
                    .map_err(|reason| invalid(prefix, "no_cut", plan_reason(reason)))?
            } else {
                plan_extend(&curve, curve.point_at(1.0), &boundaries, &policy, tol)
                    .map(|resolution| TrimPlan::Replace(resolution.geometry))
                    .map_err(|reason| invalid(prefix, "no_boundary", reason))?
            };
            match resolution {
                TrimPlan::Replace(geometry) => {
                    changes.push(CurveChange::Replace(target.clone(), geometry))
                }
                TrimPlan::Delete => changes.push(CurveChange::Delete(
                    target.clone(),
                    drawing.entity_display_state(target.id),
                )),
            }
        }
        let mut outcome = self.commit_curve_changes(document_id, changes, diagnostics, prefix)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: format!("{prefix}.partial"),
            message: format!(
                "{label}: synthetic boundary from the selected INSERT members; no interactive \
                 boundary prompt"
            ),
        });
        Ok(outcome)
    }
}

/// The message inside a plan-level refusal (the variant is preserved at the
/// planner boundary, so this only names the reason for the command diagnostic).
fn plan_reason(reason: PlanRefusal) -> String {
    match reason {
        PlanRefusal::Invalid(message) | PlanRefusal::Unsupported(message) => message,
    }
}

/// Copy one INSERT's visible member geometry into model space, baking the
/// placement transform.
fn insert_member_geometry(
    drawing: &DrawingDatabase,
    block: BlockId,
    transform: &Transform3,
    prefix: &str,
) -> CadResult<Vec<(LayerId, SemanticGeometry)>> {
    if !transform.is_uniform_scale(INSERT_SIMILARITY_TOLERANCE) {
        return Err(unsupported(
            prefix,
            "unsupported",
            "a non-uniformly scaled INSERT cannot be copied exactly",
        ));
    }
    let definition = drawing.block(block).ok_or_else(|| {
        unsupported(
            prefix,
            "unsupported",
            format!("block {block:?} is not in the database"),
        )
    })?;
    let members = match drawing.block_visible_entities(block) {
        Some(visible) => visible,
        None => definition.entities.clone(),
    };
    let mut out = Vec::with_capacity(members.len());
    for member in members {
        let child = drawing.entity(member).ok_or_else(|| {
            unsupported(
                prefix,
                "unsupported",
                format!("block member {} is not in the database", member.0),
            )
        })?;
        let geometry = match &child.geometry {
            SemanticGeometry::Insert {
                block: nested,
                transform: nested_transform,
            } => SemanticGeometry::Insert {
                block: *nested,
                transform: transform.matrix_mul(nested_transform),
            },
            other => transform_geometry(other, transform).map_err(|error| {
                unsupported(
                    prefix,
                    "unsupported",
                    format!(
                        "block member {} ({}) cannot be placed: {error}",
                        member.0,
                        class_name(other)
                    ),
                )
            })?,
        };
        out.push((child.layer, geometry));
    }
    Ok(out)
}

/// Rewrite one supported geometry's z coordinate to 0.
fn flatten_geometry(geometry: &SemanticGeometry) -> CadResult<SemanticGeometry> {
    let flat = |point: Point3| Point3 {
        x: point.x,
        y: point.y,
        z: 0.0,
    };
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(SemanticGeometry::Line {
            start: flat(*start),
            end: flat(*end),
        }),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => Ok(SemanticGeometry::Polyline {
            points: points.iter().map(|point| flat(*point)).collect(),
            bulges: bulges.clone(),
            closed: *closed,
        }),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let normal = flatten_normal(normal, "CIRCLE")?;
            Ok(SemanticGeometry::Circle {
                center: flat(*center),
                normal,
                radius: *radius,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let normal = flatten_normal(normal, "ARC")?;
            Ok(SemanticGeometry::Arc {
                center: flat(*center),
                normal,
                radius: *radius,
                start: *start,
                sweep: *sweep,
            })
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let normal = flatten_normal(normal, "ELLIPSE")?;
            let mut major_axis = *major_axis;
            major_axis.z = 0.0;
            Ok(SemanticGeometry::Ellipse {
                center: flat(*center),
                normal,
                major_axis,
                ratio: *ratio,
                start: *start,
                sweep: *sweep,
            })
        }
        SemanticGeometry::Point(point) => Ok(SemanticGeometry::Point(flat(*point))),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => Ok(SemanticGeometry::Spline {
            degree: *degree,
            knots: knots.clone(),
            control_points: control_points.iter().map(|point| flat(*point)).collect(),
            weights: weights.clone(),
        }),
        SemanticGeometry::Mesh(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "MESH geometry is 3D and cannot be flattened to z = 0",
        )),
        SemanticGeometry::Insert { .. } => Err(unsupported(
            "flatten",
            "unsupported",
            "INSERT references are refused by name; flatten the block members instead",
        )),
        other => Err(unsupported(
            "flatten",
            "unsupported",
            format!("{} is not a flattenable 2D geometry", class_name(other)),
        )),
    }
}

fn flatten_normal(normal: &Point3, kind: &str) -> CadResult<Point3> {
    let len = length(*normal);
    if len <= FLAT_TOLERANCE {
        return Err(invalid(
            "flatten",
            "degenerate",
            format!("the {kind} has a zero-length normal"),
        ));
    }
    if normal.x.abs() > FLAT_TOLERANCE || normal.y.abs() > FLAT_TOLERANCE {
        return Err(unsupported(
            "flatten",
            "unsupported",
            format!("the {kind} has a non-Z normal (3D) and cannot be flattened"),
        ));
    }
    let sign = if normal.z < 0.0 { -1.0 } else { 1.0 };
    Ok(Point3 {
        x: 0.0,
        y: 0.0,
        z: sign,
    })
}
