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
