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
