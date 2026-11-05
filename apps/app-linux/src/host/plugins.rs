//! Linux plugin-store composition.
//!
//! The shared wire model and the five commands live in `cad-desktop-shell`; this
//! module only resolves the config directory and says whether the installed
//! carrier serves the plugin manager. `native-below` (native Wayland) does;
//! `dom-canvas` (X11/XWayland) does not, and keeps the bundled `tch` decoder
//! always-on in the wasm session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cad_desktop_shell::state::Shared;
use cad_platform::plugins::{
    FsPluginStore, NonPersistentPluginStore, PluginStore, PLUGIN_DIRECTORY,
};

use super::options::config_paths;

/// Resolve the plugin root (`<config-dir>/plugins`).
///
/// The policy file stays next to it (`<config-dir>/plugins.json`), which is the
/// same directory as `config.json`/`preference.json`.
fn plugin_root() -> Option<PathBuf> {
    let (config, _) = config_paths();
    config.and_then(|path| path.parent().map(|dir| dir.join(PLUGIN_DIRECTORY)))
}

/// The durable store, or the explicit non-persistent one when no config
/// directory exists (the same rule the config store follows).
fn store() -> Arc<dyn PluginStore> {
    match plugin_root() {
        Some(root) => Arc::new(FsPluginStore::new(root)),
        None => Arc::new(NonPersistentPluginStore),
    }
}

/// Wire plugin startup for the installed carrier.
///
/// On `native-below` this loads the policy, records the decoder enabled in this
/// process and enables the vendor pass only when a decoder is active. On
/// `dom-canvas` the native store stays absent/disabled and the status reports
/// the stable carrier refusal.
pub(crate) fn configure(shared: &Arc<Mutex<Shared>>, supported: bool) {
    if supported {
        cad_desktop_shell::plugins::configure_startup(shared, Some(store()), true);
    } else {
        cad_desktop_shell::plugins::configure_startup(shared, None, false);
    }
}
