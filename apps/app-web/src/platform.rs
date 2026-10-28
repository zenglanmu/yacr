//! The four **platform contracts** the browser carrier supplies to the shared
//! CAD session (`cad_host::HostPlatform`), with no DOM and no GPU in sight.
//!
//! This is the platform half of ADR 0024 stage ⑧ phase 1: the desktop session
//! ([`cad_ui_core::session::CadSession`]) is constructible inside wasm, and these
//! are the adapters that make it constructible over a carrier that has **no
//! filesystem**, **no durable storage** and **no font/image package**. They are
//! target-independent on purpose — no `web-sys`, no `js_sys`, no wgpu — so the
//! whole contract is asserted by `cargo test -p app-web` on the host instead of
//! only being observable in a browser.
//!
//! The rule for every adapter below is the project's honesty rule: a capability
//! this carrier does not have is **refused with a stable reason** (or reported as
//! absent), never turned into a silent success, an empty `Ok`, or fabricated
//! state.
//!
//! | contract | browser behaviour |
//! |---|---|
//! | [`PlatformFiles`] | every open/save/write refuses with [`UNSUPPORTED_FILESYSTEM`]; `poll` is empty because every interaction refused; `refuses_source_overwrite` is `false` (no inode identity) |
//! | [`ConfigStore`] | in-memory only: readable for the rest of this session, **never** durable — a reload starts from `Absent` ([`MemoryConfigStore::PERSISTENT`] is `false`) |
//! | [`SurfaceSource`] | the metrics the host feeds it (CSS logical size + scale), `None` before the first report |
//! | [`HostResources`] | no shaping fonts and no raster images are installed: a document that requests **none** is a real no-op success, a document that requests some is refused with [`UNSUPPORTED_RESOURCES`] |
//!
//! ## What actually works today
//!
//! The compiled-in synthetic fixture (`fixtures/dwg/synthetic-four-lines.dwg`)
//! has no text and no raster image, so it opens through these adapters
//! unchanged — the command router, snapshots and ticks are live. A **real**
//! drawing that references text still opens (the open funnel treats a resource
//! install failure as a non-fatal `fonts.failed` status, never as a failed open),
//! but no shaping engine is installed for it: this phase makes **no claim** about
//! those glyphs, and any later step that needs shaping
//! (`ensure_fonts_for_text` on the first created text entity) refuses explicitly.
//! Images are not decoded at all. Closing that gap needs a real browser font
//! fetch / image decode strategy — deliberately **not** part of this phase.
//!
//! ## What is NOT wired here
//!
//! No export in this crate uses these adapters yet: the browser keeps the
//! existing `dom-canvas` viewer session (`crate::session`) as the fallback until
//! phase 2 replaces it behind the **same** exports (ADR 0024 stage ⑧). Nothing in
//! this module changes the current rendering path.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_domain::{CadError, CadResult};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostPlatform, HostResources};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};

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

/// A [`PlatformFiles`] over a carrier with no filesystem.
///
/// Every user-initiated file interaction refuses; nothing is faked. The source
/// target of the last successful install is remembered only so
/// [`PlatformFiles::refuses_source_overwrite`] can answer honestly (`false`: this
/// platform has no device+inode identity to compare, so it cannot prove a save
/// would overwrite its own source).
#[derive(Debug, Default)]
pub struct MemoryFiles {
    source: RefCell<Option<FileTarget>>,
}

impl MemoryFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// The target of the last successful open install, if any.
    ///
    /// Diagnostics only: it is a **label**, not a path, and no operation below
    /// resolves it.
    pub fn source(&self) -> Option<FileTarget> {
        self.source.borrow().clone()
    }

    /// The one refusal shape: a stable prefix, the refused operation, and why.
    fn refuse(operation: &str) -> CadError {
        CadError::Unsupported(format!(
            "{UNSUPPORTED_FILESYSTEM}: {operation}: this carrier has no filesystem; \
             a drawing arrives as bytes and nothing is written back"
        ))
    }
}

impl PlatformFiles for MemoryFiles {
    fn begin_open(&self) -> CadResult<()> {
        Err(Self::refuse("begin_open"))
    }

    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Err(Self::refuse("begin_open_target"))
    }

    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Err(Self::refuse("begin_save"))
    }

    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(Self::refuse("write_atomic"))
    }

    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        // No device+inode identity exists here, so the guard cannot be armed.
        // Answering `true` would refuse every save; `false` is the documented
        // web answer (see `cad_platform::PlatformFiles`).
        false
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }

    fn poll(&self) -> Vec<FileEvent> {
        // Every `begin_*` refused, so no interaction can ever be in flight: an
        // empty drain is the truth, not a swallowed event.
        Vec::new()
    }
}

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

/// The browser [`HostResources`] strategy: nothing is resolved.
///
/// The decision is the drawing's **own** request, read with the same pure
/// platform helpers the desktop strategy uses (`requested_fonts`,
/// `needs_text_shaping`, `requested_images`):
///
/// * a drawing that requests no face and needs no text shaping → a real no-op
///   (nothing is installed, nothing is claimed);
/// * a drawing that requests a face / needs shaping → [`UNSUPPORTED_RESOURCES`],
///   naming how many faces were requested. The open funnel reports this as a
///   `fonts.failed` status (a real diagnostic), so the drawing still opens and its
///   text is honestly unshaped;
/// * the same for raster images.
pub struct BrowserResources;

impl BrowserResources {
    /// The font decision, pure: `Ok` only when the drawing asks for nothing.
    pub fn fonts_decision(requested: &[String], text_needed: bool) -> CadResult<()> {
        if requested.is_empty() && !text_needed {
            return Ok(());
        }
        Err(CadError::Unsupported(format!(
            "{UNSUPPORTED_RESOURCES}: fonts: {} requested face(s), text_shaping={text_needed}; \
             no font package is wired for this carrier (text is drawn unshaped)",
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
        let (requested, text_needed) = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| {
                (
                    cad_platform::fonts::requested_fonts(drawing.as_ref()),
                    cad_platform::fonts::needs_text_shaping(drawing.as_ref()),
                )
            })
            .unwrap_or_default();
        Self::fonts_decision(&requested, text_needed)?;
        // Nothing requested: clear any previous document's engine so it can never
        // be presented as if this drawing had resolved it.
        render.set_fonts(None);
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
    files: Rc<MemoryFiles>,
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

    /// The file contract (its refusals are observable for diagnostics).
    pub fn files(&self) -> &MemoryFiles {
        &self.files
    }

    /// The in-memory config store.
    pub fn config(&self) -> &MemoryConfigStore {
        &self.config
    }

    /// Compose the host platform. `capabilities` is what this carrier advertises:
    /// a browser session that cannot run a platform file chooser from inside the
    /// session passes `open: false` (the page's own picker is a DOM capability,
    /// and its bytes arrive through the session's bytes entry).
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
