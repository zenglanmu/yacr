//! The persisted 3D orbit sensitivity (`interaction.orbitSensitivityPercent`).
//!
//! Split out of `settings.rs` for the code-size gate. The closed key table, the
//! `HostSettings` field and the one validated store/persistence path stay there;
//! this module owns only the two `HostCore` operations for this leaf — the reader
//! the orbit gesture calls per event and the write that persists it. There is no
//! session state to seed: the gesture reads the effective config, so the durable
//! write is the live application.
use super::*;

impl HostCore {
    /// The 3D orbit sensitivity as a percentage of the host's base rate
    /// (`interaction.orbitSensitivityPercent`, default `100`).
    ///
    /// Read from the effective config per call — the same source the orbit
    /// gesture uses — so a settings write takes effect on the next drag without
    /// reopening the document. A missing preference is the documented `100`
    /// default, and a value outside the schema's `25.0..=200.0` range (only
    /// reachable if the durable document was edited by hand past validation)
    /// falls back to `100` rather than being reported as an effective
    /// sensitivity the orbit path would not use.
    pub(crate) fn orbit_sensitivity_percent(&self) -> f64 {
        let percent = self
            .viewer_config
            .borrow()
            .effective()
            .interaction
            .orbit_sensitivity_percent;
        if percent.is_finite() && (25.0..=200.0).contains(&percent) {
            percent
        } else {
            DEFAULT_ORBIT_SENSITIVITY_PERCENT
        }
    }

    /// Persist `interaction.orbitSensitivityPercent` (a percentage of the base
    /// 3D orbit rate) and make it live immediately.
    ///
    /// This leaf has no session state to drive (the same shape as the 2D
    /// `leftDragPans` flag): the orbit gesture reads the effective config per
    /// pointer event, so the durable write *is* the live application and the next
    /// drag honours it without reopening the document. The key table has already
    /// refused a non-finite or out-of-`25..=200` value, so the number written here
    /// is exactly what the schema validates back.
    pub(crate) fn apply_orbit_sensitivity_percent(&self, value: &CadSettingValue) -> CadResult<()> {
        let percent = match value {
            CadSettingValue::Real(percent) => *percent,
            CadSettingValue::Number(percent) => f64::from(*percent),
            _ => return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into())),
        };
        if !percent.is_finite() || !(25.0..=200.0).contains(&percent) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        self.apply_user_preference_json(&format!(
            r#"{{"interaction":{{"orbitSensitivityPercent":{percent}}}}}"#
        ))
    }
}
