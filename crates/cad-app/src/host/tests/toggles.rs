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
