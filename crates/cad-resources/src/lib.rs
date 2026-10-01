//! Logical keys only; explicit grants precede any external access.
//!
//! Spec v2.0 §7.3, §16.2: references are logical keys, never platform paths. A
//! crafted drawing must not be able to direct the app outside its allowed
//! roots, so every reference is sanitised to a bare file name and the resolver
//! chain is explicit and ordered.

use cad_domain::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

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
/// planned once even if several names resolve to it.
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

/// A logical, platform-independent resource identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceKey(pub String);

impl ResourceKey {
    /// Normalise a raw reference to a bare, case-insensitive key.
    pub fn sanitize(raw: &str) -> ResourceKey {
        ResourceKey(bare_name(raw).to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub enum ResourceKind {
    FontTtf,
    FontShx,
    BigFont,
    Image,
    ExternalReference,
}

pub struct ResourceRequest {
    pub document: DocumentId,
    pub key: ResourceKey,
    pub kind: ResourceKind,
}

pub struct ResourceData {
    pub bytes: Arc<[u8]>,
    pub version: u64,
    pub license_hint: Option<String>,
}

pub struct ResourceLimits {
    pub max_bytes: usize,
    pub max_image_pixels: u64,
    pub max_xref_depth: usize,
    pub font_cache_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            max_bytes: 64 * 1024 * 1024,
            max_image_pixels: 8192 * 8192,
            max_xref_depth: 8,
            font_cache_bytes: 64 * 1024 * 1024,
        }
    }
}

pub trait ResourceResolver {
    fn resolve(&self, request: &ResourceRequest) -> CadResult<ResourceData>;
}

/// Reduce any reference to a bare file name.
///
/// Strips directory components, Windows drive letters and control characters,
/// and rejects ascend sequences. Returns `"unnamed"` when nothing remains.
pub fn bare_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let last = match last.split_once(':') {
        Some((prefix, rest)) if prefix.len() == 1 => rest,
        _ => last,
    };
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "unnamed".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Whether a reference is safe to attempt at all.
pub fn is_safe_reference(raw: &str) -> bool {
    let t = raw.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with('/') || t.starts_with('\\') {
        return false;
    }
    if t.len() >= 2 && t.as_bytes()[1] == b':' {
        return false;
    }
    if let Some(idx) = t.find(':') {
        let scheme = &t[..idx];
        if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
            return false;
        }
    }
    !t.split(['/', '\\']).any(|seg| seg == "..")
}

/// Bounded, ordered-chain resolver backed by in-memory grants.
///
/// Hosts populate it from user resource packs and the bundled font set; it never
/// touches the filesystem or network itself.
#[derive(Default)]
pub struct MapResolver {
    entries: HashMap<String, (Arc<[u8]>, String)>,
    limits: ResourceLimits,
}

impl MapResolver {
    pub fn new(limits: ResourceLimits) -> Self {
        MapResolver {
            entries: HashMap::new(),
            limits,
        }
    }

    /// Grant a resource. Rejects unsafe keys and oversized payloads.
    pub fn grant(
        &mut self,
        raw_key: &str,
        bytes: Arc<[u8]>,
        license_hint: impl Into<String>,
    ) -> CadResult<()> {
        if !is_safe_reference(raw_key) {
            return Err(CadError::ResourceMissing(format!(
                "resource key '{raw_key}' was rejected by the path policy"
            )));
        }
        if bytes.len() > self.limits.max_bytes {
            return Err(CadError::ResourceMissing(format!(
                "resource '{raw_key}' is {} bytes, over the {} byte limit",
                bytes.len(),
                self.limits.max_bytes
            )));
        }
        let key = ResourceKey::sanitize(raw_key).0;
        self.entries.insert(key, (bytes, license_hint.into()));
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ResourceResolver for MapResolver {
    fn resolve(&self, request: &ResourceRequest) -> CadResult<ResourceData> {
        match self.entries.get(&request.key.0) {
            Some((bytes, license)) => Ok(ResourceData {
                bytes: bytes.clone(),
                version: 1,
                license_hint: Some(license.clone()),
            }),
            None => Err(CadError::ResourceMissing(format!(
                "resource '{}' is not available; a substitute may be requested",
                request.key.0
            ))),
        }
    }
}

/// The ordered resolver chain the spec requires: user pack, document map,
/// bundled set. The first hit wins; a total miss is an explicit error.
pub struct ResolverChain<'a> {
    pub user_pack: Option<&'a dyn ResourceResolver>,
    pub document_map: Option<&'a dyn ResourceResolver>,
    pub bundled: Option<&'a dyn ResourceResolver>,
}

impl ResolverChain<'_> {
    pub fn resolve(&self, request: &ResourceRequest) -> CadResult<ResourceData> {
        for resolver in [self.user_pack, self.document_map, self.bundled]
            .into_iter()
            .flatten()
        {
            if let Ok(data) = resolver.resolve(request) {
                return Ok(data);
            }
        }
        Err(CadError::ResourceMissing(format!(
            "resource '{}' is not available in any resolver",
            request.key.0
        )))
    }
}

/// Placeholder resolver kept for hosts that have not installed platform access.
pub struct PendingResourceResolver;

impl ResourceResolver for PendingResourceResolver {
    fn resolve(&self, request: &ResourceRequest) -> CadResult<ResourceData> {
        Err(CadError::ResourceMissing(format!(
            "no resource resolver installed for '{}'",
            request.key.0
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(key: &str) -> ResourceRequest {
        ResourceRequest {
            document: DocumentId(1),
            key: ResourceKey::sanitize(key),
            kind: ResourceKind::FontShx,
        }
    }

    #[test]
    fn rejects_absolute_and_traversal_references() {
        assert!(!is_safe_reference("/etc/passwd"));
        assert!(!is_safe_reference("C:\\Windows\\font.ttf"));
        assert!(!is_safe_reference("../../secret.ttf"));
        assert!(!is_safe_reference("http://example.com/x.shx"));
        assert!(is_safe_reference("romans.shx"));
        assert!(is_safe_reference("fonts/simplex.shx"));
    }

    #[test]
    fn sanitize_strips_directories_and_lowercases() {
        assert_eq!(bare_name("fonts/simplex.shx"), "simplex.shx");
        assert_eq!(
            ResourceKey::sanitize("dir/Romans.SHX").as_str(),
            "romans.shx"
        );
    }

    #[test]
    fn chain_follows_priority_order() {
        let mut user = MapResolver::new(ResourceLimits::default());
        user.grant("romans.shx", Arc::from(b"user".to_vec()), "user pack")
            .unwrap();
        let mut bundled = MapResolver::new(ResourceLimits::default());
        bundled
            .grant("romans.shx", Arc::from(b"bundled".to_vec()), "bundled")
            .unwrap();
        let empty = MapResolver::new(ResourceLimits::default());
        let chain = ResolverChain {
            user_pack: Some(&user),
            document_map: Some(&empty),
            bundled: Some(&bundled),
        };
        let data = chain.resolve(&request("romans.shx")).unwrap();
        assert_eq!(&*data.bytes, b"user");
    }

    #[test]
    fn missing_resource_is_reported_not_faked() {
        let empty = MapResolver::new(ResourceLimits::default());
        assert!(empty.resolve(&request("missing.shx")).is_err());
        assert!(PendingResourceResolver.resolve(&request("x.shx")).is_err());
    }

    #[test]
    fn unsafe_grant_is_rejected() {
        let mut r = MapResolver::new(ResourceLimits::default());
        assert!(r
            .grant("../evil.shx", Arc::from(b"x".to_vec()), "x")
            .is_err());
    }

    const CATALOG: &str = r#"[
        { "file": "simplex.shx", "name": ["simplex"], "type": "shx" },
        { "file": "@extfont2.shx", "name": ["@extfont2"], "type": "shx", "encoding": "shift-jis" },
        { "file": "AMGDT DWE Edits.shx", "name": ["AMGDT DWE Edits"], "type": "shx" },
        { "file": "simsun.woff", "name": ["SimSun", "宋体"], "type": "mesh" }
    ]"#;

    #[test]
    fn font_catalog_parses_and_indexes_by_name_file_and_stem() {
        let catalog = FontCatalog::from_json(CATALOG).unwrap();
        assert_eq!(catalog.len(), 4);
        // By declared name, case-insensitively and with a directory prefix.
        assert_eq!(catalog.get("Simplex").unwrap().file, "simplex.shx");
        assert_eq!(
            catalog.get("fonts/SIMPLEX.SHX").unwrap().file,
            "simplex.shx"
        );
        // By file name.
        assert_eq!(catalog.get("simsun.woff").unwrap().kind, FontKind::Mesh);
        // By file stem.
        assert_eq!(catalog.get("simsun").unwrap().file, "simsun.woff");
        // Unknown fonts resolve to nothing rather than a fabricated face.
        assert!(catalog.get("no-such-font").is_none());
    }

    #[test]
    fn font_catalog_records_kind_and_encoding() {
        let catalog = FontCatalog::from_json(CATALOG).unwrap();
        let ext = catalog.get("@extfont2").unwrap();
        assert_eq!(ext.kind, FontKind::Shx);
        assert_eq!(ext.encoding.as_deref(), Some("shift-jis"));
        let sun = catalog.get("宋体").unwrap();
        assert_eq!(sun.kind, FontKind::Mesh);
    }

    #[test]
    fn font_urls_encode_spaces_and_default_to_the_mlightcad_catalog() {
        let catalog = FontCatalog::from_json(CATALOG).unwrap();
        let face = catalog.get("AMGDT DWE Edits").unwrap();
        assert_eq!(
            default_font_url(face),
            "https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/AMGDT%20DWE%20Edits.shx"
        );
        let plain = catalog.get("simplex").unwrap();
        assert_eq!(
            face_url("https://example.com/f/", plain),
            "https://example.com/f/simplex.shx"
        );
    }

    #[test]
    fn malformed_font_catalog_is_rejected() {
        assert!(FontCatalog::from_json("not json").is_err());
        assert!(FontCatalog::from_json("{}").is_err());
        // Missing/blank file entries are skipped, not panicked on.
        let catalog = FontCatalog::from_json(r#"[{"name":["x"]},{"file":""}]"#).unwrap();
        assert!(catalog.is_empty());
    }

    #[test]
    fn font_plan_maps_requests_to_urls_and_dedups_files() {
        let catalog = FontCatalog::from_json(CATALOG).unwrap();
        let requested = vec![
            "SimSun".to_string(),
            "宋体".to_string(),
            "simplex.shx".to_string(),
            "missing.ttf".to_string(),
        ];
        let plan = plan_fonts(&catalog, &requested, DEFAULT_FONT_BASE_URL);
        // SimSun and 宋体 share one file; simplex resolves; missing drops.
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].file, "simsun.woff");
        assert_eq!(plan[0].request, "SimSun");
        assert!(plan[0].url.starts_with(DEFAULT_FONT_BASE_URL));
        assert_eq!(plan[1].file, "simplex.shx");
        assert_eq!(plan[1].kind, FontKind::Shx);
    }
}
