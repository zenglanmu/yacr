//! Startup document modes and the host-owned `CLOSE` lifecycle.
//!
//! These are UI-free contracts: they drive `HostCore` directly and assert the
//! authoritative document + the derived render/status inputs. The `CadStatus`
//! projection itself is a `cad-ui-core` concern and is covered there
//! (`blank_status_reports_no_document_capabilities` for the pure projection and
//! `empty_start_projects_the_start_page` for a session built with
//! [`StartDocument::Empty`]).

use super::*;

/// A core built like a production carrier: no document at startup.
fn empty_core() -> HostCore {
    empty_core_over(Rc::new(FakeFiles::default()), options())
}

/// The same core over a caller-owned file sink, so a test can observe the
/// platform reads the startup path requested.
fn empty_core_over(files: Rc<FakeFiles>, mut options: HostOptions) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    options.document_at_start = StartDocument::Empty;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("empty host core")
}

/// A core built with the explicit empty `untitled` drawing.
fn blank_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.document_at_start = StartDocument::Blank;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("blank host core")
}

#[test]
fn empty_start_installs_no_document_and_no_render_scene() {
    let core = empty_core();
    let controller = core.controller.borrow();
    assert!(
        controller.drawing().is_none(),
        "an empty start must not fabricate an authoritative document"
    );
    assert_eq!(
        controller.document_name_hint, "untitled",
        "the honest save/plot stem survives the missing document"
    );
    assert!(controller.last_import_report.is_none());
    assert!(
        controller.last_status.is_empty(),
        "a document-less start makes no readiness claim"
    );
    drop(controller);

    // The render side was never handed a drawing (HostCore::new only seeds it
    // when one exists), exactly like the post-CLOSE state.
    assert!(
        core.render.incoming_document().borrow().is_none(),
        "the CAD partition must start with no incoming document"
    );
    // Post-CLOSE vendor invariant: the first tick publishes `Some([])`.
    assert!(
        matches!(core.take_vendor_records(), Some(records) if records.is_empty()),
        "a document-less start clears any previous vendor pass like CLOSE does"
    );

    // Nothing is in flight, so the startup path is a clean no-op (in particular
    // no `fit()` is attempted for a drawing that does not exist).
    core.open_configured_drawing()
        .expect("an empty start with no --open drawing has nothing to open");
    assert!(!core.loading(), "no platform read was started");

    // Closing nothing is refused explicitly, exactly like after a real CLOSE.
    let error = core
        .command(CommandId::CloseDrawing, CommandPayload::None)
        .expect_err("closing nothing is an explicit refusal");
    match error {
        CadError::InvalidInput(reason) => assert_eq!(reason, "document not open"),
        other => panic!("expected an explicit InvalidInput, got {other:?}"),
    }
}

#[test]
fn empty_start_still_runs_the_configured_open() {
    let files = Rc::new(FakeFiles::default());
    let mut options = options();
    let path = PathBuf::from("/tmp/yacr-startup-fixture.dwg");
    options.drawing = Some(path.clone());
    let core = empty_core_over(files.clone(), options);
    assert!(
        core.controller.borrow().drawing().is_none(),
        "the configured open has not delivered bytes yet"
    );

    core.open_configured_drawing()
        .expect("a configured --open drawing is opened from an empty start");

    assert_eq!(
        files.opened_target.borrow().as_ref(),
        Some(&FileTarget::from_path(&path.to_string_lossy())),
        "the startup open reaches the platform even with no initial document"
    );
    assert!(core.loading(), "the platform read is in flight");
    assert!(
        core.controller.borrow().drawing().is_none(),
        "the document appears only when the delivered bytes are installed"
    );
}

#[test]
fn blank_start_installs_a_real_untitled_document_without_demo_geometry() {
    let core = blank_core();
    let controller = core.controller.borrow();
    let drawing = controller
        .drawing()
        .expect("a blank session still owns an authoritative document");
    assert_eq!(drawing.entity_count(), 0, "no demo geometry may survive");
    assert!(drawing.bounds().is_none(), "a blank drawing has no extent");
    assert_eq!(controller.document_name_hint, "untitled");
    assert!(controller.last_import_report.is_none());
    let generation = controller.session.generation;
    drop(controller);

    // The render side was published the chosen document at construction (not
    // the demo the controller was not asked for).
    assert!(
        core.render.incoming_document().borrow().is_some(),
        "HostCore::new must publish the blank drawing"
    );
    // No extent means the startup path must not claim a fit.
    core.controller.borrow_mut().fit().unwrap();
    assert_eq!(
        core.controller.borrow().session.generation,
        generation,
        "fitting a blank drawing is a documented no-op"
    );
}

#[test]
fn the_historical_demo_document_is_still_the_fixture_default() {
    // `StartDocument::Demo` (the test fixtures' default) keeps the demo; NEW
    // from such a session still replaces it with a real untitled drawing.
    let core = core();
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("demo document");
    assert!(drawing.entity_count() > 0, "the demo still has geometry");
    assert_eq!(controller.document_name_hint, "yacr-demo");
}

#[test]
fn close_through_the_command_funnel_drops_the_document_and_the_render_scene() {
    let core = core();
    assert!(core.controller.borrow().drawing().is_some());
    assert!(core.render.incoming_document().borrow().is_some());

    core.command(CommandId::CloseDrawing, CommandPayload::None)
        .expect("CLOSE is admitted through the normal funnel");

    assert!(
        core.controller.borrow().drawing().is_none(),
        "CLOSE removes the authoritative document"
    );
    assert!(
        core.render.incoming_document().borrow().is_none(),
        "the render side must be told there is no drawing"
    );
    assert!(!core.loading());

    // A closed session and an empty start are the same state: NEW is the way
    // back, and it installs the real untitled document (`Blank` truth).
    core.command(CommandId::NewDrawing, CommandPayload::None)
        .expect("NEW is admitted with no document open");
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("NEW installs a document");
    assert_eq!(drawing.entity_count(), 0);
    assert_eq!(controller.document_name_hint, "untitled");
}
