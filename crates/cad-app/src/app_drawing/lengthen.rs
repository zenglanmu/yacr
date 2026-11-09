//! `LENGTHEN`: change the length of one selected curve, measured from the end
//! **nearest the pick point**.
//!
//! The capture is the one-selected-curve shape
//! ([`crate::draw_tool::DrawToolKind::Lengthen`]) plus **one pick** for the
//! static modes: the selected curve is the whole target and the pick chooses
//! which end moves (the static modes travel through [`CommandPayload::Points`]
//! with exactly one point; the legacy `Lengthen { refs }` payload without a pick
//! keeps the historical "move the end" behaviour). The mode + amount are the
//! session typed channel ([`crate::draw_tool::ToolParameter::LengthenMode`] —
//! `0` Delta / `1` Percent / `2` Total — and
//! [`crate::draw_tool::ToolParameter::LengthenValue`], finite and non-zero).
//!
//! ## Modes
//!
//! | mode | LINE / open polyline / open spline | ARC / elliptical arc |
//! | --- | --- | --- |
//! | `0` Delta (`DE`) | `new = old + value` (negative shortens) | new arc length `old + value` |
//! | `1` Percent (`PER`) | `new = old · value / 100` | new arc length `old · value / 100` |
//! | `2` Total (`TOT`) | `new = value` (the absolute length) | `value` is the **included angle in degrees** |
//! | Dynamic (`DY`) | the picked new end point, projected onto the line | the picked new end point, projected radially onto the arc |
//!
//! The dynamic mode is accepted through the existing typed `Points` payload with
//! exactly two points (`pick`, `new_end`) and the selection taken from the
//! session. The first point is the object pick; the second point's projection is
//! the new end, and the end nearest that point is the one that moves. The
//! interactive capture switches to this two-pick shape when the session
//! `lengthenMode` parameter is `3` (Dynamic).
//!
//! ## Endpoint rule
//!
//! `DE`/`PER`/`TOT` move the endpoint **nearest the pick point** (or, without a
//! pick, the historical far end): a LINE's `end` or `start`, an ARC's far or
//! near end, an open polyline's or spline's nearer end. The other endpoint is
//! kept fixed and the new length is measured from it. A negative Delta shortens
//! toward the kept end; it never flips the curve through it (a non-positive new
//! length is a refusal). Dynamic mode may move either end, because the picked
//! new-end point decides which end is nearer.
//!
//! An ARC's new sweep is `sign(old_sweep) · new_length / radius`. A sweep beyond
//! one full turn is `lengthen.sweep_overflow` (never wrapped or clamped), while a
//! sweep of **exactly** one turn is accepted and stays an `Arc` (this build does
//! not silently convert it into a `Circle`).
//!
//! ## Supported objects
//!
//! LINE, ARC, **open** LWPOLYLINE (length along the polyline; extending appends
//! a straight tangent segment at the moving end, trimming splits at the new
//! endpoint and preserves the bulges), elliptical arc (`Ellipse` with
//! `sweep < 1 turn`; length/angle semantics exactly like ARC), and **open**
//! SPLINE (trimming uses the exact arc-length sub-curve helpers; extending a
//! NURBS curve has no continuation semantics and is refused by name
//! `lengthen.spline_extend_unsupported`). Closed polylines, circles and full
//! ellipses are refused as `lengthen.closed`.
//!
//! ## Transaction shape
//!
//! Every refusal happens **before** `begin_drawing_transaction`, so a refused
//! LENGTHEN writes nothing: no revision bump, no `ChangeSet`, no undo entry. A
//! result that does not change the curve reports `lengthen.unchanged` and also
//! opens no transaction. On success the **same entity id** is updated with
//! `staged.update_entity` in one transaction / one undo step (layer, space, draw
//! order and `type_key` are preserved); the committed row is read back so the
//! undo patch carries the exact stored value. Work-only through
//! [`crate::CommandId::Lengthen`].

use super::trim_curves::{cut_tolerance, norm_tau};
use super::*;

mod curves;
use curves::{lengthen_ellipse, lengthen_polyline, lengthen_spline};

const TAU: f64 = std::f64::consts::TAU;

/// The internal mode tag for the typed `Points` dynamic form (never written to
/// the session channel, whose closed set is `0..=2`).
const MODE_DYNAMIC: f64 = 4.0;

/// A LENGTHEN refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The name a refused target kind is reported under (ASCII, no class prefix).
fn kind_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Polyline { .. } => "LWPOLYLINE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Arc { .. } => "ARC",
        _ => "that entity",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LengthenMode {
    Delta,
    Percent,
    Total,
    Dynamic,
}

impl LengthenMode {
    fn name(self) -> &'static str {
        match self {
            Self::Delta => "Delta",
            Self::Percent => "Percent",
            Self::Total => "Total",
            Self::Dynamic => "Dynamic",
        }
    }
}

/// The LENGTHEN mode and amount read from the session typed channel (or the
/// dynamic `Points` payload).
struct LengthenSettings {
    mode: LengthenMode,
    value: f64,
}

impl Application {
    /// `LENGTHEN` the one selected curve (see the module docs).
    ///
    /// The same entity id is updated in one transaction / one undo step; every
    /// refusal leaves the drawing untouched.
    pub(crate) fn lengthen_curve(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (refs, settings, dynamic, pick) = match &command.payload {
            CommandPayload::Lengthen { refs } => {
                (refs.clone(), lengthen_settings(session)?, None, None)
            }
            // The static modes' nearest-endpoint pick: one typed point chooses the
            // end that moves. The object itself is the session selection.
            CommandPayload::Points(points) if points.len() == 1 => {
                if !is_finite_point(points[0]) {
                    return Err(refusal(
                        "lengthen.invalid_point",
                        "the LENGTHEN pick point must be finite",
                    ));
                }
                let settings = lengthen_settings(session)?;
                if settings.mode == LengthenMode::Dynamic {
                    return Err(refusal(
                        "lengthen.invalid_payload",
                        "the dynamic LENGTHEN mode needs two picked points (object pick and new \
                         end), got one",
                    ));
                }
                (
                    session.selection.refs().to_vec(),
                    settings,
                    None,
                    Some(points[0]),
                )
            }
            CommandPayload::Points(points) if points.len() == 2 => {
                if !is_finite_point(points[0]) || !is_finite_point(points[1]) {
                    return Err(refusal(
                        "lengthen.invalid_point",
                        "the dynamic LENGTHEN points must be finite",
                    ));
                }
                (
                    session.selection.refs().to_vec(),
                    LengthenSettings {
                        mode: LengthenMode::Dynamic,
                        value: MODE_DYNAMIC,
                    },
                    Some([points[0], points[1]]),
                    None,
                )
            }
            other => {
                return Err(refusal(
                    "lengthen.invalid_payload",
                    format!(
                        "LENGTHEN needs a Lengthen payload, one static pick point or two typed \
                         dynamic points, got {} point(s)",
                        match other {
                            CommandPayload::Points(points) => points.len(),
                            _ => 0,
                        }
                    ),
                ))
            }
        };
        // 1. Exactly one selected curve.
        let [reference] = refs.as_slice() else {
            return Err(refusal(
                "lengthen.needs_one",
                format!(
                    "LENGTHEN needs exactly one selected curve, got {}",
                    refs.len()
                ),
            ));
        };
        // 2. Directly addressable model-space entities only.
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(refusal(
                "lengthen.not_addressable",
                format!(
                    "LENGTHEN target {} is a block-instance member or sub-element; only \
                     directly addressable model-space entities can be lengthened",
                    reference.entity.0
                ),
            ));
        }

        let document_id = command.document;
        // 3./4./5./6. Resolve the target and build the new geometry as plain data,
        //    before any transaction exists.
        let (before, after_geometry, old_length, new_length, changed) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let (sources, _diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                &refs,
                "lengthen",
                "LENGTHEN",
                "lengthen",
                false,
            )
            .map_err(|error| {
                refusal(
                    "lengthen.target_missing",
                    format!(
                        "LENGTHEN target {} does not resolve: {}",
                        reference.entity.0,
                        error_detail(error)
                    ),
                )
            })?;
            let before = sources.into_iter().next().ok_or_else(|| {
                refusal(
                    "lengthen.target_missing",
                    format!("LENGTHEN target {} does not resolve", reference.entity.0),
                )
            })?;
            if before.space != SpaceId::Model {
                return Err(refusal(
                    "lengthen.not_addressable",
                    format!(
                        "LENGTHEN target {} is not a model-space entity; only model space is \
                         supported",
                        before.id.0
                    ),
                ));
            }
            let (after_geometry, old_length, new_length, changed) =
                lengthen_geometry(&before.geometry, &settings, dynamic, pick)?;
            (before, after_geometry, old_length, new_length, changed)
        };

        // A result that leaves the stored curve exactly as it was is the named
        // no-op: no transaction, no revision bump, no undo entry.
        if !changed {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(before.id.0)),
                    code: "lengthen.unchanged".into(),
                    message: format!(
                        "LENGTHEN: {} leaves entity {} at length {old_length}; nothing changed",
                        settings.mode.name(),
                        before.id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let source_id = before.id;
        let mut after = before.clone();
        after.geometry = after_geometry;

        // 7. One transaction on the **same** entity (identity, layer, space, draw
        //    order and type key preserved; only the geometry changes).
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("lengthen curve", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed row back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let after = candidate
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("lengthened entity vanished after commit".into()))?;
        let object = ObjectId(source_id.0);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "lengthen curve".into(),
                patches: vec![drawing_patch(source_id, Some(before), Some(after))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![object],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(object),
                code: "drawing.lengthen".into(),
                message: format!(
                    "LENGTHEN: source {} mode {} ({}), old length {old_length}, new length \
                     {new_length}; the selected end moved",
                    source_id.0,
                    settings.mode.name(),
                    settings.value as i64
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The channel error's inner message (a named refusal keeps its own text).
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

/// Read and defensively re-validate the LENGTHEN mode and amount.
///
/// The channel refuses a zero amount and a mode outside `{0, 1, 2}` at set time;
/// this is the defense in depth for a value injected past the channel. Percent
/// and Total additionally require a **positive** amount here (Delta is signed).
fn lengthen_settings(session: &SessionState) -> CadResult<LengthenSettings> {
    let mode_parameter = ToolParameter::LengthenMode;
    let mode_value = session.tool_parameter(mode_parameter);
    mode_parameter.validate(mode_value).map_err(|_| {
        refusal(
            "lengthen.invalid_value",
            format!(
                "LENGTHEN mode must be 0 (Delta), 1 (Percent) or 2 (Total), got {mode_value:?}"
            ),
        )
    })?;
    let value_parameter = ToolParameter::LengthenValue;
    let value = session.tool_parameter(value_parameter);
    value_parameter.validate(value).map_err(|_| {
        refusal(
            "lengthen.invalid_value",
            format!("LENGTHEN amount must be finite and non-zero, got {value:?}"),
        )
    })?;
    let (mode, value) = match (mode_value, value) {
        (ToolValue::Number(mode), ToolValue::Number(value)) => (mode, value),
        // `validate` accepted both values, so this arm is unreachable; refuse
        // instead of inventing a mode or an amount.
        other => {
            return Err(refusal(
                "lengthen.invalid_value",
                format!("LENGTHEN needs a numeric mode and amount, got {other:?}"),
            ))
        }
    };
    let mode = if mode == 0.0 {
        LengthenMode::Delta
    } else if mode == 1.0 {
        LengthenMode::Percent
    } else if mode == 2.0 {
        LengthenMode::Total
    } else if mode == crate::draw_tool::LENGTHEN_MODE_DYNAMIC {
        // The dynamic shape is resolved from its points, not a scalar setting;
        // this arm is only reachable for a static `Lengthen { refs }` payload,
        // which the geometry builder then refuses by name.
        LengthenMode::Dynamic
    } else {
        return Err(refusal(
            "lengthen.invalid_value",
            format!(
                "LENGTHEN mode must be 0 (Delta), 1 (Percent), 2 (Total) or 3 (Dynamic), got \
                 {mode}"
            ),
        ));
    };
    if (mode == LengthenMode::Percent || mode == LengthenMode::Total) && value <= 0.0 {
        return Err(refusal(
            "lengthen.invalid_value",
            format!(
                "LENGTHEN {} needs a positive amount, got {value}",
                mode.name()
            ),
        ));
    }
    Ok(LengthenSettings { mode, value })
}

/// Build the lengthened geometry, returning it with the old/new lengths and
/// whether the stored curve actually changed.
fn lengthen_geometry(
    geometry: &SemanticGeometry,
    settings: &LengthenSettings,
    dynamic: Option<[Point3; 2]>,
    pick: Option<Point3>,
) -> CadResult<(SemanticGeometry, f64, f64, bool)> {
    let eps = cut_tolerance();
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let old = distance(*start, *end);
            if !old.is_finite() || old <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target LINE has no measurable length",
                ));
            }
            let (new_start, new_end) = if settings.mode == LengthenMode::Dynamic {
                let points = dynamic.ok_or_else(|| {
                    refusal(
                        "lengthen.invalid_payload",
                        "the dynamic LENGTHEN mode needs two picked points",
                    )
                })?;
                dynamic_line(*start, *end, points)?
            } else {
                let new = new_line_length(old, settings)?;
                let unit = Point3 {
                    x: (end.x - start.x) / old,
                    y: (end.y - start.y) / old,
                    z: (end.z - start.z) / old,
                };
                // The endpoint nearest the pick (the far end when there is no
                // pick) moves; the other endpoint is kept fixed.
                if pick.is_some_and(|point| distance(point, *start) < distance(point, *end)) {
                    let new_start = Point3 {
                        x: end.x - unit.x * new,
                        y: end.y - unit.y * new,
                        z: end.z - unit.z * new,
                    };
                    if !is_finite_point(new_start) {
                        return Err(refusal(
                            "lengthen.non_positive",
                            "the LENGTHEN result is not finite",
                        ));
                    }
                    (new_start, *end)
                } else {
                    let new_end = Point3 {
                        x: start.x + unit.x * new,
                        y: start.y + unit.y * new,
                        z: start.z + unit.z * new,
                    };
                    if !is_finite_point(new_end) {
                        return Err(refusal(
                            "lengthen.non_positive",
                            "the LENGTHEN result is not finite",
                        ));
                    }
                    (*start, new_end)
                }
            };
            let new = distance(new_start, new_end);
            if !new.is_finite() || new <= eps {
                return Err(refusal(
                    "lengthen.non_positive",
                    "the LENGTHEN result has no measurable length",
                ));
            }
            let changed = distance(new_start, *start) > eps || distance(new_end, *end) > eps;
            Ok((
                SemanticGeometry::Line {
                    start: new_start,
                    end: new_end,
                },
                old,
                new,
                changed,
            ))
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let radius = radius.abs();
            if !radius.is_finite() || radius <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target ARC has no measurable length",
                ));
            }
            let old = radius * sweep.abs();
            if !old.is_finite() || old <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target ARC has no measurable length",
                ));
            }
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let end_angle = start + sweep;
            let start_point = arc_point(*center, ax, ay, radius, *start);
            let end_point = arc_point(*center, ax, ay, radius, end_angle);
            let move_start =
                pick.is_some_and(|point| distance(point, start_point) < distance(point, end_point));
            let (new_start, new_sweep) = match settings.mode {
                LengthenMode::Dynamic => {
                    let points = dynamic.ok_or_else(|| {
                        refusal(
                            "lengthen.invalid_payload",
                            "the dynamic LENGTHEN mode needs two picked points",
                        )
                    })?;
                    dynamic_arc(*center, *normal, radius, *start, *sweep, points)?
                }
                // TOT on an ARC is the included angle in degrees, not a length.
                LengthenMode::Total => {
                    let angle = settings.value.to_radians();
                    if !angle.is_finite() {
                        return Err(refusal(
                            "lengthen.invalid_value",
                            format!("LENGTHEN Total angle is not finite: {angle} rad"),
                        ));
                    }
                    let new_sweep = sweep.signum() * angle.abs();
                    if move_start {
                        (end_angle - new_sweep, new_sweep)
                    } else {
                        (*start, new_sweep)
                    }
                }
                _ => {
                    let new = new_line_length(old, settings)?;
                    let new_sweep = sweep.signum() * new / radius;
                    if move_start {
                        (end_angle - new_sweep, new_sweep)
                    } else {
                        (*start, new_sweep)
                    }
                }
            };
            if !new_sweep.is_finite() || new_sweep.abs() <= eps {
                return Err(refusal(
                    "lengthen.non_positive",
                    "the LENGTHEN result would have no sweep",
                ));
            }
            if new_sweep.abs() > TAU + 1e-9 {
                return Err(refusal(
                    "lengthen.sweep_overflow",
                    format!(
                        "the LENGTHEN result sweeps {:.9} rad, more than one full turn ({TAU}); \
                         it is refused rather than wrapped",
                        new_sweep.abs()
                    ),
                ));
            }
            let new = radius * new_sweep.abs();
            let changed = (new_start - *start).abs() > 1e-12 || (new_sweep - *sweep).abs() > 1e-12;
            // Keep centre/normal/radius; the selected end moves. A sweep of exactly
            // one turn stays an ARC (never silently a CIRCLE).
            Ok((
                SemanticGeometry::Arc {
                    center: *center,
                    normal: *normal,
                    radius,
                    start: new_start,
                    sweep: new_sweep,
                },
                old,
                new,
                changed,
            ))
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => lengthen_polyline(points, bulges, *closed, settings, pick),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => lengthen_ellipse(
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            settings,
            pick,
        ),
        SemanticGeometry::Spline { .. } => lengthen_spline(geometry, settings, pick),
        // A closed object has no ends to move: it is unaffected by LENGTHEN.
        SemanticGeometry::Circle { .. } => Err(refusal(
            "lengthen.closed",
            "LENGTHEN does not affect a closed object such as a CIRCLE",
        )),
        other => Err(CadError::Unsupported(format!(
            "LENGTHEN supports LINE, ARC, ELLIPSE, open SPLINE and open LWPOLYLINE only; {} is \
             not supported",
            kind_name(other)
        ))),
    }
}

/// The new length of a LINE, or of an ARC before converting it to a sweep.
///
/// Requires a finite, strictly positive result: a zero/negative length has no
/// direction and is refused (`lengthen.non_positive`), never clamped.
fn new_line_length(old: f64, settings: &LengthenSettings) -> CadResult<f64> {
    let eps = cut_tolerance();
    let new = match settings.mode {
        LengthenMode::Delta => old + settings.value,
        LengthenMode::Percent => old * settings.value / 100.0,
        LengthenMode::Total => settings.value,
        LengthenMode::Dynamic => {
            return Err(refusal(
                "lengthen.invalid_payload",
                "the dynamic LENGTHEN mode is resolved from its points, not a scalar length",
            ))
        }
    };
    if !new.is_finite() || new <= eps {
        return Err(refusal(
            "lengthen.non_positive",
            format!(
                "LENGTHEN {} produces the non-positive length {new} (old length {old})",
                settings.mode.name()
            ),
        ));
    }
    Ok(new)
}

/// The dynamic LINE endpoints: the picked new-end point projected onto the
/// line, with the nearer end moved.
fn dynamic_line(start: Point3, end: Point3, points: [Point3; 2]) -> CadResult<(Point3, Point3)> {
    let eps = cut_tolerance();
    let new_point = points[1];
    let d = Point3 {
        x: end.x - start.x,
        y: end.y - start.y,
        z: end.z - start.z,
    };
    let len2 = d.x * d.x + d.y * d.y + d.z * d.z;
    if !len2.is_finite() || len2 <= 0.0 {
        return Err(refusal(
            "lengthen.degenerate",
            "the dynamic LENGTHEN LINE has no measurable length",
        ));
    }
    let t = ((new_point.x - start.x) * d.x
        + (new_point.y - start.y) * d.y
        + (new_point.z - start.z) * d.z)
        / len2;
    if !t.is_finite() {
        return Err(refusal(
            "lengthen.projection_failed",
            "the dynamic LENGTHEN point has no finite projection onto the line",
        ));
    }
    let projected = Point3 {
        x: start.x + d.x * t,
        y: start.y + d.y * t,
        z: start.z + d.z * t,
    };
    let move_start = distance(new_point, start) < distance(new_point, end);
    let (new_start, new_end) = if move_start {
        (projected, end)
    } else {
        (start, projected)
    };
    let length = distance(new_start, new_end);
    if !length.is_finite() || length <= eps {
        return Err(refusal(
            "lengthen.non_positive",
            "the dynamic LENGTHEN result has no measurable length",
        ));
    }
    Ok((new_start, new_end))
}

/// The dynamic ARC start/sweep: the picked new-end point projected radially
/// onto the circle, with the nearer end moved.
fn dynamic_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    points: [Point3; 2],
) -> CadResult<(f64, f64)> {
    let eps = cut_tolerance();
    let new_point = points[1];
    let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
    let v = Point3 {
        x: new_point.x - center.x,
        y: new_point.y - center.y,
        z: new_point.z - center.z,
    };
    let rx = cad_geometry::dot(v, ax);
    let ry = cad_geometry::dot(v, ay);
    if !rx.is_finite() || !ry.is_finite() || rx.hypot(ry) <= eps {
        return Err(refusal(
            "lengthen.projection_failed",
            "the dynamic LENGTHEN point lies on the arc axis and has no radial projection",
        ));
    }
    let angle = ry.atan2(rx);
    let end_angle = start + sweep;
    let start_point = arc_point(center, ax, ay, radius, start);
    let end_point = arc_point(center, ax, ay, radius, end_angle);
    let move_start = distance(new_point, start_point) < distance(new_point, end_point);
    let (new_start, new_sweep) = if move_start {
        (angle, signed_sweep(angle, end_angle, sweep.signum()))
    } else {
        (start, signed_sweep(start, angle, sweep.signum()))
    };
    Ok((new_start, new_sweep))
}

fn arc_point(center: Point3, ax: Point3, ay: Point3, radius: f64, angle: f64) -> Point3 {
    Point3 {
        x: center.x + radius * (angle.cos() * ax.x + angle.sin() * ay.x),
        y: center.y + radius * (angle.cos() * ax.y + angle.sin() * ay.y),
        z: center.z + radius * (angle.cos() * ax.z + angle.sin() * ay.z),
    }
}

/// The sweep from `from` to `to` in the direction `sign` (`+1` / `-1`).
fn signed_sweep(from: f64, to: f64, sign: f64) -> f64 {
    if sign >= 0.0 {
        norm_tau(to - from)
    } else {
        -norm_tau(from - to)
    }
}
