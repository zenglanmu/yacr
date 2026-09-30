//! Public proxy cache decoding only; raw DWG records are not graphic_data.
use cad_domain::*;
pub struct DecodeLimits { pub max_bytes: usize, pub max_records: usize, pub max_vertices: usize, pub max_stack_depth: usize }
pub struct ProxySource { pub handle: String, pub class_name: String, pub application: Option<String>, pub dwg_version: String }
pub struct ProxyRecord<'a> { pub record_type: u32, pub data: &'a [u8] }
pub struct ReplayState { pub transform: Transform3, pub rgba: [u8; 4], pub fill: bool, pub state_reliable: bool }
pub struct ProxyOutput { pub geometry: Vec<SemanticGeometry>, pub completeness: Completeness, pub diagnostics: Vec<Diagnostic> }
pub trait ProxyRecordDecoder {
    fn registration(&self) -> Registration;
    fn decode(&self, record: &ProxyRecord<'_>, state: &mut ReplayState, limits: &DecodeLimits) -> CadResult<Vec<SemanticGeometry>>;
}
pub struct ProxyPlayer { pub limits: DecodeLimits }
impl ProxyPlayer {
    pub fn replay(&self, _source: &ProxySource, _graphic_data: &[u8]) -> CadResult<ProxyOutput> { pending("proxy.replay") }
    pub fn inspect_raw_dwg(&self, _raw: &[u8]) -> CadResult<ProxyOutput> { pending("proxy.raw_dwg_separate_parser") }
}
