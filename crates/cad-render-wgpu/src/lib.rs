//! wgpu integration contract. Actual device creation and shader pipelines are pending.
use cad_domain::*;
use cad_scene::SceneDelta;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPreference { Auto, WebGpu, WebGl2 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend { WebGpu, WebGl2, Native }
pub struct BackendCapabilities { pub actual: ActiveBackend, pub compute: bool, pub storage_buffers: bool, pub indirect_draw: bool, pub max_texture_dimension: u32 }
pub struct RenderTarget { pub host_texture: u64, pub width: u32, pub height: u32, pub device_generation: u64, pub premultiplied_alpha: bool, pub srgb: bool, pub samples: u32 }
pub struct FrameStats { pub cpu_ms: f64, pub gpu_ms: Option<f64>, pub draw_calls: u64, pub uploaded_bytes: u64 }
pub struct Renderer { pub preference: BackendPreference, pub recovery_limit: u32 }
impl Renderer {
    pub fn initialize(&mut self, _target: &RenderTarget) -> CadResult<BackendCapabilities> { pending("render.initialize_shared_device") }
    pub fn upload(&mut self, _delta: &SceneDelta) -> CadResult<()> { pending("render.upload") }
    pub fn render(&mut self, _viewport: ViewportId, _target: &RenderTarget) -> CadResult<FrameStats> { pending("render.frame") }
    pub fn rebuild_device(&mut self, _target: &RenderTarget) -> CadResult<()> { pending("render.bounded_device_recovery") }
    pub fn switch_backend(&mut self, _preference: BackendPreference) -> CadResult<()> { pending("render.switch_backend_after_save_guard") }
}
