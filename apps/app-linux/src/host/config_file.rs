//! Native host ViewerConfig and user-preference persistence.
//!
//! Mirrors the browser host data flow: a host `ViewerConfig` is read from disk
//! at startup, then a projected user-preference patch is merged. The projection
//! comes from the shared store, so a path the host has since revoked cannot be
//! resurrected on a later load. A malformed file is reported, never fatal and
//! never a silent success.
use std::path::{Path, PathBuf};

use cad_domain::{CadError, CadResult};
use cad_ui_slint::UiHandle;

use super::atomic_write;

/// File name of the full host `ViewerConfig`.
pub const CONFIG_FILE_NAME: &str = "config.json";
/// File name of the projected user-preference patch.
pub const PREFERENCE_FILE_NAME: &str = "preferences.json";

/// Host policy mirroring the web host: users may toggle the two dock panels'
/// initial-open flags. A host `config.json` replaces this policy wholesale.
const HOST_ALLOWED_PATHS_JSON: &str = r#"{"ui":{"userCustomization":{"allowedPaths":[
    "ui.components.layerPanel.initiallyOpen",
    "ui.components.propertiesPanel.initiallyOpen"
]}}}"#;

/// Per-user configuration directory, or `None` when it cannot be resolved.
///
/// `$XDG_CONFIG_HOME/yacr` when that variable is set and non-empty, otherwise
/// `$HOME/.config/yacr`. This never guesses a path.
pub fn config_dir() -> Option<PathBuf> {
    resolve_config_dir(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Pure form of [`config_dir`] so the precedence is testable without env
/// mutation. Empty values are treated as unset.
pub fn resolve_config_dir(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(base) = xdg.filter(|value| !value.is_empty()) {
        return Some(Path::new(base).join("yacr"));
    }
    if let Some(base) = home.filter(|value| !value.is_empty()) {
        return Some(Path::new(base).join(".config").join("yacr"));
    }
    None
}

/// Default full-config path, if a configuration directory resolves.
pub fn default_config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(CONFIG_FILE_NAME))
}

/// Default user-preference path, if a configuration directory resolves.
pub fn default_preference_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PREFERENCE_FILE_NAME))
}

/// Load the host config and user preference from disk at startup.
///
/// First installs the host default allowed paths (mirroring the web host), then
/// replaces the host config from `config_path` when present, then merges the
/// preference from `preference_path` and re-writes its allowed projection. An
/// absent file is normal; a malformed or unreadable one keeps the previous
/// state and adds a human-readable diagnostic. Returns every diagnostic.
pub fn load_startup(
    handle: &UiHandle,
    config_path: Option<&Path>,
    preference_path: Option<&Path>,
) -> Vec<String> {
    let mut diagnostics = Vec::new();

    if let Err(error) = handle.update_config_json(HOST_ALLOWED_PATHS_JSON) {
        diagnostics.push(format!("host config defaults rejected: {error}"));
    }

    if let Some(path) = config_path {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Err(error) = handle.set_config_json(&text) {
                    diagnostics.push(format!("config file {} rejected: {error}", path.display()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => diagnostics.push(format!(
                "config file {} unreadable: {error}",
                path.display()
            )),
        }
    }

    if let Some(path) = preference_path {
        match std::fs::read_to_string(path) {
            Ok(text) => match handle.apply_user_preference_json(&text) {
                Ok(()) => {
                    if let Err(error) = persist_preference(handle, path) {
                        diagnostics.push(format!(
                            "preferences {} not re-persisted: {error}",
                            path.display()
                        ));
                    }
                }
                Err(error) => {
                    diagnostics.push(format!("preferences {} rejected: {error}", path.display()))
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => diagnostics.push(format!(
                "preferences {} unreadable: {error}",
                path.display()
            )),
        }
    }

    diagnostics
}

/// Write the store's allowed projection of the user preference to `path`.
///
/// The projection (not the raw preference) is written, so a revoked path is
/// dropped rather than persisted. Missing parent directories are created; the
/// write itself is the shared temp-file + fsync + rename helper.
pub fn persist_preference(handle: &UiHandle, path: &Path) -> CadResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CadError::InvalidInput(e.to_string()))?;
        }
    }
    let text = handle.projected_user_preference_json();
    atomic_write(path, text.as_bytes())
}
