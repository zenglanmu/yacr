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
//! * No `ConfigStore` type of its own: [`cad_platform::NonPersistentConfigStore`]
//!   is the shared "no durable location" store (reads `Absent`, persistence is an
//!   explicit `Unsupported`).
//! * Resource resolution is `cad_host::DesktopResources` with
//!   `HostOptions.fonts_dir`/`images_dir` pointing at the directories the Kotlin
//!   lane extracted and announced through the frozen `nativeAssetsReady` JNI call
//!   ([`asset_dirs`]).
//!
//! The file contracts and the surface cell are **target-independent** on purpose:
//! their refusal tests run on the development host. Only the `nativeAssetsReady`
//! JNI entry point is Android-only (it needs a `JNIEnv`).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
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

/// The non-persistent config/preference store of this carrier.
///
/// A named alias rather than a local wrapper: the behaviour (reads `Absent`,
/// persistence is `Unsupported`) is the shared
/// [`cad_platform::NonPersistentConfigStore`] contract, so the two hosts cannot
/// drift into "sometimes stored, sometimes not".
pub fn config_store() -> cad_platform::NonPersistentConfigStore {
    cad_platform::NonPersistentConfigStore
}

/// The `ConfigStore` this host uses, as the trait object the session takes.
pub fn boxed_config_store() -> Rc<dyn ConfigStore> {
    Rc::new(config_store())
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

/// The `load_config` answer of the non-persistent store (`Absent`).
pub const fn absent_config() -> ConfigLoad {
    ConfigLoad::Absent
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
        let store = config_store();
        assert_eq!(store.load_config(), absent_config());
        assert_eq!(store.load_preference(), ConfigLoad::Absent);
        match store.persist_preference("{\"ui\":{}}") {
            Err(CadError::Unsupported(reason)) => {
                assert!(reason.contains("persistence is disabled"), "{reason}")
            }
            other => panic!("persistence must refuse explicitly, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_font_directory_is_refused_instead_of_looking_valid() {
        let error = set_asset_dirs(PathBuf::new(), PathBuf::from("/tmp/images"))
            .expect_err("an empty font directory must not be stored");
        assert!(matches!(error, CadError::InvalidInput(_)));
    }
}
