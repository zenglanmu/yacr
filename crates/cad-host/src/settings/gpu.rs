//! The accelerated-GPU-storage preference (`view.gpuStorage.disabled`).
//!
//! Split out of `settings.rs` so both files stay inside the file-size gate. The
//! closed key table and the write dispatch live in the parent module; this file
//! owns the one setting that is a renderer policy (recorded on the render session
//! and applied at the next device attach).

use super::*;

/// Fetch the persisted `view.gpuBudget.percent` leaf, if the document has it.
fn gpu_budget_percent(document: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
    document
        .get_mut("view")?
        .get_mut("gpuBudget")?
        .get_mut("percent")
}

/// Migrate a persisted `view.gpuBudget.percent` down to the current maximum.
///
/// This is **startup-only**: the durable bytes may have been written by a
/// release whose accepted range was wider (`10..=100`). A value that was valid
/// then but is above today's maximum (`81..=100`) is rewritten to the maximum
/// and reported to the caller so the host can name it in a diagnostic. Anything
/// else is left untouched: `101+` was never valid, so it keeps failing normal
/// validation instead of being silently repaired. A live `cad_set_setting`
/// write is never clamped — only this read path migrates.
pub(crate) fn migrate_persisted_gpu_budget_percent(text: &str) -> (String, Option<u32>) {
    let maximum = *cad_app::viewer_config::ViewGpuBudget::RANGE.end();
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return (text.to_string(), None);
    };
    let previous = match gpu_budget_percent(&mut document) {
        Some(value) => match value.as_u64().and_then(|value| u32::try_from(value).ok()) {
            Some(previous) => previous,
            None => return (text.to_string(), None),
        },
        None => return (text.to_string(), None),
    };
    if previous <= maximum || previous > 100 {
        return (text.to_string(), None);
    }
    if let Some(value) = gpu_budget_percent(&mut document) {
        *value = serde_json::Value::from(maximum);
    }
    match serde_json::to_string(&document) {
        Ok(rewritten) => (rewritten, Some(previous)),
        Err(_) => (text.to_string(), None),
    }
}

impl HostCore {
    /// Record the effective accelerated-GPU-storage policy on the render
    /// session.
    ///
    /// Startup-only bookkeeping: the session stores the choice and applies it
    /// at every device attach (the draw tier is chosen once per device). A live
    /// `cad_set_setting` write calls the same setter directly.
    pub(crate) fn seed_gpu_storage(&self) {
        let disabled = self
            .viewer_config
            .borrow()
            .effective()
            .view
            .gpu_storage
            .disabled;
        self.render.set_gpu_storage_enabled(!disabled);
    }

    /// Persist `view.gpuStorage.disabled`.
    ///
    /// The value is already validated as a boolean, so the JSON written here is
    /// exactly what the schema parses back. The render session records the
    /// policy **now** and applies it whenever it next attaches a device: the
    /// draw tier is chosen once per device, so a live change must not pretend a
    /// current frame switched paths (the help text says so). Keeping the policy
    /// on the engine handle is what makes it survive every re-attach, exactly
    /// like the MSAA request in the parent module.
    pub(super) fn apply_gpu_storage_disabled(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Bool(disabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"gpuStorage":{{"disabled":{disabled}}}}}}}"#
        ))?;
        self.render.set_gpu_storage_enabled(!disabled);
        Ok(())
    }
}
