//! `apply_ready_updates` and its released-payload rebuild funnel.
//!
//! Split out of `session_impl.rs` so both files stay inside the file-size gate.
//! The method owns the one retry that turns a released CPU payload into a forced
//! full-base rebuild (ADR-0057); the rest of `RenderSessionHandle` stays in its
//! original module.

use super::*;

/// Whether a runtime refusal means the committed CPU base payload was released.
fn is_missing_cpu_payload(error: &CadError) -> bool {
    matches!(error, CadError::Invariant(message) if message == super::runtime::MISSING_CPU_PAYLOAD)
}

impl RenderSessionHandle {
    pub fn apply_ready_updates(&self) -> CadResult<()> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        if let Some(ready) = current_ready_scene(&state).cloned() {
            match state.runtime.apply_ready_updates(Some(&ready)) {
                Ok(()) => {
                    // The committed base now belongs to the GPU. Drop its CPU
                    // geometry; identity and append admission are unchanged and
                    // the release is gated on no in-flight upload.
                    state.runtime.release_committed_base_payload();
                    return Ok(());
                }
                Err(error) if is_missing_cpu_payload(&error) => {
                    // The publication needs a full base stage but the committed
                    // CPU payload was released. Drop the partial upload, rebuild
                    // from the live inputs (a genuine full base, new revision)
                    // and retry exactly once.
                    state.runtime.abort_pending_upload();
                    super::prepare_full_base_synchronously(&mut state)?;
                    let ready = current_ready_scene(&state).cloned().ok_or_else(|| {
                        CadError::Invariant("render.base_payload_rebuild_missing_scene".to_string())
                    })?;
                    state.runtime.apply_ready_updates(Some(&ready))?;
                    state.runtime.release_committed_base_payload();
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        // Gate-stale scene (newer overlay inputs are still preparing): its
        // geometry must not be uploaded, so the superseded in-flight upload is
        // dropped exactly as before. Its `entity_flags`, however, are the
        // synchronously re-resolved current set, so that delta is still pushed —
        // the hover/selection tint then never reverts to an older set and never
        // waits for the next publication while the cursor moves.
        state.runtime.apply_ready_updates(None)?;
        let catch_up = state
            .controller
            .ready
            .as_ref()
            .filter(|ready| state.runtime.base_fully_resident(ready.base_revision))
            .map(|ready| {
                (
                    ready.base_revision,
                    ready.entity_flags.clone(),
                    ready.flags_revision,
                )
            });
        match catch_up {
            Some((base_revision, flags, flags_revision)) => {
                state
                    .runtime
                    .apply_flags_catch_up(base_revision, &flags, flags_revision)
            }
            None => Ok(()),
        }
    }
}
