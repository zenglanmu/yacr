//! Reverse mapping: a first-class [`HatchGeometry`] → a real DXF `HATCH`.
//!
//! The exported record is the **definition**, never the derived display: a
//! HATCH persists its boundary loops, pattern identity, pattern definition
//! rows, style and seed points, and every consumer recomputes the clipped
//! lines (ADR 0018 §0.3/C). Writing the display instead would turn a hatch into
//! loose polylines and a mesh on the next open.
//!
//! Each property this writer cannot express is declared, not approximated:
//!
//! * associativity is not modelled, so group `71` is **always** `0` (§0.4);
//! * group `47` only exists when a loop carries the `DERIVED` bit, mirroring
//!   what the writer does (ADR B3);
//! * a gradient's `color_name`/`book_name` are dropped on import, so they cannot
//!   be written back (ADR U-12).

use acadrust::entities::{
    BoundaryEdge as DxfEdge, BoundaryPath, BoundaryPathFlags, CircularArcEdge, EllipticArcEdge,
    GradientColorEntry, Hatch as DxfHatch, HatchGradientPattern, HatchPattern as DxfPattern,
    HatchPatternLine as DxfPatternLine, HatchPatternType as DxfPatternType,
    HatchStyleType as DxfStyleType, LineEdge, PolylineEdge, SplineEdge,
};
use acadrust::types::{Color, Vector2, Vector3};
use acadrust::CadDocument;
use cad_db::DbEntity;
use cad_domain::hatch::GradientStop;
use cad_domain::{
    CadResult, HatchBoundaryEdge, HatchFill, HatchGeometry, HatchGradient, HatchLoop,
    HatchPatternType, HatchStyle,
};

use super::entity::{common_for, v3, EntityContext, ExportAccumulator};
use super::report::MappingOutcome;

/// Write one hatch as a real `HATCH` and record every conversion it makes.
pub(super) fn emit_hatch(
    doc: &mut CadDocument,
    entity: &DbEntity,
    hatch: &HatchGeometry,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    let (common, extra_reasons) = common_for(doc, entity, ctx);
    let mut reasons = extra_reasons;

    let mut dxf = DxfHatch::new();
    dxf.common = common;
    // The OCS origin is constrained to `normal * elevation`; groups 10/20 are
    // always 0 and group 30 carries the elevation (ADR A.1).
    dxf.elevation = hatch.plane().elevation;
    dxf.normal = v3(hatch.plane().normal);
    // Associativity needs resolvable boundary handles, which the domain model
    // does not keep; the writer also clears `71` in that case (§0.4).
    dxf.is_associative = false;
    dxf.style = dxf_style(hatch.style());
    dxf.pattern_type = dxf_pattern_type(hatch.pattern_type());
    dxf.seed_points = hatch
        .seeds()
        .iter()
        .map(|seed| Vector2::new(seed[0], seed[1]))
        .collect();
    dxf.pixel_size = hatch.pixel_size().unwrap_or(0.0);

    match hatch.fill() {
        HatchFill::Solid => {
            dxf.is_solid = true;
            dxf.pattern.name = "SOLID".to_string();
            reasons.push("a solid fill stores no pattern name; group 2 is written as SOLID".into());
        }
        HatchFill::Pattern(pattern) => {
            dxf.is_solid = false;
            dxf.pattern = dxf_pattern(pattern);
            dxf.pattern_angle = pattern.angle;
            dxf.pattern_scale = pattern.scale;
            dxf.is_double = pattern.double;
            reasons.push(format!(
                "pattern '{}' is written as {} definition row(s); the clipped lines are \
                 recomputed from it on open",
                pattern.name,
                pattern.lines.len()
            ));
        }
        HatchFill::Gradient(gradient) => {
            // A gradient HATCH is a solid fill plus the 450-470 block.
            dxf.is_solid = true;
            dxf.pattern.name = "SOLID".to_string();
            dxf.gradient_color = dxf_gradient(gradient);
            reasons.push(format!(
                "gradient '{}' is written with {} stop(s); colour names \
                 (color_name/book_name) cannot be written (groups 63/421 only)",
                gradient.name(),
                gradient_stops(gradient).len()
            ));
            // Group 460 is passed through unchanged by both the reader and the
            // writer while its unit is unverified (ADR U-13).
            reasons.push(
                "gradient angle (group 460) is written as the opaque source value; its unit is \
                 unverified"
                    .into(),
            );
        }
    }

    for (index, loop_) in hatch.loops().iter().enumerate() {
        let (path, loop_reasons) = boundary_path(index, loop_);
        reasons.extend(loop_reasons);
        dxf.paths.push(path);
    }

    reasons.push(format!(
        "hatch style {:?} is written as DXF group 75",
        hatch.style()
    ));
    reasons.push(format!(
        "pattern type {:?} is written as DXF group 76",
        hatch.pattern_type()
    ));
    reasons.push(format!(
        "{} seed point(s) are written as group 98",
        dxf.seed_points.len()
    ));
    reasons.push(
        "the fill is recomputed from the definition; no clipped pattern segments are written"
            .into(),
    );
    reasons.push("associativity is not preserved; group 71 is written as 0".into());
    reasons.push(pixel_size_reason(hatch));

    acc.push(acadrust::EntityType::Hatch(Box::new(dxf)), ctx)?;
    acc.record(object, "hatch", MappingOutcome::Converted(reasons));
    Ok(())
}

/// The group-47 declaration: presence is coupled to a `DERIVED` loop (ADR B3).
fn pixel_size_reason(hatch: &HatchGeometry) -> String {
    let derived = hatch.loops().iter().any(|loop_| loop_.flags.derived());
    match (hatch.pixel_size(), derived) {
        (Some(size), true) => {
            format!("pixel size {size} is written as group 47 (a boundary path is derived)")
        }
        (None, true) => {
            "a boundary path is derived but the source stored no pixel size; group 47 is \
             written as 0.0"
                .to_string()
        }
        (Some(_), false) => {
            "the stored pixel size is not written; group 47 exists only for a derived boundary \
             path"
                .to_string()
        }
        (None, false) => "no derived boundary path; group 47 is not written".to_string(),
    }
}

/// Convert one boundary loop into a DXF path.
///
/// The analytic source edges are preferred (ADR B4) so an imported arc stays an
/// arc; a loop without them degrades to the sampled polyline path, which is
/// declared. The DXF reader dispatches on the `POLYLINE` bit of group 92, so the
/// polyline form must set it.
fn boundary_path(index: usize, loop_: &HatchLoop) -> (BoundaryPath, Vec<String>) {
    let mut reasons = Vec::new();
    let raw = loop_.flags.bits();
    let polyline_bit = cad_domain::HatchLoopFlags::POLYLINE;
    let source_edges = loop_
        .source_edges
        .as_ref()
        .filter(|edges| !edges.is_empty());
    let mut flags = raw;
    let mut path = BoundaryPath::with_flags(BoundaryPathFlags::from_bits(raw));

    if raw & polyline_bit == 0 {
        if let Some(edges) = source_edges {
            for edge in edges {
                path.edges.push(dxf_edge(edge));
            }
            return (path, reasons);
        }
        // No analytic edges: fall back to the sampled polyline path (ADR B4).
        flags |= polyline_bit;
        path.flags = BoundaryPathFlags::from_bits(flags);
        reasons.push(format!(
            "boundary loop {index} has no analytic source edges; written as a sampled polyline \
             path"
        ));
    } else if source_edges.is_some() {
        // Defensive: the raw bits and the stored edges normally agree, but a
        // polyline flag wins because the reader would skip the edge payload.
        reasons.push(format!(
            "boundary loop {index} carries both the polyline flag and analytic edges; the \
             polyline flag wins and the analytic edges are not written"
        ));
    }

    // A hatch loop is closed by definition, so the polyline path always says so
    // even when the preserved raw bits still carry `NOT_CLOSED`: the bits are
    // round-tripped verbatim, never reinterpreted (ADR U-5).
    let mut polyline = PolylineEdge {
        vertices: Vec::with_capacity(loop_.points.len()),
        is_closed: true,
    };
    for (position, point) in loop_.points.iter().enumerate() {
        let bulge = loop_.bulges.get(position).copied().unwrap_or(0.0);
        polyline
            .vertices
            .push(Vector3::new(point[0], point[1], bulge));
    }
    path.edges.push(DxfEdge::Polyline(polyline));
    (path, reasons)
}

/// Map one domain boundary edge onto acadrust's DXF edge record.
fn dxf_edge(edge: &HatchBoundaryEdge) -> DxfEdge {
    match edge {
        HatchBoundaryEdge::Line { start, end } => DxfEdge::Line(LineEdge {
            start: Vector2::new(start[0], start[1]),
            end: Vector2::new(end[0], end[1]),
        }),
        // The writer converts the circular-arc angles to degrees and the reader
        // converts them back, so the domain keeps radians (ADR A.1).
        HatchBoundaryEdge::CircularArc {
            center,
            radius,
            start_angle,
            end_angle,
            ccw,
        } => DxfEdge::CircularArc(CircularArcEdge {
            center: Vector2::new(center[0], center[1]),
            radius: *radius,
            start_angle: *start_angle,
            end_angle: *end_angle,
            counter_clockwise: *ccw,
        }),
        // acadrust passes the elliptic-arc angles through in both directions;
        // the domain stores exactly what the source carried.
        HatchBoundaryEdge::EllipticArc {
            center,
            major_axis,
            ratio,
            start_angle,
            end_angle,
            ccw,
        } => DxfEdge::EllipticArc(EllipticArcEdge {
            center: Vector2::new(center[0], center[1]),
            major_axis_endpoint: Vector2::new(major_axis[0], major_axis[1]),
            minor_axis_ratio: *ratio,
            start_angle: *start_angle,
            end_angle: *end_angle,
            counter_clockwise: *ccw,
        }),
        HatchBoundaryEdge::Spline {
            degree,
            rational,
            periodic,
            knots,
            control_points,
            fit_points,
            start_tangent,
            end_tangent,
        } => DxfEdge::Spline(SplineEdge {
            degree: *degree as i32,
            rational: *rational,
            periodic: *periodic,
            knots: knots.clone(),
            // The weight of a rational spline rides in `z`, exactly as stored.
            control_points: control_points
                .iter()
                .map(|point| Vector3::new(point[0], point[1], point[2]))
                .collect(),
            fit_points: fit_points
                .iter()
                .map(|point| Vector2::new(point[0], point[1]))
                .collect(),
            start_tangent: Vector2::new(start_tangent[0], start_tangent[1]),
            end_tangent: Vector2::new(end_tangent[0], end_tangent[1]),
        }),
    }
}

fn dxf_pattern(pattern: &cad_domain::HatchPattern) -> DxfPattern {
    DxfPattern {
        name: pattern.name.clone(),
        description: String::new(),
        // Definition rows are pattern-space data and round-trip unchanged; the
        // hatch-level angle/scale are applied by the consumer, not baked here.
        lines: pattern
            .lines
            .iter()
            .map(|line| DxfPatternLine {
                angle: line.angle,
                base_point: Vector2::new(line.base_x, line.base_y),
                offset: Vector2::new(line.offset_x, line.offset_y),
                dash_lengths: line.dashes.clone(),
            })
            .collect(),
    }
}

fn dxf_gradient(gradient: &HatchGradient) -> HatchGradientPattern {
    let mut dxf = HatchGradientPattern::new();
    dxf.enabled = true;
    match gradient {
        HatchGradient::TwoColor {
            name,
            reserved,
            angle_raw,
            shift,
            stops,
        } => {
            dxf.name = name.clone();
            dxf.reserved = *reserved;
            dxf.angle = *angle_raw;
            dxf.shift = *shift;
            dxf.is_single_color = false;
            dxf.color_tint = 0.0;
            dxf.colors = gradient_colors(stops);
        }
        HatchGradient::SingleColor {
            name,
            reserved,
            angle_raw,
            shift,
            tint,
            stops,
        } => {
            dxf.name = name.clone();
            dxf.reserved = *reserved;
            dxf.angle = *angle_raw;
            dxf.shift = *shift;
            dxf.is_single_color = true;
            dxf.color_tint = *tint;
            dxf.colors = gradient_colors(stops);
        }
    }
    dxf
}

fn gradient_stops(gradient: &HatchGradient) -> &[GradientStop] {
    match gradient {
        HatchGradient::TwoColor { stops, .. } | HatchGradient::SingleColor { stops, .. } => stops,
    }
}

fn gradient_colors(stops: &[GradientStop]) -> Vec<GradientColorEntry> {
    stops
        .iter()
        .map(|stop| GradientColorEntry {
            value: stop.value,
            color: Color::from_rgb(stop.rgb[0], stop.rgb[1], stop.rgb[2]),
            // The importer drops these, so there is nothing to write back.
            color_name: None,
            book_name: None,
        })
        .collect()
}

fn dxf_style(style: HatchStyle) -> DxfStyleType {
    match style {
        HatchStyle::Normal => DxfStyleType::Normal,
        HatchStyle::Outer => DxfStyleType::Outer,
        HatchStyle::Ignore => DxfStyleType::Ignore,
    }
}

fn dxf_pattern_type(pattern_type: HatchPatternType) -> DxfPatternType {
    match pattern_type {
        HatchPatternType::UserDefined => DxfPatternType::UserDefined,
        HatchPatternType::Predefined => DxfPatternType::Predefined,
        HatchPatternType::Custom => DxfPatternType::Custom,
    }
}
