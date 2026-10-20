//! One probe tick: main-thread bookkeeping, deferred GPU init, React CAD layout,
//! the CAD session bind/commands and the CAD/probe frame paths.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::HostPlatform;
use cad_platform::HostCapabilities;
use cad_render_wgpu::ProbeStageTimings;
use cad_ui_core::layout::{surface_metrics, LayoutError};
use cad_ui_core::session::CadSession;
use cad_ui_core::shell::{bind_allowed, clear_host_error, note_host_error, HostErrorKind};
use cad_ui_core::unsupported_status;

use super::admission::{
    cad_present_admission, layout_needs_revalidate, BindStep, CadPresentAdmission,
};
use super::dom::run_dom_diag;
use super::options::{ANIMATION_STEP, FRAME_INTERVAL};
use super::postload::{drive_postload_pan, ActualRenderSummary};
use super::presenter::release_presenter;
use super::telemetry::{
    cad_frame_log_state, frame_completeness_label, log_render_diagnostics, opt_telemetry_u128,
    opt_telemetry_u64, CadFrameLogState, CAD_STAGE_SLOW_US,
};
use super::{lock, Shared, BRIDGE, CAD, CAD_ENV};

/// One probe tick. Runs on the main thread. The shared lock is held only for
/// bookkeeping, never across GPU work.
pub fn tick(shared: &Arc<Mutex<Shared>>) {
    // ---- short critical section: bookkeeping only ----
    let (size, marker) = {
        let mut state = lock(shared);
        state.wake_pending.store(false, Ordering::SeqCst);

        if state.shell.stopped {
            return;
        }
        if state.shell.stop_requested {
            drop(state);
            release_presenter();
            let mut state = lock(shared);
            state.shell.stopped = true;
            state.shell.status.stopped = true;
            return;
        }

        let now = Instant::now();
        if let Some(last) = state.last_frame {
            if now.duration_since(last) < FRAME_INTERVAL {
                return;
            }
        }
        state.last_frame = Some(now);
        state.shell.ticks += 1;

        // Periodic (~1 Hz) status line for evidence; includes the last frame's
        // per-stage timings and the frame index.
        if state
            .last_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            state.last_log = Some(now);
            let stages = state
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
        }

        // Bounded DOM diagnostics at ~6s and ~14s (2 runs max); they only
        // dispatch an eval, so they never block the loop.
        if state.diag_runs < 2 {
            let due = Duration::from_secs(6 + 8 * u64::from(state.diag_runs));
            if state.diag_start.elapsed() >= due {
                state.diag_runs += 1;
                let run = state.diag_runs;
                drop(state);
                run_dom_diag(run);
                return;
            }
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

    // ---- deferred bridge GPU init (main thread, no shared lock held) ----
    // `LayoutTracker::confirmed` is the single source of truth for the CAD-rect
    // gate; the bridge keeps no second copy.
    let layout_confirmed = lock(shared).layout.confirmed();
    let init_result = BRIDGE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|owner| owner.try_init_gpu(layout_confirmed))
    });
    if let Some(result) = init_result {
        let mut state = lock(shared);
        match result {
            Ok(()) => {
                let mut surface = None;
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow().as_ref() {
                        if let Some(adapter) = owner.adapter() {
                            state.shell.status.backend = Some(adapter.backend.clone());
                            state.shell.status.adapter = Some(adapter.name.clone());
                        }
                        if let Some(mode) = owner.present_mode() {
                            state.shell.present_mode = Some(format!("{mode:?}"));
                        }
                        surface = owner.surface_size();
                    }
                });
                // A new GPU generation: keep the DOM input sequence monotonic and
                // re-validate the cached CAD rect against the current top geometry
                // (do not blindly re-apply it; there is no cross-generation reset).
                state.layout.note_generation();
                state.layout_revalidate = true;
                // Report the actual child-surface size, not the window size.
                if let Some((w, h)) = surface {
                    state.shell.status.width = w;
                    state.shell.status.height = h;
                }
            }
            Err(error) => {
                state.shell.status.error = Some(error);
            }
        }
        return;
    }

    // ---- React CAD layout -> native margins (main thread, no shared lock) ----
    // The reference bounds for the CAD viewport insets are the **logical size of
    // the GTK container that receives the margins** (the overlay the React WebView
    // fills), in the frontend's DOM/CSS coordinate space. The Tauri window
    // `inner_size()` is WRONG here: it includes CSD/decorations and is larger than
    // the DOM viewport, so using it inflated every inset (native capture A:
    // 1012x652 vs the real 960x553 viewport → margins end=86/bottom=183 instead of
    // the correct end=34/bottom=84).
    let container_bounds = BRIDGE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|owner| owner.logical_container_bounds())
    });
    // Defer explicitly while the container allocation is not settled: never
    // accept a fake empty layout (zero bounds) and never disturb a confirmed one.
    let last_rect = lock(shared).shell.input.last_rect();
    if let (Some(bounds), Some(rect)) = (container_bounds, last_rect) {
        let (revalidate, decision) = {
            let mut state = lock(shared);
            // Re-validate the cached rect when a GPU generation changed **or** the
            // margin-receiving container bounds changed — the latter even without a
            // new DOM sequence (the container resized under an unchanged rect).
            let revalidate =
                layout_needs_revalidate(state.layout_revalidate, state.layout_bounds, bounds);
            state.layout_revalidate = false;
            let decision = if revalidate {
                state.layout.revalidate(&rect, bounds.0, bounds.1)
            } else {
                state.layout.accept(&rect, bounds.0, bounds.1)
            };
            (revalidate, decision)
        };
        match decision {
            Ok(margins) => {
                lock(shared).layout_bounds = Some(bounds);
                // Keep the CAD surface source in sync with the accepted DOM
                // rect. The host `tick` re-reads this surface every cycle
                // (`HostCore::tick` -> `metrics`), so `viewport.logical_size`
                // always matches the rect the renderer draws into. Leaving it
                // at the creation-time size made `Camera::zoom_at` anchor off
                // the cursor after any DOM-rect change (window resize, panel
                // toggle, DPR change).
                let scale = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.scale_factor().ok())
                    .unwrap_or(1.0);
                CAD_ENV.with(|cell| {
                    if let Some(env) = cell.borrow().as_ref() {
                        env.surface.set_metrics(surface_metrics(&rect, scale));
                    }
                });
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow_mut().as_mut() {
                        owner.apply_layout(margins);
                    }
                });
                let geometry =
                    BRIDGE.with(|cell| cell.borrow().as_ref().map(|owner| owner.layout_geometry()));
                let surface =
                    BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
                let top = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.inner_size().ok());
                // Distinguish every geometry explicitly (evidence only; emitted
                // only on a layout change, never per frame): the DOM CAD rect,
                // the margin-receiving container, the drawing area + its
                // translated origin, the integer scale and the physical extent.
                eprintln!(
                    "[probe] geometry top={}x{} container={:.0}x{:.0} container_xy=({},{}) dom={}x{} dom_xy=({},{}) drawing_area={}x{} origin=({},{}) scale={} native_surface={}x{} margins top={} bottom={} start={} end={} revalidated={}",
                    top.map(|s| s.width).unwrap_or(0),
                    top.map(|s| s.height).unwrap_or(0),
                    bounds.0,
                    bounds.1,
                    geometry
                        .and_then(|g| g.container_origin)
                        .map(|o| o.0)
                        .unwrap_or(-1),
                    geometry
                        .and_then(|g| g.container_origin)
                        .map(|o| o.1)
                        .unwrap_or(-1),
                    rect.width,
                    rect.height,
                    rect.x,
                    rect.y,
                    geometry.map(|g| g.drawing_area.0).unwrap_or(0),
                    geometry.map(|g| g.drawing_area.1).unwrap_or(0),
                    geometry.and_then(|g| g.origin).map(|o| o.0).unwrap_or(-1),
                    geometry.and_then(|g| g.origin).map(|o| o.1).unwrap_or(-1),
                    geometry.map(|g| g.scale).unwrap_or(0),
                    surface.map(|s| s.0).unwrap_or(0),
                    surface.map(|s| s.1).unwrap_or(0),
                    margins.top,
                    margins.bottom,
                    margins.start,
                    margins.end,
                    revalidate,
                );
            }
            Err(LayoutError::StaleSequence) => {}
            Err(error) => {
                if revalidate {
                    // The cached rect no longer fits the current container: wait
                    // for fresh metrics instead of applying stale margins.
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
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
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
    // The device is attached through the presenter's `device_queue`. With no
    // presenter yet this stores the session and returns `Ok` (device deferred,
    // not a failure). A bind is attempted **at most once per presenter
    // generation**: a failure is recorded against the generation, kept visible,
    // and the session stays not-ready (waiting) until the generation changes (a
    // new device) — no per-tick initialization loop. A bridge that is not
    // installed yet, or is closing/failed, defers without a failure.
    if cad_mode && !lock(shared).shell.cad_ready {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).shell.cad_bind_failed_generation;
            let step = BRIDGE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return BindStep::NoBridge;
                };
                if owner.failure().is_some()
                    || matches!(owner.state(), crate::gtk_bridge::BridgeState::Closing)
                {
                    return BindStep::Skipped;
                }
                let generation = owner.generation();
                if !bind_allowed(false, failed_generation, generation) {
                    return BindStep::Skipped;
                }
                BindStep::Attempted(generation, owner.set_render_session(Some(render)))
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
                BindStep::NoBridge | BindStep::Skipped => {}
            }
        }
    }

    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    let cad_log = std::env::var_os("YACR_PROBE_CAD_LOG").is_some();
    // Telemetry: the latest input sequence in this batch and its IPC-to-drain
    // age (measured from the Rust `Instant` recorded at admission).
    let mut last_input_seq: Option<u64> = None;
    let mut input_age_us: Option<u128> = None;
    if cad_mode {
        let (commands, tool_parameters, tool_texts, selected_texts, inputs) = {
            let mut state = lock(shared);
            (
                std::mem::take(&mut state.shell.cad_commands),
                std::mem::take(&mut state.cad_tool_parameters),
                std::mem::take(&mut state.cad_tool_texts),
                std::mem::take(&mut state.cad_selected_texts),
                std::mem::take(&mut state.shell.cad_inputs),
            )
        };
        let queued = commands.len()
            + tool_parameters.len()
            + tool_texts.len()
            + selected_texts.len()
            + inputs.len();
        last_input_seq = inputs.last().map(|input| input.sequence);
        input_age_us = last_input_seq.and_then(|sequence| {
            let state = lock(shared);
            state
                .shell
                .cad_input_recv
                .iter()
                .rev()
                .find(|(seq, _)| *seq == sequence)
                .map(|(_, received)| received.elapsed().as_micros())
        });
        let inputs_started = Instant::now();
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    for command in &commands {
                        // Admission already returned `{accepted:true}`. The real
                        // business outcome is the CadSession snapshot error; it
                        // is not duplicated into the host transient error.
                        if let Err(error) = session.command(command) {
                            eprintln!("[probe] cad command rejected: {error}");
                        }
                    }
                    // Typed tool-parameter requests: routed through the one
                    // session path, applied before this batch's inputs so a
                    // stepper change takes effect on the same batch. A refusal
                    // (out of range / wrong kind / Viewer mode) is the session's
                    // real error, surfaced through `cad_status.error`.
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
                    // Terminal tool texts (ADR 0019 decision A.1): applied after
                    // this batch's inputs, because the last point of a text tool
                    // may arrive in the same tick and the text is only valid once
                    // the point complement is complete. The text completes the
                    // command on its own (no second confirm); a refusal is named
                    // and surfaces through `cad_status.error`.
                    for text in &tool_texts {
                        if let Err(error) = session.set_tool_text(text) {
                            eprintln!("[probe] cad tool text rejected: {error}");
                        }
                    }
                    // Panel text edits (ADR 0020 amendment B): each one targets
                    // the session's **current** selection, resolved on the owner
                    // thread at apply time (no id travelled). A refusal is named
                    // and surfaces through `cad_status.error`.
                    for text in &selected_texts {
                        if let Err(error) = session.set_selected_text(text) {
                            eprintln!("[probe] cad selected text rejected: {error}");
                        }
                    }
                }
            });
        }
        let inputs_us = inputs_started.elapsed().as_micros();
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        // A tick failure is a host transient error, cleared only by a later
        // successful tick.
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
        if cad_log && queued > 0 {
            eprintln!("[probe] cad-stage queued={queued} input_us={inputs_us} tick_us={tick_us}");
        }
    }

    // ---- GPU work, without holding the shared lock ----
    let surface = BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
    if cad_mode {
        let (logical, scale, dom_rect) = {
            let rect = lock(shared).shell.input.last_rect();
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
            (
                rect.map(|rect| [f64::from(rect.width), f64::from(rect.height)])
                    .unwrap_or([0.0, 0.0]),
                scale,
                rect.map(|rect| (rect.x, rect.y, rect.width, rect.height)),
            )
        };
        // Draw one CAD frame through the bound render session (main thread, no
        // shared lock held across the engine/GPU work).
        let frame_started = Instant::now();
        let frame_result: Option<Result<Option<u64>, String>> = CAD.with(|cell| {
            let borrowed = cell.borrow();
            let session = borrowed.as_ref()?;
            Some(BRIDGE.with(|bridge| {
                let mut slot = bridge.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return Ok(None);
                };
                // Explicit bridge-state admission: only a `Ready` bridge presents.
                // `AwaitingMap`/`Suspended` (globals/presenter still coming up, or
                // suspended for re-creation) wait — no present attempt, no error;
                // `Closing`/`Failed` are terminal and are never presented on, and
                // their diagnostics are preserved. This removes the spurious
                // `Present` error the former unconditional `frame_cad` call
                // produced while the bridge was still coming up.
                match cad_present_admission(owner.state()) {
                    CadPresentAdmission::Waiting | CadPresentAdmission::Terminal => {
                        return Ok(None);
                    }
                    CadPresentAdmission::Ready => {}
                }
                // `Ready` is only set after a successful presenter init, so this is
                // a genuine error, never a wait (no global "no presenter" success).
                if owner.failure().is_some() {
                    return Ok(None);
                }
                // The native surface must occupy exactly the React DOM CAD rect:
                // the blit is 1:1 and pointer input maps through the DOM rect, so
                // a size or origin disagreement stretches the image or offsets the
                // pointer. Drive the layout once, then refuse to render through a
                // residual disagreement — it is a bug, not a state.
                if let Some(dom) = dom_rect {
                    if !owner.drawing_area_matches_dom(dom) {
                        owner.drive_drawing_area_layout();
                    }
                    if !owner.drawing_area_matches_dom(dom) {
                        return Err(format!(
                            "native drawing area origin={:?} size={}x{} does not match the DOM CAD rect {:?}",
                            owner.drawing_area_origin(),
                            owner.drawing_area_allocation().2,
                            owner.drawing_area_allocation().3,
                            dom
                        ));
                    }
                }
                let (w, h) = owner.desired_size();
                if w == 0 || h == 0 {
                    return Ok(None);
                }
                owner.resize_to(w, h)?;
                match owner.frame_cad(logical, scale) {
                    Ok(Some(stats)) => {
                        session.note_frame(stats.frame_index);
                        Ok(Some(stats.frame_index))
                    }
                    // No new frame and no reusable retained frame: explicit
                    // waiting; no blank/fake grid is presented.
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                }
            }))
        });
        let (backend, adapter) = BRIDGE.with(|cell| {
            let bridge = cell.borrow();
            bridge
                .as_ref()
                .and_then(|owner| owner.adapter())
                .map(|info| (Some(info.backend.clone()), Some(info.name.clone())))
                .unwrap_or((None, None))
        });
        match &frame_result {
            Some(Ok(Some(frames))) => {
                let mut state = lock(shared);
                state.shell.status.frames = *frames;
                state.shell.status.error = None;
                // A good present clears only a previous present error (never a
                // bind error, and never the CadSession business error).
                clear_host_error(&mut state.shell, HostErrorKind::Present);
            }
            // Explicit waiting: no present happened, so nothing is cleared.
            Some(Ok(None)) => {}
            Some(Err(error)) => {
                let mut state = lock(shared);
                note_host_error(
                    &mut state.shell,
                    HostErrorKind::Present,
                    format!("cad present failed: {error}"),
                );
            }
            None => {}
        }
        let frame_us = frame_started.elapsed().as_micros();
        // Engine render-gate diagnostics (diagnostics only; env-gated by
        // `YACR_PROBE_CAD_LOG`). Emitted on a state change or a ~1 s heartbeat, so
        // a waiting/upload stall can be diagnosed without per-frame log spam.
        if cad_log {
            let diagnostics = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|session| session.render_session().diagnostics())
            });
            if let Some(diagnostics) = diagnostics {
                log_render_diagnostics(shared, &diagnostics);
            }
        }
        // Actual-render observation: taken **once** per real renderer invocation
        // (a retained/waiting frame can never replay stale stats). Always recorded
        // for the post-load driver's admission; logged only under CAD_LOG.
        let actual = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|session| session.render_session().take_render_stats_observation())
        });
        if let Some(actual) = actual {
            if cad_log {
                eprintln!(
                    "[probe] actual-render seq={} device_epoch={} view_rev={} applied_rev={:?} applied_base={:?} cpu_ms={:.3} gpu_ms={:?} plan_ms={:.3} uniform_ms={:.3} encode_ms={:.3} submit_ms={:.3} wait_ms={:.3} draw_calls={} verts={} tris={} instanced={} mesh={} legacy_opaque={} arc={} transparent={} images={} accelerated={} instance_streams={} instanced_batches={} tier={} over_budget={} prog={:?}/{:?} complete={} uploads_started={} rev_resets={} ready_none={} pulled={} budget_scale={:.3} budget_target_ms={:.2} budget_cost_ema_ms={:.2} budget_verts={} budget_tris={} pacing={} cull_chunks={}/{} gpu_time={}",
                    actual.render_seq,
                    actual.device_epoch,
                    actual.view_revision,
                    actual.applied_revision,
                    actual.applied_base,
                    actual.cpu_ms,
                    actual.gpu_ms,
                    actual.plan_ms,
                    actual.uniform_ms,
                    actual.encode_ms,
                    actual.submit_ms,
                    actual.wait_ms,
                    actual.draw_calls,
                    actual.vertices,
                    actual.triangles,
                    actual.instanced_draws,
                    actual.mesh_draws,
                    actual.legacy_opaque_draws,
                    actual.arc_draws,
                    actual.transparent_batches,
                    actual.image_batches,
                    actual.accelerated,
                    actual.instance_streams,
                    actual.instanced_batches,
                    actual.tier,
                    actual.over_budget,
                    actual.progressive_cursor,
                    actual.progressive_total,
                    actual.progressive_complete,
                    actual.uploads_started,
                    actual.upload_revision_resets,
                    actual.ready_none_clears,
                    actual.pulled_state,
                    actual.budget_scale,
                    actual.budget_target_ms,
                    actual.budget_cost_ema_ms,
                    actual.budget_vertices,
                    actual.budget_triangles,
                    actual.pacing,
                    actual.cull_chunks_kept,
                    actual.cull_chunks_total,
                    actual.gpu_time.as_token(),
                );
            }
            lock(shared).last_actual_render = Some(ActualRenderSummary {
                render_seq: actual.render_seq,
                view_revision: actual.view_revision,
                device_epoch: actual.device_epoch,
            });
            lock(shared).last_pulled_state = Some(actual.pulled_state.clone());
        }
        let snapshot = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|session| session.snapshot(backend, adapter, 0))
        });
        if let Some(snapshot) = &snapshot {
            lock(shared).shell.cad_status = Some(snapshot.clone());
        }
        // Bounded CAD stage telemetry (diagnostics only): ONE aggregated line
        // when an upload work unit staged/committed, or the host frame was slow
        // (> 50 ms). Both observations are taken (consumed) at most once, so a
        // stale value is never replayed; no per-batch logging.
        if cad_log {
            let stages = BRIDGE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|owner| owner.take_frame_cad_stages())
            });
            let upload = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|session| session.render_session().take_upload_timing_observation())
            });
            let frame_total = stages
                .map(|stages| stages.apply_us + stages.render_us + stages.present_us)
                .unwrap_or(0);
            if upload.is_some() || frame_total >= CAD_STAGE_SLOW_US {
                let (apply_us, render_us, present_us, presented) = stages
                    .map(|stages| {
                        (
                            stages.apply_us,
                            stages.render_us,
                            stages.present_us,
                            stages.presented,
                        )
                    })
                    .unwrap_or((0, 0, 0, false));
                let upload_text = match &upload {
                    Some(upload) => {
                        let commit = match &upload.commit {
                            Some(commit) => format!(
                                "rebase={} append={} gc={} pulled={} pick={} lineweight={} total={}",
                                commit.rebase_us,
                                commit.append_us,
                                commit.gc_us,
                                commit.pulled_us,
                                commit.pick_us,
                                commit.lineweight_us,
                                commit.total_us
                            ),
                            None => "-".to_string(),
                        };
                        format!(
                            "seq={} scene_rev={} base_changed={} committed={} group={} batches={} prepare_us={} append_us={} commit_us={} total_us={} commit[{}]",
                            upload.seq,
                            upload.scene_revision,
                            upload.base_changed,
                            upload.committed,
                            upload.group,
                            upload.batches,
                            upload.prepare_us,
                            upload.append_us,
                            upload.commit_us,
                            upload.total_us,
                            commit
                        )
                    }
                    None => "-".to_string(),
                };
                eprintln!(
                    "[probe] cad-stages apply_us={apply_us} render_us={render_us} present_us={present_us} presented={presented} frame_total_us={frame_total} upload={upload_text}"
                );
            }
        }
        // Env-gated, default-OFF bounded post-load pan evidence driver (runs after
        // the session status so it can gate on the imported document identity).
        drive_postload_pan(shared);
        // Consume the presented-frame observation exactly once, and only for a
        // real present (never for a waiting/error frame), so the telemetry can
        // never replay a stale frame.
        let observation = if matches!(&frame_result, Some(Ok(Some(_)))) {
            BRIDGE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|owner| owner.take_last_cad_frame_observation())
            })
        } else {
            None
        };
        if let Some(observation) = observation {
            // The native presenter's real CAD stage timings (this is what makes
            // the periodic `status … stages_us[…]` meaningful in CAD mode).
            lock(shared).last_timings = Some(observation.timings);
        }
        if cad_log {
            let view_current = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|session| session.render_session().view_revision())
            });
            let state_tag = cad_frame_log_state(&frame_result);
            if let (Some(obs), CadFrameLogState::Presented) = (observation, state_tag) {
                let camera = snapshot.as_ref().and_then(|status| status.camera);
                let (cx, cy, cscale) = camera
                    .map(|c| (c.center[0], c.center[1], c.scale))
                    .unwrap_or((f64::NAN, f64::NAN, f64::NAN));
                let t = obs.timings;
                eprintln!(
                    "[probe] cad-frame presented view_current={} view_rendered={} complete={} device_epoch={} texture_rev={} size={}x{} frame_id={} frame_us={} input_seq={} input_age_us={} camera_center={cx:.3},{cy:.3} camera_scale={cscale:.6} stages_us[write={} acquire={} configure={} submit={} scope={} present={} total={}]",
                    view_current.unwrap_or(0),
                    obs.rendered_view_revision,
                    frame_completeness_label(obs.completeness),
                    obs.device_epoch,
                    obs.texture_revision,
                    obs.size.0,
                    obs.size.1,
                    obs.frame_id,
                    frame_us,
                    opt_telemetry_u64(last_input_seq),
                    opt_telemetry_u128(input_age_us),
                    t.write_buffer_us,
                    t.acquire_us,
                    t.configure_us,
                    t.submit_us,
                    t.scope_pop_us,
                    t.present_us,
                    t.total_us,
                );
            } else {
                eprintln!(
                    "[probe] cad-frame {} frame_us={} view_current={}",
                    state_tag.label(),
                    frame_us,
                    view_current.unwrap_or(0),
                );
            }
        }
    } else {
        // ---- probe frame (marker path; `--probe` mode only) ----
        let outcome: Option<Result<(u64, ProbeStageTimings), String>> = BRIDGE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let owner = slot.as_mut()?;
            if owner.failure().is_some() {
                return None;
            }
            let (w, h) = owner.desired_size();
            if w == 0 || h == 0 {
                return None;
            }
            if let Err(error) = owner.resize_to(w, h) {
                return Some(Err(error));
            }
            owner.frame(marker)
        });
        if let Some(outcome) = outcome {
            let mut state = lock(shared);
            match outcome {
                Ok((frames, timings)) => {
                    state.shell.status.frames = frames;
                    // Report the actual child-surface size when the surface
                    // exists, otherwise the window size (top-level).
                    if let Some((w, h)) = surface {
                        state.shell.status.width = w;
                        state.shell.status.height = h;
                    } else {
                        state.shell.status.width = size.width;
                        state.shell.status.height = size.height;
                    }
                    state.last_timings = Some(timings);
                    state.shell.status.error = None;
                }
                Err(error) => {
                    state.shell.status.error = Some(error);
                }
            }
        }
    }
}
