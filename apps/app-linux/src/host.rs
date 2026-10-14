//! Thin desktop wrapper over the platform-common [`HostCore`].
//!
//! All controller/host orchestration (the open/save/plot funnels, the command
//! routing table, the single synchronous state push) lives in `cad-host`; this
//! module supplies the desktop platform implementations (file chooser + read,
//! XDG/APPDATA config store, live window surface), creates the UI-free render
//! session the engine and Slint facade share, installs the one
//! [`EngineAdapter`] binding, and owns the event loop and the 50 ms timer.
use cad_domain::{CadError, CadResult};
use cad_engine::protocol::EngineClient;
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostOptions, HostPlatform};
use cad_platform::HostCapabilities;
use cad_render_wgpu::BackendPreference;
use cad_ui_slint::{CadView, EngineAdapter, UiAdapter, UiCommandSink, UiConfiguration, UiHandle};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

/// Schedules one pending pump cycle on the owner-thread event loop.
type WakeDriver = Rc<dyn Fn(Box<dyn FnOnce()>)>;

/// The desktop shell's pre-attach command slot.
///
/// The single engine dispatch is installed by [`EngineAdapter::attach`]
/// immediately after construction, so this slot is never reached in production:
/// the adapter routes every command through the engine once a client is
/// attached. It refuses explicitly instead of silently succeeding.
struct DetachedCommandSink;

impl UiCommandSink for DetachedCommandSink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Err(CadError::Unsupported(
            "the desktop engine dispatch is not attached".into(),
        ))
    }
}

/// Coalesces engine queue wake-ups into at most one scheduled pump cycle.
///
/// The engine is given only a `Weak`-backed, schedule-only waker: on the queue's
/// empty→nonempty edge it schedules one cycle; a further wake while a cycle is
/// already scheduled is coalesced (no timer storm); the engine re-arms it while
/// queued work remains. The scheduled cycle clears the pending flag *before* it
/// runs, so a wake that arrives while the cycle executes schedules the next one.
/// The waker never pumps synchronously and never keeps the app alive.
struct QueueWakeScheduler {
    /// `true` between a wake scheduling a cycle and that cycle starting.
    scheduled: Rc<Cell<bool>>,
    cycle: Rc<dyn Fn()>,
    driver: WakeDriver,
}

impl QueueWakeScheduler {
    fn new(cycle: Rc<dyn Fn()>, driver: WakeDriver) -> Rc<Self> {
        Rc::new(Self {
            scheduled: Rc::new(Cell::new(false)),
            cycle,
            driver,
        })
    }

    /// The schedule-only waker to register with the engine (`Weak`, so the
    /// engine never holds a strong reference back to the app).
    fn waker(self: &Rc<Self>) -> Rc<dyn Fn()> {
        let weak = Rc::downgrade(self);
        Rc::new(move || {
            if let Some(this) = weak.upgrade() {
                this.schedule();
            }
        })
    }

    /// Schedule one cycle unless one is already pending (coalesced).
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let scheduled = Rc::clone(&self.scheduled);
        let cycle = Rc::downgrade(&self.cycle);
        (self.driver)(Box::new(move || {
            scheduled.set(false);
            if let Some(cycle) = cycle.upgrade() {
                cycle();
            }
        }));
    }
}

/// Report a pump-cycle failure through the catalog and republish the status.
fn report_cycle_error(rt: &HostCore, binding: &EngineAdapter, error: &CadError) {
    if let Some(diagnostic) = timer_error_diagnostic(error) {
        let _ = rt.set_catalog_status("linux.failed", &[("error", &diagnostic)]);
        let _ = binding.flush_updates();
    }
}

pub mod config_file;
mod file_picker;
mod platform;
mod validation;

/// A chooser run on a worker; cancellation must return `CadError::Cancelled`.
pub type FilePicker = Arc<dyn Fn() -> CadResult<PathBuf> + Send + Sync>;
/// A save-path chooser; receives the suggested default file name and returns the
/// target path. Cancellation must return `CadError::Cancelled`.
pub type SavePathProvider = Arc<dyn Fn(&str) -> CadResult<PathBuf> + Send + Sync>;

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

pub struct LinuxApp {
    pub adapter: UiAdapter,
    runtime: HostCore,
    /// The single engine→UI binding: holds the client, the UI handle and the
    /// shared render session, and drives `refresh_metrics`/`flush_updates`.
    binding: Rc<EngineAdapter>,
    /// The Slint-facing compatibility facade over the shared render session.
    /// Always `Some` once construction returns; the slot mirrors the old
    /// core-owned `view` and is retained for acceptance/inspection.
    view: RefCell<Option<CadView>>,
    /// The single-shot queue waker scheduler registered with the engine.
    scheduler: Rc<QueueWakeScheduler>,
    timer: slint::Timer,
    /// Diagnostics from reading the config / preference files at startup.
    startup_diagnostics: Vec<String>,
}
impl LinuxApp {
    pub fn new(options: LinuxOptions) -> CadResult<Self> {
        let locale = options.locale.clone();
        let save_locale = options.locale.clone();
        Self::new_with_providers(
            options,
            Arc::new(move || file_picker::pick(&locale)),
            Arc::new(move |default_name: &str| file_picker::pick_save(&save_locale, default_name)),
        )
    }
    /// Construct with a host-supplied open chooser, including deterministic contract tests.
    pub fn new_with_file_picker(options: LinuxOptions, picker: FilePicker) -> CadResult<Self> {
        let locale = options.locale.clone();
        Self::new_with_providers(
            options,
            picker,
            Arc::new(move |default_name: &str| file_picker::pick_save(&locale, default_name)),
        )
    }
    /// Construct with host-supplied open **and** save choosers.
    ///
    /// The save chooser receives the suggested default file name and returns the
    /// target path; contract tests inject a temp path here so the export never
    /// opens a real dialog.
    pub fn new_with_providers(
        options: LinuxOptions,
        picker: FilePicker,
        saver: SavePathProvider,
    ) -> CadResult<Self> {
        let config_path = options
            .config
            .clone()
            .or_else(config_file::default_config_path);
        let preference_path = options
            .preferences
            .clone()
            .or_else(config_file::default_preference_path);
        // The render session is created UI-free: the engine owns it, the desktop
        // surface only reads the live window, and Slint installs its presenter
        // hooks around the same session.
        let render = RenderSessionHandle::new(BackendPreference::WebGpu);
        // The handle slot is shared between the desktop surface source and the
        // app, so `surface()` reads the live window without the engine owning a
        // UI handle.
        let handle_slot: Rc<RefCell<Option<UiHandle>>> = Rc::new(RefCell::new(None));
        let host_options = HostOptions {
            fonts: options.fonts.clone(),
            fonts_dir: options.fonts_dir.clone(),
            images_dir: options.images_dir.clone(),
            headless: options.headless,
            drawing: options.drawing.clone(),
            size: options.size,
        };
        let platform = HostPlatform {
            files: Rc::new(platform::DesktopFiles::new(&options, picker, saver)),
            config_store: Rc::new(platform::DesktopConfigStore::new(
                config_path,
                preference_path,
            )),
            surface: Rc::new(platform::DesktopSurface::new(handle_slot.clone())),
            resources: Rc::new(cad_host::DesktopResources),
            capabilities: HostCapabilities {
                open: true,
                new: true,
                save: true,
                plot: true,
                import_worker: !options.headless,
            },
        };
        let runtime = HostCore::new(host_options, platform, render.clone())?;
        let config = {
            let c = runtime.controller.borrow();
            UiConfiguration {
                document: c.document_id,
                viewport: c.viewport_id,
                logical_size: options.size,
                locale: options.locale.clone(),
                ..UiConfiguration::default()
            }
        };
        // The legacy slot is never reached once `EngineAdapter::attach` installs
        // the single engine dispatch below.
        let mut adapter = UiAdapter::new(config, DetachedCommandSink, true)?;
        adapter
            .component()
            .set_can_open(!options.headless || options.drawing.is_some());
        adapter.component().set_can_trim(false);
        adapter.component().set_can_switch_backend(false);
        adapter.fit_window_to_logical(options.size, 1.0);
        if !options.headless {
            adapter.window().set_maximized(true);
        }
        let handle = adapter.handle();
        *handle_slot.borrow_mut() = Some(handle.clone());
        // Read the host config and user preference before the first state push;
        // a malformed file is reported and never aborts startup.
        let startup_diagnostics = runtime.load_startup();
        for diagnostic in &startup_diagnostics {
            eprintln!("yacr-linux: {diagnostic}");
        }
        runtime.open_configured_drawing()?;
        // Install Slint's presenter hooks around the engine-owned session.
        let view = cad_ui_slint::install_existing_cad_bridge(
            handle.clone(),
            adapter.window(),
            render.clone(),
        )?;
        // Fonts: the sibling `fonts/` package, `--font` entries, then the system
        // default fallback. The resource strategy installs onto the shared
        // session, so the facade needs no separate view argument.
        runtime.install_fonts()?;
        runtime.install_images()?;
        // The single engine→UI binding; it installs the dispatch callbacks and is
        // the only state path (no legacy sink is registered, so there is no
        // double dispatch).
        let client: Rc<dyn EngineClient> = Rc::new(runtime.clone());
        let binding = Rc::new(EngineAdapter::attach(&mut adapter, client, view.clone())?);
        runtime.set_catalog_status(
            if options.drawing.is_some() && options.headless {
                "linux.opened"
            } else if options.drawing.is_some() {
                "linux.picker_waiting"
            } else {
                "status.synthetic"
            },
            &[],
        )?;
        // Surface the first startup problem on the status bar; the full list is
        // logged above and retained for tests.
        if let Some(diagnostic) = startup_diagnostics.first() {
            runtime.set_catalog_status("linux.failed", &[("error", diagnostic)])?;
        }
        // Publish the initial full/navigation/config/status state through the one
        // binding. An attached host no longer pushes through a UI handle.
        binding.flush_updates()?;
        // One owner-thread pump cycle (the apps fixed order): necessary I/O poll
        // → refresh metrics → `pump_requests` (which advances the queue and
        // drains completions) → UI `flush_updates` → redraw the surface only when
        // the engine actually has a dirty CAD frame. A completion that does not
        // dirty the session never wakes the window (no revision churn).
        let cycle: Rc<dyn Fn()> = {
            let rt = runtime.clone();
            let cycle_binding = binding.clone();
            let cycle_view = view.clone();
            Rc::new(move || {
                if let Err(error) = rt.tick() {
                    report_cycle_error(&rt, &cycle_binding, &error);
                }
                if let Err(error) = cycle_binding.refresh_metrics() {
                    report_cycle_error(&rt, &cycle_binding, &error);
                }
                if let Err(error) = cycle_binding.pump_requests() {
                    report_cycle_error(&rt, &cycle_binding, &error);
                }
                if let Err(error) = cycle_binding.flush_updates() {
                    report_cycle_error(&rt, &cycle_binding, &error);
                }
                if rt.render.cad_frame_dirty() {
                    cycle_view.request_redraw();
                }
            })
        };
        // The engine holds only a weak, schedule-only waker. The queue's
        // empty→nonempty edge schedules one coalesced cycle; queued work re-arms
        // it. The waker never pumps synchronously and never keeps the app alive.
        let scheduler = QueueWakeScheduler::new(
            cycle.clone(),
            Rc::new(|fire| {
                slint::Timer::single_shot(std::time::Duration::from_millis(0), fire);
            }),
        );
        binding.set_request_waker(Some(scheduler.waker()))?;
        // The retained 50 ms timer is the I/O poll (file/import completions and
        // window resize metrics), not the queue scheduler. Its pump is a no-op
        // when the queue is empty, so there is no permanent CAD busy-poll.
        let timer = slint::Timer::default();
        let cycle_timer = cycle.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || cycle_timer(),
        );
        Ok(Self {
            adapter,
            runtime,
            binding,
            view: RefCell::new(Some(view)),
            scheduler,
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
        self.runtime.update_config_json(text)
    }
    /// Apply a host-allowed user preference, then persist the allowed projection.
    ///
    /// Mirrors the web host's call site: the projection is written after the
    /// store accepts the patch (not from a `ConfigObserver`, which would re-borrow
    /// the store while it is mutably borrowed).
    pub fn apply_user_preference_json(&self, text: &str) -> CadResult<()> {
        self.runtime.apply_user_preference_json(text)
    }
    pub fn run(&self) -> CadResult<()> {
        let _ = &self.timer;
        // Retain the owned engine→UI binding and its queue-wake scheduler for the
        // process lifetime.
        let _ = &self.binding;
        let _ = &self.scheduler;
        self.adapter
            .component()
            .run()
            .map_err(|e| CadError::Unsupported(e.to_string()))
    }
}

fn timer_error_diagnostic(error: &CadError) -> Option<String> {
    match error {
        CadError::Cancelled => None,
        other => Some(other.to_string()),
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

#[cfg(test)]
mod tests {
    use slint::ComponentHandle;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// Install the process-global offscreen platform once per test binary.
    fn install_offscreen() {
        use std::sync::OnceLock;
        static PLATFORM: OnceLock<Result<(), String>> = OnceLock::new();
        let result = PLATFORM
            .get_or_init(|| cad_ui_slint::offscreen::install().map_err(|error| error.to_string()));
        if let Err(error) = result {
            panic!("offscreen platform install failed: {error}");
        }
    }

    /// Serialize the app-showing tests: the offscreen platform installs a
    /// process-global rendering notifier, so only one shown `LinuxApp` may exist
    /// at a time (the same convention as the other desktop test binaries).
    fn serial_app_guard() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn timer_ignores_cancellation_but_reports_real_errors() {
        assert_eq!(
            super::timer_error_diagnostic(&cad_domain::CadError::Cancelled),
            None,
            "teardown cancellation is not a user-facing failure"
        );
        let failure = cad_domain::CadError::InvalidInput("metrics failed".into());
        assert_eq!(
            super::timer_error_diagnostic(&failure),
            Some("InvalidInput(\"metrics failed\")".into()),
            "non-cancellation timer errors retain their diagnostic"
        );
    }

    fn world_per_px(app: &crate::LinuxApp) -> f64 {
        let c = app.runtime.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].world_per_px()
    }

    /// Synthetic wiring contract for the P2-B desktop path: the engine client
    /// and the Slint facade share exactly one owner-thread render session, the
    /// core owns no UI handle (the engine binding publishes status), and an
    /// unchanged poll neither re-dirties nor requests a redraw.
    #[test]
    fn desktop_engine_binding_shares_one_session_and_idle_poll_does_not_redraw() {
        let _guard = serial_app_guard();
        install_offscreen();
        let app = crate::LinuxApp::new(crate::LinuxOptions {
            headless: true,
            ..Default::default()
        })
        .unwrap();
        app.adapter.component().show().unwrap();
        let view = app
            .view
            .borrow()
            .clone()
            .expect("LinuxApp installs its CadView");

        // 1. Session identity: the engine and the facade must be the same
        //    owner-thread session (`EngineAdapter::attach` rejects a mismatch).
        assert!(
            app.runtime.render.same_session(&view.render_session()),
            "engine client and UI facade must share one render session"
        );

        // 2. The core owns no UI handle: the initial status reached the shell
        //    only through the engine binding's `flush_updates`, and its text is
        //    the localized resolution of the host's own last status.
        let expected =
            cad_ui_slint::MessageSource::from_request(&crate::LinuxOptions::default().locale)
                .text("status.synthetic", &[]);
        assert_eq!(
            app.adapter.component().get_status_label().to_string(),
            expected,
            "the engine binding must publish host status without a core-owned handle"
        );

        // 3. No idle redraw: drain the initial publication, then poll and flush
        //    with no state change; the facade must not request another frame.
        app.binding.flush_updates().unwrap();
        app.runtime.tick().unwrap();
        app.binding.refresh_metrics().unwrap();
        app.binding.pump_requests().unwrap();
        app.binding.flush_updates().unwrap();
        let baseline = view.redraw_requests();
        app.runtime.tick().unwrap();
        app.binding.refresh_metrics().unwrap();
        app.binding.pump_requests().unwrap();
        app.binding.flush_updates().unwrap();
        assert_eq!(
            view.redraw_requests(),
            baseline,
            "an unchanged poll must not request another redraw"
        );
    }

    /// The desktop wheel comes from Slint's winit backend, whose positive
    /// `delta_y` is a wheel-up rotation (winit documents positive as content
    /// moving down / revealing content above). Wheel-up must therefore zoom in
    /// (a smaller world-per-pixel), matching every other host's "up = closer".
    #[test]
    fn desktop_wheel_up_zooms_in() {
        let _guard = serial_app_guard();
        install_offscreen();
        let app = crate::LinuxApp::new(crate::LinuxOptions {
            headless: true,
            ..Default::default()
        })
        .unwrap();
        app.adapter.component().show().unwrap();
        let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
        let center = slint::LogicalPosition::new(
            (rect[0] + rect[2] * 0.5) as f32,
            (rect[1] + rect[3] * 0.5) as f32,
        );

        let before = world_per_px(&app);
        app.adapter
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: center,
                delta_x: 0.0,
                delta_y: 120.0, // wheel up
            });
        let after_up = world_per_px(&app);
        assert!(
            after_up < before,
            "wheel up must zoom in: {before} -> {after_up}"
        );

        app.adapter
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: center,
                delta_x: 0.0,
                delta_y: -120.0, // wheel down
            });
        let after_down = world_per_px(&app);
        assert!(
            after_down > after_up,
            "wheel down must zoom out: {after_up} -> {after_down}"
        );
    }

    /// Deterministic stand-in for the single-shot event-loop driver: records
    /// scheduled cycles and lets a test fire them synchronously (no real timer).
    #[derive(Default)]
    struct FakeDriver {
        scheduled: Cell<usize>,
        pending: RefCell<Vec<Box<dyn FnOnce()>>>,
    }

    impl FakeDriver {
        fn driver(self: &Rc<Self>) -> super::WakeDriver {
            let me = Rc::clone(self);
            Rc::new(move |fire| {
                me.scheduled.set(me.scheduled.get() + 1);
                me.pending.borrow_mut().push(fire);
            })
        }

        fn scheduled_count(&self) -> usize {
            self.scheduled.get()
        }

        fn fire_pending(&self) {
            for _ in 0..64 {
                let next = self.pending.borrow_mut().pop();
                match next {
                    Some(fire) => fire(),
                    None => return,
                }
            }
            panic!("fake driver kept rescheduling; cycles did not settle");
        }
    }

    /// The queue waker is coalesced: waking while a cycle is already scheduled
    /// does not schedule another; after the cycle runs, a wake schedules again.
    #[test]
    fn queue_wake_coalesces_pending_cycles_and_rearms_after_firing() {
        let cycles = Rc::new(Cell::new(0usize));
        let cycle: Rc<dyn Fn()> = {
            let cycles = cycles.clone();
            Rc::new(move || cycles.set(cycles.get() + 1))
        };
        let driver = Rc::new(FakeDriver::default());
        let scheduler = super::QueueWakeScheduler::new(cycle, driver.driver());
        let waker = scheduler.waker();

        waker();
        waker();
        assert_eq!(
            driver.scheduled_count(),
            1,
            "a wake while a cycle is pending must be coalesced"
        );
        driver.fire_pending();
        assert_eq!(cycles.get(), 1);

        waker();
        assert_eq!(
            driver.scheduled_count(),
            2,
            "after firing, a wake (the host re-arm) must schedule again"
        );
        driver.fire_pending();
        assert_eq!(cycles.get(), 2);
    }

    /// The engine's waker is weak: dropping the scheduler leaves an inert waker
    /// (no panic, no schedule) instead of a strong reference cycle.
    #[test]
    fn queue_wake_weak_handle_expires_and_waker_is_inert() {
        let driver = Rc::new(FakeDriver::default());
        let scheduler = super::QueueWakeScheduler::new(Rc::new(|| {}), driver.driver());
        let waker = scheduler.waker();
        drop(scheduler);
        waker();
        assert_eq!(
            driver.scheduled_count(),
            0,
            "an expired waker must be inert"
        );
    }

    /// End-to-end queue wire against the **real** engine `RequestQueue` (no UI,
    /// so it does not contend for the process-global Slint platform): the queue's
    /// empty→nonempty edge wakes exactly once, the fired cycle executes the
    /// request through the real queue state machine, a drained queue does not
    /// re-arm, and an empty/idle queue stages no wake. The window-level
    /// idle-no-redraw contract stays covered by the app test above.
    #[test]
    fn queue_submit_schedules_once_and_empty_queue_stages_no_wake() {
        use cad_app::interaction::{InteractionRequest, PointInput};
        use cad_domain::{DocumentId, ViewportId};
        use cad_engine::protocol::{
            EngineRequest, InteractionOutcome, RequestContext, RequestResult,
        };
        use cad_engine::queue::RequestQueue;

        let queue = Rc::new(RequestQueue::new());
        let driver = Rc::new(FakeDriver::default());
        let executed = Rc::new(Cell::new(0usize));
        let cycle: Rc<dyn Fn()> = {
            let queue = queue.clone();
            let executed = executed.clone();
            Rc::new(move || {
                if let Some(mut executing) = queue.begin() {
                    let _ = executing.take_request();
                    executing.complete(Ok(RequestResult::Interaction(
                        InteractionOutcome::CursorMoved,
                    )));
                    executed.set(executed.get() + 1);
                }
            })
        };
        let scheduler = super::QueueWakeScheduler::new(cycle, driver.driver());
        let waker = scheduler.waker();

        let context = RequestContext {
            document: DocumentId(0),
            document_generation: 0,
            viewport: ViewportId(0),
            tool_generation: None,
            metrics_revision: Some(0),
            config_revision: 0,
        };
        let request =
            EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
                logical_position: [12.0, 12.0],
                metrics_revision: 0,
            }));
        queue
            .submit(context, request)
            .expect("the real queue must accept an async MoveCursor");
        if queue.take_wake() {
            waker();
        }
        assert_eq!(
            driver.scheduled_count(),
            1,
            "empty→nonempty must schedule exactly one cycle"
        );

        driver.fire_pending();
        assert_eq!(
            executed.get(),
            1,
            "the fired cycle must execute the request"
        );
        assert!(
            !queue.pending(),
            "the queue must be drained after execution"
        );

        // The host re-arms only while queued work remains.
        if queue.rearm_wake() {
            waker();
        }
        assert_eq!(
            driver.scheduled_count(),
            1,
            "a drained queue must not re-arm another wake"
        );

        // Idle: an empty queue stages no wake at all (no idle window wake).
        assert!(!queue.pending());
        assert!(!queue.take_wake());
        assert!(!queue.rearm_wake());
        assert_eq!(driver.scheduled_count(), 1);
    }

    /// An empty real queue never stages a wake — the mechanism that keeps an idle
    /// desktop from waking or redrawing the window.
    #[test]
    fn idle_queue_stages_no_wake() {
        use cad_engine::queue::RequestQueue;

        let queue = Rc::new(RequestQueue::new());
        let driver = Rc::new(FakeDriver::default());
        let cycle: Rc<dyn Fn()> = Rc::new(|| {});
        let scheduler = super::QueueWakeScheduler::new(cycle, driver.driver());
        let waker = scheduler.waker();

        assert!(!queue.pending());
        if queue.take_wake() {
            waker();
        }
        if queue.rearm_wake() {
            waker();
        }
        assert_eq!(
            driver.scheduled_count(),
            0,
            "an idle queue must stage no wake"
        );
    }
}
