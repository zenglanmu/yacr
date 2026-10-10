//! One application/controller/Slint bridge for both desktop and headless acceptance.
use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, Point3};
use cad_ui_slint::{CadView, UiAdapter, UiCommandSink, UiConfiguration, UiHandle};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

pub mod config_file;
mod file_picker;
mod fonts;
mod input;
mod state;
mod validation;

/// A chooser run on a worker; cancellation must return `CadError::Cancelled`.
pub type FilePicker = Arc<dyn Fn() -> CadResult<PathBuf> + Send + Sync>;
type PendingOpen = Rc<RefCell<Option<std::sync::mpsc::Receiver<CadResult<PathBuf>>>>>;

struct PendingRead {
    receiver: std::sync::mpsc::Receiver<CadResult<Arc<[u8]>>>,
    label: String,
    drawing: Arc<cad_db::DrawingDatabase>,
    cancelled: bool,
}

#[derive(Debug, Clone)]
pub struct LinuxOptions {
    pub fonts: Vec<(String, PathBuf)>,
    /// Explicit CAD font directory (`--fonts-dir`); `None` auto-detects a
    /// `fonts/` directory next to the executable (or `bin/`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`); `None` resolves image
    /// keys against the drawing's own directory only.
    pub images_dir: Option<PathBuf>,
    pub headless: bool,
    pub output: Option<PathBuf>,
    pub drawing: Option<PathBuf>,
    /// Full host `ViewerConfig` file; `None` uses the XDG default.
    pub config: Option<PathBuf>,
    /// Projected user-preference file; `None` uses the XDG default.
    pub preferences: Option<PathBuf>,
    pub size: [f64; 2],
    pub locale: String,
    /// Adapter preference for the desktop wgpu backend (`auto`/`high`/`low`).
    pub gpu: cad_render_wgpu::GpuSelection,
}
impl Default for LinuxOptions {
    fn default() -> Self {
        Self {
            fonts: Vec::new(),
            fonts_dir: None,
            images_dir: None,
            headless: false,
            output: None,
            drawing: None,
            config: None,
            preferences: None,
            size: [1280.0, 800.0],
            locale: "zh-CN".into(),
            gpu: cad_render_wgpu::GpuSelection::Auto,
        }
    }
}
impl LinuxOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--headless" {
                options.headless = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {arg}"))?;
            match arg.as_str() {
                "--font" => {
                    let (name, path) = value.split_once('=').ok_or("font must be NAME=PATH")?;
                    if name.is_empty() || path.is_empty() {
                        return Err("font name/path must not be empty".into());
                    }
                    options.fonts.push((name.into(), path.into()));
                }
                "--output" => options.output = Some(value.into()),
                "--open" => options.drawing = Some(value.into()),
                "--fonts-dir" => options.fonts_dir = Some(value.into()),
                "--images-dir" => options.images_dir = Some(value.into()),
                "--config" => options.config = Some(value.into()),
                "--preferences" => options.preferences = Some(value.into()),
                "--locale" if matches!(value.as_str(), "zh-CN" | "en") => options.locale = value,
                "--gpu" if cad_render_wgpu::GpuSelection::parse(&value).is_some() => {
                    options.gpu = cad_render_wgpu::GpuSelection::parse(&value).unwrap();
                }
                "--size" => {
                    let (w, h) = value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
                    options.size = [
                        w.parse().map_err(|_| "invalid width")?,
                        h.parse().map_err(|_| "invalid height")?,
                    ];
                    if options
                        .size
                        .iter()
                        .any(|v| !v.is_finite() || *v < 320.0 || *v > 4096.0)
                    {
                        return Err("size must be finite and between 320 and 4096".into());
                    }
                }
                _ => return Err(format!("unknown option or invalid value: {arg}")),
            }
        }
        if options.headless && options.output.is_none() {
            return Err("--headless requires --output NEW_DIRECTORY".into());
        }
        Ok(options)
    }
}

#[derive(Clone)]
struct Runtime {
    controller: Rc<RefCell<HostController>>,
    handle: Rc<RefCell<Option<UiHandle>>>,
    view: Rc<RefCell<Option<CadView>>>,
    options: LinuxOptions,
    /// Resolved user-preference path (`--preferences` or the XDG default).
    preference_path: Option<PathBuf>,
    picker: FilePicker,
    pending_open: PendingOpen,
    pending_read: Rc<RefCell<Option<PendingRead>>>,
    /// Path of the drawing being opened (or already open), used to resolve its
    /// sibling raster images. `None` until a document is opened.
    drawing_path: Rc<RefCell<Option<PathBuf>>>,
    /// Last raster-image loading outcome, surfaced by the diagnostics drawer.
    last_image_report: Rc<RefCell<Option<cad_platform::images::ImageLoadReport>>>,
    presenting_open: Rc<std::cell::Cell<bool>>,
    /// Last `(physical width, physical height, config revision)` that was pushed
    /// through `refresh_window_layout`. `None` forces the first refresh.
    ///
    /// The 50 ms timer calls `metrics` every tick; re-applying presentation and
    /// re-serialising the effective config each tick is pure waste when neither
    /// the window nor the config changed.
    last_layout_key: Rc<RefCell<Option<(u32, u32, u64)>>>,
}
impl Runtime {
    fn message(&self, key: &str, values: &[(&str, &str)]) -> String {
        cad_ui_slint::MessageSource::from_request(&self.options.locale).text(key, values)
    }
    fn metrics(&self) -> CadResult<()> {
        let handle = self.handle.borrow();
        let handle = handle.as_ref().ok_or(CadError::Cancelled)?;
        // Reclassify the window layout only when the physical size or the
        // resolved config changed, not every 50 ms tick.
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
        let (size, scale) = handle.cad_surface_size().ok_or(CadError::Cancelled)?;
        let mut c = self.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .ok_or(CadError::Cancelled)?;
        // Linux has no portable safe-area source (no notch/system-bar API on the
        // desktop window), so the insets are explicit zeros — never a fabricated
        // margin. The shared helper still validates the surface/DPI and keeps the
        // one mapping in `cad-app` (docs/input.md §5, docs/responsive-ui.md §6).
        let canvas = cad_app::input::inset_canvas_metrics([0.0; 2], size, [0.0; 4], scale)
            .ok_or_else(|| CadError::InvalidInput("canvas metrics are degenerate".into()))?;
        cad_app::input::apply_canvas_metrics(viewport, &canvas)
    }
    fn command(&self, id: CommandId, payload: CommandPayload) -> CadResult<()> {
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
    fn execute(&self, command: Command) -> CadResult<()> {
        self.metrics()?;
        let result = match command.id {
            CommandId::CancelLoading => {
                if let Some(read) = self.pending_read.borrow_mut().as_mut() {
                    read.cancelled = true;
                }
                self.controller.borrow_mut().cancel_async_open();
                Ok(())
            }
            CommandId::OpenDrawing => self.request_open(),
            CommandId::SwitchBackend => Err(CadError::Unsupported(
                self.message("linux.backend_unavailable", &[]),
            )),
            _ => self.controller.borrow_mut().execute(command).map(|_| ()),
        };
        self.push()?;
        if let Err(error) = &result {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle
                    .set_status(self.message("linux.failed", &[("error", &error.to_string())]))?;
            }
        }
        if result.is_ok() && !self.loading() {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.set_status(self.message("linux.completed", &[]))?;
            }
        }
        result
    }
    fn request_open(&self) -> CadResult<()> {
        if self.options.headless {
            return self.open_path(
                self.options
                    .drawing
                    .as_ref()
                    .ok_or_else(|| CadError::Unsupported(self.message("linux.open_path", &[])))?,
            );
        }
        if self.loading() {
            return Ok(());
        }
        let picker = self.picker.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("linux-file-picker".into())
            .spawn(move || {
                let _ = sender.send(picker());
            })
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        *self.pending_open.borrow_mut() = Some(receiver);
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.set_status(self.message("linux.picker_waiting", &[]))?;
        }
        Ok(())
    }
    fn poll_open(&self) -> CadResult<()> {
        use std::sync::mpsc::TryRecvError;
        self.poll_loading()?;
        let result = match self.pending_open.borrow().as_ref().map(|rx| rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return Ok(()),
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => Err(CadError::InvalidInput(
                self.message("linux.picker_empty", &[]),
            )),
        };
        *self.pending_open.borrow_mut() = None;
        let result = result.and_then(|path| self.open_path(&path));
        self.push()?;
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.set_status(match result {
                Ok(()) => self.message("linux.picker_waiting", &[]),
                Err(CadError::Cancelled) => self.message("linux.picker_cancelled", &[]),
                Err(error) => self.message("linux.failed", &[("error", &error.to_string())]),
            })?;
        }
        Ok(())
    }
    fn open_path(&self, path: &Path) -> CadResult<()> {
        // A missing path is refused up front rather than falling back to the
        // synthetic demo (whether the read happens here or on the worker).
        if !path.exists() {
            return Err(CadError::InvalidInput(format!(
                "drawing path does not exist: {}",
                path.display()
            )));
        }
        // Remember the drawing's directory so its sibling raster images can be
        // resolved once the document finishes opening.
        *self.drawing_path.borrow_mut() = Some(path.to_path_buf());
        if !self.options.headless {
            if self.pending_read.borrow().is_some()
                || self
                    .controller
                    .borrow()
                    .async_open_snapshot()
                    .is_some_and(|s| s.running)
            {
                return Err(CadError::Cancelled);
            }
            let path = path.to_owned();
            let label = path.to_string_lossy().into_owned();
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("linux-drawing-read".into())
                .spawn(move || {
                    let result = std::fs::read(path)
                        .map(Arc::from)
                        .map_err(|e| CadError::InvalidInput(e.to_string()));
                    let _ = sender.send(result);
                })
                .map_err(|e| CadError::InvalidInput(e.to_string()))?;
            let c = self.controller.borrow();
            *self.pending_read.borrow_mut() = Some(PendingRead {
                receiver,
                label,
                drawing: c.drawing().ok_or(CadError::Cancelled)?,
                cancelled: false,
            });
            self.presenting_open.set(true);
            return Ok(());
        }
        let bytes = std::fs::read(path).map_err(|e| CadError::InvalidInput(e.to_string()))?;
        self.controller
            .borrow_mut()
            .open_bytes(Arc::from(bytes), &path.to_string_lossy())?;
        self.controller.borrow_mut().fit()?;
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.cancel_draw_capture()?;
        }
        if let Some(view) = self.view.borrow().as_ref() {
            self.install_fonts(view)?;
            self.install_images(view)?;
        }
        Ok(())
    }

    /// (Re)load shaping fonts for the current drawing.
    ///
    /// Reads the sibling `fonts/` package (or `--fonts-dir`), merges explicit
    /// `--font` entries and installs a best-effort system default face as the
    /// first fallback. Called at startup and after a document finishes opening,
    /// so a drawing opened later still gets its fonts.
    fn install_fonts(&self, view: &CadView) -> CadResult<()> {
        let requested = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
            .unwrap_or_default();
        let exe = std::env::current_exe().unwrap_or_default();
        let font_dir = fonts::resolve_font_dir(self.options.fonts_dir.as_deref(), &exe)?;
        let system_default = fonts::cached_system_default_candidates();
        match fonts::load_desktop_fonts(
            font_dir.as_deref(),
            &requested,
            &self.options.fonts,
            system_default,
        )? {
            Some(engine) => view.set_fonts(engine),
            None => view.clear_fonts(),
        }
        Ok(())
    }

    /// (Re)load the drawing's raster images for the current document.
    ///
    /// Resolves each logical key under the drawing's own directory (then an
    /// explicit `--images-dir`), decodes it through the shared platform loader
    /// and installs the cache on the view. An empty request clears the cache.
    /// Unresolved/undecodable keys are retained in `last_image_report` and
    /// surfaced by the diagnostics drawer, never silently dropped.
    fn install_images(&self, view: &CadView) -> CadResult<()> {
        let requested = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            view.clear_images();
            *self.last_image_report.borrow_mut() = None;
            return Ok(());
        }
        let drawing_dir = self
            .drawing_path
            .borrow()
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let (cache, report) = cad_platform::images::local::load_image_cache(
            &requested,
            drawing_dir.as_deref(),
            self.options.images_dir.as_deref(),
            &cad_resources::ResourceLimits::default(),
        )?;
        if cache.is_empty() {
            view.clear_images();
        } else {
            view.set_images(cache);
        }
        *self.last_image_report.borrow_mut() = Some(report);
        Ok(())
    }

    fn loading(&self) -> bool {
        self.pending_open.borrow().is_some()
            || self.pending_read.borrow().is_some()
            || self.presenting_open.get()
            || self
                .controller
                .borrow()
                .async_open_snapshot()
                .is_some_and(|s| s.running)
    }

    fn poll_loading(&self) -> CadResult<()> {
        use std::sync::mpsc::TryRecvError;
        let bytes = self
            .pending_read
            .borrow()
            .as_ref()
            .map(|read| read.receiver.try_recv());
        if let Some(result) = bytes {
            if !matches!(result, Err(TryRecvError::Empty)) {
                let read = self.pending_read.borrow_mut().take().unwrap();
                let mut c = self.controller.borrow_mut();
                let unchanged = !read.cancelled
                    && c.drawing()
                        .is_some_and(|db| Arc::ptr_eq(&db, &read.drawing));
                if unchanged {
                    let bytes = match result {
                        Ok(Ok(bytes)) => bytes,
                        other => {
                            self.presenting_open.set(false);
                            return Err(match other {
                                Ok(Err(error)) => error,
                                Err(error) => CadError::InvalidInput(error.to_string()),
                                _ => unreachable!(),
                            });
                        }
                    };
                    c.begin_async_open(bytes, &read.label);
                } else {
                    self.presenting_open.set(false);
                }
            }
        }
        let poll = self.controller.borrow_mut().poll_async_open();
        let opened = matches!(poll, cad_app::AsyncOpenPoll::Opened { .. });
        if matches!(
            poll,
            cad_app::AsyncOpenPoll::Failed { .. } | cad_app::AsyncOpenPoll::Cancelled { .. }
        ) {
            self.presenting_open.set(false);
        }
        if opened {
            self.controller.borrow_mut().fit()?;
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.cancel_draw_capture()?;
            }
            if let Some(view) = self.view.borrow().as_ref() {
                self.install_fonts(view)?;
                self.install_images(view)?;
            }
            self.push()?;
        }
        let c = self.controller.borrow();
        let snapshot = if let Some(read) = self.pending_read.borrow().as_ref() {
            let mut snapshot = cad_app::ImportProgressSnapshot::running();
            snapshot.cancellable = !read.cancelled;
            Some(snapshot)
        } else {
            c.async_open_snapshot()
        };
        if let Some(handle) = self.handle.borrow().as_ref() {
            let messages = cad_ui_slint::MessageSource::from_request(&self.options.locale);
            handle.set_open_available(!self.loading())?;
            handle.set_import_state(&cad_ui_slint::ImportProgressUiState::from_snapshot(
                snapshot.as_ref(),
                &messages,
            ))?;
            if let cad_app::AsyncOpenPoll::Failed { error, .. } = poll {
                handle
                    .set_status(self.message("linux.failed", &[("error", &error.to_string())]))?;
            } else if opened {
                handle.set_status(self.message("linux.opened", &[]))?;
            }
            if self.presenting_open.get() && !snapshot.as_ref().is_some_and(|s| s.running) {
                if let Some(view) = self.view.borrow().as_ref() {
                    if let Some(error) = view.scene_error() {
                        handle.set_status(self.message("linux.failed", &[("error", &error)]))?;
                        self.presenting_open.set(false);
                    } else if view.current_drawing_presented() {
                        self.presenting_open.set(false);
                        handle.set_status(self.message("linux.opened", &[]))?;
                    } else {
                        handle.set_status(match view.loading_phase() {
                            Some("uploading") => self.message("linux.uploading", &[]),
                            Some("drawing") => {
                                let (done, total) = view.drawing_progress().unwrap_or((0, 0));
                                self.message(
                                    "linux.drawing",
                                    &[("done", &done.to_string()), ("total", &total.to_string())],
                                )
                            }
                            _ => self.message("linux.preparing", &[]),
                        })?;
                    }
                }
            }
        }
        Ok(())
    }
}
impl UiCommandSink for Runtime {
    fn send(&mut self, command: Command) -> CadResult<()> {
        self.execute(command)
    }
}

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command.
///
/// Installed on the adapter so a layout-row click runs through `Runtime::command`
/// → `execute` (`HostController::execute` + state push) — the same path every
/// other host command uses. The command layer validates the layout against the
/// drawing and refuses an unknown one; the sink never mutates the database
/// (`docs/layouts.md` §4).
impl cad_ui_slint::LayoutSwitchSink for Runtime {
    fn select(&mut self, space: cad_representation::SpaceSelection) {
        let _ = self.command(
            CommandId::SwitchSpace,
            cad_app::input::space_switch_payload(space),
        );
    }
}

pub struct LinuxApp {
    pub adapter: UiAdapter,
    runtime: Runtime,
    timer: slint::Timer,
    /// Diagnostics from reading the config / preference files at startup.
    startup_diagnostics: Vec<String>,
}
impl LinuxApp {
    pub fn new(options: LinuxOptions) -> CadResult<Self> {
        let locale = options.locale.clone();
        Self::new_with_file_picker(options, Arc::new(move || file_picker::pick(&locale)))
    }
    /// Construct with a host-supplied chooser, including deterministic contract tests.
    pub fn new_with_file_picker(options: LinuxOptions, picker: FilePicker) -> CadResult<Self> {
        let config_path = options
            .config
            .clone()
            .or_else(config_file::default_config_path);
        let preference_path = options
            .preferences
            .clone()
            .or_else(config_file::default_preference_path);
        let controller = Rc::new(RefCell::new(HostController::with_demo_document(
            options.size,
        )?));
        let runtime = Runtime {
            controller: controller.clone(),
            handle: Rc::new(RefCell::new(None)),
            view: Rc::new(RefCell::new(None)),
            options: options.clone(),
            preference_path: preference_path.clone(),
            picker,
            pending_open: Rc::new(RefCell::new(None)),
            pending_read: Rc::new(RefCell::new(None)),
            drawing_path: Rc::new(RefCell::new(None)),
            last_image_report: Rc::new(RefCell::new(None)),
            presenting_open: Rc::new(std::cell::Cell::new(false)),
            last_layout_key: Rc::new(RefCell::new(None)),
        };
        let c = controller.borrow();
        let config = UiConfiguration {
            document: c.document_id,
            viewport: c.viewport_id,
            logical_size: options.size,
            locale: options.locale.clone(),
            ..UiConfiguration::default()
        };
        drop(c);
        let mut adapter = UiAdapter::new(config, runtime.clone(), true)?;
        // The same command funnel serves as the layout-switch sink: a layout click
        // dispatches the validated `SwitchSpace` command (docs/layouts.md §4).
        adapter.set_layout_switch_sink(Box::new(runtime.clone()));
        adapter
            .component()
            .set_can_open(!options.headless || options.drawing.is_some());
        adapter.component().set_can_trim(false);
        adapter.component().set_can_switch_backend(false);
        adapter.fit_window_to_logical(options.size, 1.0);
        *runtime.handle.borrow_mut() = Some(adapter.handle());
        // Read the host config and user preference before the first state push;
        // a malformed file is reported and never aborts startup.
        let startup_diagnostics = {
            let handles = runtime.handle.borrow();
            match handles.as_ref() {
                Some(handle) => config_file::load_startup(
                    handle,
                    config_path.as_deref(),
                    preference_path.as_deref(),
                ),
                None => Vec::new(),
            }
        };
        for diagnostic in &startup_diagnostics {
            eprintln!("yacr-linux: {diagnostic}");
        }
        runtime.metrics()?;
        if let Some(path) = &options.drawing {
            runtime.open_path(path)?;
        } else {
            controller.borrow_mut().fit()?;
        }
        let incoming = Rc::new(RefCell::new(controller.borrow().drawing()));
        let view = cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming)?;
        // Fonts: the sibling `fonts/` package, `--font` entries, then the system
        // default fallback. A drawing font that is missing shapes with the
        // default instead of being dropped.
        runtime.install_fonts(&view)?;
        runtime.install_images(&view)?;
        *runtime.view.borrow_mut() = Some(view);
        // A status-bar overlay toggle changes the shared config store but does not
        // run the host `push()` funnel, so without mirroring the new visibility
        // onto the view the toggle would update only the button (grid "enabled"
        // but never drawn). Mirror it on every real config change. The observer
        // receives the config directly, so it never re-borrows the store (which
        // is mutably borrowed during `publish`).
        {
            let view_slot = runtime.view.clone();
            if let Some(handle) = runtime.handle.borrow().as_ref() {
                handle
                    .on_config_changed(Rc::new(move |config, _revision| {
                        if let Some(view) = view_slot.borrow().as_ref() {
                            view.set_overlay_visibility(config.view.overlays.clone().into());
                        }
                    }))
                    .map_err(|error| CadError::Invariant(error.to_string()))?;
            }
        }
        adapter.set_view_input(Rc::new(input::Navigation::new(runtime.clone())));
        adapter.set_canvas_pick_mapper(Rc::new(runtime.clone()));
        adapter.set_draw_command_sink(Box::new(runtime.clone()));
        adapter.set_draw_preview_sink(Rc::new(runtime.clone()));
        runtime.push()?;
        adapter.handle().set_status(runtime.message(
            if options.drawing.is_some() && options.headless {
                "linux.opened"
            } else if options.drawing.is_some() {
                "linux.picker_waiting"
            } else {
                "status.synthetic"
            },
            &[],
        ))?;
        // Surface the first startup problem on the status bar; the full list is
        // logged above and retained for tests.
        if let Some(diagnostic) = startup_diagnostics.first() {
            adapter
                .handle()
                .set_status(runtime.message("linux.failed", &[("error", diagnostic)]))?;
        }
        // Retain the timer: real desktop resize and drawer changes update content metrics without refitting.
        let timer = slint::Timer::default();
        let rt = runtime.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                if let Err(error) = rt
                    .poll_open()
                    .and_then(|_| rt.metrics())
                    .and_then(|_| rt.sync_view())
                {
                    if let Some(handle) = rt.handle.borrow().as_ref() {
                        let _ = handle.set_status(
                            rt.message("linux.failed", &[("error", &error.to_string())]),
                        );
                    }
                }
            },
        );
        Ok(Self {
            adapter,
            runtime,
            timer,
            startup_diagnostics,
        })
    }
    /// Diagnostics from reading the host config / user preference at startup.
    ///
    /// Empty when both files were absent or applied cleanly. Retained so a host
    /// (and its contract tests) can report exactly what was rejected.
    pub fn startup_diagnostics(&self) -> &[String] {
        &self.startup_diagnostics
    }
    /// Merge a host `ViewerConfig` patch through the shared store.
    pub fn update_config_json(&self, text: &str) -> CadResult<()> {
        let handle = self
            .runtime
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
    /// Mirrors the web host's call site: the projection is written after the
    /// store accepts the patch (not from a `ConfigObserver`, which would re-borrow
    /// the store while it is mutably borrowed).
    pub fn apply_user_preference_json(&self, text: &str) -> CadResult<()> {
        let handle = self
            .runtime
            .handle
            .borrow()
            .as_ref()
            .cloned()
            .ok_or(CadError::Cancelled)?;
        handle
            .apply_user_preference_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        match &self.runtime.preference_path {
            Some(path) => config_file::persist_preference(&handle, path),
            // No durable location was resolvable; the patch is still applied
            // in memory and the caller can report the missing path.
            None => Ok(()),
        }
    }
    pub fn run(&self) -> CadResult<()> {
        let _ = &self.timer;
        self.adapter
            .component()
            .run()
            .map_err(|e| CadError::Unsupported(e.to_string()))
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> CadResult<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("yacr-{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| CadError::InvalidInput(e.to_string()))?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| CadError::InvalidInput(e.to_string()))
}
