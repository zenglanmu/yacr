//! Explicit host admission gates: the CAD bind step, the bridge-state present
//! admission and the CAD-rect revalidation decision (all pure).

/// Outcome of one bind step for the CAD render session.
pub(super) enum BindStep {
    /// No bridge is installed yet (deferred, not a failure).
    NoBridge,
    /// Skipped: the bridge is closing/failed, or this generation already failed.
    Skipped,
    /// A bind was attempted for `generation` with `result`.
    Attempted(u64, Result<(), String>),
}

/// Admission for one CAD presentation attempt, derived from the **bridge
/// lifecycle state** (not from `cad_ready`, which only means the render session
/// was registered).
///
/// This is the CAD present gate's explicit table: a bridge that is still coming
/// up (or suspended for re-creation) must make the host **wait** — never a
/// spurious `Present` error — while a terminal bridge is never presented on and
/// its diagnostics are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CadPresentAdmission {
    /// `AwaitingMap`/`Suspended`: no presenter by design; wait (`Ok(None)`), do
    /// not touch/clear any prior error.
    Waiting,
    /// `Ready`: attempt the existing resize / native-health / render path.
    Ready,
    /// `Closing`/`Failed`: never present; preserve the terminal diagnostics.
    Terminal,
}

/// Map a bridge lifecycle state to the CAD presentation admission (pure).
pub(super) fn cad_present_admission(state: &crate::gtk_bridge::BridgeState) -> CadPresentAdmission {
    use crate::gtk_bridge::BridgeState;
    match state {
        BridgeState::AwaitingMap | BridgeState::Suspended => CadPresentAdmission::Waiting,
        BridgeState::Ready => CadPresentAdmission::Ready,
        BridgeState::Closing | BridgeState::Failed(_) => CadPresentAdmission::Terminal,
    }
}

/// Whether the cached CAD rect must be **re-validated** (instead of a strict
/// `accept`): a new GPU generation, or a change in the margin-receiving container
/// bounds — the latter even when the DOM sequence did not change (the container
/// resized under an unchanged rect). Pure, so the decision is unit-testable.
pub(super) fn layout_needs_revalidate(
    generation_revalidate: bool,
    last_bounds: Option<(f32, f32)>,
    bounds: (f32, f32),
) -> bool {
    generation_revalidate || last_bounds != Some(bounds)
}

/// `(logical, dpr, GTK-logical rect, physical surface size)` — the tick's render
/// geometry, resolved from one rect so the four values cannot drift apart. The
/// rect is in GTK logical px (CSS × `css_to_gtk`), the space GTK allocates in.
type CadRenderGeometry = ([f64; 2], f64, Option<(f32, f32, f32, f32)>, (u32, u32));

/// The CSS→GTK-logical ratio for the current WebView/GTK pair.
///
/// A frontend rect is in CSS px. GTK widget geometry and the Wayland parent
/// surface are in GTK logical px; the parent `wl_surface` uses
/// `buffer_scale = parent_scale` (GDK's integer scale). The WebView's
/// `devicePixelRatio` is CSS→**device** and WebKitGTK folds GTK text scaling
/// (page zoom) into it, so CSS→GTK-logical is `devicePixelRatio / parent_scale`.
/// On the reported 5K/`scale=2` host the WebView reports 2.5 while GDK reports
/// 2 → 1.25; using the GDK scale for the CSS rect made the native surface
/// 1/1.25 of the DOM hole (top-left aligned, desktop in the bottom-right).
pub(super) fn css_to_gtk(dpr: f64, parent_scale: i32) -> f64 {
    let parent = f64::from(parent_scale.max(1));
    if dpr.is_finite() && dpr > 0.0 {
        dpr / parent
    } else {
        1.0
    }
}

/// The frontend CSS rect converted to GTK logical px and grown to **cover** the
/// fractional DOM hole: the leading edge is floored and the trailing edge is
/// ceiled. Rounding the origin to nearest (218.5 → 219) left a sub-pixel strip
/// between the native child and the DOM chrome (the desktop showed through at
/// the top). Covering instead grows the child by less than one GTK logical pixel
/// per axis (hidden under the opaque chrome) and its integer size is the same
/// rect the drawing-area guard compares against.
///
/// Trade-off: the CAD content is at most one GTK logical pixel (and therefore at
/// most one device pixel per axis) larger than the DOM rect — about 0.1 % at the
/// reported 5K sizes. Device-pixel 1:1 mapping is preserved, so nothing is
/// resampled and the overhang is invisible under the chrome / window edge.
///
/// A 1e-3 logical-pixel epsilon keeps an f32-exact value from gaining a pixel
/// (`1981.6×1.25 = 2477.0000458` must stay 2477) while genuine fractions still
/// floor/ceil.
pub(super) fn gtk_rect(rect: &cad_ui_core::layout::CadMetrics, ratio: f64) -> (f32, f32, f32, f32) {
    let epsilon = 1e-3;
    let left = (f64::from(rect.x) * ratio + epsilon).floor();
    let top = (f64::from(rect.y) * ratio + epsilon).floor();
    let right = ((f64::from(rect.x) + f64::from(rect.width)) * ratio - epsilon).ceil();
    let bottom = ((f64::from(rect.y) + f64::from(rect.height)) * ratio - epsilon).ceil();
    (
        left as f32,
        top as f32,
        (right - left) as f32,
        (bottom - top) as f32,
    )
}

/// The GTK margins that place the drawing area on the converted CAD rect.
///
/// Deriving them from the **already-rounded CSS insets** and scaling afterwards
/// rounds twice: at dpr 2.5 / ratio 1.25 the allocation came out one logical
/// pixel short of the CAD height, the drawing-area guard (1 px tolerance)
/// failed, and no CAD frame was ever presented. The margins and the guard
/// therefore come from the same converted rect: the size is that rect's integer
/// size and the trailing inset absorbs the remainder, so
/// `top + height + bottom == ceil(container)` exactly (and the leading edge is
/// the rect's floor, so the fractional hole is covered).
pub(super) fn gtk_margins(
    container: (f32, f32),
    rect: (f32, f32, f32, f32),
) -> cad_ui_core::layout::ViewportInsets {
    let value = |v: f32| v as i32;
    let start = value(rect.0);
    let top = value(rect.1);
    let width = value(rect.2);
    let height = value(rect.3);
    cad_ui_core::layout::ViewportInsets {
        top,
        start,
        bottom: (container.1.ceil() as i32 - top - height).max(0),
        end: (container.0.ceil() as i32 - start - width).max(0),
    }
}

/// The CAD child-surface geometry: the logical size the render session draws and
/// the physical buffer size the host configures.
///
/// `rect.dpr` is the CSS→**device** authority: it is the WebView's reported
/// `devicePixelRatio`, which includes page zoom (GTK text scaling), not GDK's
/// integer scale. The physical size is the **covered** rect ([`gtk_rect`], in GTK
/// logical px) times the GDK integer `buffer_scale`, so the child fully covers
/// the fractional DOM hole (no seam at the top/left edge) and is at most one
/// device pixel per axis larger than the CSS rect. Deriving it from
/// `physical_content_size` directly would use the smaller CSS rounding and leave
/// that seam. The result is rounded up to an integer multiple of `buffer_scale`
/// (violating that is a fatal `wl_surface` protocol error), and the logical size
/// is derived back from it (`physical / dpr`), so `render_target_size(logical,
/// dpr)` returns exactly the configured physical size: still one authority, now
/// also a legal Wayland buffer that covers the hole.
pub(super) fn cad_surface_geometry(
    rect: &cad_ui_core::layout::CadMetrics,
    buffer_scale: i32,
) -> ([f64; 2], (u32, u32)) {
    let dpr = f64::from(rect.dpr).max(0.1);
    let ratio = css_to_gtk(dpr, buffer_scale);
    let gtk = gtk_rect(rect, ratio);
    let scale = u32::try_from(buffer_scale.max(1)).unwrap_or(1);
    // The covered rect is integer-valued, so `× scale` is exact; the shared
    // helper then rounds each axis up to a multiple of the buffer scale.
    let device = |v: f32| (f64::from(v) * f64::from(scale)).round().max(1.0) as u32;
    let physical = crate::gtk_bridge::wayland_aligned_buffer_size(
        (device(gtk.2), device(gtk.3)),
        buffer_scale,
    );
    (
        [f64::from(physical.0) / dpr, f64::from(physical.1) / dpr],
        physical,
    )
}

/// The accepted rect with its size replaced by the **effective** logical CAD
/// size (the Wayland buffer alignment can grow it by a sub-pixel). The render
/// session's stored metrics must describe the same viewport it renders into, or
/// fit/zoom/picking drift by the alignment amount.
pub(super) fn aligned_metrics(
    rect: cad_ui_core::layout::CadMetrics,
    logical: [f64; 2],
) -> cad_ui_core::layout::CadMetrics {
    cad_ui_core::layout::CadMetrics {
        width: logical[0] as f32,
        height: logical[1] as f32,
        ..rect
    }
}

/// The GTK drawing area's integer buffer scale (the value the Wayland shim sets
/// on the CAD child surface), or 1 before a bridge exists — which leaves the
/// size unchanged. This is also the GDK parent's scale: CSS→GTK-logical is
/// `dpr / buffer_scale` (see [`css_to_gtk`]).
pub(super) fn bridge_buffer_scale() -> i32 {
    super::BRIDGE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|owner| owner.buffer_scale())
            .unwrap_or(1)
    })
}

/// The render session's stored metrics for an aligned CAD geometry: the accepted
/// rect with its size replaced by the effective logical size, so fit/zoom/picking
/// describe the same viewport the renderer draws into. `scale` is the WebView
/// `dpr` (CSS→device), not GDK's integer scale.
pub(super) fn aligned_surface_metrics(
    rect: cad_ui_core::layout::CadMetrics,
    logical: [f64; 2],
    scale: f64,
) -> cad_platform::SurfaceMetrics {
    cad_ui_core::layout::surface_metrics(&aligned_metrics(rect, logical), scale)
}

/// [`cad_render_geometry`] with the bridge's actual buffer scale, so the Wayland
/// alignment can never drift from the scale the shim sets.
pub(super) fn cad_frame_geometry(
    rect: Option<cad_ui_core::layout::CadMetrics>,
) -> CadRenderGeometry {
    cad_render_geometry(rect, bridge_buffer_scale())
}

/// The tick's render geometry from the latest DOM rect.
///
/// The logical size, the GTK-logical rect for the drawing-area guard and the
/// **physical surface size** all come from the same rect. The physical size
/// starts from `cad_ui_core::layout::physical_content_size` at the rect's `dpr`
/// (the render session's own formula) and is grown to a legal Wayland buffer
/// multiple; `logical` is derived back from it so the render target and the
/// child surface are equal by construction. `scale` is that same `dpr`. The
/// returned rect is in **GTK logical px** (see [`css_to_gtk`]) because that is
/// the coordinate space GTK allocates and the drawing-area guard compares
/// against. A missing rect reports zero sizes so the host waits.
pub(super) fn cad_render_geometry(
    rect: Option<cad_ui_core::layout::CadMetrics>,
    parent_scale: i32,
) -> CadRenderGeometry {
    match rect {
        Some(rect) => {
            let (logical, physical) = cad_surface_geometry(&rect, parent_scale);
            let dpr = f64::from(rect.dpr).max(0.1);
            let ratio = css_to_gtk(dpr, parent_scale);
            (logical, dpr, Some(gtk_rect(&rect, ratio)), physical)
        }
        None => ([0.0, 0.0], 1.0, None, (0, 0)),
    }
}
