//! resolver module.

use super::*;

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
        let key = ResourceKey::sanitize(raw_key).0;
        let previous_bytes = self
            .entries
            .get(&key)
            .map_or(0, |(previous, _)| previous.len());
        let resulting_bytes = self.used_bytes - previous_bytes;
        let total = resulting_bytes.checked_add(bytes.len());
        if total.is_none_or(|total| total > self.limits.total_bytes) {
            return Err(ResourceIssue::over_budget(
                Some(&ResourceKey::sanitize(raw_key).0),
                None,
                ResourceBudget::TotalBytes,
                resulting_bytes.saturating_add(bytes.len()) as u64,
                self.limits.total_bytes as u64,
            ));
        }
        self.entries.insert(key, (bytes, license_hint.into()));
        self.used_bytes = total.expect("total was checked against the budget");
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
/// Only `ResourceMissing` permits fallback; other errors are propagated.
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
            match resolver.resolve(request) {
                Ok(data) => return Ok(data),
                Err(CadError::ResourceMissing(_)) => continue,
                Err(error) => return Err(error),
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
