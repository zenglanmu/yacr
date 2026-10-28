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
//! The imported `Database` is needed exactly twice — `build_scene` and the camera
//! fit from its bounds — and is then dropped here, before any GPU object exists.
//! `budget::DB_RETAINED` is `false` and the status says `db_retained: false`; a
//! capability that needs the database later (save, measure, entity query) must
//! flip that constant instead of quietly keeping a second copy of the document
//! alive.
//!
//! What *is* retained is the pair of world points the fit was computed from
//! ([`DrawingBounds`]): two numbers the import already measured, not another copy
//! of the drawing. That is what lets the `FIT` selector re-run the same
//! [`fit_camera`] rule after the open without the database, and it is why
//! `shell_status().canFit` can be `true` for a live session while `entityCount`
//! stays `null`.
//!
//! The display representation itself (`SceneDelta`) is the one thing that must
//! survive into the upload, and it is handed to [`super::upload`], which releases
//! each batch's CPU geometry as soon as that batch is on the device.

use std::sync::Arc;

use cad_app::render_scene::build_scene;
use cad_domain::{DatabaseId, DocumentId, TaskStamp};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_scene::SceneDelta;

use super::session_state::{
    Failure, SESSION_DATABASE, SESSION_DOCUMENT, STEP_EMPTY_SCENE, STEP_IMPORT, STEP_SCENE,
};
use crate::view::{fit_camera, retain_bounds, DrawingBounds};

/// The geometry half of an open: what [`load`] produced, before any GPU object
/// exists.
pub(super) struct Loaded {
    /// The display representation batches (`added` is what gets uploaded, in
    /// bounded chunks; see [`super::upload`]).
    pub(super) delta: SceneDelta,
    /// The camera fitted to the drawing bounds at the requested CSS size.
    pub(super) camera: cad_app::Camera,
    /// The extents that fit used ([`DrawingBounds`]), or `None` when the imported
    /// database reported no bounds. This is what makes `FIT` possible after the
    /// open; `None` means there is nothing to refit and `canFit` stays `false`.
    pub(super) bounds: Option<DrawingBounds>,
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
    // engine's initial fit; after that only pan/zoom move it. The bounds
    // themselves are kept (two points) so `FIT` can re-run this same rule later.
    let bounds = retain_bounds(&drawing.database);
    let camera = fit_camera(bounds, logical);
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
        bounds,
        bytes: bytes.len(),
        import_ms,
    })
}
