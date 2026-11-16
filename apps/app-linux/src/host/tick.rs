//! One probe tick: main-thread bookkeeping, deferred GPU init, React CAD layout,
//! the CAD session bind/commands and the CAD/probe frame paths.

mod bookkeeping;
mod bootstrap;
mod cad_frame;
mod commands;
mod probe_frame;

use std::sync::{Arc, Mutex};

use super::{lock, RenderTelemetry, Shared, BRIDGE};

/// One probe tick. Runs on the main thread. The shared lock is held only for
/// bookkeeping, never across GPU work.
pub fn tick(shared: &Arc<Mutex<Shared>>, telemetry: &Arc<Mutex<RenderTelemetry>>) {
    // ---- short critical section: bookkeeping only ----
    let Some((size, marker)) = bookkeeping::begin_frame(shared, telemetry) else {
        return;
    };

    // ---- `dom-canvas`: no native frame path at all (ADR 0024 D1/D2) ----
    //
    // The wasm session inside the WebView owns the pixels and the camera, so this
    // tick must not run the native layout/GPU/session/frame pipeline: there is no
    // CAD session to bind, no presenter to init and no surface to configure. The
    // bookkeeping above still runs (stop requests, size report, status line, DOM
    // diagnostics) so the shell's lifecycle stays observable.
    if !super::surface_plan().per_frame_tick {
        return;
    }

    // ---- child webview sizing + deferred bridge GPU init (main thread) ----
    if bootstrap::sync_and_init_gpu(shared, size) {
        return;
    }

    // ---- React CAD layout -> native margins (main thread, no shared lock) ----
    bootstrap::apply_react_layout(shared);

    // ---- CAD session: lazy creation and bind ----
    let cad_mode = lock(shared).shell.cad_mode;
    bootstrap::create_and_bind_session(shared, cad_mode);

    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    let cad_log = std::env::var_os("YACR_PROBE_CAD_LOG").is_some();
    let (last_input_seq, input_age_us) = commands::apply_batch(shared, cad_mode, cad_log);

    // ---- GPU work, without holding the shared lock ----
    let surface = BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
    if cad_mode {
        cad_frame::present_cad_frame(shared, telemetry, cad_log, last_input_seq, input_age_us);
    } else {
        // ---- probe frame (marker path; `--probe` mode only) ----
        probe_frame::present(shared, telemetry, size, marker, surface);
    }
}
