//! `DIVIDE`: place POINT marks at equal **arc-length** divisions of one curve.
//!
//! The capture is the zero-point pre-selection shape
//! ([`crate::draw_tool::DrawToolKind::Divide`]): the one selected curve is the
//! whole input and the segment count is the session typed channel
//! ([`crate::draw_tool::ToolParameter::DivideSegments`], an integer in `2..=1024`,
//! default `2`). Nothing is picked inside the command.
//!
//! ## Point placement (equal arc length, never parameter)
//!
//! | target | marks |
//! | --- | --- |
//! | `LINE`, `ARC` (open) | `N − 1` interior POINTs at parameter `k/N`, `k = 1..N−1` |
//! | `CIRCLE`, a full-turn `ARC` (periodic) | `N` POINTs at `k/N`, `k = 0..N−1` (including the start parameter) |
//! | open straight `LWPOLYLINE` | `N − 1` POINTs at `k/N · total` arc length, `k = 1..N−1` |
//! | closed straight `LWPOLYLINE` | `N` POINTs at `k/N · total`, `k = 0..N−1` |
//!
//! A LINE/ARC/CIRCLE uses the shared [`TargetCurve`] model TRIM/EXTEND/BREAK use,
//! whose `point_at` is already arc-length proportional for those kinds. A
//! **periodic** curve (a CIRCLE, or an ARC whose `|sweep|` is exactly one turn)
//! has no interior, so its `N` draws include the start parameter; an open curve
//! has `N − 1` interior marks. A straight LWPOLYLINE is walked by a locally built
//! cumulative arc-length table — `cad-geometry` is deliberately untouched — and a
//! closed ring appends its first vertex to close the loop (unless the last vertex
//! already repeats it) so the seam is measured exactly once.
//!
//! Every mark is a new `POINT` on the **active layer** in **model space**, in one
//! transaction / one undo step, through the shared create funnel; the source curve
//! is **retained**.
//!
//! ## Refusals (all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `divide.needs_one` | `refs` does not hold exactly one selected entity |
//! | `divide.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `divide.target_missing` | the ref does not resolve (or belongs to another document) |
//! | `divide.invalid_segments` | the session count is not an integer in `2..=1024` |
//! | `divide.bulged_segment` | a LWPOLYLINE with a non-zero bulge (straight segments only) |
//! | `divide.degenerate` | fewer than two polyline vertices, a non-finite point, a non-positive total/target length |
//!
//! `SPLINE` and every other geometry kind is the explicit
//! [`CadError::Unsupported`] "DIVIDE supports LINE, ARC, CIRCLE and straight
//! LWPOLYLINE only". A spline is **not** routed through [`TargetCurve`]: its
//! parameter is not arc length, so placing marks at `k/N` of it would be a lie.
//! Work-only through [`crate::CommandId::Divide`].

use super::trim_curves::{cut_tolerance, TargetCurve};
use super::*;

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

impl Application {
    /// `DIVIDE` the one selected curve into equal arc-length parts.
    ///
    /// The marks are POINT entities on the active layer in model space; the source
    /// curve is retained. One transaction / one undo step, and every refusal
    /// happens before it is opened (no revision bump, no `ChangeSet`, no undo
    /// entry).
    pub(crate) fn divide_curve(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Divide { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "DIVIDE needs a Divide { refs } payload".into(),
            ));
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
                refs,
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

        let point_count = marks.len();
        let geometries: Vec<SemanticGeometry> =
            marks.into_iter().map(SemanticGeometry::Point).collect();
        let mut outcome = self.commit_new_entities(session, command, geometries, "divide curve")?;
        outcome.diagnostics.push(Diagnostic {
            object: Some(ObjectId(source_id.0)),
            code: "drawing.divide".into(),
            message: format!(
                "DIVIDE: source {} split into {segments} equal arc-length parts ({point_count} \
                 POINT(s) on the {} curve)",
                source_id.0,
                if closed { "closed" } else { "open" }
            ),
        });
        Ok(outcome)
    }
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

/// The POINT positions for `segments` equal arc-length divisions of `geometry`,
/// plus whether the curve is closed/periodic.
fn division_points(geometry: &SemanticGeometry, segments: usize) -> CadResult<(Vec<Point3>, bool)> {
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
                "LINE, ARC, CIRCLE or straight LWPOLYLINE",
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
                .map(|k| curve.point_at(k as f64 / segments as f64))
                .collect();
            Ok((marks, closed))
        }
        // A straight-segment LWPOLYLINE: walked by its own cumulative arc-length
        // table (never `cad-geometry`, and never the parameter model, which a
        // polyline does not have).
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => polyline_division_points(points, bulges, *closed, segments),
        // A SPLINE (and everything else) is a capability gap, not bad input. A
        // spline is deliberately **not** routed through `TargetCurve`: its
        // parameter is not arc length, so `k/N` marks would be misplaced.
        other => Err(CadError::Unsupported(format!(
            "DIVIDE supports LINE, ARC, CIRCLE and straight LWPOLYLINE only; {} is not supported",
            kind_name(other)
        ))),
    }
}

/// Equal arc-length marks on a straight-segment LWPOLYLINE.
///
/// A closed ring appends its first vertex to close the loop (unless the last
/// vertex already repeats it), so the seam segment is measured exactly once. An
/// open polyline places `N − 1` interior marks; a closed one places `N` (its
/// `k = 0` mark is the start vertex). A bulged segment is refused: its exact
/// length would need an arc-length integral this v1 does not implement, and a
/// chord approximation would be a lie.
fn polyline_division_points(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    segments: usize,
) -> CadResult<(Vec<Point3>, bool)> {
    if bulges.iter().any(|bulge| *bulge != 0.0) {
        return Err(CadError::Unsupported(
            "divide.bulged_segment: DIVIDE supports straight-segment polylines only".into(),
        ));
    }
    let eps = cut_tolerance();
    let mut vertices: Vec<Point3> = points.to_vec();
    if closed && vertices.len() >= 2 {
        let first = vertices[0];
        let last = *vertices.last().expect("checked non-empty above");
        if !same_vertex(first, last, eps) {
            vertices.push(first);
        }
    }
    if vertices.len() < 2 {
        return Err(refusal(
            "divide.degenerate",
            "DIVIDE needs a polyline with at least two vertices",
        ));
    }
    if vertices.iter().any(|point| !is_finite_point(*point)) {
        return Err(refusal(
            "divide.degenerate",
            "DIVIDE polyline vertices must be finite",
        ));
    }
    // Cumulative arc-length table: `cumulative[i]` is the distance from the first
    // vertex to vertex `i`.
    let mut cumulative = Vec::with_capacity(vertices.len());
    cumulative.push(0.0);
    for pair in vertices.windows(2) {
        let step = distance(pair[0], pair[1]);
        let next = cumulative.last().copied().unwrap_or(0.0) + step;
        cumulative.push(next);
    }
    let total = cumulative.last().copied().unwrap_or(0.0);
    if !total.is_finite() || total <= eps {
        return Err(refusal(
            "divide.degenerate",
            "DIVIDE polyline has no measurable length",
        ));
    }
    let indices = if closed { 0..segments } else { 1..segments };
    let marks = indices
        .map(|k| point_at_distance(&vertices, &cumulative, total * k as f64 / segments as f64))
        .collect();
    Ok((marks, closed))
}

/// Whether two vertices are the same point within `eps` (the seam-repeat check).
fn same_vertex(a: Point3, b: Point3, eps: f64) -> bool {
    distance(a, b) <= eps
}

/// The point at `target` arc length along the vertex chain `vertices`.
///
/// `cumulative` is the matching non-decreasing length table; `target` is clamped
/// to the chain's own span, so a floating-point overshoot at the seam returns the
/// last vertex instead of a point beyond the curve.
fn point_at_distance(vertices: &[Point3], cumulative: &[f64], target: f64) -> Point3 {
    let last = vertices.len() - 1;
    if !target.is_finite() || target <= 0.0 {
        return vertices[0];
    }
    let total = cumulative[last];
    if target >= total {
        return vertices[last];
    }
    let upper = cumulative
        .partition_point(|length| *length <= target)
        .min(last);
    let lower = upper.saturating_sub(1);
    let span = cumulative[upper] - cumulative[lower];
    if !span.is_finite() || span <= 0.0 {
        return vertices[upper];
    }
    let fraction = (target - cumulative[lower]) / span;
    let start = vertices[lower];
    let end = vertices[upper];
    Point3 {
        x: start.x + (end.x - start.x) * fraction,
        y: start.y + (end.y - start.y) * fraction,
        z: start.z + (end.z - start.z) * fraction,
    }
}
