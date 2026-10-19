//! STRETCH interactive capture, wiring and production-save export contracts.
//!
//! ADR 0016 decision E: STRETCH captures the two crossing-window corners plus a
//! base and a displacement point (`Exact(4)`, no entity picks), and the stretched
//! geometry must survive the real production save path
//! (`HostController::export_dxf`).

use super::*;
use crate::{CommandPayload, DrawIntent};

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

/// A drawing with one LINE `(0,0)-(10,0)` (id 201) on layer 0.
fn fixture() -> HostController {
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
            SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(10.0, 0.0),
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

fn undo_depth(controller: &HostController) -> usize {
    controller
        .application
        .history
        .get(&controller.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

/// The four captured points: the window corners, the base and the displacement
/// point. The window `(-1,-1)-(2,1)` contains the line's start only, so the line
/// stretches to `(0,5)-(10,0)`.
fn capture_four(controller: &mut HostController) {
    for point in [p(-1.0, -1.0), p(2.0, 1.0), p(0.0, 0.0), p(0.0, 5.0)] {
        controller.capture_tool_point(point).unwrap();
    }
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
fn stretch_capture_maps_the_four_points_to_the_payload() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Stretch).unwrap();
    capture_four(&mut controller);
    // Capture is intent, not a transaction.
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::Stretch {
            rect: [p(-1.0, -1.0), p(2.0, 1.0)],
            base: p(0.0, 0.0),
            delta: p(0.0, 5.0),
        }
    );
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::StretchEntities);
    match command.payload {
        CommandPayload::Stretch {
            rect_logical,
            base,
            delta,
        } => {
            assert_eq!(base, p(0.0, 0.0));
            assert_eq!(delta, p(0.0, 5.0));
            // The window corners travel as canvas logical pixels; mapping them
            // back through the same plan view recovers the captured world corners.
            let viewport = controller
                .application
                .workspace
                .viewports
                .get(&controller.viewport_id)
                .unwrap();
            let size = viewport.logical_size;
            for (logical, world) in rect_logical.iter().zip([p(-1.0, -1.0), p(2.0, 1.0)]) {
                let back = viewport.screen_to_world(*logical, size).unwrap();
                assert!(
                    (back.x - world.x).abs() < 1e-9 && (back.y - world.y).abs() < 1e-9,
                    "{logical:?} -> {back:?}, expected {world:?}"
                );
            }
        }
        _ => panic!("expected a Stretch payload"),
    }
}

#[test]
fn stretch_result_survives_the_production_save_path() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller.begin_draw_tool(DrawToolKind::Stretch).unwrap();
    capture_four(&mut controller);
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(
        outcome.objects.len(),
        1,
        "the stretched LINE changed in place"
    );
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let lines = dxf_entities(&dxf_pairs(&text), "LINE");
    assert_eq!(lines.len(), 1, "the one stretched LINE");
    assert!(
        lines
            .iter()
            .any(|record| dxf_line_is(record, 0.0, 5.0, 10.0, 0.0)),
        "the stretched LINE (0,5)-(10,0) must be exported"
    );
}

#[test]
fn stretch_is_work_only_and_refused_in_viewer() {
    let mut controller = fixture();
    controller.set_mode(AppMode::Viewer).unwrap();
    let error = controller
        .begin_draw_tool(DrawToolKind::Stretch)
        .expect_err("STRETCH is Work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
}
