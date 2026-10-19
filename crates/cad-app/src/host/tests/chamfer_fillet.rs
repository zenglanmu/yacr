//! CHAMFER/FILLET interactive capture and production-save export contracts.
//!
//! ADR 0016 decisions B/C: the two clicks are entity picks resolved at capture
//! time (the TRIM/EXTEND shape), and the new ARC (FILLET) / LINE (CHAMFER) must
//! survive the real production save path (`HostController::export_dxf`).

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

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(x0, y0),
        end: p(x1, y1),
    }
}

/// Two crossing lines under the default top view: `201` at `y = 100`, `202` at
/// `x = 0`; they meet at `(0, 100)`.
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
        .insert_entity(entity(201, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    builder
        .insert_entity(entity(202, line(0.0, -100.0, 0.0, 300.0), 1))
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

#[test]
fn chamfer_capture_refuses_empty_space_and_maps_the_two_entity_picks() {
    let mut controller = fixture();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Chamfer).unwrap();

    // An empty-space click is refused and the capture is untouched.
    let miss = controller.capture_tool_point(p(350.0, 250.0)).unwrap_err();
    assert!(matches!(miss, CadError::InvalidInput(_)), "{miss:?}");
    assert_eq!(controller.session.draw_preview().unwrap().remaining, 2);

    controller.capture_tool_point(p(150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(
        preview.picks,
        vec![reference(&controller, 201), reference(&controller, 202)]
    );
    // Capture is intent, not a transaction.
    assert_eq!(controller.drawing().unwrap().revision(), revision);

    // The intent maps to a Chamfer payload carrying the captured refs and picks.
    let intent = controller.session.draw_intent().unwrap();
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::ChamferLines);
    match command.payload {
        CommandPayload::Chamfer(payload) => {
            assert_eq!(payload.first, reference(&controller, 201));
            assert_eq!(payload.second, reference(&controller, 202));
            assert_eq!(payload.pick_first, p(150.0, 100.0));
            assert_eq!(payload.pick_second, p(0.0, 250.0));
        }
        _ => panic!("expected a Chamfer payload"),
    }
}

#[test]
fn fillet_confirm_commits_two_updates_and_one_arc_in_one_step() {
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::FilletRadius,
            crate::ToolValue::Number(50.0),
        )
        .unwrap();

    controller.begin_draw_tool(DrawToolKind::Fillet).unwrap();
    controller.capture_tool_point(p(150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 3, "two updates + one inserted ARC");
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");

    // The corner is (0,100); θ = 90°, so the tangent distance is R = 50 and the
    // centre is (50,150).
    let arc = controller
        .drawing()
        .unwrap()
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Arc { center, radius, .. } => Some((*center, *radius)),
            _ => None,
        })
        .expect("the tangent ARC is stored");
    assert!(
        (arc.0.x - 50.0).abs() < 1e-6 && (arc.0.y - 150.0).abs() < 1e-6,
        "{:?}",
        arc.0
    );
    assert!((arc.1 - 50.0).abs() < 1e-6, "{}", arc.1);
    // The two lines were trimmed to the tangent points.
    let first = match &controller
        .drawing()
        .unwrap()
        .entity(EntityId(201))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("{other:?}"),
    };
    assert!(
        (first.0.x - 50.0).abs() < 1e-6 || (first.1.x - 50.0).abs() < 1e-6,
        "{first:?}"
    );
}

#[test]
fn chamfer_new_line_survives_the_production_save_path() {
    let mut controller = fixture();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ChamferDistanceA,
            crate::ToolValue::Number(50.0),
        )
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::ChamferDistanceB,
            crate::ToolValue::Number(50.0),
        )
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Chamfer).unwrap();
    controller.capture_tool_point(p(150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    controller.confirm_draw().unwrap();

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let pairs = dxf_pairs(&text);
    let lines = dxf_entities(&pairs, "LINE");
    // Two source lines plus the one new joining LINE.
    assert_eq!(lines.len(), 3, "one new LINE joins the two trimmed lines");
    let joined = lines.iter().any(|record| {
        let start = (group(record, 10), group(record, 20));
        let end = (group(record, 11), group(record, 21));
        let near =
            |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        (near(start, (50.0, 100.0)) && near(end, (0.0, 150.0)))
            || (near(start, (0.0, 150.0)) && near(end, (50.0, 100.0)))
    });
    assert!(
        joined,
        "the new CHAMFER LINE (50,100)-(0,150) must be exported"
    );
}

#[test]
fn fillet_new_arc_survives_the_production_save_path() {
    let mut controller = fixture();
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::FilletRadius,
            crate::ToolValue::Number(50.0),
        )
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Fillet).unwrap();
    controller.capture_tool_point(p(150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    controller.confirm_draw().unwrap();

    let (bytes, report) = controller.export_dxf().unwrap();
    assert_eq!(report.counts.dropped, 0, "nothing may be dropped");
    let text = String::from_utf8(bytes).expect("DXF text is UTF-8");
    let pairs = dxf_pairs(&text);
    let arcs = dxf_entities(&pairs, "ARC");
    assert_eq!(arcs.len(), 1, "exactly one ARC for one FILLET");
    let record = &arcs[0];
    assert!((group(record, 10) - 50.0).abs() < 1e-6, "centre x");
    assert!((group(record, 20) - 150.0).abs() < 1e-6, "centre y");
    assert!((group(record, 40) - 50.0).abs() < 1e-6, "radius");
}

#[test]
fn fillet_confirm_refuses_parallel_lines_and_changes_nothing() {
    let mut controller = fixture();
    // Replace the second line with a parallel one.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(entity(201, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    builder
        .insert_entity(entity(203, line(-200.0, 200.0, 200.0, 200.0), 1))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller
        .session
        .set_tool_parameter(
            crate::ToolParameter::FilletRadius,
            crate::ToolValue::Number(50.0),
        )
        .unwrap();

    controller.begin_draw_tool(DrawToolKind::Fillet).unwrap();
    controller.capture_tool_point(p(150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(150.0, 200.0)).unwrap();
    let error = controller
        .confirm_draw()
        .expect_err("parallel lines have no corner");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(controller.drawing().unwrap().revision(), revision_before);
    assert_eq!(undo_depth(&controller), undo_before);
    assert!(
        controller.session.draw_preview().is_some(),
        "capture preserved"
    );
}
