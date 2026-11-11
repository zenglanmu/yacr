//! `CIRCLE 2P` / `CIRCLE 3P` / `CIRCLE D` create-command contract tests.
//!
//! Each construction option builds exactly one `SemanticGeometry::Circle` on the
//! work-plane `+Z` normal, on the active layer in model space, through the shared
//! create funnel: one entity, one transaction and one undo step. These tests pin
//! the three constructions (midpoint/half-span for 2P, the shared
//! `arc_parameters` circumcircle for 3P, half the centre→pick distance for D),
//! the honest `drawing.circle2p` / `drawing.circle3p` / `drawing.circlediameter`
//! diagnostics, and every named refusal (nothing written, no revision bump, no
//! undo entry).

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

/// The stored circle `(center, radius)` of one created object, asserting the
/// work-plane normal, the layer, the model space and the `AcDbCircle` type key.
fn stored_circle(app: &Application, object: ObjectId, layer: LayerId) -> (Point3, f64) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the created circle");
    assert_eq!(entity.layer, layer);
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbCircle");
    match &entity.geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            assert_eq!(
                *normal,
                p3(0.0, 0.0, 1.0),
                "every construction option is a +Z work-plane circle"
            );
            (*center, *radius)
        }
        other => panic!("expected a circle, got {other:?}"),
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn assert_point_close(actual: Point3, expected: Point3) {
    assert_close(actual.x, expected.x);
    assert_close(actual.y, expected.y);
    assert_close(actual.z, expected.z);
}

/// A work-mode drawing with layer `0` plus `WALLS`, and `WALLS` current.
fn app_with_active_layer(mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([91u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), mode);
    session.active_layer = LayerId(7);
    (app, session)
}

/// Assert one created circle's one-entity / one-transaction / one-undo shape.
fn assert_one_transaction(outcome: &CommandOutcome, before: Revision) {
    assert_eq!(
        outcome.objects.len(),
        1,
        "one entity per construction option"
    );
    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
}

fn diagnostic_code(outcome: &CommandOutcome, code: &str) -> String {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing diagnostic {code}"))
        .message
        .clone()
}

#[test]
fn circle_2p_uses_the_midpoint_and_half_the_diameter() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle2P,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0)]),
    )
    .unwrap();
    assert_one_transaction(&outcome, before);

    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    assert_point_close(center, p3(2.0, 0.0, 0.0));
    assert_close(radius, 2.0);

    // The diagnostic names the resulting centre and radius.
    let message = diagnostic_code(&outcome, "drawing.circle2p");
    assert!(message.contains("CIRCLE 2P"), "{message}");
    assert!(message.contains("radius 2"), "{message}");

    // One undo step removes exactly the created circle.
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    let entity_id = EntityId(outcome.objects[0].0);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_none());
}

#[test]
fn circle_2p_handles_a_non_round_diagonal_pair() {
    // (1,2)-(4,6) is a 3-4-5 diagonal: centre (2.5, 4) and radius 2.5, so the
    // result is not an artefact of nice axis-aligned input.
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle2P,
        CommandPayload::Points(vec![point(1.0, 2.0), point(4.0, 6.0)]),
    )
    .unwrap();
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    assert_point_close(center, p3(2.5, 4.0, 0.0));
    assert_close(radius, 2.5);
}

#[test]
fn circle_2p_is_on_the_active_layer_in_one_undo_step() {
    let (mut app, mut session) = app_with_active_layer(AppMode::Work);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle2P,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    assert_one_transaction(&outcome, before);
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(7));
    assert_point_close(center, p3(1.0, 0.0, 0.0));
    assert_close(radius, 1.0);
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        1,
        "the whole construction is one undo step"
    );
}

#[test]
fn circle_3p_is_the_arc_circumcircle() {
    let (mut app, mut session) = drawing_app();
    let picks = [point(0.0, 0.0), point(2.0, 0.0), point(1.0, 1.0)];
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle3P,
        CommandPayload::Points(picks.to_vec()),
    )
    .unwrap();
    assert_one_transaction(&outcome, before);
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    assert_point_close(center, p3(1.0, 0.0, 0.0));
    assert_close(radius, 1.0);

    // The single circumcircle source: CIRCLE 3P and ARC through the same three
    // points must report the identical centre and radius.
    let arc = crate::draw_tool::arc_parameters(picks[0], picks[1], picks[2]).unwrap();
    assert_point_close(center, arc.center);
    assert_close(radius, arc.radius);

    let message = diagnostic_code(&outcome, "drawing.circle3p");
    assert!(message.contains("CIRCLE 3P"), "{message}");
    assert!(message.contains("radius 1"), "{message}");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn circle_3p_matches_the_stored_arc_circumcircle() {
    // The single-source contract asserted against the **committed** ARC entity,
    // not only the helper: if `create_arc` ever stopped using `arc_parameters`,
    // this would fail even though the helper comparison above still passed.
    let picks = [point(0.0, 0.0), point(3.0, 1.0), point(1.0, 4.0)];
    let (mut app, mut session) = drawing_app();
    let circle = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle3P,
        CommandPayload::Points(picks.to_vec()),
    )
    .unwrap();
    let arc = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(picks.to_vec()),
    )
    .unwrap();
    let (circle_center, circle_radius) = stored_circle(&app, circle.objects[0], LayerId(0));
    let arc_entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(arc.objects[0].0))
        .expect("the arc entity");
    let (arc_center, arc_radius) = match &arc_entity.geometry {
        SemanticGeometry::Arc { center, radius, .. } => (*center, *radius),
        other => panic!("expected an arc, got {other:?}"),
    };
    assert_point_close(circle_center, arc_center);
    assert_close(circle_radius, arc_radius);
    assert_eq!(circle_center, arc_center, "one circumcircle implementation");
    assert_eq!(circle_radius, arc_radius, "one circumcircle implementation");
}

#[test]
fn circle_3p_handles_a_non_axis_triangle() {
    // A triangle with no axis-aligned side: the circumcentre is derived, not
    // guessed. The stored circle must pass through all three picks.
    let picks = [point(0.0, 0.0), point(3.0, 1.0), point(1.0, 4.0)];
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle3P,
        CommandPayload::Points(picks.to_vec()),
    )
    .unwrap();
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    for pick in picks {
        let distance = (pick.x - center.x).hypot(pick.y - center.y);
        assert_close(distance, radius);
    }
    let arc = crate::draw_tool::arc_parameters(picks[0], picks[1], picks[2]).unwrap();
    assert_point_close(center, arc.center);
    assert_close(radius, arc.radius);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn circle_3p_refuses_collinear_and_coincident_points_without_writing() {
    let cases: &[(&str, [Point3; 3])] = &[
        (
            "collinear",
            [point(0.0, 0.0), point(2.0, 0.0), point(4.0, 0.0)],
        ),
        (
            "coincident",
            [point(1.0, 1.0), point(1.0, 1.0), point(2.0, 2.0)],
        ),
    ];
    for (label, picks) in cases {
        let (mut app, mut session) = drawing_app();
        let before = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateCircle3P,
            CommandPayload::Points(picks.to_vec()),
        )
        .unwrap_err();
        assert!(
            matches!(&err, CadError::InvalidInput(message) if message.starts_with("circle3p.degenerate")),
            "{label}: expected circle3p.degenerate, got {err:?}"
        );
        assert_eq!(revision(&app), before, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}: no undo entry");
    }
}

#[test]
fn circle_diameter_uses_half_the_center_distance() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircleDiameter,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0)]),
    )
    .unwrap();
    assert_one_transaction(&outcome, before);
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    assert_point_close(center, p3(0.0, 0.0, 0.0));
    assert_close(radius, 2.0);
    assert!(
        (radius - 2.0).abs() < 1e-9,
        "the picked distance is the diameter, so the radius is half of it"
    );
    let message = diagnostic_code(&outcome, "drawing.circlediameter");
    assert!(message.contains("CIRCLE D"), "{message}");
    assert!(message.contains("radius 2"), "{message}");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn circle_diameter_keeps_a_non_axis_center_and_halves_the_span() {
    // Centre (1,-1) and a diameter point at (4,3): a 3-4-5 span, so the radius is
    // 2.5 and the centre is the first pick (not the midpoint).
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircleDiameter,
        CommandPayload::Points(vec![point(1.0, -1.0), point(4.0, 3.0)]),
    )
    .unwrap();
    let (center, radius) = stored_circle(&app, outcome.objects[0], LayerId(0));
    assert_point_close(center, p3(1.0, -1.0, 0.0));
    assert_close(radius, 2.5);
}

#[test]
fn circle_2p_and_diameter_refuse_degenerate_and_non_finite_pairs() {
    let cases: &[(&str, CommandId, [Point3; 2])] = &[
        (
            "2p coincident",
            CommandId::CreateCircle2P,
            [point(1.0, 1.0), point(1.0, 1.0)],
        ),
        (
            "2p non-finite",
            CommandId::CreateCircle2P,
            [point(f64::NAN, 0.0), point(4.0, 0.0)],
        ),
        (
            "diameter coincident",
            CommandId::CreateCircleDiameter,
            [point(2.0, 2.0), point(2.0, 2.0)],
        ),
        (
            "diameter non-finite",
            CommandId::CreateCircleDiameter,
            [point(0.0, 0.0), point(f64::INFINITY, 0.0)],
        ),
    ];
    for (label, id, picks) in cases {
        let (mut app, mut session) = drawing_app();
        let before = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            *id,
            CommandPayload::Points(picks.to_vec()),
        )
        .unwrap_err();
        let expected = match id {
            CommandId::CreateCircle2P => "circle2p.degenerate",
            _ => "circlediameter.degenerate",
        };
        assert!(
            matches!(&err, CadError::InvalidInput(message) if message.starts_with(expected)),
            "{label}: expected {expected}, got {err:?}"
        );
        assert_eq!(revision(&app), before, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}: no undo entry");
    }
}

#[test]
fn circle_options_refuse_viewer_mode_without_writing() {
    let cases: &[(&str, CommandId, Vec<Point3>)] = &[
        (
            "CIRCLE 2P",
            CommandId::CreateCircle2P,
            vec![point(0.0, 0.0), point(4.0, 0.0)],
        ),
        (
            "CIRCLE 3P",
            CommandId::CreateCircle3P,
            vec![point(0.0, 0.0), point(2.0, 0.0), point(1.0, 1.0)],
        ),
        (
            "CIRCLE D",
            CommandId::CreateCircleDiameter,
            vec![point(0.0, 0.0), point(4.0, 0.0)],
        ),
    ];
    for (label, id, picks) in cases {
        let (mut app, _) = drawing_app();
        let before = revision(&app);
        let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
        let err = execute(
            &mut app,
            &mut session,
            *id,
            CommandPayload::Points(picks.clone()),
        )
        .unwrap_err();
        assert!(
            matches!(err, CadError::PermissionDenied),
            "{label}: Viewer must be refused, got {err:?}"
        );
        assert_eq!(revision(&app), before, "{label}: no write in Viewer");
        assert!(!app.can_undo(&DocumentId(1)), "{label}: no undo entry");
    }
}

// ---------------------------------------------------------------------------
// ARC: the alternate construction methods and the payload-length contract. ARC
// shares the circumcircle helper with `CIRCLE 3P`, so its tests live here.
// ---------------------------------------------------------------------------

/// The stored `(center, normal, radius, start, sweep)` of one created arc.
fn stored_arc(app: &Application, object: ObjectId) -> (Point3, Point3, f64, f64, f64) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the created arc");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbArc");
    match &entity.geometry {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => (*center, *normal, *radius, *start, *sweep),
        other => panic!("expected an arc, got {other:?}"),
    }
}

#[test]
fn create_arc_refuses_a_fourth_pick_instead_of_dropping_it() {
    // The three-point construction is exactly three picks. A longer list (for example
    // a start/end angle pair appended to the axis picks) used to be silently
    // truncated into a different arc.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 0.0),
            point(3.0, 1.0),
        ]),
    )
    .unwrap_err();
    match &err {
        CadError::InvalidInput(message) => assert!(
            message.starts_with("arc.payload_length"),
            "the extra pick must be refused by name: {message}"
        ),
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_arc_authored_start_center_end_matches_the_stored_arc_tuple() {
    // Every AutoCAD construction collapses into the one stored tuple, so a
    // Start-Center-End arc is authored as the programmatic `Geometry(Arc)` payload:
    // centre, radius = |start − centre|, and the sweep that runs start → end in the
    // chosen direction. This pins that contract end to end (one entity, one undo
    // step) for the construction the three-pick capture cannot express.
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let center = point(0.0, 0.0);
    let start_point = point(2.0, 0.0);
    let end_point = point(0.0, 2.0);
    let radius = 2.0;
    let start = 0.0_f64;
    let sweep = std::f64::consts::FRAC_PI_2;
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Arc {
            center,
            normal: p3(0.0, 0.0, 1.0),
            radius,
            start,
            sweep,
        })),
    )
    .unwrap();
    let (stored_center, stored_normal, stored_radius, stored_start, stored_sweep) =
        stored_arc(&app, outcome.objects[0]);
    assert_point_close(stored_center, center);
    assert_eq!(stored_normal, p3(0.0, 0.0, 1.0));
    assert_close(stored_radius, radius);
    assert_close(stored_start, start);
    assert_close(stored_sweep, sweep);
    // The stored arc's endpoints are the picked start/end points.
    assert_point_close(
        point(
            stored_center.x + stored_radius * stored_start.cos(),
            stored_center.y + stored_radius * stored_start.sin(),
        ),
        start_point,
    );
    assert_point_close(
        point(
            stored_center.x + stored_radius * (stored_start + stored_sweep).cos(),
            stored_center.y + stored_radius * (stored_start + stored_sweep).sin(),
        ),
        end_point,
    );
    assert!(revision(&app).0 > before.0);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn create_arc_authored_start_end_direction_keeps_a_negative_sweep() {
    // The clockwise direction (Start-End-Angle/Direction with the other turning
    // sense) is the negative sweep; it is stored verbatim, never normalised to a
    // counter-clockwise arc.
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Arc {
            center: point(1.0, 1.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 3.0,
            start: std::f64::consts::FRAC_PI_2,
            sweep: -std::f64::consts::FRAC_PI_4,
        })),
    )
    .unwrap();
    let (_, _, radius, start, sweep) = stored_arc(&app, outcome.objects[0]);
    assert_close(radius, 3.0);
    assert_close(start, std::f64::consts::FRAC_PI_2);
    assert_close(sweep, -std::f64::consts::FRAC_PI_4);
}
