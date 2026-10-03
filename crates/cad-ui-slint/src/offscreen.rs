//! Native validation platform: the production Slint component rendered into a wgpu texture.
//! No window server, emulator, or alternate UI implementation.
use cad_render_wgpu::{headless, wgpu, BackendPreference};
use slint::platform::{
    femtovg_renderer::FemtoVGWGPURenderer, Platform, PlatformError, WindowAdapter,
};
use std::{cell::Cell, rc::Rc};

struct OffscreenWindow {
    window: slint::Window,
    renderer: FemtoVGWGPURenderer,
    size: Cell<slint::PhysicalSize>,
}
impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }
    fn size(&self) -> slint::PhysicalSize {
        self.size.get()
    }
    fn set_size(&self, size: slint::WindowSize) {
        self.size.set(size.to_physical(1.0));
        self.window
            .dispatch_event(slint::platform::WindowEvent::Resized {
                size: size.to_logical(1.0),
            });
    }
    fn request_redraw(&self) {}
}
struct OffscreenPlatform;
impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let gpu =
            headless::create_headless_gpu(BackendPreference::Auto).map_err(|e| e.to_string())?;
        if gpu.adapter.backend != "vulkan" || gpu.adapter.device_type != "cpu" {
            return Err(format!(
                "offscreen validation requires software Vulkan, got {:?}",
                gpu.adapter
            )
            .into());
        }
        eprintln!("Slint offscreen adapter: {:?}", gpu.adapter);
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let renderer =
            FemtoVGWGPURenderer::new(wgpu::Instance::new(descriptor), gpu.device, gpu.queue)?;
        Ok(Rc::new_cyclic(|weak: &std::rc::Weak<OffscreenWindow>| {
            OffscreenWindow {
                window: slint::Window::new(weak.clone()),
                renderer,
                size: Cell::new(slint::PhysicalSize::new(1280, 800)),
            }
        }))
    }
}

/// Call before constructing the UI, in a dedicated validation process.
pub fn install() -> Result<(), PlatformError> {
    slint::platform::set_platform(Box::new(OffscreenPlatform)).map_err(|e| e.to_string().into())
}

/// Slint's official snapshot path renders the actual component to a GPU texture and reads it back.
pub fn snapshot(window: &slint::Window) -> Result<headless::RgbaImage, PlatformError> {
    slint::platform::update_timers_and_animations();
    let pixels = window.take_snapshot()?;
    Ok(headless::RgbaImage {
        width: pixels.width(),
        height: pixels.height(),
        pixels: pixels.as_bytes().to_vec(),
    })
}
