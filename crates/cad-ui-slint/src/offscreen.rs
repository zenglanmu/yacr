//! Native validation platform: the production Slint component rendered into a wgpu texture.
//! No window server, emulator, or alternate UI implementation.
use cad_render_wgpu::{headless, wgpu, BackendPreference, GpuSelection};
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

fn create_validation_gpu() -> Result<headless::HeadlessGpu, PlatformError> {
    let high_performance =
        headless::create_headless_gpu_with(BackendPreference::Auto, GpuSelection::HighPerformance)
            .map_err(|error| error.to_string())?;
    if matches!(
        high_performance.adapter.device_type.as_str(),
        "discrete_gpu" | "integrated_gpu"
    ) {
        return Ok(high_performance);
    }

    // Auto/high-performance may return its first adapter when no discrete GPU
    // exists. Probe the low-power preference before accepting a CPU fallback so
    // an integrated GPU is still preferred over software rendering.
    match headless::create_headless_gpu_with(BackendPreference::Auto, GpuSelection::LowPower) {
        Ok(low_power)
            if matches!(
                low_power.adapter.device_type.as_str(),
                "discrete_gpu" | "integrated_gpu"
            ) =>
        {
            Ok(low_power)
        }
        _ => Ok(high_performance),
    }
}

impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let gpu = create_validation_gpu()?;
        if gpu.adapter.backend != "vulkan" {
            return Err(format!(
                "offscreen validation requires Vulkan, got {:?}",
                gpu.adapter
            )
            .into());
        }
        let render_class = match gpu.adapter.device_type.as_str() {
            "discrete_gpu" | "integrated_gpu" => "hardware",
            "cpu" => "software fallback (no usable hardware adapter selected)",
            _ => "other adapter type",
        };
        eprintln!(
            "Slint offscreen adapter: class={render_class}, name={}, device_type={}, backend={}",
            gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.backend
        );
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
