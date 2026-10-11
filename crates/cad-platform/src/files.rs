//! Host file/platform contracts: event-based, object-safe, no futures, no `Send`.
//!
//! These reshape the earlier 0-implementation `FileAccess` draft into the shape
//! the shared host core (`cad-host`) actually drives:
//!
//! * [`PlatformFiles`] is **event-based** — the host calls `begin_*` inside the
//!   user gesture and then drains [`FileEvent`]s from `poll`. A synchronous
//!   platform (a headless CLI-style host) can deliver its event on the very
//!   first `poll`, so the core always calls `poll` immediately after `begin_*`.
//! * [`ConfigStore`] reads/writes the durable host config and user preference.
//! * [`SurfaceSource`] supplies the physical surface metrics the core maps onto
//!   the viewport (safe-area insets are explicit zeros on the desktop, never a
//!   fabricated margin).
//! * [`HostCapabilities`] gates which host-owned operations the shell advertises.
//!
//! None of these traits use `async`/futures or require `Send`/`Sync`: the host
//! core is single-threaded (`Rc`-based) and a platform that needs a worker owns
//! it internally, delivering results as events.
use std::sync::Arc;

use cad_domain::CadResult;

/// Which export a save dialog was opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePurpose {
    /// Lossy DXF Save As.
    SaveDxf,
    /// Vector plot/export (SVG/PDF).
    Plot,
}

/// Which file interaction a cancel/failure event refers to.
///
/// The sketch in the host-refactor design used `SavePurpose` directly, but the
/// open picker can also be cancelled or fail (a closed dialog is a cancel, a
/// portal error is a failure). This wraps both so the core keeps its explicit
/// open-cancel status instead of silently dropping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePurpose {
    Open,
    Save(SavePurpose),
}

/// A chosen file target.
///
/// `opaque` is the platform's own handle for the target: an absolute path on the
/// desktop, a Storage Access Framework URI on Android, a browser download handle
/// on the web. `display_name` is the human-facing name shown in the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub display_name: String,
    pub opaque: String,
}

impl FileTarget {
    /// A target whose opaque handle and display name are the same path string.
    pub fn from_path(path: &str) -> Self {
        Self {
            display_name: path.to_string(),
            opaque: path.to_string(),
        }
    }
}

/// One completed platform file interaction.
#[derive(Debug, Clone)]
pub enum FileEvent {
    /// A drawing was opened and fully read.
    ///
    /// `target` carries the platform's own handle (`opaque`) so the core can arm
    /// the source-overwrite guard against the exact file it opened (a SAF URI /
    /// download handle on non-desktop platforms), not a path re-derived from the
    /// display name. `bytes` are the raw drawing bytes.
    Opened {
        target: FileTarget,
        bytes: Arc<[u8]>,
    },
    /// A save target was chosen for `purpose`.
    SaveTarget {
        purpose: SavePurpose,
        target: FileTarget,
    },
    /// The interaction was cancelled (a closed dialog is not a failure).
    Cancelled { purpose: FilePurpose },
    /// The interaction failed explicitly, with an already-localized reason.
    Failed {
        purpose: FilePurpose,
        reason: String,
    },
}

/// Host file selection, read and atomic write.
pub trait PlatformFiles {
    /// Start a user-initiated open (a native/portal file chooser).
    fn begin_open(&self) -> CadResult<()>;

    /// Start opening a specific target (a `--open` path or a headless input),
    /// bypassing the chooser.
    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()>;

    /// Start a save-target chooser for `purpose` with a suggested default name.
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()>;

    /// Write `bytes` to `target` atomically (temp file + fsync + rename on the
    /// desktop; a single platform write elsewhere).
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()>;

    /// Whether `target` is the successfully-opened source file, so a save must
    /// refuse to overwrite it (data-corruption guard). Desktop compares device +
    /// inode; the web has no filesystem identity and returns `false`.
    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool;

    /// Remember (`Some`) or clear (`None`) the successfully-opened source, so
    /// [`PlatformFiles::refuses_source_overwrite`] can guard it. The core calls
    /// this only after a **successful** install, never at attempt time.
    fn set_source(&self, target: Option<&FileTarget>);

    /// Drain every completed interaction since the last call.
    fn poll(&self) -> Vec<FileEvent>;
}

/// Outcome of reading a durable host config/preference file.
///
/// The design sketch used `Option<String>`; that cannot distinguish an absent
/// file (normal) from an unreadable one (a real diagnostic), which would be a
/// silent no-op. `ConfigLoad` carries the resolved path so the core reproduces
/// the exact startup diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoad {
    /// The file is absent (normal first run).
    Absent,
    /// The file was read; `path` is its display path.
    Text { path: String, text: String },
    /// The file exists but could not be read.
    Unreadable { path: String, reason: String },
}

/// Durable host config and user-preference storage.
pub trait ConfigStore {
    /// Read the full host `ViewerConfig` file.
    fn load_config(&self) -> ConfigLoad;
    /// Read the projected user-preference patch.
    fn load_preference(&self) -> ConfigLoad;
    /// Persist the store's allowed projection of the user preference.
    fn persist_preference(&self, text: &str) -> CadResult<()>;
}

/// The physical surface the CAD canvas is mapped onto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceMetrics {
    /// Logical size of the CAD content rectangle.
    pub logical: [f64; 2],
    /// Window/display scale factor.
    pub scale: f64,
    /// Safe-area insets `[top, right, bottom, left]` in logical pixels.
    pub safe_insets: [f64; 4],
}

/// Source of the current surface metrics.
pub trait SurfaceSource {
    /// Current surface metrics, or `None` when the window does not exist yet.
    fn surface(&self) -> Option<SurfaceMetrics>;
}

/// Which host-owned operations this host advertises to the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostCapabilities {
    pub open: bool,
    pub new: bool,
    pub save: bool,
    pub plot: bool,
    /// Whether an open runs on a background worker (desktop) rather than the
    /// calling thread (headless/CLI-style). Passed in; the core carries no
    /// `cfg`.
    pub import_worker: bool,
}
