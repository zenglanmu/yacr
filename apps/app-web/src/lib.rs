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
mod browser {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use wasm_bindgen::JsCast;

    use cad_app::host::HostController;
    use cad_app::host_files::{
        export_annotations_atomically, load_recovery, parse_decision, parse_recovery,
        persist_recovery, resolve_leave,
    };
    use cad_platform::{HostFuture, Persistence};

    use cad_app::{Command, CommandId, CommandPayload};
    use cad_domain::*;
    use cad_platform::fonts::{load_font_engine, requested_fonts, FontLoadReport};
    use cad_platform::FontLoader;
    use cad_resources::DEFAULT_FONT_BASE_URL;
    use cad_ui_slint::{
        install_with_preference, CadView, IncomingDocument, UiAdapter, UiCommandSink,
        UiConfiguration, UiHandle, ViewInput,
    };
    use wasm_bindgen_futures::JsFuture;

    type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

    /// Trigger a browser download for the exported sidecar JSON.
    ///
    /// Returns whether the download was successfully initiated. The host must
    /// only mark the document saved when this is `true` (audit B07).
    fn download_text(filename: &str, text: &str) -> bool {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return false;
        };
        let parts = js_sys::Array::new();
        parts.push(&wasm_bindgen::JsValue::from_str(text));
        let Ok(blob) = web_sys::Blob::new_with_str_sequence(&parts) else {
            return false;
        };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            return false;
        };
        let mut clicked = false;
        if let Ok(element) = document.create_element("a") {
            if let Ok(anchor) = element.dyn_into::<web_sys::HtmlAnchorElement>() {
                anchor.set_href(&url);
                anchor.set_download(filename);
                anchor.click();
                clicked = true;
            }
        }
        let _ = web_sys::Url::revoke_object_url(&url);
        clicked
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

    /// Browser recovery cache over `localStorage` (audit B06).
    ///
    /// The browser host keeps a single recovery slot (`cad_ui_slint::web`), so
    /// the `DocumentId` is recorded inside the encoded snapshot rather than in
    /// the storage key; the strict fingerprint policy still refuses a snapshot
    /// that belongs to another drawing. A failed `setItem` returns an error so
    /// the caller never reloads and never reports unsaved work as preserved.
    struct WebPersistence;

    impl Persistence for WebPersistence {
        fn save_recovery(&self, _document: DocumentId, bytes: Arc<[u8]>) -> HostFuture<'_, ()> {
            Box::pin(async move {
                let text = std::str::from_utf8(&bytes).map_err(|e| {
                    CadError::CorruptData(format!("recovery snapshot is not UTF-8: {e}"))
                })?;
                cad_ui_slint::web::store_recovery_snapshot(text)
            })
        }

        fn load_recovery(&self, _document: DocumentId) -> HostFuture<'_, Option<Arc<[u8]>>> {
            Box::pin(async move {
                Ok(cad_ui_slint::web::peek_recovery_snapshot()
                    .map(|text| Arc::from(text.into_bytes().into_boxed_slice())))
            })
        }

        fn discard_recovery_after_confirmation(&self, _document: DocumentId) -> HostFuture<'_, ()> {
            Box::pin(async move {
                cad_ui_slint::web::clear_recovery_snapshot();
                Ok(())
            })
        }
    }

    /// The authoritative viewport camera as a recovery snapshot's camera.
    fn viewport_camera(controller: &HostController) -> ([f64; 3], f64) {
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
        let width = window
            .as_ref()
            .map(|w| w.inner_width().ok())
            .flatten()
            .and_then(|v| v.as_f64());
        let height = window
            .as_ref()
            .map(|w| w.inner_height().ok())
            .flatten()
            .and_then(|v| v.as_f64());
        [
            width.unwrap_or(1280.0).max(320.0),
            height.unwrap_or(800.0).max(240.0),
        ]
    }

    /// Human-facing name plus the probe results, for the status line and tests.
    fn backend_status(preference: cad_ui_slint::web::BackendPreference) -> String {
        let webgpu_api = cad_ui_slint::web::webgpu_api_present();
        let webgl2 = cad_ui_slint::web::webgl2_available();
        format!(
            "{}（navigator.gpu={webgpu_api}, webgl2={webgl2}）",
            preference_name(preference)
        )
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

    /// The UI handle for the running host, if it has started.
    ///
    /// Exposed so wasm exports (locale switch) can reach the Slint shell
    /// without duplicating the thread-local lookup.
    pub fn current_handle() -> Option<UiHandle> {
        with_runtime(|rt| rt.handle.clone())
    }

    // Last font-loading report, surfaced for diagnostics and headless tests.
    thread_local! {
        static FONT_REPORT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn set_font_status(handle: &UiHandle, text: String) {
        let _ = handle.set_status(text.clone());
        FONT_REPORT.with(|slot| *slot.borrow_mut() = Some(text));
    }

    /// Human-readable font report (catalog/plan/registered/failed counts).
    pub fn font_report() -> String {
        FONT_REPORT
            .with(|slot| slot.borrow().clone())
            .unwrap_or_else(|| "字体：未加载".to_string())
    }

    /// Fetches font bytes through the browser Fetch API (`fetch().arrayBuffer()`).
    ///
    /// Implements [`cad_platform::FontLoader`] so the catalog/plan/register
    /// pipeline is the same one Android uses. Cross-origin reads rely on the
    /// CDN's CORS headers; a non-OK response is an explicit error, never empty
    /// bytes.
    pub struct WebFontLoader;

    async fn fetch_bytes(url: &str) -> CadResult<Arc<[u8]>> {
        let window = web_sys::window()
            .ok_or_else(|| CadError::Invariant("font fetch needs a browser window".into()))?;
        let response_value = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(|e| {
                CadError::ResourceMissing(format!("font fetch failed for {url}: {e:?}"))
            })?;
        let response: web_sys::Response = response_value.dyn_into().map_err(|_| {
            CadError::Invariant(format!("font fetch {url} did not return a Response"))
        })?;
        if !response.ok() {
            return Err(CadError::ResourceMissing(format!(
                "font fetch {url} returned HTTP {}",
                response.status()
            )));
        }
        let buffer = response.array_buffer().map_err(|e| {
            CadError::Invariant(format!("font arrayBuffer failed for {url}: {e:?}"))
        })?;
        let buffer = JsFuture::from(buffer)
            .await
            .map_err(|e| CadError::ResourceMissing(format!("font body failed for {url}: {e:?}")))?;
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        Ok(Arc::from(bytes.into_boxed_slice()))
    }

    impl FontLoader for WebFontLoader {
        fn load_font(&self, url: &str) -> cad_platform::HostFuture<'_, Arc<[u8]>> {
            let url = url.to_string();
            Box::pin(async move { fetch_bytes(&url).await })
        }
    }

    /// Load and install the shaping fonts the current drawing references.
    ///
    /// Reads the referenced keys from the public database API, fetches the
    /// catalog from [`DEFAULT_FONT_BASE_URL`], registers each planned face with
    /// its catalog encoding and installs the registered keys as a fallback
    /// chain. A stale load (another drawing was opened while fetching) is
    /// discarded with [`CadError::StaleResult`]; an empty engine clears fonts
    /// rather than installing one that would fail every glyph.
    pub async fn load_current_fonts() -> CadResult<FontLoadReport> {
        let (controller, handle, view, incoming) = with_runtime(|rt| {
            (
                rt.controller.clone(),
                rt.handle.clone(),
                rt.view.clone(),
                rt.incoming.clone(),
            )
        })
        .ok_or_else(|| CadError::Invariant("浏览器宿主尚未启动".into()))?;

        let (requested, identity) = {
            let controller = controller.borrow();
            match controller.drawing() {
                Some(drawing) => (
                    requested_fonts(drawing.as_ref()),
                    Some(drawing.scene_identity()),
                ),
                None => (Vec::new(), None),
            }
        };

        if requested.is_empty() {
            view.clear_fonts();
            let report = FontLoadReport::default();
            set_font_status(&handle, "字体：图纸未引用 CAD 文本字体".into());
            return Ok(report);
        }

        let (engine, report) =
            load_font_engine(&WebFontLoader, &requested, DEFAULT_FONT_BASE_URL).await?;

        // Another drawing may have replaced this one while the fetch was in
        // flight; do not apply old fonts to the new content.
        let current = incoming.borrow().as_ref().map(|d| d.scene_identity());
        if current != identity {
            return Err(CadError::StaleResult);
        }

        if report.registered.is_empty() {
            view.clear_fonts();
        } else {
            view.set_fonts(engine);
        }
        let text = format!(
            "字体：目录 {}，引用 {}，计划 {}，注册 {}，失败 {}",
            report.catalog_entries,
            requested.len(),
            report.planned.len(),
            report.registered.len(),
            report.failed.len(),
        );
        set_font_status(&handle, text);
        Ok(report)
    }

    /// Kick off a best-effort font load for the current drawing.
    ///
    /// Used after an open; failures are surfaced on the status line and never
    /// affect the already-open document.
    pub fn spawn_font_load() {
        wasm_bindgen_futures::spawn_local(async {
            if let Err(e) = load_current_fonts().await {
                if let Some(handle) = with_runtime(|rt| rt.handle.clone()) {
                    let _ = handle.set_status(format!("字体加载未完成：{e}"));
                }
            }
        });
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
            if let (Some(view), Some(vp)) = (
                view.borrow().as_ref(),
                c.application.workspace.viewports.get(viewport),
            ) {
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
                    // Atomic path shared with Android: prepare (pure) → host
                    // write → confirm the exact revision. A failed download
                    // never marks the document saved (audit B07).
                    let result =
                        export_annotations_atomically(&mut self.controller.borrow_mut(), |json| {
                            download_text("annotations.cadnotes.json", json)
                        });
                    match result {
                        Ok(()) => set_status("批注已确认保存到下载文件".into()),
                        Err(e) => set_status(format!("导出失败：{e}（批注仍为未保存）")),
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
                        // Protect unsaved annotations across the reload (B06):
                        // capture a recovery snapshot and persist it through the
                        // shared Persistence contract first; only reload if the
                        // snapshot is durable (or there is nothing unsaved).
                        {
                            let c = self.controller.borrow();
                            let signal = c.unsaved_signal();
                            if signal.dirty {
                                let (center, wpp) = viewport_camera(&c);
                                let snapshot = match c.capture_recovery_snapshot(center, wpp) {
                                    Ok(snapshot) => snapshot,
                                    Err(e) => {
                                        set_status(format!(
                                            "切换失败：无法生成恢复快照（{e}），未重载"
                                        ));
                                        return Ok(());
                                    }
                                };
                                if let Err(e) = cad_platform::block_on(persist_recovery(
                                    &WebPersistence,
                                    c.document_id,
                                    &snapshot,
                                )) {
                                    set_status(format!(
                                        "切换失败：恢复快照未能持久化（{e}），未重载"
                                    ));
                                    return Ok(());
                                }
                            }
                        }
                        set_status(format!("切换后端为 {preference:?}，重建渲染会话…"));
                        if let Err(e) = cad_ui_slint::web::store_preference_and_reload(preference) {
                            set_status(format!("切换失败：{e}（未保存批注未丢失）"));
                        }
                    } else {
                        set_status("SwitchBackend 需要后端参数".into());
                    }
                    return Ok(());
                }
                _ => {}
            }
            dispatch(
                &self.controller,
                &self.handle,
                &self.view,
                &self.viewport,
                command,
            );
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
            dispatch(
                &self.controller,
                &self.handle,
                &self.view,
                &self.viewport,
                command,
            );
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
                        let delta = Point3 {
                            x: (x - last[0]) * wpp,
                            y: -(y - last[1]) * wpp,
                            z: 0.0,
                        };
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
            self.send(
                CommandId::Zoom,
                CommandPayload::Points(vec![Point3 {
                    x: factor,
                    y: 0.0,
                    z: 0.0,
                }]),
            );
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
                cad_ui_slint::web::select_backend(cad_ui_slint::web::BackendPreference::WebGl2)
                    .await?
            }
        };

        let controller = Rc::new(RefCell::new(HostController::with_demo_document([
            1280.0, 720.0,
        ])?));
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
            locale: cad_ui_slint::web::stored_locale().unwrap_or_else(|| "zh-CN".to_string()),
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

        let view =
            install_with_preference(handle.clone(), adapter.window(), incoming.clone(), chosen)?;
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
        // Restore any persisted recovery snapshot for the starting document;
        // only a matching fingerprint is applied, otherwise it is reported and
        // left for an explicit restore/discard.
        restore_startup_recovery(&controller, &handle);
        if !cad_ui_slint::web::has_recovery_snapshot() {
            let _ = handle.set_status(format!("就绪（{backend_label}）"));
        }

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

    /// Install an opened drawing into the bridge and shell state.
    fn install_opened(
        name: &str,
        opened: &cad_app::host::OpenedDrawing,
        controller: &Rc<RefCell<HostController>>,
        handle: &UiHandle,
        view: &CadView,
        incoming: &IncomingDocument,
        viewport: &ViewportId,
    ) {
        let drawing = {
            let mut c = controller.borrow_mut();
            let _ = c.fit();
            c.drawing()
        };
        *incoming.borrow_mut() = drawing;
        {
            let c = controller.borrow();
            sync_view_camera(&c, view, viewport);
        }
        view.request_redraw();
        let _ = handle.set_status(format!("已打开 {name}: {}", opened.completeness_label));
        // Fetch the fonts the drawing references (best effort).
        spawn_font_load();
    }

    /// Whether the current document has unsaved annotations.
    ///
    /// The JS host calls this before opening a file so it can prompt for an
    /// explicit decision; there is no silent default.
    pub fn open_needs_decision() -> bool {
        with_runtime(|rt| rt.controller.borrow().unsaved_signal().dirty).unwrap_or(false)
    }

    /// Open a drawing after the host supplied an explicit unsaved-work decision.
    ///
    /// `decision` is one of `save` / `recovery` / `discard` / `cancel`. The host
    /// writes happen here (`save` = download of the prepared sidecar, `recovery`
    /// = localStorage snapshot); a failed write is an error and never replaces
    /// the document. `Cancel` leaves everything untouched.
    pub fn open_document_decided(
        name: &str,
        bytes: Vec<u8>,
        decision: &str,
    ) -> Result<String, String> {
        let decision = parse_decision(decision)
            .ok_or_else(|| format!("未知未保存决策：{decision}（save/recovery/discard/cancel）"))?;
        let (controller, handle, view, incoming, viewport) = with_runtime(|rt| {
            (
                rt.controller.clone(),
                rt.handle.clone(),
                rt.view.clone(),
                rt.incoming.clone(),
                rt.viewport.clone(),
            )
        })
        .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let (center, wpp) = {
            let c = controller.borrow();
            viewport_camera(&c)
        };
        let persistence = WebPersistence;
        let resolution = cad_platform::block_on(resolve_leave(
            &mut controller.borrow_mut(),
            decision,
            center,
            wpp,
            Some(&persistence as &dyn Persistence),
            |ctrl| {
                export_annotations_atomically(ctrl, |json| {
                    download_text("annotations.cadnotes.json", json)
                })
            },
        ));
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(CadError::Cancelled) => {
                let _ = handle.set_status(format!("已取消打开 {name}：当前文档与未保存批注保留"));
                return Err("cancelled".into());
            }
            Err(e) => {
                let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
                return Err(e.to_string());
            }
        };
        let result = controller.borrow_mut().open_bytes_leaving(
            bytes,
            name,
            decision,
            resolution.saved,
            resolution.recovery_persisted,
        );
        match result {
            Ok(opened) => {
                install_opened(
                    name,
                    &opened,
                    &controller,
                    &handle,
                    &view,
                    &incoming,
                    &viewport,
                );
                Ok(format!("已打开 {name}: {}", opened.completeness_label))
            }
            Err(e) => {
                let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
                Err(e.to_string())
            }
        }
    }

    /// Open a drawing selected through the File API (bytes already read in JS).
    ///
    /// This is the conservative entry: a document with unsaved annotations is
    /// refused (the bytes are not applied) and the JS host must ask the user and
    /// call `open_document_decided` with the explicit decision.
    pub fn open_document(name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let (controller, handle, view, incoming, viewport) = with_runtime(|rt| {
            (
                rt.controller.clone(),
                rt.handle.clone(),
                rt.view.clone(),
                rt.incoming.clone(),
                rt.viewport.clone(),
            )
        })
        .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        if controller.borrow().unsaved_signal().dirty {
            let _ = handle.set_status(format!(
                "打开 {name} 需要未保存决策：请选择保存/保留恢复/丢弃/取消"
            ));
            return Err("需要未保存决策".into());
        }
        let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let result = controller.borrow_mut().open_bytes(bytes, name);
        match result {
            Ok(opened) => {
                install_opened(
                    name,
                    &opened,
                    &controller,
                    &handle,
                    &view,
                    &incoming,
                    &viewport,
                );
                Ok(())
            }
            Err(e) => {
                let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
                Err(e.to_string())
            }
        }
    }

    /// Attempt to restore a persisted recovery snapshot for the demo/start
    /// document. Called once at start; a mismatch is reported, never applied.
    fn restore_startup_recovery(controller: &Rc<RefCell<HostController>>, handle: &UiHandle) {
        let persistence = WebPersistence;
        let document = controller.borrow().document_id;
        match cad_platform::block_on(load_recovery(&persistence, document)) {
            Ok(Some(snapshot)) => {
                match controller.borrow_mut().restore_recovery_snapshot(&snapshot) {
                    Ok(count) => {
                        let _ = handle.set_status(format!("已从恢复快照恢复 {count} 条批注"));
                    }
                    Err(e) => {
                        let _ = handle
                            .set_status(format!("检测到恢复快照但未应用（{e}）；可恢复或丢弃"));
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                let _ = handle.set_status(format!("恢复快照损坏，未应用：{e}"));
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
                rt.incoming
                    .borrow()
                    .as_ref()
                    .map(|d| d.entity_count())
                    .unwrap_or(0),
                rt.view.camera().world_per_px,
                controller.status(),
            )
        });
        report.unwrap_or_else(|| "host not started".to_string())
    }

    /// Export annotation JSON for the JS host to download.
    ///
    /// This is a pure getter: it never marks the document saved. A caller that
    /// downloads the bytes must call [`confirm_annotation_export`] with the
    /// returned revision only after the write is confirmed (audit B07).
    pub fn export_annotations_json() -> Result<(String, u64), String> {
        let controller = with_runtime(|rt| rt.controller.clone())
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let (json, revision) = controller
            .borrow()
            .prepare_annotation_export()
            .map_err(|e| e.to_string())?;
        Ok((json, revision.0))
    }

    /// Confirm that the export at `revision` was durably written.
    pub fn confirm_annotation_export(revision: u64) -> Result<(), String> {
        let (controller, handle) = with_runtime(|rt| (rt.controller.clone(), rt.handle.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        controller
            .borrow_mut()
            .confirm_annotation_export(Revision(revision))
            .map_err(|e| e.to_string())?;
        let _ = handle.set_status("批注已确认保存".to_string());
        Ok(())
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

    /// Restore annotations from the pending recovery snapshot (audit B06).
    ///
    /// The stored bytes are a [`cad_app::RecoverySnapshot`]; it is decoded and
    /// applied through the same strict single-transaction path a sidecar import
    /// uses. Corrupt bytes or an identity mismatch are errors and the snapshot
    /// is kept (never cleared), so the recovery copy is not lost to a failed
    /// restore.
    pub fn restore_pending_recovery_snapshot() -> Result<usize, String> {
        let (controller, handle) = with_runtime(|rt| (rt.controller.clone(), rt.handle.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
        let persistence = WebPersistence;
        let document = controller.borrow().document_id;
        let snapshot = cad_platform::block_on(load_recovery(&persistence, document))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "无恢复快照".to_string())?;
        let count = controller
            .borrow_mut()
            .restore_recovery_snapshot(&snapshot)
            .map_err(|e| e.to_string())?;
        cad_ui_slint::web::clear_recovery_snapshot();
        let _ = handle.set_status(format!("已从恢复快照恢复 {count} 条批注"));
        Ok(count)
    }

    /// Explicitly discard the pending recovery snapshot.
    pub fn drop_pending_recovery_snapshot() {
        cad_ui_slint::web::clear_recovery_snapshot();
    }

    /// Decode the pending recovery snapshot for diagnostics (never clears it).
    ///
    /// Returns the encoded snapshot as stored, or `None`. A stored but
    /// undecodable value is surfaced separately so a corrupt copy is visible.
    pub fn pending_recovery_snapshot() -> Option<String> {
        cad_ui_slint::web::peek_recovery_snapshot()
    }

    /// Whether the pending recovery snapshot is a valid encoded snapshot.
    pub fn pending_recovery_is_valid() -> bool {
        cad_ui_slint::web::peek_recovery_snapshot()
            .map(|text| parse_recovery(text.as_bytes()).is_ok())
            .unwrap_or(false)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{
    confirm_annotation_export, export_annotations_json, font_report, import_annotations_json,
    open_document, open_document_decided, open_needs_decision, pending_recovery_is_valid,
    pending_recovery_snapshot, renderer_report, start,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Entry point for `web/main.js`; resolves once the Slint/wgpu session is up.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web() -> Result<(), JsValue> {
    start().await.map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Open a drawing from bytes read by the JS File API host.
///
/// Refuses to replace a document with unsaved annotations; the JS host must
/// call `open_needs_decision`, prompt, then `open_document_bytes_decided`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes(name: String, bytes: Vec<u8>) -> Result<(), JsValue> {
    open_document(&name, bytes).map_err(|e| JsValue::from_str(&e))
}

/// Whether the current document has unsaved annotations that need a decision.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_requires_decision() -> bool {
    open_needs_decision()
}

/// Open a drawing after the JS host supplied an explicit unsaved-work decision.
///
/// `decision` is `save`, `recovery`, `discard` or `cancel`. Returns a status
/// string; `cancel` and a failed write are errors and never replace the
/// document.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes_decided(
    name: String,
    bytes: Vec<u8>,
    decision: String,
) -> Result<String, JsValue> {
    open_document_decided(&name, bytes, &decision).map_err(|e| JsValue::from_str(&e))
}

/// Serializable renderer/state report for the diagnostics panel and tests.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn renderer_state_report() -> String {
    renderer_report()
}

/// Load the fonts the current drawing references; resolves to a summary.
///
/// Exposed so the JS host (and headless verification) can await the real
/// catalog fetch, font fetches and registration instead of guessing.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn load_web_fonts() -> Result<String, JsValue> {
    browser::load_current_fonts()
        .await
        .map(|report| report.summary())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The last font-loading report (catalog/plan/registered/failed).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn font_load_report() -> String {
    font_report()
}

/// Annotation JSON export (caller downloads the returned text). The revision
/// must be passed back to `annotation_confirm_export` after a successful write.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_export_json() -> Result<JsValue, JsValue> {
    let (json, revision) = export_annotations_json().map_err(|e| JsValue::from_str(&e))?;
    let out = js_sys::Object::new();
    js_sys::Reflect::set(&out, &"json".into(), &JsValue::from_str(&json))
        .map_err(|_| JsValue::from_str("failed to build export object"))?;
    js_sys::Reflect::set(
        &out,
        &"revision".into(),
        &JsValue::from_f64(revision as f64),
    )
    .map_err(|_| JsValue::from_str("failed to build export object"))?;
    Ok(out.into())
}

/// Confirm that the export at `revision` was durably written.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_confirm_export(revision: f64) -> Result<(), JsValue> {
    confirm_annotation_export(revision as u64).map_err(|e| JsValue::from_str(&e))
}

/// Annotation JSON import from user-chosen text.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_import_json(text: &str) -> Result<usize, JsValue> {
    import_annotations_json(text).map_err(|e| JsValue::from_str(&e))
}

/// Whether a backend-switch recovery snapshot is waiting (audit B06).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn has_recovery_snapshot() -> bool {
    cad_ui_slint::web::has_recovery_snapshot()
}

/// Restore the pending recovery snapshot (returns the annotation count).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn restore_recovery_snapshot() -> Result<usize, JsValue> {
    browser::restore_pending_recovery_snapshot().map_err(|e| JsValue::from_str(&e))
}

/// Discard the pending recovery snapshot explicitly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn discard_recovery_snapshot() {
    browser::drop_pending_recovery_snapshot()
}

/// Switch the UI language at runtime and persist the choice (N01 host sync).
///
/// Returns the resolved stable tag (`zh-CN` or `en`). The switch only re-applies
/// catalog-driven chrome; it never rebuilds the document, camera, annotations or
/// undo history. The choice is stored under the shared
/// `cad_ui_slint::web::LOCALE_STORAGE_KEY` so `start()` restores it next load.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn web_set_locale(tag: &str) -> Result<String, JsValue> {
    let handle =
        browser::current_handle().ok_or_else(|| JsValue::from_str("browser host not started"))?;
    let resolution = handle
        .set_locale(tag)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    cad_ui_slint::web::store_locale(resolution.locale.tag())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(resolution.locale.tag().to_string())
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
