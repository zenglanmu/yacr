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

/// `(logical, scale, DOM rect, physical surface size)` — the tick's render
/// geometry, resolved from one rect so the four values cannot drift apart.
type CadRenderGeometry = ([f64; 2], f64, Option<(f32, f32, f32, f32)>, (u32, u32));

/// The tick's render geometry from the latest DOM rect and window scale.
///
/// The logical size, the DOM rect for the drawing-area guard and the **physical
/// surface size** all come from the same rect; the physical size is
/// `cad_ui_core::layout::physical_content_size`, the render session's own
/// formula. Sizing the surface from the GTK allocation instead is a second
/// rounding path that differs by one physical pixel at a fractional rect
/// (5K/scale=2), and the presenter then refuses every frame. The scale is
/// clamped once here, the same value the render path uses. A missing rect reports
/// zero sizes so the host waits.
pub(super) fn cad_render_geometry(
    rect: Option<cad_ui_core::layout::CadMetrics>,
    scale: f64,
) -> CadRenderGeometry {
    let scale = scale.max(0.1);
    match rect {
        Some(rect) => (
            [f64::from(rect.width), f64::from(rect.height)],
            scale,
            Some((rect.x, rect.y, rect.width, rect.height)),
            cad_ui_core::layout::physical_content_size(&rect, scale),
        ),
        None => ([0.0, 0.0], scale, None, (0, 0)),
    }
}
