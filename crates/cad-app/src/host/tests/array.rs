//! ARRAYRECT/ARRAYPOLAR interactive capture and production-save export contracts.
//!
//! ADR 0016 decision D: ARRAYRECT captures **no points** (defined by the typed
//! session parameters plus the selection snapshot) and ARRAYPOLAR captures the one
//! array centre; both insert independent copies that must survive the real
//! production save path (`HostController::export_dxf`).

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

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(x0, y0),
        end: p(x1, y1),
    }
}

/// A drawing with one LINE `(0,0)-(10,0)` on layer 0, whose bounding-box
/// lower-left is the origin.
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
        .insert_entity(entity(201, line(0.0, 0.0, 10.0, 0.0), 0))
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

fn reference(controller: &HostController, entity: u128) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn undo_depth(controller: &HostController) -> usize {
    controller
        .application
        .history
        .get(&controller.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
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
fn arrayrect_captures_no_points_and_maps_to_its_payload() {
    let mut controller = fixture();
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::ArrayRect).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.points.len(), 0, "ARRAYRECT captures no points");
    assert!(
        preview.can_confirm(),
        "ARRAYRECT is ready as soon as the selection is non-empty"
    );

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(intent, DrawIntent::ArrayRect);
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::ArrayRectangular);
    match command.payload {
        CommandPayload::ArrayRect { refs } => {
            assert_eq!(refs, vec![reference(&controller, 201)]);
        }
        _ => panic!("expected an ArrayRect payload"),
    }
}

#[test]
fn arrayrect_with_an_empty_selection_is_not_ready_and_refuses_to_confirm() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::ArrayRect).unwrap();
    assert!(
        !controller.session.draw_preview().unwrap().can_confirm(),
        "ARRAYRECT needs a non-empty selection"
    );
    let error = controller
        .confirm_draw()
        .expect_err("an empty selection is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);
    assert_eq!(undo_depth(&controller), 0);
}

#[test]
fn arrayrect_confirm_inserts_the_grid_and_it_survives_the_save_path() {
    let mut controller = fixture();
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ArrayRows,
            crate::ToolValue::Number(2.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ArrayColumns,
            crate::ToolValue::Number(2.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ArrayRowSpacing,
            crate::ToolValue::Number(5.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ArrayColumnSpacing,
            crate::ToolValue::Number(10.0),
        )
        .unwrap();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);

    controller.begin_draw_tool(DrawToolKind::ArrayRect).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(
        outcome.objects.len(),
        3,
        "2x2 = three independent copies (the source's own cell is skipped)"
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
    assert_eq!(lines.len(), 4, "the retained source plus three copies");
    // The copy at cell (row 1, column 1) is the line (10,5)-(20,5).
    assert!(
        lines
            .iter()
            .any(|record| dxf_line_is(record, 10.0, 5.0, 20.0, 5.0)),
        "the arrayed copy (10,5)-(20,5) must be exported"
    );
}

#[test]
fn arraypolar_captures_one_centre_point_and_survives_the_save_path() {
    let mut controller = fixture();
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ArrayItems,
            crate::ToolValue::Number(4.0),
        )
        .unwrap();
    controller
        .begin_draw_tool(DrawToolKind::ArrayPolar)
        .unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points.len(),
        0,
        "ARRAYPOLAR starts with no captured point"
    );

    controller.capture_tool_point(p(0.0, -10.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.points.len(), 1, "the one captured centre");
    assert!(preview.can_confirm());

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::ArrayPolar {
            center: p(0.0, -10.0)
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
    assert_eq!(command.id, crate::CommandId::ArrayPolar);
    match command.payload {
        CommandPayload::ArrayPolar { refs, center } => {
            assert_eq!(refs, vec![reference(&controller, 201)]);
            assert_eq!(center, p(0.0, -10.0));
        }
        _ => panic!("expected an ArrayPolar payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 3, "items - 1 = three copies");

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let lines = dxf_entities(&dxf_pairs(&text), "LINE");
    assert_eq!(lines.len(), 4, "the retained source plus three copies");
    // Item 1 is translated to (-10,-10)-(0,-10) (translate-only placement).
    assert!(
        lines
            .iter()
            .any(|record| dxf_line_is(record, -10.0, -10.0, 0.0, -10.0)),
        "the polar copy (-10,-10)-(0,-10) must be exported"
    );
}
