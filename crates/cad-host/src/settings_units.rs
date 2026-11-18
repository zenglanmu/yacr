//! The `ui.units` and `ui.recovery` settings (ADR-0037 D9/D12).
//!
//! Split out of [`crate::settings`] so both files stay inside the file-size
//! gate; the closed key table, its validation and the dispatch stay there. What
//! lives here is the apply half: the schema is the one place tokens become a
//! [`cad_domain::UnitContext`], and the document write is
//! [`cad_app::host::HostController::set_units`] (a header-only edit).

use super::*;

impl HostCore {
    /// Persist `ui.units.drawingUnit` and apply the resulting unit context to
    /// the live document header (ADR-0037 D9).
    ///
    /// Switching the source to the untyped `drawing` token also writes `drawing`
    /// for the display token: the schema's one cross-field rule refuses a typed
    /// display over an untyped source, so that is the only legal pair and the two
    /// leaves are written in **one** patch (a two-write sequence would always be
    /// refused in the middle). Nothing else is widened: the display token is
    /// constrained, never the accepted bounds.
    pub(crate) fn apply_drawing_unit(&self, value: &CadSettingValue) -> CadResult<()> {
        let drawing = unit_token(value)?;
        let current = self.effective_units();
        let display = if drawing == cad_app::viewer_config::UnitToken::Drawing {
            cad_app::viewer_config::UnitToken::Drawing
        } else {
            current.display_unit
        };
        self.write_and_apply_units(drawing, display, current.decimal_places)
    }

    /// Persist `ui.units.displayUnit`.
    ///
    /// A typed display token over the untyped `drawing` source is refused by
    /// [`cad_app::viewer_config::UnitsConfig::unit_context`], naming
    /// `ui.units.displayUnit` — never folded back to `drawing`.
    pub(crate) fn apply_display_unit(&self, value: &CadSettingValue) -> CadResult<()> {
        let display = unit_token(value)?;
        let current = self.effective_units();
        self.write_and_apply_units(current.drawing_unit, display, current.decimal_places)
    }

    /// Persist `ui.units.decimalPlaces` (`0..=8`, already refused above).
    pub(crate) fn apply_unit_precision(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(places) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        let places = u8::try_from(*places)
            .map_err(|_| CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()))?;
        let current = self.effective_units();
        self.write_and_apply_units(current.drawing_unit, current.display_unit, places)
    }

    /// Persist `ui.recovery.intervalSeconds`.
    ///
    /// A non-zero value takes effect on the next owner tick (the writer reads the
    /// effective config per tick, so no timer is rescheduled here). `0` disables
    /// the writes **and** discards the current document's snapshot, so "off"
    /// leaves nothing behind to be offered at the next open; a store that refuses
    /// the discard is reported as a status (the interval really is off), never
    /// silently hidden and never turned into a failed setting.
    pub(crate) fn apply_recovery_interval(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(seconds) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"ui":{{"recovery":{{"intervalSeconds":{seconds}}}}}}}"#
        ))?;
        if *seconds == 0 {
            let key = self.recovery_key();
            if let Some(reason) = self.discard_recovery(key.as_ref()) {
                self.record_status("recovery.discard_failed", &[("error", &reason)]);
            }
        }
        Ok(())
    }

    /// The effective `ui.units` preference (the write path's current pair).
    fn effective_units(&self) -> cad_app::viewer_config::UnitsConfig {
        self.viewer_config.borrow().effective().ui.units
    }

    /// Persist the three `ui.units` leaves and install the derived
    /// [`cad_domain::UnitContext`] on the open document.
    ///
    /// The context is built **before** anything is written: the schema is the one
    /// place tokens become a `UnitContext`, so an illegal pair is refused with the
    /// field name and no file change. A missing document is refused up front, so
    /// a persisted preference can never disagree with the live header. The
    /// document write is [`cad_app::host::HostController::set_units`], which is a
    /// no-op for identical units (no revision).
    fn write_and_apply_units(
        &self,
        drawing: cad_app::viewer_config::UnitToken,
        display: cad_app::viewer_config::UnitToken,
        decimal_places: u8,
    ) -> CadResult<()> {
        let units = cad_app::viewer_config::UnitsConfig {
            drawing_unit: drawing,
            display_unit: display,
            decimal_places,
        };
        let context = units
            .unit_context()
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        if self.controller.borrow().drawing().is_none() {
            return Err(CadError::InvalidInput("no document is open".into()));
        }
        self.apply_user_preference_json(&format!(
            r#"{{"ui":{{"units":{{"drawingUnit":"{}","displayUnit":"{}","decimalPlaces":{}}}}}}}"#,
            drawing.as_token(),
            display.as_token(),
            decimal_places
        ))?;
        self.controller.borrow_mut().set_units(context)?;
        self.full_dirty.set(true);
        self.sync_render()
    }
}

/// The schema unit token carried by a [`CadSettingKey::DrawingUnit`] /
/// [`CadSettingKey::DisplayUnit`] write.
///
/// The key table has already refused an unknown token; this is the typed parse
/// of that same closed set, never a fallback to `drawing`.
fn unit_token(value: &CadSettingValue) -> CadResult<cad_app::viewer_config::UnitToken> {
    let CadSettingValue::Text(token) = value else {
        return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
    };
    cad_app::viewer_config::UnitToken::parse(token)
        .ok_or_else(|| CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()))
}
