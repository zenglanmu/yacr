//! `$PDMODE` / `$PDSIZE` provenance: the source header value reaches the
//! database, and nothing else happens at import time.
//!
//! The style is display-only (it changes the `cad-app` overlay marker, not the
//! point's geometry), so the importer's whole job is to carry both values
//! verbatim. Judging a mode this project cannot draw is the overlay's job, and it
//! reports it once when the markers are built — an import must not look lossy
//! because of a display nicety.

use super::*;

/// A writer-produced DWG declaring the given point display style.
fn synthetic_dwg_with_point_style(mode: i16, size: f64) -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    doc.header.point_display_mode = mode;
    doc.header.point_display_size = size;
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    line.end = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    doc.add_entity(EntityType::Line(line)).expect("add line");
    acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG")
}

#[test]
fn a_declared_point_display_style_reaches_the_database_verbatim() {
    let importer = AcadrustImporter::new();
    // Mode 35 = cross (3) + a requested circle (32); -20.0 = 20% of the viewport
    // height. Both are stored raw: the signs and bits are the display layer's
    // contract.
    let drawing = importer
        .import(&request(synthetic_dwg_with_point_style(35, -20.0)), &|| {
            false
        })
        .expect("synthetic drawing imports");
    assert_eq!(drawing.database.point_display_mode(), 35);
    assert_eq!(drawing.database.point_display_size(), -20.0);

    // No import diagnostic, and the report stays complete for this: an
    // unimplemented marker shape is not a lost entity.
    assert!(
        !drawing
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("import.point")),
        "the style must not be judged at import time: {:?}",
        drawing.report.diagnostics
    );
}

#[test]
fn an_absent_point_display_style_is_the_dwg_default() {
    // A fresh document declares neither variable; acadrust materialises the DWG
    // defaults (PDMODE 0 dot, PDSIZE 0.0 viewport-relative) and the database
    // carries exactly that.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(synthetic_dwg(1)), &|| false)
        .expect("synthetic drawing imports");
    assert_eq!(drawing.database.point_display_mode(), 0);
    assert_eq!(drawing.database.point_display_size(), 0.0);
}
