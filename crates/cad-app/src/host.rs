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
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportReport, ImportRequest, Importer};

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
    match geometry {
        SemanticGeometry::Line { .. } => "AcDbLine",
        SemanticGeometry::Circle { .. } => "AcDbCircle",
        SemanticGeometry::Polyline { .. } => "AcDbPolyline",
        SemanticGeometry::Arc { .. } => "AcDbArc",
        SemanticGeometry::Ellipse { .. } => "AcDbEllipse",
        SemanticGeometry::Spline { .. } => "AcDbSpline",
        SemanticGeometry::Mesh(_) => "AcDbSubDMesh",
        SemanticGeometry::Insert { .. } => "AcDbBlockReference",
        SemanticGeometry::Text { .. } => "AcDbText",
        SemanticGeometry::Point(_) => "AcDbPoint",
        SemanticGeometry::Opaque { .. } => "AcDbUnknown",
        SemanticGeometry::Compound(_) => "AcDbCompound",
    }
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
    /// This is the no-decision entry used when the caller has already resolved
    /// any unsaved work. If the current document has unsaved annotations it is
    /// rejected with `Cancelled`; call [`HostController::open_bytes_decided`]
    /// with an explicit decision instead.
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
    pub fn open_bytes_decided(
        &mut self,
        bytes: Arc<[u8]>,
        label: &str,
        decision: UnsavedDecision,
    ) -> CadResult<OpenedDrawing> {
        // Refuse to touch a dirty document unless the user explicitly chose to
        // discard it or has already preserved a recovery copy.
        self.application.prepare_leave(self.document_id, decision)?;

        let request = ImportRequest {
            document: self.document_id,
            database: DatabaseId(1),
            bytes,
            limits: ImportLimits::default(),
            generation: self.session.generation,
        };
        let importer = AcadrustImporter::new();
        let imported = importer.import(&request, &|| false)?;
        let opened = OpenedDrawing {
            entities: imported.database.entity_count(),
            completeness_label: imported.report.completeness_label(),
            diagnostics: imported.report.diagnostics.clone(),
        };
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
        self.document_name_hint = label.to_string();
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
    pub fn execute(&mut self, command: Command) -> CadResult<CommandOutcome> {
        let outcome = self.application.execute(&mut self.session, command)?;
        if let Some(diagnostic) = outcome.diagnostics.first() {
            self.last_status = diagnostic.message.clone();
        }
        Ok(outcome)
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
            .prepare_leave(controller.document_id, UnsavedDecision::ExplicitDiscard)
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
}
