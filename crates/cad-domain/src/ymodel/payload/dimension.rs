//! The `cad:Dimension` payload (ADR-0037 D8).
//!
//! A dimension stores its definition (subtype, points, text fields, normal), the
//! resolved `DIMSTYLE` snapshot, the raw measurement, the user text override,
//! the glyph alignment, the display provenance **and the derived display
//! geometry**. Phase 1 persists `measurement` and `display` verbatim: the
//! synthesis is the only builder that can produce them, and re-running it on
//! load would replace stored display data on the first save (ADR-0037 D8).
//!
//! Restoring therefore goes through
//! [`DimensionGeometry::from_persisted`](crate::DimensionGeometry), the
//! crate-visible constructor that re-synthesizes nothing and validates nothing.
//! The fields themselves stay private; only that constructor is reachable.

use serde_json::Value;

use super::{fields, id_ref_value, object, PayloadError};
use crate::dimension::DisplaySource;
use crate::ymodel::id::IdKind;
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints, DimensionStyleValues,
    SemanticGeometry, StyleId,
};

// ---------------------------------------------------------------------------
// The entity.
// ---------------------------------------------------------------------------

pub(super) fn encode_dimension(dimension: &DimensionGeometry) -> Value {
    object(
        "cad:Dimension",
        vec![
            ("definition", encode_definition(dimension.definition())),
            (
                "style_name",
                Value::String(dimension.style_name().to_string()),
            ),
            ("style", encode_style(dimension.style())),
            ("measurement", json::encode_f64(dimension.measurement())),
            (
                "text_override",
                dimension
                    .text_override()
                    .map_or(Value::Null, |text| Value::String(text.to_string())),
            ),
            (
                "display",
                Value::Array(dimension.display().iter().map(super::encode).collect()),
            ),
            (
                "text_h_align",
                super::basic::encode_align_h(dimension.text_h_align()),
            ),
            ("source", encode_display_source(dimension.display_source())),
        ],
    )
}

pub(super) fn decode_dimension(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Dimension")?;
    let definition = decode_definition(&reader.take("definition")?)?;
    let style_name = reader.string("style_name")?;
    let style = decode_style(&reader.take("style")?)?;
    let measurement = reader.f64("measurement")?;
    let text_override = reader.opt_string("text_override")?;
    let display = reader.vec_geometry("display")?;
    let text_h_align = super::basic::decode_align_h(&reader.take("text_h_align")?)?;
    let source = decode_display_source(&reader.take("source")?)?;
    reader.finish()?;
    Ok(SemanticGeometry::Dimension(Box::new(
        DimensionGeometry::from_persisted(
            definition,
            style_name,
            style,
            measurement,
            text_override,
            display,
            text_h_align,
            source,
        ),
    )))
}

fn encode_display_source(source: DisplaySource) -> Value {
    Value::String(
        match source {
            DisplaySource::Synthesized => "synthesized",
            DisplaySource::Picture => "picture",
        }
        .to_string(),
    )
}

fn decode_display_source(value: &Value) -> Result<DisplaySource, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "synthesized" => Ok(DisplaySource::Synthesized),
            "picture" => Ok(DisplaySource::Picture),
            other => Err(PayloadError::new(format!(
                "cad:Dimension.source: unknown display source {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "cad:Dimension.source: expected a string token",
        )),
    }
}

// ---------------------------------------------------------------------------
// Definition.
// ---------------------------------------------------------------------------

fn encode_definition(definition: &DimensionDefinition) -> Value {
    fields(vec![
        ("kind", encode_kind(definition.kind)),
        ("points", encode_points(&definition.points)),
        ("text_middle", json::encode_point3(definition.text_middle)),
        ("text_rotation", json::encode_f64(definition.text_rotation)),
        ("normal", json::encode_point3(definition.normal)),
    ])
}

fn decode_definition(value: &Value) -> Result<DimensionDefinition, PayloadError> {
    let mut reader = ObjectReader::new(value, "definition")?;
    let kind = decode_kind(&reader.take("kind")?)?;
    let points = decode_points(&reader.take("points")?)?;
    let text_middle = reader.point3("text_middle")?;
    let text_rotation = reader.f64("text_rotation")?;
    let normal = reader.point3("normal")?;
    reader.finish()?;
    Ok(DimensionDefinition {
        kind,
        points,
        text_middle,
        text_rotation,
        normal,
    })
}

fn encode_kind(kind: DimensionKind) -> Value {
    match kind {
        DimensionKind::Linear { rotation } => fields(vec![
            ("kind", Value::String("linear".to_string())),
            ("rotation", json::encode_f64(rotation)),
        ]),
        DimensionKind::Aligned => fields(vec![("kind", Value::String("aligned".to_string()))]),
        DimensionKind::Angular2Ln => {
            fields(vec![("kind", Value::String("angular_2ln".to_string()))])
        }
        DimensionKind::Angular3Pt => {
            fields(vec![("kind", Value::String("angular_3pt".to_string()))])
        }
        DimensionKind::Radius => fields(vec![("kind", Value::String("radius".to_string()))]),
        DimensionKind::Diameter => fields(vec![("kind", Value::String("diameter".to_string()))]),
        DimensionKind::Ordinate { x_ordinate } => fields(vec![
            ("kind", Value::String("ordinate".to_string())),
            ("x_ordinate", Value::Bool(x_ordinate)),
        ]),
    }
}

fn decode_kind(value: &Value) -> Result<DimensionKind, PayloadError> {
    let mut reader = ObjectReader::new(value, "kind")?;
    let kind = match reader.string("kind")?.as_str() {
        "linear" => DimensionKind::Linear {
            rotation: reader.f64("rotation")?,
        },
        "aligned" => DimensionKind::Aligned,
        "angular_2ln" => DimensionKind::Angular2Ln,
        "angular_3pt" => DimensionKind::Angular3Pt,
        "radius" => DimensionKind::Radius,
        "diameter" => DimensionKind::Diameter,
        "ordinate" => DimensionKind::Ordinate {
            x_ordinate: reader.bool("x_ordinate")?,
        },
        other => {
            return Err(PayloadError::new(format!(
                "definition.kind: unknown dimension subtype {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(kind)
}

fn encode_points(points: &DimensionPoints) -> Value {
    match points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => fields(vec![
            ("kind", Value::String("linear".to_string())),
            ("first", json::encode_point3(*first)),
            ("second", json::encode_point3(*second)),
            ("dimension_line", json::encode_point3(*dimension_line)),
        ]),
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => fields(vec![
            ("kind", Value::String("radial".to_string())),
            ("center", json::encode_point3(*center)),
            ("point_on_arc", json::encode_point3(*point_on_arc)),
            ("leader_length", json::encode_f64(*leader_length)),
        ]),
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => fields(vec![
            ("kind", Value::String("angular_2ln".to_string())),
            ("line1_start", json::encode_point3(*line1_start)),
            ("line1_end", json::encode_point3(*line1_end)),
            ("line2_start", json::encode_point3(*line2_start)),
            ("line2_end", json::encode_point3(*line2_end)),
            ("arc_point", json::encode_point3(*arc_point)),
        ]),
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => fields(vec![
            ("kind", Value::String("angular_3pt".to_string())),
            ("vertex", json::encode_point3(*vertex)),
            ("first", json::encode_point3(*first)),
            ("second", json::encode_point3(*second)),
            ("arc_point", json::encode_point3(*arc_point)),
        ]),
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => fields(vec![
            ("kind", Value::String("ordinate".to_string())),
            ("feature", json::encode_point3(*feature)),
            ("leader_end", json::encode_point3(*leader_end)),
            ("base", json::encode_point3(*base)),
        ]),
    }
}

fn decode_points(value: &Value) -> Result<DimensionPoints, PayloadError> {
    let mut reader = ObjectReader::new(value, "points")?;
    let points = match reader.string("kind")?.as_str() {
        "linear" => DimensionPoints::Linear {
            first: reader.point3("first")?,
            second: reader.point3("second")?,
            dimension_line: reader.point3("dimension_line")?,
        },
        "radial" => DimensionPoints::Radial {
            center: reader.point3("center")?,
            point_on_arc: reader.point3("point_on_arc")?,
            leader_length: reader.f64("leader_length")?,
        },
        "angular_2ln" => DimensionPoints::Angular2Ln {
            line1_start: reader.point3("line1_start")?,
            line1_end: reader.point3("line1_end")?,
            line2_start: reader.point3("line2_start")?,
            line2_end: reader.point3("line2_end")?,
            arc_point: reader.point3("arc_point")?,
        },
        "angular_3pt" => DimensionPoints::Angular3Pt {
            vertex: reader.point3("vertex")?,
            first: reader.point3("first")?,
            second: reader.point3("second")?,
            arc_point: reader.point3("arc_point")?,
        },
        "ordinate" => DimensionPoints::Ordinate {
            feature: reader.point3("feature")?,
            leader_end: reader.point3("leader_end")?,
            base: reader.point3("base")?,
        },
        other => {
            return Err(PayloadError::new(format!(
                "definition.points.kind: unknown point set {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(points)
}

// ---------------------------------------------------------------------------
// Style snapshot.
// ---------------------------------------------------------------------------

fn encode_style(style: &DimensionStyleValues) -> Value {
    fields(vec![
        ("arrow_size", json::encode_f64(style.arrow_size)),
        ("ext_offset", json::encode_f64(style.ext_offset)),
        ("ext_beyond", json::encode_f64(style.ext_beyond)),
        ("text_height", json::encode_f64(style.text_height)),
        ("text_gap", json::encode_f64(style.text_gap)),
        ("center_mark", json::encode_f64(style.center_mark)),
        (
            "dim_line_increment",
            json::encode_f64(style.dim_line_increment),
        ),
        ("text_vertical", Value::from(style.text_vertical)),
        (
            "text_vertical_position",
            json::encode_f64(style.text_vertical_position),
        ),
        ("suppress_ext1", Value::Bool(style.suppress_ext1)),
        ("suppress_ext2", Value::Bool(style.suppress_ext2)),
        ("suppress_dim1", Value::Bool(style.suppress_dim1)),
        ("suppress_dim2", Value::Bool(style.suppress_dim2)),
        ("linear_factor", json::encode_f64(style.linear_factor)),
        ("decimals", Value::from(style.decimals)),
        ("rounding", json::encode_f64(style.rounding)),
        ("postfix", Value::String(style.postfix.clone())),
        ("zero_suppression", Value::from(style.zero_suppression)),
        ("linear_unit_format", Value::from(style.linear_unit_format)),
        ("decimal_separator", Value::from(style.decimal_separator)),
        (
            "text_style_name",
            Value::String(style.text_style_name.clone()),
        ),
        (
            "text_style",
            id_ref_value(IdKind::Style, style.text_style.0),
        ),
        (
            "text_font",
            style
                .text_font
                .as_ref()
                .map_or(Value::Null, |font| Value::String(font.clone())),
        ),
    ])
}

fn decode_style(value: &Value) -> Result<DimensionStyleValues, PayloadError> {
    let mut reader = ObjectReader::new(value, "style")?;
    let style = DimensionStyleValues {
        arrow_size: reader.f64("arrow_size")?,
        ext_offset: reader.f64("ext_offset")?,
        ext_beyond: reader.f64("ext_beyond")?,
        text_height: reader.f64("text_height")?,
        text_gap: reader.f64("text_gap")?,
        center_mark: reader.f64("center_mark")?,
        dim_line_increment: reader.f64("dim_line_increment")?,
        text_vertical: reader.i16("text_vertical")?,
        text_vertical_position: reader.f64("text_vertical_position")?,
        suppress_ext1: reader.bool("suppress_ext1")?,
        suppress_ext2: reader.bool("suppress_ext2")?,
        suppress_dim1: reader.bool("suppress_dim1")?,
        suppress_dim2: reader.bool("suppress_dim2")?,
        linear_factor: reader.f64("linear_factor")?,
        decimals: reader.usize("decimals")?,
        rounding: reader.f64("rounding")?,
        postfix: reader.string("postfix")?,
        zero_suppression: reader.i16("zero_suppression")?,
        linear_unit_format: reader.i16("linear_unit_format")?,
        decimal_separator: reader.i16("decimal_separator")?,
        text_style_name: reader.string("text_style_name")?,
        text_style: StyleId(reader.id_seq("text_style", IdKind::Style)?),
        text_font: reader.opt_string("text_font")?,
    };
    reader.finish()?;
    Ok(style)
}
