//! The layer-panel IPC surface: the read's explicit "no session" answer, the
//! exact camelCase wire shape the React panel codes against, and the admission of
//! the seven layer mutations into the one bounded layer-write queue.
//!
//! Every mutation is admission-only on this side (the ack means **queued**), and
//! their real resolution/refusals are the owner-thread session's authority
//! (`cad-ui-core` session tests: `create_layer_adds_a_row_in_one_revision_bump_and_no_undo_entry`,
//! `rename_layer_resolves_under_the_new_name_and_not_the_old_one`,
//! `delete_layer_in_use_is_refused_without_changing_the_projection`,
//! `deleting_the_active_layer_makes_zero_current_again`,
//! `delete_of_the_default_layer_is_refused`,
//! `a_layer_attribute_write_reads_back_through_the_projection`). This file cannot
//! rebuild those: a `CadSession` needs `cad-engine`/`cad-platform` types, which
//! the shared shell is forbidden to depend on. What is pinned here is the wire and
//! the queue the panel actually drives.

use std::sync::Arc;

use cad_desktop_shell::commands::{
    cad_create_layer_impl, cad_delete_layer_impl, cad_layers_impl, cad_rename_layer_impl,
    cad_set_layer_property_impl, COMMAND_NAMES,
};
use cad_desktop_shell::state::{lock, new_shared, Shared};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::session::{CadLayer, CadLayerColor, CadLayerWrite, CadLayers, CadPopup};
use serde_json::json;

/// A launch in CAD mode (the default): probe mode is the opt-in.
fn cad_launch() -> LaunchOptions {
    LaunchOptions::default()
}

/// The queued layer writes, in admission order.
fn queued(shared: &Arc<std::sync::Mutex<Shared>>) -> Vec<CadLayerWrite> {
    lock(shared).cad_layer_writes.iter().cloned().collect()
}

#[test]
fn the_registered_command_set_includes_the_eight_layer_commands() {
    for name in [
        "cad_layers",
        "cad_set_layer_visible",
        "cad_restore_layers",
        "cad_set_active_layer",
        "cad_create_layer",
        "cad_rename_layer",
        "cad_delete_layer",
        "cad_set_layer_property",
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
                    color: CadLayerColor {
                        aci: Some(7),
                        rgb: "#ffffff".into(),
                    },
                    linetype: "Continuous".into(),
                    lineweight: "ByLayer".into(),
                    transparency: "0".into(),
                    locked: false,
                    frozen: false,
                    off: false,
                    plottable: true,
                },
                CadLayer {
                    name: "WALLS".into(),
                    visible: false,
                    database_visible: true,
                    overridden: true,
                    current: false,
                    color: CadLayerColor {
                        aci: None,
                        rgb: "#ff7f00".into(),
                    },
                    linetype: "DASHED".into(),
                    lineweight: "35".into(),
                    transparency: "40".into(),
                    locked: true,
                    frozen: true,
                    off: false,
                    plottable: false,
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
                    "current": true,
                    "color": {"aci": 7, "rgb": "#ffffff"},
                    "linetype": "Continuous",
                    "lineweight": "ByLayer",
                    "transparency": "0",
                    "locked": false,
                    "frozen": false,
                    "off": false,
                    "plottable": true
                },
                {
                    "name": "WALLS",
                    "visible": false,
                    "databaseVisible": true,
                    "overridden": true,
                    "current": false,
                    "color": {"aci": null, "rgb": "#ff7f00"},
                    "linetype": "DASHED",
                    "lineweight": "35",
                    "transparency": "40",
                    "locked": true,
                    "frozen": true,
                    "off": false,
                    "plottable": false
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

#[test]
fn the_structural_layer_writes_are_queued_on_the_one_layer_channel() {
    let shared = new_shared(cad_launch());

    // The ack is "queued", never "applied": `accepted` is the whole promise.
    let ack = cad_create_layer_impl(&shared, "DOORS".into()).expect("a create is admitted");
    assert!(ack.accepted);
    cad_rename_layer_impl(&shared, "DOORS".into(), "PARTITIONS".into())
        .expect("a rename is admitted");
    cad_delete_layer_impl(&shared, "PARTITIONS".into()).expect("a delete is admitted");

    // One queue, admission order, the panel's own names unchanged (no trimming or
    // folding here — the database validates on the owner thread).
    assert_eq!(
        queued(&shared),
        vec![
            CadLayerWrite::Create {
                name: "DOORS".into()
            },
            CadLayerWrite::Rename {
                layer: "DOORS".into(),
                name: "PARTITIONS".into()
            },
            CadLayerWrite::Delete {
                layer: "PARTITIONS".into()
            },
        ]
    );
}

#[test]
fn the_attribute_write_is_queued_on_the_same_layer_channel() {
    let shared = new_shared(cad_launch());

    let ack = cad_set_layer_property_impl(&shared, "WALLS".into(), "color".into(), "aci:1".into())
        .expect("a supported key is admitted");
    assert!(ack.accepted);
    cad_set_layer_property_impl(&shared, "WALLS".into(), "off".into(), "true".into())
        .expect("a flag is admitted");

    // The one queue, in admission order, with the panel's own strings untouched
    // (the value grammar is the owner thread's call).
    assert_eq!(
        queued(&shared),
        vec![
            CadLayerWrite::SetProperty {
                layer: "WALLS".into(),
                key: "color".into(),
                value: "aci:1".into()
            },
            CadLayerWrite::SetProperty {
                layer: "WALLS".into(),
                key: "off".into(),
                value: "true".into()
            },
        ]
    );
}

#[test]
fn an_unsupported_layer_property_key_is_refused_before_the_queue() {
    let shared = new_shared(cad_launch());

    // `plotstyle` is a key the protocol names but refuses as not implemented (the
    // writer only emits a style handle), so it is refused statically like an
    // unknown key, and nothing reaches the queue.
    for key in ["plotstyle", "hatch"] {
        let error =
            cad_set_layer_property_impl(&shared, "WALLS".into(), key.into(), "Normal".into())
                .expect_err("this key must be refused");
        assert!(error.contains("layer.property.unsupported"), "{error}");
        assert!(error.contains(key), "the refusal names the key: {error}");
    }
    assert!(queued(&shared).is_empty(), "nothing reached the queue");
}

#[test]
#[allow(deprecated)] // the frozen `cad_status` wire label is built by hand here
fn an_open_modal_refuses_an_attribute_write_before_it_is_queued() {
    let shared = new_shared(cad_launch());
    {
        let mut state = lock(&shared);
        state.shell.popup = Some(CadPopup {
            kind: "about".into(),
            label: "popup-1".into(),
            modal: true,
        });
    }

    let error = cad_set_layer_property_impl(&shared, "WALLS".into(), "locked".into(), "1".into())
        .expect_err("a modal popup owns input, so no CAD mutation is queued");
    assert_eq!(error, "popup.modal_open");
    assert!(queued(&shared).is_empty(), "nothing reached the queue");
}

#[test]
fn probe_mode_refuses_an_attribute_write_explicitly() {
    let shared = new_shared(LaunchOptions {
        probe_only: true,
        ..LaunchOptions::default()
    });

    let error = cad_set_layer_property_impl(&shared, "WALLS".into(), "locked".into(), "1".into())
        .expect_err("probe mode has no CAD");
    assert!(error.contains("probe mode"), "{error}");
    assert!(queued(&shared).is_empty(), "nothing reached the queue");
}

#[test]
#[allow(deprecated)] // the frozen `cad_status` wire label is built by hand here
fn an_open_modal_refuses_a_structural_layer_write_before_it_is_queued() {
    let shared = new_shared(cad_launch());
    {
        let mut state = lock(&shared);
        state.shell.popup = Some(CadPopup {
            kind: "about".into(),
            label: "popup-1".into(),
            modal: true,
        });
    }

    let error = cad_create_layer_impl(&shared, "DOORS".into())
        .expect_err("a modal popup owns input, so no CAD mutation is queued");
    assert_eq!(error, "popup.modal_open");
    assert!(queued(&shared).is_empty(), "nothing reached the queue");
}

#[test]
fn probe_mode_refuses_a_structural_layer_write_explicitly() {
    let shared = new_shared(LaunchOptions {
        probe_only: true,
        ..LaunchOptions::default()
    });

    let error = cad_delete_layer_impl(&shared, "WALLS".into()).expect_err("probe mode has no CAD");
    assert!(error.contains("probe mode"), "{error}");
    assert!(queued(&shared).is_empty(), "nothing reached the queue");
}
