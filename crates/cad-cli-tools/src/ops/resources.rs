//! Font and raster-image resource loading for render/plot.

use super::*;

/// The font keys the open drawing references, for the sibling `fonts/` package
/// loader when no explicit `--font` was given. Native-only: `build-representation`
/// reports host-font-independent text and the wasm path never loads fonts.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fonts_requested(controller: &HostController) -> Vec<String> {
    controller
        .drawing()
        .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
        .unwrap_or_default()
}

/// Whether the drawing contains text that needs shaping even if it names no
/// font (a created dimension, or TEXT in a style with no STYLE entry).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn drawing_needs_text(controller: &HostController) -> bool {
    controller
        .drawing()
        .map(|drawing| cad_platform::fonts::needs_text_shaping(drawing.as_ref()))
        .unwrap_or(false)
}

/// The drawing's text locale, from its own declared code page.
///
/// `None` (no code page) is the Latin default, never a guessed CJK page.
pub(crate) fn drawing_locale(controller: &HostController) -> cad_platform::fonts::TextLocale {
    controller
        .drawing()
        .map(|drawing| cad_platform::fonts::TextLocale::from_code_page(drawing.code_page()))
        .unwrap_or_default()
}

/// Load the `--font` entries plus the sibling `fonts/` package into a shaping
/// engine, if any are needed.
///
/// Native hosts also auto-load the `fonts/` directory next to the executable
/// (`cad_platform::fonts::local`) and a system default fallback face, so a
/// packaged CLI shapes text whose font is missing. `text_needed` keeps a
/// text-bearing drawing engine-backed even when it names no font. Wasm keeps the
/// explicit `--font` registrations only.
pub(crate) fn load_fonts(
    entries: &[(String, PathBuf)],
    requested: &[String],
    text_needed: bool,
    locale: cad_platform::fonts::TextLocale,
) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if entries.is_empty() && requested.is_empty() && !text_needed {
            return Ok(None);
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let font_dir = cad_platform::fonts::local::resolve_font_dir(None, &exe)?;
        cad_platform::fonts::local::load_engine_report(
            font_dir.as_deref(),
            requested,
            text_needed,
            entries,
            cad_platform::fonts::local::cached_system_default_candidates(),
            locale,
        )
        .map(|(engine, _report)| engine)
    }
    #[cfg(target_arch = "wasm32")]
    {
        if entries.is_empty() {
            return Ok(None);
        }
        let _ = requested;
        let _ = text_needed;
        let _ = locale;
        let mut engine = cad_representation::FontEngine::new();
        let mut keys = Vec::new();
        for (key, path) in entries {
            let bytes = std::fs::read(path)
                .map_err(|e| CadError::InvalidInput(format!("font read failed: {e}")))?;
            engine.register(key, Arc::from(bytes.into_boxed_slice()))?;
            keys.push(key.clone());
        }
        // Any registered font can stand in for a missing one.
        engine.set_fallback(keys);
        Ok(Some(Arc::new(engine)))
    }
}

/// The raster-image keys the open drawing references, for the image loader.
///
/// File-based `Image` keys only: a mesh's base-colour texture is document state
/// (`cad-db`), decoded by
/// [`cad_platform::images::local::load_image_cache_for_document`] (ADR-0038 D10).
/// [`cad_platform::images::has_document_textures`] is the companion check for
/// "does this drawing have any image at all?".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn images_requested(controller: &HostController) -> Vec<String> {
    controller
        .drawing()
        .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
        .unwrap_or_default()
}

/// Resolve and decode every image the open drawing owns, for `render`/`plot`.
///
/// Native only. The drawing's **document textures** are decoded first straight
/// from `cad-db` (no filesystem access, ADR-0038 D10); the file-based roots are
/// then the drawing's own directory followed by an explicit `--images-dir`. A
/// missing, unsafe, undecodable or over-budget key is reported, never
/// substituted. When the drawing references neither a file image nor a document
/// texture this is an empty cache and an empty report (a no-op).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_images(
    invocation: &CliInvocation,
    controller: &HostController,
) -> CadResult<(
    Arc<cad_resources::DecodedImageCache>,
    cad_platform::images::ImageLoadReport,
)> {
    let requested = images_requested(controller);
    let drawing_dir = invocation
        .input
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty());
    let limits = cad_resources::ResourceLimits::default();
    match controller.drawing() {
        Some(drawing) => cad_platform::images::local::load_image_cache_for_document(
            drawing.as_ref(),
            &requested,
            drawing_dir,
            invocation.images_dir.as_deref(),
            &limits,
        ),
        // No open document: there is nothing a texture could belong to, so the
        // file-only pass is the same (empty-request) no-op.
        None => cad_platform::images::local::load_image_cache(
            &requested,
            drawing_dir,
            invocation.images_dir.as_deref(),
            &limits,
        ),
    }
}
