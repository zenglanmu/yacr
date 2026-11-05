//! Plugin storage contract and the filesystem implementation.
//!
//! Phase 1 has **no hot reload and no live unload**: a plugin is installed,
//! enabled/disabled or removed on disk, and the change takes effect on the next
//! application launch. This module only owns the durable bytes:
//!
//! * `<config-dir>/plugins/<id>/plugin.js` — the decoder bundle;
//! * `<config-dir>/plugins/<id>/manifest.json` — the synthesized manifest;
//! * `<config-dir>/plugins.json` — the one active decoder (`activeDecoder`).
//!
//! The wire model (policy/manifest types, validation, status frames) lives in
//! `cad_ui_core::session::plugins`, which sits above this crate. To avoid a
//! dependency cycle this trait deliberately transports **raw JSON text** and
//! **raw entry bytes**: parsing/validation belongs to the shared UI layer, while
//! path safety stays here because it is the layer that touches the filesystem.
//!
//! Every refusal is a stable reason. A missing directory is not an error
//! ([`PluginStore::scan`] reports it as an empty list); an unreadable one is a
//! diagnostic, never a silent empty success. Symlinked plugin directories or
//! entry files are refused instead of followed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Upper bound of one plugin entry bundle (`plugin.js` / `.mjs`).
pub const MAX_PLUGIN_BYTES: usize = 4 * 1024 * 1024;

/// Maximum plugin id length (bytes).
pub const MAX_PLUGIN_ID_BYTES: usize = 64;

/// Entry file name inside a plugin directory.
pub const PLUGIN_ENTRY_FILE: &str = "plugin.js";

/// Synthesized manifest file name inside a plugin directory.
pub const PLUGIN_MANIFEST_FILE: &str = "manifest.json";

/// Policy file name next to the `plugins` directory.
pub const PLUGIN_POLICY_FILE: &str = "plugins.json";

/// Plugin directory name next to the config file.
pub const PLUGIN_DIRECTORY: &str = "plugins";

// ---------------------------------------------------------------------------
// Stable refusal vocabulary (shared with `cad_ui_core::session::plugins`).
// ---------------------------------------------------------------------------

/// The carrier has no plugin manager (only `dom-canvas` today).
pub const PLUGIN_UNSUPPORTED_CARRIER_REASON: &str = "plugin.unsupported_carrier: dom-canvas";
/// No durable store exists on this host.
pub const PLUGIN_STORE_UNAVAILABLE_REASON: &str = "unsupported-runtime: plugin-store-unavailable";
/// The id does not match `[a-z0-9][a-z0-9_-]{0,63}`.
pub const PLUGIN_INVALID_ID_REASON: &str = "plugin.invalid_id";
/// `tch` is the bundled built-in plugin id.
pub const PLUGIN_RESERVED_ID_REASON: &str = "plugin.reserved_id";
/// A filename is not a basename with a `.js`/`.mjs` extension.
pub const PLUGIN_INVALID_FILENAME_REASON: &str = "plugin.invalid_filename";
/// The payload is not valid base64.
pub const PLUGIN_INVALID_BASE64_REASON: &str = "plugin.invalid_base64";
/// The entry exceeds [`MAX_PLUGIN_BYTES`].
pub const PLUGIN_PAYLOAD_TOO_LARGE_REASON: &str = "plugin.payload_too_large";
/// A directory for the id already exists.
pub const PLUGIN_ALREADY_INSTALLED_REASON: &str = "plugin.already_installed";
/// No installed plugin has that id.
pub const PLUGIN_NOT_INSTALLED_REASON: &str = "plugin.not_installed";
/// The built-in plugin cannot be uninstalled.
pub const PLUGIN_UNINSTALL_BUILTIN_REASON: &str = "plugin.uninstall_builtin";
/// The built-in plugin's entry is served by its bundled URL.
pub const PLUGIN_READ_ENTRY_BUILTIN_REASON: &str = "plugin.read_entry_builtin";
/// The policy file exists but is not a valid `version: 1` policy.
pub const PLUGIN_POLICY_INVALID_REASON: &str = "plugin.policy_invalid";
/// An installed manifest cannot be parsed or is inconsistent.
pub const PLUGIN_MANIFEST_INVALID_REASON: &str = "plugin.manifest_invalid";
/// A symlink or otherwise unsafe path was found under the plugin root.
pub const PLUGIN_UNSAFE_PATH_REASON: &str = "plugin.unsafe_path";
/// The plugin directory could not be scanned.
pub const PLUGIN_SCAN_FAILED_REASON: &str = "plugin.scan_failed";
/// The entry file is missing.
pub const PLUGIN_ENTRY_MISSING_REASON: &str = "plugin.entry_missing";
/// The manifest file is missing.
pub const PLUGIN_MANIFEST_MISSING_REASON: &str = "plugin.manifest_missing";
/// A filesystem operation failed.
pub const PLUGIN_IO_REASON: &str = "plugin.io";

/// Whether `id` is a filesystem-safe plugin id.
///
/// This is the one canonical id grammar: the shared UI layer re-exports it as
/// `is_valid_plugin_id`, and every path below is derived from an id that passed
/// this check. The rule is intentionally a plain byte scan (no regex crate).
pub fn is_safe_plugin_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PLUGIN_ID_BYTES {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes[1..].iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
    })
}

/// One installed plugin directory, with the manifest read verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginRecord {
    /// Directory name (the plugin id).
    pub id: String,
    /// `manifest.json` text; `None` when the file is missing (the directory is
    /// still reported so the status can show a `manifest_missing` row).
    pub manifest: Option<String>,
}

/// Result of scanning the plugin root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginScan {
    /// Installed plugin directories, sorted by id.
    pub plugins: Vec<InstalledPluginRecord>,
    /// Stable diagnostics for entries that could not be used.
    pub diagnostics: Vec<String>,
}

/// Durable plugin storage used by the plugin manager commands.
pub trait PluginStore: Send + Sync {
    /// Read `<config-dir>/plugins.json`.
    ///
    /// `Ok(None)` means the file is absent (first run, built-in default). An
    /// unreadable file is `Err`, not an empty policy.
    fn load_policy(&self) -> Result<Option<String>, String>;

    /// Persist `<config-dir>/plugins.json` atomically.
    fn save_policy(&self, policy_json: &str) -> Result<(), String>;

    /// List installed plugin directories and their manifest text.
    ///
    /// A missing root is an empty scan. Per-entry failures are diagnostics, so a
    /// corrupt plugin cannot hide the healthy ones.
    fn scan(&self) -> PluginScan;

    /// Read `<root>/<id>/plugin.js` bytes.
    fn read_entry(&self, id: &str) -> Result<Vec<u8>, String>;

    /// Write a plugin directory atomically (temp directory + rename).
    fn write_plugin(&self, id: &str, manifest_json: &str, entry: &[u8]) -> Result<(), String>;

    /// Remove one plugin directory.
    fn remove_plugin(&self, id: &str) -> Result<(), String>;
}

/// A [`PluginStore`] with no durable location.
///
/// Every mutation and read refuses with the stable
/// [`PLUGIN_STORE_UNAVAILABLE_REASON`]; [`PluginStore::scan`] answers empty with
/// the same reason as a diagnostic. A host without a config directory therefore
/// cannot pretend a plugin was installed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NonPersistentPluginStore;

impl PluginStore for NonPersistentPluginStore {
    fn load_policy(&self) -> Result<Option<String>, String> {
        Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
    }

    fn save_policy(&self, _policy_json: &str) -> Result<(), String> {
        Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
    }

    fn scan(&self) -> PluginScan {
        PluginScan {
            plugins: Vec::new(),
            diagnostics: vec![PLUGIN_STORE_UNAVAILABLE_REASON.to_string()],
        }
    }

    fn read_entry(&self, _id: &str) -> Result<Vec<u8>, String> {
        Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
    }

    fn write_plugin(&self, _id: &str, _manifest_json: &str, _entry: &[u8]) -> Result<(), String> {
        Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
    }

    fn remove_plugin(&self, _id: &str) -> Result<(), String> {
        Err(PLUGIN_STORE_UNAVAILABLE_REASON.to_string())
    }
}

/// Filesystem plugin store rooted at `<config-dir>/plugins`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPluginStore {
    root: PathBuf,
}

impl FsPluginStore {
    /// A store over `root` (the `plugins` directory itself).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The plugin directory root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<parent-of-root>/plugins.json`.
    pub fn policy_path(&self) -> PathBuf {
        let parent = match self.root.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        parent.join(PLUGIN_POLICY_FILE)
    }

    fn plugin_dir(&self, id: &str) -> Result<PathBuf, String> {
        if !is_safe_plugin_id(id) {
            return Err(PLUGIN_INVALID_ID_REASON.to_string());
        }
        Ok(self.root.join(id))
    }

    fn refuse_symlink(path: &Path) -> Result<(), String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
                "{PLUGIN_UNSAFE_PATH_REASON}: {} is a symlink",
                path.display()
            )),
            Ok(_) => Ok(()),
            Err(error) => Err(format!("{PLUGIN_IO_REASON}: {}: {error}", path.display())),
        }
    }
}

fn io_reason(path: &Path, error: io::Error) -> String {
    format!("{PLUGIN_IO_REASON}: {}: {error}", path.display())
}

impl PluginStore for FsPluginStore {
    fn load_policy(&self) -> Result<Option<String>, String> {
        let path = self.policy_path();
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_reason(&path, error)),
        }
    }

    fn save_policy(&self, policy_json: &str) -> Result<(), String> {
        let path = self.policy_path();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|error| io_reason(parent, error))?;
            }
        }
        atomic_write(&path, policy_json.as_bytes())
    }

    fn scan(&self) -> PluginScan {
        let mut scan = PluginScan::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return scan,
            Err(error) => {
                scan.diagnostics.push(format!(
                    "{PLUGIN_SCAN_FAILED_REASON}: {}: {error}",
                    self.root.display()
                ));
                return scan;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan.diagnostics.push(format!(
                        "{PLUGIN_SCAN_FAILED_REASON}: {}: {error}",
                        self.root.display()
                    ));
                    continue;
                }
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                scan.diagnostics.push(format!(
                    "{PLUGIN_SCAN_FAILED_REASON}: non-UTF-8 entry under {}",
                    self.root.display()
                ));
                continue;
            };
            // Internal temp directories are not plugins.
            if name.starts_with('.') {
                continue;
            }
            if !is_safe_plugin_id(name) {
                scan.diagnostics
                    .push(format!("{PLUGIN_INVALID_ID_REASON}: {name}"));
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    let path = entry.path();
                    scan.diagnostics.push(io_reason(&path, error));
                    continue;
                }
            };
            if file_type.is_symlink() {
                scan.diagnostics.push(format!(
                    "{PLUGIN_UNSAFE_PATH_REASON}: {} is a symlink",
                    entry.path().display()
                ));
                continue;
            }
            if !file_type.is_dir() {
                scan.diagnostics.push(format!(
                    "{PLUGIN_SCAN_FAILED_REASON}: {} is not a directory",
                    entry.path().display()
                ));
                continue;
            }
            let manifest_path = entry.path().join(PLUGIN_MANIFEST_FILE);
            match fs::symlink_metadata(&manifest_path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    scan.diagnostics.push(format!(
                        "{PLUGIN_UNSAFE_PATH_REASON}: {} is a symlink",
                        manifest_path.display()
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    scan.diagnostics
                        .push(format!("{PLUGIN_MANIFEST_MISSING_REASON}: {name}"));
                    scan.plugins.push(InstalledPluginRecord {
                        id: name.to_string(),
                        manifest: None,
                    });
                    continue;
                }
                Err(error) => {
                    scan.diagnostics.push(io_reason(&manifest_path, error));
                    continue;
                }
            }
            match fs::read_to_string(&manifest_path) {
                Ok(manifest) => scan.plugins.push(InstalledPluginRecord {
                    id: name.to_string(),
                    manifest: Some(manifest),
                }),
                Err(error) => {
                    scan.diagnostics.push(io_reason(&manifest_path, error));
                    scan.plugins.push(InstalledPluginRecord {
                        id: name.to_string(),
                        manifest: None,
                    });
                }
            }
        }
        scan.plugins.sort_by(|left, right| left.id.cmp(&right.id));
        scan
    }

    fn read_entry(&self, id: &str) -> Result<Vec<u8>, String> {
        let dir = self.plugin_dir(id)?;
        Self::refuse_symlink(&dir)?;
        let path = dir.join(PLUGIN_ENTRY_FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "{PLUGIN_UNSAFE_PATH_REASON}: {} is a symlink",
                    path.display()
                ));
            }
            Ok(metadata) if metadata.len() > MAX_PLUGIN_BYTES as u64 => {
                return Err(PLUGIN_PAYLOAD_TOO_LARGE_REASON.to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(format!("{PLUGIN_ENTRY_MISSING_REASON}: {id}"));
            }
            Err(error) => return Err(io_reason(&path, error)),
        }
        let bytes = fs::read(&path).map_err(|error| io_reason(&path, error))?;
        if bytes.len() > MAX_PLUGIN_BYTES {
            return Err(PLUGIN_PAYLOAD_TOO_LARGE_REASON.to_string());
        }
        Ok(bytes)
    }

    fn write_plugin(&self, id: &str, manifest_json: &str, entry: &[u8]) -> Result<(), String> {
        let dir = self.plugin_dir(id)?;
        if entry.len() > MAX_PLUGIN_BYTES {
            return Err(PLUGIN_PAYLOAD_TOO_LARGE_REASON.to_string());
        }
        fs::create_dir_all(&self.root).map_err(|error| io_reason(&self.root, error))?;
        if fs::symlink_metadata(&dir).is_ok() {
            return Err(format!("{PLUGIN_ALREADY_INSTALLED_REASON}: {id}"));
        }
        // Same filesystem as the destination so the rename is atomic.
        let temp = self.root.join(format!(
            ".tmp-{id}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&temp).map_err(|error| io_reason(&temp, error))?;
        let write = (|| -> Result<(), String> {
            let entry_path = temp.join(PLUGIN_ENTRY_FILE);
            let mut file =
                fs::File::create(&entry_path).map_err(|error| io_reason(&entry_path, error))?;
            use std::io::Write;
            file.write_all(entry)
                .map_err(|error| io_reason(&entry_path, error))?;
            file.sync_all()
                .map_err(|error| io_reason(&entry_path, error))?;
            let manifest_path = temp.join(PLUGIN_MANIFEST_FILE);
            fs::write(&manifest_path, manifest_json)
                .map_err(|error| io_reason(&manifest_path, error))?;
            fs::rename(&temp, &dir).map_err(|error| io_reason(&dir, error))
        })();
        if write.is_err() {
            let _ = fs::remove_dir_all(&temp);
        }
        write
    }

    fn remove_plugin(&self, id: &str) -> Result<(), String> {
        let dir = self.plugin_dir(id)?;
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
                "{PLUGIN_UNSAFE_PATH_REASON}: {} is a symlink",
                dir.display()
            )),
            Ok(metadata) if metadata.is_dir() => {
                fs::remove_dir_all(&dir).map_err(|error| io_reason(&dir, error))
            }
            Ok(_) => Err(format!("{PLUGIN_NOT_INSTALLED_REASON}: {id}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(format!("{PLUGIN_NOT_INSTALLED_REASON}: {id}"))
            }
            Err(error) => Err(io_reason(&dir, error)),
        }
    }
}

/// Monotonic suffix so two concurrent writes never share a temp path.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Replace `path` atomically (temp file in the same directory + rename).
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_reason(parent, error))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("plugins.json"),
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let write = (|| -> Result<(), String> {
        let mut file = fs::File::create(&temp).map_err(|error| io_reason(&temp, error))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|error| io_reason(&temp, error))?;
        file.sync_all().map_err(|error| io_reason(&temp, error))?;
        fs::rename(&temp, path).map_err(|error| io_reason(path, error))
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "yacr-plugin-store-{}-{}-{name}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("create temp root");
            Self { root }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn store(tree: &TempTree) -> FsPluginStore {
        FsPluginStore::new(tree.root.join(PLUGIN_DIRECTORY))
    }

    #[test]
    fn safe_id_grammar_is_exact() {
        assert!(is_safe_plugin_id("tch"));
        assert!(is_safe_plugin_id("a"));
        assert!(is_safe_plugin_id("plugin_1-x"));
        assert!(!is_safe_plugin_id(""));
        assert!(!is_safe_plugin_id("_plugin"));
        assert!(!is_safe_plugin_id("-plugin"));
        assert!(!is_safe_plugin_id("Plugin"));
        assert!(!is_safe_plugin_id("plugin/evil"));
        assert!(!is_safe_plugin_id(".."));
        assert!(!is_safe_plugin_id(&"a".repeat(MAX_PLUGIN_ID_BYTES + 1)));
    }

    #[test]
    fn write_scan_read_remove_round_trip_is_atomic() {
        let tree = TempTree::new("roundtrip");
        let store = store(&tree);
        store
            .write_plugin("demo", r#"{"id":"demo"}"#, b"export const x = 1;")
            .expect("writes plugin");
        // The temp directory never survives a successful rename.
        let names: Vec<String> = fs::read_dir(tree.root.join(PLUGIN_DIRECTORY))
            .expect("root")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["demo".to_string()]);

        let scan = store.scan();
        assert_eq!(scan.plugins.len(), 1);
        assert_eq!(scan.plugins[0].id, "demo");
        assert_eq!(
            scan.plugins[0].manifest.as_deref(),
            Some(r#"{"id":"demo"}"#)
        );
        assert!(scan.diagnostics.is_empty(), "{:?}", scan.diagnostics);
        assert_eq!(
            store.read_entry("demo").expect("entry"),
            b"export const x = 1;"
        );

        // A second write cannot replace an existing install.
        let duplicate = store
            .write_plugin("demo", "{}", b"x")
            .expect_err("duplicate must refuse");
        assert!(duplicate.starts_with(PLUGIN_ALREADY_INSTALLED_REASON));

        store.remove_plugin("demo").expect("removes");
        assert!(store.scan().plugins.is_empty());
        assert!(store
            .remove_plugin("demo")
            .expect_err("second remove refuses")
            .starts_with(PLUGIN_NOT_INSTALLED_REASON));
    }

    #[test]
    fn ids_are_contained_and_traversal_is_refused() {
        let tree = TempTree::new("containment");
        let store = store(&tree);
        for id in ["../escape", "a/b", "..", "", "TCH"] {
            let error = store.write_plugin(id, "{}", b"x").expect_err("refuses");
            assert!(
                error.starts_with(PLUGIN_INVALID_ID_REASON) || error.starts_with(PLUGIN_IO_REASON),
                "{id}: {error}"
            );
            assert!(!tree.root.join("escape").exists());
        }
        assert!(store
            .read_entry("../escape")
            .expect_err("refuses")
            .starts_with(PLUGIN_INVALID_ID_REASON));
        assert!(store
            .remove_plugin("../escape")
            .expect_err("refuses")
            .starts_with(PLUGIN_INVALID_ID_REASON));
    }

    #[test]
    fn policy_round_trips_and_absent_is_not_an_error() {
        let tree = TempTree::new("policy");
        let store = store(&tree);
        assert_eq!(store.load_policy().expect("absent"), None);
        store
            .save_policy(r#"{"version":1,"activeDecoder":"tch"}"#)
            .expect("persists");
        assert_eq!(
            store.load_policy().expect("reads"),
            Some(r#"{"version":1,"activeDecoder":"tch"}"#.to_string())
        );
        assert!(store.policy_path().ends_with(PLUGIN_POLICY_FILE));
    }

    #[test]
    fn oversized_entry_is_refused_on_read_and_write() {
        let tree = TempTree::new("oversized");
        let store = store(&tree);
        let oversized = vec![0u8; MAX_PLUGIN_BYTES + 1];
        assert!(store
            .write_plugin("big", "{}", &oversized)
            .expect_err("write refuses")
            .starts_with(PLUGIN_PAYLOAD_TOO_LARGE_REASON));

        store.write_plugin("small", "{}", b"tiny").expect("writes");
        let path = tree
            .root
            .join(PLUGIN_DIRECTORY)
            .join("small")
            .join(PLUGIN_ENTRY_FILE);
        fs::write(&path, &oversized).expect("replace entry");
        assert!(store
            .read_entry("small")
            .expect_err("read refuses")
            .starts_with(PLUGIN_PAYLOAD_TOO_LARGE_REASON));
    }

    #[test]
    fn non_persistent_store_refuses_without_pretending() {
        let store = NonPersistentPluginStore;
        assert!(store
            .save_policy("{}")
            .expect_err("refuses")
            .starts_with("unsupported-runtime:"));
        assert!(store
            .write_plugin("demo", "{}", b"x")
            .expect_err("refuses")
            .starts_with("unsupported-runtime:"));
        let scan = store.scan();
        assert!(scan.plugins.is_empty());
        assert_eq!(scan.diagnostics, vec![PLUGIN_STORE_UNAVAILABLE_REASON]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_plugin_entries_and_directories_are_refused() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new("symlink");
        let store = store(&tree);
        let root = tree.root.join(PLUGIN_DIRECTORY);
        fs::create_dir_all(&root).expect("root");

        // A symlinked plugin directory pointing outside the root is reported and
        // never followed.
        let outside = tree.root.join("outside");
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join(PLUGIN_ENTRY_FILE), b"outside").expect("outside entry");
        fs::write(outside.join(PLUGIN_MANIFEST_FILE), "{}").expect("outside manifest");
        symlink(&outside, root.join("linked")).expect("symlink dir");

        let scan = store.scan();
        assert!(scan.plugins.is_empty());
        assert!(
            scan.diagnostics
                .iter()
                .any(|line| line.starts_with(PLUGIN_UNSAFE_PATH_REASON)),
            "{:?}",
            scan.diagnostics
        );
        assert!(store
            .read_entry("linked")
            .expect_err("refuses symlink dir")
            .starts_with(PLUGIN_UNSAFE_PATH_REASON));
        assert!(store
            .remove_plugin("linked")
            .expect_err("refuses symlink dir")
            .starts_with(PLUGIN_UNSAFE_PATH_REASON));
        assert!(outside.exists(), "the symlink target must survive");

        // A real directory with a symlinked `plugin.js` is refused too.
        store.write_plugin("real", "{}", b"real").expect("writes");
        let entry = root.join("real").join(PLUGIN_ENTRY_FILE);
        fs::remove_file(&entry).expect("remove entry");
        symlink(outside.join(PLUGIN_ENTRY_FILE), &entry).expect("symlink entry");
        assert!(store
            .read_entry("real")
            .expect_err("refuses symlink entry")
            .starts_with(PLUGIN_UNSAFE_PATH_REASON));
    }
}
