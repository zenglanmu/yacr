//! Fixture round-trip loss assertions, deterministic text output and the negative contract.

use super::*;

// ---------------------------------------------------------------------------
// 3. Fixture self round-trip with explicit loss assertions.
// ---------------------------------------------------------------------------

fn fixture_bytes(path: &str) -> Arc<[u8]> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join(path)).expect("fixture readable");
    Arc::from(bytes.into_boxed_slice())
}

/// The set of `export.*` codes a fixture's export produced, sorted.
fn code_set(report: &ExportReport) -> Vec<String> {
    let mut codes: Vec<String> = report
        .entries
        .iter()
        .map(|e| e.code.clone())
        .chain(report.notes.iter().map(|e| e.code.clone()))
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

#[test]
fn committed_fixtures_report_their_expected_losses() {
    // Hard-coded expected loss codes per manifest-governed fixture. A regression
    // that starts dropping (or no longer drops) a kind fails here.
    let expectations: &[(&str, &[&str])] = &[
        (
            "fixtures/plot/a4-layout.dwg",
            &[
                "export.dropped.paper_space",
                "export.note.bylayer_flattened",
                "export.note.units_missing",
            ],
        ),
        (
            "fixtures/dwg/synthetic-four-lines.dwg",
            &["export.note.bylayer_flattened", "export.note.units_missing"],
        ),
        (
            "fixtures/dxf/qcad-flange/flange.dxf",
            &[
                "export.converted.compound",
                "export.converted.mesh",
                "export.dropped.paper_space",
                "export.dropped.unreferenced_block",
                "export.note.bylayer_flattened",
                "export.note.units_missing",
            ],
        ),
    ];
    for (path, expected) in expectations {
        let imported = import_bytes(&fixture_bytes(path));
        let (bytes, report) = export_db(&imported);
        // The output is well-formed ASCII DXF (our reader agrees).
        let reimported = import_bytes(&bytes);

        // (a) When nothing was converted, every model geometry is exact and must
        // round-trip pointwise. A fixture with conversions legitimately changes
        // structure, so its exactness is asserted through the code set instead.
        if report.counts.converted == 0 {
            let model = |db: &DrawingDatabase| -> Vec<SemanticGeometry> {
                let mut geometries: Vec<(i64, SemanticGeometry)> = db
                    .entities()
                    .filter(|e| e.space == SpaceId::Model)
                    .map(|e| (e.draw_order, e.geometry.clone()))
                    .collect();
                geometries.sort_by_key(|(order, _)| *order);
                geometries.into_iter().map(|(_, g)| g).collect()
            };
            let before = model(&imported);
            let after = model(&reimported);
            assert_eq!(
                before.len(),
                after.len(),
                "{path}: model entity count changed"
            );
            for (a, b) in before.iter().zip(after.iter()) {
                match (a, b) {
                    (
                        SemanticGeometry::Line { start: sa, end: ea },
                        SemanticGeometry::Line { start: sb, end: eb },
                    ) => {
                        assert_point(*sa, *sb);
                        assert_point(*ea, *eb);
                    }
                    (x, y) => assert_eq!(x, y, "{path}: geometry changed"),
                }
            }
        }

        // (b) The loss code set matches the hard-coded expectation.
        assert_eq!(&code_set(&report), expected, "{path}");
        // (c) Any drop implies the export is not Complete.
        if report.counts.dropped > 0 {
            assert_ne!(report.completeness, Completeness::Complete, "{path}");
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Deterministic DXF text for a tiny synthetic document.
// ---------------------------------------------------------------------------

#[test]
fn tiny_document_writes_deterministic_ascii_dxf() {
    let db = db_with(vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        SemanticGeometry::Circle {
            center: p(2.0, 2.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 1.0,
        },
    ]);
    let (first, report) = export_db(&db);
    let (second, _) = export_db(&db);
    // Deterministic: identical input bytes produce identical output bytes.
    assert_eq!(first, second);
    assert_eq!(report.counts.bytes, first.len());
    let text = std::str::from_utf8(&first).expect("ASCII DXF is valid UTF-8");
    for marker in ["SECTION", "ENTITIES", "LINE", "CIRCLE", "EOF"] {
        assert!(text.contains(marker), "missing DXF marker {marker}");
    }
    // A stable, diffable excerpt: the entity type records in output order.
    let types: Vec<&str> = text
        .lines()
        .filter(|line| matches!(line.trim(), "LINE" | "CIRCLE" | "LWPOLYLINE"))
        .collect();
    assert_eq!(types, vec!["LINE", "CIRCLE"]);
}

// ---------------------------------------------------------------------------
// 4. Negative contract: the leaf mapping is exhaustive (compile-enforced) and
//    every dropped kind yields an explicit diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn dropping_anything_never_yields_a_complete_report() {
    let db = db_with(vec![SemanticGeometry::Opaque {
        type_key: "ACAD_PROXY_ENTITY".into(),
        version: 1,
        payload: Vec::new(),
    }]);
    let (_, report) = export_db(&db);
    assert_eq!(report.counts.dropped, 1);
    assert_eq!(report.counts.exact, 0);
    assert!(matches!(report.completeness, Completeness::Missing(_)));
    assert!(report.counts.bytes > 0);
}

#[test]
fn limits_are_enforced_explicitly() {
    let db = db_with(vec![
        SemanticGeometry::Point(p(0.0, 0.0, 0.0)),
        SemanticGeometry::Point(p(1.0, 0.0, 0.0)),
    ]);
    let result = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits {
            max_output_entities: 1,
            max_explode_depth: 32,
        },
        units: None,
    });
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
}

/// Guard against a silently empty report type: `ExportCounts` stays constructible.
#[test]
fn export_counts_default_is_zero() {
    assert_eq!(ExportCounts::default().bytes, 0);
}
