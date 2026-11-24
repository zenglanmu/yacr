//! The analytic / textual / image payload classes (ADR-0037 D4).
//!
//! Covers `cad:Line`, `cad:Polyline`, `cad:Circle`, `cad:Arc`, `cad:Ellipse`,
//! `cad:Spline`, `cad:Point`, `cad:Insert`, `cad:Text`, `cad:Shape`,
//! `cad:Opaque`, `cad:Compound`, `cad:Image` and `cad:Mask`.
//!
//! These are the classes whose Rust payload is plain data with public fields, so
//! the codec needs no restore constructor: it builds the variant directly. The
//! four families with derived display caches live in their own modules.
//!
//! Enum-valued fields are written as canonical lowercase tokens
//! (`"left"`, `"at_least"`, …); reading an unknown token is an error, never a
//! default.

use serde_json::Value;

use super::{fields, id_ref_value, object, PayloadError};
use crate::ymodel::id::IdKind;
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    BlockId, ImageAdjust, ImageClip, Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV,
    TextLineSpacing, TextLineSpacingStyle, Transform3,
};

// ---------------------------------------------------------------------------
// Line / Polyline / conics / spline / point.
// ---------------------------------------------------------------------------

pub(super) fn encode_line(start: Point3, end: Point3) -> Value {
    object(
        "cad:Line",
        vec![
            ("start", json::encode_point3(start)),
            ("end", json::encode_point3(end)),
        ],
    )
}

pub(super) fn decode_line(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Line")?;
    let start = reader.point3("start")?;
    let end = reader.point3("end")?;
    reader.finish()?;
    Ok(SemanticGeometry::Line { start, end })
}

pub(super) fn encode_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    width: f64,
) -> Value {
    object(
        "cad:Polyline",
        vec![
            ("points", encode_vec_point3(points)),
            ("bulges", encode_vec_f64(bulges)),
            ("closed", Value::Bool(closed)),
            ("width", json::encode_f64(width)),
        ],
    )
}

pub(super) fn decode_polyline(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Polyline")?;
    let points = reader.vec_point3("points")?;
    let bulges = reader.vec_f64("bulges")?;
    let closed = reader.bool("closed")?;
    let width = reader.f64("width")?;
    reader.finish()?;
    Ok(SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        width,
    })
}

pub(super) fn encode_circle(center: Point3, normal: Point3, radius: f64) -> Value {
    object(
        "cad:Circle",
        vec![
            ("center", json::encode_point3(center)),
            ("normal", json::encode_point3(normal)),
            ("radius", json::encode_f64(radius)),
        ],
    )
}

pub(super) fn decode_circle(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Circle")?;
    let center = reader.point3("center")?;
    let normal = reader.point3("normal")?;
    let radius = reader.f64("radius")?;
    reader.finish()?;
    Ok(SemanticGeometry::Circle {
        center,
        normal,
        radius,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
) -> Value {
    object(
        "cad:Arc",
        vec![
            ("center", json::encode_point3(center)),
            ("normal", json::encode_point3(normal)),
            ("radius", json::encode_f64(radius)),
            ("start", json::encode_f64(start)),
            ("sweep", json::encode_f64(sweep)),
        ],
    )
}

pub(super) fn decode_arc(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Arc")?;
    let center = reader.point3("center")?;
    let normal = reader.point3("normal")?;
    let radius = reader.f64("radius")?;
    let start = reader.f64("start")?;
    let sweep = reader.f64("sweep")?;
    reader.finish()?;
    Ok(SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_ellipse(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
) -> Value {
    object(
        "cad:Ellipse",
        vec![
            ("center", json::encode_point3(center)),
            ("normal", json::encode_point3(normal)),
            ("major_axis", json::encode_point3(major_axis)),
            ("ratio", json::encode_f64(ratio)),
            ("start", json::encode_f64(start)),
            ("sweep", json::encode_f64(sweep)),
        ],
    )
}

pub(super) fn decode_ellipse(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Ellipse")?;
    let center = reader.point3("center")?;
    let normal = reader.point3("normal")?;
    let major_axis = reader.point3("major_axis")?;
    let ratio = reader.f64("ratio")?;
    let start = reader.f64("start")?;
    let sweep = reader.f64("sweep")?;
    reader.finish()?;
    Ok(SemanticGeometry::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start,
        sweep,
    })
}

pub(super) fn encode_spline(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
    closed: bool,
) -> Value {
    object(
        "cad:Spline",
        vec![
            ("degree", Value::from(degree)),
            ("knots", encode_vec_f64(knots)),
            ("control_points", encode_vec_point3(control_points)),
            ("weights", encode_vec_f64(weights)),
            ("closed", Value::Bool(closed)),
        ],
    )
}

pub(super) fn decode_spline(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Spline")?;
    let degree = reader.u32("degree")?;
    let knots = reader.vec_f64("knots")?;
    let control_points = reader.vec_point3("control_points")?;
    let weights = reader.vec_f64("weights")?;
    let closed = reader.bool("closed")?;
    reader.finish()?;
    Ok(SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
        closed,
    })
}

pub(super) fn encode_point(point: Point3) -> Value {
    object("cad:Point", vec![("point", json::encode_point3(point))])
}

pub(super) fn decode_point(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Point")?;
    let point = reader.point3("point")?;
    reader.finish()?;
    Ok(SemanticGeometry::Point(point))
}

// ---------------------------------------------------------------------------
// Insert / transform.
// ---------------------------------------------------------------------------

pub(super) fn encode_insert(
    block: u128,
    transform: &Transform3,
    attributes: &[crate::AttributeValue],
) -> Value {
    let mut entries = vec![
        ("block", id_ref_value(IdKind::Block, block)),
        ("transform", encode_transform3(transform)),
    ];
    // ADR-0055: the list is omitted when empty, so a minor-13 reader sees the
    // two-field object it models. A present non-empty list is a minor-14 row.
    if !attributes.is_empty() {
        entries.push((
            "attributes",
            super::attribute::encode_attribute_values(attributes),
        ));
    }
    object("cad:Insert", entries)
}

pub(super) fn decode_insert(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Insert")?;
    let block = reader.id_seq("block", IdKind::Block)?;
    let transform = decode_transform3(&reader.take("transform")?)?;
    // Absent `attributes` is `[]` (a minor-13 row); a present key must be an
    // array — `"attributes": null` is refused rather than guessed.
    let attributes = match reader.raw_if_present("attributes") {
        None => Vec::new(),
        Some(Value::Null) => {
            return Err(PayloadError::new(
                "cad:Insert.attributes: expected an array, not null",
            ));
        }
        Some(value) => super::attribute::decode_attribute_values(&value, "cad:Insert.attributes")?,
    };
    reader.finish()?;
    Ok(SemanticGeometry::Insert {
        block: BlockId(block),
        transform,
        attributes,
    })
}

/// `Transform3` as `{"matrix": [[f64; 4]; 4]}` (row-major, as the type stores
/// it).
pub(super) fn encode_transform3(transform: &Transform3) -> Value {
    let rows: Vec<Value> = transform
        .matrix
        .iter()
        .map(|row| Value::Array(row.iter().map(|value| json::encode_f64(*value)).collect()))
        .collect();
    // A plain nested object: only element payloads carry a `class` key.
    let mut map = serde_json::Map::new();
    map.insert("matrix".to_string(), Value::Array(rows));
    Value::Object(map)
}

pub(super) fn decode_transform3(value: &Value) -> Result<Transform3, PayloadError> {
    let mut reader = ObjectReader::new(value, "transform")?;
    let raw = reader.take("matrix")?;
    reader.finish()?;
    let rows = match &raw {
        Value::Array(rows) if rows.len() == 4 => rows,
        _ => {
            return Err(PayloadError::new(
                "transform.matrix: expected four rows of four numbers",
            ));
        }
    };
    let mut matrix = [[0.0f64; 4]; 4];
    for (row_index, row) in rows.iter().enumerate() {
        let cells = match row {
            Value::Array(cells) if cells.len() == 4 => cells,
            _ => {
                return Err(PayloadError::new(
                    "transform.matrix: expected four rows of four numbers",
                ));
            }
        };
        for (column, cell) in cells.iter().enumerate() {
            matrix[row_index][column] = json::decode_f64(cell)
                .map_err(|error| PayloadError::with("transform.matrix", error))?;
        }
    }
    Ok(Transform3 { matrix })
}

// ---------------------------------------------------------------------------
// Text / shape.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_text(
    text: &str,
    position: Point3,
    style: u128,
    height: f64,
    rotation: f64,
    font: Option<&str>,
    h_align: TextAlignH,
    v_align: TextAlignV,
    wrap_width: Option<f64>,
    width_factor: f64,
    line_spacing: &TextLineSpacing,
    mirrored: bool,
) -> Value {
    let mut fields = vec![
        ("text", Value::String(text.to_string())),
        ("position", json::encode_point3(position)),
        ("style", id_ref_value(IdKind::Style, style)),
        ("height", json::encode_f64(height)),
        ("rotation", json::encode_f64(rotation)),
        (
            "font",
            font.map_or(Value::Null, |font| Value::String(font.to_string())),
        ),
        ("h_align", encode_align_h(h_align)),
        ("v_align", align_v_token(v_align)),
        ("wrap_width", json::encode_opt_f64(wrap_width)),
        ("width_factor", json::encode_f64(width_factor)),
        ("line_spacing", encode_line_spacing(line_spacing)),
    ];
    // Minor 15: the MIRRTEXT glyph-mirror flag. It is omitted when false so an
    // ordinary document stays byte-compatible with the minor-14 payload and a
    // minor-14 reader (which refuses unknown keys) can still open it.
    if mirrored {
        fields.push(("mirrored", Value::Bool(true)));
    }
    object("cad:Text", fields)
}

pub(super) fn decode_text(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Text")?;
    let text = reader.string("text")?;
    let position = reader.point3("position")?;
    let style = reader.id_seq("style", IdKind::Style)?;
    let height = reader.f64("height")?;
    let rotation = reader.f64("rotation")?;
    let font = reader.opt_string("font")?;
    let h_align = decode_align_h(&reader.take("h_align")?)?;
    let v_align = decode_align_v(&reader.take("v_align")?)?;
    let wrap_width = reader.opt_f64("wrap_width")?;
    let width_factor = reader.f64("width_factor")?;
    let line_spacing = decode_line_spacing(&reader.take("line_spacing")?)?;
    let mirrored = reader.boolean_if_present("mirrored")?;
    reader.finish()?;
    Ok(SemanticGeometry::Text {
        text,
        position,
        style: StyleId(style),
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
        width_factor,
        line_spacing,
        mirrored,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_shape(
    shape_name: &str,
    code: u32,
    position: Point3,
    size: f64,
    rotation: f64,
    font: Option<&str>,
) -> Value {
    object(
        "cad:Shape",
        vec![
            ("shape_name", Value::String(shape_name.to_string())),
            ("code", Value::from(code)),
            ("position", json::encode_point3(position)),
            ("size", json::encode_f64(size)),
            ("rotation", json::encode_f64(rotation)),
            (
                "font",
                font.map_or(Value::Null, |font| Value::String(font.to_string())),
            ),
        ],
    )
}

pub(super) fn decode_shape(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Shape")?;
    let shape_name = reader.string("shape_name")?;
    let code = reader.u32("code")?;
    let position = reader.point3("position")?;
    let size = reader.f64("size")?;
    let rotation = reader.f64("rotation")?;
    let font = reader.opt_string("font")?;
    reader.finish()?;
    Ok(SemanticGeometry::Shape {
        shape_name,
        code,
        position,
        size,
        rotation,
        font,
    })
}

// ---------------------------------------------------------------------------
// Opaque / compound.
// ---------------------------------------------------------------------------

pub(super) fn encode_opaque(type_key: &str, version: u32, payload: &[u8]) -> Value {
    object(
        "cad:Opaque",
        vec![
            ("type_key", Value::String(type_key.to_string())),
            ("version", Value::from(version)),
            ("payload", Value::String(super::encode_hex(payload))),
        ],
    )
}

pub(super) fn decode_opaque(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Opaque")?;
    let type_key = reader.string("type_key")?;
    let version = reader.u32("version")?;
    let payload = reader.hex_bytes("payload")?;
    reader.finish()?;
    Ok(SemanticGeometry::Opaque {
        type_key,
        version,
        payload,
    })
}

pub(super) fn encode_compound(children: &[SemanticGeometry]) -> Value {
    object(
        "cad:Compound",
        vec![(
            "children",
            Value::Array(children.iter().map(super::encode).collect()),
        )],
    )
}

pub(super) fn decode_compound(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Compound")?;
    let children = reader.vec_geometry("children")?;
    reader.finish()?;
    Ok(SemanticGeometry::Compound(children))
}

// ---------------------------------------------------------------------------
// Image / mask.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_image(
    origin: Point3,
    u: Point3,
    v: Point3,
    pixels: [f64; 2],
    file: Option<&str>,
    clip: Option<&ImageClip>,
    visible: bool,
    adjust: &ImageAdjust,
) -> Value {
    object(
        "cad:Image",
        vec![
            ("origin", json::encode_point3(origin)),
            ("u", json::encode_point3(u)),
            ("v", json::encode_point3(v)),
            ("pixels", json::encode_f64x2(pixels)),
            (
                "file",
                file.map_or(Value::Null, |file| Value::String(file.to_string())),
            ),
            ("clip", clip.map_or(Value::Null, encode_image_clip)),
            ("visible", Value::Bool(visible)),
            ("adjust", encode_image_adjust(adjust)),
        ],
    )
}

pub(super) fn decode_image(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Image")?;
    let origin = reader.point3("origin")?;
    let u = reader.point3("u")?;
    let v = reader.point3("v")?;
    let pixels = reader.f64x2("pixels")?;
    let file = reader.opt_string("file")?;
    let clip = match reader.take("clip")? {
        Value::Null => None,
        value => Some(decode_image_clip(&value)?),
    };
    let visible = reader.bool("visible")?;
    let adjust = decode_image_adjust(&reader.take("adjust")?)?;
    reader.finish()?;
    Ok(SemanticGeometry::Image {
        origin,
        u,
        v,
        pixels,
        file,
        clip,
        visible,
        adjust,
    })
}

fn encode_image_clip(clip: &ImageClip) -> Value {
    fields(vec![
        (
            "vertices",
            Value::Array(
                clip.vertices
                    .iter()
                    .map(|vertex| json::encode_f64x2(*vertex))
                    .collect(),
            ),
        ),
        ("inside", Value::Bool(clip.inside)),
    ])
}

fn decode_image_clip(value: &Value) -> Result<ImageClip, PayloadError> {
    let mut reader = ObjectReader::new(value, "clip")?;
    let vertices = reader.vec_f64x2("vertices")?;
    let inside = reader.bool("inside")?;
    reader.finish()?;
    Ok(ImageClip { vertices, inside })
}

fn encode_image_adjust(adjust: &ImageAdjust) -> Value {
    fields(vec![
        ("brightness", Value::from(adjust.brightness)),
        ("contrast", Value::from(adjust.contrast)),
        ("fade", Value::from(adjust.fade)),
        ("monochrome", Value::Bool(adjust.monochrome)),
    ])
}

fn decode_image_adjust(value: &Value) -> Result<ImageAdjust, PayloadError> {
    let mut reader = ObjectReader::new(value, "adjust")?;
    let brightness = reader.u8("brightness")?;
    let contrast = reader.u8("contrast")?;
    let fade = reader.u8("fade")?;
    let monochrome = reader.bool("monochrome")?;
    reader.finish()?;
    Ok(ImageAdjust {
        brightness,
        contrast,
        fade,
        monochrome,
    })
}

pub(super) fn encode_mask(boundary: &[Point3], inverted: bool) -> Value {
    object(
        "cad:Mask",
        vec![
            ("boundary", encode_vec_point3(boundary)),
            ("inverted", Value::Bool(inverted)),
        ],
    )
}

pub(super) fn decode_mask(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Mask")?;
    let boundary = reader.vec_point3("boundary")?;
    let inverted = reader.bool("inverted")?;
    reader.finish()?;
    Ok(SemanticGeometry::Mask { boundary, inverted })
}

// ---------------------------------------------------------------------------
// Shared encoders for the family modules.
// ---------------------------------------------------------------------------

pub(super) fn encode_vec_point3(points: &[Point3]) -> Value {
    Value::Array(
        points
            .iter()
            .map(|point| json::encode_point3(*point))
            .collect(),
    )
}

pub(super) fn encode_vec_f64(values: &[f64]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| json::encode_f64(*value))
            .collect(),
    )
}

pub(super) fn encode_line_spacing(spacing: &TextLineSpacing) -> Value {
    fields(vec![
        ("factor", json::encode_f64(spacing.factor)),
        (
            "style",
            Value::String(
                match spacing.style {
                    TextLineSpacingStyle::AtLeast => "at_least",
                    TextLineSpacingStyle::Exactly => "exactly",
                }
                .to_string(),
            ),
        ),
    ])
}

pub(super) fn decode_line_spacing(value: &Value) -> Result<TextLineSpacing, PayloadError> {
    let mut reader = ObjectReader::new(value, "line_spacing")?;
    let factor = reader.f64("factor")?;
    let style = match reader.take("style")? {
        Value::String(token) => match token.as_str() {
            "at_least" => TextLineSpacingStyle::AtLeast,
            "exactly" => TextLineSpacingStyle::Exactly,
            other => {
                return Err(PayloadError::new(format!(
                    "line_spacing.style: unknown spacing style {other:?}"
                )));
            }
        },
        _ => {
            return Err(PayloadError::new(
                "line_spacing.style: expected a string token",
            ));
        }
    };
    reader.finish()?;
    Ok(TextLineSpacing { factor, style })
}

pub(super) fn encode_align_h(align: TextAlignH) -> Value {
    Value::String(
        match align {
            TextAlignH::Left => "left",
            TextAlignH::Center => "center",
            TextAlignH::Right => "right",
        }
        .to_string(),
    )
}

pub(super) fn decode_align_h(value: &Value) -> Result<TextAlignH, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "left" => Ok(TextAlignH::Left),
            "center" => Ok(TextAlignH::Center),
            "right" => Ok(TextAlignH::Right),
            other => Err(PayloadError::new(format!(
                "h_align: unknown horizontal alignment {other:?}"
            ))),
        },
        _ => Err(PayloadError::new("h_align: expected a string token")),
    }
}

pub(super) fn align_v_token(align: TextAlignV) -> Value {
    Value::String(
        match align {
            TextAlignV::Baseline => "baseline",
            TextAlignV::Bottom => "bottom",
            TextAlignV::Middle => "middle",
            TextAlignV::Top => "top",
        }
        .to_string(),
    )
}

pub(super) fn decode_align_v(value: &Value) -> Result<TextAlignV, PayloadError> {
    match value {
        Value::String(token) => match token.as_str() {
            "baseline" => Ok(TextAlignV::Baseline),
            "bottom" => Ok(TextAlignV::Bottom),
            "middle" => Ok(TextAlignV::Middle),
            "top" => Ok(TextAlignV::Top),
            other => Err(PayloadError::new(format!(
                "v_align: unknown vertical alignment {other:?}"
            ))),
        },
        _ => Err(PayloadError::new("v_align: expected a string token")),
    }
}
