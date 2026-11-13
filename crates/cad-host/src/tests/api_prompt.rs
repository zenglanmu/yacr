//! The plugin `Editor` prompt channel through the real input funnel.
//!
//! A prompt is answered by the interactions a user already produces: a click, a
//! captured point, typed text, Enter and Esc. These contracts drive `HostCore`
//! directly, so they prove the interception happens **before** the tool machine
//! and that a prompt never invents a value.

use super::*;
use cad_app::plugin_prompt::{
    ApiPrompt, ApiPromptInput, ApiPromptKind, ApiPromptReply, ApiPromptResult,
};

fn begin(core: &HostCore, kind: ApiPromptKind) -> ApiPrompt {
    let prompt = ApiPrompt::new("req-1", kind, None);
    core.controller
        .borrow_mut()
        .begin_api_prompt(prompt.clone())
        .expect("the prompt starts");
    prompt
}

fn answers(core: &HostCore) -> Vec<ApiPromptResult> {
    core.controller.borrow_mut().take_api_prompt_results()
}

/// Dispatch one interaction through the engine gate (the real host path).
fn input(core: &HostCore, request: InteractionRequest) {
    EngineClient::dispatch(core, EngineRequest::Interaction(request))
        .expect("the interaction is admitted");
}

/// A canvas-local click at one logical pixel.
fn click_at(core: &HostCore, pixel: [f64; 2]) {
    let revision = core.request_context().metrics_revision.unwrap();
    input(
        core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: pixel,
                metrics_revision: revision,
            },
            mode: SelectionMode::Replace,
        },
    );
}

#[test]
fn an_idle_click_answers_a_point_prompt_with_the_world_point() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    begin(&core, ApiPromptKind::Point);
    let world = p(2.0, 1.0);
    let pixel = screen_of(&core, world);
    click_at(&core, pixel);

    let answers = answers(&core);
    assert_eq!(answers.len(), 1, "one click, one answer");
    let ApiPromptReply::Point(point) = answers[0].reply else {
        panic!("expected a point, got {:?}", answers[0].reply);
    };
    assert!(
        (point.x - world.x).abs() < 1e-6 && (point.y - world.y).abs() < 1e-6,
        "the answer is the click's world point: {point:?}"
    );
    assert!(!core.controller.borrow().api_prompt_pending());
    assert!(
        core.controller.borrow().selection().refs().is_empty(),
        "a point prompt must not also select"
    );
}

#[test]
fn a_captured_point_answers_a_point_prompt() {
    let core = core();
    begin(&core, ApiPromptKind::Point);
    input(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(4.0, 5.0))),
    );
    let answers = answers(&core);
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].reply,
        ApiPromptReply::Point(p(4.0, 5.0)),
        "the captured world point is the answer"
    );
}

#[test]
fn typed_coordinates_answer_a_point_prompt() {
    let core = core();
    begin(&core, ApiPromptKind::Point);
    input(
        &core,
        InteractionRequest::SetToolText {
            text: "3,4".to_string(),
        },
    );
    assert_eq!(answers(&core)[0].reply, ApiPromptReply::Point(p(3.0, 4.0)));
}

#[test]
fn invalid_text_reprompts_and_keeps_the_prompt_pending() {
    let core = core();
    begin(&core, ApiPromptKind::Integer);
    input(
        &core,
        InteractionRequest::SetToolText {
            text: "half".to_string(),
        },
    );
    assert!(
        core.controller.borrow().api_prompt_pending(),
        "invalid input keeps the prompt"
    );
    assert_eq!(
        core.controller.borrow().api_prompt_error(),
        Some("invalid input: enter a whole number")
    );
    assert!(answers(&core).is_empty(), "nothing is answered yet");

    input(
        &core,
        InteractionRequest::SetToolText {
            text: "12".to_string(),
        },
    );
    assert_eq!(answers(&core)[0].reply, ApiPromptReply::Number(12.0));
    assert_eq!(core.controller.borrow().api_prompt_error(), None);
}

#[test]
fn an_entity_prompt_answers_the_picked_entity() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    begin(&core, ApiPromptKind::Entity);
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);
    click_at(&core, pixel);

    let answers = answers(&core);
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].reply,
        ApiPromptReply::Entity(expected.entity),
        "the answer is the real pick, not a guess"
    );
    assert!(
        core.controller.borrow().selection().refs().is_empty(),
        "getEntity reports the object without publishing a selection"
    );
}

#[test]
fn an_entity_miss_reprompts_with_the_reject_sentence() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let mut prompt = ApiPrompt::new("req-1", ApiPromptKind::Entity, None);
    prompt.reject_message = Some("Pick the object".to_string());
    core.controller
        .borrow_mut()
        .begin_api_prompt(prompt)
        .expect("the prompt starts");
    click_at(&core, [1.0, 1.0]);
    assert!(core.controller.borrow().api_prompt_pending());
    assert_eq!(
        core.controller.borrow().api_prompt_error(),
        Some("Pick the object")
    );
    assert!(answers(&core).is_empty());
}

#[test]
fn a_selection_prompt_toggles_clicks_and_enter_finishes_it() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    begin(&core, ApiPromptKind::Selection);
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);
    click_at(&core, pixel);
    assert!(
        core.controller.borrow().api_prompt_pending(),
        "a click keeps the selection prompt open"
    );
    assert_eq!(
        core.controller.borrow().selection().refs(),
        &[expected.clone()],
        "the click publishes the selection the way Select does"
    );

    input(&core, InteractionRequest::Confirm);
    let answers = answers(&core);
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].reply,
        ApiPromptReply::Selection(vec![expected.entity])
    );
    assert!(!core.controller.borrow().api_prompt_pending());
}

#[test]
fn escape_answers_cancel_and_is_not_forwarded_to_the_tool_machine() {
    let core = core();
    // Start a real LINE capture so a forwarded cancel would be observable.
    input(&core, InteractionRequest::BeginDraw(DrawToolKind::Line));
    input(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(0.0, 0.0))),
    );
    assert!(
        !matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "the LINE capture is active"
    );

    begin(&core, ApiPromptKind::String);
    input(&core, InteractionRequest::Cancel);
    assert_eq!(answers(&core)[0].reply, ApiPromptReply::Cancel);
    assert!(
        !matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "the prompt consumed the Esc: the LINE capture survives"
    );
}

#[test]
fn a_prompt_only_takes_the_inputs_it_answers_for() {
    let core = core();
    begin(&core, ApiPromptKind::String);
    // A pointer press is not a prompt input: it must keep its ordinary path.
    let revision = core.request_context().metrics_revision.unwrap();
    input(
        &core,
        InteractionRequest::Pointer {
            id: cad_app::interaction::PointerId(1),
            kind: cad_app::interaction::PointerKind::Mouse,
            phase: cad_app::interaction::PointerPhase::Down,
            button: Some(cad_app::interaction::PointerButton::Primary),
            position: PointInput::CanvasLocal {
                logical_position: [10.0, 10.0],
                metrics_revision: revision,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        },
    );
    assert!(
        core.controller.borrow().api_prompt_pending(),
        "still pending"
    );
    assert!(answers(&core).is_empty());
    assert_eq!(
        core.controller
            .borrow_mut()
            .resolve_api_prompt(ApiPromptInput::Confirm),
        cad_app::plugin_prompt::ApiPromptDisposition::Handled
    );
}
