//! Android host assembly (spec v2.0 §5.3, §9.1).
//!
//! The platform-common orchestration (command routing, state push, the
//! open/plot/save funnels and the interaction dispatch) lives in the UI-free
//! `cad_host::HostCore`, reached through its `cad_engine::protocol::EngineClient`
//! surface. This crate is the Android composition root: it builds the core over
//! the Android platform impls ([`platform`]), creates the shared UI-free render
//! session, installs the Slint adapter/bridge around it, attaches the
//! [`EngineAdapter`] as the single callback binding, keeps the genuinely
//! Android-specific adapters (the [`android_fonts`] asset loader, the [`client`]
//! poll re-arm/resource injection and the [`poll`] Slint timer) and owns the
//! event loop. The legacy [`view::AndroidViewInput`] remains compiled for the
//! host-target tests but is not installed on the attached path; the old
//! draw-command sink was removed (draw transactions are engine-owned).
//!
//! No CAD logic is duplicated here; the database stays the single authority and
//! the shell is a derived presentation of it.

use std::rc::Rc;

use cad_app::{Command, CommandId};

use cad_domain::*;

use cad_engine::render::{BackendPreference, RenderSessionHandle};

use cad_host::{HostCore, HostOptions, HostPlatform};

use cad_ui_slint::{
    install_existing_cad_bridge, EngineAdapter, LayoutSwitchSink, UiAdapter, UiCommandSink,
    UiConfiguration,
};

/// Android host font loading from APK assets (`assets/fonts/…`).
#[cfg(target_os = "android")]
mod android_fonts;

mod client;
mod platform;
mod poll;
#[cfg(target_os = "android")]
mod saf;
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

/// Legacy Android command sink.
///
/// Reproduced for source compatibility and for the pre-attach fallback the
/// adapter still requires (`UiAdapter::new`). Once [`EngineAdapter::attach`]
/// binds the engine client, every callback routes through the engine dispatch
/// and this sink is never invoked, so the two paths cannot double-send. It also
/// carries the legacy `LayoutSwitchSink` implementation retained for the
/// host-target tests; the live adapter lets a layout click flow through the
/// engine `SwitchSpace` command path instead of installing this sink.
#[derive(Clone)]
pub(crate) struct AndroidSink {
    core: HostCore,
}

impl UiCommandSink for AndroidSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        let is_open = matches!(command.id, CommandId::OpenDrawing);
        let is_save = matches!(
            command.id,
            CommandId::SaveDrawingAs | CommandId::PlotDrawing
        );
        let result = state_push::run_command(&self.core, command);
        if self.core.loading() {
            // A picker/read resolved to an import in flight: show the Android
            // "opening in background" status and keep the poll timer running for
            // the publish.
            let _ = self.core.set_catalog_status("android.opening", &[]);
            poll::ensure_polling();
        } else if is_open || is_save {
            // An open or a save picker is asynchronous: keep the poll timer so
            // the SAF result is drained on a later tick. A synchronous outcome
            // already graded within the gesture still gets one tick to settle and
            // refresh the overlay; the tick stops itself when nothing pends.
            poll::ensure_polling();
        }
        result
    }
}

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command.
///
/// Legacy compatibility path retained for the host-target tests. The live
/// adapter routes a layout-row click through the engine `SwitchSpace` command
/// (the shared path every other host command uses); the command layer validates
/// the layout against the drawing and refuses an unknown one, and this sink
/// never mutates the database.
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

    let platform = HostPlatform {
        files: Rc::new(platform::AndroidFiles::new(configuration.sample_paths)),
        config_store: Rc::new(platform::AndroidConfigStore),
        surface: Rc::new(platform::AndroidSurface),
        resources: Rc::new(platform::AndroidResources),
        // Open/New/Save are wired (SAF open/save + a blank-document factory on
        // the shared core); plot stays explicitly unsupported on Android (there
        // is no vector-export surface here). Android runs a real background
        // import worker, so an open is asynchronous and driven by the Slint timer.
        capabilities: cad_platform::HostCapabilities {
            open: true,
            new: true,
            save: true,
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
    };
    // The UI-free render session is created here and shared by the host (which
    // owns the authoritative CAD snapshots) and the Slint bridge (which presents
    // the frames). The explicit WebGpu preference preserves the historical
    // Android selection (the old `install` default); `EngineAdapter::attach`
    // later rejects a facade/client pair that does not share this exact session.
    let render = RenderSessionHandle::new(BackendPreference::WebGpu);
    let core = HostCore::new(host_options, platform, render.clone())?;
    let _ = core.controller.borrow_mut().fit();
    let (document_id, viewport_id) = {
        let controller = core.controller.borrow();
        (controller.document_id, controller.viewport_id)
    };

    let ui_config = UiConfiguration {
        compact: true,
        locale: "zh-CN".into(),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD".into(),
        logical_size: DEMO_LOGICAL_SIZE,
        document: document_id,
        viewport: viewport_id,
    };

    // The legacy command sink is retained for source compatibility only: once the
    // engine client is attached below, every callback routes through
    // `EngineAdapter` and the two paths cannot double-send.
    let sink = AndroidSink { core: core.clone() };
    let mut adapter = UiAdapter::new(ui_config, sink, true)?;
    let handle = adapter.handle();
    // Install Slint's renderer hooks around the engine-owned session (no second
    // session is created here).
    let view = install_existing_cad_bridge(handle.clone(), adapter.window(), render.clone())?;
    // The single callback binding: the engine owns command and interaction
    // dispatch; the Android wrapper only re-arms the async file poll.
    let binding = EngineAdapter::attach(
        &mut adapter,
        Rc::new(client::AndroidEngineClient::new(core.clone())),
        view,
    )?;
    // Seed the surface from the configured logical size; a later rotation/resize
    // calls `set_surface_size` once the Activity forwards the size (audit U07).
    platform::set_surface_slot(DEMO_LOGICAL_SIZE, 1.0);
    core.metrics()?;
    // Replace the shell's "canvas not connected" scaffold status with the host's
    // real started state (resolved through the caller's catalog on flush).
    core.set_catalog_status("android.ready", &[])?;
    // Populate every panel from real application state and mirror the initial
    // camera now that both the shell and the render bridge exist.
    binding.flush_updates()?;
    // No legacy `ViewInput` is installed: pinch, pointer and pick all route
    // through the single attached `EngineAdapter` dispatch. Installing the
    // legacy `AndroidViewInput` here would be a second attached path.
    // Install the P3 queue wake bridge: the engine calls it only on an
    // empty→nonempty edge and re-arms while work remains; it only schedules a
    // single-shot (never a synchronous pump, never a permanent timer). The
    // scheduler is kept weakly by the callback and strongly by the runtime.
    let request_waker = poll::RequestWaker::new();
    binding.set_request_waker(Some(request_waker.install()))?;
    // Register the live core and its UI binding for the async poll timer and the
    // Activity's surface-size entry points. All run on this UI thread.
    poll::install_runtime(
        core,
        Some(poll::UiBinding {
            binding,
            handle,
            _request_waker: request_waker,
        }),
    );
    adapter.run()
}

/// Android entry point (`android-activity` 0.6 calls `fn android_main(app)`).
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    // The headless emulator's goldfish/SwiftShader Vulkan reports a
    // non-Khronos-conformant driver, which wgpu hides by default; this documented
    // escape hatch exposes it (a no-op on conformant real-device drivers). Set
    // before the wgpu instance is created (`select_wgpu_backend` reads it).
    if std::env::var_os("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER").is_none() {
        std::env::set_var("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER", "1");
    }
    // Capture the asset manager before Slint consumes the app handle; the
    // returned manager is documented to remain valid for the process.
    android_fonts::set_asset_manager(app.asset_manager());
    // Capture the JavaVM + activity for the SAF bridge. `ndk_context` exposes the
    // *Application* as its context, which cannot run `startActivityForResult`, so
    // the activity is taken from the app handle instead.
    saf::set_context(app.vm_as_ptr(), app.activity_as_ptr());
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
