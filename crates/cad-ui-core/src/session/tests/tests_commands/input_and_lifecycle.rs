//! Pointer/wheel input, navigation and session lifecycle contracts.

use super::super::super::input::{interaction_request, scroll_delta, POINTER_ID};
use super::super::super::*;
use super::super::test_support::*;

#[test]
fn dom_wheel_sign_zooms_in_on_wheel_up() {
    // DOM wheel-up is a negative deltaY; the host zooms in on a positive dy.
    assert_eq!(scroll_delta(3.0, -100.0), [3.0, 100.0]);
    assert_eq!(scroll_delta(0.0, 100.0), [0.0, -100.0]);
    // A positive host delta yields a > 1 zoom factor (the host's formula).
    assert!(1.0 + scroll_delta(0.0, -100.0)[1] * 0.0015 > 1.0);
}

#[test]
fn unsupported_and_malformed_inputs_are_explicit() {
    let resize = probe(InputKind::Resize);
    assert!(interaction_request(&resize, 0).is_err());

    let mut missing = probe(InputKind::PointerDown);
    missing.width = Some(100.0);
    assert!(interaction_request(&missing, 0).is_err());

    let mut non_finite = probe(InputKind::Wheel);
    non_finite.x = Some(1.0);
    non_finite.y = Some(1.0);
    non_finite.dy = Some(f32::NAN);
    assert!(interaction_request(&non_finite, 0).is_err());
}

#[test]
fn pointer_and_wheel_requests_carry_canvas_local_revision() {
    let mut down = probe(InputKind::PointerDown);
    down.x = Some(20.0);
    down.y = Some(30.0);
    match interaction_request(&down, 7).unwrap() {
        InteractionRequest::Pointer {
            phase: PointerPhase::Down,
            position:
                PointInput::CanvasLocal {
                    logical_position,
                    metrics_revision,
                },
            button: Some(PointerButton::Primary),
            ..
        } => {
            assert_eq!(logical_position, [20.0, 30.0]);
            assert_eq!(metrics_revision, 7);
        }
        other => panic!("unexpected request: {other:?}"),
    }

    // A cancel without coordinates is accepted with the origin placeholder.
    assert!(interaction_request(&probe(InputKind::PointerCancel), 7).is_ok());
    assert!(interaction_request(&probe(InputKind::Blur), 7).is_ok());

    let mut wheel = probe(InputKind::Wheel);
    wheel.x = Some(5.0);
    wheel.y = Some(6.0);
    wheel.dx = Some(2.0);
    wheel.dy = Some(-4.0);
    match interaction_request(&wheel, 7).unwrap() {
        InteractionRequest::Scroll { delta_logical, .. } => {
            assert_eq!(delta_logical, [2.0, 4.0]);
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
fn repeated_navigation_does_not_leak_queue_quota() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // Well past the queue's normal (240) and absolute (256) capacity: if
    // completions were not consumed, admission would fail with QueueFull.
    for index in 0..600u64 {
        let mut input = move_input(index as f32, index as f32);
        input.sequence = index + 1;
        session
            .handle_input(&input)
            .unwrap_or_else(|error| panic!("navigation {index} refused: {error}"));
    }
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0, "navigation is not an import");
    assert_eq!(status.error, None, "successful navigation leaves no error");
}

#[test]
fn stale_context_completion_surfaces_as_an_error_not_empty_success() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let context = session.core.request_context();
    let request = EngineRequest::Interaction(InteractionRequest::Pointer {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        phase: PointerPhase::Move,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: [1.0, 2.0],
            metrics_revision: context.metrics_revision.unwrap_or(0),
        },
        modifiers: Modifiers::default(),
    });
    let receipt = session.core.submit(context, request).unwrap();
    session.track_receipt(receipt.id);
    // The canvas metrics revision advances before the queued request runs,
    // so the host grades it stale.
    session.set_canvas_metrics([640.0, 480.0], 1.0).unwrap();
    session.pump().unwrap();
    let outcome = session.drain_completions();
    assert!(
        outcome.is_err(),
        "a stale completion must not be an empty success"
    );
    assert!(session.snapshot(None, None, 0).error.is_some());
}

#[test]
fn canvas_metrics_accept_valid_and_refuse_degenerate_input() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session.set_canvas_metrics([800.0, 600.0], 1.0).unwrap();
    assert!(session.set_canvas_metrics([0.0, 600.0], 1.0).is_err());
    assert!(session.set_canvas_metrics([800.0, 600.0], 0.0).is_err());
}

#[test]
fn tick_tracks_the_live_surface_rect_so_later_inputs_anchor_correctly() {
    let surface = MutableSurface::new();
    surface.set_metrics(SurfaceMetrics {
        logical: [640.0, 480.0],
        scale: 1.0,
        safe_insets: [0.0; 4],
    });
    let session = session_with_surface(
        Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }),
        surface.clone(),
    );
    assert_eq!(canvas_logical_size(&session), [640.0, 480.0]);
    // The frontend reports a new CAD rect. Until the owner tick re-reads the
    // surface the viewport still holds the creation-time size: exactly the
    // stale state that made an off-center wheel zoom anchor off the cursor.
    surface.set_metrics(SurfaceMetrics {
        logical: [1024.0, 768.0],
        scale: 1.0,
        safe_insets: [0.0; 4],
    });
    assert_eq!(
        canvas_logical_size(&session),
        [640.0, 480.0],
        "stale viewport until the owner tick re-reads the surface"
    );
    session.tick().expect("tick");
    assert_eq!(
        canvas_logical_size(&session),
        [1024.0, 768.0],
        "the tick must re-apply the accepted surface rect to the viewport"
    );
}

#[test]
fn note_frame_is_the_actual_present_count() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    assert_eq!(session.frames_presented(), 0);
    session.note_frame(12);
    assert_eq!(session.frames_presented(), 12);
    assert_eq!(session.snapshot(None, None, 0).frame_count, 12);
    assert_eq!(session.snapshot(None, None, 99).frame_count, 99);
}

#[test]
fn stale_resource_guard_clears_caches_once_per_generation() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // No failure status: nothing to guard.
    session.guard_stale_resources();
    assert_eq!(session.resource_guard.get(), None);
    // A resource-install failure on the current generation is guarded once.
    session
        .core
        .set_catalog_status(RESOURCE_FAILED_STATUS, &[("error", "no resolver")])
        .unwrap();
    session.guard_stale_resources();
    let generation = session.core.request_context().document_generation;
    assert_eq!(session.resource_guard.get(), Some(generation));
    session.guard_stale_resources();
    assert_eq!(session.resource_guard.get(), Some(generation));
}

#[test]
fn startup_diagnostics_accessor_is_exposed() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let _ = session.startup_diagnostics();
}
