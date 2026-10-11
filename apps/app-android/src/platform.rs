//! Android implementations of the `cad-platform` host contracts.
//!
//! `cad_host::HostCore` drives the open/save/plot funnels and the resource
//! installation through these traits; the Android host supplies the same
//! event-based shape the desktop does, but this build has no Storage Access
//! Framework picker and no durable config store:
//!
//! * [`AndroidFiles`] probes a fixed list of sample DWG paths on open and queues
//!   the bytes as a [`FileEvent::Opened`] (or an explicit [`FileEvent::Failed`]
//!   when none exists / the read fails). Save/plot are not wired this phase and
//!   are explicit `Unsupported`.
//! * [`AndroidConfigStore`] has no durable store: nothing to load, and persisting
//!   is refused rather than silently succeeding.
//! * [`AndroidSurface`] reports the surface the Activity forwarded through
//!   `set_surface_size`/`set_surface_insets`.
//! * [`AndroidResources`] installs fonts from the APK assets; raster images are
//!   not modeled by this host yet.
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_host::{HostCore, HostResources};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_slint::{CadView, MessageSource};

/// The Android build's catalog locale. The shell is started `zh-CN` and the host
/// does not expose a runtime language switch, so platform-localized reasons use
/// the same fixed locale.
fn messages() -> MessageSource {
    MessageSource::from_request("zh-CN")
}

thread_local! {
    /// Last logical CAD surface size + DPI scale reported by the Activity, or
    /// the configured demo size before the first `set_surface_size`. Retained so
    /// a later inset update re-applies the same surface.
    static SURFACE: Cell<([f64; 2], f64)> = const { Cell::new(([0.0, 0.0], 1.0)) };
    /// Last safe-area insets `[top, right, bottom, left]` in logical pixels.
    /// Zeros until a platform source provides them; never invented.
    static SAFE_INSETS: Cell<[f64; 4]> = const { Cell::new([0.0; 4]) };
}

/// Seed the surface from the configured logical size before the first frame.
pub(crate) fn set_surface_slot(size: [f64; 2], scale: f64) {
    SURFACE.with(|cell| cell.set((size, scale)));
}

/// The last reported surface `(logical size, DPI scale)`.
pub(crate) fn surface_snapshot() -> ([f64; 2], f64) {
    SURFACE.with(Cell::get)
}

/// The last reported safe-area insets.
pub(crate) fn safe_insets() -> [f64; 4] {
    SAFE_INSETS.with(Cell::get)
}

/// Record the latest safe-area insets reported by the Activity.
pub(crate) fn set_safe_insets(insets: [f64; 4]) {
    SAFE_INSETS.with(|cell| cell.set(insets));
}

/// Android file selection/read.
///
/// There is no SAF picker in this build: `begin_open` probes the configured
/// sample paths synchronously and queues the result. The queue is drained by the
/// shared core's `drain_files`, which runs inside the open gesture (the read is
/// synchronous, so the event is available immediately).
pub struct AndroidFiles {
    sample_paths: Vec<String>,
    events: RefCell<Vec<FileEvent>>,
    /// The successfully-opened source target (armed by the core only on success).
    source: RefCell<Option<FileTarget>>,
}

impl AndroidFiles {
    pub fn new(sample_paths: Vec<String>) -> Self {
        Self {
            sample_paths,
            events: RefCell::new(Vec::new()),
            source: RefCell::new(None),
        }
    }
}

impl PlatformFiles for AndroidFiles {
    fn begin_open(&self) -> CadResult<()> {
        // Real SAF integration is not part of this build; the search is
        // explicitly reported so it is not mistaken for a file picker (spec
        // §9.1). Every candidate is tried; a missing sample (or a candidate that
        // exists but cannot be read) is an explicit failure, never a silent
        // no-op, and the current (demo) document is kept.
        let mut read_failure: Option<String> = None;
        for path in &self.sample_paths {
            let candidate = std::path::Path::new(path);
            if !candidate.exists() {
                continue;
            }
            match std::fs::read(candidate) {
                Ok(bytes) => {
                    self.events.borrow_mut().push(FileEvent::Opened {
                        target: FileTarget::from_path(path),
                        bytes: Arc::from(bytes.into_boxed_slice()),
                    });
                    return Ok(());
                }
                Err(error) => {
                    read_failure = Some(messages().text(
                        "android.read_failed",
                        &[("path", path), ("error", &error.to_string())],
                    ));
                }
            }
        }
        let reason = read_failure.unwrap_or_else(|| messages().text("android.no_sample", &[]));
        self.events.borrow_mut().push(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason,
        });
        Ok(())
    }

    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()> {
        let path = std::path::Path::new(&target.opaque);
        let bytes = std::fs::read(path).map_err(|error| {
            CadError::InvalidInput(messages().text(
                "android.read_failed",
                &[
                    ("path", &target.display_name),
                    ("error", &error.to_string()),
                ],
            ))
        })?;
        self.events.borrow_mut().push(FileEvent::Opened {
            target: target.clone(),
            bytes: Arc::from(bytes.into_boxed_slice()),
        });
        Ok(())
    }

    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        // Save/plot are not wired on Android this phase; the capability gates
        // already refuse them, and this is an explicit refusal, never a no-op.
        Err(CadError::Unsupported(
            messages().text("android.save_unavailable", &[]),
        ))
    }

    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(CadError::Unsupported(
            messages().text("android.save_unavailable", &[]),
        ))
    }

    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        // No save is wired, so this is never consulted; the opaque handle is
        // compared as a string (Android has no filesystem identity to offer).
        self.source
            .borrow()
            .as_ref()
            .is_some_and(|source| source.opaque == target.opaque)
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }

    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// Android config/preference store.
///
/// This build has **no durable store** (no writable app-config location is
/// wired): there is nothing to load, and persisting is refused explicitly rather
/// than silently succeeding. The host preserves its current behavior and never
/// calls `load_startup`, so no startup diagnostics are fabricated.
pub struct AndroidConfigStore;

impl ConfigStore for AndroidConfigStore {
    fn load_config(&self) -> ConfigLoad {
        // No durable store: nothing to read (not a success claim).
        ConfigLoad::Absent
    }

    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }

    fn persist_preference(&self, _text: &str) -> CadResult<()> {
        Err(CadError::Unsupported(
            messages().text("android.no_config_store", &[]),
        ))
    }
}

/// Android surface metrics: the size/scale/insets the Activity forwarded.
/// `None` until a real surface size is known.
pub struct AndroidSurface;

impl SurfaceSource for AndroidSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        let (logical, scale) = surface_snapshot();
        if logical[0] > 0.0 && logical[1] > 0.0 {
            Some(SurfaceMetrics {
                logical,
                scale,
                safe_insets: safe_insets(),
            })
        } else {
            None
        }
    }
}

/// Android resource strategy: shaping fonts from packaged APK assets.
///
/// Raster images are not modeled by this host yet, so the cache is cleared and
/// the resources drawer reports `images_modeled: false` rather than claiming a
/// summary it did not produce.
pub struct AndroidResources;

impl HostResources for AndroidResources {
    fn install_fonts(&self, core: &HostCore, view: &CadView) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            view.clear_fonts();
            return Ok(());
        }
        #[cfg(target_os = "android")]
        {
            let report = match super::android_fonts::install_fonts(Some(view), &requested) {
                Ok(report) => report,
                Err(error) => {
                    // Queue the explicit font-failure text before refusing, so the
                    // poll driver can surface it as the final status of the tick.
                    super::state_push::set_pending_font_status(
                        messages().text("fonts.failed", &[("error", &error.to_string())]),
                    );
                    return Err(error);
                }
            };
            // Preserve the historical status line: the font summary is shown
            // after the shared open status. The poll driver consumes this once
            // the core has finished its own status update for the tick.
            super::state_push::set_pending_font_status(messages().text(
                "android.fonts.summary",
                &[
                    ("catalog", &report.catalog_entries.to_string()),
                    ("requested", &requested.len().to_string()),
                    ("planned", &report.planned.len().to_string()),
                    ("registered", &report.registered.len().to_string()),
                    ("failed", &report.failed.len().to_string()),
                ],
            ));
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            // Assets exist only inside the activity; a host-target build cannot
            // resolve them. Explicit, never a silent empty font set.
            let _ = requested;
            let reason = "android asset fonts require the Android target";
            super::state_push::set_pending_font_status(
                messages().text("fonts.failed", &[("error", reason)]),
            );
            Err(CadError::Unsupported(reason.into()))
        }
    }

    fn install_images(&self, _core: &HostCore, view: &CadView) -> CadResult<()> {
        view.clear_images();
        Ok(())
    }
}
