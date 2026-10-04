#![cfg(target_os = "linux")]
//! Synthetic UI wiring only; no command, GPU, keyboard or clipboard acceptance.
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, LayerId};
use cad_ui_slint::{
    offscreen, LayerPanelState, LayerRowUi, UiAdapter, UiCommandSink, UiConfiguration,
};
use slint::Model;
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Trace {
    commands: usize,
    batches: Vec<Vec<(LayerId, bool)>>,
    reject_next: bool,
}

struct Sink(Rc<RefCell<Trace>>);
impl UiCommandSink for Sink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        let mut trace = self.0.borrow_mut();
        trace.commands += 1;
        if std::mem::take(&mut trace.reject_next) {
            return Err(CadError::InvalidInput("synthetic batch refusal".into()));
        }
        if command.id == CommandId::SetLayerVisibilities {
            match command.payload {
                CommandPayload::LayerVisibilities(changes) => trace.batches.push(changes),
                _ => panic!("batch callback must send the typed layer payload"),
            }
        }
        Ok(())
    }
}

#[test]
fn completion_is_insert_only_and_layer_controls_send_one_full_identity_batch() {
    offscreen::install().unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink(trace.clone()), true).unwrap();
    let ui = adapter.component();
    ui.invoke_command_input_edited("mea dis".into());
    assert_eq!(ui.get_command_completion_items().row_count(), 0);
    ui.invoke_command_input_edited("measure dis".into());
    assert_eq!(
        ui.get_command_completion_items().row_data(0).unwrap(),
        "MEASURE DISTANCE"
    );
    assert_eq!(trace.borrow().commands, 0);
    ui.invoke_command_completion_dismissed();
    assert_eq!(ui.get_command_completion_items().row_count(), 0);
    ui.set_work_mode(false);
    ui.invoke_command_completion_context_changed();
    ui.invoke_command_input_edited("measure".into());
    assert_eq!(ui.get_command_completion_items().row_count(), 0);
    ui.invoke_command_input_edited("zoom".into());
    assert_eq!(ui.get_command_completion_items().row_count(), 3);
    assert_eq!(trace.borrow().commands, 0);

    let ids = [LayerId(u128::MAX), LayerId(1_u128 << 80)];
    adapter
        .handle()
        .set_layer_state(
            &LayerPanelState {
                rows: vec![
                    LayerRowUi {
                        id: 1,
                        name: "Walls".into(),
                        visible: true,
                        overridden: false,
                    },
                    LayerRowUi {
                        id: 2,
                        name: "Roof".into(),
                        visible: true,
                        overridden: false,
                    },
                ],
                override_count: 0,
                empty_label: String::new(),
            },
            &ids,
        )
        .unwrap();
    ui.set_layer_search_text("Walls".into());
    ui.invoke_layer_search_edited("Walls".into());
    ui.invoke_layers_visibility_requested(false);
    assert_eq!(trace.borrow().commands, 1);
    assert_eq!(
        trace.borrow().batches,
        vec![vec![(ids[0], false), (ids[1], false)]]
    );
    assert!(
        ui.get_layer_rows().row_data(0).unwrap().visible,
        "only host data may publish visibility"
    );
    trace.borrow_mut().reject_next = true;
    ui.invoke_layers_visibility_requested(true);
    assert_eq!(trace.borrow().batches.len(), 1);
    assert!(ui.get_command_expanded());
    assert!(ui.get_status_label().contains("synthetic batch refusal"));
}
