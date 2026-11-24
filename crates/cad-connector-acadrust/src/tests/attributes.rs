//! Block attribute import/export contracts (ADR-0055 M2).
//!
//! The fixture is a synthetic document written by acadrust's own DXF writer, not
//! a vendored sample: it carries one `ATTDEF` block member and one `INSERT`
//! whose `ATTRIB` child fills it. Our importer must keep the definition as an
//! `AttributeDefinition` (never a flattened `TEXT`), store the value block-local
//! on the reference, and the exporter must write both back so a re-import is
//! equal.
//!
//! These are writer/reader agreement tests (internal consistency), not a claim
//! that AutoCAD or ODA opens the output.

use super::*;

/// The block definition's local frame: the definition and the value share it.
const ATTDEF_POSITION: (f64, f64) = (1.5, 2.5);

fn fixture_document() -> acadrust::CadDocument {
    fixture_with_values(&[("PART", "PN-42")])
}

fn fixture_with_values(values: &[(&str, &str)]) -> acadrust::CadDocument {
    let mut doc = acadrust::CadDocument::new();

    let block_handle = doc.allocate_handle();
    let mut record = acadrust::tables::BlockRecord::new("TAGGED");
    record.handle = block_handle;
    doc.block_records.add(record).expect("add block record");
    // The attribute value's own layer must exist in the table, or the importer
    // correctly falls back to layer "0" (it never invents a layer row).
    doc.layers
        .add_or_replace(acadrust::tables::Layer::new("ATTRIBS"));

    let mut definition = acadrust::entities::AttributeDefinition::new(
        "PART".into(),
        "Part number?".into(),
        "PN-0001".into(),
    );
    definition.common.owner_handle = block_handle;
    definition.common.layer = "ATTRIBS".into();
    definition.insertion_point =
        acadrust::types::Vector3::new(ATTDEF_POSITION.0, ATTDEF_POSITION.1, 0.0);
    definition.alignment_point =
        acadrust::types::Vector3::new(ATTDEF_POSITION.0, ATTDEF_POSITION.1, 0.0);
    definition.height = 2.5;
    definition.rotation = 0.3;
    definition.width_factor = 0.8;
    definition.oblique_angle = 0.15;
    definition.horizontal_alignment = acadrust::entities::HorizontalAlignment::Center;
    definition.vertical_alignment = acadrust::entities::VerticalAlignment::Middle;
    definition.flags.invisible = true;
    definition.flags.verify = true;
    definition.text_style = "Standard".into();
    doc.add_entity(EntityType::AttributeDefinition(definition))
        .expect("add ATTDEF member");

    let mut insert =
        acadrust::entities::Insert::new("TAGGED", acadrust::types::Vector3::new(10.0, 20.0, 0.0));
    insert.common.layer = "0".into();
    for (tag, text) in values {
        let mut value = acadrust::entities::AttributeEntity::new((*tag).into(), (*text).into());
        value.common.layer = "ATTRIBS".into();
        value.insertion_point =
            acadrust::types::Vector3::new(ATTDEF_POSITION.0, ATTDEF_POSITION.1, 0.0);
        value.alignment_point =
            acadrust::types::Vector3::new(ATTDEF_POSITION.0, ATTDEF_POSITION.1, 0.0);
        value.height = 2.5;
        value.rotation = 0.3;
        value.width_factor = 0.8;
        value.oblique_angle = 0.15;
        value.horizontal_alignment = acadrust::entities::HorizontalAlignment::Center;
        value.vertical_alignment = acadrust::entities::VerticalAlignment::Middle;
        value.flags.invisible = true;
        value.text_style = "Standard".into();
        insert.attributes.push(value);
    }
    doc.add_entity(EntityType::Insert(insert))
        .expect("add INSERT");
    doc
}

fn import(bytes: Vec<u8>) -> cad_db::DrawingDatabase {
    AcadrustImporter::default()
        .import(&request(bytes), &|| false)
        .expect("the synthetic DXF imports")
        .database
}

fn definition_of(
    db: &cad_db::DrawingDatabase,
    block: BlockId,
) -> (&AttributeDefinition, cad_domain::EntityId) {
    db.block_entities(block)
        .into_iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::AttributeDefinition(definition) => {
                Some((definition.as_ref(), entity.id))
            }
            _ => None,
        })
        .expect("the block owns one ATTDEF")
}

fn insert_of(db: &cad_db::DrawingDatabase) -> (&SemanticGeometry, Vec<AttributeValue>) {
    let mut found = None;
    for entity in db.entities() {
        if let SemanticGeometry::Insert {
            block, attributes, ..
        } = &entity.geometry
        {
            if db.block_name(*block) == Some("TAGGED") {
                found = Some((&entity.geometry, attributes.clone()));
            }
        }
    }
    found.expect("the drawing owns one INSERT of TAGGED")
}

fn insert_id_of(db: &cad_db::DrawingDatabase) -> cad_domain::EntityId {
    db.entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Insert { block, .. } if db.block_name(*block) == Some("TAGGED") => {
                Some(entity.id)
            }
            _ => None,
        })
        .expect("the drawing owns one INSERT of TAGGED")
}

#[test]
fn attdef_and_insert_attrib_import_export_and_reimport_equal() {
    let source = acadrust::DxfWriter::new(&fixture_document())
        .write_to_vec()
        .expect("acadrust writes the fixture DXF");
    let db = import(source);

    let block = db
        .block_by_name("TAGGED")
        .expect("the block definition is imported");
    let (definition, definition_id) = definition_of(&db, block.id);
    assert_eq!(definition.tag, "PART");
    assert_eq!(definition.prompt, "Part number?");
    assert_eq!(definition.default_value, "PN-0001");
    assert!(definition.flags.invisible);
    assert!(definition.flags.verify);
    assert!(!definition.flags.constant);
    assert!((definition.text.height - 2.5).abs() < 1e-9);
    assert!((definition.text.rotation - 0.3).abs() < 1e-9);
    assert!((definition.text.width_factor - 0.8).abs() < 1e-9);
    assert!((definition.text.oblique - 0.15).abs() < 1e-9);
    assert_eq!(definition.text.h_align, TextAlignH::Center);
    assert_eq!(definition.text.v_align, TextAlignV::Middle);
    assert_eq!(definition.text.position.x, ATTDEF_POSITION.0);
    assert_eq!(definition.text.position.y, ATTDEF_POSITION.1);
    assert!(definition.source_handle.is_some());

    // The ATTDEF is **not** duplicated as a TEXT in the block definition.
    assert!(
        !db.block_entities(block.id)
            .into_iter()
            .any(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. })),
        "the ATTDEF must not be flattened into a TEXT member"
    );
    // The definition entity is the ATTDEF itself.
    assert_eq!(
        db.entity(definition_id)
            .map(|entity| entity.object.type_key.as_str()),
        Some("AcDbAttributeDefinition")
    );

    let (_, values) = insert_of(&db);
    assert_eq!(values.len(), 1, "the reference carries one attribute value");
    let value = &values[0];
    assert_eq!(value.tag, "PART");
    assert_eq!(value.value, "PN-42");
    assert!(value.flags.invisible);
    assert_eq!(value.text.h_align, TextAlignH::Center);
    assert_eq!(value.text.v_align, TextAlignV::Middle);
    assert!((value.text.height - 2.5).abs() < 1e-9);
    assert!((value.text.oblique - 0.15).abs() < 1e-9);
    assert!((value.text.position.x - ATTDEF_POSITION.0).abs() < 1e-9);
    assert!((value.text.position.y - ATTDEF_POSITION.1).abs() < 1e-9);
    assert_eq!(
        db.layer(value.layer).map(|layer| layer.name.as_str()),
        Some("ATTRIBS")
    );

    // Export and re-import. The whole point of M2 is that this is equal, not
    // just non-empty.
    let (bytes, report) = crate::export::export(&crate::export::ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        acis_version: crate::export::AcisVersion::default(),
        dxf_target: crate::export::DxfTarget::default(),
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "attribute_definition"),
        "the ATTDEF is written, not reported as dropped: {:?}",
        report.entries
    );
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF is UTF-8");
    assert!(text.contains("ATTDEF"), "the definition record is written");
    assert!(text.contains("ATTRIB"), "the value record is written");
    assert!(
        text.contains("SEQEND"),
        "the attribute sequence is terminated"
    );

    let round_trip = import(bytes.to_vec());
    let block = round_trip
        .block_by_name("TAGGED")
        .expect("the block survives the round trip");
    let (definition_again, _) = definition_of(&round_trip, block.id);
    // A re-export gets fresh handles, so identity-bearing source handles are
    // deliberately excluded: the semantic definition must be equal.
    assert_eq!(
        AttributeDefinition {
            source_handle: None,
            ..definition_again.clone()
        },
        AttributeDefinition {
            source_handle: None,
            ..definition.clone()
        }
    );
    let (_, values_again) = insert_of(&round_trip);
    assert_eq!(values_again.len(), 1);
    assert_eq!(
        AttributeValue {
            source_handle: None,
            ..values_again[0].clone()
        },
        AttributeValue {
            source_handle: None,
            ..values[0].clone()
        },
        "the value is equal after re-import"
    );
}

#[test]
fn orphan_attrib_keeps_text_fallback_and_reports_a_named_partial() {
    // An `ATTRIB` with no owning INSERT cannot be produced through
    // `CadDocument::add_entity` (it deliberately treats an ATTRIB as a
    // sub-entity and never writes it to a space), so the orphan is a raw DXF
    // record in ENTITIES — exactly the shape the reader must not guess an owner
    // for.
    let bytes = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n5\n20\n330\n1F\n8\n0\n2\nORPHANS\n70\n0\n10\n0.0\n20\n0.0\n30\n0.0\n3\nORPHANS\n\
0\nATTRIB\n5\n21\n330\n20\n8\n0\n100\nAcDbText\n\
10\n4.0\n20\n5.0\n30\n0.0\n40\n1.25\n1\nvisible\n50\n0.0\n41\n1.0\n51\n0.0\n7\nStandard\n\
71\n0\n72\n0\n11\n4.0\n21\n5.0\n31\n0.0\n210\n0.0\n220\n0.0\n230\n1.0\n\
100\nAcDbAttribute\n2\nLOOSE\n70\n0\n73\n0\n74\n0\n\
0\nENDBLK\n5\n22\n330\n20\n8\n0\n\
0\nENDSEC\n0\nEOF\n";
    let drawing = AcadrustImporter::default()
        .import(&request(bytes.to_vec()), &|| false)
        .expect("the orphan fixture imports");
    let db = &drawing.database;
    assert!(
        db.entities()
            .any(|entity| matches!(&entity.geometry, SemanticGeometry::Text { text, .. } if text == "visible")),
        "the orphan keeps its TEXT fallback"
    );
    assert!(
        drawing
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.orphan_attrib"),
        "the orphan is reported by name: {:?}",
        drawing.report.diagnostics
    );
}

#[test]
fn duplicate_tags_are_preserved_on_import() {
    let doc = fixture_with_values(&[("PART", "PN-42"), ("PART", "PN-43")]);
    let bytes = acadrust::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("write the duplicate-tag fixture");

    let db = import(bytes);
    let (_, values) = insert_of(&db);
    assert_eq!(values.len(), 2, "duplicate tags stay allowed: {values:?}");
    assert!(values.iter().all(|value| value.tag == "PART"));
    assert_eq!(values[0].value, "PN-42");
    assert_eq!(values[1].value, "PN-43");
}

#[test]
fn a_mirrored_reference_reports_dropped_attribute_values_by_name() {
    let source = acadrust::DxfWriter::new(&fixture_document())
        .write_to_vec()
        .expect("acadrust writes the fixture DXF");
    let mut db = import(source);

    // A mirrored placement is the documented representation refusal: it cannot be
    // one INSERT record, so the reference explodes and its values have nowhere to
    // attach. The export must name that loss, not drop it silently.
    let mut mirrored = Transform3::identity();
    mirrored.matrix[0][0] = -1.0;
    let mirrored = Transform3::translation(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
    .matrix_mul(&mirrored);
    let id = insert_id_of(&db);
    let mut entity = db.entity(id).expect("the INSERT exists").clone();
    if let SemanticGeometry::Insert { transform, .. } = &mut entity.geometry {
        *transform = mirrored;
    }
    let mut transaction = db
        .begin_drawing_transaction("test: mirror reference", TransactionId(5))
        .expect("open a transaction");
    transaction.update_entity(entity).expect("stage the mirror");
    transaction.commit().expect("commit the mirror");

    let (bytes, report) = crate::export::export(&crate::export::ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        acis_version: crate::export::AcisVersion::default(),
        dxf_target: crate::export::DxfTarget::default(),
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the mirrored drawing exports (exploded)");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.insert.attributes_dropped"),
        "the dropped reference-owned values are named: {:?}",
        report.entries
    );
    // Internal consistency only: the exploded members are still written.
    assert!(!bytes.is_empty());
}
