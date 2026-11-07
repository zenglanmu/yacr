//! GRADIENT creation contract tests (AutoCAD 2026).
//!
//! GRADIENT is the HATCH creation path with the fill forced to the session
//! gradient channel; these tests pin the one-entity/one-undo shape, the stored
//! `AcDbHatch` type key and gradient fill, and the named refusal surface
//! (nothing written, no transaction) for an empty selection, an open boundary,
//! a non-`+Z` plane, a non-coplanar selection, an ellipse arc and Viewer mode.

use super::hatch::{circle, entity_count, hatch_app, square, the_hatch, undo_depth};
use super::*;

use cad_domain::{HatchFill, HatchGradient, HatchPatternType};

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn gradient(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::Gradient, CommandPayload::None)
}

/// A closed rectangle in the plane `x = 0` (normal +X, not +Z).
fn tilted_square(id: u128) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Polyline {
            points: vec![
                p3(0.0, 0.0, 0.0),
                p3(0.0, 4.0, 0.0),
                p3(0.0, 4.0, 4.0),
                p3(0.0, 0.0, 4.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
        },
    )
}

#[test]
fn a_selected_rectangle_creates_one_gradient_hatch_in_one_undo_step() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    select(&mut session, &[1]);

    let out = gradient(&mut app, &mut session).expect("GRADIENT succeeds");
    assert_eq!(out.objects.len(), 1, "one hatch entity");

    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .expect("the created entity exists");
    assert_eq!(entity.object.type_key, "AcDbHatch");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);

    let hatch = the_hatch(&app);
    let HatchFill::Gradient(HatchGradient::TwoColor {
        name, angle_raw, ..
    }) = hatch.fill()
    else {
        panic!(
            "GRADIENT must store a two-colour gradient, got {:?}",
            hatch.fill()
        );
    };
    assert_eq!(name, "LINEAR", "the default session kind is linear");
    assert_eq!(*angle_raw, 0.0, "the default session angle is 0 degrees");
    assert_eq!(
        hatch.pattern_type(),
        HatchPatternType::Predefined,
        "a gradient records 76=1, exactly like HATCH's gradient branch"
    );

    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(entity_count(&app), 2, "the boundary entity is retained");
    let message = out
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.hatch")
        .map(|diagnostic| diagnostic.message.clone())
        .expect("a hatch summary diagnostic");
    assert!(message.starts_with("GRADIENT:"), "{message}");
    assert!(message.contains("gradient LINEAR"), "{message}");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "one undo removes the gradient");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_empty_selection_is_gradient_selection_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("empty selection must refuse");
    let text = format!("{error:?}");
    assert!(text.contains("gradient.selection"), "{text}");
    assert_eq!(revision(&app), before, "a refusal must not write");
    assert_eq!(undo_depth(&app), 0, "a refusal must not open a transaction");
}

#[test]
fn an_open_polyline_is_refused_with_the_gradient_prefix() {
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Polyline {
            points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
            bulges: vec![0.0; 3],
            closed: false,
        },
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("an open boundary must refuse");
    let text = format!("{error:?}");
    assert!(
        text.contains("GRADIENT boundary entity 1 is an open polyline"),
        "{text}"
    );
    assert!(
        text.contains("hatch."),
        "the shared hatch code stays named: {text}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_non_plus_z_boundary_is_refused_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![tilted_square(1)]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("a tilted boundary must refuse");
    let text = format!("{error:?}");
    assert!(text.contains("gradient.not_plus_z"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_non_coplanar_selection_is_refused_with_the_gradient_prefix() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 4.0, 0.0), square(2, 0.0, 4.0, 3.0)]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("non-coplanar must refuse");
    let text = format!("{error:?}");
    assert!(
        text.contains("GRADIENT boundaries must be coplanar"),
        "{text}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_ellipse_arc_and_a_non_boundary_kind_are_refused_by_name() {
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Ellipse {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(4.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("an ellipse arc must refuse");
    let text = format!("{error:?}");
    assert!(
        text.contains("GRADIENT boundary entity 1 (ellipse) is an arc"),
        "{text}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // A circle is accepted; nothing else here claims another boundary kind.
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    select(&mut session, &[1]);
    gradient(&mut app, &mut session).expect("a circle is an exact gradient boundary");
}

#[test]
fn viewer_mode_refuses_gradient_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    select(&mut session, &[1]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = gradient(&mut app, &mut session).expect_err("Viewer must refuse");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_payload_on_gradient_is_refused_as_gradient_payload() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Gradient,
        CommandPayload::Hatch {
            refs: vec![ref_of(1)],
        },
    )
    .expect_err("GRADIENT takes no payload");
    let text = format!("{error:?}");
    assert!(text.contains("gradient.payload"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
