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
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

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
    build_with_blocks(entities, Vec::new(), mode)
}

/// The same drawing plus block-definition table rows (`(id, member ids)`), so a
/// test can put rows in `SpaceId::Block(_)` and still build a valid database.
fn build_with_blocks(
    entities: Vec<DbEntity>,
    blocks: Vec<(u128, Vec<u128>)>,
    mode: AppMode,
) -> (Application, SessionState) {
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
    // Two layouts: `Sheet1` carries the paper-space fixtures and `Sheet2` is the
    // deliberately hatch-free layout (a no-op scope, never an error).
    for (id, name) in [(LayoutId(1), "Sheet1"), (LayoutId(2), "Sheet2")] {
        builder
            .insert_layout(Layout {
                id,
                name: name.into(),
                viewports: Vec::new(),
            })
            .unwrap();
    }
    for (id, members) in blocks {
        builder
            .insert_block(BlockDefinition {
                id: BlockId(id),
                entities: members.into_iter().map(EntityId).collect(),
                dynamic_visibility: None,
            })
            .unwrap();
    }
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
            persistence: crate::PersistenceState::untitled(),
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
fn hatch_to_back_reorders_each_space_within_its_own_draw_order_sequence() {
    // Model space and layout `Sheet1` are independent sequences: the model hatch
    // runs below the model rows, the layout hatches below the layout rows, and
    // neither order is compared with the other space.
    let paper = |id: u128, order: i64| {
        entity(
            id,
            LayerId(5),
            SpaceId::Paper(LayoutId(1)),
            hatch_geometry(
                vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
                0.0,
            ),
            order,
        )
    };
    let (mut app, mut session) = build(
        vec![
            line(1, 10),
            line(2, 30),
            hatch(3, 40),
            entity(
                4,
                LayerId(0),
                SpaceId::Paper(LayoutId(1)),
                SemanticGeometry::Line {
                    start: p3(0.0, 0.0, 0.0),
                    end: p3(1.0, 0.0, 0.0),
                },
                100,
            ),
            paper(5, 120),
            paper(6, 90),
            // `Sheet2` holds no hatch: a no-op scope, not an error.
            entity(
                7,
                LayerId(0),
                SpaceId::Paper(LayoutId(2)),
                SemanticGeometry::Line {
                    start: p3(0.0, 0.0, 0.0),
                    end: p3(1.0, 0.0, 0.0),
                },
                7,
            ),
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
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "hatchtoback.applied")
        .expect("the success diagnostic lists every scope");
    assert!(
        diagnostic.message.contains("model space: 1 moved"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("layout 'Sheet1': 2 moved"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("layout 'Sheet2': no hatch"),
        "{}",
        diagnostic.message
    );
    assert_eq!(
        outcome.objects.len(),
        3,
        "one model hatch plus two paper ones"
    );
    assert_eq!(revision(&app), Revision(before_revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // Model: min non-hatch 10, one hatch -> 9.
    assert_eq!(drawing_entity(&app, 3).draw_order, 9);
    // Layout: min non-hatch 100, two hatches -> 98..=99 in relative order 6 < 5.
    assert_eq!(drawing_entity(&app, 6).draw_order, 98);
    assert_eq!(drawing_entity(&app, 5).draw_order, 99);
    // Every other row keeps its order, the hatch-free layout included.
    assert_eq!(drawing_entity(&app, 1).draw_order, 10);
    assert_eq!(drawing_entity(&app, 2).draw_order, 30);
    assert_eq!(drawing_entity(&app, 4).draw_order, 100);
    assert_eq!(drawing_entity(&app, 7).draw_order, 7);
    for id in [3u128, 5, 6] {
        assert!(
            matches!(
                drawing_entity(&app, id).geometry,
                SemanticGeometry::Hatch(_)
            ),
            "entity {id} keeps its geometry"
        );
    }

    // One undo restores every space's exact previous orders.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo restores both spaces");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(drawing_entity(&app, 3).draw_order, 40);
    assert_eq!(drawing_entity(&app, 5).draw_order, 120);
    assert_eq!(drawing_entity(&app, 6).draw_order, 90);
}

#[test]
fn hatch_to_back_reorders_a_block_definition_behind_its_own_members() {
    // Block definitions carry their own member draw order (that is what an INSERT
    // draws), so the reorder must run inside the definition too.
    let member = |id: u128, is_hatch: bool, order: i64| {
        let geometry = if is_hatch {
            hatch_geometry(
                vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
                0.0,
            )
        } else {
            SemanticGeometry::Line {
                start: p3(0.0, 0.0, 0.0),
                end: p3(1.0, 0.0, 0.0),
            }
        };
        entity(id, LayerId(0), SpaceId::Block(BlockId(0)), geometry, order)
    };
    let (mut app, mut session) = build_with_blocks(
        vec![
            member(10, false, 5),
            member(11, true, 9),
            member(12, true, 2),
        ],
        vec![(0, vec![10, 11, 12])],
        AppMode::Work,
    );
    let revision_before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("HATCHTOBACK must reorder a block definition");
    let diagnostic = &outcome.diagnostics[0];
    assert!(
        diagnostic.message.contains("block 'B0'") || diagnostic.message.contains("block #0"),
        "{}",
        diagnostic.message
    );
    assert_eq!(outcome.objects.len(), 2, "both member hatches moved");
    assert_eq!(revision(&app), Revision(revision_before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    // min non-hatch is 5, two hatches -> 3..=4, keeping 12 (2) before 11 (9).
    assert_eq!(drawing_entity(&app, 12).draw_order, 3);
    assert_eq!(drawing_entity(&app, 11).draw_order, 4);
    assert_eq!(drawing_entity(&app, 10).draw_order, 5);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo restores the block member orders");
    assert_eq!(drawing_entity(&app, 11).draw_order, 9);
    assert_eq!(drawing_entity(&app, 12).draw_order, 2);
}

#[test]
fn hatch_to_back_reorders_a_mixed_selection_in_one_undo_step() {
    // Model space, a layout and a block definition in one run: every scope is
    // planned before anything is staged, so this is still exactly one
    // transaction / one undo step.
    let paper_hatch = entity(
        3,
        LayerId(5),
        SpaceId::Paper(LayoutId(1)),
        hatch_geometry(
            vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
            0.0,
        ),
        500,
    );
    let block_hatch = entity(
        4,
        LayerId(5),
        SpaceId::Block(BlockId(0)),
        hatch_geometry(
            vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
            0.0,
        ),
        60,
    );
    let block_line = entity(
        5,
        LayerId(0),
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        20,
    );
    let paper_line = entity(
        6,
        LayerId(0),
        SpaceId::Paper(LayoutId(1)),
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        400,
    );
    let (mut app, mut session) = build_with_blocks(
        vec![
            line(1, 10),
            hatch(2, 20),
            paper_hatch,
            block_hatch,
            block_line,
            paper_line,
        ],
        vec![(0, vec![4, 5])],
        AppMode::Work,
    );
    let undo_before = undo_depth(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::HatchToBack,
        CommandPayload::None,
    )
    .expect("HATCHTOBACK over mixed scopes");
    assert_eq!(outcome.objects.len(), 3, "one hatch per scope moved");
    assert_eq!(
        undo_depth(&app),
        undo_before + 1,
        "one transaction for every scope"
    );
    assert_eq!(drawing_entity(&app, 2).draw_order, 9);
    assert_eq!(drawing_entity(&app, 3).draw_order, 399);
    assert_eq!(drawing_entity(&app, 4).draw_order, 19);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("one undo restores every scope");
    assert_eq!(drawing_entity(&app, 2).draw_order, 20);
    assert_eq!(drawing_entity(&app, 3).draw_order, 500);
    assert_eq!(drawing_entity(&app, 4).draw_order, 60);
}

#[test]
fn hatch_to_back_refuses_an_i64_underflow_without_writing() {
    // The model scope underflows; the layout scope would succeed. The refusal is
    // for the whole run, so the layout hatch must not move either.
    let paper_hatch = entity(
        3,
        LayerId(5),
        SpaceId::Paper(LayoutId(1)),
        hatch_geometry(
            vec![straight_ring(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])],
            0.0,
        ),
        500,
    );
    let paper_line = entity(
        4,
        LayerId(0),
        SpaceId::Paper(LayoutId(1)),
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        400,
    );
    let (mut app, mut session) = build(
        vec![
            line(1, i64::MIN),
            hatch(2, i64::MAX),
            paper_hatch,
            paper_line,
        ],
        AppMode::Work,
    );
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
    assert_eq!(drawing_entity(&app, 3).draw_order, 500);
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
