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
use cad_import_acadrust::{
    AcadrustImporter, ImportLimits, ImportReport, ImportRequest, ImportedDrawing, Importer,
};
use cad_measure::{MeasurementEngine, SnapCandidate, SnapTarget};

use crate::{AppMode, Application, Command, CommandOutcome, Document, SessionState, Viewport};

/// Synthetic drawing used before a real DWG is opened.
///
/// This is **not** a compatibility claim (the fixture manifest holds only
/// synthetic fixtures and one openly-distributed sample); it exercises the
/// shared UI/render pipeline with known geometry.
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

/// Empty drawing with only the required layer `"0"`.
///
/// This is the document a host installs for "New": no entities, no bounds, and
/// exactly the default layer every DWG must contain. It makes no compatibility
/// or content claim.
pub fn blank_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
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

/// Build the local snap targets for a drawing's model-space geometry.
///
/// [`crate::drawing_pick_items`] expands `INSERT`s and returns each item's
/// geometry in its **own local** coordinates plus the accumulated world
/// transform; the snap engine has no transform parameter, so the transform is
/// baked into the geometry with [`transform_geometry`]. An item whose kind this
/// build cannot transform exactly (text, spline, shape, opaque, compound) is
/// **skipped**, never snapped at a wrong location; a top-level entity carries
/// the identity transform and is passed through unchanged (so a model-space
/// spline still snaps to its sampled points).
///
/// Every expanded item is presented in **world model space** (block children
/// are inlined by their composed transform), so all targets are tagged
/// [`SpaceId::Model`]. The precision mirrors the source: a spline's snap points
/// come from a fixed sample set ([`Precision::Approximate`]); analytic kinds are
/// [`Precision::Analytic`].
fn snap_targets_for(drawing: &DrawingDatabase, document: DocumentId) -> Vec<SnapTarget> {
    let identity = Transform3::identity();
    crate::drawing_pick_items(drawing, document)
        .into_iter()
        .filter_map(|item| {
            let geometry = if item.transform == identity {
                item.geometry
            } else {
                match transform_geometry(&item.geometry, &item.transform) {
                    Ok(world) => world,
                    // A kind this build cannot bake exactly contributes nothing
                    // rather than a snap at an un-transformed location.
                    Err(_) => return None,
                }
            };
            let precision = match &geometry {
                SemanticGeometry::Spline { .. } => Precision::Approximate { error_bound: None },
                _ => Precision::Analytic,
            };
            Some(SnapTarget::new(
                geometry,
                item.source,
                SpaceId::Model,
                precision,
            ))
        })
        .collect()
}

/// Snap candidates near a world cursor on the model work plane.
///
/// This is the pure bridge between the database and the object-snap overlay:
/// the caller (a host) holds the live cursor and the viewport's
/// `world_per_px`, and gets back the real [`SnapCandidate`]s the measurement
/// snap engine resolves, ready to hand to `CadView::set_snap_hints`.
///
/// **Cursor/ray convention.** A measurement or draw cursor is a world point on
/// the planar work plane, not a screen ray. The pick ray is therefore built as
/// `origin = cursor`, `direction = +Z`, and the snap plane is the world-Z plane
/// through the cursor (`origin = cursor`, `u = +X`, `v = +Y`). With a plane the
/// engine uses the ray∩plane point as its proximity probe, so the probe is
/// exactly `cursor`; candidate points on that plane have ray parameter `t = 0`
/// and are accepted (`accept` only rejects `t < 0`). This mirrors the
/// engine's own test convention (`snap.rs` builds a downward ray onto `z = 0`)
/// but keeps the plane at the cursor's height so a non-zero-z planar drawing is
/// not silently shifted.
///
/// Only geometry exactly on the cursor's plane is in front of the ray; that is
/// the planar 2D contract, stated rather than faked. `space` is the session's
/// active space and is passed as the engine's space filter; because
/// [`crate::drawing_pick_items`] only walks **model** space, a non-model `space`
/// honestly yields no candidates (paper-space snapping is not built here).
///
/// Returns [`CadError::InvalidInput`] for a non-finite cursor or a
/// non-positive/non-finite `world_per_px`; it never fabricates a candidate.
pub fn snap_candidates_near(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
) -> CadResult<Vec<SnapCandidate>> {
    if !cursor.x.is_finite() || !cursor.y.is_finite() || !cursor.z.is_finite() {
        return Err(CadError::InvalidInput(
            "snap cursor must be finite".to_string(),
        ));
    }
    if !world_per_px.is_finite() || world_per_px <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap world_per_px must be finite and positive".to_string(),
        ));
    }
    let plane = WorkPlane {
        origin: cursor,
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    };
    let ray = Ray3 {
        origin: cursor,
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    };
    let targets = snap_targets_for(drawing, document);
    MeasurementEngine::default().snap_targets(
        &targets,
        &ray,
        Some(&plane),
        world_per_px,
        Some(space),
    )
}

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

    /// The current immutable drawing, for the render/composition pipeline.
    pub fn drawing(&self) -> Option<Arc<DrawingDatabase>> {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|d| d.drawing.clone())
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
        self.application.history.remove(&self.document_id);
        self.application.query = cad_query::QueryService::new();
        self.session = SessionState::new(self.document_id, self.session.mode());
        self.session.generation = self.session.generation.saturating_add(1);
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
    /// The cursor lives in the tool preview (`MeasurementPreview::cursor`).
    /// The draw tool is not yet wired into [`SessionState`](crate::SessionState)
    /// at this revision, so only the measurement cursor is available; the draw
    /// cursor joins here once that state exists. Reading it dispatches no
    /// command.
    fn active_tool_cursor(&self) -> Option<Point3> {
        self.session
            .measurement_preview()
            .and_then(|preview| preview.cursor)
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
    /// Pure getter (audit U04): it reads the tool preview, the drawing and the
    /// viewport, resolves the snap candidates through the shared
    /// [`snap_candidates_near`] helper, and never dispatches a command or writes
    /// a database. With no live cursor (idle, or a tool that has not moved yet)
    /// it returns the explicit empty vector, so the host clears the snap-hint
    /// overlay rather than leaving a stale marker. A missing viewport likewise
    /// yields no hints — nothing is fabricated.
    ///
    /// The renderer independently gates the overlay on `view.overlays.snapHints`,
    /// so a host may call this unconditionally; the early return only avoids
    /// needless work when there is no cursor.
    pub fn snap_hints_near_cursor(&self) -> CadResult<Vec<SnapCandidate>> {
        let Some(cursor) = self.active_tool_cursor() else {
            return Ok(Vec::new());
        };
        let Some(world_per_px) = self.viewport_world_per_px() else {
            return Ok(Vec::new());
        };
        let drawing = self.drawing_required()?;
        snap_candidates_near(
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
    fn opening_new_content_resets_history_and_session_state() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
        assert!(controller.application.can_undo(&controller.document_id));

        controller.reset_for_new_content();
        assert!(!controller.application.can_undo(&controller.document_id));
        assert!(matches!(controller.session.tool, crate::ToolState::Idle));
        assert!(controller.session.selection.is_empty());
        assert!(controller.session.layer_overrides.is_empty());
    }

    #[test]
    fn new_blank_document_has_layer_zero_and_no_extent() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Make the demo dirty so the content reset is observable.
        controller.create_line(p(0.0, 0.0), p(100.0, 0.0)).unwrap();
        assert!(controller.application.can_undo(&controller.document_id));

        let report = controller.new_blank_document().unwrap();
        assert_eq!(report.entities, 0);
        assert!(!report.has_extent);

        let drawing = controller.drawing().unwrap();
        assert_eq!(drawing.entity_count(), 0);
        assert!(drawing.bounds().is_none());

        let rows = controller.layer_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "0");
        assert!(rows[0].database_visible);

        // Derived state is rebuilt for the new content.
        assert!(!controller.application.can_undo(&controller.document_id));
        assert!(controller.session.selection.is_empty());
        assert!(controller.session.layer_overrides.is_empty());
        assert!(matches!(controller.session.tool, crate::ToolState::Idle));
        assert_eq!(controller.document_name_hint, "untitled");
        assert!(controller.last_import_report.is_none());
    }

    #[test]
    fn new_blank_document_identities_are_fresh() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let identity = |controller: &HostController| {
            controller
                .application
                .workspace
                .documents
                .get(&controller.document_id)
                .unwrap()
                .identity
                .clone()
        };
        controller.new_blank_document().unwrap();
        let first = identity(&controller);
        controller.new_blank_document().unwrap();
        let second = identity(&controller);
        assert_ne!(first, second);
        assert!(matches!(second, DocumentIdentity::Temporary(id) if id >= 2));
    }

    #[test]
    fn fit_on_a_blank_drawing_is_a_documented_no_op() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.new_blank_document().unwrap();
        let generation = controller.session.generation;
        controller.fit().unwrap();
        assert_eq!(
            controller.session.generation, generation,
            "an empty drawing has nothing to frame; fit must not pretend to"
        );
    }

    #[test]
    fn fit_command_reports_the_empty_diagnostic_only_when_nothing_is_framed() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let fit = |controller: &mut HostController| {
            controller
                .execute(Command {
                    schema_version: 1,
                    id: crate::CommandId::FitDrawing,
                    document: controller.document_id,
                    viewport: controller.viewport_id,
                    payload: crate::CommandPayload::None,
                })
                .unwrap()
        };
        // Populated demo: a real fit, no empty diagnostic.
        assert!(fit(&mut controller).diagnostics.is_empty());

        controller.new_blank_document().unwrap();
        let outcome = fit(&mut controller);
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].code, "fit.empty");
        // The host status mirrors the code for the presentation layer.
        assert_eq!(controller.status(), "fit.empty");
    }

    #[test]
    fn new_blank_document_resets_the_viewport_camera_to_the_default() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Frame the demo, then move the camera so the reset is observable.
        controller.fit().unwrap();
        controller
            .application
            .workspace
            .viewports
            .get_mut(&controller.viewport_id)
            .unwrap()
            .camera
            .projection = crate::Projection::Orthographic { scale: 250.0 };

        controller.new_blank_document().unwrap();

        let camera = controller
            .application
            .workspace
            .viewports
            .get(&controller.viewport_id)
            .unwrap()
            .camera;
        assert_eq!(
            camera.target,
            p(0.0, 0.0),
            "New anchors the camera at origin"
        );
        assert_eq!(
            camera.projection,
            crate::Projection::Orthographic {
                scale: crate::camera::MIN_ORTHO_SCALE
            },
            "New uses the fresh-document default scale, not the old framing"
        );
    }

    #[test]
    fn new_blank_document_supersedes_a_pending_open() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        controller.begin_async_open(garbage, "pending.dwg");
        assert!(controller.pending_open_label.is_some());
        assert!(controller.async_open.is_some());
        assert!(controller.import_manager.is_running());

        controller.new_blank_document().unwrap();

        assert!(controller.pending_open_label.is_none());
        assert!(controller.pending_open_guard.is_none());
        assert!(controller.async_open.is_none());
        assert!(
            !controller.import_manager.is_running(),
            "New must release the superseded open job"
        );
    }

    #[test]
    fn application_refuses_new_drawing_as_host_owned() {
        let mut app = Application::new();
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let result = app.execute(
            &mut session,
            Command {
                schema_version: 1,
                id: crate::CommandId::NewDrawing,
                document: DocumentId(1),
                viewport: ViewportId(1),
                payload: crate::CommandPayload::None,
            },
        );
        assert!(matches!(result, Err(CadError::Unsupported(_))));
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

    #[test]
    fn snap_candidates_near_finds_endpoint_and_midpoint_of_the_demo_line() {
        let drawing = demo_database();
        let document = DocumentId(1);

        // The demo's first entity is a line (0,0)-(4000,0); a cursor 5 world
        // units from its end resolves an endpoint hint at (0,0).
        let near_end =
            snap_candidates_near(&drawing, document, p(0.0, 5.0), 1.0, SpaceId::Model).unwrap();
        let endpoint = near_end
            .iter()
            .find(|c| c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0))
            .expect("the line end is an endpoint hint");
        assert!((endpoint.logical_pixel_distance - 5.0).abs() < 1e-9);
        assert_eq!(endpoint.space, SpaceId::Model);

        // Near the segment middle the midpoint wins.
        let near_mid =
            snap_candidates_near(&drawing, document, p(2000.0, 5.0), 1.0, SpaceId::Model).unwrap();
        assert!(
            near_mid.iter().any(|c| {
                c.kind == cad_measure::SnapKind::Midpoint && c.point == p(2000.0, 0.0)
            }),
            "{near_mid:?}"
        );
    }

    #[test]
    fn snap_candidates_near_returns_nothing_far_from_geometry() {
        let drawing = demo_database();
        let far = snap_candidates_near(
            &drawing,
            DocumentId(1),
            p(1_000_000.0, 1_000_000.0),
            1.0,
            SpaceId::Model,
        )
        .unwrap();
        assert!(far.is_empty(), "{far:?}");
    }

    #[test]
    fn snap_candidates_near_rejects_a_bad_cursor_or_tolerance() {
        let drawing = demo_database();
        let nan_cursor = Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        };
        assert!(matches!(
            snap_candidates_near(&drawing, DocumentId(1), nan_cursor, 1.0, SpaceId::Model),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            snap_candidates_near(&drawing, DocumentId(1), p(0.0, 0.0), 0.0, SpaceId::Model),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn snap_candidates_near_only_offers_model_space_targets() {
        // Only model geometry is expanded, so a paper-space request honestly
        // yields no hints rather than snapping model geometry into a layout.
        let drawing = demo_database();
        let paper = snap_candidates_near(
            &drawing,
            DocumentId(1),
            p(0.0, 5.0),
            1.0,
            SpaceId::Paper(cad_domain::LayoutId(0)),
        )
        .unwrap();
        assert!(paper.is_empty(), "{paper:?}");
    }

    #[test]
    fn controller_snap_hints_are_empty_without_a_live_cursor() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // No tool, no cursor: the explicit empty overlay, never a stale hint.
        assert!(controller.snap_hints_near_cursor().unwrap().is_empty());
    }

    #[test]
    fn controller_snap_hints_follow_the_measurement_cursor() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller
            .execute(Command {
                schema_version: 1,
                id: crate::CommandId::Measure,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            })
            .unwrap();
        // A running tool with no cursor yet still yields nothing.
        assert!(controller.snap_hints_near_cursor().unwrap().is_empty());

        // The demo line starts at (0,0); a cursor just off that end resolves a
        // real endpoint hint through the viewport's world_per_px.
        controller
            .session
            .set_measurement_cursor(Some(p(0.0, 5.0)))
            .unwrap();
        let hints = controller.snap_hints_near_cursor().unwrap();
        assert!(
            hints
                .iter()
                .any(|c| { c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0) }),
            "{hints:?}"
        );
    }
}
