//! The Save-As export-version settings: the ACIS payload version and the DXF
//! container version.
//!
//! They are ordinary `ui.export.*` leaves (next to `plotFormat`), so a
//! preference write goes through the same validated store/persist path as every
//! other setting. What makes them special is where they *end*: the save funnel
//! installs the typed [`ExportOptions`] on the `HostController`, which turns them
//! into the `ExportRequest` the exporter sees. They never touch `cad-db`: no
//! revision, no history, no document mutation.
//!
//! The container/payload matrix is not duplicated here. [`export_options`]
//! returns whatever the user chose and the exporter refuses an unrepresentable
//! combination by name (`export.acis_asm_requires_r2013`,
//! `export.dwg_version_fixed`), so this module can never silently downgrade a
//! choice.
use super::*;

use cad_app::host::ExportOptions;

/// The accepted `acisVersion` tokens, default first.
pub const ACIS_VERSION_VALUES: [&str; 2] = ["7.0", "21.0"];
/// The accepted `dxfVersion` tokens, default first.
pub const DXF_VERSION_VALUES: [&str; 3] = ["2010", "2013", "2018"];

/// Validate an `acisVersion` value: one of [`ACIS_VERSION_VALUES`], never
/// normalized to the default.
pub(crate) fn acis_version_setting(value: &CadSettingValue) -> Result<(), &'static str> {
    match value {
        CadSettingValue::Text(token) if ACIS_VERSION_VALUES.contains(&token.as_str()) => Ok(()),
        _ => Err(SETTINGS_INVALID_VALUE),
    }
}

/// Validate a `dxfVersion` value: one of [`DXF_VERSION_VALUES`], never
/// normalized to the default.
pub(crate) fn dxf_version_setting(value: &CadSettingValue) -> Result<(), &'static str> {
    match value {
        CadSettingValue::Text(token) if DXF_VERSION_VALUES.contains(&token.as_str()) => Ok(()),
        _ => Err(SETTINGS_INVALID_VALUE),
    }
}

impl HostCore {
    /// The typed export options the save funnel installs on the controller.
    ///
    /// Read from the effective config (default → `config.json` → host-allowed
    /// `preference.json`), so it is the same source the Options dialog reports.
    pub(crate) fn export_options(&self) -> ExportOptions {
        let store = self.viewer_config.borrow();
        let export = &store.effective().ui.export;
        ExportOptions {
            acis_version: export.acis_version,
            dxf_target: export.dxf_version,
        }
    }

    /// Persist `ui.export.acisVersion` and make it the next Save-As payload.
    ///
    /// The save funnel reads the effective config when it builds the request, so
    /// the write is the live application: no session state and no document touch.
    pub(crate) fn apply_acis_version(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(token) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if !ACIS_VERSION_VALUES.contains(&token.as_str()) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        self.apply_user_preference_json(&format!(
            r#"{{"ui":{{"export":{{"acisVersion":"{token}"}}}}}}"#
        ))
    }

    /// Persist `ui.export.dxfVersion` and make it the next Save-As container.
    ///
    /// A DWG save always writes AC1032: the DWG writer derives its version from
    /// the document, so the DXF target is a DXF-only choice (the controller's
    /// DWG request names the DWG's own container).
    pub(crate) fn apply_dxf_version(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(token) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if !DXF_VERSION_VALUES.contains(&token.as_str()) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        self.apply_user_preference_json(&format!(
            r#"{{"ui":{{"export":{{"dxfVersion":"{token}"}}}}}}"#
        ))
    }
}
