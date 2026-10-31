//! Android host platform contracts (the `cad-platform` seam).
//!
//! This is the **explicitly limited** platform set of the Android v1 carrier:
//!
//! * [`AndroidFiles`] — [`cad_platform::PlatformFiles`] with **no** file access at
//!   all. Every entry point refuses with `CadError::Unsupported` naming the
//!   operation; `poll()` is empty. Nothing fabricates a document, a save target or
//!   an opened byte buffer, so the session's own `can_open`/`can_save` stay false
//!   (the chrome then offers no dead action).
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
use std::sync::Mutex;

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, PlatformFiles, SavePurpose, SurfaceMetrics,
    SurfaceSource,
};

use crate::policy;

/// File access on the Android v1 carrier: every operation is an explicit refusal.
///
/// There is no chooser, no Storage Access Framework wiring and no drawing bytes, so
/// the honest answer is `Unsupported` for each entry point. `poll()` never returns
/// a fabricated event, and `refuses_source_overwrite` is `false` because no open
/// can ever install a source (the core calls `set_source` only after a successful
/// install).
pub struct AndroidFiles {
    /// Set only if a future open path installs a source. Kept so the guard has an
    /// honest implementation instead of a silent "always false by accident".
    source: RefCell<Option<PathBuf>>,
}

impl Default for AndroidFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidFiles {
    /// Files for the Android v1 carrier (no file access).
    pub fn new() -> Self {
        Self {
            source: RefCell::new(None),
        }
    }
}

impl PlatformFiles for AndroidFiles {
    fn begin_open(&self) -> CadResult<()> {
        Err(CadError::Unsupported(policy::file_refusal("open")))
    }

    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        // No path, no URI and no SAF grant is read on this carrier: a target is
        // exactly as unsupported as the chooser (never a partial success).
        Err(CadError::Unsupported(policy::file_refusal("open target")))
    }

    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Err(CadError::Unsupported(policy::file_refusal("save")))
    }

    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(CadError::Unsupported(policy::file_refusal("atomic write")))
    }

    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        // No open ever succeeds, so there is no installed source to guard. The
        // stored value is returned verbatim (never invented).
        self.source.borrow().is_some()
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.map(|target| PathBuf::from(&target.opaque));
    }

    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
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
fn handshake_files_dir() -> Option<PathBuf> {
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
pub extern "system" fn Java_dev_yacr_app_MainActivity_nativeAssetsReady<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    fonts_dir: jni::objects::JString<'local>,
    images_dir: jni::objects::JString<'local>,
) {
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
            fonts: PathBuf::from("/data/user/0/dev.yacr.app/files/fonts"),
            images: PathBuf::from("/data/user/0/dev.yacr.app/files/images"),
        };
        assert_eq!(
            files_dir_from(Some(&dirs)),
            Some(PathBuf::from("/data/user/0/dev.yacr.app/files"))
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
}
