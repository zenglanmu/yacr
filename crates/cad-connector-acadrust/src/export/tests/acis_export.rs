//! Export contracts for the exact ACIS writer: a DXF `3DSOLID` with inline SAT
//! text, a DWG `3DSOLID` whose payload is a binary SAB blob in the AcDs section,
//! the per-body POLYFACE fallback with its named refusal in **both** formats, and
//! the document-global AC1024 rule (DXF only).
//!
//! Writer + our own reader agreement is **internal consistency**, not a
//! third-party compatibility claim. The DWG case is a **DWG round trip** through
//! upstream `acadrust::DwgReader` (the AcDs blob comes back attached to the
//! solid), not an AutoCAD/ODA/vendor verification.

use super::acis_fixtures::*;
use super::*;

use cad_domain::{BrepFace, BrepShell, BrepSurface};

/// Both containers, so a body-level rule is asserted for the DXF and the DWG
/// instead of only the one an older test happened to use.
const BOTH_FORMATS: [ExportFormat; 2] = [ExportFormat::DxfText, ExportFormat::Dwg];

/// Read exported bytes back with the reader for `format`.
fn read_export(bytes: &[u8], format: ExportFormat) -> CadDocument {
    match format {
        ExportFormat::DxfText => DxfReader::from_reader(Cursor::new(bytes.to_vec()))
            .expect("the written DXF parses")
            .read()
            .expect("the written DXF reads"),
        ExportFormat::Dwg => DwgReader::from_stream(Cursor::new(bytes.to_vec()))
            .read()
            .expect("acadrust reads back its own DWG output"),
    }
}

/// `(3DSOLID count, POLYFACE count)` of the model-space entities.
fn solid_and_polyface_counts(document: &CadDocument) -> (usize, usize) {
    document
        .model_space_entities()
        .fold((0, 0), |(solids, meshes), entity| match entity {
            EntityType::Solid3D(_) => (solids + 1, meshes),
            EntityType::PolyfaceMesh(_) => (solids, meshes + 1),
            _ => (solids, meshes),
        })
}

/// The one `3DSOLID` in `document`, or a panic naming what was found instead.
fn single_solid(document: &CadDocument) -> &acadrust::entities::Solid3D {
    let mut solids = document
        .model_space_entities()
        .filter_map(|entity| match entity {
            EntityType::Solid3D(solid) => Some(solid),
            _ => None,
        });
    let solid = solids.next().expect("a 3DSOLID record");
    assert!(solids.next().is_none(), "exactly one 3DSOLID record");
    solid
}

/// Lift a written `3DSOLID`'s ACIS payload back to the neutral B-rep.
///
/// A DWG (AC1032) payload is the binary SAB blob the reader attached from the
/// AcDs section; a DXF (AC1024) payload is inline SAT text.
fn lift_solid(solid: &acadrust::entities::Solid3D) -> BrepData {
    let acis = &solid.acis_data;
    if acis.is_binary {
        assert!(
            acis.sat_data.is_empty(),
            "a SAB-backed solid carries no inline SAT text"
        );
        return crate::sab_to_brep(&acis.sab_data)
            .expect("the written SAB blob parses with acadrust's own SAB reader");
    }
    assert!(
        !acis.sat_data.is_empty(),
        "a SAT-backed solid carries its text inline"
    );
    let document = acis
        .parse()
        .expect("the written payload parses as SAT text");
    crate::sat_to_brep(&document)
}

#[test]
fn an_exact_planar_solid_exports_as_a_3dsolid_with_acis_sat_text() {
    let (bytes, report) = export_one(solid_body(box_brep(1.0, 2.0, 3.0)));
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("3DSOLID"),
        "the output must carry a 3DSOLID record"
    );
    let entities = read_entities(&bytes);
    let solids: Vec<&EntityType> = entities
        .iter()
        .filter(|entity| matches!(entity, EntityType::Solid3D(_)))
        .collect();
    assert_eq!(solids.len(), 1, "one 3DSOLID record");
    let EntityType::Solid3D(solid) = solids[0] else {
        unreachable!();
    };
    assert!(
        solid.acis_data.has_data(),
        "the 3DSOLID must carry a non-empty ACIS payload"
    );
    assert!(
        !solid.acis_data.is_binary,
        "the DXF payload is SAT text, never a binary SAB blob"
    );
    let document = solid
        .acis_data
        .parse()
        .expect("the written payload parses as SAT");
    assert_eq!(
        document.records_of_type("face").len(),
        6,
        "the payload is the exact six-face box"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| matches!(entity, EntityType::PolyfaceMesh(_))),
        "the exact record replaces the display cache, it is not written twice"
    );

    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.acis_solid"
                && entry
                    .message
                    .contains("written as ACIS SAT text in a 3DSOLID")),
        "{:?}",
        report.entries
    );
    assert!(
        report.entries.iter().any(|entry| entry
            .message
            .contains("no third-party CAD reader was used to verify")),
        "the report must never imply vendor verification: {:?}",
        report.entries
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the AC1024 choice for inline SAT text is declared: {:?}",
        report.notes
    );
}

/// The DWG counterpart: a real `3DSOLID` whose ACIS payload is a binary SAB blob
/// in the AC1032 AcDs section, read back by upstream `acadrust::DwgReader`.
#[test]
fn a_dwg_export_writes_a_3dsolid_that_reads_back_as_the_same_body() {
    let authored = box_brep(1.0, 2.0, 3.0);
    let (bytes, report) = export_db_format(
        &db_with(vec![solid_body(authored.clone())]),
        ExportFormat::Dwg,
    );
    let document = read_export(&bytes, ExportFormat::Dwg);

    assert_eq!(
        document.version,
        acadrust::DxfVersion::AC1032,
        "a DWG stays at the document's own AC1032; the DXF AC1024 pin must not leak"
    );
    assert_eq!(
        solid_and_polyface_counts(&document),
        (1, 0),
        "one 3DSOLID and no POLYFACE display cache"
    );
    let solid = single_solid(&document);
    assert!(
        solid.acis_data.is_binary,
        "the DWG payload is the binary SAB blob, not SAT text"
    );
    assert!(
        !solid.acis_data.sab_data.is_empty(),
        "the SAB blob reached the solid through the AcDs section"
    );
    assert!(
        solid.common.has_ds_data,
        "the entity is flagged as data-store-backed, so a reader pairs it with its AcDs blob"
    );
    // The reader only fills `sab_data` from the AcDs (AcDsPrototype_1b) section,
    // and it says so when it attaches a blob; the inline body was written empty.
    assert!(
        document
            .notifications
            .iter()
            .any(|notification| notification.message.starts_with("AcDs: attached")),
        "the reader attached the payload from the AcDs section: {:?}",
        document.notifications.iter().collect::<Vec<_>>()
    );

    // The payload is auditable on its own, not only through the DWG reader.
    assert!(
        acadrust::SabReader::read(&solid.acis_data.sab_data).is_ok(),
        "the attached SAB blob parses with acadrust's own SAB reader"
    );

    // And it is the same body: topology and volume, through the existing lift.
    let parsed = lift_solid(solid);
    assert_brep_equivalent(&authored, &parsed);
    assert_eq!(parsed.face_count(), 6, "the exact six-face box");
    let volume = tessellated_volume(parsed);
    let expected = 1.0 * 2.0 * 3.0;
    assert!(
        (volume - expected).abs() / expected <= 1e-3,
        "box volume {volume} must match {expected} within 0.1%"
    );

    // The report states the DWG payload provenance and evidence boundary, and
    // the DXF-only AC1024 note is absent.
    let note = report
        .notes
        .iter()
        .find(|note| note.code == "export.note.acis_sab_dwg")
        .unwrap_or_else(|| panic!("no SAB note in {:?}", report.notes));
    assert!(
        note.message.contains("retained exact B-Rep"),
        "the note names the payload's source: {}",
        note.message
    );
    assert!(
        note.message.contains("no third-party CAD reader"),
        "the note names the evidence boundary: {}",
        note.message
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the AC1024 pin is a DXF rule only: {:?}",
        report.notes
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.acis_solid"
                && entry.message.contains("binary SAB blob")),
        "{:?}",
        report.entries
    );
}

/// Every fixture the SAT writer accepts must survive the DWG container with the
/// same face/loop/edge structure and the same volume.
#[test]
fn every_planar_fixture_reads_back_from_the_dwg_with_its_own_counts() {
    let fixtures: [(&str, BrepData, usize, f64); 3] = [
        ("box", box_brep(2.0, 3.0, 4.0), 6, 24.0),
        ("wedge", wedge_brep(4.0, 2.0, 3.0), 5, 12.0),
        ("pyramid", pyramid_brep(3.0, 4.0), 5, 12.0),
    ];
    for (name, authored, faces, expected_volume) in fixtures {
        let (bytes, _) = export_db_format(
            &db_with(vec![solid_body(authored.clone())]),
            ExportFormat::Dwg,
        );
        let document = read_export(&bytes, ExportFormat::Dwg);
        let solid = single_solid(&document);
        let parsed = lift_solid(solid);
        assert_eq!(parsed.face_count(), faces, "{name} face count");
        assert_brep_equivalent(&authored, &parsed);
        let volume = tessellated_volume(parsed);
        assert!(
            (volume - expected_volume).abs() / expected_volume <= 1e-3,
            "{name} volume {volume} must match {expected_volume} within 0.1%"
        );
    }
}

/// A body outside the planar subset keeps the POLYFACE display cache and records
/// the ACIS writer's own code — in both formats, because the refusal is a
/// property of the body, not of the container.
#[test]
fn a_curved_exact_body_exports_the_polyface_and_names_the_refusal() {
    let cylinder = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 3,
                surface: BrepSurface::Cylinder {
                    origin: p(0.0, 0.0, 0.0),
                    axis: p(0.0, 0.0, 1.0),
                    ref_dir: p(1.0, 0.0, 0.0),
                    radius: 1.0,
                },
                reversed: false,
                loops: Vec::new(),
                tolerance: None,
                domain: None,
            }],
        }],
        placement: None,
        tolerance: None,
    };
    for format in BOTH_FORMATS {
        let (bytes, report) =
            export_db_format(&db_with(vec![solid_body(cylinder.clone())]), format);
        let document = read_export(&bytes, format);
        assert_eq!(
            solid_and_polyface_counts(&document),
            (0, 1),
            "{format:?}: a curved body has no exact payload, so the display cache is written"
        );
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.code == "acis.curved_face_not_written"
                    && entry.message.contains("acis.planar_only")),
            "{format:?}: {:?}",
            report.entries
        );
        assert_eq!(report.counts.converted, 1, "{format:?}: one outcome");
    }
}

/// The remaining refusals are per-body and named in both formats, and none of
/// them leaves an empty `3DSOLID` behind.
#[test]
fn multi_shell_mirrored_sheared_and_open_bodies_are_refused_in_both_formats() {
    let two_shells = || {
        let mut brep = box_brep(1.0, 1.0, 1.0);
        let second = BrepShell {
            id: 1,
            faces: brep.shells[0].faces.clone(),
        };
        brep.shells.push(second);
        brep
    };
    let mirrored = || {
        let mut brep = box_brep(1.0, 1.0, 1.0);
        brep.placement = Some(BrepPlacement {
            matrix: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: p(0.0, 0.0, 0.0),
            scale: 1.0,
        });
        brep
    };
    let sheared = || {
        let mut brep = box_brep(1.0, 1.0, 1.0);
        brep.placement = Some(BrepPlacement {
            matrix: [[1.0, 0.5, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: p(0.0, 0.0, 0.0),
            scale: 1.0,
        });
        brep
    };
    // An open shell: drop one face, so four edges are used once.
    let open_shell = || {
        let mut brep = box_brep(1.0, 1.0, 1.0);
        brep.shells[0].faces.pop();
        brep
    };

    let cases: [(&str, BrepData); 4] = [
        ("acis.multi_shell_not_written", two_shells()),
        ("acis.placement_not_written", mirrored()),
        ("acis.placement_not_written", sheared()),
        ("acis.invalid_topology_not_written", open_shell()),
    ];
    for (code, authored) in cases {
        for format in BOTH_FORMATS {
            let (bytes, report) =
                export_db_format(&db_with(vec![solid_body(authored.clone())]), format);
            let document = read_export(&bytes, format);
            assert_eq!(
                solid_and_polyface_counts(&document),
                (0, 1),
                "{code} in {format:?}: the display cache, never a fabricated 3DSOLID"
            );
            assert!(
                report.entries.iter().any(|entry| entry.code == code),
                "{code} in {format:?}: {:?}",
                report.entries
            );
            assert_eq!(
                report.counts.converted, 1,
                "{code} in {format:?}: one geometry, one outcome"
            );
        }
    }
}

/// A refused body with no display mesh still names its own code: the wire branch
/// and the drop branch must not fall back to a blanket "unsupported in DWG".
#[test]
fn refused_bodies_without_a_display_mesh_keep_their_own_code() {
    // Edges only (no facets), and a body that cannot be written: the wire branch.
    let wire = solid_body_with_display(
        BrepData {
            shells: Vec::new(),
            placement: None,
            tolerance: None,
        },
        Mesh::default(),
        vec![vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)]],
    );
    for format in BOTH_FORMATS {
        let (bytes, report) = export_db_format(&db_with(vec![wire.clone()]), format);
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.code == "acis.no_shell_not_written"),
            "{format:?}: the wire branch names the ACIS refusal: {:?}",
            report.entries
        );
        assert_eq!(report.counts.converted, 1, "{format:?}: one geometry");
        let document = read_export(&bytes, format);
        let solids = document
            .model_space_entities()
            .filter(|entity| matches!(entity, EntityType::Solid3D(_)))
            .count();
        assert_eq!(solids, 0, "{format:?}: no 3DSOLID is fabricated");
    }

    // Nothing drawable at all: the drop branch.
    let empty = solid_body_with_display(
        BrepData {
            shells: Vec::new(),
            placement: None,
            tolerance: None,
        },
        Mesh::default(),
        Vec::new(),
    );
    for format in BOTH_FORMATS {
        let (bytes, report) = export_db_format(&db_with(vec![empty.clone()]), format);
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.code == "acis.no_shell_not_written"),
            "{format:?}: the drop branch names the ACIS refusal: {:?}",
            report.entries
        );
        assert_eq!(report.counts.dropped, 1, "{format:?}: one geometry");
        assert_eq!(
            read_export(&bytes, format).model_space_entities().count(),
            0,
            "{format:?}: a body with nothing to draw writes no entity"
        );
    }
}

/// A `3DSOLID` written inside a `BLOCK_RECORD` must still pin the DXF file to
/// AC1024 and keep its SAT text inline: the version is a document header, and
/// the block member is drained into the document before the writer runs.
#[test]
fn a_solid_inside_a_written_block_still_switches_the_file_to_ac1024() {
    let (bytes, report) = export_db(&db_with_blocked_solid());
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("3DSOLID"),
        "the block member is written as a 3DSOLID"
    );
    assert!(
        text.contains("AC1024"),
        "the whole file is AC1024 because a 3DSOLID is present"
    );
    assert!(
        !text.contains("ACDSDATA"),
        "inline SAT text, never an ACDSDATA SAB blob"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the version choice is declared: {:?}",
        report.notes
    );

    let document = read_document(&bytes);
    let members = block_entities(&document, "BOXBLOCK");
    assert_eq!(members.len(), 1, "one member in the block definition");
    match &members[0] {
        EntityType::Solid3D(solid) => {
            assert!(
                !solid.acis_data.is_binary,
                "the block-embedded payload stays SAT text"
            );
            assert!(
                !solid.acis_data.sat_data.is_empty(),
                "the block-embedded 3DSOLID carries its payload inline"
            );
            assert!(
                solid.acis_data.parse().is_some(),
                "the block-embedded payload parses as SAT"
            );
        }
        other => panic!("expected a 3DSOLID block member, got {other:?}"),
    }
}

/// The same block-embedded solid in a DWG: the payload is a SAB blob in AcDs,
/// and the file stays AC1032.
#[test]
fn a_solid_inside_a_written_block_carries_its_sab_in_the_dwg() {
    let (bytes, report) = export_db_format(&db_with_blocked_solid(), ExportFormat::Dwg);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sab_dwg"),
        "the DWG payload note is declared: {:?}",
        report.notes
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the AC1024 pin never fires for a DWG: {:?}",
        report.notes
    );
    let document = read_export(&bytes, ExportFormat::Dwg);
    assert_eq!(document.version, acadrust::DxfVersion::AC1032);
    assert!(
        document
            .notifications
            .iter()
            .any(|notification| notification.message.starts_with("AcDs: attached")),
        "the reader attached the block member's payload from the AcDs section: {:?}",
        document.notifications.iter().collect::<Vec<_>>()
    );
    let members = block_entities(&document, "BOXBLOCK");
    assert_eq!(members.len(), 1, "one member in the block definition");
    match &members[0] {
        EntityType::Solid3D(solid) => {
            assert!(
                solid.acis_data.is_binary && !solid.acis_data.sab_data.is_empty(),
                "the block-embedded DWG payload is a SAB blob"
            );
            let parsed = lift_solid(solid);
            assert_eq!(parsed.face_count(), 6, "the exact six-face box");
        }
        other => panic!("expected a 3DSOLID block member, got {other:?}"),
    }
}

/// The version switch is not a per-entity choice: a drawing with a solid *and*
/// ordinary entities still pins DXF to AC1024 and writes no ACDSDATA, while the
/// DWG keeps its own version, carries the note and keeps both entities.
#[test]
fn a_mixed_document_with_a_solid_and_mtext_pins_ac1024() {
    let geometry = vec![
        solid_body(box_brep(1.0, 1.0, 1.0)),
        SemanticGeometry::Text {
            text: "line one\\Pline two".into(),
            position: p(0.0, 0.0, 2.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: TextLineSpacing::default(),
            mirrored: false,
        },
        SemanticGeometry::Polyline {
            points: vec![p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0), p(4.0, 4.0, 0.0)],
            bulges: vec![0.0; 3],
            closed: true,
            width: 0.0,
        },
    ];
    let database = db_with(geometry);

    // DXF: AC1024, inline SAT text, no ACDSDATA.
    let (bytes, report) = export_db(&database);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("AC1024"), "the file is AC1024");
    assert!(!text.contains("ACDSDATA"), "no ACDSDATA section is written");
    assert!(text.contains("MTEXT"), "the MTEXT record is written");
    assert!(text.contains("3DSOLID"), "the 3DSOLID record is written");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the version note accompanies the mixed document: {:?}",
        report.notes
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sab_dwg"),
        "a DXF export never carries the DWG payload note: {:?}",
        report.notes
    );

    let document = read_document(&bytes);
    let solids = document
        .model_space_entities()
        .filter(|entity| matches!(entity, EntityType::Solid3D(_)))
        .count();
    assert_eq!(solids, 1);
    assert!(
        document
            .model_space_entities()
            .any(|entity| matches!(entity, EntityType::MText(_))),
        "the MTEXT survives next to the 3DSOLID"
    );

    // DWG: its own version, the SAB note, the same two entities.
    let (bytes, report) = export_db_format(&database, ExportFormat::Dwg);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sab_dwg"),
        "the DWG payload note accompanies the mixed document: {:?}",
        report.notes
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "the DXF version pin never leaks into the DWG: {:?}",
        report.notes
    );
    let document = read_export(&bytes, ExportFormat::Dwg);
    assert_eq!(
        document.version,
        acadrust::DxfVersion::AC1032,
        "the DWG keeps the document's own version"
    );
    assert_eq!(
        solid_and_polyface_counts(&document),
        (1, 0),
        "the exact 3DSOLID replaces the display cache"
    );
    assert!(
        document
            .model_space_entities()
            .any(|entity| matches!(entity, EntityType::MText(_))),
        "the MTEXT survives next to the 3DSOLID in the DWG"
    );
}
