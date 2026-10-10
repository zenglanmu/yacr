//! host module.

use super::*;

impl HostSink {
    fn status(&self, text: impl Into<String>) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.set_status(text.into());
        }
    }

    fn sync_camera(&self) {
        let viewport_id = self.controller.borrow().viewport_id;
        sync_view_camera(&self.view, &self.controller, viewport_id);
    }

    /// Push every derived panel model into the shell (docs/ui.md §3).
    ///
    /// One connector for all panels, so opening a drawing, executing a command
    /// and a canvas tap all refresh the same set of models from one snapshot.
    fn push_state(&self) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            push_panel_state(&self.controller, handle, &self.view);
        }
    }

    /// Open a DWG from the candidate paths. Real SAF integration is not part of
    /// this build; the search is explicitly reported so it is not mistaken for
    /// a file picker (spec §9.1).
    ///
    /// On a worker-capable target the import runs on the background worker
    /// (`begin_async_open`) and this method returns as soon as the job starts;
    /// the Slint poll timer publishes the result and drives the progress panel.
    /// A target without `std::thread` (wasm) keeps the synchronous path. A dirty
    /// document is only replaced after the host supplies an explicit
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
                    // Worker-capable targets use the real asynchronous path; the
                    // synchronous path stays as the documented fallback for
                    // targets without `std::thread` (see `docs/import-async.md`).
                    if worker_available() {
                        match self.open_async(path, bytes.clone()) {
                            Ok(()) => return,
                            Err(e) => {
                                log::warn!("async open unavailable, using sync path: {e}");
                                self.status(format!("后台打开不可用，改用同步打开：{e}"));
                            }
                        }
                    }
                    let opened = self.controller.borrow_mut().open_bytes(bytes, path);
                    match opened {
                        Ok(opened) => {
                            self.finish_open_success(path, &opened);
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

    /// Start the background import and the poll timer for `bytes`.
    ///
    /// Returns an error only when no background worker is available; the core's
    /// `begin_async_open` is otherwise infallible. The controller is not touched
    /// until a poll publishes a result, and the job is guarded by the task stamp
    /// so a superseded/cancelled open can never replace the document.
    fn open_async(&mut self, path: &str, bytes: Arc<[u8]>) -> CadResult<()> {
        if !worker_available() {
            return Err(CadError::Unsupported(
                "no background import worker on this target".into(),
            ));
        }
        self.controller.borrow_mut().begin_async_open(bytes, path);
        self.status(format!("正在后台打开 {path}…"));
        // Show the panel immediately (before the first tick) and let the timer
        // publish progress/terminal.
        if let Some(handle) = self.handle.borrow().as_ref() {
            push_import_state(&self.controller, handle);
        }
        ensure_polling();
        Ok(())
    }

    /// Mirror a synchronously published document into the bridge and panels.
    fn finish_open_success(&mut self, path: &str, opened: &cad_app::host::OpenedDrawing) {
        let drawing = {
            let mut controller = self.controller.borrow_mut();
            let _ = controller.fit();
            controller.drawing()
        };
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.cancel_draw_capture();
        }
        *self.incoming.borrow_mut() = drawing;
        self.sync_camera();
        if let Some(view) = self.view.borrow().as_ref() {
            view.request_redraw();
        }
        self.push_state();
        self.status(format!("已打开 {path}: {}", opened.completeness_label));
        #[cfg(target_os = "android")]
        self.load_fonts_for_current_document();
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
        } // `CancelLoading` (the shell's `cancel-open-requested`) falls through to
          // `execute`, which routes it to `HostController::cancel_async_open`: the
          // token is flipped and nothing is published, so the current document and
          // unsaved annotations are kept. The `push_state` below then reflects the
          // non-cancellable panel state from the retained snapshot.
        let outcome = self.controller.borrow_mut().execute(command);
        match outcome {
            Ok(outcome) => {
                self.sync_camera();
                // Undo/redo and every panel are refreshed from one snapshot;
                // `set_can_undo` alone would leave redo stale (audit U11).
                self.push_state();
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

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command.
///
/// The sink is installed on the adapter so a layout-row click runs through the
/// same command funnel as every other host command (`HostSink::send` →
/// `HostController::execute`), which validates the layout against the drawing and
/// re-syncs the camera and panels. It never touches the database directly, and an
/// unknown layout is refused by the command layer (`docs/layouts.md` §4).
impl cad_ui_slint::LayoutSwitchSink for HostSink {
    fn select(&mut self, space: cad_representation::SpaceSelection) {
        let command = Command {
            schema_version: 1,
            id: CommandId::SwitchSpace,
            document: self.controller.borrow().document_id,
            viewport: self.controller.borrow().viewport_id,
            payload: cad_app::input::space_switch_payload(space),
        };
        let _ = self.send(command);
    }
}
