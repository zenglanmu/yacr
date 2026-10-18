use super::native::{
    checked_physical_size, init_allowed, panic_payload_message, pending_prerequisite,
    pending_reason_changed, render_error_class, wayland_buffer_scale,
};
use super::BridgeState;

#[test]
fn desired_physical_size_is_allocation_times_scale() {
    // Pure check of the size rule used for the raw surface (no GTK needed).
    let alloc = (960i32, 600i32);
    for scale in [1i32, 2i32] {
        assert_eq!(
            checked_physical_size(alloc.0, alloc.1, scale),
            Ok((960 * scale as u32, 600 * scale as u32))
        );
    }
}

#[test]
fn checked_physical_size_rejects_non_positive_and_bad_scale() {
    assert!(checked_physical_size(0, 600, 1).is_err());
    assert!(checked_physical_size(960, -1, 1).is_err());
    assert!(checked_physical_size(960, 600, 0).is_err());
    assert!(checked_physical_size(960, 600, -2).is_err());
}

#[test]
fn checked_physical_size_overflow_is_explicit() {
    // A wrapping multiply would have produced a bogus but "valid-looking"
    // size; it must be an explicit error instead.
    let overflow = checked_physical_size(i32::MAX, 1, 2);
    assert!(overflow.is_err());
    assert!(overflow.unwrap_err().contains("overflows i32"));
}

#[test]
fn panic_payload_is_preserved_and_bounded() {
    let short: Box<dyn std::any::Any + Send> = Box::new("boom");
    assert_eq!(panic_payload_message(short.as_ref()), "boom");
    let other: Box<dyn std::any::Any + Send> = Box::new(42u32);
    assert_eq!(
        panic_payload_message(other.as_ref()),
        "non-string panic payload"
    );
    let long: Box<dyn std::any::Any + Send> = Box::new("x".repeat(500));
    let bounded = panic_payload_message(long.as_ref());
    assert!(bounded.chars().count() <= 201);
    assert!(bounded.ends_with('…'));
}

#[test]
fn init_gate_allows_only_awaiting_or_suspended() {
    assert!(init_allowed(&BridgeState::AwaitingMap));
    assert!(init_allowed(&BridgeState::Suspended));
    assert!(!init_allowed(&BridgeState::Ready));
    assert!(!init_allowed(&BridgeState::Closing));
    assert!(!init_allowed(&BridgeState::Failed("x".into())));
}

#[test]
fn closing_never_inits_and_failed_never_retries() {
    // Terminal states are never allowed to (re)init a surface.
    for state in [
        BridgeState::Closing,
        BridgeState::Failed("terminal".into()),
        BridgeState::Ready,
    ] {
        assert!(!init_allowed(&state), "{state:?} must not init");
    }
}

#[test]
fn unmapped_prerequisites_defer_instead_of_failing() {
    // A not-yet-mapped widget is a transient prerequisite: the caller defers.
    assert_eq!(
        pending_prerequisite(false, false, false),
        Some("drawing area does not own a window yet")
    );
    assert_eq!(
        pending_prerequisite(true, false, false),
        Some("drawing area is not realized")
    );
    assert_eq!(
        pending_prerequisite(true, true, false),
        Some("drawing area is not mapped")
    );
    // Only when realized + mapped with its own window may native
    // classification proceed.
    assert_eq!(pending_prerequisite(true, true, true), None);
}

#[test]
fn non_device_render_errors_are_not_device_loss() {
    use cad_render_wgpu::RenderError;
    // Only a device loss fails the session device; camera/size/upload/scene
    // errors keep the device attached and report their own class.
    assert!(RenderError::DeviceLost("reset".into()).is_device_loss());
    assert!(!RenderError::Frame("camera".into()).is_device_loss());
    assert!(!RenderError::NotInitialized("no device".into()).is_device_loss());
    assert_eq!(render_error_class(&RenderError::Frame("x".into())), "frame");
    assert_eq!(
        render_error_class(&RenderError::DeviceLost("x".into())),
        "device_lost"
    );
}

#[test]
fn zero_allocation_is_rejected_by_the_size_helper() {
    // A zero/negative allocation must never produce a surface size; the helper
    // rejects it explicitly (and `desired_size` reports 0x0 so the host skips).
    assert!(checked_physical_size(0, 0, 1).is_err());
    assert!(checked_physical_size(0, 600, 1).is_err());
    assert!(checked_physical_size(960, 0, 2).is_err());
}

#[test]
fn deferral_is_logged_once_per_reason_change() {
    // A persistently pending Wayland global/selection is logged once, and a
    // changed reason logs again.
    assert!(pending_reason_changed(None, "waiting"));
    assert!(!pending_reason_changed(Some("waiting"), "waiting"));
    assert!(pending_reason_changed(Some("waiting"), "other"));
}

#[test]
fn wayland_buffer_scale_is_at_least_one() {
    // GTK3 only reports integer scale; a degenerate 0/negative factor must
    // never reach the child surface (`wl_surface.set_buffer_scale` requires
    // >= 1).
    assert_eq!(wayland_buffer_scale(2), 2);
    assert_eq!(wayland_buffer_scale(1), 1);
    assert_eq!(wayland_buffer_scale(0), 1);
    assert_eq!(wayland_buffer_scale(-3), 1);
}

#[test]
fn wayland_physical_size_is_allocation_times_scale() {
    // The app-owned child surface's physical size is the CAD allocation times
    // the integer scale (the same rule as the X11 native child), so the
    // render extent stays bounded to the CAD rect.
    let alloc = (640i32, 480i32);
    for scale in [1i32, 2i32] {
        let factor = wayland_buffer_scale(scale) as u32;
        assert_eq!(
            ((alloc.0 as u32) * factor, (alloc.1 as u32) * factor),
            (640 * scale as u32, 480 * scale as u32)
        );
    }
}
