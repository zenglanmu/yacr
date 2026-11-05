//! Android host platform contracts (the `cad-platform` seam).
//!
//! This is the **explicitly limited** platform set of the Android v1 carrier:
//!
//! * [`AndroidFiles`] — [`cad_platform::PlatformFiles`] over the Storage Access
//!   Framework. On the Android target it drives the Kotlin `document*` picker /
//!   reader / writer through the JNI bridge (`crate::saf`) and queues the
//!   outcome as a [`FileEvent`]; the interaction purpose is owned here (armed),
//!   so an open result is never written as a save target and vice versa. On any
//!   other target every entry point is an explicit `CadError::Unsupported`
//!   naming the operation (the SAF bridge is Android-only) and `poll()` is empty.
//!   Nothing fabricates a document, a save target or an opened byte buffer.
//! * [`AndroidSurface`] — [`cad_platform::SurfaceSource`]: the CAD content
//!   rectangle and scale the React layout reports through `probe_input`, shared
//!   through an `Rc<Cell<..>>` exactly like the desktop `ReactSurface`. The native
//!   raster is the `SurfaceView` beneath the WebView, so the *pixels* are the
//!   platform's; this source is what the session validates canvas metrics against.
//! * [`AndroidConfigStore`] — [`cad_platform::ConfigStore`] over the app's private
//!   `filesDir` (`config.json`, `preference.json`), with the same `Absent` /
//!   `Text` / `Unreadable` answers and the same atomic replace as the desktop
//!   `ReactConfigStore`. The directory comes from the frozen `nativeAssetsReady`
//!   handshake (`filesDir` is the parent of the announced `fontsDir`); before that
//!   handshake the store keeps the honest non-persistent behaviour (reads
//!   `Absent`, writes `Unsupported`) rather than inventing a path.
//! * Resource resolution is `cad_host::DesktopResources` with
//!   `HostOptions.fonts_dir`/`images_dir` pointing at the directories the Kotlin
//!   lane extracted and announced through the frozen `nativeAssetsReady` JNI call
//!   ([`asset_dirs`]).
//!
//! The file contracts and the surface cell are **target-independent** on purpose:
//! their refusal tests run on the development host. Only the `nativeAssetsReady`
//! JNI entry point is Android-only (it needs a `JNIEnv`).

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(any(target_os = "android", test))]
use std::sync::Arc;
use std::sync::Mutex;

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};

#[cfg(not(target_os = "android"))]
use crate::policy;

/// File selection/read/write over the Android Storage Access Framework.
///
/// The interaction is event-based, exactly like the desktop host: `begin_*`
/// arms one purpose and starts the Kotlin picker/reader, and `poll()` drains the
/// outcome into a [`FileEvent`]. The purpose is owned **here**, not in Kotlin, so
/// an open-chosen URI can never be handed back as a save target.
///
/// Off the Android target every entry point refuses explicitly (the SAF bridge
/// is only compiled for Android) and `poll()` is empty; the refusal tests run on
/// the development host.
pub struct AndroidFiles {
    /// Events queued by this platform (never fabricated; `poll` only drains this).
    events: RefCell<Vec<FileEvent>>,
    /// The successfully-opened source target, armed by the core only on success.
    source: RefCell<Option<FileTarget>>,
    /// The one SAF interaction in flight (Android only).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    armed: Cell<Option<FilePurpose>>,
}

impl Default for AndroidFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidFiles {
    /// Files for the Android host (SAF on the Android target).
    pub fn new() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
            source: RefCell::new(None),
            armed: Cell::new(None),
        }
    }

    /// Refuse a second concurrent interaction: the single armed purpose would be
    /// overwritten and the results cross-wired.
    #[cfg(target_os = "android")]
    fn arm(&self, purpose: FilePurpose) -> CadResult<()> {
        if self.armed.get().is_some() {
            return Err(CadError::Unsupported(
                "a file interaction is already in progress".into(),
            ));
        }
        self.armed.set(Some(purpose));
        Ok(())
    }
}

impl PlatformFiles for AndroidFiles {
    fn begin_open(&self) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            self.arm(FilePurpose::Open)?;
            if let Err(error) = crate::saf::launch_open() {
                self.armed.set(None);
                return Err(CadError::Unsupported(format!(
                    "the Android SAF open picker could not be started: {error}"
                )));
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            Err(CadError::Unsupported(policy::file_refusal("open")))
        }
    }

    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            self.arm(FilePurpose::Open)?;
            if let Err(error) = crate::saf::read_uri_async(&target.opaque) {
                self.armed.set(None);
                return Err(CadError::Unsupported(format!(
                    "the Android SAF document read could not be started: {error}"
                )));
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = target;
            Err(CadError::Unsupported(policy::file_refusal("open target")))
        }
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            self.arm(FilePurpose::Save(purpose))?;
            if let Err(error) = crate::saf::launch_save(default_name) {
                self.armed.set(None);
                return Err(CadError::Unsupported(format!(
                    "the Android SAF save picker could not be started: {error}"
                )));
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (purpose, default_name);
            Err(CadError::Unsupported(policy::file_refusal("save")))
        }
    }

    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            crate::saf::write_uri(&target.opaque, bytes).map_err(|error| {
                CadError::InvalidInput(format!("writing {} failed: {error}", target.display_name))
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (target, bytes);
            Err(CadError::Unsupported(policy::file_refusal("atomic write")))
        }
    }

    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        // SAF URI identity: Android offers no device/inode pair, so the opaque
        // URI is the only honest source identity.
        self.source
            .borrow()
            .as_ref()
            .is_some_and(|source| source.opaque == target.opaque)
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }

    fn poll(&self) -> Vec<FileEvent> {
        #[cfg(target_os = "android")]
        {
            // Read the platform result and translate it here; the core only ever
            // sees this queue (no event is fabricated).
            let result = crate::saf::poll();
            match self.armed.get() {
                Some(purpose) => {
                    // Pending keeps the purpose armed; a terminal clears it.
                    if let Some(event) = saf_event(purpose, result) {
                        self.armed.set(None);
                        self.events.borrow_mut().push(event);
                    }
                }
                None => {
                    // Nothing armed: only an external/cold-start open is accepted.
                    if let Some(event) = cold_event(result) {
                        self.events.borrow_mut().push(event);
                    }
                }
            }
        }
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// The terminal/pending outcome of one SAF interaction.
///
/// Defined here (behind the same `cfg` as [`saf_event`]) so the purpose→event
/// mapping is unit-testable on the development host; the Android-only
/// `crate::saf` module produces it.
#[cfg(any(target_os = "android", test))]
#[derive(Debug)]
pub(crate) enum SafState {
    /// The picker/read has not completed (state `NONE`/`LOADING`).
    Pending,
    /// The user dismissed the picker (distinct from a failure).
    Cancelled,
    /// The interaction failed; the string is the raw (untranslated) reason.
    Failed(String),
    /// An open target was chosen and its bytes read.
    Opened {
        uri: String,
        name: String,
        bytes: Vec<u8>,
    },
    /// A save target was chosen (no bytes; writing happens later).
    SaveTarget { uri: String, name: String },
}

/// Map an armed purpose and a SAF result to the event to queue.
///
/// `None` means "still pending, queue nothing". A result whose kind does not
/// match the armed purpose is an explicit [`FileEvent::Failed`] — never a
/// fabricated open/save (an open-chosen URI must never be written as a DXF).
#[cfg(any(target_os = "android", test))]
pub(crate) fn saf_event(purpose: FilePurpose, result: SafState) -> Option<FileEvent> {
    match (purpose, result) {
        (_, SafState::Pending) => None,
        (purpose, SafState::Cancelled) => Some(FileEvent::Cancelled { purpose }),
        (purpose, SafState::Failed(reason)) => Some(FileEvent::Failed { purpose, reason }),
        (FilePurpose::Open, SafState::Opened { uri, name, bytes }) => Some(FileEvent::Opened {
            target: FileTarget {
                display_name: name,
                opaque: uri,
            },
            bytes: Arc::from(bytes.into_boxed_slice()),
        }),
        (FilePurpose::Save(save), SafState::SaveTarget { uri, name }) => {
            Some(FileEvent::SaveTarget {
                purpose: save,
                target: FileTarget {
                    display_name: name,
                    opaque: uri,
                },
            })
        }
        (FilePurpose::Open, SafState::SaveTarget { .. }) => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason: "the document picker returned a save target for an open request".into(),
        }),
        (FilePurpose::Save(_), SafState::Opened { .. }) => Some(FileEvent::Failed {
            purpose,
            reason: "the document picker returned an open target for a save request".into(),
        }),
    }
}

/// Map a result with **nothing armed** (an external/cold-start delivery).
///
/// Only an opened document is accepted without an armed request; a save target
/// in that position is an explicit `Failed { Open }` (there is no save request it
/// could belong to), and a cancel is ignored.
#[cfg(any(target_os = "android", test))]
pub(crate) fn cold_event(result: SafState) -> Option<FileEvent> {
    match result {
        SafState::Pending | SafState::Cancelled => None,
        SafState::Opened { uri, name, bytes } => Some(FileEvent::Opened {
            target: FileTarget {
                display_name: name,
                opaque: uri,
            },
            bytes: Arc::from(bytes.into_boxed_slice()),
        }),
        SafState::SaveTarget { .. } => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason: "a save target was delivered without an armed save request".into(),
        }),
        SafState::Failed(reason) => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason,
        }),
    }
}

/// The React-reported CAD content rectangle + scale ([`SurfaceSource`]).
///
/// The tick writes it from the accepted `probe_input` `resize` metrics
/// ([`Self::set_metrics`]); it is never derived from the platform surface, because
/// the session's canvas metrics are DOM/world geometry, not swapchain pixels.
#[derive(Clone, Default)]
pub struct AndroidSurface {
    metrics: Rc<Cell<Option<SurfaceMetrics>>>,
}

impl AndroidSurface {
    /// A surface source with no metrics yet (the session defers creation).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current CAD content rectangle (logical) and scale.
    pub fn set_metrics(&self, metrics: SurfaceMetrics) {
        self.metrics.set(Some(metrics));
    }

    /// Clear the metrics (no live WebView layout yet).
    pub fn clear(&self) {
        self.metrics.set(None);
    }
}

impl SurfaceSource for AndroidSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        self.metrics.get()
    }
}

/// The asset directories announced by the Kotlin asset handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetDirs {
    /// Extracted CAD font package (`fonts/` + `fonts.json`).
    pub fonts: PathBuf,
    /// Extracted raster-image directory (empty when the APK ships none).
    pub images: PathBuf,
}

static ASSET_DIRS: Mutex<Option<AssetDirs>> = Mutex::new(None);

/// Record the directories the Kotlin lane extracted from the APK.
///
/// Both paths are real filesystem paths (the Kotlin lane extracts APK assets
/// first); an empty `fonts` path is refused instead of stored, because a blank
/// path would look like a valid but empty font package.
pub fn set_asset_dirs(fonts: PathBuf, images: PathBuf) -> CadResult<()> {
    if fonts.as_os_str().is_empty() {
        return Err(CadError::InvalidInput(
            "nativeAssetsReady: font directory is empty; no asset package was extracted".into(),
        ));
    }
    let dirs = AssetDirs { fonts, images };
    *ASSET_DIRS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(dirs);
    Ok(())
}

/// The announced asset directories, if the handshake has run.
pub fn asset_dirs() -> Option<AssetDirs> {
    ASSET_DIRS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Durable config file name inside the app's private `filesDir`.
///
/// Same name the desktop hosts use, so the documented `ViewerConfig` document and
/// the `preference.json` user-preference patch mean the same thing on every host.
pub const CONFIG_FILE: &str = "config.json";

/// Durable user-preference file name inside the app's private `filesDir`.
pub const PREFERENCE_FILE: &str = "preference.json";

/// Where this host keeps its durable files.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Location {
    /// An explicit directory (unit tests and an explicit composition).
    Files(PathBuf),
    /// Resolve lazily from the `nativeAssetsReady` announcement. Lazy on purpose:
    /// the handshake is posted from the Android UI thread and may land after Tauri
    /// `setup`, while this store is built during `setup`.
    Handshake,
    /// No durable location: the explicit non-persistent behaviour.
    None,
}

/// Logged once when durability is requested before the handshake has run.
static HANDSHAKE_NOTICE: AtomicBool = AtomicBool::new(false);

/// The app's private `filesDir`, derived from the announced font directory.
///
/// The frozen Kotlin contract announces `fontsDir = File(filesDir, "fonts")`, so
/// `filesDir` is exactly its parent. It is **never** guessed: no announcement (or
/// an unusable one) answers `None` and the store stays non-persistent.
pub(crate) fn handshake_files_dir() -> Option<PathBuf> {
    files_dir_from(asset_dirs().as_ref())
}

/// The pure derivation behind [`handshake_files_dir`].
fn files_dir_from(dirs: Option<&AssetDirs>) -> Option<PathBuf> {
    let dirs = dirs?;
    let parent = dirs.fonts.parent()?;
    // A bare `fonts` relative path has an empty parent; that is not a location.
    (!parent.as_os_str().is_empty()).then(|| parent.to_path_buf())
}

/// The durable config/preference store of this carrier.
///
/// Reads distinguish absent from unreadable exactly like the desktop
/// `ReactConfigStore` (an absent file is a normal first run, an unreadable one is
/// a diagnostic), and a preference write creates the directory and replaces the
/// file atomically. When no durable location is known — the `nativeAssetsReady`
/// handshake has not run, or the store was explicitly built without one — the
/// behaviour is the shared [`cad_platform::NonPersistentConfigStore`] one: reads
/// answer `Absent` and persistence refuses with `Unsupported`. No path is ever
/// invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidConfigStore {
    location: Location,
}

impl Default for AndroidConfigStore {
    fn default() -> Self {
        Self::from_handshake()
    }
}

impl AndroidConfigStore {
    /// A store over an explicit files directory (the app's private `filesDir`).
    pub fn with_files_dir(files_dir: impl Into<PathBuf>) -> Self {
        Self {
            location: Location::Files(files_dir.into()),
        }
    }

    /// A store that resolves its directory from the `nativeAssetsReady` handshake
    /// on every call, so an announcement that lands after `setup` still applies.
    pub fn from_handshake() -> Self {
        Self {
            location: Location::Handshake,
        }
    }

    /// A store with no durable location: reads `Absent`, persistence `Unsupported`.
    pub fn non_persistent() -> Self {
        Self {
            location: Location::None,
        }
    }

    /// The resolved `filesDir`, or `None` while no durable location is known.
    fn files_dir(&self) -> Option<PathBuf> {
        match &self.location {
            Location::Files(dir) => Some(dir.clone()),
            Location::Handshake => {
                let resolved = handshake_files_dir();
                if resolved.is_none() && !HANDSHAKE_NOTICE.swap(true, Ordering::Relaxed) {
                    log::info!(
                        "[probe] android config: no filesDir announced yet (nativeAssetsReady); \
                         preference persistence is unavailable (reads answer Absent, writes are \
                         refused) until the handshake lands"
                    );
                }
                resolved
            }
            Location::None => None,
        }
    }

    /// The durable `config.json` path, if a location is known.
    fn config_path(&self) -> Option<PathBuf> {
        self.files_dir().map(|dir| dir.join(CONFIG_FILE))
    }

    /// The durable `preference.json` path, if a location is known.
    fn preference_path(&self) -> Option<PathBuf> {
        self.files_dir().map(|dir| dir.join(PREFERENCE_FILE))
    }
}

impl ConfigStore for AndroidConfigStore {
    fn load_config(&self) -> ConfigLoad {
        load_file(self.config_path().as_deref())
    }

    fn load_preference(&self) -> ConfigLoad {
        load_file(self.preference_path().as_deref())
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        let Some(path) = self.preference_path() else {
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
        atomic_replace(&path, text.as_bytes())
    }
}

/// The `ConfigStore` this host uses, as the trait object the session takes.
///
/// Resolves `filesDir` from the asset handshake; a later announcement still
/// applies because the resolution is per call, and the same store instance serves
/// both the startup load and every preference write.
pub fn boxed_config_store() -> Rc<dyn ConfigStore> {
    Rc::new(AndroidConfigStore::from_handshake())
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
/// Same shape as the desktop store: a failure at any step is an explicit
/// [`CadError`] naming the path, the temp file is removed, and a partial file is
/// never left in the target's place.
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
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    // Best effort: the rename already published the bytes, so a directory fsync
    // failure only weakens crash durability.
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// The fallible half of [`atomic_replace`]: create, write and sync the temp file,
/// then rename it over the target.
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

/// Build the `HostOptions.fonts_dir`/`images_dir` pair from the announced assets.
///
/// `None` means the handshake has not run yet: no font package is installed and the
/// session reports the resulting locale/resource gap explicitly (never a fabricated
/// face). The tick re-reads this every pass while no session exists, so a handshake
/// that lands after `setup` still applies to the session it builds.
pub fn option_dirs() -> (Option<PathBuf>, Option<PathBuf>) {
    match asset_dirs() {
        Some(dirs) => (Some(dirs.fonts), Some(dirs.images)),
        None => (None, None),
    }
}

/// Android-only: the Kotlin asset handshake (`nativeAssetsReady`).
///
/// `fontsDir`/`imagesDir` are real filesystem paths; the Kotlin lane already
/// extracted the APK assets and only calls this when the font package is present.
/// The directories are stored host-side and applied to the session's
/// `HostOptions`; a malformed string is an explicit `logcat` error, never a silent
/// empty package.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_io_github_zenglanmu_yacr_MainActivity_nativeAssetsReady<'local>(
    mut env: jni::JNIEnv<'local>,
    this: jni::objects::JObject<'local>,
    fonts_dir: jni::objects::JString<'local>,
    images_dir: jni::objects::JString<'local>,
) {
    // The Activity object is only available as the implicit `this` of a frozen
    // callback; capturing it here as well as in `nativeSurfaceCreated` keeps the
    // SAF bridge available regardless of which handshake runs first.
    crate::saf::capture(&mut env, &this);
    let fonts = env.get_string(&fonts_dir).map(String::from);
    let images = env.get_string(&images_dir).map(String::from);
    match (fonts, images) {
        (Ok(fonts), Ok(images)) => {
            let fonts = PathBuf::from(fonts);
            let images = PathBuf::from(images);
            match set_asset_dirs(fonts.clone(), images.clone()) {
                Ok(()) => {
                    // Stored host-side only: this call runs on the Android UI
                    // thread and the CAD session lives on the Tauri loop thread, so
                    // the announced directories are read by the tick when it builds
                    // (or re-reads) the session's `HostOptions`. No cross-thread
                    // session access happens here.
                    log::info!(
                        "nativeAssetsReady fonts={} images={}",
                        fonts.display(),
                        images.display()
                    );
                }
                Err(error) => log::error!("nativeAssetsReady refused: {error:?}"),
            }
        }
        (fonts, images) => log::error!(
            "nativeAssetsReady: unusable asset paths (fonts={:?} images={:?}); no asset package installed",
            fonts.err(),
            images.err()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> FileTarget {
        FileTarget::from_path("/storage/emulated/0/Download/plan.dwg")
    }

    #[test]
    fn every_file_operation_is_an_explicit_unsupported_refusal() {
        let files = AndroidFiles::new();
        let refusals = [
            files.begin_open(),
            files.begin_open_target(&target()),
            files.begin_save(SavePurpose::SaveDxf, "plan.dxf"),
            files.write_atomic(&target(), b"bytes"),
        ];
        for refusal in refusals {
            match refusal {
                Err(CadError::Unsupported(reason)) => {
                    assert!(reason.contains("not supported"), "{reason}");
                }
                other => panic!("expected an explicit Unsupported refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn nothing_is_fabricated_polled_or_guarded() {
        let files = AndroidFiles::new();
        assert!(files.poll().is_empty(), "no file event may be invented");
        // No open can install a source, so no target is ever "the opened source".
        assert!(!files.refuses_source_overwrite(&target()));
        assert!(files.poll().is_empty());
    }

    #[test]
    fn the_surface_source_reports_only_what_the_layout_reported() {
        let surface = AndroidSurface::new();
        assert!(SurfaceSource::surface(&surface).is_none());
        surface.set_metrics(SurfaceMetrics {
            logical: [640.0, 480.0],
            scale: 2.5,
            safe_insets: [0.0; 4],
        });
        assert_eq!(
            SurfaceSource::surface(&surface).map(|metrics| (metrics.logical, metrics.scale)),
            Some(([640.0, 480.0], 2.5))
        );
        surface.clear();
        assert!(SurfaceSource::surface(&surface).is_none());
    }

    #[test]
    fn the_config_store_is_honest_about_not_persisting() {
        // The fallback is the shared non-persistent contract: an absent read is a
        // normal first run, and a write is an explicit refusal (never a claim that
        // the preference was stored).
        let store = AndroidConfigStore::non_persistent();
        assert_eq!(store.load_config(), ConfigLoad::Absent);
        assert_eq!(store.load_preference(), ConfigLoad::Absent);
        match store.persist_preference("{\"ui\":{}}") {
            Err(CadError::Unsupported(reason)) => {
                assert!(reason.contains("persistence is disabled"), "{reason}")
            }
            other => panic!("persistence must refuse explicitly, got {other:?}"),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("yacr-android-config-{}-{name}", std::process::id()))
    }

    #[test]
    fn config_absent_unreadable_and_persist_roundtrip() {
        let dir = temp_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = AndroidConfigStore::with_files_dir(&dir);

        // Absent is a normal first run, not a failure.
        assert_eq!(store.load_config(), ConfigLoad::Absent);
        assert_eq!(store.load_preference(), ConfigLoad::Absent);

        // A directory where the config file should be is unreadable, reported with
        // its path (the diagnostic the core surfaces at startup).
        let unreadable = temp_dir("unreadable");
        let _ = std::fs::remove_dir_all(&unreadable);
        std::fs::create_dir_all(unreadable.join(CONFIG_FILE)).unwrap();
        std::fs::create_dir_all(unreadable.join(PREFERENCE_FILE)).unwrap();
        let store_unreadable = AndroidConfigStore::with_files_dir(&unreadable);
        match store_unreadable.load_config() {
            ConfigLoad::Unreadable { path, reason } => {
                assert!(path.ends_with(CONFIG_FILE), "{path}");
                assert!(!reason.is_empty(), "the reason must name the failure");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
        assert!(matches!(
            store_unreadable.load_preference(),
            ConfigLoad::Unreadable { .. }
        ));

        // A real path persists and reads back, with the path the desktop store
        // would report.
        store.persist_preference("{\"k\":1}").unwrap();
        match store.load_preference() {
            ConfigLoad::Text { path, text } => {
                assert_eq!(text, "{\"k\":1}");
                assert!(path.ends_with(PREFERENCE_FILE), "{path}");
            }
            other => panic!("expected Text, got {other:?}"),
        }

        // The full config document is read through the same store.
        std::fs::write(dir.join(CONFIG_FILE), "{\"ui\":{\"preset\":\"drafting\"}}").unwrap();
        match store.load_config() {
            ConfigLoad::Text { text, .. } => {
                assert_eq!(text, "{\"ui\":{\"preset\":\"drafting\"}}")
            }
            other => panic!("expected Text, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&unreadable);
    }

    #[test]
    fn persist_creates_the_directory_and_replaces_atomically() {
        // A missing directory (as on first launch) is created, and a second write
        // replaces the first with no temp file left behind.
        let root = temp_dir("atomic");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("files");
        let store = AndroidConfigStore::with_files_dir(&dir);
        store.persist_preference("{\"first\":true}").unwrap();
        store.persist_preference("{\"second\":true}").unwrap();
        match store.load_preference() {
            ConfigLoad::Text { text, .. } => assert_eq!(text, "{\"second\":true}"),
            other => panic!("expected the second write, got {other:?}"),
        }
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "a committed write must leave no temp file: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_handshake_directory_is_the_parent_of_the_announced_fonts_dir() {
        // `fontsDir` is `File(filesDir, "fonts")` in the frozen Kotlin contract, so
        // the app's private files directory is its parent — never guessed.
        let dirs = AssetDirs {
            fonts: PathBuf::from("/data/user/0/io.github.zenglanmu.yacr/files/fonts"),
            images: PathBuf::from("/data/user/0/io.github.zenglanmu.yacr/files/images"),
        };
        assert_eq!(
            files_dir_from(Some(&dirs)),
            Some(PathBuf::from("/data/user/0/io.github.zenglanmu.yacr/files"))
        );
        // No announcement, or an unusable one, is `None` (the store then stays
        // non-persistent) rather than an invented location.
        assert_eq!(files_dir_from(None), None);
        let bare = AssetDirs {
            fonts: PathBuf::from("fonts"),
            images: PathBuf::new(),
        };
        assert_eq!(files_dir_from(Some(&bare)), None);
    }

    #[test]
    fn an_empty_font_directory_is_refused_instead_of_looking_valid() {
        let error = set_asset_dirs(PathBuf::new(), PathBuf::from("/tmp/images"))
            .expect_err("an empty font directory must not be stored");
        assert!(matches!(error, CadError::InvalidInput(_)));
    }

    fn opened(uri: &str, name: &str, bytes: &[u8]) -> SafState {
        SafState::Opened {
            uri: uri.to_string(),
            name: name.to_string(),
            bytes: bytes.to_vec(),
        }
    }

    fn save_target(uri: &str, name: &str) -> SafState {
        SafState::SaveTarget {
            uri: uri.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn an_armed_open_only_accepts_an_opened_document() {
        let event = saf_event(
            FilePurpose::Open,
            opened("content://d/1", "plan.dwg", b"bytes"),
        )
        .expect("a completed open must produce an event");
        match event {
            FileEvent::Opened { target, bytes } => {
                assert_eq!(target.opaque, "content://d/1");
                assert_eq!(target.display_name, "plan.dwg");
                assert_eq!(&*bytes, b"bytes");
            }
            other => panic!("expected Opened, got {other:?}"),
        }
        // A save target delivered to an open request is an explicit failure, never
        // an open with an empty body and never a save.
        match saf_event(FilePurpose::Open, save_target("content://d/2", "plan.dxf"))
            .expect("a kind mismatch must be an explicit failure")
        {
            FileEvent::Failed { purpose, reason } => {
                assert_eq!(purpose, FilePurpose::Open);
                assert!(reason.contains("save target"), "{reason}");
            }
            other => panic!("expected an explicit Failed, got {other:?}"),
        }
    }

    #[test]
    fn an_armed_save_only_accepts_a_save_target() {
        match saf_event(
            FilePurpose::Save(SavePurpose::SaveDxf),
            save_target("content://d/3", "plan.dxf"),
        )
        .expect("a completed save chooser must produce an event")
        {
            FileEvent::SaveTarget { purpose, target } => {
                assert_eq!(purpose, SavePurpose::SaveDxf);
                assert_eq!(target.opaque, "content://d/3");
            }
            other => panic!("expected SaveTarget, got {other:?}"),
        }
        match saf_event(
            FilePurpose::Save(SavePurpose::SaveDxf),
            opened("content://d/4", "plan.dwg", b"bytes"),
        )
        .expect("a kind mismatch must be an explicit failure")
        {
            FileEvent::Failed { purpose, reason } => {
                assert_eq!(purpose, FilePurpose::Save(SavePurpose::SaveDxf));
                assert!(reason.contains("open target"), "{reason}");
            }
            other => panic!("expected an explicit Failed, got {other:?}"),
        }
    }

    #[test]
    fn pending_queues_nothing_and_cancelled_stays_distinct() {
        for purpose in [FilePurpose::Open, FilePurpose::Save(SavePurpose::SaveDxf)] {
            assert!(
                saf_event(purpose, SafState::Pending).is_none(),
                "a pending picker must not queue an event"
            );
            match saf_event(purpose, SafState::Cancelled)
                .expect("a cancel must be delivered, not dropped")
            {
                FileEvent::Cancelled { purpose: cancelled } => assert_eq!(cancelled, purpose),
                other => panic!("expected Cancelled, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_failure_is_delivered_for_the_armed_purpose_only() {
        match saf_event(FilePurpose::Open, SafState::Failed("provider died".into()))
            .expect("a failure must be delivered")
        {
            FileEvent::Failed { purpose, reason } => {
                assert_eq!(purpose, FilePurpose::Open);
                assert_eq!(reason, "provider died");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        match saf_event(
            FilePurpose::Save(SavePurpose::Plot),
            SafState::Failed("no writer".into()),
        )
        .expect("a save failure must carry the save purpose")
        {
            FileEvent::Failed { purpose, .. } => {
                assert_eq!(purpose, FilePurpose::Save(SavePurpose::Plot))
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_cold_start_accepts_only_an_opened_document() {
        match cold_event(opened("content://d/5", "cold.dwg", b"cold"))
            .expect("a cold-start open is a real delivery")
        {
            FileEvent::Opened { target, .. } => assert_eq!(target.opaque, "content://d/5"),
            other => panic!("expected Opened, got {other:?}"),
        }
        // A save target with nothing armed has no save request to belong to.
        match cold_event(save_target("content://d/6", "plan.dxf"))
            .expect("an unarmed save target is an explicit failure")
        {
            FileEvent::Failed { purpose, reason } => {
                assert_eq!(purpose, FilePurpose::Open);
                assert!(reason.contains("save target"), "{reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        // Cold-start failures are open failures; a cancel is ignored.
        assert!(matches!(
            cold_event(SafState::Failed("gone".into())),
            Some(FileEvent::Failed {
                purpose: FilePurpose::Open,
                ..
            })
        ));
        assert!(cold_event(SafState::Cancelled).is_none());
        assert!(cold_event(SafState::Pending).is_none());
    }

    #[test]
    fn the_source_guard_compares_saf_uri_identity() {
        let files = AndroidFiles::new();
        let source = FileTarget {
            display_name: "plan.dwg".to_string(),
            opaque: "content://com.android/document/1".to_string(),
        };
        files.set_source(Some(&source));
        assert!(files.refuses_source_overwrite(&source));
        // Same display name, different document: not the opened source.
        assert!(!files.refuses_source_overwrite(&FileTarget {
            display_name: "plan.dwg".to_string(),
            opaque: "content://com.android/document/2".to_string(),
        }));
        files.set_source(None);
        assert!(!files.refuses_source_overwrite(&source));
    }
}
