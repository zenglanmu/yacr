//! One host tick: main-thread bookkeeping, deferred GPU init, React CAD layout,
//! the CAD session bind/commands and the CAD/probe frame paths.
//!
//! The shared lock is held only for bookkeeping and queue copies; it is never
//! held across GPU or `CadSession` work.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cad_desktop_shell::commands::drain_standard_views;
use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::HostPlatform;
use cad_platform::HostCapabilities;
use cad_ui_core::layout::{surface_metrics, LayoutError};
use cad_ui_core::session::CadSession;
use cad_ui_core::shell::{bind_allowed, clear_host_error, note_host_error, HostErrorKind};
use cad_ui_core::unsupported_status;

use super::{
    lock, lock_telemetry, release_presenter, RenderTelemetry, Shared, ANIMATION_STEP, CAD, CAD_ENV,
    FRAME_INTERVAL, SURFACE,
};
use crate::win32_surface::{BridgeState, CadPresentClass, CadPresentError};

/// Emit the stable, classed present-failure marker (deduplicated).
///
/// One place, so every present failure — layout/placement, reconfigure,
/// composition, render, present — produces the identical greppable line, and the
/// class always comes from the typed [`CadPresentClass`] mapping (never free-text
/// guessing). A failure that repeats on every tick is printed **once** (the text
/// is remembered until the next successful present clears it), so a persistent
/// defect does not flood the evidence log.
fn log_present_error(telemetry: &Arc<Mutex<RenderTelemetry>>, error: &CadPresentError) {
    let text = error.marker_text();
    {
        let mut state = lock_telemetry(telemetry);
        if state.last_present_error_text.as_deref() == Some(text.as_str()) {
            return;
        }
        state.last_present_error_text = Some(text.clone());
    }
    eprintln!(
        "[probe] cad-present-error class={} text={}",
        error.class.as_str(),
        text
    );
}

/// One bind step for the CAD render session (explicit admission table).
enum BindStep {
    /// No CAD child surface exists (deferred, not a failure).
    NoSurface,
    /// Skipped: the surface is closing/failed, or this generation already failed.
    Skipped,
    /// A bind was attempted for `generation` with `result`.
    Attempted(u64, Result<(), String>),
}

/// One host tick. Runs on the main thread.
pub fn tick(shared: &Arc<Mutex<Shared>>, telemetry: &Arc<Mutex<RenderTelemetry>>) {
    // ---- short critical section: bookkeeping only ----
    let (size, marker) = {
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
            release_presenter();
            let mut state = lock(shared);
            state.shell.stopped = true;
            state.shell.status.stopped = true;
            return;
        }

        let now = Instant::now();
        if let Some(last) = telemetry.last_frame {
            if now.duration_since(last) < FRAME_INTERVAL {
                return;
            }
        }
        telemetry.last_frame = Some(now);
        state.shell.ticks += 1;

        // Periodic (~1 Hz) status line for evidence.
        if telemetry
            .last_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            telemetry.last_log = Some(now);
            let stages = telemetry
                .last_timings
                .map(|t| {
                    format!(
                        "write={} acquire={} configure={} submit={} scope={} present={} total={}",
                        t.write_buffer_us,
                        t.acquire_us,
                        t.configure_us,
                        t.submit_us,
                        t.scope_pop_us,
                        t.present_us,
                        t.total_us
                    )
                })
                .unwrap_or_else(|| "none".to_string());
            eprintln!(
                "[probe] status backend={:?} adapter={:?} size={}x{} frames={} ticks={} present_mode={:?} stopped={} marker={:.3} stages_us[{}] error={:?}",
                state.shell.status.backend,
                state.shell.status.adapter,
                state.shell.status.width,
                state.shell.status.height,
                state.shell.status.frames,
                state.shell.ticks,
                state.shell.present_mode,
                state.shell.status.stopped,
                state.shell.input.marker_offset(),
                stages,
                state.shell.status.error,
            );
            // Sibling 1 Hz evidence line: did real CAD rendering happen at all,
            // and is a stale retained texture being re-presented? `frames_rendered`
            // is the engine's own counter, so `frames>` (present attempts) with a
            // flat `frames_rendered` is the "re-present only" signature.
            //
            // The two revisions settle the remaining ambiguity:
            //   rendered_rev  = scene revision baked into the retained texture
            //                   (`None` until a complete render has been presented)
            //   committed_rev = scene revision the GPU has committed (latest)
            // `committed_rev > rendered_rev` with `dirty=false` = the dirty/commit
            // bookkeeping never re-armed a render; `committed_rev == rendered_rev`
            // with a render executing = the pixels are a rasterization/clip issue
            // (a different investigation).
            let (batches, instances, pulled) = telemetry
                .last_actual_render
                .as_ref()
                .map(|actual| {
                    (
                        actual.instanced_batches,
                        actual.instanced_draws,
                        actual.pulled.to_string(),
                    )
                })
                .unwrap_or((0, 0, "-".to_string()));
            let counters = telemetry.last_session_counters.as_ref();
            eprintln!(
                "[probe] actual-render batches={} instances={} pulled={} frames_rendered={} dirty={} rendered_rev={:?} committed_rev={:?}",
                batches,
                instances,
                pulled,
                counters.map(|c| c.frames_rendered).unwrap_or(0),
                counters.map(|c| c.dirty).unwrap_or(false),
                counters.and_then(|c| c.rendered_revision),
                counters.and_then(|c| c.committed_revision),
            );
        }

        let Some(window) = state.window.clone() else {
            return;
        };
        let minimized = window.is_minimized().unwrap_or(false);
        let size = window
            .inner_size()
            .unwrap_or(tauri::PhysicalSize::new(0, 0));

        if minimized || size.width == 0 || size.height == 0 {
            // Zero/occluded surface: pause instead of reconfiguring to zero.
            if !state.shell.paused {
                eprintln!(
                    "[probe] paused (minimized={minimized}) size={}x{}",
                    size.width, size.height
                );
                state.shell.paused = true;
            }
            state.shell.status.width = size.width;
            state.shell.status.height = size.height;
            return;
        }
        if state.shell.paused {
            eprintln!("[probe] resumed size={}x{}", size.width, size.height);
            state.shell.paused = false;
        }
        // Report the live size even when no GPU presenter is active.
        state.shell.status.width = size.width;
        state.shell.status.height = size.height;

        // Input wins over the fixed animation on the frame it arrives.
        let marker = if state.shell.input.take_dirty() {
            state.shell.input.marker_offset()
        } else {
            state.shell.input.advance_animation(ANIMATION_STEP);
            state.shell.input.marker_offset()
        };
        (size, marker)
    };

    // ---- window scale, z-order re-assertion (main thread, no shared lock) ----
    let scale = lock(shared)
        .window
        .as_ref()
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
        .max(0.1);
    // The DOM/CSS viewport is the window client area at the current scale: the
    // child webview fills it (`.auto_resize()`), so it is the reference bounds
    // the LayoutTracker validates the React CAD rect against.
    let container_bounds = (
        (f64::from(size.width) / scale) as f32,
        (f64::from(size.height) / scale) as f32,
    );
    SURFACE.with(|cell| {
        if let Some(surface) = cell.borrow_mut().as_mut() {
            // Idempotent: WebView2 may re-assert `HWND_TOP` on show/DTP change;
            // the CAD child must stay directly below the container.
            surface.ensure_order();
        }
    });

    // ---- deferred GPU init (main thread, no shared lock held) ----
    // `LayoutTracker::confirmed` is the single source of truth for the CAD-rect
    // gate; the surface keeps no second copy.
    let layout_confirmed = lock(shared).layout.confirmed();
    let init_result = SURFACE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|surface| surface.try_init_gpu(layout_confirmed))
    });
    if let Some(result) = init_result {
        let mut state = lock(shared);
        match result {
            Ok(()) => {
                let mut surface_size = None;
                SURFACE.with(|cell| {
                    if let Some(surface) = cell.borrow().as_ref() {
                        if let Some(adapter) = surface.adapter() {
                            state.shell.status.backend = Some(adapter.backend.clone());
                            state.shell.status.adapter = Some(adapter.name.clone());
                        }
                        if let Some(mode) = surface.present_mode() {
                            state.shell.present_mode = Some(format!("{mode:?}"));
                        }
                        surface_size = surface.surface_size();
                    }
                });
                // A new GPU generation: keep the DOM input sequence monotonic and
                // re-validate the cached CAD rect against the current geometry.
                state.layout.note_generation();
                state.layout_revalidate = true;
                if let Some((width, height)) = surface_size {
                    state.shell.status.width = width;
                    state.shell.status.height = height;
                }
            }
            Err(error) => {
                state.shell.status.error = Some(error);
            }
        }
        return;
    }

    // ---- React CAD layout -> native child placement ----
    let last_rect = lock(shared).shell.input.last_rect();
    if let Some(rect) = last_rect {
        let (revalidate, decision) = {
            let mut state = lock(shared);
            // Re-validate the cached rect when a GPU generation changed **or**
            // the container bounds changed (the window resized under an
            // unchanged DOM sequence).
            let revalidate =
                state.layout_revalidate || state.layout_bounds != Some(container_bounds);
            state.layout_revalidate = false;
            let decision = if revalidate {
                state
                    .layout
                    .revalidate(&rect, container_bounds.0, container_bounds.1)
            } else {
                state
                    .layout
                    .accept(&rect, container_bounds.0, container_bounds.1)
            };
            (revalidate, decision)
        };
        match decision {
            Ok(margins) => {
                lock(shared).layout_bounds = Some(container_bounds);
                // Keep the CAD surface source in sync with the accepted DOM rect
                // (the session validates canvas metrics against it).
                CAD_ENV.with(|cell| {
                    if let Some(env) = cell.borrow().as_ref() {
                        env.surface.set_metrics(surface_metrics(&rect, scale));
                    }
                });
                // The child's size comes from the same authority the render
                // session uses (`physical_content_size`), never from scaled
                // insets: see `Win32CadSurface::apply_layout`.
                let layout_result = SURFACE.with(|cell| {
                    cell.borrow_mut()
                        .as_mut()
                        .map(|surface| surface.apply_layout(&rect, scale))
                });
                if let Some(Err(error)) = layout_result {
                    log_present_error(telemetry, &error);
                    lock(shared).shell.status.error =
                        Some(format!("win32 CAD child placement failed: {}", error.text));
                }
                let geometry = SURFACE.with(|cell| {
                    let borrowed = cell.borrow();
                    borrowed.as_ref().map(|surface| {
                        (
                            surface.desired_size(),
                            surface.surface_size(),
                            format!("{:?}", surface.state()),
                        )
                    })
                });
                let (desired, presented, state_label) =
                    geometry.unwrap_or(((0, 0), None, "absent".to_string()));
                // The size invariant, stated as a verdict rather than left to the
                // reader: the child is sized from the same authority the render
                // session uses, and a presented surface must equal it. `FAIL` here
                // means the presenter's size guard will refuse every frame.
                let target = cad_ui_core::layout::physical_content_size(&rect, scale);
                // `apply_layout` reconfigures the swapchain in the same step, so
                // this verdict reflects the post-layout state (no one-tick lag).
                let size_authority =
                    crate::win32_surface::size_authority_verdict(desired, presented, target);
                // Distinguish every geometry explicitly (evidence only; emitted
                // only on a layout change, never per frame): the DOM CAD rect,
                // the margin-receiving client area and the physical child size.
                eprintln!(
                    "[probe] geometry top={}x{} container={:.0}x{:.0} dom={}x{} dom_xy=({},{}) scale={} dom_physical={}x{} native_surface={}x{} child_desired={}x{} size_authority={} margins top={} bottom={} start={} end={} revalidated={} state={}",
                    size.width,
                    size.height,
                    container_bounds.0,
                    container_bounds.1,
                    rect.width,
                    rect.height,
                    rect.x,
                    rect.y,
                    scale,
                    target.0,
                    target.1,
                    presented.map(|s| s.0).unwrap_or(0),
                    presented.map(|s| s.1).unwrap_or(0),
                    desired.0,
                    desired.1,
                    size_authority,
                    margins.top,
                    margins.bottom,
                    margins.start,
                    margins.end,
                    revalidate,
                    state_label,
                );
            }
            Err(LayoutError::StaleSequence) => {}
            Err(error) => {
                if revalidate {
                    // The cached rect no longer fits the current client area:
                    // wait for fresh metrics instead of applying stale margins.
                    eprintln!(
                        "[probe] layout revalidation deferred (waiting for fresh metrics): {error:?}"
                    );
                } else {
                    lock(shared).shell.status.error = Some(format!("layout rejected: {error:?}"));
                }
            }
        }
    }

    // ---- CAD session: lazy creation once the DOM rect is confirmed ----
    let cad_mode = lock(shared).shell.cad_mode;
    if cad_mode && CAD.with(|cell| cell.borrow().is_none()) {
        let rect = lock(shared).shell.input.last_rect();
        if let Some(rect) = rect {
            let created = CAD_ENV.with(|cell| {
                let env = cell.borrow();
                let env = env.as_ref()?;
                // The surface must report before `CadSession::new` (the open
                // funnel validates canvas metrics against it).
                env.surface.set_metrics(surface_metrics(&rect, scale));
                let platform = HostPlatform {
                    files: env.files.clone(),
                    config_store: env.config.clone(),
                    surface: std::rc::Rc::new(env.surface.clone()),
                    resources: std::rc::Rc::new(cad_host::DesktopResources),
                    capabilities: HostCapabilities {
                        open: true,
                        new: false,
                        save: false,
                        plot: false,
                        import_worker: true,
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
                    CAD.with(|cell| *cell.borrow_mut() = Some(session));
                    eprintln!("[probe] cad session created");
                }
                Some(Err(error)) => {
                    let text = format!("{error:?}");
                    eprintln!("[probe] cad session creation FAILED: {text}");
                    // A CadSession-level (core/open) failure lives in the
                    // snapshot, never in the host transient error.
                    lock(shared).shell.cad_status = Some(unsupported_status(Some(text)));
                }
                None => {}
            }
        }
    }

    // ---- CAD session bind (main thread): only a successful bind marks ready ----
    //
    // A bind is attempted **at most once per presenter generation**: a failure is
    // recorded against the generation, kept visible, and the session stays
    // not-ready (waiting) until the generation changes — no per-tick loop.
    if cad_mode && !lock(shared).shell.cad_ready {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).shell.cad_bind_failed_generation;
            let step = SURFACE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(surface) = slot.as_mut() else {
                    return BindStep::NoSurface;
                };
                if surface.failure().is_some() || matches!(surface.state(), BridgeState::Closing) {
                    return BindStep::Skipped;
                }
                let generation = surface.generation();
                if !bind_allowed(false, failed_generation, generation) {
                    return BindStep::Skipped;
                }
                BindStep::Attempted(generation, surface.set_render_session(Some(render)))
            });
            match step {
                BindStep::Attempted(generation, Ok(())) => {
                    let mut state = lock(shared);
                    state.shell.cad_ready = true;
                    state.shell.cad_bind_failed_generation = None;
                    clear_host_error(&mut state.shell, HostErrorKind::Bind);
                    eprintln!("[probe] cad render session bound (generation {generation})");
                }
                BindStep::Attempted(generation, Err(error)) => {
                    let mut state = lock(shared);
                    state.shell.cad_bind_failed_generation = Some(generation);
                    note_host_error(
                        &mut state.shell,
                        HostErrorKind::Bind,
                        format!("cad bind failed: {error}"),
                    );
                    eprintln!(
                        "[probe] cad bind FAILED (generation {generation}); fail-stop until a new generation: {error}"
                    );
                }
                BindStep::NoSurface | BindStep::Skipped => {}
            }
        }
    }

    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    if cad_mode {
        let (
            commands,
            mut standard_views,
            setting_writes,
            layer_writes,
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
        let inputs_started = Instant::now();
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    // Settings writes apply first: an Options change in this
                    // batch is visible to the commands/inputs that follow.
                    for (key, value) in setting_writes {
                        if let Err(error) = session.apply_setting(key, value) {
                            eprintln!("[probe] cad setting rejected: {error}");
                        }
                    }
                    // Typed layer writes (the React layer panel's three
                    // mutations): the session resolves each name against the
                    // drawing's own layer table and dispatches the ordinary layer
                    // command; a refusal is the session's real error.
                    super::layers::apply_writes(session, layer_writes);
                    for command in &commands {
                        if let Err(error) = session.command(command) {
                            eprintln!("[probe] cad command rejected: {error}");
                        }
                    }
                    // Typed standard-view requests: each one dispatches the real
                    // `CommandId::StandardView` through the session funnel.
                    for error in drain_standard_views(&mut standard_views, |view| {
                        session.standard_view(view)
                    }) {
                        eprintln!("[probe] cad standard view rejected: {error}");
                    }
                    // Typed tool-parameter requests: applied before this batch's
                    // inputs so a stepper change takes effect in the same batch.
                    for (parameter, value) in &tool_parameters {
                        if let Err(error) = session.set_tool_parameter(*parameter, *value) {
                            eprintln!("[probe] cad tool parameter rejected: {error}");
                        }
                    }
                    for input in &inputs {
                        if let Err(error) = session.handle_input(input) {
                            eprintln!("[probe] cad input rejected: {error}");
                        }
                    }
                    // Terminal tool texts: applied after this batch's inputs (the
                    // last point of a text tool may arrive in the same tick).
                    for text in &tool_texts {
                        if let Err(error) = session.set_tool_text(text) {
                            eprintln!("[probe] cad tool text rejected: {error}");
                        }
                    }
                    // Panel text edits target the session's **current** selection,
                    // resolved on the owner thread at apply time.
                    for text in &selected_texts {
                        if let Err(error) = session.set_selected_text(text) {
                            eprintln!("[probe] cad selected text rejected: {error}");
                        }
                    }
                    // Panel property writes (the docked palette's
                    // `SETENTITYPROPERTY` channel), resolved at apply time.
                    for (key, value) in &selected_properties {
                        if let Err(error) = session.set_selected_property(key, value) {
                            eprintln!("[probe] cad selected property rejected: {error}");
                        }
                    }
                }
            });
        }
        let inputs_us = inputs_started.elapsed().as_micros();
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        let tick_started = Instant::now();
        let tick = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.tick()));
        let tick_us = tick_started.elapsed().as_micros();
        match tick {
            Some(Ok(())) => clear_host_error(&mut lock(shared).shell, HostErrorKind::Tick),
            Some(Err(error)) => note_host_error(
                &mut lock(shared).shell,
                HostErrorKind::Tick,
                format!("cad tick failed: {error}"),
            ),
            None => {}
        }
        if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() && queued > 0 {
            eprintln!("[probe] cad-stage queued={queued} input_us={inputs_us} tick_us={tick_us}");
        }
    }

    let surface_size = SURFACE.with(|cell| cell.borrow().as_ref().and_then(|s| s.surface_size()));

    if cad_mode {
        // ---- CAD frame (main thread, no shared lock held across GPU work) ----
        let (logical, rect_scale) = {
            let rect = lock(shared).shell.input.last_rect();
            (
                rect.map(|rect| [f64::from(rect.width), f64::from(rect.height)])
                    .unwrap_or([0.0, 0.0]),
                scale,
            )
        };
        let frame_result: Option<
            Result<Option<(u64, cad_render_wgpu::ProbeStageTimings)>, CadPresentError>,
        > = CAD.with(|cell| {
            let borrowed = cell.borrow();
            let session = borrowed.as_ref()?;
            Some(SURFACE.with(|bridge| {
                let mut slot = bridge.borrow_mut();
                let Some(surface) = slot.as_mut() else {
                    return Ok(None);
                };
                // Explicit lifecycle admission: only a `Ready` surface presents.
                // Waiting states produce no present attempt and no error; a
                // terminal state is never presented on and keeps its diagnostic.
                match surface.state() {
                    BridgeState::AwaitingMap | BridgeState::Suspended => return Ok(None),
                    BridgeState::Ready => {}
                    BridgeState::Closing | BridgeState::Failed(_) => return Ok(None),
                }
                let (width, height) = surface.desired_size();
                if width == 0 || height == 0 {
                    return Ok(None);
                }
                // Idempotent backstop: `apply_layout` already reconfigured the
                // swapchain in the same tick, so this is a no-op unless a layout
                // never reached the surface. Either way the surface must be at
                // the authority size before a frame is drawn into it.
                surface.resize_to(width, height)?;
                if let Some(applied) = surface.surface_size() {
                    if applied != (width, height) {
                        return Err(CadPresentError::new(
                            CadPresentClass::SurfaceReconfigure,
                            format!(
                                "surface did not take the authority size {width}x{height} (still {}x{})",
                                applied.0, applied.1
                            ),
                        ));
                    }
                }
                match surface.frame_cad(logical, rect_scale) {
                    Ok(Some(stats)) => {
                        session.note_frame(stats.frame_index);
                        Ok(Some((stats.frame_index, stats.timings)))
                    }
                    // No new frame and no reusable retained frame: explicit
                    // waiting; no blank/fake grid is presented.
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                }
            }))
        });
        let (backend, adapter) = SURFACE.with(|cell| {
            let borrowed = cell.borrow();
            borrowed
                .as_ref()
                .and_then(|surface| surface.adapter())
                .map(|info| (Some(info.backend.clone()), Some(info.name.clone())))
                .unwrap_or((None, None))
        });
        match &frame_result {
            Some(Ok(Some((frames, timings)))) => {
                // A good present clears the marker latch so a later recurrence
                // is reported again.
                lock_telemetry(telemetry).last_present_error_text = None;
                let mut state = lock(shared);
                state.shell.status.frames = *frames;
                state.shell.status.error = None;
                // A good present clears only a previous present error (never a
                // bind error, and never the CadSession business error).
                clear_host_error(&mut state.shell, HostErrorKind::Present);
                // The native presenter's real CAD stage timings make the periodic
                // `status … stages_us[…]` line meaningful in CAD mode.
                lock_telemetry(telemetry).last_timings = Some(*timings);
            }
            // Explicit waiting: no present happened, so nothing is cleared.
            Some(Ok(None)) => {}
            Some(Err(error)) => {
                // Evidence-first: a stable marker BEFORE the status note, with a
                // class derived from the typed ProbeError/RenderError
                // discriminants (never from the text), so a run can group the
                // failure mechanically and a sticky error is self-describing.
                log_present_error(telemetry, error);
                let mut state = lock(shared);
                note_host_error(
                    &mut state.shell,
                    HostErrorKind::Present,
                    format!("cad present failed: {error}"),
                );
            }
            None => {}
        }
        // Observable state of the accelerated pulled organisation (evidence for
        // `cad_status`); consumed once per real renderer invocation only.
        let actual = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|session| session.render_session().take_render_stats_observation())
        });
        if let Some(actual) = actual {
            // Per-render marker: emitted only when the renderer really executed
            // (a retained re-present never produces an observation), so a frozen
            // `frames_rendered` is directly comparable to these lines. `rev` is
            // the scene revision committed at render time, `size` the render
            // target (the size authority), `batches`/`instances` what was drawn.
            let render_size = SURFACE.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|surface| surface.desired_size())
                    .unwrap_or((0, 0))
            });
            eprintln!(
                "[probe] render-exec rev={:?} size={}x{} batches={} instances={}",
                actual.applied_revision,
                render_size.0,
                render_size.1,
                actual.instanced_batches,
                actual.instanced_draws,
            );
            let mut telemetry = lock_telemetry(telemetry);
            telemetry.last_pulled_state = Some(actual.pulled_state.clone());
            // Kept for the 1 Hz `[probe] actual-render` line: what the renderer
            // really drew (never replayed — the observation is taken once).
            telemetry.last_actual_render = Some(crate::host::ActualRenderSnapshot {
                instanced_batches: actual.instanced_batches,
                instanced_draws: actual.instanced_draws,
                pulled: actual.pulled_state,
            });
        }
        // Session counters sampled every CAD tick (cheap bookkeeping reads, no
        // render): they show a stalled engine (frames_rendered flat) even while a
        // stale retained texture is re-presented, and they separate the retained
        // texture's revision (`rendered_rev`) from the scene's latest commit.
        let counters = CAD.with(|cell| {
            cell.borrow().as_ref().map(|session| {
                let render = session.render_session();
                crate::host::SessionCounters {
                    frames_rendered: render.frames_rendered(),
                    dirty: render.cad_frame_dirty(),
                    rendered_revision: render.rendered_revision(),
                    committed_revision: render.committed_revision(),
                }
            })
        });
        if let Some(counters) = counters {
            lock_telemetry(telemetry).last_session_counters = Some(counters);
        }
        let snapshot = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|session| session.snapshot(backend, adapter, 0))
        });
        let settings = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.settings()));
        {
            let mut state = lock(shared);
            if let Some(snapshot) = &snapshot {
                state.shell.cad_status = Some(snapshot.clone());
            }
            if let Some(settings) = settings {
                state.cad_settings = Some(settings);
            }
        }
        // The layer panel's own mirror (see `super::layers`).
        super::layers::mirror(shared);
    } else {
        // ---- probe frame (marker path; `--probe` mode only) ----
        let outcome = SURFACE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let surface = slot.as_mut()?;
            if surface.failure().is_some() {
                return None;
            }
            let (width, height) = surface.desired_size();
            if width == 0 || height == 0 {
                return None;
            }
            if let Err(error) = surface.resize_to(width, height) {
                // The marker path has no present-error class vocabulary; the
                // text still surfaces the reconfigure failure verbatim.
                return Some(Err(error.text));
            }
            surface.frame(marker)
        });
        if let Some(outcome) = outcome {
            let mut state = lock(shared);
            match outcome {
                Ok((frames, timings)) => {
                    state.shell.status.frames = frames;
                    // Report the actual child-surface size when the surface
                    // exists, otherwise the window size (top-level).
                    if let Some((width, height)) = surface_size {
                        state.shell.status.width = width;
                        state.shell.status.height = height;
                    } else {
                        state.shell.status.width = size.width;
                        state.shell.status.height = size.height;
                    }
                    lock_telemetry(telemetry).last_timings = Some(timings);
                    state.shell.status.error = None;
                }
                Err(error) => {
                    state.shell.status.error = Some(error);
                }
            }
        }
    }
}
