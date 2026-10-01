//! Android host: Activity entry, lifecycle, file access and GPU composition.
//!
//! Spec v2.0 §9.1, §5.3. This host wires the shared Slint UI to the CAD core and
//! the wgpu renderer. It does not re-implement CAD logic: opening a drawing goes
//! through the importer into the database, and rendering goes through the shared
//! representation/scene/renderer path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::{AppMode, Application, Command, CommandId, SessionState};
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::*;
use cad_import_acadrust::Importer;
use cad_ui_slint::{IncomingDocument, UiAdapter, UiCommandSink, UiConfiguration, UiHandle};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

pub struct AndroidHostConfiguration {
    pub recovery_enabled: bool,
    /// Candidate DWG locations to try on open (app-private/external dirs).
    pub sample_paths: Vec<String>,
}

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            recovery_enabled: true,
            sample_paths: vec![
                "/sdcard/Download/yacr-sample.dwg".to_string(),
                "/storage/emulated/0/Download/yacr-sample.dwg".to_string(),
            ],
        }
    }
}

/// A synthetic drawing so the app has something to show before a real DWG is
/// opened. This is *not* a compatibility claim; the fixture manifest is empty.
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

/// Commands from the UI are executed through the shared application layer.
struct HostSink {
    application: Application,
    session: SessionState,
    incoming: IncomingDocument,
    handle: SharedHandle,
    configuration: AndroidHostConfiguration,
}

impl HostSink {
    fn status(&self, text: impl Into<String>) {
        let text = text.into();
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.set_status(text);
        }
    }

    fn open_drawing(&mut self) {
        for path in &self.configuration.sample_paths {
            let candidate = std::path::Path::new(path);
            if !candidate.exists() {
                continue;
            }
            match std::fs::read(candidate) {
                Ok(bytes) => {
                    let request = cad_import_acadrust::ImportRequest {
                        document: self.session.document.clone(),
                        database: DatabaseId(1),
                        bytes: Arc::from(bytes.into_boxed_slice()),
                        limits: cad_import_acadrust::ImportLimits::default(),
                        generation: 0,
                    };
                    let importer = cad_import_acadrust::AcadrustImporter::new();
                    match importer.import(&request, &|| false) {
                        Ok(imported) => {
                            let document = cad_app::Document {
                                id: self.session.document.clone(),
                                drawing: Arc::new(imported.database),
                                annotations: cad_db::AnnotationDatabase::new(DatabaseId(2)),
                                identity: imported.report.identity.clone(),
                                units: imported.units,
                                resource_keys: Vec::new(),
                            };
                            *self.incoming.borrow_mut() = Some(document.drawing.clone());
                            self.application.workspace.documents.insert(self.session.document.clone(), document);
                            self.status(format!("已打开 {path}: {}", imported.report.completeness_label()));
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
}

impl UiCommandSink for HostSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        if command.id == CommandId::OpenDrawing {
            self.open_drawing();
            return Ok(());
        }
        match self.application.execute(&mut self.session, command) {
            Ok(outcome) => {
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

/// Build the shared UI + core + renderer stack and run it.
pub fn start(configuration: AndroidHostConfiguration) -> CadResult<()> {
    cad_ui_slint::select_wgpu_backend()?;

    let incoming: IncomingDocument = Rc::new(RefCell::new(None));
    let document_id = DocumentId(1);
    let session = SessionState::new(document_id.clone(), AppMode::Work);

    let mut application = Application::new();
    let drawing = Arc::new(demo_database());
    application.workspace.documents.insert(
        document_id.clone(),
        cad_app::Document {
            id: document_id.clone(),
            drawing: drawing.clone(),
            annotations: cad_db::AnnotationDatabase::new(DatabaseId(2)),
            identity: DocumentIdentity::Temporary(0),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    application
        .workspace
        .viewports
        .insert(ViewportId(1), cad_app::Viewport::new(ViewportId(1), document_id.clone(), [1080.0, 1920.0]));
    *incoming.borrow_mut() = Some(drawing);

    let ui_config = UiConfiguration {
        compact: true,
        locale: "zh-CN".into(),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD".into(),
        document: document_id,
        viewport: ViewportId(1),
    };

    // The sink needs the UI handle, which only exists after the adapter is
    // built, so it is shared through a slot filled immediately afterwards.
    let shared_handle: SharedHandle = Rc::new(RefCell::new(None));
    let sink = HostSink {
        application,
        session,
        incoming: incoming.clone(),
        handle: shared_handle.clone(),
        configuration,
    };
    let adapter = UiAdapter::new(ui_config, sink, true)?;
    *shared_handle.borrow_mut() = Some(adapter.handle());
    cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming)?;
    adapter.run()
}

/// Android entry point (`android-activity` 0.6 calls `fn android_main(app)`).
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    if let Err(e) = slint::android::init(app) {
        log::error!("slint android init failed: {e}");
        return;
    }
    if let Err(e) = start(AndroidHostConfiguration::default()) {
        log::error!("yacr android host failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_database_has_geometry_and_bounds() {
        let db = demo_database();
        assert!(db.entity_count() >= 6);
        let (min, max) = db.bounds().unwrap();
        assert!(max.x - min.x > 0.0);
    }
}
