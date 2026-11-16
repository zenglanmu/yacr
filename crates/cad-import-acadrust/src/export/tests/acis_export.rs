//! Export contracts for the exact ACIS SAT text writer: DXF `3DSOLID`, the
//! POLYFACE fallback, the DWG refusal and the document-global AC1024 choice.
//!
//! Writer + our own reader agreement is **internal consistency**, not a
//! third-party compatibility claim.

use super::acis_fixtures::*;
use super::*;

use cad_domain::{BrepFace, BrepShell, BrepSurface};

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

#[test]
fn a_dwg_export_keeps_the_polyface_and_records_the_unsupported_reason() {
    let database = db_with(vec![solid_body(box_brep(1.0, 2.0, 3.0))]);
    let (bytes, report) = export_db_format(&database, ExportFormat::Dwg);
    let document = DwgReader::from_stream(Cursor::new(bytes))
        .read()
        .expect("acadrust reads back its own DWG output");
    let kinds: Vec<&str> = document
        .model_space_entities()
        .map(|entity| match entity {
            EntityType::PolyfaceMesh(_) => "polyface",
            EntityType::Solid3D(_) => "3dsolid",
            other => panic!("unexpected DWG entity {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["polyface"], "DWG keeps the display cache");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "acis.dwg_3dsolid_unsupported"),
        "{:?}",
        report.entries
    );
    assert_eq!(report.counts.converted, 1, "one geometry, one outcome");
}

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
            }],
        }],
        placement: None,
    };
    let (bytes, report) = export_one(solid_body(cylinder));
    let entities = read_entities(&bytes);
    assert!(
        entities
            .iter()
            .all(|entity| !matches!(entity, EntityType::Solid3D(_))),
        "a curved body has no exact SAT payload"
    );
    assert_eq!(
        entities
            .iter()
            .filter(|entity| matches!(entity, EntityType::PolyfaceMesh(_)))
            .count(),
        1,
        "the display cache is written instead"
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "acis.curved_face_not_written"
                && entry.message.contains("acis.planar_only")),
        "{:?}",
        report.entries
    );
    assert_eq!(report.counts.converted, 1);
}

/// A `3DSOLID` written inside a `BLOCK_RECORD` must still pin the file to
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

/// The version switch is not a per-entity choice: a drawing with a solid *and*
/// ordinary entities (MTEXT, polyline) still pins AC1024 and writes no ACDSDATA.
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
        },
        SemanticGeometry::Polyline {
            points: vec![p(0.0, 0.0, 0.0), p(4.0, 0.0, 0.0), p(4.0, 4.0, 0.0)],
            bulges: vec![0.0; 3],
            closed: true,
            width: 0.0,
        },
    ];
    let (bytes, report) = export_db(&db_with(geometry));
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
}

/// Every DWG body branch keeps the named DWG refusal: a body with edges only and
/// a body with nothing drawable both report `acis.dwg_3dsolid_unsupported`
/// instead of quietly claiming there was no exact geometry to lose.
#[test]
fn dwg_bodies_without_a_display_mesh_still_record_the_dwg_reason() {
    // Edges only (no facets): the wire branch.
    let wire = solid_body_with_display(
        box_brep(1.0, 1.0, 1.0),
        Mesh::default(),
        vec![vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)]],
    );
    let (_, report) = export_db_format(&db_with(vec![wire]), ExportFormat::Dwg);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "acis.dwg_3dsolid_unsupported"),
        "the wire branch names the DWG refusal: {:?}",
        report.entries
    );
    assert_eq!(report.counts.converted, 1, "one geometry, one outcome");

    // Nothing drawable at all: the drop branch.
    let empty = solid_body_with_display(box_brep(1.0, 1.0, 1.0), Mesh::default(), Vec::new());
    let (bytes, report) = export_db_format(&db_with(vec![empty]), ExportFormat::Dwg);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "acis.dwg_3dsolid_unsupported"),
        "the drop branch names the DWG refusal: {:?}",
        report.entries
    );
    assert_eq!(report.counts.dropped, 1, "one geometry, one outcome");
    assert!(
        DwgReader::from_stream(Cursor::new(bytes))
            .read()
            .expect("the DWG reads back")
            .model_space_entities()
            .next()
            .is_none(),
        "a body with nothing to draw writes no entity"
    );
}
