//! `ATTDEF` / `ATTRIB` reverse mapping (ADR-0055 M2).
//!
//! A block definition's `ATTDEF` members are written through the normal member
//! path like every other entity; a reference's concrete values are attached to
//! the `INSERT` record, which makes acadrust write group `66`, the `ATTRIB`
//! records and the terminating `SEQEND`. Tags, flags, placement and the stored
//! appearance are carried; a field the DXF record cannot express is named in the
//! export report instead of being invented.
//!
//! Submodule of [`crate::export`].

use acadrust::entities::attribute_definition::{
    HorizontalAlignment as DxfHAlign, VerticalAlignment as DxfVAlign,
};
use acadrust::entities::{AttributeDefinition as DxfAttributeDefinition, AttributeEntity};
use acadrust::types::{Color, LineWeight, Vector3};
use acadrust::Transparency;
use cad_db::{
    sanitize_linetype_scale, DbEntity, EntityColor, EntityLineType, EntityLineWeight,
    EntityTransparency,
};
use cad_domain::*;

use super::accumulator::ExportAccumulator;
use super::common::{common_for, linetype_from_pattern, v3, with_reasons, BY_LAYER_LINETYPE};
use super::entity::EntityContext;
use super::report::MappingOutcome;

/// Map the domain's named attribute flags onto acadrust's flag bits.
fn flags_to_dxf(flags: AttributeFlags) -> acadrust::entities::AttributeFlags {
    acadrust::entities::AttributeFlags {
        invisible: flags.invisible,
        constant: flags.constant,
        verify: flags.verify,
        preset: flags.preset,
        locked_position: flags.locked_position,
        annotative: flags.annotative,
    }
}

fn h_align_to_dxf(align: TextAlignH) -> DxfHAlign {
    match align {
        TextAlignH::Left => DxfHAlign::Left,
        TextAlignH::Center => DxfHAlign::Center,
        TextAlignH::Right => DxfHAlign::Right,
    }
}

fn v_align_to_dxf(align: TextAlignV) -> DxfVAlign {
    match align {
        TextAlignV::Baseline => DxfVAlign::Baseline,
        TextAlignV::Bottom => DxfVAlign::Bottom,
        TextAlignV::Middle => DxfVAlign::Middle,
        TextAlignV::Top => DxfVAlign::Top,
    }
}

/// The text style name a placement resolves through.
fn style_name(ctx: &EntityContext, style: StyleId) -> String {
    ctx.database
        .style(style)
        .map(|style| style.name.clone())
        .unwrap_or_else(|| "Standard".to_string())
}

/// The reasons a DXF `ATTDEF`/`ATTRIB` cannot carry this placement.
fn text_losses(text: &AttributeText) -> Vec<String> {
    let mut reasons = Vec::new();
    if text.wrap_width.is_some() {
        reasons
            .push("attribute text has no width boundary in DXF (wrap width is not written)".into());
    }
    if !text.line_spacing.is_default() {
        reasons.push(
            "attribute text has no line-spacing groups (DXF 44/73); the stored factor and style \
             are not written"
                .into(),
        );
    }
    reasons
}

/// Write one `ATTDEF` member through the normal entity member path.
pub(super) fn emit_definition(
    doc: &mut acadrust::CadDocument,
    entity: &DbEntity,
    definition: &AttributeDefinition,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let (common, mut reasons) = common_for(doc, entity, ctx);
    let mut dxf = DxfAttributeDefinition::new(
        definition.tag.clone(),
        definition.prompt.clone(),
        definition.default_value.clone(),
    );
    dxf.insertion_point = v3(definition.text.position);
    dxf.alignment_point = v3(definition.text.position);
    dxf.height = definition.text.height;
    dxf.rotation = definition.text.rotation;
    dxf.width_factor = definition.text.width_factor;
    dxf.oblique_angle = definition.text.oblique;
    dxf.text_style = style_name(ctx, definition.text.style);
    dxf.horizontal_alignment = h_align_to_dxf(definition.text.h_align);
    dxf.vertical_alignment = v_align_to_dxf(definition.text.v_align);
    dxf.flags = flags_to_dxf(definition.flags);
    dxf.normal = Vector3::UNIT_Z;
    dxf.common = common;
    reasons.extend(text_losses(&definition.text));
    acc.push(acadrust::EntityType::AttributeDefinition(dxf), ctx)?;
    acc.record(
        Some(entity.object.id),
        "attribute_definition",
        with_reasons(MappingOutcome::Exact, reasons),
    );
    Ok(())
}

/// Name the one placement loss the reference transform can cause: a collapsed
/// instance scale leaves no defined world height for the value.
const ATTRIBUTE_INSTANCE_TRANSFORM_DEGENERATE: &str =
    "attribute value text cannot be mapped to world coordinates: the reference transform \
     collapses an axis";

/// Apply the reference placement to a stored block-local text placement.
///
/// The model keeps [`AttributeValue::text`] in the referenced block's local
/// frame, while each `ATTRIB` record a DXF writer emits is in the frame the
/// reference is drawn in. The point is mapped directly; the baseline direction
/// gives the world rotation and the world length of the local glyph-up vector
/// gives the height scale (exactly the inverse of the import mapping).
fn attribute_text_to_world(
    text: &mut AttributeText,
    transform: &Transform3,
) -> Option<&'static str> {
    let (sin, cos) = text.rotation.sin_cos();
    let baseline = Point3 {
        x: cos,
        y: sin,
        z: 0.0,
    };
    let up = Point3 {
        x: -sin,
        y: cos,
        z: 0.0,
    };
    let world_baseline = transform.apply_vector(baseline);
    let world_up = transform.apply_vector(up);
    let baseline_len = world_baseline.x.hypot(world_baseline.y);
    let height_scale = world_up.x.hypot(world_up.y);
    if !baseline_len.is_finite()
        || baseline_len <= 1e-12
        || !height_scale.is_finite()
        || height_scale <= 1e-12
    {
        return Some(ATTRIBUTE_INSTANCE_TRANSFORM_DEGENERATE);
    }
    let position = transform.apply_point(text.position);
    if !position.x.is_finite() || !position.y.is_finite() || !position.z.is_finite() {
        return Some(ATTRIBUTE_INSTANCE_TRANSFORM_DEGENERATE);
    }
    text.position = position;
    text.rotation = world_baseline.y.atan2(world_baseline.x);
    text.height *= height_scale;
    None
}

/// Build the `ATTRIB` children one reference carries.
///
/// The values are stored block-local (the model keeps them in the same frame as
/// the `ATTDEF` they mirror), so each placement is mapped through the
/// reference's `transform` before it is written; a DXF `ATTRIB`'s group-10
/// point and height are world-space. Returns the reference-level conversion
/// reasons: every placement/value field is written, so the only losses are the
/// text fields DXF cannot carry plus a collapsed reference transform.
pub(super) fn attributes_for(
    doc: &mut acadrust::CadDocument,
    values: &[AttributeValue],
    transform: &Transform3,
    ctx: &EntityContext,
) -> (Vec<AttributeEntity>, Vec<String>) {
    let mut output = Vec::with_capacity(values.len());
    let mut reasons = Vec::new();
    for value in values {
        let mut text = value.text.clone();
        if let Some(reason) = attribute_text_to_world(&mut text, transform) {
            reasons.push(reason.to_string());
        }
        let mut attrib = AttributeEntity::new(value.tag.clone(), value.value.clone());
        attrib.insertion_point = v3(text.position);
        attrib.alignment_point = v3(text.position);
        attrib.height = text.height;
        attrib.rotation = text.rotation;
        attrib.width_factor = text.width_factor;
        attrib.oblique_angle = text.oblique;
        attrib.text_style = style_name(ctx, text.style);
        attrib.horizontal_alignment = h_align_to_dxf(text.h_align);
        attrib.vertical_alignment = v_align_to_dxf(text.v_align);
        attrib.flags = flags_to_dxf(value.flags);
        attrib.normal = Vector3::UNIT_Z;
        attrib.common = common_for_appearance(doc, value, ctx);
        reasons.extend(text_losses(&text));
        output.push(attrib);
    }
    (output, reasons)
}

/// The `EntityCommon` of one attribute value, from its stored appearance.
///
/// This is the same provenance rule as [`common_for`], but the appearance lives
/// on the [`AttributeValue`] rather than on an entity-render-attribute row.
fn common_for_appearance(
    doc: &mut acadrust::CadDocument,
    value: &AttributeValue,
    ctx: &EntityContext,
) -> acadrust::entities::EntityCommon {
    let appearance = &value.appearance;
    let mut common = acadrust::entities::EntityCommon::new();
    common.layer = ctx
        .database
        .layer(value.layer)
        .map(|layer| layer.name.clone())
        .unwrap_or_else(|| "0".to_string());
    common.color = match appearance.color {
        EntityColor::Explicit([r, g, b]) => Color::Rgb { r, g, b },
        EntityColor::ByBlock => Color::ByBlock,
        EntityColor::ByLayer => Color::ByLayer,
    };
    common.line_weight = match appearance.lineweight {
        EntityLineWeight::Explicit(mm) => {
            LineWeight::Value((mm * 100.0).round().clamp(0.0, 211.0) as i16)
        }
        EntityLineWeight::Default => LineWeight::Default,
        EntityLineWeight::ByBlock => LineWeight::ByBlock,
        EntityLineWeight::ByLayer => LineWeight::ByLayer,
    };
    common.transparency = match appearance.transparency {
        EntityTransparency::Explicit(opacity) => Transparency::Explicit(
            ((1.0 - opacity.clamp(0.0, 1.0)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8,
        ),
        EntityTransparency::ByBlock => Transparency::ByBlock,
        EntityTransparency::ByLayer => Transparency::ByLayer,
    };
    common.linetype_scale = match &appearance.linetype {
        EntityLineType::Explicit { scale, .. } => *scale,
        EntityLineType::ByLayer | EntityLineType::ByBlock => {
            sanitize_linetype_scale(appearance.linetype_scale)
        }
    };
    common.linetype = match &appearance.linetype {
        EntityLineType::Explicit { name, pattern, .. } if !name.trim().is_empty() => {
            if name.trim().eq_ignore_ascii_case("BYLAYER") {
                BY_LAYER_LINETYPE.to_string()
            } else {
                if doc.line_types.get(name).is_none() {
                    doc.line_types
                        .add_or_replace(linetype_from_pattern(name, pattern));
                }
                name.clone()
            }
        }
        EntityLineType::Explicit { .. } => String::new(),
        EntityLineType::ByLayer => BY_LAYER_LINETYPE.to_string(),
        EntityLineType::ByBlock => "ByBlock".to_string(),
    };
    common
}
