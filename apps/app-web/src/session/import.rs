//! The geometry half of an open: DWG bytes → imported `Database` → display
//! representation (a [`SceneDelta`]) → fitted camera.
//!
//! This stage touches **no GPU object and no session state**, which is what lets
//! `open_bytes` report a bad, unsupported, empty or over-budget file without
//! disturbing the session that is already open: the live renderer, its presenter
//! and its canvas pixels are untouched until the geometry has loaded *and* both
//! memory budgets have admitted it.
//!
//! # What is released, and why that is honest
//!
//! The imported `Database` is needed exactly twice — `build_scene` and the
//! one-off camera fit from its bounds — and is then dropped here, before any GPU
//! object exists. `budget::DB_RETAINED` is `false` and the status says
//! `db_retained: false`; a capability that needs the database later (save,
//! measure, entity query) must flip that constant instead of quietly keeping a
//! second copy of the document alive.
//!
//! The display representation itself (`SceneDelta`) is the one thing that must
//! survive into the upload, and it is handed to [`super::upload`], which releases
//! each batch's CPU geometry as soon as that batch is on the device.

use std::sync::Arc;

use cad_app::render_scene::build_scene;
use cad_domain::{DatabaseId, DocumentId, Point3, TaskStamp};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_scene::SceneDelta;

use super::session_state::{
    Failure, SESSION_DATABASE, SESSION_DOCUMENT, STEP_EMPTY_SCENE, STEP_IMPORT, STEP_SCENE,
};

/// The geometry half of an open: what [`load`] produced, before any GPU object
/// exists.
pub(super) struct Loaded {
    /// The display representation batches (`added` is what gets uploaded, in
    /// bounded chunks; see [`super::upload`]).
    pub(super) delta: SceneDelta,
    /// The camera fitted to the drawing bounds at the requested CSS size.
    pub(super) camera: cad_app::Camera,
    /// Source DWG length in bytes.
    pub(super) bytes: usize,
    /// Import + scene build duration in milliseconds.
    pub(super) import_ms: f64,
}

/// DWG `bytes` → imported database → display representation.
///
/// `label` only names the file in diagnostics; `logical` is the CSS-pixel canvas
/// size the camera is fitted to. Errors are [`Failure`]s carrying the stable step
/// (`import`, `scene`, `empty-scene`) so the caller can report them without
/// parsing the message.
pub(super) fn load(bytes: &[u8], label: &str, logical: [f64; 2]) -> Result<Loaded, Failure> {
    let started = web_time::Instant::now();
    if bytes.is_empty() {
        return Err(Failure::new(
            STEP_IMPORT,
            format!("{label}: the file is empty (0 bytes)"),
        ));
    }
    let drawing = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(SESSION_DOCUMENT),
                database: DatabaseId(SESSION_DATABASE),
                bytes: Arc::from(bytes),
                limits: ImportLimits::default(),
                generation: 0,
            },
            &|| false,
        )
        .map_err(|error| Failure::new(STEP_IMPORT, format!("{label}: {error}")))?;
    let stamp = TaskStamp::new(DocumentId(SESSION_DOCUMENT), 0);
    let delta = build_scene(&drawing.database, stamp)
        .map_err(|error| Failure::new(STEP_SCENE, format!("{label}: {error}")))?;
    if delta.added.is_empty() {
        // A parsed drawing with nothing drawable must not become a blank canvas
        // that looks like a successful open.
        return Err(Failure::new(
            STEP_EMPTY_SCENE,
            format!("{label}: the drawing produced no draw batches"),
        ));
    }
    // The camera is fitted once from the drawing bounds, exactly like the
    // engine's initial fit; after that only pan/zoom move it.
    let camera = fit_camera(&drawing.database, logical);
    // Release the imported document now, while it is still the only owner: the
    // display representation above is complete and neither the upload nor the
    // frame path reads the database. Keeping it would be an unexplained second
    // copy of the file in the wasm heap (`budget::DB_RETAINED` documents the
    // rule; the status reports `db_retained: false`). The explicit `drop` is the
    // release point, so removing it is a visible decision rather than a silent
    // one.
    drop(drawing);
    let import_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(Loaded {
        delta,
        camera,
        bytes: bytes.len(),
        import_ms,
    })
}

/// The initial camera: the default 2D top view moved to the drawing centre with
/// the world-per-pixel scale that frames the bounds.
///
/// The arithmetic is the engine's `fit_camera` rule (bounding-box centre, the
/// larger of the two axis ratios, 1.05 margin), and the projection goes through
/// the validated `Projection::orthographic` constructor. Non-finite bounds keep
/// the default top view rather than producing a NaN scale.
fn fit_camera(database: &cad_db::DrawingDatabase, logical: [f64; 2]) -> cad_app::Camera {
    let mut camera = cad_app::Camera::top_view_2d();
    let Some((min, max)) = database.bounds() else {
        return camera;
    };
    let extent_x = (max.x - min.x).max(1e-6);
    let extent_y = (max.y - min.y).max(1e-6);
    let center = Point3 {
        x: (min.x + max.x) * 0.5,
        y: (min.y + max.y) * 0.5,
        z: 0.0,
    };
    let world_per_px = (extent_x / logical[0].max(1.0)).max(extent_y / logical[1].max(1.0)) * 1.05;
    if !(center.x.is_finite() && center.y.is_finite()) {
        return camera;
    }
    // Shift both eye and target, so the view direction and the eye distance of
    // the top view are preserved (the 2D path only reads the target centre and
    // the projection scale).
    let offset = Point3 {
        x: center.x - camera.target.x,
        y: center.y - camera.target.y,
        z: 0.0,
    };
    camera.target = Point3 {
        x: camera.target.x + offset.x,
        y: camera.target.y + offset.y,
        z: camera.target.z,
    };
    camera.eye = Point3 {
        x: camera.eye.x + offset.x,
        y: camera.eye.y + offset.y,
        z: camera.eye.z,
    };
    if let Ok(projection) = cad_app::Projection::orthographic(world_per_px) {
        camera.projection = projection;
    }
    camera
}
