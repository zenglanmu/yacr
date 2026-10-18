//! Click-select, gesture classification and selection-block session tests.

use super::super::input::{cancel_target, interaction_request, pointer_button, CancelTarget};
use super::super::*;
use super::test_support::*;

#[test]
fn cancel_target_keeps_the_host_cancel_out_of_an_active_capture() {
    // A capture owns the cancel: the host's import/open cancel must not also
    // fire, or an Esc inside a tool would abort an unrelated running import.
    assert_eq!(cancel_target(true), CancelTarget::Tool);
    // With no capture the selector keeps both meanings.
    assert_eq!(cancel_target(false), CancelTarget::ToolAndHost);
}

#[test]
fn plan_input_classifies_a_click_and_tags_it_with_the_current_revision() {
    let down = pointer_at(InputKind::PointerDown, 1, [10.0, 20.0], None);
    let (requests, press) = plan_input(None, &down, 7, false).unwrap();
    assert_eq!(requests.len(), 1);
    match &requests[0] {
        InteractionRequest::Pointer {
            phase: PointerPhase::Down,
            button: Some(PointerButton::Primary),
            position:
                PointInput::CanvasLocal {
                    logical_position,
                    metrics_revision,
                },
            ..
        } => {
            assert_eq!(*logical_position, [10.0, 20.0]);
            assert_eq!(*metrics_revision, 7);
        }
        other => panic!("unexpected request: {other:?}"),
    }
    let press = press.expect("a press is retained until its release");
    assert_eq!(press.button, PointerButton::Primary);
    assert!(!press.dragged);

    // A release with no movement is a click: the ordinary release (which
    // closes the host's gesture life) plus a replace-selection at the
    // release position tagged with the current metrics revision.
    let up = pointer_at(InputKind::PointerUp, 2, [10.75, 20.0], None);
    let (requests, next) = plan_input(Some(press), &up, 7, false).unwrap();
    assert!(next.is_none(), "a release ends the press");
    assert_eq!(requests.len(), 2, "release then select: {requests:?}");
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Up,
            ..
        }
    ));
    match &requests[1] {
        InteractionRequest::SelectAt {
            point:
                PointInput::CanvasLocal {
                    logical_position,
                    metrics_revision,
                },
            mode: SelectionMode::Replace,
        } => {
            assert_eq!(*logical_position, [10.75, 20.0]);
            assert_eq!(
                *metrics_revision, 7,
                "the click must carry the current revision, never a fabricated one"
            );
        }
        other => panic!("expected a SelectAt replace-selection: {other:?}"),
    }
}

#[test]
fn plan_input_keeps_a_drag_on_the_navigation_path() {
    let press = primary_press([10.0, 20.0]);
    // A move past the drag threshold marks the gesture as a drag.
    let moved = pointer_at(
        InputKind::PointerMove,
        2,
        [10.0 + DRAG_THRESHOLD_LOGICAL_PX, 20.0],
        None,
    );
    let (requests, press) = plan_input(Some(press), &moved, 7, false).unwrap();
    assert!(matches!(
        requests.as_slice(),
        [InteractionRequest::Pointer {
            phase: PointerPhase::Move,
            ..
        }]
    ));
    let press = press.expect("the press continues");
    assert!(press.dragged);

    // Releasing back at the press position is still a drag, not a click: a
    // round trip never fabricates a selection.
    let back = pointer_at(InputKind::PointerUp, 3, [10.0, 20.0], None);
    let (requests, next) = plan_input(Some(press), &back, 7, false).unwrap();
    assert!(next.is_none());
    assert_eq!(
        requests.len(),
        1,
        "a dragged release must not select: {requests:?}"
    );
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Up,
            ..
        }
    ));

    // A release whose total displacement exceeds the threshold is not a
    // click either, even when no move event reported the drag.
    let far = pointer_at(
        InputKind::PointerUp,
        4,
        [10.0 + DRAG_THRESHOLD_LOGICAL_PX, 20.0],
        None,
    );
    let (requests, _) = plan_input(Some(primary_press([10.0, 20.0])), &far, 7, false).unwrap();
    assert_eq!(
        requests.len(),
        1,
        "an end-to-end jump is a drag: {requests:?}"
    );
}

#[test]
fn plan_input_carries_the_pressed_button_through_a_gesture() {
    // The middle button is the pan gesture: the press button owns the move
    // even when the move reports no button change (the DOM's convention).
    let down = pointer_at(InputKind::PointerDown, 1, [0.0, 0.0], Some(BUTTON_MIDDLE));
    let (requests, press) = plan_input(None, &down, 3, false).unwrap();
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            button: Some(PointerButton::Middle),
            ..
        }
    ));
    let press = press.expect("the middle press is retained");
    assert_eq!(press.button, PointerButton::Middle);

    let moved = pointer_at(InputKind::PointerMove, 2, [30.0, 10.0], None);
    let (requests, press) = plan_input(Some(press), &moved, 3, false).unwrap();
    assert!(
        matches!(
            requests[0],
            InteractionRequest::Pointer {
                button: Some(PointerButton::Middle),
                ..
            }
        ),
        "the press button owns the gesture: {requests:?}"
    );
    assert!(press.expect("still pressed").dragged);

    // A middle release is never a click: no selection is invented from a
    // pan gesture.
    let up = pointer_at(InputKind::PointerUp, 3, [0.0, 0.0], Some(BUTTON_MIDDLE));
    let (requests, next) = plan_input(press, &up, 3, false).unwrap();
    assert_eq!(requests.len(), 1, "a middle click must not select");
    assert!(next.is_none());

    // The secondary button likewise never selects.
    let down = pointer_at(
        InputKind::PointerDown,
        4,
        [0.0, 0.0],
        Some(BUTTON_SECONDARY),
    );
    let (requests, press) = plan_input(None, &down, 3, false).unwrap();
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            button: Some(PointerButton::Secondary),
            ..
        }
    ));
    let up = pointer_at(InputKind::PointerUp, 5, [0.0, 0.0], Some(BUTTON_SECONDARY));
    let (requests, _) = plan_input(press, &up, 3, false).unwrap();
    assert_eq!(requests.len(), 1, "a secondary release must not select");

    // A release that names a different button than the press does not select.
    let up = pointer_at(InputKind::PointerUp, 6, [0.0, 0.0], Some(BUTTON_SECONDARY));
    let (requests, _) = plan_input(Some(primary_press([0.0, 0.0])), &up, 3, false).unwrap();
    assert_eq!(
        requests.len(),
        1,
        "a mismatched release button must not select: {requests:?}"
    );
}

#[test]
fn plan_input_refuses_an_undefined_button_instead_of_treating_it_as_primary() {
    assert_eq!(pointer_button(None), Ok(PointerButton::Primary));
    assert_eq!(
        pointer_button(Some(BUTTON_PRIMARY)),
        Ok(PointerButton::Primary)
    );
    assert_eq!(
        pointer_button(Some(BUTTON_MIDDLE)),
        Ok(PointerButton::Middle)
    );
    assert_eq!(
        pointer_button(Some(BUTTON_SECONDARY)),
        Ok(PointerButton::Secondary)
    );
    for undefined in [-1, 3, 4, 99] {
        let error = pointer_button(Some(undefined)).unwrap_err();
        assert!(
            error.contains(&undefined.to_string()),
            "the refusal names the value: {error}"
        );
        assert!(
            error.contains("unsupported pointer button"),
            "the refusal is stable: {error}"
        );
    }

    // Every pointer phase refuses it, and a refused press leaves no state.
    for kind in [
        InputKind::PointerDown,
        InputKind::PointerMove,
        InputKind::PointerUp,
    ] {
        let input = pointer_at(kind, 1, [1.0, 2.0], Some(-1));
        let error = plan_input(None, &input, 0, false).unwrap_err();
        assert!(error.contains("unsupported pointer button"), "{error}");
    }
    // The stateless mapping refuses it too (no silent primary fallback).
    assert!(interaction_request(
        &pointer_at(InputKind::PointerDown, 1, [1.0, 2.0], Some(9)),
        0
    )
    .is_err());
}

#[test]
fn plan_input_ends_the_press_on_cancel_and_keeps_it_across_a_wheel() {
    let cancel = pointer_at(InputKind::PointerCancel, 2, [1.0, 2.0], None);
    let (requests, next) = plan_input(Some(primary_press([0.0, 0.0])), &cancel, 0, false).unwrap();
    assert!(next.is_none(), "a cancel ends the press");
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Cancel,
            button: None,
            ..
        }
    ));

    let blur = probe(InputKind::Blur);
    let (_, next) = plan_input(Some(primary_press([0.0, 0.0])), &blur, 0, false).unwrap();
    assert!(next.is_none(), "a blur ends the press");

    let mut wheel = probe(InputKind::Wheel);
    wheel.x = Some(1.0);
    wheel.y = Some(2.0);
    wheel.dy = Some(-3.0);
    let press = primary_press([0.0, 0.0]);
    let (requests, next) = plan_input(Some(press), &wheel, 0, false).unwrap();
    assert!(matches!(
        requests[0],
        InteractionRequest::Scroll { delta_logical, .. } if delta_logical == [0.0, 3.0]
    ));
    assert_eq!(next, Some(press), "a wheel never disturbs the press");

    // Resize stays an explicit refusal (it goes through set_canvas_metrics).
    assert!(plan_input(None, &probe(InputKind::Resize), 0, false).is_err());
}

#[test]
fn a_synthesized_click_selects_the_picked_entity_and_feeds_the_overlay_flags_path() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    // The pick the host will run at that pixel: the click must select exactly
    // this object (never an invented one).
    let expected = {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        cad_app::picking::pick_at_screen(
            &drawing,
            c.document_id,
            &viewport.camera,
            pixel,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
        )
        .expect("the demo circle rim is pickable")
        .hit
        .expect("the demo circle rim is pickable")
        .source
    };
    let camera_before = viewport_camera(&session);

    session
        .handle_input(&pointer_at(InputKind::PointerDown, 1, pixel, None))
        .expect("the press is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerUp, 2, pixel, None))
        .expect("the release is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).error,
        None,
        "a click must complete without a business error"
    );

    let selected = session
        .core
        .controller
        .borrow()
        .session
        .selection
        .refs()
        .to_vec();
    assert_eq!(
        selected,
        vec![expected.clone()],
        "a click replaces the selection with the real pick result"
    );
    assert_eq!(
        viewport_camera(&session),
        camera_before,
        "a click must not pan the view"
    );
    // The selection reaches the overlay inputs that produce FLAG_SELECTED.
    assert!(
        session.core.render.overlays().selection.contains(&expected),
        "the picked ref must reach the entity-flags input"
    );

    // And the frozen status block describes it honestly.
    let selection = session.snapshot(None, None, 0).selection;
    assert_eq!(selection.count, 1);
    assert!(!selection.empty);
    assert!(selection.mixed_keys.is_empty());
    let id_row = selection
        .rows
        .iter()
        .find(|row| row.key == "id")
        .expect("a single selection reports its identity rows");
    assert_eq!(id_row.value, expected.entity.0.to_string());
}

#[test]
fn a_primary_drag_pans_without_selecting() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let camera_before = viewport_camera(&session);
    let start = screen_of(&session, demo_circle_rim(&session));
    let end = [
        start[0] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
        start[1] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
    ];

    session
        .handle_input(&pointer_at(InputKind::PointerDown, 1, start, None))
        .unwrap();
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, end, None))
        .unwrap();
    session
        .handle_input(&pointer_at(InputKind::PointerUp, 3, end, None))
        .unwrap();

    assert!(
        session
            .core
            .controller
            .borrow()
            .session
            .selection
            .is_empty(),
        "a drag beyond the threshold must not select"
    );
    assert_ne!(
        viewport_camera(&session),
        camera_before,
        "a primary drag still pans the view"
    );
    assert_eq!(
        session.snapshot(None, None, 0).selection,
        CadSelection::empty(),
        "the status block stays honestly empty"
    );
}

#[test]
fn a_middle_drag_pans_without_selecting() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let camera_before = viewport_camera(&session);
    let start = screen_of(&session, demo_circle_rim(&session));
    let end = [start[0], start[1] + 40.0];

    session
        .handle_input(&pointer_at(
            InputKind::PointerDown,
            1,
            start,
            Some(BUTTON_MIDDLE),
        ))
        .unwrap();
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, end, None))
        .unwrap();
    session
        .handle_input(&pointer_at(
            InputKind::PointerUp,
            3,
            end,
            Some(BUTTON_MIDDLE),
        ))
        .unwrap();

    assert!(
        session
            .core
            .controller
            .borrow()
            .session
            .selection
            .is_empty(),
        "the middle button is a pan gesture, never a selection"
    );
    assert_ne!(
        viewport_camera(&session),
        camera_before,
        "a middle drag pans the view"
    );
}

#[test]
fn cad_status_selection_block_is_always_present_and_honest() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // Empty.
    let empty = session.snapshot(None, None, 0).selection;
    assert_eq!(empty, CadSelection::empty());
    assert_eq!(empty.count, 0);
    assert!(empty.empty);
    assert!(empty.rows.is_empty() && empty.mixed_keys.is_empty());

    let (entities, document) = {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let mut ids: Vec<_> = drawing.model_space().iter().map(|e| e.id).collect();
        ids.truncate(2);
        (ids, c.document_id)
    };
    assert_eq!(entities.len(), 2, "the demo document has ≥2 entities");
    let refs: Vec<_> = entities
        .iter()
        .map(|entity| cad_app::selection::model_ref(document, *entity))
        .collect();

    // Single: the rows mirror the application's own property extraction.
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .replace([refs[0].clone()]);
    let single = session.snapshot(None, None, 0).selection;
    let expected = {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().unwrap();
        cad_app::SelectionProperties::extract(&drawing, &c.session.selection)
    };
    assert_eq!(single.count, 1);
    assert!(!single.empty);
    assert_eq!(single.count as usize, expected.count);
    assert_eq!(single.rows.len(), expected.rows.len());
    for (wire, row) in single.rows.iter().zip(expected.rows.iter()) {
        assert_eq!(wire.key, row.key, "the machine key is carried verbatim");
        assert_eq!(wire.value, row.value);
    }
    assert!(
        single.rows.iter().any(|row| row.key == "id")
            && single.rows.iter().any(|row| row.key == "type"),
        "identity rows are the stable machine keys: {:?}",
        single.rows
    );
    assert!(
        single.mixed_keys.is_empty(),
        "a single selection has no mixed keys"
    );

    // Multi: a count plus the mixed machine keys, never rows.
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .replace(refs.clone());
    let multi = session.snapshot(None, None, 0).selection;
    assert_eq!(multi.count, 2);
    assert!(!multi.empty);
    assert!(multi.rows.is_empty(), "a multi-selection carries no rows");
    assert!(
        !multi.mixed_keys.is_empty(),
        "two different entities must report mixed keys"
    );
    assert!(
        multi.mixed_keys.iter().any(|key| key == "id"),
        "the id machine key differs across distinct entities: {:?}",
        multi.mixed_keys
    );

    // Clearing it again is reflected by the next snapshot.
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .clear();
    assert_eq!(session.snapshot(None, None, 0).selection.count, 0);
}

#[test]
fn cad_status_selection_block_does_not_fabricate_rows_for_a_foreign_ref() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // A ref that names an entity this document does not contain (a foreign or
    // deleted identity) must stay an honest `unresolved` report.
    let foreign = cad_domain::SelectionRef {
        document: cad_domain::DocumentId(4242),
        entity: cad_domain::EntityId(999_999),
        instance: cad_domain::InstancePath::default(),
        sub_element: None,
    };
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .replace([foreign]);
    let selection = session.snapshot(None, None, 0).selection;
    assert_eq!(selection.count, 1);
    assert!(!selection.empty);
    let keys: Vec<&str> = selection.rows.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["id", "status"],
        "an unresolvable ref reports identity + status only"
    );
    assert_eq!(selection.rows[1].value, "unresolved");
    for fabricated in ["type", "layer", "length", "radius", "start"] {
        assert!(
            !keys.contains(&fabricated),
            "no geometry row may be invented for an unresolved ref"
        );
    }
    assert!(selection.mixed_keys.is_empty());
}

#[test]
fn a_session_without_a_document_reports_an_honest_empty_selection_block() {
    let status = unsupported_status(Some("CAD is disabled".into()));
    assert_eq!(status.selection, CadSelection::empty());
    assert!(status.selection.empty);
    assert_eq!(status.selection.count, 0);
    assert!(status.selection.rows.is_empty());
    assert!(status.selection.mixed_keys.is_empty());
}

#[test]
fn a_missing_property_source_never_hides_a_real_selection_behind_a_zero() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let (document, entity) = {
        let c = session.core.controller.borrow();
        let entity = c.drawing().expect("the demo document").model_space()[0].id;
        (c.document_id, entity)
    };
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .replace([cad_app::selection::model_ref(document, entity)]);
    // Remove the document: the property extraction has no source any more.
    session
        .core
        .controller
        .borrow_mut()
        .application
        .workspace
        .documents
        .remove(&document);

    let status = session.snapshot(None, None, 0);
    assert_eq!(
        status.document_label, None,
        "the missing document is honest"
    );
    assert_eq!(
        status.selection.count, 1,
        "the real ref count must not be hidden behind a fabricated 0"
    );
    assert!(!status.selection.empty);
    assert!(
        status.selection.rows.is_empty(),
        "no property row may be invented without a property source"
    );
    assert!(status.selection.mixed_keys.is_empty());
}
