//! Wave 9 command-line alias contracts: `-BLOCK`, `-INSERT`, `-BOUNDARY`,
//! `-HATCH`, `-HATCHEDIT`, `EXOFFSET`.
//!
//! An alias is not a second implementation. These tests prove each alias is
//! Work-only and answers a malformed request with exactly the owning handler's
//! answer, so there is one planner per feature.

use super::*;

const ALIASES: [(ExtendedCommandId, CommandId); 6] = [
    (ExtendedCommandId::DashBlock, CommandId::DefineBlock),
    (ExtendedCommandId::DashInsert, CommandId::InsertBlock),
    (ExtendedCommandId::DashBoundary, CommandId::Boundary),
    (ExtendedCommandId::DashHatch, CommandId::Hatch),
    (ExtendedCommandId::DashHatchEdit, CommandId::HatchEdit),
    (ExtendedCommandId::ExOffset, CommandId::OffsetEntity),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave9_alias_is_work_only() {
    for (ext, _) in ALIASES {
        assert!(ext.requires_work_mode(), "{ext:?} writes through its owner");
    }
}

#[test]
fn aliases_answer_exactly_like_the_owning_command_for_a_missing_payload() {
    for (ext, parent) in ALIASES {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let alias_error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a missing payload is refused");
        let parent_error = app
            .execute(&mut session, command(parent, CommandPayload::None))
            .expect_err("the parent refuses the same payload");
        assert_eq!(
            format!("{alias_error:?}"),
            format!("{parent_error:?}"),
            "{ext:?} must not invent its own validation"
        );
        assert_eq!(revision(&app), before, "{ext:?} wrote nothing");
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{ext:?} recorded no undo step"
        );
    }
}

#[test]
fn aliases_are_refused_in_viewer_mode_before_any_write() {
    for (ext, _) in ALIASES {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("Viewer cannot run a write alias");
        assert!(matches!(error, CadError::PermissionDenied));
    }
}
