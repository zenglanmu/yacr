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

/// The ACI identity of a layer colour, when the source carried one.
///
/// Only `Color::Index` has an index to preserve; a true-colour layer keeps its
/// sRGB value in [`layer_rgb`] and answers `None` here. Symbolic variants
/// (`ByLayer`/`ByBlock`/`None`) are degenerate on a layer record and likewise
/// answer `None` rather than an invented index.
pub(crate) fn layer_aci(color: acadrust::Color) -> Option<u8> {
    match color {
        acadrust::Color::Index(index) => Some(index),
        acadrust::Color::ByLayer
        | acadrust::Color::ByBlock
        | acadrust::Color::None
        | acadrust::Color::Rgb { .. } => None,
    }
}

/// Map an acadrust lineweight onto the stored layer lineweight.
///
/// The concrete variant keeps the source's 1/100 mm integer so a re-export
/// writes back exactly what it read; a negative value is unrepresentable as a
/// weight and clamps to `0` (0.00 mm, an explicit "hairline") instead of
/// wrapping into a huge `u16`.
pub(crate) fn layer_line_weight(weight: acadrust::LineWeight) -> LayerLineWeight {
    match weight {
        acadrust::LineWeight::Value(hundredths) => {
            LayerLineWeight::Hundredths(hundredths.max(0) as u16)
        }
        acadrust::LineWeight::Default => LayerLineWeight::Default,
        acadrust::LineWeight::ByLayer | acadrust::LineWeight::ByBlock => LayerLineWeight::ByLayer,
    }
}

/// Build the stored attribute row for one source layer.
///
/// `visible` is deliberately **not** recomputed here: the caller keeps using
/// `!(off || frozen)` for [`Layer::visible`](cad_db::Layer::visible), and the
/// row records the two flags separately so the invariant can be asserted.
pub(crate) fn layer_attributes(layer: &acadrust::tables::Layer) -> LayerAttributes {
    let linetype = layer.line_type.trim();
    LayerAttributes {
        off: layer.flags.off,
        frozen: layer.flags.frozen,
        frozen_in_new_viewport: layer.flags.frozen_in_new_viewport,
        locked: layer.flags.locked,
        xref_dependent: layer.flags.xref_dependent,
        plottable: layer.is_plottable,
        color: LayerColor {
            aci: layer_aci(layer.color),
            rgb: layer_rgb(layer.color),
        },
        // An empty name means the source did not set one; `Continuous` is the
        // DXF default and the only name that is always defined, so it is a
        // substitution of the documented default, not a guess.
        linetype: if linetype.is_empty() {
            "Continuous".to_string()
        } else {
            linetype.to_string()
        },
        lineweight: layer_line_weight(layer.line_weight),
        opacity: layer_opacity(layer.transparency),
        plot_style: {
            let name = layer.plot_style.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        },
    }
}

/// Preserve an entity's source colour as its symbolic/explicit identity.
///
/// An explicit entity colour (`ByObject`: true colour or an ACI index) wins over
/// the layer and is stored as `Explicit`, byte-identically (no rounding, no
/// palette lookup beyond acadrust's own ACI table). `ByLayer` stays **symbolic**:
/// the representation resolves it against the owning layer's
/// `LayerAttributes::color.rgb` at draw time, so a layer colour edit recolours
/// the entity without any entity write. `ByBlock` is likewise kept symbolic so
/// INSERT expansion substitutes the containing reference's colour. `None` (no
/// colour) means ByLayer in DXF, so it is stored the same way rather than drawn
/// black.
pub(crate) fn resolve_entity_color(color: acadrust::Color) -> EntityColor {
    match concrete_rgb(color) {
        Some(rgb) => EntityColor::Explicit(rgb),
        None => match color {
            acadrust::Color::ByBlock => EntityColor::ByBlock,
            _ => EntityColor::ByLayer,
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

/// Preserve an entity's own lineweight provenance.
///
/// A concrete weight wins over the layer and is stored as `Explicit` in
/// millimetres (1/100 mm integers via acadrust's own conversion, so no scale
/// factor is invented here); acadrust's `Default` keeps its explicit "default"
/// meaning. `ByLayer` and `ByBlock` stay **symbolic**: the representation
/// resolves `ByLayer` from the owning layer's
/// [`LayerAttributes::lineweight`](cad_db::LayerAttributes::lineweight) and
/// `ByBlock` from the containing INSERT at draw time, so editing a layer
/// lineweight takes effect on the next expansion with no entity write. (Before
/// this change `ByLayer` was flattened into an `Explicit` copy of the layer's
/// millimetres right here, which made a later layer edit change nothing.)
pub(crate) fn resolve_entity_lineweight(weight: acadrust::LineWeight) -> EntityLineWeight {
    match weight {
        acadrust::LineWeight::Value(_) => {
            EntityLineWeight::Explicit(lineweight_mm(weight).unwrap_or(0.0))
        }
        acadrust::LineWeight::Default => EntityLineWeight::Default,
        acadrust::LineWeight::ByBlock => EntityLineWeight::ByBlock,
        acadrust::LineWeight::ByLayer => EntityLineWeight::ByLayer,
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

/// Preserve an entity's linetype provenance.
///
/// Precedence mirrors colour/lineweight, but nothing about the owning layer is
/// baked in here:
/// * `ByBlock` stays symbolic for INSERT expansion;
/// * the standard solid linetype is known and continuous, so it is recorded
///   explicitly rather than marked unresolved;
/// * `ByLayer` (and an absent name, which DXF treats the same way) stays
///   **symbolic**: the representation looks the owning layer's linetype *name*
///   up in the drawing's linetype table on every expansion, so a layer linetype
///   edit changes the entity's dashes with no entity write. (Before this change
///   the layer's pattern was substituted here as an `Explicit` copy, which froze
///   it at import time.) A name the table does not define is still reported by
///   the caller and resolves to continuous **and** unresolved at draw time;
/// * an explicit named linetype is `Explicit` with the entity's own `scale`
///   (sanitised to a positive finite value), byte-identical to the source
///   pattern;
/// * an unknown explicit name becomes an explicit **continuous** pattern
///   carrying the source name — never invented dashes.
///
/// A symbolic entity linetype carries no per-entity linetype scale: `cad_db`
/// has no slot for one beside `ByLayer`/`ByBlock` (the same as the existing
/// `ByBlock` path), so the source's group `48` is only preserved for `Explicit`.
pub(crate) fn resolve_entity_linetype(
    name: &str,
    scale: f64,
    named_pattern: Option<LinetypePattern>,
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
        return EntityLineType::ByLayer;
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

/// Best-effort `style name -> font file` from a DXF STYLE table's XDATA.
///
/// QCAD writes the real TrueType face under the STYLE record's `Acad` XDATA
/// (`1001 ACAD` / `1000 Arial`) and leaves group 3 empty. acadrust 0.6.3 types
/// that face into `TextStyle.true_type_font`, so this scan is only the last
/// fallback in the resolution chain
/// `typed true_type_font > font_file (group 3/4) > dxf_style_xdata_fonts byte scan`.
/// The never-override rule below is scoped to this scan's own output: within it,
/// a declared group 3 font wins over the XDATA face. It returns nothing for
/// non-DXF bytes.
pub(crate) fn dxf_style_xdata_fonts(bytes: &[u8]) -> HashMap<String, String> {
    // Cheap DXF sniff: the first two non-empty ASCII pair lines are `0`/`SECTION`
    // (allow the common leading spaces and CRLF). Binary DXF is skipped.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(64)]);
    let mut lines = head.lines();
    let ascii = matches!(
        (lines.next(), lines.next()),
        (Some(code), Some(value)) if code.trim() == "0" && value.trim() == "SECTION"
    );
    if !ascii {
        return HashMap::new();
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024 * 1024)]);
    let mut lines = text.lines();
    let mut pairs: Vec<(String, String)> = Vec::new();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        pairs.push((code.trim().to_string(), value.trim().to_string()));
    }
    let mut fonts: HashMap<String, String> = HashMap::new();
    let mut in_tables = false;
    let mut in_style = false;
    let mut name = String::new();
    let mut declared = String::new();
    let mut in_acad = false;
    let mut xdata = String::new();
    for (code, value) in &pairs {
        if code == "0" {
            if in_style {
                let font = if declared.is_empty() {
                    xdata.as_str()
                } else {
                    declared.as_str()
                };
                if !name.is_empty() && !font.is_empty() {
                    fonts.insert(name.to_ascii_lowercase(), font.to_string());
                }
            }
            if value == "SECTION" || value == "ENDSEC" {
                in_tables = false;
            }
            in_style = in_tables && value == "STYLE";
            name.clear();
            declared.clear();
            xdata.clear();
            in_acad = false;
            continue;
        }
        if !in_style {
            if code == "2" && value == "TABLES" {
                in_tables = true;
            }
            continue;
        }
        match code.as_str() {
            "2" => name = value.clone(),
            "3" if declared.is_empty() => declared = value.clone(),
            "1001" => in_acad = value.eq_ignore_ascii_case("ACAD"),
            "1000" if in_acad => {
                xdata = value.clone();
                in_acad = false;
            }
            _ => {}
        }
    }
    fonts
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(style_block: &str) -> Vec<u8> {
        format!(
            "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n{style_block}\n0\nENDTAB\n0\nENDSEC\n0\nEOF\n"
        )
        .into_bytes()
    }

    #[test]
    fn dxf_style_xdata_font_is_recovered_when_group_3_is_empty() {
        let bytes = dxf("0\nSTYLE\n2\ntextstyle0\n3\n\n1001\nACAD\n1000\nArial\n1071\n0");
        let fonts = dxf_style_xdata_fonts(&bytes);
        assert_eq!(fonts.get("textstyle0").map(String::as_str), Some("Arial"));
    }

    #[test]
    fn declared_group_3_wins_over_xdata_and_non_dxf_is_empty() {
        // Scoped to this scan's own output: the byte scan never overrides a
        // declared group 3. The overall resolution chain is
        // `typed true_type_font > font_file (group 3/4) > this scan`, so a typed
        // face would still win in `read_styles` (see tests.rs).
        let bytes = dxf("0\nSTYLE\n2\ns\n3\ntxt.shx\n1001\nACAD\n1000\nArial");
        let fonts = dxf_style_xdata_fonts(&bytes);
        assert_eq!(fonts.get("s").map(String::as_str), Some("txt.shx"));
        assert!(dxf_style_xdata_fonts(b"not a dxf").is_empty());
    }
}
