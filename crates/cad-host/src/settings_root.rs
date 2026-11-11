//! The host-owned appearance/navigation preferences and the UI theme.
//!
//! `theme`, `navbarVisible`, `navvcubeDisplay`, `viewCubeIn2d` and
//! `viewCubeIn3d` are **root-level** `preference.json` leaves outside the
//! `ViewerConfig` schema (exactly like `wgpu_backend`). This module owns their
//! in-memory shape, their extraction/merge helpers and the writes that keep them
//! durable; [`crate::settings`] owns the closed key table and the projection into
//! [`crate::settings::HostSettings`].

use super::*;

use crate::settings::{preference_persist_refusal, CadSettingValue, SETTINGS_INVALID_VALUE};

// The renderer backend choice is the same kind of root preference: one leaf
// outside the `ViewerConfig` schema. Its helpers live here with the others.

/// The two UI themes this build really draws.
///
/// The tokens are the wire values of `CadSettingValue::Text` for
/// [`crate::settings::CadSettingKey::Theme`], the durable `theme` preference leaf and the status
/// `theme` field. Each theme is one pair of canvas backgrounds
/// ([`THEME_DARK_BACKGROUND`] / [`THEME_LIGHT_BACKGROUND`]); nothing else about
/// the chrome is re-skinned here.
pub const THEME_VALUES: [&str; 2] = ["dark", "light"];
/// The model/paper canvas background the `dark` theme applies.
pub const THEME_DARK_BACKGROUND: &str = "#000000";
/// The model/paper canvas background the `light` theme applies.
pub const THEME_LIGHT_BACKGROUND: &str = "#ffffff";

/// The five host-owned appearance/navigation preference keys.
///
/// They are **root-level** leaves of `preference.json`, exactly like
/// [`WGPU_BACKEND_PREFERENCE_KEY`]: they describe the React shell (theme,
/// navigation bar, ViewCube), not the `ViewerConfig` scene schema, so they are
/// not camelCased into a dotted viewer path and are not in
/// [`HOST_ALLOWED_PATHS_JSON`].
pub const ROOT_PREFERENCE_KEYS: [&str; 5] = [
    "theme",
    "navbarVisible",
    "navvcubeDisplay",
    "viewCubeIn2d",
    "viewCubeIn3d",
];

/// The five host-owned leaves, in the shape the host keeps in memory.
///
/// `None` means "the user never chose", which is exactly the documented default
/// ([`HostSettings::theme`] etc.). Keeping `None` present-or-absent lets a real
/// clear remove the leaves instead of writing defaults the user never picked.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RootPreferences {
    pub(crate) theme: Option<String>,
    pub(crate) navbar_visible: Option<bool>,
    pub(crate) navvcube_display: Option<u32>,
    pub(crate) view_cube_in_2d: Option<bool>,
    pub(crate) view_cube_in_3d: Option<bool>,
}

impl RootPreferences {
    /// The `theme` token, or the documented `dark` default.
    pub(crate) fn theme(&self) -> &str {
        self.theme.as_deref().unwrap_or(THEME_VALUES[0])
    }

    pub(crate) fn navbar_visible(&self) -> bool {
        self.navbar_visible.unwrap_or(true)
    }

    pub(crate) fn navvcube_display(&self) -> u32 {
        self.navvcube_display.unwrap_or(3)
    }

    pub(crate) fn view_cube_in_2d(&self) -> bool {
        self.view_cube_in_2d.unwrap_or(true)
    }

    pub(crate) fn view_cube_in_3d(&self) -> bool {
        self.view_cube_in_3d.unwrap_or(true)
    }
}

/// Extract the five host-owned leaves from a durable preference document.
///
/// One malformed or absent leaf yields that leaf's default (`None` = "not
/// chosen"), never an error and never a whole-document rejection: a preference
/// file written by another build must still open.
pub(crate) fn root_preferences_from_preference(text: &str) -> RootPreferences {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(text) else {
        return RootPreferences::default();
    };
    let theme = document
        .get("theme")
        .and_then(serde_json::Value::as_str)
        .filter(|token| THEME_VALUES.contains(token))
        .map(str::to_string);
    let navbar_visible = document
        .get("navbarVisible")
        .and_then(serde_json::Value::as_bool);
    let navvcube_display = document
        .get("navvcubeDisplay")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value <= 3);
    let view_cube_in_2d = document
        .get("viewCubeIn2d")
        .and_then(serde_json::Value::as_bool);
    let view_cube_in_3d = document
        .get("viewCubeIn3d")
        .and_then(serde_json::Value::as_bool);
    RootPreferences {
        theme,
        navbar_visible,
        navvcube_display,
        view_cube_in_2d,
        view_cube_in_3d,
    }
}

/// Remove the five host-owned leaves from a durable preference document.
///
/// The store's allowed-path validation is strict, so the leaves are captured
/// separately ([`root_preferences_from_preference`]) and stripped here before the
/// rest of the document is applied. Only the root level is touched: these keys
/// have no `ViewerConfig` path. Text that is not valid JSON is returned as-is so
/// the store reports its own parse error.
pub(crate) fn without_root_preferences(text: &str) -> String {
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    if let Some(object) = document.as_object_mut() {
        for key in ROOT_PREFERENCE_KEYS {
            object.remove(key);
        }
    }
    serde_json::to_string(&document).unwrap_or_else(|_| text.to_string())
}

/// Merge the five host-owned leaves into the projected preference document.
///
/// `projected` is the store's allowed-leaf projection (exactly what the host
/// persists). A chosen leaf is written at the root; an unchosen (`None`) leaf is
/// removed, so a real clear drops it and the documented default applies. A
/// projection that is not a JSON object is an explicit error, never a silently
/// rewritten file.
pub(crate) fn merge_root_preferences_into_preference(
    projected: &str,
    preferences: &RootPreferences,
) -> CadResult<String> {
    let mut document: serde_json::Value = serde_json::from_str(projected).map_err(|error| {
        CadError::InvalidInput(format!("preference projection is not JSON: {error}"))
    })?;
    let object = document.as_object_mut().ok_or_else(|| {
        CadError::InvalidInput("preference projection is not a JSON object".into())
    })?;
    match &preferences.theme {
        Some(theme) => {
            object.insert(
                "theme".to_string(),
                serde_json::Value::String(theme.clone()),
            );
        }
        None => {
            object.remove("theme");
        }
    }
    for (key, value) in [
        ("navbarVisible", preferences.navbar_visible),
        ("viewCubeIn2d", preferences.view_cube_in_2d),
        ("viewCubeIn3d", preferences.view_cube_in_3d),
    ] {
        match value {
            Some(enabled) => {
                object.insert(key.to_string(), serde_json::Value::Bool(enabled));
            }
            None => {
                object.remove(key);
            }
        }
    }
    match preferences.navvcube_display {
        Some(display) => {
            object.insert(
                "navvcubeDisplay".to_string(),
                serde_json::Value::Number(serde_json::Number::from(display)),
            );
        }
        None => {
            object.remove("navvcubeDisplay");
        }
    }
    serde_json::to_string(&document)
        .map_err(|error| CadError::InvalidInput(format!("preference write is not JSON: {error}")))
}

/// The durable preference key of the renderer backend choice.
///
/// It is a **root-level** leaf of the host preference document (`preference.json`)
/// with exactly this spelling — the same key the UI writes and the hosts' late
/// presenter construction reads. Unlike the other settings it is not camelCased
/// into the `ViewerConfig` schema: it is renderer state, so the host owns it.
pub const WGPU_BACKEND_PREFERENCE_KEY: &str = "wgpu_backend";

/// The closed set of renderer backend tokens, `auto` first (the default).
///
/// These are the wire values of the `wgpu_backend` preference *and* of
/// `CadSettingValue::Text` for [`CadSettingKey::WgpuBackend`]: the Options dialog,
/// the durable preference and the renderer policy parse the same three strings,
/// so a new spelling would have to be added in all three places at once.
pub const WGPU_BACKEND_VALUES: [&str; 3] = ["auto", "vulkan", "dx12"];

/// Normalize a raw durable value to the closed set; absent/unknown → `auto`.
pub fn normalize_wgpu_backend(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else {
        return WGPU_BACKEND_VALUES[0];
    };
    WGPU_BACKEND_VALUES
        .into_iter()
        .find(|candidate| *candidate == raw)
        .unwrap_or(WGPU_BACKEND_VALUES[0])
}

/// Extract the `wgpu_backend` leaf from a durable preference document.
///
/// The root key is canonical; a nested `ui.wgpu_backend` is tolerated so a
/// projection that nests it cannot silently disable the setting. A non-string or
/// an unknown token yields `None` (the caller falls back to `auto`), never an
/// invented value.
pub(crate) fn wgpu_backend_from_preference(text: &str) -> Option<String> {
    let document: serde_json::Value = serde_json::from_str(text).ok()?;
    let raw = document
        .get(WGPU_BACKEND_PREFERENCE_KEY)
        .or_else(|| {
            document
                .get("ui")
                .and_then(|ui| ui.get(WGPU_BACKEND_PREFERENCE_KEY))
        })
        .and_then(serde_json::Value::as_str)?;
    WGPU_BACKEND_VALUES.contains(&raw).then(|| raw.to_string())
}

/// Remove the renderer backend leaf from a durable preference document.
///
/// The store's allowed-path validation is strict: one leaf outside the host
/// allowances rejects the whole patch. The backend choice is the one leaf the
/// `ViewerConfig` schema cannot express, so it is captured separately
/// ([`wgpu_backend_from_preference`]) and stripped here before the rest of the
/// document is applied — every other leaf is passed through unchanged. Text that
/// is not valid JSON is returned as-is so the store reports its own parse error
/// instead of this helper inventing one.
pub(crate) fn without_wgpu_backend(text: &str) -> String {
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    if let Some(object) = document.as_object_mut() {
        object.remove(WGPU_BACKEND_PREFERENCE_KEY);
        if let Some(ui) = object.get_mut("ui").and_then(|ui| ui.as_object_mut()) {
            ui.remove(WGPU_BACKEND_PREFERENCE_KEY);
        }
    }
    serde_json::to_string(&document).unwrap_or_else(|_| text.to_string())
}

/// Merge the renderer backend choice into the projected preference document.
///
/// `projected` is the store's allowed-leaf projection (exactly what the host
/// persists). The backend leaf is added at the root — or removed when the choice
/// is `None` — so every preference write keeps the one leaf the schema cannot
/// express. A projection that is not a JSON object is an explicit error, never a
/// silently rewritten file.
pub(crate) fn merge_wgpu_backend_into_preference(
    projected: &str,
    backend: Option<&str>,
) -> CadResult<String> {
    let mut document: serde_json::Value = serde_json::from_str(projected).map_err(|error| {
        CadError::InvalidInput(format!("preference projection is not JSON: {error}"))
    })?;
    let object = document.as_object_mut().ok_or_else(|| {
        CadError::InvalidInput("preference projection is not a JSON object".into())
    })?;
    match backend {
        Some(token) => {
            object.insert(
                WGPU_BACKEND_PREFERENCE_KEY.to_string(),
                serde_json::Value::String(token.to_string()),
            );
        }
        None => {
            object.remove(WGPU_BACKEND_PREFERENCE_KEY);
        }
    }
    serde_json::to_string(&document)
        .map_err(|error| CadError::InvalidInput(format!("preference write is not JSON: {error}")))
}

impl HostCore {
    /// Persist one boolean root preference (`navbarVisible`, `viewCubeIn2d`,
    /// `viewCubeIn3d`).
    ///
    /// The leaf lives outside the `ViewerConfig` schema, so this is the
    /// `apply_wgpu_backend` pattern: keep the live value in memory, write
    /// the whole projected document with the leaf merged in
    /// ([`HostCore::persist_preference`]), and roll the in-memory value back and
    /// return the error when the write fails. Never a success the next start
    /// would lose.
    pub(crate) fn apply_root_flag(
        &self,
        value: &CadSettingValue,
        field: fn(&mut RootPreferences) -> &mut Option<bool>,
    ) -> CadResult<()> {
        let CadSettingValue::Bool(enabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        let previous = {
            let mut preferences = self.root_preferences.borrow_mut();
            field(&mut preferences).replace(*enabled)
        };
        if let Err(error) = self.persist_preference() {
            *field(&mut self.root_preferences.borrow_mut()) = previous;
            return Err(preference_persist_refusal(&error));
        }
        Ok(())
    }

    /// Persist the ViewCube display bitmask (root `navvcubeDisplay`).
    pub(crate) fn apply_navvcube_display(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(display) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if *display > 3 {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        let previous = self
            .root_preferences
            .borrow_mut()
            .navvcube_display
            .replace(*display);
        if let Err(error) = self.persist_preference() {
            self.root_preferences.borrow_mut().navvcube_display = previous;
            return Err(preference_persist_refusal(&error));
        }
        Ok(())
    }

    /// Apply the UI theme: persist the root `theme` leaf **and** the two canvas
    /// backgrounds the theme names.
    ///
    /// All-or-nothing: the backgrounds are written first (a failure restores the
    /// one already written and leaves the theme leaf untouched), then the theme
    /// leaf; if that write fails both backgrounds are restored from the values
    /// captured before the call. The function never returns `Ok` with only part
    /// of the theme applied.
    pub(crate) fn apply_theme(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(token) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if !THEME_VALUES.contains(&token.as_str()) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        let (previous_model, previous_paper) = {
            let store = self.viewer_config.borrow();
            let effective = store.effective();
            (
                effective.view.background.model.to_hex(),
                effective.view.background.paper.to_hex(),
            )
        };
        let colour = if token == "light" {
            THEME_LIGHT_BACKGROUND
        } else {
            THEME_DARK_BACKGROUND
        };
        let background = |value: &str| CadSettingValue::Text(value.to_string());
        // 1. Model background. A failure here changes nothing durable: the write
        //    path restores its own store snapshot.
        self.apply_background_preference("model", &background(colour))?;
        // 2. Paper background. A failure restores the model background.
        if let Err(error) = self.apply_background_preference("paper", &background(colour)) {
            let _ = self.apply_background_preference("model", &background(&previous_model));
            return Err(error);
        }
        // 3. The theme leaf itself. A failure restores both backgrounds.
        let previous_theme = self
            .root_preferences
            .borrow_mut()
            .theme
            .replace(token.clone());
        if let Err(error) = self.persist_preference() {
            self.root_preferences.borrow_mut().theme = previous_theme;
            let _ = self.apply_background_preference("model", &background(&previous_model));
            let _ = self.apply_background_preference("paper", &background(&previous_paper));
            return Err(preference_persist_refusal(&error));
        }
        Ok(())
    }
}
