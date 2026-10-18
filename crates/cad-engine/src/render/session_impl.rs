use super::*;

impl RenderSessionHandle {
    /// Whether both handles refer to the exact same owner-thread session state.
    pub fn same_session(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn new(preference: BackendPreference) -> Self {
        let state = State {
            preference,
            ..State::default()
        };
        Self(Rc::new(RefCell::new(state)))
    }

    pub fn with_incoming(incoming: IncomingDocument, preference: BackendPreference) -> Self {
        let session = Self::new(preference);
        session.0.borrow_mut().incoming = incoming;
        session
    }

    pub fn sync_drawing(&self, drawing: Option<Arc<DrawingDatabase>>) -> bool {
        let mut state = self.0.borrow_mut();
        let mut incoming = state.incoming.borrow_mut();
        let changed = match (incoming.as_ref(), &drawing) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            *incoming = drawing;
        }
        drop(incoming);
        if changed {
            state.observe_drawing();
        }
        changed
    }

    pub fn validate_snapshot(&self, view: &ViewSnapshot) -> CadResult<()> {
        view.camera.validate()?;
        if view.mode == ProjectionKind::TwoD {
            view.camera.camera2d_params()?;
        }
        if let Some(doc) = self.0.borrow().incoming.borrow().as_ref() {
            cad_app::validate_space(doc, view.space)?;
        }
        Ok(())
    }

    pub fn apply_view_snapshot(&self, view: ViewSnapshot) -> CadResult<bool> {
        self.validate_snapshot(&view)?;
        let mut state = self.0.borrow_mut();
        let previous_input_revision = state.input_revision;
        state.observe_drawing();
        if state.view == view {
            return Ok(state.input_revision != previous_input_revision);
        }
        // A document/space switch changes the authoritative scene identity and
        // must invalidate the published scene. A camera-only change (pan/zoom,
        // rotation, mode, canvas size) must not: every overlay batch is world-space
        // geometry that renders correctly under any camera, so only
        // `view_revision` advances and the reference-grid bounds are re-scoped.
        let identity_changed =
            view.document != state.view.document || view.space != state.view.space;
        // Deferral is admitted **only** when the two snapshots differ solely in
        // camera fields (center/zoom/rotation): a canvas-size, mode, document or
        // space change keeps today's immediate path.
        let camera_only = state.view.document == view.document
            && state.view.space == view.space
            && state.view.mode == view.mode
            && state.view.canvas_size == view.canvas_size
            && state.view.camera != view.camera;
        state.view = view;
        state.view_revision = state
            .view_revision
            .checked_add(1)
            .expect("view revision exhausted");
        let bounds = viewport_world_bounds(&state.view);
        let bounds_changed = state.overlays.viewport_bounds != bounds;
        if bounds_changed {
            state.overlays.viewport_bounds = bounds;
            state.overlay_revision += 1;
        }
        if identity_changed {
            // Never defer across a scene-identity change: this clears any pending
            // camera refresh as well (`advance_input` -> `advance_content`).
            state.advance_input();
        } else if bounds_changed {
            if camera_only && state.camera_settle_enabled() {
                // Defer: the published scene keeps the old grid extent and one
                // refresh is committed when the gesture settles.
                state.arm_camera_settle(Instant::now());
            } else {
                // Today's immediate path (non-camera bounds change, or the
                // zero-timeout kill-switch). Clears any pending refresh.
                state.advance_content();
            }
        } else if !camera_only {
            // A non-camera view change that did not move the bounds schedules
            // nothing by itself. It must NOT drop a pending refresh for the
            // **current** bounds (reachable through a fit-on-resize, where canvas
            // size and camera change together while the world extent stays
            // identical): cover those bounds with one content advance, which also
            // clears the pending flag. A genuine immediate path never lands here.
            if state.grid_refresh_pending {
                state.advance_content();
            }
        }
        state.runtime.dirty.invalidate();
        Ok(true)
    }

    pub fn set_fonts(&self, fonts: Option<Arc<FontEngine>>) -> bool {
        let mut state = self.0.borrow_mut();
        if state.fonts.as_ref().map(Arc::as_ptr) != fonts.as_ref().map(Arc::as_ptr) {
            state.fonts = fonts;
            state.fonts_generation = state
                .fonts_generation
                .checked_add(1)
                .expect("font resource generation exhausted");
            state.controller.fonts_changed();
            state.advance_input();
            return true;
        }
        false
    }

    pub fn set_images(&self, images: Option<Arc<DecodedImageCache>>) -> bool {
        let mut state = self.0.borrow_mut();
        if state.images.as_ref().map(Arc::as_ptr) != images.as_ref().map(Arc::as_ptr) {
            state.images = images;
            state.images_generation = state
                .images_generation
                .checked_add(1)
                .expect("image resource generation exhausted");
            state.advance_input();
            return true;
        }
        false
    }

    pub fn set_layers(&self, layers: LayerOverrideSet) -> bool {
        let mut state = self.0.borrow_mut();
        if state.layers == layers {
            return false;
        }
        state.layers = layers;
        state.advance_input();
        true
    }
    pub fn set_overlays(&self, overlays: OverlayInputs) -> bool {
        let mut state = self.0.borrow_mut();
        if state.overlays == overlays {
            return false;
        }
        // A change confined to the camera-derived `viewport_bounds` only re-scopes
        // the reference grid: it must schedule a background re-prepare but never
        // invalidate the published scene. Any other overlay input (visibility,
        // selection, previews, snap hints) changes real overlay batches and keeps
        // invalidating exactly as before.
        let bounds_only = {
            let mut probe = state.overlays.clone();
            probe.viewport_bounds = overlays.viewport_bounds;
            probe == overlays
        };
        state.overlays = overlays;
        state.overlay_revision += 1;
        if bounds_only {
            state.advance_content();
        } else {
            state.advance_input();
        }
        true
    }

    /// CPU-only scene preparation; safe to drive without constructing a UI.
    pub fn prepare(&self) -> CadResult<()> {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return Err(CadError::InvalidInput("render session stopped".into()));
        }
        prepare_synchronously(&mut state)
    }

    /// Register the newest immutable preparation input before a UI frame. The
    /// returned bool indicates that a completion/poll should be scheduled.
    pub fn request_preparation(&self) -> CadResult<bool> {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return Ok(false);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            state.observe_drawing();
            state.settle_camera_if_due(Instant::now());
            let input = preparation_input(&state);
            match state.preparation.update(input) {
                Ok(Some((token, controller))) => {
                    publish_controller(&mut state, controller, token);
                }
                Ok(None) => {}
                Err(error) => state.controller.diagnostic = Some(error.to_string()),
            }
            Ok(true)
        }
        #[cfg(target_arch = "wasm32")]
        {
            prepare_synchronously(&mut state)?;
            Ok(false)
        }
    }

    /// Poll asynchronous native preparation and publish only the latest input.
    pub fn poll_preparation(&self) -> CadResult<bool> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = self.0.borrow_mut();
            if state.stopped {
                return Ok(false);
            }
            state.observe_drawing();
            state.settle_camera_if_due(Instant::now());
            let input = preparation_input(&state);
            match state.preparation.update(input) {
                Ok(Some((token, controller))) => {
                    publish_controller(&mut state, controller, token);
                }
                Ok(None) => {}
                Err(error) => state.controller.diagnostic = Some(error.to_string()),
            }
            Ok(state.preparation.busy())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(false)
        }
    }

    pub fn incoming_document(&self) -> IncomingDocument {
        self.0.borrow().incoming.clone()
    }
    pub fn view_snapshot(&self) -> ViewSnapshot {
        self.0.borrow().view.clone()
    }
    pub fn camera(&self) -> Camera {
        self.0.borrow().view.camera
    }
    pub fn space(&self) -> SpaceSelection {
        self.0.borrow().view.space
    }
    pub fn view_mode(&self) -> ProjectionKind {
        self.0.borrow().view.mode
    }
    pub fn view_diagnostic(&self) -> Option<String> {
        let state = self.0.borrow();
        state
            .view_diagnostic
            .clone()
            .or_else(|| state.controller.diagnostic.clone())
            .or_else(|| state.runtime.diagnostic.clone())
            .or_else(|| {
                state
                    .controller
                    .ready
                    .as_ref()
                    .and_then(|s| s.highlight_diagnostics.first().map(|d| d.message.clone()))
            })
    }
    pub fn scene_error(&self) -> Option<String> {
        let state = self.0.borrow();
        state
            .controller
            .diagnostic
            .clone()
            .or_else(|| state.runtime.diagnostic.clone())
    }
    pub fn set_view_diagnostic(&self, value: Option<String>) {
        self.0.borrow_mut().view_diagnostic = value;
    }
    pub fn set_runtime_diagnostic(&self, value: Option<String>) {
        self.0.borrow_mut().runtime.diagnostic = value;
    }
    pub fn fonts(&self) -> Option<Arc<FontEngine>> {
        self.0.borrow().fonts.clone()
    }
    pub fn overlays(&self) -> OverlayInputs {
        self.0.borrow().overlays.clone()
    }
    pub fn overlay_revision(&self) -> u64 {
        self.0.borrow().overlay_revision
    }
    pub fn cad_frame_dirty(&self) -> bool {
        self.0.borrow().runtime.dirty.dirty()
    }
    pub fn current_drawing_presented(&self) -> bool {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        if !Self::preparation_current_state(&state) {
            return false;
        }
        // A base-only navigation frame is never the presented current drawing.
        if state.runtime.overlays_suppressed() {
            return false;
        }
        state.runtime.presented_revision.is_some_and(|revision| {
            state
                .controller
                .ready
                .as_ref()
                .is_some_and(|scene| scene.revision == revision)
        }) && !state.runtime.uploading()
    }
    fn preparation_current_state(state: &State) -> bool {
        current_ready_scene(state).is_some()
    }
    pub fn preparation_current(&self) -> bool {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        current_ready_scene(&state).is_some()
    }
    pub fn input_revision(&self) -> u64 {
        self.0.borrow().input_revision
    }
    /// Monotonic revision of the authoritative view (camera/space). Advances only
    /// on an actual view change; presented frames are tagged with it.
    pub fn view_revision(&self) -> u64 {
        self.0.borrow().view_revision
    }
    /// Whether the 2D navigation fallback is currently withholding overlays.
    pub fn overlays_suppressed(&self) -> bool {
        self.0.borrow().runtime.overlays_suppressed()
    }
    /// Whether a camera-only (pan/zoom/rotate) reference-grid refresh is deferred
    /// until the navigation settles. Honest: it is `true` only while the current
    /// bounds have **not** been re-prepared yet, and it clears exactly when the
    /// content revision advances (settle commit, or any immediate input).
    pub fn grid_refresh_pending(&self) -> bool {
        self.0.borrow().grid_refresh_pending
    }
    /// Human-readable companion for [`Self::grid_refresh_pending`]; never a fake
    /// final-commit claim.
    pub fn grid_refresh_diagnostic(&self) -> &'static str {
        grid_refresh_label(self.grid_refresh_pending())
    }
    /// The camera-settle timeout currently in force.
    pub fn grid_settle_timeout(&self) -> Duration {
        self.0.borrow().settle_timeout
    }
    /// Test hook: set the camera-settle timeout. `Duration::ZERO` disables the
    /// deferral (today's immediate behaviour) and flushes any pending refresh on
    /// the next pump.
    pub fn set_grid_settle_timeout(&self, timeout: Duration) {
        self.0.borrow_mut().settle_timeout = timeout;
    }
    /// Cumulative overlay-suffix commits (uploads or truncations) the runtime has
    /// performed. A camera-only navigation step must not advance this: it is the
    /// cheap churn signal for the "camera never invalidates the scene" contract.
    pub fn overlay_suffix_commits(&self) -> u64 {
        self.0.borrow().runtime.overlay_suffix_commits()
    }
    pub fn current_prepared_scene(&self) -> Option<controller::PreparedScene> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        current_ready_scene(&state).cloned()
    }
    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| caps.actual)
    }
    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| (caps.actual, caps.compute, caps.max_texture_dimension))
    }
    pub fn backend_label(&self) -> Option<String> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| caps.api.into())
    }
    pub fn preparation_busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.borrow().preparation.busy()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.0.borrow().runtime.drawing_progress()
    }
    pub fn uploading(&self) -> bool {
        self.0.borrow().runtime.uploading()
    }
    pub fn drawing_pages(&self) -> bool {
        self.0.borrow().runtime.drawing_pages()
    }
    pub fn preference(&self) -> BackendPreference {
        self.0.borrow().preference
    }
    pub fn backend_outcome(&self) -> Option<BackendOutcome> {
        self.0.borrow().runtime.outcome.clone()
    }
    pub fn controller_ready(&self) -> Option<controller::PreparedScene> {
        self.0.borrow().controller.ready.clone()
    }
    pub fn set_overlay_visibility(&self, visibility: OverlayVisibility) -> bool {
        let mut overlays = self.overlays();
        if overlays.visibility == visibility {
            return false;
        }
        overlays.visibility = visibility;
        self.set_overlays(overlays)
    }
    pub fn overlay_visibility(&self) -> OverlayVisibility {
        self.0.borrow().overlays.visibility
    }
    pub fn set_selection(&self, selection: SelectionSet) -> bool {
        let mut overlays = self.overlays();
        if overlays.selection == selection {
            return false;
        }
        overlays.selection = selection;
        self.set_overlays(overlays)
    }
    pub fn selection(&self) -> SelectionSet {
        self.0.borrow().overlays.selection.clone()
    }
    pub fn set_measurement_preview(&self, preview: Option<MeasurementPreview>) -> bool {
        let mut overlays = self.overlays();
        if overlays.measurement == preview {
            return false;
        }
        overlays.measurement = preview;
        self.set_overlays(overlays)
    }
    pub fn set_snap_hints(&self, hints: Vec<SnapHint>) -> bool {
        let mut overlays = self.overlays();
        if overlays.snap_hints_input == hints {
            return false;
        }
        overlays.snap_hints_input = hints;
        self.set_overlays(overlays)
    }
    pub fn snap_hints(&self) -> Vec<SnapHint> {
        self.0.borrow().overlays.snap_hints_input.clone()
    }
    pub fn set_draw_preview(&self, preview: Option<cad_app::DrawPreview>) -> bool {
        let mut overlays = self.overlays();
        if overlays.draw_preview == preview {
            return false;
        }
        overlays.draw_preview = preview;
        self.set_overlays(overlays)
    }
    pub fn draw_preview(&self) -> Option<cad_app::DrawPreview> {
        self.0.borrow().overlays.draw_preview.clone()
    }
    pub fn set_viewport_bounds(&self, bounds: Option<(Point3, Point3)>) {
        let mut overlays = self.overlays();
        if overlays.viewport_bounds == bounds {
            return;
        }
        overlays.viewport_bounds = bounds;
        self.set_overlays(overlays);
    }

    pub fn attach_device(&self, device: wgpu::Device, queue: wgpu::Queue) -> Result<(), String> {
        self.attach_device_with_gpu_time(
            device,
            queue,
            cad_render_wgpu::gpu_time::GpuTimeConfig::disabled(),
        )
    }

    /// [`RenderSessionHandle::attach_device`] with the measurement-only GPU
    /// frame-time opt-in (ADR 0013 class 1). The config is threaded to the
    /// renderer through [`CadRenderRuntime::attach_with_gpu_time`]; the engine
    /// never reads the environment.
    pub fn attach_device_with_gpu_time(
        &self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        gpu_time: cad_render_wgpu::gpu_time::GpuTimeConfig,
    ) -> Result<(), String> {
        let mut state = self.0.borrow_mut();
        let preference = state.preference;
        state
            .runtime
            .attach_with_gpu_time(device, queue, preference, gpu_time);
        match &state.runtime.lifecycle {
            RenderLifecycle::Ready { .. } => Ok(()),
            RenderLifecycle::Failed { reason } | RenderLifecycle::Lost { reason } => {
                Err(reason.clone())
            }
            RenderLifecycle::Detached => {
                Err("renderer remained detached after device attach".into())
            }
        }
    }

    pub fn detach_device(&self) {
        self.0.borrow_mut().runtime.detach();
    }

    pub fn fail_device(&self, reason: impl Into<String>, lost: bool) {
        let mut state = self.0.borrow_mut();
        let preference = state.preference;
        state.runtime.fail(preference, reason.into(), lost);
    }

    pub fn apply_ready_updates(&self) -> CadResult<()> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        let ready = current_ready_scene(&state).cloned();
        state.runtime.apply_ready_updates(ready.as_ref())
    }

    pub fn render(
        &self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        let view = state.view.clone();
        let view_revision = state.view_revision;
        if Self::preparation_current_state(&state) {
            // The CPU scene is current. Only render it as `CurrentScene` once the
            // GPU has actually committed that scene revision and its base; a
            // chunked `apply_ready_updates` upload may still be pending, and
            // rendering then would present the resident old scene (or a partially
            // uploaded one) under a `CurrentScene` label.
            let committed = match current_ready_scene(&state) {
                Some(scene) => state
                    .runtime
                    .scene_committed(scene.revision, scene.base_revision),
                None => false,
            };
            if committed {
                return state
                    .runtime
                    .render_if_dirty(&view, logical, scale, view_revision);
            }
        }
        // Strict gate is not met (or the ready scene is not yet GPU-committed).
        // Enter the narrow 2D navigation fallback only when the authoritative base
        // is unchanged and fully resident; render the latest camera over the base
        // while overlays wait. Never a new/uncommitted base.
        if base_only_navigation_eligible(&state) {
            return state
                .runtime
                .render_base_only(&view, logical, scale, view_revision);
        }
        Ok(None)
    }

    pub fn lifecycle(&self) -> RenderLifecycle {
        self.0.borrow().runtime.lifecycle.clone()
    }
    pub fn diagnostic(&self) -> Option<String> {
        self.0.borrow().runtime.diagnostic.clone()
    }
    /// Take the observation of the most recent **actual** renderer invocation.
    ///
    /// Delivered at most once per render sequence, so a host can never replay
    /// stale stats for a retained/reused or waiting frame. `None` when no new
    /// actual render happened since the previous take.
    pub fn take_render_stats_observation(&self) -> Option<RenderStatsObservation> {
        self.0.borrow_mut().runtime.take_render_observation()
    }
    /// Take the latest upload stage-timing observation (at most once per upload
    /// work unit). `None` when no new upload staged/committed since the last take.
    pub fn take_upload_timing_observation(&self) -> Option<UploadTimingObservation> {
        self.0.borrow_mut().runtime.take_upload_timing_observation()
    }
    /// The current device epoch (increments on every successful attach).
    pub fn device_epoch(&self) -> u64 {
        self.0.borrow().runtime.device_epoch()
    }
    pub fn frames_rendered(&self) -> u64 {
        self.0.borrow().runtime.frames_rendered
    }
    pub fn redraw_requests(&self) -> u64 {
        self.0.borrow().redraw_requests
    }
    pub fn note_redraw_request(&self) {
        self.0.borrow_mut().redraw_requests += 1;
    }
    pub fn request_redraw(&self, wake: &impl RenderWake) {
        let mut s = self.0.borrow_mut();
        s.redraw_requests += 1;
        if !s.stopped {
            drop(s);
            wake.request_redraw();
        }
    }
    pub fn wake_redraw(&self, wake: &impl RenderWake) {
        if !self.0.borrow().stopped {
            wake.request_redraw();
        }
    }
    pub fn schedule_preparation(&self, wake: &impl RenderWake) {
        if !self.0.borrow().stopped {
            wake.request_preparation();
        }
    }
    pub fn stop(&self) {
        let mut s = self.0.borrow_mut();
        s.stopped = true;
        #[cfg(not(target_arch = "wasm32"))]
        s.preparation.stop();
        s.runtime.detach();
    }
}
