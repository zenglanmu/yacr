//! The `cad:Hatch` payload (ADR-0037 D8).
//!
//! A hatch stores its boundary loops (hatch-plane 2D points, bulges, raw flag
//! word and the optional analytic source edges), the plane, the style/fill
//! definition, seeds, pixel size, pattern type and phase displacement **and the
//! derived display geometry**.
//!
//! Phase 1 persists `display` verbatim: [`HatchGeometry::new`] validates a
//! boundary this build can represent today and rebuilds the display, and
//! `from_import` rebuilds it too. Restoring through either would replace stored
//! display data on the first save (ADR-0037 D8) and could reject a legacy row,
//! so the reader calls `HatchGeometry::from_persisted`, which takes every field
//! as stored.
//!
//! [`HatchGeometry::new`]: crate::HatchGeometry::new

use serde_json::Value;

use super::{fields, object, PayloadError};
use crate::hatch::GradientStop;
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    BoundaryEdge, HatchFill, HatchGeometry, HatchGradient, HatchLoop, HatchLoopFlags, HatchPattern,
    HatchPatternLine, HatchPatternType, HatchPlane, HatchStyle, Point3, SemanticGeometry,
};

// ---------------------------------------------------------------------------
// The entity.
// ---------------------------------------------------------------------------

pub(super) fn encode_hatch(hatch: &HatchGeometry) -> Value {
    object(
        "cad:Hatch",
        vec![
            (
                "loops",
                Value::Array(hatch.loops().iter().map(encode_loop).collect()),
            ),
            ("plane", encode_plane(hatch.plane())),
            ("style", encode_style(hatch.style())),
            ("fill", encode_fill(hatch.fill())),
            (
                "seeds",
                Value::Array(
                    hatch
                        .seeds()
                        .iter()
                        .map(|seed| json::encode_f64x2(*seed))
                        .collect(),
                ),
            ),
            ("pixel_size", json::encode_opt_f64(hatch.pixel_size())),
            ("pattern_type", encode_pattern_type(hatch.pattern_type())),
            ("pattern_origin", json::encode_f64x2(hatch.pattern_origin())),
            (
                "display",
                Value::Array(hatch.display().iter().map(super::encode).collect()),
            ),
        ],
    )
}

pub(super) fn decode_hatch(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Hatch")?;
    let raw_loops = reader.take("loops")?;
    let loops = match &raw_loops {
        Value::Array(items) => items.iter().map(decode_loop).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("cad:Hatch.loops: expected an array"));
        }
    };
    let plane = decode_plane(&reader.take("plane")?)?;
    let style = decode_style(&reader.take("style")?)?;
    let fill = decode_fill(&reader.take("fill")?)?;
    let seeds = reader.vec_f64x2("seeds")?;
    let pixel_size = reader.opt_f64("pixel_size")?;
    let pattern_type = decode_pattern_type(&reader.take("pattern_type")?)?;
    let pattern_origin = reader.f64x2("pattern_origin")?;
    let display = reader.vec_geometry("display")?;
    reader.finish()?;
    Ok(SemanticGeometry::Hatch(Box::new(
        HatchGeometry::from_persisted(
            loops,
            plane,
            style,
            fill,
            seeds,
            pixel_size,
            pattern_type,
            pattern_origin,
            display,
        ),
    )))
}

// ---------------------------------------------------------------------------
// Loops and their source edges.
// ---------------------------------------------------------------------------

fn encode_loop(loop_: &HatchLoop) -> Value {
    fields(vec![
        (
            "points",
            Value::Array(
                loop_
                    .points
                    .iter()
                    .map(|point| json::encode_f64x2(*point))
                    .collect(),
            ),
        ),
        ("bulges", super::basic::encode_vec_f64(&loop_.bulges)),
        ("flags", Value::from(loop_.flags.bits())),
        (
            "source_edges",
            loop_.source_edges.as_ref().map_or(Value::Null, |edges| {
                Value::Array(edges.iter().map(encode_edge).collect())
            }),
        ),
    ])
}

fn decode_loop(value: &Value) -> Result<HatchLoop, PayloadError> {
    let mut reader = ObjectReader::new(value, "loop")?;
    let points = reader.vec_f64x2("points")?;
    let bulges = reader.vec_f64("bulges")?;
    let flags = HatchLoopFlags(reader.u32("flags")?);
    let source_edges = match reader.take("source_edges")? {
        Value::Null => None,
        value => {
            let Value::Array(items) = &value else {
                return Err(PayloadError::new(
                    "loop.source_edges: expected null or an array",
                ));
            };
            Some(items.iter().map(decode_edge).collect::<Result<_, _>>()?)
        }
    };
    reader.finish()?;
    Ok(HatchLoop {
        points,
        bulges,
        flags,
        source_edges,
    })
}

fn encode_edge(edge: &BoundaryEdge) -> Value {
    match edge {
        BoundaryEdge::Line { start, end } => fields(vec![
            ("kind", Value::String("line".to_string())),
            ("start", json::encode_f64x2(*start)),
            ("end", json::encode_f64x2(*end)),
        ]),
        BoundaryEdge::CircularArc {
            center,
            radius,
            start_angle,
            end_angle,
            ccw,
        } => fields(vec![
            ("kind", Value::String("circular_arc".to_string())),
            ("center", json::encode_f64x2(*center)),
            ("radius", json::encode_f64(*radius)),
            ("start_angle", json::encode_f64(*start_angle)),
            ("end_angle", json::encode_f64(*end_angle)),
            ("ccw", Value::Bool(*ccw)),
        ]),
        BoundaryEdge::EllipticArc {
            center,
            major_axis,
            ratio,
            start_angle,
            end_angle,
            ccw,
        } => fields(vec![
            ("kind", Value::String("elliptic_arc".to_string())),
            ("center", json::encode_f64x2(*center)),
            ("major_axis", json::encode_f64x2(*major_axis)),
            ("ratio", json::encode_f64(*ratio)),
            ("start_angle", json::encode_f64(*start_angle)),
            ("end_angle", json::encode_f64(*end_angle)),
            ("ccw", Value::Bool(*ccw)),
        ]),
        BoundaryEdge::Spline {
            degree,
            rational,
            periodic,
            knots,
            control_points,
            fit_points,
            start_tangent,
            end_tangent,
        } => fields(vec![
            ("kind", Value::String("spline".to_string())),
            ("degree", Value::from(*degree)),
            ("rational", Value::Bool(*rational)),
            ("periodic", Value::Bool(*periodic)),
            ("knots", super::basic::encode_vec_f64(knots)),
            (
                "control_points",
                Value::Array(
                    control_points
                        .iter()
                        .map(|point| {
                            Value::Array(
                                point.iter().map(|value| json::encode_f64(*value)).collect(),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "fit_points",
                Value::Array(
                    fit_points
                        .iter()
                        .map(|point| json::encode_f64x2(*point))
                        .collect(),
                ),
            ),
            ("start_tangent", json::encode_f64x2(*start_tangent)),
            ("end_tangent", json::encode_f64x2(*end_tangent)),
        ]),
    }
}

fn decode_edge(value: &Value) -> Result<BoundaryEdge, PayloadError> {
    let mut reader = ObjectReader::new(value, "edge")?;
    let edge = match reader.string("kind")?.as_str() {
        "line" => BoundaryEdge::Line {
            start: reader.f64x2("start")?,
            end: reader.f64x2("end")?,
        },
        "circular_arc" => BoundaryEdge::CircularArc {
            center: reader.f64x2("center")?,
            radius: reader.f64("radius")?,
            start_angle: reader.f64("start_angle")?,
            end_angle: reader.f64("end_angle")?,
            ccw: reader.bool("ccw")?,
        },
        "elliptic_arc" => BoundaryEdge::EllipticArc {
            center: reader.f64x2("center")?,
            major_axis: reader.f64x2("major_axis")?,
            ratio: reader.f64("ratio")?,
            start_angle: reader.f64("start_angle")?,
            end_angle: reader.f64("end_angle")?,
            ccw: reader.bool("ccw")?,
        },
        "spline" => BoundaryEdge::Spline {
            degree: reader.u32("degree")?,
            rational: reader.bool("rational")?,
            periodic: reader.bool("periodic")?,
            knots: reader.vec_f64("knots")?,
            control_points: reader.vec_f64x3("control_points")?,
            fit_points: reader.vec_f64x2("fit_points")?,
            start_tangent: reader.f64x2("start_tangent")?,
            end_tangent: reader.f64x2("end_tangent")?,
        },
        other => {
            return Err(PayloadError::new(format!(
                "edge.kind: unknown boundary edge {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(edge)
}

// ---------------------------------------------------------------------------
// Plane, style and pattern type.
// ---------------------------------------------------------------------------

fn encode_plane(plane: HatchPlane) -> Value {
    fields(vec![
        ("normal", json::encode_point3(plane.normal)),
        ("elevation", json::encode_f64(plane.elevation)),
    ])
}

fn decode_plane(value: &Value) -> Result<HatchPlane, PayloadError> {
    let mut reader = ObjectReader::new(value, "plane")?;
    let normal: Point3 = reader.point3("normal")?;
    let elevation = reader.f64("elevation")?;
    reader.finish()?;
    Ok(HatchPlane { normal, elevation })
}

fn encode_style(style: HatchStyle) -> Value {
    Value::String(
        match style {
            HatchStyle::Normal => "normal",
            HatchStyle::Outer => "outer",
            HatchStyle::Ignore => "ignore",
        }
        .to_string(),
    )
}

fn decode_style(value: &Value) -> Result<HatchStyle, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "normal" => Ok(HatchStyle::Normal),
            "outer" => Ok(HatchStyle::Outer),
            "ignore" => Ok(HatchStyle::Ignore),
            other => Err(PayloadError::new(format!(
                "cad:Hatch.style: unknown hatch style {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "cad:Hatch.style: expected a string token",
        )),
    }
}

fn encode_pattern_type(pattern_type: HatchPatternType) -> Value {
    Value::String(
        match pattern_type {
            HatchPatternType::UserDefined => "user_defined",
            HatchPatternType::Predefined => "predefined",
            HatchPatternType::Custom => "custom",
        }
        .to_string(),
    )
}

fn decode_pattern_type(value: &Value) -> Result<HatchPatternType, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "user_defined" => Ok(HatchPatternType::UserDefined),
            "predefined" => Ok(HatchPatternType::Predefined),
            "custom" => Ok(HatchPatternType::Custom),
            other => Err(PayloadError::new(format!(
                "cad:Hatch.pattern_type: unknown pattern type {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "cad:Hatch.pattern_type: expected a string token",
        )),
    }
}

// ---------------------------------------------------------------------------
// Fill.
// ---------------------------------------------------------------------------

fn encode_fill(fill: &HatchFill) -> Value {
    match fill {
        HatchFill::Solid => fields(vec![("kind", Value::String("solid".to_string()))]),
        HatchFill::Pattern(pattern) => fields(vec![
            ("kind", Value::String("pattern".to_string())),
            ("name", Value::String(pattern.name.clone())),
            ("angle", json::encode_f64(pattern.angle)),
            ("scale", json::encode_f64(pattern.scale)),
            ("double", Value::Bool(pattern.double)),
            (
                "lines",
                Value::Array(pattern.lines.iter().map(encode_pattern_line).collect()),
            ),
        ]),
        HatchFill::Gradient(gradient) => encode_gradient(gradient),
    }
}

fn decode_fill(value: &Value) -> Result<HatchFill, PayloadError> {
    let mut reader = ObjectReader::new(value, "fill")?;
    let fill = match reader.string("kind")?.as_str() {
        "solid" => HatchFill::Solid,
        "pattern" => HatchFill::Pattern(HatchPattern {
            name: reader.string("name")?,
            angle: reader.f64("angle")?,
            scale: reader.f64("scale")?,
            double: reader.bool("double")?,
            lines: {
                let raw = reader.take("lines")?;
                let Value::Array(items) = &raw else {
                    return Err(PayloadError::new("fill.lines: expected an array"));
                };
                items
                    .iter()
                    .map(decode_pattern_line)
                    .collect::<Result<_, _>>()?
            },
        }),
        "gradient_two_color" => HatchFill::Gradient(HatchGradient::TwoColor {
            name: reader.string("name")?,
            reserved: reader.i32("reserved")?,
            angle_raw: reader.f64("angle_raw")?,
            shift: reader.f64("shift")?,
            stops: decode_stops(&reader.take("stops")?)?,
        }),
        "gradient_single_color" => HatchFill::Gradient(HatchGradient::SingleColor {
            name: reader.string("name")?,
            reserved: reader.i32("reserved")?,
            angle_raw: reader.f64("angle_raw")?,
            shift: reader.f64("shift")?,
            tint: reader.f64("tint")?,
            stops: decode_stops(&reader.take("stops")?)?,
        }),
        other => {
            return Err(PayloadError::new(format!(
                "cad:Hatch.fill.kind: unknown fill {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(fill)
}

fn encode_pattern_line(line: &HatchPatternLine) -> Value {
    fields(vec![
        ("angle", json::encode_f64(line.angle)),
        ("base_x", json::encode_f64(line.base_x)),
        ("base_y", json::encode_f64(line.base_y)),
        ("offset_x", json::encode_f64(line.offset_x)),
        ("offset_y", json::encode_f64(line.offset_y)),
        ("dashes", super::basic::encode_vec_f64(&line.dashes)),
    ])
}

fn decode_pattern_line(value: &Value) -> Result<HatchPatternLine, PayloadError> {
    let mut reader = ObjectReader::new(value, "pattern_line")?;
    let line = HatchPatternLine {
        angle: reader.f64("angle")?,
        base_x: reader.f64("base_x")?,
        base_y: reader.f64("base_y")?,
        offset_x: reader.f64("offset_x")?,
        offset_y: reader.f64("offset_y")?,
        dashes: reader.vec_f64("dashes")?,
    };
    reader.finish()?;
    Ok(line)
}

fn encode_gradient(gradient: &HatchGradient) -> Value {
    match gradient {
        HatchGradient::TwoColor {
            name,
            reserved,
            angle_raw,
            shift,
            stops,
        } => fields(vec![
            ("kind", Value::String("gradient_two_color".to_string())),
            ("name", Value::String(name.clone())),
            ("reserved", Value::from(*reserved)),
            ("angle_raw", json::encode_f64(*angle_raw)),
            ("shift", json::encode_f64(*shift)),
            ("stops", encode_stops(stops)),
        ]),
        HatchGradient::SingleColor {
            name,
            reserved,
            angle_raw,
            shift,
            tint,
            stops,
        } => fields(vec![
            ("kind", Value::String("gradient_single_color".to_string())),
            ("name", Value::String(name.clone())),
            ("reserved", Value::from(*reserved)),
            ("angle_raw", json::encode_f64(*angle_raw)),
            ("shift", json::encode_f64(*shift)),
            ("tint", json::encode_f64(*tint)),
            ("stops", encode_stops(stops)),
        ]),
    }
}

fn encode_stops(stops: &[GradientStop]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|stop| {
                fields(vec![
                    ("value", json::encode_f64(stop.value)),
                    (
                        "rgb",
                        Value::Array(stop.rgb.iter().map(|byte| Value::from(*byte)).collect()),
                    ),
                ])
            })
            .collect(),
    )
}

fn decode_stops(value: &Value) -> Result<Vec<GradientStop>, PayloadError> {
    let Value::Array(items) = value else {
        return Err(PayloadError::new("fill.stops: expected an array"));
    };
    items
        .iter()
        .map(|item| {
            let mut reader = ObjectReader::new(item, "stop")?;
            let stop = GradientStop {
                value: reader.f64("value")?,
                rgb: reader.u8x3("rgb")?,
            };
            reader.finish()?;
            Ok(stop)
        })
        .collect()
}
