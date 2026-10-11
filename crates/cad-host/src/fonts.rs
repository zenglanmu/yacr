//! Host resource installation: shaping fonts and raster images.
//!
//! The shared open funnel is platform-general, but *how* a drawing's resources
//! are resolved is not: the desktop reads a sibling `fonts/`/`images/` package
//! plus a system default face, Android reads APK assets, the browser fetches over
//! the network. [`HostResources`] is that platform strategy; the core never
//! imports a platform type.
use super::*;

/// How a host installs the shaping fonts and raster images a drawing references.
///
/// The core calls these from the open funnel ([`HostCore::deliver_open`] and the
/// asynchronous publish in `poll_loading`); a host supplies its strategy when it
/// builds the [`HostCore`]. A strategy that cannot resolve a resource must
/// refuse explicitly (`Unsupported`) rather than silently no-op.
pub trait HostResources {
    /// (Re)load the shaping fonts the current drawing references onto `view`.
    fn install_fonts(&self, core: &HostCore, view: &CadView) -> CadResult<()>;

    /// (Re)load the drawing's raster images onto `view`.
    fn install_images(&self, core: &HostCore, view: &CadView) -> CadResult<()>;
}

/// A strategy for a host that resolves resources through its own path (or does
/// not model them yet). Installing is refused explicitly, never a silent no-op.
pub struct UnsupportedResources;

impl HostResources for UnsupportedResources {
    fn install_fonts(&self, _core: &HostCore, _view: &CadView) -> CadResult<()> {
        Err(CadError::Unsupported(
            "font loading is not wired for this host yet".into(),
        ))
    }

    fn install_images(&self, _core: &HostCore, _view: &CadView) -> CadResult<()> {
        Err(CadError::Unsupported(
            "image loading is not wired for this host yet".into(),
        ))
    }
}

impl HostCore {
    /// (Re)load shaping fonts for the current drawing through the host strategy.
    pub fn install_fonts(&self, view: &CadView) -> CadResult<()> {
        self.resources.install_fonts(self, view)
    }

    /// (Re)load the drawing's raster images through the host strategy.
    pub fn install_images(&self, view: &CadView) -> CadResult<()> {
        self.resources.install_images(self, view)
    }
}

/// Native desktop resource strategy: the sibling `fonts/`/`images/` package.
///
/// Not available on wasm (no filesystem); the browser resolves its own resources.
#[cfg(not(target_arch = "wasm32"))]
pub struct DesktopResources;

#[cfg(not(target_arch = "wasm32"))]
impl HostResources for DesktopResources {
    /// (Re)load shaping fonts for the current drawing.
    ///
    /// Reads the sibling `fonts/` package (or `--fonts-dir`), merges explicit
    /// `--font` entries and installs a best-effort system default face as the
    /// first fallback.
    fn install_fonts(&self, core: &HostCore, view: &CadView) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
            .unwrap_or_default();
        match core.load_font_engine(&requested)? {
            Some(engine) => view.set_fonts(engine),
            None => view.clear_fonts(),
        }
        Ok(())
    }

    /// (Re)load the drawing's raster images for the current document.
    ///
    /// Resolves each logical key under the drawing's own directory (then an
    /// explicit `--images-dir`), decodes it through the shared platform loader
    /// and installs the cache on the view. An empty request clears the cache.
    /// Unresolved/undecodable keys are retained in `last_image_report` and
    /// surfaced by the diagnostics drawer, never silently dropped.
    fn install_images(&self, core: &HostCore, view: &CadView) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            view.clear_images();
            *core.last_image_report.borrow_mut() = None;
            return Ok(());
        }
        let drawing_dir = core
            .drawing_path
            .borrow()
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf);
        let (cache, report) = cad_platform::images::local::load_image_cache(
            &requested,
            drawing_dir.as_deref(),
            core.options.images_dir.as_deref(),
            &cad_resources::ResourceLimits::default(),
        )?;
        if cache.is_empty() {
            view.clear_images();
        } else {
            view.set_images(cache);
        }
        *core.last_image_report.borrow_mut() = Some(report);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HostCore {
    /// The desktop font engine for the current drawing, or `None` when nothing
    /// is requested/registered.
    pub(crate) fn load_font_engine(
        &self,
        requested: &[String],
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        let exe = std::env::current_exe().unwrap_or_default();
        let font_dir =
            cad_platform::fonts::local::resolve_font_dir(self.options.fonts_dir.as_deref(), &exe)?;
        let system_default = cad_platform::fonts::local::cached_system_default_candidates();
        cad_platform::fonts::local::load_engine(
            font_dir.as_deref(),
            requested,
            &self.options.fonts,
            system_default,
        )
    }

    /// The plot font engine (the same desktop shaping engine).
    pub(crate) fn load_plot_fonts(
        &self,
        requested: &[String],
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        self.load_font_engine(requested)
    }
}

#[cfg(target_arch = "wasm32")]
impl HostCore {
    /// The browser shaping engine already installed on the view.
    ///
    /// The web host fetches and registers fonts asynchronously (`browser::fonts`
    /// → `view.set_fonts`) after an open; plotting reuses that exact engine so
    /// text is shaped the same way the canvas shapes it, rather than re-fetching.
    /// A drawing that references text fonts before they are installed is an
    /// explicit refusal, never a plot with missing glyphs.
    pub(crate) fn load_plot_fonts(
        &self,
        requested: &[String],
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        if requested.is_empty() {
            return Ok(None);
        }
        match self.view.borrow().as_ref().and_then(|view| view.fonts()) {
            Some(engine) => Ok(Some(engine)),
            None => Err(CadError::Unsupported(
                self.message("web.fonts_unavailable", &[]),
            )),
        }
    }
}
