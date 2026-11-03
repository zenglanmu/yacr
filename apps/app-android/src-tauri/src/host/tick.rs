//! One owner tick of the Android `native-below` carrier.
//!
//! Mirrors the native path of `apps/app-linux/src/host/tick.rs` /
//! `apps/app-macos/src/host/tick.rs` with the platform substitutions this carrier
//! needs, and nothing else:
//!
//! 1. main-thread bookkeeping (stop request, ~60 fps throttle, 1 Hz evidence line);
//! 2. ensure the presenter for the live platform surface ([`crate::surface`]);
//! 3. feed the surface source from the platform surface extent and compare it with
//!    the accepted `probe_input` layout (one is the pixel authority, the other the
//!    layout signal; a disagreement is logged, never silently rescaled);
//! 4. lazily create the native [`CadSession`], attach it to the presenter's device
//!    and drain the shell queues into it;
//! 5. one `CadSession::tick`;
//! 6. draw/present one CAD frame (or explicitly wait);
//! 7. mirror the status/settings/layers/fonts into [`Shared`] and push one event batch
//!    with the shared `native-below` profile.
//!
//! # Honest limits of v1
//!
//! * **Documents arrive through the platform picker, never `--open`.** There is
//!   no `--open` argument and no fixture funnel, so the session is created with
//!   `HostOptions.drawing = None`: it starts from the **shared host core's
//!   built-in starting document** (`HostCore::new` always builds it; every
//!   desktop host launched without `--open` shows the same one). After startup,
//!   `OPEN` and an external VIEW intent are served by the SAF picker
//!   ([`crate::platform::AndroidFiles`] + [`crate::saf`]); those bytes come from
//!   a real user choice, and Kotlin holds no document state. `canOpen` is
//!   advertised because that funnel exists; `NEW`/`PLOT` remain explicitly
//!   unsupported. Creating a session without a document is not an available API;
//!   if a no-document-at-all carrier is required, that is a `cad-host` change,
//!   not something this host may fake.
//! * **Popups are in-DOM overlays.** The host registers a real
//!   `ShellPopupHost` ([`crate::popup`]) that marshals the shared lifecycle onto
//!   this thread and derives the `textEdit` payload from the live session, so
//!   Options/About/text-edit work here with the same modal gate as the desktop.
//! * **Preferences persist in the app's private `filesDir`** once the
//!   `nativeAssetsReady` handshake has announced it (`config.json` /
//!   `preference.json`, atomic replace). Before that handshake the store keeps the
//!   honest non-persistent behaviour; no path is invented.
//! * **No probe marker path.** `YACR_PROBE_ONLY` disables CAD entirely (the same
//!   "CAD is disabled" status as the desktop hosts) and presents nothing; the
//!   desktop grid marker is not a product path and is not ported.
//! * **Pixels are the `SurfaceView`'s.** The Kotlin lane inserts a `MATCH_PARENT`
//!   `SurfaceView` behind the WebView, so the platform surface is the full window
//!   and its extent is the only size an Android Vulkan swapchain may take (the
//!   spec requires `imageExtent == currentExtent`). The session's canvas metrics
//!   and the render viewport are therefore the **surface's** logical size
//!   (`physical / scale`), which keeps the camera and the presented pixels mutually
//!   consistent; the renderer refuses to stretch a mismatched texture, so the image
//!   is never distorted. The React `.native-input` rect still drives the layout
//!   diagnostic: when its physical size disagrees with the surface, `[probe]
//!   geometry` says so instead of silently rescaling (a SurfaceView sized to the
//!   partition, or a partition covering the surface, makes the two equal).
//! * **Input is partition-local and must be moved.** The chrome reports pointer and
//!   wheel positions relative to the `.native-input` rect, while the canvas is the
//!   full window, so those positions differ by exactly that rect's origin. The input
//!   drain adds it before the session sees the event ([`crate::input`]); a pick,
//!   hover, crosshair or wheel-zoom anchor is otherwise off by the partition
//!   origin. `resize` (the rect itself) and the coordinate-free kinds are passed
//!   through unchanged — never rescaled, and never "fixed" with `safe_insets`,
//!   which is a surface reservation and would add a scale error.
//! * **Known limitation (tracked follow-up, engine/viewport scope).**
//!   `fit_camera` frames the canvas the session was given — the **full window** —
//!   so a fitted drawing can sit partly **under the chrome** (ribbon/panels). The
//!   fix is a viewport/inset concept in `cad-engine` (canvas = surface minus the
//!   chrome, plus the matching projection), not a platform-side transform; it is
//!   tracked outside this lane. Input mapping is exact meanwhile.
//!
//! # Pause / resume
//!
//! `surfaceDestroyed` releases the presenter (see [`crate::surface`]); this tick
//! then reports `paused` and presents nothing. A new `surfaceCreated` produces a new
//! generation, the next tick rebuilds the presenter and re-attaches the **same**
//! session, so the drawing survives a rotation or a background/foreground cycle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::HostPlatform;
use cad_platform::HostCapabilities;
use cad_ui_core::protocol::ProbeInput;
use cad_ui_core::session::CadSession;
use cad_ui_core::shell::{
    bind_allowed, clear_host_error, frame_interval, note_host_error, HostErrorKind,
};
use cad_ui_core::{session::HostProfile, unsupported_status};

use crate::policy;
use crate::surface::{FrameOutcome, SurfaceStep};

use super::{lock, lock_telemetry, RenderTelemetry, Shared, CAD, CAD_ENV};

/// One owner tick. Runs on the Tauri/tao loop thread (not the Android UI thread).
pub fn tick(shared: &Arc<Mutex<Shared>>, telemetry: &Arc<Mutex<RenderTelemetry>>) {
    // ---- short critical section: bookkeeping only ----
    let scale = {
        let mut state = lock(shared);
        // Lock order (frozen): Shared -> RenderTelemetry, never the reverse.
        let mut telemetry = lock_telemetry(telemetry);
        telemetry.wake_pending.store(false, Ordering::SeqCst);

        if state.shell.stopped {
            return;
        }
        if state.shell.stop_requested {
            drop(telemetry);
            drop(state);
            crate::surface::release();
            let mut state = lock(shared);
            state.shell.stopped = true;
            state.shell.status.stopped = true;
            return;
        }

        let now = Instant::now();
        if let Some(last) = telemetry.last_frame {
            if now.duration_since(last) < frame_interval(state.shell.input_active(now)) {
                return;
            }
        }
        telemetry.last_frame = Some(now);
        state.shell.ticks += 1;

        if telemetry
            .last_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            telemetry.last_log = Some(now);
            log_status_line(&state, telemetry.last_timings);
        }

        let window_scale = state
            .window
            .as_ref()
            .and_then(|window| window.scale_factor().ok())
            .unwrap_or(1.0);
        // The chrome measured every wire position with `devicePixelRatio`, so that
        // ratio — not `tao`'s separately-reported density — is the physical/logical
        // scale the camera and input must share. A disagreement here scales the
        // whole input space and puts a pinch anchor away from the fingers.
        let dom_dpr = state.shell.input.last_rect().map(|rect| rect.dpr);
        let scale = crate::input::canvas_scale(window_scale, dom_dpr);
        static SCALE_LOGGED: AtomicBool = AtomicBool::new(false);
        if dom_dpr.is_some() && !SCALE_LOGGED.swap(true, Ordering::Relaxed) {
            log::info!(
                "[probe] canvas scale: window.scale_factor={window_scale:.4} \
                 webview.dpr={dom_dpr:?} using={scale:.4}"
            );
        }
        scale
    };

    if !lock(shared).shell.cad_mode {
        // Evidence-only `YACR_PROBE_ONLY`: CAD is off, nothing is presented, and the
        // shared "CAD is disabled" snapshot is pushed so the chrome can show it.
        flush(shared);
        return;
    }

    // ---- presenter for the live platform surface (create / rebuild / resize) ----
    let gpu_time = cad_render_wgpu::gpu_time::GpuTimeConfig {
        enabled: std::env::var_os("YACR_PROBE_GPU_TIME").is_some(),
    };
    let (width, height, generation) = match crate::surface::ensure_presenter(gpu_time) {
        SurfaceStep::Ready {
            width,
            height,
            generation,
        } => (width, height, generation),
        SurfaceStep::Awaiting => {
            let paused = {
                let mut state = lock(shared);
                let was = state.shell.paused;
                state.shell.paused = true;
                // A hidden surface is not an error: keep any real session error.
                was
            };
            if !paused {
                log::info!("[probe] paused: no native surface (surfaceMode=native-below)");
            }
            flush(shared);
            return;
        }
        SurfaceStep::Refused { reason } => {
            let mut state = lock(shared);
            state.shell.paused = true;
            // The refusal is a present-side host error, not only a `ProbeStatus`
            // field: `cad_status.error` is what the chrome can say out loud about
            // an area that is dark, so it must carry the exact refusal text. The
            // stable marker line itself was logged once by `surface` when the
            // refusal was created (a software adapter, or no Vulkan presenter).
            let text = format!("native surface refused: {reason}");
            state.shell.status.error = Some(text.clone());
            note_host_error(&mut state.shell, HostErrorKind::Present, text);
            // Before the first snapshot there is no `cad_status` for the shared
            // projection to fold the host error into, and a refusal returns before the
            // session is ever created — so on a software/absent Vulkan adapter the
            // chrome would read `cad_status.error = null` forever. Publish the
            // explicit unsupported status (no error of its own: the host error above
            // provides the text exactly once) so the refusal is readable.
            if state.shell.cad_status.is_none() {
                state.shell.cad_status = Some(unsupported_status(None));
            }
            drop(state);
            flush(shared);
            return;
        }
    };

    {
        let mut state = lock(shared);
        if state.shell.paused {
            log::info!("[probe] resumed {width}x{height} (presenter generation {generation})");
            state.shell.paused = false;
        }
        state.shell.status.width = width;
        state.shell.status.height = height;
        if let Some((backend, adapter)) = crate::surface::adapter_info() {
            state.shell.status.backend = Some(backend);
            state.shell.status.adapter = Some(adapter);
        }
        if let Some(mode) = crate::surface::present_mode() {
            state.shell.present_mode = Some(mode);
        }
    }
    let new_generation = {
        let mut telemetry = lock_telemetry(telemetry);
        let changed = telemetry.last_surface_generation != Some(generation);
        telemetry.last_surface_generation = Some(generation);
        changed
    };
    if new_generation {
        // A new GPU generation: keep the DOM input sequence monotonic and
        // re-validate the cached CAD rect against the current geometry.
        let mut state = lock(shared);
        state.layout.note_generation();
        state.layout_revalidate = true;
    }

    // ---- surface source: the platform surface is the pixel authority ----
    //
    // A DOM-rect-sized texture cannot be presented to a full-window Android
    // surface (Vulkan requires the swapchain extent to equal the native window), and
    // the renderer refuses to stretch one. So the session's canvas metrics and the
    // render viewport are the surface's logical size — the only choice that keeps
    // pixels and camera consistent. Input does **not** follow for free: the chrome
    // reports positions relative to its `.native-input` rect, so the input drain
    // below adds that rect's origin before `session.handle_input` (see
    // `crate::input`). The React layout rect is still reported (`probe_input`) and
    // compared here: a size disagreement is logged, never silently rescaled.
    let metrics = cad_platform::SurfaceMetrics {
        logical: [f64::from(width) / scale, f64::from(height) / scale],
        scale,
        safe_insets: [0.0; 4],
    };
    CAD_ENV.with(|cell| {
        if let Some(env) = cell.borrow().as_ref() {
            env.surface.set_metrics(metrics);
        }
    });
    let dom_rect = lock(shared).shell.input.last_rect();
    if let Some(rect) = dom_rect {
        let physical = cad_ui_core::layout::physical_content_size(&rect, scale);
        if physical != (width, height) {
            // One line per geometry change, deduplicated: never per tick.
            log_dom_surface_disagreement(telemetry, physical, (width, height));
        }
    }

    // ---- refresh the announced assets while no session exists yet ----
    //
    // `nativeAssetsReady` is posted from the Android UI thread and may land after
    // `setup` built the composition. Re-read the announcement here (the tick thread
    // owns `CAD_ENV`); a session's `HostOptions` are fixed once it is built, and a
    // missing font/image package would then be an explicit resource gap in the
    // session's own state, never a silent success.
    if CAD.with(|cell| cell.borrow().is_none()) {
        let (fonts_dir, images_dir) = crate::platform::option_dirs();
        CAD_ENV.with(|cell| {
            if let Some(env) = cell.borrow_mut().as_mut() {
                env.options.fonts_dir = fonts_dir;
                env.options.images_dir = images_dir;
            }
        });
    }

    // ---- CAD session: lazy creation (the owner thread owns the document) ----
    let cad_mode = lock(shared).shell.cad_mode;
    if cad_mode && dom_rect.is_some() && CAD.with(|cell| cell.borrow().is_none()) {
        let created = CAD_ENV.with(|cell| {
            let env = cell.borrow();
            let env = env.as_ref()?;
            // The surface must report before `CadSession::new`: the open funnel
            // validates canvas metrics against it.
            env.surface.set_metrics(metrics);
            let platform = HostPlatform {
                files: env.files.clone(),
                config_store: env.config.clone(),
                surface: std::rc::Rc::new(env.surface.clone()),
                // The desktop resource strategy reads `HostOptions.fonts_dir` /
                // `images_dir`, which point at the directories the Kotlin lane
                // extracted and announced (`nativeAssetsReady`). A resource that
                // cannot be resolved is reported by the session, never dropped
                // silently.
                resources: std::rc::Rc::new(cad_host::DesktopResources),
                capabilities: HostCapabilities {
                    open: policy::CAN_OPEN,
                    new: policy::CAN_NEW,
                    save: policy::CAN_SAVE,
                    plot: policy::CAN_PLOT,
                    import_worker: policy::IMPORT_WORKER,
                },
            };
            Some(CadSession::new(
                env.options.clone(),
                platform,
                RenderSessionHandle::new(BackendPreference::WebGpu),
            ))
        });
        match created {
            Some(Ok(session)) => {
                // The Options dialog's GPU budget is a renderer policy: install
                // the configured percentage before the first device attach (the
                // engine re-applies it on every re-attach).
                let percent = session.settings().gpu_budget_percent;
                if let Ok(percent) = u8::try_from(percent) {
                    session.render_session().set_gpu_budget_percent(percent);
                }
                CAD.with(|cell| *cell.borrow_mut() = Some(session));
                log::info!("[probe] cad session created (native document owner)");
            }
            Some(Err(error)) => {
                let text = format!("{error:?}");
                log::error!("[probe] cad session creation FAILED: {text}");
                // A CadSession-level (core/open) failure lives in the snapshot,
                // never in the host transient error.
                lock(shared).shell.cad_status = Some(unsupported_status(Some(text)));
            }
            None => {}
        }
    }

    // ---- bind the render session to this presenter generation (once) ----
    //
    // Bound generation, not `cad_ready`: `cad_ready` means "the session has been
    // bound at least once" (it gates input admission), while a surface that was
    // destroyed and re-created gets a **new** presenter generation with no device
    // attached. Comparing the generation is what re-attaches the same session after
    // a rotation/backgrounding instead of waiting forever for a bind that never runs.
    if cad_mode && crate::surface::bound_generation() != Some(generation) {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).shell.cad_bind_failed_generation;
            if bind_allowed(false, failed_generation, generation) {
                match crate::surface::bind_session(&render) {
                    Ok(bound) => {
                        let mut state = lock(shared);
                        state.shell.cad_ready = true;
                        state.shell.cad_bind_failed_generation = None;
                        clear_host_error(&mut state.shell, HostErrorKind::Bind);
                        log::info!("[probe] cad render session bound (generation {bound})");
                    }
                    Err(error) => {
                        let mut state = lock(shared);
                        state.shell.cad_bind_failed_generation = Some(generation);
                        note_host_error(
                            &mut state.shell,
                            HostErrorKind::Bind,
                            format!("cad bind failed: {error}"),
                        );
                        log::error!(
                            "[probe] cad bind FAILED (generation {generation}); fail-stop until a \
                             new generation: {error}"
                        );
                    }
                }
            }
        }
    }

    // ---- queued commands + ordered inputs (batches copied, lock released) ----
    if cad_mode {
        let (
            commands,
            mut standard_views,
            setting_writes,
            mut layer_writes,
            tool_parameters,
            tool_texts,
            selected_texts,
            selected_properties,
            inputs,
        ) = {
            let mut state = lock(shared);
            (
                std::mem::take(&mut state.shell.cad_commands),
                std::mem::take(&mut state.cad_standard_views),
                std::mem::take(&mut state.cad_setting_writes),
                std::mem::take(&mut state.cad_layer_writes),
                std::mem::take(&mut state.cad_tool_parameters),
                std::mem::take(&mut state.cad_tool_texts),
                std::mem::take(&mut state.cad_selected_texts),
                std::mem::take(&mut state.cad_selected_properties),
                std::mem::take(&mut state.shell.cad_inputs),
            )
        };
        let queued = commands.len()
            + standard_views.len()
            + setting_writes.len()
            + layer_writes.len()
            + tool_parameters.len()
            + tool_texts.len()
            + selected_texts.len()
            + selected_properties.len()
            + inputs.len();
        // The `.native-input` partition origin (CSS px) of the accepted layout rect.
        // Positions from the chrome are relative to that rect while the canvas is
        // the whole window, so the drain below moves them by this origin. No rect
        // yet means no session either (creation is gated on `dom_rect`), and a zero
        // origin is then the honest identity.
        let partition_origin = dom_rect.map(|rect| (rect.x, rect.y)).unwrap_or((0.0, 0.0));
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    // Settings first: an Options change in this batch is visible to
                    // the commands/inputs that follow.
                    for (key, value) in setting_writes {
                        let before = session.settings().gpu_budget_percent;
                        match session.apply_setting(key, value) {
                            Ok(settings) if settings.gpu_budget_percent != before => {
                                // The GPU budget is a renderer policy: install the new
                                // percentage on the live render session immediately.
                                if let Ok(percent) = u8::try_from(settings.gpu_budget_percent) {
                                    session.render_session().set_gpu_budget_percent(percent);
                                }
                            }
                            Ok(_) => {}
                            Err(error) => log::warn!("[probe] cad setting rejected: {error}"),
                        }
                    }
                    for error in cad_desktop_shell::commands::drain_layer_writes(
                        &mut layer_writes,
                        |write| write.apply(session),
                    ) {
                        log::warn!("[probe] cad layer write rejected: {error}");
                    }
                    for command in &commands {
                        if let Err(error) = session.command(command) {
                            log::warn!("[probe] cad command rejected: {error}");
                        }
                    }
                    for error in cad_desktop_shell::commands::drain_standard_views(
                        &mut standard_views,
                        |view| session.standard_view(view),
                    ) {
                        log::warn!("[probe] cad standard view rejected: {error}");
                    }
                    for (parameter, value) in &tool_parameters {
                        if let Err(error) = session.set_tool_parameter(*parameter, *value) {
                            log::warn!("[probe] cad tool parameter rejected: {error}");
                        }
                    }
                    for input in &inputs {
                        // Partition-local -> canvas space: the native surface is the
                        // full window, while the chrome reports positions relative to
                        // its `.native-input` rect. Pointer and wheel positions get
                        // that rect's origin added; `resize` (which carries the rect
                        // itself) and the coordinate-free kinds pass through
                        // untouched. No rescale, no `safe_insets`.
                        let (x, y) = crate::input::canvas_position(
                            input.kind.as_str(),
                            partition_origin,
                            input.x,
                            input.y,
                        );
                        let translated = ProbeInput {
                            x,
                            y,
                            ..input.clone()
                        };
                        if let Err(error) = session.handle_input(&translated) {
                            log::warn!("[probe] cad input rejected: {error}");
                        }
                    }
                    // Terminal tool texts apply after this batch's inputs (the last
                    // point of a text tool may arrive in the same tick).
                    for text in &tool_texts {
                        if let Err(error) = session.set_tool_text(text) {
                            log::warn!("[probe] cad tool text rejected: {error}");
                        }
                    }
                    for text in &selected_texts {
                        if let Err(error) = session.set_selected_text(text) {
                            log::warn!("[probe] cad selected text rejected: {error}");
                        }
                    }
                    for (key, value) in &selected_properties {
                        if let Err(error) = session.set_selected_property(key, value) {
                            log::warn!("[probe] cad selected property rejected: {error}");
                        }
                    }
                }
            });
        }
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        let tick_result = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.tick()));
        match tick_result {
            Some(Ok(())) => clear_host_error(&mut lock(shared).shell, HostErrorKind::Tick),
            Some(Err(error)) => note_host_error(
                &mut lock(shared).shell,
                HostErrorKind::Tick,
                format!("cad tick failed: {error}"),
            ),
            None => {}
        }
    }

    // ---- one CAD frame (tick thread, no shared lock held across GPU work) ----
    let mut presented = false;
    if cad_mode && lock(shared).shell.cad_ready {
        match crate::surface::frame(scale) {
            FrameOutcome::Wait => {}
            FrameOutcome::Presented {
                frame_index,
                timings,
            } => {
                presented = true;
                CAD.with(|cell| {
                    if let Some(session) = cell.borrow().as_ref() {
                        session.note_frame(frame_index);
                    }
                });
                let mut state = lock(shared);
                state.shell.status.frames = frame_index;
                // A good present clears only a previous present error.
                clear_host_error(&mut state.shell, HostErrorKind::Present);
                lock_telemetry(telemetry).last_timings = Some(timings);
            }
            FrameOutcome::Rebuild { reason } => {
                log::warn!("[probe] cad presenter rebuild required: {reason}");
                crate::surface::release();
                let mut state = lock(shared);
                state.shell.cad_ready = false;
                state.shell.cad_bind_failed_generation = None;
                note_host_error(
                    &mut state.shell,
                    HostErrorKind::Present,
                    format!("cad presenter rebuild required: {reason}"),
                );
            }
            FrameOutcome::Failed { class, text } => {
                log::error!(
                    "[probe] cad-present-error class={} text={}",
                    class.as_str(),
                    text
                );
                note_host_error(
                    &mut lock(shared).shell,
                    HostErrorKind::Present,
                    format!("cad present failed: {text}"),
                );
            }
        }
    }

    // ---- status mirror + one event batch (the exact `cad_status` payload) ----
    let (backend, adapter) = crate::surface::adapter_info()
        .map(|(backend, adapter)| (Some(backend), Some(adapter)))
        .unwrap_or((None, None));
    let snapshot = CAD.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|session| session.snapshot(backend, adapter, 0))
    });
    let settings = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.settings()));
    let layers = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.layers()));
    let fonts = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.fonts()));
    {
        let mut state = lock(shared);
        if let Some(snapshot) = &snapshot {
            state.shell.cad_status = Some(snapshot.clone());
        }
        if let Some(settings) = settings {
            state.cad_settings = Some(settings);
        }
        if let Some(layers) = layers {
            // A projection error is mirrored as the error itself, never replaced by
            // stale rows.
            state.cad_layers = Some(layers);
        }
        if let Some(fonts) = fonts {
            // The read-only Font Manager's mirror: the session's real report,
            // never an invented row.
            state.cad_fonts = Some(fonts);
        }
    }
    if presented {
        let actual = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|session| session.render_session().take_render_stats_observation())
        });
        if let Some(actual) = actual {
            lock_telemetry(telemetry).last_pulled_state = Some(actual.pulled_state);
        }
    }
    // The flush is outside the `Some(snapshot)` arm: the lazy-session window and a
    // creation failure have no snapshot but still owe the chrome the same status.
    // This carrier has no wasm `canvasSession`.
    let _ = cad_desktop_shell::events::flush_events_current(
        shared,
        &HostProfile::native_below(),
        None::<String>,
    );
}

/// Push one status batch with this carrier's profile (no canvas descriptor).
fn flush(shared: &Arc<Mutex<Shared>>) {
    let _ = cad_desktop_shell::events::flush_events_current(
        shared,
        &HostProfile::native_below(),
        None::<String>,
    );
}

/// The ~1 Hz evidence line (`adb logcat -s yacr`).
fn log_status_line(state: &Shared, timings: Option<cad_render_wgpu::ProbeStageTimings>) {
    let stages = timings
        .map(|timings| {
            format!(
                "write={} acquire={} configure={} submit={} scope={} present={} total={}",
                timings.write_buffer_us,
                timings.acquire_us,
                timings.configure_us,
                timings.submit_us,
                timings.scope_pop_us,
                timings.present_us,
                timings.total_us
            )
        })
        .unwrap_or_else(|| "none".to_string());
    log::info!(
        "[probe] status backend={:?} adapter={:?} size={}x{} frames={} ticks={} present_mode={:?} \
         stopped={} paused={} cad_ready={} stages_us[{}] error={:?}",
        state.shell.status.backend,
        state.shell.status.adapter,
        state.shell.status.width,
        state.shell.status.height,
        state.shell.status.frames,
        state.shell.ticks,
        state.shell.present_mode,
        state.shell.status.stopped,
        state.shell.paused,
        state.shell.cad_ready,
        stages,
        state.shell.status.error,
    );
}

/// Log a DOM-rect / platform-surface disagreement once per distinct geometry.
fn log_dom_surface_disagreement(
    telemetry: &Arc<Mutex<RenderTelemetry>>,
    dom_physical: (u32, u32),
    surface: (u32, u32),
) {
    {
        let mut telemetry = lock_telemetry(telemetry);
        if telemetry.last_geometry_notice == Some((dom_physical, surface)) {
            return;
        }
        telemetry.last_geometry_notice = Some((dom_physical, surface));
    }
    log::info!(
        "[probe] geometry dom_physical={}x{} native_surface={}x{} (the native viewport is the \
         surface's own extent; the DOM rect drives input and canvas metrics)",
        dom_physical.0,
        dom_physical.1,
        surface.0,
        surface.1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // The source-structure assertions of this file live in `crate::contract`, which
    // is compiled on the development host (this module is `cfg(target_os =
    // "android")`, so tests here would never run on a gate).

    #[test]
    fn a_cad_disabled_tick_reports_the_shared_unsupported_status() {
        // Android-target only: `unsupported_status` comes from `cad-ui-core`, an
        // Android-only dependency of this crate. The shape is also pinned by
        // `cad-ui-core`'s own tests and the `cad-desktop-shell` wire goldens.
        //
        // `YACR_PROBE_ONLY` (cad_mode = false) has no session; the status the chrome
        // receives is `unsupported_status` with no document and no capabilities.
        let status = unsupported_status(None);
        assert!(status.document_label.is_none());
        assert!(status.entity_count.is_none());
        assert!(!status.can_open && !status.can_new && !status.can_save && !status.can_plot);
        assert!(status.camera.is_none());
    }
}
