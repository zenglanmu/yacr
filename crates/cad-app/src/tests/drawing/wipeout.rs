//! WIPEOUT create-command contract tests.
//!
//! The pinned shape: a `Points([boundary…])` payload with at least three finite
//! `+Z`-coplanar vertices becomes **one** `SemanticGeometry::Mask`
//! (stored type key `AcDbWipeout`, `inverted: false`, the first vertex not
//! repeated) in one transaction / one undo step on the active layer. Every
//! refusal (`wipeout.*`) happens before the transaction and changes nothing.
//!
//! Also pinned here: the `Polyline` (+ `Erase Polyline Yes/No`) conversion and
//! the `WIPEOUTFRAME` (`Frames ON|OFF|Display but not plot`) option, which is
//! **session/drawing-wide display state**: it records no database write, no
//! revision and no undo entry. The frame's *visibility* is a renderer concern —
//! the display build draws the frame for `ON`/`DISPLAY`, and the vector-plot
//! build only for `ON` — and that mapping is pinned by the `cad-representation`
//! and `cad-host` tests, not here. This crate's tests therefore prove the token
//! write/read path and the no-write discipline only; they are **not** a pixel or
//! plotting acceptance of the frame itself.
//!
//! Still declared limits: no picking an existing wipeout, no `FRAME` system
//! variable command, and self-intersection is not rejected (the ring is stored as
//! given and filled even-odd; only a zero net area is refused).

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

// --- 2026-11 WIPEOUT Frames / Polyline options -------------------------------

use crate::command_payload::{UtilityPayload, WipeoutPayload};

fn wipeout_payload(payload: WipeoutPayload) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(Box::new(
        UtilityPayload::Wipeout(payload),
    ))))
}

#[test]
fn wipeout_frames_state_is_session_wide_and_writes_nothing() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    for (mode, value, label) in [
        (crate::WipeoutFrames::Off, 0, "OFF"),
        (crate::WipeoutFrames::On, 1, "ON"),
        (crate::WipeoutFrames::DisplayNotPlot, 2, "DISPLAY-NOT-PLOT"),
    ] {
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::Wipeout,
            wipeout_payload(WipeoutPayload::Frames(mode)),
        )
        .expect("Frames stores the session state");
        assert_eq!(session.drafting_extras.wipeout_frames(), mode);
        assert_eq!(session.drafting_extras.wipeout_frames().value(), value);
        assert_eq!(session.drafting_extras.wipeout_frames().label(), label);
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "wipeout.frames"));
        assert!(
            outcome
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "wipeout.frames_display_only"),
            "the frame/plot split is implemented now, so the old display-only \
             diagnostic must be gone"
        );
    }
    assert_eq!(revision(&app), before, "Frames never writes the drawing");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "Frames records no undo entry"
    );
}

#[test]
fn wipeout_polyline_erase_yes_and_no_share_one_transaction() {
    let (mut app, mut session) = drawing_app();
    let rectangle = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap()
    .objects[0]
        .0;
    let before = revision(&app);
    let undo_before = app.history[&DocumentId(1)].undo_depth();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        wipeout_payload(WipeoutPayload::Polyline {
            target: ref_of(rectangle),
            erase: true,
        }),
    )
    .expect("Polyline converts and erases the source");
    let mask = outcome.objects[0];
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(rectangle))
            .is_none(),
        "Erase Polyline Yes deletes the source"
    );
    assert_mask(
        &app,
        mask,
        &[
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 3.0),
            point(0.0, 3.0),
        ],
    );
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        undo_before + 1,
        "one undo step"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(rectangle))
            .is_some(),
        "undo restores the erased polyline"
    );
    assert!(find_wipeout(&app).is_none(), "undo removes the mask");

    let rectangle = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(20.0, 0.0), point(24.0, 3.0)]),
    )
    .unwrap()
    .objects[0]
        .0;
    execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        wipeout_payload(WipeoutPayload::Polyline {
            target: ref_of(rectangle),
            erase: false,
        }),
    )
    .expect("Polyline keeps the source when Erase is No");
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(rectangle))
            .is_some(),
        "Erase Polyline No keeps the source"
    );
}

fn find_wipeout(app: &Application) -> Option<cad_db::DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Mask { .. }))
        .cloned()
}

#[test]
fn wipeout_polyline_refuses_open_and_non_polyline_targets_without_writing() {
    let (mut app, mut session) = drawing_app();
    let open = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap()
    .objects[0]
        .0;
    let before = revision(&app);
    let message = match execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        wipeout_payload(WipeoutPayload::Polyline {
            target: ref_of(open),
            erase: false,
        }),
    )
    .unwrap_err()
    {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(message.contains("wipeout.polyline_open"), "{message}");
    assert_eq!(revision(&app), before);

    let message = match execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        wipeout_payload(WipeoutPayload::Polyline {
            target: ref_of(1),
            erase: false,
        }),
    )
    .unwrap_err()
    {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(
        message.contains("wipeout.polyline_not_polyline"),
        "{message}"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn wipeout_frames_are_display_state_and_leave_a_created_mask_untouched() {
    // The frame flag is drawing-wide display state, not entity data: a mask
    // created while frames are OFF stores exactly the same ring/inversion, and
    // the token survives the write unchanged.
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        wipeout_payload(WipeoutPayload::Frames(crate::WipeoutFrames::Off)),
    )
    .expect("Frames OFF");
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Points(square()),
    )
    .expect("a mask is created with frames hidden");
    assert_eq!(revision(&app).0, before.0 + 1, "only the mask write bumps");
    assert_mask(&app, outcome.objects[0], &square());
    assert_eq!(
        session.drafting_extras.wipeout_frames(),
        crate::WipeoutFrames::Off,
        "creating a mask does not reset WIPEOUTFRAME"
    );
    // A mask carries no per-entity frame flag, so the display decision can only
    // come from the session token (the renderer wiring lives outside this crate).
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the mask");
    assert!(
        matches!(
            entity.geometry,
            SemanticGeometry::Mask {
                inverted: false,
                ..
            }
        ),
        "the stored mask is the only frame-related datum this crate owns"
    );
}

/// The current undo depth (0 when the drawing has no history yet).
fn wipeout_undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn wipeout_refuses_another_utility_payload_and_keeps_the_frames_token() {
    // A Utility payload that is not WIPEOUT's own is refused by name before any
    // write, and the session frame token is untouched by the refusal.
    let (mut app, mut session) = drawing_app();
    session
        .drafting_extras
        .set_wipeout_frames(crate::WipeoutFrames::DisplayNotPlot);
    let before = revision(&app);
    let undo_before = wipeout_undo_depth(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::Wipeout,
        CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(Box::new(
            UtilityPayload::TextMask(crate::TextMaskOptions::default()),
        )))),
    )
    .unwrap_err();
    let CadError::InvalidInput(message) = err else {
        panic!("expected an InvalidInput refusal, got {err:?}");
    };
    assert!(message.contains("wipeout.payload"), "{message}");
    assert_eq!(revision(&app), before);
    assert_eq!(wipeout_undo_depth(&app), undo_before);
    assert_eq!(
        session.drafting_extras.wipeout_frames(),
        crate::WipeoutFrames::DisplayNotPlot,
        "a refused WIPEOUT never rewrites the frame token"
    );
}
