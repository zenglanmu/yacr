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
mod memory;
mod record;

pub use locale::{locale_gap, LocaleFallbackGap, TextLocale};
pub use memory::MemoryFontLoader;
pub use record::{FontFaceRecord, FontFaceSource, FontFaceStatus};

use record::kind_label;

/// Reserved engine key for a host-supplied default fallback face.
///
/// A desktop host registers its system default font under this key so a
/// drawing font that is missing falls back to the platform default; see
/// [`register_default_face`].
pub const DEFAULT_FALLBACK_KEY: &str = "__yacr_default__";

/// Catalogue names tried, in order, for the last-resort default outline face.
///
/// `noto sans cjk sc` is the committed OFL Noto Sans CJK SC face
/// (`fonts/NotoSansCJKsc-Regular.otf`) and the **default fallback** for every
/// host: it covers Latin, kana and CJK, so a drawing whose own font is missing
/// does not silently lose CJK glyphs. `osifont` (QCAD ISO 3098) and the
/// `arial`/`simplex` catalog fallbacks remain after it so a package that lacks
/// Noto still has a usable outline face.
pub const DEFAULT_FALLBACK_NAMES: &[&str] = &["noto sans cjk sc", "osifont", "arial", "simplex"];

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
    /// Every face this load touched, in discovery order, for a UI list.
    ///
    /// Additive to `registered`/`failed`/`unresolved`; rows are deduplicated by
    /// `(file, status)` so a face registered as default and again as the locale
    /// face appears once.
    pub faces: Vec<FontFaceRecord>,
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

    /// Append a face record unless an equivalent row is already present.
    ///
    /// Rows that resolved to a file dedupe by `(file, status)`, so the planned,
    /// default and locale-preferred passes cannot record the same face twice.
    /// Rows without a file (unresolved names) dedupe by `(key, status)` so two
    /// different missing names stay two rows.
    pub fn push_face(&mut self, face: FontFaceRecord) {
        let duplicate = self.faces.iter().any(|existing| {
            existing.status == face.status
                && match (&existing.file, &face.file) {
                    (Some(a), Some(b)) => a == b,
                    _ => existing.key == face.key,
                }
        });
        if !duplicate {
            self.faces.push(face);
        }
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
        // Every requested name is accounted for in `faces`, including the two
        // cases the key lists only summarise.
        for key in report.unresolved.clone() {
            report.push_face(FontFaceRecord {
                key: key.clone(),
                file: None,
                kind: "unknown".to_string(),
                source: FontFaceSource::Package,
                status: FontFaceStatus::Unresolved,
                detail: None,
            });
        }
        for name in &resolution.unsupported {
            let face = catalog.get(name);
            report.push_face(FontFaceRecord {
                key: name.clone(),
                file: face.map(|face| face.file.clone()),
                kind: face
                    .map(|face| kind_label(&face.kind))
                    .unwrap_or_else(|| "unknown".to_string()),
                source: FontFaceSource::Package,
                status: FontFaceStatus::Unsupported,
                detail: Some("unsupported font technology".to_string()),
            });
        }
        for font in &plan {
            report.planned.push(font.file.clone());
            match loader.load_font(&font.url).await {
                Ok(bytes) => {
                    match engine.register_with_encoding(&font.file, bytes, font.encoding.as_deref())
                    {
                        Ok(()) => {
                            report.registered.push(font.file.clone());
                            report.push_face(FontFaceRecord {
                                key: font.request.clone(),
                                file: Some(font.file.clone()),
                                kind: kind_label(&font.kind),
                                source: FontFaceSource::Package,
                                status: FontFaceStatus::Loaded,
                                detail: None,
                            });
                        }
                        Err(e) => {
                            report.failed.push(format!("{}: {e}", font.file));
                            report.push_face(FontFaceRecord {
                                key: font.request.clone(),
                                file: Some(font.file.clone()),
                                kind: kind_label(&font.kind),
                                source: FontFaceSource::Package,
                                status: FontFaceStatus::Failed,
                                detail: Some(e.to_string()),
                            });
                        }
                    }
                }
                Err(e) => {
                    report.failed.push(format!("{}: {e}", font.file));
                    report.push_face(FontFaceRecord {
                        key: font.request.clone(),
                        file: Some(font.file.clone()),
                        kind: kind_label(&font.kind),
                        source: FontFaceSource::Package,
                        status: FontFaceStatus::Failed,
                        detail: Some(e.to_string()),
                    });
                }
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
                    report.push_face(FontFaceRecord {
                        key: (*name).to_string(),
                        file: Some(face.file.clone()),
                        kind: kind_label(&face.kind),
                        source: FontFaceSource::Package,
                        status: FontFaceStatus::Loaded,
                        detail: None,
                    });
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
                                report.push_face(FontFaceRecord {
                                    key: (*name).to_string(),
                                    file: Some(face.file.clone()),
                                    kind: kind_label(&face.kind),
                                    source: FontFaceSource::Package,
                                    status: FontFaceStatus::Loaded,
                                    detail: None,
                                });
                                break;
                            }
                            Err(e) => {
                                report.failed.push(format!("{}: {e}", face.file));
                                report.push_face(FontFaceRecord {
                                    key: (*name).to_string(),
                                    file: Some(face.file.clone()),
                                    kind: kind_label(&face.kind),
                                    source: FontFaceSource::Package,
                                    status: FontFaceStatus::Failed,
                                    detail: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        report.failed.push(format!("{}: {e}", face.file));
                        report.push_face(FontFaceRecord {
                            key: (*name).to_string(),
                            file: Some(face.file.clone()),
                            kind: kind_label(&face.kind),
                            source: FontFaceSource::Package,
                            status: FontFaceStatus::Failed,
                            detail: Some(e.to_string()),
                        });
                    }
                }
            }
        }
        // Locale-preferred faces: the drawing's own style fonts may all be Latin
        // even though its text is Chinese. Try the locale's candidates in order,
        // filtered by what the catalog/package actually offers, and stop at the
        // first one that registers (so a full CJK package is not fetched whole).
        // A locale with nothing available is recorded as a named gap.
        if locale != TextLocale::Latin && text_needed {
            // A preferred face may already be registered: the default pass now
            // tries Noto first, and Noto covers every CJK locale. Reuse it
            // instead of fetching the earlier candidates, so a lazy package that
            // carries only the requested + default faces does not report the
            // available-but-unfetched locale faces as failures.
            let already_preferred = locale.preferred_faces().iter().find_map(|name| {
                let face = catalog.get(name)?;
                report
                    .registered
                    .iter()
                    .any(|file| file.as_str() == face.file)
                    .then_some((*name, &face.file, &face.kind))
            });
            if let Some((name, file, kind)) = already_preferred {
                report.locale_faces.push(file.clone());
                report.push_face(FontFaceRecord {
                    key: name.to_string(),
                    file: Some(file.clone()),
                    kind: kind_label(kind),
                    source: FontFaceSource::Package,
                    status: FontFaceStatus::Loaded,
                    detail: None,
                });
            } else {
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
                        report.push_face(FontFaceRecord {
                            key: (*name).to_string(),
                            file: Some(face.file.clone()),
                            kind: kind_label(&face.kind),
                            source: FontFaceSource::Package,
                            status: FontFaceStatus::Loaded,
                            detail: None,
                        });
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
                                    report.push_face(FontFaceRecord {
                                        key: (*name).to_string(),
                                        file: Some(face.file.clone()),
                                        kind: kind_label(&face.kind),
                                        source: FontFaceSource::Package,
                                        status: FontFaceStatus::Loaded,
                                        detail: None,
                                    });
                                    break;
                                }
                                Err(e) => {
                                    report.failed.push(format!("{}: {e}", face.file));
                                    report.push_face(FontFaceRecord {
                                        key: (*name).to_string(),
                                        file: Some(face.file.clone()),
                                        kind: kind_label(&face.kind),
                                        source: FontFaceSource::Package,
                                        status: FontFaceStatus::Failed,
                                        detail: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            report.failed.push(format!("{}: {e}", face.file));
                            report.push_face(FontFaceRecord {
                                key: (*name).to_string(),
                                file: Some(face.file.clone()),
                                kind: kind_label(&face.kind),
                                source: FontFaceSource::Package,
                                status: FontFaceStatus::Failed,
                                detail: Some(e.to_string()),
                            });
                        }
                    }
                }
                if report.locale_faces.is_empty() {
                    report.locale_missing = Some(LocaleFallbackGap {
                        locale,
                        candidates: tried,
                    });
                }
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
pub mod local;

#[cfg(test)]
mod tests;
