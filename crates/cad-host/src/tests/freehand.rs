//! The FREEHAND stroke on the real host gesture path.
//!
//! The contract pinned here: `SKETCH` starts the freehand capture, a primary
//! drag samples the stroke, and the release commits exactly one open polyline
//! through the existing `SKETCH` command — one revision, one undo step. A click
//! with no drag is refused by name and writes nothing; a cancel drops the stroke
//! without writing. No GUI or GPU is involved.
use super::*;

fn modifiers() -> cad_app::interaction::Modifiers {
    cad_app::interaction::Modifiers::default()
}

fn pointer_request(
    core: &HostCore,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(1),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers: modifiers(),
    }
}

fn press(core: &HostCore, phase: cad_app::interaction::PointerPhase, position: [f64; 2]) {
    dispatch_pointer(core, pointer_request(core, phase, position));
}

fn begin_freehand(core: &HostCore) {
    dispatch_pointer(
        core,
        InteractionRequest::BeginDraw(cad_app::DrawToolKind::Freehand),
    );
    core.controller.borrow_mut().set_snap_enabled(false);
}

fn revision(core: &HostCore) -> cad_domain::Revision {
    core.controller.borrow().drawing().unwrap().revision()
}

fn undo_depth(core: &HostCore) -> usize {
    let c = core.controller.borrow();
    c.application
        .history
        .get(&c.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_entity_count(core: &HostCore) -> usize {
    core.controller
        .borrow()
        .drawing()
        .unwrap()
        .model_space()
        .len()
}

#[test]
fn a_freehand_drag_commits_one_open_polyline_and_one_undo_step() {
    let core = core();
    begin_freehand(&core);
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        Some(cad_app::DrawToolKind::Freehand)
    );
    let revision_before = revision(&core);
    let entities_before = model_entity_count(&core);
    let undo_before = undo_depth(&core);
    let selection_before = core.controller.borrow().session.selection.refs().to_vec();

    let start = screen_of(&core, p(0.0, 0.0));
    let path = [
        [start[0] + 10.0, start[1]],
        [start[0] + 20.0, start[1] - 10.0],
        [start[0] + 30.0, start[1]],
        [start[0] + 40.0, start[1] - 10.0],
    ];
    press(&core, cad_app::interaction::PointerPhase::Down, start);
    for point in path {
        press(&core, cad_app::interaction::PointerPhase::Move, point);
    }
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        [start[0] + 50.0, start[1]],
    );

    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        None,
        "the release confirmed and completed the capture"
    );
    assert_eq!(model_entity_count(&core), entities_before + 1);
    assert_eq!(revision(&core).0, revision_before.0 + 1, "one revision");
    assert_eq!(undo_depth(&core), undo_before + 1, "one undo step");
    assert!(core.box_selection_preview().is_none(), "the trail is gone");
    assert_eq!(
        core.controller.borrow().session.selection.refs().to_vec(),
        selection_before,
        "a stroke must not leak into the selection"
    );

    let controller = core.controller.borrow();
    let drawing = controller.drawing().unwrap();
    let model_space = drawing.model_space();
    let entity = model_space.last().expect("the new entity");
    let cad_domain::SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = &entity.geometry
    else {
        panic!(
            "FREEHAND must store one LWPOLYLINE, got {:?}",
            entity.geometry
        );
    };
    assert!(points.len() >= 3, "several samples: {}", points.len());
    assert_eq!(bulges.len(), points.len());
    assert!(!closed, "freehand is an open polyline");
    drop(drawing);
    drop(controller);

    // Undo removes exactly the stroke.
    core.command(CommandId::Undo, CommandPayload::None)
        .expect("undo the stroke");
    assert_eq!(model_entity_count(&core), entities_before);
    assert_eq!(undo_depth(&core), undo_before);
}

#[test]
fn a_click_without_a_drag_is_refused_by_name_and_keeps_the_tool() {
    let core = core();
    begin_freehand(&core);
    let revision_before = revision(&core);
    let entities_before = model_entity_count(&core);
    let start = screen_of(&core, p(0.0, 0.0));
    press(&core, cad_app::interaction::PointerPhase::Down, start);
    press(&core, cad_app::interaction::PointerPhase::Up, start);

    assert_eq!(model_entity_count(&core), entities_before);
    assert_eq!(revision(&core), revision_before, "nothing was written");
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        Some(cad_app::DrawToolKind::Freehand),
        "the tool still waits for a real stroke"
    );
    let status = core.last_status().expect("the refusal is published");
    let text = match status {
        cad_engine::protocol::StatusMessage::ExternalText(text) => text,
        other => panic!("expected the named refusal, got {other:?}"),
    };
    assert!(
        text.contains("sketch.too_few_points"),
        "the refusal is named: {text}"
    );
}

#[test]
fn a_cancel_drops_the_stroke_without_writing() {
    let core = core();
    begin_freehand(&core);
    let revision_before = revision(&core);
    let start = screen_of(&core, p(0.0, 0.0));
    press(&core, cad_app::interaction::PointerPhase::Down, start);
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [start[0] + 40.0, start[1] - 20.0],
    );
    press(&core, cad_app::interaction::PointerPhase::Cancel, start);
    assert_eq!(revision(&core), revision_before, "nothing was written");
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        None,
        "a pointer cancel ends the tool (no open capture survives an OS/browser cancel)"
    );
}
