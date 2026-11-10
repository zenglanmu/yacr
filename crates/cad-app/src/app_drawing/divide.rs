//! `DIVIDE`: place POINT marks (or block references) at equal **arc-length**
//! divisions of one curve.
//!
//! The capture is the zero-point pre-selection shape
//! ([`crate::draw_tool::DrawToolKind::Divide`]): the one selected curve is the
//! whole input and the segment count is the session typed channel
//! ([`crate::draw_tool::ToolParameter::DivideSegments`], an integer in `2..=1024`,
//! default `2`). Nothing is picked inside the command.
//!
//! ## Point placement (equal arc length, never raw parameter)
//!
//! | target | marks |
//! | --- | --- |
//! | `LINE`, `ARC` (open) | `N − 1` interior POINTs at arc length `k/N`, `k = 1..N−1` |
//! | `CIRCLE`, a full-turn `ARC`, a full `ELLIPSE` (periodic) | `N` POINTs at `k/N`, `k = 0..N−1` |
//! | `SPLINE` | `N − 1` POINTs at equal arc length of the sampled curve |
//! | open `LWPOLYLINE` (straight **or bulged**) | `N − 1` POINTs at arc length `k/N · total` |
//! | closed `LWPOLYLINE` (straight **or bulged**) | `N` POINTs at arc length `k/N · total` |
//! | elliptical arc (open) | `N − 1` POINTs at equal ellipse arc length |
//!
//! A LINE/ARC/CIRCLE uses the shared [`TargetCurve`] model TRIM/EXTEND/BREAK use,
//! whose `point_at` is already arc-length proportional for those kinds. SPLINE
//! and ELLIPSE instead use a local adaptive chord table and invert it to equal
//! arc length; a spline's raw knot parameter is never used directly. A LWPOLYLINE
//! is walked by an exact per-segment table: straight chords and circular bulges
//! (`radius · |sweep|`), so a bulged segment is divided exactly, not approximated.
//!
//! Every mark is a new `POINT` on the **active layer** in **model space**, in one
//! transaction / one undo step, through the shared create funnel; the source curve
//! is **retained**.
//!
//! ## Block option
//!
//! AutoCAD's `DIVIDE … B` inserts a named block at each division point instead of
//! a POINT. That option is accepted through the existing typed
//! `InsertBlock { name, .. }` payload: `name` is the existing block to place, the
//! `insertion` field is a finite placeholder (the division points are the real
//! insertions), and the curve selection comes from the session. The interactive
//! capture selects this shape when the session `divideUseBlock` flag is set: the
//! terminal text step supplies the block name. Each mark becomes an independent
//! `INSERT` on the active layer, with the importer's own
//! `T(insertion) · T(−base_point)` transform and uniform scale `1`.
//!
//! AutoCAD 2026 then asks "Align block with object? [Yes/No]"
//! ([`crate::draw_tool::ToolParameter::DivideAlignBlock`], default No). With
//! Yes, each block is rotated to the object's **local tangent direction at its
//! division point** (the transform becomes
//! `T(insertion) · R(tangent) · T(−base_point)`); with No every block keeps
//! rotation `0`. Uniform scale stays `1` either way.
//!
//! ## Refusals (all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `divide.needs_one` | `refs` does not hold exactly one selected entity |
//! | `divide.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `divide.target_missing` | the ref does not resolve (or belongs to another document) |
//! | `divide.invalid_segments` | the session count is not an integer in `2..=1024` |
//! | `divide.degenerate` | non-finite/degenerate geometry, fewer than two vertices, non-positive total length |
//! | `divide.invalid_name` / `divide.unknown_block` / `divide.block_metadata_missing` | the block option's name or definition |
//! | `divide.bulged_segment` | a non-planar polyline carries a bulge (no circular arc exists) |
//!
//! Every other geometry kind is the explicit [`CadError::Unsupported`]. Work-only
//! through [`crate::CommandId::Divide`].

use super::arc_length::ArcLengthTable;
use super::polyline_curve::PolylineCurve;
use super::trim_curves::{cut_tolerance, TargetCurve};
use super::*;
use crate::draw_tool::rotation_transform;

/// A DIVIDE refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The name a refused target kind is reported under (ASCII, no class prefix).
fn kind_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Point(_) => "POINT",
        SemanticGeometry::Text { .. } => "TEXT",
        SemanticGeometry::Shape { .. } => "SHAPE",
        SemanticGeometry::Insert { .. } => "INSERT",
        SemanticGeometry::Hatch(_) => "HATCH",
        SemanticGeometry::MultiLeader(_) => "MULTILEADER",
        SemanticGeometry::Polyline { .. } => "LWPOLYLINE",
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Arc { .. } => "ARC",
        SemanticGeometry::Circle { .. } => "CIRCLE",
        _ => "that entity",
    }
}

/// What the command should insert at each division point.
enum DivideMarks {
    Points,
    Block { name: String },
}

impl Application {
    /// `DIVIDE` the one selected curve into equal arc-length parts.
    ///
    /// The marks are POINT entities on the active layer in model space (or
    /// independent INSERTs for the block option); the source curve is retained.
    /// One transaction / one undo step, and every refusal happens before it is
    /// opened (no revision bump, no `ChangeSet`, no undo entry).
    pub(crate) fn divide_curve(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (refs, marks_kind) = match &command.payload {
            CommandPayload::Divide { refs } => (refs.clone(), DivideMarks::Points),
            // The block option's name channel (see the module docs).
            CommandPayload::InsertBlock { insertion, name } => {
                if !is_finite_point(*insertion) {
                    return Err(refusal(
                        "divide.invalid_point",
                        "the DIVIDE block placeholder point must be finite",
                    ));
                }
                (
                    session.selection.refs().to_vec(),
                    DivideMarks::Block { name: name.clone() },
                )
            }
            other => {
                return Err(refusal(
                    "divide.invalid_payload",
                    format!(
                        "DIVIDE needs a Divide payload or an InsertBlock name, got {} point(s)",
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
                "divide.needs_one",
                format!(
                    "DIVIDE needs exactly one selected curve, got {}",
                    refs.len()
                ),
            ));
        };
        // 2. Directly addressable model-space entities only. The check runs
        //    before the drawing is read so a block-instance member or a
        //    sub-element is never mistaken for a whole selectable curve.
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(refusal(
                "divide.not_addressable",
                format!(
                    "DIVIDE target {} is a block-instance member or sub-element; only \
                     directly addressable model-space entities can be divided",
                    reference.entity.0
                ),
            ));
        }
        let segments = divide_segments(session)?;

        let document_id = command.document;
        let (source_id, marks, closed) = {
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
                "divide",
                "DIVIDE",
                "divide",
                false,
            )
            .map_err(|error| {
                refusal(
                    "divide.target_missing",
                    format!(
                        "DIVIDE target {} does not resolve: {}",
                        reference.entity.0,
                        error_detail(error)
                    ),
                )
            })?;
            let source = sources.into_iter().next().ok_or_else(|| {
                refusal(
                    "divide.target_missing",
                    format!("DIVIDE target {} does not resolve", reference.entity.0),
                )
            })?;
            if source.space != SpaceId::Model {
                return Err(refusal(
                    "divide.not_addressable",
                    format!(
                        "DIVIDE target {} is not a model-space entity; only model space is \
                         supported",
                        source.id.0
                    ),
                ));
            }
            let (marks, closed) = division_points(&source.geometry, segments)?;
            (source.id, marks, closed)
        };

        let mark_count = marks.len();
        let (geometries, message) = match marks_kind {
            DivideMarks::Points => (
                marks
                    .into_iter()
                    .map(|(insertion, _tangent)| SemanticGeometry::Point(insertion))
                    .collect(),
                format!(
                    "DIVIDE: source {} split into {segments} equal arc-length parts ({mark_count} \
                     POINT(s) on the {} curve)",
                    source_id.0,
                    if closed { "closed" } else { "open" }
                ),
            ),
            DivideMarks::Block { name } => {
                let (block, block_name, base_point) = resolve_divide_block(
                    self.workspace
                        .documents
                        .get(&document_id)
                        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
                        .drawing
                        .as_ref(),
                    &name,
                )?;
                let align = matches!(
                    session.tool_parameter(ToolParameter::DivideAlignBlock),
                    ToolValue::Flag(true)
                );
                let to_origin = Transform3::translation(Point3 {
                    x: -base_point.x,
                    y: -base_point.y,
                    z: -base_point.z,
                });
                let geometries = marks
                    .into_iter()
                    .map(|(insertion, tangent)| {
                        // "Align block with object? [Yes/No]": yes rotates each
                        // block to the curve's local tangent at its division point;
                        // no keeps rotation 0. Uniform scale is 1 either way.
                        let rotation = if align { tangent } else { 0.0 };
                        let rotate = rotation_transform(
                            Point3 {
                                x: 0.0,
                                y: 0.0,
                                z: 0.0,
                            },
                            rotation,
                        );
                        SemanticGeometry::Insert {
                            block,
                            transform: Transform3::translation(insertion)
                                .matrix_mul(&rotate)
                                .matrix_mul(&to_origin),
                        }
                    })
                    .collect();
                (
                    geometries,
                    format!(
                        "DIVIDE: source {} split into {segments} equal arc-length parts \
                         ({mark_count} INSERT(s) of block '{block_name}' on the active layer, \
                         uniform scale 1, rotation {})",
                        source_id.0,
                        if align {
                            "aligned to the curve tangent"
                        } else {
                            "0"
                        }
                    ),
                )
            }
        };
        let mut outcome = self.commit_new_entities(session, command, geometries, "divide curve")?;
        outcome.diagnostics.push(Diagnostic {
            object: Some(ObjectId(source_id.0)),
            code: "drawing.divide".into(),
            message,
        });
        Ok(outcome)
    }
}

/// Resolve the named block and its base point for the block option.
fn resolve_divide_block(
    drawing: &cad_db::DrawingDatabase,
    name: &str,
) -> CadResult<(cad_domain::BlockId, String, Point3)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(refusal(
            "divide.invalid_name",
            "DIVIDE needs a non-empty block name",
        ));
    }
    if name.contains('\r') || name.contains('\n') {
        return Err(refusal(
            "divide.invalid_name",
            "the DIVIDE block name must be a single line",
        ));
    }
    let characters = name.chars().count();
    if characters > MAX_TOOL_TEXT_CHARS {
        return Err(refusal(
            "divide.invalid_name",
            format!("DIVIDE allows at most {MAX_TOOL_TEXT_CHARS} characters, got {characters}"),
        ));
    }
    let definition = drawing.block_by_name(name).ok_or_else(|| {
        refusal(
            "divide.unknown_block",
            format!("no block named '{name}' in this drawing"),
        )
    })?;
    let metadata = drawing.block_metadata(definition.id).ok_or_else(|| {
        refusal(
            "divide.block_metadata_missing",
            format!("block {:?} has no stored name/base point", definition.id),
        )
    })?;
    Ok((definition.id, metadata.name.clone(), metadata.base_point))
}

/// The channel error's inner message (a named refusal keeps its own text).
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

/// Read and defensively re-validate the DIVIDE segment count.
///
/// The typed channel already refuses an out-of-range value at set time; this is
/// the defense in depth for a value injected past the channel. It is a named
/// refusal (`divide.invalid_segments`), never a clamp.
fn divide_segments(session: &SessionState) -> CadResult<usize> {
    let parameter = ToolParameter::DivideSegments;
    let value = session.tool_parameter(parameter);
    parameter.validate(value).map_err(|_| {
        refusal(
            "divide.invalid_segments",
            format!("DIVIDE needs an integer segment count in 2..=1024, got {value:?}"),
        )
    })?;
    match value {
        ToolValue::Number(number) => Ok(number as usize),
        // `validate` accepted the value, so this arm is unreachable; refuse
        // instead of inventing a count.
        other => Err(refusal(
            "divide.invalid_segments",
            format!("DIVIDE needs a numeric segment count, got {other:?}"),
        )),
    }
}

/// The mark positions for `segments` equal arc-length divisions of `geometry`
/// (each with the curve's local tangent angle), plus whether the curve is
/// closed/periodic.
fn division_points(
    geometry: &SemanticGeometry,
    segments: usize,
) -> CadResult<(Vec<(Point3, f64)>, bool)> {
    match geometry {
        // LINE/ARC/CIRCLE: the shared parameter model, whose `point_at` is
        // arc-length proportional for these kinds. A periodic curve (a CIRCLE, or
        // an ARC of exactly one turn) has no interior, so its marks include the
        // start parameter.
        SemanticGeometry::Line { .. }
        | SemanticGeometry::Arc { .. }
        | SemanticGeometry::Circle { .. } => {
            let curve = TargetCurve::new(
                geometry,
                "DIVIDE",
                "LINE, ARC, CIRCLE, SPLINE, ELLIPSE or LWPOLYLINE",
            )?;
            let length = curve.length();
            let eps = cut_tolerance();
            if !length.is_finite() || length <= eps {
                return Err(refusal(
                    "divide.degenerate",
                    format!(
                        "the DIVIDE target {} has no measurable length",
                        curve.label()
                    ),
                ));
            }
            let closed = curve.is_periodic();
            let count = if closed { segments } else { segments - 1 };
            let first = if closed { 0 } else { 1 };
            let marks = (first..first + count)
                .map(|k| {
                    let u = k as f64 / segments as f64;
                    (curve.point_at(u), curve_tangent_angle(&curve, u))
                })
                .collect();
            Ok((marks, closed))
        }
        // SPLINE: sample the exact curve, build a chord-length table and invert
        // it. The raw knot parameter is deliberately never used as "length".
        SemanticGeometry::Spline { .. } => {
            let curve = TargetCurve::new(
                geometry,
                "DIVIDE",
                "LINE, ARC, CIRCLE, SPLINE, ELLIPSE or LWPOLYLINE",
            )?;
            sampled_marks(|u| curve.point_at(u), false, segments)
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => ellipse_division_points(
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            segments,
        ),
        // A LWPOLYLINE: exact straight chords and exact circular bulges.
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.iter().any(|point| !is_finite_point(*point)) {
                return Err(refusal(
                    "divide.degenerate",
                    "DIVIDE polyline vertices must be finite",
                ));
            }
            let curve = PolylineCurve::new(points, bulges, *closed, "divide")?;
            let total = curve.total_length();
            let eps = cut_tolerance();
            if !total.is_finite() || total <= eps {
                return Err(refusal(
                    "divide.degenerate",
                    "DIVIDE polyline has no measurable length",
                ));
            }
            let count = if *closed { segments } else { segments - 1 };
            let first = if *closed { 0 } else { 1 };
            let marks = (first..first + count)
                .map(|k| {
                    let distance = total * k as f64 / segments as f64;
                    (
                        curve.point_at_distance(distance),
                        polyline_tangent_angle(&curve, distance, total),
                    )
                })
                .collect();
            Ok((marks, *closed))
        }
        other => Err(CadError::Unsupported(format!(
            "DIVIDE supports LINE, ARC, CIRCLE, SPLINE, ELLIPSE and LWPOLYLINE only; {} is not \
             supported",
            kind_name(other)
        ))),
    }
}

/// The path kinds an ARRAYPATH distribution supports (exactly DIVIDE's set).
const ARRAY_PATH_KINDS: &str = "LINE, ARC, CIRCLE, SPLINE, ELLIPSE or LWPOLYLINE";

/// The equal-arc-length positions along `geometry` for an ARRAYPATH.
///
/// `count` positions sit at `i / (count - 1)` of the path arc length for
/// `count >= 2` (**both ends included**); `count == 1` places the single position
/// at the path start. Each position carries the curve's local XY tangent angle in
/// radians. The supported kinds and the underlying curve model are exactly
/// [`division_points`]'s; every other kind and a path with no measurable length
/// is an explicit refusal, never a fabricated position.
pub(super) fn path_distribution(
    geometry: &SemanticGeometry,
    count: usize,
) -> CadResult<Vec<(Point3, f64)>> {
    if count == 0 {
        return Err(refusal(
            "arraypath.invalid_count",
            "ARRAYPATH needs a positive position count",
        ));
    }
    let fractions: Vec<f64> = if count == 1 {
        vec![0.0]
    } else {
        (0..count)
            .map(|index| index as f64 / (count - 1) as f64)
            .collect()
    };
    match geometry {
        SemanticGeometry::Line { .. }
        | SemanticGeometry::Arc { .. }
        | SemanticGeometry::Circle { .. } => {
            let curve = TargetCurve::new(geometry, "ARRAYPATH", ARRAY_PATH_KINDS)?;
            let length = curve.length();
            let eps = cut_tolerance();
            if !length.is_finite() || length <= eps {
                return Err(path_degenerate("LINE/ARC/CIRCLE"));
            }
            Ok(fractions
                .iter()
                .map(|&u| (curve.point_at(u), curve_tangent_angle(&curve, u)))
                .collect())
        }
        SemanticGeometry::Spline { .. } => {
            let curve = TargetCurve::new(geometry, "ARRAYPATH", ARRAY_PATH_KINDS)?;
            sampled_path_positions(|u| curve.point_at(u), &fractions)
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let point_at =
                ellipse_path_point_at(*center, *normal, *major_axis, *ratio, *start, *sweep)?;
            sampled_path_positions(point_at, &fractions)
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.iter().any(|point| !is_finite_point(*point)) {
                return Err(refusal(
                    "arraypath.degenerate",
                    "ARRAYPATH polyline vertices must be finite",
                ));
            }
            let curve = PolylineCurve::new(points, bulges, *closed, "arraypath")?;
            let total = curve.total_length();
            let eps = cut_tolerance();
            if !total.is_finite() || total <= eps {
                return Err(path_degenerate("LWPOLYLINE"));
            }
            Ok(fractions
                .iter()
                .map(|&fraction| {
                    let distance = total * fraction;
                    (
                        curve.point_at_distance(distance),
                        polyline_tangent_angle(&curve, distance, total),
                    )
                })
                .collect())
        }
        other => Err(CadError::Unsupported(format!(
            "ARRAYPATH supports LINE, ARC, CIRCLE, SPLINE, ELLIPSE and LWPOLYLINE only; {} is not \
             supported",
            kind_name(other)
        ))),
    }
}

/// The `arraypath.degenerate` refusal for a path with no measurable length.
fn path_degenerate(kind: &str) -> CadError {
    refusal(
        "arraypath.degenerate",
        format!("the ARRAYPATH path {kind} has no measurable length"),
    )
}

/// Equal-arc-length positions on a sampled curve at the given normalized
/// distances, each with its local tangent angle.
fn sampled_path_positions(
    point_at: impl Fn(f64) -> Point3,
    fractions: &[f64],
) -> CadResult<Vec<(Point3, f64)>> {
    let table = ArcLengthTable::sample(&point_at);
    let total = table.total();
    let eps = cut_tolerance();
    if !total.is_finite() || total <= eps {
        return Err(path_degenerate("sampled"));
    }
    let h = 1e-6;
    Ok(fractions
        .iter()
        .map(|&fraction| {
            let u = table.parameter_at(total * fraction);
            (
                point_at(u),
                direction_angle(point_at(u + h), point_at(u - h)),
            )
        })
        .collect())
}

/// The validated ELLIPSE parameter mapping used by [`path_distribution`].
fn ellipse_path_point_at(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
) -> CadResult<impl Fn(f64) -> Point3> {
    let eps = cut_tolerance();
    let major_len = cad_geometry::length(major_axis);
    let normal_len = cad_geometry::length(normal);
    let ratio = ratio.abs();
    if !major_len.is_finite()
        || major_len <= eps
        || !normal_len.is_finite()
        || normal_len <= eps
        || !ratio.is_finite()
        || ratio <= eps
        || !sweep.is_finite()
        || sweep.abs() <= eps
    {
        return Err(path_degenerate("ELLIPSE"));
    }
    if sweep.abs() > std::f64::consts::TAU + 1e-9 {
        return Err(refusal(
            "arraypath.degenerate",
            format!("the ARRAYPATH path ELLIPSE sweeps more than one full turn: {sweep}"),
        ));
    }
    let unit_major = cad_geometry::scale(major_axis, 1.0 / major_len);
    let cross = cad_geometry::cross(normal, unit_major);
    let cross_len = cad_geometry::length(cross);
    if !cross_len.is_finite() || cross_len <= eps {
        return Err(path_degenerate("ELLIPSE"));
    }
    let unit_minor = cad_geometry::scale(cross, 1.0 / cross_len);
    let minor = cad_geometry::scale(unit_minor, major_len * ratio);
    Ok(move |u: f64| {
        let angle = start + sweep * u;
        Point3 {
            x: center.x + angle.cos() * major_axis.x + angle.sin() * minor.x,
            y: center.y + angle.cos() * major_axis.y + angle.sin() * minor.y,
            z: center.z + angle.cos() * major_axis.z + angle.sin() * minor.z,
        }
    })
}

/// The equal-arc-length marks of an ELLIPSE (or elliptical arc).
fn ellipse_division_points(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    segments: usize,
) -> CadResult<(Vec<(Point3, f64)>, bool)> {
    let eps = cut_tolerance();
    let major_len = cad_geometry::length(major_axis);
    let normal_len = cad_geometry::length(normal);
    let ratio = ratio.abs();
    if !major_len.is_finite()
        || major_len <= eps
        || !normal_len.is_finite()
        || normal_len <= eps
        || !ratio.is_finite()
        || ratio <= eps
        || !sweep.is_finite()
        || sweep.abs() <= eps
    {
        return Err(refusal(
            "divide.degenerate",
            "the DIVIDE target ELLIPSE has no measurable length",
        ));
    }
    if sweep.abs() > std::f64::consts::TAU + 1e-9 {
        return Err(refusal(
            "divide.degenerate",
            format!("the DIVIDE target ELLIPSE sweeps more than one full turn: {sweep}"),
        ));
    }
    let unit_major = cad_geometry::scale(major_axis, 1.0 / major_len);
    let cross = cad_geometry::cross(normal, unit_major);
    let cross_len = cad_geometry::length(cross);
    if !cross_len.is_finite() || cross_len <= eps {
        return Err(refusal(
            "divide.degenerate",
            "the DIVIDE target ELLIPSE has a degenerate minor axis",
        ));
    }
    let unit_minor = cad_geometry::scale(cross, 1.0 / cross_len);
    let minor = cad_geometry::scale(unit_minor, major_len * ratio);
    let point_at = |u: f64| {
        let angle = start + sweep * u;
        Point3 {
            x: center.x + angle.cos() * major_axis.x + angle.sin() * minor.x,
            y: center.y + angle.cos() * major_axis.y + angle.sin() * minor.y,
            z: center.z + angle.cos() * major_axis.z + angle.sin() * minor.z,
        }
    };
    let closed = sweep.abs() >= std::f64::consts::TAU - 1e-9;
    sampled_marks(point_at, closed, segments)
}

/// Equal arc-length marks on a sampled curve, each with its local tangent angle.
fn sampled_marks(
    point_at: impl Fn(f64) -> Point3,
    closed: bool,
    segments: usize,
) -> CadResult<(Vec<(Point3, f64)>, bool)> {
    let table = ArcLengthTable::sample(&point_at);
    let total = table.total();
    let eps = cut_tolerance();
    if !total.is_finite() || total <= eps {
        return Err(refusal(
            "divide.degenerate",
            "the DIVIDE target has no measurable length",
        ));
    }
    let count = if closed { segments } else { segments - 1 };
    let first = if closed { 0 } else { 1 };
    let h = 1e-6;
    let marks = (first..first + count)
        .map(|k| {
            let u = table.parameter_at(total * k as f64 / segments as f64);
            (
                point_at(u),
                direction_angle(point_at(u + h), point_at(u - h)),
            )
        })
        .collect();
    Ok((marks, closed))
}

/// The tangent direction angle (radians) from `from` to `to` in the XY plane.
fn direction_angle(to: Point3, from: Point3) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if !dx.is_finite() || !dy.is_finite() || dx.hypot(dy) <= 1e-12 {
        0.0
    } else {
        dy.atan2(dx)
    }
}

/// The tangent angle of a LINE/ARC/CIRCLE/SPLINE target at parameter `u`.
fn curve_tangent_angle(curve: &TargetCurve, u: f64) -> f64 {
    let h = 1e-6;
    direction_angle(curve.point_at(u + h), curve.point_at(u - h))
}

/// The tangent angle of a polyline at `distance` along its arc length.
fn polyline_tangent_angle(curve: &PolylineCurve, distance: f64, total: f64) -> f64 {
    let h = (total * 1e-6).max(1e-9);
    direction_angle(
        curve.point_at_distance(distance + h),
        curve.point_at_distance(distance - h),
    )
}
