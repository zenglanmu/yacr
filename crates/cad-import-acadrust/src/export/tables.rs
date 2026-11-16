//! Layer, text-style, linetype and dimension-style tables for the DXF export.
//!
//! Entities reference layers/styles/linetypes/dim styles by name, so every table
//! entry an exported entity can name must exist before the entities are added.
//! Missing text-style names are substituted with `Standard` (with a diagnostic),
//! never written as a dangling reference.
//!
//! # DIMSTYLE rows: the table pass writes the row, the entity pass the snapshot
//!
//! Every stored [`cad_db::DimStyle`] row is written here, so a row created by
//! `DIMSTYLE NEW` (or edited by `DIMSTYLE MODIFY`) survives SAVE → OPEN even
//! when no dimension references it yet. A dimension then writes (replaces) the
//! record for the style **it names** from its own resolved snapshot
//! ([`super::dimension`]): DXF has exactly one DIMSTYLE record per name, so a
//! non-retroactive table edit cannot be represented separately from the entity
//! that still carries the old values. That conflict is reported, never silent.

use acadrust::tables::{
    DimStyle as DxfDimStyle, Layer as DxfLayer, LineType as DxfLineType, LineTypeElement, TextStyle,
};
use acadrust::{CadDocument, Color as DxfColor, LineWeight as DxfLineWeight, Transparency};
use cad_db::{
    DimStyle as DbDimStyle, DrawingDatabase, LayerAttributes, LayerLineWeight,
    LineType as DbLineType,
};
use cad_domain::Unit;

use super::report::ExportReport;
use crate::style::is_continuous_linetype;

/// DXF `$INSUNITS` code for a source unit, when representable.
///
/// `None` for a unit this build cannot represent (or `DrawingUnits`), so the
/// caller reports the fallback rather than inventing a code.
pub(crate) fn unit_insunits(unit: &Unit) -> Option<i16> {
    match unit {
        Unit::Inch => Some(1),
        Unit::Foot => Some(2),
        Unit::Millimeter => Some(4),
        Unit::Meter => Some(6),
        Unit::DrawingUnits => None,
    }
}

/// Create the layer and linetype tables from the database.
///
/// Text styles are created lazily by [`ensure_text_style`] as text entities are
/// mapped: the DXF writer serialises the whole TABLES section independently of
/// when an entry was added, so the style exists before any entity that names it
/// is written.
pub(crate) fn write_tables(
    doc: &mut CadDocument,
    database: &DrawingDatabase,
    report: &mut ExportReport,
) {
    // Layers. The document already defines layer "0"; add_or_replace keeps a
    // database layer named "0" from erroring as a duplicate.
    for layer in database.layers() {
        let attributes = database.layer_attributes(layer.id);
        let mut entry = DxfLayer::new(layer.name.clone());
        write_layer_attributes(&mut entry, &layer.name, &attributes, database, report);
        doc.layers.add_or_replace(entry);
    }

    // Linetypes, so an explicit entity linetype name is never dangling.
    for linetype in database.linetypes() {
        doc.line_types.add_or_replace(linetype_entry(linetype));
    }

    // Dimension styles. Entities are emitted after this pass, and a dimension
    // replaces its own style's record with the entity's baked snapshot, so an
    // unreferenced row (a `DIMSTYLE NEW` row, or one whose only dimension was
    // deleted) is written exactly as stored.
    for style in database.dim_styles() {
        let font = database
            .style_by_name(style.text_style_name.trim())
            .and_then(|text| text.resource_keys.first().map(String::as_str));
        let (text_style, reasons) = ensure_text_style(doc, &style.text_style_name, font);
        for reason in reasons {
            report.notes.push(super::report::ExportDiagnostic {
                object: None,
                code: "export.dimstyle_text_style".into(),
                message: format!(
                    "dimension style '{}': {reason}; group 340 names '{text_style}'",
                    style.name
                ),
            });
        }
        let mut entry = dimstyle_entry(style);
        entry.dimtxsty = text_style;
        doc.dim_styles.add_or_replace(entry);
    }
}

/// One DXF DIMSTYLE record from a stored table row, field for field.
///
/// `DIMSCALE` is written as the row's own scale and the distance variables
/// unscaled, so [`cad_db::DimStyle::resolved`] reproduces the row exactly on
/// re-import (unlike the entity pass, which writes the already-scaled snapshot
/// with `DIMSCALE = 1`).
///
/// Two declared drops: `DIMCONTINUEMODE` is drawing-level and has no acadrust
/// `DimStyle` field (ADR 0017 B.2(b)), and the acadrust record carries no field
/// for a true-colour override of `DIMCLRD`/`DIMCLRE`/`DIMCLRT`, so the ACI
/// values are written and an RGB override cannot round-trip.
fn dimstyle_entry(style: &DbDimStyle) -> DxfDimStyle {
    let mut entry = DxfDimStyle::new(style.name.trim());
    // B.2(a): display-driving values, verbatim.
    entry.dimscale = style.scale;
    entry.dimtxt = style.text_height;
    entry.dimasz = style.arrow_size;
    entry.dimexo = style.ext_offset;
    entry.dimexe = style.ext_beyond;
    entry.dimgap = style.text_gap;
    entry.dimtad = style.text_vertical;
    entry.dimtvp = style.text_vertical_position;
    entry.dimdec = style.decimals;
    entry.dimzin = style.zero_suppression;
    entry.dimlfac = style.linear_factor;
    entry.dimrnd = style.rounding;
    entry.dimpost = style.postfix.clone();
    entry.dimcen = style.center_mark;
    entry.dimtxsty = style.text_style_name.trim().to_string();
    entry.dimse1 = style.suppress_ext1;
    entry.dimse2 = style.suppress_ext2;
    entry.dimsd1 = style.suppress_dim1;
    entry.dimsd2 = style.suppress_dim2;
    // B.2(b): snapshot-only values, written for table fidelity.
    entry.dimlunit = style.linear_unit_format;
    entry.dimdsep = style.decimal_separator;
    entry.dimjust = style.text_justification;
    entry.dimtih = style.text_inside_horizontal;
    entry.dimtoh = style.text_outside_horizontal;
    entry.dimatfit = style.text_fit;
    entry.dimtix = style.text_inside_extensions;
    entry.dimtofl = style.text_outside_flow_line;
    entry.dimclrd = style.dim_line_color;
    entry.dimclre = style.ext_line_color;
    entry.dimclrt = style.text_color;
    entry.dimdle = style.dim_line_extension;
    entry.dimdli = style.dim_line_increment;
    entry
}

/// Copy the stored attribute row onto the DXF layer entry.
///
/// # The off bit versus `!visible`
///
/// The pre-attribute exporter wrote `flags.off = !layer.visible`, which
/// *conflated* a frozen layer with an off one (both made `visible == false`).
/// The stored row lets the two be written distinctly: `flags.off` now comes from
/// [`LayerAttributes::off`] and the frozen bits from their own fields.
///
/// Because the importer defines `visible == !(off || frozen)`, re-importing the
/// output still yields exactly the same `visible` value as before, so no
/// drawing's **effective visibility** changes. What does change is that a frozen
/// layer now round-trips as frozen instead of being rewritten as off — a
/// previously-lost attribute, which is the point of this phase.
fn write_layer_attributes(
    entry: &mut DxfLayer,
    layer_name: &str,
    attributes: &LayerAttributes,
    database: &DrawingDatabase,
    report: &mut ExportReport,
) {
    entry.flags.off = attributes.off;
    entry.flags.frozen = attributes.frozen;
    entry.flags.frozen_in_new_viewport = attributes.frozen_in_new_viewport;
    entry.flags.locked = attributes.locked;
    entry.flags.xref_dependent = attributes.xref_dependent;
    entry.is_plottable = attributes.plottable;

    // ACI identity when the source had one (group 62), otherwise the resolved
    // sRGB value (group 420). Never both invented: a true-colour source keeps
    // its RGB and reports no index.
    entry.color = match attributes.color.aci {
        Some(index) => DxfColor::Index(index),
        None => {
            let [r, g, b] = attributes.color.rgb;
            DxfColor::Rgb { r, g, b }
        }
    };

    entry.line_type = exportable_linetype(&attributes.linetype, database, layer_name, report);

    entry.line_weight = match attributes.lineweight {
        LayerLineWeight::Hundredths(value) => DxfLineWeight::Value(value as i16),
        LayerLineWeight::Default => DxfLineWeight::Default,
        LayerLineWeight::ByLayer => DxfLineWeight::ByLayer,
    };

    // The source stores transparency with `0` meaning opaque; `opacity` is its
    // complement, so the DWG byte scale is restored by inverting it. Fully
    // opaque maps to `Explicit(0)`, which the writer omits entirely, so an
    // opaque layer still writes no XDATA.
    let alpha = ((1.0 - attributes.opacity.clamp(0.0, 1.0)) * 255.0).round() as u8;
    entry.transparency = Transparency::Explicit(alpha);

    // A plot-style **name** cannot survive export: both writers emit the
    // plot-style *handle* (group 390), never the name. The entry therefore keeps
    // the writer's default handle and the dropped name is reported.
    if let Some(name) = attributes.plot_style.as_deref() {
        report.notes.push(super::report::ExportDiagnostic {
            object: None,
            code: "export.layer_plot_style_dropped".into(),
            message: format!(
                "layer '{layer_name}' plot style '{name}' cannot be written (the DXF writer \
                 emits a plot-style handle, not a name); the default plot style is used"
            ),
        });
    }
}

/// The linetype name to write, never a dangling reference.
///
/// `Continuous` is the DXF-implied solid linetype and is always defined, so it
/// is always writable. Any other name is written only when the database's
/// linetype table carries it (case-insensitively, as AutoCAD names are); an
/// unresolvable name is substituted with `Continuous` and reported, because a
/// layer naming a linetype the file does not define is a broken reference.
fn exportable_linetype(
    name: &str,
    database: &DrawingDatabase,
    layer_name: &str,
    report: &mut ExportReport,
) -> String {
    let name = name.trim();
    if name.is_empty() || is_continuous_linetype(name) {
        return "Continuous".to_string();
    }
    let known = database
        .linetypes()
        .any(|linetype| linetype.name.trim().eq_ignore_ascii_case(name));
    if known {
        return name.to_string();
    }
    report.notes.push(super::report::ExportDiagnostic {
        object: None,
        code: "export.layer_linetype_substituted".into(),
        message: format!(
            "layer '{layer_name}' names linetype '{name}', which this drawing's linetype table \
             does not define; written as 'Continuous'"
        ),
    });
    "Continuous".to_string()
}

/// Ensure a text style named `name` exists, returning the name to reference and
/// any conversion reasons.
///
/// A missing or empty name is substituted with `Standard`; a later different
/// font key on an already-defined style is ignored. Both are returned as
/// reasons so the caller can fold them into that one entity's `Converted`
/// outcome (never counted twice, never reported as `Exact`).
pub(crate) fn ensure_text_style(
    doc: &mut CadDocument,
    name: &str,
    font_key: Option<&str>,
) -> (String, Vec<String>) {
    let name = name.trim();
    if name.is_empty() {
        return (
            "Standard".to_string(),
            vec!["text has no style name; written with 'Standard'".to_string()],
        );
    }
    if let Some(existing) = doc.text_styles.get_mut(name) {
        // The resource key is a logical font key, not a platform path. First
        // writer wins; a later different key is reported, not silently ignored.
        if existing.font_file.is_empty() {
            if let Some(key) = font_key {
                existing.font_file = key.to_string();
            }
            return (name.to_string(), Vec::new());
        }
        if let Some(key) = font_key {
            if existing.font_file != key {
                return (
                    name.to_string(),
                    vec![format!(
                        "text style '{name}' already maps font '{}'; this text's font '{key}' is \
                         ignored",
                        existing.font_file
                    )],
                );
            }
        }
        return (name.to_string(), Vec::new());
    }
    let mut entry = TextStyle::new(name);
    if let Some(key) = font_key {
        entry.font_file = key.to_string();
    }
    doc.text_styles.add_or_replace(entry);
    (name.to_string(), Vec::new())
}

fn linetype_entry(linetype: &DbLineType) -> DxfLineType {
    let mut entry = DxfLineType::new(linetype.name.clone());
    entry.elements = linetype
        .pattern
        .elements
        .iter()
        .map(|length| LineTypeElement {
            length: *length,
            complex: None,
        })
        .collect();
    entry.pattern_length = linetype.pattern.cycle;
    entry
}
