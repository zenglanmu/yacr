//! Shared UI boundary: a real Slint shell compositing a wgpu-produced CAD frame.
//!
//! Spec v2.0 §5: a single presentation coordinator (Slint) owns the window and
//! surface; the CAD renderer draws into a texture that Slint composites. The UI
//! never creates GPU objects and never draws CAD entities itself.

use std::cell::RefCell;
use std::rc::Rc;

use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, DocumentId, ViewportId};

/// Source of the shared shell, kept for packaging/documentation tooling.
pub const UI_DEFINITION: &str = include_str!("../ui/app.slint");
/// Default (Chinese) UI strings; UI text is externalised per spec §3.5.
pub const ZH_CN_MESSAGES: &str = include_str!("../i18n/zh-CN.json");

slint::include_modules!();

pub mod bridge;
pub use bridge::{install as install_cad_bridge, IncomingDocument};

use slint::{ComponentHandle, Image, Weak};

/// Layout/locale configuration for the shell.
#[derive(Debug, Clone)]
pub struct UiConfiguration {
    pub compact: bool,
    pub locale: String,
    pub safe_insets: [f64; 4],
    pub application_title: String,
    pub document: DocumentId,
    pub viewport: ViewportId,
}

impl Default for UiConfiguration {
    fn default() -> Self {
        UiConfiguration {
            compact: false,
            locale: "zh-CN".to_string(),
            safe_insets: [0.0; 4],
            application_title: "yacr CAD".to_string(),
            document: DocumentId(0),
            viewport: ViewportId(0),
        }
    }
}

/// Receives commands emitted by the UI; the application executes them.
pub trait UiCommandSink: 'static {
    fn send(&mut self, command: Command) -> CadResult<()>;
}

/// Cloneable handle the host uses to push state into the UI.
#[derive(Clone)]
pub struct UiHandle {
    ui: Weak<YacrWindow>,
}

impl UiHandle {
    fn with(&self, f: impl FnOnce(&YacrWindow)) -> CadResult<()> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        f(&ui);
        Ok(())
    }

    /// Replace the composited CAD frame with a new texture-backed image.
    pub fn set_cad_frame(&self, image: Image) -> CadResult<()> {
        self.with(|ui| ui.set_cad_frame(image))
    }

    pub fn set_status(&self, status: impl Into<slint::SharedString>) -> CadResult<()> {
        let s = status.into();
        self.with(|ui| ui.set_status_label(s))
    }

    pub fn set_work_mode(&self, work: bool) -> CadResult<()> {
        self.with(|ui| ui.set_work_mode(work))
    }

    /// Trigger a redraw without restarting the event loop.
    pub fn request_redraw(&self) -> CadResult<()> {
        self.with(|ui| ui.window().request_redraw())
    }

    /// Current physical size of the window, if it still exists.
    pub fn physical_size(&self) -> Option<slint::PhysicalSize> {
        Some(self.ui.upgrade()?.window().size())
    }
}

/// Owns the Slint component and routes UI callbacks into commands.
pub struct UiAdapter {
    pub configuration: UiConfiguration,
    ui: YacrWindow,
}

impl UiAdapter {
    /// Build the shell and connect its callbacks to `sink`.
    pub fn new<S: UiCommandSink>(configuration: UiConfiguration, sink: S, work_mode: bool) -> CadResult<Self> {
        let ui = YacrWindow::new().map_err(|e| CadError::Invariant(format!("slint: {e}")))?;
        ui.set_application_title(configuration.application_title.clone().into());
        ui.set_open_label("打开".into());
        ui.set_measure_label("测量".into());
        ui.set_annotate_label("批注".into());
        ui.set_status_label("就绪".into());
        ui.set_work_mode(work_mode);

        let document = configuration.document.clone();
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));

        // A command is built inside each callback: Command is not Clone and the
        // payload set is heterogeneous.
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_open_requested(move || {
                let _ = s.borrow_mut().send(Command {
                    schema_version: 1,
                    id: CommandId::OpenDrawing,
                    document: doc.clone(),
                    viewport,
                    payload: CommandPayload::None,
                });
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_measure_requested(move || {
                let _ = s.borrow_mut().send(Command {
                    schema_version: 1,
                    id: CommandId::Measure,
                    document: doc.clone(),
                    viewport,
                    payload: CommandPayload::None,
                });
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_annotate_requested(move || {
                let _ = s.borrow_mut().send(Command {
                    schema_version: 1,
                    id: CommandId::CreateAnnotation,
                    document: doc.clone(),
                    viewport,
                    payload: CommandPayload::None,
                });
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_fit_requested(move || {
                let _ = s.borrow_mut().send(Command {
                    schema_version: 1,
                    id: CommandId::FitDrawing,
                    document: doc.clone(),
                    viewport,
                    payload: CommandPayload::None,
                });
            });
        }

        Ok(UiAdapter { configuration, ui })
    }

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle { ui: self.ui.as_weak() }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }

    /// Show the window and run the platform event loop.
    ///
    /// On Android this is called after `slint::android::init()`; on desktop it
    /// is the winit event loop.
    pub fn run(&self) -> CadResult<()> {
        self.ui
            .show()
            .map_err(|e| CadError::Invariant(format!("slint show: {e}")))?;
        slint::run_event_loop().map_err(|e| CadError::Invariant(format!("slint loop: {e}")))?;
        Ok(())
    }

    /// The component, for hosts that need to install a rendering notifier.
    pub fn component(&self) -> &YacrWindow {
        &self.ui
    }
}

/// Ask Slint to render with wgpu so a CAD renderer can share the device.
///
/// Must be called before creating any window. On Android the backend is Skia
/// behind wgpu (`unstable-wgpu-30`), which is what makes a shared texture
/// possible at all; see `docs/render-backends.md`.
pub fn select_wgpu_backend() -> CadResult<()> {
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::default())
        .select()
        .map_err(|e| CadError::GpuFailure(format!("slint wgpu backend: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_definition_mentions_a_cad_frame_property() {
        assert!(UI_DEFINITION.contains("cad-frame"));
        assert!(ZH_CN_MESSAGES.contains('{'));
    }
}
