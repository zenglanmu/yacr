//! Bounded telemetry: render-gate diagnostics, frame logging helpers and the
//! top-level native-handle log lines. Diagnostics only; never UI text.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cad_engine::render::{FrameCompleteness, RenderDiagnostics};
use cad_render_wgpu::GpuMemoryReport;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

use super::{lock_telemetry, RenderTelemetry};

/// The telemetry state of one CAD frame attempt (never UI text).
///
/// `NoSession`/`Waiting`/`Error` are explicitly **not** `Presented`, so a log
/// consumer can never mistake a non-present for a new frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CadFrameLogState {
    Presented,
    Waiting,
    Error,
    NoSession,
}

impl CadFrameLogState {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Presented => "presented",
            Self::Waiting => "waiting",
            Self::Error => "error",
            Self::NoSession => "no-session",
        }
    }
}

/// Whether the render-gate diagnostic line should be emitted now: on a change, or
/// at least once per second while unchanged (never per frame).
pub(super) fn should_log_render_diag(unchanged: bool, recent: bool) -> bool {
    !(unchanged && recent)
}

/// A host CAD frame at or above this wall time (µs) is logged even without an
/// upload observation: `50 ms` (well above the 16 ms cadence, so it never spams).
pub(super) const CAD_STAGE_SLOW_US: u64 = 50_000;

/// Emit the engine render-gate diagnostic on a meaningful state change or a ~1 s
/// heartbeat (never per frame). The caller checks `YACR_PROBE_CAD_LOG`.
pub(super) fn log_render_diagnostics(
    telemetry: &Arc<Mutex<RenderTelemetry>>,
    diagnostics: &RenderDiagnostics,
) {
    let now = Instant::now();
    {
        let state = lock_telemetry(telemetry);
        let unchanged = state.last_render_diag.as_ref() == Some(diagnostics);
        let recent = state
            .last_render_diag_log
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1));
        if !should_log_render_diag(unchanged, recent) {
            return;
        }
    }
    {
        let mut state = lock_telemetry(telemetry);
        state.last_render_diag = Some(diagnostics.clone());
        state.last_render_diag_log = Some(now);
    }
    let runtime = &diagnostics.runtime;
    let upload = runtime
        .upload
        .as_ref()
        .map(|upload| {
            format!(
                "{}/{}/{}",
                upload.scene_revision, upload.group, upload.offset
            )
        })
        .unwrap_or_else(|| "-".to_string());
    eprintln!(
        "[probe] pan-wait gate={:?} lifecycle={} input_rev={} requested_token={} prepared_token={:?} prepared_identity={:?} view_rev={} ready_rev={:?} ready_base={:?} applied_rev={:?} applied_base={:?} resident_batches={} prep_busy={} upload={} upload_base_changed={:?} uploads_started={} upload_rev_resets={} ready_none_clears={} last_upload_reset={:?} suppressed={} grid_refresh_pending={} grid_refresh={} suffix_commits={} pulled_reuses={} pulled_rebuilds={} frames_rendered={} status={:?}",
        diagnostics.gate,
        diagnostics.lifecycle,
        diagnostics.input_revision,
        diagnostics.requested_token,
        diagnostics.prepared_token,
        diagnostics.prepared_identity,
        diagnostics.view_revision,
        diagnostics.ready_revision,
        diagnostics.ready_base_revision,
        runtime.applied_revision,
        runtime.applied_base,
        runtime.resident_base_batches,
        diagnostics.preparation_busy,
        upload,
        runtime.upload.as_ref().map(|upload| upload.base_changed),
        runtime.uploads_started,
        runtime.upload_revision_resets,
        runtime.ready_none_clears,
        runtime.last_upload_reset,
        runtime.overlays_suppressed,
        diagnostics.grid_refresh_pending,
        diagnostics.grid_refresh,
        runtime.overlay_suffix_commits,
        runtime.pulled_prefix_reuses,
        runtime.pulled_prefix_rebuilds,
        runtime.frames_rendered,
        diagnostics.result_status,
    );
}

/// Whether the bounded GPU-memory status line is due: **at most once per second**.
///
/// The renderer's report calls the driver's allocator query, and the budget policy
/// only changes on a settings write, so a per-frame read would be pure waste. The
/// line is therefore emitted on this cadence, under the same `YACR_PROBE_CAD_LOG`
/// gate as the other renderer diagnostics.
pub(super) fn gpu_memory_due(telemetry: &Arc<Mutex<RenderTelemetry>>) -> bool {
    let now = Instant::now();
    let mut state = lock_telemetry(telemetry);
    if state
        .last_gpu_memory_log
        .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
    {
        return false;
    }
    state.last_gpu_memory_log = Some(now);
    true
}

/// `Option<u64>` as the GPU-memory line spells it: the number, or `none`.
fn count_or_none(value: Option<u64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "none".to_string(),
    }
}

/// One machine-readable GPU-memory status line: the card probe, the budget policy
/// in force, and the renderer's own totals.
///
/// `vram_bytes` is the backend's **advertised device-local** memory, not free
/// memory, and `none` when the backend cannot answer. `budget_source` is the
/// renderer's stable token (`vram-percent` / `explicit-bytes` / `estimate`), never
/// `Debug` output, so a script can tell a percentage cap from a fallback.
/// Diagnostics only — never user-facing copy.
pub(super) fn gpu_memory_line(report: &GpuMemoryReport) -> String {
    format!(
        "[probe] gpu-memory vram_bytes={} budget_percent={} budget_source={} total={} peak={} \
         cap={}",
        count_or_none(report.vram_bytes),
        match report.budget_percent {
            Some(percent) => percent.to_string(),
            None => "none".to_string(),
        },
        report.budget_source.label(),
        report.total_bytes,
        report.peak_bytes,
        count_or_none(report.cap_bytes),
    )
}

/// Emit the bounded GPU-memory status line when it is due (≤1 Hz).
///
/// `report` is only called once the line is due, so a per-frame tick never hits
/// the driver's allocator query. `None` (no attached renderer yet) skips the line
/// instead of printing a fabricated zero report.
pub(super) fn log_gpu_memory(
    telemetry: &Arc<Mutex<RenderTelemetry>>,
    report: impl FnOnce() -> Option<GpuMemoryReport>,
) {
    if !gpu_memory_due(telemetry) {
        return;
    }
    if let Some(report) = report() {
        eprintln!("{}", gpu_memory_line(&report));
    }
}

/// Classify a CAD frame result for the env-gated telemetry log.
pub(super) fn cad_frame_log_state(
    result: &Option<Result<Option<u64>, String>>,
) -> CadFrameLogState {
    match result {
        Some(Ok(Some(_))) => CadFrameLogState::Presented,
        Some(Ok(None)) => CadFrameLogState::Waiting,
        Some(Err(_)) => CadFrameLogState::Error,
        None => CadFrameLogState::NoSession,
    }
}

/// A rendered-frame completeness label for the telemetry log (technical, not UI).
pub(super) fn frame_completeness_label(completeness: FrameCompleteness) -> &'static str {
    match completeness {
        FrameCompleteness::CurrentScene => "current_scene",
        FrameCompleteness::BaseOnlyPendingOverlays => "base_only_pending_overlays",
    }
}

/// Render an optional telemetry number, using `-` for an absent value.
pub(super) fn opt_telemetry_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Render an optional telemetry duration (µs), using `-` for an absent value.
pub(super) fn opt_telemetry_u128(value: Option<u128>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Log the top-level native window handle the probe surface is created on.
///
/// Both supported backends are reported as first-class: the X11 XID line is
/// unchanged (capture scripts parse it), and a native Wayland toplevel logs the
/// actual `wl_surface`/`wl_display` pair as surface identity — it is never an
/// error. The Wayland *window* handle carries only the `wl_surface` (this
/// `raw-window-handle` version has no `display` field on it), so the paired
/// `wl_display` is read separately from the window's display handle instead of
/// being invented.
pub(super) fn log_top_level_native_handle(window: &tauri::Window) {
    match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Xlib(xlib) => eprintln!(
                "[probe] top-level X11 XID (Xlib) = 0x{:x} visual=0x{:x}",
                xlib.window, xlib.visual_id
            ),
            RawWindowHandle::Xcb(xcb) => {
                eprintln!("[probe] top-level X11 XID (Xcb) = 0x{:x}", xcb.window.get())
            }
            RawWindowHandle::Wayland(wayland) => match wayland_display_ptr(window) {
                Some(display) => eprintln!(
                    "[probe] top-level native Wayland surface={:p} display={:p}",
                    wayland.surface.as_ptr(),
                    display
                ),
                None => eprintln!(
                    "[probe] top-level native Wayland surface={:p} display=<unavailable>",
                    wayland.surface.as_ptr()
                ),
            },
            other => eprintln!("[probe] unexpected top-level raw handle: {other:?}"),
        },
        Err(error) => eprintln!("[probe] could not read the top-level window handle: {error}"),
    }
}

/// The raw `wl_display` pointer paired with the toplevel native window.
///
/// A Wayland window handle exposes only the `wl_surface`, so the display is read
/// from the window's display handle; a non-Wayland or unreadable display returns
/// `None` rather than a fabricated pointer.
fn wayland_display_ptr(window: &tauri::Window) -> Option<*mut std::ffi::c_void> {
    match window.display_handle() {
        Ok(handle) => match handle.as_raw() {
            RawDisplayHandle::Wayland(display) => Some(display.display.as_ptr()),
            _ => None,
        },
        Err(_) => None,
    }
}
