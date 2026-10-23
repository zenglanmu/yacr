//! Windows host platform contracts.
//!
//! Concrete `cad-platform` implementations for the Windows desktop host,
//! decoupled from the Tauri/WebView2 wiring:
//!
//! * [`ReactFiles`] — [`PlatformFiles`]: a native open via the COM
//!   `IFileOpenDialog` on a worker thread, plus the `--open`/fixture target read.
//!   It never fabricates a success: a cancelled chooser is a
//!   [`FileEvent::Cancelled`], a missing/unreadable file is an explicit
//!   [`FileEvent::Failed`], and a second open is refused while one is pending
//!   (bounded, single-flight). Save/plot are explicitly `Unsupported`.
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
//! `windows.*` keys so a future platform label change cannot leak across hosts.
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

const KEY_PICKER_TITLE: &str = "windows.picker_title";
const KEY_PICKER_FILTER: &str = "windows.picker_filter";
const KEY_PICKER_EMPTY: &str = "windows.picker_empty";
const KEY_PICKER_LOCAL: &str = "windows.picker_local";
const KEY_PICKER_FAILED: &str = "windows.picker_failed";

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
pub type FileChooser = Arc<dyn Fn(&WindowsOpenDialogLabels) -> FileChooserOutcome + Send + Sync>;

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
    chooser: FileChooser,
    state: Arc<Mutex<OpenState>>,
    source: Arc<Mutex<Option<PathBuf>>>,
}

impl ReactFiles {
    /// Files with the real COM `IFileOpenDialog` open chooser.
    pub fn new(locale: &str) -> CadResult<Self> {
        Self::with_chooser(locale, Arc::new(native_choose))
    }

    /// Files with a dependency-injected chooser (unit tests, no GUI).
    pub fn with_chooser(locale: &str, chooser: FileChooser) -> CadResult<Self> {
        let labels = Arc::new(WindowsOpenDialogLabels::from_locale(locale)?);
        Ok(Self {
            labels,
            chooser,
            state: Arc::new(Mutex::new(OpenState::new())),
            source: Arc::new(Mutex::new(None)),
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

    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Err(CadError::Unsupported(
            "save is not supported by the Windows host".into(),
        ))
    }

    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(CadError::Unsupported(
            "atomic write is not supported by the Windows host".into(),
        ))
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
        atomic_write(path, text.as_bytes())
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

/// Write `bytes` to `path` atomically (temp file in the same directory + rename).
fn atomic_write(path: &Path, bytes: &[u8]) -> CadResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("yacr");
    let tmp = parent.join(format!(".{stem}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|error| {
        CadError::InvalidInput(format!("write {} failed: {error}", tmp.display()))
    })?;
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        CadError::InvalidInput(format!("rename to {} failed: {error}", path.display()))
    })
}

/// Whether `target` is the opened source file (canonical identity).
fn same_file(source: &Path, target: &Path) -> bool {
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

/// A COM apartment for the worker thread that shows the native chooser.
///
/// `IFileOpenDialog` is an STA object; the worker owns the apartment for its
/// whole lifetime. `CoUninitialize` is called only when initialization
/// succeeded (including `S_FALSE`, "already initialized on this thread").
struct Apartment {
    initialized: bool,
}

impl Apartment {
    fn new() -> Result<Self, String> {
        let result = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
        };
        if result.is_err() {
            return Err(format!("CoInitializeEx failed: {result:?}"));
        }
        Ok(Self { initialized: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// The real native open chooser (COM `IFileOpenDialog`).
///
/// Runs on a worker thread with its own STA. The dialog has no owner HWND: the
/// host never passes a window handle across threads, so the dialog is not
/// modal to the Tauri window (it still appears in the foreground because the
/// calling process owns it). Failures are explicit and localized; a user
/// cancellation is [`FileChooserOutcome::Cancelled`], never a failure.
fn native_choose(labels: &WindowsOpenDialogLabels) -> FileChooserOutcome {
    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
        SIGDN_FILESYSPATH,
    };

    let _apartment = match Apartment::new() {
        Ok(apartment) => apartment,
        Err(reason) => return FileChooserOutcome::Failed(labels.picker_failed(&reason)),
    };

    let dialog: IFileOpenDialog = match unsafe {
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
    } {
        Ok(dialog) => dialog,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };

    let title = wide(&labels.picker_title);
    let filter_name = wide(&labels.picker_filter);
    let filter_spec = wide("*.dwg;*.dxf");
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR::from_raw(filter_name.as_ptr()),
        pszSpec: PCWSTR::from_raw(filter_spec.as_ptr()),
    }];

    let configure = unsafe {
        dialog
            .SetTitle(PCWSTR::from_raw(title.as_ptr()))
            .and_then(|()| dialog.SetFileTypes(&filters))
            .and_then(|()| dialog.SetOptions(FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM))
    };
    if let Err(error) = configure {
        return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string()));
    }

    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return FileChooserOutcome::Cancelled;
        }
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }

    let item: IShellItem = match unsafe { dialog.GetResult() } {
        Ok(item) => item,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };
    match unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
        Ok(raw) => match crate::support::take_pwstr(raw) {
            Some(path) => FileChooserOutcome::Path(PathBuf::from(path)),
            None => FileChooserOutcome::Failed(labels.picker_local.clone()),
        },
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

/// A NUL-terminated UTF-16 copy of `value` (kept alive across the COM call).
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yacr-win-platform-test-{}-{name}",
            std::process::id()
        ))
    }

    /// Poll until at least one event arrives (bounded), never unbounded.
    fn wait_event(files: &ReactFiles) -> Vec<FileEvent> {
        for _ in 0..300 {
            let events = files.poll();
            if !events.is_empty() {
                return events;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no file event within the bounded wait");
    }

    fn files_with(chooser: FileChooser) -> ReactFiles {
        ReactFiles::with_chooser("zh-CN", chooser).expect("labels load")
    }

    /// A chooser that blocks until the test releases it, so a pending open can be
    /// observed deterministically. Admission happens synchronously in
    /// `begin_open`, so the request is already busy before the worker runs.
    fn blocking_chooser() -> (FileChooser, std::sync::mpsc::Sender<()>) {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let chooser: FileChooser = Arc::new(move |_| {
            // A released or dropped sender both unblock; tests send exactly once.
            let _ = release_rx.lock().unwrap().recv();
            FileChooserOutcome::Cancelled
        });
        (chooser, release_tx)
    }

    #[test]
    fn labels_come_from_the_i18n_catalogue() {
        let zh = WindowsOpenDialogLabels::from_locale("zh-CN").unwrap();
        let en = WindowsOpenDialogLabels::from_locale("en").unwrap();
        let other = WindowsOpenDialogLabels::from_locale("fr").unwrap();
        assert_ne!(zh.picker_title, en.picker_title);
        assert_eq!(other, en, "unknown locale defaults to English");
        assert!(zh.picker_title.contains('打') || zh.picker_title.contains('开'));
        assert!(en.picker_title.contains("Open"));
        assert!(zh.picker_failed("boom").contains("boom"));
    }

    #[test]
    fn opened_target_reads_real_bytes() {
        let path = temp_path("opened.dwg");
        let payload = b"not-a-real-dwg-but-real-bytes";
        std::fs::write(&path, payload).unwrap();
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));
        files
            .begin_open_target(&FileTarget::from_path(&path.to_string_lossy()))
            .unwrap();
        let events = wait_event(&files);
        let _ = std::fs::remove_file(&path);
        match events.as_slice() {
            [FileEvent::Opened { target, bytes }] => {
                assert_eq!(&bytes[..], payload);
                assert_eq!(target.opaque, path.to_string_lossy().into_owned());
            }
            other => panic!("expected Opened, got {other:?}"),
        }
    }

    #[test]
    fn missing_target_reports_an_error_event() {
        let path = temp_path("missing.dwg");
        let _ = std::fs::remove_file(&path);
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));
        files
            .begin_open_target(&FileTarget::from_path(&path.to_string_lossy()))
            .unwrap();
        match files.poll().as_slice() {
            [FileEvent::Failed { purpose, reason }] => {
                assert_eq!(*purpose, FilePurpose::Open);
                assert!(reason.contains("does not exist"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn cancelled_chooser_is_a_cancel_not_a_success() {
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));
        files.begin_open().unwrap();
        match wait_event(&files).as_slice() {
            [FileEvent::Cancelled { purpose }] => assert_eq!(*purpose, FilePurpose::Open),
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn failed_chooser_reports_its_localized_reason() {
        let files = files_with(Arc::new(|labels| {
            FileChooserOutcome::Failed(labels.picker_failed("dialog down"))
        }));
        files.begin_open().unwrap();
        match wait_event(&files).as_slice() {
            [FileEvent::Failed { reason, .. }] => assert!(reason.contains("dialog down")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn second_open_is_rejected_until_the_queued_result_is_polled() {
        // A missing target is an accepted request whose terminal event is queued
        // synchronously, so "busy but result queued" is observable without a race.
        let missing = temp_path("queued-missing.dwg");
        let _ = std::fs::remove_file(&missing);
        let target = FileTarget::from_path(&missing.to_string_lossy());
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));

        files.begin_open_target(&target).unwrap();
        assert!(
            files.begin_open().is_err(),
            "a queued, unpolled result must still be single-flight"
        );

        // Draining the accepted terminal releases the single flight.
        match files.poll().as_slice() {
            [FileEvent::Failed { purpose, .. }] => assert_eq!(*purpose, FilePurpose::Open),
            other => panic!("expected Failed, got {other:?}"),
        }
        files.begin_open_target(&target).unwrap();
        let _ = files.poll();
    }

    #[test]
    fn consumed_terminal_does_not_unlock_a_new_pending_open() {
        let missing = temp_path("old-terminal.dwg");
        let _ = std::fs::remove_file(&missing);
        let target = FileTarget::from_path(&missing.to_string_lossy());
        let (chooser, release) = blocking_chooser();
        let files = files_with(chooser);
        files.begin_open_target(&target).unwrap();
        let _ = files.poll();

        files.begin_open().unwrap();
        assert!(files.poll().is_empty());
        assert!(
            files.begin_open().is_err(),
            "an empty poll must not unlock the new pending open"
        );

        release.send(()).unwrap();
        match wait_event(&files).as_slice() {
            [FileEvent::Cancelled { .. }] => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn terminal_slot_capacity_is_bounded_to_one() {
        let missing = temp_path("bounded.dwg");
        let _ = std::fs::remove_file(&missing);
        let target = FileTarget::from_path(&missing.to_string_lossy());
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));

        files.begin_open_target(&target).unwrap();
        for _ in 0..16 {
            assert!(
                files.begin_open_target(&target).is_err(),
                "extra requests must be rejected, never queued"
            );
        }
        assert_eq!(files.poll().len(), 1, "capacity 1: no unbounded queue");
        assert!(files.poll().is_empty(), "the slot is drained exactly once");
    }

    #[test]
    fn source_guard_uses_canonical_identity() {
        let source = temp_path("source.dwg");
        let other = temp_path("other.dwg");
        std::fs::write(&source, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));
        assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
        files.set_source(Some(&FileTarget::from_path(&source.to_string_lossy())));
        assert!(files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
        assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&other.to_string_lossy())));
        files.set_source(None);
        assert!(!files.refuses_source_overwrite(&FileTarget::from_path(&source.to_string_lossy())));
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&other);
    }

    #[test]
    fn save_and_write_are_explicitly_unsupported() {
        let files = files_with(Arc::new(|_| FileChooserOutcome::Cancelled));
        assert!(matches!(
            files.begin_save(SavePurpose::Plot, "out.svg"),
            Err(CadError::Unsupported(_))
        ));
        assert!(matches!(
            files.write_atomic(&FileTarget::from_path("C:\\x"), b"x"),
            Err(CadError::Unsupported(_))
        ));
    }

    #[test]
    fn surface_metrics_set_and_clear() {
        let surface = ReactSurface::new();
        assert_eq!(surface.surface(), None);
        let metrics = SurfaceMetrics {
            logical: [960.0, 439.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        };
        surface.set_metrics(metrics);
        assert_eq!(surface.surface(), Some(metrics));
        surface.clear();
        assert_eq!(surface.surface(), None);
    }

    #[test]
    fn config_absent_unreadable_and_persist_roundtrip() {
        let absent = temp_path("absent-config.json");
        let _ = std::fs::remove_file(&absent);
        let store = ReactConfigStore::new(absent.clone(), temp_path("absent-pref.json"));
        assert_eq!(store.load_config(), ConfigLoad::Absent);

        // A directory is unreadable, reported as a diagnostic.
        let dir = temp_path("a-directory");
        std::fs::create_dir_all(&dir).unwrap();
        let store = ReactConfigStore::new(dir.clone(), dir.clone());
        assert!(matches!(store.load_config(), ConfigLoad::Unreadable { .. }));

        // A real path persists atomically and reads back.
        let pref = temp_path("pref.json");
        let store = ReactConfigStore::new(absent, pref.clone());
        store.persist_preference("{\"k\":1}").unwrap();
        match store.load_preference() {
            ConfigLoad::Text { text, .. } => assert_eq!(text, "{\"k\":1}"),
            other => panic!("expected Text, got {other:?}"),
        }
        let _ = std::fs::remove_file(&pref);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_persistent_store_refuses_persistence_explicitly() {
        let store = ReactConfigStore::non_persistent();
        assert_eq!(store.load_config(), ConfigLoad::Absent);
        assert!(matches!(
            store.persist_preference("{}"),
            Err(CadError::Unsupported(_))
        ));
    }

    /// The renderer backend preference is read through the same store every
    /// other user preference uses, and every failure mode is a silent `None`
    /// (the presenter then uses `auto`) rather than an invented value.
    #[test]
    fn wgpu_backend_preference_reads_the_durable_store() {
        let absent = temp_path("absent-wgpu-pref.json");
        let _ = std::fs::remove_file(&absent);
        let store = ReactConfigStore::new(temp_path("unused-config.json"), absent);
        assert_eq!(wgpu_backend_preference(&store), None);

        let preferences = [
            (r#"{"wgpu_backend":"dx12"}"#, Some("dx12")),
            (r#"{"wgpu_backend":"vulkan"}"#, Some("vulkan")),
            (r#"{"wgpu_backend":"auto"}"#, Some("auto")),
            // A nested projection is tolerated, the root key is canonical.
            (r#"{"ui":{"wgpu_backend":"dx12"}}"#, Some("dx12")),
            // Unknown tokens are returned verbatim; the renderer resolves them
            // to `auto` (one parser owns the wire contract).
            (r#"{"wgpu_backend":"gl"}"#, Some("gl")),
            // Absent key, non-string value and malformed JSON are all `None`.
            (r#"{"interaction":{"snapEnabled":true}}"#, None),
            (r#"{"wgpu_backend":1}"#, None),
            (r#"not json"#, None),
        ];
        for (text, expected) in preferences {
            assert_eq!(
                backend_preference_from_json(text),
                expected.map(str::to_string),
                "{text} must extract {expected:?}"
            );
        }

        // End-to-end through the durable store (the path the host reads).
        let path = temp_path("wgpu-pref.json");
        let store = ReactConfigStore::new(temp_path("unused-config.json"), path.clone());
        store
            .persist_preference(r#"{"wgpu_backend":"dx12"}"#)
            .unwrap();
        assert_eq!(wgpu_backend_preference(&store), Some("dx12".to_string()));
        let _ = std::fs::remove_file(&path);
    }
}
