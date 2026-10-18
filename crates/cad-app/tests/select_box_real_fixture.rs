//! Window/crossing box selection against the committed four-line DWG
//! (`fixtures/dwg/synthetic-four-lines.dwg`), at the metrics of a real GUI run.
//!
//! This pins the **command** level: it loads the real fixture, builds the same
//! plan camera the desktop run reported (centre `(1.5, 0.5)`, ~0.005032 world
//! units per logical pixel, 926x341 logical canvas) and dispatches
//! `CommandId::SelectInBox` directly for WINDOW and CROSSING rectangles. It
//! exists because a real GUI run read `wide_window_count=0` where the rectangle
//! fully contains the drawing; this contract proves the predicate itself is
//! correct and localises that defect to the host gesture path, not here.
//!
//! It is a synthetic command contract, not an end-to-end/visual validation.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::Arc;

use cad_app::camera::{Camera, Projection};
use cad_app::interaction::SelectionMode;
use cad_app::{
    AppMode, Application, Command, CommandId, CommandPayload, Document, SessionState, Viewport,
};
use cad_domain::*;
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_spatial::pick_world_bounds;

const DOC: DocumentId = DocumentId(1);
const VIEWPORT: ViewportId = ViewportId(1);
/// The `.native-input` rect of the real GUI run.
const CANVAS: [f64; 2] = [926.0, 341.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn fixture_path() -> Option<PathBuf> {
    let path = std::env::var_os("YACR_TEST_DWG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .join("fixtures")
                .join("dwg")
                .join("synthetic-four-lines.dwg")
        });
    path.is_file().then_some(path)
}

fn load() -> Option<(Application, SessionState)> {
    let path = fixture_path()?;
    let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
    let request = ImportRequest {
        document: DOC,
        database: DatabaseId(1),
        bytes,
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("import the four-line fixture");
    let database = imported.database;

    // The entity world AABBs and the drawing extent the WINDOW predicate sees.
    for entity in database.model_space() {
        eprintln!(
            "select_box_real_fixture entity={:?} geometry={:?} bounds={:?}",
            entity.id,
            entity.geometry,
            pick_world_bounds(&entity.geometry, &Transform3::identity())
        );
    }
    eprintln!(
        "select_box_real_fixture drawing bounds={:?}",
        database.bounds()
    );

    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(database),
            identity: DocumentIdentity::Temporary(1),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    let mut viewport = Viewport::new(VIEWPORT, DOC, CANVAS);
    viewport.camera = Camera {
        eye: Point3 {
            x: 1.5,
            y: 0.5,
            z: 1000.0,
        },
        target: Point3 {
            x: 1.5,
            y: 0.5,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: Projection::Orthographic { scale: 0.005032 },
    };
    app.workspace.viewports.insert(VIEWPORT, viewport);
    Some((app, SessionState::new(DOC, AppMode::Work)))
}

fn box_command(rect_logical: [[f64; 2]; 2], crossing: bool) -> Command {
    Command {
        schema_version: 1,
        id: CommandId::SelectInBox,
        document: DOC,
        viewport: VIEWPORT,
        payload: CommandPayload::BoxSelect {
            rect_logical,
            crossing,
            mode: SelectionMode::Replace,
        },
    }
}

/// The `select.box.count` number and precision of one box dispatch.
fn count_and_precision(
    app: &mut Application,
    session: &mut SessionState,
    rect: [[f64; 2]; 2],
    crossing: bool,
) -> (usize, String) {
    let outcome = app.execute(session, box_command(rect, crossing)).unwrap();
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "select.box.count")
        .expect("select.box.count diagnostic");
    let count = diagnostic
        .message
        .split(';')
        .find_map(|field| field.strip_prefix("count="))
        .expect("count field")
        .parse()
        .expect("numeric count");
    let precision = diagnostic
        .message
        .split(';')
        .find_map(|field| field.strip_prefix("precision="))
        .expect("precision field")
        .to_string();
    (count, precision)
}

/// The world rectangle the command derives from a logical rectangle at these
/// metrics, and the fixture entities it therefore contains.
#[test]
fn a_full_canvas_window_selects_every_line_in_the_real_fixture() {
    let Some((mut app, mut session)) = load() else {
        eprintln!("SKIP: no four-line fixture (set YACR_TEST_DWG)");
        return;
    };
    // (a) WINDOW over nearly the whole canvas: must contain all four lines.
    let (count, precision) =
        count_and_precision(&mut app, &mut session, [[0.0, 0.0], CANVAS], false);
    eprintln!("select_box_real_fixture full-canvas WINDOW count={count} precision={precision}");
    assert_eq!(count, 4, "a full-canvas WINDOW contains the whole drawing");
    assert_eq!(precision, "window");
}

/// The near-full-canvas rectangle (2% -> 98%) from the GUI run also contains
/// the drawing; a WINDOW dispatch returns all four lines, never zero.
#[test]
fn the_near_full_canvas_window_from_the_gui_run_selects_the_drawing() {
    let Some((mut app, mut session)) = load() else {
        eprintln!("SKIP: no four-line fixture (set YACR_TEST_DWG)");
        return;
    };
    let rect = [
        [CANVAS[0] * 0.02, CANVAS[1] * 0.02],
        [CANVAS[0] * 0.98, CANVAS[1] * 0.98],
    ];
    let (count, precision) = count_and_precision(&mut app, &mut session, rect, false);
    eprintln!(
        "select_box_real_fixture 2%->98% WINDOW (rect_logical={rect:?}) count={count} precision={precision}"
    );
    assert_eq!(count, 4);
    assert_eq!(precision, "window");
}

/// The 25% -> 60% WINDOW rectangle genuinely does not fully contain any line
/// (each spans y = 0..1, the rectangle's top edge is at ~0.33 world), so a zero
/// WINDOW result there is correct geometry, not a defect.
#[test]
fn a_partial_window_rectangle_contains_no_line() {
    let Some((mut app, mut session)) = load() else {
        eprintln!("SKIP: no four-line fixture (set YACR_TEST_DWG)");
        return;
    };
    let rect = [
        [CANVAS[0] * 0.25, CANVAS[1] * 0.25],
        [CANVAS[0] * 0.60, CANVAS[1] * 0.60],
    ];
    let (count, _) = count_and_precision(&mut app, &mut session, rect, false);
    eprintln!("select_box_real_fixture 25%->60% WINDOW count={count}");
    assert_eq!(count, 0);
}

/// The same 60% -> 25% rectangle as CROSSING selects the one line it
/// intersects (x = 1); this is the path the GUI run showed working.
#[test]
fn the_crossing_rectangle_selects_the_line_it_intersects() {
    let Some((mut app, mut session)) = load() else {
        eprintln!("SKIP: no four-line fixture (set YACR_TEST_DWG)");
        return;
    };
    let rect = [
        [CANVAS[0] * 0.60, CANVAS[1] * 0.60],
        [CANVAS[0] * 0.25, CANVAS[1] * 0.25],
    ];
    let (count, precision) = count_and_precision(&mut app, &mut session, rect, true);
    eprintln!("select_box_real_fixture 60%->25% CROSSING count={count} precision={precision}");
    assert_eq!(count, 1);
    assert_eq!(precision, "exact");
}
