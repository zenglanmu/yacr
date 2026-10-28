//! The per-command equivalence matrix: every wire command, driven through the
//! desktop queue sink and the `dom-canvas` direct sink with the same request and
//! compared.
//!
//! See the parent module (`direct::tests`) for what is compared and why, and for
//! the honest limits of the harness.

use super::*;

// ---------------------------------------------------------------------------
// The matrix: every wire command, valid / malformed / session-refused
// ---------------------------------------------------------------------------

/// The CAD-enabled, no-modal-popup gate inputs of a live host.
const LIVE: (bool, bool) = (true, false);
/// Probe mode (`--probe`): the CAD path is disabled.
const PROBE: (bool, bool) = (false, false);
/// A modal popup owns input.
const MODAL: (bool, bool) = (true, true);

#[test]
fn every_wire_command_answers_identically_on_both_paths() {
    let cases: &[(&str, &str, &str, (bool, bool))] = &[
        // ---- cad_command -------------------------------------------------
        (
            "command/valid",
            "cad_command",
            r#"{"command":"LINE"}"#,
            LIVE,
        ),
        ("command/alias", "cad_command", r#"{"command":"l"}"#, LIVE),
        (
            "command/argument",
            "cad_command",
            r#"{"command":"snapunit 5"}"#,
            LIVE,
        ),
        (
            "command/malformed",
            "cad_command",
            r#"{"command":"FLOOPY"}"#,
            LIVE,
        ),
        ("command/missing-arg", "cad_command", r#"{}"#, LIVE),
        (
            "command/wrong-type",
            "cad_command",
            r#"{"command":7}"#,
            LIVE,
        ),
        (
            "command/probe-mode",
            "cad_command",
            r#"{"command":"LINE"}"#,
            PROBE,
        ),
        (
            "command/modal",
            "cad_command",
            r#"{"command":"LINE"}"#,
            MODAL,
        ),
        // ---- cad_set_tool_parameter --------------------------------------
        (
            "tool-parameter/valid",
            "cad_set_tool_parameter",
            r#"{"parameter":"polygonSides","number":4}"#,
            LIVE,
        ),
        (
            "tool-parameter/flag",
            "cad_set_tool_parameter",
            r#"{"parameter":"alignScale","flag":true}"#,
            LIVE,
        ),
        (
            "tool-parameter/unknown-key",
            "cad_set_tool_parameter",
            r#"{"parameter":"nope","number":1}"#,
            LIVE,
        ),
        (
            "tool-parameter/no-value",
            "cad_set_tool_parameter",
            r#"{"parameter":"polygonSides"}"#,
            LIVE,
        ),
        (
            "tool-parameter/both-values",
            "cad_set_tool_parameter",
            r#"{"parameter":"polygonSides","number":1,"flag":true}"#,
            LIVE,
        ),
        // ---- cad_set_tool_text -------------------------------------------
        (
            "tool-text/valid",
            "cad_set_tool_text",
            r#"{"text":"hello"}"#,
            LIVE,
        ),
        (
            "tool-text/empty",
            "cad_set_tool_text",
            r#"{"text":"   "}"#,
            LIVE,
        ),
        ("tool-text/missing", "cad_set_tool_text", r#"{}"#, LIVE),
        // ---- cad_set_selected_text ---------------------------------------
        (
            "selected-text/valid",
            "cad_set_selected_text",
            r#"{"text":"hello"}"#,
            LIVE,
        ),
        (
            "selected-text/empty",
            "cad_set_selected_text",
            r#"{"text":""}"#,
            LIVE,
        ),
        // ---- cad_set_selected_property -----------------------------------
        (
            "selected-property/valid",
            "cad_set_selected_property",
            r#"{"key":"layer","value":"0"}"#,
            LIVE,
        ),
        (
            "selected-property/unknown-key",
            "cad_set_selected_property",
            r#"{"key":"bogus","value":"x"}"#,
            LIVE,
        ),
        (
            "selected-property/modal",
            "cad_set_selected_property",
            r#"{"key":"layer","value":"0"}"#,
            MODAL,
        ),
        // ---- cad_standard_view -------------------------------------------
        (
            "standard-view/valid",
            "cad_standard_view",
            r#"{"view":"isometric"}"#,
            LIVE,
        ),
        (
            "standard-view/case",
            "cad_standard_view",
            r#"{"view":"TOP"}"#,
            LIVE,
        ),
        (
            "standard-view/unknown",
            "cad_standard_view",
            r#"{"view":"nope"}"#,
            LIVE,
        ),
        (
            "standard-view/empty",
            "cad_standard_view",
            r#"{"view":""}"#,
            LIVE,
        ),
        (
            "standard-view/modal",
            "cad_standard_view",
            r#"{"view":"top"}"#,
            MODAL,
        ),
        // ---- cad_set_setting ---------------------------------------------
        (
            "setting/valid",
            "cad_set_setting",
            r#"{"key":"grid","value":false}"#,
            LIVE,
        ),
        (
            "setting/real",
            "cad_set_setting",
            r#"{"key":"snapSpacing","value":2.5}"#,
            LIVE,
        ),
        (
            "setting/unknown-key",
            "cad_set_setting",
            r#"{"key":"notAKey","value":true}"#,
            LIVE,
        ),
        (
            "setting/invalid-value",
            "cad_set_setting",
            r#"{"key":"grid","value":2.5}"#,
            LIVE,
        ),
        (
            "setting/probe-mode",
            "cad_set_setting",
            r#"{"key":"grid","value":false}"#,
            PROBE,
        ),
        // ---- layer writes -------------------------------------------------
        (
            "layer-visible/valid",
            "cad_set_layer_visible",
            r#"{"name":"0","visible":false}"#,
            LIVE,
        ),
        (
            "layer-visible/unknown-layer",
            "cad_set_layer_visible",
            r#"{"name":"NOPE","visible":true}"#,
            LIVE,
        ),
        (
            "layer-visible/missing-arg",
            "cad_set_layer_visible",
            r#"{"name":"0"}"#,
            LIVE,
        ),
        (
            "layer-visible/modal",
            "cad_set_layer_visible",
            r#"{"name":"0","visible":true}"#,
            MODAL,
        ),
        ("restore-layers/valid", "cad_restore_layers", "{}", LIVE),
        ("restore-layers/modal", "cad_restore_layers", "{}", MODAL),
        (
            "active-layer/valid",
            "cad_set_active_layer",
            r#"{"name":"0"}"#,
            LIVE,
        ),
        (
            "active-layer/unknown",
            "cad_set_active_layer",
            r#"{"name":"NOPE"}"#,
            LIVE,
        ),
        (
            "create-layer/valid",
            "cad_create_layer",
            r#"{"name":"DOORS"}"#,
            LIVE,
        ),
        (
            "create-layer/empty-name",
            "cad_create_layer",
            r#"{"name":"   "}"#,
            LIVE,
        ),
        (
            "create-layer/duplicate",
            "cad_create_layer",
            r#"{"name":"0"}"#,
            LIVE,
        ),
        (
            "rename-layer/valid",
            "cad_rename_layer",
            r#"{"layer":"0","name":"BASE"}"#,
            LIVE,
        ),
        (
            "rename-layer/unknown",
            "cad_rename_layer",
            r#"{"layer":"NOPE","name":"X"}"#,
            LIVE,
        ),
        (
            "delete-layer/default",
            "cad_delete_layer",
            r#"{"layer":"0"}"#,
            LIVE,
        ),
        (
            "delete-layer/unknown",
            "cad_delete_layer",
            r#"{"layer":"NOPE"}"#,
            LIVE,
        ),
        (
            "layer-property/valid",
            "cad_set_layer_property",
            r#"{"layer":"0","key":"color","value":"aci:1"}"#,
            LIVE,
        ),
        (
            "layer-property/unsupported-key",
            "cad_set_layer_property",
            r#"{"layer":"0","key":"plotstyle","value":"Normal"}"#,
            LIVE,
        ),
        (
            "layer-property/unknown-layer",
            "cad_set_layer_property",
            r#"{"layer":"NOPE","key":"color","value":"aci:1"}"#,
            LIVE,
        ),
        // ---- popups -------------------------------------------------------
        (
            "open-popup/no-host",
            "cad_open_popup",
            r#"{"kind":"about"}"#,
            LIVE,
        ),
        ("close-popup/no-host", "cad_close_popup", "{}", LIVE),
    ];

    let (mut gate_refused, mut applied_ok, mut session_refused) = (0, 0, 0);
    for (case, name, args, gates) in cases {
        let (desktop, _canvas) = compare(case, name, args, *gates);
        match desktop.admission {
            Admission::Refused(_) => gate_refused += 1,
            Admission::Admitted => match desktop.applied {
                Ok(()) => applied_ok += 1,
                Err(_) => session_refused += 1,
            },
        }
    }

    // Coverage: the matrix must really exercise all three verdict shapes, or an
    // "everything is refused identically" suite would pass while proving nothing.
    assert!(gate_refused > 0, "no case was refused by the shared gate");
    assert!(applied_ok > 0, "no case was admitted and applied");
    assert!(
        session_refused > 0,
        "no case was admitted by the gate and refused by the session"
    );
}

/// The two reads answer the same object on both paths: the desktop from its
/// owner-tick mirror, the direct sink from the session itself.
#[test]
fn the_control_reads_answer_the_same_object_on_both_paths() {
    let session = session();
    let settings = session.settings();
    let layers = session.layers();
    let projected = layers.clone().expect("the fixture projects its layers");
    let mirrored = QueueReads::new(Some(&settings), Some(&layers));
    let direct = DirectReads::new(&session);

    assert_eq!(
        wire::settings(&mirrored).expect("a mirrored snapshot"),
        wire::settings(&direct).expect("the session's own snapshot")
    );
    assert_eq!(
        wire::layers(&mirrored).expect("a mirrored projection"),
        wire::layers(&direct).expect("the session's own projection")
    );

    // The dispatcher serves the same JSON for both sources.
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let (mirrored_settings, direct_settings, mirrored_layers, direct_layers) = {
        let mut sink = queues.sink();
        (
            dispatch("cad_settings", "{}", &mut sink, &mirrored, &mut popups)
                .expect("the mirrored settings read"),
            dispatch("cad_settings", "{}", &mut sink, &direct, &mut popups)
                .expect("the direct settings read"),
            dispatch("cad_layers", "{}", &mut sink, &mirrored, &mut popups)
                .expect("the mirrored layers read"),
            dispatch("cad_layers", "{}", &mut sink, &direct, &mut popups)
                .expect("the direct layers read"),
        )
    };
    assert_eq!(mirrored_settings, direct_settings);
    assert_eq!(mirrored_layers, direct_layers);
    assert_eq!(
        direct_settings,
        serde_json::to_value(&settings).expect("serializes")
    );
    assert_eq!(
        direct_layers,
        serde_json::to_value(&projected).expect("serializes")
    );
}

/// Before the first owner tick there is no mirror: the read is the explicit
/// `cad_waiting`, never a fabricated default object.
#[test]
fn the_mirrorless_read_answers_cad_waiting() {
    let empty = QueueReads::default();
    assert_eq!(
        wire::settings(&empty).unwrap_err(),
        cad_ui_core::session::CAD_WAITING
    );
    assert_eq!(
        wire::layers(&empty).unwrap_err(),
        cad_ui_core::session::CAD_WAITING
    );
}

/// A mirror refusal is relayed, never replaced by stale rows.
#[test]
fn a_mirror_refusal_is_relayed() {
    let refused: Result<CadLayers, String> = Err("layer.projection_failed: boom".to_string());
    let reads = QueueReads::new(None, Some(&refused));
    assert_eq!(
        wire::layers(&reads).unwrap_err(),
        "layer.projection_failed: boom"
    );
}

/// Both hosts advertise the **same** command vocabulary and alias table, from the
/// same tables — that is the point of the round.
#[test]
fn both_hosts_advertise_the_same_vocabulary() {
    let expected_commands: Value = serde_json::json!(cad_ui_core::session::admissible_names());
    let expected_aliases: Value = serde_json::json!(cad_ui_core::session::alias_pairs()
        .into_iter()
        .map(|(alias, command)| serde_json::json!({"alias": alias, "command": command}))
        .collect::<Vec<_>>());

    assert_eq!(serde_json::json!(wire::commands()), expected_commands);
    assert_eq!(serde_json::json!(wire::aliases()), expected_aliases);

    // …and through the dispatcher, on both sinks.
    let mut queues = Queues::new(true, false);
    let reads = QueueReads::default();
    let mut popups = NoPopups;
    let (commands, aliases) = {
        let mut sink = queues.sink();
        (
            dispatch("cad_commands", "{}", &mut sink, &reads, &mut popups),
            dispatch("cad_command_aliases", "{}", &mut sink, &reads, &mut popups),
        )
    };
    let session = session();
    let mut direct = DirectSink::new(&session);
    let direct_reads = DirectReads::new(&session);
    assert_eq!(
        commands.expect("the queue path's vocabulary"),
        expected_commands
    );
    assert_eq!(aliases.expect("the queue path's aliases"), expected_aliases);
    assert_eq!(
        dispatch(
            "cad_commands",
            "{}",
            &mut direct,
            &direct_reads,
            &mut popups
        )
        .expect("the direct path's vocabulary"),
        expected_commands
    );
    assert_eq!(
        dispatch(
            "cad_command_aliases",
            "{}",
            &mut direct,
            &direct_reads,
            &mut popups
        )
        .expect("the direct path's aliases"),
        expected_aliases
    );
}
