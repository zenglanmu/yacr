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

/// Category of an external resource.
///
/// The category is what the capability table and budgets key off; it is
/// deliberately finer-grained than a raw `ResourceKind` so BigFont and image
/// data can be reported separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    FontTtf,
    FontShx,
    BigFont,
    Image,
    ExternalReference,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::FontTtf => "font_ttf",
            ResourceKind::FontShx => "font_shx",
            ResourceKind::BigFont => "big_font",
            ResourceKind::Image => "image",
            ResourceKind::ExternalReference => "external_reference",
        }
    }
}

/// What this crate can actually do with a resource category.
///
/// The audit (F10/F11) found the old model implied every [`ResourceKind`] was
/// supported because the enum existed. This table states the truth: planning a
/// fetch URL is not decoding, and BigFont/image/xref are not implemented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceCapability {
    pub kind: ResourceKind,
    /// Whether a logical fetch URL / resolver lookup is available.
    pub resolve: SupportStatus,
    /// Whether the bytes can be turned into glyphs/geometry anywhere in core.
    pub decode: SupportStatus,
}

/// The capability table for every known resource category.
///
/// `resolve` is what `cad-resources` itself provides; `decode` records the
/// upstream state so a report cannot claim a category is fully supported.
/// These values must be updated alongside the corresponding implementation,
/// never pre-emptively.
pub fn resource_capabilities() -> Vec<ResourceCapability> {
    vec![
        ResourceCapability {
            kind: ResourceKind::FontTtf,
            resolve: SupportStatus::Verified,
            // TTF/OTF/WOFF1 outlining exists but has no authorized-font evidence.
            decode: SupportStatus::Unverified,
        },
        ResourceCapability {
            kind: ResourceKind::FontShx,
            resolve: SupportStatus::Verified,
            // SHX shape fonts parse, but bigfont/encoding coverage is untested.
            decode: SupportStatus::Partial,
        },
        ResourceCapability {
            kind: ResourceKind::BigFont,
            resolve: SupportStatus::Unverified,
            // No dedicated big-font handling exists.
            decode: SupportStatus::NotImplemented,
        },
        ResourceCapability {
            kind: ResourceKind::Image,
            resolve: SupportStatus::NotImplemented,
            decode: SupportStatus::NotImplemented,
        },
        ResourceCapability {
            kind: ResourceKind::ExternalReference,
            resolve: SupportStatus::NotImplemented,
            decode: SupportStatus::NotImplemented,
        },
    ]
}

/// Look up the capability for one category.
pub fn resource_capability(kind: ResourceKind) -> ResourceCapability {
    resource_capabilities()
        .into_iter()
        .find(|capability| capability.kind == kind)
        .expect("resource capability table must cover every ResourceKind")
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

/// Stable, machine-readable resource diagnostic codes (schema keys).
pub mod codes {
    /// A resource is not available from any resolver.
    pub const RESOURCE_MISSING: &str = "resource.missing";
    /// A resource exceeds a configured budget.
    pub const RESOURCE_OVER_BUDGET: &str = "resource.over_budget";
    /// A reference nested deeper than the configured recursion limit.
    pub const RESOURCE_RECURSION_LIMIT: &str = "resource.recursion_limit";
    /// A requested font name resolved to no catalog entry.
    pub const FONT_UNRESOLVED: &str = "resource.font_unresolved";
    /// A referenced font resolved but its technology is not supported.
    pub const FONT_UNSUPPORTED: &str = "resource.font_unsupported";
}

/// Which budget a [`ResourceIssue`] exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceBudget {
    /// A single resource's own size cap (`ResourceLimits::max_bytes`).
    PerResource,
    /// The running total of granted bytes (`ResourceLimits::total_bytes`).
    TotalBytes,
    /// The font-cache byte cap (`ResourceLimits::font_cache_bytes`).
    FontCache,
    /// The decoded-image pixel cap (`ResourceLimits::max_image_pixels`).
    ImagePixels,
    /// The external-reference recursion depth (`ResourceLimits::max_xref_depth`).
    XrefDepth,
}

impl ResourceBudget {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceBudget::PerResource => "per_resource",
            ResourceBudget::TotalBytes => "total_bytes",
            ResourceBudget::FontCache => "font_cache_bytes",
            ResourceBudget::ImagePixels => "image_pixels",
            ResourceBudget::XrefDepth => "xref_depth",
        }
    }
}

/// A structured, locale-independent reason a resource operation did not fully
/// succeed. Callers aggregate these instead of a bare `Err` string so an
/// over-budget drop cannot be mistaken for success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceIssue {
    pub code: String,
    pub kind: Option<ResourceKind>,
    pub budget: Option<ResourceBudget>,
    /// The measured value (bytes, pixels or depth) that triggered the issue.
    pub actual: u64,
    /// The configured limit that was exceeded.
    pub limit: u64,
    /// Logical key of the resource, when known (already sanitized).
    pub key: Option<String>,
}

impl ResourceIssue {
    fn new(code: &str) -> Self {
        ResourceIssue {
            code: code.to_string(),
            kind: None,
            budget: None,
            actual: 0,
            limit: 0,
            key: None,
        }
    }

    pub fn over_budget(
        key: Option<&str>,
        kind: Option<ResourceKind>,
        budget: ResourceBudget,
        actual: u64,
        limit: u64,
    ) -> Self {
        ResourceIssue {
            code: codes::RESOURCE_OVER_BUDGET.to_string(),
            kind,
            budget: Some(budget),
            actual,
            limit,
            key: key.map(str::to_string),
        }
    }

    pub fn recursion_limit(key: Option<&str>, actual: u64, limit: u64) -> Self {
        ResourceIssue {
            code: codes::RESOURCE_RECURSION_LIMIT.to_string(),
            kind: Some(ResourceKind::ExternalReference),
            budget: Some(ResourceBudget::XrefDepth),
            actual,
            limit,
            key: key.map(str::to_string),
        }
    }

    pub fn font_unresolved(request: &str) -> Self {
        let mut issue = Self::new(codes::FONT_UNRESOLVED);
        issue.kind = Some(ResourceKind::FontShx);
        issue.key = Some(ResourceKey::sanitize(request).0);
        issue
    }

    pub fn font_unsupported(request: &str, kind: &FontKind) -> Self {
        let mut issue = Self::new(codes::FONT_UNSUPPORTED);
        issue.kind = Some(match kind {
            FontKind::Shx => ResourceKind::FontShx,
            FontKind::Mesh => ResourceKind::FontTtf,
            FontKind::Other(_) => ResourceKind::FontTtf,
        });
        issue.key = Some(ResourceKey::sanitize(request).0);
        issue
    }

    /// A stable machine identifier for the category, when known.
    pub fn kind_name(&self) -> Option<&'static str> {
        self.kind.map(ResourceKind::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_bytes: usize,
    pub max_image_pixels: u64,
    pub max_xref_depth: usize,
    pub font_cache_bytes: usize,
    /// Running total of every granted resource; the previous model only capped a
    /// single payload, so an unbounded number of grants could exhaust memory
    /// (audit F10).
    pub total_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            max_bytes: 64 * 1024 * 1024,
            max_image_pixels: 8192 * 8192,
            max_xref_depth: 8,
            font_cache_bytes: 64 * 1024 * 1024,
            total_bytes: 256 * 1024 * 1024,
        }
    }
}

impl ResourceLimits {
    /// Enforce the image pixel cap for a declared decode size.
    pub fn check_image_pixels(&self, width: u64, height: u64) -> Result<u64, ResourceIssue> {
        let pixels = width.saturating_mul(height);
        if pixels > self.max_image_pixels {
            return Err(ResourceIssue::over_budget(
                None,
                Some(ResourceKind::Image),
                ResourceBudget::ImagePixels,
                pixels,
                self.max_image_pixels,
            ));
        }
        Ok(pixels)
    }

    /// Enforce the external-reference recursion limit at `depth`.
    pub fn check_xref_depth(&self, key: Option<&str>, depth: usize) -> Result<(), ResourceIssue> {
        if depth > self.max_xref_depth {
            return Err(ResourceIssue::recursion_limit(
                key,
                depth as u64,
                self.max_xref_depth as u64,
            ));
        }
        Ok(())
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
    used_bytes: usize,
}

impl MapResolver {
    pub fn new(limits: ResourceLimits) -> Self {
        MapResolver {
            entries: HashMap::new(),
            limits,
            used_bytes: 0,
        }
    }

    /// Grant a resource. Rejects unsafe keys, oversized payloads and, unlike the
    /// previous per-payload-only check, anything that pushes the running total
    /// over [`ResourceLimits::total_bytes`].
    ///
    /// Returns a structured [`ResourceIssue`] so an over-budget grant is an
    /// explicit diagnostic rather than a silent drop.
    pub fn grant(
        &mut self,
        raw_key: &str,
        bytes: Arc<[u8]>,
        license_hint: impl Into<String>,
    ) -> Result<(), ResourceIssue> {
        if !is_safe_reference(raw_key) {
            let mut issue = ResourceIssue::new(codes::RESOURCE_MISSING);
            issue.key = Some(ResourceKey::sanitize(raw_key).0);
            return Err(issue);
        }
        if bytes.len() > self.limits.max_bytes {
            return Err(ResourceIssue::over_budget(
                Some(&ResourceKey::sanitize(raw_key).0),
                None,
                ResourceBudget::PerResource,
                bytes.len() as u64,
                self.limits.max_bytes as u64,
            ));
        }
        if self.used_bytes.saturating_add(bytes.len()) > self.limits.total_bytes {
            return Err(ResourceIssue::over_budget(
                Some(&ResourceKey::sanitize(raw_key).0),
                None,
                ResourceBudget::TotalBytes,
                self.used_bytes.saturating_add(bytes.len()) as u64,
                self.limits.total_bytes as u64,
            ));
        }
        let key = ResourceKey::sanitize(raw_key).0;
        if let Some((previous, _)) = self
            .entries
            .insert(key, (bytes.clone(), license_hint.into()))
        {
            self.used_bytes = self.used_bytes.saturating_sub(previous.len());
        }
        self.used_bytes = self.used_bytes.saturating_add(bytes.len());
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes currently granted against this resolver's total budget.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
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

    #[test]
    fn grant_enforces_the_running_total_budget() {
        let limits = ResourceLimits {
            max_bytes: 1024,
            total_bytes: 100,
            ..ResourceLimits::default()
        };
        let mut resolver = MapResolver::new(limits);
        resolver
            .grant("a.shx", Arc::from(vec![0u8; 60]), "pack")
            .unwrap();
        assert_eq!(resolver.used_bytes(), 60);
        // A second grant that would push the total over 100 is rejected with a
        // structured over-budget issue, not silently dropped.
        let issue = resolver
            .grant("b.shx", Arc::from(vec![0u8; 60]), "pack")
            .unwrap_err();
        assert_eq!(issue.code, codes::RESOURCE_OVER_BUDGET);
        assert_eq!(issue.budget, Some(ResourceBudget::TotalBytes));
        assert_eq!(issue.actual, 120);
        assert_eq!(issue.limit, 100);
        // The rejected grant did not consume budget.
        assert_eq!(resolver.used_bytes(), 60);
        // A later grant that fits still succeeds.
        resolver
            .grant("c.shx", Arc::from(vec![0u8; 40]), "pack")
            .unwrap();
        assert_eq!(resolver.used_bytes(), 100);
    }

    #[test]
    fn per_resource_and_pixel_budgets_are_explicit() {
        let limits = ResourceLimits {
            max_bytes: 10,
            max_image_pixels: 100,
            ..ResourceLimits::default()
        };
        let mut resolver = MapResolver::new(limits.clone());
        let issue = resolver
            .grant("big.shx", Arc::from(vec![0u8; 11]), "pack")
            .unwrap_err();
        assert_eq!(issue.budget, Some(ResourceBudget::PerResource));
        // Image decoding is capped before allocation.
        assert!(limits.check_image_pixels(10, 10).is_ok());
        let issue = limits.check_image_pixels(11, 10).unwrap_err();
        assert_eq!(issue.budget, Some(ResourceBudget::ImagePixels));
        assert_eq!(issue.kind, Some(ResourceKind::Image));
    }

    #[test]
    fn xref_recursion_limit_is_enforced() {
        let limits = ResourceLimits {
            max_xref_depth: 2,
            ..ResourceLimits::default()
        };
        assert!(limits.check_xref_depth(Some("base.dwg"), 2).is_ok());
        let issue = limits.check_xref_depth(Some("base.dwg"), 3).unwrap_err();
        assert_eq!(issue.code, codes::RESOURCE_RECURSION_LIMIT);
        assert_eq!(issue.budget, Some(ResourceBudget::XrefDepth));
        assert_eq!(issue.actual, 3);
        assert_eq!(issue.limit, 2);
    }

    #[test]
    fn capability_table_does_not_claim_unsupported_categories() {
        let table = resource_capabilities();
        // Every category is described exactly once.
        for kind in [
            ResourceKind::FontTtf,
            ResourceKind::FontShx,
            ResourceKind::BigFont,
            ResourceKind::Image,
            ResourceKind::ExternalReference,
        ] {
            assert_eq!(
                table.iter().filter(|c| c.kind == kind).count(),
                1,
                "missing capability for {}",
                kind.as_str()
            );
        }
        // The old model implied every kind was supported. Image and xref are
        // not implemented and must say so.
        assert_eq!(
            resource_capability(ResourceKind::Image).decode,
            SupportStatus::NotImplemented
        );
        assert_eq!(
            resource_capability(ResourceKind::ExternalReference).resolve,
            SupportStatus::NotImplemented
        );
        assert_eq!(
            resource_capability(ResourceKind::BigFont).decode,
            SupportStatus::NotImplemented
        );
        // Fonts are resolvable but not yet verified as fully decoded.
        assert_ne!(
            resource_capability(ResourceKind::FontTtf).decode,
            SupportStatus::Verified
        );
    }

    #[test]
    fn font_plan_report_accounts_for_unresolved_and_unsupported() {
        let catalog = FontCatalog::from_json(CATALOG).unwrap();
        // Extend the catalog with an unknown-technology entry: it must not be
        // planned as if it were usable.
        let catalog_with_woff2 = FontCatalog::from_json(
            r#"[
                { "file": "simplex.shx", "name": ["simplex"], "type": "shx" },
                { "file": "modern.woff2", "name": ["modern"], "type": "woff2" }
            ]"#,
        )
        .unwrap();
        let requested = vec![
            "simplex".to_string(),
            "modern".to_string(),
            "ghost.ttf".to_string(),
        ];
        let report = plan_fonts_report(&catalog_with_woff2, &requested, DEFAULT_FONT_BASE_URL);
        assert_eq!(report.planned.len(), 1);
        assert_eq!(report.planned[0].file, "simplex.shx");
        assert_eq!(report.unresolved, vec!["ghost.ttf".to_string()]);
        assert_eq!(report.unsupported, vec!["modern".to_string()]);
        assert!(!report.is_complete());
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == codes::FONT_UNRESOLVED));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == codes::FONT_UNSUPPORTED));
        // The legacy `plan_fonts` still returns just the planned faces.
        assert_eq!(
            plan_fonts(&catalog, &requested, DEFAULT_FONT_BASE_URL).len(),
            1
        );
    }
}
