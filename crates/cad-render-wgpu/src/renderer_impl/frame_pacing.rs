use super::*;

impl Renderer {
    /// Opt-in bounded multipass accumulation. Opaque/transparent global order
    /// is preserved across pages, with color/depth retained between submissions.
    pub fn set_progressive_rendering(&mut self, enabled: bool) {
        self.progressive = enabled;
        self.progressive_frame = None;
    }

    /// Override the adaptive page-budget target (`cpu_ms + wait_ms`, default
    /// [`pacing::DEFAULT_TARGET_MS`]).
    ///
    /// A non-finite or non-positive target falls back to the default rather than
    /// silently producing a nonsense budget. Streaks are cleared because they
    /// were measured against the previous target; the scale and cost EMA (real
    /// scene knowledge) are kept.
    pub fn set_frame_budget_target_ms(&mut self, target_ms: f64) {
        self.pacing.target_ms = pacing::sanitize_target_ms(target_ms);
        self.pacing.over_streak = 0;
        self.pacing.under_streak = 0;
    }

    /// The adaptive page-budget target in milliseconds.
    pub fn frame_budget_target_ms(&self) -> f64 {
        self.pacing.target_ms
    }

    /// Current adaptive scale (`1.0` until the controller has adapted).
    pub fn pacing_scale(&self) -> f64 {
        self.pacing.scale
    }

    /// Current cost EWMA in milliseconds (`0.0` before the first observation).
    pub fn pacing_cost_ema_ms(&self) -> f64 {
        self.pacing.cost_ema_ms
    }

    /// Test hook: force the "a completion signal exists" decision.
    ///
    /// Native has one (the blocking poll measures completion); the wasm
    /// non-blocking poll does not, so pacing is disabled there and the static
    /// budget is used. This override makes the disabled path assertable on a
    /// native test device.
    pub fn set_pacing_signal_override(&mut self, available: Option<bool>) {
        self.pacing_signal_override = available;
    }

    /// The pacing regime for the current configuration.
    pub(super) fn pacing_mode(&self) -> pacing::PacingMode {
        let signal = self
            .pacing_signal_override
            .unwrap_or(!cfg!(target_arch = "wasm32"));
        if !signal {
            pacing::PacingMode::DisabledNoCompletionSignal
        } else if self.progressive {
            pacing::PacingMode::Active
        } else {
            pacing::PacingMode::AtCeiling
        }
    }

    /// The page budget this frame's plan uses.
    ///
    /// Always derived from the un-paced ceiling [`Renderer::frame_budget`], so a
    /// test that injects a tiny ceiling keeps governing every page.
    pub(super) fn active_frame_budget(&self) -> FrameBudget {
        match self.pacing_mode() {
            pacing::PacingMode::Active => {
                pacing::apply_scale(&self.frame_budget, self.pacing.scale)
            }
            pacing::PacingMode::DisabledNoCompletionSignal | pacing::PacingMode::AtCeiling => {
                self.frame_budget
            }
        }
    }

    /// Forget scene-specific cost knowledge. Called on a **base-changing** scene
    /// mutation (`keep_prefix == 0`), `clear_batches` and a device-generation
    /// change / device loss; **never** on camera motion or a suffix-only overlay
    /// commit (both preserve the cost model — see [`Renderer::commit_upload`]).
    pub(super) fn reset_pacing(&mut self) {
        self.pacing = pacing::PacingState::new(self.pacing.target_ms);
        self.pacing_restart_pending = true;
    }

    pub fn frame_pending(&self) -> bool {
        self.progressive_frame
            .as_ref()
            .is_some_and(|frame| frame.cursor < frame.ordered.len())
    }

    /// Submitted visible batches versus all ordered visible batches.
    pub fn frame_progress(&self) -> Option<(usize, usize)> {
        self.progressive_frame
            .as_ref()
            .map(|frame| (frame.cursor, frame.ordered.len()))
    }

    /// Identity of the actual offscreen attachment, not just its dimensions.
    pub fn texture_revision(&self) -> u64 {
        self.texture_revision
    }

    /// Wall-clock milliseconds of the most recent [`Renderer::upload`] call.
    ///
    /// `None` until an upload has run. This is the real upload phase measured by
    /// the renderer, not a derived estimate.
    pub fn last_upload_ms(&self) -> Option<f64> {
        self.last_upload_ms
    }

    /// Microsecond substage timings of the most recent [`Renderer::commit_upload`]
    /// (diagnostics only; `None` until a commit has run).
    pub fn last_commit_timing(&self) -> Option<CommitTiming> {
        self.last_commit_timing
    }
}
