//! Interim MULTILEADER export tests (P7b-1 stopgap).
//!
//! The real MULTILEADER writer is P7b-2; until then `export/entity.rs` flattens
//! the derived display and must say so in the report.

use std::io::Cursor;

use acadrust::{DxfReader, EntityType};
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};

use super::*;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn sample_multileader() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 0.3,
            landing_gap: 0.05,
            arrowhead_size: 0.2,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "Note".into(),
            location: p(10.0, 0.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 1,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(10.0, 0.0, 0.0),
            dogleg_vector: p(1.0, 0.0, 0.0),
            dogleg_length: 0.3,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.2,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

fn db_with(geometry: SemanticGeometry) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(cad_db::Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbMultiLeader".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn first_class_multileader_kind_name_is_multileader() {
    let geometry = SemanticGeometry::MultiLeader(Box::new(sample_multileader()));
    assert_eq!(crate::export::kind_name(&geometry), "MultiLeader");
}

#[test]
fn interim_multileader_export_flattens_the_display_and_declares_the_stopgap() {
    let database = db_with(SemanticGeometry::MultiLeader(
        Box::new(sample_multileader()),
    ));
    let (bytes, report) = export::export(&export::ExportRequest {
        database: &database,
        document_id: DocumentId(1),
        space: export::ExportSpace::Model,
        format: export::ExportFormat::DxfText,
        limits: export::ExportLimits::default(),
        units: None,
    })
    .expect("export runs");

    // The display is flattened into its primitives, not dropped: the report
    // declares the conversion and the output is not empty.
    assert!(!bytes.is_empty());
    assert!(
        report.entries.iter().any(|entry| {
            entry.code == "export.converted.multileader" && entry.message.contains("P7b-2")
        }),
        "{:?}",
        report.entries
    );
    let entities: Vec<EntityType> = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap()
        .entities()
        .cloned()
        .collect();
    assert!(
        !entities
            .iter()
            .any(|entity| matches!(entity, EntityType::MultiLeader(_))),
        "the interim stopgap must not emit a MULTILEADER entity yet"
    );
    // Leader line + arrowhead + dogleg + text: more than one output entity.
    assert!(entities.len() > 1, "{entities:?}");
}
