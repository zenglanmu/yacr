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
            let SemanticGeometry::Insert {
                block, transform, ..
            } = &entity.geometry
            else {
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
            let geometry = flatten_geometry(&drawing, &entity.geometry, &mut Vec::new())?;
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
                code: "flatten.applied".into(),
                message: format!(
                    "FLATTEN: set z = 0 on {count} entity/entities (Express-tool behaviour \
                     reduced to z = 0); INSERT references move their insertion z to 0 without \
                     editing the block definition, and a reference whose members are not already \
                     flat is refused by name"
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
        let SemanticGeometry::Insert {
            block, transform, ..
        } = &entities[0].geometry
        else {
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
            // The pick channel: TRIM/BEXTEND have no interactive pick, so the
            // target's own curve supplies one deterministic point — the arc-length
            // midpoint for a TRIM (the point a user would click to select the
            // object, exactly like `EXTRIM`), the far end for an EXTEND (the end
            // the planner would grow from an end pick).
            let pick = if ext == ExtendedCommandId::BTrim {
                trim_pick(&target.geometry, prefix)?
            } else {
                extend_pick(&target.geometry, prefix)?
            };
            let plan = if ext == ExtendedCommandId::BTrim {
                plan_trim_target(&target.geometry, pick, &boundaries, &policy, tol)
            } else {
                plan_extend_target(&target.geometry, pick, &boundaries, &policy, tol)
            }
            .map_err(|error| match error {
                CadError::Unsupported(message) | CadError::InvalidInput(message) => {
                    unsupported(prefix, "target", message)
                }
                other => other,
            })?;
            match plan.plan {
                TrimPlan::Replace(geometry) => {
                    changes.push(CurveChange::Replace(target.clone(), *geometry))
                }
                TrimPlan::Delete => changes.push(CurveChange::Delete(
                    target.clone(),
                    drawing.entity_display_state(target.id),
                )),
            }
        }
        let standalone = if ext == ExtendedCommandId::BTrim {
            "TRIM"
        } else {
            "EXTEND"
        };
        let mut outcome = self.commit_curve_changes(document_id, changes, diagnostics, prefix)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: format!("{prefix}.applied"),
            message: format!(
                "{label}: the boundary is the selected INSERT's member geometry and every target \
                 goes through the same planner and target set as the standalone {standalone} \
                 command; no interactive boundary prompt (global scope)"
            ),
        });
        Ok(outcome)
    }
}

/// The deterministic TRIM pick for one block-boundary target.
///
/// The target kinds are exactly `TRIM`'s own (the shared planner names any other
/// kind), so this only computes the pick: the arc-length midpoint of an
/// LWPOLYLINE, the midpoint of a HATCH boundary loop, or the parameter midpoint
/// of the single-curve targets.
fn trim_pick(geometry: &SemanticGeometry, prefix: &str) -> CadResult<Point3> {
    if let Some(error) = trim_target_gap(geometry) {
        return Err(map_target_error(prefix, error));
    }
    match geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => Ok(best_effort_midpoint(points, bulges, *closed)),
        SemanticGeometry::Hatch(hatch) => Ok(hatch_pick(hatch)),
        other => TargetCurve::new_trim_extend(
            other,
            "BTRIM",
            "a LINE, ARC, CIRCLE, ELLIPSE, SPLINE or LWPOLYLINE",
        )
        .map(|curve| curve.point_at(0.5))
        .map_err(|error| map_target_error(prefix, error)),
    }
}

/// The deterministic EXTEND pick for one block-boundary target.
///
/// The target kinds are exactly `EXTEND`'s own (the shared planner refuses a
/// closed target and names every kind it cannot extend); the pick is the target's
/// far end, the point an end pick would project onto.
fn extend_pick(geometry: &SemanticGeometry, prefix: &str) -> CadResult<Point3> {
    if let Some(error) = extend_target_gap(geometry) {
        return Err(map_target_error(prefix, error));
    }
    TargetCurve::new_trim_extend(
        geometry,
        "BEXTEND",
        "a LINE, ARC, ELLIPSE, CIRCLE or SPLINE",
    )
    .map(|curve| curve.point_at(1.0))
    .map_err(|error| map_target_error(prefix, error))
}

/// The arc-length midpoint of a stored polyline, or its first vertex when the
/// polyline is degenerate (the shared planner then names the real refusal, e.g.
/// `trim.polyline_degenerate`; a placeholder pick is never used to decide a valid
/// trim because a valid polyline always has a midpoint).
fn best_effort_midpoint(points: &[Point3], bulges: &[f64], closed: bool) -> Point3 {
    polyline_midpoint(points, bulges, closed)
        .ok()
        .or_else(|| points.first().copied())
        .unwrap_or(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })
}

/// The midpoint of a HATCH boundary loop, or its first world vertex when the loop
/// is degenerate; the hatch planner owns the named refusal either way.
fn hatch_pick(hatch: &cad_domain::HatchGeometry) -> Point3 {
    let plane = hatch.plane();
    let Some(loop_) = hatch.loops().first() else {
        return Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    };
    let points: Vec<Point3> = loop_
        .points
        .iter()
        .map(|point| plane.to_world(*point))
        .collect();
    best_effort_midpoint(&points, &loop_.bulges, true)
}

/// Prefix a shared-planner target refusal with the command name, keeping the
/// planner's own stable reason string as the detail.
fn map_target_error(prefix: &str, error: CadError) -> CadError {
    match error {
        CadError::Unsupported(message) => unsupported(prefix, "target", message),
        CadError::InvalidInput(message) => invalid(prefix, "target", message),
        other => other,
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
                attributes: nested_attributes,
            } => SemanticGeometry::Insert {
                block: *nested,
                transform: transform.matrix_mul(nested_transform),
                attributes: nested_attributes.clone(),
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
///
/// The representable 2D cases are the analytic 2D primitives plus the
/// annotations whose placement is a single world point (`TEXT`, `SHAPE`), the
/// world-space `MASK` polygon, an `IMAGE` already parallel to world XY, a
/// `COMPOUND` whose members all flatten, and an upright `INSERT` whose block
/// members are already flat (only its insertion z and its attribute text z
/// move; the block definition is never edited). Anything whose flat form would
/// need projected/derived geometry (`Dimension`/`Hatch`/`MultiLeader`/`Table`
/// display caches) or that is genuinely 3D (`MESH`, `Body3d`, opaque proxies) is
/// refused **by name** — never faked into a half-flat entity.
fn flatten_geometry(
    drawing: &DrawingDatabase,
    geometry: &SemanticGeometry,
    visiting: &mut Vec<BlockId>,
) -> CadResult<SemanticGeometry> {
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
            ..
        } => Ok(SemanticGeometry::Polyline {
            points: points.iter().map(|point| flat(*point)).collect(),
            bulges: bulges.clone(),
            closed: *closed,
            width: 0.0,
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
            closed,
        } => Ok(SemanticGeometry::Spline {
            degree: *degree,
            knots: knots.clone(),
            control_points: control_points.iter().map(|point| flat(*point)).collect(),
            weights: weights.clone(),
            // Flattening maps the control polygon, so it keeps the closedness.
            closed: *closed,
        }),
        SemanticGeometry::Text { .. } => {
            let mut out = geometry.clone();
            if let SemanticGeometry::Text { position, .. } = &mut out {
                *position = flat(*position);
            }
            Ok(out)
        }
        SemanticGeometry::Shape { .. } => {
            let mut out = geometry.clone();
            if let SemanticGeometry::Shape { position, .. } = &mut out {
                *position = flat(*position);
            }
            Ok(out)
        }
        SemanticGeometry::Mask { .. } => {
            let mut out = geometry.clone();
            if let SemanticGeometry::Mask { boundary, .. } = &mut out {
                for point in boundary.iter_mut() {
                    *point = flat(*point);
                }
            }
            Ok(out)
        }
        SemanticGeometry::Image { u, v, .. } => {
            // Only an image already parallel to world XY has a z-only flat form;
            // tilting `u`/`v` would be a projection, which this command does not
            // invent.
            if u.z.abs() > FLAT_TOLERANCE || v.z.abs() > FLAT_TOLERANCE {
                return Err(unsupported(
                    "flatten",
                    "unsupported",
                    "the IMAGE plane is not parallel to world XY; flattening it would need a                      projection, which is not implemented",
                ));
            }
            let mut out = geometry.clone();
            if let SemanticGeometry::Image { origin, u, v, .. } = &mut out {
                *origin = flat(*origin);
                u.z = 0.0;
                v.z = 0.0;
            }
            Ok(out)
        }
        SemanticGeometry::Compound(members) => {
            let flattened = members
                .iter()
                .map(|member| flatten_geometry(drawing, member, visiting))
                .collect::<CadResult<Vec<_>>>()?;
            Ok(SemanticGeometry::Compound(flattened))
        }
        SemanticGeometry::Insert {
            block,
            transform,
            attributes,
        } => flatten_insert(drawing, *block, transform, attributes, visiting),
        SemanticGeometry::Mesh(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "MESH geometry is 3D and cannot be flattened to z = 0",
        )),
        SemanticGeometry::Body3d(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "3D body geometry is not a 2D case; FLATTEN does not project B-rep solids",
        )),
        SemanticGeometry::Dimension(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "AcDbDimension's display cache is derived; this build has no dimension rebuild that would keep the annotation correct after a z rewrite",
        )),
        SemanticGeometry::Hatch(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "AcDbHatch's fill/pattern display is derived in the hatch plane; this build has no hatch rebuild after a z rewrite",
        )),
        SemanticGeometry::MultiLeader(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "MULTILEADER's leader/text display is derived from its plane frame; this build has no multileader rebuild after a z rewrite",
        )),
        SemanticGeometry::Table(_) => Err(unsupported(
            "flatten",
            "unsupported",
            "AcDbTable has no z-flatten model in this build",
        )),
        other => Err(unsupported(
            "flatten",
            "unsupported",
            format!("{} is not a flattenable 2D geometry", class_name(other)),
        )),
    }
}

/// Flatten an INSERT reference: zero its insertion z and its attribute text z.
///
/// The block definition's own members are **not** edited (they can be shared by
/// every reference). The reference is therefore only flattenable when its
/// placement keeps the block's XY plane parallel to world XY and every visible
/// member is already flat; otherwise the command refuses by name instead of
/// leaving a half-flat reference behind.
fn flatten_insert(
    drawing: &DrawingDatabase,
    block: BlockId,
    transform: &Transform3,
    attributes: &[cad_domain::AttributeValue],
    visiting: &mut Vec<BlockId>,
) -> CadResult<SemanticGeometry> {
    let m = &transform.matrix;
    let near_zero = |value: f64| value.abs() <= FLAT_TOLERANCE;
    if !near_zero(m[2][0]) || !near_zero(m[2][1]) || !near_zero(m[0][2]) || !near_zero(m[1][2]) {
        return Err(unsupported(
            "flatten",
            "unsupported",
            "the INSERT is rotated out of the world XY plane; only an upright placement has a z-only flat form",
        ));
    }
    if !m[2][2].is_finite() || near_zero(m[2][2]) {
        return Err(unsupported(
            "flatten",
            "unsupported",
            "the INSERT's z scale is zero or non-finite; its placement cannot be flattened",
        ));
    }
    if !insert_members_flat(drawing, block, visiting)? {
        return Err(unsupported(
            "flatten",
            "unsupported",
            format!(
                "block {block:?} has visible member geometry away from z = 0; the block \
                 definition is never edited, so the reference would stay non-flat"
            ),
        ));
    }
    let mut transform = *transform;
    transform.matrix[2][3] = 0.0;
    let mut flattened_attributes = Vec::with_capacity(attributes.len());
    for value in attributes {
        let mut value = value.clone();
        value.text.position.z = 0.0;
        flattened_attributes.push(value);
    }
    Ok(SemanticGeometry::Insert {
        block,
        transform,
        attributes: flattened_attributes,
    })
}

/// Whether every visible member of `block` is already at z = 0 (recursively,
/// with a cycle guard). A member kind FLATTEN cannot express is a named refusal
/// rather than "not flat".
fn insert_members_flat(
    drawing: &DrawingDatabase,
    block: BlockId,
    visiting: &mut Vec<BlockId>,
) -> CadResult<bool> {
    if visiting.contains(&block) {
        return Err(unsupported(
            "flatten",
            "unsupported",
            format!("block {block:?} is recursive; there is no finite flattening"),
        ));
    }
    let definition = drawing.block(block).ok_or_else(|| {
        unsupported(
            "flatten",
            "unsupported",
            format!("block {block:?} is not in the database"),
        )
    })?;
    let members = match drawing.block_visible_entities(block) {
        Some(visible) => visible,
        None => definition.entities.clone(),
    };
    visiting.push(block);
    let result = (|| {
        for member in members {
            let child = drawing.entity(member).ok_or_else(|| {
                unsupported(
                    "flatten",
                    "unsupported",
                    format!("block member {} is not in the database", member.0),
                )
            })?;
            let flattened = flatten_geometry(drawing, &child.geometry, visiting).map_err(
                |error| match error {
                    CadError::Unsupported(message) => unsupported(
                        "flatten",
                        "unsupported",
                        format!(
                            "block member {} ({}) cannot be flattened: {message}",
                            member.0,
                            class_name(&child.geometry)
                        ),
                    ),
                    other => other,
                },
            )?;
            if flattened != child.geometry {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    visiting.pop();
    result
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
