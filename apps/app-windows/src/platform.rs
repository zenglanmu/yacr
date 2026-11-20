//! Windows host platform contracts.
//!
//! Concrete `cad-platform` implementations for the Windows desktop host,
//! decoupled from the Tauri/WebView2 wiring:
//!
//! * [`ReactFiles`] — [`PlatformFiles`]: native open **and** save-target choosers
//!   via COM `IFileOpenDialog`/`IFileSaveDialog` on a worker thread
//!   (`platform/native.rs`), the `--open`/fixture target read, and the real
//!   atomic save write. It never fabricates a success: a cancelled chooser is a
//!   [`FileEvent::Cancelled`], a missing/unreadable file or a failed dialog is an
//!   explicit [`FileEvent::Failed`], and open/save share one single-flight
//!   admission (bounded, capacity-1 terminal). The save write is temp file →
//!   `sync_all` → `.bak` of the previous file → replace → best-effort
//!   directory fsync, so a partial file is never left in the target's place.
//! * [`ReactSurface`] — [`SurfaceSource`]: the CAD content rectangle and scale
//!   reported by the frontend DOM, shared through an `Rc<Cell<..>>`.
//! * [`ReactConfigStore`] — [`ConfigStore`]: std-fs config/preference loading
//!   (`Absent` vs `Unreadable` distinguished) and atomic preference persistence
//!   under `%APPDATA%\yacr`, or an explicit `Unsupported` when no durable path is
//!   configured.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`), selected by locale through
//! `cad_ui_core::i18n` — no host-side hardcoded UI text. Windows owns its own
//! `windows.picker_*` keys so a future platform label change cannot leak across
//! hosts; the save titles/filters reuse the shipped shared wording until
//! `windows.save_*` entries exist (see `platform/save.rs`).
//!
//! The module is target-independent: everything except the COM choosers is
//! plain std-fs/trait code, so the save/backup/config contracts are unit-tested
//! on any host. Only `platform/native.rs` needs the real Windows target to
//! type-check.
//!
//! Resource resolution (`cad_host::DesktopResources`) and the `HostPlatform`
//! composition are built by the caller; this module only supplies the contracts.

use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_core::i18n::MessageSource;

#[cfg(target_os = "windows")]
#[path = "platform/native.rs"]
pub mod native;

#[path = "platform/save.rs"]
mod save;

pub use save::{
    save_default_extension, save_filter_spec, SaveChooser, WindowsSaveDialogLabels,
    EXPORT_FILTER_SPEC, PLOT_FILTER_SPEC, YMODEL_FILTER_SPEC,
};

#[cfg(test)]
#[path = "platform/tests.rs"]
mod tests;

const KEY_PICKER_TITLE: &str = "windows.picker_title";
const KEY_PICKER_FILTER: &str = "windows.picker_filter";
const KEY_PICKER_EMPTY: &str = "windows.picker_empty";
const KEY_PICKER_LOCAL: &str = "windows.picker_local";
const KEY_PICKER_FAILED: &str = "windows.picker_failed";

/// The extensions the open dialog accepts, in display order.
///
/// `.rfa`/`.rft` are listed **on purpose**: the RVT adapter refuses a family file
/// with a precise `import.rvt.family_no_geometry` diagnostic, which is more
/// useful than the file being unselectable. They are deliberately *not* in any
/// "supported formats" copy (see `README.md` / the i18n filter labels).
/// `.skp` is the SketchUp preview lane (triangles only, never a measurement
/// source; an edge-only model is refused by name, not by the picker).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const OPEN_EXTENSIONS: &[&str] = &[
    "ymodel", "dwg", "dxf", "obj", "stl", "ply", "off", "coff", "gltf", "glb", "step", "stp",
    "ifc", "rvt", "rte", "rfa", "rft", "skp", "dgn",
];

/// The COM `COMDLG_FILTERSPEC` pattern (`"*.dwg;*.dxf;…"`).
///
/// The shell matches this case-insensitively, so no per-character bracketing is
/// needed (unlike the XDG portal glob in the Linux host).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn open_filter_spec() -> String {
    OPEN_EXTENSIONS
        .iter()
        .map(|extension| format!("*.{extension}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// The localized labels the native open dialog and open funnel use.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsOpenDialogLabels {
    pub picker_title: String,
    pub picker_filter: String,
    pub picker_empty: String,
    pub picker_local: String,
    /// Template containing a `{error}` placeholder.
    pub picker_failed: String,
}

impl WindowsOpenDialogLabels {
    /// Labels for `locale`, resolved through the shared catalog. An unknown
    /// locale falls back to English (with a recorded reason) rather than showing
    /// a wrong language.
    pub fn from_locale(locale: &str) -> CadResult<Self> {
        let source = MessageSource::from_request(locale);
        let get = |key: &str| {
            source
                .catalog()
                .get(key)
                .map(str::to_string)
                .ok_or_else(|| CadError::InvalidInput(format!("i18n key missing: {key}")))
        };
        Ok(Self {
            picker_title: get(KEY_PICKER_TITLE)?,
            picker_filter: get(KEY_PICKER_FILTER)?,
            picker_empty: get(KEY_PICKER_EMPTY)?,
            picker_local: get(KEY_PICKER_LOCAL)?,
            picker_failed: get(KEY_PICKER_FAILED)?,
        })
    }

    /// Fill the `{error}` placeholder in the picker-failure template.
    pub fn picker_failed(&self, error: &str) -> String {
        self.picker_failed.replace("{error}", error)
    }
}

/// The outcome of a (possibly injected) file chooser.
///
/// Cancellation is distinct from failure so the funnel can grade them
/// differently; neither is ever a fabricated success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChooserOutcome {
    Path(PathBuf),
    Cancelled,
    Failed(String),
}

/// A dependency-injected chooser, `Send + Sync` so it can run on the worker.
pub type FileChooser =
    Arc<dyn Fn(&WindowsOpenDialogLabels, Option<&Path>) -> FileChooserOutcome + Send + Sync>;

/// The single-flight open state, guarded by one mutex so admission and the
/// terminal hand-off cannot interleave.
///
/// `busy` covers the whole lifetime of a request: it is set when a request is
/// admitted and cleared **only** by [`ReactFiles::poll`] when it takes the
/// accepted terminal event. A worker publishes the result but never touches
/// `busy`, so a stale worker cannot unlock a newer request.
struct OpenState {
    /// True from admission until the accepted terminal event is consumed.
    busy: bool,
    /// The single accepted terminal event awaiting delivery (capacity 1).
    terminal: Option<FileEvent>,
}

impl OpenState {
    fn new() -> Self {
        Self {
            busy: false,
            terminal: None,
        }
    }
}

/// React host file open/read implementation of [`PlatformFiles`].
///
/// Single-flight: at most one open/read worker runs at a time, and a request
/// stays busy until its accepted terminal event has been drained by
/// [`PlatformFiles::poll`] on the main thread. Results are handed over through a
/// single terminal slot (capacity 1) rather than an unbounded queue; the trait
/// itself carries no `Send` requirement.
pub struct ReactFiles {
    labels: Arc<WindowsOpenDialogLabels>,
    save_labels: Arc<WindowsSaveDialogLabels>,
    chooser: FileChooser,
    /// `None` means "no save chooser is configured": an admitted save still
    /// publishes an explicit [`FileEvent::Failed`], never a fabricated target.
    save_chooser: Option<SaveChooser>,
    state: Arc<Mutex<OpenState>>,
    /// The directory the next open chooser starts in, derived from the last
    /// successfully opened/reopened target (and the newest durable recent entry
    /// at startup). `None` means no hint.
    recent_dir: Arc<Mutex<Option<PathBuf>>>,
}

impl ReactFiles {
    /// Files with the real COM `IFileOpenDialog`/`IFileSaveDialog` choosers.
    #[cfg(target_os = "windows")]
    pub fn new(locale: &str) -> CadResult<Self> {
        Self::with_parts(
            locale,
            Arc::new(native::native_choose),
            Some(Arc::new(native::native_save)),
        )
    }

    /// Files with a dependency-injected open chooser and no save chooser
    /// (unit tests, no GUI); a save admitted here fails explicitly.
    pub fn with_chooser(locale: &str, chooser: FileChooser) -> CadResult<Self> {
        Self::with_parts(locale, chooser, None)
    }

    /// Files with dependency-injected open **and** save choosers (unit tests,
    /// no GUI).
    pub fn with_choosers(
        locale: &str,
        chooser: FileChooser,
        save_chooser: SaveChooser,
    ) -> CadResult<Self> {
        Self::with_parts(locale, chooser, Some(save_chooser))
    }

    fn with_parts(
        locale: &str,
        chooser: FileChooser,
        save_chooser: Option<SaveChooser>,
    ) -> CadResult<Self> {
        let labels = Arc::new(WindowsOpenDialogLabels::from_locale(locale)?);
        let save_labels = Arc::new(WindowsSaveDialogLabels::from_locale(locale)?);
        Ok(Self {
            labels,
            save_labels,
            chooser,
            save_chooser,
            state: Arc::new(Mutex::new(OpenState::new())),
            recent_dir: Arc::new(Mutex::new(None)),
        })
    }

    /// Admit one request, or reject while another is in flight or while its
    /// accepted terminal event is still queued. Holding the state mutex makes
    /// admission atomic against [`PlatformFiles::poll`].
    fn admit(&self) -> CadResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.busy {
            return Err(CadError::InvalidInput(
                "an open is already in progress".into(),
            ));
        }
        state.busy = true;
        Ok(())
    }

    fn spawn(&self, name: &str, job: impl FnOnce() + Send + 'static) -> CadResult<()> {
        std::thread::Builder::new()
            .name(name.into())
            .spawn(job)
            .map(|_| ())
            .map_err(|error| {
                // The worker never started, so no terminal event can arrive:
                // release the admission this call just took.
                self.state.lock().unwrap().busy = false;
                CadError::InvalidInput(format!("open worker spawn failed: {error}"))
            })
    }
}

impl PlatformFiles for ReactFiles {
    fn begin_open(&self) -> CadResult<()> {
        self.admit()?;
        let chooser = self.chooser.clone();
        let labels = self.labels.clone();
        let state = self.state.clone();
        // Start in the last successfully opened target's parent (a real
        // directory) when one is known.
        let recent_dir = self.recent_dir.lock().unwrap().clone();
        self.spawn("react-open-chooser", move || {
            let event = match chooser(&labels, recent_dir.as_deref()) {
                FileChooserOutcome::Path(path) => match std::fs::read(&path) {
                    Ok(bytes) => FileEvent::Opened {
                        target: FileTarget::from_path(&path.to_string_lossy()),
                        bytes: Arc::from(bytes),
                    },
                    Err(error) => FileEvent::Failed {
                        purpose: FilePurpose::Open,
                        reason: read_failure(&path, error),
                    },
                },
                FileChooserOutcome::Cancelled => FileEvent::Cancelled {
                    purpose: FilePurpose::Open,
                },
                FileChooserOutcome::Failed(reason) => FileEvent::Failed {
                    purpose: FilePurpose::Open,
                    reason,
                },
            };
            // Publish only; `busy` stays set until `poll` takes this terminal.
            state.lock().unwrap().terminal = Some(event);
        })
    }

    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()> {
        // Every target branch goes through the same admission — including the
        // missing-path branch — so a burst of missing targets is rejected by the
        // single-flight guard instead of queuing unbounded error events.
        self.admit()?;
        let path = PathBuf::from(&target.opaque);
        // A missing path is an explicit failure event, never a synthetic demo
        // fallback and never an empty success.
        if !path.exists() {
            self.state.lock().unwrap().terminal = Some(FileEvent::Failed {
                purpose: FilePurpose::Open,
                reason: format!("drawing path does not exist: {}", path.display()),
            });
            return Ok(());
        }
        let state = self.state.clone();
        let target = target.clone();
        self.spawn("react-open-read", move || {
            let event = match std::fs::read(&path) {
                Ok(bytes) => FileEvent::Opened {
                    target,
                    bytes: Arc::from(bytes),
                },
                Err(error) => FileEvent::Failed {
                    purpose: FilePurpose::Open,
                    reason: read_failure(&path, error),
                },
            };
            // Publish only; `busy` stays set until `poll` takes this terminal.
            state.lock().unwrap().terminal = Some(event);
        })
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        // Same admission as open: one file interaction at a time, never a queue.
        self.admit()?;
        let chooser = self.save_chooser.clone();
        let labels = self.save_labels.clone();
        let state = self.state.clone();
        let default_name = default_name.to_string();
        self.spawn("react-save-chooser", move || {
            let event = match chooser {
                Some(chooser) => match chooser(purpose, &labels, &default_name) {
                    FileChooserOutcome::Path(path) => FileEvent::SaveTarget {
                        purpose,
                        target: FileTarget::from_path(&path.to_string_lossy()),
                    },
                    FileChooserOutcome::Cancelled => FileEvent::Cancelled {
                        purpose: FilePurpose::Save(purpose),
                    },
                    FileChooserOutcome::Failed(reason) => FileEvent::Failed {
                        purpose: FilePurpose::Save(purpose),
                        reason,
                    },
                },
                // No chooser wired: an explicit failure, never a fabricated path.
                None => FileEvent::Failed {
                    purpose: FilePurpose::Save(purpose),
                    reason: labels.picker_failed("no save chooser is configured"),
                },
            };
            // Publish only; `busy` stays set until `poll` takes this terminal.
            state.lock().unwrap().terminal = Some(event);
        })
    }

    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        // The save write always keeps a `.bak` of the bytes it replaces
        // (ADR-0041 D4): overwriting the opened source is an explicit, allowed
        // save, and the backup is what protects the previous document.
        atomic_replace(Path::new(&target.opaque), bytes, true)
    }

    fn read_target(&self, target: &FileTarget) -> Result<Vec<u8>, String> {
        // Used only for the in-place `.ymodel` conflict check: the host compares
        // these bytes with the archive it opened. Never fabricate empty bytes.
        std::fs::read(&target.opaque)
            .map_err(|error| format!("read {} failed: {error}", target.opaque))
    }

    fn set_source(&self, _target: Option<&FileTarget>) {
        // Bookkeeping only: overwriting the opened source is a legal, explicit
        // export (ADR-0041 D4), so this carrier keeps no identity to refuse.
    }

    fn set_recent_target(&self, target: Option<&FileTarget>) {
        *self.recent_dir.lock().unwrap() = target.and_then(recent_directory);
    }

    fn recent_location(&self, target: &FileTarget) -> Option<String> {
        recent_directory(target).map(|directory| directory.to_string_lossy().into_owned())
    }

    fn poll(&self) -> Vec<FileEvent> {
        // Take the accepted terminal and release the admission under one lock,
        // so a new request can only be admitted after the terminal is taken and
        // an in-flight worker can never be unlocked by this reset.
        let mut state = self.state.lock().unwrap();
        match state.terminal.take() {
            Some(event) => {
                state.busy = false;
                vec![event]
            }
            None => Vec::new(),
        }
    }
}

/// The React host's CAD surface: the frontend-reported content rect + scale.
///
/// `SurfaceSource` reads the shared cell; the DOM resize handler writes it
/// ([`Self::set_metrics`]) and a lost window clears it ([`Self::clear`]).
#[derive(Clone, Default)]
pub struct ReactSurface {
    metrics: Rc<Cell<Option<SurfaceMetrics>>>,
}

impl ReactSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current CAD content rectangle (logical) and scale.
    pub fn set_metrics(&self, metrics: SurfaceMetrics) {
        self.metrics.set(Some(metrics));
    }

    /// Clear the metrics (no mapped window yet).
    pub fn clear(&self) {
        self.metrics.set(None);
    }
}

impl SurfaceSource for ReactSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        self.metrics.get()
    }
}

/// React host durable config/preference store (std-fs, no UI toolkit).
pub struct ReactConfigStore {
    config_path: Option<PathBuf>,
    preference_path: Option<PathBuf>,
}

impl ReactConfigStore {
    /// A store over explicit paths.
    pub fn new(config_path: PathBuf, preference_path: PathBuf) -> Self {
        Self {
            config_path: Some(config_path),
            preference_path: Some(preference_path),
        }
    }

    /// A store with no durable location: reads are `Absent`, persistence is an
    /// explicit `Unsupported` (the host falls back to in-memory defaults).
    pub fn non_persistent() -> Self {
        Self {
            config_path: None,
            preference_path: None,
        }
    }
}

impl ConfigStore for ReactConfigStore {
    fn load_config(&self) -> ConfigLoad {
        load_file(self.config_path.as_deref())
    }

    fn load_preference(&self) -> ConfigLoad {
        load_file(self.preference_path.as_deref())
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        let Some(path) = &self.preference_path else {
            return Err(CadError::Unsupported(
                "no durable preference path; preference persistence is disabled".into(),
            ));
        };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| CadError::InvalidInput(error.to_string()))?;
            }
        }
        atomic_replace(path, text.as_bytes(), false)
    }
}

/// Resolve the durable config/preference paths under `%APPDATA%\yacr`.
///
/// `None` when `APPDATA` is not resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
pub fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    match std::env::var_os("APPDATA") {
        Some(dir) => {
            let app = PathBuf::from(dir).join("yacr");
            (
                Some(app.join("config.json")),
                Some(app.join("preference.json")),
            )
        }
        None => (None, None),
    }
}

/// The recovery-snapshot directory (ADR-0037 D12), beside the config files:
/// `%APPDATA%\yacr\recovery`.
///
/// Carrier-private on purpose: it is **not** the document's directory, so a
/// snapshot can never be mistaken for the document or collide with a Save As.
/// `None` when `APPDATA` is not resolvable — the session then installs no
/// persistence and writes no snapshot, rather than inventing a path. Created
/// lazily by the store on the first write.
pub fn recovery_dir() -> Option<PathBuf> {
    config_paths()
        .0
        .and_then(|config| config.parent().map(|dir| dir.join("recovery")))
}

/// Durable wire key of the renderer backend preference (`auto`/`vulkan`/`dx12`).
///
/// It is a top-level user-preference leaf, i.e. the store path `wgpu_backend` in
/// the same durable `preference.json` every other user preference uses.
pub const WGPU_BACKEND_KEY: &str = "wgpu_backend";

/// Read the persisted `wgpu_backend` preference through the host's own store.
///
/// The probe renderer reads this once per presenter construction (never per
/// frame) through the same [`ConfigStore::load_preference`] every other user
/// preference uses — no second store and no ad-hoc file path. An absent or
/// unreadable file, malformed JSON, or a non-string value yields `None`, which
/// the caller maps to the renderer's `auto` default. `ui.wgpu_backend` is also
/// accepted so a nested projection cannot silently disable the setting; the
/// canonical path stays `wgpu_backend`.
pub fn wgpu_backend_preference(store: &dyn ConfigStore) -> Option<String> {
    let ConfigLoad::Text { text, .. } = store.load_preference() else {
        return None;
    };
    backend_preference_from_json(&text)
}

/// Pure extraction behind [`wgpu_backend_preference`] (unit-testable).
fn backend_preference_from_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value
        .get(WGPU_BACKEND_KEY)
        .or_else(|| value.get("ui").and_then(|ui| ui.get(WGPU_BACKEND_KEY)))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// Read a config/preference file, distinguishing absent from unreadable.
fn load_file(path: Option<&Path>) -> ConfigLoad {
    let Some(path) = path else {
        return ConfigLoad::Absent;
    };
    match std::fs::read_to_string(path) {
        Ok(text) => ConfigLoad::Text {
            path: path.display().to_string(),
            text,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigLoad::Absent,
        Err(error) => ConfigLoad::Unreadable {
            path: path.display().to_string(),
            reason: error.to_string(),
        },
    }
}

/// Monotonic suffix so two concurrent writes never share a temp path.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically: temp file in the target's directory,
/// `sync_all`, an optional `.bak` of the previous file, replace the target, then
/// a best-effort parent-directory fsync.
///
/// The backup is taken **before** the replace (hard link, falling back to a
/// copy), never by moving the target away: a move would leave a window in which
/// the document does not exist at all. A hard link costs no extra bytes, so the
/// save peak stays at roughly 2x the file (temp + target) instead of 3x.
///
/// On Windows the replace is `ReplaceFileW` (it keeps the target's identity and
/// attributes) with `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` as the fallback for
/// filesystems/directories where `ReplaceFileW` refuses; on other targets it is
/// `std::fs::rename` (which itself maps to `MoveFileExW` on Windows). This is
/// the only target-specific step: the temp/sync/backup/fsync sequence and its
/// failure contract are identical everywhere, which is what the contract tests
/// exercise on the development host.
///
/// A failure at any step is an explicit [`CadError`] naming the path, and the
/// temp file is removed; a half-written file is never left in the target's
/// place, so the previous bytes survive a failed save untouched.
///
/// A backup that cannot be created does **not** fail the save: the replace is
/// still atomic and the user's edit is the thing at risk. It is reported on
/// stderr instead of being swallowed, because "saved with no backup" and "saved"
/// are different contracts.
///
/// `pub(crate)` so the recovery store ([`crate::recovery_store`]) uses this one
/// implementation instead of a second, subtly different one (without a backup:
/// no recovery flow consumes one and it would leave a `.bak` beside every
/// snapshot).
pub(crate) fn atomic_replace(path: &Path, bytes: &[u8], keep_backup: bool) -> CadResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("yacr");
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{stem}.{}.{sequence}.tmp", std::process::id()));
    // The backup must describe the file we are about to replace, so it is taken
    // before the replace and skipped for a first save (nothing to back up).
    if keep_backup && path.exists() {
        if let Err(reason) = backup_previous(path) {
            eprintln!(
                "yacr: warning: saved {} without a .bak ({reason})",
                path.display()
            );
        }
    }
    if let Err(error) = write_then_replace(&tmp, path, bytes) {
        // Never leave a partial temp file behind, whichever step failed.
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    // Best effort: the replace already published the bytes, so a directory fsync
    // failure is not a write failure (it only weakens crash durability). Windows
    // cannot open a directory as a file; that failure is ignored on purpose.
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Refresh `path.bak` from the current `path`.
///
/// A hard link is preferred: it is atomic, costs no extra bytes and shares the
/// inode, so the backup is byte-identical by construction. Filesystems without
/// hard links (FAT/exFAT, some SMB mounts) fall back to a copy.
fn backup_previous(path: &Path) -> Result<(), String> {
    let backup = path.with_extension(match path.extension().and_then(|ext| ext.to_str()) {
        Some(extension) => format!("{extension}.bak"),
        None => "bak".to_string(),
    });
    // A stale backup must not block a fresh one: `hard_link` refuses an existing
    // destination, and `copy` would silently keep the older bytes if it did not
    // truncate.
    let _ = std::fs::remove_file(&backup);
    match std::fs::hard_link(path, &backup) {
        Ok(()) => Ok(()),
        Err(link_error) => std::fs::copy(path, &backup)
            .map(|_| ())
            .map_err(|copy_error| format!("link: {link_error}; copy: {copy_error}")),
    }
}

/// The fallible half of [`atomic_replace`]: create, write and sync the temp file,
/// then replace the target with it. Every error names the path it failed on.
fn write_then_replace(tmp: &Path, path: &Path, bytes: &[u8]) -> CadResult<()> {
    let mut file = std::fs::File::create(tmp).map_err(|error| {
        CadError::InvalidInput(format!(
            "create temp file {} failed: {error}",
            tmp.display()
        ))
    })?;
    file.write_all(bytes).map_err(|error| {
        CadError::InvalidInput(format!("write {} failed: {error}", tmp.display()))
    })?;
    file.sync_all().map_err(|error| {
        CadError::InvalidInput(format!("sync {} failed: {error}", tmp.display()))
    })?;
    drop(file);
    replace_file(tmp, path)
}

/// Publish `tmp` at `path`, replacing whatever was there.
#[cfg(target_os = "windows")]
fn replace_file(tmp: &Path, path: &Path) -> CadResult<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    let tmp_wide = wide(&tmp.to_string_lossy());
    let path_wide = wide(&path.to_string_lossy());
    let tmp_pcwstr = PCWSTR::from_raw(tmp_wide.as_ptr());
    let path_pcwstr = PCWSTR::from_raw(path_wide.as_ptr());
    // `ReplaceFileW` keeps the target's ACLs/streams and is the documented
    // atomic replace; `None` for the backup name because `atomic_replace` already
    // took the `.bak` itself (a second backup argument would rotate it twice).
    let replaced = unsafe {
        ReplaceFileW(
            path_pcwstr,
            tmp_pcwstr,
            PCWSTR::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            None,
            None,
        )
    };
    if replaced.is_ok() {
        return Ok(());
    }
    // A target that does not exist yet (a first save) is refused by
    // `ReplaceFileW`; so are some non-NTFS destinations. `MoveFileExW` with
    // `MOVEFILE_REPLACE_EXISTING` covers both and is still a single atomic
    // directory-entry swap for an existing target.
    match unsafe { MoveFileExW(tmp_pcwstr, path_pcwstr, MOVEFILE_REPLACE_EXISTING) } {
        Ok(()) => Ok(()),
        Err(error) => Err(CadError::InvalidInput(format!(
            "replace {} failed: ReplaceFileW: {replaced:?}; MoveFileExW: {error}",
            path.display()
        ))),
    }
}

/// Publish `tmp` at `path`, replacing whatever was there.
#[cfg(not(target_os = "windows"))]
fn replace_file(tmp: &Path, path: &Path) -> CadResult<()> {
    std::fs::rename(tmp, path).map_err(|error| {
        CadError::InvalidInput(format!(
            "rename {} to {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

/// A NUL-terminated UTF-16 copy of `value` (kept alive across the Win32 call).
#[cfg(target_os = "windows")]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A localized reason for a failed drawing read.
fn read_failure(path: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("drawing path does not exist: {}", path.display())
    } else {
        error.to_string()
    }
}

/// The parent directory of a successfully-opened target, when it is a real
/// directory.
///
/// Windows' opaque handle is an absolute filesystem path, so the parent is the
/// directory the chooser should start in. A scheme handle or a missing parent
/// yields `None`, never a bogus folder.
fn recent_directory(target: &FileTarget) -> Option<PathBuf> {
    if target.opaque.is_empty() || target.opaque.contains("://") {
        return None;
    }
    let parent = Path::new(&target.opaque).parent()?;
    if parent.as_os_str().is_empty() || !parent.is_dir() {
        return None;
    }
    Some(parent.to_path_buf())
}
