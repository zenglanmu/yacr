//! Layer, text-style and linetype tables for the DXF export.
//!
//! Entities reference layers/styles/linetypes by name, so every table entry an
//! exported entity can name must exist before the entities are added. Missing
//! text-style names are substituted with `Standard` (with a diagnostic), never
//! written as a dangling reference.

use acadrust::tables::{Layer as DxfLayer, LineType as DxfLineType, LineTypeElement, TextStyle};
use acadrust::CadDocument;
use cad_db::{DrawingDatabase, LineType as DbLineType};
use cad_domain::Unit;

use super::report::ExportReport;

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
        let mut entry = DxfLayer::new(layer.name.clone());
        entry.flags.off = !layer.visible;
        doc.layers.add_or_replace(entry);
    }

    // Linetypes, so an explicit entity linetype name is never dangling.
    for linetype in database.linetypes() {
        doc.line_types.add_or_replace(linetype_entry(linetype));
    }

    let _ = report;
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
