//! MLEADER pure draw-tool state-machine tests (ADR 0019 decision F.2, P7c).
//!
//! MLEADER is the third text tool: two free points (arrow position, landing
//! point) then the **same** terminal typed-text step TEXT uses. These tests pin
//! the kind metadata, the prompt phase and the tool-level refusals; the command
//! contract (geometry, transaction, export round trip) lives in
//! `crate::tests::drawing::mleader_create`.

use super::*;

fn mleader_tool() -> DrawTool {
    DrawTool::new(DrawToolKind::MLeader, 0)
}

fn refusal_code(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn the_mleader_kind_metadata_is_explicit() {
    assert_eq!(DrawToolKind::MLeader.key(), "mleader");
    assert_eq!(
        DrawToolKind::from_key("mleader"),
        Some(DrawToolKind::MLeader)
    );
    assert_eq!(
        DrawToolKind::from_label("MLEADER"),
        Some(DrawToolKind::MLeader)
    );
    assert_eq!(DrawToolKind::MLeader.label(), "MLEADER");
    // Arrow position + landing point; the text is the terminal typed step.
    assert_eq!(DrawToolKind::MLeader.arity(), PointArity::Exact(2));
    assert_eq!(DrawToolKind::MLeader.required_points(), 2);
    assert!(!DrawToolKind::MLeader.arity().is_open_ended());
    // No entity picks: both steps are free points.
    assert_eq!(DrawToolKind::MLeader.picked_step_count(), 0);
    assert!(!DrawToolKind::MLeader.needs_picked_entities());
    assert!(!DrawToolKind::MLeader.requires_selection());
    assert!(DrawToolKind::MLeader.requires_text());
}

#[test]
fn awaiting_text_starts_only_after_both_points_and_blocks_confirm() {
    let mut tool = mleader_tool();
    assert!(!tool.awaiting_text(), "no point captured yet");
    tool.push_point(p(0.0, 0.0)).expect("the arrow position");
    assert!(
        !tool.awaiting_text(),
        "one point does not complete the complement"
    );
    assert!(!tool.is_ready());
    // Text before the points are complete is refused by name.
    let error = refusal_code(tool.set_text("too early").expect_err("points incomplete"));
    assert!(error.contains("tool.text_points_incomplete"), "{error}");
    assert_eq!(tool.text(), None);

    tool.push_point(p(10.0, 2.0)).expect("the landing point");
    assert!(tool.awaiting_text());
    assert!(tool.is_ready(), "the point complement is complete");
    // `Confirm` cannot complete the command through the back door.
    assert!(!tool.preview().can_confirm(), "text is terminal");
    let error = refusal_code(tool.intent().expect_err("text is missing"));
    assert!(error.contains("tool.text_required"), "{error}");
    let line = tool.preview().status_line();
    assert!(line.is_ascii(), "no CJK in the Rust fallback line: {line}");
    assert!(line.contains("type the text"), "{line}");
}

#[test]
fn a_submitted_text_completes_the_capture_and_travels_on_the_intent() {
    let mut tool = mleader_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(10.0, 2.0)).unwrap();
    tool.set_text("Note").expect("the annotation text");
    assert!(!tool.awaiting_text(), "the prompt is closed");
    assert!(tool.preview().can_confirm());
    assert_eq!(
        tool.intent().expect("the commit intent"),
        DrawIntent::MLeader {
            arrow: p(0.0, 0.0),
            landing: p(10.0, 2.0),
            text: "Note".to_string(),
        }
    );
    // A second submission is refused: the text is terminal, not accumulating.
    let error = refusal_code(tool.set_text("again").expect_err("already has text"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
    assert_eq!(tool.text(), Some("Note"));
    // The exact arity still refuses a third point.
    let error = refusal_code(tool.push_point(p(1.0, 1.0)).expect_err("arity is two"));
    assert!(error.contains("already has its 2 points"), "{error}");
}

#[test]
fn empty_and_over_long_text_is_refused_and_leaves_the_prompt_open() {
    for text in ["", " ", "\t\n"] {
        let mut tool = mleader_tool();
        tool.push_point(p(0.0, 0.0)).unwrap();
        tool.push_point(p(10.0, 2.0)).unwrap();
        let error = refusal_code(tool.set_text(text).expect_err("empty text"));
        assert!(error.contains("tool.text_empty"), "{text:?}: {error}");
        assert_eq!(tool.text(), None, "{text:?} must not be stored");
        assert!(tool.awaiting_text(), "{text:?} leaves the prompt open");
    }
    // The 1024 bound is counted in characters, not bytes.
    let mut tool = mleader_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(10.0, 2.0)).unwrap();
    let too_long = "a".repeat(MAX_TOOL_TEXT_CHARS + 1);
    let error = refusal_code(tool.set_text(&too_long).expect_err("one over the bound"));
    assert!(error.contains("tool.text_too_long"), "{error}");
    assert_eq!(tool.text(), None);
}

#[test]
fn set_text_before_the_points_is_refused_by_the_points_reason() {
    let mut tool = mleader_tool();
    let error = refusal_code(tool.set_text("hello").expect_err("no points yet"));
    assert!(error.contains("tool.text_points_incomplete"), "{error}");
    assert!(!tool.awaiting_text());
}
