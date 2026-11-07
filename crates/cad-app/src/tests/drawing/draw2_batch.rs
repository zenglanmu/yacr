//! The 2026-11-07 draw/modify batch contract tests.
//!
//! One section per command; every write is one transaction / one undo step and
//! every refusal is checked to leave the revision and the undo stack untouched.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
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

fn build(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().map(|id| ref_of(*id)).collect::<Vec<_>>());
}

fn exec(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: Draw2Payload,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(id, CommandPayload::Draw2(Box::new(payload))),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .expect("entity must exist")
}

fn assert_refused_unchanged(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: Draw2Payload,
    code: &str,
) {
    let before = revision(app);
    let depth = undo_depth(app);
    let error = exec(app, session, id, payload).expect_err("must be refused");
    let text = error.to_string();
    assert!(text.contains(code), "expected {code}, got {text}");
    assert_eq!(
        revision(app),
        before,
        "a refusal must not bump the revision"
    );
    assert_eq!(undo_depth(app), depth, "a refusal must not record undo");
}

fn undo(app: &mut Application, session: &mut SessionState) {
    app.execute(session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
}

// ---------------------------------------------------------------- DONUT ----

#[test]
fn donut_creates_two_concentric_circles_in_one_undo() {
    let (mut app, mut session) = build(Vec::new());
    let before = revision(&app);
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(2.0, 3.0, 0.0),
            inner: 3.0,
            outer: 5.0,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(
        changes.changes.len(),
        2,
        "two entity inserts in one transaction"
    );
    assert_eq!(undo_depth(&app), 1);

    let radii: Vec<f64> = outcome
        .objects
        .iter()
        .map(|object| match &entity(&app, object.0).geometry {
            SemanticGeometry::Circle { radius, .. } => *radius,
            other => panic!("expected a circle, got {other:?}"),
        })
        .collect();
    assert_eq!(radii, vec![5.0, 3.0], "outer first, inner second");
    let ids: Vec<u128> = outcome.objects.iter().map(|object| object.0).collect();
    undo(&mut app, &mut session);
    assert!(ids.iter().all(|id| app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(*id))
        .is_none()));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn donut_refuses_bad_radii_and_a_non_finite_centre() {
    let (mut app, mut session) = build(Vec::new());
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: -1.0,
            outer: 2.0,
        },
        "donut.invalid_radius",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: 2.0,
            outer: 2.0,
        },
        "donut.radius_order",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(f64::NAN, 0.0, 0.0),
            inner: 1.0,
            outer: 2.0,
        },
        "donut.invalid_point",
    );
}

#[test]
fn donut_zero_inner_stores_the_outer_circle_only() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: 0.0,
            outer: 4.0,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "donut.inner_zero"));
}

// ---------------------------------------------------------------- SOLID ----

#[test]
fn solid_creates_a_filled_triangle_and_a_quad() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid {
            points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(0.0, 4.0, 0.0)],
        },
    )
    .unwrap();
    let id = outcome.objects[0].0;
    let stored = entity(&app, id);
    assert_eq!(stored.object.type_key, "AcDbSolid");
    match stored.geometry {
        SemanticGeometry::Mesh(mesh) => {
            assert_eq!(mesh.vertices.len(), 3);
            assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
            assert_eq!(mesh.normals.len(), 3);
        }
        other => panic!("SOLID must store a Mesh, got {other:?}"),
    }

    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid { points: square() },
    )
    .unwrap();
    let id = outcome.objects[0].0;
    match entity(&app, id).geometry {
        SemanticGeometry::Mesh(mesh) => {
            assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 2, 3]]);
        }
        other => panic!("SOLID must store a Mesh, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 2);
}

fn square() -> Vec<Point3> {
    vec![
        p3(0.0, 0.0, 0.0),
        p3(4.0, 0.0, 0.0),
        p3(4.0, 4.0, 0.0),
        p3(0.0, 4.0, 0.0),
    ]
}

#[test]
fn solid_refuses_point_count_crossing_planarity_and_area() {
    let (mut app, mut session) = build(Vec::new());
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid {
            points: vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
        },
        "solid.point_count",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid {
            points: vec![
                p3(0.0, 0.0, 0.0),
                p3(4.0, 4.0, 0.0),
                p3(4.0, 0.0, 0.0),
                p3(0.0, 4.0, 0.0),
            ],
        },
        "solid.self_crossing",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid {
            points: vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 1.0), p3(0.0, 2.0, 0.0)],
        },
        "solid.not_planar",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Solid,
        Draw2Payload::Solid {
            points: vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0), p3(2.0, 2.0, 0.0)],
        },
        "solid.zero_area",
    );
}

// ---------------------------------------------------------------- HELIX ----

#[test]
fn helix_builds_a_flat_spiral_with_the_requested_turns() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Helix,
        Draw2Payload::Helix {
            base: p3(1.0, 2.0, 0.0),
            radius: 3.0,
            turns: 2.0,
            height: 0.0,
        },
    )
    .unwrap();
    let stored = entity(&app, outcome.objects[0].0);
    match stored.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points.len(), 2 * 64 + 1, "64 segments per turn");
            assert_eq!(bulges.len(), points.len());
            assert!(!closed);
            assert!((points[0].x - 1.0).abs() < 1e-12);
            assert!((points[0].y - 2.0).abs() < 1e-12);
            let last = points[points.len() - 1];
            assert!((last.x - 4.0).abs() < 1e-9, "spiral ends at base + radius");
            assert!((last.y - 2.0).abs() < 1e-9);
        }
        other => panic!("HELIX must store a Polyline, got {other:?}"),
    }
}

#[test]
fn helix_refuses_degenerate_inputs_and_the_3d_spring() {
    let (mut app, mut session) = build(Vec::new());
    for (radius, turns, height, code) in [
        (0.0, 1.0, 0.0, "helix.invalid_radius"),
        (1.0, 0.0, 0.0, "helix.invalid_turns"),
        (1.0, 1.0, -1.0, "helix.invalid_height"),
        (1.0, 1.0, 2.0, "helix.3d_not_implemented"),
    ] {
        let before = revision(&app);
        let error = exec(
            &mut app,
            &mut session,
            CommandId::Helix,
            Draw2Payload::Helix {
                base: p3(0.0, 0.0, 0.0),
                radius,
                turns,
                height,
            },
        )
        .expect_err("must be refused");
        assert!(error.to_string().contains(code), "{}", error);
        assert_eq!(revision(&app), before);
    }
}

// ---------------------------------------------------------------- MLINE ----

#[test]
fn mline_creates_two_parallel_lines_at_the_spacing() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Mline,
        Draw2Payload::Mline {
            spacing: Some(2.0),
            offset: 0.0,
            points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    let mut ys: Vec<f64> = outcome
        .objects
        .iter()
        .map(|object| match entity(&app, object.0).geometry {
            SemanticGeometry::Line { start, .. } => start.y,
            other => panic!("expected a line, got {other:?}"),
        })
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(ys, vec![-1.0, 1.0]);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn mline_reads_the_current_mlstyle_spacing_and_applies_the_offset() {
    let (mut app, mut session) = build(Vec::new());
    exec(
        &mut app,
        &mut session,
        CommandId::MlStyle,
        Draw2Payload::MlStyle(MlStyleCommand::Set {
            name: "WALL".into(),
            spacing: 3.0,
        }),
    )
    .unwrap();
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Mline,
        Draw2Payload::Mline {
            spacing: None,
            offset: 0.5,
            points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
        },
    )
    .unwrap();
    let mut ys: Vec<f64> = outcome
        .objects
        .iter()
        .map(|object| match entity(&app, object.0).geometry {
            SemanticGeometry::Line { start, .. } => start.y,
            other => panic!("expected a line, got {other:?}"),
        })
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!((ys[0] - (-1.0)).abs() < 1e-12, "got {ys:?}");
    assert!((ys[1] - 2.0).abs() < 1e-12, "got {ys:?}");
}

#[test]
fn mline_refuses_a_degenerate_baseline_and_bad_spacing() {
    let (mut app, mut session) = build(Vec::new());
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Mline,
        Draw2Payload::Mline {
            spacing: Some(0.0),
            offset: 0.0,
            points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
        },
        "mline.invalid_spacing",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Mline,
        Draw2Payload::Mline {
            spacing: Some(1.0),
            offset: 0.0,
            points: vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 0.0)],
        },
        "mline.degenerate_baseline",
    );
}

// -------------------------------------------------------------- MLSTYLE ----

#[test]
fn mlstyle_lists_and_sets_bounded_session_styles() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::MlStyle,
        Draw2Payload::MlStyle(MlStyleCommand::List),
    )
    .unwrap();
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("STANDARD")));

    exec(
        &mut app,
        &mut session,
        CommandId::MlStyle,
        Draw2Payload::MlStyle(MlStyleCommand::Set {
            name: "WALL".into(),
            spacing: 2.5,
        }),
    )
    .unwrap();
    assert_eq!(session.ml_styles.current_name(), "WALL");
    assert_eq!(session.ml_styles.current_spacing(), 2.5);
    // A style store is session state: no drawing write, no undo entry.
    assert_eq!(undo_depth(&app), 0);

    for index in 0..14 {
        exec(
            &mut app,
            &mut session,
            CommandId::MlStyle,
            Draw2Payload::MlStyle(MlStyleCommand::Set {
                name: format!("S{index}"),
                spacing: 1.0,
            }),
        )
        .unwrap();
    }
    assert_eq!(session.ml_styles.styles().len(), crate::MULTILINE_STYLE_CAP);
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::MlStyle,
        Draw2Payload::MlStyle(MlStyleCommand::Set {
            name: "OVERFLOW".into(),
            spacing: 1.0,
        }),
        "mlstyle.capacity",
    );
}

// ----------------------------------------------------------------- TREX ----

fn cross_fixture() -> (Application, SessionState) {
    build(vec![
        ent(
            1,
            SemanticGeometry::Line {
                start: p3(0.0, 0.0, 0.0),
                end: p3(10.0, 0.0, 0.0),
            },
            0,
        ),
        ent(
            2,
            SemanticGeometry::Line {
                start: p3(2.0, -5.0, 0.0),
                end: p3(2.0, 5.0, 0.0),
            },
            1,
        ),
    ])
}

#[test]
fn trex_trims_the_selection_against_itself_in_one_undo() {
    let (mut app, mut session) = cross_fixture();
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Trex,
        Draw2Payload::Trex(TrexMode::Trim),
    )
    .unwrap();
    assert!(outcome.changes.is_some());
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1);
    let trimmed = entity(&app, 1);
    match trimmed.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 2.0).abs() < 1e-9, "kept the midpoint side");
            assert!((end.x - 10.0).abs() < 1e-9);
        }
        other => panic!("expected a line, got {other:?}"),
    }
    undo(&mut app, &mut session);
    match entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(start, p3(0.0, 0.0, 0.0));
            assert_eq!(end, p3(10.0, 0.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn trex_refuses_a_single_object() {
    let (mut app, mut session) = cross_fixture();
    select(&mut session, &[1]);
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Trex,
        Draw2Payload::Trex(TrexMode::Trim),
        "trex.selection",
    );
}

// --------------------------------------------------------------- EXTRIM ----

#[test]
fn extrim_trims_every_target_at_the_boundary_in_one_undo() {
    let (mut app, mut session) = build(vec![
        ent(
            10,
            SemanticGeometry::Circle {
                center: p3(0.0, 0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 5.0,
            },
            0,
        ),
        ent(
            1,
            SemanticGeometry::Line {
                start: p3(-10.0, 0.0, 0.0),
                end: p3(10.0, 0.0, 0.0),
            },
            1,
        ),
        ent(
            2,
            SemanticGeometry::Line {
                start: p3(-10.0, 1.0, 0.0),
                end: p3(10.0, 1.0, 0.0),
            },
            2,
        ),
    ]);
    let before = revision(&app);
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Extrim,
        Draw2Payload::Extrim {
            boundary: ref_of(10),
            targets: vec![ref_of(1), ref_of(2)],
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    for id in [1, 2] {
        match entity(&app, id).geometry {
            SemanticGeometry::Line { start, end } => {
                assert!(start.x.abs() < 5.0 + 1e-9, "trimmed inside the circle");
                assert!(end.x.abs() < 5.0 + 1e-9);
            }
            other => panic!("expected a line, got {other:?}"),
        }
    }
    undo(&mut app, &mut session);
    match entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(start.x, -10.0);
            assert_eq!(end.x, 10.0);
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extrim_refuses_an_unsupported_boundary_by_name() {
    let (mut app, mut session) = build(vec![
        ent(10, SemanticGeometry::Point(p3(0.0, 0.0, 0.0)), 0),
        ent(
            1,
            SemanticGeometry::Line {
                start: p3(-10.0, 0.0, 0.0),
                end: p3(10.0, 0.0, 0.0),
            },
            1,
        ),
    ]);
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Extrim,
        Draw2Payload::Extrim {
            boundary: ref_of(10),
            targets: vec![ref_of(1)],
        },
        "extrim.unsupported_boundary",
    );
}

// --------------------------------------------------------------- MPEDIT ----

#[test]
fn mpedit_closes_every_selected_polyline_and_skips_the_line() {
    let (mut app, mut session) = build(vec![
        ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
                bulges: vec![0.0; 3],
                closed: false,
            },
            0,
        ),
        ent(
            2,
            SemanticGeometry::Polyline {
                points: vec![p3(10.0, 0.0, 0.0), p3(14.0, 0.0, 0.0), p3(14.0, 4.0, 0.0)],
                bulges: vec![0.0; 3],
                closed: false,
            },
            1,
        ),
        ent(
            3,
            SemanticGeometry::Line {
                start: p3(0.0, -5.0, 0.0),
                end: p3(1.0, -5.0, 0.0),
            },
            2,
        ),
    ]);
    select(&mut session, &[1, 2, 3]);
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        Draw2Payload::Mpedit(PeditOption::Close),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "mpedit.skipped"));
    for id in [1, 2] {
        match entity(&app, id).geometry {
            SemanticGeometry::Polyline { closed, .. } => assert!(closed),
            other => panic!("expected a polyline, got {other:?}"),
        }
    }
    assert_eq!(undo_depth(&app), 1);
    undo(&mut app, &mut session);
    match entity(&app, 1).geometry {
        SemanticGeometry::Polyline { closed, .. } => assert!(!closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

// ------------------------------------------------------------- DRAWORDER ----

fn order_fixture() -> (Application, SessionState) {
    build(
        (1..=4)
            .map(|id| {
                ent(
                    id,
                    SemanticGeometry::Point(p3(id as f64, 0.0, 0.0)),
                    id as i64 - 1,
                )
            })
            .collect(),
    )
}

fn order(app: &Application, id: u128) -> i64 {
    entity(app, id).draw_order
}

#[test]
fn draworder_front_and_back_move_the_selection_as_a_block() {
    let (mut app, mut session) = order_fixture();
    select(&mut session, &[1, 2]);
    exec(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        Draw2Payload::DrawOrder(DrawOrderMode::Front),
    )
    .unwrap();
    assert_eq!((order(&app, 1), order(&app, 2)), (4, 5));
    assert_eq!(undo_depth(&app), 1);
    undo(&mut app, &mut session);
    assert_eq!((order(&app, 1), order(&app, 2)), (0, 1));

    select(&mut session, &[2, 3]);
    exec(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        Draw2Payload::DrawOrder(DrawOrderMode::Back),
    )
    .unwrap();
    assert_eq!((order(&app, 2), order(&app, 3)), (-2, -1));
}

#[test]
fn draworder_above_and_below_use_the_first_selected_as_reference() {
    let (mut app, mut session) = order_fixture();
    // ABOVE: reference id 2 (order 1), moving id 1 (order 0). The objects above
    // shift up to make room.
    select(&mut session, &[2, 1]);
    exec(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        Draw2Payload::DrawOrder(DrawOrderMode::Above),
    )
    .unwrap();
    assert_eq!(order(&app, 1), 2);
    assert_eq!(order(&app, 3), 3);
    assert_eq!(order(&app, 4), 4);
    assert_eq!(order(&app, 2), 1, "the reference does not move");
    undo(&mut app, &mut session);

    // BELOW: reference id 2 (order 1), moving id 3 (order 2). The objects below
    // shift down to make room.
    select(&mut session, &[2, 3]);
    exec(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        Draw2Payload::DrawOrder(DrawOrderMode::Below),
    )
    .unwrap();
    assert_eq!(order(&app, 3), 0);
    assert_eq!(order(&app, 1), -1);
    assert_eq!(order(&app, 2), 1);
}

#[test]
fn draworder_above_needs_a_reference_object() {
    let (mut app, mut session) = order_fixture();
    select(&mut session, &[1]);
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        Draw2Payload::DrawOrder(DrawOrderMode::Above),
        "draworder.reference_required",
    );
}
