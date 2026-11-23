//! XREF export refusal: a resolved attachment is never silently flattened.
//!
//! Import merges a resolved xref's model entities into the host xref block. v1
//! DXF export cannot write an xref attachment record or its path, so expanding
//! that block would turn the attachment into ordinary geometry with no record
//! of what it was. These tests pin the refusal and prove a normal block still
//! expands as before.

use super::*;

use crate::export::report::ExportDiagnostic;
use cad_db::{ExternalReference, ExternalReferenceStatus};

/// Tables every synthetic database needs (layer `0` + `Standard` text style).
fn builder_with_tables() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
}

fn entity(id: u128, geometry: SemanticGeometry, space: SpaceId, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "test".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order,
    }
}

fn insert_at(block: BlockId, x: f64) -> SemanticGeometry {
    let mut transform = Transform3::identity();
    transform.matrix[0][3] = x;
    SemanticGeometry::Insert { block, transform }
}

fn xref(
    name: &str,
    raw_path: &str,
    block: BlockId,
    status: ExternalReferenceStatus,
) -> ExternalReference {
    ExternalReference {
        name: name.into(),
        raw_path: raw_path.into(),
        resolved_path: Some(raw_path.into()),
        block: Some(block),
        overlay: false,
        depth: 0,
        status,
        reason: None,
    }
}

fn entries_with_code<'a>(report: &'a ExportReport, code: &str) -> Vec<&'a ExportDiagnostic> {
    report
        .entries
        .iter()
        .filter(|entry| entry.code == code)
        .collect()
}

#[test]
fn resolved_xref_insert_is_refused_and_never_flattened() {
    let mut builder = builder_with_tables();

    // A normal host block: member line (0,0)-(1,0), inserted at x = 10.
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(1, insert_at(BlockId(1), 10.0), SpaceId::Model, 0))
        .unwrap();

    // The xref block: its member is resolved external content, with a unique
    // coordinate so flattening it would be unmistakable in the output.
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            3,
            SemanticGeometry::Line {
                start: p(1000.0, 1000.0, 0.0),
                end: p(1001.0, 1000.0, 0.0),
            },
            SpaceId::Block(BlockId(2)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(4, insert_at(BlockId(2), 0.0), SpaceId::Model, 1))
        .unwrap();
    builder
        .insert_external_reference(xref(
            "SITE.dwg",
            "/refs/site.dwg",
            BlockId(2),
            ExternalReferenceStatus::Loaded,
        ))
        .unwrap();

    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);

    // The xref attachment is reported as dropped, with its identity, path and
    // status, and the reason the v1 writer cannot preserve it.
    let dropped = entries_with_code(&report, "export.xref_content_dropped");
    assert_eq!(dropped.len(), 1, "{:?}", report.entries);
    assert_eq!(dropped[0].object, Some(ObjectId(4)));
    let message = &dropped[0].message;
    assert!(message.contains("SITE.dwg"), "{message}");
    assert!(message.contains("/refs/site.dwg"), "{message}");
    assert!(message.contains("loaded"), "{message}");
    assert!(
        message.contains("no xref attachment record or path"),
        "{message}"
    );

    // The xref block is handled by the INSERT refusal, never re-reported as a
    // generic unreferenced definition, and no success entry mentions the xref
    // object.
    assert!(entries_with_code(&report, "export.dropped.unreferenced_block").is_empty());
    assert!(
        entries_with_code(&report, "export.xref_definition_not_written").is_empty(),
        "{:?}",
        report.entries
    );
    for entry in report
        .entries
        .iter()
        .filter(|e| e.object == Some(ObjectId(4)))
    {
        assert_eq!(entry.code, "export.xref_content_dropped");
    }
    assert_eq!(report.counts.dropped, 1);
    // A drop means the save cannot be called complete/loss-free.
    assert_ne!(report.completeness, Completeness::Complete);
    assert!(matches!(report.completeness, Completeness::Missing(_)));

    // The normal block still expands exactly as before.
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1, "only the normal block's LINE is written");
    match &entities[0] {
        EntityType::Line(line) => {
            assert_eq!(line.start, Vector3::new(10.0, 0.0, 0.0));
            assert_eq!(line.end, Vector3::new(11.0, 0.0, 0.0));
        }
        other => panic!("expected the normal block's exploded LINE, got {other:?}"),
    }
    assert!(entries_with_code(&report, "export.converted.insert")
        .iter()
        .all(|entry| entry.object == Some(ObjectId(1))));

    // No entity from the xref member reaches the file, and nothing claims the
    // attachment was preserved.
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF is valid UTF-8");
    assert!(
        !text.contains("SITE.dwg"),
        "the xref name must not appear as a written block/record"
    );
    assert!(!text.contains("1000.0"), "xref member geometry leaked");
}

#[test]
fn missing_or_unloaded_xref_without_members_still_reports_the_attachment() {
    for status in [
        ExternalReferenceStatus::Missing,
        ExternalReferenceStatus::Unloaded,
    ] {
        let mut builder = builder_with_tables();
        // No materialised members: an unloaded/missing reference has none.
        builder
            .insert_block(BlockDefinition {
                id: BlockId(1),
                entities: Vec::new(),
                dynamic_visibility: None,
            })
            .unwrap();
        builder
            .insert_entity(entity(1, insert_at(BlockId(1), 0.0), SpaceId::Model, 0))
            .unwrap();
        builder
            .insert_external_reference(xref("PLAN.dwg", "/refs/plan.dwg", BlockId(1), status))
            .unwrap();

        let db = builder.finish().unwrap();
        let (bytes, report) = export_db(&db);

        assert!(read_entities(&bytes).is_empty());
        let dropped = entries_with_code(&report, "export.xref_content_dropped");
        assert_eq!(dropped.len(), 1, "{status:?}: {:?}", report.entries);
        assert_eq!(dropped[0].object, Some(ObjectId(1)));
        let message = &dropped[0].message;
        assert!(message.contains("PLAN.dwg"), "{message}");
        assert!(message.contains("/refs/plan.dwg"), "{message}");
        assert!(message.contains(status.as_str()), "{message}");
        assert_eq!(report.counts.dropped, 1, "{status:?}");
        assert_ne!(report.completeness, Completeness::Complete, "{status:?}");
        assert!(entries_with_code(&report, "export.dropped.unreferenced_block").is_empty());
    }
}

#[test]
fn xref_block_without_an_insert_is_reported_as_an_xref_definition_not_written() {
    let mut builder = builder_with_tables();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_external_reference(xref(
            "DETAIL.dwg",
            "/refs/detail.dwg",
            BlockId(1),
            ExternalReferenceStatus::Missing,
        ))
        .unwrap();

    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);

    assert!(read_entities(&bytes).is_empty());
    let dropped = entries_with_code(&report, "export.xref_definition_not_written");
    assert_eq!(dropped.len(), 1, "{:?}", report.entries);
    let message = &dropped[0].message;
    assert!(message.contains("DETAIL.dwg"), "{message}");
    assert!(message.contains("/refs/detail.dwg"), "{message}");
    assert!(message.contains("missing"), "{message}");
    // Never the generic "unreferenced block definition" drop.
    assert!(entries_with_code(&report, "export.dropped.unreferenced_block").is_empty());
    assert_eq!(report.counts.dropped, 1);
    assert_ne!(report.completeness, Completeness::Complete);
}
