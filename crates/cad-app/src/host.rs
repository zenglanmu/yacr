//! Shared host wiring: application + session + one document slot.
//!
//! Spec v2.0 §2.1(4), §4.2, §9: every platform host (Android, Web, CLI)
//! assembles the same core objects and drives them through the same
//! `Application::execute` command path. Hosts own platform I/O (files, GPU,
//! lifecycle); this module owns the business path so hosts cannot drift apart.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_annotations::{AnnotationCommand, AnnotationFile, AnnotationService, FingerprintPolicy, SCHEMA_VERSION};
use cad_db::{AnnotationDatabase, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::*;
use cad_history::{patch, UndoRecord};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportReport, ImportRequest, Importer};

use crate::{AppMode, Application, Command, CommandOutcome, Document, SessionState, Viewport};

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
    builder.insert_layer(Layer { id: LayerId(0), name: "0".into(), visible: true }).unwrap();
    builder.insert_layer(Layer { id: LayerId(1), name: "WALLS".into(), visible: true }).unwrap();

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
    push(1, SemanticGeometry::Line { start: p(0.0, 0.0), end: p(4000.0, 0.0) }, LayerId(1), 0);
    push(2, SemanticGeometry::Line { start: p(4000.0, 0.0), end: p(4000.0, 3000.0) }, LayerId(1), 1);
    push(3, SemanticGeometry::Line { start: p(4000.0, 3000.0), end: p(0.0, 3000.0) }, LayerId(1), 2);
    push(4, SemanticGeometry::Line { start: p(0.0, 3000.0), end: p(0.0, 0.0) }, LayerId(1), 3);
    push(5, SemanticGeometry::Circle { center: p(2000.0, 1500.0), normal: pz(0.0, 0.0, 1.0), radius: 800.0 }, LayerId(0), 4);
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
            document_id.clone(),
            Document {
                id: document_id.clone(),
                drawing,
                annotations: AnnotationDatabase::new(DatabaseId(2)),
                identity: DocumentIdentity::Temporary(0),
                units: UnitContext::drawing_units(),
                resource_keys: Vec::new(),
            },
        );
        application
            .workspace
            .viewports
            .insert(viewport_id.clone(), Viewport::new(viewport_id.clone(), document_id.clone(), logical_size));
        Ok(HostController {
            application,
            session: SessionState::new(document_id.clone(), AppMode::Work),
            document_id,
            viewport_id,
            last_status: "就绪（内置演示几何，非兼容性声明）".to_string(),
            document_name_hint: "yacr-demo".to_string(),
            last_import_report: None,
        })
    }

    /// The current immutable drawing, for the render/composition pipeline.
    pub fn drawing(&self) -> Option<Arc<DrawingDatabase>> {
        self.application.workspace.documents.get(&self.document_id).map(|d| d.drawing.clone())
    }

    /// Import a DWG byte stream through the single importer boundary.
    ///
    /// Import failures leave the current document untouched.
    pub fn open_bytes(&mut self, bytes: Arc<[u8]>, label: &str) -> CadResult<OpenedDrawing> {
        let request = ImportRequest {
            document: self.document_id.clone(),
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
            id: self.document_id.clone(),
            drawing: Arc::new(imported.database),
            annotations: AnnotationDatabase::new(DatabaseId(2)),
            identity: imported.report.identity.clone(),
            units: imported.units,
            resource_keys: Vec::new(),
        };
        self.application.workspace.documents.insert(self.document_id.clone(), document);
        self.session.generation += 1;
        self.document_name_hint = label.to_string();
        self.last_import_report = Some(imported.report.clone());
        self.last_status = format!("已打开 {label}: {}", opened.completeness_label);
        Ok(opened)
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
        self.application.fit_viewport(&mut self.session, &self.viewport_id)
    }

    /// Serialise the document's annotations as the versioned sidecar JSON.
    ///
    /// Spec §3.4: schema_version, fingerprint, name hint, unit context and the
    /// annotation list are mandatory; the format is independent of the DWG.
    pub fn export_annotations_json(&self) -> CadResult<String> {
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
        String::from_utf8(bytes)
            .map_err(|e| CadError::Invariant(format!("annotation JSON is not UTF-8: {e}")))
    }

    /// Mark the current annotation revision as durably exported. Only called
    /// after the host confirms the write succeeded (spec §3.4).
    pub fn mark_annotations_saved(&mut self) -> CadResult<()> {
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let revision = document.annotations.revision();
        document.annotations.mark_exported(revision)
    }

    /// Import a sidecar file as one transaction, recording one undo step.
    ///
    /// The caller chooses the fingerprint policy; mismatches never attach
    /// silently (spec §3.4).
    pub fn import_annotations_json(&mut self, text: &str, policy: FingerprintPolicy) -> CadResult<usize> {
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
        let change_set =
            document.annotations.apply_annotation_changes("import annotations", transaction, changes)?;
        self.application.history.entry(self.document_id.clone()).or_default().record(UndoRecord {
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
            document: self.document_id.clone(),
            viewport: self.viewport_id.clone(),
            payload: crate::CommandPayload::Annotation(command),
        };
        self.execute(command)
    }

    pub fn status(&self) -> &str {
        &self.last_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn annotation_export_import_roundtrip_records_one_undo_step() {
        use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle};
        let mut source = HostController::with_demo_document([800.0, 600.0]).unwrap();
        source
            .apply_annotation(AnnotationCommand::Create(Annotation {
                id: AnnotationId(42),
                space: SpaceId::Model,
                geometry: AnnotationGeometry::Text(Point3 { x: 1.0, y: 2.0, z: 0.0 }),
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
        let count = target.import_annotations_json(&json, FingerprintPolicy::ImportUnanchored).unwrap();
        assert_eq!(count, 1);
        let document = target.application.workspace.documents.get(&target.document_id).unwrap();
        assert_eq!(document.annotations.len(), 1);
        assert!(document.annotations.is_dirty());
        assert!(target.application.can_undo(&target.document_id));

        target.mark_annotations_saved().unwrap();
        assert!(!target.application.workspace.documents[&target.document_id].annotations.is_dirty());
    }
}