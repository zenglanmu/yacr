//! Evidence-only status lines emitted from the probe tick.
//!
//! These are diagnostics, never UI text: one aggregated `[probe] status` line per
//! second carrying the last frame's per-stage timings and the frame index. Split
//! out of `tick` so the tick itself stays a reader of the loop's control flow.

use cad_render_wgpu::ProbeStageTimings;

use super::Shared;

/// Emit the ~1 Hz `[probe] status` line (evidence only).
///
/// `last_timings` is the last frame's per-stage breakdown, or `None` before the
/// first frame. The line is emitted at most once per second; the caller owns the
/// throttle (`RenderTelemetry::last_log`) so this helper is a pure formatter plus
/// one `eprintln!`.
pub(super) fn log_status_line(state: &Shared, last_timings: Option<ProbeStageTimings>) {
    let stages = last_timings
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
