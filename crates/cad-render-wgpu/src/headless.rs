//! Native, headless wgpu path: create a software device, render offscreen,
//! read the target back to a packed RGBA image and encode it as PNG.
//!
//! Spec §5.2/§6/§8/§11/§19; audit F11 (offline `render` must not fake success)
//! and F12 (device-loss classification). The contract is frozen in
//! `docs/headless-render.md`.
//!
//! This module is **native only** (the `mod` declaration in `lib.rs` is
//! `#[cfg(not(target_arch = "wasm32"))]`). In a normal host the UI framework
//! owns the `Device`/`Queue` and hands them to [`Renderer::initialize_with_device`];
//! the headless path exists for CI/agent environments with no window, no display
//! and no host device. It creates `Instance` → `Adapter` → `Device`/`Queue` and
//! then re-uses the exact same renderer. No second event loop, no present, no
//! per-frame readback (`docs/adr/0002-gpu-ui-composition.md`).
//!
//! wgpu objects never escape this crate: callers receive the renderer through
//! the existing public API, and CLI integration goes through
//! `cad-render-wgpu` (see `docs/headless-render.md` §1.3).

use crate::{BackendPreference, GpuSelection, RenderError, Renderer};
use cad_domain::{CadError, CadResult};
use std::collections::HashSet;

/// A read-only description of a wgpu adapter, with stable lowercase strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    /// Stable lowercase backend name: `"vulkan"`, `"gl"`, `"metal"`, `"dx12"`
    /// `"webgpu"` or `"noop"`.
    pub backend: String,
    /// Adapter/driver-reported name (for example `"llvmpipe (LLVM 21.1.8, 128 bits)"`).
    pub name: String,
    /// Stable lowercase device type: `"cpu"`, `"integrated_gpu"`,
    /// `"discrete_gpu"`, `"virtual_gpu"` or `"other"`.
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Free-form driver info string.
    pub driver_info: String,
}

/// An owned headless wgpu session: the device/queue the renderer draws with and
/// the adapter it came from.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: AdapterInfo,
}

/// The backends a preference is allowed to use.
///
/// Native `Auto`/`WebGpu` accept Vulkan and GL (Vulkan is preferred when both
/// are present); `WebGl2` is restricted to GL.
fn backends_for(preference: BackendPreference) -> wgpu::Backends {
    match preference {
        BackendPreference::WebGl2 => wgpu::Backends::GL,
        BackendPreference::Auto | BackendPreference::WebGpu => {
            wgpu::Backends::VULKAN | wgpu::Backends::GL
        }
    }
}

/// Backend priority used to pick one adapter deterministically.
fn backend_priority(preference: BackendPreference) -> &'static [wgpu::Backend] {
    match preference {
        BackendPreference::WebGl2 => &[wgpu::Backend::Gl],
        BackendPreference::Auto | BackendPreference::WebGpu => {
            &[wgpu::Backend::Vulkan, wgpu::Backend::Gl]
        }
    }
}

fn make_instance(preference: BackendPreference) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends_for(preference);
    wgpu::Instance::new(desc)
}

fn device_type_str(device_type: wgpu::DeviceType) -> &'static str {
    match device_type {
        wgpu::DeviceType::Cpu => "cpu",
        wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
        wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
        wgpu::DeviceType::VirtualGpu => "virtual_gpu",
        wgpu::DeviceType::Other => "other",
    }
}

/// Convert wgpu's adapter description into our stable, owned form.
///
/// The wgpu type is qualified because it collides with [`AdapterInfo`].
fn adapter_info(info: &wgpu::AdapterInfo) -> AdapterInfo {
    AdapterInfo {
        backend: info.backend.to_str().to_string(),
        name: info.name.clone(),
        device_type: device_type_str(info.device_type).to_string(),
        driver: info.driver.clone(),
        driver_info: info.driver_info.clone(),
    }
}

/// Enumerate adapters visible to wgpu (for diagnostics). Empty when no adapter
/// is available; that empty list is itself the honest answer.
pub fn enumerate_adapters(preference: BackendPreference) -> Vec<AdapterInfo> {
    let instance = make_instance(preference);
    futures_lite::future::block_on(instance.enumerate_adapters(backends_for(preference)))
        .iter()
        .map(|adapter| adapter_info(&adapter.get_info()))
        .collect()
}

/// Create a headless device with `Auto` adapter selection.
///
/// `Auto`/`WebGpu` prefer Vulkan, then GL; `WebGl2` restricts to GL. Returns
/// [`CadError::GpuFailure`] when no adapter matches or the device request fails;
/// it never returns a device-less success and never panics on this path.
pub fn create_headless_gpu(preference: BackendPreference) -> CadResult<HeadlessGpu> {
    create_headless_gpu_with(preference, GpuSelection::Auto)
}

/// Create a headless device, preferring a GPU kind through [`GpuSelection`]
/// within the active backend. `GpuSelection::Auto` keeps the historical
/// deterministic first-adapter order.
pub fn create_headless_gpu_with(
    preference: BackendPreference,
    gpu: GpuSelection,
) -> CadResult<HeadlessGpu> {
    let instance = make_instance(preference);
    let adapters =
        futures_lite::future::block_on(instance.enumerate_adapters(backends_for(preference)));
    if adapters.is_empty() {
        return Err(CadError::GpuFailure(format!(
            "no wgpu adapter available for preference {preference:?}"
        )));
    }

    let infos: Vec<AdapterInfo> = adapters
        .iter()
        .map(|a| adapter_info(&a.get_info()))
        .collect();
    let index =
        choose_adapter_index(&infos, backend_priority(preference), gpu).ok_or_else(|| {
            CadError::GpuFailure(format!(
                "no adapter matched preference {preference:?} (found {})",
                infos
                    .iter()
                    .map(|i| i.backend.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    let adapter = adapters[index].clone();

    let info = infos[index].clone();
    let (device, queue) =
        futures_lite::future::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("cad-headless-device"),
            ..Default::default()
        }))
        .map_err(|e| CadError::GpuFailure(format!("wgpu device request failed: {e}")))?;

    Ok(HeadlessGpu {
        device,
        queue,
        adapter: info,
    })
}

/// Deterministic adapter choice shared by the headless device path and its
/// unit tests.
///
/// Restricts to the highest-priority backend that has at least one adapter
/// (`backend_priority`), then applies the [`GpuSelection`] preference inside
/// that backend. `Auto` and `HighPerformance` both prefer a discrete GPU
/// (dual-GPU default); `LowPower` prefers an integrated one. A requested kind
/// that does not exist falls back to the first adapter of that backend, never
/// to a different backend.
pub(crate) fn choose_adapter_index(
    infos: &[AdapterInfo],
    backend_priority: &[wgpu::Backend],
    gpu: GpuSelection,
) -> Option<usize> {
    for backend in backend_priority {
        let backend_name = backend.to_str();
        let indices: Vec<usize> = infos
            .iter()
            .enumerate()
            .filter(|(_, info)| info.backend == backend_name)
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            continue;
        }
        let first = indices[0];
        return Some(match gpu {
            GpuSelection::Auto | GpuSelection::HighPerformance => indices
                .iter()
                .copied()
                .find(|&i| infos[i].device_type == "discrete_gpu")
                .unwrap_or(first),
            GpuSelection::LowPower => indices
                .iter()
                .copied()
                .find(|&i| infos[i].device_type == "integrated_gpu")
                .unwrap_or(first),
        });
    }
    None
}

/// A tightly packed RGBA8 image (`pixels.len() == width * height * 4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// The pixel at `(x, y)`, with `(0, 0)` at the top-left of the image.
    ///
    /// Out-of-bounds coordinates return transparent black rather than panicking.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// Number of pixels whose RGBA differs from `background` by more than
    /// `tolerance` on any channel.
    pub fn count_differing_from(&self, background: [u8; 4], tolerance: u8) -> usize {
        self.pixels
            .chunks_exact(4)
            .filter(|pixel| {
                (0..4).any(|channel| pixel[channel].abs_diff(background[channel]) > tolerance)
            })
            .count()
    }

    /// Number of distinct RGBA colors in the image.
    pub fn distinct_colors(&self) -> usize {
        self.pixels
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            .collect::<HashSet<_>>()
            .len()
    }
}

/// Encode a tightly packed RGBA8 image as lossless PNG bytes.
///
/// The bytes are already the sRGB-encoded target values; this performs no
/// further color conversion, so readback → PNG → decode round-trips exactly.
/// `Err` is a human-readable message and never a panic.
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let expected = image.width as usize * image.height as usize * 4;
    if image.pixels.len() != expected {
        return Err(format!(
            "RGBA buffer has {} bytes but {}x{} needs {}",
            image.pixels.len(),
            image.width,
            image.height,
            expected
        ));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG header encode failed: {e}"))?;
        writer
            .write_image_data(&image.pixels)
            .map_err(|e| format!("PNG pixel encode failed: {e}"))?;
    }
    Ok(out)
}

/// Row stride for `copy_texture_to_buffer`: `width * 4` rounded up to wgpu's
/// 256-byte `COPY_BYTES_PER_ROW_ALIGNMENT`.
fn aligned_bytes_per_row(width: u32) -> u32 {
    let row = width * 4;
    row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

impl Renderer {
    /// Copy the current offscreen target (`frame_texture`) to a buffer and
    /// return a tightly packed RGBA8 image.
    ///
    /// Returns [`RenderError::NotInitialized`] when no frame has been rendered
    /// yet (the target texture only exists after the first `render`/`render_3d`).
    /// This is the one GPU→CPU readback permitted on the headless evidence path;
    /// it is never part of production composition.
    pub fn read_target_rgba(&self) -> Result<RgbaImage, RenderError> {
        let Some(device) = self.device.as_ref() else {
            return Err(RenderError::NotInitialized(
                "no GPU device initialized".into(),
            ));
        };
        let Some(queue) = self.queue.as_ref() else {
            return Err(RenderError::NotInitialized(
                "no GPU queue initialized".into(),
            ));
        };
        let Some(texture) = self.target.as_ref() else {
            return Err(RenderError::NotInitialized(
                "no rendered frame to read back".into(),
            ));
        };

        let (width, height) = self.target_size;
        let bytes_per_row = aligned_bytes_per_row(width);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-readback-encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_result| {});
        // Block until the copy has completed and the mapping callback has run.
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| RenderError::Frame(format!("readback poll failed: {e}")))?;

        let pixels = {
            let view = slice
                .get_mapped_range()
                .map_err(|e| RenderError::Frame(format!("readback map failed: {e}")))?;
            let row_bytes = width as usize * 4;
            let mut pixels = vec![0u8; row_bytes * height as usize];
            for row in 0..height as usize {
                let src = row * bytes_per_row as usize;
                pixels[row * row_bytes..(row + 1) * row_bytes]
                    .copy_from_slice(&view[src..src + row_bytes]);
            }
            pixels
        };
        // Never leave the buffer mapped.
        buffer.unmap();

        Ok(RgbaImage {
            width,
            height,
            pixels,
        })
    }
}

#[cfg(test)]
mod adapter_selection_tests {
    use super::*;

    fn info(backend: &str, device_type: &str) -> AdapterInfo {
        AdapterInfo {
            backend: backend.into(),
            name: format!("{backend}-{device_type}"),
            device_type: device_type.into(),
            driver: "test".into(),
            driver_info: String::new(),
        }
    }

    const VULKAN: wgpu::Backend = wgpu::Backend::Vulkan;
    const GL: wgpu::Backend = wgpu::Backend::Gl;

    #[test]
    fn auto_prefers_discrete_on_hybrid_machines() {
        let list = vec![
            info("vulkan", "integrated_gpu"),
            info("vulkan", "discrete_gpu"),
        ];
        assert_eq!(
            choose_adapter_index(&list, &[VULKAN, GL], GpuSelection::Auto),
            Some(1),
            "dual-GPU default must prefer the discrete GPU"
        );
        // `high` is the explicit spelling of the same discrete-first default.
        assert_eq!(
            choose_adapter_index(&list, &[VULKAN, GL], GpuSelection::HighPerformance),
            Some(1)
        );
    }

    #[test]
    fn high_prefers_discrete_and_low_prefers_integrated() {
        let list = vec![
            info("vulkan", "integrated_gpu"),
            info("vulkan", "discrete_gpu"),
            info("vulkan", "cpu"),
        ];
        for selection in [GpuSelection::Auto, GpuSelection::HighPerformance] {
            assert_eq!(
                choose_adapter_index(&list, &[VULKAN], selection),
                Some(1),
                "{selection:?} must prefer the discrete GPU"
            );
        }
        assert_eq!(
            choose_adapter_index(&list, &[VULKAN], GpuSelection::LowPower),
            Some(0)
        );
    }

    #[test]
    fn missing_preferred_kind_falls_back_within_backend() {
        let list = vec![info("vulkan", "integrated_gpu"), info("vulkan", "cpu")];
        for selection in [GpuSelection::Auto, GpuSelection::HighPerformance] {
            assert_eq!(
                choose_adapter_index(&list, &[VULKAN], selection),
                Some(0),
                "no discrete adapter: fall back to the first Vulkan adapter"
            );
        }
        assert_eq!(
            choose_adapter_index(&list, &[VULKAN], GpuSelection::LowPower),
            Some(0)
        );
    }

    #[test]
    fn preference_never_promotes_a_lower_priority_backend() {
        // Only GL is present; the Vulkan slot is empty, so GL is chosen whole.
        let list = vec![info("gl", "cpu"), info("gl", "discrete_gpu")];
        for selection in [GpuSelection::Auto, GpuSelection::HighPerformance] {
            assert_eq!(
                choose_adapter_index(&list, &[VULKAN, GL], selection),
                Some(1),
                "{selection:?} still prefers the discrete adapter inside the available backend"
            );
        }
        assert_eq!(
            choose_adapter_index(&list, &[VULKAN, GL], GpuSelection::LowPower),
            Some(0),
            "no integrated adapter inside the available backend: fall back to its first adapter"
        );
    }

    #[test]
    fn empty_list_is_none() {
        assert_eq!(
            choose_adapter_index(&[], &[VULKAN, GL], GpuSelection::Auto),
            None
        );
    }

    #[test]
    fn parse_and_as_str_round_trip() {
        for value in ["auto", "high", "low"] {
            let parsed = GpuSelection::parse(value).unwrap();
            assert_eq!(parsed.as_str(), value);
        }
        assert_eq!(GpuSelection::parse("discrete"), None);
        assert_eq!(GpuSelection::default(), GpuSelection::Auto);
    }
}
