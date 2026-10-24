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
    // Let the backend police its own memory budget: 80 % of it makes resource
    // creation start returning OOM (which this crate classifies explicitly) and
    // 95 % loses the device to memory pressure instead of the process dying. See
    // `gpu_memory::memory_budget_thresholds` for the wgpu doc this comes from.
    crate::gpu_memory::apply_memory_budget_thresholds(&mut desc);
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

/// The limits a device was asked for, the limits it granted, and whether the
/// negotiated request had to be abandoned for the WebGPU defaults.
///
/// This is the *reporting* half of device-limit negotiation
/// ([`negotiated_device_limits`]); it is plain data, so a host can log or
/// inspect it without touching `wgpu` handles. `requested` is the negotiated
/// policy: on the degraded path it is the request that **failed**, which is why
/// the report keeps both `requested` and `granted` instead of one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLimitsReport {
    /// The adapter's own `max_buffer_size` (single-allocation ceiling).
    pub adapter_max_buffer_size: u64,
    /// The adapter's own `max_storage_buffer_binding_size`.
    pub adapter_max_storage_buffer_binding_size: u64,
    /// Limits requested in the first `request_device` attempt.
    pub requested: wgpu::Limits,
    /// Limits the live device reports after creation.
    pub granted: wgpu::Limits,
    /// `None` when the negotiated request succeeded. `Some(wgpu error text)`
    /// when it failed and the device was created with [`wgpu::Limits::default`]
    /// instead — an explicit degraded state, never a silent one.
    pub degraded_reason: Option<String>,
}

impl DeviceLimitsReport {
    /// `max_buffer_size` the negotiation asked for.
    ///
    /// On the degraded path this is the value that was rejected, i.e. the
    /// capacity the negotiation *tried* to obtain; `granted` is what is really
    /// usable.
    pub fn negotiated_max_buffer_size(&self) -> u64 {
        self.requested.max_buffer_size
    }

    /// `max_buffer_size` the live device actually allows.
    pub fn granted_max_buffer_size(&self) -> u64 {
        self.granted.max_buffer_size
    }

    /// Whether the device fell back to the WebGPU defaults.
    pub fn is_degraded(&self) -> bool {
        self.degraded_reason.is_some()
    }

    /// Single-line, whitespace-free token for probe logs and status lines.
    ///
    /// Shape:
    /// `adapter_max_buffer_size=… requested_max_buffer_size=…
    /// granted_max_buffer_size=… requested_max_storage_buffer_binding_size=…
    /// granted_max_storage_buffer_binding_size=… degraded=… reason=…`
    /// (`reason` is `-` when not degraded; spaces in wgpu error text become
    /// underscores so the line stays one token per field).
    pub fn summary(&self) -> String {
        let reason = match &self.degraded_reason {
            None => "-".to_string(),
            Some(reason) => reason.split_whitespace().collect::<Vec<_>>().join("_"),
        };
        format!(
            "adapter_max_buffer_size={} requested_max_buffer_size={} \
             granted_max_buffer_size={} \
             requested_max_storage_buffer_binding_size={} \
             granted_max_storage_buffer_binding_size={} degraded={} reason={}",
            self.adapter_max_buffer_size,
            self.requested.max_buffer_size,
            self.granted.max_buffer_size,
            self.requested.max_storage_buffer_binding_size,
            self.granted.max_storage_buffer_binding_size,
            self.is_degraded(),
            reason,
        )
    }
}

/// A device created by [`request_device_with_negotiated_limits`], with the
/// negotiation report attached.
pub struct NegotiatedDevice {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub limits: DeviceLimitsReport,
    /// Measurement-only feature negotiation (ADR 0013 class 1). `Disabled` on the
    /// product configuration; see [`crate::gpu_time`].
    pub gpu_time: crate::gpu_time::GpuTimeReport,
}

/// The single policy for the limits this crate asks a device for.
///
/// **Why this exists.** `wgpu::Limits::default()` is the WebGPU *portability*
/// floor, not the hardware's capability: its `max_buffer_size` is 256 MiB even
/// when the adapter reports ~4 GiB (`VkPhysicalDeviceLimits::
/// maxMemoryAllocationSize`). The renderer derives its whole-scene pulled
/// budget from the **device's** `max_buffer_size` (see
/// `Renderer::pulled_budget_bytes`), so requesting the defaults silently capped
/// every real drawing's accelerated organisation at 256 MiB and skipped it —
/// a capacity limit created by the request, not by the GPU. Measured on the
/// acceptance machine (Vulkan, NVIDIA Quadro P620): adapter and negotiated
/// `max_buffer_size` are both 4 292 870 144 B, which is what lets the
/// complex-test pulled payload (2 281 858 648 B) fit the budget gate.
///
/// **Policy.** Start from [`wgpu::Limits::default`] clamped *down* to what the
/// adapter accepts ([`wgpu::Limits::or_worse_values_from`], for downlevel
/// adapters that are worse than the WebGPU floor), then raise exactly one
/// field: `max_buffer_size` = the adapter's reported value.
///
/// * `max_buffer_size` is the driver's **per-allocation ceiling**; it reserves
///   nothing and cannot make a smaller allocation fail, so no artificial cap is
///   applied. Raising it is what lets the capacity gate see real capacity. The
///   renderer's own budget policy (one `max_buffer_size` of additional memory
///   for the pulled organisation) and its pre-flight gate are unchanged.
/// * `max_storage_buffer_binding_size` is deliberately **left at the WebGPU
///   default (128 MiB)** even though this machine's adapter advertises
///   2 147 483 644 B. This cap is evidence-based, not theoretical: the pulled
///   segment limit is `min(max_storage_buffer_binding_size, max_buffer_size)`,
///   so raising this field makes the segmenter emit multi-GiB **single**
///   allocations. A real-host run of `fixtures/complex-test.dwg` with the
///   raised binding limit created such a ~2 GiB segment and the allocation
///   failed during a pulled *rebuild* (the retained prefix's old segments are
///   still alive while the replacement set is allocated). wgpu 30.0.1 then
///   returns an invalid buffer handle for the failed `create_buffer`, and the
///   following `Queue::write_buffer` is reported as a **validation** error — a
///   kind an `ErrorFilter::OutOfMemory` scope does not capture — so it reached
///   wgpu's default handler and aborted the process
///   (`Buffer with 'cad-pulled-line-storage' label is invalid`). With the
///   binding limit at the default the same rebuild succeeds: the allocations
///   are 128 MiB-sized and the driver reclaims the old segments as they are
///   dropped. The cost is more pulled segments (draw calls) for the same
///   payload; **no pixel changes**. This bound is what makes negotiation
///   *safe* on this class of 4 GiB adapter; behaviour beyond this machine is
///   not claimed.
///
/// Every other limit stays at the WebGPU default: nothing in this crate reads
/// them beyond the portability floor, and requesting more would grant
/// capabilities no code uses (and could fail on adapters that do not offer
/// them). Capability negotiation (features, storage-buffer availability) is a
/// separate decision and deliberately untouched.
pub fn negotiated_device_limits(supported: &wgpu::Limits) -> wgpu::Limits {
    let mut limits = wgpu::Limits::default().or_worse_values_from(supported);
    limits.max_buffer_size = supported.max_buffer_size;
    limits
}

/// Try `request(negotiated)`; on failure retry `request(defaults)` and report
/// the first error as the degraded reason.
///
/// Pure decision logic: the caller supplies the actual device request, so the
/// fallback/verdict path is unit-testable without a GPU. Returning `Err` means
/// **both** requests failed; the message carries both errors and the negotiated
/// `max_buffer_size`, and the caller maps it to its own error type (never a
/// panic, never a device-less success).
fn request_with_default_fallback<D, E: std::fmt::Display>(
    negotiated: &wgpu::Limits,
    mut request: impl FnMut(&wgpu::Limits) -> Result<D, E>,
) -> Result<(D, Option<String>), String> {
    match request(negotiated) {
        Ok(value) => Ok((value, None)),
        Err(error) => {
            let reason = error.to_string();
            match request(&wgpu::Limits::default()) {
                Ok(value) => Ok((value, Some(reason))),
                Err(fallback_error) => Err(format!(
                    "device request failed with negotiated limits (max_buffer_size={}): {reason}; \
                     retry with wgpu defaults failed: {fallback_error}",
                    negotiated.max_buffer_size
                )),
            }
        }
    }
}

/// Create a device asking for [`negotiated_device_limits`] and the measurement
/// feature set implied by `gpu_time` (ADR 0013 class 1).
///
/// Infallible **in effect**: if the negotiated request is rejected, the device
/// is created with [`wgpu::Limits::default`] and [`DeviceLimitsReport`] carries
/// the negotiated `max_buffer_size` plus the rejection reason (degraded state).
/// Only a failure of *both* attempts is an `Err`; nothing here panics and a
/// degraded creation is never reported as if the negotiation had succeeded.
///
/// `gpu_time` is the explicit measurement opt-in. Disabled ⇒ no feature is
/// requested (the product default). Enabled ⇒ the timestamp feature set is
/// requested **only** when the adapter supports it, and the granted/requested
/// sets plus the resulting state are carried on [`NegotiatedDevice::gpu_time`].
pub fn request_device_with_negotiated_limits(
    adapter: &wgpu::Adapter,
    label: &str,
    gpu_time: crate::gpu_time::GpuTimeConfig,
) -> Result<NegotiatedDevice, String> {
    let supported = adapter.limits();
    let requested = negotiated_device_limits(&supported);
    let gpu_time_report = crate::gpu_time::negotiate_gpu_time(adapter.features(), gpu_time);
    let descriptor = |limits: &wgpu::Limits| wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: gpu_time_report.granted,
        required_limits: limits.clone(),
        ..Default::default()
    };
    let ((device, queue), degraded_reason) = request_with_default_fallback(&requested, |limits| {
        futures_lite::future::block_on(adapter.request_device(&descriptor(limits)))
    })?;
    let report = DeviceLimitsReport {
        adapter_max_buffer_size: supported.max_buffer_size,
        adapter_max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        requested,
        granted: device.limits(),
        degraded_reason,
    };
    Ok(NegotiatedDevice {
        device,
        queue,
        limits: report,
        gpu_time: gpu_time_report,
    })
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
///
/// The measurement-only GPU-time opt-in is **off** here (the CLI/headless caller
/// has no environment gate); see [`create_headless_gpu_with_gpu_time`].
pub fn create_headless_gpu_with(
    preference: BackendPreference,
    gpu: GpuSelection,
) -> CadResult<HeadlessGpu> {
    create_headless_gpu_with_gpu_time(preference, gpu, crate::gpu_time::GpuTimeConfig::disabled())
        .map(|(gpu, _report)| gpu)
}

/// [`create_headless_gpu_with`] with an explicit measurement-only GPU-time opt-in
/// (ADR 0013 class 1).
///
/// Returns the device **and** the measurement negotiation report (requested /
/// granted features and the explicit state), so a caller can tell whether the
/// feature was granted without a second probe. The product/CLI path uses
/// [`create_headless_gpu_with`], which defaults the flag off.
pub fn create_headless_gpu_with_gpu_time(
    preference: BackendPreference,
    gpu: GpuSelection,
    gpu_time: crate::gpu_time::GpuTimeConfig,
) -> CadResult<(HeadlessGpu, crate::gpu_time::GpuTimeReport)> {
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
    let negotiated =
        request_device_with_negotiated_limits(&adapter, "cad-headless-device", gpu_time)
            .map_err(|e| CadError::GpuFailure(format!("wgpu device request failed: {e}")))?;
    // The headless path has no probe stdout contract, and a CLI success must not
    // write to stderr, so the only headless signal is the **degraded** case: an
    // explicit warning rather than a silent fallback to the 256 MiB defaults.
    if let Some(reason) = &negotiated.limits.degraded_reason {
        eprintln!(
            "[probe] device-limits degraded: negotiated limits rejected, using wgpu defaults \
             ({}); reason={reason}",
            negotiated.limits.summary()
        );
    }

    Ok((
        HeadlessGpu {
            device: negotiated.device,
            queue: negotiated.queue,
            adapter: info,
        },
        negotiated.gpu_time,
    ))
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
mod device_limit_tests {
    //! Contracts for [`negotiated_device_limits`] and the degraded request path.
    //!
    //! The policy tests are pure. The request-fallback tests drive the real
    //! decision function with a stand-in request closure (no GPU): they assert
    //! the *verdict* and the reported reason, not wgpu's device creation. A real
    //! `request_device` rejection cannot be produced from the shipped policy
    //! because the policy only ever asks for values the adapter advertises —
    //! the last test covers the real device path on this machine instead.

    use super::*;

    /// Synthetic adapter limits: the WebGPU floor plus the 4 GiB-ish
    /// single-allocation ceiling the desktop adapter on this machine reports.
    fn synthetic_adapter() -> wgpu::Limits {
        wgpu::Limits {
            max_buffer_size: 4_292_870_144,
            max_storage_buffer_binding_size: 4_292_870_144,
            ..Default::default()
        }
    }

    #[test]
    fn policy_never_exceeds_supported_and_raises_max_buffer_size() {
        let supported = synthetic_adapter();
        let requested = negotiated_device_limits(&supported);

        assert!(
            requested.check_limits(&supported),
            "a request must never ask for more than the adapter supports"
        );
        assert!(
            requested.max_buffer_size > wgpu::Limits::default().max_buffer_size,
            "the negotiated max_buffer_size must exceed the 256 MiB WebGPU floor"
        );
        assert_eq!(requested.max_buffer_size, supported.max_buffer_size);
        assert_eq!(
            requested.max_storage_buffer_binding_size,
            wgpu::Limits::default().max_storage_buffer_binding_size,
            "the binding limit is deliberately capped at the WebGPU default even \
             though the adapter advertises more (the documented multi-GiB cap)"
        );
        assert!(
            supported.max_storage_buffer_binding_size
                > wgpu::Limits::default().max_storage_buffer_binding_size,
            "the synthetic adapter must advertise more than the cap for this to mean anything"
        );
        // Untouched portability fields stay at their defaults.
        assert_eq!(
            requested.max_texture_dimension_2d,
            wgpu::Limits::default().max_texture_dimension_2d
        );
    }

    #[test]
    fn policy_clamps_to_a_downlevel_adapter() {
        // An adapter worse than the WebGPU floor (downlevel GL-style limits):
        // the request must still be satisfiable.
        let supported = wgpu::Limits {
            max_buffer_size: 128 << 20,
            max_storage_buffer_binding_size: 64 << 20,
            max_texture_dimension_2d: 2048,
            ..Default::default()
        };
        let requested = negotiated_device_limits(&supported);

        assert!(requested.check_limits(&supported));
        assert_eq!(requested.max_buffer_size, 128 << 20);
        assert_eq!(requested.max_storage_buffer_binding_size, 64 << 20);
        assert_eq!(requested.max_texture_dimension_2d, 2048);
    }

    #[test]
    fn binding_limit_is_capped_at_the_webgpu_default() {
        let supported = wgpu::Limits {
            max_buffer_size: 4 << 30,
            // An adapter whose binding range is far wider than the cap.
            max_storage_buffer_binding_size: 4 << 30,
            ..Default::default()
        };
        let requested = negotiated_device_limits(&supported);
        assert!(requested.check_limits(&supported));
        assert_eq!(
            requested.max_storage_buffer_binding_size,
            wgpu::Limits::default().max_storage_buffer_binding_size,
            "the cap must bound every pulled segment allocation, whatever the adapter claims"
        );
        // A binding limit below the cap is never raised.
        let low = wgpu::Limits {
            max_buffer_size: 4 << 30,
            max_storage_buffer_binding_size: 32 << 20,
            ..Default::default()
        };
        assert_eq!(
            negotiated_device_limits(&low).max_storage_buffer_binding_size,
            32 << 20
        );
    }

    #[test]
    fn degraded_request_falls_back_to_defaults_with_the_reason() {
        let negotiated = negotiated_device_limits(&synthetic_adapter());
        let defaults = wgpu::Limits::default();
        let mut attempts: Vec<u64> = Vec::new();
        let outcome = request_with_default_fallback(&negotiated, |limits| {
            attempts.push(limits.max_buffer_size);
            if *limits == defaults {
                Ok("device-on-defaults")
            } else {
                Err("limits exceeded: max_buffer_size")
            }
        });

        let (device, degraded_reason) =
            outcome.expect("a defaults fallback must still create a device");
        assert_eq!(device, "device-on-defaults");
        assert_eq!(
            degraded_reason.as_deref(),
            Some("limits exceeded: max_buffer_size"),
            "the degraded state must carry the negotiation failure reason"
        );
        assert_eq!(
            attempts,
            vec![negotiated.max_buffer_size, defaults.max_buffer_size],
            "the fallback must retry exactly once, with the WebGPU defaults"
        );
        // The report the caller receives carries the rejected capacity.
        let report = DeviceLimitsReport {
            adapter_max_buffer_size: negotiated.max_buffer_size,
            adapter_max_storage_buffer_binding_size: negotiated.max_storage_buffer_binding_size,
            requested: negotiated.clone(),
            granted: defaults.clone(),
            degraded_reason,
        };
        assert!(report.is_degraded());
        assert_eq!(
            report.negotiated_max_buffer_size(),
            negotiated.max_buffer_size
        );
        assert_eq!(report.granted_max_buffer_size(), defaults.max_buffer_size);
        let summary = report.summary();
        assert!(summary.contains("degraded=true"));
        assert!(summary.contains(&format!(
            "requested_max_buffer_size={}",
            negotiated.max_buffer_size
        )));
        assert!(summary.contains(&format!(
            "granted_max_buffer_size={}",
            defaults.max_buffer_size
        )));
    }

    #[test]
    fn accepted_request_is_not_degraded() {
        let negotiated = negotiated_device_limits(&synthetic_adapter());
        let outcome = request_with_default_fallback(&negotiated, |_| Ok::<_, String>("device"));
        let (device, degraded_reason) = outcome.expect("an accepted request must succeed");
        assert_eq!(device, "device");
        assert_eq!(degraded_reason, None);
    }

    #[test]
    fn both_attempts_failing_is_an_explicit_error() {
        let negotiated = negotiated_device_limits(&synthetic_adapter());
        let mut attempts = 0;
        let outcome = request_with_default_fallback(&negotiated, |_| {
            attempts += 1;
            Err::<(), _>("adapter lost")
        });
        let message = outcome.expect_err("two failures must be an explicit error, not a panic");
        assert_eq!(attempts, 2);
        assert!(message.contains(&format!("max_buffer_size={}", negotiated.max_buffer_size)));
        assert!(message.contains("adapter lost"));
        assert!(message.contains("retry with wgpu defaults failed"));
    }

    #[test]
    fn real_adapter_grants_the_negotiated_max_buffer_size() {
        // Real-device contract (skipped explicitly when the machine has no
        // adapter; a skip is not a pass). This proves the negotiated request is
        // actually accepted and granted, not merely well-formed.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let adapters = futures_lite::future::block_on(instance.enumerate_adapters(backends));
        let Some(adapter) = adapters.first() else {
            eprintln!("[device-limit-test] SKIP: no wgpu adapter; NOT a pass");
            return;
        };
        let created = match request_device_with_negotiated_limits(
            adapter,
            "cad-limit-test",
            crate::gpu_time::GpuTimeConfig::disabled(),
        ) {
            Ok(created) => created,
            Err(error) => panic!("negotiated device request failed: {error}"),
        };
        // Fence 1 (device half): the default (disabled) path requests no feature
        // and the granted set is empty.
        assert_eq!(
            created.gpu_time.requested,
            wgpu::Features::empty(),
            "the product default must request no measurement feature"
        );
        assert_eq!(created.gpu_time.granted, wgpu::Features::empty());
        assert_eq!(
            created.gpu_time.state,
            crate::gpu_time::GpuTimeState::Disabled
        );
        assert!(
            !created
                .device
                .features()
                .contains(crate::gpu_time::GPU_TIME_FEATURES),
            "a disabled measurement config must leave the device without the \
             timestamp feature set"
        );
        assert!(
            !created.limits.is_degraded(),
            "a policy request within adapter limits must not degrade: {:?}",
            created.limits.degraded_reason
        );
        assert_eq!(
            created.limits.granted_max_buffer_size(),
            adapter.limits().max_buffer_size,
            "the granted max_buffer_size must be the adapter's, not the 256 MiB floor"
        );
        assert_eq!(
            created.limits.granted.max_storage_buffer_binding_size,
            adapter
                .limits()
                .max_storage_buffer_binding_size
                .min(wgpu::Limits::default().max_storage_buffer_binding_size),
            "the granted binding limit must be the capped one, never a multi-GiB segment budget"
        );
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
