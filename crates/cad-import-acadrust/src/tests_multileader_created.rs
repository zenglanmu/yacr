//! The first export round trip of a **created** MULTILEADER (ADR 0019 decision
//! F.2, P7c).
//!
//! Unlike the P7b-2 tests (which start from an imported/synthesised definition),
//! this one starts where the `MLEADER` command starts: the pinned `MLeaderStyle`
//! table row, resolved into the creation snapshot, turned into a definition by
//! [`MultiLeaderGeometry::created`] — the **same** constructor the command calls.
//! It then writes the drawing, reads the bytes back with **acadrust's own
//! reader** and asserts the real `MULTILEADER` record, and finally re-imports the
//! bytes to prove the written `MLEADERSTYLE` record feeds the `cad-db` table.
//!
//! This test cannot live in `cad-app` (the application crate depends on this one
//! and may not use acadrust directly — `check-architecture.py`), so it lives here
//! and calls the same domain constructor; the command-level assertions (one
//! transaction, active layer, `AcDbMultiLeader` identity) are in
//! `cad-app`'s `tests::drawing::mleader_create`.

use std::io::Cursor;
use std::sync::Arc;

use acadrust::entities::LeaderContentType;
use acadrust::objects::ObjectType;
use acadrust::{DxfReader, EntityType};

use crate::export::{self, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use crate::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, MLeaderStyle};
use cad_domain::MLeaderStyleId;
use cad_domain::*;

const ARROW: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};
const LANDING: Point3 = Point3 {
    x: 10.0,
    y: 2.0,
    z: 0.0,
};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The definition the `MLEADER` command creates: the pinned style row resolved
/// into the creation snapshot, then `MultiLeaderGeometry::created`.
fn created_geometry() -> (MLeaderStyleValues, MultiLeaderGeometry) {
    let row = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    let resolved = row.resolved("Standard".to_string(), StyleId(0), None);
    let geometry = MultiLeaderGeometry::created(
        resolved.values.clone(),
        "Standard".to_string(),
        resolved.left_attachment,
        ARROW,
        LANDING,
        "Created".to_string(),
    )
    .expect("the creation path builds the definition");
    (resolved.values, geometry)
}

fn database_with(geometry: MultiLeaderGeometry) -> cad_db::DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
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
            geometry: SemanticGeometry::MultiLeader(Box::new(geometry)),
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn export_bytes(database: &cad_db::DrawingDatabase) -> Vec<u8> {
    let (bytes, _report) = export::export(&ExportRequest {
        database,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the created leader exports");
    bytes
}

fn import_bytes(bytes: &[u8]) -> cad_db::DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("re-import")
        .database
}

#[test]
fn a_created_multileader_writes_a_real_multileader_acadrust_can_read() {
    let (snapshot, geometry) = created_geometry();
    assert_eq!(snapshot.landing_distance, 0.36);
    assert!(approx(snapshot.landing_gap, 0.09));
    assert!(approx(snapshot.arrowhead_size, 0.18));
    assert!(approx(snapshot.text_height, 0.18));
    let database = database_with(geometry);
    let bytes = export_bytes(&database);

    // ─── acadrust reads the written file ───
    let document = DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("the written DXF parses");
    let written = document
        .entities()
        .find_map(|entity| match entity {
            EntityType::MultiLeader(multileader) => Some(multileader.as_ref()),
            _ => None,
        })
        .expect("one real MULTILEADER entity (never a flattened display)");

    // Our mask: a created leader overrides nothing.
    assert_eq!(written.property_override_flags.bits(), 0);
    assert_eq!(written.content_type, LeaderContentType::MText);
    assert!(approx(written.text_height, 0.18));
    assert!(approx(written.dogleg_length, 0.36));
    assert!(approx(written.arrowhead_size, 0.18));
    assert!(approx(written.scale_factor, 1.0));

    // The content block: the text, its placement and the style values it uses.
    let context = &written.context;
    assert_eq!(context.text_string, "Created");
    assert!(approx(context.text_height, 0.18));
    assert!(approx(context.landing_gap, 0.09));
    assert!(approx(context.arrowhead_size, 0.18));
    assert!(approx(context.text_location.x, LANDING.x));
    assert!(approx(context.text_location.y, LANDING.y));
    assert!(context.has_text_contents);

    // One root, one line, the two created vertices.
    assert_eq!(context.leader_roots.len(), 1);
    let root = &context.leader_roots[0];
    assert!(approx(root.connection_point.x, LANDING.x));
    assert!(approx(root.connection_point.y, LANDING.y));
    assert!(approx(root.direction.x, 1.0));
    assert!(approx(root.direction.y, 0.0));
    assert!(approx(root.landing_distance, 0.36));
    assert_eq!(root.lines.len(), 1);
    let line = &root.lines[0];
    assert_eq!(line.points.len(), 2, "exactly the two created vertices");
    assert!(approx(line.points[0].x, ARROW.x));
    assert!(approx(line.points[0].y, ARROW.y));
    // The derived dogleg start: landing - (dogleg length · +X).
    assert!(approx(line.points[1].x, LANDING.x - 0.36));
    assert!(approx(line.points[1].y, LANDING.y));
    assert_eq!(line.index, 0);

    // The attachment pair and the style record the entity points at.
    assert_eq!(
        written.text_left_attachment as i16, 1,
        "the table's vertical attachment"
    );
    assert_eq!(context.text_left_attachment as i16, 1);
    let style_handle = written.style_handle.expect("340 points at a style record");
    let style = document
        .objects
        .get(&style_handle)
        .and_then(|object| match object {
            ObjectType::MultiLeaderStyle(style) => Some(style),
            _ => None,
        })
        .expect("the referenced MLEADERSTYLE object is readable");
    assert_eq!(style.name, "Standard");
    assert!(approx(style.landing_distance, 0.36));
    assert!(approx(style.landing_gap, 0.09));
    assert!(approx(style.arrowhead_size, 0.18));
    assert!(approx(style.text_height, 0.18));
}

#[test]
fn the_written_style_record_feeds_the_mleader_style_table_on_reimport() {
    let (snapshot, geometry) = created_geometry();
    let bytes = export_bytes(&database_with(geometry));

    let reimported = import_bytes(&bytes);
    // The MLEADERSTYLE record is now a table row (P7c): the import no longer
    // drops it after resolving one entity's snapshot.
    let row = reimported
        .mleader_style_by_name("Standard")
        .expect("the written style record is imported into the table");
    assert_eq!(row.content_type, MLeaderStyle::CONTENT_MTEXT);
    assert_eq!(row.path_type, MLeaderPathType::StraightLineSegments);
    assert!(approx(row.dogleg_length, snapshot.landing_distance));
    assert!(approx(row.landing_gap, snapshot.landing_gap));
    assert!(approx(row.arrowhead_size, snapshot.arrowhead_size));
    assert!(approx(row.text_height, snapshot.text_height));
    assert_eq!(row.left_attachment, 1);
    assert_eq!(row.text_style_name, "Standard");

    // And that row resolves to the same effective snapshot the command stores.
    let resolved = reimported
        .resolve_mleader_style("Standard")
        .expect("the imported row resolves");
    assert!(approx(
        resolved.values.landing_distance,
        snapshot.landing_distance
    ));
    assert!(approx(resolved.values.landing_gap, snapshot.landing_gap));
    assert!(approx(
        resolved.values.arrowhead_size,
        snapshot.arrowhead_size
    ));
    assert!(approx(resolved.values.text_height, snapshot.text_height));
    assert_eq!(resolved.left_attachment, 1);
    // The stored leader re-imports as the same MULTILEADER variant.
    let leader = reimported
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref()),
            _ => None,
        })
        .expect("the created leader re-imports");
    assert_eq!(leader.override_mask(), 0);
    assert_eq!(leader.text().string, "Created");
    assert_eq!(leader.lines()[0].points.len(), 2);
    assert!(approx(leader.lines()[0].points[1].x, LANDING.x - 0.36));
}
