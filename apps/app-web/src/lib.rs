//! Browser assembly slot; no wasm-bindgen export or working canvas yet.
use cad_domain::*;
pub struct WebHostConfiguration {
    pub ui_canvas_id: String,
    pub cad_canvas_id: String,
    pub recovery_enabled: bool,
}
pub fn start(_configuration: WebHostConfiguration) -> CadResult<()> {
    pending("host.web.file_api_canvas_composition")
}
