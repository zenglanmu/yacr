//! Strict `CDORDER` / `BTRIM` / `BEXTEND` contracts (2026-11-24).
//!
//! Split out of `extended_edit4` for the 1000-line code gate. Pins:
//!
//! * `CDORDER` plans **per draw-order scope** (model space, each paper layout,
//!   each block definition): a no-selection run reorders every scope in one
//!   transaction / one undo step, a selection only touches the scopes it lives in,
//!   and paper-space rows are addressable instead of refused;
//! * `BTRIM` / `BEXTEND` accept exactly the standalone TRIM/EXTEND target set
//!   (LWPOLYLINE, HATCH and elliptical targets included) through the shared
//!   target dispatch, keep the first selected INSERT's members as the boundary,
//!   and refuse a target kind the planner cannot model **without writing**.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, Layer, Layout,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn part(
    id: u128,
    geometry: SemanticGeometry,
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
        layer: LayerId(0),
        space,
        geometry,
        draw_order: order,
    }
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

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn insert(block: u128) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform: Transform3::identity(),
        attributes: Vec::new(),
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

fn order_of(app: &Application, id: u128) -> i64 {
    drawing_of(app).entity(EntityId(id)).unwrap().draw_order
}

/// Build one document with `Sheet1`, block `B1` and the given rows/colours.
fn cdorder_app() -> (Application, SessionState) {
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
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(7), EntityId(8), EntityId(9)],
        dynamic_visibility: None,
    })
    .unwrap();
    // Each scope holds red (ACI 1), yellow (ACI 2) and green (ACI 3) drawn in
    // reverse colour order (green first, red last).
    let scopes = [
        (1u128, SpaceId::Model),
        (4, SpaceId::Paper(LayoutId(1))),
        (7, SpaceId::Block(BlockId(1))),
    ];
    for (base, space) in scopes {
        for (offset, rgb) in [(0u128, [255u8, 0, 0]), (1, [255, 255, 0]), (2, [0, 255, 0])] {
            let id = base + offset;
            b.insert_entity(part(
                id,
                line(p3(id as f64, 0.0, 0.0), p3(id as f64, 1.0, 0.0)),
                space.clone(),
                "AcDbLine",
                // red last (2), yellow 1, green 0
                2 - offset as i64,
            ))
            .unwrap();
            b.set_entity_render_attributes(
                EntityId(id),
                EntityRenderAttributes {
                    clip: None,
                    color: EntityColor::Explicit(rgb),
                    ..EntityRenderAttributes::default()
                },
            )
            .unwrap();
        }
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

#[test]
fn cdorder_reorders_model_paper_and_block_scopes_in_one_step() {
    let (mut app, mut session) = cdorder_app();
    session.selection.clear();
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::CDOrder).unwrap();
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    // Every scope: red (ACI 1) first, then yellow (2), then green (3).
    for (red, yellow, green) in [(1u128, 2, 3), (4, 5, 6), (7, 8, 9)] {
        assert_eq!(order_of(&app, red), 0, "red ACI 1 sorts first");
        assert_eq!(order_of(&app, yellow), 1);
        assert_eq!(order_of(&app, green), 2);
    }
    let message = &outcome.diagnostics[0].message;
    assert_eq!(outcome.diagnostics[0].code, "cdorder.applied");
    assert!(message.contains("model space: 2 moved"), "{message}");
    assert!(message.contains("layout 'Sheet1': 2 moved"), "{message}");
    assert!(message.contains("block #1: 2 moved"), "{message}");
    assert_eq!(
        outcome.objects.len(),
        6,
        "the yellow rows already had their slot"
    );
}

#[test]
fn cdorder_selection_reorders_paper_space_rows_without_touching_model_space() {
    let (mut app, mut session) = cdorder_app();
    // Layout sheet rows 4/5/6 (red last).
    select(&mut session, &[4, 5, 6]);
    run(&mut app, &mut session, ExtendedCommandId::CDOrder).unwrap();
    assert_eq!(order_of(&app, 4), 0);
    assert_eq!(order_of(&app, 5), 1);
    assert_eq!(order_of(&app, 6), 2);
    // Model and block scopes keep their stored order.
    assert_eq!(order_of(&app, 1), 2);
    assert_eq!(order_of(&app, 7), 2);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn cdorder_is_a_named_no_op_when_every_scope_is_already_ordered() {
    let (mut app, mut session) = cdorder_app();
    session.selection.clear();
    run(&mut app, &mut session, ExtendedCommandId::CDOrder).unwrap();
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, ExtendedCommandId::CDOrder).unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics[0].code, "cdorder.unchanged");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 1, "only the first run wrote");
}

/// Block `B1` holds the given member geometry; the entity is the boundary
/// reference plus the target rows.
fn boundary_app(
    members: Vec<SemanticGeometry>,
    targets: Vec<(u128, SemanticGeometry, &str)>,
) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut member_ids = Vec::new();
    for (index, geometry) in members.into_iter().enumerate() {
        let id = 100 + index as u128;
        member_ids.push(EntityId(id));
        b.insert_entity(part(
            id,
            geometry,
            SpaceId::Block(BlockId(1)),
            "AcDbLine",
            0,
        ))
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: member_ids,
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(1, insert(1), SpaceId::Model, "AcDbBlockReference", 0))
        .unwrap();
    for (order, (id, geometry, type_key)) in targets.into_iter().enumerate() {
        b.insert_entity(part(
            id,
            geometry,
            SpaceId::Model,
            type_key,
            order as i64 + 1,
        ))
        .unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

fn vertical_boundary(x: f64) -> SemanticGeometry {
    line(p3(x, -10.0, 0.0), p3(x, 10.0, 0.0))
}

#[test]
fn btrim_trims_an_lwpolyline_target_against_the_insert_members() {
    let (mut app, mut session) = boundary_app(
        vec![vertical_boundary(2.0)],
        vec![(
            2,
            SemanticGeometry::Polyline {
                points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
                width: 0.0,
            },
            "AcDbPolyline",
        )],
    );
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BTrim).unwrap();
    assert_eq!(undo_depth(&app), 1);
    match &drawing_of(&app).entity(EntityId(2)).unwrap().geometry {
        SemanticGeometry::Polyline { points, .. } => {
            assert!((points[0].x - 2.0).abs() < 1e-9, "{points:?}");
            assert!((points[1].x - 10.0).abs() < 1e-9, "{points:?}");
        }
        other => panic!("expected the trimmed polyline, got {other:?}"),
    }
}

#[test]
fn btrim_trims_an_ellipse_target_against_the_insert_members() {
    let (mut app, mut session) = boundary_app(
        vec![vertical_boundary(2.0)],
        vec![(
            2,
            SemanticGeometry::Ellipse {
                center: p3(0.0, 0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(5.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
            "AcDbEllipse",
        )],
    );
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BTrim).unwrap();
    assert_eq!(undo_depth(&app), 1);
    match &drawing_of(&app).entity(EntityId(2)).unwrap().geometry {
        SemanticGeometry::Ellipse { sweep, .. } => {
            assert!(
                sweep.abs() < std::f64::consts::TAU - 1e-6,
                "the full ellipse becomes an elliptical arc, got sweep {sweep}"
            );
        }
        other => panic!("expected the trimmed elliptical arc, got {other:?}"),
    }
}

#[test]
fn btrim_deletes_a_hatch_wholly_inside_the_insert_boundary() {
    // The block boundary is a closed polyline rectangle around the hatch loop.
    let boundary = SemanticGeometry::Polyline {
        points: vec![
            p3(-5.0, -5.0, 0.0),
            p3(15.0, -5.0, 0.0),
            p3(15.0, 15.0, 0.0),
            p3(-5.0, 15.0, 0.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    };
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            vec![0.0; 3],
            HatchLoopFlags(HatchLoopFlags::POLYLINE),
        )],
        HatchPlane::new(p3(0.0, 0.0, 1.0), 0.0),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    )
    .expect("fixture hatch");
    let (mut app, mut session) = boundary_app(
        vec![boundary],
        vec![(2, SemanticGeometry::Hatch(Box::new(hatch)), "AcDbHatch")],
    );
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BTrim).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert!(
        drawing_of(&app).entity(EntityId(2)).is_none(),
        "a hatch wholly inside the closed boundary is deleted"
    );
}

#[test]
fn btrim_refuses_an_insert_target_by_name_without_writing() {
    let (mut app, mut session) = boundary_app(
        vec![vertical_boundary(2.0)],
        vec![(2, insert(1), "AcDbBlockReference")],
    );
    let before = revision(&app);
    select(&mut session, &[1, 2]);
    let error = run(&mut app, &mut session, ExtendedCommandId::BTrim).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("btrim.target"), "{text}");
    assert!(text.contains("trim.target_insert"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn bextend_extends_an_elliptical_arc_to_the_insert_members() {
    // The block holds a horizontal line below the arc's far end; the elliptical
    // arc spans `0..π` so its picked end is at (-1, 0) and the line crosses the
    // ellipse at a parameter just past π.
    let (mut app, mut session) = boundary_app(
        vec![line(p3(-2.0, -0.3, 0.0), p3(2.0, -0.3, 0.0))],
        vec![(
            2,
            SemanticGeometry::Ellipse {
                center: p3(0.0, 0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(1.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: std::f64::consts::PI,
            },
            "AcDbEllipse",
        )],
    );
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, ExtendedCommandId::BExtend).unwrap();
    assert_eq!(undo_depth(&app), 1);
    match &drawing_of(&app).entity(EntityId(2)).unwrap().geometry {
        SemanticGeometry::Ellipse { sweep, .. } => {
            assert!(
                *sweep > std::f64::consts::PI + 1e-6,
                "the picked end grows past π, got sweep {sweep}"
            );
        }
        other => panic!("expected the extended elliptical arc, got {other:?}"),
    }
}

#[test]
fn bextend_refuses_a_polyline_target_by_name_without_writing() {
    let (mut app, mut session) = boundary_app(
        vec![vertical_boundary(2.0)],
        vec![(
            2,
            SemanticGeometry::Polyline {
                points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
                width: 0.0,
            },
            "AcDbPolyline",
        )],
    );
    let before = revision(&app);
    select(&mut session, &[1, 2]);
    let error = run(&mut app, &mut session, ExtendedCommandId::BExtend).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("bextend.target"), "{text}");
    assert!(text.contains("extend.target_polyline"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
