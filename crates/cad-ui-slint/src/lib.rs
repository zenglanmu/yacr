//! Shared UI boundary. Slint compilation/integration is not yet wired.
use cad_app::Command;
use cad_domain::*;
pub const UI_DEFINITION: &str = include_str!("../ui/app.slint");
pub const ZH_CN_MESSAGES: &str = include_str!("../i18n/zh-CN.json");
pub struct UiConfiguration { pub compact: bool, pub locale: String, pub safe_insets: [f64; 4] }
pub trait UiCommandSink { fn send(&mut self, command: Command) -> CadResult<()>; }
pub struct UiAdapter { pub configuration: UiConfiguration }
impl UiAdapter {
    pub fn run(&mut self) -> CadResult<()> { pending("ui.slint_host_integration") }
    pub fn refresh(&mut self, _document: DocumentId, _revision: Revision) -> CadResult<()> { pending("ui.incremental_models") }
}
