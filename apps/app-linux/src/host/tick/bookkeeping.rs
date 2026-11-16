//! Start-of-tick bookkeeping: stop requests, the frame-rate gate, the periodic
//! status line, the bounded DOM diagnostics and the live size/marker snapshot.
//!
//! This is the tick's only critical section; no GPU work happens here.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::dom::run_dom_diag;
use super::super::options::{frame_interval, ANIMATION_STEP};
use super::super::presenter::release_presenter;
use super::super::tick_evidence::log_status_line;
use super::super::{lock, lock_telemetry, RenderTelemetry, Shared};

/// One tick's bookkeeping pass. Runs on the main thread.
///
/// Returns `None` when the tick must not continue: already stopped, a stop
/// request, the frame-rate gate, a dispatched DOM diagnostic, no window, or a
/// zero/occluded surface. Otherwise the live window size and the input marker.
pub(super) fn begin_frame(
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) -> Option<(tauri::PhysicalSize<u32>, f32)> {
    let mut state = lock(shared);
    // Lock order (frozen): Shared -> RenderTelemetry, never the reverse.
    let mut telemetry = lock_telemetry(telemetry);
    telemetry.wake_pending.store(false, Ordering::SeqCst);

    if state.shell.stopped {
        return None;
    }
    if state.shell.stop_requested {
        drop(telemetry);
        drop(state);
        release_presenter();
        let mut state = lock(shared);
        state.shell.stopped = true;
        state.shell.status.stopped = true;
        return None;
    }

    let now = Instant::now();
    if let Some(last) = telemetry.last_frame {
        // Input owns a faster gate: queued work or a move within two idle
        // periods keeps the 8 ms cadence so the crosshair gets a fresh
        // position each refresh; a still window keeps the 16 ms cap.
        if now.duration_since(last) < frame_interval(state.shell.input_active(now)) {
            return None;
        }
    }
    telemetry.last_frame = Some(now);
    state.shell.ticks += 1;

    // Periodic (~1 Hz) status line for evidence; includes the last frame's
    // per-stage timings and the frame index.
    if telemetry
        .last_log
        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
    {
        telemetry.last_log = Some(now);
        log_status_line(&state, telemetry.last_timings);
    }

    // Bounded DOM diagnostics at ~6s and ~14s (2 runs max); they only
    // dispatch an eval, so they never block the loop.
    if telemetry.diag_runs < 2 {
        let due = Duration::from_secs(6 + 8 * u64::from(telemetry.diag_runs));
        if telemetry.diag_start.elapsed() >= due {
            telemetry.diag_runs += 1;
            let run = telemetry.diag_runs;
            drop(telemetry);
            drop(state);
            run_dom_diag(run);
            return None;
        }
    }

    let window = state.window.clone()?;
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
        return None;
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
    Some((size, marker))
}
