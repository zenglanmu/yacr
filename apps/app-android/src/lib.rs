//! Android host: Activity entry, lifecycle, file access and GPU composition.
//!
//! Spec v2.0 §9.1, §5.3. This host wires the shared Slint UI to the CAD core and
//! the wgpu renderer through `cad_app::host::HostController`. It does not
//! re-implement CAD logic or duplicate the web host's business path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::{Command, CommandId};
use cad_domain::*;
use cad_ui_slint::{CadView, IncomingDocument, UiAdapter, UiCommandSink, UiConfiguration, UiHandle};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;
type SharedView = Rc<RefCell<Option<CadView>>>;

pub struct AndroidHostConfiguration {
    pub recovery_enabled: bool,
    /// Candidate DWG locations to try on open (app-private/external dirs).
    pub sample_paths: Vec<String>,
}

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            recovery_enabled: true,
            sample_paths: vec![
                "/sdcard/Download/yacr-sample.dwg".to_string(),
                "/storage/emulated/0/Download/yacr-sample.dwg".to_string(),
            ],
        }
    }
}

/// Commands from the UI are executed through the shared application layer.
struct HostSink {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    incoming: IncomingDocument,
    configuration: AndroidHostConfiguration,
}

impl HostSink {
    fn status(&self, text: impl Into<String>) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.set_status(text.into());
        }
    }

    fn sync_camera(&self) {
        let controller = self.controller.borrow();
        if let (Some(view), Some(viewport)) = (
            self.view.borrow().as_ref(),
            controller.application.workspace.viewports.get(&controller.viewport_id),
        ) {
            view.set_camera(viewport.camera.target, viewport.world_per_px());
        }
    }

    /// Open a DWG from the candidate paths. Real SAF integration is not part of
    /// this build; the search is explicitly reported so it is not mistaken for
    /// a file picker (spec §9.1).
    fn open_drawing(&mut self) {
        for path in &self.configuration.sample_paths {
            let candidate = std::path::Path::new(path);
            if !candidate.exists() {
                continue;
            }
            match std::fs::read(candidate) {
                Ok(bytes) => {
                    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
                    let opened = self.controller.borrow_mut().open_bytes(bytes, path);
                    match opened {
                        Ok(opened) => {
                            let drawing = {
                                let mut controller = self.controller.borrow_mut();
                                let _ = controller.fit();
                                controller.drawing()
                            };
                            *self.incoming.borrow_mut() = drawing;
                            self.sync_camera();
                            if let Some(view) = self.view.borrow().as_ref() {
                                view.request_redraw();
                            }
                            self.status(format!("已打开 {path}: {}", opened.completeness_label));
                            return;
                        }
                        Err(e) => self.status(format!("打开 {path} 失败: {e}")),
                    }
                }
                Err(e) => self.status(format!("读取 {path} 失败: {e}")),
            }
        }
        // No DWG found: keep the synthetic demo visible and say so.
        self.status("未找到样本 DWG；显示内置演示几何（非兼容性声明）");
    }
}

impl UiCommandSink for HostSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        if command.id == CommandId::OpenDrawing {
            self.open_drawing();
            return Ok(());
        }
        let outcome = self.controller.borrow_mut().execute(command);
        match outcome {
            Ok(outcome) => {
                self.sync_camera();
                let can_undo = {
                    let controller = self.controller.borrow();
                    controller.application.can_undo(&controller.document_id)
                };
                if let Some(handle) = self.handle.borrow().as_ref() {
                    let _ = handle.set_can_undo(can_undo);
                }
                if let Some(diag) = outcome.diagnostics.first() {
                    self.status(diag.message.clone());
                }
                Ok(())
            }
            Err(CadError::NotImplemented(feature)) => {
                self.status(format!("未实现：{feature}"));
                Ok(())
            }
            Err(e) => {
                self.status(format!("命令失败：{e}"));
                Ok(())
            }
        }
    }
}

/// Build the shared UI + core + renderer stack and run it.
pub fn start(configuration: AndroidHostConfiguration) -> CadResult<()> {
    cad_ui_slint::select_wgpu_backend()?;

    let controller = Rc::new(RefCell::new(HostController::with_demo_document([1080.0, 1920.0])?));
    let (drawing, document_id, viewport_id) = {
        let mut controller = controller.borrow_mut();
        let _ = controller.fit();
        (controller.drawing(), controller.document_id.clone(), controller.viewport_id.clone())
    };
    let incoming: IncomingDocument = Rc::new(RefCell::new(drawing));

    let ui_config = UiConfiguration {
        compact: true,
        locale: "zh-CN".into(),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD".into(),
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
    let view = cad_ui_slint::install_cad_bridge(handle.clone(), adapter.window(), incoming)?;
    {
        let controller = controller.borrow();
        if let Some(viewport) = controller.application.workspace.viewports.get(&controller.viewport_id) {
            view.set_camera(viewport.camera.target, view.world_per_px());
        }
    }
    *shared_view.borrow_mut() = Some(view);
    adapter.run()
}

/// Android entry point (`android-activity` 0.6 calls `fn android_main(app)`).
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    if let Err(e) = slint::android::init(app) {
        log::error!("slint android init failed: {e}");
        return;
    }
    if let Err(e) = start(AndroidHostConfiguration::default()) {
        log::error!("yacr android host failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_database_has_geometry_and_bounds() {
        let controller = HostController::with_demo_document([1080.0, 1920.0]).unwrap();
        let db = controller.drawing().unwrap();
        assert!(db.entity_count() >= 6);
        let (min, max) = db.bounds().unwrap();
        assert!(max.x - min.x > 0.0);
    }
}