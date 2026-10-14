//! Android implementations of the `cad-platform` host contracts.
//!
//! `cad_host::HostCore` drives the open/save/plot funnels and the resource
//! installation through these traits; the Android host supplies the same
//! event-based shape the desktop does:
//!
//! * [`AndroidFiles`] runs the Storage Access Framework picker on Android
//!   (`ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT`) through the Java
//!   `NativeActivity` subclass and reads/writes document URIs through the
//!   `ContentResolver`; it queues the outcome as the corresponding [`FileEvent`].
//!   Off the Android target it falls back to a fixed sample-DWG path probe (the
//!   host-target test path) and save is an explicit `Unsupported`.
//! * [`AndroidConfigStore`] has no durable store: nothing to load, and persisting
//!   is refused rather than silently succeeding.
//! * [`AndroidSurface`] reports the surface the Activity forwarded through
//!   `set_surface_size`/`set_surface_insets`.
//! * [`AndroidResources`] installs fonts from the APK assets; raster images are
//!   not modeled by this host yet.
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostResources};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_slint::MessageSource;

/// The Android build's catalog locale. The shell is started `zh-CN` and the host
/// does not expose a runtime language switch, so platform-localized reasons use
/// the same fixed locale.
pub(crate) fn messages() -> MessageSource {
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
/// On the Android target `begin_open`/`begin_save` launch the SAF picker
/// (`ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT`) through the Java
/// `NativeActivity` subclass and `poll` reads the outcome, resolving a document
/// URI's bytes via the `ContentResolver`. The picker is asynchronous: the queue
/// stays empty until the user chooses, and the shared core's poll timer drains it
/// on a later tick.
///
/// Off the Android target (the host-target test build) `begin_open` probes the
/// configured sample paths synchronously and queues the result immediately; save
/// is an explicit `Unsupported`.
pub struct AndroidFiles {
    /// Legacy sample-DWG probe paths (host-target fallback only).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    sample_paths: Vec<String>,
    events: RefCell<Vec<FileEvent>>,
    /// The successfully-opened source target (armed by the core only on success).
    source: RefCell<Option<FileTarget>>,
    /// The SAF interaction currently in flight, if any (Android only).
    #[cfg(target_os = "android")]
    armed: Cell<Option<FilePurpose>>,
}

impl AndroidFiles {
    pub fn new(sample_paths: Vec<String>) -> Self {
        Self {
            sample_paths,
            events: RefCell::new(Vec::new()),
            source: RefCell::new(None),
            #[cfg(target_os = "android")]
            armed: Cell::new(None),
        }
    }
}

impl PlatformFiles for AndroidFiles {
    fn begin_open(&self) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            // Only one SAF interaction at a time; a second one would overwrite the
            // armed purpose and cross-wire the results.
            if self.armed.get().is_some() {
                return Err(CadError::Unsupported(messages().text("linux.busy", &[])));
            }
            self.armed.set(Some(FilePurpose::Open));
            if let Err(error) = super::saf::launch(false, None) {
                self.armed.set(None);
                return Err(CadError::Unsupported(
                    messages().text("android.saf_unavailable", &[("error", &error)]),
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            // Host-target fallback: probe a fixed list of sample DWG paths and
            // queue the bytes (or an explicit failure). Never a silent no-op.
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
    }

    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()> {
        let bytes = self.read_target(target)?;
        self.events.borrow_mut().push(FileEvent::Opened {
            target: target.clone(),
            bytes: Arc::from(bytes.into_boxed_slice()),
        });
        Ok(())
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            if self.armed.get().is_some() {
                return Err(CadError::Unsupported(messages().text("linux.busy", &[])));
            }
            self.armed.set(Some(FilePurpose::Save(purpose)));
            if let Err(error) = super::saf::launch(true, Some(default_name)) {
                self.armed.set(None);
                return Err(CadError::Unsupported(
                    messages().text("android.saf_unavailable", &[("error", &error)]),
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (purpose, default_name);
            // The host-target fallback has no writer; an explicit refusal, never a
            // no-op. The Android build wires SAF save through `saf::launch`.
            Err(CadError::Unsupported(
                messages().text("android.save_unavailable", &[]),
            ))
        }
    }

    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        #[cfg(target_os = "android")]
        {
            super::saf::write_uri(&target.opaque, bytes).map_err(|error| {
                CadError::InvalidInput(
                    messages().text("android.saf_write_failed", &[("error", &error)]),
                )
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (target, bytes);
            Err(CadError::Unsupported(
                messages().text("android.save_unavailable", &[]),
            ))
        }
    }

    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        // Compare the platform handle (a SAF content URI on Android): Android has
        // no filesystem identity to offer, so the opaque string is the identity.
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
        self.drain_saf();
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

impl AndroidFiles {
    /// Read a target's bytes: a `content://` URI on Android, else a filesystem
    /// path. A read failure is an explicit, localized error.
    fn read_target(&self, target: &FileTarget) -> CadResult<Vec<u8>> {
        #[cfg(target_os = "android")]
        if target.opaque.starts_with("content://") {
            return super::saf::read_uri(&target.opaque).map_err(|error| {
                CadError::InvalidInput(messages().text(
                    "android.read_failed",
                    &[("path", &target.display_name), ("error", &error)],
                ))
            });
        }
        std::fs::read(std::path::Path::new(&target.opaque)).map_err(|error| {
            CadError::InvalidInput(messages().text(
                "android.read_failed",
                &[
                    ("path", &target.display_name),
                    ("error", &error.to_string()),
                ],
            ))
        })
    }

    /// Drain the in-flight SAF interaction into the event queue (Android only).
    #[cfg(target_os = "android")]
    fn drain_saf(&self) {
        let Some(purpose) = self.armed.get() else {
            return;
        };
        let result = super::saf::poll(purpose);
        // `None` means the picker is still open: keep the armed purpose.
        if let Some(event) = saf_event(purpose, result) {
            self.armed.set(None);
            self.events.borrow_mut().push(event);
        }
    }
}

/// A completed (or still-pending) SAF interaction, independent of the Android
/// JNI layer so the purpose→event mapping is unit-testable on the host target.
#[cfg(any(target_os = "android", test))]
pub(crate) enum SafResult {
    /// The picker is still open.
    Pending,
    /// The user dismissed the picker.
    Cancelled,
    /// The interaction failed; `detail` is a raw (untranslated) reason.
    Failed(String),
    /// An open target was chosen; its bytes have been read.
    Opened {
        uri: String,
        name: String,
        bytes: Vec<u8>,
    },
    /// A save target was chosen; the URI is the opaque handle.
    SaveTarget { uri: String, name: String },
}

/// Map an armed purpose and a SAF result to the event to queue.
///
/// `None` means "still pending, queue nothing". A result whose kind does not
/// match the armed purpose is an explicit [`FileEvent::Failed`] — never a
/// fabricated open/save (an open-chosen URI must never be written as a DXF).
#[cfg(any(target_os = "android", test))]
pub(crate) fn saf_event(purpose: FilePurpose, result: SafResult) -> Option<FileEvent> {
    let fail = |detail: &str| {
        Some(FileEvent::Failed {
            purpose,
            reason: messages().text("android.saf_failed", &[("error", detail)]),
        })
    };
    match (purpose, result) {
        (_, SafResult::Pending) => None,
        (_, SafResult::Cancelled) => Some(FileEvent::Cancelled { purpose }),
        (_, SafResult::Failed(detail)) => fail(&detail),
        (FilePurpose::Open, SafResult::Opened { uri, name, bytes }) => Some(FileEvent::Opened {
            target: FileTarget {
                display_name: name,
                opaque: uri,
            },
            bytes: Arc::from(bytes.into_boxed_slice()),
        }),
        (FilePurpose::Save(save), SafResult::SaveTarget { uri, name }) => {
            Some(FileEvent::SaveTarget {
                purpose: save,
                target: FileTarget {
                    display_name: name,
                    opaque: uri,
                },
            })
        }
        (FilePurpose::Open, SafResult::SaveTarget { .. }) => {
            fail("the picker returned a save target for an open request")
        }
        (FilePurpose::Save(_), SafResult::Opened { .. }) => {
            fail("the picker returned an open target for a save request")
        }
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
    fn install_fonts(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            render.set_fonts(None);
            return Ok(());
        }
        #[cfg(target_os = "android")]
        {
            let report = match super::android_fonts::install_fonts(render, &requested) {
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

    fn install_images(&self, _core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        render.set_images(None);
        Ok(())
    }
}
