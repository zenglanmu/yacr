#![cfg(target_os = "linux")]
//! Synthetic callback contracts, not visual, keyboard or CAD execution acceptance.
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult};
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration};
use slint::Model;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Trace {
    select_all: usize,
    reject_next: bool,
}

struct Sink(Rc<RefCell<Trace>>);
impl UiCommandSink for Sink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        let mut trace = self.0.borrow_mut();
        if std::mem::take(&mut trace.reject_next) {
            return Err(CadError::Unsupported("synthetic selection refusal".into()));
        }
        if command.id == CommandId::SelectAll {
            assert!(matches!(command.payload, CommandPayload::None));
            trace.select_all += 1;
        }
        Ok(())
    }
}

#[test]
fn select_all_uses_one_read_only_command_and_reports_a_refusal() {
    offscreen::install().unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink(trace.clone()), true).unwrap();
    let ui = adapter.component();

    ui.invoke_select_all_requested();
    ui.invoke_command_submitted("SELECTALL".into());
    assert_eq!(trace.borrow().select_all, 2);
    ui.set_work_mode(false);
    ui.invoke_command_input_edited("sele".into());
    assert_eq!(
        ui.get_command_completion_items().row_data(0).unwrap(),
        "SELECTALL"
    );
    trace.borrow_mut().reject_next = true;
    ui.invoke_select_all_requested();
    assert_eq!(trace.borrow().select_all, 2);
    assert!(ui.get_command_expanded());
    assert!(ui
        .get_status_label()
        .contains("synthetic selection refusal"));
    adapter.handle().set_locale("en").unwrap();
    assert_eq!(ui.get_selection_all_label(), "Select all");
}
