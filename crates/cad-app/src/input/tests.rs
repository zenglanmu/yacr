use super::*;
use crate::{AppMode, DocumentId, ViewportId};
use cad_domain::{Completeness, ObjectId};

fn viewport() -> Viewport {
    Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0])
}

fn session_with_tool(tool: ToolState) -> SessionState {
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.tool = tool;
    session
}

fn press(policy: &mut InputPolicy, pos: [f64; 2]) -> InputOutcome {
    policy.handle(PointerUpdate {
        logical_position: pos,
        contacts: 1,
    })
}

fn move_to(policy: &mut InputPolicy, pos: [f64; 2]) -> InputOutcome {
    policy.handle(PointerUpdate {
        logical_position: pos,
        contacts: 1,
    })
}

fn release(policy: &mut InputPolicy, pos: [f64; 2]) -> InputOutcome {
    policy.handle(PointerUpdate {
        logical_position: pos,
        contacts: 0,
    })
}

// --- U05 pointer policy ---

#[test]
fn a_tap_commits_at_the_down_point() {
    let mut policy = InputPolicy::new();
    assert_eq!(
        press(&mut policy, [10.0, 20.0]),
        InputOutcome::Preview {
            logical: [10.0, 20.0]
        }
    );
    // Released without moving: a mouse click commits the down point.
    assert_eq!(
        release(&mut policy, [10.0, 20.0]),
        InputOutcome::Commit {
            logical: [10.0, 20.0]
        }
    );
    assert_eq!(*policy.phase(), PointerPhase::Idle);
}

#[test]
fn movement_under_the_drag_threshold_does_not_commit() {
    let mut policy = InputPolicy::new();
    press(&mut policy, [0.0, 0.0]);
    let out = move_to(&mut policy, [DRAG_THRESHOLD_LOGICAL_PX - 0.5, 0.0]);
    assert!(matches!(out, InputOutcome::Preview { .. }));
    assert!(matches!(policy.phase(), PointerPhase::Pending { .. }));
}

#[test]
fn crossing_the_drag_threshold_commits_the_anchor_once() {
    let mut policy = InputPolicy::new();
    press(&mut policy, [0.0, 0.0]);
    let out = move_to(&mut policy, [DRAG_THRESHOLD_LOGICAL_PX, 0.0]);
    assert_eq!(
        out,
        InputOutcome::Commit {
            logical: [0.0, 0.0]
        }
    );
    assert!(matches!(policy.phase(), PointerPhase::Dragging { .. }));
    // Further movement only moves the preview.
    let out = move_to(&mut policy, [100.0, 100.0]);
    assert!(matches!(out, InputOutcome::Preview { .. }));
    assert_eq!(
        release(&mut policy, [100.0, 100.0]),
        InputOutcome::Commit {
            logical: [0.0, 0.0]
        }
    );
}

#[test]
fn two_finger_gesture_never_commits_and_cancels_the_tool() {
    let mut policy = InputPolicy::new();
    // A finger is mid-draw.
    press(&mut policy, [5.0, 5.0]);
    move_to(&mut policy, [50.0, 5.0]);
    assert!(matches!(policy.phase(), PointerPhase::Dragging { .. }));

    // The second finger lands: the tool is cancelled, never committed.
    let out = policy.handle(PointerUpdate {
        logical_position: [60.0, 5.0],
        contacts: 2,
    });
    assert_eq!(
        out,
        InputOutcome::ToolCancelled {
            reason: CancelReason::MultiTouch
        }
    );
    assert!(matches!(policy.phase(), PointerPhase::Navigate { .. }));

    // Lifting both fingers produces no commit.
    assert_eq!(release(&mut policy, [60.0, 5.0]), InputOutcome::Ignored);
}

#[test]
fn two_finger_pan_reports_a_delta_then_navigates() {
    let mut policy = InputPolicy::new();
    let first = policy.handle(PointerUpdate {
        logical_position: [10.0, 10.0],
        contacts: 2,
    });
    // Starting navigation from idle cancels nothing.
    assert_eq!(first, InputOutcome::Pan { delta: [0.0, 0.0] });
    let moved = policy.handle(PointerUpdate {
        logical_position: [30.0, 25.0],
        contacts: 2,
    });
    assert_eq!(
        moved,
        InputOutcome::Pan {
            delta: [20.0, 15.0]
        }
    );
}

#[test]
fn remaining_single_contact_during_navigation_does_not_snap_into_drawing() {
    let mut policy = InputPolicy::new();
    policy.handle(PointerUpdate {
        logical_position: [0.0, 0.0],
        contacts: 2,
    });
    // One finger lifts; still in navigation, no commit.
    let out = press(&mut policy, [0.0, 0.0]);
    assert_eq!(out, InputOutcome::Ignored);
    assert!(matches!(policy.phase(), PointerPhase::Navigate { .. }));
}

#[test]
fn pointer_cancel_cancels_an_active_tool() {
    let mut policy = InputPolicy::new();
    press(&mut policy, [1.0, 1.0]);
    let out = policy.pointer_cancelled();
    assert_eq!(
        out,
        InputOutcome::ToolCancelled {
            reason: CancelReason::PointerCancelled
        }
    );
    assert_eq!(*policy.phase(), PointerPhase::Idle);
    // A second cancel with no active gesture is a no-op.
    assert_eq!(policy.pointer_cancelled(), InputOutcome::Ignored);
}

#[test]
fn non_finite_pointer_is_ignored_not_committed() {
    let mut policy = InputPolicy::new();
    let out = policy.handle(PointerUpdate {
        logical_position: [f64::NAN, 0.0],
        contacts: 1,
    });
    assert_eq!(out, InputOutcome::Ignored);
    assert_eq!(*policy.phase(), PointerPhase::Idle);
}

// --- U06 keyboard/IME policy ---

#[test]
fn text_focus_suppresses_shortcuts_and_preserves_escape_for_the_field() {
    let policy = {
        let mut p = InputPolicy::new();
        p.set_text_focus(true);
        p
    };
    assert_eq!(classify_key(&policy, false), KeyAction::Suppressed);
    assert_eq!(classify_key(&policy, true), KeyAction::Suppressed);
    let session = session_with_tool(ToolState::Idle);
    // With focus held by a field, Esc does not reach the canvas.
    assert_eq!(
        escape_action(&policy, &session),
        EscapeAction::DismissTextFocus
    );
}

#[test]
fn composing_text_is_never_a_shortcut() {
    let mut policy = InputPolicy::new();
    policy.set_composing(true);
    assert!(policy.shortcuts_suppressed());
    assert_eq!(classify_key(&policy, false), KeyAction::Suppressed);
    // Ending composition releases the shortcut.
    policy.set_composing(false);
    assert_eq!(classify_key(&policy, false), KeyAction::Shortcut);
}

#[test]
fn losing_text_focus_clears_a_stale_composition() {
    let mut policy = InputPolicy::new();
    policy.set_text_focus(true);
    policy.set_composing(true);
    policy.set_text_focus(false);
    assert!(!policy.composing());
    assert!(!policy.shortcuts_suppressed());
}

#[test]
fn escape_cancels_the_active_tool_before_exit() {
    let policy = InputPolicy::new();
    let measuring = session_with_tool(ToolState::Measuring(crate::MeasurementTool::new(
        crate::MeasurementToolKind::Distance,
    )));
    assert_eq!(escape_action(&policy, &measuring), EscapeAction::CancelTool);
    let idle = session_with_tool(ToolState::Idle);
    assert_eq!(escape_action(&policy, &idle), EscapeAction::Exit);
}

#[test]
fn escape_with_text_focus_never_cancels_the_canvas_tool() {
    let mut policy = InputPolicy::new();
    policy.set_text_focus(true);
    let measuring = session_with_tool(ToolState::Measuring(crate::MeasurementTool::new(
        crate::MeasurementToolKind::Distance,
    )));
    assert_eq!(
        escape_action(&policy, &measuring),
        EscapeAction::DismissTextFocus
    );
}

// --- U07 view metrics ---

#[test]
fn canvas_origin_is_subtracted_before_mapping_to_world() {
    let mut vp = viewport();
    vp.camera.projection = crate::Projection::Orthographic { scale: 2.0 };
    vp.camera.target = Point3 {
        x: 10.0,
        y: 0.0,
        z: 0.0,
    };
    // Canvas occupies the bottom region of the surface: origin (0, 100).
    let canvas = CanvasMetrics::new([0.0, 100.0], [800.0, 600.0], 1.0);
    let metrics = ViewMetrics::new(canvas, &vp);

    // A surface point at the canvas centre maps to the camera target.
    let world = metrics.surface_to_world([400.0, 400.0]).unwrap();
    assert!((world.x - 10.0).abs() < 1e-9);
    assert!((world.y - 0.0).abs() < 1e-9);

    // Without subtracting the origin the result would differ, proving the
    // origin is actually applied.
    let naive = metrics.surface_to_world([400.0, 300.0]).unwrap();
    assert!((naive.y - 0.0).abs() > 1.0);
}

#[test]
fn canvas_to_physical_uses_the_dpi_scale() {
    let canvas = CanvasMetrics::new([0.0, 0.0], [400.0, 300.0], 2.0);
    let vp = viewport();
    let metrics = ViewMetrics::new(canvas, &vp);
    assert_eq!(metrics.canvas.size_physical(), Some([800.0, 600.0]));
    assert_eq!(metrics.canvas_to_physical([10.0, 20.0]), Some([20.0, 40.0]));
}

#[test]
fn degenerate_metrics_are_rejected_explicitly() {
    let zero = CanvasMetrics::new([0.0, 0.0], [0.0, 600.0], 1.0);
    assert!(!zero.is_valid());
    assert_eq!(zero.size_physical(), None);
    let vp = viewport();
    let metrics = ViewMetrics::new(zero, &vp);
    assert!(metrics.surface_to_world([1.0, 1.0]).is_none());

    let nan_dpi = CanvasMetrics::new([0.0, 0.0], [800.0, 600.0], f64::NAN);
    assert!(!nan_dpi.is_valid());

    let mut vp = viewport();
    assert!(apply_canvas_metrics(&mut vp, &zero).is_err());
}

#[test]
fn dpi_scale_does_not_change_the_world_point_of_a_logical_pixel() {
    // The camera is defined in logical pixels; the same logical surface
    // point must map to the same world point at DPR 1 and DPR 2, while the
    // physical pixel differs. This is the "drawing and picking agree"
    // invariant.
    let mut vp = viewport();
    vp.camera.projection = crate::Projection::Orthographic { scale: 1.0 };
    let dpr1 = ViewMetrics::new(CanvasMetrics::new([0.0, 0.0], [800.0, 600.0], 1.0), &vp);
    let dpr2 = ViewMetrics::new(CanvasMetrics::new([0.0, 0.0], [800.0, 600.0], 2.0), &vp);
    let a = dpr1.surface_to_world([200.0, 150.0]).unwrap();
    let b = dpr2.surface_to_world([200.0, 150.0]).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        dpr1.canvas_to_physical([200.0, 150.0]),
        Some([200.0, 150.0])
    );
    assert_eq!(
        dpr2.canvas_to_physical([200.0, 150.0]),
        Some([400.0, 300.0])
    );
}

#[test]
fn pan_delta_converts_physical_pixels_down_to_logical() {
    let vp = {
        let mut v = viewport();
        v.camera.projection = crate::Projection::Orthographic { scale: 4.0 };
        v
    };
    let metrics = ViewMetrics::new(CanvasMetrics::new([0.0, 0.0], [800.0, 600.0], 2.0), &vp);
    // 20 physical px at DPR 2 = 10 logical px * 4 world/px = 40 world.
    assert_eq!(metrics.pan_delta_world([20.0, 0.0]), Some([40.0, 0.0]));
}

#[test]
fn safe_area_insets_shrink_the_canvas_origin_and_size() {
    // [top, right, bottom, left] on a 800x600 surface at DPR 1.
    let canvas = inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [24.0, 8.0, 16.0, 4.0], 1.0)
        .expect("a real inset yields a canvas");
    assert_eq!(canvas.origin_logical, [4.0, 24.0]);
    assert_eq!(
        canvas.size_logical,
        [800.0 - 8.0 - 4.0, 600.0 - 24.0 - 16.0]
    );
    assert_eq!(canvas.size_physical(), Some([788.0, 560.0]));
}

#[test]
fn zero_insets_leave_the_surface_rect_untouched() {
    // Linux has no portable safe area: explicit zeros must be a no-op, not a
    // fabricated margin.
    let canvas = inset_canvas_metrics([10.0, 20.0], [800.0, 600.0], [0.0; 4], 2.0)
        .expect("zero insets are valid");
    assert_eq!(
        canvas,
        CanvasMetrics::new([10.0, 20.0], [800.0, 600.0], 2.0)
    );
}

#[test]
fn inset_canvas_metrics_keeps_the_dpi_scale_for_physical_pixels() {
    // The inset is logical-pixel geometry; only `size_physical` applies DPR,
    // and the same logical surface point maps to the same world point at
    // DPR 1/2/3 (the U07 invariant).
    let dpr1 =
        inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [10.0, 0.0, 10.0, 0.0], 1.0).unwrap();
    let dpr3 =
        inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [10.0, 0.0, 10.0, 0.0], 3.0).unwrap();
    assert_eq!(dpr1.size_logical, dpr3.size_logical);
    assert_eq!(dpr1.origin_logical, dpr3.origin_logical);
    assert_eq!(dpr1.size_physical(), Some([800.0, 580.0]));
    assert_eq!(dpr3.size_physical(), Some([2400.0, 1740.0]));

    let mut vp = viewport();
    vp.camera.projection = crate::Projection::Orthographic { scale: 1.0 };
    let a = ViewMetrics::new(dpr1, &vp)
        .surface_to_world([200.0, 150.0])
        .unwrap();
    let b = ViewMetrics::new(dpr3, &vp)
        .surface_to_world([200.0, 150.0])
        .unwrap();
    assert_eq!(a, b);
}

#[test]
fn degenerate_insets_are_rejected_explicitly() {
    // Negative inset: refused, never clamped to zero.
    assert!(inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [-1.0, 0.0, 0.0, 0.0], 1.0).is_none());
    // NaN inset: refused.
    assert!(
        inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [f64::NAN, 0.0, 0.0, 0.0], 1.0).is_none()
    );
    // Insets larger than the surface leave no canvas.
    assert!(
        inset_canvas_metrics([0.0, 0.0], [100.0, 100.0], [60.0, 60.0, 60.0, 60.0], 1.0).is_none()
    );
    // A degenerate surface or DPI is refused through the same check.
    assert!(inset_canvas_metrics([0.0, 0.0], [0.0, 600.0], [0.0; 4], 1.0).is_none());
    assert!(inset_canvas_metrics([0.0, 0.0], [800.0, 600.0], [0.0; 4], f64::NAN).is_none());
}

// --- U08 status ---

#[test]
fn status_summary_never_embeds_a_diagnostic_message() {
    let session = session_with_tool(ToolState::Measuring(crate::MeasurementTool::new(
        crate::MeasurementToolKind::Distance,
    )));
    let status = StatusModel::from_session(&session, Some("mm".to_string()), LoadingState::Idle);
    let line = status.summary_line();
    assert!(line.contains("测量"));
    assert!(line.contains("mm"));
    // A status line is fixed prose, not the first diagnostic text.
    assert_eq!(line, "就绪 · 测量 · mm");
}

#[test]
fn diagnostics_drawer_keeps_every_reason_and_object() {
    let mut drawer = DiagnosticsDrawer::new();
    drawer.backend = Some("WebGPU".to_string());
    // Two objects, two reasons on the first: never flatten to the first.
    drawer.model.add(
        ObjectId(1),
        cad_diagnostics::DiagnosticReason::missing(
            cad_diagnostics::codes::REPRESENTATION_UNAVAILABLE,
            vec![],
        ),
    );
    drawer.model.add(
        ObjectId(1),
        cad_diagnostics::DiagnosticReason::partial(
            cad_diagnostics::codes::RESOURCE_MISSING,
            vec![],
        ),
    );
    drawer.model.add(
        ObjectId(2),
        cad_diagnostics::DiagnosticReason::missing(cad_diagnostics::codes::FONT_UNRESOLVED, vec![]),
    );
    drawer
        .model
        .add_document(cad_diagnostics::DiagnosticReason::partial(
            cad_diagnostics::codes::RESOURCE_OVER_BUDGET,
            vec![],
        ));

    assert!(drawer.has_findings());
    assert_eq!(drawer.objects_with_reasons().len(), 2);
    assert_eq!(drawer.reason_count(), 4);
    let summary = drawer.summary();
    assert_eq!(summary.objects, 2);
    assert_eq!(summary.missing_objects, 2);
    assert!(matches!(summary.completeness, Completeness::Missing(_)));
    // The worst verdict wins; a complete object cannot hide a missing one.
    assert!(!summary.complete);
}

#[test]
fn an_empty_drawer_is_complete() {
    let drawer = DiagnosticsDrawer::new();
    assert!(!drawer.has_findings());
    assert!(drawer.objects_with_reasons().is_empty());
    assert_eq!(drawer.reason_count(), 0);
}

#[test]
fn apply_canvas_metrics_updates_size_and_dpi_but_not_the_camera() {
    let mut vp = viewport();
    let before = vp.camera;
    let canvas = CanvasMetrics::new([0.0, 40.0], [400.0, 300.0], 3.0);
    apply_canvas_metrics(&mut vp, &canvas).unwrap();
    assert_eq!(vp.logical_size, [400.0, 300.0]);
    assert_eq!(vp.dpi_scale, 3.0);
    assert_eq!(vp.camera, before);
}

#[test]
fn space_switch_payload_maps_model_and_paper_to_the_command_payload() {
    use cad_domain::{LayoutId, SpaceId};
    // `CommandPayload` has no `PartialEq`; match the variant instead.
    assert!(matches!(
        space_switch_payload(cad_representation::SpaceSelection::Model),
        crate::CommandPayload::Space(SpaceId::Model)
    ));
    assert!(matches!(
        space_switch_payload(cad_representation::SpaceSelection::Paper(LayoutId(7))),
        crate::CommandPayload::Space(SpaceId::Paper(LayoutId(7)))
    ));
}
