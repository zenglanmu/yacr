//! WIPEOUT create-command contract tests.
//!
//! The pinned shape: a `Points([boundary…])` payload with at least three finite
//! `+Z`-coplanar vertices becomes **one** `SemanticGeometry::Mask`
//! (stored type key `AcDbWipeout`, `inverted: false`, the first vertex not
//! repeated) in one transaction / one undo step on the active layer. Every
//! refusal (`wipeout.*`) happens before the transaction and changes nothing.
//! The declared v1 limits — no `Frames` model, no `Polyline`/`Erase Polyline`
//! option, no self-intersection rejection, no picking — are documented in
//! [`crate::command_id::draw_variants::WipeoutCommand`].

use super::*;

/// Assert the newest model-space entity is exactly a mask with `boundary` on
/// layer 0 in model space, stored under the `AcDbWipeout` type key.
fn assert_mask(app: &Application, object: ObjectId, boundary: &[Point3]) -> EntityId {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the wipeout entity");
    match &entity.geometry {
        SemanticGeometry::Mask {
            boundary: stored,
            inverted,
        } => {
            assert_eq!(stored, boundary, "the ring is stored exactly as clicked");
            assert!(!*inverted, "the create path authors inverted: false only");
        }
        other => panic!("expected a Mask, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbWipeout");
    EntityId(object.0)
}

/// A square boundary, in click order, with the first vertex **not** repeated.
fn square() -> Vec<Point3> {
    vec![
        point(0.0, 0.0),
        point(10.0, 0.0),
        point(10.0, 10.0),
        point(0.0, 10.0),
    ]
}

#[test]
fn create_wipeout_inserts_one_mask_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Points(square()),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one wipeout");

    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let entity_id = assert_mask(&app, outcome.objects[0], &square());
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the mask exactly; redo restores it.
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
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_some());
}

#[test]
fn create_wipeout_accepts_a_non_convex_polygon_and_keeps_click_order() {
    // Nothing in v1 re-orders, deduplicates or convexifies the ring: the stored
    // boundary is exactly the click order (a renderer fills it even-odd).
    let (mut app, mut session) = drawing_app();
    let boundary = vec![
        point(0.0, 0.0),
        point(4.0, 0.0),
        point(4.0, 1.0),
        point(1.0, 1.0),
        point(1.0, 4.0),
        point(0.0, 4.0),
    ];
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Points(boundary.clone()),
    )
    .unwrap();
    assert_mask(&app, outcome.objects[0], &boundary);
}

#[test]
fn create_wipeout_refuses_fewer_than_three_vertices_without_change() {
    for boundary in [
        vec![],
        vec![point(0.0, 0.0)],
        vec![point(0.0, 0.0), point(1.0, 0.0)],
        // A repeated closing first point is a duplicate edge, not a triangle.
        vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 0.0)],
    ] {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::Wipeout,
            CommandPayload::Points(boundary.clone()),
        )
        .unwrap_err();
        let CadError::InvalidInput(message) = err else {
            panic!("expected an InvalidInput refusal, got {err:?}");
        };
        assert!(
            message.contains("wipeout.too_few_points")
                || message.contains("wipeout.duplicate_vertex"),
            "unexpected refusal: {message}"
        );
        assert_eq!(revision(&app), rev, "a refused wipeout must not write");
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_wipeout_refuses_a_non_finite_vertex_without_change() {
    let cases = [
        vec![point(f64::NAN, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
        vec![
            point(0.0, 0.0),
            point(f64::INFINITY, 0.0),
            point(10.0, 10.0),
        ],
        vec![point(0.0, 0.0), point(10.0, 0.0), p3(10.0, 10.0, f64::NAN)],
    ];
    for boundary in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::Wipeout,
            CommandPayload::Points(boundary),
        )
        .unwrap_err();
        let CadError::InvalidInput(message) = err else {
            panic!("expected an InvalidInput refusal, got {err:?}");
        };
        assert!(
            message.contains("wipeout.invalid_point"),
            "unexpected refusal: {message}"
        );
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_wipeout_refuses_a_non_planar_ring_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Points(vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            p3(10.0, 10.0, 5.0),
            point(0.0, 10.0),
        ]),
    )
    .unwrap_err();
    let CadError::InvalidInput(message) = err else {
        panic!("expected an InvalidInput refusal, got {err:?}");
    };
    assert!(message.contains("wipeout.not_planar"), "{message}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_wipeout_refuses_a_duplicate_vertex_and_a_collapsed_ring() {
    // A zero-length edge (two consecutive identical clicks) and an all-collinear
    // ring are distinct named refusals; neither writes.
    let cases = [
        (
            vec![
                point(0.0, 0.0),
                point(2.0, 0.0),
                point(2.0, 0.0),
                point(0.0, 2.0),
            ],
            "wipeout.duplicate_vertex",
        ),
        (
            vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)],
            "wipeout.zero_area",
        ),
    ];
    for (boundary, reason) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::Wipeout,
            CommandPayload::Points(boundary),
        )
        .unwrap_err();
        let CadError::InvalidInput(message) = err else {
            panic!("expected an InvalidInput refusal, got {err:?}");
        };
        assert!(message.contains(reason), "expected {reason}, got {message}");
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_wipeout_refuses_a_wrong_payload_and_a_missing_active_layer() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Mask {
            boundary: square(),
            inverted: false,
        })),
    )
    .unwrap_err();
    let CadError::InvalidInput(message) = err else {
        panic!("expected an InvalidInput refusal, got {err:?}");
    };
    assert!(message.contains("wipeout.payload"), "{message}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));

    // A missing active layer is refused by the shared create funnel.
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Points(square()),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_wipeout_is_work_only() {
    // The command layer refuses it in Viewer mode (it writes the drawing).
    assert!(CommandId::Wipeout.requires_work_mode());
}
