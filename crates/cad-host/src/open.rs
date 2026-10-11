//! The open pipeline: picker/read → begin_async_open (or sync) → poll → install.
use super::*;
use cad_app::AsyncOpenPoll;
use cad_platform::{FileEvent, FilePurpose};

impl HostCore {
    /// Arm the supersession guard for a new open attempt.
    ///
    /// Captures the current drawing so an attempt whose document was replaced
    /// (by New or another open) is discarded rather than installed. A fresh
    /// attempt also clears a stale cancel flag so it cannot abort this read.
    pub(crate) fn begin_open_attempt(&self) {
        *self.open_guard.borrow_mut() = self.controller.borrow().drawing();
        self.open_abort.set(false);
        *self.open_target.borrow_mut() = None;
    }

    /// Drain every platform file event into the pending open/save slots.
    pub(crate) fn drain_files(&self) -> CadResult<()> {
        for event in self.files.poll() {
            match event {
                FileEvent::Opened { target, bytes } => {
                    self.open_pending.set(false);
                    self.deliver_open(target, bytes)?;
                }
                FileEvent::SaveTarget { purpose, target } => {
                    self.save_pending.set(false);
                    *self.pending_save.borrow_mut() = Some(PendingSave {
                        purpose,
                        result: Ok(target),
                    });
                }
                FileEvent::Cancelled { purpose } => {
                    self.on_file_failure(purpose, FileAbort::Cancelled);
                }
                FileEvent::Failed { purpose, reason } => {
                    self.on_file_failure(purpose, FileAbort::Failed(reason));
                }
            }
        }
        Ok(())
    }

    /// Record a cancel/failure for an open or a save interaction.
    ///
    /// `abort` carries the already-localized reason verbatim; the core never
    /// re-wraps it (which would double-format the message).
    fn on_file_failure(&self, purpose: FilePurpose, abort: FileAbort) {
        match purpose {
            FilePurpose::Open => {
                self.open_pending.set(false);
                match abort {
                    FileAbort::Cancelled => self.open_cancelled.set(true),
                    FileAbort::Failed(reason) => *self.open_failure.borrow_mut() = Some(reason),
                }
            }
            FilePurpose::Save(purpose) => {
                self.save_pending.set(false);
                *self.pending_save.borrow_mut() = Some(PendingSave {
                    purpose,
                    result: Err(abort),
                });
            }
        }
    }

    /// Install delivered bytes: synchronously (no worker) or via the async import.
    ///
    /// Discards a cancelled or superseded delivery, arms the corruption guard
    /// against the delivered `target` only on a successful install, and never
    /// reports a failed read as success.
    fn deliver_open(&self, target: FileTarget, bytes: Arc<[u8]>) -> CadResult<()> {
        // A cancelled read (CancelLoading arrived first) is discarded, never
        // installed. Clear the flag so a later open is not aborted by it.
        if self.open_abort.replace(false) {
            self.presenting_open.set(false);
            return Ok(());
        }
        // A superseded attempt: the document changed since the attempt began.
        let superseded = {
            let guard = self.open_guard.borrow();
            match guard.as_ref() {
                Some(guard) => !self
                    .controller
                    .borrow()
                    .drawing()
                    .is_some_and(|db| Arc::ptr_eq(&db, guard)),
                None => false,
            }
        };
        if superseded {
            self.presenting_open.set(false);
            return Ok(());
        }
        if self.capabilities.import_worker {
            self.presenting_open.set(true);
            // Retain the real target until the import publishes, so the source
            // guard arms against the platform's own handle (not a re-derived path).
            *self.open_target.borrow_mut() = Some(target.clone());
            self.controller
                .borrow_mut()
                .begin_async_open(bytes, &target.display_name);
            return Ok(());
        }
        // Synchronous (headless/CLI-style) install.
        self.controller
            .borrow_mut()
            .open_bytes(bytes, &target.display_name)?;
        // Arm the corruption guard only now that the open succeeded, before
        // `install_images` reads the directory for sibling raster resources.
        self.files.set_source(Some(&target));
        *self.drawing_path.borrow_mut() = Some(PathBuf::from(&target.opaque));
        self.controller.borrow_mut().fit()?;
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.cancel_draw_capture()?;
        }
        if let Some(view) = self.view.borrow().as_ref() {
            self.install_fonts(view)?;
            self.install_images(view)?;
        }
        Ok(())
    }

    /// User-initiated open: run the chooser (or the headless configured drawing).
    pub(crate) fn request_open(&self) -> CadResult<()> {
        if !self.options.headless && self.loading() {
            return Ok(());
        }
        self.begin_open_attempt();
        self.open_pending.set(true);
        if let Err(error) = self.files.begin_open() {
            self.open_pending.set(false);
            return Err(error);
        }
        // A synchronous platform (headless) completes within the gesture.
        self.drain_files()?;
        if !self.options.headless {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.set_status(self.message("linux.picker_waiting", &[]))?;
            }
        }
        Ok(())
    }

    /// Replace the document with a new blank drawing (host-owned NewDrawing).
    ///
    /// The shared controller installs the blank database and rebuilds the
    /// session; the caller's `execute` then pushes the empty state. UI-side draw
    /// capture is cancelled so no stale preview survives the swap. Starting New
    /// while an open is in flight is refused explicitly rather than raced.
    pub(crate) fn request_new(&self) -> CadResult<()> {
        if !self.capabilities.new {
            return Err(CadError::Unsupported(
                self.message("command.unsupported", &[("command", "NEW")]),
            ));
        }
        if self.loading() {
            return Err(CadError::Unsupported(self.message("linux.busy", &[])));
        }
        self.controller.borrow_mut().new_blank_document()?;
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.cancel_draw_capture()?;
        }
        Ok(())
    }

    /// One open tick: drain events, advance the import, grade cancel/failure.
    pub(crate) fn poll_open(&self) -> CadResult<()> {
        self.drain_files()?;
        self.poll_loading()?;
        if self.open_cancelled.replace(false) {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.set_status(self.message("linux.picker_cancelled", &[]))?;
            }
        } else if let Some(reason) = self.open_failure.borrow_mut().take() {
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.set_status(self.message("linux.failed", &[("error", &reason)]))?;
            }
        }
        Ok(())
    }

    /// Advance the asynchronous import and refresh availability/presentation.
    ///
    /// Always sets an explicit status and never returns an error the timer would
    /// turn into a generic failure.
    pub(crate) fn poll_loading(&self) -> CadResult<()> {
        let poll = self.controller.borrow_mut().poll_async_open();
        let opened = matches!(poll, AsyncOpenPoll::Opened { .. });
        if matches!(
            poll,
            AsyncOpenPoll::Failed { .. } | AsyncOpenPoll::Cancelled { .. }
        ) {
            self.presenting_open.set(false);
            // Drop a target that will never publish so it cannot arm a later guard.
            *self.open_target.borrow_mut() = None;
        }
        if opened {
            self.controller.borrow_mut().fit()?;
            if let Some(handle) = self.handle.borrow().as_ref() {
                handle.cancel_draw_capture()?;
            }
            // Arm the corruption guard only now that the open succeeded, against
            // the real delivered target (its opaque handle, not a path derived
            // from the display name).
            if let Some(target) = self.open_target.borrow_mut().take() {
                self.files.set_source(Some(&target));
                *self.drawing_path.borrow_mut() = Some(PathBuf::from(&target.opaque));
            }
            if let Some(view) = self.view.borrow().as_ref() {
                self.install_fonts(view)?;
                self.install_images(view)?;
            }
            self.push()?;
        }
        let snapshot = self.import_snapshot();
        if let Some(handle) = self.handle.borrow().as_ref() {
            let messages = self.messages();
            handle.set_open_available(!self.loading())?;
            // Save As availability tracks the same idle condition as open **and**
            // the host capability, so a host without save never advertises it.
            handle.set_save_available(self.capabilities.save && !self.loading())?;
            handle.set_import_state(&cad_ui_slint::ImportProgressUiState::from_snapshot(
                snapshot.as_ref(),
                &messages,
            ))?;
            if let AsyncOpenPoll::Failed { error, .. } = poll {
                handle
                    .set_status(self.message("linux.failed", &[("error", &error_text(&error))]))?;
            } else if opened {
                handle.set_status(self.message("linux.opened", &[]))?;
            }
            if self.presenting_open.get() && !snapshot.as_ref().is_some_and(|s| s.running) {
                if let Some(view) = self.view.borrow().as_ref() {
                    if let Some(error) = view.scene_error() {
                        handle.set_status(self.message("linux.failed", &[("error", &error)]))?;
                        self.presenting_open.set(false);
                    } else if view.current_drawing_presented() {
                        self.presenting_open.set(false);
                        handle.set_status(self.message("linux.opened", &[]))?;
                    } else {
                        handle.set_status(match view.loading_phase() {
                            Some("uploading") => self.message("linux.uploading", &[]),
                            Some("drawing") => {
                                let (done, total) = view.drawing_progress().unwrap_or((0, 0));
                                self.message(
                                    "linux.drawing",
                                    &[("done", &done.to_string()), ("total", &total.to_string())],
                                )
                            }
                            _ => self.message("linux.preparing", &[]),
                        })?;
                    }
                }
            }
        }
        Ok(())
    }
}
