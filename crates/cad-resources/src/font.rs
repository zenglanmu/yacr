//! font module.

use super::*;

/// Default base URL for the bundled CAD font set.
///
/// This is the same catalog the `mlightcad/cad-viewer` web viewer uses
/// (`mlightcad/cad-data`, served through jsDelivr). Hosts may override it; the
/// core only builds URLs and never performs network access itself.
pub const DEFAULT_FONT_BASE_URL: &str =
    "https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/";

/// Font technology, as declared by the catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontKind {
    /// AutoCAD compiled shape font (`.shx`).
    Shx,
    /// Outline font (`.ttf`/`.otf`/`.woff`).
    Mesh,
    /// A declared type this crate does not recognise (for example `woff2`).
    ///
    /// Kept as an explicit variant so a catalog entry cannot be silently
    /// treated as a usable face: [`plan_fonts_report`] surfaces it instead.
    Other(String),
}

/// One font face from the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFace {
    /// File name inside the font directory, e.g. `simplex.shx`.
    pub file: String,
    /// Font names a drawing may reference, e.g. `["simplex"]`.
    pub names: Vec<String>,
    pub kind: FontKind,
    /// Optional code page, e.g. `shift-jis`.
    pub encoding: Option<String>,
}

impl FontFace {
    /// Keys this face can be looked up by (names, file name and file stem).
    fn lookup_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .names
            .iter()
            .map(|n| ResourceKey::sanitize(n).0)
            .collect();
        keys.push(ResourceKey::sanitize(&self.file).0);
        if let Some(stem) = self.file.rsplit_once('.').map(|(stem, _)| stem) {
            keys.push(ResourceKey::sanitize(stem).0);
        }
        keys.retain(|k| !k.is_empty());
        keys
    }
}

/// An indexed font catalog in the `mlightcad/cad-data` JSON format.
#[derive(Debug, Clone, Default)]
pub struct FontCatalog {
    faces: Vec<FontFace>,
    by_key: HashMap<String, usize>,
}

impl FontCatalog {
    /// Parse the catalog JSON (an array of `{ file, name[], type, encoding }`).
    pub fn from_json(json: &str) -> CadResult<FontCatalog> {
        let value: Value = serde_json::from_str(json)
            .map_err(|e| CadError::InvalidInput(format!("font catalog is not valid JSON: {e}")))?;
        let entries = value.as_array().ok_or_else(|| {
            CadError::InvalidInput("font catalog must be a JSON array".to_string())
        })?;
        let mut catalog = FontCatalog::default();
        for entry in entries {
            let Some(file) = entry.get("file").and_then(Value::as_str) else {
                continue;
            };
            if file.trim().is_empty() {
                continue;
            }
            let names: Vec<String> = entry
                .get("name")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let kind = match entry.get("type").and_then(Value::as_str) {
                Some("shx") => FontKind::Shx,
                Some("mesh") => FontKind::Mesh,
                Some(other) => FontKind::Other(other.to_string()),
                None => FontKind::Other(String::new()),
            };
            let encoding = entry
                .get("encoding")
                .and_then(Value::as_str)
                .map(str::to_string);
            let face = FontFace {
                file: file.to_string(),
                names,
                kind,
                encoding,
            };
            let index = catalog.faces.len();
            for key in face.lookup_keys() {
                catalog.by_key.entry(key).or_insert(index);
            }
            catalog.faces.push(face);
        }
        Ok(catalog)
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn faces(&self) -> impl Iterator<Item = &FontFace> {
        self.faces.iter()
    }

    /// Look up a face by drawing font name, file name or file stem.
    pub fn get(&self, name: &str) -> Option<&FontFace> {
        let key = ResourceKey::sanitize(name).0;
        self.by_key.get(&key).map(|index| &self.faces[*index])
    }
}

/// Build the fetch URL for a face under `base`.
///
/// Spaces (present in some catalog file names) are percent-encoded so the URL
/// stays valid; the core does not otherwise rewrite the base.
pub fn face_url(base: &str, face: &FontFace) -> String {
    let base = base.trim_end_matches('/');
    let file = face.file.replace(' ', "%20");
    format!("{base}/{file}")
}

/// Fetch URL for a face using the default mlightcad/cad-data catalog.
pub fn default_font_url(face: &FontFace) -> String {
    face_url(DEFAULT_FONT_BASE_URL, face)
}

/// A font a drawing needs, resolved against the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFont {
    /// The name the drawing referenced (for example `arial.ttf`).
    pub request: String,
    pub file: String,
    pub kind: FontKind,
    pub encoding: Option<String>,
    pub url: String,
}

/// Resolve the fonts a document asks for into catalog faces and fetch URLs.
///
/// Unknown names are skipped (the host may then fall back), and each file is
/// planned once even if several names resolve to it. This preserves the
/// original behaviour for hosts that fetch first and rely on the shaping engine
/// to reject undecodable bytes; use [`plan_fonts_report`] to classify
/// unresolved and unsupported technologies up front.
pub fn plan_fonts(catalog: &FontCatalog, requested: &[String], base: &str) -> Vec<PlannedFont> {
    let mut planned: Vec<PlannedFont> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for request in requested {
        let Some(face) = catalog.get(request) else {
            continue;
        };
        if !seen.insert(face.file.clone()) {
            continue;
        }
        planned.push(PlannedFont {
            request: request.clone(),
            file: face.file.clone(),
            kind: face.kind.clone(),
            encoding: face.encoding.clone(),
            url: face_url(base, face),
        });
    }
    planned
}

/// Result of planning fonts, with every request accounted for.
///
/// The audit (F10) found `plan_fonts` silently dropped unknown names. This
/// report keeps the dropped requests plus a structured [`ResourceIssue`] per
/// name so a missing or unsupported font is an explicit diagnostic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontPlanReport {
    /// Faces that resolved to a catalog entry and can be fetched.
    pub planned: Vec<PlannedFont>,
    /// Requests that resolved to no catalog entry.
    pub unresolved: Vec<String>,
    /// Requests that resolved but whose declared technology is not supported.
    pub unsupported: Vec<String>,
    /// Structured reasons for `unresolved` and `unsupported`.
    pub issues: Vec<ResourceIssue>,
}

impl FontPlanReport {
    /// True when every requested name was planned (no gaps).
    pub fn is_complete(&self) -> bool {
        self.unresolved.is_empty() && self.unsupported.is_empty()
    }

    /// Planned faces only; convenience for callers that ignore the report.
    pub fn into_planned(self) -> Vec<PlannedFont> {
        self.planned
    }
}

/// Like [`plan_fonts`] but accounts for every requested name.
///
/// Names that resolve to nothing become [`ResourceIssue`]s instead of being
/// dropped, and catalog entries whose declared technology this crate does not
/// support (for example `woff2`, parsed as [`FontKind::Other`]) are reported as
/// unsupported rather than planned as if usable.
pub fn plan_fonts_report(
    catalog: &FontCatalog,
    requested: &[String],
    base: &str,
) -> FontPlanReport {
    let mut report = FontPlanReport::default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for request in requested {
        let Some(face) = catalog.get(request) else {
            report.unresolved.push(request.clone());
            let code = codes::FONT_UNRESOLVED;
            if !report.issues.iter().any(|issue| {
                issue.code == code
                    && issue.key.as_deref() == Some(ResourceKey::sanitize(request).0.as_str())
            }) {
                report.issues.push(ResourceIssue::font_unresolved(request));
            }
            continue;
        };
        if !supported_font_kind(&face.kind) {
            report.unsupported.push(request.clone());
            report
                .issues
                .push(ResourceIssue::font_unsupported(request, &face.kind));
            continue;
        }
        if !seen.insert(face.file.clone()) {
            continue;
        }
        report.planned.push(PlannedFont {
            request: request.clone(),
            file: face.file.clone(),
            kind: face.kind.clone(),
            encoding: face.encoding.clone(),
            url: face_url(base, face),
        });
    }
    report
}

/// Whether a catalog font technology can be turned into glyphs by core.
///
/// Mirrors `cad-representation::text`: sfnt (TTF/OTF/WOFF1) and SHX are
/// decodable; anything else (for example WOFF2) is not.
pub fn supported_font_kind(kind: &FontKind) -> bool {
    matches!(kind, FontKind::Shx | FontKind::Mesh)
}
