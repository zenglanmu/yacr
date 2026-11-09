//! Interactive capture contracts for the parameter-selected curve edits:
//! LENGTHEN `DY`, BREAK `F` and DIVIDE's block option.
//!
//! Each test drives the real session capture (`begin_draw_tool` →
//! `capture_tool_point`/`set_tool_text` → `confirm_draw`), asserts the produced
//! [`DrawIntent`]/[`CommandPayload`] and pins the database effect: one
//! transaction and one undo step for a write, and nothing at all for a refusal.

use super::*;
use crate::{CommandPayload, DrawIntent, ToolParameter, ToolValue};

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

/// A drawing with the given model entities on layer 0. `block` adds a named
/// block definition (used by DIVIDE's block option).
fn fixture(entities: Vec<DbEntity>, block: Option<(&str, Point3)>) -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    if let Some((name, base)) = block {
        builder
            .insert_block(cad_db::BlockDefinition {
                id: cad_domain::BlockId(0),
                entities: Vec::new(),
                dynamic_visibility: None,
            })
            .unwrap();
        builder
            .set_block_metadata(cad_domain::BlockId(0), name, base)
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
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

fn line(id: u128, x0: f64, y0: f64, x1: f64, y1: f64) -> DbEntity {
    entity(
        id,
        SemanticGeometry::Line {
            start: p(x0, y0),
            end: p(x1, y1),
        },
        0,
    )
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

fn line_geometry(controller: &HostController, entity: u128) -> (Point3, Point3) {
    match &controller
        .drawing()
        .unwrap()
        .entity(EntityId(entity))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

fn assert_line(controller: &HostController, id: u128, x0: f64, y0: f64, x1: f64, y1: f64) {
    let (start, end) = line_geometry(controller, id);
    let near = |a: Point3, x: f64, y: f64| (a.x - x).abs() < 1e-9 && (a.y - y).abs() < 1e-9;
    assert!(
        (near(start, x0, y0) && near(end, x1, y1)) || (near(start, x1, y1) && near(end, x0, y0)),
        "expected ({x0},{y0})-({x1},{y1}), got {start:?}-{end:?}"
    );
}

// --- LENGTHEN DY -------------------------------------------------------------

#[test]
fn lengthen_dynamic_capture_takes_the_object_pick_and_the_new_end() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 5.0, 0.0)], None);
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(
            ToolParameter::LengthenMode,
            ToolValue::Number(crate::draw_tool::LENGTHEN_MODE_DYNAMIC),
        )
        .unwrap();
    let revision_before = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Lengthen).unwrap();

    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(
        preview.remaining_points(),
        Some(2),
        "DY replans LENGTHEN to two picks"
    );
    assert!(
        !preview.can_confirm(),
        "the selection alone is not the DY input"
    );

    controller.capture_tool_point(p(5.0, 0.0)).unwrap();
    assert_eq!(
        controller
            .session
            .draw_preview()
            .unwrap()
            .remaining_points(),
        Some(1)
    );
    controller.capture_tool_point(p(12.0, 3.0)).unwrap();

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::LengthenDynamic {
            pick: p(5.0, 0.0),
            new_end: p(12.0, 3.0),
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
    assert_eq!(command.id, crate::CommandId::Lengthen);
    match command.payload {
        CommandPayload::Points(ref points) => {
            assert_eq!(points, &vec![p(5.0, 0.0), p(12.0, 3.0)])
        }
        _ => panic!("expected the two-point dynamic LENGTHEN payload"),
    }

    assert_eq!(controller.drawing().unwrap().revision(), revision_before);
    let undo_before = undo_depth(&controller);
    controller.confirm_draw().unwrap();
    assert_line(&controller, 201, 0.0, 0.0, 12.0, 0.0);
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
}

#[test]
fn lengthen_static_modes_capture_one_nearest_endpoint_pick() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 5.0, 0.0)], None);
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(0.0))
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Lengthen).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(
        preview.remaining_points(),
        Some(1),
        "a static mode picks the end that moves"
    );
    assert!(!preview.can_confirm(), "the pick is still missing");
    controller.capture_tool_point(p(1.0, 0.0)).unwrap();
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::Lengthen {
            pick: Some(p(1.0, 0.0)),
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
    match command.payload {
        CommandPayload::Points(ref points) => assert_eq!(points, &vec![p(1.0, 0.0)]),
        _ => panic!("expected the one-point static LENGTHEN payload"),
    }
}

// --- BREAK F -----------------------------------------------------------------

#[test]
fn break_f_capture_takes_the_target_then_the_two_break_points() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 10.0, 0.0)], None);
    controller
        .session
        .set_tool_parameter(ToolParameter::BreakFirstPoint, ToolValue::Flag(true))
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Break).unwrap();
    assert_eq!(
        controller
            .session
            .draw_preview()
            .unwrap()
            .remaining_points(),
        Some(3),
        "the F form is entity pick + first point + second point"
    );
    // The first click only resolves the target.
    controller.capture_tool_point(p(5.0, 0.0)).unwrap();
    assert_eq!(
        controller
            .session
            .draw_preview()
            .unwrap()
            .remaining_points(),
        Some(2)
    );
    controller.capture_tool_point(p(3.0, 0.0)).unwrap();
    controller.capture_tool_point(p(7.0, 0.0)).unwrap();

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::Break {
            target: reference(&controller, 201),
            first: p(3.0, 0.0),
            second: p(7.0, 0.0),
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
    assert_eq!(command.id, crate::CommandId::Break);
    match command.payload {
        CommandPayload::Break { first, second, .. } => {
            assert_eq!((first, second), (p(3.0, 0.0), p(7.0, 0.0)))
        }
        _ => panic!("expected a Break payload"),
    }

    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller.confirm_draw().unwrap();
    assert_line(&controller, 201, 0.0, 0.0, 3.0, 0.0);
    let pieces: Vec<DbEntity> = controller
        .drawing()
        .unwrap()
        .model_space()
        .iter()
        .filter(|entity| entity.id != EntityId(201))
        .map(|entity| DbEntity::clone(entity))
        .collect();
    assert_eq!(pieces.len(), 1, "one complementary piece");
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
}

#[test]
fn break_without_f_keeps_the_two_click_form() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 10.0, 0.0)], None);
    controller.begin_draw_tool(DrawToolKind::Break).unwrap();
    assert_eq!(
        controller
            .session
            .draw_preview()
            .unwrap()
            .remaining_points(),
        Some(2)
    );
    controller.capture_tool_point(p(5.0, 0.0)).unwrap();
    controller.capture_tool_point(p(7.0, 0.0)).unwrap();
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::Break {
            target: reference(&controller, 201),
            first: p(5.0, 0.0),
            second: p(7.0, 0.0),
        }
    );
}

// --- DIVIDE ------------------------------------------------------------------

#[test]
fn divide_without_a_block_places_points_and_captures_no_text() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 9.0, 0.0)], None);
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(ToolParameter::DivideSegments, ToolValue::Number(3.0))
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Divide).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert!(!preview.awaiting_text, "the point form takes no text");

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(intent, DrawIntent::Divide);
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert!(matches!(command.payload, CommandPayload::Divide { .. }));

    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller.confirm_draw().unwrap();
    let points = controller
        .drawing()
        .unwrap()
        .model_space()
        .iter()
        .filter(|entity| matches!(&entity.geometry, SemanticGeometry::Point(_)))
        .count();
    assert_eq!(points, 2, "two interior marks for three parts");
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
}

#[test]
fn divide_block_option_prompts_for_the_name_and_inserts_the_block() {
    let mut controller = fixture(
        vec![line(201, 0.0, 0.0, 9.0, 0.0)],
        Some(("MARK", p(0.0, 0.0))),
    );
    controller
        .set_selection(vec![reference(&controller, 201)])
        .unwrap();
    controller
        .session
        .set_tool_parameter(ToolParameter::DivideSegments, ToolValue::Number(3.0))
        .unwrap();
    controller
        .session
        .set_tool_parameter(ToolParameter::DivideUseBlock, ToolValue::Flag(true))
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::Divide).unwrap();

    let preview = controller.session.draw_preview().unwrap();
    assert!(
        preview.awaiting_text,
        "the block option opens the terminal name step"
    );
    assert!(!preview.can_confirm());
    let error = controller
        .confirm_draw()
        .expect_err("a confirm without the block name is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");

    controller.session.set_tool_text("MARK").unwrap();
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        DrawIntent::DivideBlock {
            name: "MARK".into(),
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
    assert_eq!(command.id, crate::CommandId::Divide);
    match command.payload {
        CommandPayload::InsertBlock { name, insertion } => {
            assert_eq!(name, "MARK");
            assert!(insertion.x.is_finite() && insertion.y.is_finite() && insertion.z.is_finite());
        }
        _ => panic!("expected the InsertBlock name channel"),
    }

    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);
    controller.confirm_draw().unwrap();
    let inserts = controller
        .drawing()
        .unwrap()
        .model_space()
        .iter()
        .filter(|entity| matches!(&entity.geometry, SemanticGeometry::Insert { .. }))
        .count();
    assert_eq!(inserts, 2, "two independent INSERTs for three parts");
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
}

#[test]
fn the_new_capture_shapes_stay_work_only() {
    let mut controller = fixture(vec![line(201, 0.0, 0.0, 10.0, 0.0)], None);
    controller.set_mode(AppMode::Viewer).unwrap();
    for kind in [
        DrawToolKind::Break,
        DrawToolKind::Lengthen,
        DrawToolKind::Divide,
    ] {
        let error = controller
            .begin_draw_tool(kind)
            .expect_err("these edits are Work-only");
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{kind:?}: {error:?}"
        );
    }
    // The interactive-shape parameters are authorized against the same Work-only
    // commands, so a Viewer set is refused before it can change the plan.
    for (parameter, value) in [
        (ToolParameter::BreakFirstPoint, ToolValue::Flag(true)),
        (
            ToolParameter::LengthenMode,
            ToolValue::Number(crate::draw_tool::LENGTHEN_MODE_DYNAMIC),
        ),
        (ToolParameter::DivideUseBlock, ToolValue::Flag(true)),
        (ToolParameter::DivideAlignBlock, ToolValue::Flag(true)),
    ] {
        let error = controller
            .session
            .set_tool_parameter(parameter, value)
            .expect_err("the parameter set is Work-only too");
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{parameter:?}: {error:?}"
        );
    }
}
