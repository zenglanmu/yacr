//! Browser host assembly (spec v2.0 §5.3, §6, §9.2).
//!
//! The web host is a thin shell: the Slint canvas is the single presentation
//! coordinator, the shared `UiAdapter` drives the same commands as Android, and
//! the CAD renderer receives Slint's wgpu device through the same bridge. File
//! opening uses the user-authorized File API; export uses a download. No CAD
//! logic is duplicated here.
//!
//! Compiled for wasm32 the crate exports `start_web`, `open_document_bytes`,
//! `annotation_export_json` and `annotation_import_json` for the minimal JS
//! host in `web/main.js`.

#[cfg(target_arch = "wasm32")]
use cad_app::host::HostController;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use wasm_bindgen::JsCast;

    use cad_app::{Command, CommandId, CommandPayload};
    use cad_domain::*;
    use cad_ui_slint::{
        install_cad_bridge, CadView, IncomingDocument, UiAdapter, UiCommandSink, UiConfiguration,
        UiHandle, ViewInput,
    };

    use super::HostController;

    type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

    /// Trigger a browser download for the exported sidecar JSON.
    fn download_text(filename: &str, text: &str) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let parts = js_sys::Array::new();
        parts.push(&wasm_bindgen::JsValue::from_str(text));
        let Ok(blob) = web_sys::Blob::new_with_str_sequence(&parts) else { return };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
        if let Ok(element) = document.create_element("a") {
            if let Ok(anchor) = element.dyn_into::<web_sys::HtmlAnchorElement>() {
                anchor.set_href(&url);
                anchor.set_download(filename);
                anchor.click();
            }
        }
        let _ = web_sys::Url::revoke_object_url(&url);
    }

    /// Ask the JS host to open the annotation sidecar picker.
    fn open_annotation_dialog() {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            if let Some(element) = document.get_element_by_id("annotation-input") {
                if let Ok(input) = element.dyn_into::<web_sys::HtmlInputElement>() {
                    input.click();
                }
            }
        }
    }

    fn preference_for(choice: cad_app::BackendChoice) -> cad_ui_slint::web::BackendPreference {
        match choice {
            cad_app::BackendChoice::Auto => cad_ui_slint::web::BackendPreference::Auto,
            cad_app::BackendChoice::WebGpu => cad_ui_slint::web::BackendPreference::WebGpu,
            cad_app::BackendChoice::WebGl2 => cad_ui_slint::web::BackendPreference::WebGl2,
        }
    }

    fn backend_index(preference: cad_ui_slint::web::BackendPreference) -> i32 {
        match preference {
            cad_ui_slint::web::BackendPreference::Auto => 0,
            cad_ui_slint::web::BackendPreference::WebGpu => 1,
            cad_ui_slint::web::BackendPreference::WebGl2 => 2,
        }
    }

    /// Logical viewport size from `window.innerWidth/innerHeight` (CSS pixels).
    fn web_viewport_size() -> [f64; 2] {
        let window = web_sys::window();
        let width = window.as_ref().map(|w| w.inner_width().ok()).flatten().and_then(|v| v.as_f64());
        let height = window.as_ref().map(|w| w.inner_height().ok()).flatten().and_then(|v| v.as_f64());
        [
            width.unwrap_or(1280.0).max(320.0),
            height.unwrap_or(800.0).max(240.0),
        ]
    }

    /// Human-facing name plus the probe results, for the status line and tests.
    fn backend_status(preference: cad_ui_slint::web::BackendPreference) -> String {
        let webgpu_api = cad_ui_slint::web::webgpu_api_present();
        let webgl2 = cad_ui_slint::web::webgl2_available();
        format!("{}（navigator.gpu={webgpu_api}, webgl2={webgl2}）", preference_name(preference))
    }

    fn preference_name(preference: cad_ui_slint::web::BackendPreference) -> &'static str {
        match preference {
            cad_ui_slint::web::BackendPreference::Auto => "Auto",
            cad_ui_slint::web::BackendPreference::WebGpu => "WebGPU",
            cad_ui_slint::web::BackendPreference::WebGl2 => "WebGL2",
        }
    }

    /// Runtime handed to the JS exports after `start_web` completes setup.
    struct HostRuntime {
        controller: Rc<RefCell<HostController>>,
        handle: UiHandle,
        view: CadView,
        incoming: IncomingDocument,
        viewport: ViewportId,
    }

    thread_local! {
        static HOST: RefCell<Option<HostRuntime>> = const { RefCell::new(None) };
    }

    fn with_runtime<T>(f: impl FnOnce(&HostRuntime) -> T) -> Option<T> {
        HOST.with(|slot| slot.borrow().as_ref().map(f))
    }

    /// Mirror the authoritative application viewport into the render camera.
    fn sync_view_camera(controller: &HostController, view: &CadView, viewport: &ViewportId) {
        if let Some(vp) = controller.application.workspace.viewports.get(viewport) {
            view.set_camera(vp.camera.target, vp.world_per_px());
        }
    }

    /// Send a command and adjust the shell state (status, history, camera).
    fn dispatch(
        controller: &Rc<RefCell<HostController>>,
        handle: &SharedHandle,
        view: &Rc<RefCell<Option<CadView>>>,
        viewport: &ViewportId,
        command: Command,
    ) {
        let can_undo;
        let status;
        {
            let mut c = controller.borrow_mut();
            match c.execute(command) {
                Ok(outcome) => {
                    can_undo = c.application.can_undo(&c.document_id);
                    status = outcome
                        .diagnostics
                        .first()
                        .map(|d| d.message.clone())
                        .unwrap_or_else(|| c.status().to_string());
                }
                Err(CadError::NotImplemented(feature)) => {
                    can_undo = c.application.can_undo(&c.document_id);
                    status = format!("未实现：{feature}");
                }
                Err(CadError::Unsupported(reason)) => {
                    can_undo = c.application.can_undo(&c.document_id);
                    status = format!("仅宿主执行：{reason}");
                }
                Err(e) => {
                    can_undo = c.application.can_undo(&c.document_id);
                    status = format!("命令失败：{e}");
                }
            }
            if let (Some(view), Some(vp)) =
                (view.borrow().as_ref(), c.application.workspace.viewports.get(viewport))
            {
                view.set_camera(vp.camera.target, vp.world_per_px());
            }
        }
        if let Some(handle) = handle.borrow().as_ref() {
            let _ = handle.set_status(status);
            let _ = handle.set_can_undo(can_undo);
        }
    }

    struct WebSink {
        controller: Rc<RefCell<HostController>>,
        handle: SharedHandle,
        view: Rc<RefCell<Option<CadView>>>,
        viewport: ViewportId,
    }

    fn open_dialog() {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            if let Some(element) = document.get_element_by_id("file-input") {
                if let Ok(input) = element.dyn_into::<web_sys::HtmlInputElement>() {
                    input.click();
                }
            }
        }
    }

    impl UiCommandSink for WebSink {
        fn send(&mut self, command: Command) -> CadResult<()> {
            let set_status = |text: String| {
                if let Some(handle) = self.handle.borrow().as_ref() {
                    let _ = handle.set_status(text);
                }
            };
            match command.id {
                CommandId::OpenDrawing => {
                    open_dialog();
                    set_status("请选择 DWG 文件…".into());
                    return Ok(());
                }
                CommandId::ExportAnnotations => {
                    match export_annotations_json() {
                        Ok(json) => {
                            download_text("annotations.cadnotes.json", &json);
                            set_status(format!("已导出 {} 字节批注 JSON", json.len()));
                        }
                        Err(e) => set_status(format!("导出失败：{e}")),
                    }
                    return Ok(());
                }
                CommandId::ImportAnnotations => {
                    open_annotation_dialog();
                    set_status("请选择批注 JSON 文件…".into());
                    return Ok(());
                }
                CommandId::SwitchBackend => {
                    if let CommandPayload::Backend(choice) = command.payload {
                        let preference = preference_for(choice);
                        set_status(format!("切换后端为 {preference:?}，重建渲染会话…"));
                        let _ = cad_ui_slint::web::store_preference_and_reload(preference);
                    } else {
                        set_status("SwitchBackend 需要后端参数".into());
                    }
                    return Ok(());
                }
                _ => {}
            }
            dispatch(&self.controller, &self.handle, &self.view, &self.viewport, command);
            Ok(())
        }
    }

    /// Pointer/scroll navigation routed through application commands.
    struct WebViewInput {
        controller: Rc<RefCell<HostController>>,
        handle: SharedHandle,
        view: Rc<RefCell<Option<CadView>>>,
        viewport: ViewportId,
        last: Cell<[f64; 2]>,
        dragging: Cell<bool>,
    }

    impl WebViewInput {
        fn send(&self, id: CommandId, payload: CommandPayload) {
            let document = self.controller.borrow().document_id.clone();
            let command = Command {
                schema_version: 1,
                id,
                document,
                viewport: self.viewport.clone(),
                payload,
            };
            dispatch(&self.controller, &self.handle, &self.view, &self.viewport, command);
        }
    }

    impl ViewInput for WebViewInput {
        fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
            match kind {
                0 => {
                    if button == 1 || button == 0 {
                        self.dragging.set(true);
                        self.last.set([x, y]);
                    }
                }
                2 => {
                    let last = self.last.get();
                    if self.dragging.get() {
                        let wpp = self
                            .controller
                            .borrow()
                            .application
                            .workspace
                            .viewports
                            .get(&self.viewport)
                            .map(|v| v.world_per_px())
                            .unwrap_or(1.0);
                        // Command delta is in world units; screen y is inverted.
                        let delta = Point3 { x: (x - last[0]) * wpp, y: -(y - last[1]) * wpp, z: 0.0 };
                        self.send(CommandId::Pan, CommandPayload::Points(vec![delta]));
                    }
                    self.last.set([x, y]);
                }
                1 | 3 => {
                    self.dragging.set(false);
                    self.last.set([x, y]);
                }
                _ => {}
            }
        }

        fn scroll(&self, _dx: f64, dy: f64) {
            let factor = (1.0 - dy * 0.0015).clamp(0.2, 5.0);
            self.last.set([0.0, 0.0]);
            self.send(CommandId::Zoom, CommandPayload::Points(vec![Point3 { x: factor, y: 0.0, z: 0.0 }]));
        }
    }

    /// Start the browser host. Must be called after the DOM is ready.
    pub async fn start() -> CadResult<()> {
        let preference = cad_ui_slint::web::stored_preference();
        let chosen = match cad_ui_slint::web::select_backend(preference).await {
            Ok(chosen) => chosen,
            Err(e) => {
                // A forced backend that cannot initialize must report and fall
                // back instead of leaving a blank page (spec §6).
                cad_ui_slint::web::console_error(&format!("{e}"));
                if preference == cad_ui_slint::web::BackendPreference::Auto {
                    return Err(e);
                }
                cad_ui_slint::web::select_backend(cad_ui_slint::web::BackendPreference::WebGl2).await?
            }
        };

        let controller = Rc::new(RefCell::new(HostController::with_demo_document([1280.0, 720.0])?));
        let (drawing, document_id, viewport_id) = {
            let mut c = controller.borrow_mut();
            let _ = c.fit();
            (c.drawing(), c.document_id.clone(), c.viewport_id.clone())
        };
        let incoming: IncomingDocument = Rc::new(RefCell::new(drawing));
        let shared_handle: SharedHandle = Rc::new(RefCell::new(None));
        let view_slot: Rc<RefCell<Option<CadView>>> = Rc::new(RefCell::new(None));

        let configuration = UiConfiguration {
            compact: false,
            locale: "zh-CN".into(),
            safe_insets: [0.0; 4],
            application_title: "yacr CAD (Web)".into(),
            document: document_id,
            viewport: viewport_id.clone(),
            logical_size: web_viewport_size(),
        };

        let sink = WebSink {
            controller: controller.clone(),
            handle: shared_handle.clone(),
            view: view_slot.clone(),
            viewport: viewport_id.clone(),
        };
        let adapter = UiAdapter::new(configuration, sink, true)?;
        let handle = adapter.handle();
        *shared_handle.borrow_mut() = Some(handle.clone());
        let _ = handle.set_backend_index(backend_index(preference));

        let view = install_cad_bridge(handle.clone(), adapter.window(), incoming.clone())?;
        {
            let c = controller.borrow();
            if let Some(vp) = c.application.workspace.viewports.get(&viewport_id) {
                view.set_camera(vp.camera.target, vp.world_per_px());
            }
        }
        view.request_redraw();
        *view_slot.borrow_mut() = Some(view.clone());
        adapter.set_view_input(Rc::new(WebViewInput {
            controller: controller.clone(),
            handle: shared_handle.clone(),
            view: view_slot.clone(),
            viewport: viewport_id.clone(),
            last: Cell::new([0.0, 0.0]),
            dragging: Cell::new(false),
        }));

        let backend_label = backend_status(chosen);
        let _ = handle.set_status(format!("就绪（{backend_label}）"));

        HOST.with(|slot| {
            *slot.borrow_mut() = Some(HostRuntime {
                controller: controller.clone(),
                handle: handle.clone(),
                view: view.clone(),
                incoming,
                viewport: viewport_id.clone(),
            });
        });

        // Hands the browser event loop its winit app. On wasm this may not
        // return: winit uses a thrown exception as its control-flow handoff,
        // which `main.js` catches explicitly.
        adapter.run()
    }

    /// Open a drawing selected through the File API (bytes already read in JS).
    pub fn open_document(name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let (controller, handle, view, incoming, viewport) = with_runtime(|rt| {
            (rt.controller.clone(), rt.handle.clone(), rt.view.clone(), rt.incoming.clone(), rt.viewport.clone())
        })
        .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let result = controller.borrow_mut().open_bytes(bytes, name);
        match result {
            Ok(opened) => {
                let drawing = {
                    let mut c = controller.borrow_mut();
                    let _ = c.fit();
                    c.drawing()
                };
                *incoming.borrow_mut() = drawing;
                {
                    let c = controller.borrow();
                    sync_view_camera(&c, &view, &viewport);
                }
                view.request_redraw();
                let _ = handle.set_status(format!("已打开 {name}: {}", opened.completeness_label));
                Ok(())
            }
            Err(e) => {
                let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
                Err(e.to_string())
            }
        }
    }

    /// Report renderer state for diagnostics and headless tests.
    pub fn renderer_report() -> String {
        let report = with_runtime(|rt| {
            let controller = rt.controller.borrow();
            format!(
                "chosen={:?} adapter={:?} caps={:?} error={:?} entities={} wpp={:.6} status={}",
                rt.view.preference(),
                rt.view.active_backend(),
                rt.view.capabilities(),
                rt.view.last_error(),
                rt.incoming.borrow().as_ref().map(|d| d.entity_count()).unwrap_or(0),
                rt.view.camera().world_per_px,
                controller.status(),
            )
        });
        report.unwrap_or_else(|| "host not started".to_string())
    }

    /// Export annotation JSON for the JS host to download.
    pub fn export_annotations_json() -> Result<String, String> {
        let (controller, handle) = with_runtime(|rt| (rt.controller.clone(), rt.handle.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let json = controller.borrow().export_annotations_json().map_err(|e| e.to_string())?;
        // Only mark saved after the bytes are in hand for the download.
        controller.borrow_mut().mark_annotations_saved().map_err(|e| e.to_string())?;
        let _ = handle.set_status(format!("已导出 {} 字节批注 JSON", json.len()));
        Ok(json)
    }

    /// Import annotations from JSON text chosen by the user.
    pub fn import_annotations_json(text: &str) -> Result<usize, String> {
        let (controller, handle) = with_runtime(|rt| (rt.controller.clone(), rt.handle.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let mut c = controller.borrow_mut();
        let count = c
            .import_annotations_json(text, cad_annotations::FingerprintPolicy::RejectMismatch)
            .map_err(|e| e.to_string())?;
        drop(c);
        let _ = handle.set_status(format!("已导入 {count} 条批注"));
        Ok(count)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{export_annotations_json, import_annotations_json, open_document, renderer_report, start};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Entry point for `web/main.js`; resolves once the Slint/wgpu session is up.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web() -> Result<(), JsValue> {
    start().await.map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Open a drawing from bytes read by the JS File API host.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes(name: String, bytes: Vec<u8>) -> Result<(), JsValue> {
    open_document(&name, bytes).map_err(|e| JsValue::from_str(&e))
}

/// Serializable renderer/state report for the diagnostics panel and tests.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn renderer_state_report() -> String {
    renderer_report()
}

/// Annotation JSON export (caller downloads the returned text).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_export_json() -> Result<String, JsValue> {
    export_annotations_json().map_err(|e| JsValue::from_str(&e))
}

/// Annotation JSON import from user-chosen text.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_import_json(text: &str) -> Result<usize, JsValue> {
    import_annotations_json(text).map_err(|e| JsValue::from_str(&e))
}

/// Native builds cannot run the browser host; this is a platform constraint,
/// not a placeholder. Use `wasm32-unknown-unknown` (see `docs/build.md`).
#[cfg(not(target_arch = "wasm32"))]
pub fn start() -> cad_domain::CadResult<()> {
    Err(cad_domain::CadError::Unsupported(
        "app-web is a wasm32 host; build with --target wasm32-unknown-unknown".into(),
    ))
}

/// Native builds of the browser host have no browser File API.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_document(_name: &str, _bytes: Vec<u8>) -> Result<(), String> {
    Err("app-web is a wasm32 host".into())
}