//! Wave 4 real-edit contracts: `CENTERMARK`, `CENTERLINE`, `NCOPY`,
//! `COPYTOLAYER`, `-COPYTOLAYER`, `CDORDER`, `FLATTEN`, `XPLODE`, `BTRIM`,
//! `BEXTEND`, `CHANGE`, `CHPROP`.
//!
//! Each test asserts the **actual database effect** (entity count/layer/z/draw
//! order), exactly one transaction / one undo step for a write, and the Viewer
//! refusal for the Work-only gate. No command may fake success.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineWeight,
    EntityRenderAttributes, Layer,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform,
    }
}

fn part(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
    type_key: &str,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: order,
    }
}

fn drawing_of(app: &Application) -> &DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
}

fn revision(app: &Application) -> Revision {
    drawing_of(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_count(app: &Application) -> usize {
    drawing_of(app).model_space().len()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn build(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

#[test]
fn centermark_creates_two_crossing_lines_per_circle_and_undoes_once() {
    let (mut app, mut session) = build(
        vec![part(
            1,
            SemanticGeometry::Circle {
                center: p3(5.0, 5.0, 2.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 10.0,
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbCircle",
            0,
        )],
        Vec::new(),
    );
    let before = revision(&app);
    select(&mut session, &[1]);

    let outcome = run(&mut app, &mut session, ExtendedCommandId::CenterMark).unwrap();

    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(model_count(&app), 3, "two marks plus the source circle");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    let lines: Vec<_> = drawing_of(&app)
        .model_space()
        .into_iter()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Line { .. }))
        .map(|entity| entity.geometry.clone())
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.contains(&line(p3(4.0, 5.0, 2.0), p3(6.0, 5.0, 2.0))));
    assert!(lines.contains(&line(p3(5.0, 4.0, 2.0), p3(5.0, 6.0, 2.0))));
}

#[test]
fn centermark_refuses_a_non_circle_selection_by_name_and_writes_nothing() {
    let (mut app, mut session) = build(
        vec![part(
            1,
            line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            0,
        )],
        Vec::new(),
    );
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = run(&mut app, &mut session, ExtendedCommandId::CenterMark).unwrap_err();
    assert!(error.to_string().contains("centermark.unsupported"));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(model_count(&app), 1);
}

#[test]
fn centerline_creates_one_midway_line_spanning_the_overlap() {
    let (mut app, mut session) = build(
        vec![
            part(
                1,
                line(p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                0,
            ),
            part(
                2,
                line(p3(2.0, 4.0, 0.0), p3(8.0, 4.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                1,
            ),
        ],
        Vec::new(),
    );
    select(&mut session, &[1, 2]);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::CenterLine).unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(undo_depth(&app), 1);
    let created = drawing_of(&app)
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    assert_eq!(created.geometry, line(p3(2.0, 2.0, 0.0), p3(8.0, 2.0, 0.0)));
}

#[test]
fn centerline_refuses_non_parallel_lines_by_name() {
    let (mut app, mut session) = build(
        vec![
            part(
                1,
                line(p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                0,
            ),
            part(
                2,
                line(p3(0.0, 4.0, 0.0), p3(5.0, 9.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                1,
            ),
        ],
        Vec::new(),
    );
    let before = revision(&app);
    select(&mut session, &[1, 2]);
    let error = run(&mut app, &mut session, ExtendedCommandId::CenterLine).unwrap_err();
    assert!(error.to_string().contains("centerline.not_parallel"));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

/// Block 1 has one LINE member (0,0)-(1,0) and entity 1 is an INSERT of it.
fn block_app(transform: Transform3) -> (Application, SessionState) {
    build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, transform),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                1,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

#[test]
fn ncopy_copies_member_geometry_at_the_world_transform_and_keeps_the_source() {
    let (mut app, mut session) = block_app(Transform3::translation(p3(5.0, 5.0, 0.0)));
    select(&mut session, &[1]);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::NCopy).unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(model_count(&app), 2, "the source INSERT stays");
    let created = drawing_of(&app)
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    assert_eq!(created.geometry, line(p3(5.0, 5.0, 0.0), p3(6.0, 5.0, 0.0)));
}

#[test]
fn ncopy_refuses_a_non_insert_selection_by_name() {
    let (mut app, mut session) = build(
        vec![part(
            1,
            line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            0,
        )],
        Vec::new(),
    );
    select(&mut session, &[1]);
    let error = run(&mut app, &mut session, ExtendedCommandId::NCopy).unwrap_err();
    assert!(error.to_string().contains("ncopy.unsupported"));
}

#[test]
fn copytolayer_copies_the_rest_onto_the_first_entity_layer_without_touching_originals() {
    let (mut app, mut session) = build(
        vec![
            part(
                1,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(3),
                SpaceId::Model,
                "AcDbLine",
                0,
            ),
            part(
                2,
                line(p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                1,
            ),
            part(
                3,
                line(p3(0.0, 2.0, 0.0), p3(1.0, 2.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                2,
            ),
        ],
        Vec::new(),
    );
    select(&mut session, &[1, 2, 3]);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::CopyToLayer).unwrap();
    assert_eq!(outcome.objects.len(), 2, "the source entity is not copied");
    assert_eq!(model_count(&app), 5);
    assert_eq!(undo_depth(&app), 1);
    for id in [2u128, 3] {
        assert_eq!(
            drawing_of(&app).entity(EntityId(id)).unwrap().layer,
            LayerId(0),
            "original {id} keeps its layer"
        );
    }
    for object in &outcome.objects {
        let created = drawing_of(&app).entity(EntityId(object.0)).unwrap();
        assert_eq!(created.layer, LayerId(3));
    }
}

#[test]
fn dash_copytolayer_is_the_same_behaviour() {
    let (mut app, mut session) = build(
        vec![
            part(
                1,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(3),
                SpaceId::Model,
                "AcDbLine",
                0,
            ),
            part(
                2,
                line(p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                1,
            ),
        ],
        Vec::new(),
    );
    select(&mut session, &[1, 2]);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::DashCopyToLayer).unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(
        drawing_of(&app)
            .entity(EntityId(outcome.objects[0].0))
            .unwrap()
            .layer,
        LayerId(3)
    );
}

#[test]
fn cdorder_rewrites_draw_order_by_colour_index_in_one_step() {
    let (mut app, mut session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Red is drawn last, green first; CDORDER must put red first.
    for (id, rgb, order) in [
        (1u128, [255u8, 0, 0], 2i64),
        (2, [255, 255, 0], 1),
        (3, [0, 255, 0], 0),
    ] {
        b.insert_entity(part(
            id,
            line(p3(id as f64, 0.0, 0.0), p3(id as f64, 1.0, 0.0)),
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            order,
        ))
        .unwrap();
        b.set_entity_render_attributes(
            EntityId(id),
            EntityRenderAttributes {
                color: EntityColor::Explicit(rgb),
                ..EntityRenderAttributes::default()
            },
        )
        .unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    session.selection.clear();

    run(&mut app, &mut session, ExtendedCommandId::CDOrder).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(drawing_of(&app).entity(EntityId(1)).unwrap().draw_order, 0);
    assert_eq!(drawing_of(&app).entity(EntityId(2)).unwrap().draw_order, 1);
    assert_eq!(drawing_of(&app).entity(EntityId(3)).unwrap().draw_order, 2);
}

#[test]
fn flatten_sets_z_to_zero_and_refuses_mesh_by_name() {
    let (mut app, mut session) = build(
        vec![
            part(
                1,
                line(p3(0.0, 0.0, 5.0), p3(1.0, 0.0, 7.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbLine",
                0,
            ),
            part(
                2,
                SemanticGeometry::Point(p3(2.0, 2.0, 9.0)),
                LayerId(0),
                SpaceId::Model,
                "AcDbPoint",
                1,
            ),
        ],
        Vec::new(),
    );
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        drawing_of(&app).entity(EntityId(1)).unwrap().geometry,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))
    );
    assert_eq!(
        drawing_of(&app).entity(EntityId(2)).unwrap().geometry,
        SemanticGeometry::Point(p3(2.0, 2.0, 0.0))
    );

    // A MESH is refused by name and writes nothing.
    let mesh = SemanticGeometry::Mesh(cad_domain::Mesh {
        vertices: vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        ..Default::default()
    });
    let (mut app, mut session) = build(
        vec![part(1, mesh, LayerId(0), SpaceId::Model, "AcDbSubDMesh", 0)],
        Vec::new(),
    );
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap_err();
    assert!(error.to_string().contains("flatten.unsupported"));
    assert_eq!(revision(&app), before);
}

#[test]
fn xplode_reuses_explode_and_reports_the_attributes_gap() {
    let (mut app, mut session) = block_app(Transform3::identity());
    select(&mut session, &[1]);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::XPlode).unwrap();
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "xplode.attributes_not_implemented"),
        "the attributes/property gap must be reported"
    );
    assert_eq!(undo_depth(&app), 1);
    assert!(
        drawing_of(&app).entity(EntityId(1)).is_none(),
        "source deleted"
    );
    assert_eq!(model_count(&app), 1, "one produced LINE");
}

/// Block 1 has one vertical LINE boundary at `x`.
fn boundary_app(x: f64, target: SemanticGeometry) -> (Application, SessionState) {
    build(
        vec![
            part(
                100,
                line(p3(x, -10.0, 0.0), p3(x, 10.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, Transform3::identity()),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                1,
            ),
            part(2, target, LayerId(0), SpaceId::Model, "AcDbLine", 2),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

#[test]
fn btrim_trims_against_the_selected_insert_members() {
    let (mut app, mut session) = boundary_app(2.0, line(p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)));
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BTrim).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        drawing_of(&app).entity(EntityId(2)).unwrap().geometry,
        line(p3(2.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
        "the piece before the boundary is cut away"
    );
}

#[test]
fn bextend_extends_to_the_selected_insert_members() {
    let (mut app, mut session) = boundary_app(10.0, line(p3(3.0, 0.0, 0.0), p3(5.0, 0.0, 0.0)));
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BExtend).unwrap();
    assert_eq!(undo_depth(&app), 1);
    let extended = drawing_of(&app)
        .entity(EntityId(2))
        .unwrap()
        .geometry
        .clone();
    let (start, end) = match extended {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a LINE, got {other:?}"),
    };
    assert!((start.x - 3.0).abs() < 1e-9 && start.y.abs() < 1e-9);
    assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9);
}

#[test]
fn change_and_chprop_move_to_the_current_layer_and_reset_to_bylayer() {
    for ext in [ExtendedCommandId::Change, ExtendedCommandId::ChProp] {
        let (mut app, mut session) = application_with_document();
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
            b.insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
        }
        b.insert_entity(part(
            1,
            line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            LayerId(3),
            SpaceId::Model,
            "AcDbLine",
            0,
        ))
        .unwrap();
        b.set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::Explicit([255, 0, 0]),
                lineweight: EntityLineWeight::Explicit(0.5),
                ..EntityRenderAttributes::default()
            },
        )
        .unwrap();
        app.workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = Arc::new(b.finish().unwrap());
        assert_eq!(session.active_layer, LayerId(0));
        select(&mut session, &[1]);

        run(&mut app, &mut session, ext).unwrap();
        assert_eq!(undo_depth(&app), 1, "{ext:?} is one undo step");
        let entity = drawing_of(&app).entity(EntityId(1)).unwrap();
        assert_eq!(entity.layer, LayerId(0));
        let attributes = drawing_of(&app).entity_render_attributes(EntityId(1));
        assert_eq!(attributes.color, EntityColor::ByLayer);
        assert_eq!(attributes.lineweight, EntityLineWeight::ByLayer);
    }
}

#[test]
fn every_wave4_write_is_refused_in_viewer_mode_without_writing() {
    for ext in [
        ExtendedCommandId::CenterMark,
        ExtendedCommandId::CenterLine,
        ExtendedCommandId::NCopy,
        ExtendedCommandId::CopyToLayer,
        ExtendedCommandId::DashCopyToLayer,
        ExtendedCommandId::CDOrder,
        ExtendedCommandId::Flatten,
        ExtendedCommandId::XPlode,
        ExtendedCommandId::BTrim,
        ExtendedCommandId::BExtend,
        ExtendedCommandId::Change,
        ExtendedCommandId::ChProp,
    ] {
        let (mut app, mut session) = build(
            vec![part(
                1,
                SemanticGeometry::Circle {
                    center: p3(0.0, 0.0, 0.0),
                    normal: p3(0.0, 0.0, 1.0),
                    radius: 1.0,
                },
                LayerId(0),
                SpaceId::Model,
                "AcDbCircle",
                0,
            )],
            Vec::new(),
        );
        session.switch_mode(AppMode::Viewer).unwrap();
        let before = revision(&app);
        select(&mut session, &[1]);
        let error = run(&mut app, &mut session, ext).unwrap_err();
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{} must be Work-only",
            ext.selector()
        );
        assert_eq!(revision(&app), before, "{} writes nothing", ext.selector());
        assert_eq!(undo_depth(&app), 0, "{} records no history", ext.selector());
    }
}
