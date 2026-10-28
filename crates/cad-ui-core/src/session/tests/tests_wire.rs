//! The shared command-semantics layer (`session::wire`).
//!
//! What is pinned here is the part no host can differ on:
//!
//! * the acknowledgement shape and the vocabulary/alias tables (one source);
//! * the **queue bound**, enforced through [`AdmissionTarget::pending`] — the one
//!   refusal a synchronous sink structurally cannot reach, so it has to be pinned
//!   against the queue sink itself;
//! * the two control reads: the explicit `cad_waiting` before any mirror exists and
//!   a relayed mirror refusal (never stale rows).
//!
//! The cross-host equivalence proof (queue sink vs. direct sink, every command) is
//! `apps/app-web/src/direct/tests.rs`, where a real `CadSession` is constructible.

use super::super::wire::{self, AdmissionTarget, CadCommandAck, Channel, QueueFamily, QueueReads};
use super::super::{
    CadLayerWrite, CadLayers, CadSettingKey, CadSettingValue, CAD_LAYER_WRITE_CAP,
    CAD_SELECTED_TEXT_CAP, CAD_TOOL_PARAMETER_CAP,
};
use cad_app::{StandardView, ToolValue};

/// A queue-sink harness: the desktop's bounded family with explicit gate inputs.
struct Queues {
    tool_parameters: Vec<(cad_app::ToolParameter, ToolValue)>,
    tool_texts: Vec<String>,
    selected_texts: Vec<String>,
    selected_properties: Vec<(String, String)>,
    standard_views: std::collections::VecDeque<StandardView>,
    setting_writes: std::collections::VecDeque<(CadSettingKey, CadSettingValue)>,
    layer_writes: std::collections::VecDeque<CadLayerWrite>,
    commands: Vec<String>,
    cad_mode: bool,
    modal_open: bool,
}

impl Queues {
    fn new(cad_mode: bool, modal_open: bool) -> Self {
        Self {
            tool_parameters: Vec::new(),
            tool_texts: Vec::new(),
            selected_texts: Vec::new(),
            selected_properties: Vec::new(),
            standard_views: std::collections::VecDeque::new(),
            setting_writes: std::collections::VecDeque::new(),
            layer_writes: std::collections::VecDeque::new(),
            commands: Vec::new(),
            cad_mode,
            modal_open,
        }
    }

    fn sink(&mut self) -> wire::QueueSink<'_> {
        wire::QueueSink::new(
            self.cad_mode,
            self.modal_open,
            QueueFamily {
                commands: &mut self.commands,
                tool_parameters: &mut self.tool_parameters,
                tool_texts: &mut self.tool_texts,
                selected_texts: &mut self.selected_texts,
                selected_properties: &mut self.selected_properties,
                standard_views: &mut self.standard_views,
                setting_writes: &mut self.setting_writes,
                layer_writes: &mut self.layer_writes,
            },
        )
    }
}

/// The acknowledgement is the frozen `{accepted:true}` object, and nothing else.
#[test]
fn the_acknowledgement_is_accepted_true() {
    let ack = CadCommandAck::accepted();
    assert!(ack.accepted);
    assert_eq!(
        serde_json::to_value(ack).expect("serializes"),
        serde_json::json!({"accepted": true})
    );
}

/// Both vocabulary answers come from the one source table.
#[test]
fn the_vocabulary_and_aliases_are_the_shared_tables() {
    assert_eq!(wire::commands(), super::super::admissible_names());
    let expected: Vec<wire::CadCommandAlias> = super::super::alias_pairs()
        .into_iter()
        .map(|(alias, command)| wire::CadCommandAlias {
            alias: alias.to_string(),
            command: command.to_string(),
        })
        .collect();
    assert_eq!(wire::aliases(), expected);
    // The wire shape is the frozen camelCase `{alias, command}` row.
    assert_eq!(
        serde_json::to_value(&expected[0]).expect("serializes"),
        serde_json::json!({"alias": expected[0].alias, "command": expected[0].command})
    );
}

/// The **queue bound** is a shared rule applied through the sink's occupancy: a
/// full queue is refused with the same string the concrete-queue `admit_*` uses,
/// before anything is stored.
#[test]
fn a_full_queue_is_refused_through_the_sink_occupancy() {
    // The command channel: full at `CAD_COMMAND_CAP`.
    let mut queues = Queues::new(true, false);
    {
        let mut sink = queues.sink();
        for _ in 1..super::super::super::shell::CAD_COMMAND_CAP {
            wire::command(&mut sink, "FIT").expect("within capacity");
        }
        assert_eq!(
            sink.pending(Channel::Command),
            super::super::super::shell::CAD_COMMAND_CAP - 1
        );
        wire::command(&mut sink, "FIT").expect("the bound is accepted");
        let refusal = wire::command(&mut sink, "FIT").expect_err("the queue is full");
        assert_eq!(refusal, "CAD command queue is full");
        assert_eq!(
            sink.pending(Channel::Command),
            super::super::super::shell::CAD_COMMAND_CAP,
            "a refused request never reaches the queue"
        );
    }
    assert_eq!(
        queues.commands.len(),
        super::super::super::shell::CAD_COMMAND_CAP
    );

    // The typed channels report their own bound and their own reason.
    let mut queues = Queues::new(true, false);
    {
        let mut sink = queues.sink();
        for _ in 0..CAD_LAYER_WRITE_CAP {
            wire::set_active_layer(&mut sink, "0").expect("within capacity");
        }
        assert_eq!(
            wire::set_active_layer(&mut sink, "0").expect_err("full"),
            "CAD layer queue is full"
        );
        for _ in 0..CAD_TOOL_PARAMETER_CAP {
            wire::set_tool_parameter(&mut sink, "polygonSides", Some(4.0), None)
                .expect("within capacity");
        }
        assert_eq!(
            wire::set_tool_parameter(&mut sink, "polygonSides", Some(4.0), None).expect_err("full"),
            "CAD tool-parameter queue is full"
        );
        for _ in 0..CAD_SELECTED_TEXT_CAP {
            wire::set_selected_text(&mut sink, "hello").expect("within capacity");
        }
        assert_eq!(
            wire::set_selected_text(&mut sink, "hello").expect_err("full"),
            "CAD selected-text queue is full"
        );
    }
}

/// The shared gate order is the one every host applies: modal popup gate first,
/// then probe mode (never the other way round), and a malformed request is refused
/// before either.
#[test]
fn the_gate_order_is_modal_then_probe_mode() {
    let mut queues = Queues::new(false, true);
    let mut sink = queues.sink();
    assert_eq!(
        wire::command(&mut sink, "FIT").expect_err("a popup owns input"),
        "popup.modal_open"
    );
    assert_eq!(
        wire::set_layer_visible(&mut sink, "0", false).expect_err("a popup owns input"),
        "popup.modal_open"
    );
    assert_eq!(
        wire::standard_view(&mut sink, "nope").expect_err("a bad view"),
        "view.standard_unknown: cad_standard_view has no view 'nope' (expected one of top, \
         bottom, front, back, left, right, isometric)"
    );

    // Same state, but the view is valid: the modal gate still wins over probe mode.
    assert_eq!(
        wire::standard_view(&mut sink, "top").expect_err("a popup owns input"),
        "popup.modal_open"
    );

    // With no popup, probe mode is the refusal.
    let mut probe = Queues::new(false, false);
    let mut sink = probe.sink();
    assert_eq!(
        wire::command(&mut sink, "FIT").expect_err("probe mode"),
        "CAD is disabled in probe mode"
    );
    // `cad_set_setting` is deliberately not behind the modal gate, so the Options
    // dialog can write while it is open; a probe-mode host still refuses it.
    assert_eq!(
        wire::set_setting(&mut sink, "grid", &serde_json::json!(false)).expect_err("probe mode"),
        "CAD is disabled in probe mode"
    );
}

/// The two reads are host-served: no mirror is the explicit `cad_waiting`, and a
/// mirror refusal is relayed unchanged.
#[test]
fn the_control_reads_are_explicit_before_and_after_a_mirror_exists() {
    let empty = QueueReads::default();
    assert_eq!(wire::settings(&empty).unwrap_err(), wire::CAD_WAITING);
    assert_eq!(wire::layers(&empty).unwrap_err(), wire::CAD_WAITING);

    let refusal: Result<CadLayers, String> = Err("layer.projection_failed: boom".to_string());
    let reads = QueueReads::new(None, Some(&refusal));
    assert_eq!(
        wire::layers(&reads).expect_err("relayed, not replaced"),
        "layer.projection_failed: boom"
    );
}

/// A host with no popup seam refuses both popup commands with the shared reason and
/// never fabricates an acknowledgement.
#[test]
fn a_host_without_popups_refuses_both_popup_commands() {
    let mut popups = wire::NoPopups;
    assert_eq!(
        wire::open_popup(&mut popups, "about").unwrap_err(),
        wire::POPUP_HOST_UNAVAILABLE
    );
    assert_eq!(
        wire::close_popup(&mut popups).unwrap_err(),
        wire::POPUP_HOST_UNAVAILABLE
    );
}
