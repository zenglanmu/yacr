//! MTEXT pure draw-tool state-machine tests (interactive subset, 2026-10-26).
//!
//! MTEXT captures two opposite corners (first = insertion point, opposite = the
//! wrap-width reference) and then the **same** terminal typed text step TEXT
//! uses. The width derivation, the default attachment and the zero-rectangle
//! refusal live in the host intent mapping (`host::draw_intent_map`), so this
//! file pins the capture shape and the text gate only.

use super::*;

fn mtext_tool() -> DrawTool {
    DrawTool::new(DrawToolKind::MText, 0)
}

fn refusal_code(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn the_mtext_kind_metadata_is_explicit() {
    assert_eq!(DrawToolKind::MText.key(), "mtext");
    assert_eq!(DrawToolKind::from_key("mtext"), Some(DrawToolKind::MText));
    assert_eq!(DrawToolKind::from_label("MTEXT"), Some(DrawToolKind::MText));
    // The Rust-side label stays locale-neutral ASCII: the catalog
    // (`draw.kind.mtext`) carries the chrome copy and the CJK baseline is frozen.
    assert_eq!(DrawToolKind::MText.label(), "MTEXT");
    // Two opposite corners; the text is a typed string, not a point.
    assert_eq!(DrawToolKind::MText.arity(), PointArity::Exact(2));
    assert_eq!(DrawToolKind::MText.required_points(), 2);
    assert!(!DrawToolKind::MText.arity().is_open_ended());
    assert!(!DrawToolKind::MText.requires_selection());
    assert_eq!(DrawToolKind::MText.picked_step_count(), 0);
    assert!(!DrawToolKind::MText.needs_picked_entities());
    assert!(DrawToolKind::MText.requires_text());
}

#[test]
fn mtext_captures_two_corners_then_awaits_the_text() {
    let mut tool = mtext_tool();
    assert_eq!(tool.remaining(), Some(2));
    assert!(!tool.awaiting_text(), "no corner captured yet");
    // A text step only opens once the point complement is complete.
    let error = refusal_code(tool.set_text("A").expect_err("corners missing"));
    assert!(error.contains("tool.text_points_incomplete"), "{error}");

    tool.push_point(p(1.0, 2.0)).expect("the first corner");
    assert!(!tool.awaiting_text(), "one corner is not the complement");
    assert_eq!(tool.remaining(), Some(1));
    tool.push_point(p(9.0, 6.0)).expect("the opposite corner");
    assert!(tool.is_ready(), "the point complement is complete");
    assert!(tool.awaiting_text());
    assert!(
        !tool.preview().can_confirm(),
        "the text step is terminal: no Confirm may complete it"
    );
    // The fallback status line is locale-neutral ASCII (the catalog owns chrome).
    let line = tool.preview().status_line();
    assert!(line.is_ascii(), "no CJK in the Rust fallback: {line}");
    assert!(line.contains("type the text"), "{line}");

    // No intent without the text.
    let error = refusal_code(tool.intent().expect_err("text missing"));
    assert!(error.contains("tool.text_required"), "{error}");

    // A newline is accepted: the commit target is an `AcDbMText`, so the shared
    // text policy turns it into `\P` downstream.
    tool.set_text("A\nB").expect("MTEXT accepts a line break");
    assert!(!tool.awaiting_text(), "the prompt is closed");
    assert!(tool.preview().can_confirm());
    assert_eq!(
        tool.intent().expect("the commit intent"),
        DrawIntent::MText {
            first: p(1.0, 2.0),
            opposite: p(9.0, 6.0),
            text: "A\nB".to_string(),
        }
    );

    // The text is terminal: a second submission is refused by name.
    let error = refusal_code(tool.set_text("again").expect_err("already has text"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
}

#[test]
fn mtext_refuses_the_third_corner_and_non_finite_points() {
    let mut tool = mtext_tool();
    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(4.0, 3.0)).unwrap();
    let error = refusal_code(tool.push_point(p(9.0, 9.0)).expect_err("arity is two"));
    assert!(error.contains("already has its 2 points"), "{error}");
    assert_eq!(tool.points().len(), 2);

    let mut fresh = mtext_tool();
    assert!(fresh.push_point(p(f64::NAN, 0.0)).is_err());
    assert!(fresh.points().is_empty());
}

#[test]
fn mtext_first_escape_only_dismisses_the_text_focus() {
    let mut tool = mtext_tool();
    let error = refusal_code(tool.dismiss_text_focus().expect_err("no prompt yet"));
    assert!(error.contains("tool.text_not_awaiting"), "{error}");

    tool.push_point(p(0.0, 0.0)).unwrap();
    tool.push_point(p(2.0, 1.0)).unwrap();
    tool.dismiss_text_focus().expect("the first Esc is legal");
    assert!(tool.awaiting_text(), "the tool still awaits its text");
    tool.set_text("still editable after Esc")
        .expect("the text is still accepted");
}
