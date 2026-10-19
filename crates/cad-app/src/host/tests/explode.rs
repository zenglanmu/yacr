//! EXPLODE production-save export contract (ADR 0016 decision G).
//!
//! The exploded products (polyline segments and bulge ARCs) must survive the real
//! production save path (`HostController::export_dxf`).

use super::*;
use crate::CommandPayload;

fn entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: crate::app_drawing::class_name(&geometry).to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn reference(controller: &HostController, entity: u128) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A drawing with one closed straight polyline (entity 201).
fn straight_fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            201,
            SemanticGeometry::Polyline {
                points: vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 3.0)],
                bulges: vec![0.0, 0.0, 0.0],
                closed: true,
            },
            0,
        ))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller.set_snap_enabled(false);
    controller
}

/// A drawing with one bulge polyline (entity 201): a unit-radius semicircle.
fn bulge_fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            201,
            SemanticGeometry::Polyline {
                points: vec![p(0.0, 0.0), p(2.0, 0.0)],
                bulges: vec![1.0],
                closed: false,
            },
            0,
        ))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller.set_snap_enabled(false);
    controller
}

/// Run EXPLODE on the current selection through the ordinary command path.
fn explode(controller: &mut HostController, entity: u128) -> CommandOutcome {
    controller
        .set_selection(vec![reference(controller, entity)])
        .unwrap();
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    controller
        .execute(crate::Command {
            schema_version: 1,
            id: crate::CommandId::ExplodeEntities,
            document,
            viewport,
            payload: CommandPayload::None,
        })
        .unwrap()
}

/// Split a DXF text document into `(group code, value)` pairs.
fn dxf_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < lines.len() {
        let code: i32 = lines[index]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("group code expected at line {index}: {:?}", lines[index]));
        pairs.push((code, lines[index + 1].trim().to_string()));
        index += 2;
    }
    pairs
}

/// The `(code, value)` pairs of every `name` entity record.
fn dxf_entities(pairs: &[(i32, String)], name: &str) -> Vec<Vec<(i32, String)>> {
    let mut records = Vec::new();
    let mut current: Option<Vec<(i32, String)>> = None;
    for (code, value) in pairs {
        if *code == 0 {
            if let Some(record) = current.take() {
                records.push(record);
            }
            current = (value == name).then(Vec::new);
            continue;
        }
        if let Some(record) = current.as_mut() {
            record.push((*code, value.clone()));
        }
    }
    if let Some(record) = current.take() {
        records.push(record);
    }
    records
}

/// The single numeric value of `code` in an entity record.
fn group(record: &[(i32, String)], code: i32) -> f64 {
    record
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, value)| {
            value
                .parse::<f64>()
                .unwrap_or_else(|_| panic!("numeric group {code}: {value:?}"))
        })
        .unwrap_or_else(|| panic!("no group code {code} in {record:?}"))
}

/// Whether a DXF LINE record runs from `(x0, y0)` to `(x1, y1)` (either order).
fn dxf_line_is(record: &[(i32, String)], x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    let start = (group(record, 10), group(record, 20));
    let end = (group(record, 11), group(record, 21));
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    (near(start, (x0, y0)) && near(end, (x1, y1))) || (near(start, (x1, y1)) && near(end, (x0, y0)))
}

#[test]
fn exploded_polyline_segments_survive_the_production_save_path() {
    let mut controller = straight_fixture();
    let outcome = explode(&mut controller, 201);
    assert_eq!(
        outcome.objects.len(),
        3,
        "a closed triangle explodes to three LINEs"
    );

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let lines = dxf_entities(&dxf_pairs(&text), "LINE");
    assert_eq!(lines.len(), 3, "the three exploded segments are exported");
    assert!(lines.iter().any(|r| dxf_line_is(r, 0.0, 0.0, 4.0, 0.0)));
    assert!(lines.iter().any(|r| dxf_line_is(r, 4.0, 0.0, 4.0, 3.0)));
    assert!(
        lines.iter().any(|r| dxf_line_is(r, 4.0, 3.0, 0.0, 0.0)),
        "the closing segment is exported"
    );
}

#[test]
fn exploded_bulge_arc_survives_the_production_save_path() {
    let mut controller = bulge_fixture();
    let outcome = explode(&mut controller, 201);
    assert_eq!(
        outcome.objects.len(),
        1,
        "one bulge segment becomes one ARC"
    );

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let arcs = dxf_entities(&dxf_pairs(&text), "ARC");
    assert_eq!(arcs.len(), 1, "exactly one ARC for the one bulge segment");
    let record = &arcs[0];
    assert!((group(record, 10) - 1.0).abs() < 1e-6, "centre x");
    assert!(group(record, 20).abs() < 1e-6, "centre y");
    assert!((group(record, 40) - 1.0).abs() < 1e-6, "radius");
}
