//! `MULTIPLE` repeat-loop contracts (2026-11-24).
//!
//! The arm is session state; the loop lives on the one capture-confirm funnel.
//! A bare `MULTIPLE` pins the next capture command, a typed target repeats only
//! that command, and `Esc`/`CANCEL` stops the loop. Every repetition needs a real
//! successful confirm, so no test can spin.

use super::*;

fn multiple(
    controller: &mut HostController,
    payload: crate::CommandPayload,
) -> CadResult<CommandOutcome> {
    controller.execute(Command {
        schema_version: 1,
        id: crate::CommandId::Multiple,
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload,
    })
}

fn draw_line(controller: &mut HostController) -> CadResult<CommandOutcome> {
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(10.0, 20.0)).unwrap();
    controller.capture_tool_point(p(40.0, 20.0)).unwrap();
    controller.confirm_draw()
}

#[test]
fn multiple_repeats_the_next_capture_until_cancel() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    multiple(&mut controller, crate::CommandPayload::None).unwrap();
    assert_eq!(
        controller.session.multiple_target(),
        Some(&crate::MultipleRepeat::NextCommand)
    );
    let count = controller.drawing().unwrap().entity_count();

    let first = draw_line(&mut controller).unwrap();
    assert_eq!(first.objects.len(), 1);
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "multiple.repeated"),
        "the re-open is reported: {:?}",
        first.diagnostics
    );
    assert_eq!(
        controller.drawing().unwrap().entity_count(),
        count + 1,
        "exactly one entity per pass"
    );
    assert!(
        matches!(controller.session.tool, crate::ToolState::Drawing(_)),
        "the capture is open again"
    );
    assert_eq!(
        controller.session.multiple_target(),
        Some(&crate::MultipleRepeat::Command(
            crate::CommandId::CreateLine
        )),
        "the bare arm is pinned to the command that ran"
    );

    // A second real confirm repeats again, until the user cancels.
    draw_line(&mut controller).unwrap();
    assert_eq!(controller.drawing().unwrap().entity_count(), count + 2);
    assert!(matches!(
        controller.session.tool,
        crate::ToolState::Drawing(_)
    ));

    controller.cancel_tool().unwrap();
    assert!(
        controller.session.multiple_target().is_none(),
        "cancel stops the loop"
    );
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert_eq!(
        controller.session.multiple_repeat_count(),
        0,
        "a stopped loop forgets its count"
    );
}

#[test]
fn multiple_named_target_repeats_only_that_command() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    multiple(
        &mut controller,
        crate::CommandPayload::Multiple(Some(crate::MultipleTarget {
            selector: "LINE".into(),
            command: crate::CommandId::CreateLine,
        })),
    )
    .unwrap();

    // Another capture command completes once and does not repeat.
    controller.begin_draw_tool(DrawToolKind::Circle).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(5.0, 0.0)).unwrap();
    let circle = controller.confirm_draw().unwrap();
    assert!(
        !circle
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "multiple.repeated"),
        "a non-target command is not repeated"
    );
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert_eq!(
        controller.session.multiple_target(),
        Some(&crate::MultipleRepeat::Command(
            crate::CommandId::CreateLine
        )),
        "the target stays armed for its own next run"
    );

    // The named target repeats.
    let line = draw_line(&mut controller).unwrap();
    assert!(
        line.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "multiple.repeated"),
        "{:?}",
        line.diagnostics
    );
    assert!(matches!(
        controller.session.tool,
        crate::ToolState::Drawing(_)
    ));
    controller.cancel_tool().unwrap();
}

#[test]
fn multiple_cancel_before_the_first_confirm_writes_nothing_and_disarms() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    multiple(&mut controller, crate::CommandPayload::None).unwrap();
    let revision = controller.drawing().unwrap().revision();
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.cancel_tool().unwrap();

    assert!(controller.session.multiple_target().is_none());
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
    assert_eq!(controller.drawing().unwrap().revision(), revision);
    assert_eq!(controller.drawing().unwrap().entity_count(), count);
    assert!(!controller.application.can_undo(&controller.document_id));
}
