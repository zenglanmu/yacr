//! Export contract tests.
//!
//! Writer + our reader agreement is **internal consistency**, not third-party
//! compatibility: nothing here claims AutoCAD, ODA or any other consumer opens
//! the output.

use std::io::Cursor;
use std::sync::Arc;

use acadrust::{CadDocument, DwgReader, DxfReader, DxfWriter, EntityType, Transparency, Vector3};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder,
    DynamicBlockState, DynamicBlockVisibility, EntityLineType, EntityRenderAttributes,
    EntityTransparency, Layer, LineType as DbLineType, LinetypePattern, Style,
};
use cad_domain::*;

use super::report::{ExportCounts, ExportFormat, ExportLimits, ExportReport, ExportSpace};
use super::{export, AcisVersion, DxfTarget, ExportRequest};
use crate::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

mod acis;
mod acis_export;
mod acis_fixtures;
mod acis_version_matrix;
mod body3d;
mod dimension;
mod dimension_radial;
mod dwg;
mod entities;
mod hatch;
mod header_points_scales;
mod insert_blocks;
mod insert_depth;
mod layers;
mod linetype_scale;
mod linetype_table;
mod mtext_spacing;
mod point_style;
mod report_limits;
mod solid;
mod writer;
mod xref;

/// Import bytes through the single acadrust boundary (test-local helper).
fn import_bytes(bytes: &[u8]) -> DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
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
    export_db_format(db, ExportFormat::DxfText)
}

/// Import bytes and keep the unit context the importer derived (test-local).
fn import_drawing_with_units(bytes: &[u8]) -> (DrawingDatabase, UnitContext) {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("fixture imports");
    (imported.database, imported.units)
}

/// Export a database with an explicit unit context (test-local).
fn export_with_units(db: &DrawingDatabase, units: UnitContext) -> (Vec<u8>, ExportReport) {
    export(&ExportRequest {
        database: db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        acis_version: AcisVersion::default(),
        dxf_target: DxfTarget::default(),
        limits: ExportLimits::default(),
        units: Some(units),
    })
    .unwrap()
}

fn export_db_format(db: &DrawingDatabase, format: ExportFormat) -> (Vec<u8>, ExportReport) {
    export(&ExportRequest {
        database: db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format,
        // A DWG's container version is fixed at AC1032 by the upstream writer, so
        // the request names the DWG's own container; the DXF target only selects
        // the DXF container (default R2010).
        acis_version: AcisVersion::default(),
        dxf_target: match format {
            ExportFormat::DxfText => DxfTarget::default(),
            ExportFormat::Dwg => DxfTarget::R2018,
        },
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

/// DXF group-code/value pairs of a text export, in file order.
///
/// The text writer emits `code\r\nvalue\r\n` pairs, so splitting on the line
/// breaks and pairing them is enough to inspect what the writer actually wrote
/// (no re-parsing through our own reader).
fn dxf_pairs(bytes: &[u8]) -> Vec<(i32, String)> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    lines
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|pair| {
            let code: i32 = pair[0].trim().parse().ok()?;
            Some((code, pair[1].trim().to_string()))
        })
        .collect()
}

/// The `(code, value)` pair of one `$HEADER` variable, from the raw bytes.
///
/// Header variables are `9` / `$NAME` followed by one or more value pairs, so
/// the scan stops at the next variable (code `9`) or the section's `0 / ENDSEC`.
fn header_variable_pairs(bytes: &[u8], name: &str) -> Vec<(i32, String)> {
    let pairs = dxf_pairs(bytes);
    let Some(start) = pairs
        .iter()
        .position(|(code, value)| *code == 9 && value == name)
    else {
        panic!("no {name} header variable in the export");
    };
    pairs[start + 1..]
        .iter()
        .take_while(|(code, _)| *code != 9 && *code != 0)
        .cloned()
        .collect()
}

/// The `(code, value)` pairs written inside the first entity record named
/// `record_name` (the `0/<name>` pair itself excluded).
fn record_pairs_of(bytes: &[u8], record_name: &str) -> Vec<(i32, String)> {
    let pairs = dxf_pairs(bytes);
    let Some(start) = pairs
        .iter()
        .position(|(code, value)| *code == 0 && value == record_name)
    else {
        panic!("no {record_name} record in the export");
    };
    pairs[start + 1..]
        .iter()
        .take_while(|(code, _)| *code != 0)
        .cloned()
        .collect()
}

/// The `10/20/30`, `11/21/31`, `12/22/32`, `13/23/33` points written inside the
/// first entity record named `record_name`, read straight from the bytes.
fn record_corner_points(bytes: &[u8], record_name: &str) -> Vec<(i32, [f64; 3])> {
    let record = record_pairs_of(bytes, record_name);
    let mut corners = Vec::new();
    let mut index = 0;
    while index < record.len() {
        let (code, _) = &record[index];
        if matches!(code, 10..=13) {
            let coordinates = |offset: i32| -> f64 {
                record
                    .get(index + offset as usize)
                    .and_then(|(group, text)| (*group == *code + 10 * offset).then_some(text))
                    .and_then(|text| text.parse().ok())
                    .unwrap_or_else(|| panic!("group {code} at pair {index} is not a 3D point"))
            };
            corners.push((*code, [coordinates(0), coordinates(1), coordinates(2)]));
        }
        index += 1;
    }
    corners
}

fn assert_point(actual: Point3, expected: Point3) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "point {actual:?} != {expected:?}"
    );
}
