//! Core import lifecycle: request plumbing, cancellation, corrupt input and progress.

use super::*;
#[test]
#[ignore = "one-off fixture generator; run with YACR_PLOT_FIXTURE_OUT"]
fn generate_plot_fixture() {
    use acadrust::io::dwg::dwg_writer::DwgWriter;
    let Some(out) = std::env::var_os("YACR_PLOT_FIXTURE_OUT") else {
        return;
    };
    let mut doc = acadrust::CadDocument::new();
    // A fresh document already has a "Layout1"; reuse it.
    let layout_handle = doc
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            acadrust::objects::ObjectType::Layout(l) if l.name == "Layout1" => Some(*handle),
            _ => None,
        })
        .expect("Layout1 exists in a new document");
    doc.add_entity_to_layout(
        acadrust::EntityType::Line(acadrust::entities::Line::from_coords(
            20.0, 20.0, 0.0, 190.0, 277.0, 0.0,
        )),
        "Layout1",
    )
    .unwrap();
    // Populate the layout's embedded plot data (A4 landscape-ish, 5mm margins).
    if let Some(acadrust::objects::ObjectType::Layout(layout)) = doc.objects.get_mut(&layout_handle)
    {
        layout.paper_size = "ISO_A4_(210.00_x_297.00_MM)".into();
        layout.paper_width = 210.0;
        layout.paper_height = 297.0;
        layout.plot_margin_left = 5.0;
        layout.plot_margin_bottom = 5.0;
        layout.plot_margin_right = 5.0;
        layout.plot_margin_top = 5.0;
        layout.plot_paper_units = 1;
        layout.plot_type = 5;
        layout.plot_rotation = 0;
        layout.plot_scale_numerator = 1.0;
        layout.plot_scale_denominator = 1.0;
    }
    // A standalone PLOTSETTINGS object too (the DXF-style path), with values
    // that differ from the embedded ones so the preferred source is observable.
    let settings_handle = doc.allocate_handle();
    let mut settings = acadrust::objects::PlotSettings::new("Layout1");
    settings.handle = settings_handle;
    settings.owner = layout_handle;
    settings.paper_size = "ISO_A3_(297.00_x_420.00_MM)".into();
    settings.paper_width = 297.0;
    settings.paper_height = 420.0;
    settings.margins = acadrust::objects::PaperMargin::new(12.0, 12.0, 12.0, 12.0);
    settings.rotation = acadrust::objects::PlotRotation::Degrees90;
    settings.paper_units = acadrust::objects::PlotPaperUnits::Millimeters;
    settings.plot_type = acadrust::objects::PlotType::Layout;
    settings.scale_numerator = 1.0;
    settings.scale_denominator = 1.0;
    doc.objects.insert(
        settings_handle,
        acadrust::objects::ObjectType::PlotSettings(settings),
    );
    let bytes = DwgWriter::write_to_vec(&doc).unwrap();
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {} bytes to {:?}", bytes.len(), out);
}

#[test]
fn garbage_input_fails_without_panicking() {
    let importer = AcadrustImporter::new();
    let result = importer.import(&request(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]), &|| {
        false
    });
    assert!(result.is_err(), "random bytes must not import as a drawing");
}

#[test]
fn oversize_input_is_rejected_before_parsing() {
    let importer = AcadrustImporter::new();
    let mut req = request(vec![0u8; 64]);
    req.limits.max_file_bytes = 8;
    assert!(matches!(
        importer.import(&req, &|| false),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn cancellation_is_honoured() {
    let importer = AcadrustImporter::new();
    let result = importer.import(&request(vec![0u8; 64]), &|| true);
    assert!(matches!(result, Err(CadError::Cancelled)));
}
#[test]
fn progress_reports_real_phases_in_order() {
    use std::sync::Mutex;
    let importer = AcadrustImporter::new();
    let ticks = Mutex::new(Vec::new());
    let drawing = importer
        .import_with_progress(
            &request(synthetic_dwg(3)),
            &|| false,
            &|p: ImportProgress| ticks.lock().unwrap().push(p),
        )
        .expect("synthetic drawing imports");
    assert_eq!(drawing.database.entity_count(), 3);

    let ticks = ticks.into_inner().unwrap();
    let phases: Vec<ImportPhase> = ticks.iter().map(|p| p.phase).collect();
    // Read start/end, table build, entity batches, resolve, finish — in order.
    assert_eq!(
        phases.first(),
        Some(&ImportPhase::Reading),
        "read start is the first reported boundary"
    );
    assert!(phases.contains(&ImportPhase::Parsing));
    assert!(phases.contains(&ImportPhase::Tables));
    assert!(phases.contains(&ImportPhase::Entities));
    assert!(phases.contains(&ImportPhase::Resolving));
    assert_eq!(
        phases.last(),
        Some(&ImportPhase::Finishing),
        "finish is the last boundary"
    );
    let first_index = |phase: ImportPhase| phases.iter().position(|p| *p == phase).unwrap();
    assert!(first_index(ImportPhase::Reading) < first_index(ImportPhase::Parsing));
    assert!(first_index(ImportPhase::Parsing) < first_index(ImportPhase::Tables));
    assert!(first_index(ImportPhase::Tables) < first_index(ImportPhase::Entities));
    assert!(first_index(ImportPhase::Entities) < first_index(ImportPhase::Resolving));
    assert!(first_index(ImportPhase::Resolving) < first_index(ImportPhase::Finishing));

    // Entity counts only rise, and end at the real total.
    let mut last = 0usize;
    for p in ticks.iter().filter(|p| p.phase == ImportPhase::Entities) {
        assert!(p.entities_done >= last, "counts never go backwards");
        last = p.entities_done;
        if let Some(total) = p.entities_total {
            assert!(
                p.entities_done <= total,
                "done never exceeds the real total"
            );
        }
    }
    assert_eq!(last, 3);
    // The read-start byte count is the real request length, not a guess.
    let reading = ticks.first().unwrap();
    assert!(reading.bytes.unwrap_or(0) > 0);
    assert!(
        reading.entities_total.is_none(),
        "total unknown before parse"
    );
}

#[test]
fn cancellation_during_import_returns_cancelled_without_a_database() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Cancel as soon as the first entity batch is reported. The importer polls
    // cancellation at every batch boundary and must abort with `Cancelled`.
    let seen = AtomicUsize::new(0);
    let importer = AcadrustImporter::new();
    let cancelled = || seen.load(Ordering::SeqCst) > 0;
    let result = importer.import_with_progress(
        &request(synthetic_dwg(2000)),
        &cancelled,
        &|p: ImportProgress| {
            if p.phase == ImportPhase::Entities {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    assert!(
        matches!(result, Err(CadError::Cancelled)),
        "a cancelled import must not return a partial drawing"
    );
}

#[test]
fn malformed_dwg_is_corrupt_not_cancelled() {
    let importer = AcadrustImporter::new();
    // A valid signature followed by garbage is a truncated/corrupt stream: the
    // failsafe reader recovers an empty document, which must be surfaced as
    // corrupt data rather than published as an empty success.
    let mut bytes = b"AC1032".to_vec();
    bytes.extend_from_slice(&[0xAB; 512]);
    let result = importer.import_with_progress(&request(bytes), &|| false, &NoopProgress);
    match result {
        Err(CadError::CorruptData(_)) => {}
        Err(other) => panic!("expected CorruptData, got {other:?}"),
        Ok(_) => panic!("malformed DWG must not import successfully"),
    }
}

#[test]
fn truncated_synthetic_dwg_is_corrupt() {
    let importer = AcadrustImporter::new();
    let full = synthetic_dwg(2);
    // Cut the stream before its end so `stream_completed` is false.
    let truncated = full[..full.len() / 3].to_vec();
    let result = importer.import_with_progress(&request(truncated), &|| false, &NoopProgress);
    assert!(
        matches!(result, Err(CadError::CorruptData(_))),
        "a truncated DWG must not be published as an empty success"
    );
}

#[test]
fn space_block_names_are_case_insensitive() {
    for name in ["*Model_Space", "*MODEL_SPACE", "*model_space"] {
        assert!(is_space_block_name(name), "{name}");
    }
    for name in ["*Paper_Space", "*PAPER_SPACE", "*Paper_Space0"] {
        assert!(is_space_block_name(name), "{name}");
        assert!(is_paper_space_name(name), "{name}");
    }
    assert!(!is_space_block_name("MyBlock"));
    assert!(!is_paper_space_name("*Model_Space"));
}

#[test]
fn display_support_separates_drawn_from_unrendered() {
    let line = SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    };
    assert_eq!(display_support(&line).0, SupportStatus::Verified);
    let text = SemanticGeometry::Text {
        text: "x".into(),
        position: Point3::default(),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    };
    // Text is a supported entity whose drawing depends on a host font, so it
    // is Unverified rather than Unsupported (audit B20 still holds: no claim
    // that the glyphs will be available).
    assert_eq!(display_support(&text).0, SupportStatus::Unverified);
    let opaque = SemanticGeometry::Opaque {
        type_key: "ACIS".into(),
        version: 1,
        payload: Vec::new(),
    };
    assert_eq!(display_support(&opaque).0, SupportStatus::Unsupported);
    let insert = SemanticGeometry::Insert {
        block: BlockId(0),
        transform: Transform3::identity(),
    };
    assert_eq!(display_support(&insert).0, SupportStatus::Unverified);
}

#[test]
fn completeness_never_reports_complete_for_unrendered_content() {
    // Text-only drawing: parsed but not drawable -> Missing, never Complete.
    let c = aggregate_completeness(
        SupportStatus::Unsupported,
        false,
        vec!["AcDbText".into()],
        vec![],
    );
    assert!(matches!(c, Completeness::Missing(_)), "{c:?}");
    // Mixed drawable + unrendered -> Partial.
    let c = aggregate_completeness(
        SupportStatus::Unsupported,
        true,
        vec!["AcDbText".into()],
        vec![],
    );
    assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
    // Fully drawable -> Complete.
    assert_eq!(
        aggregate_completeness(SupportStatus::Verified, true, vec![], vec![]),
        Completeness::Complete
    );
    // Empty drawing with no fault -> Complete.
    assert_eq!(
        aggregate_completeness(SupportStatus::Verified, false, vec![], vec![]),
        Completeness::Complete
    );
    // A separate import fault is still Partial, not Complete.
    let c = aggregate_completeness(
        SupportStatus::Verified,
        false,
        vec![],
        vec!["stream".into()],
    );
    assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
}
