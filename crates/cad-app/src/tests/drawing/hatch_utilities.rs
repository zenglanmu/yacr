//! `HATCHGENERATEBOUNDARY` and `HATCHTOBACK` contract tests (AutoCAD 2026).
//!
//! Pins the two hatch utilities end to end:
//!
//! * one closed, non-associative polyline per hatch loop, mapped through the
//!   hatch plane, on the hatch's own layer and space, with bulges preserved, in
//!   one transaction / one undo step; the source hatch is retained;
//! * every refusal (empty selection, non-hatch, non-addressable, Viewer mode)
//!   writes nothing, bumps no revision and records no undo entry;
//! * HATCHTOBACK's contiguous backward draw-order block with relative order
//!   preserved, its exact undo restoration, its no-op cases and the checked
//!   `i64` underflow refusal.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn hatch_geometry(loops: Vec<HatchLoop>, elevation: f64) -> SemanticGeometry {
    let hatch = HatchGeometry::new(
        loops,
        HatchPlane::new(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            elevation,
        ),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    )
    .expect("fixture hatch must be valid");
    SemanticGeometry::Hatch(Box::new(hatch))
}

fn ring(points: &[[f64; 2]], bulges: Vec<f64>) -> HatchLoop {
    HatchLoop::new(
        points.to_vec(),
        bulges,
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    )
}

fn straight_ring(points: &[[f64; 2]]) -> HatchLoop {
    ring(points, vec![0.0; points.len()])
}

/// A circle of radius 1 around `(5, 3)` in the hatch OCS frame: four quarter
/// arcs, exactly the loop `HATCH` itself writes for a circle boundary.
fn circle_ring() -> HatchLoop {
    ring(
        &[[6.0, 3.0], [5.0, 4.0], [4.0, 3.0], [5.0, 2.0]],
        vec![std::f64::consts::FRAC_PI_8.tan(); 4],
    )
}

fn entity(
    id: u128,
    layer: LayerId,
    space: SpaceId,
    geometry: SemanticGeometry,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: match &geometry {
                SemanticGeometry::Hatch(_) => "AcDbHatch".into(),
                _ => "AcDbEntity".into(),
            },
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

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "HATCH")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for row in entities {
        builder.insert_entity(row).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
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

fn drawing_entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .expect("entity must exist")
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .count()
}

fn polyline(app: &Application, id: u128) -> (Vec<Point3>, Vec<f64>, bool, LayerId, SpaceId) {
    let row = drawing_entity(app, id);
    match row.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => (points, bulges, closed, row.layer, row.space),
        other => panic!("entity {id} is not a polyline: {other:?}"),
    }
}

fn hatch_fixture() -> DbEntity {
    entity(
        1,
        LayerId(5),
        SpaceId::Model,
        hatch_geometry(
            vec![
                straight_ring(&[[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]]),
                circle_ring(),
            ],
            2.5,
        ),
        20,
    )
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| ref_of(*id)));
}

// ---------------------------------------------------------------------------
// HATCHGENERATEBOUNDARY
// ---------------------------------------------------------------------------

#[test]
fn hatch_generate_boundary_writes_one_closed_polyline_per_loop_on_the_hatch_layer() {
    let mut entities = vec![hatch_fixture()];
    entities.push(entity(
        2,
        LayerId(0),
        SpaceId::Model,
        SemanticGeometry::Line {
            start: p3(-1.0, -1.0, 0.0),
            end: p3(-1.0, 1.0, 0.0),
        },
        0,
    ));
    let (mut app, mut session) = build(entities, AppMode::Work);
    select(&mut session, &[1]);
    let before_revision = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect("HATCHGENERATEBOUNDARY must succeed");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "hatchgenerateboundary.applied")
        .expect("the success diagnostic names the loop count");
    assert!(diagnostic.message.contains('2'), "{}", diagnostic.message);
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(revision(&app), Revision(before_revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // Source hatch retained; exactly two new polylines beside the existing line.
    assert!(matches!(
        drawing_entity(&app, 1).geometry,
        SemanticGeometry::Hatch(_)
    ));
    assert_eq!(entity_count(&app), 4);
    let created: Vec<u128> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|row| row.id.0 != 1 && row.id.0 != 2)
        .map(|row| row.id.0)
        .collect();
    assert_eq!(created.len(), 2);

    let mut saw_rectangle = false;
    let mut saw_circle = false;
    for id in created {
        let (points, bulges, closed, layer, space) = polyline(&app, id);
        assert!(closed, "a hatch boundary is a closed polyline");
        assert_eq!(layer, LayerId(5), "it keeps the hatch's layer");
        assert_eq!(space, SpaceId::Model, "it keeps the hatch's space");
        assert_eq!(points.len(), 4);
        assert_eq!(bulges.len(), 4);
        assert!(points.iter().all(|point| point.z == 2.5));
        if points[0] == p3(0.0, 0.0, 2.5) {
            saw_rectangle = true;
            assert_eq!(bulges, vec![0.0; 4], "straight edges stay straight");
        } else {
            saw_circle = true;
            let bulge = std::f64::consts::FRAC_PI_8.tan();
            assert!(bulges.iter().all(|value| (*value - bulge).abs() < 1e-12));
            assert_eq!(points[0], p3(6.0, 3.0, 2.5));
        }
    }
    assert!(saw_rectangle && saw_circle, "one polyline per loop");

    // The whole command is one undo step, and the source hatch survives it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the boundary creation");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity_count(&app), 2);
    assert!(matches!(
        drawing_entity(&app, 1).geometry,
        SemanticGeometry::Hatch(_)
    ));
}

#[test]
fn hatch_generate_boundary_drops_a_repeated_last_point_and_its_closing_bulge() {
    let repeated = HatchLoop::new(
        vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]],
        vec![0.1, 0.2, 0.3, 0.4, 0.5],
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    );
    let (hatch, _) = HatchGeometry::from_import(
        vec![repeated],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    );
    let (mut app, mut session) = build(
        vec![entity(
            1,
            LayerId(5),
            SpaceId::Model,
            SemanticGeometry::Hatch(Box::new(hatch)),
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect("a stored duplicated closure is normalised, not refused");

    let (points, bulges, closed, _, _) = polyline(&app, 2);
    assert!(closed);
    assert_eq!(points.len(), 4, "the repeated last vertex is dropped");
    assert_eq!(bulges, vec![0.1, 0.2, 0.3, 0.4]);
    assert_eq!(points[0], p3(0.0, 0.0, 0.0));
}

#[test]
fn hatch_generate_boundary_refusals_write_nothing() {
    let mut entities = vec![hatch_fixture()];
    entities.push(entity(
        2,
        LayerId(0),
        SpaceId::Model,
        SemanticGeometry::Line {
            start: p3(-1.0, -1.0, 0.0),
            end: p3(-1.0, 1.0, 0.0),
        },
        0,
    ));
    let (mut app, mut session) = build(entities, AppMode::Work);
    let before_revision = revision(&app);
    let before_count = entity_count(&app);

    // Empty selection.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect_err("an empty selection is refused");
    assert!(error
        .to_string()
        .contains("hatchgenerateboundary.selection"));

    // A selected non-hatch.
    select(&mut session, &[2]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect_err("a LINE is not a hatch");
    assert!(error
        .to_string()
        .contains("hatchgenerateboundary.not_hatch"));

    // A block-instance member is not directly addressable.
    let instance = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(2)]),
        sub_element: None,
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect_err("an instance-path ref is refused");
    assert!(error
        .to_string()
        .contains("hatchgenerateboundary.not_addressable"));

    assert_eq!(revision(&app), before_revision);
    assert_eq!(entity_count(&app), before_count);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_generate_boundary_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build(vec![hatch_fixture()], AppMode::Viewer);
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchGenerateBoundary,
        CommandPayload::None,
    )
    .expect_err("Viewer mode refuses a write");
    assert!(matches!(error, CadError::PermissionDenied));
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// HATCHTOBACK
// ---------------------------------------------------------------------------

fn line(id: u128, order: i64) -> DbEntity {
    entity(
        id,
        LayerId(0),
        SpaceId::Model,
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        order,
    )
}

fn hatch(id: u128, order: i64) -> DbEntity {
    entity(
        id,
        LayerId(5),
        SpaceId::Model,
        hatch_geometry(
            vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
            0.0,
        ),
        order,
    )
}

#[test]
fn hatch_to_back_stages_every_hatch_below_the_lowest_non_hatch_in_relative_order() {
    let (mut app, mut session) = build(
        vec![
            line(1, 10),
            hatch(2, 30),
            line(3, 15),
            hatch(4, 20),
            hatch(5, 7),
        ],
        AppMode::Work,
    );
    let before_revision = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("HATCHTOBACK must succeed");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "hatchtoback.applied"));
    // Entity 5 is already at its target (7); only 4 and 2 change.
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(revision(&app), Revision(before_revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // min_order = 10, three hatches -> 7..=9, preserving 5 < 4 < 2.
    assert_eq!(drawing_entity(&app, 1).draw_order, 10);
    assert_eq!(drawing_entity(&app, 3).draw_order, 15);
    assert_eq!(drawing_entity(&app, 5).draw_order, 7);
    assert_eq!(drawing_entity(&app, 4).draw_order, 8);
    assert_eq!(drawing_entity(&app, 2).draw_order, 9);

    // Geometry is untouched: only the stored draw order changed.
    let hatches: Vec<DbEntity> = [2u128, 4, 5]
        .iter()
        .map(|id| drawing_entity(&app, *id))
        .collect();
    assert!(hatches
        .iter()
        .all(|row| matches!(row.geometry, SemanticGeometry::Hatch(_))));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo restores the draw orders");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(drawing_entity(&app, 1).draw_order, 10);
    assert_eq!(drawing_entity(&app, 3).draw_order, 15);
    assert_eq!(drawing_entity(&app, 2).draw_order, 30);
    assert_eq!(drawing_entity(&app, 4).draw_order, 20);
    assert_eq!(drawing_entity(&app, 5).draw_order, 7);
}

#[test]
fn hatch_to_back_is_a_no_op_without_hatches_or_when_every_hatch_is_already_behind() {
    // No hatch at all.
    let (mut app, mut session) = build(vec![line(1, 10)], AppMode::Work);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("no hatch is a named no-op, not an error");
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics[0].code, "hatchtoback.none");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);

    // Every hatch already strictly below min_order.
    let (mut app, mut session) = build(vec![line(1, 10), hatch(2, 3)], AppMode::Work);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("already-behind is a named no-op");
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics[0].code, "hatchtoback.unchanged");
    assert_eq!(drawing_entity(&app, 2).draw_order, 3);
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_to_back_leaves_paper_space_hatches_alone() {
    let paper = entity(
        2,
        LayerId(5),
        SpaceId::Paper(LayoutId(1)),
        hatch_geometry(
            vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
            0.0,
        ),
        99,
    );
    let (mut app, mut session) = build(vec![line(1, 10), paper], AppMode::Work);
    let before = drawing_entity(&app, 2);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("paper-space hatches are outside this command's scope");
    assert_eq!(outcome.diagnostics[0].code, "hatchtoback.none");
    let after = drawing_entity(&app, 2);
    assert_eq!(before.draw_order, after.draw_order);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_to_back_refuses_an_i64_underflow_without_writing() {
    let (mut app, mut session) = build(vec![line(1, i64::MIN), hatch(2, i64::MAX)], AppMode::Work);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect_err("the target range underflows i64");
    assert!(error.to_string().contains("hatchtoback.order_exhausted"));
    assert_eq!(drawing_entity(&app, 2).draw_order, i64::MAX);
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_to_back_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build(vec![line(1, 10), hatch(2, 20)], AppMode::Viewer);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect_err("Viewer mode refuses a write");
    assert!(matches!(error, CadError::PermissionDenied));
    assert_eq!(drawing_entity(&app, 2).draw_order, 20);
    assert_eq!(undo_depth(&app), 0);
}
