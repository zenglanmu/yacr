//! Kernel objects must remain inside this adapter.
use cad_domain::*;
pub enum SolidExchange { Sat(Vec<u8>), Sab(Vec<u8>), Unsupported { type_key: String, data: Vec<u8> } }
pub struct TessellationRequest { pub data: SolidExchange, pub tolerance: TolerancePolicy, pub stamp: TaskStamp }
pub struct TessellationResult { pub mesh: Mesh, pub edges: Vec<Vec<Point3>>, pub precision: Precision, pub completeness: Completeness, pub stamp: TaskStamp }
pub trait SolidTessellator {
    fn registration(&self) -> Registration;
    fn tessellate(&self, request: &TessellationRequest, cancelled: &dyn Fn() -> bool) -> CadResult<TessellationResult>;
}
pub struct PendingTessellator;
impl SolidTessellator for PendingTessellator {
    fn registration(&self) -> Registration { Registration { type_key: "yacr.kernel.pending".into(), version: 1, priority: 0, entity_types: vec![], capabilities: vec![] } }
    fn tessellate(&self, _: &TessellationRequest, cancelled: &dyn Fn() -> bool) -> CadResult<TessellationResult> {
        if cancelled() { Err(CadError::Cancelled) } else { pending("kernel.tessellate") }
    }
}
