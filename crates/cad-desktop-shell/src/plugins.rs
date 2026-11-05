//! Plugin manager commands for every **native** Tauri carrier.
//!
//! The carrier-neutral wire model and validation live in
//! [`cad_ui_core::session::plugins`]; the durable bytes live behind
//! [`cad_platform::plugins::PluginStore`]. This module owns only the five
//! `#[tauri::command]` wrappers, the startup gating and the composed registration
//! list.
//!
//! # Phase 1 semantics
//!
//! * One active decoder (radio). `tch` is active when no policy file exists.
//! * Every mutation persists immediately; the running session is never mutated,
//!   so `restartRequired` is the honest "takes effect on next launch" signal.
//! * `dom-canvas` has no native plugin registry. Its status is
//!   `supported:false` with `plugin.unsupported_carrier: dom-canvas` in
//!   `diagnostics`, mutations answer `accepted:false` with the same reason, and
//!   the bundled `tch` stays always-on (the wasm session keeps decoding).
//! * A refusal is a real `PluginMutationResult` with `accepted:false` and the
//!   current registry snapshot, not an empty success and not an exception the
//!   frontend cannot render.
//!
//! Only `app-linux` (`native-below`), `app-windows`, `app-macos` and
//! `app-android` register these commands. Web refuses them by name in the
//! browser adapter.

use std::sync::{Arc, Mutex};

use cad_platform::plugins::{PluginStore, PLUGIN_STORE_UNAVAILABLE_REASON};
use cad_ui_core::session::plugins::{
    decode_plugin_payload, format_installed_at, is_reserved_plugin_id, is_valid_plugin_id,
    plugin_id_for_filename, PluginEntryFrame, PluginManifest, PluginMutationResult, PluginPolicy,
    PluginRow, PluginRowStatus, PluginStatusFrame, BUILTIN_PLUGIN, BUILTIN_PLUGIN_ID,
    PLUGIN_INVALID_ID_REASON, PLUGIN_MANIFEST_MISSING_REASON, PLUGIN_NOT_INSTALLED_REASON,
    PLUGIN_POLICY_INVALID_REASON, PLUGIN_POLICY_VERSION, PLUGIN_READ_ENTRY_BUILTIN_REASON,
    PLUGIN_RESERVED_ID_REASON, PLUGIN_UNINSTALL_BUILTIN_REASON, PLUGIN_UNSUPPORTED_CARRIER_REASON,
};
use serde::Deserialize;
use tauri::ipc::Invoke;

use crate::state::{lock, ProbeShared, Shared};

/// Read the registry snapshot.
pub const CAD_PLUGIN_STATUS_COMMAND: &str = "cad_plugin_status";
/// Install one base64 `.js`/`.mjs` bundle.
pub const CAD_PLUGIN_INSTALL_COMMAND: &str = "cad_plugin_install";
/// Persist the active decoder / disable it.
pub const CAD_PLUGIN_ENABLE_COMMAND: &str = "cad_plugin_enable";
/// Remove one user plugin.
pub const CAD_PLUGIN_UNINSTALL_COMMAND: &str = "cad_plugin_uninstall";
/// Read one installed entry bundle (base64).
pub const CAD_PLUGIN_READ_ENTRY_COMMAND: &str = "cad_plugin_read_entry";

/// Optional manifest fields the frontend sends after validating the ES module.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMetaArgs {
    /// The module's declared id. Accepted but ignored: the directory id is
    /// derived from the filename so two uploads cannot alias the same directory.
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

/// Whether `name` is one of this module's five commands.
pub fn is_plugin_command(name: &str) -> bool {
    matches!(
        name,
        CAD_PLUGIN_STATUS_COMMAND
            | CAD_PLUGIN_INSTALL_COMMAND
            | CAD_PLUGIN_ENABLE_COMMAND
            | CAD_PLUGIN_UNINSTALL_COMMAND
            | CAD_PLUGIN_READ_ENTRY_COMMAND
    )
}

/// Configure plugin startup on this host: store, carrier support, the decoder
/// actually enabled in this process, and the vendor pass gate.
///
/// `supported` is the **carrier** answer (`native-below` only). A host with no
/// durable store still keeps the bundled `tch` working; it only cannot manage
/// external plugins.
pub fn configure_startup(
    shared: &Arc<Mutex<Shared>>,
    store: Option<Arc<dyn PluginStore>>,
    supported: bool,
) {
    let mut state = lock(shared);
    state.plugin_store = if supported { store } else { None };
    state.plugins_supported = supported;
    let session_active = if supported {
        enabled_at_launch(state.plugin_store.as_deref())
    } else {
        // `dom-canvas` keeps the bundled decoder on; the wasm session owns the
        // actual pass, so the native vendor store stays disabled.
        Some(BUILTIN_PLUGIN_ID.to_string())
    };
    state.plugins_session_active = session_active.clone();
    if supported && session_active.is_some() {
        state.vendor.enable();
    }
}

/// The decoder this launch enables, validated against the installed set.
fn enabled_at_launch(store: Option<&dyn PluginStore>) -> Option<String> {
    let Some(store) = store else {
        // The built-in ships with the application; no store is needed to run it.
        return Some(BUILTIN_PLUGIN_ID.to_string());
    };
    let policy = match store.load_policy() {
        Ok(None) | Err(_) => PluginPolicy::builtin_default(),
        Ok(Some(text)) => PluginPolicy::parse(&text).unwrap_or_default(),
    };
    let active = policy.active_decoder?;
    if active == BUILTIN_PLUGIN_ID {
        return Some(active);
    }
    let installed = store.scan().plugins.into_iter().any(|record| {
        record.id == active
            && record
                .manifest
                .as_deref()
                .and_then(|text| PluginManifest::parse(text).ok())
                .is_some_and(|manifest| manifest.id == active)
    });
    installed.then_some(active)
}

/// Unix epoch seconds (0 when the system clock is before the epoch).
fn unix_seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The carrier wire name.
fn carrier_name(supported: bool) -> &'static str {
    if supported {
        "native-below"
    } else {
        "dom-canvas"
    }
}

/// Load the policy for a status projection.
fn status_policy(store: &dyn PluginStore, diagnostics: &mut Vec<String>) -> PluginPolicy {
    match store.load_policy() {
        Ok(None) => PluginPolicy::builtin_default(),
        Ok(Some(text)) => match PluginPolicy::parse(&text) {
            Ok(policy) => policy,
            Err(error) => {
                diagnostics.push(error);
                PluginPolicy::builtin_default()
            }
        },
        Err(error) => {
            diagnostics.push(error);
            PluginPolicy::builtin_default()
        }
    }
}

/// Load the policy for a mutation. A malformed policy refuses instead of being
/// overwritten with a guess.
fn mutation_policy(store: &dyn PluginStore) -> Result<PluginPolicy, String> {
    match store.load_policy() {
        Ok(None) => Ok(PluginPolicy::builtin_default()),
        Ok(Some(text)) => PluginPolicy::parse(&text),
        Err(error) => Err(format!("{PLUGIN_POLICY_INVALID_REASON}: {error}")),
    }
}

fn require_supported(state: &Shared) -> Result<(), String> {
    if state.plugins_supported {
        Ok(())
    } else {
        Err(PLUGIN_UNSUPPORTED_CARRIER_REASON.to_string())
    }
}

fn require_store(state: &Shared) -> Result<Arc<dyn PluginStore>, String> {
    state
        .plugin_store
        .clone()
        .ok_or_else(|| PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
}

/// Build the full status from the current shared state.
///
/// This is the only projection: the status command and every mutation result
/// use it, so a mutation can never report a registry shape the status command
/// would not.
fn status_from_state(state: &Shared) -> PluginStatusFrame {
    if !state.plugins_supported {
        return PluginStatusFrame::unsupported();
    }
    let session = state.plugins_session_active.clone();
    let Some(store) = state.plugin_store.as_deref() else {
        return PluginStatusFrame::new(
            true,
            carrier_name(true),
            Some(BUILTIN_PLUGIN_ID.to_string()),
            session,
            vec![BUILTIN_PLUGIN.row(
                true,
                state.plugins_session_active.as_deref() == Some(BUILTIN_PLUGIN_ID),
            )],
            vec![PLUGIN_STORE_UNAVAILABLE_REASON.to_string()],
        );
    };
    let mut diagnostics = Vec::new();
    let policy = status_policy(store, &mut diagnostics);
    let scan = store.scan();
    diagnostics.extend(scan.diagnostics);

    enum Parsed {
        Healthy(PluginManifest),
        Missing(String),
        Invalid(String),
    }
    let mut parsed: Vec<Parsed> = Vec::with_capacity(scan.plugins.len());
    for record in scan.plugins {
        match record.manifest {
            None => parsed.push(Parsed::Missing(record.id)),
            Some(text) => match PluginManifest::parse(&text) {
                Ok(manifest) if manifest.id == record.id => parsed.push(Parsed::Healthy(manifest)),
                Ok(manifest) => {
                    diagnostics.push(format!(
                        "plugin.manifest_invalid: {}: id {} does not match the directory",
                        record.id, manifest.id
                    ));
                    parsed.push(Parsed::Invalid(record.id));
                }
                Err(error) => {
                    diagnostics.push(error);
                    parsed.push(Parsed::Invalid(record.id));
                }
            },
        }
    }

    // A configured decoder must exist. A missing target is reported, and the
    // effective answer is "none" rather than pointing at bytes we did not find.
    let active = match policy.active_decoder.clone() {
        None => None,
        Some(id) if id == BUILTIN_PLUGIN_ID => Some(id),
        Some(id)
            if parsed.iter().any(|entry| match entry {
                Parsed::Healthy(manifest) => manifest.id == id,
                Parsed::Missing(record_id) | Parsed::Invalid(record_id) => record_id == &id,
            }) =>
        {
            Some(id)
        }
        Some(id) => {
            diagnostics.push(format!("plugin.active_missing: {id}"));
            None
        }
    };

    let mut plugins = Vec::with_capacity(parsed.len() + 1);
    plugins.push(BUILTIN_PLUGIN.row(
        active.as_deref() == Some(BUILTIN_PLUGIN_ID),
        session.as_deref() == Some(BUILTIN_PLUGIN_ID),
    ));
    for entry in parsed {
        match entry {
            Parsed::Healthy(manifest) => {
                let id = manifest.id.clone();
                plugins.push(PluginRow::from_manifest(
                    &manifest,
                    active.as_deref() == Some(id.as_str()),
                    session.as_deref() == Some(id.as_str()),
                ));
            }
            Parsed::Missing(id) => plugins.push(PluginRow::invalid(
                &id,
                PluginRowStatus::ManifestMissing,
                active.as_deref() == Some(id.as_str()),
                session.as_deref() == Some(id.as_str()),
            )),
            Parsed::Invalid(id) => plugins.push(PluginRow::invalid(
                &id,
                PluginRowStatus::ManifestInvalid,
                active.as_deref() == Some(id.as_str()),
                session.as_deref() == Some(id.as_str()),
            )),
        }
    }

    // The policy and the scan can report the same unavailable store; a stable
    // deduplicated list is easier for the chrome to show than a repeated line.
    let mut seen = std::collections::HashSet::new();
    diagnostics.retain(|diagnostic| seen.insert(diagnostic.clone()));

    PluginStatusFrame::new(
        true,
        carrier_name(true),
        active,
        session,
        plugins,
        diagnostics,
    )
}

/// The current registry snapshot as a Tauri response body.
pub fn cad_plugin_status_impl(shared: &Arc<Mutex<Shared>>) -> PluginStatusFrame {
    status_from_state(&lock(shared))
}

/// `cad_plugin_status`: read the registry snapshot.
#[tauri::command]
pub fn cad_plugin_status(shared: tauri::State<'_, ProbeShared>) -> PluginStatusFrame {
    cad_plugin_status_impl(&shared.0)
}

/// `cad_plugin_install`: write one bundle atomically and make it the active
/// decoder for the next launch.
pub fn cad_plugin_install_impl(
    shared: &Arc<Mutex<Shared>>,
    filename: String,
    payload: String,
    meta: Option<PluginMetaArgs>,
) -> PluginMutationResult {
    let state = lock(shared);
    let outcome = (|| -> Result<(), String> {
        require_supported(&state)?;
        let store = require_store(&state)?;
        let meta = meta.unwrap_or_default();
        // The module's own id is authoritative when it exports one; a missing
        // id falls back to the file stem. A reserved or malformed id refuses
        // rather than silently installing under a different name.
        let id = match meta
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(raw) => {
                if !is_valid_plugin_id(raw) {
                    return Err(format!("{PLUGIN_INVALID_ID_REASON}: {raw}"));
                }
                if is_reserved_plugin_id(raw) {
                    return Err(format!("{PLUGIN_RESERVED_ID_REASON}: {raw}"));
                }
                raw.to_string()
            }
            None => plugin_id_for_filename(&filename)?,
        };
        let bytes = decode_plugin_payload(&payload)?;
        let name = meta
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&id)
            .to_string();
        let version = meta
            .version
            .as_deref()
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .unwrap_or("0.0.0")
            .to_string();
        let manifest = PluginManifest::synthesize(
            &id,
            &name,
            &version,
            &filename,
            format_installed_at(unix_seconds_now()),
        );
        store.write_plugin(&id, &manifest.to_json()?, &bytes)?;
        let policy = PluginPolicy {
            version: PLUGIN_POLICY_VERSION,
            active_decoder: Some(id.clone()),
        };
        if let Err(error) = store.save_policy(&policy.to_json()?) {
            // Keep the registry consistent: an installed bundle that could not
            // be activated would be a half-written install.
            let _ = store.remove_plugin(&id);
            return Err(error);
        }
        Ok(())
    })();
    let status = status_from_state(&state);
    match outcome {
        Ok(()) => PluginMutationResult::persisted(status),
        Err(reason) => PluginMutationResult::refused(reason, status),
    }
}

/// `cad_plugin_install`: install a base64 bundle.
#[tauri::command]
pub fn cad_plugin_install(
    shared: tauri::State<'_, ProbeShared>,
    filename: String,
    payload: String,
    meta: Option<PluginMetaArgs>,
) -> PluginMutationResult {
    cad_plugin_install_impl(&shared.0, filename, payload, meta)
}

/// `cad_plugin_enable`: select (`enabled:true`) or clear (`enabled:false`) the
/// active decoder.
pub fn cad_plugin_enable_impl(
    shared: &Arc<Mutex<Shared>>,
    id: String,
    enabled: bool,
) -> PluginMutationResult {
    let state = lock(shared);
    let outcome = (|| -> Result<(), String> {
        require_supported(&state)?;
        let store = require_store(&state)?;
        if id != BUILTIN_PLUGIN_ID {
            let record = store
                .scan()
                .plugins
                .into_iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("{PLUGIN_NOT_INSTALLED_REASON}: {id}"))?;
            let text = record
                .manifest
                .ok_or_else(|| format!("{PLUGIN_MANIFEST_MISSING_REASON}: {id}"))?;
            let manifest = PluginManifest::parse(&text)?;
            if manifest.id != id {
                return Err(format!(
                    "plugin.manifest_invalid: {id}: id {} does not match the directory",
                    manifest.id
                ));
            }
        }
        let mut policy = mutation_policy(store.as_ref())?;
        policy.version = PLUGIN_POLICY_VERSION;
        if enabled {
            policy.active_decoder = Some(id);
        } else if policy.active_decoder.as_deref() == Some(id.as_str()) {
            policy.active_decoder = None;
        }
        store.save_policy(&policy.to_json()?)
    })();
    let status = status_from_state(&state);
    match outcome {
        Ok(()) => PluginMutationResult::persisted(status),
        Err(reason) => PluginMutationResult::refused(reason, status),
    }
}

/// `cad_plugin_enable`: persist the active decoder choice.
#[tauri::command]
pub fn cad_plugin_enable(
    shared: tauri::State<'_, ProbeShared>,
    id: String,
    enabled: bool,
) -> PluginMutationResult {
    cad_plugin_enable_impl(&shared.0, id, enabled)
}

/// `cad_plugin_uninstall`: remove a user plugin and clear it if it was active.
pub fn cad_plugin_uninstall_impl(shared: &Arc<Mutex<Shared>>, id: String) -> PluginMutationResult {
    let state = lock(shared);
    let outcome = (|| -> Result<(), String> {
        require_supported(&state)?;
        let store = require_store(&state)?;
        if id == BUILTIN_PLUGIN_ID {
            return Err(PLUGIN_UNINSTALL_BUILTIN_REASON.to_string());
        }
        // Load first: a malformed policy must not be silently rewritten after
        // the directory is already gone.
        let mut policy = mutation_policy(store.as_ref())?;
        store.remove_plugin(&id)?;
        if policy.active_decoder.as_deref() == Some(id.as_str()) {
            policy.active_decoder = None;
            store.save_policy(&policy.to_json()?)?;
        }
        Ok(())
    })();
    let status = status_from_state(&state);
    match outcome {
        Ok(()) => PluginMutationResult::persisted(status),
        Err(reason) => PluginMutationResult::refused(reason, status),
    }
}

/// `cad_plugin_uninstall`: remove one installed plugin.
#[tauri::command]
pub fn cad_plugin_uninstall(
    shared: tauri::State<'_, ProbeShared>,
    id: String,
) -> PluginMutationResult {
    cad_plugin_uninstall_impl(&shared.0, id)
}

/// `cad_plugin_read_entry`: return the installed `plugin.js` as base64.
///
/// The built-in refuses because its bundle is served from the application's own
/// bundled URL; the frontend never needs the host to read it.
pub fn cad_plugin_read_entry_impl(
    shared: &Arc<Mutex<Shared>>,
    id: String,
) -> Result<PluginEntryFrame, String> {
    let state = lock(shared);
    require_supported(&state)?;
    if id == BUILTIN_PLUGIN_ID {
        return Err(PLUGIN_READ_ENTRY_BUILTIN_REASON.to_string());
    }
    let store = require_store(&state)?;
    let bytes = store.read_entry(&id)?;
    Ok(PluginEntryFrame::new(&bytes))
}

/// `cad_plugin_read_entry`: read one installed entry.
#[tauri::command]
pub fn cad_plugin_read_entry(
    shared: tauri::State<'_, ProbeShared>,
    id: String,
) -> Result<PluginEntryFrame, String> {
    cad_plugin_read_entry_impl(&shared.0, id)
}

/// The five plugin commands as one typed handler.
pub fn register_plugin_commands<R: tauri::Runtime>(
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        cad_plugin_status,
        cad_plugin_install,
        cad_plugin_enable,
        cad_plugin_uninstall,
        cad_plugin_read_entry
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_platform::plugins::FsPluginStore;
    use cad_ui_core::session::plugins::PluginSource;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempTree {
        root: std::path::PathBuf,
    }

    impl TempTree {
        fn new(name: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "yacr-plugin-cmd-{}-{}-{name}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create temp root");
            Self { root }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn shared_with_plugins(name: &str) -> (TempTree, Arc<Mutex<Shared>>, Arc<FsPluginStore>) {
        let tree = TempTree::new(name);
        let store = Arc::new(FsPluginStore::new(tree.root.join("plugins")));
        let shared = Arc::new(Mutex::new(Shared::new(
            cad_ui_core::launch::LaunchOptions::default(),
        )));
        configure_startup(&shared, Some(store.clone()), true);
        (tree, shared, store)
    }

    fn frame(shared: &Arc<Mutex<Shared>>) -> PluginStatusFrame {
        cad_plugin_status_impl(shared)
    }

    fn install(shared: &Arc<Mutex<Shared>>, filename: &str, source: &[u8]) -> PluginMutationResult {
        cad_plugin_install_impl(
            shared,
            filename.to_string(),
            cad_ui_core::session::plugins::encode_plugin_entry(source),
            Some(PluginMetaArgs {
                id: None,
                name: Some("Demo".to_string()),
                version: Some("1.2.3".to_string()),
            }),
        )
    }

    #[test]
    fn install_status_read_uninstall_round_trip() {
        let (tree, shared, _store) = shared_with_plugins("roundtrip");
        let first = frame(&shared);
        assert!(first.supported);
        assert_eq!(first.carrier, "native-below");
        assert_eq!(first.active_decoder.as_deref(), Some("tch"));
        assert_eq!(first.session_active_decoder.as_deref(), Some("tch"));
        assert!(!first.restart_required);
        assert_eq!(first.plugins.len(), 1);
        assert_eq!(first.plugins[0].source, PluginSource::Builtin);

        let source = b"export const abiVersion = 1;";
        let installed = install(&shared, "Demo Tools.js", source);
        assert!(installed.accepted, "{:?}", installed.reason);
        assert_eq!(
            installed.status.active_decoder.as_deref(),
            Some("demo-tools")
        );
        assert_eq!(
            installed.status.session_active_decoder.as_deref(),
            Some("tch")
        );
        assert!(installed.restart_required);
        assert_eq!(installed.status.plugins.len(), 2);
        let row = &installed.status.plugins[1];
        assert_eq!(row.id, "demo-tools");
        assert_eq!(row.source, PluginSource::User);
        assert!(row.enabled);
        assert!(!row.session_active);
        assert_eq!(row.status, PluginRowStatus::Ok);
        assert_eq!(row.version, "1.2.3");

        // Persisted immediately, applied only next launch.
        assert!(tree.root.join("plugins.json").exists());
        let persisted = cad_plugin_status_impl(&shared);
        assert_eq!(persisted.active_decoder.as_deref(), Some("demo-tools"));
        assert!(persisted.restart_required);

        let entry = cad_plugin_read_entry_impl(&shared, "demo-tools".to_string()).expect("entry");
        assert_eq!(entry.entry, "plugin.js");
        assert_eq!(
            cad_ui_core::session::plugins::decode_plugin_payload(&entry.payload).expect("base64"),
            source
        );

        let removed = cad_plugin_uninstall_impl(&shared, "demo-tools".to_string());
        assert!(removed.accepted);
        assert_eq!(removed.status.active_decoder, None);
        assert_eq!(removed.status.plugins.len(), 1);
        assert!(!tree.root.join("plugins/demo-tools").exists());
    }

    #[test]
    fn install_uses_the_module_id_and_refuses_reserved_or_taken_ids() {
        let (_tree, shared, _store) = shared_with_plugins("meta-id");
        let source = b"export const abiVersion = 1;";
        let payload = cad_ui_core::session::plugins::encode_plugin_entry(source);

        let installed = cad_plugin_install_impl(
            &shared,
            "index.js".to_string(),
            payload.clone(),
            Some(PluginMetaArgs {
                id: Some("demo".to_string()),
                name: None,
                version: None,
            }),
        );
        assert!(installed.accepted, "{:?}", installed.reason);
        assert_eq!(installed.status.active_decoder.as_deref(), Some("demo"));
        assert!(installed.status.plugins.iter().any(|row| row.id == "demo"));

        let again = cad_plugin_install_impl(
            &shared,
            "other.js".to_string(),
            payload.clone(),
            Some(PluginMetaArgs {
                id: Some("demo".to_string()),
                name: None,
                version: None,
            }),
        );
        assert!(!again.accepted);
        assert!(again
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("plugin.already_installed"));

        let reserved = cad_plugin_install_impl(
            &shared,
            "tch.js".to_string(),
            payload,
            Some(PluginMetaArgs {
                id: Some("tch".to_string()),
                name: None,
                version: None,
            }),
        );
        assert!(!reserved.accepted);
        assert!(reserved
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("plugin.reserved_id"));
    }

    #[test]
    fn enable_and_disable_persist_radio_selection() {
        let (_tree, shared, _store) = shared_with_plugins("enable");
        assert!(install(&shared, "demo.js", b"demo").accepted);

        let disabled = cad_plugin_enable_impl(&shared, "demo".to_string(), false);
        assert!(disabled.accepted);
        assert_eq!(disabled.status.active_decoder, None);
        assert!(disabled.restart_required);

        let enabled = cad_plugin_enable_impl(&shared, "demo".to_string(), true);
        assert!(enabled.accepted);
        assert_eq!(enabled.status.active_decoder.as_deref(), Some("demo"));

        // Disabling a non-active decoder leaves the radio selection alone.
        assert!(install(&shared, "other.js", b"other").accepted);
        let noop = cad_plugin_enable_impl(&shared, "demo".to_string(), false);
        assert!(noop.accepted);
        assert_eq!(noop.status.active_decoder.as_deref(), Some("other"));

        let builtin = cad_plugin_enable_impl(&shared, "tch".to_string(), true);
        assert!(builtin.accepted);
        assert_eq!(builtin.status.active_decoder.as_deref(), Some("tch"));
        let all_off = cad_plugin_enable_impl(&shared, "tch".to_string(), false);
        assert!(all_off.accepted);
        assert_eq!(all_off.status.active_decoder, None);
    }

    #[test]
    fn builtin_and_invalid_install_refusals_are_wire_results() {
        let (_tree, shared, _store) = shared_with_plugins("refusals");

        let reserved = install(&shared, "tch.js", b"x");
        assert!(!reserved.accepted);
        assert!(reserved
            .reason
            .as_deref()
            .unwrap_or_default()
            .starts_with(cad_platform::plugins::PLUGIN_RESERVED_ID_REASON));

        let wrong_extension = install(&shared, "demo.txt", b"x");
        assert!(!wrong_extension.accepted);
        assert!(wrong_extension.reason.is_some());

        let read_builtin =
            cad_plugin_read_entry_impl(&shared, "tch".to_string()).expect_err("builtin refuses");
        assert_eq!(read_builtin, PLUGIN_READ_ENTRY_BUILTIN_REASON);

        let uninstall_builtin = cad_plugin_uninstall_impl(&shared, "tch".to_string());
        assert!(!uninstall_builtin.accepted);
        assert_eq!(
            uninstall_builtin.reason.as_deref(),
            Some(PLUGIN_UNINSTALL_BUILTIN_REASON)
        );

        let unknown = cad_plugin_uninstall_impl(&shared, "missing".to_string());
        assert!(!unknown.accepted);
        assert!(unknown
            .reason
            .as_deref()
            .unwrap_or_default()
            .starts_with(PLUGIN_NOT_INSTALLED_REASON));
    }

    #[test]
    fn dom_canvas_reports_unsupported_and_mutations_refuse() {
        let tree = TempTree::new("domcanvas");
        let store = Arc::new(FsPluginStore::new(tree.root.join("plugins")));
        let shared = Arc::new(Mutex::new(Shared::new(
            cad_ui_core::launch::LaunchOptions::default(),
        )));
        configure_startup(&shared, Some(store), false);

        let status = frame(&shared);
        assert!(!status.supported);
        assert_eq!(status.carrier, "dom-canvas");
        assert_eq!(
            status.diagnostics,
            vec![PLUGIN_UNSUPPORTED_CARRIER_REASON.to_string()]
        );
        assert_eq!(status.active_decoder.as_deref(), Some("tch"));
        assert_eq!(status.plugins.len(), 1);
        assert_eq!(status.plugins[0].source, PluginSource::Builtin);

        let install = install(&shared, "demo.js", b"x");
        assert!(!install.accepted);
        assert_eq!(
            install.reason.as_deref(),
            Some(PLUGIN_UNSUPPORTED_CARRIER_REASON)
        );
        assert!(!install.status.supported);
        let enable = cad_plugin_enable_impl(&shared, "tch".to_string(), false);
        assert!(!enable.accepted);
        assert_eq!(
            enable.reason.as_deref(),
            Some(PLUGIN_UNSUPPORTED_CARRIER_REASON)
        );
        let uninstall = cad_plugin_uninstall_impl(&shared, "demo".to_string());
        assert!(!uninstall.accepted);
        assert_eq!(
            uninstall.reason.as_deref(),
            Some(PLUGIN_UNSUPPORTED_CARRIER_REASON)
        );
        assert!(cad_plugin_read_entry_impl(&shared, "demo".to_string()).is_err());
    }

    #[test]
    fn restart_required_clears_only_when_the_launch_matches_the_policy() {
        let (_tree, shared, _store) = shared_with_plugins("restart");
        assert!(install(&shared, "demo.js", b"demo").accepted);
        assert!(frame(&shared).restart_required);
        // A new launch reads the same policy: now the configured and session
        // decoders agree.
        let policy = _store.load_policy().expect("policy").expect("exists");
        assert!(policy.contains("\"activeDecoder\":\"demo\""));
        let mut state = lock(&shared);
        state.plugins_session_active = Some("demo".to_string());
        drop(state);
        assert!(!frame(&shared).restart_required);
    }

    #[test]
    fn only_the_five_names_are_plugin_commands() {
        for command in [
            CAD_PLUGIN_STATUS_COMMAND,
            CAD_PLUGIN_INSTALL_COMMAND,
            CAD_PLUGIN_ENABLE_COMMAND,
            CAD_PLUGIN_UNINSTALL_COMMAND,
            CAD_PLUGIN_READ_ENTRY_COMMAND,
        ] {
            assert!(is_plugin_command(command));
        }
        for command in ["cad_vendor_apply", "cad_status", "", "cad_plugin"] {
            assert!(!is_plugin_command(command));
        }
    }
}
