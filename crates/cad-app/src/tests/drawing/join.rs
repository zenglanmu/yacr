//! JOIN edit-command contract tests.
//!
//! Pins the two implemented subsets (collinear LINEs → one LINE, contiguous
//! concentric ARCs → one ARC with the summed sweep), the fresh-id/one-transaction/
//! one-undo-step shape (an UNDO restores every source), the explicit refusal set
//! (nothing written, no revision bump, no undo entry) and the locked-layer
//! chokepoint.

use super::explode::part;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, LayerAttributes};

fn line_geometry(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(x0, y0),
        end: point(x1, y1),
    }
}

fn arc_geometry(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

/// A work-mode app with layer 0 and the given model entities (type key chosen from
/// the geometry kind, so `class_name` agrees with the fixture).
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, false)
}

/// As [`app_with`], with layer 0 **locked**.
fn app_with_locked_layer(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, true)
}

/// As [`app_with`], plus one drawable paper layout (`LayoutId(1)`) so a paper-space
/// entity is a real, representable space.
fn app_with_paper_layout(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let corners = || vec![point(0.0, 0.0), point(100.0, 50.0), point(10.0, 20.0)];
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: vec![cad_db::PaperViewport {
            clip: corners(),
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([32u8; 32]),
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

fn build(entities: Vec<DbEntity>, lock_layer_zero: bool) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    if lock_layer_zero {
        b.set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                locked: true,
                ..LayerAttributes::default()
            },
        )
        .unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([31u8; 32]),
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

fn line_entity(id: u128, layer: LayerId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(id, geometry, layer, SpaceId::Model, "AcDbLine", order)
}

fn arc_entity(id: u128, layer: LayerId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(id, geometry, layer, SpaceId::Model, "AcDbArc", order)
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(entities.iter().copied().map(ref_of).collect()),
    )
    .unwrap();
}

fn join(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::Join, CommandPayload::None)
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_geometry(app: &Application) -> Vec<(EntityId, SemanticGeometry)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| (entity.id, entity.geometry.clone()))
        .collect()
}

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

fn three_collinear_lines() -> (Application, SessionState) {
    app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(4.0, 0.0, 8.0, 0.0), 1),
        line_entity(3, LayerId(0), line_geometry(8.0, 0.0, 10.0, 0.0), 2),
    ])
}

#[test]
fn three_collinear_lines_merge_into_one_with_a_fresh_id() {
    let (mut app, mut session) = three_collinear_lines();
    let before_revision = revision(&app);
    let before_count = entity_count(&app);
    select(&mut app, &mut session, &[1, 2, 3]);

    let outcome = join(&mut app, &mut session).expect("the chain merges");
    assert_eq!(outcome.objects.len(), 1, "exactly one entity is produced");
    let new_id = EntityId(outcome.objects[0].0);
    assert!(
        ![EntityId(1), EntityId(2), EntityId(3)].contains(&new_id),
        "the merged LINE gets a fresh id, not one of its sources: {new_id:?}"
    );

    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1, "three sources deleted, one inserted");
    let (id, geometry) = &produced[0];
    assert_eq!(*id, new_id);
    match geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(*start, point(0.0, 0.0)) && near(*end, point(10.0, 0.0)),
                "the weld spans the union min→max: {start:?} → {end:?}"
            );
        }
        other => panic!("expected one LINE, got {other:?}"),
    }
    assert_eq!(entity_count(&app), before_count - 2, "N sources → 1 entity");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
}

#[test]
fn undo_restores_every_source_and_removes_the_weld() {
    let (mut app, mut session) = three_collinear_lines();
    select(&mut app, &mut session, &[1, 2, 3]);
    join(&mut app, &mut session).expect("the chain merges");
    assert_eq!(entity_count(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the join is undoable");
    assert_eq!(entity_count(&app), 3, "every source is restored");
    assert_eq!(undo_depth(&app), 0);
    let mut ids: Vec<u128> = model_geometry(&app)
        .into_iter()
        .map(|(id, _)| id.0)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3], "the original ids come back");
}

#[test]
fn overlapping_and_reversed_lines_still_merge_into_one_span() {
    // The second pick runs right-to-left and overlaps the first: the merge is over
    // the union of the spans, not over the pick order.
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 6.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(10.0, 0.0, 4.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("an overlap is collinear and contiguous");
    match &model_geometry(&app)[0].1 {
        SemanticGeometry::Line { start, end } => assert!(
            near(*start, point(0.0, 0.0)) && near(*end, point(10.0, 0.0)),
            "{start:?} → {end:?}"
        ),
        other => panic!("expected one LINE, got {other:?}"),
    }
}

#[test]
fn collinear_lines_at_a_non_zero_elevation_keep_their_elevation() {
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            SemanticGeometry::Line {
                start: p3(0.0, 0.0, 3.0),
                end: p3(4.0, 0.0, 3.0),
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            0,
        ),
        part(
            2,
            SemanticGeometry::Line {
                start: p3(4.0, 0.0, 3.0),
                end: p3(9.0, 0.0, 3.0),
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("a flat chain at z=3 merges");
    match &model_geometry(&app)[0].1 {
        SemanticGeometry::Line { start, end } => {
            assert!(near(*start, p3(0.0, 0.0, 3.0)) && near(*end, p3(9.0, 0.0, 3.0)));
        }
        other => panic!("expected one LINE, got {other:?}"),
    }
}

#[test]
fn two_contiguous_arcs_merge_with_the_summed_sweep() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 1.0),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 1.0, 0.5),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("two contiguous arcs merge");
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1);
    match &produced[0].1 {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            normal,
        } => {
            assert!(near(*center, point(0.0, 0.0)));
            assert!((radius - 5.0).abs() < 1e-12);
            assert!(near(*normal, p3(0.0, 0.0, 1.0)));
            assert!((start - 0.0).abs() < 1e-12, "start = {start}");
            assert!((sweep - 1.5).abs() < 1e-12, "sweep = {sweep}");
        }
        other => panic!("expected one ARC, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn clockwise_arcs_merge_with_a_negative_summed_sweep() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 2.0, 0.0, -0.5),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 2.0, -0.5, -0.25),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join(&mut app, &mut session).expect("a clockwise chain merges");
    match &model_geometry(&app)[0].1 {
        SemanticGeometry::Arc { sweep, .. } => {
            assert!((sweep + 0.75).abs() < 1e-12, "sweep = {sweep}")
        }
        other => panic!("expected one ARC, got {other:?}"),
    }
}

/// Every refusal must change nothing: no entity change, no revision bump and no
/// undo entry.
fn assert_refusal_changes_nothing(
    app: &mut Application,
    session: &mut SessionState,
    expected: &str,
    unsupported: bool,
) {
    let before_revision = revision(app);
    let before_count = entity_count(app);
    let error = join(app, session).expect_err("the refusal is explicit");
    if unsupported {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    } else {
        let message = format!("{error:?}");
        assert!(
            message.contains(expected),
            "expected the named code {expected}, got {message}"
        );
    }
    assert_eq!(revision(app), before_revision, "no revision bump");
    assert_eq!(entity_count(app), before_count, "no partial write");
    assert_eq!(undo_depth(app), 0, "no empty undo entry");
}

#[test]
fn a_single_entity_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![line_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 10.0, 0.0),
        0,
    )]);
    select(&mut app, &mut session, &[1]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.needs_two", false);
}

#[test]
fn an_empty_selection_is_refused_by_name() {
    let (mut app, mut session) = three_collinear_lines();
    assert_refusal_changes_nothing(&mut app, &mut session, "join.needs_two", false);
}

#[test]
fn mixed_line_and_arc_picks_are_refused() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 1.0),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.mixed_kinds", false);
}

#[test]
fn a_parallel_but_offset_line_is_not_collinear() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(10.0, 1.0, 20.0, 1.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.not_collinear", false);
}

#[test]
fn a_gap_between_lines_is_not_contiguous() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(5.0, 0.0, 9.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.not_contiguous", false);
}

#[test]
fn a_layer_mismatch_is_refused() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        line_entity(2, LayerId(7), line_geometry(4.0, 0.0, 9.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.layer_mismatch", false);
}

#[test]
fn a_space_mismatch_is_refused() {
    let (mut app, mut session) = app_with_paper_layout(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            line_geometry(4.0, 0.0, 9.0, 0.0),
            LayerId(0),
            SpaceId::Paper(LayoutId(1)),
            "AcDbLine",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.space_mismatch", false);
}

#[test]
fn a_summed_sweep_beyond_one_turn_is_refused_not_wrapped() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 3.5),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 3.5, 3.5),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.sweep_overflow", false);
}

#[test]
fn arcs_with_a_different_radius_are_not_contiguous() {
    let (mut app, mut session) = app_with(vec![
        arc_entity(
            1,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 5.0, 0.0, 1.0),
            0,
        ),
        arc_entity(
            2,
            LayerId(0),
            arc_geometry(point(0.0, 0.0), 6.0, 1.0, 1.0),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "join.not_contiguous", false);
}

#[test]
fn a_non_line_or_arc_kind_is_unsupported() {
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            SemanticGeometry::Polyline {
                points: vec![point(4.0, 0.0), point(9.0, 0.0)],
                bulges: vec![0.0, 0.0],
                closed: false,
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbPolyline",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    assert_refusal_changes_nothing(&mut app, &mut session, "", true);
}

#[test]
fn a_payload_is_refused_rather_than_ignored() {
    let (mut app, mut session) = three_collinear_lines();
    select(&mut app, &mut session, &[1, 2, 3]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Join,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
    )
    .expect_err("JOIN takes no payload");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 3);
}

#[test]
fn a_locked_layer_is_refused_by_the_database_chokepoint() {
    // Delete + insert are the write path, so the existing `layer.locked` rule
    // refuses the whole merge: nothing is deleted and no undo entry exists.
    let (mut app, mut session) = app_with_locked_layer(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        line_entity(2, LayerId(0), line_geometry(4.0, 0.0, 9.0, 0.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before_revision = revision(&app);
    let error = join(&mut app, &mut session).expect_err("a locked layer refuses the write");
    let message = format!("{error:?}");
    assert!(message.contains("layer.locked"), "{message}");
    assert_eq!(entity_count(&app), 2, "nothing was deleted");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0, "no empty undo entry");
}

#[test]
fn viewer_mode_refuses_join() {
    let (mut app, mut session) = three_collinear_lines();
    select(&mut app, &mut session, &[1, 2, 3]);
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    let error = join(&mut app, &mut session).expect_err("Viewer refuses a write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(entity_count(&app), 3);
}
