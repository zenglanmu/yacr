//! macOS host platform contracts.
//!
//! Concrete `cad-platform` implementations for the macOS desktop host,
//! decoupled from the Tauri/AppKit wiring:
//!
//! * [`ReactFiles`] — [`PlatformFiles`]: a native open via an in-process
//!   `NSOpenPanel` **and** a native save via `NSSavePanel`, plus the
//!   `--open`/fixture target read and the atomic save write. AppKit requires
//!   panel work on the main thread, so `begin_*` spawns a worker that hops onto
//!   the main thread with [`tauri::AppHandle::run_on_main_thread`] and blocks on
//!   a one-shot channel for the outcome before publishing it. It never
//!   fabricates a success: the panel's OK/cancel response is the real one, a
//!   cancelled chooser is a [`FileEvent::Cancelled`], a missing/unreadable file
//!   or a failed panel is an explicit [`FileEvent::Failed`], and a second
//!   interaction is refused while one is pending (bounded, single-flight). The
//!   save write is temp file → `sync_all` → `.bak` of the previous file →
//!   rename → best-effort parent-directory fsync (ADR-0041 D4).
//! * [`ReactSurface`] — [`SurfaceSource`]: the CAD content rectangle and scale
//!   reported by the frontend DOM, shared through an `Rc<Cell<..>>`.
//! * [`ReactConfigStore`] — [`ConfigStore`]: std-fs config/preference loading
//!   (`Absent` vs `Unreadable` distinguished) and atomic preference persistence
//!   under `$HOME/Library/Application Support/yacr`, or an explicit
//!   `Unsupported` when no durable path is configured.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`), selected by locale through
//! `cad_ui_core::i18n` — no host-side hardcoded UI text. macOS owns its own
//! `macos.*` keys so a future platform label change cannot leak across hosts.
//!
//! Every native failure is surfaced as a localized [`FileChooserOutcome::Failed`]
//! string; only the panel's own OK response with a resolved local path is a
//! path, and a non-OK modal response is a cancellation. Nothing here turns a
//! failure or a cancellation into a synthetic success.
//!
//! The module is target-independent: everything except the AppKit panels is
//! plain std-fs/trait code, so the open/save/write/config contracts are
//! unit-tested on any host. Only [`native_choose`] and the save panel
//! (`platform/save.rs`) need the real macOS target to compile.
//!
//! Resource resolution (`cad_host::DesktopResources`) and the `HostPlatform`
//! composition are built by the caller; this module only supplies the contracts.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_core::i18n::MessageSource;

#[path = "platform/fs.rs"]
pub(crate) mod fs;

#[path = "platform/save.rs"]
pub mod save;

pub(crate) use fs::atomic_replace;
pub use save::{save_allowed_extensions, MacosSaveDialogLabels, SaveChooser};

const KEY_PICKER_TITLE: &str = "macos.picker_title";
const KEY_PICKER_FILTER: &str = "macos.picker_filter";
const KEY_PICKER_EMPTY: &str = "macos.picker_empty";
const KEY_PICKER_LOCAL: &str = "macos.picker_local";
const KEY_PICKER_FAILED: &str = "macos.picker_failed";

/// The extensions `NSOpenPanel` allows, in display order.
///
/// `.rfa`/`.rft` are listed **on purpose**: the RVT adapter refuses a family file
/// with a precise `import.rvt.family_no_geometry` diagnostic, which is more
/// useful than the file being unselectable. They are deliberately *not* in any
/// "supported formats" copy (see `README.md` / the i18n filter labels).
/// `.skp` is the SketchUp preview lane (triangles only, never a measurement
/// source; an edge-only model is refused by name, not by the picker).
const OPEN_EXTENSIONS: &[&str] = &[
    "ymodel", "dwg", "dxf", "obj", "stl", "ply", "off", "coff", "gltf", "glb", "step", "stp",
    "ifc", "rvt", "rte", "rfa", "rft", "skp", "dgn",
];

/// The localized labels the native open dialog and open funnel use.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacosOpenDialogLabels {
    pub picker_title: String,
    pub picker_filter: String,
    pub picker_empty: String,
    pub picker_local: String,
    /// Template containing a `{error}` placeholder.
    pub picker_failed: String,
}

impl MacosOpenDialogLabels {
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
    Arc<dyn Fn(&MacosOpenDialogLabels, Option<&Path>) -> FileChooserOutcome + Send + Sync>;

/// The single-flight file-interaction state (open **and** save share it),
/// guarded by one mutex so admission and the terminal hand-off cannot
/// interleave.
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

/// React host file open/save/read implementation of [`PlatformFiles`].
///
/// Single-flight: at most one open or save worker runs at a time, and a request
/// stays busy until its accepted terminal event has been drained by
/// [`PlatformFiles::poll`] on the main thread. Results are handed over through a
/// single terminal slot (capacity 1) rather than an unbounded queue; the trait
/// itself carries no `Send` requirement.
pub struct ReactFiles {
    labels: Arc<MacosOpenDialogLabels>,
    chooser: FileChooser,
    save_labels: Arc<MacosSaveDialogLabels>,
    save_chooser: Option<SaveChooser>,
    state: Arc<Mutex<OpenState>>,
    /// The directory the next open chooser starts in, derived from the last
    /// successfully opened/reopened target (and the newest durable recent entry
    /// at startup). `None` means no hint.
    recent_dir: Arc<Mutex<Option<PathBuf>>>,
}

impl ReactFiles {
    /// Files with the real in-process `NSOpenPanel`/`NSSavePanel` choosers.
    ///
    /// The Tauri handle is needed only for the main-thread hop: the panels
    /// themselves are created inside the main-thread closure, never on the
    /// worker.
    #[cfg(target_os = "macos")]
    pub fn new(locale: &str, app: tauri::AppHandle) -> CadResult<Self> {
        let open_app = app.clone();
        let save_app = app;
        Self::with_choosers(
            locale,
            Arc::new(
                move |labels: &MacosOpenDialogLabels, initial: Option<&Path>| {
                    native_choose(&open_app, labels, initial)
                },
            ),
            Some(Arc::new(
                move |purpose, labels: &MacosSaveDialogLabels, default_name| {
                    save::native_save(&save_app, purpose, labels, default_name)
                },
            )),
        )
    }

    /// Files with a dependency-injected **open** chooser and no save chooser
    /// (unit tests, no GUI). A save request then fails explicitly, never with a
    /// fabricated path.
    ///
    /// The injected chooser runs on the same worker thread the native chooser
    /// uses, so its threading contract matches production.
    pub fn with_chooser(locale: &str, chooser: FileChooser) -> CadResult<Self> {
        Self::with_choosers(locale, chooser, None)
    }

    /// Files with both choosers injected (unit tests, no GUI).
    pub fn with_choosers(
        locale: &str,
        chooser: FileChooser,
        save_chooser: Option<SaveChooser>,
    ) -> CadResult<Self> {
        let labels = Arc::new(MacosOpenDialogLabels::from_locale(locale)?);
        let save_labels = Arc::new(MacosSaveDialogLabels::from_locale(locale)?);
        Ok(Self {
            labels,
            chooser,
            save_labels,
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
        std::fs::read(&target.opaque).map_err(|error| {
            format!(
                "read {} failed: {error}",
                Path::new(&target.opaque).display()
            )
        })
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
        atomic_write(path, text.as_bytes())
    }
}

/// Resolve the durable config/preference paths under
/// `$HOME/Library/Application Support/yacr`.
///
/// `None` when `HOME` is not resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
pub fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    match config_dir() {
        Some(app) => (
            Some(app.join("config.json")),
            Some(app.join("preference.json")),
        ),
        None => (None, None),
    }
}

/// The carrier-private directory for durable host data
/// (`$HOME/Library/Application Support/yacr`).
///
/// `None` when `HOME` is not resolvable: a caller that needs a durable private
/// place must then refuse rather than invent one. This is the one carrier helper
/// both [`config_paths`] and [`recovery_dir`] derive from.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("yacr")
    })
}

/// The recovery-snapshot directory (ADR-0037 D12), beside the config files:
/// `~/Library/Application Support/yacr/recovery`.
///
/// Carrier-private on purpose: it is **not** the document's directory, so a
/// snapshot can never be mistaken for the document or collide with a Save As.
/// Created lazily by the store on the first write.
pub fn recovery_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("recovery"))
}

/// Durable wire key of the renderer backend preference
/// (`auto`/`vulkan`/`dx12`/`metal`).
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
/// the caller maps to the renderer's `auto` default (Metal on macOS). The value
/// is returned verbatim, so `metal` is the normal explicit choice on this host.
/// `ui.wgpu_backend` is also accepted so a nested projection cannot silently
/// disable the setting; the canonical path stays `wgpu_backend`.
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

/// Write `bytes` to `path` atomically (temp file in the same directory + rename).
///
/// Config/preference writes ask for no `.bak`; only a document save keeps one.
fn atomic_write(path: &Path, bytes: &[u8]) -> CadResult<()> {
    atomic_replace(path, bytes, false)
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
/// The macOS opaque handle is an absolute filesystem path, so the parent is the
/// directory the panel should start in. A scheme handle or a missing parent
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

/// The real native open chooser (in-process AppKit `NSOpenPanel`).
///
/// Runs on the single-flight worker; the panel itself is created inside a
/// main-thread hop (`AppHandle::run_on_main_thread`) and the worker blocks on a
/// one-shot channel for the outcome. AppKit requires the panel on the main
/// thread and `runModal` runs its own modal event loop there, so the host stays
/// responsive while the panel is open; the worker is the only thread that waits
/// on the result channel. The panel's modal response is the real user answer —
/// OK with a resolved local path is [`FileChooserOutcome::Path`], any non-OK
/// response is [`FileChooserOutcome::Cancelled`] — and every other failure is a
/// localized [`FileChooserOutcome::Failed`], never a synthetic success.
#[cfg(target_os = "macos")]
fn native_choose(
    app: &tauri::AppHandle,
    labels: &MacosOpenDialogLabels,
    initial: Option<&Path>,
) -> FileChooserOutcome {
    let (outcome_tx, outcome_rx) = std::sync::mpsc::sync_channel::<FileChooserOutcome>(1);
    let main_labels = labels.clone();
    let initial = initial.map(Path::to_path_buf);
    let dispatched = app.run_on_main_thread(move || {
        // Capacity 1 means this send never blocks, even when the worker has
        // already gone away; a dropped receiver is not an error worth reporting.
        let _ = outcome_tx.send(show_open_panel(&main_labels, initial.as_deref()));
    });
    if let Err(error) = dispatched {
        return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string()));
    }
    match outcome_rx.recv() {
        Ok(outcome) => outcome,
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

/// Build, configure and run `NSOpenPanel` on the main thread.
///
/// Never called on the main thread *while blocking*: it is the body of the hop,
/// so it runs to completion inline (its modal loop pumps AppKit events) and its
/// only result crosses back through the channel.
#[cfg(target_os = "macos")]
#[allow(deprecated)] // `setAllowedFileTypes:` is deprecated but still the only
                     // extension filter available without UTType bindings.
fn show_open_panel(labels: &MacosOpenDialogLabels, initial: Option<&Path>) -> FileChooserOutcome {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let mtm = match objc2::MainThreadMarker::new() {
        Some(mtm) => mtm,
        None => {
            return FileChooserOutcome::Failed(
                labels.picker_failed("the system open panel requires the main thread"),
            );
        }
    };

    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);

    let title = NSString::from_str(&labels.picker_title);
    panel.setTitle(Some(&title));
    // Start in the remembered drawing's directory when it still exists. The
    // panel API takes a file URL and only navigates to a real directory; a
    // stale hint is ignored (a convenience loss, never a failed open).
    if let Some(directory) = initial.filter(|directory| directory.is_dir()) {
        let path = NSString::from_str(&directory.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        panel.setDirectoryURL(Some(&url));
    }
    let filter = NSString::from_str(&labels.picker_filter);
    panel.setMessage(Some(&filter));

    let types = NSArray::from_retained_slice(
        &OPEN_EXTENSIONS
            .iter()
            .map(|extension| NSString::from_str(extension))
            .collect::<Vec<_>>(),
    );
    panel.setAllowedFileTypes(Some(&types));

    let accepted = panel.runModal() == NSModalResponseOK;
    if !accepted {
        return FileChooserOutcome::Cancelled;
    }

    match panel.URL().and_then(|url| url.path()) {
        Some(path) => FileChooserOutcome::Path(PathBuf::from(path.to_string())),
        None => FileChooserOutcome::Failed(labels.picker_local.clone()),
    }
}

#[cfg(test)]
#[path = "platform/tests.rs"]
mod tests;
