//! Host resource installation: shaping fonts and raster images.
//!
//! The shared open funnel is platform-general, but *how* a drawing's resources
//! are resolved is not: the desktop reads a sibling `fonts/`/`images/` package
//! plus a system default face, Android reads APK assets, the browser fetches over
//! the network. [`HostResources`] is that platform strategy; the core never
//! imports a platform type.
use super::*;

use cad_platform::NamedFileError;
use std::path::Path;

/// How a host installs the shaping fonts and raster images a drawing references,
/// and reads a resource file the user names on the CAD command line.
///
/// The core calls the two installers from the open funnel ([`HostCore::deliver_open`]
/// and the asynchronous publish in `poll_loading`); a host supplies its strategy
/// when it builds the [`HostCore`]. A strategy that cannot resolve a resource must
/// refuse explicitly (`Unsupported`) rather than silently no-op.
pub trait HostResources {
    /// (Re)load the shaping fonts the current drawing references onto `view`.
    fn install_fonts(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()>;

    /// (Re)load the drawing's raster images onto `view`.
    fn install_images(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()>;

    /// Read one **user-named** host file, outside the drawing open/save funnels.
    ///
    /// This is the one channel a typed command has to a path the user typed
    /// (`HATCHEDIT PATTERNFILE <path.pat>`): the command line is parsed in the
    /// session, which must stay filesystem-free (it is also compiled to wasm), so
    /// the strategy that already owns native file access supplies the bytes.
    ///
    /// The default refuses with [`NamedFileError::Unavailable`]: a runtime with no
    /// filesystem (the browser) must say so instead of answering "missing file".
    /// Native hosts override this and read through `cad_platform::read_named_file`.
    fn read_named_file(&self, path: &Path) -> Result<Arc<[u8]>, NamedFileError> {
        let _ = path;
        Err(NamedFileError::Unavailable)
    }

    /// Write one **user-named** host file, the write counterpart of
    /// [`Self::read_named_file`] (`TABLEEXPORT <path.csv>`).
    ///
    /// The same rule applies in the other direction: the session owns the typed
    /// grammar and the bytes, the strategy owns the filesystem, and a runtime
    /// without one refuses with [`NamedFileError::Unavailable`] rather than
    /// reporting success for a file nobody wrote. Native hosts override this and
    /// write atomically through `cad_platform::write_named_file`.
    fn write_named_file(&self, path: &Path, bytes: &[u8]) -> Result<(), NamedFileError> {
        let _ = (path, bytes);
        Err(NamedFileError::Unavailable)
    }
}

/// The host status key for a font/image installation failure.
///
/// One stable key for both resource kinds: the UI localises by key, so the
/// budget/failure text never leaks platform prose. A successful font install
/// retires a stale occurrence of this key (see [`HostCore::install_fonts`]).
const RESOURCE_FAILED_STATUS: &str = "fonts.failed";

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
    ///
    /// A **successful** install supersedes an earlier `fonts.failed` status: that
    /// status is what the session's stale-resource guard watches, so leaving it
    /// behind after a later document's fonts loaded would clear the freshly
    /// installed engine and keep `cad_status.error` stale. A failed install keeps
    /// its explicit status (the strategy or the open funnel records it).
    pub fn install_fonts(&self) -> CadResult<()> {
        let outcome = self.resources.install_fonts(self, &self.render);
        if outcome.is_ok() {
            self.clear_stale_font_failure();
        }
        outcome
    }

    /// Replace a stale `fonts.failed` catalog status after a successful load.
    ///
    /// Only that one key is retired: a command/open failure recorded by another
    /// path is a different fact and stays visible.
    fn clear_stale_font_failure(&self) {
        let stale = matches!(
            self.last_status(),
            Some(StatusMessage::Catalog { ref key, .. }) if key == RESOURCE_FAILED_STATUS
        );
        if stale {
            self.clear_last_status();
        }
    }

    /// The last real shaping-font load outcome for the **open document**, if any.
    ///
    /// Public so a host outside this crate (the `dom-canvas` browser host
    /// resolves its own fonts) can publish the report it actually loaded. A
    /// document replacement clears it; the engine-scoped rows survive internally
    /// so a reused engine can republish them ([`Self::reused_font_report`]).
    pub fn last_font_report(&self) -> Option<cad_platform::fonts::FontLoadReport> {
        self.last_font_report.borrow().clone()
    }

    /// Publish the real shaping-font load outcome (see [`Self::last_font_report`]).
    ///
    /// This is the **document-scoped** projection; the engine-scoped copy that a
    /// reused engine republishes is written only by a real load.
    pub fn set_last_font_report(&self, report: cad_platform::fonts::FontLoadReport) {
        *self.last_font_report.borrow_mut() = Some(report);
    }

    /// Remember the real load that produced the installed engine.
    #[cfg(not(target_arch = "wasm32"))]
    fn set_engine_font_report(&self, report: cad_platform::fonts::FontLoadReport) {
        *self.engine_font_report.borrow_mut() = Some(report);
    }

    /// Drop the published font report (document replaced/closed).
    pub fn clear_last_font_report(&self) {
        *self.last_font_report.borrow_mut() = None;
        *self.engine_font_report.borrow_mut() = None;
    }

    /// Clear the document-scoped report only, keeping the installed engine's own
    /// real load for reuse after the replacement.
    pub(crate) fn clear_document_font_report(&self) {
        *self.last_font_report.borrow_mut() = None;
    }

    /// (Re)load the drawing's raster images through the host strategy.
    ///
    /// The requested **normalised key set** is recorded after the attempt so a
    /// later committed edit can cheaply tell whether anything changed; an
    /// unchanged set is never decoded again by [`Self::sync_images_if_changed`].
    pub fn install_images(&self) -> CadResult<()> {
        let outcome = self.resources.install_images(self, &self.render);
        *self.last_image_keys.borrow_mut() = requested_image_keys(self);
        outcome
    }

    /// Reinstall the drawing's images after a committed edit **iff** the set of
    /// referenced logical image keys changed.
    ///
    /// This is the `IMAGEATTACH`-without-reopen path: the open funnel installs
    /// the resources once, but a command that adds, removes or retargets an image
    /// entity changes the request set, and the installed [`DecodedImageCache`]
    /// would otherwise be stale until the drawing was reopened. The check is a
    /// set comparison only; an unchanged set does no decoding.
    pub(crate) fn sync_images_if_changed(&self) -> CadResult<()> {
        let requested = requested_image_keys(self);
        if !image_keys_changed(&self.last_image_keys.borrow(), &requested) {
            return Ok(());
        }
        self.install_images()
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

/// The normalised image-key set the current drawing references.
///
/// Two sources, both normalised to the identity the decoded cache stores with
/// [`cad_resources::ResourceKey`]:
///
/// * file-based `Image` keys go through [`cad_resources::image_resource_key`]
///   (an unsafe key that yields no normalised form is kept verbatim so a change
///   from one unsafe reference to another is still noticed);
/// * the document's own mesh textures (ADR-0038 D3) are already canonical
///   content keys, so they enter as `ResourceKey(key)`.
///
/// A document with only mesh textures must still produce a non-empty set: it is
/// the set that decides whether the cache is (re)installed after a committed
/// edit.
fn requested_image_keys(core: &HostCore) -> std::collections::BTreeSet<cad_resources::ResourceKey> {
    core.controller
        .borrow()
        .drawing()
        .map(|drawing| {
            let mut keys: std::collections::BTreeSet<cad_resources::ResourceKey> =
                cad_platform::images::requested_images(drawing.as_ref())
                    .into_iter()
                    .map(|key| {
                        cad_resources::image_resource_key(&key)
                            .unwrap_or(cad_resources::ResourceKey(key))
                    })
                    .collect();
            keys.extend(
                drawing
                    .textures()
                    .map(|(key, _)| cad_resources::ResourceKey(key.to_string())),
            );
            keys
        })
        .unwrap_or_default()
}

/// Whether a committed edit changed the drawing's image-key set.
///
/// Pure and order-independent: the caller only reinstalls when this is `true`,
/// so an unchanged set never triggers a decode pass.
pub(crate) fn image_keys_changed(
    installed: &std::collections::BTreeSet<cad_resources::ResourceKey>,
    requested: &std::collections::BTreeSet<cad_resources::ResourceKey>,
) -> bool {
    installed != requested
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
                // TXTEXP outlines with the same faces the display shapes with.
                // The first installation wins, so every later document reuses the
                // already-loaded engine instead of changing typefaces mid-session.
                cad_app::install_outline_fonts(engine.clone());
                render.set_fonts(Some(engine));
            }
            None => {
                render.set_fonts(None);
            }
        }
        // Keep the real per-face outcome, including a drawing with no text and no
        // named font (that report is the honest "nothing was needed/loaded", not
        // a stale previous document's rows). `load_font_engine` already published
        // it on every path; this keeps the strategy self-contained.
        core.set_last_font_report(report.clone());
        // A locale with no available face is a named gap (stable code), never a
        // silent Latin substitution. Recorded once per open, deduplicated.
        core.record_font_diagnostics(&report);
        Ok(())
    }

    /// (Re)load the drawing's raster images for the current document.
    ///
    /// Document-owned mesh textures are decoded first straight from `cad-db`
    /// (ADR-0038 D10: no filesystem access, same `ResourceLimits`); the file
    /// references are then resolved under the drawing's own directory (then an
    /// explicit `--images-dir`) and decoded through the shared platform loader.
    /// The cache is installed on the view. A texture-only drawing still installs
    /// one — the "no images at all" early return requires **both** sources to be
    /// empty. Unresolved/undecodable keys (either source) are retained in
    /// `last_image_report` and surfaced by the diagnostics drawer, never silently
    /// dropped and never replaced by a placeholder.
    fn install_images(&self, core: &HostCore, render: &RenderSessionHandle) -> CadResult<()> {
        let drawing = core.controller.borrow().drawing();
        let Some(drawing) = drawing else {
            render.set_images(None);
            *core.last_image_report.borrow_mut() = None;
            return Ok(());
        };
        let requested = cad_platform::images::requested_images(drawing.as_ref());
        let owns_textures = cad_platform::images::has_document_textures(drawing.as_ref());
        if requested.is_empty() && !owns_textures {
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
        let (cache, report) = cad_platform::images::local::load_image_cache_for_document(
            drawing.as_ref(),
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

    /// Read one user-named host file (a `.pat` pattern library named on the
    /// command line) through the platform carrier.
    ///
    /// The path is read exactly as typed. The policy (a regular file, the
    /// [`cad_platform::NAMED_FILE_MAX_BYTES`] bound) lives in
    /// `cad_platform::read_named_file`, so the same rules apply to every native
    /// host and a browser cannot claim them.
    fn read_named_file(&self, path: &Path) -> Result<Arc<[u8]>, NamedFileError> {
        cad_platform::read_named_file(path)
    }

    /// Write one user-named host file (a `TABLEEXPORT <path.csv>` destination)
    /// through the platform carrier.
    ///
    /// The policy (an existing parent directory, the size bound, an atomic
    /// temp-file rename) lives in `cad_platform::write_named_file`, so every native
    /// host writes exactly the same way and a browser cannot claim a write.
    fn write_named_file(&self, path: &Path, bytes: &[u8]) -> Result<(), NamedFileError> {
        cad_platform::write_named_file(path, bytes)
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
        use cad_platform::fonts::TextLocale;
        let locale = self
            .controller
            .borrow()
            .drawing()
            .map(|drawing| TextLocale::from_code_page(drawing.code_page()))
            .unwrap_or(TextLocale::Latin);
        // A drawing that needs no shaping at all has no face rows to report. The
        // installed engine (if any) is left untouched — it is host-wide — but the
        // document's report is the explicit empty one, mirroring the empty-request
        // image install and the platform loader's own early return. An explicit
        // `--font` registration still loads (it is host configuration, not a
        // drawing reference), so it falls through.
        if requested.is_empty() && self.options.fonts.is_empty() && !text_needed {
            let report = cad_platform::fonts::FontLoadReport {
                locale,
                ..cad_platform::fonts::FontLoadReport::default()
            };
            self.set_last_font_report(report.clone());
            return Ok((self.render.fonts(), report));
        }
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
                let report = self.reused_font_report(locale, &existing, text_needed);
                self.set_last_font_report(report.clone());
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
                let report = self.reused_font_report(locale, &retargeted, text_needed);
                self.set_last_font_report(report.clone());
                return Ok((Some(Arc::new(retargeted)), report));
            }
        }
        let exe = std::env::current_exe().unwrap_or_default();
        // `--fonts-dir` keeps its hard-error semantics (a missing directory
        // refuses the load). A persisted `ui.paths.fontsDir` is only handed over
        // while it still names a directory: a stale one falls through to
        // discovery here instead of failing every open (the preference itself is
        // kept and diagnosed at startup — see `settings::paths`).
        let font_dir = cad_platform::fonts::local::resolve_font_dir(
            self.explicit_font_dir().as_deref(),
            &exe,
        )?;
        let system_default = cad_platform::fonts::local::cached_system_default_candidates();
        let loaded = cad_platform::fonts::local::load_engine_report(
            font_dir.as_deref(),
            requested,
            text_needed,
            &self.options.fonts,
            system_default,
            locale,
        );
        if let Ok((_, report)) = &loaded {
            self.set_engine_font_report(report.clone());
            self.set_last_font_report(report.clone());
        }
        loaded
    }

    /// The report a **reused/retargeted** engine publishes.
    ///
    /// The installed chain was not reloaded, so the real per-face rows are the
    /// ones the host stored when it built it. Only the document-specific facts are
    /// refreshed: the locale the chain now serves and the locale gap for *this*
    /// document's text (a reused CJK-less chain still reports the missing face).
    fn reused_font_report(
        &self,
        locale: cad_platform::fonts::TextLocale,
        engine: &cad_representation::FontEngine,
        text_needed: bool,
    ) -> cad_platform::fonts::FontLoadReport {
        // The engine-scoped copy is the real load that built the installed
        // chain; the document-scoped copy is the fallback for a host that only
        // published a report (for example the browser carrier).
        let mut report = self
            .engine_font_report
            .borrow()
            .clone()
            .or_else(|| self.last_font_report())
            .unwrap_or_default();
        report.locale = locale;
        report.locale_missing = cad_platform::fonts::locale_gap(engine, locale, text_needed);
        report
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

#[cfg(test)]
mod tests {
    use super::*;
    use cad_resources::image_resource_key;

    fn keys(references: &[&str]) -> std::collections::BTreeSet<cad_resources::ResourceKey> {
        references
            .iter()
            .map(|reference| image_resource_key(reference).unwrap())
            .collect()
    }

    #[test]
    fn image_keys_changed_only_when_the_reference_set_differs() {
        let empty = keys(&[]);
        let one = keys(&["textures/logo.png"]);
        assert!(!image_keys_changed(&empty, &keys(&[])));
        assert!(!image_keys_changed(&one, &keys(&["textures/LOGO.PNG"])));
        assert!(image_keys_changed(&empty, &one));
        assert!(image_keys_changed(&one, &empty));
        assert!(image_keys_changed(&one, &keys(&["textures/other.png"])));
    }
}
