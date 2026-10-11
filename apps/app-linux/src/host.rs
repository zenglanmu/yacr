//! Thin desktop wrapper over the platform-common [`HostCore`].
//!
//! All controller/Slint orchestration (the open/save/plot funnels, the command
//! routing table, the state push, the navigation/draw sinks) lives in
//! `cad-host`; this module supplies the desktop platform implementations
//! (file chooser + read, XDG/APPDATA config store, window surface) and owns the
//! event loop and the 50 ms timer.
use cad_domain::{CadError, CadResult};
use cad_host::{HostCore, HostOptions, HostPlatform};
use cad_platform::HostCapabilities;
use cad_ui_slint::{UiAdapter, UiConfiguration};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

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
        // The handle slot is shared between the core and the desktop surface
        // source, so `surface()` reads the live window without the core having
        // to hand its handle out.
        let handle_slot: Rc<RefCell<Option<cad_ui_slint::UiHandle>>> = Rc::new(RefCell::new(None));
        let host_options = HostOptions {
            fonts: options.fonts.clone(),
            fonts_dir: options.fonts_dir.clone(),
            images_dir: options.images_dir.clone(),
            headless: options.headless,
            drawing: options.drawing.clone(),
            size: options.size,
            locale: options.locale.clone(),
        };
        let platform = HostPlatform {
            files: Rc::new(platform::DesktopFiles::new(&options, picker, saver)),
            config_store: Rc::new(platform::DesktopConfigStore::new(
                config_path,
                preference_path,
            )),
            surface: Rc::new(platform::DesktopSurface::new(handle_slot.clone())),
            capabilities: HostCapabilities {
                open: true,
                new: true,
                save: true,
                plot: true,
                import_worker: !options.headless,
            },
        };
        let runtime = HostCore::new(host_options, handle_slot, platform)?;
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
        runtime.set_handle(adapter.handle());
        // Read the host config and user preference before the first state push;
        // a malformed file is reported and never aborts startup.
        let startup_diagnostics = runtime.load_startup();
        for diagnostic in &startup_diagnostics {
            eprintln!("yacr-linux: {diagnostic}");
        }
        runtime.open_configured_drawing()?;
        let incoming = Rc::new(RefCell::new(runtime.controller.borrow().drawing()));
        let view = cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming)?;
        // Fonts: the sibling `fonts/` package, `--font` entries, then the system
        // default fallback. A drawing font that is missing shapes with the
        // default instead of being dropped.
        runtime.install_fonts(&view)?;
        runtime.install_images(&view)?;
        runtime.set_view(view);
        // A status-bar overlay toggle changes the shared config store but does not
        // run the host `push()` funnel, so without mirroring the new visibility
        // onto the view the toggle would update only the button (grid "enabled"
        // but never drawn). Mirror it on every real config change. The observer
        // receives the config directly, so it never re-borrows the store (which
        // is mutably borrowed during `publish`).
        {
            let view_slot = runtime.view.clone();
            adapter
                .handle()
                .on_config_changed(Rc::new(move |config, _revision| {
                    if let Some(view) = view_slot.borrow().as_ref() {
                        view.set_overlay_visibility(config.view.overlays.clone().into());
                    }
                }))
                .map_err(|error| CadError::Invariant(error.to_string()))?;
        }
        adapter.set_view_input(Rc::new(runtime.clone()));
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
        // Retain the timer: real desktop resize and drawer changes update content
        // metrics without refitting.
        let timer = slint::Timer::default();
        let rt = runtime.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                if let Err(error) = rt.tick() {
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

#[cfg(test)]
mod tests {
    use slint::ComponentHandle;

    fn world_per_px(app: &crate::LinuxApp) -> f64 {
        let c = app.runtime.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].world_per_px()
    }

    /// The desktop wheel comes from Slint's winit backend, whose positive
    /// `delta_y` is a wheel-up rotation (winit documents positive as content
    /// moving down / revealing content above). Wheel-up must therefore zoom in
    /// (a smaller world-per-pixel), matching every other host's "up = closer".
    #[test]
    fn desktop_wheel_up_zooms_in() {
        cad_ui_slint::offscreen::install().unwrap();
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
}
