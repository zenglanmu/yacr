//! The read-only font-report projection (`cad_fonts`).
//!
//! The face registry is the host's: this module resolves nothing, registers
//! nothing and retries nothing. Every row is the host's **real** shaping-font
//! load outcome ([`cad_platform::fonts::FontLoadReport`]) mapped one-to-one, so a
//! drawing that loaded no faces reports an empty list rather than a synthetic
//! one. A host that has no report yet (before its first install/tick) answers
//! [`CadFonts::empty`] — the explicit "registry answered with nothing", never a
//! fabricated row.
//!
//! The answer is a read-only IPC projection, deliberately **not** a `cad_status`
//! field: the frozen status contract is untouched, and the Font Manager owns its
//! own report.
//!
//! Two fields are not report-derived: `requested`/`text_needed` answer what the
//! **open drawing** needs even before any host strategy ran. A lazily loading
//! carrier (the browser fetching a CDN package) reads those from `cad_fonts`
//! while `faces` is still empty, then calls [`CadSession::reinstall_fonts`] once
//! the package arrived.

use cad_engine::protocol::EngineClient;
use cad_platform::fonts::{FontFaceSource, FontFaceStatus, FontLoadReport, TextLocale};
use serde::Serialize;

use super::diagnostics::cad_error_text;
use super::CadSession;

/// One face row of the [`CadFonts`] answer.
///
/// `source` and `status` are the **stable lowercase machine labels**
/// (`package|explicit|system`, `loaded|failed|unresolved|unsupported`), never
/// localized prose: the panel localises by label and never parses a sentence.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadFont {
    /// The lookup key/name the host recorded the face under.
    pub key: String,
    /// Resolved file name, or `None` when the host resolved none.
    pub file: Option<String>,
    /// Font technology (`shx`, `mesh`, `system`, the catalog's own label, ...).
    pub kind: String,
    /// Where the face's bytes came from: `package`, `explicit` or `system`.
    pub source: String,
    /// What happened during the load: `loaded`, `failed`, `unresolved` or
    /// `unsupported`.
    pub status: String,
    /// The host's own failure reason, verbatim, when `status` is `failed`.
    pub detail: Option<String>,
}

/// The complete `cad_fonts` answer.
///
/// Every key is always present. `available: false` is its own state with a named
/// machine `reason` (a catalog key, e.g. `host.fonts_unavailable`); it is not the
/// same thing as an available report with no faces.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadFonts {
    /// Whether this host can report its font registry at all.
    pub available: bool,
    /// Named machine reason when `available` is false, else `None`.
    pub reason: Option<String>,
    /// The effective font package directory, or `None` when unset/unknown.
    pub fonts_dir: Option<String>,
    /// Number of entries in the fetched font catalog.
    pub catalog_entries: usize,
    /// The text locale the installed fallback chain was built for (`latin`,
    /// `zh-Hans`, `zh-Hant`, `ja`, `ko`).
    pub locale: String,
    /// The default outline face actually registered as the last resort, if any.
    pub default_face: Option<String>,
    /// Machine detail when the locale's preferred face is unavailable: the
    /// locale plus the candidates that were tried. `None` when the locale is
    /// satisfied (or the drawing needs no text).
    pub locale_missing: Option<String>,
    /// Logical font keys the open drawing references, read from the database
    /// (no catalog lookup, no path resolution). Populated even before a host
    /// report exists, so a lazy carrier knows what to install.
    pub requested: Vec<String>,
    /// Whether the open drawing holds text that needs shaping even when it
    /// names no font. Independent of the report, which does not carry it.
    pub text_needed: bool,
    /// Every face the load touched, in the host's discovery order.
    pub faces: Vec<CadFont>,
}

impl CadFonts {
    /// The honest "no report yet" answer: available, no faces, no directory.
    ///
    /// [`CadSession::fonts`] fills `fonts_dir` from the host options; a host that
    /// has never loaded fonts still did not fail.
    pub fn empty() -> Self {
        Self {
            available: true,
            reason: None,
            fonts_dir: None,
            catalog_entries: 0,
            locale: TextLocale::Latin.as_str().to_string(),
            default_face: None,
            locale_missing: None,
            requested: Vec::new(),
            text_needed: false,
            faces: Vec::new(),
        }
    }

    /// The explicit "this host cannot report fonts" answer with a named reason.
    ///
    /// The reason is a catalog key (lowercase dotted machine string); no prose is
    /// invented here and `faces` stays empty rather than pretending a registry
    /// answered with nothing.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            fonts_dir: None,
            catalog_entries: 0,
            locale: TextLocale::Latin.as_str().to_string(),
            default_face: None,
            locale_missing: None,
            requested: Vec::new(),
            text_needed: false,
            faces: Vec::new(),
        }
    }
}

impl CadSession {
    /// The read-only font-registry projection of the open document.
    ///
    /// A host with no report yet answers [`CadFonts::empty`] with the configured
    /// `--fonts-dir` (display string), never an error: "nothing loaded yet" is a
    /// real state. `requested`/`text_needed` are always derived from the open
    /// drawing, so a lazily loading carrier can decide what to install before any
    /// strategy ran. Once a load ran, every report-derived field and every face
    /// row is the report's own value, mapped 1:1 — nothing is coalesced, sorted
    /// or synthesized.
    pub fn fonts(&self) -> Result<CadFonts, String> {
        let fonts_dir = self
            .core
            .options
            .fonts_dir
            .as_ref()
            .map(|dir| dir.display().to_string())
            .filter(|dir| !dir.is_empty());
        let (requested, text_needed) = self.drawing_font_needs();
        let Some(report) = self.core.last_font_report() else {
            return Ok(CadFonts {
                fonts_dir,
                requested,
                text_needed,
                ..CadFonts::empty()
            });
        };
        Ok(project_report(&report, fonts_dir, text_needed))
    }

    /// Re-run the host font strategy (after a lazily installed package) and
    /// rebuild the display representation.
    ///
    /// The open document is untouched: only its resources are re-resolved. A
    /// strategy that still cannot resolve refuses with the host's own error,
    /// verbatim — a failed reinstall is never an empty success.
    pub fn reinstall_fonts(&self) -> Result<(), String> {
        self.core
            .install_fonts()
            .map_err(|error| cad_error_text(&error))?;
        // The same rebuild the host open funnel performs: the freshly installed
        // engine is prepared against the current authoritative drawing.
        self.core
            .render
            .sync_drawing(self.core.controller.borrow().drawing());
        // The freshly installed caches belong to this document generation, so a
        // later tick must not retire them through the stale-resource guard.
        self.resource_guard
            .set(Some(self.core.request_context().document_generation));
        Ok(())
    }

    /// The drawing's own font needs, independent of any host report.
    fn drawing_font_needs(&self) -> (Vec<String>, bool) {
        let drawing = self.core.controller.borrow().drawing();
        let Some(drawing) = drawing else {
            return (Vec::new(), false);
        };
        (
            unique_font_keys(cad_platform::fonts::requested_fonts(drawing.as_ref())),
            cad_platform::fonts::needs_text_shaping(drawing.as_ref()),
        )
    }
}

/// Deduplicate font keys case-insensitively, keeping the first spelling/order.
///
/// `requested_fonts` walks every text entity, so a drawing with thousands of
/// labels repeats the same names thousands of times. The wire/UI needs the
/// distinct set, not the traversal's multiplicity.
fn unique_font_keys(keys: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter()
        .filter(|key| seen.insert(key.to_ascii_lowercase()))
        .collect()
}

/// Project one real host report into the wire answer.
fn project_report(
    report: &FontLoadReport,
    fonts_dir: Option<String>,
    text_needed: bool,
) -> CadFonts {
    CadFonts {
        available: true,
        reason: None,
        fonts_dir,
        catalog_entries: report.catalog_entries,
        locale: report.locale.as_str().to_string(),
        default_face: report.default_face.clone(),
        locale_missing: report.locale_missing.as_ref().map(|gap| {
            // Machine detail, never an i18n key: the locale plus every candidate
            // that was tried, so a reader can tell which face is missing without
            // parsing localized prose.
            format!("{} ({})", gap.locale.as_str(), gap.candidates.join(", "))
        }),
        requested: unique_font_keys(report.requested.clone()),
        text_needed,
        faces: report.faces.iter().map(project_face).collect(),
    }
}

/// Project one host face record; every field is the host's own value.
fn project_face(face: &cad_platform::fonts::FontFaceRecord) -> CadFont {
    CadFont {
        key: face.key.clone(),
        file: face.file.clone(),
        kind: face.kind.clone(),
        source: source_label(face.source).to_string(),
        status: status_label(face.status).to_string(),
        detail: face.detail.clone(),
    }
}

/// The stable lowercase label of a face source.
fn source_label(source: FontFaceSource) -> &'static str {
    match source {
        FontFaceSource::Package => "package",
        FontFaceSource::Explicit => "explicit",
        FontFaceSource::System => "system",
    }
}

/// The stable lowercase label of a face status.
fn status_label(status: FontFaceStatus) -> &'static str {
    match status {
        FontFaceStatus::Loaded => "loaded",
        FontFaceStatus::Failed => "failed",
        FontFaceStatus::Unresolved => "unresolved",
        FontFaceStatus::Unsupported => "unsupported",
    }
}
