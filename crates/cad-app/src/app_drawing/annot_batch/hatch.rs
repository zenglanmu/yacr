//! `HATCHSETBOUNDARY` (rebuild the hatch loops from selected closed entities) and
//! `HATCHSETORIGIN` (a named refusal: the hatch domain has no pattern-origin
//! field).
//!
//! Boundary loops use the same OCS projection and the same closed shapes as the
//! existing HATCH/BOUNDARY path; the rebuild goes through
//! [`cad_domain::HatchGeometry::new`] so the fill display is derived, never hand
//! edited.

use super::super::hatch_boundary::plane_projection;
use super::*;

/// The ellipse sampling count the boundary builder uses (`hatch_boundary`).
const ELLIPSE_SEGMENTS: usize = 64;

/// The absolute/relative coplanarity tolerance.
const PLANE_TOLERANCE: f64 = 1e-7;

impl Application {
    /// `HATCHSETBOUNDARY`: one selected hatch plus closed boundary entities.
    pub(crate) fn hatch_set_boundary(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "hatchsetboundary.payload",
                "HATCHSETBOUNDARY takes no payload; the selection is its input",
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(invalid(
                "hatchsetboundary.selection",
                "HATCHSETBOUNDARY needs one selected hatch plus at least one closed boundary",
            ));
        }
        let document_id = command.document;
        let candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let mut target: Option<(DbEntity, HatchGeometry)> = None;
        let mut loops = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            if reference.document != document_id {
                return Err(invalid(
                    "hatchsetboundary.not_addressable",
                    format!(
                        "the selected ref for entity {} belongs to another document",
                        reference.entity.0
                    ),
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "hatchsetboundary.not_addressable",
                    format!(
                        "entity {} is a block-instance member or sub-element; select the \
                         model-space entities themselves",
                        reference.entity.0
                    ),
                ));
            }
            if !seen.insert(reference.entity) {
                continue;
            }
            let entity = candidate.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "hatchsetboundary.not_addressable",
                    format!("entity {} no longer exists", reference.entity.0),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "hatchsetboundary.not_addressable",
                    format!("entity {} is not in model space", entity.id.0),
                ));
            }
            match &entity.geometry {
                SemanticGeometry::Hatch(hatch) => {
                    if target.is_some() {
                        return Err(invalid(
                            "hatchsetboundary.multiple_hatches",
                            "HATCHSETBOUNDARY rebuilds exactly one hatch, but more than one \
                             hatch is selected",
                        ));
                    }
                    target = Some((entity.clone(), hatch.as_ref().clone()));
                }
                SemanticGeometry::Polyline { .. }
                | SemanticGeometry::Circle { .. }
                | SemanticGeometry::Ellipse { .. } => {
                    loops.push(boundary_loop(&entity, target.as_ref().map(|(_, h)| h))?);
                }
                other => {
                    return Err(CadError::Unsupported(format!(
                        "hatchsetboundary.unsupported_boundary: {} is not a closed \
                         Polyline/Circle/full Ellipse boundary",
                        class_name(other)
                    )));
                }
            }
        }
        let Some((before, hatch)) = target else {
            return Err(invalid(
                "hatchsetboundary.selection",
                "HATCHSETBOUNDARY needs exactly one selected hatch in the selection",
            ));
        };
        if loops.is_empty() {
            return Err(invalid(
                "hatchsetboundary.selection",
                "HATCHSETBOUNDARY needs at least one selected closed boundary entity",
            ));
        }
        // The earlier boundary calls needed the hatch plane; the hatch is now
        // guaranteed present, so rebuild every loop once more with its plane and
        // validate coplanarity against that exact plane.
        let mut checked = Vec::with_capacity(loops.len());
        for reference in &references {
            if reference.entity == before.id {
                continue;
            }
            let entity = candidate.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "hatchsetboundary.not_addressable",
                    format!("entity {} no longer exists", reference.entity.0),
                )
            })?;
            checked.push(boundary_loop(&entity, Some(&hatch))?);
        }
        let loops = checked;
        let rebuilt = HatchGeometry::new(
            loops.clone(),
            hatch.plane(),
            hatch.style(),
            hatch.fill().clone(),
            // The stored internal seeds described the *old* boundary, so they
            // are dropped rather than left pointing at the replaced loops.
            Vec::new(),
            hatch.pixel_size(),
            hatch.pattern_type(),
        )
        .map_err(|error| {
            invalid(
                "hatchsetboundary.rebuild_failed",
                format!("the domain constructor rejected the new boundary set: {error}"),
            )
        })?;
        if rebuilt.loops() == hatch.loops() {
            return Ok(unchanged(
                "hatchsetboundary.unchanged",
                format!(
                    "HATCHSETBOUNDARY: the selected boundaries already are the hatch's loops \
                     (entity {}; nothing was written)",
                    before.id.0
                ),
            ));
        }
        let mut after = before.clone();
        after.geometry = SemanticGeometry::Hatch(Box::new(rebuilt));
        commit_one_entity(
            self,
            document_id,
            candidate,
            before,
            after,
            "hatchsetboundary",
            "HATCHSETBOUNDARY: rebuilt the hatch boundary loops from the selected closed \
             entities (source entities retained; one transaction)"
                .to_string(),
        )
    }

    /// `HATCHSETORIGIN <x> <y>`: parsed but unconditionally refused because the
    /// hatch model has no pattern-origin field.
    pub(crate) fn hatch_set_origin(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (x, y) = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::HatchSetOrigin(request) => (request.x, request.y),
                _ => {
                    return Err(invalid(
                        "hatchsetorigin.payload",
                        "HATCHSETORIGIN takes `<x> <y>`",
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    "hatchsetorigin.needs_argument",
                    "HATCHSETORIGIN needs `<x> <y>`",
                ))
            }
            _ => {
                return Err(invalid(
                    "hatchsetorigin.payload",
                    "HATCHSETORIGIN takes `<x> <y>`",
                ))
            }
        };
        if !x.is_finite() || !y.is_finite() {
            return Err(invalid(
                "hatchsetorigin.bad_point",
                "HATCHSETORIGIN needs finite X/Y values",
            ));
        }
        let references = session.selection.refs();
        if references.len() != 1 {
            return Err(invalid(
                "hatchsetorigin.selection",
                format!(
                    "HATCHSETORIGIN reads exactly one selected hatch, got {}",
                    references.len()
                ),
            ));
        }
        let reference = &references[0];
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        if reference.document != document_id
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(invalid(
                "hatchsetorigin.not_addressable",
                "HATCHSETORIGIN needs a directly addressable model-space hatch",
            ));
        }
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            invalid(
                "hatchsetorigin.not_addressable",
                format!("entity {} no longer exists", reference.entity.0),
            )
        })?;
        if !matches!(entity.geometry, SemanticGeometry::Hatch(_)) {
            return Err(CadError::Unsupported(format!(
                "hatchsetorigin.not_hatch: entity {} is {}",
                entity.id.0,
                class_name(&entity.geometry)
            )));
        }
        Err(CadError::Unsupported(format!(
            "hatchsetorigin.missing_field: cad_domain::HatchGeometry has no hatch \
             pattern-origin field (only HatchPlane.elevation and per-line \
             HatchPatternLine.base_x/base_y exist, neither of which is the group-10 origin), \
             so the requested origin ({x}, {y}) changes nothing"
        )))
    }
}

/// Convert one selected closed entity into a `HatchLoop` in the hatch's OCS.
///
/// `plane = None` in the first pass only collects shapes before the hatch has
/// been found; the second pass always supplies the hatch plane and performs the
/// coplanarity check.
pub(crate) fn boundary_loop(
    entity: &DbEntity,
    plane: Option<&HatchGeometry>,
) -> CadResult<HatchLoop> {
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed: true,
        } => {
            if points.len() < 3 {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("polyline {} has fewer than three vertices", entity.id.0),
                ));
            }
            if !bulges.is_empty() && bulges.len() != points.len() {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!(
                        "polyline {} has {} bulges for {} vertices",
                        entity.id.0,
                        bulges.len(),
                        points.len()
                    ),
                ));
            }
            if points.iter().any(|point| !is_finite_point(*point)) {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("polyline {} has a non-finite vertex", entity.id.0),
                ));
            }
            let Some(hatch) = plane else {
                return Ok(HatchLoop::new(
                    Vec::new(),
                    Vec::new(),
                    HatchLoopFlags(HatchLoopFlags::POLYLINE),
                ));
            };
            let to_2d = plane_projection(&hatch.plane());
            for point in points {
                require_on_plane(*point, hatch, "polyline")?;
            }
            let local: Vec<[f64; 2]> = points.iter().map(|point| to_2d(*point)).collect();
            let bulges = if bulges.is_empty() {
                vec![0.0; local.len()]
            } else {
                bulges.clone()
            };
            Ok(HatchLoop::new(
                local,
                bulges,
                HatchLoopFlags(HatchLoopFlags::POLYLINE),
            ))
        }
        SemanticGeometry::Polyline { closed: false, .. } => Err(invalid(
            "hatchsetboundary.not_closed",
            format!("polyline {} is open", entity.id.0),
        )),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("circle {} has a non-positive radius", entity.id.0),
                ));
            }
            let Some(hatch) = plane else {
                return Ok(HatchLoop::new(
                    Vec::new(),
                    Vec::new(),
                    HatchLoopFlags(HatchLoopFlags::POLYLINE),
                ));
            };
            require_on_plane(*center, hatch, "circle")?;
            require_parallel_normal(*normal, hatch, "circle")?;
            let to_2d = plane_projection(&hatch.plane());
            let center = to_2d(*center);
            let bulge = std::f64::consts::FRAC_PI_8.tan();
            let points = (0..4)
                .map(|index| {
                    let angle = index as f64 * std::f64::consts::FRAC_PI_2;
                    let (sin, cos) = angle.sin_cos();
                    [center[0] + radius * cos, center[1] + radius * sin]
                })
                .collect();
            Ok(HatchLoop::new(
                points,
                vec![bulge; 4],
                HatchLoopFlags(HatchLoopFlags::POLYLINE),
            ))
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            if !ratio.is_finite() || ratio.abs() <= 0.0 {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("ellipse {} has a degenerate ratio", entity.id.0),
                ));
            }
            if (sweep.abs() - std::f64::consts::TAU).abs() > 1e-6 {
                return Err(invalid(
                    "hatchsetboundary.open_ellipse",
                    format!(
                        "ellipse {} is not a full ellipse (sweep {}), so it is not a closed \
                         boundary",
                        entity.id.0, sweep
                    ),
                ));
            }
            let Some(hatch) = plane else {
                return Ok(HatchLoop::new(
                    Vec::new(),
                    Vec::new(),
                    HatchLoopFlags(HatchLoopFlags::POLYLINE),
                ));
            };
            require_on_plane(*center, hatch, "ellipse")?;
            require_parallel_normal(*normal, hatch, "ellipse")?;
            let Some(major_unit) = unit(*major_axis) else {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("ellipse {} has a degenerate major axis", entity.id.0),
                ));
            };
            let Some(normal_unit) = unit(*normal) else {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("ellipse {} has a degenerate normal", entity.id.0),
                ));
            };
            let major_length = length(*major_axis);
            if !major_length.is_finite() || major_length <= 0.0 {
                return Err(invalid(
                    "hatchsetboundary.degenerate",
                    format!("ellipse {} has a degenerate major axis length", entity.id.0),
                ));
            }
            let minor = scale(cross(normal_unit, major_unit), major_length * ratio.abs());
            let to_2d = plane_projection(&hatch.plane());
            let points = (0..ELLIPSE_SEGMENTS)
                .map(|index| {
                    let t = *start + sweep * index as f64 / ELLIPSE_SEGMENTS as f64;
                    let (sin, cos) = t.sin_cos();
                    to_2d(add(
                        *center,
                        add(scale(*major_axis, cos), scale(minor, sin)),
                    ))
                })
                .collect();
            Ok(HatchLoop::new(
                points,
                vec![0.0; ELLIPSE_SEGMENTS],
                HatchLoopFlags(HatchLoopFlags::POLYLINE),
            ))
        }
        other => Err(CadError::Unsupported(format!(
            "hatchsetboundary.unsupported_boundary: {} is not a closed boundary",
            class_name(other)
        ))),
    }
}

/// Require a point to lie on the hatch plane's OCS surface.
fn require_on_plane(point: Point3, hatch: &HatchGeometry, shape: &str) -> CadResult<()> {
    if !is_finite_point(point) {
        return Err(invalid(
            "hatchsetboundary.degenerate",
            format!("{shape} boundary has a non-finite point"),
        ));
    }
    let plane = hatch.plane();
    let to_2d = plane_projection(&plane);
    let round_trip = plane.to_world(to_2d(point));
    let scale = point.x.abs().max(point.y.abs()).max(point.z.abs()).max(1.0);
    if length(sub(round_trip, point)) > PLANE_TOLERANCE * scale {
        return Err(invalid(
            "hatchsetboundary.non_planar",
            format!(
                "{shape} boundary point ({}, {}, {}) is not on the hatch plane; it cannot be \
                 flattened without changing the geometry",
                point.x, point.y, point.z
            ),
        ));
    }
    Ok(())
}

/// Require a circle/ellipse normal to be parallel to the hatch plane normal.
fn require_parallel_normal(normal: Point3, hatch: &HatchGeometry, shape: &str) -> CadResult<()> {
    let Some(normal) = unit(normal) else {
        return Err(invalid(
            "hatchsetboundary.degenerate",
            format!("{shape} boundary has a degenerate normal"),
        ));
    };
    let plane_normal = hatch.plane().normal;
    if (dot(normal, plane_normal).abs() - 1.0).abs() > PLANE_TOLERANCE {
        return Err(invalid(
            "hatchsetboundary.non_planar",
            format!(
                "{shape} boundary normal is not parallel to the hatch plane normal; it is not \
                 coplanar with the hatch"
            ),
        ));
    }
    Ok(())
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale(v: Point3, factor: f64) -> Point3 {
    Point3 {
        x: v.x * factor,
        y: v.y * factor,
        z: v.z * factor,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length(v: Point3) -> f64 {
    dot(v, v).sqrt()
}

fn unit(v: Point3) -> Option<Point3> {
    let len = length(v);
    (len.is_finite() && len > 1e-12).then(|| scale(v, 1.0 / len))
}
