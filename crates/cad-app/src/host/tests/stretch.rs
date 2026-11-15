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

// --- the polygon/lasso crossing window through the real capture path ---------

/// Set the live `stretchWindow` mode through the one parameter channel (the same
/// entry the UI's capture-mode chooser uses) and start the STRETCH capture.
fn begin_stretch(controller: &mut HostController, mode: f64) {
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::StretchWindow,
            crate::ToolValue::Number(mode),
        )
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Stretch).unwrap();
}

/// The square crossing ring used by the polygon/lasso captures: it encloses only
/// the line's `(0,0)` endpoint, so the line stretches to `(0,5)-(10,0)`.
fn square_ring() -> Vec<Point3> {
    vec![p(-1.0, -1.0), p(2.0, -1.0), p(2.0, 1.0), p(-1.0, 1.0)]
}

#[test]
fn stretch_polygon_capture_reaches_the_command_through_the_tool() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    begin_stretch(&mut controller, crate::STRETCH_WINDOW_POLYGON);

    // The capture reports its polygon window step, not the rectangle's four picks.
    assert_eq!(
        controller.session.draw_window_step(),
        Some(crate::StretchWindowStep::PolygonVertex)
    );

    // Successive clicks add boundary vertices; none of them is a displacement
    // point, so the capture is not ready and no command can be built yet.
    for vertex in square_ring() {
        controller.capture_tool_point(vertex).unwrap();
    }
    assert_eq!(controller.session.draw_window_ring_len(), 4);
    assert!(controller.session.draw_window_step().is_some());
    let preview = controller
        .session
        .draw_preview()
        .expect("the capture is active");
    assert!(
        !preview.can_confirm(),
        "an open ring is not a defined window"
    );
    assert!(preview.is_open_ended(), "the ring accepts more vertices");
    assert!(preview.window_open, "the window step is the ring");

    // CONFIRM closes the ring; only then do the two displacement picks follow.
    controller.session.close_draw_window_ring().unwrap();
    assert_eq!(controller.session.draw_window_step(), None);
    for point in [p(0.0, 0.0), p(0.0, 5.0)] {
        controller.capture_tool_point(point).unwrap();
    }
    // Capture is intent, not a transaction.
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::StretchPolygon {
            ring: square_ring(),
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
        CommandPayload::StretchPolygon(polygon) => {
            assert_eq!(polygon.base, p(0.0, 0.0));
            assert_eq!(polygon.delta, p(0.0, 5.0));
            // The ring travels as canvas logical pixels; mapping it back through
            // the same plan view recovers the captured world vertices exactly.
            assert_eq!(polygon.polygon_logical.len(), 4);
            let viewport = controller
                .application
                .workspace
                .viewports
                .get(&controller.viewport_id)
                .unwrap();
            let size = viewport.logical_size;
            for (logical, world) in polygon.polygon_logical.iter().zip(square_ring()) {
                let back = viewport.screen_to_world(*logical, size).unwrap();
                assert!(
                    (back.x - world.x).abs() < 1e-9 && (back.y - world.y).abs() < 1e-9,
                    "{logical:?} -> {back:?}, expected {world:?}"
                );
            }
        }
        _ => panic!("expected a StretchPolygon payload"),
    }

    // One transaction / one undo step, and the line really stretched.
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(
        outcome.objects.len(),
        1,
        "the stretched LINE changed in place"
    );
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1)
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
    match controller
        .drawing()
        .unwrap()
        .entity(EntityId(201))
        .unwrap()
        .geometry
        .clone()
    {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(start, p(0.0, 5.0));
            assert_eq!(end, p(10.0, 0.0));
        }
        other => panic!("expected the line, got {other:?}"),
    }
}

#[test]
fn stretch_lasso_capture_reaches_the_same_planner() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    begin_stretch(&mut controller, crate::STRETCH_WINDOW_LASSO);
    assert_eq!(
        controller.session.draw_window_step(),
        Some(crate::StretchWindowStep::LassoTrace)
    );

    // A click cannot define a lasso: the capture refuses by name and stores
    // nothing, so it keeps waiting for the drag.
    let error = controller.capture_tool_point(p(0.0, 0.0)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stretch.window.lasso_needs_drag"),
        "{error}"
    );
    assert_eq!(controller.session.draw_window_ring_len(), 0);
    assert_eq!(
        controller.session.draw_window_step(),
        Some(crate::StretchWindowStep::LassoTrace)
    );

    // The host hands the traced drag path over as one closed ring (the same
    // entry the release of a lasso gesture uses).
    controller
        .session
        .set_draw_window_ring(square_ring())
        .unwrap();
    assert_eq!(controller.session.draw_window_step(), None);
    for point in [p(0.0, 0.0), p(0.0, 5.0)] {
        controller.capture_tool_point(point).unwrap();
    }
    assert_eq!(
        controller.session.draw_intent().unwrap(),
        DrawIntent::StretchPolygon {
            ring: square_ring(),
            base: p(0.0, 0.0),
            delta: p(0.0, 5.0),
        }
    );

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1)
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
    match controller
        .drawing()
        .unwrap()
        .entity(EntityId(201))
        .unwrap()
        .geometry
        .clone()
    {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(start, p(0.0, 5.0));
            assert_eq!(end, p(10.0, 0.0));
        }
        other => panic!("expected the line, got {other:?}"),
    }
}

#[test]
fn a_degenerate_ring_is_refused_by_name_and_changes_nothing() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    begin_stretch(&mut controller, crate::STRETCH_WINDOW_LASSO);

    // Two vertices are a line, not a ring: the refusal is named and the capture
    // stays exactly where it was (still waiting for its lasso).
    let error = controller
        .session
        .set_draw_window_ring(vec![p(0.0, 0.0), p(1.0, 1.0)])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stretch.window.too_few_vertices"),
        "{error}"
    );
    assert_eq!(
        controller.session.draw_window_step(),
        Some(crate::StretchWindowStep::LassoTrace)
    );
    assert_eq!(controller.session.draw_window_ring_len(), 0);

    // Collinear vertices enclose nothing: refused by name too, never stored as a
    // window that would silently select nothing.
    let error = controller
        .session
        .set_draw_window_ring(vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)])
        .unwrap_err();
    assert!(
        error.to_string().contains("stretch.window.degenerate_ring"),
        "{error}"
    );

    // A polygon ring cannot close before it has three vertices either.
    begin_stretch(&mut controller, crate::STRETCH_WINDOW_POLYGON);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(1.0, 0.0)).unwrap();
    let error = controller.session.close_draw_window_ring().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stretch.window.too_few_vertices"),
        "{error}"
    );

    // Nothing was written by any refused capture step.
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);
    assert_eq!(undo_depth(&controller), 0);
}
