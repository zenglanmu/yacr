//! The probe frame path (`--probe` mode, no CAD session): draw the marker frame
//! and mirror its status.

use std::sync::{Arc, Mutex};

use cad_render_wgpu::ProbeStageTimings;

use super::super::{lock, lock_telemetry, RenderTelemetry, Shared, BRIDGE};

/// Draw one probe frame and record its outcome.
pub(super) fn present(
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
    size: tauri::PhysicalSize<u32>,
    marker: f32,
    surface: Option<(u32, u32)>,
) {
    // ---- probe frame (marker path; `--probe` mode only) ----
    let outcome: Option<Result<(u64, ProbeStageTimings), String>> = BRIDGE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let owner = slot.as_mut()?;
        if owner.failure().is_some() {
            return None;
        }
        let (w, h) = owner.probe_surface_size();
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
                lock_telemetry(telemetry).last_timings = Some(timings);
                state.shell.status.error = None;
            }
            Err(error) => {
                state.shell.status.error = Some(error);
            }
        }
    }
    // Probe mode has no CAD session, but the host still owns the popup state
    // `cad_status` reports. Flush the same projection so an open modal is not
    // stranded behind the push channel.
    let _ = cad_desktop_shell::events::flush_events_current(
        shared,
        &cad_ui_core::session::HostProfile::native_below(),
        None::<String>,
    );
}
