//! TEXT text channel: selector routing, `awaiting_text`, terminal submission and
//! the named refusal surface (ADR 0019 decisions A.1/A.2, P7a).

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// Click at a canvas-logical pixel (a press + its release), the shape a real DOM
/// click produces.
fn click(session: &CadSession, pixel: [f64; 2], sequence: u64) {
    session
        .handle_input(&pointer_at(InputKind::PointerDown, sequence, pixel, None))
        .expect("the press is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerUp, sequence + 1, pixel, None))
        .expect("the release is admitted");
}

fn text_entities(session: &CadSession) -> Vec<(String, Point3)> {
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    let entities = drawing.model_space();
    entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, position, .. } => Some((text.clone(), *position)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_text_selector_starts_the_text_kind_with_the_documented_height() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("TEXT starts a tool");
    assert_eq!(tool.key, "draw.text");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready, "no insertion point yet");
    assert!(
        !tool.awaiting_text,
        "the prompt opens only once the point is complete"
    );
    assert_eq!(
        tool.params.text_height,
        Some(cad_app::draw_tool::TEXT_DEFAULT_HEIGHT),
        "the session channel carries the documented default"
    );
}

#[test]
fn awaiting_text_is_projected_and_confirm_is_refused_by_name() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.points, 1);
    assert!(
        tool.awaiting_text,
        "one point completes the complement, so the prompt is open"
    );
    assert!(
        !tool.ready,
        "`ready` is false because nothing can be confirmed until the text arrives"
    );

    // `Confirm` while awaiting text must never complete the command.
    let error = session.command("CONFIRM").unwrap_err();
    assert!(error.contains("tool.awaiting_text"), "{error}");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("the refused confirm keeps the tool");
    assert!(tool.awaiting_text);
    assert!(text_entities(&session).is_empty(), "nothing was written");

    // The first Esc only dismisses focus: the tool stays open and awaiting.
    session
        .dismiss_text_focus()
        .expect("the first Esc is legal");
    let tool = session.snapshot(None, None, 0).tool.expect("still active");
    assert!(tool.awaiting_text, "the tool still awaits its text");

    // The second Esc is the ordinary CANCEL and resets the tool (no transaction).
    session.command("CANCEL").expect("CANCEL");
    assert!(session.snapshot(None, None, 0).tool.is_none());
    assert!(text_entities(&session).is_empty(), "cancel writes nothing");
}

#[test]
fn set_tool_text_is_terminal_and_completes_without_a_second_confirm() {
    let session = fresh_session();
    let before = {
        let controller = session.core.controller.borrow();
        controller.drawing().expect("a document").entity_count()
    };
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    session
        .set_tool_text("Hello")
        .expect("the text is accepted");

    let tool = session.snapshot(None, None, 0).tool;
    assert!(
        tool.is_none(),
        "the committed text completes the tool immediately"
    );
    let texts = text_entities(&session);
    assert_eq!(texts.len(), 1, "exactly one TEXT entity");
    assert_eq!(texts[0].0, "Hello");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    assert_eq!(
        drawing.entity_count(),
        before + 1,
        "one entity, not a second one"
    );
    assert!(controller.application.can_undo(&controller.document_id));
}

#[test]
fn the_text_refusals_are_named_and_write_nothing() {
    // No tool at all.
    let session = fresh_session();
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_not_awaiting"), "{error}");

    // TEXT active but no insertion point yet.
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_points_incomplete"), "{error}");

    // Empty / whitespace-only and over the 1024-character bound.
    for (text, expected) in [
        ("".to_string(), "tool.text_empty"),
        ("   ".to_string(), "tool.text_empty"),
        (
            "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1),
            "tool.text_too_long",
        ),
    ] {
        let session = fresh_session();
        session.command("TEXT").expect("TEXT is admitted");
        click(&session, [400.0, 300.0], 1);
        let error = session.set_tool_text(&text).unwrap_err();
        assert!(error.contains(expected), "expected {expected}, got {error}");
        // The refused text left the prompt open and wrote nothing.
        let tool = session.snapshot(None, None, 0).tool.expect("still active");
        assert!(tool.awaiting_text, "{expected}: the prompt stays open");
        assert!(text_entities(&session).is_empty(), "{expected}: no write");
    }

    // A non-text tool refuses text.
    let session = fresh_session();
    session.command("LINE").expect("LINE is admitted");
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
}

#[test]
fn the_text_height_parameter_is_present_only_for_the_text_tool() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        Some(2.5)
    );
    session
        .set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(5.0))
        .expect("a positive height is accepted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        Some(5.0)
    );
    // A non-text tool reports `null`, never a stale height.
    session.command("LINE").expect("LINE is admitted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        None
    );
    // The channel refuses non-positive / non-finite values explicitly.
    assert!(session
        .set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(0.0))
        .is_err());
}
