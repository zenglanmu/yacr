//! Native host ViewerConfig and user-preference persistence.
//!
//! Mirrors the browser host data flow: a host `ViewerConfig` is read from disk
//! at startup, then a projected user-preference patch is merged. The projection
//! comes from the shared store, so a path the host has since revoked cannot be
//! resurrected on a later load. A malformed file is reported, never fatal and
//! never a silent success.
//!
//! The load/persist logic itself now lives in `cad_host::HostCore` (driven
//! through the `cad_platform::ConfigStore` contract); this module keeps only the
//! platform path resolvers.
use std::path::{Path, PathBuf};

/// File name of the full host `ViewerConfig`.
pub const CONFIG_FILE_NAME: &str = "config.json";
/// File name of the projected user-preference patch.
pub const PREFERENCE_FILE_NAME: &str = "preferences.json";

/// Per-user configuration directory, or `None` when it cannot be resolved.
///
/// Windows: `%APPDATA%\yacr`, falling back to `%LOCALAPPDATA%\yacr`.
/// macOS: `$HOME/Library/Application Support/yacr` (Apple's per-user data
/// location; `$XDG_CONFIG_HOME` is deliberately not consulted so a stray shell
/// variable cannot redirect the store).
/// Everywhere else: `$XDG_CONFIG_HOME/yacr` when set and non-empty, otherwise
/// `$HOME/.config/yacr`. This never guesses a path.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        resolve_windows_config_dir(
            std::env::var("APPDATA").ok().as_deref(),
            std::env::var("LOCALAPPDATA").ok().as_deref(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        resolve_macos_config_dir(std::env::var("HOME").ok().as_deref())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        resolve_config_dir(
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        )
    }
}

/// Windows form of [`config_dir`]: `%APPDATA%\yacr` then `%LOCALAPPDATA%\yacr`.
///
/// Pure so the precedence is testable without environment mutation; empty
/// values are treated as unset.
pub fn resolve_windows_config_dir(appdata: Option<&str>, local: Option<&str>) -> Option<PathBuf> {
    appdata
        .filter(|value| !value.is_empty())
        .or_else(|| local.filter(|value| !value.is_empty()))
        .map(|base| Path::new(base).join("yacr"))
}

/// Unix form of [`config_dir`] (`$XDG_CONFIG_HOME/yacr` then `$HOME/.config/yacr`).
/// Pure so the precedence is testable without env mutation. Empty values are
/// treated as unset.
pub fn resolve_config_dir(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(base) = xdg.filter(|value| !value.is_empty()) {
        return Some(Path::new(base).join("yacr"));
    }
    if let Some(base) = home.filter(|value| !value.is_empty()) {
        return Some(Path::new(base).join(".config").join("yacr"));
    }
    None
}

/// macOS form of [`config_dir`]: `$HOME/Library/Application Support/yacr`.
///
/// Pure so the precedence is testable without environment mutation; an empty
/// value is treated as unset.
pub fn resolve_macos_config_dir(home: Option<&str>) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty()).map(|base| {
        Path::new(base)
            .join("Library")
            .join("Application Support")
            .join("yacr")
    })
}

/// Default full-config path, if a configuration directory resolves.
pub fn default_config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(CONFIG_FILE_NAME))
}

/// Default user-preference path, if a configuration directory resolves.
pub fn default_preference_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PREFERENCE_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_config_dir_uses_application_support() {
        assert_eq!(
            resolve_macos_config_dir(Some("/Users/alice")),
            Some(PathBuf::from(
                "/Users/alice/Library/Application Support/yacr"
            ))
        );
        // Empty/absent HOME never guesses a path.
        assert_eq!(resolve_macos_config_dir(Some("")), None);
        assert_eq!(resolve_macos_config_dir(None), None);
    }

    #[test]
    fn macos_config_path_does_not_consult_xdg() {
        // Guard against a regression reusing the XDG resolver on macOS: the pure
        // macOS resolver takes only HOME and always yields the Library location.
        let dir = resolve_macos_config_dir(Some("/Users/alice")).unwrap();
        assert!(dir.ends_with("Library/Application Support/yacr"));
        assert!(!dir.to_string_lossy().contains(".config"));
    }
}
