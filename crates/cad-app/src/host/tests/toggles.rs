use super::*;

// --- idle hover preview toggle (default ON, AutoCAD-matching) ----------------

#[test]
fn hover_preview_defaults_on_and_the_controller_toggles_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    assert!(
        controller.hover_preview_enabled(),
        "the default matches AutoCAD's SELECTIONPREVIEW initial value 3"
    );
    assert_eq!(
        controller.session.hover_dwell_ms(),
        20,
        "the documented dwell default is 20 ms (an explicit extension, lowered from 300)"
    );
    controller.session.set_hover_dwell_ms(0);
    assert_eq!(controller.session.hover_dwell_ms(), 0);
    controller.session.set_hover_dwell_ms(300);

    assert!(
        !controller.toggle_hover_preview(),
        "the first toggle disables"
    );
    assert!(!controller.hover_preview_enabled());
    assert!(
        controller.toggle_hover_preview(),
        "the second toggle enables"
    );
    assert!(controller.hover_preview_enabled());

    // Session/display state only: no database revision and no history step.
    let after = controller.drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
}

#[test]
fn toggle_hover_preview_is_allowed_in_viewer_mode_and_reports_the_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Start explicitly from off so both directions are exercised regardless of
    // the default.
    controller.session.set_hover_preview(false);
    controller.set_mode(AppMode::Viewer).unwrap();
    assert_eq!(controller.mode(), AppMode::Viewer);
    assert!(
        !crate::CommandId::ToggleHoverPreview.requires_work_mode(),
        "the hover preview is display state, allowed in Viewer and Work"
    );

    // Enabling from Viewer is admitted and reports the resulting value in a
    // locale-neutral diagnostic.
    let enabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleHoverPreview,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the display-state toggle");
    assert!(controller.hover_preview_enabled());
    assert_eq!(enabled.diagnostics.len(), 1);
    assert_eq!(enabled.diagnostics[0].code, "view.hover_preview");
    assert_eq!(enabled.diagnostics[0].message, "hover preview: true");

    // The reverse direction reports its own value too.
    let disabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleHoverPreview,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the reverse toggle");
    assert!(!controller.hover_preview_enabled());
    assert_eq!(disabled.diagnostics[0].message, "hover preview: false");
    assert!(!controller.application.can_undo(&controller.document_id));
}

// --- object-snap master switch (default ON, ADR 0014 A.4) ---------------------

#[test]
fn snap_defaults_on_and_the_controller_toggles_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    assert!(controller.snap_enabled(), "object snap defaults on");

    assert!(!controller.toggle_snap(), "the first toggle disables");
    assert!(!controller.snap_enabled());
    assert!(
        controller.resolve_snap(p(10.0, 20.0)).unwrap().is_none(),
        "a disabled switch resolves no candidate, so the raw point is used"
    );
    assert!(controller.toggle_snap(), "the second toggle enables");
    assert!(controller.snap_enabled());

    // Session/display state only: no database revision and no history step.
    let after = controller.drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
}

#[test]
fn toggle_snap_is_allowed_in_viewer_mode_and_reports_the_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Start explicitly from off so both directions are exercised regardless of
    // the default.
    controller.set_snap_enabled(false);
    controller.set_mode(AppMode::Viewer).unwrap();
    assert_eq!(controller.mode(), AppMode::Viewer);
    assert!(
        !crate::CommandId::ToggleSnap.requires_work_mode(),
        "the object-snap switch is display/session state, allowed in Viewer and Work"
    );

    // Enabling from Viewer is admitted and reports the resulting value in a
    // locale-neutral diagnostic; no history entry is produced.
    let enabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleSnap,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the display-state toggle");
    assert!(controller.snap_enabled());
    assert!(enabled.changes.is_none(), "no ChangeSet is emitted");
    assert!(enabled.objects.is_empty());
    assert!(enabled.measurement.is_none());
    assert_eq!(enabled.diagnostics.len(), 1);
    assert_eq!(enabled.diagnostics[0].code, "view.snap");
    assert_eq!(enabled.diagnostics[0].message, "object snap: true");

    // The reverse direction reports its own value too.
    let disabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleSnap,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the reverse toggle");
    assert!(!controller.snap_enabled());
    assert_eq!(disabled.diagnostics[0].code, "view.snap");
    assert_eq!(disabled.diagnostics[0].message, "object snap: false");
    assert!(!controller.application.can_undo(&controller.document_id));
}

// --- display grid switch (default ON, `GRID`/F7) -----------------------------

#[test]
fn grid_defaults_on_and_the_controller_toggles_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    assert!(controller.grid_enabled(), "the display grid defaults on");

    assert!(!controller.toggle_grid(), "the first toggle disables");
    assert!(!controller.grid_enabled());
    assert!(controller.toggle_grid(), "the second toggle enables");
    assert!(controller.grid_enabled());

    // Session/display state only: no database revision and no history step.
    let after = controller.drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));

    // The display grid is a separate switch from object snap.
    assert!(
        controller.snap_enabled(),
        "flipping the display grid must not flip object snapping"
    );
}

#[test]
fn toggle_grid_is_allowed_in_viewer_mode_and_reports_the_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Start explicitly from off so both directions are exercised regardless of
    // the default.
    controller.set_grid_enabled(false);
    controller.set_mode(AppMode::Viewer).unwrap();
    assert_eq!(controller.mode(), AppMode::Viewer);
    assert!(
        !crate::CommandId::ToggleGrid.requires_work_mode(),
        "the display-grid switch is display/session state, allowed in Viewer and Work"
    );

    // Enabling from Viewer is admitted and reports the resulting value in a
    // locale-neutral diagnostic; no history entry is produced.
    let revision_before = controller.drawing().unwrap().revision();
    let enabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleGrid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the display-state toggle");
    assert!(controller.grid_enabled());
    assert!(enabled.changes.is_none(), "no ChangeSet is emitted");
    assert!(enabled.objects.is_empty());
    assert!(enabled.measurement.is_none());
    assert_eq!(enabled.diagnostics.len(), 1);
    assert_eq!(enabled.diagnostics[0].code, "view.grid");
    assert_eq!(enabled.diagnostics[0].message, "grid: true");

    // The reverse direction reports its own value too.
    let disabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleGrid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the reverse toggle");
    assert!(!controller.grid_enabled());
    assert_eq!(disabled.diagnostics[0].code, "view.grid");
    assert_eq!(disabled.diagnostics[0].message, "grid: false");
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "toggling never bumps the database revision"
    );
}

// --- ORTHO drafting constraint (default OFF, `ORTHO`/F8) ---------------------

#[test]
fn ortho_defaults_off_and_the_controller_toggles_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    assert!(
        !controller.ortho_enabled(),
        "ORTHO defaults off, matching AutoCAD's initial state"
    );

    assert!(controller.toggle_ortho(), "the first toggle enables");
    assert!(controller.ortho_enabled());
    assert!(!controller.toggle_ortho(), "the second toggle disables");
    assert!(!controller.ortho_enabled());

    // Session/display state only: no database revision and no history step.
    let after = controller.drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));

    // ORTHO is independent of the other drafting switches.
    assert!(
        controller.snap_enabled(),
        "flipping ORTHO must not flip OSNAP"
    );
    assert!(
        controller.grid_enabled(),
        "flipping ORTHO must not flip GRID"
    );
    assert!(
        controller.hover_preview_enabled(),
        "flipping ORTHO must not flip hover preview"
    );
}

#[test]
fn toggle_ortho_is_allowed_in_viewer_mode_and_reports_the_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_ortho_enabled(false);
    controller.set_mode(AppMode::Viewer).unwrap();
    assert_eq!(controller.mode(), AppMode::Viewer);
    assert!(
        !crate::CommandId::ToggleOrtho.requires_work_mode(),
        "the ORTHO switch is display/session state, allowed in Viewer and Work"
    );

    let revision_before = controller.drawing().unwrap().revision();
    let enabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleOrtho,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the drafting-state toggle");
    assert!(controller.ortho_enabled());
    assert!(enabled.changes.is_none(), "no ChangeSet is emitted");
    assert!(enabled.objects.is_empty());
    assert!(enabled.measurement.is_none());
    assert_eq!(enabled.diagnostics.len(), 1);
    assert_eq!(enabled.diagnostics[0].code, "view.ortho");
    assert_eq!(enabled.diagnostics[0].message, "ortho: true");

    let disabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleOrtho,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the reverse toggle");
    assert!(!controller.ortho_enabled());
    assert_eq!(disabled.diagnostics[0].code, "view.ortho");
    assert_eq!(disabled.diagnostics[0].message, "ortho: false");
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "toggling never bumps the database revision"
    );
}

// --- grid snap switch (default OFF, `SNAP`/F9) -------------------------------

#[test]
fn snap_grid_defaults_off_and_the_controller_toggles_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = controller.drawing().unwrap();
    assert!(
        !controller.snap_grid_enabled(),
        "SNAP defaults off, matching AutoCAD's initial state"
    );

    assert!(controller.toggle_snap_grid(), "the first toggle enables");
    assert!(controller.snap_grid_enabled());
    assert!(!controller.toggle_snap_grid(), "the second toggle disables");
    assert!(!controller.snap_grid_enabled());

    // Session/display state only: no database revision and no history step.
    let after = controller.drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));

    // Grid snap is independent of the other drafting switches.
    assert!(
        controller.snap_enabled(),
        "flipping SNAP must not flip object snapping"
    );
    assert!(
        controller.grid_enabled(),
        "flipping SNAP must not flip GRID"
    );
    assert!(
        !controller.ortho_enabled(),
        "flipping SNAP must not flip ORTHO"
    );
}

#[test]
fn toggle_snap_grid_is_allowed_in_viewer_mode_and_reports_the_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_grid_enabled(false);
    controller.set_mode(AppMode::Viewer).unwrap();
    assert_eq!(controller.mode(), AppMode::Viewer);
    assert!(
        !crate::CommandId::ToggleSnapGrid.requires_work_mode(),
        "the grid-snap switch is display/session state, allowed in Viewer and Work"
    );

    let revision_before = controller.drawing().unwrap().revision();
    let enabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleSnapGrid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the drafting-state toggle");
    assert!(controller.snap_grid_enabled());
    assert!(enabled.changes.is_none(), "no ChangeSet is emitted");
    assert!(enabled.objects.is_empty());
    assert!(enabled.measurement.is_none());
    assert_eq!(enabled.diagnostics.len(), 1);
    assert_eq!(enabled.diagnostics[0].code, "view.snap_grid");
    assert_eq!(enabled.diagnostics[0].message, "grid snap: true");

    let disabled = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleSnapGrid,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("Viewer admits the reverse toggle");
    assert!(!controller.snap_grid_enabled());
    assert_eq!(disabled.diagnostics[0].code, "view.snap_grid");
    assert_eq!(disabled.diagnostics[0].message, "grid snap: false");
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "toggling never bumps the database revision"
    );
}

// --- SNAPUNIT spacing sysvar (typed payload, `SNAPUNIT <value>`) -------------

#[test]
fn set_snap_spacing_is_allowed_in_viewer_mode_and_is_session_state_only() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_mode(AppMode::Viewer).unwrap();
    assert!(
        !crate::CommandId::SetSnapSpacing.requires_work_mode(),
        "the spacing sysvar is session state, allowed in Viewer and Work"
    );
    let revision_before = controller.drawing().unwrap().revision();

    let outcome = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetSnapSpacing,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::SnapSpacing(2.5),
        })
        .expect("Viewer admits the spacing sysvar");
    assert_eq!(controller.snap_spacing(), 2.5);
    assert!(outcome.changes.is_none(), "no ChangeSet is emitted");
    assert!(outcome.objects.is_empty());
    assert!(outcome.measurement.is_none());
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "view.snap_spacing");
    assert_eq!(outcome.diagnostics[0].message, "snap spacing: 2.5");
    assert!(!controller.application.can_undo(&controller.document_id));
    assert!(!controller.application.can_redo(&controller.document_id));
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "setting the spacing never bumps the database revision"
    );
}

#[test]
fn set_snap_spacing_refuses_a_missing_or_invalid_payload_explicitly() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_spacing(12.0).unwrap();

    // A bare `SNAPUNIT` (no argument) carries no payload: refused by name.
    let missing = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetSnapSpacing,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect_err("a bare SNAPUNIT must be refused");
    assert!(
        format!("{missing:?}").contains("SNAPUNIT"),
        "the refusal names the command: {missing:?}"
    );

    // An out-of-grammar value that somehow reached the command layer is refused
    // by the session setter, never clamped.
    let invalid = controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetSnapSpacing,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::SnapSpacing(0.0),
        })
        .expect_err("a zero spacing must be refused");
    assert!(
        format!("{invalid:?}").contains("finite and positive"),
        "the refusal names the rule: {invalid:?}"
    );
    assert_eq!(
        controller.snap_spacing(),
        12.0,
        "a refusal never changes the stored spacing"
    );
}
