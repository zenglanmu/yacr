//! TEXT pure draw-tool state-machine tests (ADR 0019 decision A/F.1, P7a).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! TEXT is the first tool whose last step is a **typed string**: one insertion
//! point, then a terminal `SetToolText`. The pure tool owns the prompt phase
//! (`awaiting_text`), the four named refusals and the 1024-character bound.

use super::*;

fn text_tool() -> DrawTool {
    DrawTool::new(DrawToolKind::Text, 0)
}

fn refusal_code(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn the_text_kind_metadata_is_explicit() {
    assert_eq!(DrawToolKind::Text.key(), "text");
    assert_eq!(DrawToolKind::from_key("text"), Some(DrawToolKind::Text));
    assert_eq!(DrawToolKind::from_label("TEXT"), Some(DrawToolKind::Text));
    assert_eq!(DrawToolKind::Text.label(), "TEXT");
    assert_eq!(DrawToolKind::Text.arity(), PointArity::Exact(1));
    assert_eq!(DrawToolKind::Text.required_points(), 1);
    assert!(!DrawToolKind::Text.requires_selection());
    assert_eq!(DrawToolKind::Text.picked_step_count(), 0);
    assert!(!DrawToolKind::Text.arity().is_open_ended());
    assert!(DrawToolKind::Text.requires_text());
    // `requires_text()` is the single authority: exactly one kind takes text.
    let text_kinds: Vec<DrawToolKind> = DrawToolKind::ALL
        .into_iter()
        .filter(|kind| kind.requires_text())
        .collect();
    assert_eq!(text_kinds, vec![DrawToolKind::Text]);
}

#[test]
fn awaiting_text_starts_after_the_insertion_point_and_blocks_confirm() {
    let mut tool = text_tool();
    assert!(!tool.awaiting_text(), "no point captured yet");
    assert!(!tool.is_ready());
    assert!(!tool.preview().awaiting_text);

    tool.push_point(p(1.0, 2.0)).expect("the insertion point");
    assert!(tool.awaiting_text(), "one point completes the complement");
    assert!(tool.is_ready(), "the point complement is complete");
    assert!(
        !tool.preview().can_confirm(),
        "a text tool awaiting text is not confirmable"
    );
    assert!(tool.preview().awaiting_text);
    // The prompt line is locale-neutral ASCII (the catalog owns chrome copy).
    let line = tool.preview().status_line();
    assert!(
        line.is_ascii(),
        "the Rust fallback line must not add CJK: {line}"
    );
    assert!(line.contains("type the text"), "{line}");

    // No intent without text: `Confirm` cannot complete it through the back door.
    let error = refusal_code(tool.intent().expect_err("text is missing"));
    assert!(error.contains("tool.text_required"), "{error}");
    assert_eq!(tool.text(), None);
}

#[test]
fn set_text_before_the_point_is_refused_with_the_points_reason() {
    let mut tool = text_tool();
    let error = refusal_code(tool.set_text("hello").expect_err("no insertion point"));
    assert!(error.contains("tool.text_points_incomplete"), "{error}");
    assert_eq!(tool.text(), None);
    assert!(!tool.awaiting_text());
}

#[test]
fn set_text_on_a_non_text_tool_is_refused_by_name() {
    let mut tool = DrawTool::new(DrawToolKind::Line, 0);
    let error = refusal_code(tool.set_text("hello").expect_err("LINE takes no text"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
}

#[test]
fn empty_and_whitespace_only_text_is_refused() {
    for text in ["", " ", "\t", "\n  \r\n"] {
        let mut tool = text_tool();
        tool.push_point(p(0.0, 0.0)).unwrap();
        let error = refusal_code(tool.set_text(text).expect_err("empty text"));
        assert!(error.contains("tool.text_empty"), "{text:?}: {error}");
        assert_eq!(tool.text(), None, "{text:?} must not be stored");
        assert!(tool.awaiting_text(), "{text:?} leaves the prompt open");
    }
}

#[test]
fn the_1024_character_bound_is_counted_in_characters_not_bytes() {
    // Exactly at the bound (ASCII) is accepted.
    let mut tool = text_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.set_text(&"a".repeat(MAX_TOOL_TEXT_CHARS))
        .expect("exactly the bound is accepted");
    assert_eq!(tool.text().map(str::len), Some(MAX_TOOL_TEXT_CHARS));

    // Exactly at the bound in CJK (3 bytes per character in UTF-8) is accepted:
    // the bound is characters, not bytes.
    let cjk = "字".repeat(MAX_TOOL_TEXT_CHARS);
    assert!(cjk.len() > MAX_TOOL_TEXT_CHARS);
    let mut tool = text_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.set_text(&cjk)
        .expect("1024 CJK characters are accepted");
    assert_eq!(tool.text().map(|text| text.chars().count()), Some(1024));

    // One character over is refused, never truncated.
    let mut tool = text_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    let too_long = "a".repeat(MAX_TOOL_TEXT_CHARS + 1);
    let error = refusal_code(tool.set_text(&too_long).expect_err("one over the bound"));
    assert!(error.contains("tool.text_too_long"), "{error}");
    assert!(error.contains("1024"), "{error}");
    assert_eq!(tool.text(), None, "the over-long text is not stored");
    assert!(tool.awaiting_text());
}

#[test]
fn a_submitted_text_completes_the_capture_and_travels_on_the_intent() {
    let mut tool = text_tool();
    tool.push_point(p(3.0, -4.0)).unwrap();
    tool.set_text("  AB  ")
        .expect("text with inner spaces is accepted");
    // Whitespace is preserved verbatim; only whitespace-only input is refused.
    assert_eq!(tool.text(), Some("  AB  "));
    assert!(!tool.awaiting_text(), "the prompt is closed");
    assert!(tool.preview().can_confirm(), "points + text are complete");
    assert!(!tool.preview().awaiting_text);
    assert_eq!(
        tool.intent().expect("the commit intent"),
        DrawIntent::Text {
            position: p(3.0, -4.0),
            text: "  AB  ".to_string(),
        }
    );

    // A second submission is refused: the text is terminal, not accumulating.
    let error = refusal_code(tool.set_text("again").expect_err("already has text"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
    assert_eq!(tool.text(), Some("  AB  "));
}

#[test]
fn dismiss_text_focus_keeps_the_tool_awaiting_and_only_validates_the_prompt() {
    let mut tool = text_tool();
    // Before the point there is no prompt to dismiss.
    let error = refusal_code(tool.dismiss_text_focus().expect_err("no prompt"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");

    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.dismiss_text_focus().expect("the first Esc is legal");
    // The first Esc is focus-only: the tool still awaits its text.
    assert!(tool.awaiting_text());
    assert_eq!(tool.text(), None);
    assert!(!tool.preview().can_confirm());
    // The text may still be submitted after the focus change.
    tool.set_text("after esc").expect("text still accepted");
    assert_eq!(tool.text(), Some("after esc"));
}

#[test]
fn the_second_point_is_still_refused_by_the_exact_arity() {
    let mut tool = text_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    let error = refusal_code(tool.push_point(p(1.0, 1.0)).expect_err("arity is one"));
    assert!(error.contains("already has its 1 points"), "{error}");
}
