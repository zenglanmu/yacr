//! The `native-below` CAD frame path: present one frame through the bound render
//! session, mirror the resulting status/snapshot, and emit the bounded
//! frame/stage diagnostics.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use cad_ui_core::shell::{clear_host_error, note_host_error, HostErrorKind};

use super::super::admission::{cad_frame_geometry, cad_present_admission, CadPresentAdmission};
use super::super::postload::{drive_postload_pan, ActualRenderSummary};
use super::super::telemetry::{
    cad_frame_log_state, frame_completeness_label, log_gpu_memory, log_render_diagnostics,
    opt_telemetry_u128, opt_telemetry_u64, CadFrameLogState, CAD_STAGE_SLOW_US,
};
use super::super::{fonts, layers, lock, lock_telemetry, performance, vendor};
use super::super::{RenderTelemetry, Shared, BRIDGE, CAD};

/// Draw one CAD frame and record its outcome (status, mirrors, evidence).
pub(super) fn present_cad_frame(
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
    cad_log: bool,
    last_input_seq: Option<u64>,
    input_age_us: Option<u128>,
) {
    let (logical, scale, dom_rect, physical) = {
        let rect = lock(shared).shell.input.last_rect();
        cad_frame_geometry(rect)
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
            let (w, h) = physical;
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
            let message = format!("cad present failed: {error}");
            let changed = state
                .shell
                .cad_host_error
                .as_ref()
                .is_none_or(|previous| previous.text != message);
            note_host_error(&mut state.shell, HostErrorKind::Present, message.clone());
            drop(state);
            if changed {
                eprintln!("[probe] {message}");
            }
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
            log_render_diagnostics(telemetry, &diagnostics);
        }
        // Bounded GPU-memory status (≤1 Hz): the card probe, the budget policy
        // in force and the renderer's own total/peak. The report is read only
        // when the line is due, never per frame.
        log_gpu_memory(telemetry, || {
            CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|session| session.render_session().gpu_memory_report())
            })
        });
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
        lock_telemetry(telemetry).last_actual_render = Some(ActualRenderSummary {
            render_seq: actual.render_seq,
            view_revision: actual.view_revision,
            device_epoch: actual.device_epoch,
        });
        lock_telemetry(telemetry).last_pulled_state = Some(actual.pulled_state.clone());
    }
    let mut snapshot = CAD.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|session| session.snapshot(backend, adapter, 0))
    });
    performance::attach(&mut snapshot, shared);
    // Owner-thread mirrors for the IPC reads: settings + runtime aliases and
    // the reflected plugin-API snapshot.
    let settings = CAD.with(|c| c.borrow().as_ref().map(|session| session.owner_mirror()));
    // `None` = no session yet (`cad_waiting`); a snapshot with no document is
    // `Some(..)` and answers `{"document": null}`.
    let api_snapshot =
        CAD.with(|cell| cell.borrow().as_ref().map(|session| session.api_snapshot()));
    {
        let mut state = lock(shared);
        if let Some(snapshot) = &snapshot {
            state.shell.cad_status = Some(snapshot.clone());
        }
        if let Some((settings, command_aliases)) = settings {
            state.cad_settings = Some(settings);
            state.shell.command_aliases = command_aliases;
        }
        state.cad_api_snapshot = api_snapshot;
    }
    // Hand the importer's records to the pass and install a settled derived
    // snapshot on this owner thread (display-only; no `cad-db` write).
    vendor::tick(shared);
    // One cad-event flush per owner tick, at the same point the status mirror
    // is final: the event's `status` is the exact `cad_status` payload (host
    // error + projected popup included). The flush is deliberately outside the
    // `Some(snapshot)` arm: producer/lazy-session ticks and a session-creation
    // failure have no snapshot but still owe the chrome a status event.
    // Gap recovery keeps reading `shell.cad_status`.
    //
    // Native-only: on this path the carrier is always `native-below` (the
    // `dom-canvas`/X11 case returned above). There the wasm session's own
    // `cad_event` stream is the only producer; the native host emits nothing.
    let _ = cad_desktop_shell::events::flush_events_current(
        shared,
        &cad_ui_core::session::HostProfile::native_below(),
        None::<String>,
    );
    // The layer panel's own mirror (see `layers`).
    layers::mirror(shared);
    // The Font Manager's own mirror (see `fonts`).
    fonts::mirror(shared);
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
    drive_postload_pan(shared, telemetry);
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
        lock_telemetry(telemetry).last_timings = Some(observation.timings);
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
}
