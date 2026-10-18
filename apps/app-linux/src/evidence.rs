//! Env-gated, evidence-only lifecycle and interaction drivers for the
//! real-window probe run.
//!
//! Each driver is enabled only by its own `YACR_PROBE_*` variable; a normal run
//! is unaffected. A driver runs on a background thread and schedules actions
//! relative to its own start:
//! - synthetic input injected through the real webview (`Webview::eval`),
//!   explicitly **synthetic, not a real mouse**;
//! - a window resize;
//! - a minimize, then an unminimize (restore);
//! - a final resize.
//!
//! The threads only call public `Window`/`Webview` dispatch methods. The GPU
//! presenter still runs on the main-thread tick, and all GPU work stays inside
//! `SurfaceProbePresenter`; these modules touch no GPU state.
//!
//! This is a layout split of the former single `evidence.rs`: every public
//! function, env var name, log prefix and scheduling behaviour is unchanged.
//! - [`input`]: the `YACR_PROBE_EVIDENCE` input/resize driver;
//! - [`lifecycle`]: the `YACR_PROBE_LIFECYCLE` window lifecycle driver;
//! - [`pan`]: the `YACR_PROBE_POSTLOAD_PAN` gesture scripts (polled by
//!   `host::postload`);
//! - [`tool`]: the `YACR_PROBE_TOOL_EVIDENCE` interactive tool driver;
//! - [`hover`]: the `YACR_PROBE_HOVER` hover-cost driver;
//! - [`box_select`]: the `YACR_PROBE_BOX_EVIDENCE` box-selection driver.

use std::time::{Duration, Instant};

mod box_select;
mod hover;
mod input;
mod lifecycle;
mod pan;
mod tool;

pub use box_select::{box_evidence_enabled, maybe_spawn_box_evidence};
pub use hover::{hover_evidence_enabled, maybe_spawn_hover_evidence};
pub use input::{enabled, maybe_spawn};
pub use lifecycle::maybe_spawn_lifecycle;
pub use pan::{postload_pan_enabled, POSTLOAD_PAN_JS, POSTLOAD_PAN_REPORT_JS};
pub use tool::{maybe_spawn_tool_evidence, tool_evidence_enabled};

/// Sleep until `seconds` after `start`, then run `action` and log the result.
///
/// Shared by the [`input`] and [`lifecycle`] drivers (their log prefixes are the
/// caller's labels; this helper always logs the `[probe] evidence step:` line).
fn schedule(
    start: &Instant,
    seconds: u64,
    label: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    let target = Duration::from_secs(seconds);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    match action() {
        Ok(()) => eprintln!("[probe] evidence step: {label} -> ok"),
        Err(error) => eprintln!("[probe] evidence step: {label} -> error: {error}"),
    }
}
