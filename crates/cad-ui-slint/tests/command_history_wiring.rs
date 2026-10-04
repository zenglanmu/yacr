#![cfg(target_os = "linux")]
//! Synthetic callback wiring, not command execution or visual acceptance.
use cad_domain::CadResult;
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration};
use slint::Model;

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

#[test]
fn submitted_input_archive_recall_and_clear_share_the_same_history() {
    offscreen::install().unwrap();
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    let ui = adapter.component();
    ui.invoke_command_submitted("FIT".into());
    ui.invoke_command_submitted("FIT".into());
    ui.invoke_command_submitted("Unknown /CaseSensitive/路径".into());
    let entries = ui.get_command_history_entries();
    assert_eq!(entries.row_count(), 2);
    assert_eq!(entries.row_data(0).unwrap(), "FIT");
    assert_eq!(entries.row_data(1).unwrap(), "Unknown /CaseSensitive/路径");
    assert!(ui
        .get_status_label()
        .contains("Unknown /CaseSensitive/路径"));
    assert_eq!(
        ui.invoke_command_history_recalled(-1, "unfinished Draft".into()),
        "Unknown /CaseSensitive/路径"
    );
    assert_eq!(
        ui.invoke_command_history_recalled(-1, "ignored".into()),
        "FIT"
    );
    assert_eq!(
        ui.invoke_command_history_recalled(1, "ignored".into()),
        "Unknown /CaseSensitive/路径"
    );
    assert_eq!(
        ui.invoke_command_history_recalled(1, "ignored".into()),
        "unfinished Draft"
    );
    ui.invoke_command_submitted("x".repeat(4097).into());
    assert_eq!(ui.get_command_history_entries().row_count(), 2);
    assert!(ui.get_command_history_storage_limited());
    assert!(ui.get_command_expanded());
    ui.invoke_command_history_cleared();
    assert!(!ui.get_command_history_storage_limited());
    assert_eq!(ui.get_command_history_entries().row_count(), 0);
    assert_eq!(
        ui.invoke_command_history_recalled(-1, "new draft".into()),
        "new draft"
    );
    adapter.handle().set_locale("en").unwrap();
    assert_eq!(ui.get_command_history_label(), "Submitted commands");
    assert_eq!(
        ui.get_command_history_clear_label(),
        "Clear command history"
    );
    adapter.handle().set_locale("zh-CN").unwrap();
    assert_eq!(ui.get_command_history_label(), "已提交命令");
}
