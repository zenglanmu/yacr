//! Evidence-only status lines emitted from the probe tick.
//!
//! These are diagnostics, never UI text: one aggregated `[probe] status` line per
//! second carrying the last frame's per-stage timings and the frame index. Split
//! out of `tick` so the tick itself stays a reader of the loop's control flow.

use cad_render_wgpu::ProbeStageTimings;

use super::Shared;

/// Emit the ~1 Hz `[probe] status` line (evidence only).
///
/// Default-OFF: emitted only when the shared evidence gate is on
/// (`YACR_PROBE_CAD_LOG`, see `cad_desktop_shell::probe_log`), like the other
/// host probe diagnostics. A normal session's stderr stays quiet; evidence runs
/// keep the line unchanged.
///
/// `last_timings` is the last frame's per-stage breakdown, or `None` before the
/// first frame. The line is emitted at most once per second; the caller owns the
/// throttle (`RenderTelemetry::last_log`) so this helper is a pure formatter plus
/// one `eprintln!`.
pub(super) fn log_status_line(state: &Shared, last_timings: Option<ProbeStageTimings>) {
    if !cad_desktop_shell::probe_log::evidence_log_enabled() {
        return;
    }
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

/// Emit the bounded `[probe] geometry` evidence line for an accepted layout.
///
/// Split out of `tick` (same reason as [`log_status_line`]): the tick stays a
/// reader of the loop's control flow. Every geometry is named explicitly so a
/// text-scale/page-zoom drift (CSS vs GTK logical vs device) is a number in the
/// log instead of a stretched CAD surface: the DOM CAD rect in **CSS px**, the
/// margin-receiving container in **GTK logical px**, the drawing area + its
/// translated origin, the GDK integer scale, the WebView `dpr` (CSS→device,
/// page zoom included), the CSS→GTK `ratio` and the physical extent.
#[allow(clippy::too_many_arguments)]
pub(super) fn log_layout_geometry(
    top: Option<tauri::PhysicalSize<u32>>,
    container: (f32, f32),
    geometry: Option<crate::gtk_bridge::LayoutGeometry>,
    rect: cad_ui_core::layout::CadMetrics,
    ratio: f64,
    physical: (u32, u32),
    surface: Option<(u32, u32)>,
    margins: cad_ui_core::layout::ViewportInsets,
    revalidate: bool,
) {
    eprintln!(
        "[probe] geometry top={}x{} container={:.0}x{:.0} container_xy=({},{}) dom={}x{} dom_xy=({},{}) drawing_area={}x{} origin=({},{}) scale={} dpr={} ratio={:.3} dom_physical={}x{} native_surface={}x{} margins top={} bottom={} start={} end={} revalidated={}",
        top.map(|s| s.width).unwrap_or(0),
        top.map(|s| s.height).unwrap_or(0),
        container.0,
        container.1,
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
        rect.dpr,
        ratio,
        physical.0,
        physical.1,
        surface.map(|s| s.0).unwrap_or(0),
        surface.map(|s| s.1).unwrap_or(0),
        margins.top,
        margins.bottom,
        margins.start,
        margins.end,
        revalidate,
    );
}
