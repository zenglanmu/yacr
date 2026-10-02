//! Shared host wiring: application + session + one document slot.
//!
//! Spec v2.0 §2.1(4), §4.2, §9: every platform host (Android, Web, CLI)
//! assembles the same core objects and drives them through the same
//! `Application::execute` command path. Hosts own platform I/O (files, GPU,
//! lifecycle); this module owns the business path so hosts cannot drift apart.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cad_annotations::{
    AnnotationCommand, AnnotationFile, AnnotationService, FingerprintPolicy, SCHEMA_VERSION,
};
use cad_db::{
    AnnotationDatabase, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};
use cad_domain::*;
use cad_history::{patch, UndoRecord};
use cad_import_acadrust::{
    AcadrustImporter, ImportLimits, ImportReport, ImportRequest, ImportedDrawing, Importer,
};

use crate::recovery::{RecoverySnapshot, UnsavedOutcome};
use crate::{
    AppMode, Application, Command, CommandOutcome, Document, SessionState, UnsavedDecision,
    Viewport,
};

static TX_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_transaction() -> TransactionId {
    TransactionId(TX_COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}

/// Synthetic drawing used before a real DWG is opened.
///
/// This is **not** a compatibility claim (the fixture manifest is empty); it
/// exercises the shared UI/render pipeline with known geometry.
pub fn demo_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: LayerId(1),
            name: "WALLS".into(),
            visible: true,
        })
        .unwrap();

    let mut push = |id: u128, geometry: SemanticGeometry, layer: LayerId, order: i64| {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: class_name(&geometry).into(),
                    revision: Revision(0),
                    source_handle: Some(format!("{id:X}")),
                },
                id: EntityId(id),
                layer,
                space: SpaceId::Model,
                geometry,
                draw_order: order,
            })
            .unwrap();
    };

    // A simple room outline with a circle and a polyline-with-bulge.
    push(
        1,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(4000.0, 0.0),
        },
        LayerId(1),
        0,
    );
    push(
        2,
        SemanticGeometry::Line {
            start: p(4000.0, 0.0),
            end: p(4000.0, 3000.0),
        },
        LayerId(1),
        1,
    );
    push(
        3,
        SemanticGeometry::Line {
            start: p(4000.0, 3000.0),
            end: p(0.0, 3000.0),
        },
        LayerId(1),
        2,
    );
    push(
        4,
        SemanticGeometry::Line {
            start: p(0.0, 3000.0),
            end: p(0.0, 0.0),
        },
        LayerId(1),
        3,
    );
    push(
        5,
        SemanticGeometry::Circle {
            center: p(2000.0, 1500.0),
            normal: pz(0.0, 0.0, 1.0),
            radius: 800.0,
        },
        LayerId(0),
        4,
    );
    push(
        6,
        SemanticGeometry::Polyline {
            points: vec![p(500.0, 500.0), p(1500.0, 500.0), p(1500.0, 1000.0)],
            bulges: vec![0.0, 1.0, 0.0],
            closed: false,
        },
        LayerId(0),
        5,
    );
    builder.finish().unwrap()
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn pz(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn class_name(geometry: &SemanticGeometry) -> &'static str {
    crate::app_drawing::class_name(geometry)
}

/// Result of a successful drawing open, for status lines and diagnostics.
#[derive(Debug, Clone)]
pub struct OpenedDrawing {
    pub entities: usize,
    pub completeness_label: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// One document session shared by a host and its UI.
pub struct HostController {
    pub application: Application,
    pub session: SessionState,
    pub document_id: DocumentId,
    pub viewport_id: ViewportId,
    /// Last human-facing status derived from a command outcome.
    pub last_status: String,
    /// Name hint used when exporting the sidecar annotation file.
    pub document_name_hint: String,
    /// Report of the most recent successful open, for diagnostics/CLI.
    pub last_import_report: Option<ImportReport>,
    /// The single current background import (spec F01). Empty until a host
    /// starts an asynchronous open; the synchronous path never uses it.
    pub(crate) import_manager: crate::tasks::ImportManager,
    /// Label for the running asynchronous open, applied on publish.
    pub(crate) pending_open_label: Option<String>,
    /// Last UI-facing snapshot of the asynchronous open (F01).
    ///
    /// This is a *projection* of [`crate::tasks::AsyncOpenPoll`], updated only
    /// by `begin_async_open` / `cancel_async_open` / `poll_async_open`; the pure
    /// getter [`HostController::async_open_snapshot`] just reads it, so reading
    /// the panel state never drains progress or publishes a document.
    pub(crate) async_open: Option<crate::tasks::ImportProgressSnapshot>,
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
                annotations: AnnotationDatabase::new(DatabaseId(2)),
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
            async_open: None,
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

    /// Human-facing state of the current document for an unsaved-work prompt.
    ///
    /// Pure getter: it reports whether the annotations are dirty, how many there
    /// are and the name hint, without performing any write. Hosts use this to
    /// decide whether a prompt is needed before replacing the document.
    pub fn unsaved_signal(&self) -> crate::host_files::UnsavedSignal {
        let annotations = self.workspace_annotations();
        crate::host_files::UnsavedSignal {
            dirty: annotations.map(|a| a.is_dirty()).unwrap_or(false),
            annotation_count: annotations.map(|a| a.len()).unwrap_or(0),
            name_hint: self.document_name_hint.clone(),
        }
    }

    /// The current document's annotation database (read-only view).
    pub fn workspace_annotations(&self) -> Option<&AnnotationDatabase> {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|d| &d.annotations)
    }

    /// Import a DWG byte stream through the single importer boundary.
    ///
    /// This is the no-decision entry for callers that open a file without an
    /// unsaved-work interaction (CLI, a fresh open). A clean document proceeds;
    /// if the current document has unsaved annotations it is rejected with
    /// `Cancelled`, and the caller must resolve the decision via
    /// [`HostController::open_bytes_leaving`].
    pub fn open_bytes(&mut self, bytes: Arc<[u8]>, label: &str) -> CadResult<OpenedDrawing> {
        self.open_bytes_decided(bytes, label, UnsavedDecision::Cancel)
    }

    /// Import a DWG byte stream after applying an explicit unsaved-work decision.
    ///
    /// Only a successful import replaces the document. On success the session,
    /// history, selection, layer overrides and active tool are rebuilt for the
    /// new content so the previous drawing's undo records and references can
    /// never act on it (audit B05). Failures and cancellation leave the current
    /// document and session untouched.
    ///
    /// `Save`/`PreserveRecovery` are refused here because this entry point does
    /// not perform the host-owned writes; the host must first persist and confirm
    /// them, then call [`HostController::open_bytes_leaving`] with the results.
    pub fn open_bytes_decided(
        &mut self,
        bytes: Arc<[u8]>,
        label: &str,
        decision: UnsavedDecision,
    ) -> CadResult<OpenedDrawing> {
        self.open_bytes_leaving(bytes, label, decision, false, false)
    }

    /// Import a DWG byte stream, applying the decision with explicit write results.
    ///
    /// `save_succeeded` / `recovery_succeeded` are the host's confirmed results
    /// for the atomic annotation export and the recovery snapshot. A failed save
    /// or recovery never replaces the document (audit B07/U09); `Discard`
    /// proceeds and `Cancel` keeps everything.
    pub fn open_bytes_leaving(
        &mut self,
        bytes: Arc<[u8]>,
        label: &str,
        decision: UnsavedDecision,
        save_succeeded: bool,
        recovery_succeeded: bool,
    ) -> CadResult<OpenedDrawing> {
        // Refuse to touch a dirty document unless the decision model allows it.
        match self.application.resolve_leave(
            self.document_id,
            decision,
            save_succeeded,
            recovery_succeeded,
        ) {
            UnsavedOutcome::Proceed => {}
            UnsavedOutcome::Cancelled => return Err(CadError::Cancelled),
            UnsavedOutcome::SaveFailed => {
                return Err(CadError::Unsupported(
                    "保存未确认成功，未替换当前文档".into(),
                ))
            }
            UnsavedOutcome::RecoveryFailed => {
                return Err(CadError::Invariant(
                    "恢复快照未能持久化，未替换当前文档".into(),
                ))
            }
        }

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
            annotations: AnnotationDatabase::new(DatabaseId(2)),
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
        self.application.history.remove(&self.document_id);
        self.application.query = cad_query::QueryService::new();
        self.session = SessionState::new(self.document_id, self.session.mode());
        self.session.generation = self.session.generation.saturating_add(1);
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
    pub fn fit(&mut self) -> CadResult<()> {
        self.application
            .fit_viewport(&mut self.session, &self.viewport_id)
    }

    /// Serialise the document's annotations as the versioned sidecar JSON.
    ///
    /// Spec §3.4: schema_version, fingerprint, name hint, unit context and the
    /// annotation list are mandatory; the format is independent of the DWG.
    pub fn export_annotations_json(&self) -> CadResult<String> {
        Ok(self.prepare_annotation_export()?.0)
    }

    /// Prepare an export bundle: the JSON plus the revision it was taken at.
    ///
    /// This is a pure getter and never marks the document saved. The caller must
    /// confirm the durable write with [`HostController::confirm_annotation_export`]
    /// and the same revision (audit B07).
    pub fn prepare_annotation_export(&self) -> CadResult<(String, Revision)> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let file = AnnotationFile {
            schema_version: SCHEMA_VERSION,
            application_version: env!("CARGO_PKG_VERSION").to_string(),
            document_fingerprint: document.identity.clone(),
            document_name_hint: self.document_name_hint.clone(),
            unit_context: document.units.clone(),
            annotations: document.annotations.annotations().cloned().collect(),
            view_bookmarks: Vec::new(),
            extensions_json: Default::default(),
        };
        let bytes = AnnotationService.encode(&file)?;
        let json = String::from_utf8(bytes)
            .map_err(|e| CadError::Invariant(format!("annotation JSON is not UTF-8: {e}")))?;
        Ok((json, document.annotations.revision()))
    }

    /// Mark the exact exported revision as durably saved.
    ///
    /// Only call after the write is confirmed to have succeeded. Using the
    /// exported revision prevents a later edit from being marked saved by an
    /// earlier export (audit B07).
    pub fn confirm_annotation_export(&mut self, revision: Revision) -> CadResult<()> {
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        document.annotations.mark_exported(revision)
    }

    /// Mark the current annotation revision as durably exported. Only called
    /// after the host confirms the write succeeded (spec §3.4).
    pub fn mark_annotations_saved(&mut self) -> CadResult<()> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let revision = document.annotations.revision();
        self.confirm_annotation_export(revision)
    }

    /// Capture a recovery snapshot of the current unsaved annotation work.
    ///
    /// The snapshot uses the same atomic export encoder as the sidecar export,
    /// so a partially-encoded snapshot is impossible: either the annotations
    /// fully encode or this returns an error and no snapshot is produced. The
    /// snapshot carries the document identity and camera so a restored session
    /// rebuilds the scene from the database, never from lost GPU batches
    /// (audit F12). Preparing a snapshot does **not** mark the document saved.
    pub fn capture_recovery_snapshot(
        &self,
        camera_center: [f64; 3],
        camera_world_per_px: f64,
    ) -> CadResult<RecoverySnapshot> {
        let (annotations_json, _revision) = self.prepare_annotation_export()?;
        let identity = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .identity
            .clone();
        Ok(RecoverySnapshot {
            identity,
            name_hint: self.document_name_hint.clone(),
            annotations_json,
            camera_center,
            camera_world_per_px: if camera_world_per_px.is_finite() && camera_world_per_px > 0.0 {
                camera_world_per_px
            } else {
                1.0
            },
        })
    }

    /// Restore a recovery snapshot into the current document.
    ///
    /// The annotation payload is decoded with the document's own identity using
    /// the strict fingerprint policy; a mismatch or corrupt payload is an error
    /// and nothing is applied (the recovery copy must not silently attach to the
    /// wrong drawing). Restoration re-imports through the same transaction path
    /// as a sidecar import, so it records one undo step and publishes changes.
    pub fn restore_recovery_snapshot(&mut self, snapshot: &RecoverySnapshot) -> CadResult<usize> {
        self.import_annotations_json(
            &snapshot.annotations_json,
            FingerprintPolicy::RejectMismatch,
        )
    }

    /// Import a sidecar file as one transaction, recording one undo step.
    ///
    /// Host-owned I/O stays here, but the business import is gated by the same
    /// Work-only permission as the command path, so a Viewer session cannot
    /// import through the host API, JS or CLI (audit B08). Mismatches never
    /// attach silently (spec §3.4).
    pub fn import_annotations_json(
        &mut self,
        text: &str,
        policy: FingerprintPolicy,
    ) -> CadResult<usize> {
        // Authorize before any decoding so a rejected import changes nothing.
        self.session
            .authorize(crate::CommandId::ImportAnnotations)?;
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let file = AnnotationService.decode(text.as_bytes(), &document.identity, policy)?;
        if file.annotations.is_empty() {
            return Ok(0);
        }
        let mut changes = Vec::with_capacity(file.annotations.len());
        let mut patches = Vec::with_capacity(file.annotations.len());
        for annotation in &file.annotations {
            let before = document.annotations.get(annotation.id).cloned();
            changes.push((annotation.id, Some(annotation.clone())));
            patches.push(patch(annotation.id, before, Some(annotation.clone())));
        }
        let transaction = next_transaction();
        let change_set = document.annotations.apply_annotation_changes(
            "import annotations",
            transaction,
            changes,
        )?;
        self.application
            .history
            .entry(self.document_id)
            .or_default()
            .record(UndoRecord {
                transaction: change_set.transaction,
                label: "import annotations".to_string(),
                patches,
                merge_key: None,
            })?;
        self.last_status = format!("已导入 {} 条批注", file.annotations.len());
        Ok(file.annotations.len())
    }

    /// Apply one annotation command through the shared service (used by tools).
    pub fn apply_annotation(&mut self, command: AnnotationCommand) -> CadResult<CommandOutcome> {
        let command = Command {
            schema_version: 1,
            id: match command {
                AnnotationCommand::Create(_) => crate::CommandId::CreateAnnotation,
                AnnotationCommand::Update(_) => crate::CommandId::UpdateAnnotation,
                AnnotationCommand::Delete(_) => crate::CommandId::DeleteAnnotation,
            },
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Annotation(Box::new(command)),
        };
        self.execute(command)
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

    /// Whether a confirmed measurement is available to save as an annotation.
    ///
    /// Pure getter (audit U04); hosts use it to enable the save affordance
    /// without re-running the measurement tool.
    pub fn has_last_measurement(&self) -> bool {
        self.session.has_last_measurement()
    }

    /// Persist the last confirmed measurement as an annotation through the
    /// shared annotation transaction/history path (F06/F07).
    ///
    /// Refuses with `InvalidInput` when no measurement has been confirmed and
    /// enforces the Work-mode permission at the command layer.
    pub fn save_measurement_as_annotation(&mut self) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SaveMeasurementAsAnnotation,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::None,
        })
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

    /// Read-only management rows for the annotation list panel (F09).
    ///
    /// Pure getter: it projects the document's annotation database plus the
    /// session visibility overrides and the current selection. It dispatches no
    /// command and never mutates an annotation.
    pub fn annotation_rows(&self) -> CadResult<Vec<crate::AnnotationRow>> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        Ok(self.session.annotation_rows(&document.annotations))
    }

    /// The annotation currently selected for edit/delete, if any.
    pub fn selected_annotation(&self) -> Option<AnnotationId> {
        self.session.selected_annotation
    }

    /// Temporary annotation visibility set (session-scoped, never persisted).
    pub fn annotation_visibility(&self) -> &crate::AnnotationVisibilitySet {
        &self.session.annotation_visibility
    }

    /// Hide or show one annotation through the command path.
    ///
    /// Session state only: no transaction, no history entry and no change to
    /// the annotation revision or sidecar.
    pub fn set_annotation_visible(&mut self, id: AnnotationId, visible: bool) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetAnnotationVisibility,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::AnnotationVisibility(id, visible),
        })?;
        Ok(())
    }

    /// Select (or clear) an annotation for edit/delete through the command path.
    pub fn select_annotation(&mut self, id: Option<AnnotationId>) -> CadResult<()> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SelectAnnotation,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::SelectAnnotation(id),
        })?;
        Ok(())
    }

    /// Delete one annotation through the shared transaction/history path.
    ///
    /// Exactly one transaction and one undo record; a missing id is refused by
    /// the database before anything is recorded.
    pub fn delete_annotation(&mut self, id: AnnotationId) -> CadResult<()> {
        self.apply_annotation(AnnotationCommand::Delete(id))?;
        // A deleted annotation must not linger as a hidden override or as the
        // management selection; clear both so the panel cannot show a ghost.
        self.session.annotation_visibility.remove(id);
        if self.session.selected_annotation == Some(id) {
            self.session.selected_annotation = None;
        }
        Ok(())
    }

    /// Active annotation tool preview for the tool panel, if a tool is running.
    ///
    /// Pure getter (audit U04): it never dispatches a command or advances tool.
    pub fn annotation_preview(&self) -> Option<crate::AnnotationPreview> {
        self.session.annotation_preview()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle};

    /// A small text annotation for dirty/open tests.
    fn text_note(id: u128, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            text: text.into(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    #[test]
    fn demo_document_has_geometry_and_bounds() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let drawing = controller.drawing().unwrap();
        assert!(drawing.entity_count() >= 6);
        let (min, max) = drawing.bounds().unwrap();
        assert!(max.x - min.x > 0.0);
    }

    #[test]
    fn open_bytes_failure_keeps_the_current_document() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let before = controller.drawing().unwrap().id();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        assert!(controller.open_bytes(garbage, "bad.dwg").is_err());
        assert_eq!(controller.drawing().unwrap().id(), before);
    }

    #[test]
    fn open_while_dirty_is_rejected_without_a_decision() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "keep me")))
            .unwrap();
        assert!(controller
            .workspace_annotations()
            .map(|a| a.is_dirty())
            .unwrap_or(false));
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        // A dirty document refuses to be replaced by default.
        assert!(matches!(
            controller.open_bytes(garbage, "next.dwg"),
            Err(CadError::Cancelled)
        ));
        // The unsaved annotation survives the cancelled open.
        assert_eq!(controller.workspace_annotations().unwrap().len(), 1);
    }

    #[test]
    fn opening_new_content_resets_history_and_annotations() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "old")))
            .unwrap();
        assert!(controller.application.can_undo(&controller.document_id));

        // Explicit discard, then a valid empty-content open through the decided
        // path (demo bytes are not a real DWG, so drive the reset directly).
        controller
            .application
            .prepare_leave(controller.document_id, UnsavedDecision::Discard)
            .unwrap();
        controller.reset_for_new_content();
        assert!(!controller.application.can_undo(&controller.document_id));
        assert!(matches!(controller.session.tool, crate::ToolState::Idle));
        assert!(controller.session.selection.is_empty());
        assert!(controller.session.layer_overrides.is_empty());
    }

    #[test]
    fn annotation_export_import_roundtrip_records_one_undo_step() {
        let mut source = HostController::with_demo_document([800.0, 600.0]).unwrap();
        source
            .apply_annotation(AnnotationCommand::Create(Annotation {
                id: AnnotationId(42),
                space: SpaceId::Model,
                geometry: AnnotationGeometry::Text(Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                }),
                text: "检验批注".into(),
                style: AnnotationStyle::default(),
                created_unix_ms: 0,
                modified_unix_ms: 0,
                anchor: None,
                precision: Precision::Analytic,
            }))
            .unwrap();
        let json = source.export_annotations_json().unwrap();
        assert!(json.contains("schema_version"));
        assert!(json.contains("document_fingerprint"));

        let mut target = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let count = target
            .import_annotations_json(&json, FingerprintPolicy::ImportUnanchored)
            .unwrap();
        assert_eq!(count, 1);
        let document = target
            .application
            .workspace
            .documents
            .get(&target.document_id)
            .unwrap();
        assert_eq!(document.annotations.len(), 1);
        assert!(document.annotations.is_dirty());
        assert!(target.application.can_undo(&target.document_id));

        target.mark_annotations_saved().unwrap();
        assert!(!target.application.workspace.documents[&target.document_id]
            .annotations
            .is_dirty());
    }

    #[test]
    fn import_annotations_is_rejected_in_viewer_mode() {
        // A Viewer session must not import through the host API even though the
        // bytes are decoded here (audit B08).
        let mut source = HostController::with_demo_document([800.0, 600.0]).unwrap();
        source
            .apply_annotation(AnnotationCommand::Create(text_note(1, "note")))
            .unwrap();
        let json = source.export_annotations_json().unwrap();

        let mut viewer = HostController::with_demo_document([800.0, 600.0]).unwrap();
        viewer.session.switch_mode(AppMode::Viewer).unwrap();
        let revision_before = viewer.workspace_annotations().unwrap().revision();
        assert!(matches!(
            viewer.import_annotations_json(&json, FingerprintPolicy::ImportUnanchored),
            Err(CadError::PermissionDenied)
        ));
        // Nothing changed: no annotations, no revision advance, no history.
        assert_eq!(viewer.workspace_annotations().unwrap().len(), 0);
        assert_eq!(
            viewer.workspace_annotations().unwrap().revision(),
            revision_before
        );
        assert!(!viewer.application.can_undo(&viewer.document_id));
    }

    #[test]
    fn export_prepare_is_a_pure_getter() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "note")))
            .unwrap();
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        let (json, revision) = controller.prepare_annotation_export().unwrap();
        assert!(json.contains("schema_version"));
        // Preparing the export must not mark the document saved.
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        assert_eq!(
            revision,
            controller.workspace_annotations().unwrap().revision()
        );
    }

    #[test]
    fn measurement_preview_getter_is_pure_and_reports_the_active_tool() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // No tool: no preview and no command dispatched.
        assert!(controller.measurement_preview().is_none());

        controller
            .execute(Command {
                schema_version: 1,
                id: crate::CommandId::Measure,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            })
            .unwrap();
        let preview = controller.measurement_preview().expect("active preview");
        assert_eq!(preview.kind, crate::MeasurementToolKind::Distance);
        assert!(preview.points.is_empty());

        // Reading the preview twice is stable: no points are captured.
        let again = controller.measurement_preview().unwrap();
        assert_eq!(again.points.len(), preview.points.len());
        assert_eq!(controller.session.measurement_preview().unwrap(), again);
    }

    #[test]
    fn unit_label_reports_the_document_unit_context() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // The demo document has unknown units: never silently "mm".
        assert_eq!(controller.unit_label(), "drawing units");
    }

    #[test]
    fn layer_rows_come_from_the_demo_database_and_track_overrides() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let rows = controller.layer_rows().unwrap();
        assert_eq!(rows.len(), 2);
        // Both demo layers are stored visible and start un-overridden.
        assert!(rows.iter().all(|r| r.database_visible));
        assert!(rows.iter().all(|r| !r.is_overridden()));

        controller.set_layer_visible(LayerId(1), false).unwrap();
        let rows = controller.layer_rows().unwrap();
        let walls = rows.iter().find(|r| r.id == LayerId(1)).unwrap();
        assert!(!walls.effective_visible);
        assert!(walls.database_visible, "stored flag is untouched");
        assert!(walls.is_overridden());

        controller.restore_layers().unwrap();
        let rows = controller.layer_rows().unwrap();
        assert!(rows.iter().all(|r| !r.is_overridden()));
        assert!(rows.iter().all(|r| r.effective_visible));

        // layer_ids matches layer_rows order.
        let ids = controller.layer_ids().unwrap();
        assert_eq!(ids, rows.iter().map(|r| r.id).collect::<Vec<_>>());
    }

    #[test]
    fn layer_search_returns_real_matches_only() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        assert_eq!(controller.search_layers("wall").unwrap().len(), 1);
        assert!(controller.search_layers("roof").unwrap().is_empty());
    }

    #[test]
    fn selection_properties_are_read_only_and_report_demo_geometry() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Empty selection is explicit.
        assert!(controller.selection_properties().unwrap().empty);

        let revision_before = controller.drawing().unwrap().revision();
        controller
            .set_selection(vec![SelectionRef {
                document: controller.document_id,
                entity: EntityId(1),
                instance: InstancePath::default(),
                sub_element: None,
            }])
            .unwrap();
        let props = controller.selection_properties().unwrap();
        assert_eq!(props.count, 1);
        let row = |key: &str| {
            props
                .rows
                .iter()
                .find(|r| r.key == key)
                .map(|r| r.value.clone())
        };
        assert_eq!(row("id").as_deref(), Some("1"));
        assert_eq!(row("type").as_deref(), Some("AcDbLine"));
        assert_eq!(row("layer").as_deref(), Some("WALLS (1)"));
        assert_eq!(row("length").as_deref(), Some("4000.000000"));
        // Selection did not mutate the drawing or create history.
        assert_eq!(controller.drawing().unwrap().revision(), revision_before);
        assert!(!controller.application.can_undo(&controller.document_id));
    }

    #[test]
    fn export_at_rev_n_does_not_mark_later_rev_n_plus_1_saved() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "first")))
            .unwrap();
        let (_, rev_n) = controller.prepare_annotation_export().unwrap();
        // A later edit raises the revision.
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(2, "second")))
            .unwrap();
        let rev_n1 = controller.workspace_annotations().unwrap().revision();
        assert_ne!(rev_n, rev_n1);
        // Confirming the older export only marks that revision: still dirty.
        controller.confirm_annotation_export(rev_n).unwrap();
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        // Confirming the current revision clears dirty.
        controller.confirm_annotation_export(rev_n1).unwrap();
        assert!(!controller.workspace_annotations().unwrap().is_dirty());
    }

    #[test]
    fn annotation_rows_project_the_database_and_track_visibility() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Empty: explicit empty list, no fabricated rows.
        assert!(controller.annotation_rows().unwrap().is_empty());

        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "note one")))
            .unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(2, "note two")))
            .unwrap();
        let rows = controller.annotation_rows().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, AnnotationId(1));
        assert_eq!(rows[0].kind, "text");
        assert_eq!(rows[0].text, "note one");
        assert!(rows.iter().all(|r| r.visible));

        // Hide one: session state only.
        let revision_before = controller.workspace_annotations().unwrap().revision();
        controller
            .set_annotation_visible(AnnotationId(1), false)
            .unwrap();
        let rows = controller.annotation_rows().unwrap();
        assert!(!rows[0].visible);
        assert!(rows[0].is_overridden());
        assert_eq!(
            controller.workspace_annotations().unwrap().revision(),
            revision_before
        );

        // Show again: round-trips.
        controller
            .set_annotation_visible(AnnotationId(1), true)
            .unwrap();
        assert!(controller.annotation_rows().unwrap()[0].visible);
    }

    #[test]
    fn selecting_and_deleting_an_annotation_uses_the_shared_path() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(7, "doomed")))
            .unwrap();
        controller.select_annotation(Some(AnnotationId(7))).unwrap();
        assert_eq!(controller.selected_annotation(), Some(AnnotationId(7)));
        assert!(controller.annotation_rows().unwrap()[0].selected);

        controller.delete_annotation(AnnotationId(7)).unwrap();
        assert!(controller.workspace_annotations().unwrap().is_empty());
        // Delete is one undoable transaction and clears the selection.
        assert!(controller.application.can_undo(&controller.document_id));
        assert_eq!(controller.selected_annotation(), None);
        assert!(controller.annotation_rows().unwrap().is_empty());
    }

    #[test]
    fn deleting_a_missing_annotation_is_refused_and_records_nothing() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        assert!(controller.delete_annotation(AnnotationId(99)).is_err());
        assert!(!controller.application.can_undo(&controller.document_id));
    }

    #[test]
    fn failed_save_does_not_replace_the_document() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "unsaved")))
            .unwrap();
        let drawing_before = controller.drawing().unwrap().id();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        // Save chosen but the host could not confirm the write: never proceed,
        // never report saved (audit B07).
        assert!(matches!(
            controller.open_bytes_leaving(
                garbage.clone(),
                "next.dwg",
                UnsavedDecision::Save,
                false,
                false,
            ),
            Err(CadError::Unsupported(_))
        ));
        assert_eq!(controller.drawing().unwrap().id(), drawing_before);
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        assert_eq!(controller.workspace_annotations().unwrap().len(), 1);
    }

    #[test]
    fn failed_recovery_write_does_not_replace_the_document() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .apply_annotation(AnnotationCommand::Create(text_note(1, "unsaved")))
            .unwrap();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        assert!(matches!(
            controller.open_bytes_leaving(
                garbage.clone(),
                "next.dwg",
                UnsavedDecision::PreserveRecovery,
                false,
                false,
            ),
            Err(CadError::Invariant(_))
        ));
        assert!(controller.workspace_annotations().unwrap().is_dirty());
        // A confirmed recovery write is allowed past the leave guard; the
        // subsequent import still fails on the garbage bytes…
        assert!(controller
            .open_bytes_leaving(
                garbage,
                "next.dwg",
                UnsavedDecision::PreserveRecovery,
                false,
                true,
            )
            .is_err());
        // …and that import failure also leaves the document untouched.
        assert!(controller.workspace_annotations().unwrap().is_dirty());
    }

    #[test]
    fn recovery_snapshot_round_trips_annotations_through_the_host() {
        let mut source = HostController::with_demo_document([800.0, 600.0]).unwrap();
        source
            .apply_annotation(AnnotationCommand::Create(text_note(7, "recovered")))
            .unwrap();
        let snapshot = source
            .capture_recovery_snapshot([10.0, 20.0, 0.0], 0.5)
            .unwrap();
        // Capturing a snapshot is a pure getter: still dirty, still unsaved.
        assert!(source.workspace_annotations().unwrap().is_dirty());

        // Encode/decode through the storage form, then restore into a fresh
        // host that has the same document identity.
        let decoded = crate::RecoverySnapshot::decode(&snapshot.encode()).unwrap();
        let mut target = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Demo identities are Temporary(0) on both hosts, so the strict
        // fingerprint policy accepts the recovery copy.
        let restored = target.restore_recovery_snapshot(&decoded).unwrap();
        assert_eq!(restored, 1);
        assert_eq!(target.workspace_annotations().unwrap().len(), 1);
        let annotation = target
            .workspace_annotations()
            .unwrap()
            .annotations()
            .next()
            .unwrap();
        assert_eq!(annotation.id, AnnotationId(7));
        assert_eq!(annotation.text, "recovered");
    }

    #[test]
    fn host_drawing_commands_round_trip_with_undo() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let before = controller.drawing().unwrap().entity_count();

        // Create a line on layer 0 (the demo's default active layer).
        let outcome = controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
        assert_eq!(outcome.objects.len(), 1);
        assert_eq!(controller.drawing().unwrap().entity_count(), before + 1);
        assert!(controller.application.can_undo(&controller.document_id));
        assert!(controller.history_availability().can_undo);

        // Undo removes it.
        controller
            .execute(Command {
                schema_version: 1,
                id: crate::CommandId::Undo,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            })
            .unwrap();
        assert_eq!(controller.drawing().unwrap().entity_count(), before);
    }

    #[test]
    fn host_drawing_commands_are_refused_in_viewer_mode() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.set_mode(AppMode::Viewer).unwrap();
        let before = controller.drawing().unwrap().entity_count();
        let result = controller.create_circle(p(0.0, 0.0), p(10.0, 0.0));
        assert!(matches!(result, Err(CadError::PermissionDenied)));
        assert_eq!(controller.drawing().unwrap().entity_count(), before);
    }
}
