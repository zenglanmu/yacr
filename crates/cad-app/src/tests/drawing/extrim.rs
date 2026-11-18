//! `EXTRIM` contract tests: trim every crossing target at one boundary.
//!
//! Split out of `draw2_batch` to keep that file inside the code-size gate. The
//! contracts are the same shape as the rest of the batch: every write is one
//! transaction / one undo step, a refusal writes nothing, and an unsupported
//! target is a named per-object skip (never a silent no-op).

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
            persistence: crate::PersistenceState::untitled(),
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

/// The support domain is TRIM's cuttable set: an ELLIPSE target (refused by the
/// pre-promotion kind list) and a LWPOLYLINE target (the shared polyline planner)
/// are cut in the same transaction.
#[test]
fn extrim_cuts_ellipse_and_polyline_targets_in_one_undo() {
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
            SemanticGeometry::Ellipse {
                center: p3(0.0, 0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(10.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
            1,
        ),
        ent(
            2,
            SemanticGeometry::Polyline {
                points: vec![p3(-10.0, 1.0, 0.0), p3(10.0, 1.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
                width: 0.0,
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
    // The kept piece contains each target's midpoint (the declared no-side-pick
    // rule): the ellipse's midpoint at parameter 0.5 is (-10, 0), so the window
    // between the two cuts at ±90 degrees survives.
    match entity(&app, 1).geometry {
        SemanticGeometry::Ellipse {
            center,
            start,
            sweep,
            ratio,
            ..
        } => {
            assert_eq!(center, p3(0.0, 0.0, 0.0));
            assert_eq!(ratio, 0.5);
            // The circle/ellipse pair is solved analytically; the solver's numeric
            // precision shows up in the parameter at the flat point of the conic,
            // so the assertions use a parameter tolerance rather than an epsilon.
            assert!(
                (start - std::f64::consts::FRAC_PI_2).abs() < 0.05,
                "{start}"
            );
            assert!((sweep.abs() - std::f64::consts::PI).abs() < 0.05, "{sweep}");
        }
        other => panic!("expected an ellipse, got {other:?}"),
    }
    // The polyline's arc-length midpoint (0, 1) is inside the circle, so the
    // inside window survives as an open polyline at the same id.
    match entity(&app, 2).geometry {
        SemanticGeometry::Polyline {
            ref points,
            ref bulges,
            closed,
            width,
        } => {
            assert!(!closed);
            assert_eq!(width, 0.0);
            assert_eq!(points.len(), 2);
            let cut_x = 24.0_f64.sqrt();
            assert!((points[0].x + cut_x).abs() < 1e-6, "{:?}", points[0]);
            assert!((points[1].x - cut_x).abs() < 1e-6, "{:?}", points[1]);
            assert!((points[0].y - 1.0).abs() < 1e-9, "{:?}", points[0]);
            assert!((points[1].y - 1.0).abs() < 1e-9, "{:?}", points[1]);
            assert!(bulges.iter().all(|bulge| bulge.abs() < 1e-12));
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    undo(&mut app, &mut session);
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Ellipse { start, sweep, .. }
            if start == 0.0 && (sweep - std::f64::consts::TAU).abs() < 1e-12
    ));
    assert_eq!(entity(&app, 2).geometry, {
        SemanticGeometry::Polyline {
            points: vec![p3(-10.0, 1.0, 0.0), p3(10.0, 1.0, 0.0)],
            bulges: vec![0.0],
            closed: false,
            width: 0.0,
        }
    });
}

/// One un-cuttable neighbouring object is a **named per-target skip**, not a
/// whole-command abort: the rest of the crossing set is still trimmed in one
/// transaction and the skipped object is reported by name.
#[test]
fn extrim_skips_an_uncuttable_target_by_name_and_still_trims_the_rest() {
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
        ent(2, SemanticGeometry::Point(p3(0.0, 0.0, 0.0)), 2),
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
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "extrim.unsupported_target"
                && diagnostic.object == Some(ObjectId(2))),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app).0, before.0 + 1);
}
