//! ASCII DXF ingestion and typed attribute round-trips.

use super::*;
#[test]
fn ascii_dxf_enters_shared_database_and_truncation_is_not_empty_success() {
    let bytes = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n5\n10\n8\n0\n10\n1\n20\n2\n30\n0\n11\n4\n21\n6\n31\n0\n0\nENDSEC\n0\nEOF\n";
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.as_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let drawing = AcadrustImporter::default()
        .import(&request, &|| false)
        .unwrap();
    assert!(drawing
        .database
        .entities()
        .any(|e| matches!(e.geometry, SemanticGeometry::Line { .. })));
    let acad = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
        .unwrap()
        .read()
        .unwrap();
    let binary = acadrust::DxfWriter::new_binary(&acad)
        .write_to_vec()
        .unwrap();
    let binary_request = ImportRequest {
        bytes: Arc::from(binary),
        document: request.document,
        database: request.database,
        limits: request.limits,
        generation: 0,
    };
    let binary_drawing = AcadrustImporter::default()
        .import(&binary_request, &|| false)
        .unwrap();
    assert!(binary_drawing
        .database
        .entities()
        .any(|e| matches!(e.geometry, SemanticGeometry::Line { .. })));
    let truncated = ImportRequest {
        bytes: Arc::from(&bytes[..bytes.len() - 6]),
        ..request
    };
    assert!(AcadrustImporter::default()
        .import(&truncated, &|| false)
        .is_err());
}

// ---- acadrust 0.6.3 capability delta: LAYOUT plot codes and STYLE xdata ----

fn parse_dxf(bytes: &[u8]) -> acadrust::CadDocument {
    acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
        .expect("the DXF reader opens")
        .read()
        .expect("the DXF parses")
}

/// A minimal OBJECTS section with one LAYOUT carrying `group 72`/`group 73`
/// inside its `AcDbPlotSettings` subclass.
fn layout_dxf(paper_units: i32, rotation: i32) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nOBJECTS\n\
         0\nLAYOUT\n5\n20\n\
         100\nAcDbPlotSettings\n\
         4\nCustom_(210.00_x_297.00)\n\
         44\n210.0\n45\n297.0\n\
         72\n{paper_units}\n73\n{rotation}\n\
         100\nAcDbLayout\n1\nLayout1\n330\n1F\n\
         0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

/// A minimal TABLES section with one STYLE entry.
fn style_table_dxf(style_block: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n{style_block}\n\
         0\nENDTAB\n0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

#[test]
fn acadrust_types_dxf_layout_group_72_and_73() {
    // A capability note claimed the locked acadrust reader drops a LAYOUT's
    // `group 72` (paper units) and `group 73` (plot rotation). Source-verified
    // against 0.6.3 — and identical in 0.5.5 — `read_layout` applies both, so
    // the typed fields are populated. The note was never accurate.
    let bytes = layout_dxf(1, 1);
    let acad = parse_dxf(&bytes);
    // acadrust also seeds a default "Model" layout, so select the file's own
    // layout by name rather than by nondeterministic HashMap order.
    let layout = acad
        .objects
        .values()
        .find_map(|object| match object {
            acadrust::objects::ObjectType::Layout(layout) if layout.name == "Layout1" => {
                Some(layout)
            }
            _ => None,
        })
        .expect("the DXF carries a LAYOUT object");
    assert_eq!(layout.plot_paper_units, 1, "group 72 is applied");
    assert_eq!(layout.plot_rotation, 1, "group 73 is applied");
}

#[test]
fn acadrust_types_the_style_xdata_face_and_the_scan_keeps_group_3() {
    // 0.6.3 types the `1001 ACAD`/`1000` face into `TextStyle.true_type_font`
    // while `group 3` still fills `font_file`. yacr's byte scan must never
    // override the declared group 3.
    let bytes = style_table_dxf("0\nSTYLE\n2\ns\n3\ntxt.shx\n1001\nACAD\n1000\nArial");
    let acad = parse_dxf(&bytes);
    let style = acad
        .text_styles
        .iter()
        .find(|style| style.name == "s")
        .expect("the STYLE entry is parsed");
    assert_eq!(style.font_file, "txt.shx");
    assert_eq!(style.true_type_font, "Arial");
    assert_eq!(
        dxf_style_xdata_fonts(&bytes).get("s").map(String::as_str),
        Some("txt.shx"),
        "the byte scan never overrides a declared group 3"
    );
}

#[test]
fn importer_resolves_the_typed_true_type_face_without_the_scan() {
    // QCAD leaves group 3 empty and writes the face as STYLE XDATA. 0.6.3 now
    // types it, so `read_styles` resolves the face through the typed field.
    let bytes = style_table_dxf("0\nSTYLE\n2\ntextstyle0\n3\n\n1001\nACAD\n1000\nArial\n1071\n0");
    let acad = parse_dxf(&bytes);
    let req = request(bytes);
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &acad, stats, compute_identity(&[]));
    importer.read_styles().unwrap();
    assert_eq!(
        importer.style_fonts.get("textstyle0").map(String::as_str),
        Some("Arial")
    );
}

#[test]
fn importer_style_font_chain_is_typed_face_then_group_3_then_scan() {
    // The `read_styles` chain is unchanged by the upgrade: an explicitly named
    // TrueType face (now typed from xdata) wins over group 3, which in turn
    // wins over the byte scan. This pins the interaction between
    // `dxf_style_fonts` and `style.true_type_font`; the scan's own
    // never-override contract is covered by the test above.
    let bytes = style_table_dxf("0\nSTYLE\n2\ns\n3\ntxt.shx\n1001\nACAD\n1000\nArial");
    let acad = parse_dxf(&bytes);
    let req = request(bytes);
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &acad, stats, compute_identity(&[]));
    importer.read_styles().unwrap();
    assert_eq!(
        importer.style_fonts.get("s").map(String::as_str),
        Some("Arial")
    );
}

/// The drawing's own text code page (`$DWGCODEPAGE`) must survive import into
/// `cad-db`; it is the only authority for the text locale and must not be
/// dropped or guessed.
#[test]
fn dxf_code_page_is_carried_into_the_database() {
    let bytes = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\nANSI_936\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n0\nLINE\n5\n10\n8\n0\n10\n1\n20\n2\n30\n0\n11\n4\n21\n6\n31\n0\n0\nENDSEC\n0\nEOF\n";
    let drawing = AcadrustImporter::default()
        .import(&request(bytes.to_vec()), &|| false)
        .unwrap();
    assert_eq!(drawing.database.code_page(), Some("ANSI_936"));

    // Reload the same bytes again: the value is a stored property, not a
    // one-shot read of the reader.
    let reloaded = AcadrustImporter::default()
        .import(&request(bytes.to_vec()), &|| false)
        .unwrap();
    assert_eq!(reloaded.database.code_page(), Some("ANSI_936"));
}

/// An XLINE/RAY can only be stored as a finite segment, so the per-class
/// capability must say `Partial` — never `Verified`, which would tell the host
/// the infinite entity survived the import.
#[test]
fn construct_lines_import_as_partial_capabilities_not_verified() {
    // A LINE first so the drawing extent (the clip box) is a real one.
    let bytes = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n5\n10\n8\n0\n10\n-10\n20\n-10\n30\n0\n11\n10\n21\n10\n31\n0\n\
0\nXLINE\n5\n11\n8\n0\n10\n0\n20\n0\n30\n0\n11\n1\n21\n0\n31\n0\n\
0\nRAY\n5\n12\n8\n0\n10\n0\n20\n5\n30\n0\n11\n0\n21\n1\n31\n0\n\
0\nENDSEC\n0\nEOF\n";
    let drawing = AcadrustImporter::default()
        .import(&request(bytes.to_vec()), &|| false)
        .expect("the DXF imports");
    for type_key in ["AcDbXline", "AcDbRay"] {
        // The geometry is a real finite line (not opaque), so the clipping
        // worked; only the verdict is partial.
        let stored = drawing
            .database
            .entities()
            .find(|e| e.object.type_key == type_key)
            .unwrap_or_else(|| panic!("{type_key} was not imported"));
        assert!(
            matches!(stored.geometry, SemanticGeometry::Line { .. }),
            "{type_key} must still reduce to a finite line"
        );
        let capability = drawing
            .report
            .capabilities
            .iter()
            .find(|c| c.type_key == type_key)
            .unwrap_or_else(|| panic!("{type_key} has no capability entry"));
        assert_eq!(
            capability.semantic,
            SupportStatus::Partial,
            "{type_key} is stored as a finite segment, not the infinite entity"
        );
    }
    // The reason is one of the two named construct-line reasons; which one
    // depends on whether the drawing extent was already known.
    let reasons = match &drawing.report.completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected a partial import, got {other:?}"),
    };
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("finite segment")
                || reason.contains("default 1000-unit box")),
        "no construct-line reason in {reasons:?}"
    );
}
