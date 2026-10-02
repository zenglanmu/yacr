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

use crate::{BackendPreference, RenderError, Renderer};
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

/// Create a headless device.
///
/// `Auto`/`WebGpu` prefer Vulkan, then GL; `WebGl2` restricts to GL. Returns
/// [`CadError::GpuFailure`] when no adapter matches or the device request fails;
/// it never returns a device-less success and never panics on this path.
pub fn create_headless_gpu(preference: BackendPreference) -> CadResult<HeadlessGpu> {
    let instance = make_instance(preference);
    let adapters =
        futures_lite::future::block_on(instance.enumerate_adapters(backends_for(preference)));
    if adapters.is_empty() {
        return Err(CadError::GpuFailure(format!(
            "no wgpu adapter available for preference {preference:?}"
        )));
    }

    // Pick the highest-priority backend that has an adapter. Adapter is cheap to
    // clone (it wraps an Arc), so this keeps the priority ordering explicit.
    let adapter = backend_priority(preference)
        .iter()
        .find_map(|backend| {
            adapters
                .iter()
                .find(|candidate| candidate.get_info().backend == *backend)
                .cloned()
        })
        .ok_or_else(|| {
            CadError::GpuFailure(format!(
                "no adapter matched preference {preference:?} (found {})",
                adapters
                    .iter()
                    .map(|a| a.get_info().backend.to_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    let info = adapter_info(&adapter.get_info());
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
