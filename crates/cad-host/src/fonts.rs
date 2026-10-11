//! Desktop font/image installation and the plot font engine.
//!
//! Native only: the wasm host resolves resources through the browser stack (a
//! later phase) and never calls these, so the wasm stubs return an explicit
//! `Unsupported` rather than a silent no-op.
use super::*;

#[cfg(not(target_arch = "wasm32"))]
impl HostCore {
    /// (Re)load shaping fonts for the current drawing.
    ///
    /// Reads the sibling `fonts/` package (or `--fonts-dir`), merges explicit
    /// `--font` entries and installs a best-effort system default face as the
    /// first fallback.
    pub fn install_fonts(&self, view: &CadView) -> CadResult<()> {
        let requested = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
            .unwrap_or_default();
        match self.load_font_engine(&requested)? {
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
    pub fn install_images(&self, view: &CadView) -> CadResult<()> {
        let requested = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            view.clear_images();
            *self.last_image_report.borrow_mut() = None;
            return Ok(());
        }
        let drawing_dir = self
            .drawing_path
            .borrow()
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf);
        let (cache, report) = cad_platform::images::local::load_image_cache(
            &requested,
            drawing_dir.as_deref(),
            self.options.images_dir.as_deref(),
            &cad_resources::ResourceLimits::default(),
        )?;
        if cache.is_empty() {
            view.clear_images();
        } else {
            view.set_images(cache);
        }
        *self.last_image_report.borrow_mut() = Some(report);
        Ok(())
    }

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
    /// Not wired for this host yet; the wasm host resolves fonts through the
    /// browser stack in a later phase.
    pub fn install_fonts(&self, _view: &CadView) -> CadResult<()> {
        Err(CadError::Unsupported(
            "font loading is not wired for this host yet".into(),
        ))
    }

    /// Not wired for this host yet.
    pub fn install_images(&self, _view: &CadView) -> CadResult<()> {
        Err(CadError::Unsupported(
            "image loading is not wired for this host yet".into(),
        ))
    }

    /// Not wired for this host yet.
    pub(crate) fn load_plot_fonts(
        &self,
        _requested: &[String],
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        Err(CadError::Unsupported(
            "plot font loading is not wired for this host yet".into(),
        ))
    }
}
