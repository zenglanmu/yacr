//! TEXT host contracts: the terminal text channel, the `Confirm` refusal and the
//! two-stage `Esc` (ADR 0019 decision A.1, P7a).
//!
//! These drive `HostCore` through the typed interaction path (no UI, no GPU).

use super::*;

fn begin_text(core: &HostCore) {
    let mut controller = core.controller.borrow_mut();
    // Object snap is on by default; these contracts are about the text channel,
    // not about where a nearby entity pulls the insertion point.
    controller.session.set_snap_enabled(false);
    controller
        .begin_draw_tool(DrawToolKind::Text)
        .expect("TEXT starts");
}

fn capture_world(core: &HostCore, x: f64, y: f64) {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(x, y)))),
    )
    .expect("the point is captured");
}

fn awaiting_text(core: &HostCore) -> bool {
    matches!(
        core.tool_snapshot(),
        cad_app::ToolSnapshot::Drawing(preview) if preview.awaiting_text
    )
}

fn text_entities(core: &HostCore) -> Vec<(String, Point3, f64)> {
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    let entities = drawing.model_space();
    entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Text {
                text,
                position,
                height,
                ..
            } => Some((text.clone(), *position, *height)),
            _ => None,
        })
        .collect()
}

#[test]
fn confirm_is_refused_while_the_text_prompt_is_open() {
    let core = core();
    let before = core.controller.borrow().drawing().unwrap().clone();
    begin_text(&core);
    capture_world(&core, 10.0, 20.0);
    assert!(awaiting_text(&core));

    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Confirm),
    )
    .expect_err("Confirm must not complete a text tool");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("tool.awaiting_text"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    // Nothing was written and the tool is still awaiting text.
    assert!(awaiting_text(&core));
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        before.revision(),
        "a refused Confirm writes nothing"
    );
    assert!(text_entities(&core).is_empty());
}

#[test]
fn the_dismiss_request_keeps_the_tool_and_the_second_esc_cancels() {
    let core = core();
    begin_text(&core);
    capture_world(&core, 0.0, 0.0);
    // The first Esc (focus only): the tool stays active and still awaits text.
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::DismissTextFocus),
    )
    .expect("the first Esc is acknowledged");
    assert!(awaiting_text(&core), "the tool still awaits its text");
    // The second Esc is the ordinary cancel: the tool resets and nothing is written.
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Cancel),
    )
    .expect("the second Esc cancels");
    assert!(matches!(core.tool_snapshot(), cad_app::ToolSnapshot::Idle));
    assert!(text_entities(&core).is_empty(), "Esc never writes");
}

#[test]
fn set_tool_text_completes_the_command_immediately() {
    let core = core();
    begin_text(&core);
    capture_world(&core, 12.0, 34.0);
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::SetToolText {
            text: "Hello".into(),
        }),
    )
    .expect("the terminal text is accepted");
    // One step, no second Confirm: the tool is gone and exactly one TEXT exists.
    assert!(matches!(core.tool_snapshot(), cad_app::ToolSnapshot::Idle));
    let texts = text_entities(&core);
    assert_eq!(texts.len(), 1, "one TEXT entity: {texts:?}");
    assert_eq!(texts[0].0, "Hello");
    assert!((texts[0].1.x - 12.0).abs() < 1e-9, "{:?}", texts[0].1);
    assert!((texts[0].1.y - 34.0).abs() < 1e-9, "{:?}", texts[0].1);
    assert!(
        (texts[0].2 - cad_app::draw_tool::TEXT_DEFAULT_HEIGHT).abs() < 1e-9,
        "the documented default height: {:?}",
        texts[0].2
    );
    // One undo step is recorded (the single write path).
    let controller = core.controller.borrow();
    assert!(controller.application.can_undo(&controller.document_id));
}

#[test]
fn set_tool_text_refusals_are_named_and_leave_no_transaction() {
    let core = core();
    // No text tool at all.
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::SetToolText {
            text: "Hello".into(),
        }),
    )
    .expect_err("no tool is awaiting text");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("tool.text_not_awaiting"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }

    // TEXT started but no insertion point: the points reason, not "not awaiting".
    begin_text(&core);
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::SetToolText {
            text: "Hello".into(),
        }),
    )
    .expect_err("the insertion point is missing");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("tool.text_points_incomplete"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }

    // Empty text and an over-long text after the point.
    capture_world(&core, 0.0, 0.0);
    for (text, expected) in [
        (String::new(), "tool.text_empty"),
        ("  ".to_string(), "tool.text_empty"),
        (
            "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1),
            "tool.text_too_long",
        ),
    ] {
        let error = EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(InteractionRequest::SetToolText { text }),
        )
        .expect_err(expected);
        match error {
            RequestError::Execution(CadError::InvalidInput(reason)) => {
                assert!(reason.contains(expected), "{reason}");
            }
            other => panic!("expected {expected}, got {other:?}"),
        }
        assert!(awaiting_text(&core), "{expected}: the prompt stays open");
    }
    assert!(text_entities(&core).is_empty(), "nothing was written");
}

#[test]
fn the_esc_action_policy_still_resolves_text_focus_before_the_tool() {
    // The first Esc is the existing `EscapeAction::DismissTextFocus` (a UI focus
    // change), the second is `CancelTool`: the Rust side must not invent a third
    // meaning, and the text prompt only opens after the point is complete.
    let core = core();
    let mut policy = cad_app::input::InputPolicy::new();
    begin_text(&core);
    capture_world(&core, 1.0, 1.0);
    policy.set_text_focus(true);
    assert_eq!(
        cad_app::input::escape_action(&policy, &core.controller.borrow().session),
        cad_app::input::EscapeAction::DismissTextFocus
    );
    policy.set_text_focus(false);
    assert_eq!(
        cad_app::input::escape_action(&policy, &core.controller.borrow().session),
        cad_app::input::EscapeAction::CancelTool
    );
}
