//! Typed `INSERT <name>` / `BLOCK <name>`: the pending block name is applied as
//! the capture's terminal text once the point complement is complete.
//!
//! Contract level: the session state machine only; the create commands and their
//! refusals are pinned in their own tests.

use super::*;

#[test]
fn a_pending_insert_name_completes_the_capture_after_one_point() {
    let (_app, mut session) = application_with_document();
    session
        .begin_draw(DrawToolKind::InsertBlock)
        .expect("the INSERT capture starts");
    session.set_pending_tool_text(Some("Widget".into()));
    let ready = session
        .capture_point_with_snap(point(1.0, 2.0), None)
        .expect("the insertion point is captured");
    assert!(ready, "the pending name completes the capture");
    assert_eq!(
        session.draw_intent().expect("the complete intent"),
        DrawIntent::InsertBlock {
            insertion: point(1.0, 2.0),
            name: "Widget".into(),
        }
    );
    // The point-only capture has no text prompt left.
    assert!(!session.draw_awaiting_text());
}

#[test]
fn a_pending_block_name_completes_the_define_capture_after_its_base_point() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        ),
    )
    .expect("a member line");
    let id = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .last()
        .expect("the line")
        .id;
    session.selection = SelectionSet::from_refs([SelectionRef {
        document: DocumentId(1),
        entity: id,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    session
        .begin_draw(DrawToolKind::BlockDefine)
        .expect("the BLOCK capture starts");
    session.set_pending_tool_text(Some("  Cabin  ".into()));
    let ready = session
        .capture_point_with_snap(point(3.0, 4.0), None)
        .expect("the base point is captured");
    assert!(ready);
    assert_eq!(
        session.draw_intent().expect("the complete intent"),
        DrawIntent::BlockDefine {
            base: point(3.0, 4.0),
            name: "Cabin".into(),
        }
    );
}

#[test]
fn a_new_capture_never_inherits_a_previous_pending_name() {
    let (_app, mut session) = application_with_document();
    session
        .begin_draw(DrawToolKind::InsertBlock)
        .expect("the first capture");
    session.set_pending_tool_text(Some("Widget".into()));
    session.cancel_tool().expect("cancel");
    session
        .begin_draw(DrawToolKind::InsertBlock)
        .expect("the second capture");
    let ready = session
        .capture_point_with_snap(point(1.0, 1.0), None)
        .expect("point");
    assert!(!ready, "no stale name may complete a new capture");
    assert!(
        session.draw_awaiting_text(),
        "the name still has to be typed"
    );
    assert!(session.draw_intent().is_err(), "the name is missing");
}

#[test]
fn an_empty_or_multiline_pending_name_is_not_recorded() {
    let (_app, mut session) = application_with_document();
    session
        .begin_draw(DrawToolKind::InsertBlock)
        .expect("start");
    session.set_pending_tool_text(Some("   ".into()));
    let ready = session
        .capture_point_with_snap(point(1.0, 1.0), None)
        .expect("point");
    assert!(!ready);
    assert!(session.draw_awaiting_text());
    session.set_pending_tool_text(Some("A\nB".into()));
    assert!(
        session.draw_awaiting_text(),
        "a newline is not a block name"
    );
    assert!(session.draw_intent().is_err());
}
