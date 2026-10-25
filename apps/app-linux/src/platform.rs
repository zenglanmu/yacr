//! Linux host platform contracts.
//!
//! Concrete `cad-platform` implementations for the Linux desktop host,
//! decoupled from the Tauri/GTK wiring and from any specific UI toolkit:
//!
//! * [`ReactFiles`] — [`PlatformFiles`]: native open **and** save-target choosers
//!   via the XDG desktop portal (`ashpd`) on a worker thread, the
//!   `--open`/fixture target read, and the real atomic save write. It never
//!   fabricates a success: a cancelled chooser is a [`FileEvent::Cancelled`], a
//!   missing/unreadable file or a failed portal call is an explicit
//!   [`FileEvent::Failed`], and open/save share one single-flight admission
//!   (bounded, capacity-1 terminal) — a second interaction is refused while one
//!   is pending. The save write is temp file → `sync_all` → rename → best-effort
//!   parent-directory fsync, so a partial file is never left in place.
//! * [`ReactSurface`] — [`SurfaceSource`]: the CAD content rectangle and scale
//!   reported by the frontend DOM, shared through an `Rc<Cell<..>>`.
//! * [`ReactConfigStore`] — [`ConfigStore`]: std-fs config/preference loading
//!   (`Absent` vs `Unreadable` distinguished) and atomic preference persistence,
//!   or an explicit `Unsupported` when no durable path is configured.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`), selected by locale through
//! `cad_ui_core::i18n` — no host-side hardcoded UI text.
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

#[path = "platform/save.rs"]
mod save;

pub use save::{SaveChooser, UseSaveDialogLabels};

const KEY_PICKER_TITLE: &str = "linux.picker_title";
const KEY_PICKER_FILTER: &str = "linux.picker_filter";
const KEY_PICKER_EMPTY: &str = "linux.picker_empty";
const KEY_PICKER_LOCAL: &str = "linux.picker_local";
const KEY_PICKER_FAILED: &str = "linux.picker_failed";
const KEY_OPEN_PATH: &str = "linux.open_path";

/// The localized labels the native open dialog and open funnel use.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseOpenDialogLabels {
    pub picker_title: String,
    pub picker_filter: String,
    pub picker_empty: String,
    pub picker_local: String,
    /// Template containing a `{error}` placeholder.
    pub picker_failed: String,
    pub open_path: String,
}

impl UseOpenDialogLabels {
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
            open_path: get(KEY_OPEN_PATH)?,
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
pub type FileChooser = Arc<dyn Fn(&UseOpenDialogLabels) -> FileChooserOutcome + Send + Sync>;

/// The single-flight file-interaction state, guarded by one mutex so admission
/// and the terminal hand-off cannot interleave.
///
/// `busy` covers the whole lifetime of a request (open or save): it is set when a
/// request is admitted and cleared **only** by [`ReactFiles::poll`] when it takes
/// the accepted terminal event. A worker publishes the result but never touches
/// `busy`, so a stale worker cannot unlock a newer request.
struct FileState {
    /// True from admission until the accepted terminal event is consumed.
    busy: bool,
    /// The single accepted terminal event awaiting delivery (capacity 1).
    terminal: Option<FileEvent>,
}

impl FileState {
    fn new() -> Self {
        Self {
            busy: false,
            terminal: None,
        }
    }
}

/// React host file open/save implementation of [`PlatformFiles`].
///
/// Single-flight: at most one file interaction (open, read or save chooser) runs
/// at a time, and a request stays busy until its accepted terminal event has been
/// drained by [`PlatformFiles::poll`] on the main thread. Results are handed over
/// through a single terminal slot (capacity 1) rather than an unbounded queue; the
/// trait itself carries no `Send` requirement.
pub struct ReactFiles {
    labels: Arc<UseOpenDialogLabels>,
    save_labels: Arc<UseSaveDialogLabels>,
    chooser: FileChooser,
    /// `None` means "no save chooser is configured": an admitted save still
    /// publishes an explicit [`FileEvent::Failed`], never a fabricated target.
    save_chooser: Option<SaveChooser>,
    state: Arc<Mutex<FileState>>,
    source: Arc<Mutex<Option<PathBuf>>>,
}

impl ReactFiles {
    /// Files with the real XDG-portal open and save choosers (Linux).
    pub fn new(locale: &str) -> CadResult<Self> {
        Self::with_parts(
            locale,
            Arc::new(portal_choose),
            Some(Arc::new(save::portal_save)),
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
        let labels = Arc::new(UseOpenDialogLabels::from_locale(locale)?);
        let save_labels = Arc::new(UseSaveDialogLabels::from_locale(locale)?);
        Ok(Self {
            labels,
            save_labels,
            chooser,
            save_chooser,
            state: Arc::new(Mutex::new(FileState::new())),
            source: Arc::new(Mutex::new(None)),
        })
    }

    /// Admit one request, or reject while another (open or save) is in flight or
    /// while its accepted terminal event is still queued. Holding the state mutex
    /// makes admission atomic against [`PlatformFiles::poll`].
    fn admit(&self) -> CadResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.busy {
            return Err(CadError::InvalidInput(
                "a file interaction is already in progress".into(),
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
                CadError::InvalidInput(format!("file worker spawn failed: {error}"))
            })
    }
}

impl PlatformFiles for ReactFiles {
    fn begin_open(&self) -> CadResult<()> {
        self.admit()?;
        let chooser = self.chooser.clone();
        let labels = self.labels.clone();
        let state = self.state.clone();
        self.spawn("react-open-chooser", move || {
            let event = match chooser(&labels) {
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
        atomic_replace(Path::new(&target.opaque), bytes)
    }

    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        match self.source.lock().unwrap().as_ref() {
            Some(source) => same_file(source, Path::new(&target.opaque)),
            None => false,
        }
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.lock().unwrap() = target.map(|target| PathBuf::from(&target.opaque));
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
        atomic_replace(path, text.as_bytes())
    }
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
/// `sync_all`, rename over the target, then a best-effort parent-directory fsync.
///
/// A failure at any step is an explicit [`CadError`] naming the path, and the
/// temp file is removed; a partial file is never left in the target's place.
fn atomic_replace(path: &Path, bytes: &[u8]) -> CadResult<()> {
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
    if let Err(error) = write_then_rename(&tmp, path, bytes) {
        // Never leave a partial temp file behind, whichever step failed.
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    // Best effort: the rename already published the bytes, so a directory fsync
    // failure is not a write failure (it only weakens crash durability).
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// The fallible half of [`atomic_replace`]: create, write and sync the temp file,
/// then rename it over the target. Every error names the path it failed on.
fn write_then_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> CadResult<()> {
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
    std::fs::rename(tmp, path).map_err(|error| {
        CadError::InvalidInput(format!(
            "rename {} to {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

/// Whether `target` is the opened source file (device + inode, else canonical path).
fn same_file(source: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(source_meta), Ok(target_meta)) =
            (std::fs::metadata(source), std::fs::metadata(target))
        {
            return source_meta.dev() == target_meta.dev()
                && source_meta.ino() == target_meta.ino();
        }
    }
    let canonical = |candidate: &Path| std::fs::canonicalize(candidate).ok();
    match (canonical(source), canonical(target)) {
        (Some(source), Some(target)) => source == target,
        _ => source == target,
    }
}

/// A localized reason for a failed drawing read.
fn read_failure(path: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("drawing path does not exist: {}", path.display())
    } else {
        error.to_string()
    }
}

/// The real native open chooser (XDG desktop portal via `ashpd`).
fn portal_choose(labels: &UseOpenDialogLabels) -> FileChooserOutcome {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use ashpd::desktop::ResponseError;

    let result = pollster::block_on(async {
        SelectedFiles::open_file()
            .title(labels.picker_title.as_str())
            .modal(true)
            .multiple(false)
            .filter(
                FileFilter::new(&labels.picker_filter)
                    .glob("*.[dD][wW][gG]")
                    .glob("*.[dD][xX][fF]"),
            )
            .send()
            .await?
            .response()
    });
    match result {
        Ok(files) => match files.uris().first() {
            Some(uri) => match uri.to_file_path() {
                Ok(path) => FileChooserOutcome::Path(path),
                Err(_) => FileChooserOutcome::Failed(labels.picker_local.clone()),
            },
            None => FileChooserOutcome::Failed(labels.picker_empty.clone()),
        },
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => FileChooserOutcome::Cancelled,
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

#[cfg(test)]
#[path = "platform/tests.rs"]
mod tests;
