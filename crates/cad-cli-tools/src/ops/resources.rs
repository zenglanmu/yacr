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
) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if entries.is_empty() && requested.is_empty() && !text_needed {
            return Ok(None);
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let font_dir = cad_platform::fonts::local::resolve_font_dir(None, &exe)?;
        cad_platform::fonts::local::load_engine(
            font_dir.as_deref(),
            requested,
            text_needed,
            entries,
            cad_platform::fonts::local::cached_system_default_candidates(),
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        if entries.is_empty() {
            return Ok(None);
        }
        let _ = requested;
        let _ = text_needed;
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
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn images_requested(controller: &HostController) -> Vec<String> {
    controller
        .drawing()
        .map(|drawing| cad_platform::images::requested_images(drawing.as_ref()))
        .unwrap_or_default()
}

/// Resolve and decode the drawing's raster images for `render`/`plot`.
///
/// Native only. Roots are the drawing's own directory first, then an explicit
/// `--images-dir`; a missing, unsafe or undecodable key is reported, never
/// substituted. When the drawing references no images this is an empty cache
/// and an empty report (a no-op).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_images(
    invocation: &CliInvocation,
    requested: &[String],
) -> CadResult<(
    Arc<cad_resources::DecodedImageCache>,
    cad_platform::images::ImageLoadReport,
)> {
    let drawing_dir = invocation
        .input
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty());
    cad_platform::images::local::load_image_cache(
        requested,
        drawing_dir,
        invocation.images_dir.as_deref(),
        &cad_resources::ResourceLimits::default(),
    )
}
