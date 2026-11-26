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
            api_prompt: None,
            api_prompt_error: None,
            api_prompt_results: Vec::new(),
            tolerance: TolerancePolicy::default(),
            export_options: ExportOptions::default(),
            dirty_scope: cad_db::ChangeScope::default(),
        };
        controller.new_blank_document()?;
        Ok(controller)
    }

    /// Build a work-mode session with **no** document: exactly what
    /// [`HostController::close_document`] leaves behind, but at construction.
    ///
    /// The single viewport slot is installed (so a later New/Open reuses it with
    /// the same ids the host caches); nothing else is. There is no document,
    /// hence no `untitled` drawing, no extent and no published content. The name
    /// hint is `"untitled"` so the save/plot suggestion has a stem, the vendor
    /// pass is marked pending so the first tick clears any previous pass, and the
    /// camera is the viewport's own construction default: no `fit()` is
    /// fabricated for a drawing that does not exist.
    pub fn with_no_document(logical_size: [f64; 2]) -> CadResult<Self> {
        let document_id = DocumentId(1);
        let viewport_id = ViewportId(1);
        let mut application = Application::new();
        application.workspace.viewports.insert(
            viewport_id,
            Viewport::new(viewport_id, document_id, logical_size),
        );
        Ok(HostController {
            application,
            session: SessionState::new(document_id, AppMode::Work),
            document_id,
            viewport_id,
            // No document was published, so no readiness claim is made.
            last_status: String::new(),
            document_name_hint: "untitled".to_string(),
            last_import_report: None,
            last_vendor_records: Vec::new(),
            // Same invariant as the post-CLOSE state: `Some([])` on the next tick
            // clears any previous vendor pass.
            vendor_records_pending: true,
            import_manager: crate::tasks::ImportManager::new(document_id),
            pending_open_label: None,
            pending_open_guard: None,
            async_open: None,
            temporary_identity: 0,
            api_prompt: None,
            api_prompt_error: None,
            api_prompt_results: Vec::new(),
            tolerance: TolerancePolicy::default(),
            export_options: ExportOptions::default(),
            dirty_scope: cad_db::ChangeScope::default(),
        })
    }

    /// Rebuild session/history/query/overrides after the document content changed.
    ///
    /// The document id and viewport id stay stable (hosts cache them), but all
    /// derived state that referenced the old drawing is dropped: undo/redo can
    /// no longer reach across documents, and stale selections/overrides/tools
    /// cannot point at new objects.
    pub(super) fn reset_for_new_content(&mut self) {
        // A boolean result computed against the old document must never publish
        // into the replacement: cancel and drop the job before the slot changes.
        self.application.abort_solid_boolean();
        // The mesh boolean/repair slot has the same rule (ADR-0061 D7).
        self.application.abort_mesh_job();
        let tool_generation = self.session.tool_generation.saturating_add(1);
        self.application.history.remove(&self.document_id);
        self.application.query = cad_query::QueryService::new();
        // The accumulated dirty set described edits to the **replaced** content;
        // it must never narrow the next save of a different document (Open/New/
        // Close all funnel through here, ADR-0037 D8/D12 Phase 2).
        self.dirty_scope = cad_db::ChangeScope::default();
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

impl HostController {
    /// Fit the active viewport to the current drawing bounds.
    ///
    /// Discards the "framed" flag for open flows: a blank drawing's fit is a
    /// documented no-op. Callers that need to report that use the command path
    /// (`CommandId::FitDrawing`), which carries the `fit.empty` diagnostic.
    ///
    /// Moved here from `host/mod.rs` (physical-line gate) together with
    /// [`Self::fit_opened_document`], so the open-path framing pair stays in one
    /// place. The method is unchanged.
    pub fn fit(&mut self) -> CadResult<()> {
        self.application
            .fit_viewport(&mut self.session, &self.viewport_id)
            .map(|_| ())
    }

    /// Fit a freshly opened document and enter its default observation.
    ///
    /// The plan fit runs first, so every open has a valid, bounded framing (and
    /// an empty document keeps the documented no-op).
    ///
    /// A drawing opened from the 2D CAD connectors (`dwg`/`dxf`) **always** stays
    /// in the plan view. Those formats routinely carry a few 3D bodies (catalog
    /// fixtures, survey solids) inside an otherwise two-dimensional drawing, and
    /// [`Application::document_is_3d`] would otherwise reorient the whole sheet
    /// into an isometric perspective. A user who wants the 3D observation can
    /// still pick a standard view explicitly.
    ///
    /// Every other document (a native `.ymodel`, a BIM/mesh/STEP source) keeps
    /// the previous rule: a 3D model ([`Application::document_is_3d`]) opens in
    /// the isometric standard view, which re-orients and re-frames from the
    /// bounds for a 3D start instead of a top-down plan of its XY footprint.
    ///
    /// The isometric step is applied **directly** rather than through
    /// [`Self::execute`]: it is the open path's default observation, not a user
    /// view command, so it must not record a `ZOOM PREVIOUS` step — otherwise
    /// the first `ZOOM PREVIOUS` after opening would restore a view the user
    /// never saw.
    pub fn fit_opened_document(&mut self) -> CadResult<()> {
        let opened_in_plan = self.opened_from_2d_connector();
        if opened_in_plan {
            // Opening a DWG/DXF is a request for the 2D drawing view. The
            // viewport is reused across documents, so a previous 3D observation
            // would otherwise make the fit below stay 3D (and a front/side eye
            // can even leave the plan camera degenerate). Reset to the same
            // canonical top view a fresh viewport gets, then frame it. An
            // already-plan viewport is left untouched.
            if let Some(viewport) = self
                .application
                .workspace
                .viewports
                .get_mut(&self.viewport_id)
            {
                let already_plan = matches!(viewport.view_mode, crate::ViewMode2d3d::TwoD { .. })
                    && viewport.camera.projection.is_orthographic();
                if !already_plan {
                    let camera = crate::Camera::top_view_2d();
                    viewport.camera = camera;
                    viewport.view_mode = crate::ViewMode2d3d::TwoD { saved: camera };
                    viewport.work_plane = crate::xy_work_plane(0.0);
                }
            }
        }
        self.application
            .fit_viewport(&mut self.session, &self.viewport_id)?;
        if !opened_in_plan && self.application.document_is_3d(self.document_id) {
            let command = Command {
                schema_version: 1,
                id: crate::CommandId::StandardView,
                document: self.document_id,
                viewport: self.viewport_id,
                payload: crate::CommandPayload::StandardView(crate::StandardView::Isometric),
            };
            self.application.apply_standard_view(
                &mut self.session,
                &command,
                crate::StandardView::Isometric,
            )?;
        }
        Ok(())
    }

    /// Whether the active drawing was opened through a two-dimensional CAD
    /// connector (`dwg`/`dxf`), which keeps the open path in the plan view.
    ///
    /// Derived from the document's own import report, never from the file name
    /// extension: a `.ymodel` open and a blank document report `None` and keep
    /// the geometry-based default.
    fn opened_from_2d_connector(&self) -> bool {
        matches!(
            self.last_import_report
                .as_ref()
                .and_then(|report| report.connector),
            Some("dwg" | "dxf")
        )
    }

    /// The provenance of the connector that opened the current document
    /// (ADR-0041 D5), or `None`.
    ///
    /// Derived from the document's own **import report** — the host record the
    /// successful publish wrote and New/Close/`install_snapshot` clear — so the
    /// answer is per-document by construction. It is never inferred from
    /// [`Self::document_name_hint`] (extension sniffing is what this replaces):
    /// a blank drawing, a native `.ymodel` install and a public-spec connector
    /// all report `None` here.
    pub fn opened_source(&self) -> Option<crate::OpenedSource> {
        crate::connector::opened_source(
            self.last_import_report
                .as_ref()
                .and_then(|report| report.connector),
        )
    }
}
