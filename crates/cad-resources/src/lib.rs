//! Logical keys only; explicit grants precede any external access.
use cad_domain::*;
use std::sync::Arc;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceKey(pub String);
pub enum ResourceKind { FontTtf, FontShx, BigFont, Image, ExternalReference }
pub struct ResourceRequest { pub document: DocumentId, pub key: ResourceKey, pub kind: ResourceKind }
pub struct ResourceData { pub bytes: Arc<[u8]>, pub version: u64, pub license_hint: Option<String> }
pub struct ResourceLimits { pub max_bytes: usize, pub max_image_pixels: u64, pub max_xref_depth: usize, pub font_cache_bytes: usize }
pub trait ResourceResolver { fn resolve(&self, request: &ResourceRequest) -> CadResult<ResourceData>; }
pub struct PendingResourceResolver;
impl ResourceResolver for PendingResourceResolver {
    fn resolve(&self, _: &ResourceRequest) -> CadResult<ResourceData> { pending("resources.resolve") }
}
