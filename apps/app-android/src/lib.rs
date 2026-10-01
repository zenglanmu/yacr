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
use cad_ui_slint::{
    CadView, IncomingDocument, UiAdapter, UiCommandSink, UiConfiguration, UiHandle,
};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;
type SharedView = Rc<RefCell<Option<CadView>>>;

/// Android host font loading from APK assets (`assets/fonts/…`).
///
/// The mechanism is the same catalog → plan → register pipeline the web host
/// uses, but bytes come from the activity's `AssetManager` instead of the
/// network. **No font files are bundled** (licence, see `docs/fonts.md`), so an
/// APK without the optional `assets/fonts/` package reports a missing asset
/// rather than faking a font.
#[cfg(target_os = "android")]
mod android_fonts {
    use std::cell::RefCell;
    use std::ffi::CString;
    use std::future::Future;
    use std::io::Read;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use android_activity::ndk::asset::AssetManager;
    use cad_domain::{CadError, CadResult};
    use cad_platform::fonts::{load_font_engine, FontLoadReport};
    use cad_platform::{FontLoader, HostFuture};
    use cad_ui_slint::CadView;

    /// Logical base for fonts packaged under the APK's `assets/` directory.
    pub const ASSET_BASE: &str = "asset://fonts/";

    thread_local! {
        static ASSETS: RefCell<Option<AssetManager>> = const { RefCell::new(None) };
    }

    /// Record the activity asset manager once, before the UI starts.
    pub fn set_asset_manager(manager: AssetManager) {
        ASSETS.with(|slot| *slot.borrow_mut() = Some(manager));
    }

    fn read_asset(url: &str) -> CadResult<Arc<[u8]>> {
        let path = url.strip_prefix(ASSET_BASE).unwrap_or(url);
        let name = CString::new(path)
            .map_err(|_| CadError::InvalidInput(format!("font asset path has NUL: {path}")))?;
        ASSETS.with(|slot| {
            let borrow = slot.borrow();
            let manager = borrow.as_ref().ok_or_else(|| {
                CadError::Unsupported(
                    "Android font loading needs the activity AssetManager".to_string(),
                )
            })?;
            let mut asset = manager.open(&name).ok_or_else(|| {
                CadError::ResourceMissing(format!(
                    "font asset not packaged (assets/{path}); fonts are not bundled (licence)"
                ))
            })?;
            let mut bytes = Vec::with_capacity(asset.length());
            asset.read_to_end(&mut bytes).map_err(|e| {
                CadError::ResourceMissing(format!("reading font asset assets/{path} failed: {e}"))
            })?;
            Ok(Arc::from(bytes.into_boxed_slice()))
        })
    }

    struct AssetFontLoader;

    impl FontLoader for AssetFontLoader {
        fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
            let url = url.to_string();
            Box::pin(async move { read_asset(&url) })
        }
    }

    /// Asset reads resolve immediately, so a no-op-waker executor is enough;
    /// this is not a general-purpose runtime.
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    /// Load the referenced fonts into `view` from packaged assets.
    pub fn install_fonts(
        view: Option<&CadView>,
        requested: &[String],
    ) -> CadResult<FontLoadReport> {
        let (engine, report) = block_on(load_font_engine(&AssetFontLoader, requested, ASSET_BASE))?;
        if let Some(view) = view {
            if report.registered.is_empty() {
                view.clear_fonts();
            } else {
                view.set_fonts(engine);
            }
        }
        Ok(report)
    }

    /// Human-facing summary of one loading pass.
    pub fn summary(report: &FontLoadReport, requested: usize) -> String {
        format!(
            "字体：目录 {}，引用 {}，计划 {}，注册 {}，失败 {}",
            report.catalog_entries,
            requested,
            report.planned.len(),
            report.registered.len(),
            report.failed.len(),
        )
    }
}

/// Initial logical size of the demo viewport. The UI configuration and the
/// initial viewport must agree; real dimensions arrive from the surface later.
const DEMO_LOGICAL_SIZE: [f64; 2] = [1080.0, 1920.0];

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
            controller
                .application
                .workspace
                .viewports
                .get(&controller.viewport_id),
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
                            #[cfg(target_os = "android")]
                            self.load_fonts_for_current_document();
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

    /// Load the fonts the current drawing references from packaged assets.
    ///
    /// Not compiled off Android (assets only exist inside the activity). A
    /// missing catalogue/font asset is reported; it is never treated as a
    /// successful empty font set.
    #[cfg(target_os = "android")]
    fn load_fonts_for_current_document(&self) {
        let requested = {
            let controller = self.controller.borrow();
            match controller.drawing() {
                Some(drawing) => cad_platform::fonts::requested_fonts(drawing.as_ref()),
                None => Vec::new(),
            }
        };
        let view = self.view.borrow();
        if requested.is_empty() {
            if let Some(view) = view.as_ref() {
                view.clear_fonts();
            }
            return;
        }
        match android_fonts::install_fonts(view.as_ref(), &requested) {
            Ok(report) => self.status(android_fonts::summary(&report, requested.len())),
            Err(e) => self.status(format!("字体加载未完成：{e}")),
        }
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
    let view = cad_ui_slint::install_cad_bridge(handle.clone(), adapter.window(), incoming)?;
    {
        let controller = controller.borrow();
        if let Some(viewport) = controller
            .application
            .workspace
            .viewports
            .get(&controller.viewport_id)
        {
            view.set_camera(viewport.camera.target, viewport.world_per_px());
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
