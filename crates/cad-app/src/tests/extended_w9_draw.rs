//! Wave 9 real-edit contracts: `BLEND` and `BREAKLINE`, plus the `ARRAY`
//! router report and the stable named refusals.
//!
//! A real edit must create exactly the declared entity and commit exactly one
//! transaction / one undo step. A refusal must write nothing, bump no revision
//! and record no history.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
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
        geometry: SemanticGeometry::Line { start, end },
        draw_order: id as i64,
    }
}

/// Replace the default 0-layer document with one holding the given entities.
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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .into_iter()
        .cloned()
        .collect()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

#[test]
fn breakline_creates_one_open_polyline_with_the_declared_symbol() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::BreakLine,
        CommandPayload::Points(vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)]),
    )
    .expect("BREAKLINE is admitted");
    assert!(
        outcome.changes.is_some(),
        "BREAKLINE writes one transaction"
    );
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 1);
    match &entities[0].geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points.len(), 7, "start, five symbol points, end");
            assert!(bulges.is_empty());
            assert!(!closed, "BREAKLINE creates an open polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "breakline.partial"));
}

#[test]
fn breakline_refuses_coincident_points_without_change() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::BreakLine,
        CommandPayload::Points(vec![p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)]),
    )
    .expect_err("coincident points refuse");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(model_entities(&app).is_empty());
}

#[test]
fn blend_bridges_two_lines_with_one_cubic_spline() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)),
        line(2, p3(6.0, 2.0, 0.0), p3(10.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Blend,
        CommandPayload::None,
    )
    .expect("BLEND is admitted");
    assert!(outcome.changes.is_some());
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 3, "two sources plus one spline");
    let spline = entities
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Spline { .. }))
        .expect("BLEND creates a spline");
    match &spline.geometry {
        SemanticGeometry::Spline {
            degree,
            control_points,
            weights,
            ..
        } => {
            assert_eq!(*degree, 3);
            assert_eq!(control_points.len(), 4);
            assert_eq!(weights.len(), 4);
        }
        _ => unreachable!(),
    }
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "blend.partial"));
}

#[test]
fn blend_requires_exactly_two_open_curves() {
    let (mut app, mut session) = app_with(vec![line(1, p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0))]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Blend,
        CommandPayload::None,
    )
    .expect_err("one curve is not enough");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn array_without_a_payload_reports_the_two_planners_and_writes_nothing() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Array,
        CommandPayload::None,
    )
    .expect("the router report is admitted");
    assert!(outcome.changes.is_none());
    let row = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "array.needs_payload")
        .expect("the router names the missing payload");
    assert!(row.message.contains("ARRAYRECT"));
    assert!(row.message.contains("ARRAYPOLAR"));
    assert_eq!(revision(&app), before);
}

const REFUSALS: [(ExtendedCommandId, &str); 6] = [
    (
        ExtendedCommandId::ArcText,
        "arctext.unsupported: text along an arc is not modeled; no combined arc+text payload",
    ),
    (
        ExtendedCommandId::MLeaderAlign,
        "mleaderalign.unsupported: no multileader alignment model",
    ),
    (
        ExtendedCommandId::MLeaderEdit,
        "mleaderedit.unsupported: no add/remove leader-line API",
    ),
    (
        ExtendedCommandId::MLeaderCollect,
        "mleadercollect.unsupported: no multileader collection/table model",
    ),
    (
        ExtendedCommandId::Tolerance,
        "tolerance.unsupported: no feature-control-frame (tolerance) object model",
    ),
    (
        ExtendedCommandId::DimArc,
        "dimarc.unsupported: the dimension domain has no arc-length kind",
    ),
];

#[test]
fn wave9_draw_refusals_answer_with_their_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext, CommandPayload::None)
            .expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn blend_and_breakline_are_work_only() {
    for ext in [ExtendedCommandId::Blend, ExtendedCommandId::BreakLine] {
        assert!(ext.requires_work_mode(), "{ext:?} writes entities");
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is a legal mode");
        let error = execute(&mut app, &mut session, ext, CommandPayload::None)
            .expect_err("Viewer cannot edit");
        assert!(matches!(error, CadError::PermissionDenied));
    }
}
