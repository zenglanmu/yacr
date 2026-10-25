//! Status projection, diagnostics text and session-build contracts.

use super::super::super::*;
use super::super::test_support::*;
use crate::session::status::{document_capabilities, DocumentCapabilities};

/// Host capabilities with every file command advertised, so a projection test
/// can prove the live session state still gates them.
fn advertised_capabilities() -> HostCapabilities {
    HostCapabilities {
        open: true,
        new: true,
        save: true,
        plot: true,
        import_worker: false,
    }
}

/// Host capabilities that advertise nothing: the honest probe/unsupported host.
fn no_capabilities() -> HostCapabilities {
    HostCapabilities {
        open: false,
        new: false,
        save: false,
        plot: false,
        import_worker: false,
    }
}

#[test]
fn status_error_is_locale_neutral() {
    let failed = StatusMessage::Catalog {
        key: "command.failed".into(),
        args: vec![cad_engine::protocol::StatusArgument {
            key: "error".into(),
            value: "parse failed".into(),
        }],
    };
    assert_eq!(status_error(&failed).as_deref(), Some("parse failed"));
    let opened = StatusMessage::Catalog {
        key: "linux.opened".into(),
        args: Vec::new(),
    };
    assert_eq!(status_error(&opened), None);
    assert_eq!(
        status_error(&StatusMessage::ExternalText("sync failed".into())).as_deref(),
        Some("sync failed")
    );
}

#[test]
fn resource_diagnostics_are_technical_not_ui_text() {
    let diagnostic = RawDiagnostic {
        object: None,
        code: RESOURCE_MISSING_CODE.into(),
        message: String::new(),
        severity: None,
        completeness: None,
        parameters: vec![RawDiagnosticParameter::Key("logo.png".into())],
    };
    assert_eq!(
        resource_diagnostic_text(&diagnostic),
        "resource.missing logo.png"
    );
    let keys = ["a.png".to_string(), "b.png".to_string()];
    assert_eq!(
        resource_keys_error(RESOURCE_MISSING_CODE, keys.iter()),
        "resource.missing a.png b.png"
    );
}

#[test]
fn blank_status_reports_no_document_capabilities() {
    let status = assemble_status(
        false,
        "ignored".into(),
        None,
        false,
        true,
        // NEW needs no source file; SAVE/PLOT still need a document, so the
        // projection of an empty session with all caps advertised is
        // `true/false/false`.
        document_capabilities(advertised_capabilities(), false, false, false),
        None,
        Some(CadCamera {
            center: [1.0, 2.0],
            scale: 1.0,
        }),
        None,
        None,
        CadSelection::empty(),
        0,
        0,
        None,
        false,
        false,
        true,
        // Grid snap is on with a 2.5-unit spacing in this hand-built snapshot, so
        // both new fields are proven to be carried verbatim.
        true,
        2.5,
        true,
        None,
        false,
        // No cursor on a blank session: no world coordinate to report.
        None,
        // No pointer, no gesture: nothing is dragging.
        false,
    );
    assert_eq!(status.document_label, None);
    assert_eq!(status.entity_count, None);
    assert_eq!(status.camera, None, "a missing document has no camera");
    assert!(!status.can_fit);
    assert!(status.can_open);
    assert!(status.can_new, "a blank document needs no open file");
    assert!(!status.can_save, "saving needs an authoritative document");
    assert!(!status.can_plot, "plotting needs an authoritative document");
    assert_eq!(status.import_count, 0);
    assert_eq!(status.frame_count, 0);
    assert_eq!(
        status.cursor_world, None,
        "a session without a cursor has no world coordinate, never the origin"
    );
    assert!(
        !status.hover_preview,
        "the assembled status carries the flag it was given"
    );
    assert!(
        !status.snap_enabled,
        "the assembled status carries the object-snap flag it was given"
    );
    assert!(
        status.grid_enabled,
        "the assembled status carries the display-grid flag it was given"
    );
    assert!(
        status.snap_grid_enabled,
        "the assembled status carries the grid-snap flag it was given"
    );
    assert_eq!(
        status.snap_spacing, 2.5,
        "the assembled status carries the snap spacing it was given, never a default"
    );
    assert!(
        status.ortho_enabled,
        "the assembled status carries the ORTHO flag it was given"
    );
    // The selection block is always carried, never omitted.
    assert_eq!(status.selection, CadSelection::empty());
    assert!(
        !status.grip_drag,
        "a blank session has no armed grip drag to report"
    );
}

#[test]
fn document_status_reports_counts_and_camera() {
    let status = assemble_status(
        true,
        "fixture.dwg".into(),
        Some(3),
        false,
        true,
        document_capabilities(advertised_capabilities(), true, false, false),
        None,
        Some(CadCamera {
            center: [10.0, 20.0],
            scale: 2.0,
        }),
        Some("Vulkan".into()),
        Some("hardware".into()),
        CadSelection::empty(),
        1,
        10,
        None,
        true,
        true,
        false,
        // Grid snap off with the documented default spacing.
        false,
        10.0,
        false,
        None,
        true,
        // The world coordinate is carried verbatim, never re-derived here.
        Some([1.5, -2.0, 3.0]),
        // An armed grip drag is carried verbatim too, alongside `tool: null`.
        true,
    );
    assert_eq!(status.document_label.as_deref(), Some("fixture.dwg"));
    assert_eq!(status.entity_count, Some(3));
    assert!(status.can_fit);
    assert!(
        status.can_new && status.can_save && status.can_plot,
        "an idle session with a document advertises NEW/SAVE/PLOT"
    );
    assert_eq!(status.frame_count, 10);
    assert!(
        status.hover_preview,
        "the status projects the session's hover preview flag"
    );
    assert!(
        status.snap_enabled,
        "the status projects the session's object-snap flag"
    );
    assert!(
        !status.grid_enabled,
        "the display-grid flag is projected from what the caller supplied, not guessed"
    );
    assert!(
        !status.ortho_enabled,
        "the ORTHO flag is projected from what the caller supplied, not guessed"
    );
    assert!(
        status.crosshair,
        "the status carries the crosshair flag it was given (the 3D pointer gate)"
    );
    assert_eq!(
        status.cursor_world,
        Some([1.5, -2.0, 3.0]),
        "the status carries the world cursor it was given, verbatim"
    );
    assert!(
        status.grip_drag,
        "the status carries the armed-drag flag it was given, even with `tool: null`"
    );
    assert!(
        status.tool.is_none(),
        "a grip drag never fabricates a tool block"
    );
}

#[test]
fn document_capabilities_follow_the_host_flags_and_the_live_session_state() {
    // Advertised, idle, with a document: everything is available.
    let ready = document_capabilities(advertised_capabilities(), true, false, false);
    assert_eq!(
        ready,
        DocumentCapabilities {
            can_new: true,
            can_save: true,
            can_plot: true,
        }
    );

    // A host that advertises nothing reports nothing, even with a document.
    let unsupported = document_capabilities(no_capabilities(), true, false, false);
    assert_eq!(
        unsupported,
        DocumentCapabilities {
            can_new: false,
            can_save: false,
            can_plot: false,
        }
    );

    // No authoritative document: NEW still needs no source file, SAVE/PLOT do.
    let blank = document_capabilities(advertised_capabilities(), false, false, false);
    assert!(blank.can_new);
    assert!(!blank.can_save && !blank.can_plot);

    // While an open/import is running nothing can start, including a new
    // drawing (the session is busy).
    let loading = document_capabilities(advertised_capabilities(), true, true, false);
    assert!(!loading.can_new && !loading.can_save && !loading.can_plot);

    // A save/plot target chooser (or its pending export) blocks SAVE and PLOT
    // but not NEW: the host serialises only the two export funnels.
    let pending = document_capabilities(advertised_capabilities(), true, false, true);
    assert!(pending.can_new);
    assert!(!pending.can_save && !pending.can_plot);
}

#[test]
fn session_status_projects_the_real_save_pending_state_onto_can_save() {
    // A desktop-shaped platform (NEW/SAVE/PLOT advertised) whose save picker
    // opens but never resolves: the host must drop SAVE/PLOT availability while
    // that target is pending, and never invent one before/after.
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.files = Rc::new(PendingSaveFiles);
    host_platform.capabilities = advertised_capabilities();
    let session = session_with_platform(host_platform);

    let idle = session.snapshot(None, None, 0);
    assert!(idle.document_label.is_some(), "the demo document is open");
    assert!(idle.can_new && idle.can_save && idle.can_plot);

    // SAVE opens the picker and holds the pending slot.
    session.command("SAVE").expect("SAVE opens the picker");
    let pending = session.snapshot(None, None, 0);
    assert!(pending.can_new, "an open picker does not block NEW");
    assert!(
        !pending.can_save && !pending.can_plot,
        "a pending save target blocks SAVE and PLOT"
    );
}

#[test]
fn the_status_carries_the_grip_drag_flag_on_the_wire_in_both_states() {
    // The status contract is "every key always exists", so the additive flag is
    // present in the idle/unsupported state too — with the honest `false`.
    let idle = unsupported_status(None);
    assert!(!idle.grip_drag);

    // The wire name is the block's camelCase `gripDrag`. `cad-ui-core`
    // deliberately carries no `serde_json`, so the field-name recorder is the
    // pinned proof that a `serde(rename)` did not silently win; the values are
    // asserted on the struct (exactly like the `commandLog` contract).
    let names = super::super::tests_text::wire_field_names(&idle);
    assert!(
        names.iter().any(|name| name == "gripDrag"),
        "the additive key must reach the wire as `gripDrag`: {names:?}"
    );
    // Nothing was reordered or dropped: the pre-existing keys are all still here.
    for key in ["popup", "tool", "commandLog", "cursorWorld", "frameCount"] {
        assert!(
            names.iter().any(|name| name == key),
            "the pre-existing key `{key}` disappeared: {names:?}"
        );
    }

    // …and the same key is emitted with `true` while a drag is armed.
    let dragging = assemble_status(
        true,
        "fixture.dwg".into(),
        Some(1),
        false,
        true,
        document_capabilities(advertised_capabilities(), true, false, false),
        None,
        None,
        None,
        None,
        CadSelection::empty(),
        0,
        0,
        None,
        true,
        true,
        true,
        false,
        10.0,
        false,
        None,
        false,
        None,
        true,
    );
    assert!(dragging.grip_drag);
    let names = super::super::tests_text::wire_field_names(&dragging);
    assert!(names.iter().any(|name| name == "gripDrag"), "{names:?}");
}

#[test]
fn an_armed_grip_drag_reaches_the_status_flag_and_cancel_drops_it() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session
        .core
        .controller
        .borrow_mut()
        .fit()
        .expect("the demo document fits");

    // The demo drawing's entity 1 is the line from (0, 0) to (4000, 0); its start
    // point is a real grip (`cad_app::grips` exposes a LINE's two endpoints).
    let reference = SelectionRef {
        document: session.core.controller.borrow().document_id,
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    };
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![reference])
        .expect("the selection");

    // Idle: no gesture, and the status says exactly that.
    assert!(!session.core.grip_drag_active());
    assert!(!session.snapshot(None, None, 0).grip_drag);

    // A primary press on the handle arms the drag; the projection is the host's
    // own answer, never a re-derivation from the pointer stream.
    let grip_pixel = screen_of(
        &session,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    session
        .handle_input(&pointer_at(InputKind::PointerDown, 1, grip_pixel, None))
        .expect("the press on the handle is admitted");
    assert!(
        session.core.grip_drag_active(),
        "the press on a handle of the selection arms the drag"
    );
    let dragging = session.snapshot(None, None, 0);
    assert_eq!(
        dragging.grip_drag,
        session.core.grip_drag_active(),
        "the status flag is the host's real gesture state, not a guess"
    );
    assert!(dragging.grip_drag);
    assert!(
        dragging.tool.is_none(),
        "a grip drag is not a tool: the tool block stays null, so no tool-only \
         affordance (CONFIRM, the tool prompt) can turn on because of a drag"
    );

    // Escape reaches the host as `CANCEL` (the desktop UI's gate sends exactly
    // this selector) and a cancel is a pure state drop: no write, no commit.
    session.command("CANCEL").expect("CANCEL is admitted");
    assert!(
        !session.core.grip_drag_active(),
        "CANCEL drops the armed drag"
    );
    let after = session.snapshot(None, None, 0);
    assert!(
        !after.grip_drag,
        "a dropped drag is projected as `false`, never left stale"
    );
}

#[test]
fn import_progress_projects_only_a_live_import_and_never_invents_counts() {
    // A pre-tick running snapshot: no phase, no totals, no bytes — the UI
    // must render indeterminate rather than a fabricated 0/0.
    let running = CadImportProgress::from_snapshot(&cad_app::ImportProgressSnapshot::running())
        .expect("a running import projects a progress block");
    assert_eq!(running.phase, None);
    assert_eq!(running.entities_done, 0);
    assert_eq!(running.entities_total, None);
    assert_eq!(running.bytes, None);
    assert!(running.cancellable);

    // A terminal (opened) snapshot is never exposed: the affordance must not
    // persist after the document is published.
    let opened =
        cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Opened { entities: 3 });
    assert_eq!(CadImportProgress::from_snapshot(&opened), None);
}

#[test]
fn session_builds_and_reports_the_demo_document() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let status = session.snapshot(None, None, 0);
    assert!(status.document_label.is_some());
    assert!(status.entity_count.is_some());
    assert!(status.can_fit);
    assert!(!status.loading);
    assert_eq!(status.import_count, 0, "the demo document is not an import");
    assert!(status.camera.is_some(), "the demo view is a 2D plan view");
}

#[test]
fn resource_install_failure_is_propagated_not_ignored() {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(FailingResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: false,
            plot: false,
            import_worker: false,
        },
    };
    let result = CadSession::new(options(), platform, RenderSessionHandle::default());
    assert!(matches!(result, Err(CadError::Unsupported(_))));
}
