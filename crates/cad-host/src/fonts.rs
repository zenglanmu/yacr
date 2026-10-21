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
    fn install_fonts(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()>;

    /// (Re)load the drawing's raster images onto `view`.
    fn install_images(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()>;
}

/// A strategy for a host that resolves resources through its own path (or does
/// not model them yet). Installing is refused explicitly, never a silent no-op.
pub struct UnsupportedResources;

impl HostResources for UnsupportedResources {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Err(CadError::Unsupported(
            "font loading is not wired for this host yet".into(),
        ))
    }

    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Err(CadError::Unsupported(
            "image loading is not wired for this host yet".into(),
        ))
    }
}

impl HostCore {
    /// (Re)load shaping fonts through the host strategy.
    pub fn install_fonts(&self) -> CadResult<()> {
        self.resources.install_fonts(self, &self.render)
    }

    /// (Re)load the drawing's raster images through the host strategy.
    pub fn install_images(&self) -> CadResult<()> {
        self.resources.install_images(self, &self.render)
    }

    /// Install fonts when a committed command introduced the drawing's first
    /// text after the open funnel already ran.
    ///
    /// Opening a drawing without text installs no engine (the optimisation), but
    /// creating its first dimension afterwards makes the measurement text
    /// shapeable: without this the new text stays an unshaped primitive and is
    /// not drawn. The check is a cheap `fonts().is_some()` first, so a drawing
    /// that already has an engine (or still has no text) is untouched.
    pub(crate) fn ensure_fonts_for_text(&self) -> CadResult<()> {
        if self.render.fonts().is_some() {
            return Ok(());
        }
        let text_needed = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::fonts::needs_text_shaping(drawing.as_ref()))
            .unwrap_or(false);
        if text_needed {
            self.install_fonts()?;
        }
        Ok(())
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
        let (engine, report) = core.load_font_engine(&requested, text_needed)?;
        match engine {
            Some(engine) => {
                render.set_fonts(Some(engine));
            }
            None => {
                render.set_fonts(None);
            }
        }
        // A locale with no available face is a named gap (stable code), never a
        // silent Latin substitution. Recorded once per open, deduplicated.
        core.record_font_diagnostics(&report);
        Ok(())
    }

    /// (Re)load the drawing's raster images for the current document.
    ///
    /// Resolves each logical key under the drawing's own directory (then an
    /// explicit `--images-dir`), decodes it through the shared platform loader
    /// and installs the cache on the view. An empty request clears the cache.
    /// Unresolved/undecodable keys are retained in `last_image_report` and
    /// surfaced by the diagnostics drawer, never silently dropped.
    fn install_images(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        let requested = core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
            .unwrap_or_default();
        if requested.is_empty() {
            render.set_images(None);
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
            render.set_images(None);
        } else {
            render.set_images(Some(cache));
        }
        *core.last_image_report.borrow_mut() = Some(report);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HostCore {
    /// The desktop font engine for the current drawing, plus its load report.
    ///
    /// `text_needed` keeps the optimisation honest: a drawing with no text and
    /// no named font installs nothing, while a text-bearing drawing with no
    /// font-bearing style still gets the fallback engine.
    ///
    /// The fallback chain follows the drawing's **own** text locale (derived
    /// from its code page). The installed engine is host-wide, so when it was
    /// built for a different locale the chain is retargeted instead of being
    /// ignored: parsed faces are reused when the new locale's face is already
    /// loaded, otherwise the faces are (re)loaded for the new locale.
    pub(crate) fn load_font_engine(
        &self,
        requested: &[String],
        text_needed: bool,
    ) -> CadResult<(
        Option<Arc<cad_representation::FontEngine>>,
        cad_platform::fonts::FontLoadReport,
    )> {
        use cad_platform::fonts::{FontLoadReport, TextLocale};
        let locale = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| TextLocale::from_code_page(drawing.code_page()))
            .unwrap_or(TextLocale::Latin);
        // Same locale and every named face already parsed: reuse the installed
        // engine instead of re-reading the package for every document.
        if let Some(existing) = self.render.fonts() {
            let covers = requested.iter().all(|key| existing.contains(key));
            // Reuse only when the installed chain already satisfies this
            // document's locale. A locale gap falls through to a reload so the
            // package is actually (re)tried for a document that needs text —
            // otherwise a text-less first open could freeze a CJK-less chain.
            if existing.locale_tag() == Some(locale.as_str())
                && covers
                && cad_platform::fonts::locale_gap(&existing, locale, text_needed).is_none()
            {
                let report = FontLoadReport {
                    locale,
                    ..FontLoadReport::default()
                };
                return Ok((Some(existing), report));
            }
            // The locale changed, but the new locale's preferred face is already
            // parsed: retarget the fallback order in place (faces are shared)
            // instead of rebuilding the engine. A Latin target is only
            // retargeted from an already-Latin chain: the loader's registration
            // order is not retained, so a Chinese chain cannot be restored to
            // the original Latin order without reloading.
            let retargetable = covers
                && match locale {
                    TextLocale::Latin => existing.locale_tag() == Some(TextLocale::Latin.as_str()),
                    _ => locale
                        .preferred_faces()
                        .iter()
                        .any(|name| existing.contains(name)),
                };
            if retargetable {
                let mut retargeted =
                    existing.with_fallback(locale.ordered_fallback(existing.fallback_keys()));
                retargeted.set_locale_tag(locale.as_str());
                let report = FontLoadReport {
                    locale,
                    locale_missing: cad_platform::fonts::locale_gap(
                        &retargeted,
                        locale,
                        text_needed,
                    ),
                    ..FontLoadReport::default()
                };
                return Ok((Some(Arc::new(retargeted)), report));
            }
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let font_dir =
            cad_platform::fonts::local::resolve_font_dir(self.options.fonts_dir.as_deref(), &exe)?;
        let system_default = cad_platform::fonts::local::cached_system_default_candidates();
        cad_platform::fonts::local::load_engine_report(
            font_dir.as_deref(),
            requested,
            text_needed,
            &self.options.fonts,
            system_default,
            locale,
        )
    }

    /// Surface font-load diagnostics on the document's diagnostic list.
    ///
    /// Existing import diagnostics are never removed; the same code+payload is
    /// only added once, so a post-commit `ensure_fonts_for_text` cannot turn one
    /// missing-locale-face report into a per-entity flood.
    fn record_font_diagnostics(&self, report: &cad_platform::fonts::FontLoadReport) {
        let diagnostics = report.diagnostics();
        if diagnostics.is_empty() {
            return;
        }
        let mut controller = self.controller.borrow_mut();
        let Some(import) = controller.last_import_report.as_mut() else {
            return;
        };
        for diagnostic in diagnostics {
            let duplicate = import.diagnostics.iter().any(|existing| {
                existing.code == diagnostic.code && existing.message == diagnostic.message
            });
            if !duplicate {
                import.diagnostics.push(diagnostic);
            }
        }
    }

    /// The plot font engine (the same desktop shaping engine).
    pub(crate) fn load_plot_fonts(
        &self,
        requested: &[String],
        text_needed: bool,
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        Ok(self.load_font_engine(requested, text_needed)?.0)
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
        text_needed: bool,
    ) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
        if requested.is_empty() && !text_needed {
            return Ok(None);
        }
        match self.render.fonts() {
            Some(engine) => Ok(Some(engine)),
            None => Err(CadError::Unsupported(
                self.message("web.fonts_unavailable", &[]),
            )),
        }
    }
}
