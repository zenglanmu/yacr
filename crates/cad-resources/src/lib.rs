//! Logical keys only; explicit grants precede any external access.
//!
//! Spec v2.0 §7.3, §16.2: references are logical keys, never platform paths. A
//! crafted drawing must not be able to direct the app outside its allowed
//! roots, so every reference is sanitised to a bare file name and the resolver
//! chain is explicit and ordered.

use cad_domain::*;
use std::collections::HashMap;
use std::sync::Arc;

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
    let cleaned: String = last.chars().filter(|c| !c.is_control() && *c != '\0').collect();
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
        MapResolver { entries: HashMap::new(), limits }
    }

    /// Grant a resource. Rejects unsafe keys and oversized payloads.
    pub fn grant(&mut self, raw_key: &str, bytes: Arc<[u8]>, license_hint: impl Into<String>) -> CadResult<()> {
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
        for resolver in [self.user_pack, self.document_map, self.bundled].into_iter().flatten() {
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
        ResourceRequest { document: DocumentId(1), key: ResourceKey::sanitize(key), kind: ResourceKind::FontShx }
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
        assert_eq!(ResourceKey::sanitize("dir/Romans.SHX").as_str(), "romans.shx");
    }

    #[test]
    fn chain_follows_priority_order() {
        let mut user = MapResolver::new(ResourceLimits::default());
        user.grant("romans.shx", Arc::from(b"user".to_vec()), "user pack").unwrap();
        let mut bundled = MapResolver::new(ResourceLimits::default());
        bundled.grant("romans.shx", Arc::from(b"bundled".to_vec()), "bundled").unwrap();
        let empty = MapResolver::new(ResourceLimits::default());
        let chain = ResolverChain { user_pack: Some(&user), document_map: Some(&empty), bundled: Some(&bundled) };
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
        assert!(r.grant("../evil.shx", Arc::from(b"x".to_vec()), "x").is_err());
    }
}
