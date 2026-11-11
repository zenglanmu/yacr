//! The real `NAVBAR` / `NAVVCUBE` / `NAVVCUBEDISPLAY` /
//! `DISPLAYVIEWCUBEIN2D` / `DISPLAYVIEWCUBEIN3D` shell selectors.
//!
//! These five names used to be two named refusals (`NAVBAR`, `NAVVCUBE`) and
//! three unmodelled names. They are now **settings writes**: each resolves to a
//! [`cad_host::CadSettingKey`] and goes through the host's one validated,
//! persisted settings path. They add no `CommandId` variant and do not change the
//! command-count contract — `CadSession::command` intercepts them before the
//! command table.
//!
//! Nothing here writes a preference directly: a failed write returns the host's
//! real error (and `cad_status.error` carries it) instead of a fabricated
//! success.

use cad_host::{CadSettingKey, CadSettingValue};

use super::commands::{
    parse_sysvar_toggle, DISPLAYVIEWCUBEIN2D_SELECTOR, DISPLAYVIEWCUBEIN3D_SELECTOR,
    NAVBAR_SELECTOR, NAVVCUBEDISPLAY_SELECTOR, NAVVCUBE_SELECTOR, SHELL_SETTING_SELECTORS,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// `navvcubeDisplay` bit for the 3D view class.
pub(super) const VIEW_CUBE_3D_BIT: u32 = 1;
/// `navvcubeDisplay` bit for the 2D view class.
pub(super) const VIEW_CUBE_2D_BIT: u32 = 2;

/// `() => ON|OFF|1|0`: `Some(None)` for the bare toggle form, `Some(Some(value))`
/// for an explicit value, `None` for an argument outside the grammar.
fn parse_switch(argument: &str) -> Option<Option<bool>> {
    if argument.is_empty() {
        return Some(None);
    }
    parse_sysvar_toggle(argument).map(Some)
}

/// Parse a `navvcubeDisplay` mask: exactly `0`, `1`, `2` or `3`.
pub(super) fn parse_display_mask(argument: &str) -> Option<u32> {
    let value: u32 = argument.trim().parse().ok()?;
    (value <= 3).then_some(value)
}

impl CadSession {
    /// Resolve one typed line against the five shell settings selectors.
    ///
    /// `None` means the line is not one of them (the ordinary command funnel
    /// continues). `Some(outcome)` is the real result of the settings write or
    /// its explicit refusal.
    pub(super) fn shell_setting_command(&self, text: &str) -> Option<Result<(), String>> {
        let trimmed = text.trim();
        let (name, argument) = match trimmed.find([' ', '\t']) {
            Some(index) => (&trimmed[..index], trimmed[index..].trim()),
            None => (trimmed, ""),
        };
        let selector = super::canonical_command_name(name)?;
        if !SHELL_SETTING_SELECTORS.contains(&selector) {
            return None;
        }
        Some(self.apply_shell_setting(selector, argument))
    }

    /// Apply one shell settings selector (already resolved by
    /// [`Self::shell_setting_command`]).
    fn apply_shell_setting(&self, selector: &'static str, argument: &str) -> Result<(), String> {
        match selector {
            NAVBAR_SELECTOR => {
                let desired = match parse_switch(argument) {
                    Some(Some(value)) => value,
                    Some(None) => !self.core.settings().navbar_visible,
                    None => {
                        return self.refuse_argument(format!(
                            "navbar.invalid_argument: '{}' is not ON/OFF/1/0 \
                             (the bare form toggles)",
                            argument
                        ))
                    }
                };
                self.write_shell_setting(
                    selector,
                    CadSettingKey::NavbarVisible,
                    CadSettingValue::Bool(desired),
                )
            }
            NAVVCUBE_SELECTOR => self.navvcube(argument),
            NAVVCUBEDISPLAY_SELECTOR => {
                let Some(display) = parse_display_mask(argument) else {
                    return self.refuse_argument(
                        "navvcubedisplay.needs_value: enter NAVVCUBEDISPLAY <0-3>".to_string(),
                    );
                };
                self.write_shell_setting(
                    selector,
                    CadSettingKey::NavvcubeDisplay,
                    CadSettingValue::Number(display),
                )
            }
            DISPLAYVIEWCUBEIN2D_SELECTOR => {
                self.view_cube_flag(selector, argument, CadSettingKey::ViewCubeIn2d)
            }
            DISPLAYVIEWCUBEIN3D_SELECTOR => {
                self.view_cube_flag(selector, argument, CadSettingKey::ViewCubeIn3d)
            }
            _ => Err(format!("unsupported shell settings selector: {selector}")),
        }
    }

    /// `NAVVCUBE` plus its argument forms.
    fn navvcube(&self, argument: &str) -> Result<(), String> {
        if argument.eq_ignore_ascii_case("PROPERTIES") {
            return self.refuse_argument(
                "navvcube.properties_not_implemented: this build has no ViewCube settings dialog"
                    .to_string(),
            );
        }
        let bit = self.view_class_bit();
        let current = self.core.settings().navvcube_display;
        let desired = match parse_switch(argument) {
            Some(Some(true)) => current | bit,
            Some(Some(false)) => current & !bit,
            Some(None) => current ^ bit,
            None => {
                return self.refuse_argument(format!(
                    "navvcube.invalid_argument: '{}' is not ON/OFF/1/0 (the bare form toggles the current view class)",
                    argument
                ))
            }
        };
        self.write_shell_setting(
            NAVVCUBE_SELECTOR,
            CadSettingKey::NavvcubeDisplay,
            CadSettingValue::Number(desired),
        )
    }

    /// `DISPLAYVIEWCUBEIN2D` / `DISPLAYVIEWCUBEIN3D` plus their argument forms.
    fn view_cube_flag(
        &self,
        selector: &'static str,
        argument: &str,
        key: CadSettingKey,
    ) -> Result<(), String> {
        let Some(enabled) = parse_sysvar_toggle(argument) else {
            return self.refuse_argument(format!(
                "{}.needs_value: enter {selector} ON|OFF",
                selector.to_ascii_lowercase()
            ));
        };
        self.write_shell_setting(selector, key, CadSettingValue::Bool(enabled))
    }

    /// The `navvcubeDisplay` bit of the **current** view class, read from the
    /// same authoritative `view_mode` the status `view.mode` projects.
    fn view_class_bit(&self) -> u32 {
        let is_3d = {
            let controller = self.core.controller.borrow();
            controller
                .application
                .workspace
                .viewports
                .get(&controller.viewport_id)
                .map(|viewport| viewport.view_mode.kind() == cad_app::ProjectionKind::ThreeD)
                .unwrap_or(false)
        };
        if is_3d {
            VIEW_CUBE_3D_BIT
        } else {
            VIEW_CUBE_2D_BIT
        }
    }

    /// Record the admitted selector and apply one settings write.
    ///
    /// The host validates and persists the value; a rejection is surfaced as the
    /// session's real error (and therefore `cad_status.error`), never swallowed.
    pub(super) fn write_shell_setting(
        &self,
        selector: &'static str,
        key: CadSettingKey,
        value: CadSettingValue,
    ) -> Result<(), String> {
        self.core.record_command(selector);
        match self.core.apply_setting(key, value) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
