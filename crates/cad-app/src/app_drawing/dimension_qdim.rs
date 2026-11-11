//! `QDIM` (AutoCAD 2026): the documented generation modes over the current
//! selection.
//!
//! The nine documented modes map onto one typed payload
//! ([`crate::QDimCommand`]): **Continuous** (default), **Staggered**,
//! **Baseline**, **Ordinate**, **Radius** and **Diameter** are the generation
//! modes; **Datum Point**, **Edit** and **Settings** are the payload's `datum`,
//! `remove` and `snap` option fields. Angular is deliberately absent — AutoCAD
//! 2026 has no angular QDIM mode.
//!
//! Linear modes derive **extension-line origins** from the selection: a straight
//! `LINE` contributes both endpoints (or its pairwise intersections when
//! `SETTINGS INTERSECTIONS` is selected — in that setting only selected LINEs
//! contribute; other entities are still per-item skips), a non-associative
//! linear/aligned
//! dimension its two extension origins, an ordinate dimension its feature point
//! and a radius/diameter dimension its arc point. Radial modes consume selected
//! `CIRCLE`/`ARC` entities instead.
//!
//! Every unsupported selected entity is a **per-item skip** with the stable
//! `qdim.skipped` diagnostic (never a silent drop); a selection with no usable
//! target is the named `qdim.no_targets` refusal. Whole-run refusals (another
//! document, block-instance member, sub-element, missing entity, non-model space)
//! are named before any write and change nothing.
//!
//! Declared readings (AutoCAD 2026 does not document them):
//!
//! * the **Staggered** increment is the resolved `DIMDLI` unless the payload
//!   overrides it (the documented `AutoCAD` help does not name the increment
//!   source);
//! * the chain direction is the first selected `LINE`'s direction (or the first
//!   linear dimension's rotation), and origins are ordered by projection on it;
//! * **Continuous** places every row on one line at `+DIMDLI` off the first
//!   origin; **Staggered** steps `i · increment`; **Baseline** starts at the
//!   datum (or the first origin) and steps `i · DIMDLI` (an origin with the
//!   datum's own projection would measure zero and is skipped);
//! * **Ordinate** measures the dominant axis of `origin − datum` with the leader
//!   on the chain normal column; the default datum is the drawing origin;
//! * **Edit** removes 0-based indices from the ordered origin list (linear
//!   modes) or the radial target list, before generation.
//!
//! The whole batch lands in **one transaction / one undo step** on the active
//! layer through the shared insert funnel.

use super::dimension_edit::drawing_of;
use super::*;

use crate::{DimensionBatchPayload, QDimCommand, QDimMode, QDimSnap};

/// The minimum world extent for a QDIM origin/segment (the same order as the
/// other create commands' degeneracy guards).
const QDIM_EPS: f64 = 1e-9;

/// A named input refusal for QDIM.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// One selected radial target (a circle or an arc) in world space.
struct RadialTarget {
    center: Point3,
    normal: Point3,
    radius: f64,
    /// The in-plane start angle (arcs) or `0` (circles).
    start_angle: f64,
}

impl Application {
    /// `QDIM`: generate the requested dimension mode from the current selection.
    pub(crate) fn qdim(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match &command.payload {
            CommandPayload::None => QDimCommand::default(),
            CommandPayload::Dimension(payload) => match payload.as_ref() {
                DimensionBatchPayload::QDim(request) => request.clone(),
                _ => {
                    return Err(invalid(
                        "qdim.payload",
                        "QDIM takes a QDIM option payload; the selection is the input",
                    ))
                }
            },
            _ => {
                return Err(invalid(
                    "qdim.payload",
                    "QDIM takes a QDIM option payload; the selection is the input",
                ))
            }
        };
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "qdim.selection",
                "QDIM needs at least one selected object",
            ));
        }
        let drawing = drawing_of(self, command)?;
        let (_, style) = super::helpers::current_dim_style(drawing.as_ref(), session)?;
        let spacing = style.dim_line_increment;
        if !spacing.is_finite() || spacing <= QDIM_EPS {
            return Err(invalid(
                "qdim.bad_spacing",
                format!(
                    "the current dimension style's DIMDLI must be finite and positive, got                      {spacing}"
                ),
            ));
        }
        let increment = request.increment.unwrap_or(spacing);
        if !increment.is_finite() || increment <= QDIM_EPS {
            return Err(invalid(
                "qdim.bad_increment",
                format!(
                    "the QDIM STAGGERED increment must be finite and positive, got {increment}"
                ),
            ));
        }

        // 1. Classify every reference before any geometry is planned. Unsupported
        //    kinds are per-item skips; addressability/space problems are refusals.
        let mut seen = std::collections::BTreeSet::new();
        let mut skipped = Vec::new();
        let mut lines = Vec::new();
        let mut dimension_origins = Vec::new();
        let mut direction_hint = None;
        let mut radial = Vec::new();
        for reference in references {
            if !seen.insert(reference.entity) {
                return Err(invalid(
                    "qdim.duplicate",
                    format!("entity {} is selected twice", reference.entity.0),
                ));
            }
            if reference.document != command.document {
                return Err(invalid(
                    "qdim.not_addressable",
                    "the selected object belongs to another document",
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "qdim.not_addressable",
                    "a block-instance member or sub-element cannot be measured; select the \
                     entity itself",
                ));
            }
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                invalid(
                    "qdim.target_missing",
                    format!(
                        "the selected entity {} no longer exists",
                        reference.entity.0
                    ),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "qdim.space",
                    "QDIM creates dimensions in model space only",
                ));
            }
            match (&entity.geometry, request.mode) {
                (SemanticGeometry::Line { start, end }, QDimMode::Continuous)
                | (SemanticGeometry::Line { start, end }, QDimMode::Staggered)
                | (SemanticGeometry::Line { start, end }, QDimMode::Baseline)
                | (SemanticGeometry::Line { start, end }, QDimMode::Ordinate) => {
                    if distance(*start, *end) <= QDIM_EPS {
                        skipped.push(skip(reference.entity, "the LINE has no measurable length"));
                        continue;
                    }
                    if direction_hint.is_none() {
                        direction_hint = Some(normalize_xy(sub(*end, *start)));
                    }
                    lines.push((reference.entity, *start, *end));
                }
                (SemanticGeometry::Dimension(dimension), mode) if is_linear_mode(mode) => {
                    if request.snap == QDimSnap::Intersections {
                        skipped.push(skip(
                            reference.entity,
                            "an existing dimension does not contribute under SETTINGS \
                             INTERSECTIONS",
                        ));
                        continue;
                    }
                    match dimension.kind() {
                        DimensionKind::Linear { rotation } => {
                            if direction_hint.is_none() {
                                direction_hint = Some(Point3 {
                                    x: rotation.cos(),
                                    y: rotation.sin(),
                                    z: 0.0,
                                });
                            }
                            if let DimensionPoints::Linear { first, second, .. } =
                                dimension.definition().points
                            {
                                dimension_origins.push(first);
                                dimension_origins.push(second);
                            }
                        }
                        DimensionKind::Aligned => {
                            if let DimensionPoints::Linear { first, second, .. } =
                                dimension.definition().points
                            {
                                if direction_hint.is_none() {
                                    direction_hint = Some(normalize_xy(sub(second, first)));
                                }
                                dimension_origins.push(first);
                                dimension_origins.push(second);
                            }
                        }
                        DimensionKind::Ordinate { .. } => {
                            if let DimensionPoints::Ordinate { feature, .. } =
                                dimension.definition().points
                            {
                                dimension_origins.push(feature);
                            }
                        }
                        DimensionKind::Radius | DimensionKind::Diameter => {
                            if let DimensionPoints::Radial { point_on_arc, .. } =
                                dimension.definition().points
                            {
                                dimension_origins.push(point_on_arc);
                            }
                        }
                        DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => skipped.push(
                            skip(reference.entity, "an angular dimension is not a QDIM input"),
                        ),
                    }
                }
                (
                    SemanticGeometry::Circle {
                        center,
                        normal,
                        radius,
                    },
                    mode,
                ) if is_radial_mode(mode) => {
                    if !radius.is_finite() || *radius <= QDIM_EPS {
                        skipped.push(skip(reference.entity, "the CIRCLE has no radius"));
                        continue;
                    }
                    radial.push(RadialTarget {
                        center: *center,
                        normal: *normal,
                        radius: *radius,
                        start_angle: 0.0,
                    });
                }
                (
                    SemanticGeometry::Arc {
                        center,
                        normal,
                        radius,
                        start,
                        ..
                    },
                    mode,
                ) if is_radial_mode(mode) => {
                    if !radius.is_finite() || *radius <= QDIM_EPS {
                        skipped.push(skip(reference.entity, "the ARC has no radius"));
                        continue;
                    }
                    radial.push(RadialTarget {
                        center: *center,
                        normal: *normal,
                        radius: *radius,
                        start_angle: *start,
                    });
                }
                (geometry, mode) => skipped.push(skip(
                    reference.entity,
                    &format!(
                        "{} is not a {} QDIM input",
                        geometry_kind(geometry),
                        mode_name(mode)
                    ),
                )),
            }
        }

        let planned = if is_radial_mode(request.mode) {
            radial_plan(&request, radial, &style, spacing, &mut skipped)?
        } else {
            linear_plan(
                &request,
                &lines,
                dimension_origins,
                direction_hint,
                &style,
                spacing,
                increment,
                &mut skipped,
            )?
        };
        if planned.is_empty() {
            return Err(invalid(
                "qdim.no_targets",
                format!(
                    "QDIM {} found no usable target in the selection",
                    mode_name(request.mode)
                ),
            ));
        }
        let mut dimensions = Vec::with_capacity(planned.len());
        for definition in planned {
            dimensions.push(SemanticGeometry::Dimension(Box::new(
                DimensionGeometry::synthesized(
                    definition,
                    "Standard".to_string(),
                    style.clone(),
                    None,
                )?,
            )));
        }
        let mut outcome = self.commit_new_entities(session, command, dimensions, "qdim")?;
        outcome.diagnostics.extend(skipped);
        Ok(outcome)
    }
}

fn is_linear_mode(mode: QDimMode) -> bool {
    matches!(
        mode,
        QDimMode::Continuous | QDimMode::Staggered | QDimMode::Baseline | QDimMode::Ordinate
    )
}

fn is_radial_mode(mode: QDimMode) -> bool {
    matches!(mode, QDimMode::Radius | QDimMode::Diameter)
}

fn mode_name(mode: QDimMode) -> &'static str {
    match mode {
        QDimMode::Continuous => "CONTINUOUS",
        QDimMode::Staggered => "STAGGERED",
        QDimMode::Baseline => "BASELINE",
        QDimMode::Ordinate => "ORDINATE",
        QDimMode::Radius => "RADIUS",
        QDimMode::Diameter => "DIAMETER",
    }
}

/// One stable per-item skip diagnostic (the item is not dimensioned).
fn skip(entity: EntityId, reason: &str) -> Diagnostic {
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: "qdim.skipped".into(),
        message: format!("QDIM skipped entity {}: {reason}", entity.0),
    }
}

/// Plan the four linear-family modes.
#[allow(clippy::too_many_arguments)]
fn linear_plan(
    request: &QDimCommand,
    lines: &[(EntityId, Point3, Point3)],
    mut dimension_origins: Vec<Point3>,
    direction_hint: Option<Point3>,
    _style: &cad_domain::DimensionStyleValues,
    spacing: f64,
    increment: f64,
    _skipped: &mut Vec<Diagnostic>,
) -> CadResult<Vec<DimensionDefinition>> {
    let mut origins: Vec<Point3> = match request.snap {
        QDimSnap::Endpoints => {
            let mut points = Vec::new();
            for (_, start, end) in lines {
                points.push(*start);
                points.push(*end);
            }
            points.append(&mut dimension_origins);
            points
        }
        QDimSnap::Intersections => {
            if lines.len() < 2 {
                return Err(invalid(
                    "qdim.no_intersections",
                    "QDIM SETTINGS INTERSECTIONS needs at least two selected LINEs",
                ));
            }
            let mut points = Vec::new();
            for first in 0..lines.len() {
                for second in (first + 1)..lines.len() {
                    if let Some(point) = segment_intersection(
                        lines[first].1,
                        lines[first].2,
                        lines[second].1,
                        lines[second].2,
                    ) {
                        points.push(point);
                    }
                }
            }
            if points.is_empty() {
                return Err(invalid(
                    "qdim.no_intersections",
                    "QDIM SETTINGS INTERSECTIONS found no intersection between the selected LINEs",
                ));
            }
            points
        }
    };
    if origins.is_empty() {
        return Err(invalid(
            "qdim.no_targets",
            "QDIM found no extension-line origins in the selection",
        ));
    }
    let plane_z = origins[0].z;
    let z_tolerance = 1e-9 * (1.0 + origins.iter().map(|point| point.z.abs()).sum::<f64>());
    if origins
        .iter()
        .any(|point| (point.z - plane_z).abs() > z_tolerance)
    {
        return Err(invalid(
            "qdim.off_plane",
            "QDIM origins must share one +Z plane; off-plane QDIM is NOT IMPLEMENTED",
        ));
    }
    let direction = match direction_hint {
        Some(direction) if length_of(direction) > QDIM_EPS => normalize_xy(direction),
        _ => {
            let mut sorted = origins.clone();
            sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
            let first = sorted[0];
            let last = *sorted.last().expect("non-empty");
            let direction = sub(last, first);
            if length_of(direction) <= QDIM_EPS {
                return Err(invalid(
                    "qdim.degenerate",
                    "QDIM cannot derive a chain direction from coincident origins",
                ));
            }
            normalize_xy(direction)
        }
    };
    let normal = Point3 {
        x: -direction.y,
        y: direction.x,
        z: 0.0,
    };
    // Order and de-duplicate the origins by their in-plane projection.
    let anchor = origins[0];
    origins.sort_by(|a, b| {
        dot(sub(*a, anchor), direction)
            .total_cmp(&dot(sub(*b, anchor), direction))
            .then(dot(sub(*a, anchor), normal).total_cmp(&dot(sub(*b, anchor), normal)))
    });
    origins.dedup_by(|a, b| distance(*a, *b) <= QDIM_EPS);
    // The Edit option: 0-based indices removed before generation.
    for index in &request.remove {
        if *index >= origins.len() {
            return Err(invalid(
                "qdim.edit_range",
                format!(
                    "QDIM EDIT index {} is outside the {} ordered origin(s)",
                    index,
                    origins.len()
                ),
            ));
        }
    }
    let kept: Vec<Point3> = origins
        .iter()
        .enumerate()
        .filter(|(index, _)| !request.remove.contains(index))
        .map(|(_, point)| *point)
        .collect();
    if kept.len() < 2 {
        return Err(invalid(
            "qdim.no_targets",
            "QDIM needs at least two origins after EDIT",
        ));
    }
    let line_point = add(kept[0], scale_point(normal, spacing));
    let mut out = Vec::new();
    match request.mode {
        QDimMode::Continuous => {
            for pair in kept.windows(2) {
                out.push(linear_definition(direction, pair[0], pair[1], line_point));
            }
        }
        QDimMode::Staggered => {
            for (index, pair) in kept.windows(2).enumerate() {
                ensure_advance(direction, pair[0], pair[1])?;
                let line = scale_point(normal, increment * (index + 1) as f64);
                out.push(linear_definition(
                    direction,
                    pair[0],
                    pair[1],
                    add(kept[0], line),
                ));
            }
        }
        QDimMode::Baseline => {
            let base = request.datum.unwrap_or(kept[0]);
            let mut row = 0usize;
            for point in &kept {
                if distance(base, *point) <= QDIM_EPS {
                    continue;
                }
                // A point with the datum's own projection would measure zero and is
                // skipped (there is no row to draw).
                if dot(sub(*point, base), direction).abs() <= QDIM_EPS {
                    continue;
                }
                row += 1;
                let line = add(base, scale_point(normal, spacing * row as f64));
                out.push(linear_definition(direction, base, *point, line));
            }
        }
        QDimMode::Ordinate => {
            let datum = request.datum.unwrap_or(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
            for point in &kept {
                let delta = sub(*point, datum);
                out.push(DimensionDefinition {
                    kind: DimensionKind::Ordinate {
                        x_ordinate: delta.x.abs() >= delta.y.abs(),
                    },
                    points: DimensionPoints::Ordinate {
                        feature: *point,
                        leader_end: add(*point, scale_point(normal, spacing)),
                        base: datum,
                    },
                    text_middle: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    text_rotation: 0.0,
                    normal: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    },
                });
            }
        }
        QDimMode::Radius | QDimMode::Diameter => unreachable!("radial mode routed separately"),
    }
    Ok(out)
}

/// Refuse a row whose projection along the chain direction is zero (two origins
/// that only differ across the dimension line would measure nothing).
fn ensure_advance(direction: Point3, first: Point3, second: Point3) -> CadResult<()> {
    let advance = dot(sub(second, first), direction).abs();
    if !advance.is_finite() || advance <= QDIM_EPS {
        return Err(invalid(
            "qdim.degenerate",
            "two consecutive QDIM origins share their projection on the chain direction and \
             would measure zero",
        ));
    }
    Ok(())
}

/// One linear definition on the chain direction at `line_point`.
fn linear_definition(
    direction: Point3,
    first: Point3,
    second: Point3,
    line_point: Point3,
) -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Linear {
            rotation: direction.y.atan2(direction.x),
        },
        points: DimensionPoints::Linear {
            first,
            second,
            dimension_line: line_point,
        },
        text_middle: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        text_rotation: 0.0,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    }
}

/// Plan `RADIUS` / `DIAMETER` over the selected circles and arcs.
fn radial_plan(
    request: &QDimCommand,
    mut targets: Vec<RadialTarget>,
    _style: &cad_domain::DimensionStyleValues,
    spacing: f64,
    _skipped: &mut Vec<Diagnostic>,
) -> CadResult<Vec<DimensionDefinition>> {
    if targets.is_empty() {
        return Err(invalid(
            "qdim.no_targets",
            format!(
                "QDIM {} needs at least one selected CIRCLE or ARC",
                mode_name(request.mode)
            ),
        ));
    }
    for index in &request.remove {
        if *index >= targets.len() {
            return Err(invalid(
                "qdim.edit_range",
                format!(
                    "QDIM EDIT index {} is outside the {} radial target(s)",
                    index,
                    targets.len()
                ),
            ));
        }
    }
    targets = targets
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !request.remove.contains(index))
        .map(|(_, target)| target)
        .collect();
    let mut out = Vec::new();
    for target in targets {
        let (ax, ay, _) = cad_geometry::arbitrary_axis(target.normal);
        let unit = Point3 {
            x: ax.x * target.start_angle.cos() + ay.x * target.start_angle.sin(),
            y: ax.y * target.start_angle.cos() + ay.y * target.start_angle.sin(),
            z: ax.z * target.start_angle.cos() + ay.z * target.start_angle.sin(),
        };
        let point_on_arc = add(target.center, scale_point(unit, target.radius));
        let (kind, center) = match request.mode {
            QDimMode::Radius => (DimensionKind::Radius, target.center),
            _ => (
                DimensionKind::Diameter,
                sub(target.center, scale_point(unit, target.radius)),
            ),
        };
        out.push(DimensionDefinition {
            kind,
            points: DimensionPoints::Radial {
                center,
                point_on_arc,
                // The synthesis places the text on the leader; group 40 is stored
                // but never drives this build's display (declared).
                leader_length: spacing,
            },
            text_middle: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            text_rotation: 0.0,
            normal: if length_of(target.normal) > QDIM_EPS {
                target.normal
            } else {
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                }
            },
        });
    }
    Ok(out)
}

/// The intersection of two closed 2D segments, or `None` when they are parallel,
/// collinear or touch outside both segments.
fn segment_intersection(a1: Point3, a2: Point3, b1: Point3, b2: Point3) -> Option<Point3> {
    let d1 = sub(a2, a1);
    let d2 = sub(b2, b1);
    let denominator = d1.x * d2.y - d1.y * d2.x;
    if denominator.abs() <= 1e-12 {
        return None;
    }
    let offset = sub(b1, a1);
    let t = (offset.x * d2.y - offset.y * d2.x) / denominator;
    let u = (offset.x * d1.y - offset.y * d1.x) / denominator;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(Point3 {
            x: a1.x + d1.x * t,
            y: a1.y + d1.y * t,
            z: a1.z,
        })
    } else {
        None
    }
}

fn normalize_xy(point: Point3) -> Point3 {
    let length = length_of(point);
    if !length.is_finite() || length <= QDIM_EPS {
        return Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
    }
    Point3 {
        x: point.x / length,
        y: point.y / length,
        z: 0.0,
    }
}

fn length_of(point: Point3) -> f64 {
    dot(point, point).sqrt()
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

fn scale_point(point: Point3, factor: f64) -> Point3 {
    Point3 {
        x: point.x * factor,
        y: point.y * factor,
        z: point.z * factor,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

#[cfg(test)]
mod tests;
