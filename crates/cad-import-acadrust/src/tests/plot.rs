//! Plot settings resolution and plot-style sheets.

use super::*;
// ---- plot settings (docs/plot.md) ----------------------------------------

/// Path to the committed synthetic plot fixture.
fn plot_fixture_bytes() -> Vec<u8> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn plot_fixture_imports_its_standalone_plot_settings() {
    // The fixture carries both embedded layout plot data (A4) and a standalone
    // PLOTSETTINGS object (A3, 90°). The object is the preferred source, so the
    // imported values must be the A3 ones, proven by the margins and rotation.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(plot_fixture_bytes()), &|| false)
        .expect("fixture imports");
    let layout = drawing
        .database
        .layouts()
        .next()
        .expect("fixture has a paper layout");
    let record = drawing
        .database
        .plot_settings(layout.id)
        .expect("standalone PLOTSETTINGS must be imported");
    assert_eq!(record.paper_size_name, "ISO_A3_(297.00_x_420.00_MM)");
    assert_eq!(record.paper_width, 297.0);
    assert_eq!(record.paper_height, 420.0);
    assert_eq!(record.rotation, cad_db::PlotRotation::Degrees90);
    assert_eq!(record.margins.left, 12.0);
    assert!(matches!(
        record.provenance,
        cad_db::PlotProvenance::Imported
    ));
    // The synthetic fixture has no model geometry; it must not claim Complete
    // render support for content it does not have. The import itself is valid.
    assert!(drawing.database.layout(layout.id).is_some());
}

#[test]
fn plot_fixture_without_plot_data_resolves_to_an_explicit_default() {
    // A layout id with no stored record must produce the documented A4 default,
    // never zero/unknown dimensions.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(plot_fixture_bytes()), &|| false)
        .expect("fixture imports");
    let fallback = drawing.database.plot_settings_for(LayoutId(999));
    assert!(matches!(
        fallback.provenance,
        cad_db::PlotProvenance::DefaultPage { .. }
    ));
    assert_eq!(fallback.paper_width, 210.0);
    assert_eq!(fallback.paper_height, 297.0);
}

/// A writer-produced DWG whose default paper-space layout references the given
/// plot style table. An empty `style_sheet` is the default (no table).
fn synthetic_dwg_with_plot_style(style_sheet: &str) -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    let layout_handle = doc
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            acadrust::objects::ObjectType::Layout(l) if l.name == "Layout1" => Some(*handle),
            _ => None,
        })
        .expect("Layout1 exists in a new document");
    if let Some(acadrust::objects::ObjectType::Layout(layout)) = doc.objects.get_mut(&layout_handle)
    {
        layout.paper_size = "ISO_A4_(210.00_x_297.00_MM)".into();
        layout.paper_width = 210.0;
        layout.paper_height = 297.0;
        layout.plot_paper_units = 1;
        layout.plot_style_sheet = style_sheet.into();
    }
    acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG")
}

#[test]
fn a_referenced_plot_style_sheet_is_reported_unsupported() {
    // Rule 2: a drawing carrying a plot style sheet must not look imported
    // clean. The reader now surfaces the reference as an explicit finding.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(
            &request(synthetic_dwg_with_plot_style("monochrome.ctb")),
            &|| false,
        )
        .expect("synthetic drawing imports");
    let finding = drawing
        .report
        .diagnostics
        .iter()
        .find(|d| d.code == "import.plot_style_unsupported")
        .expect("plot style sheet must yield an explicit Unsupported finding");
    assert!(finding.message.contains("monochrome.ctb"));
    // The finding is a real import item, so completeness is not `Complete`.
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_layout_without_a_plot_style_sheet_is_not_reported_unsupported() {
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(synthetic_dwg_with_plot_style("")), &|| false)
        .expect("synthetic drawing imports");
    assert!(
        !drawing
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "import.plot_style_unsupported"),
        "an empty/default style sheet must not be reported unsupported"
    );
}
