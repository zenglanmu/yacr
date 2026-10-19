//! Status projection, diagnostics text and session-build contracts.

use super::super::super::*;
use super::super::test_support::*;

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
        None,
    );
    assert_eq!(status.document_label, None);
    assert_eq!(status.entity_count, None);
    assert_eq!(status.camera, None, "a missing document has no camera");
    assert!(!status.can_fit);
    assert!(status.can_open);
    assert_eq!(status.import_count, 0);
    assert_eq!(status.frame_count, 0);
    assert!(
        !status.hover_preview,
        "the assembled status carries the flag it was given"
    );
    assert!(
        !status.snap_enabled,
        "the assembled status carries the object-snap flag it was given"
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
        None,
    );
    assert_eq!(status.document_label.as_deref(), Some("fixture.dwg"));
    assert_eq!(status.entity_count, Some(3));
    assert!(status.can_fit);
    assert_eq!(status.frame_count, 10);
    assert!(
        status.hover_preview,
        "the status projects the session's hover preview flag"
    );
    assert!(
        status.snap_enabled,
        "the status projects the session's object-snap flag"
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
