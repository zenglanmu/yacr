//! The 2026-11-07 extras batch contract tests: `SKETCH`, `TRACE`, `MOCORO`,
//! `MSTRETCH`, `-PAN`, `VIEWRES`, `VSCURRENT`, `REGEN3` and `LINETYPE`.
//!
//! All are contract-level: they pin the one-transaction/one-undo discipline,
//! the declared geometry shape (LWPOLYLINE, never SKETCH/TRACE entity types),
//! the view-only discipline, and every named refusal.

use super::*;

use cad_db::{DbEntity, DbObject, LinetypePattern};

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_linetype(cad_db::LineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn drawing(app: &Application) -> &DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
}

fn entity_count(app: &Application) -> usize {
    drawing(app).entity_count()
}

fn reference(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn one_geometry(app: &Application) -> SemanticGeometry {
    let entity = drawing(app).model_space()[0].clone();
    entity.geometry
}

#[test]
fn sketch_commits_one_open_polyline_and_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Sketch,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 3.0)]),
    )
    .expect("SKETCH is admitted");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(entity_count(&app), 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = one_geometry(&app)
    else {
        panic!("SKETCH must store one LWPOLYLINE");
    };
    assert_eq!(points.len(), 3, "one vertex per sampled point");
    assert_eq!(bulges, vec![0.0; 3]);
    assert!(!closed, "SKETCH is an open polyline");
    assert!(app.can_undo(&DocumentId(1)), "one undo step is recorded");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 0, "the single undo removes the sketch");

    let short = execute(
        &mut app,
        &mut session,
        CommandId::Sketch,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    );
    assert!(short.is_err(), "SKETCH needs >= 2 points");
    let non_finite = execute(
        &mut app,
        &mut session,
        CommandId::Sketch,
        CommandPayload::Points(vec![point(0.0, 0.0), point(f64::NAN, 0.0)]),
    );
    assert!(non_finite.is_err(), "SKETCH refuses a non-finite point");
    assert_eq!(entity_count(&app), 0, "refusals write nothing");
}

#[test]
fn trace_commits_a_closed_four_vertex_band_and_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    execute(
        &mut app,
        &mut session,
        CommandId::Trace,
        CommandPayload::Trace {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
            width: 2.0,
        },
    )
    .expect("TRACE is admitted");
    let SemanticGeometry::Polyline {
        points,
        closed,
        bulges,
    } = one_geometry(&app)
    else {
        panic!("TRACE must store one LWPOLYLINE");
    };
    assert!(closed);
    assert_eq!(points.len(), 4);
    assert_eq!(bulges, vec![0.0; 4]);
    // Width 2 ⇒ half-width 1 perpendicular to +X ⇒ y = ±1 at both ends.
    let ys: Vec<f64> = points.iter().map(|p| p.y).collect();
    assert_eq!(ys, vec![1.0, 1.0, -1.0, -1.0]);
    assert_eq!(entity_count(&app), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 0);

    for width in [0.0, -1.0, f64::NAN] {
        let refused = execute(
            &mut app,
            &mut session,
            CommandId::Trace,
            CommandPayload::Trace {
                start: point(0.0, 0.0),
                end: point(10.0, 0.0),
                width,
            },
        );
        assert!(refused.is_err(), "TRACE refuses width {width}");
    }
    let degenerate = execute(
        &mut app,
        &mut session,
        CommandId::Trace,
        CommandPayload::Trace {
            start: point(1.0, 1.0),
            end: point(1.0, 1.0),
            width: 1.0,
        },
    );
    assert!(degenerate.is_err(), "TRACE refuses coincident endpoints");
}

#[test]
fn mocoro_modes_apply_the_existing_transform_paths() {
    let line = || SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(2.0, 0.0),
    };
    let (mut app, mut session) = app_with(vec![ent(1, line(), 0)]);
    session.selection = SelectionSet::from_refs(vec![reference(1)]);

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Move {
            delta: point(3.0, 4.0),
        }),
    )
    .expect("MOCORO MOVE is admitted");
    let SemanticGeometry::Line { start, end } = one_geometry(&app) else {
        panic!("expected the moved line");
    };
    assert_eq!((start.x, start.y), (3.0, 4.0));
    assert_eq!((end.x, end.y), (5.0, 4.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Rotate {
            base: point(3.0, 4.0),
            degrees: 90.0,
        }),
    )
    .expect("MOCORO ROTATE is admitted");
    let SemanticGeometry::Line { end, .. } = one_geometry(&app) else {
        panic!("expected the rotated line");
    };
    assert!((end.x - 3.0).abs() < 1e-9, "90° rotate puts the end on +Y");
    assert!((end.y - 6.0).abs() < 1e-9);

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Scale {
            base: point(3.0, 4.0),
            factor: 2.0,
        }),
    )
    .expect("MOCORO SCALE is admitted");
    let SemanticGeometry::Line { end, .. } = one_geometry(&app) else {
        panic!("expected the scaled line");
    };
    assert!(
        (end.y - 8.0).abs() < 1e-9,
        "doubling keeps the 90° direction"
    );

    assert_eq!(entity_count(&app), 1, "MOVE/ROTATE/SCALE keep one entity");
    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Copy {
            delta: point(10.0, 0.0),
        }),
    )
    .expect("MOCORO COPY is admitted");
    assert_eq!(
        entity_count(&app),
        2,
        "COPY retains the source and adds one"
    );

    session.selection.clear();
    let refused = execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Move {
            delta: point(1.0, 0.0),
        }),
    );
    assert!(refused.is_err(), "MOCORO needs a selection");
}

#[test]
fn mstretch_applies_the_planner_across_multiple_windows_in_one_undo_step() {
    let (mut app, mut session) = app_with(vec![ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    )]);
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    let logical = |world: Point3| vp.camera.world_to_screen(world, vp.logical_size).unwrap();
    // Two disjoint windows, one around each endpoint: the line is partial, so
    // each in-window endpoint moves and the segment between the windows does not
    // make the whole line "enclosed".
    let left = [logical(point(-1.0, -1.0)), logical(point(1.0, 1.0))];
    let right = [logical(point(9.0, -1.0)), logical(point(11.0, 1.0))];
    let depth_before = app.history.get(&DocumentId(1)).map(|h| h.undo_depth());
    execute(
        &mut app,
        &mut session,
        CommandId::MStretch,
        CommandPayload::MStretch {
            rects_logical: vec![left, right],
            base: point(0.0, 0.0),
            delta: point(0.0, 5.0),
        },
    )
    .expect("MSTRETCH is admitted");
    let SemanticGeometry::Line { start, end } = one_geometry(&app) else {
        panic!("expected the stretched line");
    };
    assert_eq!((start.x, start.y), (0.0, 5.0));
    assert_eq!((end.x, end.y), (10.0, 5.0));
    let depth_after = app.history.get(&DocumentId(1)).map(|h| h.undo_depth());
    assert_eq!(depth_after, depth_before.map(|d| d + 1).or(Some(1)));
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let SemanticGeometry::Line { start, end } = one_geometry(&app) else {
        panic!("expected the original line");
    };
    assert_eq!((start.y, end.y), (0.0, 0.0), "one undo restores both ends");
}

#[test]
fn typed_pan_is_view_only() {
    let (mut app, mut session) = application_with_document();
    let revision = drawing(&app).revision();
    let eye = app.workspace.viewports[&ViewportId(1)].camera.eye;
    execute(
        &mut app,
        &mut session,
        CommandId::Pan,
        CommandPayload::Points(vec![point(5.0, -3.0)]),
    )
    .expect("PAN is admitted");
    let moved = app.workspace.viewports[&ViewportId(1)].camera.eye;
    assert_eq!(moved.x, eye.x - 5.0);
    assert_eq!(moved.y, eye.y + 3.0);
    assert_eq!(drawing(&app).revision(), revision, "pan writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)), "pan records no undo step");
}

#[test]
fn viewres_is_a_session_percentage_with_named_range_refusals() {
    let (mut app, mut session) = application_with_document();
    let revision = drawing(&app).revision();
    execute(
        &mut app,
        &mut session,
        CommandId::ViewRes,
        CommandPayload::ViewRes(250),
    )
    .expect("VIEWRES is admitted");
    assert_eq!(session.view_extras.viewres_percent, 250);
    assert_eq!(drawing(&app).revision(), revision);
    assert!(!app.can_undo(&DocumentId(1)));
    for percent in [0, 1001] {
        let refused = execute(
            &mut app,
            &mut session,
            CommandId::ViewRes,
            CommandPayload::ViewRes(percent),
        );
        assert!(refused.is_err(), "VIEWRES {percent} is out of range");
    }
}

#[test]
fn vscurrent_stores_wireframe_and_refuses_other_styles_by_name() {
    let (mut app, mut session) = application_with_document();
    execute(
        &mut app,
        &mut session,
        CommandId::VisualStyle,
        CommandPayload::VisualStyle("wireframe".into()),
    )
    .expect("VSCURRENT WIREFRAME is admitted");
    assert_eq!(
        session.view_extras.visual_style,
        VisualStyleToken::Wireframe
    );
    let refused = execute(
        &mut app,
        &mut session,
        CommandId::VisualStyle,
        CommandPayload::VisualStyle("SHADED".into()),
    );
    let error = format!("{}", refused.expect_err("SHADED is not implemented"));
    assert!(
        error.contains("vscurrent.style_not_implemented"),
        "the refusal must be named: {error}"
    );
}

#[test]
fn linetype_lists_and_sets_the_selection_then_refuses_unknown_names() {
    let (mut app, mut session) = app_with(vec![ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    )]);
    let listed = execute(
        &mut app,
        &mut session,
        CommandId::Linetype,
        CommandPayload::Linetype(LinetypeCommand::List),
    )
    .expect("LINETYPE LIST is admitted");
    assert!(
        listed
            .diagnostics
            .iter()
            .any(|d| d.code == "linetype.list" && d.message.contains("DASHED")),
        "LIST reports the drawing's linetypes"
    );
    assert!(!app.can_undo(&DocumentId(1)));

    session.selection = SelectionSet::from_refs(vec![reference(1)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Linetype,
        CommandPayload::Linetype(LinetypeCommand::Set("DASHED".into())),
    )
    .expect("LINETYPE SET is admitted");
    let attributes = drawing(&app).entity_render_attributes(EntityId(1));
    match attributes.linetype {
        cad_db::EntityLineType::Explicit { ref name, .. } => assert_eq!(name, "DASHED"),
        other => panic!("expected an explicit linetype, got {other:?}"),
    }

    let unknown = execute(
        &mut app,
        &mut session,
        CommandId::Linetype,
        CommandPayload::Linetype(LinetypeCommand::Set("NOPE".into())),
    );
    let error = format!("{}", unknown.expect_err("unknown linetype is refused"));
    assert!(
        error.contains("property.unknown_linetype"),
        "the refusal must be named: {error}"
    );
}

#[test]
fn linetype_set_re_authorizes_work_mode_even_though_list_is_viewer_legal() {
    let (mut app, session) = app_with(vec![ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    )]);
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    viewer.selection = SelectionSet::from_refs(vec![reference(1)]);
    let listed = execute(
        &mut app,
        &mut viewer,
        CommandId::Linetype,
        CommandPayload::Linetype(LinetypeCommand::List),
    );
    assert!(listed.is_ok(), "LIST is read-only and Viewer-legal");
    let refused = execute(
        &mut app,
        &mut viewer,
        CommandId::Linetype,
        CommandPayload::Linetype(LinetypeCommand::Set("DASHED".into())),
    );
    assert!(
        matches!(refused, Err(CadError::PermissionDenied)),
        "SET re-authorizes the work-only property write"
    );
    let _ = session;
}
