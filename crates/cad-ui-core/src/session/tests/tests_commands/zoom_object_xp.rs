//! `ZOOM OBJECT`, `ZOOM nX` and `ZOOM nXP` at the session dispatch layer.
//!
//! The grammars live in `arguments.rs`; these tests pin the routing the typed
//! line performs: `OBJECT` reaches `CommandPayload::ZoomObject` and reads the
//! current session selection, `nX` is the cursor-less factor step, and `nXP`
//! requires an active paper layout and then reuses the Work-only `VPSCALE`
//! write (one undo step, refused in Viewer mode).

use super::super::super::*;
use super::super::test_support::*;

use cad_app::SelectionSet;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
use cad_domain::{
    Completeness, DatabaseId, LayerId, LayoutId, ObjectId, Revision, SpaceId, Transform3,
};

const SHEET: LayoutId = LayoutId(2);

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    }
}

fn paper_viewport() -> PaperViewport {
    PaperViewport {
        clip: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 50.0,
                z: 0.0,
            },
        ],
        model_to_paper: Transform3::scale(100.0),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

/// The demo session replaced with one model-space LINE and one paper layout
/// `Sheet One` whose first viewport starts at `model_to_paper = 100`.
fn session_with_line_and_sheet() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(line(10)).unwrap();
    builder
        .insert_layout(Layout {
            id: SHEET,
            name: "Sheet One".into(),
            viewports: vec![paper_viewport()],
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    {
        let mut controller = session.core.controller.borrow_mut();
        let document_id = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document_id)
            .expect("the session document")
            .drawing = Arc::new(drawing);
        controller.session.selection = SelectionSet::new();
    }
    session
}

fn select_line(session: &CadSession) {
    let mut controller = session.core.controller.borrow_mut();
    let document = controller.document_id;
    controller.session.selection = SelectionSet::from_refs([SelectionRef {
        document,
        entity: EntityId(10),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
}

fn camera(session: &CadSession) -> CadCamera {
    session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view stays a 2D plan")
}

fn zoom_history_len(session: &CadSession) -> usize {
    let controller = session.core.controller.borrow();
    controller.application.workspace.viewports[&controller.viewport_id].zoom_history_len()
}

fn revision(session: &CadSession) -> Revision {
    let controller = session.core.controller.borrow();
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .revision()
}

fn viewport_scale(session: &CadSession) -> f64 {
    let controller = session.core.controller.borrow();
    let document = &controller.application.workspace.documents[&controller.document_id];
    document.drawing.layout(SHEET).unwrap().viewports[0]
        .model_to_paper
        .matrix[0][0]
}

fn undo_available(session: &CadSession) -> bool {
    let controller = session.core.controller.borrow();
    controller.application.can_undo(&controller.document_id)
}

fn redo_available(session: &CadSession) -> bool {
    let controller = session.core.controller.borrow();
    controller.application.can_redo(&controller.document_id)
}

#[test]
fn zoom_object_requires_a_selection_and_uses_it_when_present() {
    let session = session_with_line_and_sheet();
    let error = session
        .command("ZOOM OBJECT")
        .expect_err("an empty selection is refused");
    assert!(error.contains("zoom.object.empty_selection"), "{error}");

    select_line(&session);
    let before = camera(&session);
    let steps = zoom_history_len(&session);
    session
        .command("ZOOM OBJECT")
        .expect("ZOOM OBJECT frames the selected line");
    let after = camera(&session);
    assert_ne!(after, before, "the selection frame moved the camera");
    // The line spans (0,0)-(10,0): the frame centre is its midpoint.
    assert!((after.center[0] - 5.0).abs() < 1e-9, "{:?}", after.center);
    assert!(after.center[1].abs() < 1e-9, "{:?}", after.center);
    assert_eq!(
        zoom_history_len(&session),
        steps + 1,
        "one ZOOM PREVIOUS step"
    );
    assert!(!undo_available(&session), "a view move is not an undo step");
}

#[test]
fn zoom_relative_factor_is_the_cursor_less_step_and_records_one_step() {
    let session = session_with_line_and_sheet();
    let base = camera(&session).scale;
    let steps = zoom_history_len(&session);
    session.command("ZOOM 2X").expect("ZOOM 2X is admitted");
    let zoomed = camera(&session).scale;
    assert!(
        (zoomed - base / 2.0).abs() < 1e-9,
        "a relative factor > 1 zooms in: {base} -> {zoomed}"
    );
    assert_eq!(
        zoom_history_len(&session),
        steps + 1,
        "one ZOOM PREVIOUS step"
    );
    assert!(!undo_available(&session), "nX never writes the drawing");
}

#[test]
fn zoom_paper_scale_requires_paper_space() {
    let session = session_with_line_and_sheet();
    let before = revision(&session);
    let error = session
        .command("ZOOM 2XP")
        .expect_err("model space has no paper viewport scale");
    assert!(error.contains("zoom.xp.requires_paper"), "{error}");
    assert_eq!(revision(&session), before, "nothing was written");
    assert!(!undo_available(&session));
}

#[test]
fn zoom_paper_scale_writes_the_first_viewport_as_one_undo_step() {
    let session = session_with_line_and_sheet();
    session
        .command("LAYOUT SET Sheet One")
        .expect("the paper layout becomes active");
    let before = revision(&session);

    session
        .command("ZOOM 2XP")
        .expect("ZOOM 2XP routes through VPSCALE");
    assert_eq!(viewport_scale(&session), 2.0, "2XP is VPSCALE 1:2");
    assert_eq!(revision(&session), Revision(before.0 + 1));
    assert!(undo_available(&session), "the scale write is undoable");

    session
        .command("UNDO")
        .expect("UNDO replays the scale write");
    assert_eq!(viewport_scale(&session), 100.0);
    assert!(redo_available(&session));

    session
        .command("REDO")
        .expect("REDO replays the scale write");
    assert_eq!(viewport_scale(&session), 2.0);

    // The ratio spelling maps to the same VPSCALE ratio.
    session.command("UNDO").expect("UNDO before the ratio form");
    session
        .command("ZOOM 1/50XP")
        .expect("ZOOM 1/50XP routes through VPSCALE");
    assert_eq!(viewport_scale(&session), 50.0, "1/50XP is VPSCALE 1:50");
}

#[test]
fn zoom_paper_scale_is_refused_in_viewer_mode() {
    let session = session_with_line_and_sheet();
    session
        .command("LAYOUT SET Sheet One")
        .expect("the paper layout becomes active");
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");
    let before = revision(&session);

    let error = session
        .command("ZOOM 2XP")
        .expect_err("a Viewer cannot write the viewport scale");
    assert!(!error.is_empty(), "an explicit refusal");
    assert_eq!(viewport_scale(&session), 100.0, "nothing was written");
    assert_eq!(revision(&session), before);
}

#[test]
fn zoom_dynamic_is_the_named_gap() {
    let session = session_with_line_and_sheet();
    let before = camera(&session);
    let error = session
        .command("ZOOM DYNAMIC")
        .expect_err("ZOOM DYNAMIC is not implemented");
    assert!(error.contains("zoom.dynamic_not_implemented"), "{error}");
    assert_eq!(camera(&session), before, "nothing changed");
}
