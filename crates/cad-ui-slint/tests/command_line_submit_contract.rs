#![cfg(target_os = "linux")]
//! Synthetic submit-callback wiring for the AutoCAD-style command line.
//!
//! Proves vocabulary resolution plus the consumed/kept contract against the real
//! Slint component on the offscreen platform. It is not rendering or CAD-command
//! acceptance. The Slint offscreen platform installs once per thread, so a single
//! test owns the process-wide install and exercises every case in sequence.
use cad_domain::CadResult;
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration};
use std::cell::Cell;
use std::rc::Rc;

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

fn build_adapter() -> UiAdapter {
    UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap()
}

fn build_adapter_viewer() -> UiAdapter {
    UiAdapter::new(UiConfiguration::default(), Sink, false).unwrap()
}

#[test]
fn command_line_prefix_alias_repeat_and_explicit_failures() {
    offscreen::install().unwrap();

    // Exact canonical command: consumed, and remembered for empty replay.
    let adapter = build_adapter();
    let ui = adapter.component();
    let before = ui.get_ribbon_expanded();
    assert!(ui.invoke_command_submitted("TOOLS".into()));
    let toggled = ui.get_ribbon_expanded();
    assert_ne!(before, toggled, "TOOLS toggles the ribbon");

    // Empty Enter (and whitespace-only) repeats the last dispatched command.
    assert!(ui.invoke_command_submitted("".into()));
    assert_eq!(
        ui.get_ribbon_expanded(),
        before,
        "empty Enter repeats TOOLS"
    );
    assert!(ui.invoke_command_submitted("   ".into()));
    assert_eq!(ui.get_ribbon_expanded(), toggled);

    // An ambiguous prefix is not consumed and must not run anything.
    assert!(!ui.invoke_command_submitted("ZOOM".into()));
    assert!(!ui.get_draw_tool_active());
    // A unique prefix executes and is consumed; a gated alias does too.
    assert!(ui.invoke_command_submitted("LIN".into()));
    assert!(ui.get_draw_tool_active());
    assert!(ui.invoke_command_submitted("ESC".into()));
    assert!(!ui.get_draw_tool_active());
    assert!(ui.invoke_command_submitted("ZO".into()));

    // Recognized-but-unimplemented vs unknown are both explicit and consumed.
    adapter.handle().set_locale("en").unwrap();
    assert!(ui.invoke_command_submitted("HATCH".into()));
    assert_eq!(
        ui.get_status_label().to_string(),
        "Command HATCH is not supported yet."
    );
    assert!(ui.invoke_command_submitted("NOPE".into()));
    assert_eq!(ui.get_status_label().to_string(), "Unknown command: NOPE");

    // The unsupported placeholder keeps the input as typed (case and payload),
    // never the uppercased, space-joined dispatch key.
    assert!(ui.invoke_command_submitted("SAVE plan.dwg".into()));
    assert_eq!(
        ui.get_status_label().to_string(),
        "Command SAVE plan.dwg is not supported yet."
    );

    // CONFIRM with nothing active is a permanent no-op and must not become the
    // empty-Enter replay target: the last real command (TOOLS) still repeats.
    let replay_target = ui.get_ribbon_expanded();
    assert!(ui.invoke_command_submitted("TOOLS".into()));
    let after_tools = ui.get_ribbon_expanded();
    assert!(ui.invoke_command_submitted("CONFIRM".into()));
    assert!(ui.invoke_command_submitted("".into()));
    assert_eq!(
        ui.get_ribbon_expanded(),
        replay_target,
        "CONFIRM must not replace the repeat-last command"
    );
    assert_ne!(after_tools, replay_target);

    // Disabled keyboard shortcuts: a short alias stays unknown, while canonical
    // names and full-word synonyms still resolve.
    let adapter = build_adapter();
    let ui = adapter.component();
    adapter
        .handle()
        .update_config_json(r#"{"interaction":{"keyboardShortcuts":false}}"#)
        .unwrap();
    assert!(ui.invoke_command_submitted("L".into()));
    assert!(
        !ui.get_draw_tool_active(),
        "a gated alias must not run LINE"
    );
    assert!(ui.get_status_label().contains('L'));
    assert!(ui.invoke_command_submitted("LINE".into()));
    assert!(ui.get_draw_tool_active(), "a canonical name always works");
    assert!(
        ui.invoke_command_submitted("FIT".into()),
        "a full-word synonym always works"
    );

    // Viewer mode + a work-only ambiguous prefix (`MEAS`): explicit read-only
    // feedback and consumed, never a silent empty completion popup.
    let adapter = build_adapter_viewer();
    let ui = adapter.component();
    adapter.handle().set_locale("en").unwrap();
    assert!(ui.invoke_command_submitted("MEAS".into()));
    assert_eq!(
        ui.get_status_label().to_string(),
        "Read-only (Viewer) mode does not allow drawing or editing"
    );

    // NEW is host-owned: with `can-new` false the command line reports the
    // explicit unsupported text and does not fire the callback; with it true the
    // host request fires. `can-new` defaults false, so web/android stay explicit.
    let adapter = build_adapter();
    let ui = adapter.component();
    adapter.handle().set_locale("en").unwrap();
    let new_calls = Rc::new(Cell::new(0));
    let observed = new_calls.clone();
    ui.on_new_requested(move || observed.set(observed.get() + 1));
    assert!(ui.invoke_command_submitted("NEW".into()));
    assert_eq!(
        new_calls.get(),
        0,
        "can-new false must not fire the request"
    );
    assert_eq!(
        ui.get_status_label().to_string(),
        "Command NEW is not supported yet."
    );
    ui.set_can_new(true);
    assert!(ui.invoke_command_submitted("NEW".into()));
    assert_eq!(new_calls.get(), 1, "can-new true fires exactly one request");

    // PLOT is host-owned vector export: `can-plot` false reports the explicit
    // unsupported text and does not fire; true fires the request. EXPORT/PRINT
    // are synonyms that resolve to PLOT.
    let adapter = build_adapter();
    let ui = adapter.component();
    adapter.handle().set_locale("en").unwrap();
    let plot_calls = Rc::new(Cell::new(0));
    let observed = plot_calls.clone();
    ui.on_plot_requested(move || observed.set(observed.get() + 1));
    assert!(ui.invoke_command_submitted("PLOT".into()));
    assert_eq!(
        plot_calls.get(),
        0,
        "can-plot false must not fire the request"
    );
    assert_eq!(
        ui.get_status_label().to_string(),
        "Command PLOT is not supported yet."
    );
    ui.set_can_plot(true);
    assert!(ui.invoke_command_submitted("EXPORT".into()));
    assert_eq!(plot_calls.get(), 1, "EXPORT resolves to PLOT");
    assert!(ui.invoke_command_submitted("PRINT".into()));
    assert_eq!(plot_calls.get(), 2, "PRINT resolves to PLOT");
}
