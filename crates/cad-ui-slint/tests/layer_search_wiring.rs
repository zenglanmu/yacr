#![cfg(target_os = "linux")]
//! Synthetic adapter wiring contracts, not runtime or rendering acceptance.
use cad_app::viewer_config::ViewerConfig;
use cad_domain::{CadResult, LayerId};
use cad_ui_slint::{
    offscreen, LayerPanelState, LayerRowUi, UiAdapter, UiCommandSink, UiConfiguration,
};
use slint::Model;

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

#[test]
fn search_refreshes_on_model_replacement_and_config_changes_reach_keyboard_gate() {
    offscreen::install().unwrap();
    let adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    let ui = adapter.component();
    let state = LayerPanelState {
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
                visible: false,
                overridden: true,
            },
        ],
        override_count: 1,
        empty_label: String::new(),
    };
    adapter
        .handle()
        .set_layer_state(&state, &[LayerId(1), LayerId(2)])
        .unwrap();
    assert_eq!(ui.get_layer_matching_count(), 2);
    ui.set_layer_search_text("ROO".into());
    ui.invoke_layer_search_edited("ROO".into());
    assert_eq!(ui.get_layer_matching_count(), 1);
    assert_eq!(ui.get_layer_rows().row_count(), 2);
    assert_eq!(ui.get_layer_rows().row_data(1).unwrap().name, "Roof");
    adapter
        .handle()
        .set_layer_state(&LayerPanelState::default(), &[])
        .unwrap();
    assert_eq!(ui.get_layer_matching_count(), 0);
    adapter
        .handle()
        .set_layer_state(&state, &[LayerId(1), LayerId(2)])
        .unwrap();
    assert_eq!(ui.get_layer_matching_count(), 1);
    ui.set_layer_search_text("".into());
    ui.invoke_layer_search_edited("".into());
    assert_eq!(ui.get_layer_matching_count(), 2);

    let mut config = ViewerConfig::default();
    config.interaction.keyboard_shortcuts = false;
    adapter.handle().set_config(config.clone()).unwrap();
    assert!(!ui.get_keyboard_shortcuts_enabled());
    config.interaction.keyboard_shortcuts = true;
    adapter.handle().set_config(config).unwrap();
    assert!(ui.get_keyboard_shortcuts_enabled());
}
