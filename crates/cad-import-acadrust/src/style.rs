//! Entity style resolution (colour, transparency, lineweight, text alignment).

use super::*;

/// Convert acadrust's DWG transparency byte (0 opaque .. 255 transparent) to an
/// opacity in `[0, 1]` (1 opaque .. 0 transparent).
pub(crate) fn dwg_transparency_to_opacity(alpha: u8) -> f32 {
    (1.0 - alpha as f32 / 255.0).clamp(0.0, 1.0)
}

/// Effective opacity of a layer entry.
///
/// A layer normally carries an explicit transparency; the `ByLayer`/`ByBlock`
/// variants are degenerate on a layer and fall back to opaque rather than
/// fabricating a value.
pub(crate) fn layer_opacity(transparency: acadrust::Transparency) -> f32 {
    match transparency {
        acadrust::Transparency::Explicit(alpha) => dwg_transparency_to_opacity(alpha),
        acadrust::Transparency::ByLayer | acadrust::Transparency::ByBlock => 1.0,
    }
}

/// Resolve an entity's own DWG transparency into the effective display value.
///
/// `ByObject` (`Explicit`) overrides the layer; `ByLayer` uses the pre-resolved
/// `layer_alpha`; `ByBlock` is kept symbolic so INSERT expansion substitutes the
/// containing reference's opacity.
pub(crate) fn resolve_entity_transparency(
    transparency: acadrust::Transparency,
    layer_alpha: f32,
) -> EntityTransparency {
    match transparency {
        acadrust::Transparency::Explicit(alpha) => {
            EntityTransparency::Explicit(dwg_transparency_to_opacity(alpha))
        }
        acadrust::Transparency::ByLayer => {
            EntityTransparency::Explicit(layer_alpha.clamp(0.0, 1.0))
        }
        acadrust::Transparency::ByBlock => EntityTransparency::ByBlock,
    }
}

/// sRGB value of a concrete acadrust colour, or `None` when symbolic.
pub(crate) fn concrete_rgb(color: acadrust::Color) -> Option<[u8; 3]> {
    match color {
        acadrust::Color::Rgb { r, g, b } => Some([r, g, b]),
        // ACI indices resolve through acadrust's own canonical table, so an
        // `Index(1)` becomes true red rather than a guessed constant.
        acadrust::Color::Index(_) => color.rgb().map(|(r, g, b)| [r, g, b]),
        acadrust::Color::ByLayer | acadrust::Color::ByBlock | acadrust::Color::None => None,
    }
}

/// Resolve a layer's colour to sRGB bytes.
///
/// A layer carries a concrete colour in practice; the symbolic variants are
/// degenerate on a layer and fall back to white (AutoCAD's nominal default
/// entity colour) rather than fabricating a value.
pub(crate) fn layer_rgb(color: acadrust::Color) -> [u8; 3] {
    concrete_rgb(color).unwrap_or([255, 255, 255])
}

/// Resolve an entity's own colour into the effective display value.
///
/// An explicit entity colour (`ByObject`: true colour or an ACI index) wins over
/// the layer. `ByLayer` uses the layer's pre-resolved colour. `ByBlock` is kept
/// symbolic so INSERT expansion substitutes the containing reference's colour;
/// `None` (no colour) is treated as unresolved `ByLayer` rather than drawn black.
pub(crate) fn resolve_entity_color(
    color: acadrust::Color,
    layer_color: Option<[u8; 3]>,
) -> EntityColor {
    match concrete_rgb(color) {
        Some(rgb) => EntityColor::Explicit(rgb),
        None => match color {
            acadrust::Color::ByBlock => EntityColor::ByBlock,
            _ => match layer_color {
                Some(rgb) => EntityColor::Explicit(rgb),
                None => EntityColor::ByLayer,
            },
        },
    }
}

/// A lineweight in millimetres, or `None` for the symbolic/`Default` variants.
///
/// acadrust stores concrete weights as 1/100 mm; `LineWeight::millimeters`
/// performs that conversion, so no scale factor is invented here.
pub(crate) fn lineweight_mm(weight: acadrust::LineWeight) -> Option<f32> {
    match weight {
        acadrust::LineWeight::Value(_) => weight.millimeters().map(|mm| mm as f32),
        acadrust::LineWeight::ByLayer
        | acadrust::LineWeight::ByBlock
        | acadrust::LineWeight::Default => None,
    }
}

/// Resolve an entity's own lineweight into the effective display value.
///
/// A concrete entity weight wins over the layer; `ByLayer` uses the layer's
/// pre-resolved weight; `ByBlock` stays symbolic for INSERT expansion;
/// `Default` keeps acadrust's explicit "default" meaning.
pub(crate) fn resolve_entity_lineweight(
    weight: acadrust::LineWeight,
    layer_weight: Option<f32>,
) -> EntityLineWeight {
    match weight {
        acadrust::LineWeight::Value(_) => {
            EntityLineWeight::Explicit(lineweight_mm(weight).unwrap_or(0.0))
        }
        acadrust::LineWeight::ByBlock => EntityLineWeight::ByBlock,
        acadrust::LineWeight::Default => EntityLineWeight::Default,
        acadrust::LineWeight::ByLayer => match layer_weight {
            Some(mm) => EntityLineWeight::Explicit(mm),
            None => EntityLineWeight::ByLayer,
        },
    }
}

/// `true` when a linetype name is the symbolic `ByLayer` (or absent).
///
/// acadrust stores an empty string for an entity that did not set a linetype,
/// and the literal `"ByLayer"` when it did; both mean "use the layer's".
pub(crate) fn is_bylayer_linetype(name: &str) -> bool {
    let name = name.trim();
    name.is_empty() || name.eq_ignore_ascii_case("ByLayer")
}

/// `true` when a linetype name is the symbolic `ByBlock`.
pub(crate) fn is_byblock_linetype(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("ByBlock")
}

/// `true` when a named linetype is the standard solid one.
pub(crate) fn is_continuous_linetype(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("Continuous")
}

/// Resolve an entity's linetype into the effective display value.
///
/// Precedence mirrors colour/lineweight: an explicit named linetype wins, then
/// `ByLayer` uses the layer's pre-resolved pattern, and `ByBlock` stays
/// symbolic for INSERT expansion. `named_pattern` is the caller's lookup of a
/// concrete pattern for an explicit name (`None` when unknown); `layer_pattern`
/// is the layer's resolved pattern.
///
/// The result is never a fabricated pattern:
/// * a concrete pattern is `Explicit` with the entity's own `scale`;
/// * `ByLayer` with a reachable layer pattern is `Explicit` using that pattern;
/// * `ByBlock` is symbolic;
/// * an unknown named linetype falls back to an explicit **continuous** pattern
///   carrying the source name, and the caller reports a `Partial` reason;
/// * a `ByLayer` with no reachable pattern is symbolic `ByLayer`, which the
///   representation treats as continuous and marks unresolved.
pub(crate) fn resolve_entity_linetype(
    name: &str,
    scale: f64,
    named_pattern: Option<LinetypePattern>,
    layer_pattern: Option<LinetypePattern>,
) -> EntityLineType {
    // A non-finite/zero scale is treated as 1.0; the representation sanitises
    // again, and this keeps a corrupt scale from disabling dashes entirely.
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    if is_byblock_linetype(name) {
        return EntityLineType::ByBlock;
    }
    // The standard solid linetype is known and continuous; record it explicitly
    // rather than marking it unresolved.
    if is_continuous_linetype(name) {
        return EntityLineType::Explicit {
            name: name.trim().to_string(),
            pattern: LinetypePattern::continuous(),
            scale,
        };
    }
    if is_bylayer_linetype(name) {
        return match layer_pattern {
            Some(pattern) => EntityLineType::Explicit {
                name: "ByLayer".to_string(),
                pattern,
                scale,
            },
            None => EntityLineType::ByLayer,
        };
    }
    match named_pattern {
        Some(pattern) => EntityLineType::Explicit {
            name: name.trim().to_string(),
            pattern,
            scale,
        },
        None => EntityLineType::Explicit {
            name: name.trim().to_string(),
            // Unknown name: an explicit continuous fallback, never invented
            // dashes. The importer reports the missing definition.
            pattern: LinetypePattern::continuous(),
            scale,
        },
    }
}

/// Convert an acadrust linetype's elements into a [`LinetypePattern`].
///
/// Complex (shape/text) elements carry no dash length of their own; their
/// element length is still meaningful as a gap/dash, so it is kept, and the
/// `complex` flag tells the caller that glyphs were dropped.
pub(crate) fn linetype_pattern(linetype: &acadrust::LineType) -> (LinetypePattern, bool) {
    let elements = linetype.elements.iter().map(|e| e.length);
    let pattern = LinetypePattern::from_elements(elements);
    (pattern, linetype.is_complex())
}

/// Lower-cased extension of a font reference, without the dot.
pub(crate) fn font_extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) fn map_h_align(align: TextHorizontalAlignment) -> TextAlignH {
    match align {
        TextHorizontalAlignment::Center | TextHorizontalAlignment::Middle => TextAlignH::Center,
        TextHorizontalAlignment::Right => TextAlignH::Right,
        // Aligned/Fit still start at the left point; we do not stretch.
        TextHorizontalAlignment::Left
        | TextHorizontalAlignment::Aligned
        | TextHorizontalAlignment::Fit => TextAlignH::Left,
    }
}

pub(crate) fn map_v_align(align: TextVerticalAlignment) -> TextAlignV {
    match align {
        TextVerticalAlignment::Baseline => TextAlignV::Baseline,
        TextVerticalAlignment::Bottom => TextAlignV::Bottom,
        TextVerticalAlignment::Middle => TextAlignV::Middle,
        TextVerticalAlignment::Top => TextAlignV::Top,
    }
}

pub(crate) fn attach_align(attachment: AttachmentPoint) -> (TextAlignH, TextAlignV) {
    use AttachmentPoint::*;
    let h = match attachment {
        TopLeft | MiddleLeft | BottomLeft => TextAlignH::Left,
        TopCenter | MiddleCenter | BottomCenter => TextAlignH::Center,
        TopRight | MiddleRight | BottomRight => TextAlignH::Right,
    };
    let v = match attachment {
        TopLeft | TopCenter | TopRight => TextAlignV::Top,
        MiddleLeft | MiddleCenter | MiddleRight => TextAlignV::Middle,
        BottomLeft | BottomCenter | BottomRight => TextAlignV::Bottom,
    };
    (h, v)
}
