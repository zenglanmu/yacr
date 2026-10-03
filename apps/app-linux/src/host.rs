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

mod input;
mod state;
mod validation;

#[derive(Debug, Clone)]
pub struct LinuxOptions {
    pub headless: bool,
    pub output: Option<PathBuf>,
    pub drawing: Option<PathBuf>,
    pub annotation_export: Option<PathBuf>,
    pub annotation_import: Option<PathBuf>,
    pub size: [f64; 2],
    pub locale: String,
}
impl Default for LinuxOptions {
    fn default() -> Self {
        Self {
            headless: false,
            output: None,
            drawing: None,
            annotation_export: None,
            annotation_import: None,
            size: [1280.0, 800.0],
            locale: "zh-CN".into(),
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
                "--output" => options.output = Some(value.into()),
                "--open" => options.drawing = Some(value.into()),
                "--export-annotations" => options.annotation_export = Some(value.into()),
                "--import-annotations" => options.annotation_import = Some(value.into()),
                "--locale" if matches!(value.as_str(), "zh-CN" | "en") => options.locale = value,
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
}
impl Runtime {
    fn message(&self, key: &str, values: &[(&str, &str)]) -> String {
        cad_ui_slint::MessageSource::from_request(&self.options.locale).text(key, values)
    }
    fn metrics(&self) -> CadResult<()> {
        let handle = self.handle.borrow();
        let handle = handle.as_ref().ok_or(CadError::Cancelled)?;
        handle.refresh_window_layout()?;
        let (size, scale) = handle.cad_surface_size().ok_or(CadError::Cancelled)?;
        let mut c = self.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .ok_or(CadError::Cancelled)?;
        cad_app::input::apply_canvas_metrics(
            viewport,
            &cad_app::input::CanvasMetrics::new([0.0; 2], size, scale),
        )
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
            CommandId::OpenDrawing => self.open(),
            CommandId::ExportAnnotations => self.export_annotations(),
            CommandId::ImportAnnotations => self.import_annotations(),
            _ => self.controller.borrow_mut().execute(command).map(|_| ()),
        };
        self.push()?;
        if let Err(error) = &result {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle
                    .set_status(self.message("linux.failed", &[("error", &error.to_string())]))?;
            }
        }
        if result.is_ok() {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.set_status(self.message("linux.completed", &[]))?;
            }
        }
        result
    }
    fn open(&self) -> CadResult<()> {
        let path = self
            .options
            .drawing
            .as_ref()
            .ok_or_else(|| CadError::Unsupported(self.message("linux.open_path", &[])))?;
        // No implicit discard and no fake file dialog; dirty work requires a future explicit decision UI.
        if self.controller.borrow().unsaved_signal().dirty {
            return Err(CadError::Unsupported(self.message("linux.dirty", &[])));
        }
        let bytes = std::fs::read(path).map_err(|e| CadError::InvalidInput(e.to_string()))?;
        self.controller
            .borrow_mut()
            .open_bytes(Arc::from(bytes), &path.to_string_lossy())?;
        self.controller.borrow_mut().fit()?;
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.cancel_draw_capture()?;
        }
        Ok(())
    }
    fn export_annotations(&self) -> CadResult<()> {
        let path = self
            .options
            .annotation_export
            .as_ref()
            .ok_or_else(|| CadError::Unsupported(self.message("linux.export_path", &[])))?;
        let (json, revision) = self.controller.borrow().prepare_annotation_export()?;
        atomic_write(path, json.as_bytes())?;
        self.controller
            .borrow_mut()
            .confirm_annotation_export(revision)
    }
    fn import_annotations(&self) -> CadResult<()> {
        let path = self
            .options
            .annotation_import
            .as_ref()
            .ok_or_else(|| CadError::Unsupported(self.message("linux.import_path", &[])))?;
        let text =
            std::fs::read_to_string(path).map_err(|e| CadError::InvalidInput(e.to_string()))?;
        self.controller
            .borrow_mut()
            .import_annotations_json(&text, cad_annotations::FingerprintPolicy::RejectMismatch)
            .map(|_| ())
    }
}
impl UiCommandSink for Runtime {
    fn send(&mut self, command: Command) -> CadResult<()> {
        self.execute(command)
    }
}

pub struct LinuxApp {
    pub adapter: UiAdapter,
    runtime: Runtime,
    timer: slint::Timer,
}
impl LinuxApp {
    pub fn new(options: LinuxOptions) -> CadResult<Self> {
        let controller = Rc::new(RefCell::new(HostController::with_demo_document(
            options.size,
        )?));
        let runtime = Runtime {
            controller: controller.clone(),
            handle: Rc::new(RefCell::new(None)),
            view: Rc::new(RefCell::new(None)),
            options: options.clone(),
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
        adapter.fit_window_to_logical(options.size, 1.0);
        *runtime.handle.borrow_mut() = Some(adapter.handle());
        runtime.metrics()?;
        if options.drawing.is_some() {
            runtime.open()?;
        } else {
            controller.borrow_mut().fit()?;
        }
        let incoming = Rc::new(RefCell::new(controller.borrow().drawing()));
        let view = cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming)?;
        *runtime.view.borrow_mut() = Some(view);
        adapter.set_view_input(Rc::new(input::Navigation::new(runtime.clone())));
        adapter.set_canvas_pick_mapper(Rc::new(runtime.clone()));
        adapter.set_draw_command_sink(Box::new(runtime.clone()));
        adapter.set_draw_preview_sink(Rc::new(runtime.clone()));
        runtime.push()?;
        adapter.handle().set_status(runtime.message(
            if options.drawing.is_some() {
                "linux.opened"
            } else {
                "status.synthetic"
            },
            &[],
        ))?;
        // Retain the timer: real desktop resize and drawer changes update content metrics without refitting.
        let timer = slint::Timer::default();
        let rt = runtime.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                if let Err(error) = rt.metrics().and_then(|_| rt.sync_view()) {
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
        })
    }
    pub fn run(&self) -> CadResult<()> {
        let rt = self.runtime.clone();
        self.adapter.window().on_close_requested(move || {
            if rt.controller.borrow().unsaved_signal().dirty {
                if let Some(handle) = rt.handle.borrow().as_ref() {
                    let _ = handle.set_status(rt.message("linux.dirty", &[]));
                }
                slint::CloseRequestResponse::KeepWindowShown
            } else {
                slint::CloseRequestResponse::HideWindow
            }
        });
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
