//! The STRETCH per-vertex geometry planner.
//!
//! Split out of `stretch.rs` so both files stay inside the code-size limit; the
//! command-level classification/transaction planner is the sibling module. The
//! behaviour is unchanged: a fully enclosed entity is translated through the
//! same `transform_geometry` MOVE uses, and a partially enclosed
//! `LINE`/`POLYLINE`/`SPLINE`/`ARC`/`DIMENSION` moves only the vertices,
//! control points, endpoints or definition points that lie inside a window.

use super::*;

/// The whole-entity translation used by `Enclosure::Full` — exactly MOVE's
/// result, through the same `cad-db` `transform_geometry` (a NURBS is affinely
/// invariant), so the two commands cannot disagree.
pub(super) fn moved_geometry(
    geometry: &SemanticGeometry,
    translation: Point3,
) -> CadResult<SemanticGeometry> {
    transform_geometry(geometry, &Transform3::translation(translation))
}

/// Build the per-vertex stretched geometry for `LINE`, `POLYLINE`, `SPLINE`,
/// `ARC` or `DIMENSION`, moving only what lies inside a window.
///
/// `Err(reason)` means nothing moved and carries the stable diagnostic reason, so
/// an in-window entity is never silent. Bulges are kept as-is (UNVERIFIED by
/// Autodesk; our declared rule); a spline's stored control points move with its
/// `degree`/`knots`/`weights` copied verbatim; the arc helpers below reproduce the
/// tessellator's angle frame term for term and use the grip model's sweep slack.
///
/// `inside` / `shift` are the windows' containment and displacement predicates:
/// the rectangle and polygon windows share them, so a vertex moves under exactly
/// the same containment rule the command classified the entity with.
pub(super) fn stretched_geometry(
    geometry: &SemanticGeometry,
    inside: impl Fn(Point3) -> bool,
    shift: impl Fn(Point3) -> Point3,
) -> Result<SemanticGeometry, &'static str> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let new_start = if inside(*start) {
                shift(*start)
            } else {
                *start
            };
            let new_end = if inside(*end) { shift(*end) } else { *end };
            if new_start == *start && new_end == *end {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Line {
                start: new_start,
                end: new_end,
            })
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            let new_points: Vec<Point3> = points
                .iter()
                .map(|point| {
                    if inside(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *points {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Polyline {
                points: new_points,
                // UNVERIFIED: the stored bulge values are preserved as-is.
                bulges: bulges.clone(),
                closed: *closed,
                width: 0.0,
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            // A stored definition that is not indexable cannot be edited
            // faithfully: refuse by name instead of staging vectors that no
            // consumer (`cad-geometry::NurbsCurve`, the control-point grips) can
            // evaluate. This is the same relation `cad-db` enforces on write, with
            // the same two empty-vector shorthands (`knots: []` = clamped uniform,
            // `weights: []` = all ones) exempt in their own respect only.
            let expected_knots = control_points.len() + *degree as usize + 1;
            let weights_indexable = weights.is_empty() || weights.len() == control_points.len();
            let knots_indexable = knots.is_empty() || knots.len() == expected_knots;
            if !weights_indexable || !knots_indexable {
                return Err("spline_inconsistent");
            }
            let new_points: Vec<Point3> = control_points
                .iter()
                .map(|point| {
                    if inside(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *control_points {
                return Err("no_vertex");
            }
            // `degree`, `knots` and `weights` are copied verbatim: the moved
            // points are a different NURBS with the same parameterisation, so the
            // structural invariants above still hold.
            Ok(SemanticGeometry::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: new_points,
                weights: weights.clone(),
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let axes = arc_plane_axes(*normal);
            let end_angle = start + sweep;
            let start_point = arc_point(*center, *radius, axes, *start);
            let end_point = arc_point(*center, *radius, axes, end_angle);
            let start_inside = inside(start_point);
            let end_inside = inside(end_point);
            if !start_inside && !end_inside {
                return Err("no_vertex");
            }
            // Each in-window endpoint moves to the in-plane direction of its
            // translated position; the other one keeps its stored angle.
            let mut new_start = *start;
            let mut new_end = end_angle;
            if start_inside {
                new_start =
                    in_plane_angle(*center, axes, shift(start_point)).ok_or("arc_no_direction")?;
            }
            if end_inside {
                new_end =
                    in_plane_angle(*center, axes, shift(end_point)).ok_or("arc_no_direction")?;
            }
            let direction = if *sweep < 0.0 { -1.0 } else { 1.0 };
            let new_sweep = sweep_in_direction(new_start, new_end, direction);
            let extent = new_sweep.abs();
            if !new_sweep.is_finite()
                || extent <= crate::grips::ARC_SWEEP_EPSILON
                || std::f64::consts::TAU - extent <= crate::grips::ARC_SWEEP_EPSILON
            {
                return Err("arc_degenerate_sweep");
            }
            if new_start == *start && new_sweep == *sweep {
                // The in-plane rule left the arc identical: the displacement has
                // no tangential component in the arc's own plane (purely radial or
                // purely out-of-plane), so "the endpoints move" is a no-op here.
                return Err("arc_unchanged");
            }
            Ok(SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: new_start,
                sweep: new_sweep,
            })
        }
        SemanticGeometry::Dimension(dimension) => {
            stretched_dimension(dimension, &inside, &shift).ok_or("dimension_unchanged")
        }
        // The plan only ever routes the kinds above (LINE, POLYLINE, SPLINE, ARC
        // and DIMENSION) into `Stretch`; anything else here would be a planning
        // defect, reported rather than guessed.
        _ => Err("not_stretchable"),
    }
}

/// Stretch a dimension by moving only its **definition points** that lie inside
/// a window.
///
/// Autodesk documents dimensions as stretchable: the extension-line origins and
/// the dimension-line location move with the window, and the measurement follows.
/// Every point slot of [`DimensionPoints`] plus the group-11 text middle point is
/// moved independently; `kind`, `text_rotation` (an explicit rotation is kept),
/// `normal`, the style snapshot and the text override are copied. The display is
/// rebuilt through the domain's single synthesizer ([`DimensionGeometry::synthesized`])
/// so measurement and drawn number stay in step (ADR 0017 A.4 invariant 1) — this
/// command never hand-builds dimension geometry.
///
/// `None` means nothing moved (no point inside the window) or the stretched
/// definition cannot be synthesized (e.g. it collapsed); the caller reports the
/// named `dimension_unchanged` diagnostic and writes nothing for this entity.
/// This is **our declared choice**, not an Autodesk assertion about which
/// individual definition point moves first.
fn stretched_dimension(
    dimension: &DimensionGeometry,
    inside: &impl Fn(Point3) -> bool,
    shift: &impl Fn(Point3) -> Point3,
) -> Option<SemanticGeometry> {
    let source = dimension.definition();
    let moved = |point: Point3| {
        if inside(point) {
            shift(point)
        } else {
            point
        }
    };
    let points = match &source.points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => DimensionPoints::Linear {
            first: moved(*first),
            second: moved(*second),
            dimension_line: moved(*dimension_line),
        },
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => DimensionPoints::Radial {
            center: moved(*center),
            point_on_arc: moved(*point_on_arc),
            leader_length: *leader_length,
        },
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => DimensionPoints::Angular2Ln {
            line1_start: moved(*line1_start),
            line1_end: moved(*line1_end),
            line2_start: moved(*line2_start),
            line2_end: moved(*line2_end),
            arc_point: moved(*arc_point),
        },
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => DimensionPoints::Angular3Pt {
            vertex: moved(*vertex),
            first: moved(*first),
            second: moved(*second),
            arc_point: moved(*arc_point),
        },
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => DimensionPoints::Ordinate {
            feature: moved(*feature),
            leader_end: moved(*leader_end),
            base: moved(*base),
        },
    };
    let text_middle = moved(source.text_middle);
    if points == source.points && text_middle == source.text_middle {
        return None;
    }
    let definition = DimensionDefinition {
        kind: source.kind,
        points,
        text_middle,
        text_rotation: source.text_rotation,
        normal: source.normal,
    };
    let rebuilt = DimensionGeometry::synthesized(
        definition,
        dimension.style_name().to_string(),
        dimension.style().clone(),
        dimension.text_override().map(str::to_string),
    )
    .ok()?;
    Some(SemanticGeometry::Dimension(Box::new(rebuilt)))
}

/// The arc's plane frame `(ax, ay)`: the AutoCAD arbitrary-axis frame, i.e. the
/// same axes the tessellator, the representation provider and the arc endpoint
/// grips measure `start`/`sweep` in. Sharing the frame is what makes an angle here
/// denote the point the display draws for the same stored values.
///
/// Duplicating the two-line frame locally (as `select_box`/`trim_boundary` also
/// do) is deliberate: the grip helpers are module-private and STRETCH must not
/// reach into the grip model.
fn arc_plane_axes(normal: Point3) -> (Point3, Point3) {
    let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
    (ax, ay)
}

/// One point on the arc: `centre + radius·(cos θ·ax + sin θ·ay)`, the
/// tessellator's expression term for term.
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    cad_geometry::add(
        center,
        cad_geometry::add(
            cad_geometry::scale(axes.0, angle.cos() * radius),
            cad_geometry::scale(axes.1, angle.sin() * radius),
        ),
    )
}

/// The in-plane angle of `to` as seen from `center`, or `None` when it has no
/// in-plane direction at all (it sits on the centre, on the normal through it, or
/// the projection is non-finite). The out-of-plane offset is discarded by
/// construction, so stretching an arc endpoint can never tilt the arc out of its
/// own plane.
fn in_plane_angle(center: Point3, axes: (Point3, Point3), to: Point3) -> Option<f64> {
    let offset = cad_geometry::sub(to, center);
    let u = cad_geometry::dot(offset, axes.0);
    let v = cad_geometry::dot(offset, axes.1);
    if !u.is_finite() || !v.is_finite() || (u == 0.0 && v == 0.0) {
        return None;
    }
    Some(v.atan2(u))
}

/// Wrap an angle into `(0, 2π]` — the positive-direction extent between two
/// angles, the same normalization the importer's `normalize_sweep` performs.
fn wrap_positive(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped <= 0.0 {
        wrapped += tau;
    }
    wrapped
}

/// The signed sweep from `from` to `to` **in the stored direction**.
///
/// `direction` is `+1.0` for a counter-clockwise (positive-sweep) arc and `-1.0`
/// for a clockwise/reflected one, so a stretch never flips the arc's side. The
/// caller rejects a value of 0 or a full turn.
fn sweep_in_direction(from: f64, to: f64, direction: f64) -> f64 {
    if direction < 0.0 {
        -wrap_positive(from - to)
    } else {
        wrap_positive(to - from)
    }
}
