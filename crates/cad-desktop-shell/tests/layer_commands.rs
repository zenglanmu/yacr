//! The layer-panel IPC surface: the read's explicit "no session" answer and the
//! exact camelCase wire shape the React panel codes against.
//!
//! The three mutations are admission-only on this side (the ack means
//! **queued**), and their real resolution/refusals are the owner-thread session's
//! authority (`cad-ui-core` session tests). What is pinned here is the wire the
//! panel actually consumes.

use cad_desktop_shell::commands::{cad_layers_impl, COMMAND_NAMES};
use cad_desktop_shell::state::{lock, new_shared};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::session::{CadLayer, CadLayers};
use serde_json::json;

#[test]
fn the_registered_command_set_includes_the_four_layer_commands() {
    for name in [
        "cad_layers",
        "cad_set_layer_visible",
        "cad_restore_layers",
        "cad_set_active_layer",
    ] {
        assert!(
            COMMAND_NAMES.contains(&name),
            "{name} is not registered in cad-desktop-shell"
        );
    }
}

#[test]
fn cad_layers_refuses_explicitly_before_a_session_exists() {
    let shared = new_shared(LaunchOptions::default());
    let error = cad_layers_impl(&shared).expect_err("no session yet");
    assert_eq!(error, "cad_waiting");
}

#[test]
fn cad_layers_serializes_the_mirrored_snapshot_in_camel_case() {
    let shared = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&shared);
        state.cad_layers = Some(Ok(CadLayers {
            layers: vec![
                CadLayer {
                    name: "0".into(),
                    visible: true,
                    database_visible: true,
                    overridden: false,
                    current: true,
                },
                CadLayer {
                    name: "WALLS".into(),
                    visible: false,
                    database_visible: true,
                    overridden: true,
                    current: false,
                },
            ],
            active: Some("0".into()),
            overridden: 1,
        }));
    }

    let layers = cad_layers_impl(&shared).expect("the mirrored snapshot");
    let value = serde_json::to_value(&layers).expect("serializes");
    assert_eq!(
        value,
        json!({
            "layers": [
                {
                    "name": "0",
                    "visible": true,
                    "databaseVisible": true,
                    "overridden": false,
                    "current": true
                },
                {
                    "name": "WALLS",
                    "visible": false,
                    "databaseVisible": true,
                    "overridden": true,
                    "current": false
                }
            ],
            "active": "0",
            "overridden": 1
        })
    );
}

#[test]
fn an_empty_cad_layers_answer_keeps_every_key() {
    let value = serde_json::to_value(CadLayers::empty()).expect("serializes");
    assert_eq!(
        value,
        json!({"layers": [], "active": null, "overridden": 0})
    );
}

#[test]
fn a_projection_error_is_relayed_not_replaced_by_stale_rows() {
    let shared = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&shared);
        state.cad_layers = Some(Err("layer.projection_failed: boom".into()));
    }
    let error = cad_layers_impl(&shared).expect_err("an error is not an empty success");
    assert!(error.contains("layer.projection_failed"), "{error}");
}
