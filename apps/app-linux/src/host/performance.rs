//! Owner-tick performance sampling for the diagnostics panel.
//!
//! The shared sampler ([`cad_desktop_shell::performance::PerformanceState`]) owns
//! the cadence; this module only connects it to the concrete sources:
//! `/proc` (inside the sampler) and the renderer's own GPU-memory report. Every
//! value is measured or explicitly absent — never invented.

use std::sync::{Arc, Mutex};

use cad_ui_core::session::CadStatus;

use super::telemetry::gpu_memory_stats;
use super::{lock, Shared, CAD};

/// Attach the current performance block to `snapshot`, or do nothing when no
/// session snapshot exists.
///
/// The GPU report is read **only** when the sampler says it is due, so a 60 Hz
/// tick never queries the driver allocator. `record_gpu(None)` clears the GPU
/// fields when no renderer is attached instead of keeping stale numbers.
pub(super) fn attach(snapshot: &mut Option<CadStatus>, shared: &Arc<Mutex<Shared>>) {
    let gpu_due = lock(shared).performance.gpu_sample_due();
    let gpu_stats = if gpu_due {
        CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|session| session.render_session().gpu_memory_report())
        })
        .map(|report| gpu_memory_stats(&report))
    } else {
        None
    };
    let mut state = lock(shared);
    state.performance.refresh_system();
    if gpu_due {
        state.performance.record_gpu(gpu_stats);
    }
    if let Some(snapshot) = snapshot.as_mut() {
        // Linux is the only carrier wired for GPU metrics in this change; the
        // other native hosts leave this block `None` until they wire their own
        // sampler — explicit absence, never a fabricated zero.
        snapshot.performance = Some(state.performance.snapshot());
    }
}
