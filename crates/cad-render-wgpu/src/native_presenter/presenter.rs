use super::*;

/// A native, Vulkan-only surface probe.
///
/// Construct one per window, call [`SurfaceProbePresenter::frame`] once per host
/// tick, and drop it to release the surface. Rebuilding is just constructing a
/// new instance (possibly with a new window handle).
pub struct SurfaceProbePresenter {
    /// Kept alive for the lifetime of the presenter so the backend instance
    /// owning the surface never drops early.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: ProbeAdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    max_dimension: u32,
    size: (u32, u32),
    preferred_names: Vec<String>,
    /// Requested present-mode policy (the concrete mode lives in `config`).
    present_mode_request: ProbePresentMode,
    /// Wall-clock microseconds of the most recent `Surface::configure`.
    last_configure_us: u64,
    frame_index: u64,
    stopped: bool,
    /// Lazily built fullscreen textured pipeline (surface format bound).
    texture_pipeline: Option<wgpu::RenderPipeline>,
    texture_layout: Option<wgpu::BindGroupLayout>,
    texture_sampler: Option<wgpu::Sampler>,
    /// Cached bind group for the last presented texture, keyed by the full key.
    texture_bind: Option<(NativeTextureKey, wgpu::Texture, wgpu::BindGroup)>,
    /// The limits this device was created with (see
    /// [`crate::headless::negotiated_device_limits`]), including the degraded
    /// fallback verdict.
    device_limits: crate::headless::DeviceLimitsReport,
    /// Measurement-only feature negotiation (ADR 0013 class 1): what the opt-in
    /// requested, what the device granted, and the explicit state.
    gpu_time: crate::gpu_time::GpuTimeReport,
    /// Consecutive native-texture/surface size disagreements. A transient (a
    /// mid-resize tick or a one-pixel DOM rounding) waits; a persistent one is
    /// the explicit size-guard error.
    size_mismatch: SizeMismatchWait,
}

impl SurfaceProbePresenter {
    /// Create and configure a Vulkan-only surface for `window`.
    ///
    /// `window` is an owning `raw-window-handle` provider (for example a Tauri
    /// window or an X11 child handle); the resulting `Surface<'static>` keeps it
    /// alive, so it must outlive nothing beyond the returned presenter.
    ///
    /// Uses [`DEFAULT_PREFERRED_ADAPTER_NAMES`] as the adapter preference. See
    /// [`SurfaceProbePresenter::new_with_preferred_names`] to override it.
    pub fn new<W>(window: W, width: u32, height: u32) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(
            window,
            width,
            height,
            preferred_names,
            ProbePresentMode::Fifo,
            crate::gpu_time::GpuTimeConfig::disabled(),
        )
    }

    /// Create a probe with an explicit present-mode policy.
    ///
    /// [`ProbePresentMode::MailboxElseImmediate`] is the single allowed
    /// present-mode comparison; it never silently falls back.
    pub fn new_with_present_mode<W>(
        window: W,
        width: u32,
        height: u32,
        present_mode: ProbePresentMode,
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(
            window,
            width,
            height,
            preferred_names,
            present_mode,
            crate::gpu_time::GpuTimeConfig::disabled(),
        )
    }

    /// Create a probe with an explicit adapter-name preference list.
    ///
    /// Empty entries are ignored by the matcher. The list is only a preference;
    /// a machine with none of the names still gets a Vulkan hardware adapter.
    pub fn new_with_preferred_names<W>(
        window: W,
        width: u32,
        height: u32,
        preferred_names: &[&str],
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = preferred_names
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(
            window,
            width,
            height,
            preferred_names,
            ProbePresentMode::Fifo,
            crate::gpu_time::GpuTimeConfig::disabled(),
        )
    }

    pub(super) fn build<W>(
        window: W,
        width: u32,
        height: u32,
        preferred_names: Vec<String>,
        present_mode_request: ProbePresentMode,
        gpu_time: crate::gpu_time::GpuTimeConfig,
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let instance = make_probe_instance();
        // `window` is owned for `'static`, so the surface is `Surface<'static>`
        // and keeps the window handle alive.
        let surface = instance
            .create_surface(window)
            .map_err(|error| ProbeError::SurfaceCreation(error.to_string()))?;
        Self::build_with_surface(
            instance,
            surface,
            width,
            height,
            preferred_names,
            present_mode_request,
            gpu_time,
        )
    }

    /// Create a probe surface from **raw** display/window handles.
    ///
    /// This is the entry point for hosts that own a non-`Send` native widget
    /// (for example a GTK child) and therefore cannot use the safe owning
    /// constructors: the surface does **not** own the window, and its lifetime
    /// is entirely the caller's responsibility. `wgpu`'s unsafe surface creation
    /// stays inside this crate; no `wgpu` object escapes.
    ///
    /// # Safety
    ///
    /// The caller must guarantee, for the entire lifetime of the returned
    /// presenter (until it is **fully dropped**):
    ///
    /// - `display` and `window` are a **matched, live** pair identifying the
    ///   same drawable. Two backends satisfy this: an **Xlib/Xcb** pair for the
    ///   same X server/window, **or** a **Wayland** pair of the `wl_display` and
    ///   the **app-owned** `wl_surface` presented through that display. The raw
    ///   handles are not validated here, so a mismatched or stale pair is
    ///   undefined behaviour;
    /// - the native display and window stay alive and keep their identity until
    ///   the presenter is dropped: they are never destroyed, unrealized,
    ///   re-created or given a new XID, and no surface role/lifetime change
    ///   (e.g. a Wayland surface re-role) happens while it exists;
    /// - this presenter is the **single presentation owner** of that drawable:
    ///   no other Vulkan presenter/surface writes to the same window, and for a
    ///   Wayland `wl_surface` the surface is app-owned — it is **never** a
    ///   GTK/GDK-owned surface that GDK itself attaches/commits to (no double
    ///   writer against GDK's own composition);
    /// - construction, every `frame`/`resize`, and the final drop all happen on
    ///   the host's **main thread** (the same thread that owns the native
    ///   window), matching the safe path's semantics.
    ///
    /// The returned `Surface<'static>` only *claims* `'static`; it does not keep
    /// the handles alive. Violating the above is undefined behaviour. On any
    /// error the GPU objects created so far are dropped before returning.
    pub unsafe fn new_from_raw_handles(
        display: wgpu::rwh::RawDisplayHandle,
        window: wgpu::rwh::RawWindowHandle,
        width: u32,
        height: u32,
        present_mode: ProbePresentMode,
        gpu_time: crate::gpu_time::GpuTimeConfig,
    ) -> Result<Self, ProbeError> {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        let instance = make_probe_instance();
        // SAFETY: forwarded from the caller's contract documented above.
        let surface: wgpu::Surface<'static> = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window,
            })
        }
        .map_err(|error| ProbeError::SurfaceCreation(error.to_string()))?;
        Self::build_with_surface(
            instance,
            surface,
            width,
            height,
            preferred_names,
            present_mode,
            gpu_time,
        )
    }

    /// Shared factory used by the safe and unsafe constructors.
    ///
    /// Everything after the surface exists lives here, so both paths reuse the
    /// exact same adapter selection, device request, pipeline build, present-mode
    /// choice, configure and timing code. On any error path the surface, instance
    /// and (if created) device are dropped before returning, so a failed call
    /// leaves no GPU resource behind.
    pub(super) fn build_with_surface(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        preferred_names: Vec<String>,
        present_mode_request: ProbePresentMode,
        gpu_time_config: crate::gpu_time::GpuTimeConfig,
    ) -> Result<Self, ProbeError> {
        let adapters = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
        if adapters.is_empty() {
            return Err(ProbeError::NoVulkanAdapter(
                "no Vulkan adapter was enumerated".into(),
            ));
        }
        let infos: Vec<ProbeAdapterInfo> = adapters
            .iter()
            .map(|adapter| probe_info(adapter, &surface))
            .collect();
        let ranked = rank_probe_adapters(&infos, &preferred_names);
        let index = ranked.first().copied().ok_or_else(|| {
            ProbeError::NoVulkanAdapter(format!(
                "no Vulkan hardware adapter can present to the surface (enumerated: {})",
                infos
                    .iter()
                    .map(|info| format!("{}/{}", info.backend, info.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let adapter = adapters[index].clone();
        let adapter_info = infos[index].clone();

        let capabilities = surface.get_capabilities(&adapter);
        let format =
            choose_surface_format(&capabilities.formats).ok_or(ProbeError::NoSupportedFormat)?;
        let max_dimension = adapter.limits().max_texture_dimension_2d;
        validate_size(width, height, max_dimension)?;

        let negotiated = crate::headless::request_device_with_negotiated_limits(
            &adapter,
            "cad-surface-probe-device",
            gpu_time_config,
        )
        .map_err(ProbeError::DeviceRequest)?;
        let device = negotiated.device;
        let queue = negotiated.queue;
        // One-off startup diagnostic (never per frame): the negotiated capacity
        // is what explains the pulled budget in `[probe] actual-render`'s
        // `pulled=…` token, which reports the same `max_buffer_size` as
        // `budget_bytes`. A degraded line means the device fell back to the
        // WebGPU defaults and the pulled budget is 256 MiB again.
        eprintln!("[probe] device-limits {}", negotiated.limits.summary());
        // Measurement-only (ADR 0013 class 1): the explicit opt-in, what the
        // device granted, and the resulting state. Appended as an explicit token
        // so a missing feature is named rather than silently ignored.
        eprintln!(
            "[probe] gpu-time requested={:?} granted={:?} state={}",
            negotiated.gpu_time.requested,
            negotiated.gpu_time.granted,
            negotiated.gpu_time.state.as_token(),
        );

        let (pipeline, bind_group, uniform) = build_probe_pipeline(&device, format);

        let supported_present_modes = capabilities.present_modes.clone();
        let chosen_present_mode = match present_mode_request {
            ProbePresentMode::Fifo => wgpu::PresentMode::Fifo,
            ProbePresentMode::MailboxElseImmediate => {
                if supported_present_modes.contains(&wgpu::PresentMode::Mailbox) {
                    wgpu::PresentMode::Mailbox
                } else {
                    wgpu::PresentMode::Immediate
                }
            }
        };
        eprintln!(
            "[probe] present mode requested={present_mode_request:?} supported={supported_present_modes:?} chosen={chosen_present_mode:?}"
        );

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: chosen_present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        let configure_started = std::time::Instant::now();
        surface.configure(&device, &config);
        let last_configure_us = configure_started.elapsed().as_micros() as u64;
        eprintln!("[probe] initial configure took {last_configure_us} us");

        Ok(Self {
            _instance: instance,
            surface,
            adapter: adapter_info,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            uniform,
            max_dimension,
            size: (width, height),
            preferred_names,
            present_mode_request,
            last_configure_us,
            frame_index: 0,
            stopped: false,
            texture_pipeline: None,
            texture_layout: None,
            texture_sampler: None,
            texture_bind: None,
            device_limits: negotiated.limits,
            gpu_time: negotiated.gpu_time,
            size_mismatch: SizeMismatchWait::default(),
        })
    }

    /// Reconfigure the surface for a new physical size.
    ///
    /// Both dimensions must be non-zero and within the adapter's texture limit;
    /// otherwise an explicit [`ProbeError`] is returned instead of a configure
    /// panic. A no-op resize leaves the surface untouched.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), ProbeError> {
        validate_size(width, height, self.max_dimension)?;
        if (width, height) == self.size {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        let started = std::time::Instant::now();
        self.surface.configure(&self.device, &self.config);
        self.last_configure_us = started.elapsed().as_micros() as u64;
        eprintln!(
            "[probe] resize configure {width}x{height} took {} us",
            self.last_configure_us
        );
        self.size = (width, height);
        Ok(())
    }

    /// Wall-clock microseconds of the most recent `Surface::configure`.
    pub fn last_configure_us(&self) -> u64 {
        self.last_configure_us
    }

    /// Render and present one probe frame with the given marker offset.
    ///
    /// `marker_offset` must be finite; it is wrapped into `[0, 1)` and used to
    /// translate the colour block, so frames are visually distinguishable without
    /// any readback. A recoverable `Outdated` acquire is retried once after a
    /// reconfigure; every other acquire failure is returned explicitly. Drawing
    /// after [`SurfaceProbePresenter::stop`] returns [`ProbeError::Stopped`].
    pub fn frame(&mut self, marker_offset: f32) -> Result<ProbeFrameStats, ProbeError> {
        let frame_started = std::time::Instant::now();
        if self.stopped {
            return Err(ProbeError::Stopped);
        }
        let marker = sanitize_marker(marker_offset)?;
        let (width, height) = self.size;
        let mut timings = ProbeStageTimings::default();

        // Params uniform: resolution (vec2, 8 bytes) + marker (f32) + pad (f32).
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&(width as f32).to_le_bytes());
        params[4..8].copy_from_slice(&(height as f32).to_le_bytes());
        params[8..12].copy_from_slice(&marker.to_le_bytes());
        let stage = std::time::Instant::now();
        self.queue.write_buffer(&self.uniform, 0, &params);
        timings.write_buffer_us = stage.elapsed().as_micros() as u64;

        let acquire_started = std::time::Instant::now();
        let mut configure_us = 0u64;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // `Outdated` is recoverable: the documented remedy is to reconfigure
            // and acquire again. Bound it to a single retry so a persistent
            // mismatch still fails explicitly rather than looping.
            wgpu::CurrentSurfaceTexture::Outdated => {
                let stage = std::time::Instant::now();
                self.surface.configure(&self.device, &self.config);
                configure_us += stage.elapsed().as_micros() as u64;
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        return Err(ProbeError::Acquire("outdated"))
                    }
                    wgpu::CurrentSurfaceTexture::Timeout => {
                        return Err(ProbeError::Acquire("timeout"))
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => {
                        return Err(ProbeError::Acquire("occluded"))
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        return Err(ProbeError::DeviceLost(
                            "surface lost; rebuild the presenter".into(),
                        ))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(ProbeError::Acquire("validation"))
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => return Err(ProbeError::Acquire("timeout")),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(ProbeError::Acquire("occluded")),
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(ProbeError::DeviceLost(
                    "surface lost; rebuild the presenter".into(),
                ))
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ProbeError::Acquire("validation"))
            }
        };
        timings.acquire_us = acquire_started.elapsed().as_micros() as u64;
        timings.configure_us = configure_us;

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cad-surface-probe-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-surface-probe-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.03,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let stage = std::time::Instant::now();
        self.queue.submit(Some(encoder.finish()));
        timings.submit_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        if let Some(error) = collect_error_scope_bounded(&self.device, scope, SCOPE_COLLECT_BUDGET)
        {
            // `frame` is dropped without present, which discards the swapchain
            // image; the error is surfaced rather than hidden.
            return Err(ProbeError::Frame(format!(
                "probe frame rejected by GPU validation: {error}"
            )));
        }
        timings.scope_pop_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        self.queue.present(frame);
        timings.present_us = stage.elapsed().as_micros() as u64;

        self.frame_index += 1;
        timings.total_us = frame_started.elapsed().as_micros() as u64;
        Ok(ProbeFrameStats {
            frame_index: self.frame_index,
            marker_offset: marker,
            surface_size: (width, height),
            timings,
        })
    }

    /// Clone the device/queue this presenter uses.
    ///
    /// A host uses this to attach its own session to the **same** device (the
    /// app gets `wgpu` types through this crate's re-exports, so it needs no
    /// direct `wgpu` dependency).
    pub fn device_queue(&self) -> (wgpu::Device, wgpu::Queue) {
        (self.device.clone(), self.queue.clone())
    }

    /// Present a native CAD texture directly on the surface (no CPU readback, no
    /// extra CAD surface, no second event loop).
    ///
    /// `texture` must live on the **same device** as this presenter; the app
    /// guarantees that by attaching its session through [`Self::device_queue`].
    /// The bind group is cached by `key`; a changed `device_epoch`,
    /// `texture_revision` or `size` rebinds. Validation (format/usage/size) and
    /// any GPU validation error are explicit [`ProbeError`]s.
    pub fn present_texture(
        &mut self,
        texture: &wgpu::Texture,
        key: NativeTextureKey,
    ) -> Result<ProbeFrameStats, ProbeError> {
        let frame_started = std::time::Instant::now();
        if self.stopped {
            return Err(ProbeError::Stopped);
        }
        validate_texture_key(&key, self.max_dimension)?;
        validate_texture(texture, &key)?;
        // The blit draws this texture across the WHOLE surface with a
        // fullscreen triangle and no aspect correction (see `TEXTURE_SHADER`),
        // so a texture whose size disagrees with the configured surface size
        // would be stretched anisotropically (a defect observed as ~1.24x
        // vertical stretch when the native drawing area did not match the React
        // DOM CAD rect). Reject it explicitly instead of presenting a distorted
        // frame.
        //
        // A disagreement is not necessarily a defect: the texture size is
        // DOM-derived and rounded while the surface size is allocation-derived
        // and integral, so a mid-resize tick (or a fractional DOM rect at
        // `scale >= 2`) can disagree for a frame or two. The wait tolerates the
        // first `SIZE_MISMATCH_WAIT_TICKS - 1` consecutive disagreements, reported
        // as `TextureNotReady` (the caller waits, exactly like the "no frame"
        // path); the disagreement that reaches the limit is the explicit error
        // below. The texture is never presented either way.
        //
        // Do NOT "fix" this by rendering the texture at the surface size: that
        // keeps the image square but misaligns every pointer event, which maps
        // through the DOM CAD rect, not the native surface. The correct remedy
        // is to make the native drawing area match the DOM rect (the GTK bridge
        // layout does that); this guard only turns a residual disagreement into
        // an explicit error rather than a silent stretch.
        if key.size != self.size {
            let message = format!(
                "native texture {}x{} does not match the {}x{} surface; refusing to stretch it \
                 (the native drawing area must match the DOM CAD rect)",
                key.size.0, key.size.1, self.size.0, self.size.1
            );
            return Err(if self.size_mismatch.record_mismatch() {
                ProbeError::TextureNotReady(message)
            } else {
                ProbeError::TextureInvalid(message)
            });
        }
        self.size_mismatch.record_match();
        let mut timings = ProbeStageTimings::default();

        let stage = std::time::Instant::now();
        self.ensure_texture_bind_group(texture, &key);
        timings.write_buffer_us = stage.elapsed().as_micros() as u64;

        let (frame, configure_us) = self.acquire_frame()?;
        timings.acquire_us = frame_started.elapsed().as_micros() as u64;
        timings.configure_us = configure_us;

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cad-texture-encoder"),
            });
        {
            let pipeline = self
                .texture_pipeline
                .as_ref()
                .ok_or_else(|| ProbeError::Frame("texture pipeline missing".into()))?;
            let bind = self
                .texture_bind
                .as_ref()
                .map(|(_, _, bind)| bind)
                .ok_or_else(|| ProbeError::Frame("texture bind group missing".into()))?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-texture-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let stage = std::time::Instant::now();
        self.queue.submit(Some(encoder.finish()));
        timings.submit_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        if let Some(error) = collect_error_scope_bounded(&self.device, scope, SCOPE_COLLECT_BUDGET)
        {
            return Err(ProbeError::Frame(format!(
                "native texture frame rejected by GPU validation: {error}"
            )));
        }
        timings.scope_pop_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        self.queue.present(frame);
        timings.present_us = stage.elapsed().as_micros() as u64;

        self.frame_index += 1;
        timings.total_us = frame_started.elapsed().as_micros() as u64;
        Ok(ProbeFrameStats {
            frame_index: self.frame_index,
            marker_offset: 0.0,
            surface_size: self.size,
            timings,
        })
    }

    /// Acquire a surface frame, retrying once after a reconfigure on `Outdated`.
    /// Returns the frame and the microseconds spent reconfiguring.
    pub(super) fn acquire_frame(&self) -> Result<(wgpu::SurfaceTexture, u64), ProbeError> {
        let mut configure_us = 0u64;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated => {
                let stage = std::time::Instant::now();
                self.surface.configure(&self.device, &self.config);
                configure_us += stage.elapsed().as_micros() as u64;
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        return Err(ProbeError::Acquire("outdated"))
                    }
                    wgpu::CurrentSurfaceTexture::Timeout => {
                        return Err(ProbeError::Acquire("timeout"))
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => {
                        return Err(ProbeError::Acquire("occluded"))
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        return Err(ProbeError::DeviceLost(
                            "surface lost; rebuild the presenter".into(),
                        ))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(ProbeError::Acquire("validation"))
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => return Err(ProbeError::Acquire("timeout")),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(ProbeError::Acquire("occluded")),
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(ProbeError::DeviceLost(
                    "surface lost; rebuild the presenter".into(),
                ))
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ProbeError::Acquire("validation"))
            }
        };
        Ok((frame, configure_us))
    }

    /// Build the textured pipeline on first use and (re)bind the CAD texture.
    /// The bind group is cached by the full [`NativeTextureKey`].
    pub(super) fn ensure_texture_bind_group(
        &mut self,
        texture: &wgpu::Texture,
        key: &NativeTextureKey,
    ) {
        // Reuse only when BOTH the key and the texture identity match: the key
        // alone is not unique across engine sessions, and `wgpu::Texture`
        // equality is a stable identity comparison.
        let reusable = self
            .texture_bind
            .as_ref()
            .is_some_and(|(cached_key, cached_texture, _)| {
                binding_cache_is_reusable(cached_key, key, cached_texture == texture)
            });
        if reusable {
            return;
        }
        if self.texture_pipeline.is_none() {
            let (pipeline, layout, sampler) =
                build_texture_pipeline(&self.device, self.config.format);
            self.texture_pipeline = Some(pipeline);
            self.texture_layout = Some(layout);
            self.texture_sampler = Some(sampler);
        }
        let layout = self
            .texture_layout
            .as_ref()
            .expect("texture layout exists once the pipeline is built");
        let sampler = self
            .texture_sampler
            .as_ref()
            .expect("texture sampler exists once the pipeline is built");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-texture-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        self.texture_bind = Some((*key, texture.clone(), bind));
    }

    /// Drop the cached native-texture bind group.
    ///
    /// A host calls this when it replaces the render session or after a failure,
    /// so a later frame can never reuse a binding that belongs to a dead device
    /// or a different document.
    pub fn clear_binding_cache(&mut self) {
        self.texture_bind = None;
    }

    /// Stop drawing. Subsequent [`SurfaceProbePresenter::frame`] calls return
    /// [`ProbeError::Stopped`]. Rebuilding requires a new presenter.
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// Whether [`SurfaceProbePresenter::stop`] has been called.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The adapter the probe selected (backend, name, device type, driver).
    pub fn adapter(&self) -> &ProbeAdapterInfo {
        &self.adapter
    }

    /// The negotiated device limits and whether the fallback to WebGPU defaults
    /// was taken (see [`crate::headless::DeviceLimitsReport`]).
    pub fn device_limits(&self) -> &crate::headless::DeviceLimitsReport {
        &self.device_limits
    }

    /// The measurement-only feature negotiation (ADR 0013 class 1): what the
    /// opt-in requested, what the device granted, and the explicit state.
    pub fn gpu_time(&self) -> &crate::gpu_time::GpuTimeReport {
        &self.gpu_time
    }

    /// The configured surface texture format.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// The current surface size, in physical pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        self.size
    }

    /// The configured present mode.
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.config.present_mode
    }

    /// The requested present-mode policy (as constructed).
    pub fn present_mode_request(&self) -> ProbePresentMode {
        self.present_mode_request
    }

    /// The adapter-name preference list, as constructed.
    pub fn preferred_adapter_names(&self) -> &[String] {
        &self.preferred_names
    }
}

impl std::fmt::Debug for SurfaceProbePresenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceProbePresenter")
            .field("adapter", &self.adapter)
            .field("surface_format", &self.config.format)
            .field("surface_size", &self.size)
            .field("present_mode", &self.config.present_mode)
            .field("frame_index", &self.frame_index)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}
