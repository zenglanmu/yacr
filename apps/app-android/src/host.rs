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
