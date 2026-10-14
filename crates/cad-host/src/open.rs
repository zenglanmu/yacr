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
        self.open_status_graded.set(false);
        *self.open_target.borrow_mut() = None;
        // A fresh attempt starts a new eligibility scope: a previous attempt's
        // sync terminal or async job must not be shown for this one.
        self.open_attempt
            .set(self.open_attempt.get().saturating_add(1));
        self.async_report_attempt.set(None);
        *self.sync_terminal.borrow_mut() = None;
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
        self.full_dirty.set(true);
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
            self.full_dirty.set(true);
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
            self.full_dirty.set(true);
            return Ok(());
        }
        if self.capabilities.import_worker {
            self.presenting_open.set(true);
            self.full_dirty.set(true);
            // Retain the real target until the import publishes, so the source
            // guard arms against the platform's own handle (not a re-derived path).
            *self.open_target.borrow_mut() = Some(target.clone());
            // This attempt now owns the controller's async report: mark it before
            // starting the job so its running/terminal state is eligible.
            self.async_report_attempt.set(Some(self.open_attempt.get()));
            self.controller
                .borrow_mut()
                .begin_async_open(bytes, &target.display_name);
            return Ok(());
        }
        // Synchronous (headless/CLI-style) install.
        let opened = match self
            .controller
            .borrow_mut()
            .open_bytes(bytes, &target.display_name)
        {
            Ok(opened) => opened,
            Err(error) => {
                // A real failure keeps an explicit terminal; never a fake success.
                *self.sync_terminal.borrow_mut() =
                    Some(ImportProgressSnapshot::terminal(ImportTerminal::Failed {
                        error: error.clone(),
                    }));
                self.full_dirty.set(true);
                return Err(error);
            }
        };
        // The authoritative document was replaced: bump the host document epoch
        // and record the real opened terminal before any fallible post-step.
        self.note_document_replaced();
        *self.sync_terminal.borrow_mut() =
            Some(ImportProgressSnapshot::terminal(ImportTerminal::Opened {
                entities: opened.entities,
            }));
        // Arm the corruption guard only now that the open succeeded, before
        // `install_images` reads the directory for sibling raster resources.
        self.files.set_source(Some(&target));
        *self.drawing_path.borrow_mut() = Some(PathBuf::from(&target.opaque));
        self.controller.borrow_mut().fit()?;
        self.controller.borrow_mut().cancel_tool()?;
        // Resource installation is NON-FATAL for presentation (mirroring the
        // asynchronous publish in `poll_loading`): the drawing is still shown,
        // and the failure is reported as a status rather than failing the open.
        let mut resource_error: Option<String> = None;
        if let Err(error) = self.install_fonts() {
            resource_error = Some(error_text(&error));
        }
        if let Err(error) = self.install_images() {
            if resource_error.is_none() {
                resource_error = Some(error_text(&error));
            }
        }
        if let Some(error) = &resource_error {
            self.set_catalog_status("fonts.failed", &[("error", error)])?;
        }
        self.render.sync_drawing(self.controller.borrow().drawing());
        self.full_dirty.set(true);
        Ok(())
    }

    /// Deliver externally supplied bytes as an authoritative document open.
    ///
    /// Reuses the existing open pipeline ([`Self::begin_open_attempt`] +
    /// [`Self::deliver_open`]): the sync/worker choice follows the host
    /// capability exactly, publication records the document epoch immediately,
    /// and a failed parse returns `Err` without any fake success. Queued old
    /// input is allowed (an IO replace makes it stale at execution), but active
    /// work — an executing request, a pump, a clock sample or another sync
    /// window — is refused with the mapped Busy `CadError` and no mutation.
    ///
    /// The whole delivery runs under an RAII sync guard, and it never routes
    /// through the public `command`/`execute` entries (so it cannot self-Busy).
    /// A successful `Ok(())` means the document was delivered; for the worker
    /// capability the import is only *started* — the caller distinguishes
    /// completion via [`Self::loading`]/status, this never claims "opened".
    pub fn deliver_external_open(&self, target: FileTarget, bytes: Arc<[u8]>) -> CadResult<()> {
        if self.request_engine_busy() || self.queue.executing_id().is_some() {
            return Err(Self::config_busy_error());
        }
        let _guard = ActiveGuard::new(&self.sync_active);
        // Arm the supersession/corruption guards against the current document
        // immediately before delivering.
        self.begin_open_attempt();
        // End any actual picker/read still pending (including a graded failure or
        // cancel from a superseded gesture): real bytes are here now. This is not
        // a fake cancel — the fresh supersession guard from `begin_open_attempt`
        // is left intact.
        self.open_pending.set(false);
        *self.open_failure.borrow_mut() = None;
        self.open_cancelled.set(false);
        self.deliver_open(target, bytes)
    }

    /// User-initiated open: run the chooser (or the headless configured drawing).
    pub(crate) fn request_open(&self) -> CadResult<()> {
        // A save chooser is in flight: an open would cross-wire the platform's
        // single file-interaction slot (a SAF save target would be read as an
        // open). Explicit refusal, never a silent no-op.
        if self.save_pending() {
            return Err(CadError::Unsupported(
                "file operation already in progress".into(),
            ));
        }
        if !self.options.headless && self.loading() {
            return Ok(());
        }
        self.begin_open_attempt();
        self.open_pending.set(true);
        if let Err(error) = self.files.begin_open() {
            self.open_pending.set(false);
            return Err(error);
        }
        // A synchronous platform (headless/CLI-style, or the Android sample-path
        // probe) completes within the gesture; a picker/read still running leaves
        // `open_pending` set.
        self.drain_files()?;
        if self.open_pending.get() {
            if !self.options.headless {
                self.set_catalog_status("linux.picker_waiting", &[])?;
            }
        } else {
            // Grade any cancel/failure now, so a synchronous platform's outcome is
            // surfaced within the gesture rather than waiting for a poll tick that
            // may never come.
            self.grade_pending_open()?;
        }
        Ok(())
    }

    /// Grade a pending open cancel/failure into an explicit status.
    ///
    /// Called at the end of a synchronous open gesture ([`Self::request_open`])
    /// and from [`Self::poll_open`], so a platform whose read completes within the
    /// gesture still surfaces its failure/cancel (and marks the outcome graded so
    /// the generic "completed" cannot overwrite it).
    pub(crate) fn grade_pending_open(&self) -> CadResult<()> {
        if self.open_cancelled.replace(false) {
            self.open_status_graded.set(true);
            self.full_dirty.set(true);
            self.set_catalog_status("linux.picker_cancelled", &[])?;
        } else if let Some(reason) = self.open_failure.borrow_mut().take() {
            self.open_status_graded.set(true);
            self.full_dirty.set(true);
            self.set_catalog_status("command.failed", &[("error", &reason)])?;
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
                "NEW is not supported by this host".into(),
            ));
        }
        if self.loading() {
            return Err(CadError::Unsupported(
                "document loading is in progress".into(),
            ));
        }
        self.controller.borrow_mut().new_blank_document()?;
        // New replaces the authoritative document with a fresh blank one: record
        // the epoch before any fallible post-step and drop any stale synchronous
        // open terminal.
        self.note_document_replaced();
        *self.sync_terminal.borrow_mut() = None;
        self.async_report_attempt.set(None);
        self.controller.borrow_mut().cancel_tool()?;
        self.render.sync_drawing(self.controller.borrow().drawing());
        self.full_dirty.set(true);
        Ok(())
    }

    /// One open tick: drain events, advance the import, grade cancel/failure.
    pub(crate) fn poll_open(&self) -> CadResult<()> {
        self.drain_files()?;
        self.poll_loading()?;
        self.grade_pending_open()?;
        Ok(())
    }

    /// Advance the asynchronous import and refresh availability/presentation.
    ///
    /// Always sets an explicit status and never returns an error the timer would
    /// turn into a generic failure.
    pub(crate) fn poll_loading(&self) -> CadResult<()> {
        let poll = self.controller.borrow_mut().poll_async_open();
        let opened = matches!(poll, AsyncOpenPoll::Opened { .. });
        let cancelled = matches!(poll, AsyncOpenPoll::Cancelled { .. });
        // The asynchronous publish already replaced the authoritative document.
        // Record the epoch IMMEDIATELY, before any fallible post-step (fit,
        // cancel, resources): the replacement is real and old requests must go
        // stale even if a later step fails.
        if opened {
            self.note_document_replaced();
        }
        if matches!(
            poll,
            AsyncOpenPoll::Failed { .. } | AsyncOpenPoll::Cancelled { .. }
        ) {
            self.presenting_open.set(false);
            self.full_dirty.set(true);
            // Drop a target that will never publish so it cannot arm a later guard.
            *self.open_target.borrow_mut() = None;
        }
        // Resource installation is NON-FATAL for presentation: the drawing is
        // still shown, and the failure is reported after the opened/presenting
        // grading so it is not erased within this tick.
        let mut resource_error: Option<String> = None;
        if opened {
            self.controller.borrow_mut().fit()?;
            self.controller.borrow_mut().cancel_tool()?;
            // Arm the corruption guard only now that the open succeeded, against
            // the real delivered target (its opaque handle, not a path derived
            // from the display name).
            if let Some(target) = self.open_target.borrow_mut().take() {
                self.files.set_source(Some(&target));
                *self.drawing_path.borrow_mut() = Some(PathBuf::from(&target.opaque));
            }
            if let Err(error) = self.install_fonts() {
                resource_error = Some(error_text(&error));
            }
            if let Err(error) = self.install_images() {
                if resource_error.is_none() {
                    resource_error = Some(error_text(&error));
                }
            }
            self.render.sync_drawing(self.controller.borrow().drawing());
            self.full_dirty.set(true);
        }
        let snapshot = self.import_snapshot();
        if snapshot.is_some() {
            self.full_dirty.set(true);
        }
        if let AsyncOpenPoll::Failed { error, .. } = poll {
            self.set_catalog_status("linux.failed", &[("error", &error_text(&error))])?;
        } else if cancelled {
            self.set_catalog_status("linux.picker_cancelled", &[])?;
        } else if opened {
            self.set_catalog_status("linux.opened", &[])?;
        }
        if self.presenting_open.get() && !snapshot.as_ref().is_some_and(|s| s.running) {
            if let Some(error) = self.render.scene_error() {
                self.set_catalog_status("linux.failed", &[("error", &error)])?;
                self.presenting_open.set(false);
            } else if self.render.preparation_current() {
                self.presenting_open.set(false);
                self.set_catalog_status("linux.opened", &[])?;
            }
        }
        // The resource failure is the final status of this tick.
        if let Some(error) = &resource_error {
            self.set_catalog_status("fonts.failed", &[("error", error)])?;
        }
        Ok(())
    }
}
