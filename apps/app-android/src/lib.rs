use std::cell::{Cell, RefCell};

use std::rc::Rc;

use std::sync::Arc;

use cad_app::host::HostController;

use cad_app::input::{InputOutcome, InputPolicy, PointerPhase, PointerUpdate};

use cad_app::{Command, CommandId, CommandPayload};

use cad_domain::*;

use cad_ui_slint::{
    CadView, IncomingDocument, UiAdapter, UiCommandSink, UiConfiguration, UiHandle, ViewInput,
};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

type SharedView = Rc<RefCell<Option<CadView>>>;

/// Android host font loading from APK assets (`assets/fonts/…`).
#[cfg(target_os = "android")]
mod android_fonts;

/// Initial logical size of the demo viewport. The UI configuration and the
/// initial viewport must agree; real dimensions arrive from the surface later.
const DEMO_LOGICAL_SIZE: [f64; 2] = [1080.0, 1920.0];

/// Status shown once the shell and its CAD canvas are actually connected.
///
/// The shell's built-in default (`status.scaffold`) claims the canvas is not
/// wired yet; that is true only before `install_cad_bridge` runs, so the host
/// replaces it instead of leaving the stale placeholder on screen.
const READY_STATUS: &str = "就绪（内置演示几何，非兼容性声明）";

pub struct AndroidHostConfiguration {
    /// Candidate DWG locations to try on open (app-private/external dirs).
    pub sample_paths: Vec<String>,
}

/// Canvas navigation for Android: one-finger drag pans, wheel/pinch zooms.
///
/// The shell's `TouchArea` emits logical-pixel pointer events. This host turns
/// them into the same `Pan`/`Zoom` commands the web host uses, so the
/// authoritative viewport stays the single camera truth. Without this the
/// `pointer-input`/`scroll-input` callbacks were dropped and the Android canvas
/// could not be panned or zoomed at all (verified: a swipe changed 0 pixels).
struct AndroidViewInput {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    viewport: ViewportId,
    last: Cell<[f64; 2]>,
    dragging: Cell<bool>,
    /// Shared pointer/gesture state machine (audit U05). Used to tell a tap from
    /// a drag so a one-finger drag pans and only a real tap turns into a
    /// selection pick.
    policy: RefCell<InputPolicy>,
}

/// Commands from the UI are executed through the shared application layer.
struct HostSink {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    incoming: IncomingDocument,
    configuration: AndroidHostConfiguration,
}

/// Build the shared UI + core + renderer stack and run it.
pub fn start(configuration: AndroidHostConfiguration) -> CadResult<()> {
    log::info!("yacr android host starting");
    cad_ui_slint::select_wgpu_backend()?;

    let controller = Rc::new(RefCell::new(HostController::with_demo_document(
        DEMO_LOGICAL_SIZE,
    )?));
    let (drawing, document_id, viewport_id) = {
        let mut controller = controller.borrow_mut();
        let _ = controller.fit();
        (
            controller.drawing(),
            controller.document_id.clone(),
            controller.viewport_id.clone(),
        )
    };
    let incoming: IncomingDocument = Rc::new(RefCell::new(drawing));

    let ui_config = UiConfiguration {
        compact: true,
        locale: "zh-CN".into(),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD".into(),
        logical_size: DEMO_LOGICAL_SIZE,
        document: document_id,
        viewport: viewport_id,
    };

    // The sink needs the UI handle, which only exists after the adapter is
    // built, so it is shared through a slot filled immediately afterwards.
    let shared_handle: SharedHandle = Rc::new(RefCell::new(None));
    let shared_view: SharedView = Rc::new(RefCell::new(None));
    let sink = HostSink {
        controller: controller.clone(),
        handle: shared_handle.clone(),
        view: shared_view.clone(),
        incoming: incoming.clone(),
        configuration,
    };
    let adapter = UiAdapter::new(ui_config, sink, true)?;
    let handle = adapter.handle();
    *shared_handle.borrow_mut() = Some(handle.clone());
    // Replace the shell's "canvas not connected" scaffold status with the
    // host's real started state.
    let _ = handle.set_status(READY_STATUS);
    let view =
        cad_ui_slint::install_cad_bridge(handle.clone(), adapter.window(), incoming.clone())?;
    sync_view_camera(&shared_view, &controller, viewport_id);
    // Establish the surface→viewport sizing seam (U07). The configured logical
    // size is the initial surface; a later rotation/resize calls the same helper
    // once the Activity forwards the size (see docs/validation-android.md §8).
    apply_surface_size(&controller, DEMO_LOGICAL_SIZE, 1.0)?;
    // Install the logical-pixel → world mapper so a canvas tap with a measure or
    // annotation tool produces a real point instead of "取点未接线" (audit U04).
    adapter.set_canvas_pick_mapper(Rc::new(AndroidCanvasPickMapper::new(
        controller.clone(),
        shared_handle.clone(),
    )));
    // Draw/edit tools: the shell captures points and emits an intent; the host
    // maps it to one drawing command (line/circle/move/trim). Without this the
    // confirm path reports "not wired" instead of fabricating a command.
    install_draw_sinks(
        &adapter,
        controller.clone(),
        shared_handle.clone(),
        shared_view.clone(),
        viewport_id,
    );
    // Wire real canvas interaction: drag pans and wheel/pinch zooms through the
    // same `Pan`/`Zoom` commands the web host uses. A tap with no capture tool
    // active is routed to selection (`pick_at_screen`), not to a fake hit.
    adapter.set_view_input(Rc::new(AndroidViewInput {
        controller: controller.clone(),
        handle: shared_handle.clone(),
        view: shared_view.clone(),
        viewport: viewport_id,
        last: Cell::new([0.0, 0.0]),
        dragging: Cell::new(false),
        policy: RefCell::new(InputPolicy::new()),
    }));
    // Layout (paper-space) switching intentionally uses the adapter's default
    // command path: with no `LayoutSwitchSink` installed, `on_layout_selected`
    // emits `CommandId::SwitchSpace` + `CommandPayload::Space(...)`, which
    // `HostSink::send` runs through `HostController::execute` and then re-syncs
    // the camera and the layout panel. Installing a sink here would *replace*
    // that command, so it is deliberately not installed (see `docs/ui.md` §3.2
    // and the `android_layout_selection_routes_through_switch_space` test).
    *shared_view.borrow_mut() = Some(view);
    // Register the live objects for the async poll timer and the Activity's
    // surface-size entry point (`set_surface_size`). Both run on this UI thread.
    install_runtime(
        controller.clone(),
        shared_handle.clone(),
        shared_view.clone(),
        incoming.clone(),
    );
    // Populate every panel from real application state now that both the shell
    // and the render bridge exist (docs/ui.md §3, docs/panels.md §3.2).
    push_panel_state(&controller, &handle, &shared_view);
    adapter.run()
}

/// Android entry point (`android-activity` 0.6 calls `fn android_main(app)`).
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    // Capture the asset manager before Slint consumes the app handle; the
    // returned manager is documented to remain valid for the process.
    android_fonts::set_asset_manager(app.asset_manager());
    if let Err(e) = slint::android::init(app) {
        log::error!("slint android init failed: {e}");
        return;
    }
    if let Err(e) = start(AndroidHostConfiguration::default()) {
        log::error!("yacr android host failed: {e}");
    }
}

mod draw;
mod host;
mod poll;
mod state_push;
mod view;

// The Activity calls `set_surface_size` from outside the crate, so it is public
// at the crate root; the rest of the host wiring stays crate-internal.
pub use poll::set_surface_size;
pub(crate) use poll::{ensure_polling, install_runtime, worker_available};
// The poll-processing items are Android-only (their tests compile for the
// Android target); `set_surface_size`/`apply_surface_resize` are target-agnostic.
pub(crate) use draw::install_draw_sinks;
#[cfg(target_os = "android")]
pub(crate) use poll::{apply_surface_resize, poll_import_once, ImportPollOutcome};
pub(crate) use state_push::*;
pub(crate) use view::*;

#[cfg(test)]
mod tests;
