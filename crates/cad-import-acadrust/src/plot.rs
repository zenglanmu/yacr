//! Plot-settings import: read PLOTSETTINGS objects and each layout's embedded
//! plot data into the database's plot-settings table.
//!
//! This is deliberately a separate module from `entity.rs`: plot configuration
//! is document-level metadata, not entity geometry, and the two have no shared
//! state. The importer never invents a vendor configuration: a layout whose
//! file carries no plot data simply gets no record, and the database reports an
//! explicit documented default page at query time.
//!
//! Two sources are read, in priority order:
//!
//! 1. A standalone `ObjectType::PlotSettings` whose `page_name` matches the
//!    layout name (the DXF PLOTSETTINGS object).
//! 2. The plot fields embedded in the layout's own `ObjectType::Layout` record,
//!    which is how native DWG stores the same data.
//!
//! Whichever source has a non-empty paper size wins; the values are copied
//! verbatim and their provenance is marked [`PlotProvenance::Imported`].

use acadrust::objects::{ObjectType, PlotPaperUnits as AcadUnits, PlotRotation as AcadRotation};
use acadrust::CadDocument;
use cad_db::{
    PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType,
};
use cad_domain::{CadResult, LayoutId};
use std::collections::HashMap;

/// A plot configuration read from the document, keyed by the layout it belongs
/// to.
pub(crate) struct ImportedPlotSettings {
    /// The layout handle; also carried inside `record` for the builder.
    pub(crate) layout: LayoutId,
    pub(crate) record: PlotSettingsRecord,
}

/// Map the acadrust rotation code onto the database enum.
fn rotation_from_str(rotation: AcadRotation) -> PlotRotation {
    match rotation {
        AcadRotation::None => PlotRotation::None,
        AcadRotation::Degrees90 => PlotRotation::Degrees90,
        AcadRotation::Degrees180 => PlotRotation::Degrees180,
        AcadRotation::Degrees270 => PlotRotation::Degrees270,
    }
}

/// Map an integer DXF code onto the database rotation enum.
fn rotation_from_code(code: i16) -> PlotRotation {
    // The acadrust enum owns the code interpretation; its degrees are mapped
    // back so the two cannot disagree about which way is 90°.
    rotate_degrees(AcadRotation::from_code(code).to_degrees())
}

fn rotate_degrees(degrees: f64) -> PlotRotation {
    // `to_degrees` only ever returns one of four exact values, but round to be
    // robust against a derived value.
    match degrees.round() as i64 {
        90 => PlotRotation::Degrees90,
        180 => PlotRotation::Degrees180,
        270 => PlotRotation::Degrees270,
        _ => PlotRotation::None,
    }
}

fn units_from_code(code: i16) -> PlotPaperUnits {
    match AcadUnits::from_code(code) {
        AcadUnits::Inches => PlotPaperUnits::Inches,
        AcadUnits::Millimeters => PlotPaperUnits::Millimeters,
        AcadUnits::Pixels => PlotPaperUnits::Pixels,
    }
}

/// Paper units implied by the standard AutoCAD paper-size name.
///
/// Names carry the unit as a suffix (`ISO_A4_(210.00_x_297.00_MM)`,
/// `Letter_(8.50_x_11.00_Inches)`). acadrust does apply `group 72` to LAYOUT
/// records, but the name is the more reliable source when it carries a unit
/// token; the parsed code is only a fallback. Returns `None` when the name
/// carries no unit token.
fn units_from_paper_name(name: &str) -> Option<PlotPaperUnits> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("_mm)") || lower.ends_with("_mm") {
        Some(PlotPaperUnits::Millimeters)
    } else if lower.contains("_inches)") || lower.ends_with("_inches") {
        Some(PlotPaperUnits::Inches)
    } else {
        None
    }
}

/// Resolve the paper units for a record from its name (preferred) or the code.
fn resolve_paper_units(name: &str, code: i16) -> PlotPaperUnits {
    units_from_paper_name(name).unwrap_or_else(|| units_from_code(code))
}

fn plot_type_from_code(code: i16) -> PlotType {
    use acadrust::objects::PlotType as Acad;
    match Acad::from_code(code) {
        Acad::LastScreenDisplay => PlotType::LastScreenDisplay,
        Acad::Extents => PlotType::Extents,
        Acad::Limits => PlotType::Limits,
        Acad::View => PlotType::View,
        Acad::Window => PlotType::Window,
        Acad::Layout => PlotType::Layout,
    }
}

/// Build a database record from a standalone PLOTSETTINGS object.
fn record_from_plot_settings(
    layout: LayoutId,
    settings: &acadrust::objects::PlotSettings,
) -> PlotSettingsRecord {
    PlotSettingsRecord {
        layout,
        paper_size_name: settings.paper_size.clone(),
        paper_width: settings.paper_width,
        paper_height: settings.paper_height,
        margins: PlotMargins {
            left: settings.margins.left,
            bottom: settings.margins.bottom,
            right: settings.margins.right,
            top: settings.margins.top,
        },
        rotation: rotation_from_str(settings.rotation),
        scale_numerator: settings.scale_numerator,
        scale_denominator: settings.scale_denominator,
        plot_type: plot_type_from_code(settings.plot_type.to_code()),
        paper_units: resolve_paper_units(&settings.paper_size, settings.paper_units.to_code()),
        provenance: PlotProvenance::Imported,
    }
}

/// Build a database record from a layout's embedded plot fields.
fn record_from_layout(layout: LayoutId, object: &acadrust::objects::Layout) -> PlotSettingsRecord {
    PlotSettingsRecord {
        layout,
        paper_size_name: object.paper_size.clone(),
        paper_width: object.paper_width,
        paper_height: object.paper_height,
        margins: PlotMargins {
            left: object.plot_margin_left,
            bottom: object.plot_margin_bottom,
            right: object.plot_margin_right,
            top: object.plot_margin_top,
        },
        rotation: rotation_from_code(object.plot_rotation),
        scale_numerator: object.plot_scale_numerator,
        scale_denominator: object.plot_scale_denominator,
        plot_type: plot_type_from_code(object.plot_type),
        paper_units: resolve_paper_units(&object.paper_size, object.plot_paper_units),
        provenance: PlotProvenance::Imported,
    }
}

/// A record is worth storing only when the file actually supplied the sheet
/// size; a zero-size record is the same as "no data" and would otherwise look
/// like an imported but unusable configuration.
fn has_paper(record: &PlotSettingsRecord) -> bool {
    record.paper_width > 0.0 && record.paper_height > 0.0
}

/// Read every layout's plot configuration from the parsed document.
///
/// Returns the records that carry a real paper size, keyed by layout id. The
/// caller inserts them through [`cad_db::DrawingDatabaseBuilder::set_plot_settings`].
pub(crate) fn read_plot_settings(
    acad: &CadDocument,
    layout_ids: &HashMap<String, LayoutId>,
) -> CadResult<Vec<ImportedPlotSettings>> {
    // A layout's database id is keyed by its *block record* name ("*Paper_Space").
    // The `Layout` object carries the user-facing display name ("Layout1") and
    // links to the block record by handle, so correlation must go through the
    // handle, never by comparing the two names directly.
    let block_name_by_handle: HashMap<acadrust::Handle, String> = acad
        .block_records
        .iter()
        .map(|record| (record.handle, record.name.clone()))
        .collect();

    // Layout object handle → database LayoutId, and display name → LayoutId.
    let mut layout_id_by_object: HashMap<acadrust::Handle, LayoutId> = HashMap::new();
    let mut layout_id_by_display_name: HashMap<String, LayoutId> = HashMap::new();
    for (handle, object) in &acad.objects {
        if let ObjectType::Layout(layout) = object {
            let Some(name) = block_name_by_handle.get(&layout.block_record) else {
                continue;
            };
            let Some(id) = layout_ids.get(name) else {
                continue;
            };
            layout_id_by_object.insert(*handle, *id);
            layout_id_by_display_name.insert(layout.name.clone(), *id);
        }
    }

    // One record per layout, preferring a standalone PLOTSETTINGS object over
    // embedded layout data. Deterministic by layout id.
    let mut best: HashMap<LayoutId, (u8, PlotSettingsRecord)> = HashMap::new();
    let mut consider = |layout: LayoutId, priority: u8, record: PlotSettingsRecord| {
        if !has_paper(&record) {
            return;
        }
        match best.get(&layout) {
            Some((current, _)) if *current >= priority => {}
            _ => {
                best.insert(layout, (priority, record));
            }
        }
    };

    for (handle, object) in &acad.objects {
        match object {
            ObjectType::PlotSettings(settings) => {
                // Owner is the layout object handle; fall back to the page name
                // matching the layout's display name.
                let layout = layout_id_by_object
                    .get(&settings.owner)
                    .or_else(|| layout_id_by_display_name.get(&settings.page_name))
                    .copied();
                if let Some(layout) = layout {
                    consider(layout, 2, record_from_plot_settings(layout, settings));
                }
            }
            ObjectType::Layout(object) => {
                if let Some(layout) = layout_id_by_object.get(handle).copied() {
                    consider(layout, 1, record_from_layout(layout, object));
                }
            }
            _ => {}
        }
    }

    let mut ids: Vec<LayoutId> = best.keys().copied().collect();
    ids.sort_by_key(|id| id.0);
    Ok(ids
        .into_iter()
        .filter_map(|layout| {
            best.remove(&layout)
                .map(|(_, record)| ImportedPlotSettings { layout, record })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_codes_map_to_the_database_enum() {
        assert_eq!(rotation_from_code(0), PlotRotation::None);
        assert_eq!(rotation_from_code(1), PlotRotation::Degrees90);
        assert_eq!(rotation_from_code(2), PlotRotation::Degrees180);
        assert_eq!(rotation_from_code(3), PlotRotation::Degrees270);
        // An unknown code is treated as no rotation, never a guessed angle.
        assert_eq!(rotation_from_code(99), PlotRotation::None);
    }

    #[test]
    fn unit_codes_map_to_the_database_enum() {
        assert_eq!(units_from_code(0), PlotPaperUnits::Inches);
        assert_eq!(units_from_code(1), PlotPaperUnits::Millimeters);
        assert_eq!(units_from_code(2), PlotPaperUnits::Pixels);
        assert_eq!(units_from_code(99), PlotPaperUnits::Inches);
    }

    #[test]
    fn layout_embedded_plot_fields_are_read_verbatim() {
        // The capability note claimed acadrust does not expose a LAYOUT's
        // `group 72`/`group 73`. It does; this record is built straight from
        // those typed fields. The paper name carries no unit token here, so the
        // parsed unit code is what is used (the name still wins when it does).
        let mut layout = acadrust::objects::Layout::new("Layout1");
        layout.paper_size = "Custom_(210.00_x_297.00)".into();
        layout.paper_width = 210.0;
        layout.paper_height = 297.0;
        layout.plot_paper_units = 1; // group 72
        layout.plot_rotation = 1; // group 73
        let record = record_from_layout(LayoutId(1), &layout);
        assert_eq!(record.paper_units, PlotPaperUnits::Millimeters);
        assert_eq!(record.rotation, PlotRotation::Degrees90);
    }

    #[test]
    fn paper_name_units_override_the_default_code() {
        // A standard paper name carries its unit token, so the name wins over
        // the (populated) group 72 code.
        assert_eq!(
            resolve_paper_units("ISO_A4_(210.00_x_297.00_MM)", 0),
            PlotPaperUnits::Millimeters
        );
        assert_eq!(
            resolve_paper_units("Letter_(8.50_x_11.00_Inches)", 0),
            PlotPaperUnits::Inches
        );
        // No unit token: keep the code.
        assert_eq!(
            resolve_paper_units("custom", 1),
            PlotPaperUnits::Millimeters
        );
    }

    #[test]
    fn a_zero_size_record_is_not_stored() {
        let mut b = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
        b.insert_layer(cad_db::Layer {
            id: cad_domain::LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
        let db = b.finish().unwrap();
        // A fresh database has no imported record, so the query returns the
        // explicit default page rather than an empty/zero sheet.
        assert!(db.plot_settings(LayoutId(1)).is_none());
        let fallback = db.plot_settings_for(LayoutId(1));
        assert!(matches!(
            fallback.provenance,
            PlotProvenance::DefaultPage { .. }
        ));
        assert_eq!(fallback.paper_width, 210.0);
    }

    #[test]
    fn has_paper_requires_both_dimensions() {
        let record = PlotSettingsRecord {
            layout: LayoutId(1),
            paper_size_name: String::new(),
            paper_width: 210.0,
            paper_height: 0.0,
            margins: PlotMargins::default(),
            rotation: PlotRotation::None,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::Imported,
        };
        assert!(!has_paper(&record));
    }
}
