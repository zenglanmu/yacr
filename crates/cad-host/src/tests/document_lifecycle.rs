//! Start-blank startup and the host-owned `CLOSE` lifecycle.
//!
//! These are UI-free contracts: they drive `HostCore` directly and assert the
//! authoritative document + the derived render/status inputs. The `CadStatus`
//! projection itself is a `cad-ui-core` concern and is covered there.

use super::*;

/// A core built like a production host: `start_blank: true`.
fn blank_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.start_blank = true;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("blank host core")
}

#[test]
fn start_blank_installs_a_real_untitled_document_without_demo_geometry() {
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
fn the_historical_demo_document_is_still_the_default() {
    // `start_blank: false` (the test fixtures' default) keeps the demo.
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

    // A second CLOSE is refused explicitly (never an empty success).
    let error = core
        .command(CommandId::CloseDrawing, CommandPayload::None)
        .expect_err("closing nothing is an explicit refusal");
    match error {
        CadError::InvalidInput(reason) => assert_eq!(reason, "document not open"),
        other => panic!("expected an explicit InvalidInput, got {other:?}"),
    }
}
