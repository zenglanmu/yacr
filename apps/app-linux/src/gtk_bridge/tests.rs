use super::native::{
    drawing_area_matches_dom, gpu_memory_refusal_line, init_allowed, panic_payload_message,
    pending_prerequisite, pending_reason_changed, render_error_class, wayland_aligned_buffer_size,
    wayland_buffer_scale, webview_target_size, WebviewSkip, GPU_MEMORY_CLASS,
};
use super::BridgeState;

#[test]
fn webview_skip_reasons_are_stable_log_labels() {
    // `skipped=` is the field a run is diagnosed by; keep the taxonomy fixed so a
    // regression report ("skipped=same-allocation" vs "unmapped-parent") stays
    // readable across versions. Exactly one reason means "applied".
    assert_eq!(WebviewSkip::None.label(), "none");
    assert_eq!(WebviewSkip::SameAllocation.label(), "same-allocation");
    assert_eq!(WebviewSkip::NotRegistered.label(), "not-registered");
    assert_eq!(WebviewSkip::NoParent.label(), "no-parent");
    assert_eq!(WebviewSkip::UnmappedParent.label(), "unmapped-parent");
    assert_eq!(WebviewSkip::Nested.label(), "nested");
}

#[test]
fn webview_fills_its_container_and_is_never_zero() {
    // The overlay (the container the React DOM fills) is authoritative: this is
    // the size the webview widget must occupy, not the Tauri window inner size.
    assert_eq!(
        webview_target_size(Some((926, 553)), (960, 600)),
        (926, 553)
    );
    // `1x1` is GTK's unsized requisition, not a viewport: it falls back to the
    // window inner size instead of leaving the webview at its creation size.
    assert_eq!(webview_target_size(Some((1, 1)), (960, 600)), (960, 600));
    assert_eq!(webview_target_size(Some((0, 0)), (960, 600)), (960, 600));
    assert_eq!(webview_target_size(None, (960, 600)), (960, 600));
    // A partial (zero-height) allocation is not settled either.
    assert_eq!(webview_target_size(Some((926, 0)), (960, 600)), (960, 600));
    // A real, small allocation is still honoured (only the degenerate 1x1 is not).
    assert_eq!(
        webview_target_size(Some((320, 240)), (960, 600)),
        (320, 240)
    );
    // No container and no usable fallback: an explicit 1x1 "not sized yet",
    // never a 0x0 allocation (and the size log reports it as a number).
    assert_eq!(webview_target_size(None, (0, 0)), (1, 1));
    assert_eq!(webview_target_size(None, (1, 1)), (1, 1));
    assert_eq!(webview_target_size(Some((-3, -4)), (0, 0)), (1, 1));
}

#[test]
fn drawing_area_matches_dom_within_one_logical_pixel() {
    let dom = (300.0, 175.0, 626.0, 341.0);
    // Exact match: the drawing area's toplevel origin is the container origin
    // plus the DOM rect origin, and its size is the DOM rect size.
    assert!(drawing_area_matches_dom(
        Some((0, 0)),
        Some((300, 175)),
        (626, 341),
        dom
    ));
    // Sub-pixel rounding passes.
    assert!(drawing_area_matches_dom(
        Some((0, 0)),
        Some((300, 175)),
        (626, 341),
        (300.4, 174.6, 625.6, 341.4)
    ));
    // A non-zero container origin is accounted for: 101 + 175 = 276.
    assert!(drawing_area_matches_dom(
        Some((0, 101)),
        Some((300, 276)),
        (626, 341),
        dom
    ));
    // The observed defect: the install-time geometry (margins 101/60) against
    // the real DOM CAD rect — a size *and* origin disagreement.
    assert!(!drawing_area_matches_dom(
        Some((0, 0)),
        Some((0, 101)),
        (960, 439),
        dom
    ));
    // An origin-only disagreement fails (the pointer would be offset).
    assert!(!drawing_area_matches_dom(
        Some((0, 0)),
        Some((0, 0)),
        (626, 341),
        dom
    ));
    // A two-pixel size disagreement is beyond tolerance.
    assert!(!drawing_area_matches_dom(
        Some((0, 0)),
        Some((300, 175)),
        (628, 341),
        dom
    ));
    // Unrealized/not-allocated widgets are never a silent match.
    assert!(!drawing_area_matches_dom(
        None,
        Some((300, 175)),
        (626, 341),
        dom
    ));
    assert!(!drawing_area_matches_dom(
        Some((0, 0)),
        None,
        (626, 341),
        dom
    ));
}

#[test]
fn the_surface_size_is_the_dom_rect_times_scale_not_the_gtk_allocation() {
    // Regression fence for the reported 5K/scale=2 flatpak failure
    // (`native texture 3851x1645 does not match the 3852x1644 surface`). A
    // fractional CAD rect inside an integral GTK allocation makes the two
    // rounding paths disagree by one physical pixel:
    //
    //   allocation × scale = (1926×2, 822×2) = 3852x1644
    //   round(rect × scale)                  = 3851x1645
    //
    // The render session uses the second value, so the child surface must too;
    // deriving it from the allocation made the presenter refuse every frame
    // (`TextureInvalid`), leaving the CAD region transparent.
    use cad_ui_core::layout::{insets_from, physical_content_size, CadMetrics};
    let rect = CadMetrics {
        x: 0.4,
        y: 0.6,
        width: 1925.6,
        height: 822.5,
        dpr: 2.0,
        sequence: 1,
    };
    let insets = insets_from(&rect, 1926.0, 823.0);
    let allocation = (
        1926 - insets.start - insets.end,
        823 - insets.top - insets.bottom,
    );
    assert_eq!(allocation, (1926, 822));
    assert_eq!(
        (allocation.0 as u32 * 2, allocation.1 as u32 * 2),
        (3852, 1644)
    );
    assert_eq!(physical_content_size(&rect, 2.0), (3851, 1645));
    // The host stores exactly this value (`apply_layout`) and the render session
    // allocates its target with the same formula, so texture == surface by
    // construction instead of "equal for integral sizes".
    assert_eq!(
        physical_content_size(&rect, 2.0),
        cad_engine::render::render_target_size(
            [f64::from(rect.width), f64::from(rect.height)],
            2.0
        )
    );
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
    assert_eq!(
        render_error_class(&RenderError::NotInitialized("x".into())),
        "not_initialized"
    );
    // A GPU-memory refusal has its own stable class: it is neither a generic
    // frame error nor a device loss (the resident scene is intact). The class is
    // pinned here because the host appends it to the status error verbatim; the
    // `RenderError::GpuMemory` value itself cannot be constructed outside
    // `cad-render-wgpu` (its message has no public constructor), so the match arm
    // above is what keeps the variant exhaustive at compile time.
    assert_eq!(GPU_MEMORY_CLASS, "gpu_memory");
    assert_ne!(GPU_MEMORY_CLASS, "frame");
    assert_ne!(GPU_MEMORY_CLASS, "device_lost");
}

#[test]
fn a_gpu_memory_refusal_line_is_stable_and_spells_absent_fields_none() {
    // Same vocabulary as the headless CLI's `gpu-memory-refused:` contract, so an
    // evidence run and a CLI run can be compared field by field.
    assert_eq!(
        gpu_memory_refusal_line(
            false,
            "stream",
            1_900_000_000,
            144,
            Some(3_000_000_000),
            Some(3_000_000_000)
        ),
        "[probe] gpu-memory-refused: origin=application-budget category=stream \
         used=1900000000 requested=144 cap=3000000000 limit=3000000000"
    );
    // A device out-of-memory refusal is a different origin; an uncapped policy has
    // no cap/limit and must say `none`, never `0`.
    assert_eq!(
        gpu_memory_refusal_line(true, "mask", 10, 20, None, None),
        "[probe] gpu-memory-refused: origin=device-out-of-memory category=mask used=10 \
         requested=20 cap=none limit=none"
    );
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
fn wayland_buffer_size_is_grown_to_the_buffer_scale_multiple() {
    // The exact protocol error from the 5K/scale=2 run:
    //   wl_surface#49 error 2: "Buffer size (3963x1645) must be an integer
    //   multiple of the buffer_scale (2)."
    assert_eq!(wayland_aligned_buffer_size((3963, 1645), 2), (3964, 1646));
    assert_eq!(wayland_aligned_buffer_size((3851, 1645), 2), (3852, 1646));
    // Already legal: unchanged (never shrink, never move a multiple).
    assert_eq!(wayland_aligned_buffer_size((3964, 1646), 2), (3964, 1646));
    assert_eq!(wayland_aligned_buffer_size((1926, 823), 1), (1926, 823));
    assert_eq!(wayland_aligned_buffer_size((1012, 690), 3), (1014, 690));
    // Degenerate inputs stay explicit: a zero axis means "wait", and a scale
    // below one is one.
    assert_eq!(wayland_aligned_buffer_size((0, 900), 2), (0, 900));
    assert_eq!(wayland_aligned_buffer_size((3963, 1645), 0), (3963, 1645));
    assert_eq!(wayland_aligned_buffer_size((3963, 1645), -3), (3963, 1645));
    // Saturating: never wraps at the top of the range.
    assert_eq!(
        wayland_aligned_buffer_size((u32::MAX, 1), 2),
        (u32::MAX - 1, 2)
    );
}
