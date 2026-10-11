//! Android host assembly (spec v2.0 §5.3, §9.1).
//!
//! The platform-common orchestration (command routing, state push, the
//! open/plot/save funnels, the navigation/pick/draw sinks) lives in
//! `cad_host::HostCore`. This crate is the Android composition root: it builds
//! the core over the Android platform impls ([`platform`]), installs the Slint
//! adapter and the CAD render bridge, keeps the genuinely Android-specific
//! adapters (the [`view::AndroidViewInput`] `InputPolicy` state machine, the
//! [`draw`] command sink, the [`android_fonts`] asset loader and the [`poll`]
//! Slint timer) and owns the event loop.
//!
//! No CAD logic is duplicated here; the database stays the single authority and
//! the shell is a derived presentation of it.

use std::cell::RefCell;

use std::rc::Rc;

use cad_app::{Command, CommandId};

use cad_domain::*;

use cad_host::{HostCore, HostOptions, HostPlatform};

use cad_ui_slint::{
    IncomingDocument, LayoutSwitchSink, UiAdapter, UiCommandSink, UiConfiguration, UiHandle,
};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

/// Android host font loading from APK assets (`assets/fonts/…`).
#[cfg(target_os = "android")]
mod android_fonts;

mod draw;
mod platform;
mod poll;
mod state_push;
mod view;

/// Initial logical size of the demo viewport. The UI configuration and the
/// initial viewport must agree; real dimensions arrive from the surface later.
const DEMO_LOGICAL_SIZE: [f64; 2] = [1080.0, 1920.0];

/// Host configuration (sample DWG locations to try on open).
#[derive(Clone)]
pub struct AndroidHostConfiguration {
    /// Candidate DWG locations to try on open (app-private/external dirs).
    pub sample_paths: Vec<String>,
}

/// The Android command funnel: run through the shared core, then re-apply the
/// Android overlay (import progress + resources drawer) so a command cannot
/// clobber them or claim images are modeled.
///
/// Cloneable so the same funnel can be installed as the adapter's
/// `UiCommandSink` **and** as the layout-switch sink: a layout click must run the
/// exact `SwitchSpace` command path (validate against the drawing, re-sync the
/// camera and panels) rather than a parallel path.
#[derive(Clone)]
pub(crate) struct AndroidSink {
    core: HostCore,
}

impl UiCommandSink for AndroidSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        let is_open = matches!(command.id, CommandId::OpenDrawing);
        let result = state_push::run_command(&self.core, command);
        if self.core.loading() {
            // The sample probe resolved to an import in flight: there is no
            // picker, so show the Android "opening in background" status and keep
            // the poll timer running for the publish.
            let _ = self
                .core
                .set_status(self.core.message("android.opening", &[]));
            poll::ensure_polling();
        } else if is_open {
            // A synchronous open (or a synchronous failure already graded within
            // the gesture) still gets one tick so a pending outcome settles and
            // the overlay is refreshed; the tick stops itself when nothing pends.
            poll::ensure_polling();
        }
        result
    }
}

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command.
///
/// Installed on the adapter so a layout-row click runs through the same command
/// path every other host command uses. The command layer validates the layout
/// against the drawing and refuses an unknown one; the sink never mutates the
/// database.
impl LayoutSwitchSink for AndroidSink {
    fn select(&mut self, space: cad_representation::SpaceSelection) {
        let (document, viewport) = {
            let c = self.core.controller.borrow();
            (c.document_id, c.viewport_id)
        };
        let command = Command {
            schema_version: 1,
            id: CommandId::SwitchSpace,
            document,
            viewport,
            payload: cad_app::input::space_switch_payload(space),
        };
        let _ = state_push::run_command(&self.core, command);
    }
}

/// Build the shared UI + core + renderer stack and run it.
pub fn start(configuration: AndroidHostConfiguration) -> CadResult<()> {
    log::info!("yacr android host starting");
    cad_ui_slint::select_wgpu_backend()?;

    let handle_slot: SharedHandle = Rc::new(RefCell::new(None));
    let platform = HostPlatform {
        files: Rc::new(platform::AndroidFiles::new(configuration.sample_paths)),
        config_store: Rc::new(platform::AndroidConfigStore),
        surface: Rc::new(platform::AndroidSurface),
        resources: Rc::new(platform::AndroidResources),
        // File open is wired (sample-path probe); New/Save/Plot stay off this
        // phase (P4 adds SAF open/save). Android runs a real background import
        // worker, so the open is asynchronous and driven by the Slint timer.
        capabilities: cad_platform::HostCapabilities {
            open: true,
            new: false,
            save: false,
            plot: false,
            import_worker: poll::worker_available(),
        },
    };
    let host_options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: DEMO_LOGICAL_SIZE,
        locale: "zh-CN".into(),
    };
    let core = HostCore::new(host_options, handle_slot, platform)?;
    let controller = core.controller.clone();
    let (drawing, document_id, viewport_id) = {
        let mut controller = controller.borrow_mut();
        let _ = controller.fit();
        (
            controller.drawing(),
            controller.document_id,
            controller.viewport_id,
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

    let sink = AndroidSink { core: core.clone() };
    let layout_sink = sink.clone();
    let adapter = UiAdapter::new(ui_config, sink, true)?;
    adapter.set_layout_switch_sink(Box::new(layout_sink));
    let handle = adapter.handle();
    core.set_handle(handle.clone());
    // Replace the shell's "canvas not connected" scaffold status with the host's
    // real started state.
    let _ = handle.set_status(core.message("android.ready", &[]));
    // Seed the surface from the configured logical size; a later rotation/resize
    // calls `set_surface_size` once the Activity forwards the size (audit U07).
    platform::set_surface_slot(DEMO_LOGICAL_SIZE, 1.0);
    let view =
        cad_ui_slint::install_cad_bridge(handle.clone(), adapter.window(), incoming.clone())?;
    core.set_view(view);
    // Map the surface onto the authoritative viewport now that both the shell and
    // the render bridge exist.
    core.metrics()?;
    // The shared core resolves a canvas-local logical point to a world point on
    // the viewport work plane (the same mapper every host installs).
    adapter.set_canvas_pick_mapper(Rc::new(core.clone()));
    // Draw/edit tools: the shell captures points and emits an intent; the host
    // maps it to one drawing command (line/circle/move/trim). Without this the
    // confirm path reports "not wired" instead of fabricating a command.
    draw::install_draw_sinks(&adapter, core.clone(), viewport_id);
    // Wire real canvas interaction: drag pans, wheel/pinch zooms and a real tap
    // selects, all through the shared command funnel.
    adapter.set_view_input(Rc::new(view::AndroidViewInput::new(
        core.clone(),
        viewport_id,
    )));
    // Register the live core for the async poll timer and the Activity's
    // surface-size entry points. All run on this UI thread.
    poll::install_runtime(core.clone());
    // Populate every panel from real application state now that both the shell
    // and the render bridge exist (docs/ui.md §3, docs/panels.md §3.2).
    state_push::push_android_state(&core);
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

// The Activity calls `set_surface_size` from outside the crate, so it is public
// at the crate root; the rest of the host wiring stays crate-internal.
pub use poll::{set_surface_insets, set_surface_size};

#[cfg(test)]
pub(crate) use platform::{AndroidConfigStore, AndroidFiles, AndroidResources, AndroidSurface};
#[cfg(test)]
pub(crate) use poll::install_runtime;
#[cfg(test)]
pub(crate) use state_push::*;
#[cfg(test)]
pub(crate) use view::AndroidViewInput;

#[cfg(test)]
mod tests;
