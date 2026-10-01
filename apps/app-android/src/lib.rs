//! Android host: Activity entry, lifecycle, file access and GPU composition.
//!
//! Spec v2.0 §9.1, §5.3. This host wires the shared Slint UI to the CAD core and
//! the wgpu renderer through `cad_app::host::HostController`. It does not
//! re-implement CAD logic or duplicate the web host's business path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::host_files::{
    export_annotations_atomically, load_recovery, resolve_leave, UnsavedDecisionSource,
};
use cad_app::{Command, CommandId, UnsavedDecision};
use cad_domain::*;
use cad_platform::{HostFuture, Persistence};
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

/// Recovery cache stored as one JSON file per document under a host directory.
///
/// The directory is supplied by the embedding Activity (app-private files dir);
/// no path is guessed. A host that leaves it `None` has no recovery cache and
/// `PreserveRecovery` is refused explicitly (see `cad_app::host_files`).
#[derive(Debug, Clone)]
pub struct AndroidRecovery {
    directory: std::path::PathBuf,
}

impl AndroidRecovery {
    pub fn new(directory: impl Into<std::path::PathBuf>) -> Self {
        AndroidRecovery {
            directory: directory.into(),
        }
    }

    /// Deterministic per-document file path, so a bookmarked id maps to one file.
    fn path_for(&self, document: DocumentId) -> std::path::PathBuf {
        self.directory
            .join(format!("yacr-recovery-{:032x}.json", document.0))
    }
}

impl Persistence for AndroidRecovery {
    fn save_recovery(&self, document: DocumentId, bytes: Arc<[u8]>) -> HostFuture<'_, ()> {
        let path = self.path_for(document);
        Box::pin(async move {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CadError::ResourceMissing(format!(
                        "cannot create recovery dir {}: {e}",
                        parent.display()
                    ))
                })?;
            }
            std::fs::write(&path, &bytes).map_err(|e| {
                CadError::ResourceMissing(format!(
                    "cannot write recovery snapshot {}: {e}",
                    path.display()
                ))
            })
        })
    }

    fn load_recovery(&self, document: DocumentId) -> HostFuture<'_, Option<Arc<[u8]>>> {
        let path = self.path_for(document);
        Box::pin(async move {
            match std::fs::read(&path) {
                Ok(bytes) => Ok(Some(Arc::from(bytes.into_boxed_slice()))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(CadError::ResourceMissing(format!(
                    "cannot read recovery snapshot {}: {e}",
                    path.display()
                ))),
            }
        })
    }

    fn discard_recovery_after_confirmation(&self, document: DocumentId) -> HostFuture<'_, ()> {
        let path = self.path_for(document);
        Box::pin(async move {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(CadError::ResourceMissing(format!(
                    "cannot remove recovery snapshot {}: {e}",
                    path.display()
                ))),
            }
        })
    }
}

/// The host's recovery store, if one is configured and enabled.
fn recovery_store(configuration: &AndroidHostConfiguration) -> Option<AndroidRecovery> {
    if !configuration.recovery_enabled {
        return None;
    }
    configuration
        .recovery_directory
        .as_ref()
        .map(AndroidRecovery::new)
}

/// Write the annotation sidecar JSON into a host directory.
///
/// Returns whether the durable write succeeded. A missing directory or any I/O
/// error returns `false`, so the caller never confirms an unsaved export.
fn write_annotation_export(directory: Option<&str>, json: &str) -> bool {
    let Some(directory) = directory else {
        return false;
    };
    let directory = std::path::Path::new(directory);
    if std::fs::create_dir_all(directory).is_err() {
        return false;
    }
    std::fs::write(directory.join("annotations.cadnotes.json"), json.as_bytes()).is_ok()
}

pub struct AndroidHostConfiguration {
    pub recovery_enabled: bool,
    /// Candidate DWG locations to try on open (app-private/external dirs).
    pub sample_paths: Vec<String>,
    /// Directory for the per-document recovery cache. `None` means the host has
    /// no recovery storage and the decision model refuses `PreserveRecovery`
    /// explicitly instead of pretending the work was preserved.
    pub recovery_directory: Option<String>,
    /// Directory for annotation sidecar export. `None` refuses `Save` with a
    /// concrete error; it never reports the annotations as saved.
    pub export_directory: Option<String>,
    /// The host's explicit unsaved-work decision source. Android has no dialog
    /// in this build, so the default is `None`, which means a dirty document is
    /// never replaced by an open (reported, not silently discarded).
    pub unsaved_decision: Option<Rc<dyn UnsavedDecisionSource>>,
}

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            recovery_enabled: true,
            sample_paths: vec![
                "/sdcard/Download/yacr-sample.dwg".to_string(),
                "/storage/emulated/0/Download/yacr-sample.dwg".to_string(),
            ],
            recovery_directory: None,
            export_directory: None,
            unsaved_decision: None,
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
    ///
    /// A dirty document is only replaced after the host supplies an explicit
    /// [`UnsavedDecision`] and the required host write is confirmed; otherwise
    /// the current document (and its recovery data) is kept.
    fn open_drawing(&mut self) {
        let sample_paths = self.configuration.sample_paths.clone();
        for path in &sample_paths {
            let candidate = std::path::Path::new(path);
            if !candidate.exists() {
                continue;
            }
            match std::fs::read(candidate) {
                Ok(bytes) => {
                    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
                    match self.open_through_leave_flow(path, bytes) {
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
                        Err(CadError::Cancelled) => {
                            self.status(format!("已取消打开 {path}：当前文档与未保存批注保留"));
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

    /// The host's recovery store, if it has one configured.
    fn recovery_store(&self) -> Option<AndroidRecovery> {
        recovery_store(&self.configuration)
    }

    /// Camera state for a recovery snapshot: the authoritative viewport.
    fn camera_state(&self) -> ([f64; 3], f64) {
        let controller = self.controller.borrow();
        controller
            .application
            .workspace
            .viewports
            .get(&controller.viewport_id)
            .map(|vp| {
                (
                    [vp.camera.target.x, vp.camera.target.y, vp.camera.target.z],
                    vp.world_per_px(),
                )
            })
            .unwrap_or(([0.0, 0.0, 0.0], 1.0))
    }

    /// Write the annotation sidecar to the configured export directory.
    fn export_directory(&self) -> Option<String> {
        self.configuration.export_directory.clone()
    }

    /// Apply the leave flow, then replace the document with `bytes`.
    fn open_through_leave_flow(
        &mut self,
        label: &str,
        bytes: Arc<[u8]>,
    ) -> CadResult<cad_app::host::OpenedDrawing> {
        let signal = self.controller.borrow().unsaved_signal();
        let decision = if signal.dirty {
            // The host must supply the decision; a missing source or a cancelled
            // prompt keeps the current document (no silent Discard).
            match self
                .configuration
                .unsaved_decision
                .as_ref()
                .and_then(|source| source.decide(&signal))
            {
                Some(decision) => decision,
                None => return Err(CadError::Cancelled),
            }
        } else {
            // Nothing unsaved: no write is required.
            UnsavedDecision::Discard
        };
        let store = self.recovery_store();
        let persistence = store.as_ref().map(|s| s as &dyn Persistence);
        let (center, wpp) = self.camera_state();
        let export_directory = self.export_directory();
        let resolution = cad_platform::block_on(resolve_leave(
            &mut self.controller.borrow_mut(),
            decision,
            center,
            wpp,
            persistence,
            |controller| {
                export_annotations_atomically(controller, |json| {
                    write_annotation_export(export_directory.as_deref(), json)
                })
            },
        ))?;
        self.controller.borrow_mut().open_bytes_leaving(
            bytes,
            label,
            decision,
            resolution.saved,
            resolution.recovery_persisted,
        )
    }

    /// Run the atomic annotation export through the host file write (F09).
    fn export_annotations(&mut self) {
        let Some(directory) = self.export_directory() else {
            self.status("导出失败：宿主未配置导出目录".to_string());
            return;
        };
        let result = export_annotations_atomically(&mut self.controller.borrow_mut(), |json| {
            write_annotation_export(Some(&directory), json)
        });
        match result {
            Ok(()) => self.status(format!(
                "已导出批注 JSON 到 {directory}/annotations.cadnotes.json"
            )),
            Err(e) => self.status(format!("导出失败：{e}")),
        }
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
        if command.id == CommandId::ExportAnnotations {
            self.export_annotations();
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

/// Restore a persisted recovery snapshot for the starting document, if any.
///
/// Returns a status message only when something was attempted or found, so a
/// host with no recovery storage stays silent (not a fake "recovered"). A
/// snapshot whose fingerprint does not match the current document is refused by
/// the strict policy and nothing is applied.
fn restore_recovery_for_start(
    controller: &Rc<RefCell<HostController>>,
    configuration: &AndroidHostConfiguration,
) -> Option<String> {
    let store = recovery_store(configuration)?;
    let document = controller.borrow().document_id;
    match cad_platform::block_on(load_recovery(&store, document)) {
        Ok(Some(snapshot)) => match controller.borrow_mut().restore_recovery_snapshot(&snapshot) {
            Ok(count) => Some(format!("已从恢复快照恢复 {count} 条批注")),
            Err(e) => Some(format!("恢复快照未应用：{e}")),
        },
        Ok(None) => None,
        Err(e) => Some(format!("恢复快照读取失败（未应用）：{e}")),
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
    let restore_message = restore_recovery_for_start(&controller, &configuration);
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
    if let Some(message) = restore_message {
        let _ = handle.set_status(message);
    }
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
