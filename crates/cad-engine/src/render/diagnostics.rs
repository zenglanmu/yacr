use super::*;

/// Why the render gate did (not) produce a CAD frame. Diagnostics only — it is
/// derived from the same read-only facts `RenderSessionHandle::render` uses and
/// never relaxes the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderGateReason {
    /// No renderer attached (device detached / never attached).
    Detached,
    /// Terminal backend failure/loss.
    Failed,
    /// CPU scene preparation has not published the current scene yet (the
    /// `prepared_identity`/`content_revision` path is still busy or has not run).
    Preparing,
    /// The current scene is published but its **base** upload has not committed.
    UploadingBase,
    /// The current scene is published but an optional overlay-suffix upload has
    /// not committed.
    UploadingOverlay,
    /// The current scene is published and no upload is pending, but the GPU base
    /// resident for its revision does not match the live base identity (for
    /// example after a device rebuild) — the gate cannot draw the current scene.
    BaseIdentityMismatch,
    /// Gate met: the current scene and its base are GPU-committed.
    Ready,
}

/// The facts the gate decision is made from (pure, so it is unit-testable
/// without a GPU).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateFacts {
    detached: bool,
    ready_lifecycle: bool,
    scene_current: bool,
    committed: bool,
    /// `Some(base_changed)` when an upload is in flight.
    pending: Option<bool>,
    base_resident: bool,
}

fn classify_gate(facts: GateFacts) -> RenderGateReason {
    if facts.detached {
        return RenderGateReason::Detached;
    }
    if !facts.ready_lifecycle {
        return RenderGateReason::Failed;
    }
    if !facts.scene_current {
        return RenderGateReason::Preparing;
    }
    if facts.committed {
        return RenderGateReason::Ready;
    }
    match facts.pending {
        Some(true) => RenderGateReason::UploadingBase,
        Some(false) => RenderGateReason::UploadingOverlay,
        None if facts.base_resident => RenderGateReason::Preparing,
        None => RenderGateReason::BaseIdentityMismatch,
    }
}

/// Read-only structured snapshot of the render session's gate/waiting state.
///
/// Projected from the same state `render` reads; producing it never advances a
/// revision, an upload or the view, and never forces engine work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDiagnostics {
    /// The gate reason the next `render` would see.
    pub gate: RenderGateReason,
    /// `"detached"` / `"ready"` / `"lost"` / `"failed"`.
    pub lifecycle: &'static str,
    /// Monotonic scene-identity revision (advances on identity changes only).
    pub input_revision: u64,
    /// Monotonic preparation token: the **most recently requested** input.
    pub requested_token: u64,
    /// Monotonic authoritative view revision.
    pub view_revision: u64,
    /// Content revision of the last **published** scene.
    pub prepared_token: Option<u64>,
    /// Scene-identity revision the published scene was built from.
    pub prepared_identity: Option<u64>,
    /// Published scene revision, if a current ready scene exists.
    pub ready_revision: Option<u64>,
    /// Published scene's base revision.
    pub ready_base_revision: Option<u64>,
    /// Cumulative scene publication revision.
    pub scene_publication_revision: u64,
    /// Cumulative base publication revision.
    pub base_publication_revision: u64,
    /// Whether async CPU preparation is busy.
    pub preparation_busy: bool,
    /// Whether async CPU preparation exists on this target (false on wasm).
    pub preparation_available: bool,
    /// Last real scene/runtime diagnostic (never a converted success).
    pub result_status: Option<String>,
    /// Whether a camera-only (pan/zoom/rotate) reference-grid refresh is deferred
    /// until the navigation settles. `true` only while the current bounds have not
    /// been re-prepared yet, so it can never claim a fake final commit.
    pub grid_refresh_pending: bool,
    /// Human-readable companion for [`Self::grid_refresh_pending`].
    pub grid_refresh: &'static str,
    /// Runtime upload/render bookkeeping.
    pub runtime: RuntimeDiagnostics,
}

impl RenderSessionHandle {
    /// Read-only render-gate diagnostics (see [`RenderDiagnostics`]).
    ///
    /// Deliberately does **not** call `observe_drawing` (which can advance the
    /// input revision): a telemetry read must not mutate readiness.
    pub fn diagnostics(&self) -> RenderDiagnostics {
        let state = self.0.borrow();
        let scene = current_ready_scene(&state);
        let committed = scene.is_some_and(|scene| {
            state
                .runtime
                .scene_committed(scene.revision, scene.base_revision)
        });
        let facts = GateFacts {
            detached: matches!(state.runtime.lifecycle, RenderLifecycle::Detached),
            ready_lifecycle: matches!(state.runtime.lifecycle, RenderLifecycle::Ready { .. }),
            scene_current: scene.is_some(),
            committed,
            pending: state.runtime.pending_upload_kind(),
            base_resident: scene
                .is_some_and(|scene| state.runtime.base_fully_resident(scene.base_revision)),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let preparation_busy = state.preparation.busy();
        #[cfg(target_arch = "wasm32")]
        let preparation_busy = false;
        let runtime = state.runtime.diagnostics();
        RenderDiagnostics {
            gate: classify_gate(facts),
            lifecycle: runtime.lifecycle,
            input_revision: state.input_revision,
            requested_token: state.content_revision,
            view_revision: state.view_revision,
            prepared_token: state.prepared_token,
            prepared_identity: state.prepared_identity,
            ready_revision: scene.map(|scene| scene.revision),
            ready_base_revision: scene.map(|scene| scene.base_revision),
            scene_publication_revision: state.scene_publication_revision,
            base_publication_revision: state.base_publication_revision,
            preparation_busy,
            preparation_available: !cfg!(target_arch = "wasm32"),
            result_status: state
                .controller
                .diagnostic
                .clone()
                .or_else(|| state.runtime.diagnostic.clone()),
            grid_refresh_pending: state.grid_refresh_pending,
            grid_refresh: grid_refresh_label(state.grid_refresh_pending),
            runtime,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        detached: bool,
        ready_lifecycle: bool,
        scene_current: bool,
        committed: bool,
        pending: Option<bool>,
        base_resident: bool,
    ) -> GateFacts {
        GateFacts {
            detached,
            ready_lifecycle,
            scene_current,
            committed,
            pending,
            base_resident,
        }
    }

    #[test]
    fn render_gate_reason_covers_every_waiting_stage() {
        // Detached: no renderer at all.
        assert_eq!(
            classify_gate(facts(true, false, false, false, None, false)),
            RenderGateReason::Detached
        );
        // Attached-but-terminal.
        assert_eq!(
            classify_gate(facts(false, false, false, false, None, false)),
            RenderGateReason::Failed
        );
        // CPU scene preparation has not published the current scene.
        assert_eq!(
            classify_gate(facts(false, true, false, false, None, false)),
            RenderGateReason::Preparing
        );
        // Published scene, base upload still in flight.
        assert_eq!(
            classify_gate(facts(false, true, true, false, Some(true), false)),
            RenderGateReason::UploadingBase
        );
        // Published scene, overlay-suffix upload in flight.
        assert_eq!(
            classify_gate(facts(false, true, true, false, Some(false), true)),
            RenderGateReason::UploadingOverlay
        );
        // Published scene, nothing pending, base not resident (identity mismatch /
        // device reset) — never reported Ready.
        assert_eq!(
            classify_gate(facts(false, true, true, false, None, false)),
            RenderGateReason::BaseIdentityMismatch
        );
        // Gate met.
        assert_eq!(
            classify_gate(facts(false, true, true, true, None, true)),
            RenderGateReason::Ready
        );
        // A committed scene is Ready regardless of a stale pending flag.
        assert_eq!(
            classify_gate(facts(false, true, true, true, Some(true), true)),
            RenderGateReason::Ready
        );
    }
}
