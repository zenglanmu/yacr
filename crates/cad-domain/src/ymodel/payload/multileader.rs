//! The `cad:MultiLeader` payload (ADR-0037 D8).
//!
//! A multileader stores its resolved style snapshot, style name, raw override
//! mask, annotation content (none / block / text / tolerance), the text record,
//! the leader roots and lines, the verbatim `reserved` bag, the plane frame
//! **and the derived display geometry**.
//!
//! Phase 1 persists `display` verbatim: every other write path rebuilds it, and
//! `new` additionally rejects definitions it cannot draw. Restoring through
//! either would replace stored display data on the first save (ADR-0037 D8) and
//! could reject a legacy row, so the reader calls
//! `MultiLeaderGeometry::from_persisted`, which takes every field as stored.

use serde_json::{Map, Value};

use super::{fields, id_ref_value, object, PayloadError};
use crate::ymodel::id::IdKind;
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    BlockId, MLeaderBackground, MLeaderBlockContent, MLeaderBreak, MLeaderColor, MLeaderColumns,
    MLeaderContent, MLeaderLine, MLeaderLineBreak, MLeaderLineExtras, MLeaderPathType,
    MLeaderReserved, MLeaderRoot, MLeaderRootExtras, MLeaderStyleValues, MLeaderText,
    MLeaderTextAngle, MLeaderToleranceContent, MultiLeaderGeometry, PlaneFrame, SemanticGeometry,
    StyleId,
};

// ---------------------------------------------------------------------------
// The entity.
// ---------------------------------------------------------------------------

pub(super) fn encode_multileader(multileader: &MultiLeaderGeometry) -> Value {
    object(
        "cad:MultiLeader",
        vec![
            ("style", encode_style(multileader.style())),
            (
                "style_name",
                Value::String(multileader.style_name().to_string()),
            ),
            ("override_mask", Value::from(multileader.override_mask())),
            ("content", encode_content(multileader.content())),
            ("text", encode_text(multileader.text())),
            (
                "roots",
                Value::Array(multileader.roots().iter().map(encode_root).collect()),
            ),
            (
                "lines",
                Value::Array(multileader.lines().iter().map(encode_line).collect()),
            ),
            ("reserved", encode_reserved(multileader.reserved())),
            ("plane", encode_plane(multileader.plane())),
            (
                "display",
                Value::Array(multileader.display().iter().map(super::encode).collect()),
            ),
        ],
    )
}

pub(super) fn decode_multileader(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:MultiLeader")?;
    let style = decode_style(&reader.take("style")?)?;
    let style_name = reader.string("style_name")?;
    let override_mask = reader.u32("override_mask")?;
    let content = decode_content(&reader.take("content")?)?;
    let text = decode_text(&reader.take("text")?)?;
    let raw_roots = reader.take("roots")?;
    let roots = match &raw_roots {
        Value::Array(items) => items.iter().map(decode_root).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new(
                "cad:MultiLeader.roots: expected an array",
            ));
        }
    };
    let raw_lines = reader.take("lines")?;
    let lines = match &raw_lines {
        Value::Array(items) => items.iter().map(decode_line).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new(
                "cad:MultiLeader.lines: expected an array",
            ));
        }
    };
    let reserved = decode_reserved(&reader.take("reserved")?)?;
    let plane = decode_plane(&reader.take("plane")?)?;
    let display = reader.vec_geometry("display")?;
    reader.finish()?;
    Ok(SemanticGeometry::MultiLeader(Box::new(
        MultiLeaderGeometry::from_persisted(
            style,
            style_name,
            override_mask,
            content,
            text,
            roots,
            lines,
            reserved,
            plane,
            display,
        ),
    )))
}

// ---------------------------------------------------------------------------
// Style snapshot.
// ---------------------------------------------------------------------------

fn encode_style(style: &MLeaderStyleValues) -> Value {
    fields(vec![
        ("path_type", encode_path_type(style.path_type)),
        ("enable_landing", Value::Bool(style.enable_landing)),
        ("enable_dogleg", Value::Bool(style.enable_dogleg)),
        ("landing_distance", json::encode_f64(style.landing_distance)),
        ("landing_gap", json::encode_f64(style.landing_gap)),
        ("arrowhead_size", json::encode_f64(style.arrowhead_size)),
        ("text_height", json::encode_f64(style.text_height)),
        (
            "text_style",
            id_ref_value(IdKind::Style, style.text_style.0),
        ),
        (
            "text_style_name",
            Value::String(style.text_style_name.clone()),
        ),
        (
            "text_font",
            style
                .text_font
                .as_ref()
                .map_or(Value::Null, |font| Value::String(font.clone())),
        ),
        ("scale_factor", json::encode_f64(style.scale_factor)),
        ("text_angle_type", encode_text_angle(style.text_angle_type)),
    ])
}

fn decode_style(value: &Value) -> Result<MLeaderStyleValues, PayloadError> {
    let mut reader = ObjectReader::new(value, "style")?;
    let style = MLeaderStyleValues {
        path_type: decode_path_type(&reader.take("path_type")?)?,
        enable_landing: reader.bool("enable_landing")?,
        enable_dogleg: reader.bool("enable_dogleg")?,
        landing_distance: reader.f64("landing_distance")?,
        landing_gap: reader.f64("landing_gap")?,
        arrowhead_size: reader.f64("arrowhead_size")?,
        text_height: reader.f64("text_height")?,
        text_style: StyleId(reader.id_seq("text_style", IdKind::Style)?),
        text_style_name: reader.string("text_style_name")?,
        text_font: reader.opt_string("text_font")?,
        scale_factor: reader.f64("scale_factor")?,
        text_angle_type: decode_text_angle(&reader.take("text_angle_type")?)?,
    };
    reader.finish()?;
    Ok(style)
}

fn encode_path_type(path_type: MLeaderPathType) -> Value {
    Value::String(
        match path_type {
            MLeaderPathType::Invisible => "invisible",
            MLeaderPathType::StraightLineSegments => "straight",
            MLeaderPathType::Spline => "spline",
        }
        .to_string(),
    )
}

fn decode_path_type(value: &Value) -> Result<MLeaderPathType, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "invisible" => Ok(MLeaderPathType::Invisible),
            "straight" => Ok(MLeaderPathType::StraightLineSegments),
            "spline" => Ok(MLeaderPathType::Spline),
            other => Err(PayloadError::new(format!(
                "style.path_type: unknown path type {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "style.path_type: expected a string token",
        )),
    }
}

fn encode_text_angle(angle: MLeaderTextAngle) -> Value {
    Value::String(
        match angle {
            MLeaderTextAngle::ParallelToLastLeaderLine => "parallel",
            MLeaderTextAngle::Horizontal => "horizontal",
            MLeaderTextAngle::Optimized => "optimized",
        }
        .to_string(),
    )
}

fn decode_text_angle(value: &Value) -> Result<MLeaderTextAngle, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "parallel" => Ok(MLeaderTextAngle::ParallelToLastLeaderLine),
            "horizontal" => Ok(MLeaderTextAngle::Horizontal),
            "optimized" => Ok(MLeaderTextAngle::Optimized),
            other => Err(PayloadError::new(format!(
                "style.text_angle_type: unknown text angle type {other:?}"
            ))),
        },
        _ => Err(PayloadError::new(
            "style.text_angle_type: expected a string token",
        )),
    }
}

// ---------------------------------------------------------------------------
// Content.
// ---------------------------------------------------------------------------

fn encode_content(content: &MLeaderContent) -> Value {
    match content {
        MLeaderContent::None => fields(vec![("kind", Value::String("none".to_string()))]),
        MLeaderContent::Text => fields(vec![("kind", Value::String("text".to_string()))]),
        MLeaderContent::Block(block) => {
            let mut map = Map::new();
            map.insert("kind".to_string(), Value::String("block".to_string()));
            map.insert(
                "block".to_string(),
                block
                    .block
                    .map_or(Value::Null, |block| id_ref_value(IdKind::Block, block.0)),
            );
            map.insert("name".to_string(), Value::String(block.name.clone()));
            map.insert(
                "handle".to_string(),
                block.handle.map_or(Value::Null, Value::from),
            );
            map.insert(
                "base_point".to_string(),
                json::encode_opt_point3(block.base_point),
            );
            map.insert("location".to_string(), json::encode_point3(block.location));
            map.insert("scale".to_string(), json::encode_point3(block.scale));
            map.insert("rotation".to_string(), json::encode_f64(block.rotation));
            map.insert("connection".to_string(), Value::from(block.connection));
            Value::Object(map)
        }
        MLeaderContent::Tolerance(tolerance) => fields(vec![
            ("kind", Value::String("tolerance".to_string())),
            ("text", Value::String(tolerance.text.clone())),
            ("height", json::encode_f64(tolerance.height)),
        ]),
    }
}

fn decode_content(value: &Value) -> Result<MLeaderContent, PayloadError> {
    let mut reader = ObjectReader::new(value, "content")?;
    let content = match reader.string("kind")?.as_str() {
        "none" => MLeaderContent::None,
        "text" => MLeaderContent::Text,
        "block" => {
            let block = match reader.take("block")? {
                Value::Null => None,
                value => Some(BlockId(super::decode_id_value(
                    &value,
                    "block",
                    IdKind::Block,
                )?)),
            };
            MLeaderContent::Block(MLeaderBlockContent {
                block,
                name: reader.string("name")?,
                handle: reader.opt_u64("handle")?,
                base_point: reader.opt_point3("base_point")?,
                location: reader.point3("location")?,
                scale: reader.point3("scale")?,
                rotation: reader.f64("rotation")?,
                connection: reader.i16("connection")?,
            })
        }
        "tolerance" => MLeaderContent::Tolerance(MLeaderToleranceContent {
            text: reader.string("text")?,
            height: reader.f64("height")?,
        }),
        other => {
            return Err(PayloadError::new(format!(
                "cad:MultiLeader.content.kind: unknown content {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(content)
}

// ---------------------------------------------------------------------------
// Text, roots and lines.
// ---------------------------------------------------------------------------

fn encode_text(text: &MLeaderText) -> Value {
    fields(vec![
        ("string", Value::String(text.string.clone())),
        ("location", json::encode_point3(text.location)),
        ("rotation", json::encode_f64(text.rotation)),
        ("height", json::encode_f64(text.height)),
        ("width", json::encode_f64(text.width)),
        ("attachment_point", Value::from(text.attachment_point)),
        ("left_attachment", Value::from(text.left_attachment)),
    ])
}

fn decode_text(value: &Value) -> Result<MLeaderText, PayloadError> {
    let mut reader = ObjectReader::new(value, "text")?;
    let text = MLeaderText {
        string: reader.string("string")?,
        location: reader.point3("location")?,
        rotation: reader.f64("rotation")?,
        height: reader.f64("height")?,
        width: reader.f64("width")?,
        attachment_point: reader.i16("attachment_point")?,
        left_attachment: reader.i16("left_attachment")?,
    };
    reader.finish()?;
    Ok(text)
}

fn encode_root(root: &MLeaderRoot) -> Value {
    fields(vec![
        ("leader_index", Value::from(root.leader_index)),
        ("landing_point", json::encode_point3(root.landing_point)),
        ("dogleg_vector", json::encode_point3(root.dogleg_vector)),
        ("dogleg_length", json::encode_f64(root.dogleg_length)),
    ])
}

fn decode_root(value: &Value) -> Result<MLeaderRoot, PayloadError> {
    let mut reader = ObjectReader::new(value, "root")?;
    let root = MLeaderRoot {
        leader_index: reader.i32("leader_index")?,
        landing_point: reader.point3("landing_point")?,
        dogleg_vector: reader.point3("dogleg_vector")?,
        dogleg_length: reader.f64("dogleg_length")?,
    };
    reader.finish()?;
    Ok(root)
}

fn encode_line(line: &MLeaderLine) -> Value {
    fields(vec![
        ("root_index", Value::from(line.root_index as u64)),
        ("index", Value::from(line.index)),
        ("points", super::basic::encode_vec_point3(&line.points)),
        ("path_type", encode_path_type(line.path_type)),
        ("override_flags", Value::from(line.override_flags)),
        ("arrowhead_size", json::encode_f64(line.arrowhead_size)),
        (
            "arrowhead_handle",
            line.arrowhead_handle.map_or(Value::Null, Value::from),
        ),
    ])
}

fn decode_line(value: &Value) -> Result<MLeaderLine, PayloadError> {
    let mut reader = ObjectReader::new(value, "line")?;
    let root_index = reader.usize("root_index")?;
    let index = reader.i32("index")?;
    let points = reader.vec_point3("points")?;
    let path_type = decode_path_type(&reader.take("path_type")?)?;
    let override_flags = reader.u32("override_flags")?;
    let arrowhead_size = reader.f64("arrowhead_size")?;
    let arrowhead_handle = reader.opt_u64("arrowhead_handle")?;
    reader.finish()?;
    Ok(MLeaderLine {
        root_index,
        index,
        points,
        path_type,
        override_flags,
        arrowhead_size,
        arrowhead_handle,
    })
}

// ---------------------------------------------------------------------------
// The reserved bag.
// ---------------------------------------------------------------------------

fn encode_reserved(reserved: &MLeaderReserved) -> Value {
    fields(vec![
        (
            "roots",
            Value::Array(reserved.roots.iter().map(encode_root_extras).collect()),
        ),
        (
            "lines",
            Value::Array(reserved.lines.iter().map(encode_line_extras).collect()),
        ),
        (
            "background",
            reserved
                .background
                .as_ref()
                .map_or(Value::Null, encode_background),
        ),
        (
            "columns",
            reserved
                .columns
                .as_ref()
                .map_or(Value::Null, encode_columns),
        ),
    ])
}

fn decode_reserved(value: &Value) -> Result<MLeaderReserved, PayloadError> {
    let mut reader = ObjectReader::new(value, "reserved")?;
    let raw_roots = reader.take("roots")?;
    let roots = match &raw_roots {
        Value::Array(items) => items
            .iter()
            .map(decode_root_extras)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("reserved.roots: expected an array"));
        }
    };
    let raw_lines = reader.take("lines")?;
    let lines = match &raw_lines {
        Value::Array(items) => items
            .iter()
            .map(decode_line_extras)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("reserved.lines: expected an array"));
        }
    };
    let background = match reader.take("background")? {
        Value::Null => None,
        value => Some(decode_background(&value)?),
    };
    let columns = match reader.take("columns")? {
        Value::Null => None,
        value => Some(decode_columns(&value)?),
    };
    reader.finish()?;
    Ok(MLeaderReserved {
        roots,
        lines,
        background,
        columns,
    })
}

fn encode_root_extras(root: &MLeaderRootExtras) -> Value {
    fields(vec![
        ("root_index", Value::from(root.root_index as u64)),
        ("content_valid", Value::Bool(root.content_valid)),
        ("unknown", Value::Bool(root.unknown)),
        (
            "text_attachment_direction",
            Value::from(root.text_attachment_direction),
        ),
        (
            "breaks",
            Value::Array(root.breaks.iter().map(encode_break).collect()),
        ),
    ])
}

fn decode_root_extras(value: &Value) -> Result<MLeaderRootExtras, PayloadError> {
    let mut reader = ObjectReader::new(value, "root_extras")?;
    let root = MLeaderRootExtras {
        root_index: reader.usize("root_index")?,
        content_valid: reader.bool("content_valid")?,
        unknown: reader.bool("unknown")?,
        text_attachment_direction: reader.i16("text_attachment_direction")?,
        breaks: decode_breaks(&reader.take("breaks")?)?,
    };
    reader.finish()?;
    Ok(root)
}

fn encode_line_extras(line: &MLeaderLineExtras) -> Value {
    fields(vec![
        ("root_index", Value::from(line.root_index as u64)),
        ("line_index", Value::from(line.line_index as u64)),
        ("segment_index", Value::from(line.segment_index)),
        (
            "breaks",
            Value::Array(line.breaks.iter().map(encode_line_break).collect()),
        ),
        ("break_info_count", Value::from(line.break_info_count)),
        (
            "legacy_breaks",
            Value::Array(line.legacy_breaks.iter().map(encode_break).collect()),
        ),
        ("line_color", encode_color(line.line_color)),
        (
            "line_type_handle",
            line.line_type_handle.map_or(Value::Null, Value::from),
        ),
        ("line_weight", Value::from(line.line_weight)),
    ])
}

fn decode_line_extras(value: &Value) -> Result<MLeaderLineExtras, PayloadError> {
    let mut reader = ObjectReader::new(value, "line_extras")?;
    let raw_breaks = reader.take("breaks")?;
    let breaks = match &raw_breaks {
        Value::Array(items) => items
            .iter()
            .map(decode_line_break)
            .collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("line_extras.breaks: expected an array"));
        }
    };
    let line = MLeaderLineExtras {
        root_index: reader.usize("root_index")?,
        line_index: reader.usize("line_index")?,
        segment_index: reader.i32("segment_index")?,
        breaks,
        break_info_count: reader.i32("break_info_count")?,
        legacy_breaks: decode_breaks(&reader.take("legacy_breaks")?)?,
        line_color: decode_color(&reader.take("line_color")?)?,
        line_type_handle: reader.opt_u64("line_type_handle")?,
        line_weight: reader.i16("line_weight")?,
    };
    reader.finish()?;
    Ok(line)
}

fn encode_line_break(record: &MLeaderLineBreak) -> Value {
    fields(vec![
        ("segment_index", Value::from(record.segment_index)),
        (
            "points",
            Value::Array(record.points.iter().map(encode_break).collect()),
        ),
    ])
}

fn decode_line_break(value: &Value) -> Result<MLeaderLineBreak, PayloadError> {
    let mut reader = ObjectReader::new(value, "line_break")?;
    let record = MLeaderLineBreak {
        segment_index: reader.i32("segment_index")?,
        points: decode_breaks(&reader.take("points")?)?,
    };
    reader.finish()?;
    Ok(record)
}

fn encode_break(break_: &MLeaderBreak) -> Value {
    fields(vec![
        ("start", json::encode_point3(break_.start)),
        ("end", json::encode_point3(break_.end)),
    ])
}

fn decode_break(value: &Value) -> Result<MLeaderBreak, PayloadError> {
    let mut reader = ObjectReader::new(value, "break")?;
    let break_ = MLeaderBreak {
        start: reader.point3("start")?,
        end: reader.point3("end")?,
    };
    reader.finish()?;
    Ok(break_)
}

fn decode_breaks(value: &Value) -> Result<Vec<MLeaderBreak>, PayloadError> {
    let Value::Array(items) = value else {
        return Err(PayloadError::new("breaks: expected an array"));
    };
    items.iter().map(decode_break).collect()
}

fn encode_color(color: MLeaderColor) -> Value {
    fields(vec![
        ("index", color.index.map_or(Value::Null, Value::from)),
        (
            "true_color",
            color.true_color.map_or(Value::Null, Value::from),
        ),
    ])
}

fn decode_color(value: &Value) -> Result<MLeaderColor, PayloadError> {
    let mut reader = ObjectReader::new(value, "line_color")?;
    let index = reader.opt_u16("index")?;
    let true_color = reader.opt_i32("true_color")?;
    reader.finish()?;
    Ok(MLeaderColor { index, true_color })
}

fn encode_background(background: &MLeaderBackground) -> Value {
    fields(vec![
        ("fill_enabled", Value::Bool(background.fill_enabled)),
        ("mask_fill_on", Value::Bool(background.mask_fill_on)),
        ("fill_color", encode_color(background.fill_color)),
        ("scale_factor", json::encode_f64(background.scale_factor)),
        ("transparency", Value::from(background.transparency)),
    ])
}

fn decode_background(value: &Value) -> Result<MLeaderBackground, PayloadError> {
    let mut reader = ObjectReader::new(value, "background")?;
    let background = MLeaderBackground {
        fill_enabled: reader.bool("fill_enabled")?,
        mask_fill_on: reader.bool("mask_fill_on")?,
        fill_color: decode_color(&reader.take("fill_color")?)?,
        scale_factor: reader.f64("scale_factor")?,
        transparency: reader.i32("transparency")?,
    };
    reader.finish()?;
    Ok(background)
}

fn encode_columns(columns: &MLeaderColumns) -> Value {
    fields(vec![
        ("column_type", Value::from(columns.column_type)),
        ("width", json::encode_f64(columns.width)),
        ("gutter", json::encode_f64(columns.gutter)),
        ("flow_reversed", Value::Bool(columns.flow_reversed)),
        ("sizes", super::basic::encode_vec_f64(&columns.sizes)),
    ])
}

fn decode_columns(value: &Value) -> Result<MLeaderColumns, PayloadError> {
    let mut reader = ObjectReader::new(value, "columns")?;
    let columns = MLeaderColumns {
        column_type: reader.i16("column_type")?,
        width: reader.f64("width")?,
        gutter: reader.f64("gutter")?,
        flow_reversed: reader.bool("flow_reversed")?,
        sizes: reader.vec_f64("sizes")?,
    };
    reader.finish()?;
    Ok(columns)
}

// ---------------------------------------------------------------------------
// Plane frame.
// ---------------------------------------------------------------------------

fn encode_plane(plane: PlaneFrame) -> Value {
    fields(vec![
        ("origin", json::encode_point3(plane.origin)),
        ("u", json::encode_point3(plane.u)),
        ("v", json::encode_point3(plane.v)),
        ("normal_reversed", Value::Bool(plane.normal_reversed)),
    ])
}

fn decode_plane(value: &Value) -> Result<PlaneFrame, PayloadError> {
    let mut reader = ObjectReader::new(value, "plane")?;
    let plane = PlaneFrame {
        origin: reader.point3("origin")?,
        u: reader.point3("u")?,
        v: reader.point3("v")?,
        normal_reversed: reader.bool("normal_reversed")?,
    };
    reader.finish()?;
    Ok(plane)
}
