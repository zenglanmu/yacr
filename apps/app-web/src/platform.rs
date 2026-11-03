//! The four **platform contracts** the browser carrier supplies to the shared
//! CAD session (`cad_host::HostPlatform`), with no GPU and no rendering path in
//! sight.
//!
//! This is the platform half of ADR 0024 stage ⑧ phase 1: the desktop session
//! ([`cad_ui_core::session::CadSession`]) is constructible inside wasm, and these
//! are the adapters that make it constructible over a carrier that has **no
//! filesystem**, **no durable storage** and **no font/image package**. They are
//! target-independent on purpose — no `web-sys`, no `js_sys`, no wgpu — so the
//! whole contract is asserted by `cargo test -p app-web` on the host instead of
//! only being observable in a browser. (The one DOM dependency, the way a save
//! becomes a browser download, is isolated in [`crate::download`], whose wasm32 sink is
//! behind `cfg(target_arch = "wasm32")`.)
//!
//! The rule for every adapter below is the project's honesty rule: a capability
//! this carrier does not have is **refused with a stable reason** (or reported as
//! absent), never turned into a silent success, an empty `Ok`, or fabricated
//! state.
//!
//! | contract | browser behaviour |
//! |---|---|
//! | [`PlatformFiles`] | **save/plot are real downloads** ([`crate::download`]): `begin_save` synthesizes a `download://<name>` target, `write_atomic` publishes the finished bytes through the carrier's [`DownloadSink`](crate::download::DownloadSink) — a DOM download on wasm32, a recording sink on the host target; **open** still refuses with [`UNSUPPORTED_FILESYSTEM`] because the page's file picker owns selection and the bytes arrive through the session's `open_bytes` entry; `refuses_source_overwrite` is `false` (no inode identity) |
//! | [`ConfigStore`] | in-memory only: readable for the rest of this session, **never** durable — a reload starts from `Absent` ([`MemoryConfigStore::PERSISTENT`] is `false`) |
//! | [`SurfaceSource`] | the metrics the host feeds it (CSS logical size + scale), `None` before the first report |
//! | [`HostResources`] | the committed font package is installed from memory when the page staged it ([`crate::fonts`]); a drawing that requests nothing is a real no-op success, a drawing that requests faces before any package was staged is refused with [`UNSUPPORTED_RESOURCES`] (never a silent empty success) |
//!
//! ## What actually works today
//!
//! The compiled-in synthetic fixture (`fixtures/dwg/synthetic-four-lines.dwg`)
//! has no text and no raster image, so it opens through these adapters
//! unchanged — the command router, snapshots and ticks are live. A **real**
//! drawing that references text also opens; when the page staged the committed
//! font package through [`crate::set_font_package`], [`BrowserResources`] installs
//! the real shaping engine from that package ([`crate::fonts`]). Without a staged
//! package the open funnel reports a non-fatal `fonts.failed` status and the text
//! is honestly unshaped, and any later step that needs shaping
//! (`ensure_fonts_for_text` on the first created text entity) refuses explicitly.
//! Images are not decoded at all.
//!
//! ## What is wired, and what is not
//!
//! * **Save and plot are wired.** The session's own export funnel
//!   (`cad_host::save`) runs the lossy DXF export / vector plot and writes the
//!   bytes through [`BrowserFiles`] in [`crate::download`]. `begin_save` synthesizes the
//!   download target, so the whole export finishes inside the SAVE/PLOT click and
//!   the browser sink can publish it as a real download
//!   (`Blob` + `<a download>`). On a native build the same adapter records the
//!   download in memory instead (there is no page); that is a recorded write, not
//!   a silent success.
//! * **Open is not wired here.** A page has no picker this session can drive: the
//!   file input is the page's DOM capability, and the chosen bytes arrive through
//!   the session's `open_bytes` entry. `begin_open`/`begin_open_target` therefore
//!   refuse with [`UNSUPPORTED_FILESYSTEM`] rather than pretending to select a
//!   file.
//! * **Nothing here changes the rendering path.** The canvas still presents
//!   through the page carrier's WebGL2 presenter.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_domain::{CadError, CadResult};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostPlatform, HostResources};
use cad_platform::{ConfigLoad, ConfigStore, HostCapabilities, SurfaceMetrics, SurfaceSource};

use crate::download::{BrowserFiles, DownloadRecord, PlatformDownload};

/// Stable prefix of every file-operation refusal of the browser carrier.
///
/// A host that shows this string shows a real limitation: there is no filesystem
/// in the wasm module, and a drawing arrives as bytes.
pub const UNSUPPORTED_FILESYSTEM: &str = "unsupported-filesystem";

/// Stable prefix of the resource refusal (shaping fonts / raster images).
pub const UNSUPPORTED_RESOURCES: &str = "unsupported-resources";

/// Display path reported for the in-memory config / preference (never a real
/// file — it exists so a diagnostic can name *where* a value came from).
pub const MEMORY_CONFIG_PATH: &str = "memory://config";
pub const MEMORY_PREFERENCE_PATH: &str = "memory://preference";

/// The host-owned operations the browser carrier advertises to the session.
///
/// * `save` / `plot` are **true**: the session itself performs the export and
///   the [`crate::download`] adapter owns the download, so a `true` is a claim the
///   carrier really keeps.
/// * `open` / `new` are **false**: the file picker is the page's DOM capability,
///   not something this session can drive. The TS transport lifts `canOpen`
///   itself, and the chosen bytes arrive through the session's `open_bytes`
///   entry — the session never claims to have opened a chooser.
/// * `import_worker` is **false**: in wasm an open runs on the calling thread.
pub const BROWSER_CAPABILITIES: HostCapabilities = HostCapabilities {
    open: false,
    new: false,
    save: true,
    plot: true,
    import_worker: false,
};

/// An **in-memory**, explicitly non-persistent [`ConfigStore`].
///
/// Mirrors the Linux host's non-persistent store mode
/// (`ReactConfigStore::non_persistent`) with one deliberate difference: a write
/// is kept **in memory** so the rest of this session reads back what it wrote
/// (`load_config` / `load_preference` return it), instead of failing every
/// settings write. Nothing claims durability — [`Self::PERSISTENT`] is `false`,
/// and a fresh store (what a page reload produces) starts from
/// [`ConfigLoad::Absent`].
#[derive(Debug, Default)]
pub struct MemoryConfigStore {
    config: RefCell<Option<String>>,
    preference: RefCell<Option<String>>,
}

impl MemoryConfigStore {
    /// This store is never durable: values live for the lifetime of this object.
    pub const PERSISTENT: bool = false;

    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the in-memory host config document (a real in-memory write).
    pub fn store_config(&self, text: &str) {
        *self.config.borrow_mut() = Some(text.to_string());
    }

    /// The in-memory preference text, if anything was written this session.
    pub fn preference(&self) -> Option<String> {
        self.preference.borrow().clone()
    }
}

impl ConfigStore for MemoryConfigStore {
    fn load_config(&self) -> ConfigLoad {
        read_memory(&self.config, MEMORY_CONFIG_PATH)
    }

    fn load_preference(&self) -> ConfigLoad {
        read_memory(&self.preference, MEMORY_PREFERENCE_PATH)
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        *self.preference.borrow_mut() = Some(text.to_string());
        Ok(())
    }
}

fn read_memory(slot: &RefCell<Option<String>>, path: &str) -> ConfigLoad {
    match slot.borrow().as_ref() {
        Some(text) => ConfigLoad::Text {
            path: path.to_string(),
            text: text.clone(),
        },
        None => ConfigLoad::Absent,
    }
}

/// A [`SurfaceSource`] fed by the host: the CSS logical CAD rectangle, the
/// display scale and the safe-area insets.
///
/// `None` until the host reports the first real layout — an unmapped surface is
/// an explicit refusal downstream (`HostCore::metrics` → `Cancelled`), never a
/// fabricated canvas size. Cloning shares the same cell, so the DOM-side writer
/// and the session observe one value.
#[derive(Clone, Debug, Default)]
pub struct ReportedSurface {
    metrics: Rc<Cell<Option<SurfaceMetrics>>>,
}

impl ReportedSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// Report the CAD content rectangle (logical CSS px) and the display scale.
    ///
    /// Safe-area insets are explicit zeros: the browser's layout already excludes
    /// the host chrome, so there is no fabricated margin.
    pub fn report(&self, logical: [f64; 2], scale: f64) {
        self.report_with_insets(logical, scale, [0.0; 4]);
    }

    /// [`Self::report`] with explicit safe-area insets (Android gesture bar).
    pub fn report_with_insets(&self, logical: [f64; 2], scale: f64, safe_insets: [f64; 4]) {
        self.metrics.set(Some(SurfaceMetrics {
            logical,
            scale,
            safe_insets,
        }));
    }

    /// Forget the metrics (no mapped canvas any more).
    pub fn clear(&self) {
        self.metrics.set(None);
    }
}

impl SurfaceSource for ReportedSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        self.metrics.get()
    }
}

/// The browser [`HostResources`] strategy: the staged in-memory font package.
///
/// The decision is the drawing's **own** request, read with the same pure
/// platform helpers the desktop strategy uses (`requested_fonts`,
/// `needs_text_shaping`, `requested_images`):
///
/// * a package staged by [`crate::set_font_package`] → load the requested faces
///   (plus the catalog fallback and the drawing's locale face) from memory and
///   install the real engine ([`crate::fonts::load_package`]);
/// * no staged package and the drawing requests no face / needs no shaping → a
///   real no-op (nothing is installed, nothing is claimed);
/// * no staged package and the drawing requests a face / needs shaping →
///   [`UNSUPPORTED_RESOURCES`], naming how many faces were requested. The open
///   funnel reports this as a `fonts.failed` status (a real diagnostic), so the
///   drawing still opens and its text is honestly unshaped;
/// * the same for raster images (still no decoder).
pub struct BrowserResources;

impl BrowserResources {
    /// The font decision, pure: `Ok` only when the drawing asks for nothing.
    pub fn fonts_decision(requested: &[String], text_needed: bool) -> CadResult<()> {
        if requested.is_empty() && !text_needed {
            return Ok(());
        }
        Err(CadError::Unsupported(format!(
            "{UNSUPPORTED_RESOURCES}: fonts: {} requested face(s), text_shaping={text_needed}; \
             no font package was staged before this session started (text is drawn unshaped)",
            requested.len()
        )))
    }

    /// The raster-image decision, pure: `Ok` only when the drawing has none.
    pub fn images_decision(requested: &[String]) -> CadResult<()> {
        if requested.is_empty() {
            return Ok(());
        }
        Err(CadError::Unsupported(format!(
            "{UNSUPPORTED_RESOURCES}: images: {} referenced image(s); \
             no image decoder is wired for this carrier",
            requested.len()
        )))
    }
}

impl HostResources for BrowserResources {
    fn install_fonts(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        let (requested, text_needed, locale) = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| {
                (
                    cad_platform::fonts::requested_fonts(drawing.as_ref()),
                    cad_platform::fonts::needs_text_shaping(drawing.as_ref()),
                    cad_platform::fonts::TextLocale::from_code_page(drawing.code_page()),
                )
            })
            .unwrap_or((Vec::new(), false, cad_platform::fonts::TextLocale::Latin));
        let Some(package) = crate::fonts::current() else {
            // No package was staged before this session started: keep the
            // explicit refusal (never a silent empty success) and ensure a
            // previous document's engine is not presented as this one's.
            Self::fonts_decision(&requested, text_needed)?;
            render.set_fonts(None);
            core.clear_last_font_report();
            return Ok(());
        };
        let (engine, report) =
            crate::fonts::load_package(package.as_ref(), &requested, text_needed, locale)?;
        match engine {
            Some(engine) => {
                render.set_fonts(Some(engine));
            }
            None => {
                render.set_fonts(None);
            }
        }
        // The real per-face outcome, the same authority the desktop strategy
        // publishes. `HostCore::install_fonts` also retires a stale
        // `fonts.failed` status on a successful install, so the session's
        // stale-resource guard cannot clear the engine we just installed.
        core.set_last_font_report(report);
        Ok(())
    }

    fn install_images(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
            .unwrap_or_default();
        Self::images_decision(&requested)?;
        render.set_images(None);
        Ok(())
    }
}

/// The browser platform: the four contracts plus the surface cell the DOM writes.
///
/// One instance per session. [`Self::host_platform`] composes the
/// [`HostPlatform`] the session is built from, so a host never assembles the four
/// adapters itself (and cannot pass a mismatched surface).
#[derive(Clone, Default)]
pub struct BrowserPlatform {
    files: Rc<BrowserFiles<PlatformDownload>>,
    config: Rc<MemoryConfigStore>,
    surface: ReportedSurface,
}

impl BrowserPlatform {
    pub fn new() -> Self {
        Self::default()
    }

    /// The surface the host reports canvas metrics into.
    pub fn surface(&self) -> &ReportedSurface {
        &self.surface
    }

    /// The file contract: save/plot download targets plus the open refusal.
    pub fn files(&self) -> &BrowserFiles<PlatformDownload> {
        &self.files
    }

    /// Every successful export of this platform, in call order.
    ///
    /// Diagnostics/tests: on wasm32 these are downloads the browser sink
    /// published; on a native build they are the recording sink's entries.
    pub fn downloads(&self) -> Vec<DownloadRecord> {
        self.files.downloads()
    }

    /// The in-memory config store.
    pub fn config(&self) -> &MemoryConfigStore {
        &self.config
    }

    /// Compose the host platform. `capabilities` is what this carrier advertises:
    /// a browser session that cannot run a platform file chooser from inside the
    /// session passes `open: false` (the page's own picker is a DOM capability,
    /// and its bytes arrive through the session's bytes entry) while still
    /// advertising `save`/`plot`, which the session performs itself
    /// ([`BROWSER_CAPABILITIES`] is the production answer).
    pub fn host_platform(&self, capabilities: HostCapabilities) -> HostPlatform {
        HostPlatform {
            files: self.files.clone(),
            config_store: self.config.clone(),
            surface: Rc::new(self.surface.clone()),
            resources: Rc::new(BrowserResources),
            capabilities,
        }
    }
}

#[cfg(test)]
mod tests;
