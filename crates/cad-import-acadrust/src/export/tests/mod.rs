//! Export contract tests.
//!
//! Writer + our reader agreement is **internal consistency**, not third-party
//! compatibility: nothing here claims AutoCAD, ODA or any other consumer opens
//! the output.

use std::io::Cursor;
use std::sync::Arc;

use acadrust::{CadDocument, DxfReader, DxfWriter, EntityType, Transparency, Vector3};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder,
    DynamicBlockState, DynamicBlockVisibility, EntityLineType, EntityRenderAttributes,
    EntityTransparency, Layer, LineType as DbLineType, LinetypePattern, Style,
};
use cad_domain::*;

use super::report::{ExportCounts, ExportFormat, ExportLimits, ExportReport, ExportSpace};
use super::{export, ExportRequest};
use crate::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

mod entities;
mod report_limits;
mod writer;

/// Import bytes through the single acadrust boundary (test-local helper).
fn import_bytes(bytes: &[u8]) -> DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("fixture imports")
        .database
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn db_with(geometries: Vec<SemanticGeometry>) -> DrawingDatabase {
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
    for (index, geometry) in geometries.into_iter().enumerate() {
        let id = index as u128 + 1;
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "test".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn export_db(db: &DrawingDatabase) -> (Vec<u8>, ExportReport) {
    export(&ExportRequest {
        database: db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .unwrap()
}

fn export_one(geometry: SemanticGeometry) -> (Vec<u8>, ExportReport) {
    export_db(&db_with(vec![geometry]))
}

fn read_entities(bytes: &[u8]) -> Vec<EntityType> {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .unwrap()
        .read()
        .unwrap()
        .entities()
        .cloned()
        .collect()
}

fn assert_point(actual: Point3, expected: Point3) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "point {actual:?} != {expected:?}"
    );
}
