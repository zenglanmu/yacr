//! Shared host wiring: application + session + one document slot.
//!
//! Spec v2.0 §2.1(4), §4.2, §9: every platform host (Android, Web, CLI)
//! assembles the same core objects and drives them through the same
//! `Application::execute` command path. Hosts own platform I/O (files, GPU,
//! lifecycle); this module owns the business path so hosts cannot drift apart.

use std::sync::Arc;

use cad_db::{
    transform_geometry, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};
use cad_domain::*;
use cad_import_acadrust::export::{
    export, ExportFormat, ExportLimits, ExportReport, ExportRequest, ExportSpace,
};
use cad_import_acadrust::{
    AcadrustImporter, ImportLimits, ImportReport, ImportRequest, ImportedDrawing, Importer,
};
use cad_measure::{MeasurementEngine, SnapCandidate, SnapTarget};

use crate::{
    AppMode, Application, Command, CommandOutcome, Document, DrawIntent, DrawToolKind,
    SessionState, Viewport,
};

mod databases;

#[cfg(test)]
mod tests;

pub use databases::{
    blank_database, demo_database, snap_candidates_near, snap_candidates_near_uncached,
};

/// Result of a successful drawing open, for status lines and diagnostics.
#[derive(Debug, Clone)]
pub struct OpenedDrawing {
    pub entities: usize,
    pub completeness_label: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of starting a new blank drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewDrawing {
    /// Entity count of the fresh drawing (zero for the blank database).
    pub entities: usize,
    /// Whether the drawing has measurable bounds. Always `false` for a blank
    /// drawing, so a caller knows a fit is a documented no-op rather than a
    /// real framing.
    pub has_extent: bool,
}

/// One document session shared by a host and its UI.
pub struct HostController {
    pub application: Application,
    pub session: SessionState,
    pub document_id: DocumentId,
    pub viewport_id: ViewportId,
    /// Last human-facing status derived from a command outcome.
    pub last_status: String,
    /// Name hint for the currently open document.
    pub document_name_hint: String,
    /// Report of the most recent successful open, for diagnostics/CLI.
    pub last_import_report: Option<ImportReport>,
    /// The single current background import (spec F01). Empty until a host
    /// starts an asynchronous open; the synchronous path never uses it.
    pub(crate) import_manager: crate::tasks::ImportManager,
    /// Label for the running asynchronous open, applied on publish.
    pub(crate) pending_open_label: Option<String>,
    pub(crate) pending_open_guard: Option<Arc<DrawingDatabase>>,
    /// Last UI-facing snapshot of the asynchronous open (F01).
    ///
    /// This is a *projection* of [`crate::tasks::AsyncOpenPoll`], updated only
    /// by `begin_async_open` / `cancel_async_open` / `poll_async_open`; the pure
    /// getter [`HostController::async_open_snapshot`] just reads it, so reading
    /// the panel state never drains progress or publishes a document.
    pub(crate) async_open: Option<crate::tasks::ImportProgressSnapshot>,
    /// Monotonic counter for a fresh `DocumentIdentity::Temporary` on each new
    /// blank document, so two New operations never share an identity.
    temporary_identity: u128,
}

impl HostController {
    /// Build a work-mode session with the synthetic demo drawing loaded.
    pub fn with_demo_document(logical_size: [f64; 2]) -> CadResult<Self> {
        let document_id = DocumentId(1);
        let viewport_id = ViewportId(1);
        let mut application = Application::new();
        let drawing = Arc::new(demo_database());
        application.workspace.documents.insert(
            document_id,
            Document {
                id: document_id,
                drawing,
                identity: DocumentIdentity::Temporary(0),
                units: UnitContext::drawing_units(),
                resource_keys: Vec::new(),
            },
        );
        application.workspace.viewports.insert(
            viewport_id,
            Viewport::new(viewport_id, document_id, logical_size),
        );
        Ok(HostController {
            application,
            session: SessionState::new(document_id, AppMode::Work),
            document_id,
            viewport_id,
            last_status: "就绪（内置演示几何，非兼容性声明）".to_string(),
            document_name_hint: "yacr-demo".to_string(),
            last_import_report: None,
            import_manager: crate::tasks::ImportManager::new(document_id),
            pending_open_label: None,
            pending_open_guard: None,
            async_open: None,
            temporary_identity: 0,
        })
    }

    /// Start a session-owned draw capture. The UI remains a presenter only.
    pub fn begin_draw_tool(&mut self, kind: DrawToolKind) -> CadResult<()> {
        self.session.begin_draw(kind)
    }

    /// Capture a world point through the active session tool.
    ///
    /// The point is first resolved through [`HostController::resolve_snap`] so
    /// the committed point is the snapped point (ADR 0014 A.2). When there is
    /// no candidate the raw point is used unchanged; a resolution error (for
    /// example a non-finite point) is propagated, never swallowed.
    pub fn capture_tool_point(&mut self, point: Point3) -> CadResult<bool> {
        let point = self.resolve_snap(point)?.map_or(point, |snap| snap.point);
        self.session.capture_point(point)
    }

    /// Move the active tool preview cursor without advancing its generation.
    ///
    /// Like [`HostController::capture_tool_point`], the stored cursor is the
    /// resolved snap point so the preview and the eventual commit share one
    /// truth (ADR 0014 A.2).
    pub fn move_tool_cursor(&mut self, point: Point3) -> CadResult<()> {
        let point = self.resolve_snap(point)?.map_or(point, |snap| snap.point);
        self.session.move_cursor(point)
    }

    /// Cancel an active tool without executing a drawing command.
    pub fn cancel_tool(&mut self) -> CadResult<()> {
        self.session.cancel_tool()
    }

    /// Confirm a captured draw through the ordinary command/transaction path.
    /// The capture is cleared only after execution succeeds.
    pub fn confirm_draw(&mut self) -> CadResult<CommandOutcome> {
        let intent = self.session.draw_intent()?;
        let selection = self
            .session
            .draw_selection_snapshot()
            .ok_or_else(|| CadError::InvalidInput("no drawing tool is active".into()))?
            .to_vec();
        let command = self.command_for_draw_intent(intent, &selection)?;
        let outcome = self.execute(command)?;
        self.session.complete_tool();
        Ok(outcome)
    }

    /// Confirm a captured draw and return the exact command trace. Tool intent
    /// parsing can fail before a command is entered; an executed command failure
    /// is retained in the returned trace and leaves the capture untouched.
    pub fn confirm_draw_traced(&mut self) -> CadResult<crate::TracedCommandOutcome> {
        let intent = self.session.draw_intent()?;
        let selection = self
            .session
            .draw_selection_snapshot()
            .ok_or_else(|| CadError::InvalidInput("no drawing tool is active".into()))?
            .to_vec();
        let command = self.command_for_draw_intent(intent, &selection)?;
        let traced = self.execute_traced(command)?;
        if traced.result.is_ok() {
            self.session.complete_tool();
        }
        Ok(traced)
    }

    fn command_for_draw_intent(
        &self,
        intent: DrawIntent,
        selection: &[SelectionRef],
    ) -> CadResult<Command> {
        let (id, payload) = match intent {
            DrawIntent::Line { start, end } => (
                crate::CommandId::CreateLine,
                crate::CommandPayload::Points(vec![start, end]),
            ),
            DrawIntent::Circle { center, edge } => (
                crate::CommandId::CreateCircle,
                crate::CommandPayload::Points(vec![center, edge]),
            ),
            DrawIntent::Arc { points } => (
                crate::CommandId::CreateArc,
                crate::CommandPayload::Points(points.to_vec()),
            ),
            DrawIntent::Move { delta } => (
                crate::CommandId::MoveEntities,
                crate::CommandPayload::Move {
                    refs: selection.to_vec(),
                    delta,
                },
            ),
            DrawIntent::Copy { delta } => (
                crate::CommandId::CopyEntities,
                crate::CommandPayload::Copy {
                    refs: selection.to_vec(),
                    delta,
                },
            ),
            DrawIntent::Trim {
                target_pick,
                boundary_pick,
            } => (
                crate::CommandId::TrimEntity,
                crate::CommandPayload::Trim {
                    target: self.resolve_draw_pick(target_pick)?,
                    boundary: vec![self.resolve_draw_pick(boundary_pick)?],
                    pick_point: target_pick,
                },
            ),
            DrawIntent::Extend {
                target_pick,
                boundary_pick,
            } => (
                crate::CommandId::ExtendEntity,
                crate::CommandPayload::Extend {
                    target: self.resolve_draw_pick(target_pick)?,
                    boundary: vec![self.resolve_draw_pick(boundary_pick)?],
                    pick_point: target_pick,
                },
            ),
            DrawIntent::Rotate { base, angle } => (
                crate::CommandId::RotateEntities,
                crate::CommandPayload::Rotate {
                    refs: selection.to_vec(),
                    base,
                    angle,
                },
            ),
            DrawIntent::Scale { base, factor } => (
                crate::CommandId::ScaleEntities,
                crate::CommandPayload::Scale {
                    refs: selection.to_vec(),
                    base,
                    factor,
                },
            ),
            DrawIntent::Mirror { p1, p2 } => (
                crate::CommandId::MirrorEntities,
                crate::CommandPayload::Mirror {
                    refs: selection.to_vec(),
                    axis_start: p1,
                    axis_end: p2,
                },
            ),
            DrawIntent::Offset { side_point } => (
                crate::CommandId::OffsetEntity,
                crate::CommandPayload::Offset {
                    refs: selection.to_vec(),
                    side_point,
                },
            ),
        };
        Ok(Command {
            schema_version: 1,
            id,
            document: self.document_id,
            viewport: self.viewport_id,
            payload,
        })
    }

    fn resolve_draw_pick(&self, world: Point3) -> CadResult<SelectionRef> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let viewport = self
            .application
            .workspace
            .viewports
            .get(&self.viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let logical = viewport
            .camera
            .world_to_screen(world, viewport.logical_size)
            .ok_or_else(|| CadError::InvalidInput("draw pick is outside the viewport".into()))?;
        let report = crate::pick_at_screen(
            &document.drawing,
            self.document_id,
            &viewport.camera,
            logical,
            viewport.logical_size,
            &TolerancePolicy::default(),
            cad_spatial::BackFacePolicy::DoubleSided,
        )?;
        report.hit.map(|hit| hit.source).ok_or_else(|| {
            CadError::InvalidInput("draw pick did not hit supported geometry".into())
        })
    }

    /// The current immutable drawing, for the render/composition pipeline.
    pub fn drawing(&self) -> Option<Arc<DrawingDatabase>> {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|d| d.drawing.clone())
    }

    /// Reverse-map the active document to lossy DXF bytes plus the export report.
    ///
    /// Host-owned like [`HostController::open_bytes`]: the platform host chooses
    /// the path and writes the bytes. This is a **Save As**, never a round-trip
    /// save — the report records every conversion and drop. The database has no
    /// document in the slot only before the first open; that is an explicit
    /// `InvalidInput`, never an empty success.
    pub fn export_dxf(&self) -> CadResult<(Vec<u8>, ExportReport)> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let request = ExportRequest {
            database: document.drawing.as_ref(),
            document_id: self.document_id,
            space: ExportSpace::Model,
            format: ExportFormat::DxfText,
            limits: ExportLimits::default(),
            units: Some(document.units.clone()),
        };
        export(&request)
    }

    /// Import a DWG byte stream through the single importer boundary.
    ///
    /// Only a successful import replaces the document. On success the session,
    /// history, selection, layer overrides and active tool are rebuilt for the
    /// new content so the previous drawing's undo records and references can
    /// never act on it (audit B05). A failure leaves the current document and
    /// session untouched.
    pub fn open_bytes(&mut self, bytes: Arc<[u8]>, label: &str) -> CadResult<OpenedDrawing> {
        let request = ImportRequest {
            document: self.document_id,
            database: DatabaseId(1),
            bytes,
            limits: ImportLimits::default(),
            generation: self.session.generation,
        };
        let importer = AcadrustImporter::new();
        let imported = importer.import(&request, &|| false)?;
        self.pending_open_label = Some(label.to_string());
        self.publish_imported(
            imported,
            &TaskStamp::new(self.document_id, self.session.generation),
        )
    }

    /// Publish an imported drawing into the current document slot.
    ///
    /// The whole imported database is built before this point, so publication is
    /// atomic: a cancelled or failed import never reaches here and can never
    /// partially mutate the session. `stamp` is the identity the caller asserts;
    /// this method does not re-derive it, so a superseded job that somehow
    /// called it would still be rejected by the stamp guard at the call site.
    pub(crate) fn publish_imported(
        &mut self,
        imported: ImportedDrawing,
        _stamp: &TaskStamp,
    ) -> CadResult<OpenedDrawing> {
        let opened = OpenedDrawing {
            entities: imported.database.entity_count(),
            completeness_label: imported.report.completeness_label(),
            diagnostics: imported.report.diagnostics.clone(),
        };
        let label = self
            .pending_open_label
            .take()
            .unwrap_or_else(|| self.document_name_hint.clone());
        let document = Document {
            id: self.document_id,
            drawing: Arc::new(imported.database),
            identity: imported.report.identity.clone(),
            units: imported.units,
            resource_keys: Vec::new(),
        };
        self.application
            .workspace
            .documents
            .insert(self.document_id, document);

        // Rebuild everything bound to the previous content identity.
        self.reset_for_new_content();
        self.document_name_hint = label.clone();
        self.last_import_report = Some(imported.report.clone());
        self.last_status = format!("已打开 {label}: {}", opened.completeness_label);
        Ok(opened)
    }

    /// Rebuild session/history/query/overrides after the document content changed.
    ///
    /// The document id and viewport id stay stable (hosts cache them), but all
    /// derived state that referenced the old drawing is dropped: undo/redo can
    /// no longer reach across documents, and stale selections/overrides/tools
    /// cannot point at new objects.
    fn reset_for_new_content(&mut self) {
        let tool_generation = self.session.tool_generation.saturating_add(1);
        self.application.history.remove(&self.document_id);
        self.application.query = cad_query::QueryService::new();
        self.session = SessionState::new(self.document_id, self.session.mode());
        self.session.generation = self.session.generation.saturating_add(1);
        self.session.tool_generation = tool_generation;
    }

    /// Replace the active document with a new, empty drawing.
    ///
    /// Builds the blank database (only layer `"0"`) and installs it in the
    /// single document slot with a fresh temporary identity, then rebuilds every
    /// derived state through [`HostController::reset_for_new_content`] so no undo
    /// record, selection, layer override or active tool can reference the
    /// previous drawing. The previous diagnostics report is dropped too. Hosts
    /// must cancel any UI-side draw capture themselves (the session reset clears
    /// the session tool and selection here).
    ///
    /// **New supersedes an in-flight open**: any running/retained background
    /// import is cancelled and its pending label, guard and UI snapshot are
    /// cleared, so a later `poll_async_open` cannot clobber the fresh drawing.
    ///
    /// The active viewport camera is reset to the deterministic fresh-document
    /// plan view (origin-anchored, minimum orthographic scale) — a default, not a
    /// fabricated fit. The returned [`NewDrawing::has_extent`] is always `false`.
    pub fn new_blank_document(&mut self) -> CadResult<NewDrawing> {
        // Supersede any open before the slot changes.
        self.import_manager.cancel();
        self.pending_open_label = None;
        self.pending_open_guard = None;
        self.async_open = None;

        let drawing = blank_database();
        let entities = drawing.entity_count();
        self.temporary_identity = self.temporary_identity.saturating_add(1);
        let document = Document {
            id: self.document_id,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Temporary(self.temporary_identity),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        };
        self.application
            .workspace
            .documents
            .insert(self.document_id, document);
        self.reset_for_new_content();
        self.document_name_hint = "untitled".to_string();
        self.last_import_report = None;
        // Fresh-document default plan view anchored at the origin. This is NOT a
        // fabricated fit: the blank drawing has no extent, so the previous
        // drawing's framing must not linger.
        if let Some(viewport) = self
            .application
            .workspace
            .viewports
            .get_mut(&self.viewport_id)
        {
            let camera = crate::Camera {
                eye: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1000.0,
                },
                target: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                up: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                projection: crate::Projection::Orthographic {
                    scale: crate::camera::MIN_ORTHO_SCALE,
                },
            };
            viewport.camera = camera;
            viewport.view_mode = crate::ViewMode2d3d::TwoD { saved: camera };
            viewport.work_plane = crate::xy_work_plane(0.0);
        }
        Ok(NewDrawing {
            entities,
            has_extent: false,
        })
    }

    /// Run one command through the single application entry point.
    ///
    /// `CancelLoading` is intercepted here because file open/cancel is
    /// host-owned I/O: the application layer does not run it (it returns
    /// `Unsupported`). Routing it to [`HostController::cancel_async_open`] lets
    /// the UI's cancel affordance work through the ordinary command path
    /// without changing the application's contract.
    pub fn execute(&mut self, command: Command) -> CadResult<CommandOutcome> {
        if command.id == crate::CommandId::CancelLoading {
            self.cancel_async_open();
            return Ok(CommandOutcome::none());
        }
        let outcome = self.application.execute(&mut self.session, command)?;
        if let Some(diagnostic) = outcome.diagnostics.first() {
            self.last_status = diagnostic.message.clone();
        }
        Ok(outcome)
    }

    /// Execute once and retain a raw trace for the single command actually
    /// entered. The legacy owned-outcome method above remains unchanged.
    pub fn execute_traced(&mut self, command: Command) -> CadResult<crate::TracedCommandOutcome> {
        let command_id = command.id;
        if command_id == crate::CommandId::CancelLoading {
            self.cancel_async_open();
            return Ok(crate::api::traced_command(
                command_id,
                Ok(Arc::new(CommandOutcome::none())),
            ));
        }
        let traced = self
            .application
            .execute_traced(&mut self.session, command)?;
        if let Ok(outcome) = &traced.result {
            if let Some(diagnostic) = outcome.diagnostics.first() {
                self.last_status = diagnostic.message.clone();
            }
        }
        Ok(traced)
    }

    /// Pure getter: the last UI-facing snapshot of the asynchronous open (F01).
    ///
    /// Returns `None` when the controller is genuinely idle (no running job and
    /// no retained terminal state). Reading it dispatches no command, drains no
    /// progress and never publishes a document; hosts call it after
    /// `poll_async_open` to push a progress panel.
    pub fn async_open_snapshot(&self) -> Option<crate::tasks::ImportProgressSnapshot> {
        self.async_open.clone()
    }

    /// Fit the active viewport to the current drawing bounds.
    ///
    /// Discards the "framed" flag for open flows: a blank drawing's fit is a
    /// documented no-op. Callers that need to report that use the command path
    /// (`CommandId::FitDrawing`), which carries the `fit.empty` diagnostic.
    pub fn fit(&mut self) -> CadResult<()> {
        self.application
            .fit_viewport(&mut self.session, &self.viewport_id)
            .map(|_| ())
    }

    /// Set the session's active drawing layer through the command path.
    ///
    /// Session state only: no transaction, no history. The create commands
    /// validate the layer against the drawing before writing.
    pub fn set_active_layer(&mut self, layer: LayerId) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetActiveLayer,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::ActiveLayer(layer),
        })?;
        Ok(())
    }

    /// Draw a LINE through two world points in one transaction (spec F-EDIT).
    pub fn create_line(&mut self, start: Point3, end: Point3) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateLine,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![start, end]),
        })
    }

    /// Draw a CIRCLE from a centre and an edge point in one transaction.
    pub fn create_circle(&mut self, center: Point3, edge: Point3) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateCircle,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![center, edge]),
        })
    }

    /// Move a set of selected refs by a world delta in one transaction.
    pub fn move_entities(
        &mut self,
        refs: Vec<SelectionRef>,
        delta: Point3,
    ) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::MoveEntities,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Move { refs, delta },
        })
    }

    /// Trim one target against a boundary in one transaction (spec §3 subset).
    pub fn trim_entity(
        &mut self,
        target: SelectionRef,
        boundary: Vec<SelectionRef>,
        pick_point: Point3,
    ) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::TrimEntity,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Trim {
                target,
                boundary,
                pick_point,
            },
        })
    }

    pub fn status(&self) -> &str {
        &self.last_status
    }

    /// Independent undo/redo availability for the shell to bind to (audit U11).
    ///
    /// This is a pure getter: refreshing button state never dispatches a
    /// command.
    pub fn history_availability(&self) -> crate::HistoryAvailability {
        self.application.history_availability(&self.document_id)
    }

    /// Active measurement preview for the tool panel, if a tool is running.
    ///
    /// Pure getter: it reads `SessionState::measurement_preview()` and never
    /// dispatches a command or advances the tool (audit U04).
    pub fn measurement_preview(&self) -> Option<crate::MeasurementPreview> {
        self.session.measurement_preview()
    }

    /// The live world cursor of the active tool, if any tool is capturing.
    ///
    /// The cursor lives in the tool preview: [`crate::MeasurementPreview::cursor`]
    /// for a measurement tool and [`crate::DrawPreview::cursor`] for a drawing
    /// tool. Both are read here (drawing tools were previously omitted, which
    /// left LINE/CIRCLE/ARC without snap hints). Reading it dispatches no
    /// command.
    fn active_tool_cursor(&self) -> Option<Point3> {
        self.session
            .measurement_preview()
            .and_then(|preview| preview.cursor)
            .or_else(|| {
                self.session
                    .draw_preview()
                    .and_then(|preview| preview.cursor)
            })
    }

    /// The session's object-snap switch (session state only).
    pub fn snap_enabled(&self) -> bool {
        self.session.snap_enabled()
    }

    /// Enable or disable object snapping for this session.
    ///
    /// Session state only: no database write, no history entry, and no change
    /// to the persisted `view.overlays.snapHints` preference (ADR 0014 A.4).
    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.session.set_snap_enabled(enabled);
    }

    /// Flip object snapping and return the new state (F3 binding).
    pub fn toggle_snap(&mut self) -> bool {
        self.session.toggle_snap()
    }

    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// See [`crate::SessionState::hover_preview`] for the deliberate default
    /// (`false`, unlike AutoCAD's `SELECTIONPREVIEW` initial `3`).
    pub fn hover_preview_enabled(&self) -> bool {
        self.session.hover_preview()
    }

    /// Flip the idle hover entity preview and return the new state.
    ///
    /// Session/display state only: no database write, no history entry and no
    /// ChangeSet. Object-snap markers are unaffected.
    pub fn toggle_hover_preview(&mut self) -> bool {
        self.session.toggle_hover_preview()
    }

    /// Resolve the single object-snap candidate for a raw world point.
    ///
    /// This is the one acquisition entry point used by both tool boundaries
    /// ([`HostController::move_tool_cursor`] and
    /// [`HostController::capture_tool_point`], ADR 0014 A.1–A.3). It reads the
    /// exact same inputs as [`HostController::snap_hints_near_cursor`]: the
    /// viewport's `world_per_px`, the session's active space, the drawing's
    /// [`snap_candidates_near`] set and the default [`TolerancePolicy`] inside
    /// the measurement engine.
    ///
    /// The transform-baked snap targets are cached per scene identity, so a
    /// pointer move does not re-expand the drawing. The full candidate set this
    /// call resolves is recorded as the last move's query; the hint overlay
    /// reuses it for the stored cursor instead of walking the drawing again.
    /// The snapped point itself is stored by the caller, so it stays the single
    /// truth a commit reads.
    ///
    /// `Ok(None)` means only "nothing to snap to": snapping is disabled, the
    /// document or viewport is absent, or no candidate is near the point. A
    /// non-finite point or an unusable `world_per_px` is an explicit
    /// [`CadError::InvalidInput`], never a silent raw-point success.
    pub fn resolve_snap(&self, raw: Point3) -> CadResult<Option<SnapCandidate>> {
        if !raw.x.is_finite() || !raw.y.is_finite() || !raw.z.is_finite() {
            return Err(CadError::InvalidInput("snap cursor must be finite".into()));
        }
        if !self.session.snap_enabled() {
            return Ok(None);
        }
        let Some(viewport) = self.application.workspace.viewports.get(&self.viewport_id) else {
            return Ok(None);
        };
        let world_per_px = viewport.world_per_px();
        if !world_per_px.is_finite() || world_per_px <= 0.0 {
            return Err(CadError::InvalidInput(
                "snap world_per_px must be finite and positive".into(),
            ));
        }
        let Some(document) = self.application.workspace.documents.get(&self.document_id) else {
            return Ok(None);
        };
        let candidates = snap_candidates_near(
            document.drawing.as_ref(),
            self.document_id,
            raw,
            world_per_px,
            self.session.active_space.clone(),
        )?;
        Ok(candidates.into_iter().next())
    }

    /// The world units per logical pixel of the active viewport camera.
    ///
    /// Returns `None` when the viewport is missing or the value is not finite
    /// and positive, so the caller skips snapping rather than feeding a
    /// fabricated tolerance. `Viewport::world_per_px` already floors an
    /// orthographic scale, so this only fails for a genuinely absent viewport.
    fn viewport_world_per_px(&self) -> Option<f64> {
        let world_per_px = self
            .application
            .workspace
            .viewports
            .get(&self.viewport_id)?
            .world_per_px();
        (world_per_px.is_finite() && world_per_px > 0.0).then_some(world_per_px)
    }

    /// Real object-snap candidates under the active tool's world cursor.
    ///
    /// Pure getter (audit U04): it reads the tool preview (measurement **or**
    /// drawing cursor), the drawing and the viewport, resolves the snap
    /// candidates through the shared [`snap_candidates_near`] helper, and never
    /// dispatches a command or writes a database. When snapping is disabled the
    /// explicit empty vector is returned, so the host clears the snap-hint
    /// overlay rather than showing markers that a click would not honour. With
    /// no live cursor (idle, or a tool that has not moved yet) it also returns
    /// the explicit empty vector. A missing viewport likewise yields no hints —
    /// nothing is fabricated.
    ///
    /// The resolved candidate set is the **same one** the pointer move resolved
    /// and stored: [`HostController::resolve_snap`] records it, and this getter
    /// reuses it when the stored cursor (the snapped point) and the scene
    /// identity are unchanged, instead of walking the drawing a second time.
    /// The commit still reads the stored tool point, not this overlay set.
    ///
    /// The renderer independently gates the overlay on `view.overlays.snapHints`,
    /// so a host may call this unconditionally; the early returns only avoid
    /// needless work.
    pub fn snap_hints_near_cursor(&self) -> CadResult<Vec<SnapCandidate>> {
        if !self.session.snap_enabled() {
            return Ok(Vec::new());
        }
        let Some(cursor) = self.active_tool_cursor() else {
            return Ok(Vec::new());
        };
        let Some(world_per_px) = self.viewport_world_per_px() else {
            return Ok(Vec::new());
        };
        let drawing = self.drawing_required()?;
        databases::snap_candidates_for_hint(
            drawing,
            self.document_id,
            cursor,
            world_per_px,
            self.session.active_space.clone(),
        )
    }

    /// Human-facing unit label of the open document, for the status area.
    ///
    /// Unknown units report the spec's "drawing units" default rather than
    /// assuming millimetres (spec §16, audit N01).
    pub fn unit_label(&self) -> &'static str {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|d| d.units.label())
            .unwrap_or("drawing units")
    }

    /// The authoritative session mode (Viewer/Work) — the single source the UI
    /// label and affordances must reflect (audit U02).
    pub fn mode(&self) -> AppMode {
        self.session.mode()
    }

    /// Switch the session mode through the command path (audit U02).
    ///
    /// An unconfirmed tool is cancelled, never silently committed, and no
    /// Viewer-incompatible write is performed by the switch itself.
    pub fn set_mode(&mut self, mode: AppMode) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetMode,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Mode(mode),
        })?;
        Ok(())
    }

    /// The open document's immutable drawing, or an error when none is loaded.
    fn drawing_required(&self) -> CadResult<&DrawingDatabase> {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|d| d.drawing.as_ref())
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))
    }

    /// Full layer-panel projection: database layer table + temporary overrides.
    ///
    /// Pure getter (F03): it never rewrites the DWG layer table and never
    /// dispatches a command.
    pub fn layer_rows(&self) -> CadResult<Vec<crate::layers::LayerRow>> {
        let drawing = self.drawing_required()?;
        Ok(crate::layers::layer_rows(
            drawing,
            &self.session.layer_overrides,
        ))
    }

    /// Layer rows filtered by a case-insensitive name needle (search entry).
    pub fn search_layers(&self, needle: &str) -> CadResult<Vec<crate::layers::LayerRow>> {
        Ok(crate::layers::filter_layer_rows(
            &self.layer_rows()?,
            needle,
        ))
    }

    /// Ordered layer ids matching [`HostController::layer_rows`].
    ///
    /// The UI panel addresses rows by index, so the host passes this list to
    /// `UiHandle::set_layer_state` to keep index→LayerId exact (no lossy cast).
    pub fn layer_ids(&self) -> CadResult<Vec<LayerId>> {
        Ok(self.layer_rows()?.into_iter().map(|row| row.id).collect())
    }

    /// Apply a temporary layer visibility through the command path.
    pub fn set_layer_visible(&mut self, layer: LayerId, visible: bool) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::ToggleLayer,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Layer(layer, visible),
        })?;
        Ok(())
    }

    /// Atomically apply temporary layer visibility values through the command path.
    /// Empty, duplicate, unknown or stale targets are refused without publication.
    pub fn set_layer_visibilities(&mut self, changes: Vec<(LayerId, bool)>) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetLayerVisibilities,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::LayerVisibilities(changes),
        })?;
        Ok(())
    }

    /// Drop every temporary layer override through the command path.
    pub fn restore_layers(&mut self) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::RestoreLayers,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::None,
        })?;
        Ok(())
    }

    /// The current selection set (read-only view).
    pub fn selection(&self) -> &crate::SelectionSet {
        &self.session.selection
    }

    /// Replace the selection through the command path. Read-only: never writes
    /// the DWG and produces no history entry (F05).
    pub fn set_selection(&mut self, refs: Vec<SelectionRef>) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::Select,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Selection(refs),
        })?;
        Ok(())
    }

    /// Read-only property projection for the current selection.
    pub fn selection_properties(&self) -> CadResult<crate::SelectionProperties> {
        let drawing = self.drawing_required()?;
        Ok(crate::SelectionProperties::extract(
            drawing,
            &self.session.selection,
        ))
    }
}
