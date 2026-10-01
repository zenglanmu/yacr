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
///
/// Re-exported from [`i18n::ZH_CN_JSON`]; the catalog module owns the source of
/// truth. Kept for packaging/documentation tooling.
pub const ZH_CN_MESSAGES: &str = i18n::ZH_CN_JSON;

slint::include_modules!();

pub mod bridge;
pub mod i18n;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub use bridge::{install as install_cad_bridge, CadView, IncomingDocument};
pub use i18n::{Locale, LocaleResolution, Message, MessageCatalog, MessageSource};

use slint::{ComponentHandle, Image, Weak};

/// Navigator input routed from the shell's canvas into the CAD view.
pub trait ViewInput {
    /// kind: 0=down 1=up 2=move 3=cancel; button: 0=none 1=left 2=right 3=middle.
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64);
    fn scroll(&self, dx: f64, dy: f64);
}

/// Layout/locale configuration for the shell.
#[derive(Debug, Clone)]
pub struct UiConfiguration {
    pub compact: bool,
    pub locale: String,
    pub safe_insets: [f64; 4],
    pub application_title: String,
    pub document: DocumentId,
    pub viewport: ViewportId,
    /// Initial logical window size; hosts keep it in sync with the surface.
    pub logical_size: [f64; 2],
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
            logical_size: [1280.0, 800.0],
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

    pub fn set_can_undo(&self, can_undo: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_undo(can_undo))
    }

    pub fn set_backend_index(&self, index: i32) -> CadResult<()> {
        self.with(|ui| ui.set_backend_index(index))
    }

    /// Re-apply the catalog for `locale` and update the UI labels.
    ///
    /// Returns the resolution actually applied; callers can log a fallback. Full
    /// UI-chrome translation (every button, HTML lang, preference persistence) is
    /// the ui3d/host workstream's job — this is the catalog/core hook.
    pub fn set_locale(&self, locale: &str) -> CadResult<LocaleResolution> {
        let messages = MessageSource::from_request(locale);
        let updated = messages.clone();
        self.with(|ui| {
            ui.set_open_label(updated.text("file.open", &[]).into());
        })?;
        Ok(messages.resolution().clone())
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
    view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>>,
}

/// Build the command a shell callback emits for the configured document.
fn command_for(
    id: CommandId,
    document: &DocumentId,
    viewport: ViewportId,
    payload: CommandPayload,
) -> Command {
    Command {
        schema_version: 1,
        id,
        document: document.clone(),
        viewport,
        payload,
    }
}

impl UiAdapter {
    /// Build the shell and connect its callbacks to `sink`.
    pub fn new<S: UiCommandSink>(
        configuration: UiConfiguration,
        sink: S,
        work_mode: bool,
    ) -> CadResult<Self> {
        let ui = YacrWindow::new().map_err(|e| CadError::Invariant(format!("slint: {e}")))?;
        ui.set_application_title(configuration.application_title.clone().into());
        ui.window().set_size(slint::LogicalSize::new(
            configuration.logical_size[0].max(1.0) as f32,
            configuration.logical_size[1].max(1.0) as f32,
        ));
        // The configured locale selects the catalog; no Rust string literals for
        // user-facing text live here (N01). Fallbacks are recorded, never silent.
        let messages = MessageSource::from_request(&configuration.locale);
        if let Some(reason) = messages.resolution().fallback {
            log::info!(
                "locale {:?} resolved to {} ({:?})",
                messages.resolution().requested,
                messages.locale().tag(),
                reason
            );
        }
        ui.set_open_label(messages.text("file.open", &[]).into());
        ui.set_status_label(messages.text("status.scaffold", &[]).into());
        ui.set_work_mode(work_mode);

        let document = configuration.document.clone();
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));
        let view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>> = Rc::new(RefCell::new(None));

        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_open_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::OpenDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_fit_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::FitDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_measure_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Measure,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_annotate_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CreateAnnotation,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_undo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Undo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_redo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Redo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_backend_selected(move |name| {
                let choice = match name.as_str() {
                    "WebGPU" => cad_app::BackendChoice::WebGpu,
                    "WebGL2" => cad_app::BackendChoice::WebGl2,
                    _ => cad_app::BackendChoice::Auto,
                };
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchBackend,
                    &doc,
                    viewport,
                    CommandPayload::Backend(choice),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_export_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ExportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_import_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ImportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_diagnostics_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Diagnostics,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let input = view_input.clone();
            ui.on_pointer_input(move |kind, button, x, y| {
                if let Some(input) = input.borrow().as_ref() {
                    input.pointer(kind, button, x as f64, y as f64);
                }
            });
        }
        {
            let input = view_input.clone();
            ui.on_scroll_input(move |dx, dy| {
                if let Some(input) = input.borrow().as_ref() {
                    input.scroll(dx as f64, dy as f64);
                }
            });
        }

        Ok(UiAdapter {
            configuration,
            ui,
            view_input,
        })
    }

    /// Route canvas input to the CAD view; call once the view exists.
    pub fn set_view_input(&self, input: Rc<dyn ViewInput>) {
        *self.view_input.borrow_mut() = Some(input);
    }

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle {
            ui: self.ui.as_weak(),
        }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }

    /// Show the window and run the platform event loop.
    ///
    /// On Android this is called after `slint::android::init()`; on desktop it
    /// is the winit event loop; on the web it hands over to the browser event
    /// loop (the call may not return — see `apps/app-web`).
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
/// possible at all; on the web `web::select_backend` chooses WebGPU or WebGL2.
/// See `docs/render-backends.md`.
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

    #[test]
    fn catalog_drives_ui_defaults() {
        // The shell no longer hardcodes its initial labels: they come from the
        // embedded catalog, so both languages stay reachable through config.
        assert_eq!(
            MessageSource::for_locale(Locale::ZhCn).text("file.open", &[]),
            "打开图纸"
        );
        assert_eq!(
            MessageSource::for_locale(Locale::En).text("file.open", &[]),
            "Open drawing"
        );
    }
}
