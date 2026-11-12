//! Experimental DWG export: synthetic acadrust self-roundtrip **only**.
//!
//! These contracts prove internal consistency of the write path we own (our
//! reverse-mapped `CadDocument` → upstream `acadrust::DwgWriter` → upstream
//! `acadrust::DwgReader`). They do **not** prove that AutoCAD, ODA, BricsCAD or
//! any other consumer opens the bytes, and they are not vendor-compatibility
//! evidence. The explicit report note (`export.note.dwg_experimental`) states
//! the same boundary to the user.

use super::*;

fn dwg_of(db: &DrawingDatabase) -> (Vec<u8>, ExportReport) {
    export_db_format(db, ExportFormat::Dwg)
}

fn read_dwg(bytes: &[u8]) -> CadDocument {
    DwgReader::from_stream(Cursor::new(bytes.to_vec()))
        .read()
        .expect("acadrust reads back its own DWG output")
}

fn kind(entity: &EntityType) -> &'static str {
    match entity {
        EntityType::Line(_) => "line",
        EntityType::Circle(_) => "circle",
        EntityType::LwPolyline(_) => "lwpolyline",
        EntityType::Polyline3D(_) => "polyline3d",
        EntityType::Text(_) => "text",
        other => panic!("unexpected entity kind in the DWG roundtrip: {other:?}"),
    }
}

/// A small 2D drawing: LINE, CIRCLE and a closed LWPOLYLINE (triangle).
fn small_2d() -> DrawingDatabase {
    db_with(vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(10.0, 0.0, 0.0),
        },
        SemanticGeometry::Circle {
            center: p(5.0, 5.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 2.5,
        },
        SemanticGeometry::Polyline {
            points: vec![p(0.0, 10.0, 0.0), p(4.0, 10.0, 0.0), p(4.0, 14.0, 0.0)],
            bulges: vec![0.0; 3],
            closed: true,
            width: 0.0,
        },
    ])
}

#[test]
fn synthetic_self_roundtrip_keeps_line_circle_and_closed_polyline() {
    let (bytes, report) = dwg_of(&small_2d());

    assert_eq!(report.format, ExportFormat::Dwg);
    assert_eq!(report.counts.bytes, bytes.len());
    assert!(
        bytes.starts_with(b"AC10"),
        "the output must carry a DWG version signature: {:?}",
        &bytes[..bytes.len().min(6)]
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.dwg_experimental"),
        "a DWG export always states the experimental/vendor-unverified boundary"
    );

    let read = read_dwg(&bytes);
    let mut kinds: Vec<&str> = read.model_space_entities().map(kind).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["circle", "line", "lwpolyline"]);

    // The closed flag is part of the geometry, not presentation: it must survive.
    let closed = read
        .model_space_entities()
        .filter_map(|entity| match entity {
            EntityType::LwPolyline(polyline) => Some(polyline.is_closed),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(closed, [true], "the closed polyline stays closed");
}

#[test]
fn synthetic_self_roundtrip_of_an_empty_drawing_is_valid_and_non_empty() {
    let (bytes, report) = dwg_of(&db_with(Vec::new()));

    assert!(
        !bytes.is_empty(),
        "an empty drawing still writes a DWG file"
    );
    assert!(
        bytes.starts_with(b"AC10"),
        "even an empty export is a DWG file, not an empty buffer"
    );
    assert_eq!(report.counts.dropped, 0);
    let read = read_dwg(&bytes);
    assert_eq!(read.model_space_entities().count(), 0);
}

#[test]
fn an_over_limit_dwg_export_is_an_explicit_error_not_a_panic() {
    // The same limit gate as the DXF path: the exporter fails by name before any
    // writer runs, so the DWG branch cannot turn a refusal into a partial file.
    let database = small_2d();
    let error = export(&ExportRequest {
        database: &database,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::Dwg,
        limits: ExportLimits {
            max_output_entities: 0,
            max_explode_depth: ExportLimits::default().max_explode_depth,
        },
        units: None,
    })
    .expect_err("the entity limit must reject the export explicitly");

    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("entity output limit"),
            "the error names the real limit: {message}"
        ),
        other => panic!("expected an explicit InvalidInput refusal, got {other:?}"),
    }
}
