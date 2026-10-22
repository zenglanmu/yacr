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
//! - [`edit`]: the `YACR_PROBE_EDIT_EVIDENCE` TRIM/EXTEND entity-pick driver;
//! - [`draw`]: the `YACR_PROBE_DRAW_EVIDENCE` new drawing-command driver;
//! - [`draw_edit`]: the `YACR_PROBE_DRAW_EVIDENCE` P4 edit-command phases
//!   (ARRAYRECT/ARRAYPOLAR/ALIGN/STRETCH/EXPLODE) and the combined report;
//! - [`draw_dimension`]: the `YACR_PROBE_DRAW_EVIDENCE` P5c dimension-creation
//!   phases (DIMLINEAR/DIMALIGNED);
//! - [`draw_dimension_radial`]: the `YACR_PROBE_DRAW_EVIDENCE` P5d radial
//!   dimension-creation phases (DIMRADIUS/DIMDIAMETER/DIMCENTER);
//! - [`draw_dimension_angular_ordinate`]: the `YACR_PROBE_DRAW_EVIDENCE` P5e
//!   two-line angular + ordinate dimension phases (DIMANGULAR/DIMORDINATE);
//! - [`draw_mleader`]: the `YACR_PROBE_DRAW_EVIDENCE` P7c `MLEADER`
//!   creation phase (arrow + landing picks, then the real `cad_set_tool_text`
//!   admission; the pixel criterion ADR 0019 decision J owes for P7c);
//! - [`draw_baseline_continue`]: the `YACR_PROBE_DRAW_EVIDENCE` P5f
//!   `DIMBASELINE`/`DIMCONTINUE` phases (create a linear base, pick it, then
//!   three measured/chained points; `entityCount` +3 each);
//! - [`hover`]: the `YACR_PROBE_HOVER` hover-cost driver;
//! - [`box_select`]: the `YACR_PROBE_BOX_EVIDENCE` box-selection driver;
//! - [`popup`]: the `YACR_PROBE_POPUP_EVIDENCE` modal-popup pipeline driver
//!   (open → real gate refusal → close → real gate release);
//! - [`popup_textedit`]: the same driver's `textEdit` phase (host-derived payload
//!   → real `cad_set_selected_text` queue + tick drain → status verification →
//!   close → gate release), scheduled after the `about` phase;
//! - [`popup_textedit_reopen`]: that phase's reopen steps (stored result visible
//!   in the dialog for an external grab, then closed again);
//! - [`popup_textedit_summary`]: that phase's recorded reads and single-line
//!   summary (both split at the 1000-line code-file gate);
//! - [`view3d`]: the `YACR_PROBE_VIEW3D` driver that switches the real session
//!   to the 3D view (`SWITCH2D3D`), orbits it with a synthetic primary drag and
//!   reports the real `cad_status` reads (2D camera present -> absent), including
//!   a 2D control click and a 3D click on the solid so 3D picking is measured
//!   rather than assumed;
//! - [`ui_dialogs`] (+ its [`ui_dialogs_js`] blobs, [`ui_dialogs_steps`] step
//!   bodies and [`ui_dialogs_report`] reads/log formats): the
//!   `YACR_PROBE_UI_DIALOGS` driver that drives the React chrome through its own
//!   `Ctrl+1` handler and command field, so an external grab can capture the
//!   Options dialog, the populated Properties palette and the command-line echo.

use std::time::{Duration, Instant};

mod box_select;
mod draw;
mod draw_baseline_continue;
mod draw_baseline_continue_js;
mod draw_dimension;
mod draw_dimension_angular_ordinate;
mod draw_dimension_radial;
mod draw_edit;
mod draw_hatch;
mod draw_mleader;
mod edit;
mod hover;
mod input;
mod lifecycle;
mod pan;
mod popup;
mod popup_textedit;
mod popup_textedit_reopen;
mod popup_textedit_summary;
mod tool;
mod ui_dialogs;
mod ui_dialogs_js;
mod ui_dialogs_report;
mod ui_dialogs_steps;
mod view3d;

pub use box_select::{box_evidence_enabled, maybe_spawn_box_evidence};
pub use draw::{draw_evidence_enabled, maybe_spawn_draw_evidence};
pub use edit::{edit_evidence_enabled, maybe_spawn_edit_evidence};
pub use hover::{hover_evidence_enabled, maybe_spawn_hover_evidence};
pub use input::{enabled, maybe_spawn};
pub use lifecycle::maybe_spawn_lifecycle;
pub use pan::{postload_pan_enabled, POSTLOAD_PAN_JS, POSTLOAD_PAN_REPORT_JS};
pub use popup::{maybe_spawn_popup_evidence, popup_evidence_enabled};
pub use tool::{maybe_spawn_tool_evidence, tool_evidence_enabled};
pub use ui_dialogs::{maybe_spawn_ui_dialogs, ui_dialogs_enabled};
pub use view3d::{maybe_spawn_view3d, view3d_enabled, view3d_pick_enabled};

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
