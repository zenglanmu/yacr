//! The single document slot's lifecycle: initial construction, content reset and
//! close.
//!
//! Extracted from `host/mod.rs` (kept under the physical-line gate) but still one
//! `impl HostController`: the blank database, identity, name hint and camera come
//! from the existing [`HostController::new_blank_document`] builder.

use super::*;

impl HostController {
    /// Build a work-mode session that starts on an empty `untitled` drawing.
    ///
    /// The blank database, the `untitled` name hint, the fresh temporary
    /// identity and the default plan-view camera all come from the one existing
    /// builder ([`HostController::new_blank_document`]); this constructor only
    /// installs the single document slot's viewport and then delegates, so there
    /// is no second blank-database construction path.
    pub fn with_blank_document(logical_size: [f64; 2]) -> CadResult<Self> {
        let document_id = DocumentId(1);
        let viewport_id = ViewportId(1);
        let mut application = Application::new();
        application.workspace.viewports.insert(
            viewport_id,
            Viewport::new(viewport_id, document_id, logical_size),
        );
        let mut controller = HostController {
            application,
            session: SessionState::new(document_id, AppMode::Work),
            document_id,
            viewport_id,
            // No demo geometry was installed, so no readiness claim is made.
            last_status: String::new(),
            document_name_hint: "untitled".to_string(),
            last_import_report: None,
            last_vendor_records: Vec::new(),
            vendor_records_pending: false,
            import_manager: crate::tasks::ImportManager::new(document_id),
            pending_open_label: None,
            pending_open_guard: None,
            async_open: None,
            temporary_identity: 0,
            tolerance: TolerancePolicy::default(),
        };
        controller.new_blank_document()?;
        Ok(controller)
    }

    /// Rebuild session/history/query/overrides after the document content changed.
    ///
    /// The document id and viewport id stay stable (hosts cache them), but all
    /// derived state that referenced the old drawing is dropped: undo/redo can
    /// no longer reach across documents, and stale selections/overrides/tools
    /// cannot point at new objects.
    pub(super) fn reset_for_new_content(&mut self) {
        let tool_generation = self.session.tool_generation.saturating_add(1);
        self.application.history.remove(&self.document_id);
        self.application.query = cad_query::QueryService::new();
        self.session = SessionState::new(self.document_id, self.session.mode());
        self.session.generation = self.session.generation.saturating_add(1);
        self.session.tool_generation = tool_generation;
    }

    /// Close the active document, leaving the host with no authoritative drawing.
    ///
    /// An explicit `InvalidInput` when nothing is open — never an empty success.
    /// Any in-flight background import is cancelled and its pending label,
    /// supersession guard and UI snapshot are dropped, so a later
    /// `poll_async_open` cannot resurrect the closed document.
    ///
    /// The document slot and its derived state are removed/rebuilt exactly like
    /// a content replacement (undo/redo, query, selection, overrides and the
    /// session tool cannot reference the closed drawing), but the **viewport
    /// slot is kept** so the next New/Open reuses it. No camera is fabricated:
    /// the viewport keeps its last camera until the next document frames itself.
    pub fn close_document(&mut self) -> CadResult<()> {
        if self.drawing().is_none() {
            return Err(CadError::InvalidInput("document not open".into()));
        }
        // Supersede any open before the slot disappears.
        self.import_manager.cancel();
        self.pending_open_label = None;
        self.pending_open_guard = None;
        self.async_open = None;

        self.application
            .workspace
            .documents
            .remove(&self.document_id);
        // Drops the history entry, replaces the query service and installs a
        // fresh session (same id/mode, bumped generation/tool_generation).
        self.reset_for_new_content();
        // Honest placeholder: the status label is `None` because `drawing()` is
        // `None`; the name hint only keeps the save/plot suggestion working.
        self.document_name_hint = "untitled".to_string();
        self.last_import_report = None;
        self.last_vendor_records = Vec::new();
        // `Some([])` on the next tick clears any previous vendor pass.
        self.vendor_records_pending = true;
        Ok(())
    }
}
