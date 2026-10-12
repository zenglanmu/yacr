//! Platform-common host orchestration.
//!
//! [`HostCore`] owns the controller/Slint bridge and the open/save/plot funnels
//! that were previously duplicated inside the desktop host. It is a `Clone`
//! handle over shared `Rc`/`Arc` state and implements the shell's sink traits
//! ([`cad_ui_slint::UiCommandSink`], `LayoutSwitchSink`, `ViewInput`,
//! `CanvasPickMapper`, `DrawCommandSink`, `DrawPreviewSink`).
//!
//! All platform I/O is expressed through the `cad-platform` contracts
//! ([`cad_platform::PlatformFiles`], [`cad_platform::ConfigStore`],
//! [`cad_platform::SurfaceSource`]); this crate never imports `slint`, `wgpu`,
//! `web-sys` or any Android type, and carries no `cfg` for a host platform.
//!
//! The business path is unchanged: `HostController::execute` still runs every
//! command, the database stays the single authority, and the open/save/plot
//! handlers keep their exact status grading and corruption guards.
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandOutcome, CommandPayload, ImportProgressSnapshot};
use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_slint::{CadView, LayoutSwitchSink, UiCommandSink, UiHandle};

pub(crate) fn pick_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
}

mod fonts;
mod input;
mod open;
pub mod policy;
mod save;
mod state;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod zoom_anchor_tests;

#[cfg(not(target_arch = "wasm32"))]
pub use fonts::DesktopResources;
pub use fonts::{HostResources, UnsupportedResources};

/// Host policy mirroring the web host: users may toggle the two dock panels'
/// initial-open flags. A host `config.json` replaces this policy wholesale.
pub const HOST_ALLOWED_PATHS_JSON: &str = r#"{"ui":{"userCustomization":{"allowedPaths":[
    "ui.components.layerPanel.initiallyOpen",
    "ui.components.propertiesPanel.initiallyOpen"
]}}}"#;

/// A user-facing rendering of a command error.
///
/// `CadError::Display` delegates to `Debug`, so `to_string()` would print
/// `Unsupported("…")`/`NotImplemented("…")` with Debug quoting. This unwraps
/// those two variants to their inner (already-localized) string and falls back
/// to `Display` for the rest.
pub(crate) fn error_text(error: &CadError) -> String {
    match error {
        CadError::Unsupported(reason) => reason.clone(),
        CadError::NotImplemented(feature) => (*feature).to_string(),
        other => other.to_string(),
    }
}

fn use_light_navigation_push(
    id: CommandId,
    succeeded: bool,
    viewport_matches: bool,
    has_drawing: bool,
    has_handle: bool,
    has_view: bool,
    loading: bool,
) -> bool {
    succeeded
        && matches!(id, CommandId::Pan | CommandId::Zoom)
        && viewport_matches
        && has_drawing
        && has_handle
        && has_view
        && !loading
}

/// Select viewport metrics shared by rendering and canvas-local input.
///
/// A live UI handle supplies the actual CAD rectangle (excluding chrome) and
/// window scale. With no handle, retain the platform surface + safe-inset path
/// used by headless hosts and tests.
fn select_canvas_metrics(
    surface: SurfaceMetrics,
    cad_surface: Option<([f64; 2], f64)>,
) -> CadResult<cad_app::input::CanvasMetrics> {
    // SurfaceSource implementations do not guarantee finite dimensions/scale
    // or valid insets. Validate the platform contract even when a UI handle
    // provides the authoritative CAD rectangle; do not use the inset-adjusted
    // result to size that live canvas.
    cad_app::input::inset_canvas_metrics(
        [0.0; 2],
        surface.logical,
        surface.safe_insets,
        surface.scale,
    )
    .ok_or_else(|| CadError::InvalidInput("canvas metrics are degenerate".into()))?;

    if let Some((size, scale)) = cad_surface {
        // Input commands already use canvas-local coordinates, so the origin
        // must be zero. The renderer consumes this same CAD size and scale.
        Ok(cad_app::input::CanvasMetrics::new([0.0; 2], size, scale))
    } else {
        // The platform metrics were validated above; recompute their inset
        // rectangle as the headless fallback canvas.
        cad_app::input::inset_canvas_metrics(
            [0.0; 2],
            surface.logical,
            surface.safe_insets,
            surface.scale,
        )
        .ok_or_else(|| CadError::InvalidInput("canvas metrics are degenerate".into()))
    }
}

/// Platform-general host options.
///
/// A host-specific options type (for example `LinuxOptions`, which also carries
/// a GPU selection and an acceptance output directory) projects onto this.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// Explicit `--font NAME=PATH` registrations.
    pub fonts: Vec<(String, PathBuf)>,
    /// Explicit CAD font directory (`--fonts-dir`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`).
    pub images_dir: Option<PathBuf>,
    pub headless: bool,
    /// A drawing to open at startup (`--open`) or the headless input.
    pub drawing: Option<PathBuf>,
    pub size: [f64; 2],
    pub locale: String,
}

/// The platform contracts a [`HostCore`] is built from.
///
/// All are single-threaded (`Rc`): the host core runs on one thread and the
/// traits carry no `Send`/`Sync` requirement.
pub struct HostPlatform {
    pub files: Rc<dyn PlatformFiles>,
    pub config_store: Rc<dyn ConfigStore>,
    pub surface: Rc<dyn SurfaceSource>,
    /// How the host resolves the drawing's shaping fonts and raster images
    /// (desktop sibling package, Android APK assets, browser fetch). The open
    /// funnel is platform-general; resource *resolution* is not.
    pub resources: Rc<dyn HostResources>,
    pub capabilities: HostCapabilities,
}

/// A cancel or an already-localized failure from a platform file interaction.
///
/// The reason string is already localized by the platform; the core shows it
/// verbatim rather than re-wrapping it in an error (which would double-format it).
#[derive(Debug, Clone)]
pub(crate) enum FileAbort {
    Cancelled,
    Failed(String),
}

/// A pending save result drained from the platform, with its purpose.
pub(crate) struct PendingSave {
    pub purpose: SavePurpose,
    pub result: Result<FileTarget, FileAbort>,
}

/// Per-instance navigation state (drag/scroll), driven by [`ViewInput`].
pub(crate) struct NavInner {
    pub last: Cell<Option<[f64; 2]>>,
    pub dragged: Cell<bool>,
}

impl Default for NavInner {
    fn default() -> Self {
        Self {
            last: Cell::new(None),
            dragged: Cell::new(false),
        }
    }
}

/// Clone handle over the shared host orchestration state.
#[derive(Clone)]
pub struct HostCore {
    /// The single application/controller instance.
    pub controller: Rc<RefCell<HostController>>,
    /// The shell handle; `None` until the adapter is built.
    pub handle: Rc<RefCell<Option<UiHandle>>>,
    /// The CAD view; `None` until the bridge is installed.
    pub view: Rc<RefCell<Option<CadView>>>,
    pub options: HostOptions,
    pub files: Rc<dyn PlatformFiles>,
    pub config_store: Rc<dyn ConfigStore>,
    pub surface: Rc<dyn SurfaceSource>,
    /// The host's font/image installation strategy (see [`HostResources`]).
    pub resources: Rc<dyn HostResources>,
    pub capabilities: HostCapabilities,
    /// The active catalog locale, **shared** across every clone.
    ///
    /// A runtime locale switch must reach the shared-catalog text produced by
    /// `message()`/`messages()` on *every* clone (the web host holds clones in
    /// the sink, the input sink and the layout sink), so this is one `Rc` slot,
    /// not a per-clone copy of `options.locale`.
    pub(crate) locale: Rc<RefCell<String>>,
    /// A picker/read is in flight (analogous to the old `pending_open`).
    pub(crate) open_pending: Rc<Cell<bool>>,
    /// The drawing captured when an open attempt began, so a superseded result
    /// is discarded.
    pub(crate) open_guard: Rc<RefCell<Option<Arc<DrawingDatabase>>>>,
    /// Set by `CancelLoading`; a delivery that arrives afterwards is discarded
    /// rather than installed (mirrors the old `pending_read.cancelled`).
    pub(crate) open_abort: Rc<Cell<bool>>,
    /// The platform target of the open in flight, retained until the async import
    /// publishes so the source-overwrite guard arms against the real handle.
    pub(crate) open_target: Rc<RefCell<Option<FileTarget>>>,
    /// A localized open failure delivered by the platform.
    pub(crate) open_failure: Rc<RefCell<Option<String>>>,
    /// Whether an open-cancel event was delivered this tick.
    pub(crate) open_cancelled: Rc<Cell<bool>>,
    /// Whether the open funnel graded a cancel/failure status this gesture/tick,
    /// so the generic "completed" must not overwrite it.
    pub(crate) open_status_graded: Rc<Cell<bool>>,
    /// A document open is presenting (bytes delivered, import/presentation).
    pub(crate) presenting_open: Rc<Cell<bool>>,
    /// A save dialog is in flight.
    pub(crate) save_pending: Rc<Cell<bool>>,
    /// A completed save interaction, awaiting export.
    pub(crate) pending_save: Rc<RefCell<Option<PendingSave>>>,
    /// Path of the drawing being opened (or already open), used to resolve its
    /// sibling raster images. Armed only on a **successful** open.
    pub(crate) drawing_path: Rc<RefCell<Option<PathBuf>>>,
    /// Last status text the core graded. Delivered to the shell handle when one
    /// exists; retained so a headless/test core (no window) can observe the exact
    /// graded text and a host can mirror it.
    pub(crate) last_status: Rc<RefCell<Option<String>>>,
    /// Last raster-image loading outcome, surfaced by the diagnostics drawer.
    pub(crate) last_image_report: Rc<RefCell<Option<cad_platform::images::ImageLoadReport>>>,
    /// Last `(physical width, physical height, config revision)` pushed through
    /// `refresh_window_layout`. `None` forces the first refresh.
    pub(crate) last_layout_key: Rc<RefCell<Option<(u32, u32, u64)>>>,
    /// Per-instance pointer navigation state.
    pub(crate) navigation: Rc<NavInner>,
    /// Entity refs captured when a MOVE tool starts; selection may change while
    /// its points are being captured.
    pub(crate) move_selection: Rc<RefCell<Option<Vec<cad_domain::SelectionRef>>>>,
    /// Diagnostics from reading the config / preference files at startup.
    pub(crate) startup_diagnostics: Rc<RefCell<Vec<String>>>,
}

impl HostCore {
    /// Build the core over a demo document and the platform contracts.
    pub fn new(
        options: HostOptions,
        handle: Rc<RefCell<Option<UiHandle>>>,
        platform: HostPlatform,
    ) -> CadResult<Self> {
        let controller = Rc::new(RefCell::new(HostController::with_demo_document(
            options.size,
        )?));
        let locale = Rc::new(RefCell::new(options.locale.clone()));
        Ok(Self {
            controller,
            handle,
            view: Rc::new(RefCell::new(None)),
            options,
            files: platform.files,
            config_store: platform.config_store,
            surface: platform.surface,
            resources: platform.resources,
            capabilities: platform.capabilities,
            locale,
            open_pending: Rc::new(Cell::new(false)),
            open_guard: Rc::new(RefCell::new(None)),
            open_abort: Rc::new(Cell::new(false)),
            open_target: Rc::new(RefCell::new(None)),
            open_failure: Rc::new(RefCell::new(None)),
            open_cancelled: Rc::new(Cell::new(false)),
            open_status_graded: Rc::new(Cell::new(false)),
            presenting_open: Rc::new(Cell::new(false)),
            save_pending: Rc::new(Cell::new(false)),
            pending_save: Rc::new(RefCell::new(None)),
            drawing_path: Rc::new(RefCell::new(None)),
            last_status: Rc::new(RefCell::new(None)),
            last_image_report: Rc::new(RefCell::new(None)),
            last_layout_key: Rc::new(RefCell::new(None)),
            navigation: Rc::new(NavInner::default()),
            move_selection: Rc::new(RefCell::new(None)),
            startup_diagnostics: Rc::new(RefCell::new(Vec::new())),
        })
    }

    /// Install the shell handle (called once the adapter is built).
    pub fn set_handle(&self, handle: UiHandle) {
        *self.handle.borrow_mut() = Some(handle);
    }

    /// Install the CAD view (called once the bridge is installed).
    pub fn set_view(&self, view: CadView) {
        *self.view.borrow_mut() = Some(view);
    }

    /// The localized message for `key` in the host's **current** locale.
    ///
    /// Reads the shared locale slot, so a runtime `set_locale` on any clone
    /// affects every clone's text.
    pub fn message(&self, key: &str, values: &[(&str, &str)]) -> String {
        self.messages().text(key, values)
    }

    /// The catalog for the host's current locale (a clone of the shared source).
    pub fn messages(&self) -> cad_ui_slint::MessageSource {
        cad_ui_slint::MessageSource::from_request(self.locale.borrow().as_str())
    }

    /// Switch the host's catalog locale at runtime.
    ///
    /// The slot is shared, so this reaches the text produced by every clone
    /// (the web host keeps clones in its sink, input sink and layout sink).
    pub fn set_locale(&self, tag: &str) {
        *self.locale.borrow_mut() = tag.to_string();
    }

    /// Set the host status line, recording it for host mirrors and tests.
    ///
    /// Delivered to the shell handle when one exists; the recorded copy lets a
    /// headless/test core (no window) observe the exact graded text.
    pub fn set_status(&self, text: String) -> CadResult<()> {
        *self.last_status.borrow_mut() = Some(text.clone());
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.set_status(text)?;
        }
        Ok(())
    }

    /// The last status text the core graded, if any.
    pub fn last_status(&self) -> Option<String> {
        self.last_status.borrow().clone()
    }

    /// Diagnostics from reading the host config / user preference at startup.
    pub fn startup_diagnostics(&self) -> Vec<String> {
        self.startup_diagnostics.borrow().clone()
    }

    /// Reclassify the window layout and map the surface onto the active viewport.
    ///
    /// The window layout is reclassified only when the physical size or the
    /// resolved config changed, not every tick (the 50 ms timer calls this). A
    /// host with no handle yet (a headless/test core) skips that presentation
    /// step; the surface mapping below is still applied.
    pub fn metrics(&self) -> CadResult<()> {
        if let Some(handle) = self.handle.borrow().as_ref() {
            let layout_key = handle
                .physical_size()
                .map(|size| (size.width, size.height, handle.config_revision()));
            let layout_changed = {
                let mut last = self.last_layout_key.borrow_mut();
                if *last == layout_key {
                    false
                } else {
                    *last = layout_key;
                    true
                }
            };
            if layout_changed {
                handle.refresh_window_layout()?;
            }
        }
        let surface = self.surface.surface().ok_or(CadError::Cancelled)?;
        let cad_surface = match self.handle.borrow().as_ref() {
            Some(handle) => Some(handle.cad_surface_size().ok_or(CadError::Cancelled)?),
            None => None,
        };
        let mut c = self.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .ok_or(CadError::Cancelled)?;
        let canvas = select_canvas_metrics(surface, cad_surface)?;
        cad_app::input::apply_canvas_metrics(viewport, &canvas)
    }

    /// Build and run one command through the single host funnel.
    pub fn command(&self, id: CommandId, payload: CommandPayload) -> CadResult<()> {
        let c = self.controller.borrow();
        let command = Command {
            schema_version: 1,
            id,
            payload,
            document: c.document_id,
            viewport: c.viewport_id,
        };
        drop(c);
        self.execute(command)
    }

    /// Run one command, push state and grade the status line.
    pub fn execute(&self, command: Command) -> CadResult<()> {
        let command_id = command.id;
        let command_viewport = command.viewport;
        let profile = command.id == CommandId::Select && pick_profile_enabled();
        let started = profile.then(std::time::Instant::now);
        let metrics_result = self.metrics();
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK execute_metrics elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        metrics_result?;
        // Plot and Save As own their own success/failure status
        // (plot.exported / save.exported / ...); the generic completed/failed
        // text must not overwrite it.
        let owns_status = policy::owns_status(command_id);
        let started = profile.then(std::time::Instant::now);
        let result: CadResult<Option<CommandOutcome>> = match command.id {
            CommandId::CancelLoading => {
                self.cancel_loading();
                Ok(None)
            }
            CommandId::OpenDrawing => self.request_open().map(|_| None),
            CommandId::NewDrawing => self.request_new().map(|_| None),
            CommandId::PlotDrawing => self.request_plot().map(|_| None),
            CommandId::SaveDrawingAs => self.request_save().map(|_| None),
            CommandId::SwitchBackend => Err(CadError::Unsupported(
                self.message("linux.backend_unavailable", &[]),
            )),
            _ => self.controller.borrow_mut().execute(command).map(Some),
        };
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK execute_controller elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        // The push is best-effort relative to the command result: a failed push
        // must never mask the command's own error (it is surfaced only when the
        // command itself succeeded).
        let (viewport_matches, has_drawing) = {
            let c = self.controller.borrow();
            (c.viewport_id == command_viewport, c.drawing().is_some())
        };
        let light_navigation_push = use_light_navigation_push(
            command_id,
            result.is_ok(),
            viewport_matches,
            has_drawing,
            self.handle.borrow().is_some(),
            self.view.borrow().is_some(),
            self.loading(),
        );
        let started = profile.then(std::time::Instant::now);
        let push_result = if light_navigation_push {
            self.push_navigation()
        } else {
            self.push()
        };
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK execute_push elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        let started = profile.then(std::time::Instant::now);
        // An open cancel/failure graded within this gesture owns the status; the
        // generic "completed" must not overwrite it.
        let open_graded = self.open_status_graded.replace(false);
        if let Err(error) = &result {
            if !owns_status {
                self.set_status(self.message("command.failed", &[("error", &error_text(error))]))?;
            }
        }
        if result.is_ok() && !self.loading() && !owns_status && !open_graded {
            // A no-op fit (empty drawing) carries the `fit.empty` code; show
            // that instead of the generic "completed", never a fake success.
            let diagnostic = result
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.as_ref())
                .and_then(|outcome| outcome.diagnostics.first());
            let status = match diagnostic {
                Some(diagnostic) if diagnostic.code == "fit.empty" => {
                    self.message("fit.empty", &[])
                }
                _ => self.message("command.completed", &[]),
            };
            self.set_status(status)?;
        }
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK execute_status elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        match result {
            Ok(_) => push_result,
            Err(error) => Err(error),
        }
    }

    /// Whether a save-target chooser is in flight or a chosen target is awaiting
    /// its export.
    ///
    /// A platform with an asynchronous save picker (Android SAF) needs its poll
    /// driver to keep ticking until the chosen target has been drained and
    /// exported; a synchronous platform clears this within the gesture.
    pub fn save_pending(&self) -> bool {
        self.save_pending.get() || self.pending_save.borrow().is_some()
    }

    /// Whether a document open is in flight (picker, read, import or presentation).
    pub fn loading(&self) -> bool {
        self.open_pending.get()
            || self.presenting_open.get()
            || self
                .controller
                .borrow()
                .async_open_snapshot()
                .is_some_and(|s| s.running)
    }

    /// Cancel an in-flight open/read (`CancelLoading`).
    ///
    /// Sets the abort flag so a platform read that completes *after* the cancel
    /// is discarded rather than installed, and asks the async import manager to
    /// cancel a job that has already started.
    pub(crate) fn cancel_loading(&self) {
        self.open_abort.set(true);
        self.controller.borrow_mut().cancel_async_open();
    }

    /// The import-drawer snapshot for the current open state.
    ///
    /// Prefers the controller's real async snapshot. While a platform read is in
    /// flight (picker/read, no async job yet) it synthesizes a running snapshot
    /// whose `cancellable` reflects the abort flag, so the drawer shows real
    /// progress with a working cancel — and goes idle once nothing is pending.
    pub fn import_snapshot(&self) -> Option<ImportProgressSnapshot> {
        if let Some(snapshot) = self.controller.borrow().async_open_snapshot() {
            return Some(snapshot);
        }
        if self.open_pending.get() {
            let mut running = ImportProgressSnapshot::running();
            running.cancellable = !self.open_abort.get();
            return Some(running);
        }
        None
    }

    /// Read and apply the durable config and user preference at startup.
    ///
    /// An absent file is normal; a malformed or unreadable one keeps the
    /// previous state and adds a human-readable diagnostic. Returns every
    /// diagnostic.
    pub fn load_startup(&self) -> Vec<String> {
        let mut diagnostics = Vec::new();
        let handle = match self.handle.borrow().as_ref().cloned() {
            Some(handle) => handle,
            None => return diagnostics,
        };

        if let Err(error) = handle.update_config_json(HOST_ALLOWED_PATHS_JSON) {
            diagnostics.push(format!("host config defaults rejected: {error}"));
        }

        match self.config_store.load_config() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => {
                if let Err(error) = handle.set_config_json(&text) {
                    diagnostics.push(format!("config file {path} rejected: {error}"));
                }
            }
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("config file {path} unreadable: {reason}"));
            }
        }

        match self.config_store.load_preference() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => match handle.apply_user_preference_json(&text) {
                Ok(()) => {
                    if let Err(error) = self.persist_preference(&handle) {
                        diagnostics.push(format!("preferences {path} not re-persisted: {error}"));
                    }
                }
                Err(error) => diagnostics.push(format!("preferences {path} rejected: {error}")),
            },
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("preferences {path} unreadable: {reason}"));
            }
        }

        *self.startup_diagnostics.borrow_mut() = diagnostics.clone();
        diagnostics
    }

    /// Write the store's allowed projection of the user preference.
    fn persist_preference(&self, handle: &UiHandle) -> CadResult<()> {
        self.config_store
            .persist_preference(&handle.projected_user_preference_json())
    }

    /// Merge a host `ViewerConfig` patch through the shared store.
    pub fn update_config_json(&self, text: &str) -> CadResult<()> {
        let handle = self
            .handle
            .borrow()
            .as_ref()
            .cloned()
            .ok_or(CadError::Cancelled)?;
        handle
            .update_config_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))
    }

    /// Apply a host-allowed user preference, then persist the allowed projection.
    ///
    /// The projection is written after the store accepts the patch (not from a
    /// `ConfigObserver`, which would re-borrow the store while it is mutably
    /// borrowed).
    pub fn apply_user_preference_json(&self, text: &str) -> CadResult<()> {
        let handle = self
            .handle
            .borrow()
            .as_ref()
            .cloned()
            .ok_or(CadError::Cancelled)?;
        handle
            .apply_user_preference_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.persist_preference(&handle)
    }

    /// One timer tick: file events → open/save funnels → metrics → view sync.
    ///
    /// The order is the desktop host's existing order and is load-bearing: the
    /// open funnel refreshes availability/status, and `sync_view` runs last.
    pub fn tick(&self) -> CadResult<()> {
        self.poll_open()
            .and_then(|_| self.poll_pending_save())
            .and_then(|_| self.metrics())
            .and_then(|_| self.sync_view())
    }

    /// Open the configured `--open` drawing at startup (or fit the demo).
    ///
    /// Runs before the CAD view exists, so the font/image installs are deferred
    /// to the caller once the bridge is installed.
    pub fn open_configured_drawing(&self) -> CadResult<()> {
        self.metrics()?;
        if let Some(path) = &self.options.drawing {
            let target = FileTarget::from_path(&path.to_string_lossy());
            self.begin_open_attempt();
            self.open_pending.set(true);
            if let Err(error) = self.files.begin_open_target(&target) {
                self.open_pending.set(false);
                return Err(error);
            }
            self.drain_files()?;
        } else {
            self.controller.borrow_mut().fit()?;
        }
        Ok(())
    }
}

impl UiCommandSink for HostCore {
    fn send(&mut self, command: Command) -> CadResult<()> {
        self.execute(command)
    }
}

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command.
///
/// Installed on the adapter so a layout-row click runs through the same command
/// path every other host command uses. The command layer validates the layout
/// against the drawing and refuses an unknown one; the sink never mutates the
/// database.
impl LayoutSwitchSink for HostCore {
    fn select(&mut self, space: cad_representation::SpaceSelection) {
        let _ = self.command(
            CommandId::SwitchSpace,
            cad_app::input::space_switch_payload(space),
        );
    }
}
