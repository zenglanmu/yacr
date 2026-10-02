//! bundle module.

use super::*;

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
    pub(crate) fn new(code: &str) -> Self {
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
