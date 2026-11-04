//! Import-side format/resource reporting contracts.
//!
//! Covers the three surfaces added together:
//!
//! * the DWG read resource budgets threaded through [`ImportLimits`];
//! * source-format fidelity notes (R11/R12 family, approximate code page) whose
//!   codes must stay outside the `import.` namespace;
//! * decoded object-level EED (XDATA), summarised per application and reported
//!   only, never applied.
//!
//! The tests build an in-memory `CadDocument` and run `ImporterBuilder` directly,
//! like `layout_order.rs`: the contract here is the builder's reporting, not the
//! writer/parser round trip.

use super::*;

fn import(doc: &acadrust::CadDocument) -> ImportedDrawing {
    let req = request(Vec::new());
    let importer = ImporterBuilder::new(&req, doc, ReadStats::default(), compute_identity(&[]));
    importer.run().expect("in-memory document imports")
}

fn diagnostic_codes(drawing: &ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn import_limits_default_leaves_dwg_budgets_unset() {
    let limits = ImportLimits::default();
    // `None` is acadrust's own behaviour; a default that invented a budget
    // would silently change what an unconfigured host accepts.
    assert_eq!(limits.max_mesh_faces, None);
    assert_eq!(limits.max_mesh_face_indices, None);
    assert_eq!(limits.max_decompressed_bytes, None);
}

#[test]
fn dwg_read_options_copies_import_limits() {
    let limits = ImportLimits {
        max_mesh_faces: Some(11),
        max_mesh_face_indices: Some(22),
        max_decompressed_bytes: Some(33),
        ..ImportLimits::default()
    };
    let options = dwg_read_options(&limits);
    assert!(options.failsafe, "the importer always reads failsafe");
    assert_eq!(options.max_mesh_faces, Some(11));
    assert_eq!(options.max_mesh_face_indices, Some(22));
    assert_eq!(options.max_decompressed_bytes, Some(33));

    // Defaults stay defaults: no invented budget.
    let default_options = dwg_read_options(&ImportLimits::default());
    assert!(default_options.failsafe);
    assert_eq!(default_options.max_mesh_faces, None);
    assert_eq!(default_options.max_mesh_face_indices, None);
    assert_eq!(default_options.max_decompressed_bytes, None);
}

#[test]
fn r11_r12_source_is_reported_without_import_loss_prefix() {
    // Baseline: the same (empty) document at the default version. An in-memory
    // build legitimately emits `import.annotation_scale_unknown` and
    // `import.incomplete_stream` here, so the contract is *equality* with this
    // baseline, not the absence of `import.` codes.
    let baseline = import(&acadrust::CadDocument::new());

    let mut doc = acadrust::CadDocument::new();
    doc.version = acadrust::DxfVersion::AC1009;
    let drawing = import(&doc);

    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&"format.r11_r12"),
        "R11/R12 family is reported: {codes:?}"
    );
    let format_note = drawing
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "format.r11_r12")
        .expect("format.r11_r12 diagnostic exists");
    assert!(
        !format_note.code.starts_with("import."),
        "a fidelity note must not be turned into an import loss"
    );
    // `run` folds every `import.` code into `Completeness::Partial`; the note
    // must not add a reason, so the verdict matches the baseline exactly.
    assert_eq!(
        drawing.report.completeness, baseline.report.completeness,
        "the R11/R12 note must not change completeness: {codes:?}"
    );
}

#[test]
fn unfaithful_code_page_is_reported_as_approximate() {
    let mut doc = acadrust::CadDocument::new();
    // AC1255 is the documented real exception: WHATWG windows-1255 differs.
    doc.header.code_page = "ANSI_1255".to_string();
    let drawing = import(&doc);

    let diagnostic = drawing
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "format.code_page_approximate")
        .unwrap_or_else(|| {
            panic!(
                "approximate code page is reported: {:?}",
                diagnostic_codes(&drawing)
            )
        });
    assert!(
        diagnostic.message.contains("ANSI_1255"),
        "message names the code page: {}",
        diagnostic.message
    );
    assert!(
        !diagnostic.code.starts_with("import."),
        "a fidelity note must not be turned into an import loss"
    );
}

fn extended_data(applications: &[&str]) -> acadrust::xdata::ExtendedData {
    let mut data = acadrust::xdata::ExtendedData::new();
    for application in applications {
        data.upsert_record(acadrust::xdata::ExtendedDataRecord::new(*application));
    }
    data
}

#[test]
fn object_xdata_is_summarised_per_application() {
    let mut doc = acadrust::CadDocument::new();
    // Object 1 carries both applications, plus a malformed duplicate `A`
    // record: the summary counts distinct names per object, not records.
    let mut both = extended_data(&["B", "A"]);
    both.add_record(acadrust::xdata::ExtendedDataRecord::new("A"));
    doc.object_xdata.insert(acadrust::Handle::new(1), both);
    // Object 2 carries only `A`.
    doc.object_xdata
        .insert(acadrust::Handle::new(2), extended_data(&["A"]));

    let drawing = import(&doc);
    assert_eq!(
        drawing.report.object_xdata,
        vec![
            ObjectXdataApplication {
                application: "A".to_string(),
                objects: 2,
            },
            ObjectXdataApplication {
                application: "B".to_string(),
                objects: 1,
            },
        ],
        "objects per application, sorted by application name"
    );
    assert!(
        diagnostic_codes(&drawing).contains(&"xdata.object_ignored"),
        "ignored xdata is reported once: {:?}",
        diagnostic_codes(&drawing)
    );

    // An empty document yields an empty summary (and no xdata diagnostic).
    let empty = import(&acadrust::CadDocument::new());
    assert!(empty.report.object_xdata.is_empty());
    assert!(!diagnostic_codes(&empty).contains(&"xdata.object_ignored"));
}
