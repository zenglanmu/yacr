#![cfg(target_os = "linux")]
//! Idle-redraw regression: pushing an identical view snapshot must not schedule
//! a repaint, and a changed camera must. The Linux host re-pushes the session
//! on a 50 ms timer; before the fix that redrew an idle view at ~20 fps.
use cad_app::{Command, CommandId};
use cad_domain::CadResult;
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

struct Sink(Rc<RefCell<Vec<CommandId>>>);
impl UiCommandSink for Sink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        self.0.borrow_mut().push(command.id);
        Ok(())
    }
}

#[test]
fn identical_view_snapshots_do_not_request_extra_redraws() {
    offscreen::install().unwrap();
    let adapter = UiAdapter::new(
        UiConfiguration::default(),
        Sink(Rc::new(RefCell::new(Vec::new()))),
        true,
    )
    .unwrap();
    let initial_size = adapter.handle().cad_surface_size().unwrap().0;
    let mut host = cad_app::host::HostController::with_demo_document(initial_size).unwrap();
    host.fit().unwrap();
    let incoming = Rc::new(RefCell::new(Some(host.drawing().unwrap())));
    let view =
        cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming).unwrap();
    adapter.component().show().unwrap();
    // install_cad_bridge itself schedules one redraw.
    let baseline = view.redraw_requests();
    assert!(baseline >= 1);

    // First push from the default snapshot changes the view and redraws.
    view.sync_session(
        &host.session.active_space,
        &host.application.workspace.viewports[&host.viewport_id],
    );
    let after_first = view.redraw_requests();
    assert!(
        after_first > baseline,
        "a changed snapshot must schedule a redraw (install={baseline}, after={after_first})"
    );

    // Re-pushing the identical snapshot repeatedly (the host's 50 ms timer
    // pattern) must not schedule any further redraws.
    for _ in 0..5 {
        view.sync_session(
            &host.session.active_space,
            &host.application.workspace.viewports[&host.viewport_id],
        );
    }
    assert_eq!(
        view.redraw_requests(),
        after_first,
        "identical snapshots must not keep requesting redraws"
    );

    // A camera change must invalidate and redraw again.
    {
        let viewport = host
            .application
            .workspace
            .viewports
            .get_mut(&host.viewport_id)
            .unwrap();
        viewport.camera.target.x += 25.0;
    }
    view.sync_session(
        &host.session.active_space,
        &host.application.workspace.viewports[&host.viewport_id],
    );
    assert!(
        view.redraw_requests() > after_first,
        "a camera change must schedule a redraw"
    );
}
