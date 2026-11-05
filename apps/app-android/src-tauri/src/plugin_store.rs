//! Durable plugin storage that resolves Android `filesDir` lazily.
//!
//! The Kotlin asset handshake (`nativeAssetsReady`) may land **after** Tauri
//! `setup`, so a fixed root captured at startup would silently disable plugin
//! management on a real device. Every call resolves the current `filesDir`
//! instead and delegates to the shared [`FsPluginStore`]; before the handshake
//! the honest answer is the stable store-unavailable refusal, never an invented
//! path.

use std::path::PathBuf;

use cad_platform::plugins::{
    FsPluginStore, PluginScan, PluginStore, PLUGIN_DIRECTORY, PLUGIN_STORE_UNAVAILABLE_REASON,
};

use crate::platform::handshake_files_dir;

#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidPluginStore;

impl AndroidPluginStore {
    fn root() -> Option<PathBuf> {
        handshake_files_dir().map(|dir| dir.join(PLUGIN_DIRECTORY))
    }

    fn with_store<T>(&self, operation: impl FnOnce(&FsPluginStore) -> T) -> Result<T, String> {
        match Self::root() {
            Some(root) => Ok(operation(&FsPluginStore::new(root))),
            None => Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string()),
        }
    }
}

impl PluginStore for AndroidPluginStore {
    fn load_policy(&self) -> Result<Option<String>, String> {
        self.with_store(|store| store.load_policy())?
    }

    fn save_policy(&self, policy_json: &str) -> Result<(), String> {
        self.with_store(|store| store.save_policy(policy_json))?
    }

    fn scan(&self) -> PluginScan {
        match Self::root() {
            Some(root) => FsPluginStore::new(root).scan(),
            None => PluginScan {
                plugins: Vec::new(),
                diagnostics: vec![PLUGIN_STORE_UNAVAILABLE_REASON.to_string()],
            },
        }
    }

    fn read_entry(&self, id: &str) -> Result<Vec<u8>, String> {
        self.with_store(|store| store.read_entry(id))?
    }

    fn write_plugin(&self, id: &str, manifest_json: &str, entry: &[u8]) -> Result<(), String> {
        self.with_store(|store| store.write_plugin(id, manifest_json, entry))?
    }

    fn remove_plugin(&self, id: &str) -> Result<(), String> {
        self.with_store(|store| store.remove_plugin(id))?
    }
}
