//! Catalog-driven host font loading: plan → fetch → register → fallback.
//!
//! Spec v2.0 §7.3, §10: the core only builds logical keys and catalog URLs
//! (`cad-resources`); the host fetches the bytes through [`FontLoader`] and
//! registers them with the shaping engine (`cad-representation`). This module
//! holds the parts both browser and Android hosts share so the two platforms
//! cannot drift apart:
//!
//! 1. [`requested_fonts`] reads, read-only, the font keys a drawing references.
//! 2. [`load_font_engine`] fetches `fonts.json`, resolves those keys with
//!    `plan_fonts`, fetches each file and registers it with its encoding, then
//!    installs every registered key as a fallback.
//!
//! Fetching is the only host-specific step; it is expressed as [`FontLoader`],
//! whose future may resolve on a browser network stack or an Android asset
//! read. Unknown names are reported and may use explicit catalog fallbacks (never aliases).

use std::sync::Arc;

use std::collections::HashSet;

use cad_domain::{CadError, CadResult, Diagnostic};
use cad_representation::FontEngine;
use cad_resources::{face_url, plan_fonts, plan_fonts_report, FontCatalog};

use super::{FontLoader, HostFuture};

mod locale;

pub use locale::{locale_gap, LocaleFallbackGap, TextLocale};

/// Reserved engine key for a host-supplied default fallback face.
///
/// A desktop host registers its system default font under this key so a
/// drawing font that is missing falls back to the platform default; see
/// [`register_default_face`].
pub const DEFAULT_FALLBACK_KEY: &str = "__yacr_default__";

/// Catalogue names tried, in order, for the last-resort default outline face.
///
/// `osifont` is the committed QCAD ISO 3098 face (web bundles ship it);
/// `arial`/`simplex` mirror the explicit catalog fallbacks below so a CDN-only
/// build still has a usable default when the drawing's own font is missing.
pub const DEFAULT_FALLBACK_NAMES: &[&str] = &["osifont", "arial", "simplex"];

/// Outcome of one catalog-driven loading pass.
///
/// Failures are recorded verbatim instead of being folded into a success;
/// callers surface them so a missing font is visible in diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontLoadReport {
    /// Number of entries in the fetched catalog.
    pub catalog_entries: usize,
    /// Font keys the drawing asked for (before catalog resolution).
    pub requested: Vec<String>,
    /// Catalog file names the plan selected (deduplicated by file).
    pub planned: Vec<String>,
    /// Keys successfully registered in the engine.
    pub registered: Vec<String>,
    /// `<file>: <reason>` for each planned font that could not be registered.
    pub failed: Vec<String>,
    /// Original names missing from the catalog, even when fallback rendering succeeds.
    pub unresolved: Vec<String>,
    /// The default outline face actually registered as the last-resort fallback
    /// (catalog name file when available, or [`DEFAULT_FALLBACK_KEY`] for a
    /// host-supplied system default), if any.
    pub default_face: Option<String>,
    /// The text locale the fallback chain was built for.
    pub locale: TextLocale,
    /// Locale-preferred faces registered for this document, in preference order.
    pub locale_faces: Vec<String>,
    /// `Some` when the locale prefers a face this font package/system cannot
    /// supply. Callers surface [`Self::diagnostics`] so a Chinese drawing with no
    /// CJK face is a named gap, not a silent Latin substitution.
    pub locale_missing: Option<LocaleFallbackGap>,
}

impl FontLoadReport {
    /// Whether no fonts were requested (the drawing uses no CAD text fonts).
    pub fn is_empty(&self) -> bool {
        self.requested.is_empty() && self.planned.is_empty() && self.registered.is_empty()
    }

    /// Stable diagnostics this load produced (currently the locale gap only).
    ///
    /// Emitted once per load/open, never per text entity. The code and its
    /// payload are ASCII machine detail; the UI localises by code.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.locale_missing
            .as_ref()
            .map(LocaleFallbackGap::diagnostic)
            .into_iter()
            .collect()
    }

    /// One-line, platform-neutral summary for diagnostics.
    pub fn summary(&self) -> String {
        format!(
            "catalog={} requested={} planned={} registered={} failed={} unresolved={} locale={} locale_faces={}",
            self.catalog_entries,
            self.requested.len(),
            self.planned.len(),
            self.registered.len(),
            self.failed.len(),
            self.unresolved.len(),
            self.locale.as_str(),
            self.locale_faces.len(),
        )
    }
}

/// The URL of the catalog JSON under a font base such as
/// `https://…/fonts/` or `asset://fonts/`.
pub fn catalog_url(base: &str) -> String {
    format!("{}/fonts.json", base.trim_end_matches('/'))
}

pub use crate::font_keys::{needs_text_shaping, requested_fonts};

/// Fetch the catalog, plan the `requested` names and register the bytes.
///
/// `text_needed` says the drawing contains text even when `requested` is empty
/// (see [`needs_text_shaping`]): the last-resort default face is then registered
/// so that text shapes through the fallback chain instead of being dropped.
///
/// Returns the populated engine plus a [`FontLoadReport`]. An empty catalog or
/// an empty plan is not an error: the caller decides whether an empty engine
/// should be installed or `clear_fonts` used instead.
///
/// This variant assumes [`TextLocale::Latin`] (the historical behaviour); use
/// [`load_font_engine_for_locale`] for a document whose code page is known.
pub fn load_font_engine<'a>(
    loader: &'a dyn FontLoader,
    requested: &'a [String],
    text_needed: bool,
    base: &'a str,
) -> HostFuture<'a, (Arc<FontEngine>, FontLoadReport)> {
    load_font_engine_for_locale(loader, requested, text_needed, base, TextLocale::Latin)
}

/// Like [`load_font_engine`], but builds the fallback chain for `locale`.
///
/// The drawing's own faces are registered first. A non-Latin locale then
/// registers its first available preferred face (see
/// [`TextLocale::preferred_faces`]) and reorders the chain so that face comes
/// before the Latin defaults. When no preferred face can be registered, the
/// load still succeeds with the Latin chain and records a
/// [`LocaleFallbackGap`], surfaced as a `font.locale_fallback_missing`
/// diagnostic — a missing CJK face is never silently treated as Latin.
pub fn load_font_engine_for_locale<'a>(
    loader: &'a dyn FontLoader,
    requested: &'a [String],
    text_needed: bool,
    base: &'a str,
    locale: TextLocale,
) -> HostFuture<'a, (Arc<FontEngine>, FontLoadReport)> {
    Box::pin(async move {
        let catalog_bytes = loader.load_catalog(&catalog_url(base)).await?;
        let catalog_json = std::str::from_utf8(&catalog_bytes)
            .map_err(|e| CadError::InvalidInput(format!("font catalog is not UTF-8: {e}")))?;
        let catalog = FontCatalog::from_json(catalog_json)?;

        let resolution = plan_fonts_report(&catalog, requested, base);
        let mut plan = resolution.planned;
        if !resolution.unresolved.is_empty() || !resolution.unsupported.is_empty() {
            // Explicit, bounded fallbacks, never aliases masquerading as the original font.
            for fallback in plan_fonts(&catalog, &["arial".into(), "simplex".into()], base) {
                if !plan.iter().any(|font| font.file == fallback.file) {
                    plan.push(fallback);
                }
            }
        }
        let mut engine = FontEngine::new();
        let mut report = FontLoadReport {
            catalog_entries: catalog.len(),
            requested: requested.to_vec(),
            unresolved: resolution.unresolved,
            failed: resolution
                .unsupported
                .iter()
                .map(|key| format!("{key}: unsupported font technology"))
                .collect(),
            locale,
            ..FontLoadReport::default()
        };
        for font in &plan {
            report.planned.push(font.file.clone());
            match loader.load_font(&font.url).await {
                Ok(bytes) => {
                    match engine.register_with_encoding(&font.file, bytes, font.encoding.as_deref())
                    {
                        Ok(()) => report.registered.push(font.file.clone()),
                        Err(e) => report.failed.push(format!("{}: {e}", font.file)),
                    }
                }
                Err(e) => report.failed.push(format!("{}: {e}", font.file)),
            }
        }
        // Guarantee a default outline face whenever a requested font could not
        // be resolved or fetched: the first catalogue name that resolves is
        // registered once (never re-fetching a file already attempted) and
        // reported. This is what lets a missing drawing font shape with a
        // default instead of being dropped.
        let attempted: HashSet<&String> = plan.iter().map(|font| &font.file).collect();
        // The default face is needed when something the drawing named could not
        // be resolved, or when the drawing has text but names no font at all
        // (`registered` is empty in that case). A drawing whose named fonts all
        // registered pays no extra multi-MB default fetch.
        let needs_default = (text_needed && report.registered.is_empty())
            || !report.unresolved.is_empty()
            || !report.failed.is_empty();
        if needs_default {
            for name in DEFAULT_FALLBACK_NAMES {
                let Some(face) = catalog.get(name) else {
                    continue;
                };
                if report.registered.iter().any(|file| file == &face.file) {
                    report.default_face = Some(face.file.clone());
                    break;
                }
                if attempted.contains(&face.file) {
                    // Already planned (and either registered above or reported as
                    // failed); never fetch it a second time.
                    continue;
                }
                match loader.load_font(&face_url(base, face)).await {
                    Ok(bytes) => {
                        match engine.register_with_encoding(
                            &face.file,
                            bytes,
                            face.encoding.as_deref(),
                        ) {
                            Ok(()) => {
                                report.registered.push(face.file.clone());
                                report.default_face = Some(face.file.clone());
                                break;
                            }
                            Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                        }
                    }
                    Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                }
            }
        }
        // Locale-preferred faces: the drawing's own style fonts may all be Latin
        // even though its text is Chinese. Try the locale's candidates in order,
        // filtered by what the catalog/package actually offers, and stop at the
        // first one that registers (so a full CJK package is not fetched whole).
        // A locale with nothing available is recorded as a named gap.
        if locale != TextLocale::Latin && text_needed {
            let mut tried = Vec::new();
            for name in locale.preferred_faces() {
                tried.push((*name).to_string());
                let Some(face) = catalog.get(name) else {
                    continue;
                };
                if let Some(registered) = report
                    .registered
                    .iter()
                    .find(|file| file.as_str() == face.file)
                    .map(|file| (*file).clone())
                {
                    report.locale_faces.push(registered);
                    break;
                }
                if attempted.contains(&face.file) {
                    // Already planned and either registered above (handled by the
                    // find) or reported as failed; never fetch it twice.
                    continue;
                }
                match loader.load_font(&face_url(base, face)).await {
                    Ok(bytes) => {
                        match engine.register_with_encoding(
                            &face.file,
                            bytes,
                            face.encoding.as_deref(),
                        ) {
                            Ok(()) => {
                                report.registered.push(face.file.clone());
                                report.locale_faces.push(face.file.clone());
                                break;
                            }
                            Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                        }
                    }
                    Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                }
            }
            if report.locale_faces.is_empty() {
                report.locale_missing = Some(LocaleFallbackGap {
                    locale,
                    candidates: tried,
                });
            }
        }
        // A registered face stands in for any missing one so text is not
        // silently dropped. A non-Latin locale moves its preferred face ahead of
        // the Latin defaults; Latin keeps the loader's order unchanged.
        engine.set_fallback(locale.ordered_fallback(&report.registered));
        engine.set_locale_tag(locale.as_str());
        Ok((Arc::new(engine), report))
    })
}

/// Register a host-supplied default face and make it the **first** fallback.
///
/// Desktop hosts use this to prefer a system font over the drawing/catalog
/// fallbacks when a requested font is missing. The bytes must parse exactly as
/// in [`FontEngine::register`]; a host that cannot obtain a system font simply
/// does not call this and the catalog default is used instead.
pub fn register_default_face(engine: &mut FontEngine, bytes: Arc<[u8]>) -> CadResult<()> {
    engine.register(DEFAULT_FALLBACK_KEY, bytes)?;
    let mut keys = vec![DEFAULT_FALLBACK_KEY.to_string()];
    for key in engine.fallback_keys() {
        if key != DEFAULT_FALLBACK_KEY {
            keys.push(key.clone());
        }
    }
    engine.set_fallback(keys);
    Ok(())
}

/// Local (native) font-package loader shared by desktop hosts.
///
/// This is the "sibling `fonts/` package" half of the font design: a directory
/// next to the executable holds `fonts.json` plus the committed faces, and the
/// host loads exactly the faces a drawing requests (plus a default) from it, so
/// a packaged client needs no network. Kept out of wasm builds, which use the
/// browser network stack instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod local {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use cad_domain::{CadError, CadResult};

    use super::*;
    use crate::block_on;

    /// A [`FontLoader`] rooted at a font directory.
    ///
    /// The base handed to [`load_font_engine`] is the directory itself, so the
    /// catalog and face URLs `cad-resources` builds are already paths under it.
    pub struct DirFontLoader {
        root: PathBuf,
    }

    impl DirFontLoader {
        pub fn new(root: PathBuf) -> Self {
            Self { root }
        }

        fn path_for(&self, url: &str) -> CadResult<PathBuf> {
            let root = self.root.to_string_lossy();
            // `face_url` percent-encodes, but catalog parsing guarantees a bare
            // file name; the prefix check is defence in depth against a caller
            // passing a URL that points outside the granted directory.
            if !url.starts_with(root.as_ref()) {
                return Err(CadError::InvalidInput(format!(
                    "font path escapes the font directory: {url}"
                )));
            }
            Ok(PathBuf::from(url))
        }
    }

    impl FontLoader for DirFontLoader {
        fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
            let path = self.path_for(url);
            Box::pin(async move {
                let path = path?;
                std::fs::read(&path)
                    .map(|bytes| Arc::from(bytes.into_boxed_slice()))
                    .map_err(|e| {
                        CadError::ResourceMissing(format!(
                            "font read failed for {}: {e}",
                            path.display()
                        ))
                    })
            })
        }
    }

    /// Candidate `fonts/` directories for an executable path, most specific
    /// first: next to the binary, next to its `bin/` parent, under a macOS
    /// `.app` bundle's `Contents/Resources/`, then the working directory.
    pub fn font_dir_candidates(exe: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(dir) = exe.parent() {
            out.push(dir.join("fonts"));
            if let Some(parent) = dir.parent() {
                out.push(parent.join("fonts"));
                // macOS `.app` layout: `Contents/MacOS/<exe>` with the font
                // package at `Contents/Resources/fonts`. Harmless on other
                // platforms (the path simply does not exist).
                out.push(parent.join("Resources").join("fonts"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            out.push(cwd.join("fonts"));
        }
        out
    }

    /// Resolve the font directory to use.
    ///
    /// An explicit path must exist (a typo is an error, not a silent fallback);
    /// otherwise the first existing candidate for `exe` is used, or `None`.
    pub fn resolve_font_dir(explicit: Option<&Path>, exe: &Path) -> CadResult<Option<PathBuf>> {
        match explicit {
            Some(dir) => {
                if dir.is_dir() {
                    Ok(Some(dir.to_path_buf()))
                } else {
                    Err(CadError::InvalidInput(format!(
                        "font directory does not exist: {}",
                        dir.display()
                    )))
                }
            }
            None => Ok(font_dir_candidates(exe)
                .into_iter()
                .find(|dir| dir.is_dir())),
        }
    }

    /// Ask fontconfig for the file backing `family`; `None` when fontconfig is
    /// absent or the family resolves to nothing.
    ///
    /// Unix (and Android) only: Windows and macOS resolve their own system fonts
    /// from the font directories instead of `fc-match`.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn fc_match(family: &str) -> Option<String> {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", family])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path = String::from_utf8(output.stdout).ok()?;
        let path = path.trim();
        (!path.is_empty()).then(|| path.to_string())
    }

    /// Best-effort system default face bytes, most specific first.
    ///
    /// Entries the shaping engine cannot parse (for example a `.ttc` collection)
    /// are skipped by the caller; this only collects candidate bytes.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
        let mut out = Vec::new();
        let mut push = |path: &str| {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(Arc::from(bytes.into_boxed_slice()));
            }
        };
        for family in [
            "sans-serif",
            "DejaVu Sans",
            "Liberation Sans",
            "Noto Sans",
            "Arial",
        ] {
            if let Some(path) = fc_match(family) {
                push(&path);
            }
        }
        for path in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ] {
            push(path);
        }
        out
    }

    /// Windows: read the usual UI faces from the system font directory.
    ///
    /// There is no `fc-match`; `%WINDIR%\Fonts` is the machine-wide directory and
    /// `%LOCALAPPDATA%\Microsoft\Windows\Fonts` holds per-user installs. Only the
    /// byte candidates are collected here; the shaping engine skips anything it
    /// cannot parse (for example a `.ttc` collection).
    #[cfg(target_os = "windows")]
    pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
        let mut out = Vec::new();
        let mut push = |path: &Path| {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(Arc::from(bytes.into_boxed_slice()));
            }
        };
        let mut dirs = Vec::new();
        if let Some(windir) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        for dir in dirs {
            for name in [
                "segoeui.ttf",
                "arial.ttf",
                "calibri.ttf",
                "tahoma.ttf",
                "verdana.ttf",
                "consola.ttf",
            ] {
                push(&dir.join(name));
            }
        }
        out
    }

    /// macOS: read the usual UI faces from the system font directories.
    ///
    /// There is no `fc-match`; the machine-wide faces live in
    /// `/System/Library/Fonts` (with supplemental faces under
    /// `Supplemental/`) and the administrator-installed faces in `/Library/Fonts`.
    /// Only the byte candidates are collected here; the shaping engine skips
    /// anything it cannot parse (for example the many `.ttc` collections and the
    /// variable-axis `SFNS.ttf`). System Integrity Protection guarantees these
    /// paths exist and are readable on a stock install.
    #[cfg(target_os = "macos")]
    pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
        let mut out = Vec::new();
        let mut push = |path: &str| {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(Arc::from(bytes.into_boxed_slice()));
            }
        };
        for path in [
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Helvetica.ttf",
            "/System/Library/Fonts/Geneva.ttf",
            "/System/Library/Fonts/Supplemental/Verdana.ttf",
            "/Library/Fonts/Arial.ttf",
        ] {
            push(path);
        }
        out
    }

    /// Cached [`system_default_candidates`] for the process.
    ///
    /// Reading a large system collection (`NotoSansCJK.ttc` is ~19 MiB) on every
    /// document open would be wasteful, so the candidates are materialised once.
    pub fn cached_system_default_candidates() -> &'static Vec<Arc<[u8]>> {
        static CACHE: std::sync::OnceLock<Vec<Arc<[u8]>>> = std::sync::OnceLock::new();
        CACHE.get_or_init(system_default_candidates)
    }

    /// Build a shaping engine from the `fonts/` package, explicit entries and a
    /// system default, or `None` when nothing is needed.
    ///
    /// `requested` are the drawing's font keys, `text_needed` whether the drawing
    /// contains text that must be shaped even though it names no font, `explicit`
    /// the `--font` entries and `system_default` the ordered default-face
    /// candidates. A drawing with neither text nor a named font installs nothing;
    /// a text-bearing drawing always gets an engine so its glyphs resolve through
    /// the fallback chain instead of being silently dropped.
    ///
    /// This variant assumes [`TextLocale::Latin`]; use [`load_engine_report`]
    /// with the locale derived from the drawing's code page.
    pub fn load_engine(
        font_dir: Option<&Path>,
        requested: &[String],
        text_needed: bool,
        explicit: &[(String, PathBuf)],
        system_default: &[Arc<[u8]>],
    ) -> CadResult<Option<Arc<FontEngine>>> {
        Ok(load_engine_report(
            font_dir,
            requested,
            text_needed,
            explicit,
            system_default,
            TextLocale::Latin,
        )?
        .0)
    }

    /// Like [`load_engine`], but for a document whose text locale is known, and
    /// with the load report (locale gap included) returned to the caller.
    ///
    /// A locale with no preferred face available still installs the Latin chain
    /// and reports the gap; it never fails the open and never silently pretends
    /// the locale was satisfied.
    pub fn load_engine_report(
        font_dir: Option<&Path>,
        requested: &[String],
        text_needed: bool,
        explicit: &[(String, PathBuf)],
        system_default: &[Arc<[u8]>],
        locale: TextLocale,
    ) -> CadResult<(Option<Arc<FontEngine>>, FontLoadReport)> {
        let mut report = FontLoadReport {
            locale,
            ..FontLoadReport::default()
        };
        if requested.is_empty() && explicit.is_empty() && !text_needed {
            return Ok((None, report));
        }
        let mut engine = match font_dir {
            Some(dir) => {
                let base = dir.to_string_lossy().to_string();
                let loader = DirFontLoader::new(dir.to_path_buf());
                let (loaded, loaded_report) = block_on(load_font_engine_for_locale(
                    &loader,
                    requested,
                    text_needed,
                    &base,
                    locale,
                ))?;
                report = loaded_report;
                // `load_font_engine_for_locale` returns a uniquely owned engine;
                // unwrapping it lets the explicit/system registrations below
                // extend the same set.
                Arc::try_unwrap(loaded)
                    .map_err(|_| CadError::Invariant("font engine unexpectedly shared".into()))?
            }
            None => {
                // No package to resolve the locale's faces from: the gap is
                // known here rather than after a fetch attempt.
                if locale != TextLocale::Latin && text_needed {
                    report.locale_missing = Some(LocaleFallbackGap {
                        locale,
                        candidates: locale
                            .preferred_faces()
                            .iter()
                            .map(|name| (*name).to_string())
                            .collect(),
                    });
                }
                FontEngine::new()
            }
        };
        let mut fallback = engine.fallback_keys().to_vec();
        for (name, path) in explicit {
            let bytes = std::fs::read(path)
                .map_err(|e| CadError::InvalidInput(format!("font '{}': {e}", path.display())))?;
            engine.register(name, Arc::from(bytes.into_boxed_slice()))?;
            if !fallback.iter().any(|key| key == name) {
                fallback.push(name.clone());
            }
        }
        engine.set_fallback(fallback);
        // The first parseable system face becomes the preferred default; a `.ttc`
        // that the engine rejects is simply skipped.
        for bytes in system_default {
            if register_default_face(&mut engine, bytes.clone()).is_ok() {
                break;
            }
        }
        if engine.is_empty() {
            return Ok((None, report));
        }
        engine.set_locale_tag(locale.as_str());
        // An explicit/`--font` CJK face also satisfies the locale; the gap is
        // only real when the engine truly has none of the preferred faces.
        if report.locale_missing.is_some()
            && locale
                .preferred_faces()
                .iter()
                .any(|name| engine.contains(name))
        {
            report.locale_missing = None;
        }
        Ok((Some(Arc::new(engine)), report))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn candidates_prefer_the_binary_side_fonts_dir() {
            let exe = Path::new("/opt/yacr/bin/yacr-linux");
            let candidates = font_dir_candidates(exe);
            assert_eq!(candidates[0], PathBuf::from("/opt/yacr/bin/fonts"));
            assert_eq!(candidates[1], PathBuf::from("/opt/yacr/fonts"));
            // macOS `.app` layout: `Contents/MacOS/<exe>` -> `Contents/Resources/fonts`.
            let app_exe = Path::new("/opt/Yacr.app/Contents/MacOS/yacr-macos");
            let app_candidates = font_dir_candidates(app_exe);
            assert_eq!(
                app_candidates[0],
                PathBuf::from("/opt/Yacr.app/Contents/MacOS/fonts")
            );
            assert_eq!(
                app_candidates[1],
                PathBuf::from("/opt/Yacr.app/Contents/fonts")
            );
            assert_eq!(
                app_candidates[2],
                PathBuf::from("/opt/Yacr.app/Contents/Resources/fonts")
            );
        }

        #[test]
        fn explicit_missing_font_dir_is_an_error_not_a_silent_fallback() {
            let missing = Path::new("/definitely/not/a/font/dir");
            let result = resolve_font_dir(Some(missing), Path::new("/opt/yacr/bin/yacr-linux"));
            assert!(matches!(result, Err(CadError::InvalidInput(_))));
        }

        #[test]
        fn nothing_requested_and_no_explicit_fonts_installs_nothing() {
            let engine = load_engine(None, &[], false, &[], &[]).unwrap();
            assert!(engine.is_none());
        }

        #[test]
        fn text_with_no_named_font_installs_a_fallback_engine() {
            // A drawing can hold text whose style names no font. It still needs
            // shaping, so the default face must be installed instead of returning
            // `None`; the glyphs then come from the fallback chain.
            let engine = load_engine(None, &[], true, &[], &[Arc::from(synthetic_shx())])
                .unwrap()
                .expect("text without a named font installs an engine");
            assert_eq!(
                engine.fallback_keys().first().map(String::as_str),
                Some(DEFAULT_FALLBACK_KEY)
            );
        }

        #[test]
        fn text_without_a_named_font_loads_the_font_package_default() {
            // `--fonts-dir` (or the sibling package) alone must rescue text with
            // no named font: the catalog default is registered even though
            // `requested` is empty.
            let dir =
                std::env::temp_dir().join(format!("yacr-font-package-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("fonts.json"),
                br#"[{ "file": "osifont.shx", "name": ["osifont"], "type": "shx" }]"#,
            )
            .unwrap();
            std::fs::write(dir.join("osifont.shx"), synthetic_shx()).unwrap();
            let engine = load_engine(Some(&dir), &[], true, &[], &[])
                .unwrap()
                .expect("the package default installs an engine");
            assert!(engine.contains("osifont.shx"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn explicit_font_is_registered_and_used_as_fallback() {
            let dir = std::env::temp_dir().join(format!("yacr-font-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("explicit.shx");
            std::fs::write(&path, synthetic_shx()).unwrap();
            let engine = load_engine(
                None,
                &["missing.shx".to_string()],
                false,
                &[("MyFont".to_string(), path.clone())],
                &[],
            )
            .unwrap()
            .expect("explicit font installs an engine");
            assert!(engine.contains("MyFont"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn system_default_becomes_the_first_fallback() {
            let engine = load_engine(
                None,
                &["missing.shx".to_string()],
                false,
                &[],
                &[Arc::from(synthetic_shx())],
            )
            .unwrap()
            .expect("a system default installs an engine");
            assert_eq!(
                engine.fallback_keys().first().map(String::as_str),
                Some(DEFAULT_FALLBACK_KEY)
            );
            assert!(engine.contains(DEFAULT_FALLBACK_KEY));
        }

        /// Minimal but parseable compiled SHX shape font bytes.
        fn synthetic_shx() -> Vec<u8> {
            let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
            let info = b"Synthetic\0\x15\x07\x02\0";
            for value in [0u16, 0, 1, 0, info.len() as u16] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(info);
            bytes
        }
    }
}

#[cfg(test)]
mod tests;
