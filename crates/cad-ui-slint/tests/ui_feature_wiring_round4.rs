#![cfg(target_os = "linux")]
//! Synthetic callback contracts, not visual, keyboard or CAD execution acceptance.
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{AnnotationId, CadError, CadResult};
use cad_ui_slint::{
    offscreen, AnnotationPanelState, AnnotationRowUi, UiAdapter, UiCommandSink, UiConfiguration,
};
use slint::Model;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Trace {
    selected: Vec<AnnotationId>,
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
        match command.id {
            CommandId::SelectAll => {
                assert!(matches!(command.payload, CommandPayload::None));
                trace.select_all += 1;
            }
            CommandId::SelectAnnotation => {
                if let CommandPayload::SelectAnnotation(Some(id)) = command.payload {
                    trace.selected.push(id);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[test]
fn annotation_search_retains_exact_id_mapping_and_select_all_uses_one_read_only_command() {
    offscreen::install().unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink(trace.clone()), true).unwrap();
    let ui = adapter.component();
    let ids = [AnnotationId(1), AnnotationId(u128::MAX)];
    let state = AnnotationPanelState {
        rows: vec![
            AnnotationRowUi {
                id: 0,
                kind: "Cloud".into(),
                text: "Roof".into(),
                visible: true,
                overridden: false,
                selected: false,
            },
            AnnotationRowUi {
                id: 1,
                kind: "Text".into(),
                text: "Étage 北墙".into(),
                visible: true,
                overridden: false,
                selected: false,
            },
        ],
        ..Default::default()
    };
    adapter.handle().set_annotation_state(&state, &ids).unwrap();
    assert_eq!(ui.get_annotation_matching_count(), 2);
    ui.set_annotation_search_text("ÉTAGE".into());
    ui.invoke_annotation_search_edited("ÉTAGE".into());
    assert_eq!(ui.get_annotation_matching_count(), 1);
    assert_eq!(ui.get_annotation_rows().row_count(), 2);
    ui.invoke_annotation_selected(1);
    assert_eq!(trace.borrow().selected, vec![ids[1]]);
    adapter
        .handle()
        .set_annotation_state(&AnnotationPanelState::default(), &[])
        .unwrap();
    assert_eq!(ui.get_annotation_matching_count(), 0);
    adapter.handle().set_annotation_state(&state, &ids).unwrap();
    assert_eq!(ui.get_annotation_matching_count(), 1);
    ui.set_annotation_search_text("".into());
    ui.invoke_annotation_search_edited("".into());
    assert_eq!(ui.get_annotation_matching_count(), 2);

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
    assert_eq!(
        ui.get_annotation_search_placeholder(),
        "Search annotation text or type"
    );
}
