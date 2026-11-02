//! Shared entity-data helpers for the reverse mapping.
//!
//! Split verbatim out of [`super::entity`] so that module stays under the
//! 1000-line source limit: the `EntityCommon` provenance mapping (colour,
//! lineweight, linetype, transparency) and the small geometry/alignment
//! primitives every entity writer needs live here. Nothing here decides
//! *whether* something is exported; it only builds the shared `EntityCommon`
//! plus the per-entity conversion reasons, so the move changes no output.

use acadrust::entities::{AttachmentPoint, EntityCommon};
use acadrust::tables::{LineType as DxfLineType, LineTypeElement};
use acadrust::types::{Color, LineWeight, Matrix3, Vector3};
use acadrust::{CadDocument, Transparency};
use cad_db::{
    sanitize_linetype_scale, DbEntity, EntityColor, EntityLineType, EntityLineWeight,
    EntityTransparency, LinetypePattern,
};
use cad_domain::{Point3, TextAlignH, TextAlignV};

use super::entity::EntityContext;
use super::report::MappingOutcome;

/// DXF group `6` value meaning "the layer's linetype".
///
/// The DXF spelling is `BYLAYER`; acadrust omits group `6` when the value is
/// empty or exactly `ByLayer`, so the canonical upper-case token is what makes
/// the symbolic form visible in the written file instead of implied by an
/// omission.
const BY_LAYER_LINETYPE: &str = "BYLAYER";

/// Fold extra per-entity reasons into a base outcome.
///
/// An `Exact` outcome with reasons becomes `Converted`, so one geometry is never
/// counted as both faithful and lossy.
pub(super) fn with_reasons(base: MappingOutcome, extra: Vec<String>) -> MappingOutcome {
    if extra.is_empty() {
        return base;
    }
    match base {
        MappingOutcome::Exact => MappingOutcome::Converted(extra),
        MappingOutcome::Converted(mut reasons) => {
            reasons.extend(extra);
            MappingOutcome::Converted(reasons)
        }
        MappingOutcome::Dropped(mut reasons) => {
            reasons.extend(extra);
            MappingOutcome::Dropped(reasons)
        }
    }
}

/// Build the common entity data and collect any per-entity conversion reasons.
pub(super) fn common_for(
    doc: &mut CadDocument,
    entity: &DbEntity,
    ctx: &EntityContext,
) -> (EntityCommon, Vec<String>) {
    let attributes = ctx.database.entity_render_attributes(entity.id);
    let mut reasons: Vec<String> = Vec::new();
    let mut common = EntityCommon::new();
    common.layer = ctx
        .database
        .layer(entity.layer)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "0".to_string());
    common.color = match attributes.color {
        EntityColor::Explicit([r, g, b]) => Color::Rgb { r, g, b },
        EntityColor::ByBlock => Color::ByBlock,
        EntityColor::ByLayer => Color::ByLayer,
    };
    common.line_weight = match attributes.lineweight {
        EntityLineWeight::Explicit(mm) => {
            LineWeight::Value((mm * 100.0).round().clamp(0.0, 211.0) as i16)
        }
        EntityLineWeight::Default => LineWeight::Default,
        EntityLineWeight::ByBlock => LineWeight::ByBlock,
        EntityLineWeight::ByLayer => LineWeight::ByLayer,
    };
    // Transparency: the stored provenance is written back symbolically where the
    // source was symbolic. acadrust's DXF writer omits group 440 when the value
    // `is_by_layer()`, and an omitted group 440 **is** the DXF encoding of
    // "by layer" (the layer table's `AcCmTransparency` XDATA carries the value),
    // so `ByLayer` needs no special case here. An explicit opacity is still
    // written on the inverted byte scale (0 = opaque, 255 = transparent) and
    // `ByBlock` stays symbolic.
    common.transparency = match attributes.transparency {
        EntityTransparency::Explicit(opacity) => Transparency::Explicit(
            ((1.0 - opacity.clamp(0.0, 1.0)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8,
        ),
        EntityTransparency::ByBlock => Transparency::ByBlock,
        EntityTransparency::ByLayer => Transparency::ByLayer,
    };
    // Linetype scale (group 48): the entity's own factor is written for **every**
    // provenance. An explicit entry keeps the copy stored inside its variant
    // (unchanged); a symbolic `ByLayer`/`ByBlock` entry now writes the factor the
    // importer recorded on the attributes, so a source with group `48` = 2.0
    // round-trips instead of silently dropping to 1.0. acadrust's DXF writer emits
    // group `48` when the value differs from `1.0`, which is the DXF default, so
    // the identity stays implicit. A corrupt stored factor is sanitised rather
    // than written as a non-finite double.
    common.linetype_scale = match &attributes.linetype {
        EntityLineType::Explicit { scale, .. } => *scale,
        EntityLineType::ByLayer | EntityLineType::ByBlock => {
            sanitize_linetype_scale(attributes.linetype_scale)
        }
    };
    common.linetype = match &attributes.linetype {
        EntityLineType::Explicit { name, pattern, .. } if !name.trim().is_empty() => {
            // A source that was stored as an explicit copy of `ByLayer` (an
            // older import) must not create a linetype table entry literally
            // named "ByLayer": the symbolic DXF token is written instead, so the
            // layer table keeps carrying the pattern.
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
        EntityLineType::Explicit { pattern, .. } => {
            if !pattern.elements.is_empty() {
                reasons.push(
                    "linetype has an empty name; its dash pattern is not written (continuous)"
                        .into(),
                );
            }
            String::new()
        }
        // Keep the symbol: the layer table entry carries the pattern, so a layer
        // linetype edit still round-trips. `BYLAYER` is the DXF spelling of
        // "by layer" for group 6 (acadrust writes group 6 only when the value is
        // neither empty nor the exact string `ByLayer`, which is why the
        // canonical upper-case token is used here: it is unambiguous in the
        // output rather than implied by an omitted group).
        EntityLineType::ByLayer => BY_LAYER_LINETYPE.to_string(),
        EntityLineType::ByBlock => "ByBlock".to_string(),
    };
    (common, reasons)
}

fn linetype_from_pattern(name: &str, pattern: &LinetypePattern) -> DxfLineType {
    let mut entry = DxfLineType::new(name);
    entry.elements = pattern
        .elements
        .iter()
        .map(|length| LineTypeElement {
            length: *length,
            complex: None,
        })
        .collect();
    entry.pattern_length = pattern.cycle;
    entry
}

/// Back-project a WCS point into the OCS frame `arbitrary_axis` rebuilds from
/// `normal`. Import maps a CIRCLE/ARC centre to WCS via
/// `Matrix3::arbitrary_axis(normal) * center`; the export must therefore apply
/// the transpose, or a tilted circle is silently displaced.
pub(super) fn ocs_from_wcs(normal: Point3, wcs: Point3) -> Vector3 {
    let basis = Matrix3::arbitrary_axis(v3(normal));
    basis.transpose() * v3(wcs)
}

pub(super) fn map_h_align(align: TextAlignH) -> acadrust::entities::TextHorizontalAlignment {
    use acadrust::entities::TextHorizontalAlignment as H;
    match align {
        TextAlignH::Left => H::Left,
        TextAlignH::Center => H::Center,
        TextAlignH::Right => H::Right,
    }
}

pub(super) fn map_v_align(align: TextAlignV) -> acadrust::entities::TextVerticalAlignment {
    use acadrust::entities::TextVerticalAlignment as V;
    match align {
        TextAlignV::Baseline => V::Baseline,
        TextAlignV::Bottom => V::Bottom,
        TextAlignV::Middle => V::Middle,
        TextAlignV::Top => V::Top,
    }
}

/// Inverse of `crate::style::attach_align`: map a domain text alignment onto an
/// MTEXT attachment point. MTEXT has no baseline row, so a baseline alignment is
/// approximated as bottom and reported.
pub(super) fn mtext_attachment(
    align_h: TextAlignH,
    align_v: TextAlignV,
) -> (AttachmentPoint, Option<String>) {
    use AttachmentPoint::*;
    let (row, reason) = match align_v {
        TextAlignV::Top => (0, None),
        TextAlignV::Middle => (1, None),
        TextAlignV::Bottom => (2, None),
        TextAlignV::Baseline => (
            2,
            Some(
                "MTEXT has no baseline attachment; vertical alignment written as bottom"
                    .to_string(),
            ),
        ),
    };
    let attachment = match (align_h, row) {
        (TextAlignH::Left, 0) => TopLeft,
        (TextAlignH::Center, 0) => TopCenter,
        (TextAlignH::Right, 0) => TopRight,
        (TextAlignH::Left, 1) => MiddleLeft,
        (TextAlignH::Center, 1) => MiddleCenter,
        (TextAlignH::Right, 1) => MiddleRight,
        (TextAlignH::Left, _) => BottomLeft,
        (TextAlignH::Center, _) => BottomCenter,
        (TextAlignH::Right, _) => BottomRight,
    };
    (attachment, reason)
}

pub(super) fn v3(p: Point3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}
