use cad_domain::*;
pub struct MemoryBudget {
    pub file_bytes: u64,
    pub domain_bytes: u64,
    pub cpu_geometry_bytes: u64,
    pub gpu_estimated_bytes: u64,
    pub atlas_bytes: u64,
    pub attachment_bytes: u64,
}
pub struct LoadTimings {
    pub parse_ms: f64,
    pub build_ms: f64,
    pub upload_ms: f64,
    pub first_usable_ms: f64,
    pub complete_ms: f64,
}
pub struct BenchmarkContext {
    pub sample_hash: [u8; 32],
    pub device: String,
    pub browser: Option<String>,
    pub release_build: bool,
    pub viewport: ViewportId,
    pub quality_configuration: String,
}
pub struct DiagnosticsModel {
    pub entries: Vec<Diagnostic>,
    pub completeness: Completeness,
}
pub struct DiagnosticPackage {
    pub build_version: String,
    pub upstream_commit: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub memory: MemoryBudget,
    pub timings: LoadTimings,
    pub backend: String,
}
impl DiagnosticPackage {
    /// Must exclude source bytes, text, full paths and user identity by default.
    pub fn encode_redacted(&self) -> CadResult<Vec<u8>> {
        pending("diagnostics.redacted_package")
    }
}
