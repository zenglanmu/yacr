//! `ZOOM OBJECT`: frame the current selection (2D orthographic only).
//!
//! The contract pinned here: the selection is the input (not a payload point),
//! model space resolves block-instance members through the `INSERT`-expanded
//! pick items and a selected `INSERT` frames its members, paper space frames
//! only directly addressable layout entities, every refusal is named and changes
//! nothing, and a real object zoom records exactly one `ZOOM PREVIOUS` step
//! without writing the database or the undo history.

use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout};

fn viewport(app: &Application) -> &Viewport {
    &app.workspace.viewports[&ViewportId(1)]
}

fn camera(app: &Application) -> Camera {
    viewport(app).camera
}

fn scale(app: &Application) -> f64 {
    match viewport(app).camera.projection {
        Projection::Orthographic { scale } => scale,
        Projection::Perspective { .. } => panic!("expected the 2D plan projection"),
    }
}

fn object(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Zoom, CommandPayload::ZoomObject),
    )
}

fn model_ref(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn block_member_ref(insert: u128, member: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(member),
        instance: InstancePath(vec![EntityId(insert)]),
        sub_element: None,
    }
}

fn entity(id: u128, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

/// A drawing with a model LINE `(0,0)-(10,0)`, a CIRCLE at `(100,100)` radius 4,
/// a block `BOX` (one member LINE `(0,0)-(1,0)`) and a model INSERT at `(50,50)`.
fn app_with_selectables() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(
        10,
        SpaceId::Model,
        line(p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
    ))
    .unwrap();
    b.insert_entity(entity(
        11,
        SpaceId::Model,
        SemanticGeometry::Circle {
            center: p3(100.0, 100.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 4.0,
        },
    ))
    .unwrap();
    b.insert_entity(entity(
        20,
        SpaceId::Block(BlockId(1)),
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
    ))
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(20)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(entity(
        21,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::translation(p3(50.0, 50.0, 0.0)),
        },
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

/// A drawing with one paper layout and one paper LINE `(0,0)-(10,0)` on it.
fn app_with_paper_entity() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(entity(
        30,
        SpaceId::Paper(LayoutId(1)),
        line(p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn object_frames_a_selected_line_and_records_one_step() {
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([model_ref(10)]);
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let generation = session.generation;

    object(&mut app, &mut session).expect("ZOOM OBJECT frames the selected line");

    let viewport = viewport(&app);
    assert!((viewport.camera.target.x - 5.0).abs() < 1e-9);
    assert!(viewport.camera.target.y.abs() < 1e-9);
    assert_eq!(viewport.view_mode.kind(), ProjectionKind::TwoD);
    assert_eq!(viewport.view_mode.saved_2d_camera(), viewport.camera);
    // The line is 10 units wide on an 800x600 canvas, with the FIT margin.
    assert!(
        (scale(&app) - (10.0 / 800.0) * FIT_MARGIN).abs() < 1e-9,
        "scale {}",
        scale(&app)
    );
    assert_eq!(viewport.zoom_history_len(), 1);
    assert_eq!(session.generation, generation);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision
    );
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability::default()
    );
}

#[test]
fn object_frames_a_selected_circle() {
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([model_ref(11)]);
    object(&mut app, &mut session).expect("ZOOM OBJECT frames the selected circle");
    assert!((viewport(&app).camera.target.x - 100.0).abs() < 1e-9);
    assert!((viewport(&app).camera.target.y - 100.0).abs() < 1e-9);
    assert_eq!(viewport(&app).zoom_history_len(), 1);
}

#[test]
fn object_frames_a_selected_insert_and_its_member() {
    // Selecting the INSERT itself expands to the member placement.
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([model_ref(21)]);
    object(&mut app, &mut session).expect("ZOOM OBJECT frames the selected INSERT");
    assert!((viewport(&app).camera.target.x - 50.5).abs() < 1e-6);
    assert!((viewport(&app).camera.target.y - 50.0).abs() < 1e-9);
    let insert_frame = camera(&app);

    // Selecting the expanded member ref frames the same placement.
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([block_member_ref(21, 20)]);
    object(&mut app, &mut session).expect("ZOOM OBJECT frames the block member");
    assert!((viewport(&app).camera.target.x - 50.5).abs() < 1e-6);
    assert!((viewport(&app).camera.target.y - 50.0).abs() < 1e-9);
    assert_eq!(camera(&app), insert_frame);
}

#[test]
fn object_unions_multiple_selected_objects() {
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([model_ref(10), model_ref(11)]);
    object(&mut app, &mut session).expect("ZOOM OBJECT frames the union");
    // Union spans x 0..104, y 0..104: centre is (52, 52).
    assert!((viewport(&app).camera.target.x - 52.0).abs() < 1e-6);
    assert!((viewport(&app).camera.target.y - 52.0).abs() < 1e-6);
}

#[test]
fn an_empty_selection_is_a_named_refusal_and_changes_nothing() {
    let (mut app, mut session) = app_with_selectables();
    let before = camera(&app);
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let error = object(&mut app, &mut session).expect_err("ZOOM OBJECT needs a selection");
    assert!(
        error.to_string().contains("zoom.object.empty_selection"),
        "{error}"
    );
    assert_eq!(camera(&app), before);
    assert_eq!(viewport(&app).zoom_history_len(), 0);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision
    );
}

#[test]
fn a_three_d_view_refuses_object_by_name() {
    let (mut app, mut session) = app_with_selectables();
    session.selection = SelectionSet::from_refs([model_ref(10)]);
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let before = camera(&app);
    let steps = viewport(&app).zoom_history_len();

    let error = object(&mut app, &mut session).expect_err("a 3D view has no object frame");
    assert!(
        error.to_string().contains("zoom.object.requires_2d"),
        "{error}"
    );
    assert_eq!(camera(&app), before, "nothing moved");
    assert_eq!(viewport(&app).zoom_history_len(), steps);
}

#[test]
fn a_foreign_document_ref_is_refused() {
    let (mut app, mut session) = app_with_selectables();
    let foreign = SelectionRef {
        document: DocumentId(2),
        entity: EntityId(10),
        instance: InstancePath::default(),
        sub_element: None,
    };
    session.selection = SelectionSet::from_refs([foreign]);
    let before = camera(&app);
    let error = object(&mut app, &mut session).expect_err("a foreign ref is refused");
    assert!(
        error.to_string().contains("zoom.object.foreign_ref"),
        "{error}"
    );
    assert_eq!(camera(&app), before);
    assert_eq!(viewport(&app).zoom_history_len(), 0);
}

#[test]
fn paper_space_frames_an_addressable_layout_entity() {
    let (mut app, mut session) = app_with_paper_entity();
    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        ),
    )
    .unwrap();
    session.selection = SelectionSet::from_refs([model_ref(30)]);

    object(&mut app, &mut session).expect("ZOOM OBJECT frames the paper entity");
    assert!((viewport(&app).camera.target.x - 5.0).abs() < 1e-9);
    assert!(viewport(&app).camera.target.y.abs() < 1e-9);
    assert_eq!(viewport(&app).zoom_history_len(), 1);
}

#[test]
fn paper_space_refuses_block_instance_member_bounds() {
    let (mut app, mut session) = app_with_paper_entity();
    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        ),
    )
    .unwrap();
    session.selection = SelectionSet::from_refs([block_member_ref(21, 30)]);
    let before = camera(&app);
    let error = object(&mut app, &mut session).expect_err("paper space has no block-member bounds");
    assert!(
        error.to_string().contains("zoom.object.not_addressable"),
        "{error}"
    );
    assert_eq!(camera(&app), before);
}
