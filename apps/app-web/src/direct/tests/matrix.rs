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
        // ---- cad_api_call (the first reflected plugin-API read) -----------
        (
            "api-call/document",
            "cad_api_call",
            r#"{"namespace":"ApplicationServices","operation":"documentManager.mdiActiveDocument"}"#,
            LIVE,
        ),
        (
            "api-call/parameters-ignored",
            "cad_api_call",
            r#"{"namespace":"ApplicationServices","operation":"documentManager.mdiActiveDocument","parameters":{"unused":[1,2]}}"#,
            LIVE,
        ),
        (
            "api-call/documents",
            "cad_api_call",
            r#"{"namespace":"ApplicationServices","operation":"documentManager.documents"}"#,
            LIVE,
        ),
        (
            "api-call/revision",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.revision","parameters":{"database":"1"}}"#,
            LIVE,
        ),
        (
            "api-call/revision-mismatch",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.revision","parameters":{"database":"999"}}"#,
            LIVE,
        ),
        (
            "api-call/revision-missing-parameter",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.revision"}"#,
            LIVE,
        ),
        (
            "api-call/symbol-tables",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.symbolTables","parameters":{"database":"1"}}"#,
            LIVE,
        ),
        (
            "api-call/symbol-tables-mismatch",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.symbolTables","parameters":{"database":"999"}}"#,
            LIVE,
        ),
        (
            "api-call/symbol-tables-missing-parameter",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"database.symbolTables"}"#,
            LIVE,
        ),
        (
            "api-call/current-selection",
            "cad_api_call",
            r#"{"namespace":"EditorInput","operation":"editor.currentSelection"}"#,
            LIVE,
        ),
        (
            "api-call/current-selection-parameters-ignored",
            "cad_api_call",
            r#"{"namespace":"EditorInput","operation":"editor.currentSelection","parameters":{"unused":[1,2]}}"#,
            LIVE,
        ),
        (
            "api-call/editor-unknown-operation",
            "cad_api_call",
            r#"{"namespace":"EditorInput","operation":"editor.notAnOperation"}"#,
            LIVE,
        ),
        // ---- the mutating lane: submit acknowledges, poll answers -----------
        (
            "api-submit/accepted",
            "cad_api_submit",
            r#"{"requestId":"matrix-1","namespace":"DatabaseServices","operation":"transaction.commit"}"#,
            LIVE,
        ),
        (
            "api-submit/bad-id",
            "cad_api_submit",
            r#"{"requestId":"bad id","namespace":"DatabaseServices","operation":"transaction.commit"}"#,
            LIVE,
        ),
        (
            "api-submit/missing-id",
            "cad_api_submit",
            r#"{"namespace":"DatabaseServices","operation":"transaction.commit"}"#,
            LIVE,
        ),
        ("api-poll/empty", "cad_api_poll", "{}", LIVE),
        (
            "api-call/unknown-operation",
            "cad_api_call",
            r#"{"namespace":"ApplicationServices","operation":"documentManager.notAnOperation"}"#,
            LIVE,
        ),
        (
            "api-call/unknown-namespace",
            "cad_api_call",
            r#"{"namespace":"DatabaseServices","operation":"documentManager.mdiActiveDocument"}"#,
            LIVE,
        ),
        (
            "api-call/empty-namespace",
            "cad_api_call",
            r#"{"namespace":"","operation":"documentManager.mdiActiveDocument"}"#,
            LIVE,
        ),
        (
            "api-call/missing-arg",
            "cad_api_call",
            r#"{"namespace":"ApplicationServices"}"#,
            LIVE,
        ),
    ];

    let (mut gate_refused, mut applied_ok, mut session_refused) = (0, 0, 0);
    // The object-read cases live in the mutating lane's module so this file stays
    // under the physical-line gate; they are driven through the same harness.
    for (case, name, args, gates) in cases.iter().chain(api::OBJECT_READ_CASES.iter()) {
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
    let fonts = session.fonts();
    let api_snapshot = session.api_snapshot();
    let projected = layers.clone().expect("the fixture projects its layers");
    let projected_fonts = fonts.clone().expect("the fixture projects its fonts");
    let mirrored = QueueReads::new(
        Some(&settings),
        Some(&layers),
        Some(&fonts),
        Some(&api_snapshot),
    );
    let direct = DirectReads::new(&session);

    assert_eq!(
        wire::settings(&mirrored).expect("a mirrored snapshot"),
        wire::settings(&direct).expect("the session's own snapshot")
    );
    assert_eq!(
        wire::layers(&mirrored).expect("a mirrored projection"),
        wire::layers(&direct).expect("the session's own projection")
    );
    assert_eq!(
        wire::fonts(&mirrored).expect("a mirrored font registry"),
        wire::fonts(&direct).expect("the session's own font registry")
    );
    assert_eq!(
        wire::api_call(
            &mirrored,
            "ApplicationServices",
            "documentManager.mdiActiveDocument",
            None
        )
        .expect("a mirrored document snapshot"),
        wire::api_call(
            &direct,
            "ApplicationServices",
            "documentManager.mdiActiveDocument",
            None
        )
        .expect("the session's own document snapshot")
    );

    // The dispatcher serves the same JSON for both sources.
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let (
        mirrored_settings,
        direct_settings,
        mirrored_layers,
        direct_layers,
        mirrored_fonts,
        direct_fonts,
        mirrored_document,
        direct_document,
    ) = {
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
            dispatch("cad_fonts", "{}", &mut sink, &mirrored, &mut popups)
                .expect("the mirrored font read"),
            dispatch("cad_fonts", "{}", &mut sink, &direct, &mut popups)
                .expect("the direct font read"),
            dispatch(
                "cad_api_call",
                r#"{"namespace":"ApplicationServices","operation":"documentManager.mdiActiveDocument"}"#,
                &mut sink,
                &mirrored,
                &mut popups,
            )
            .expect("the mirrored document read"),
            dispatch(
                "cad_api_call",
                r#"{"namespace":"ApplicationServices","operation":"documentManager.mdiActiveDocument"}"#,
                &mut sink,
                &direct,
                &mut popups,
            )
            .expect("the direct document read"),
        )
    };
    assert_eq!(mirrored_settings, direct_settings);
    assert_eq!(mirrored_layers, direct_layers);
    assert_eq!(mirrored_fonts, direct_fonts);
    assert_eq!(mirrored_document, direct_document);
    assert_eq!(
        direct_settings,
        serde_json::to_value(&settings).expect("serializes")
    );
    assert_eq!(
        direct_layers,
        serde_json::to_value(&projected).expect("serializes")
    );
    assert_eq!(
        direct_fonts,
        serde_json::to_value(&projected_fonts).expect("serializes")
    );
    // The one served plugin-API operation reports the session's own document.
    // The reply is the `CadApiOutcome` envelope, so the value lives under `value`.
    let document = api_snapshot
        .document
        .as_ref()
        .expect("the fixture session holds a document");
    assert_eq!(direct_document["ok"], serde_json::json!(true));
    assert_eq!(
        direct_document["value"]["document"]["id"],
        serde_json::json!(document.id)
    );
    assert_eq!(
        direct_document["value"]["document"]["name"],
        serde_json::json!("synthetic-four-lines.dwg")
    );
    assert_eq!(
        direct_document["value"]["document"]["isReadOnly"],
        serde_json::json!(false)
    );
    assert_eq!(
        direct_document["value"]["document"]["database"]["id"],
        serde_json::json!(document.database_id)
    );
    assert_eq!(
        direct_document["value"]["document"]["database"]["version"],
        serde_json::json!(document.database_version)
    );

    // The fixture's database id is the one the static matrix revision case
    // names, so that case really is the "match" one (the mismatch case uses 999).
    assert_eq!(document.database_id, "1", "the fixture database id");

    // The other two served reads answer the same object on both sources.
    let documents = |reads: &dyn cad_ui_core::session::ControlReads| {
        wire::api_call(
            reads,
            "ApplicationServices",
            "documentManager.documents",
            None,
        )
        .expect("the envelope")
    };
    let mirrored_documents = documents(&mirrored);
    assert_eq!(mirrored_documents, documents(&direct));
    assert_eq!(
        mirrored_documents.value.expect("a value"),
        serde_json::json!({ "documents": [ document.wire_value() ] })
    );

    let revision_parameters = serde_json::json!({ "database": document.database_id });
    let revision = |reads: &dyn cad_ui_core::session::ControlReads| {
        wire::api_call(
            reads,
            "DatabaseServices",
            "database.revision",
            Some(&revision_parameters),
        )
        .expect("the envelope")
    };
    let mirrored_revision = revision(&mirrored);
    assert_eq!(mirrored_revision, revision(&direct));
    assert_eq!(
        mirrored_revision.value.expect("a value"),
        serde_json::json!({
            "revision": document.database_version.parse::<u64>().expect("a decimal revision"),
        })
    );
}

/// A session that exists **without** a document is a fact, not a wait.
///
/// `None` at the read seam means "no snapshot yet"; a snapshot whose document is
/// `None` answers the real "nothing is open" value for every served read, on both
/// paths: `{"document": null}`, `{"documents": []}`, and `cad_api.no_document`.
#[test]
fn a_session_without_a_document_answers_facts_on_both_paths() {
    let session = session();
    session
        .command("CLOSE")
        .expect("CLOSE is the host's own close command");
    let snapshot = session.api_snapshot();
    assert!(
        snapshot.document.is_none(),
        "the fixture document was really closed"
    );

    // The desktop's owner tick mirrors exactly this snapshot.
    let mirrored = QueueReads::new(None, None, None, Some(&snapshot));
    let direct = DirectReads::new(&session);
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;

    let cases: [(&str, &str, Option<Value>, bool, Value); 5] = [
        (
            "ApplicationServices",
            "documentManager.mdiActiveDocument",
            None,
            true,
            serde_json::json!({ "document": null }),
        ),
        (
            "ApplicationServices",
            "documentManager.documents",
            None,
            true,
            serde_json::json!({ "documents": [] }),
        ),
        (
            "DatabaseServices",
            "database.revision",
            Some(serde_json::json!({ "database": "1" })),
            false,
            serde_json::json!(cad_ui_core::session::API_CALL_NO_DOCUMENT),
        ),
        (
            "DatabaseServices",
            "database.symbolTables",
            Some(serde_json::json!({ "database": "1" })),
            false,
            serde_json::json!(cad_ui_core::session::API_CALL_NO_DOCUMENT),
        ),
        (
            "EditorInput",
            "editor.currentSelection",
            None,
            false,
            serde_json::json!(cad_ui_core::session::API_CALL_NO_DOCUMENT),
        ),
    ];
    for (namespace, operation, parameters, expects_value, expected) in cases {
        let args = serde_json::json!({
            "namespace": namespace,
            "operation": operation,
            "parameters": &parameters,
        })
        .to_string();
        let (mirror_answer, direct_answer) = {
            let mut sink = queues.sink();
            (
                dispatch("cad_api_call", &args, &mut sink, &mirrored, &mut popups)
                    .expect("the mirrored answer"),
                dispatch("cad_api_call", &args, &mut sink, &direct, &mut popups)
                    .expect("the direct answer"),
            )
        };
        assert_eq!(mirror_answer, direct_answer, "{namespace}.{operation}");
        if expects_value {
            assert_eq!(mirror_answer["ok"], serde_json::json!(true));
            assert_eq!(mirror_answer["value"], expected);
        } else {
            // A document-scoped read has nothing to report: an explicit refusal.
            assert_eq!(mirror_answer["ok"], serde_json::json!(false));
            assert_eq!(mirror_answer["error"], expected);
        }

        // The shared `wire::*` function answers the same object on both sources.
        assert_eq!(
            wire::api_call(&mirrored, namespace, operation, parameters.as_ref())
                .expect("the mirrored envelope"),
            wire::api_call(&direct, namespace, operation, parameters.as_ref())
                .expect("the direct envelope")
        );
    }
}

/// The `database.symbolTables` read is the same derived projection on both paths,
/// and it reports the drawing's real tables (never an invented row).
#[test]
fn the_symbol_tables_projection_is_the_same_object_on_both_paths() {
    let session = session();
    let snapshot = session.api_snapshot();
    let database = snapshot
        .document
        .as_ref()
        .expect("the fixture session holds a document")
        .database_id
        .clone();
    let parameters = serde_json::json!({ "database": database });
    let mirrored = QueueReads::new(None, None, None, Some(&snapshot));
    let direct = DirectReads::new(&session);
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let args = serde_json::json!({
        "namespace": "DatabaseServices",
        "operation": "database.symbolTables",
        "parameters": &parameters,
    })
    .to_string();
    let (mirror_answer, direct_answer) = {
        let mut sink = queues.sink();
        (
            dispatch("cad_api_call", &args, &mut sink, &mirrored, &mut popups)
                .expect("the mirrored answer"),
            dispatch("cad_api_call", &args, &mut sink, &direct, &mut popups)
                .expect("the direct answer"),
        )
    };
    assert_eq!(mirror_answer, direct_answer);
    assert_eq!(mirror_answer["ok"], serde_json::json!(true));

    let value = &mirror_answer["value"];
    assert_eq!(value["blockTable"]["name"], serde_json::json!("blocks"));
    assert_eq!(value["layerTable"]["name"], serde_json::json!("layers"));
    let blocks = value["blockTable"]["records"]
        .as_array()
        .expect("a block record list");
    assert!(
        blocks.is_empty(),
        "the synthetic fixture declares no user blocks (spaces are not blocks)"
    );
    for block in blocks {
        assert_eq!(
            block["isLayout"],
            serde_json::json!(false),
            "cad-db spaces are not blocks"
        );
        let origin = block["origin"].as_array().expect("a base point");
        assert_eq!(origin.len(), 3, "the base point has three numbers");
        assert!(origin.iter().all(serde_json::Value::is_number));
        assert!(block["name"].is_string(), "a named block");
    }
    let layers = value["layerTable"]["records"]
        .as_array()
        .expect("a layer record list");
    assert!(!layers.is_empty(), "the fixture drawing has a layer table");
    for layer in layers {
        assert!(layer["name"].is_string());
        assert!(layer["visible"].is_boolean());
        assert!(layer["locked"].is_boolean());
        assert!(layer["frozen"].is_boolean());
        assert!(layer["linetype"].is_string());
        assert!(
            layer["color"]["aci"].is_number() || layer["color"]["aci"].is_null(),
            "an ACI index or the honest null of a true colour"
        );
        let rgb = layer["color"]["rgb"].as_str().expect("an #rrggbb value");
        assert!(rgb.starts_with('#'), "{rgb} is an sRGB value");
        assert_eq!(rgb, rgb.to_lowercase(), "{rgb} is lowercase");
    }
    assert!(
        layers
            .iter()
            .any(|layer| layer["name"] == serde_json::json!("0")),
        "the fixture's layer 0 is listed"
    );
    // The active layer is a real row of the same projection.
    assert_eq!(value["clayer"], serde_json::json!("0"));
}

/// The `EditorInput.editor.currentSelection` read is the same ordered object-id
/// list on both paths: an empty selection is a fact, and `SELECTALL` really fills
/// the host's selection from the fixture drawing.
#[test]
fn the_current_selection_read_is_the_same_object_on_both_paths() {
    let session = session();
    let empty_snapshot = session.api_snapshot();
    assert!(
        empty_snapshot.selection.is_empty(),
        "a fresh session selects nothing"
    );

    let read = |session: &CadSession, snapshot: &cad_ui_core::session::CadApiSnapshot| {
        let mirrored = QueueReads::new(None, None, None, Some(snapshot));
        let direct = DirectReads::new(session);
        let mut queues = Queues::new(true, false);
        let mut popups = NoPopups;
        let args = r#"{"namespace":"EditorInput","operation":"editor.currentSelection"}"#;
        let mut sink = queues.sink();
        (
            dispatch("cad_api_call", args, &mut sink, &mirrored, &mut popups)
                .expect("the mirrored answer"),
            dispatch("cad_api_call", args, &mut sink, &direct, &mut popups)
                .expect("the direct answer"),
        )
    };

    // Nothing selected: `{"objectIds": []}` on both paths, not a refusal.
    let (mirror_empty, direct_empty) = read(&session, &empty_snapshot);
    assert_eq!(mirror_empty, direct_empty);
    assert_eq!(mirror_empty["ok"], serde_json::json!(true));
    assert_eq!(
        mirror_empty["value"],
        serde_json::json!({ "objectIds": [] })
    );

    // The real current selection: four fixture lines, in selection order.
    session
        .command("SELECTALL")
        .expect("SELECTALL is a host command");
    let snapshot = session.api_snapshot();
    assert_eq!(
        snapshot.selection.len(),
        4,
        "the synthetic fixture holds four lines"
    );
    let (mirror, direct) = read(&session, &snapshot);
    assert_eq!(mirror, direct);
    assert_eq!(mirror["ok"], serde_json::json!(true));
    let ids = mirror["value"]["objectIds"]
        .as_array()
        .expect("an object-id list");
    let expected: Vec<Value> = snapshot
        .selection
        .iter()
        .cloned()
        .map(Value::String)
        .collect();
    assert_eq!(ids, &expected, "selection order, decimal strings");
    for (index, id) in ids.iter().enumerate() {
        let id = id.as_str().expect("a decimal string");
        assert!(
            !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()),
            "{id} is decimal"
        );
        assert!(
            !ids[..index].contains(&Value::String(id.to_string())),
            "each selected entity is named once"
        );
    }
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
    assert_eq!(
        wire::fonts(&empty).unwrap_err(),
        cad_ui_core::session::CAD_WAITING
    );
    // The plugin-API read has the same "no snapshot yet" answer, inside the
    // outcome envelope — not a fabricated `{"document":null}`.
    let outcome = wire::api_call(
        &empty,
        "ApplicationServices",
        "documentManager.mdiActiveDocument",
        None,
    )
    .expect("the envelope is the reply");
    assert!(!outcome.ok);
    assert_eq!(outcome.value, None);
    assert_eq!(
        outcome.error.as_deref(),
        Some(cad_ui_core::session::CAD_WAITING)
    );
}

/// A mirror refusal is relayed, never replaced by stale rows.
#[test]
fn a_mirror_refusal_is_relayed() {
    let refused: Result<CadLayers, String> = Err("layer.projection_failed: boom".to_string());
    let reads = QueueReads::new(None, Some(&refused), None, None);
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

/// The mutating transport's matrix tests (submit/poll, the commit round trip).
mod api;
