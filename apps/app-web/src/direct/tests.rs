//! The **equivalence matrix**: the desktop queue sink and the `dom-canvas` direct
//! sink must answer the same request identically.
//!
//! # What is compared, and why it is the acceptance for this round
//!
//! The desktop and the `dom-canvas` hosts now share one command-semantics layer
//! (`cad_ui_core::session::wire`). The only thing that differs is the transport:
//! the desktop **queues** onto the owner thread, wasm **applies** inline. So the
//! proof has to show that the two sinks, driven through the *same* dispatcher with
//! the *same* argument objects, produce:
//!
//! * the same **admission verdict** — either the shared gate refused (and with the
//!   identical string) or the request reached the sink;
//! * the same **acknowledgement** (`{accepted:true}`) whenever the request was
//!   admitted and applied;
//! * the same **session verdict** — the string the session produces when it
//!   refuses a request the gate admitted. The desktop sees it when the owner tick
//!   drains the queue (exactly what [`Queues::drain`] does here, through the same
//!   `CadSession` funnel), the direct sink sees it in the same call.
//!
//! Each command is driven with a **valid** request, a **malformed** one and — where
//! the session has an opinion — a request the session refuses by name, plus the
//! shared gate refusals (probe mode, the modal popup gate) via the sink's declared
//! gate inputs.
//!
//! The two vocabulary answers are compared too: both hosts serve
//! `admissible_names()`/`alias_pairs()` through `wire::commands`/`wire::aliases`,
//! so the chrome sees one command set everywhere.
//!
//! # Honest limits of this matrix
//!
//! * It runs on the **host** target (no browser, no GPU): a `CadSession` built from
//!   the compiled-in synthetic fixture's bytes with a `RenderSessionHandle` that has
//!   no device attached. That is the same session the browser builds; what it does
//!   not prove is anything about pixels.
//! * A **full queue** is a desktop-only state (the direct sink holds nothing between
//!   calls), so the "queue is full" strings are pinned in `cad-ui-core`'s admission
//!   tests, not here.
//! * The desktop's refusal is *deferred*: `{accepted:true}` means queued, and the
//!   session's verdict arrives on the next tick. [`Queues::drain`] reproduces that
//!   tick with the same `CadSession` methods, which is what makes the strings
//!   comparable at all.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use cad_app::{StandardView, ToolParameter, ToolValue};
use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::{HostOptions, RequestClock};
use cad_ui_core::session::wire::{self, CadCommandAck};
use cad_ui_core::session::{
    CadLayerWrite, CadLayers, CadSession, CadSettingKey, CadSettingValue, NoPopups, QueueFamily,
    QueueReads, QueueSink,
};
use cad_ui_core::shell::ShellState;
use serde_json::Value;

use super::*;

/// The compiled-in synthetic fixture: four `LINE`s, no text and no raster image.
const SYNTHETIC: &[u8] = include_bytes!("../../../../fixtures/dwg/synthetic-four-lines.dwg");
const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

/// A monotonic millisecond clock (the host-target stand-in for
/// `performance.now()`), so the session's request pump sees a real progression.
fn monotonic_clock() -> RequestClock {
    let millis = Cell::new(0.0_f64);
    Rc::new(move || {
        let now = millis.get();
        millis.set(now + 0.05);
        now
    })
}

/// A fresh copy of the drawing the browser carrier opens.
///
/// Every case starts from this, so a mutating request cannot leak into the next
/// comparison.
fn session() -> CadSession {
    let platform = crate::platform::BrowserPlatform::new();
    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [800.0, 600.0],
    };
    // The carrier's surface must be reported before the session is built: the open
    // funnel validates canvas metrics against it, and an unmapped surface would
    // make every later host request an explicit `Cancelled` instead of the real
    // verdict this matrix compares.
    platform.surface().report([800.0, 600.0], 1.0);
    let session = CadSession::from_bytes(
        options,
        platform.host_platform(cad_platform::HostCapabilities::default()),
        RenderSessionHandle::new(BackendPreference::WebGl2),
        monotonic_clock(),
        cad_ui_core::session::BytesDocument {
            label: SYNTHETIC_LABEL,
            bytes: SYNTHETIC,
        },
    )
    .expect("the synthetic fixture opens from bytes");
    session
        .set_canvas_metrics([800.0, 600.0], 1.0)
        .expect("the reported surface accepts the canvas metrics");
    session
}

// ---------------------------------------------------------------------------
// The desktop-shaped sink: a `ShellState` command queue plus the bounded family
// ---------------------------------------------------------------------------

/// The host's bounded queue family, wired exactly as `Shared::queue_sink` wires it.
struct Queues {
    shell: ShellState,
    tool_parameters: Vec<(ToolParameter, ToolValue)>,
    tool_texts: Vec<String>,
    selected_texts: Vec<String>,
    selected_properties: Vec<(String, String)>,
    standard_views: VecDeque<StandardView>,
    setting_writes: VecDeque<(CadSettingKey, CadSettingValue)>,
    layer_writes: VecDeque<CadLayerWrite>,
    cad_mode: bool,
    modal_open: bool,
}

impl Queues {
    fn new(cad_mode: bool, modal_open: bool) -> Self {
        Self {
            shell: ShellState::new(cad_mode),
            tool_parameters: Vec::new(),
            tool_texts: Vec::new(),
            selected_texts: Vec::new(),
            selected_properties: Vec::new(),
            standard_views: VecDeque::new(),
            setting_writes: VecDeque::new(),
            layer_writes: VecDeque::new(),
            cad_mode,
            modal_open,
        }
    }

    /// The sink view over these queues (the desktop's `Shared::queue_sink`).
    fn sink(&mut self) -> QueueSink<'_> {
        QueueSink::new(
            self.cad_mode,
            self.modal_open,
            QueueFamily {
                commands: &mut self.shell.cad_commands,
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

    /// Whether anything at all was queued (i.e. the gate admitted the request).
    fn is_empty(&self) -> bool {
        self.shell.cad_commands.is_empty()
            && self.tool_parameters.is_empty()
            && self.tool_texts.is_empty()
            && self.selected_texts.is_empty()
            && self.selected_properties.is_empty()
            && self.standard_views.is_empty()
            && self.setting_writes.is_empty()
            && self.layer_writes.is_empty()
    }

    /// The **owner tick**: apply everything queued, in admission order, through the
    /// same `CadSession` funnels the desktop tick uses.
    ///
    /// Returns the first refusal (a real one is never swallowed).
    fn drain(&self, session: &CadSession) -> Result<(), String> {
        for command in &self.shell.cad_commands {
            session.command(command)?;
        }
        for (parameter, value) in &self.tool_parameters {
            session.set_tool_parameter(*parameter, *value)?;
        }
        for text in &self.tool_texts {
            session.set_tool_text(text)?;
        }
        for text in &self.selected_texts {
            session.set_selected_text(text)?;
        }
        for (key, value) in &self.selected_properties {
            session.set_selected_property(key, value)?;
        }
        for view in &self.standard_views {
            session.standard_view(*view)?;
        }
        for (key, value) in &self.setting_writes {
            session.apply_setting(*key, value.clone())?;
        }
        for write in &self.layer_writes {
            write.apply(session)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// Whether the shared gate admitted the request, and if not, why.
#[derive(Debug, PartialEq, Eq)]
enum Admission {
    /// Refused before (or while decoding, i.e. before) the sink: nothing stored.
    Refused(String),
    /// The gate admitted it; the sink was reached.
    Admitted,
}

/// One path's answer for one request.
#[derive(Debug, PartialEq)]
struct Run {
    admission: Admission,
    /// The wire answer, when the call returned one.
    ack: Option<Value>,
    /// What applying the request produced (the session's own verdict).
    applied: Result<(), String>,
}

impl Run {
    /// Compare two runs, allowing the acknowledged-vs-refused timing difference
    /// only in the case it is inherent to.
    fn assert_same(&self, other: &Run, case: &str) {
        assert_eq!(
            self.admission, other.admission,
            "admission verdict differs for {case}"
        );
        assert_eq!(
            self.applied, other.applied,
            "session verdict differs for {case}"
        );
        if self.applied.is_ok() {
            assert_eq!(self.ack, other.ack, "wire answer differs for {case}");
        }
    }
}

/// The **queue** path: dispatch into [`Queues`], then drain like the owner tick.
fn via_queue(name: &str, args: &str, gates: (bool, bool)) -> Run {
    let mut queues = Queues::new(gates.0, gates.1);
    let reads = QueueReads::default();
    let mut popups = NoPopups;
    let result = {
        let mut sink = queues.sink();
        dispatch(name, args, &mut sink, &reads, &mut popups)
    };
    match result {
        Ok(ack) => Run {
            admission: Admission::Admitted,
            ack: Some(ack),
            applied: queues.drain(&session()),
        },
        Err(error) => {
            assert!(
                queues.is_empty(),
                "{case} was refused but something was queued",
                case = name
            );
            Run {
                admission: Admission::Refused(error),
                ack: None,
                applied: Ok(()),
            }
        }
    }
}

/// The **direct** path: dispatch into a [`DirectSink`] over a live session.
fn via_direct(name: &str, args: &str, gates: (bool, bool)) -> Run {
    let session = session();
    let mut sink = DirectSink::with_gates(&session, gates.0, gates.1);
    let mut popups = NoPopups;
    let result = dispatch(
        name,
        args,
        &mut sink,
        &DirectReads::new(&session),
        &mut popups,
    );
    match result {
        Ok(ack) => Run {
            admission: Admission::Admitted,
            ack: Some(ack),
            applied: Ok(()),
        },
        Err(error) => {
            // The sink records what an *apply* produced, so a refusal it did not
            // produce must have come from the shared gate.
            match sink.take_refusal() {
                Some(refusal) => {
                    assert_eq!(
                        refusal, error,
                        "the direct sink's refusal must be the returned one ({name})"
                    );
                    Run {
                        admission: Admission::Admitted,
                        ack: None,
                        applied: Err(error),
                    }
                }
                None => Run {
                    admission: Admission::Refused(error),
                    ack: None,
                    applied: Ok(()),
                },
            }
        }
    }
}

/// One request, driven through both paths and compared.
///
/// Returns the two runs so the matrix can assert it really covered every outcome
/// shape (a suite that only ever saw one bucket would prove nothing).
fn compare(case: &str, name: &str, args: &str, gates: (bool, bool)) -> (Run, Run) {
    let desktop = via_queue(name, args, gates);
    let canvas = via_direct(name, args, gates);
    desktop.assert_same(&canvas, &format!("{case} [{name} {args}]"));
    (desktop, canvas)
}

/// The per-command matrix lives in its own module so this file stays readable.
mod matrix;

// ---------------------------------------------------------------------------
// The `invoke` envelope and the name set
// ---------------------------------------------------------------------------

#[test]
fn the_envelope_is_one_shape_for_success_and_refusal() {
    let ok: Value = serde_json::from_str(&envelope(Ok(serde_json::json!({"accepted": true}))))
        .expect("valid JSON");
    assert_eq!(ok["ok"], Value::Bool(true));
    assert_eq!(ok["value"]["accepted"], Value::Bool(true));
    assert!(ok.get("error").is_none());

    let refused: Value =
        serde_json::from_str(&envelope(Err("popup.modal_open".to_string()))).expect("valid JSON");
    assert_eq!(refused["ok"], Value::Bool(false));
    assert_eq!(refused["error"], Value::String("popup.modal_open".into()));
    assert!(refused.get("value").is_none());
}

#[test]
fn the_native_entry_refuses_a_session_command_and_answers_the_vocabulary() {
    let unknown: Value = serde_json::from_str(&invoke("cad_floop", "{}")).expect("valid JSON");
    assert_eq!(unknown["ok"], Value::Bool(false));
    assert_eq!(
        unknown["error"],
        Value::String(unsupported_runtime("cad_floop"))
    );

    // A known command with no session in this build: an explicit refusal, never a
    // fabricated acceptance.
    let no_session: Value =
        serde_json::from_str(&invoke("cad_command", r#"{"command":"LINE"}"#)).expect("valid JSON");
    assert_eq!(no_session["ok"], Value::Bool(false));
    let error = no_session["error"].as_str().expect("a reason");
    assert!(
        error.starts_with(UNSUPPORTED_RUNTIME)
            && error.contains("cad_command")
            && error.contains(NO_SESSION),
        "a session-backed command must refuse explicitly: {error}"
    );

    // The vocabulary is pure, so it is served even here — and it is the desktop's.
    for (name, expected) in [
        (
            "cad_commands",
            serde_json::json!(cad_ui_core::session::admissible_names()),
        ),
        (
            "cad_command_aliases",
            serde_json::json!(cad_ui_core::session::alias_pairs()
                .into_iter()
                .map(|(alias, command)| serde_json::json!({"alias": alias, "command": command}))
                .collect::<Vec<_>>()),
        ),
    ] {
        let answer: Value = serde_json::from_str(&invoke(name, "{}")).expect("valid JSON");
        assert_eq!(answer["ok"], Value::Bool(true), "{name}: {answer}");
        assert_eq!(answer["value"], expected, "{name}");
    }
}

#[test]
fn the_wire_command_set_is_the_desktop_command_set() {
    // Every name this transport serves is a desktop command name, and every
    // desktop CAD command name is served here.
    let desktop: Vec<&str> = crate::direct::WIRE_COMMANDS.to_vec();
    let shell: Vec<&str> = cad_desktop_command_names()
        .into_iter()
        // `cad_status` is the carrier's own status projection (`shell_status`),
        // not a request-carrying command; see the test below.
        .filter(|name| name.starts_with("cad_") && *name != "cad_status")
        .collect();
    let mut missing: Vec<&str> = shell
        .into_iter()
        .filter(|name| !desktop.contains(name))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "these desktop commands have no dom-canvas wire entry: {missing:?}"
    );
    for name in desktop {
        assert!(is_wire_command(name), "{name}");
    }
}

/// The desktop command names this crate mirrors, kept in one place.
///
/// The authoritative list is `cad_desktop_shell::COMMAND_NAMES`; that crate is a
/// Tauri layer this host cannot depend on, so the list is spelled out here and the
/// test above is what keeps it honest (an added desktop command that is not served
/// here fails the test).
fn cad_desktop_command_names() -> Vec<&'static str> {
    vec![
        "cad_command",
        "cad_command_aliases",
        "cad_status",
        "cad_commands",
        "cad_set_tool_parameter",
        "cad_set_tool_text",
        "cad_set_selected_text",
        "cad_set_selected_property",
        "cad_standard_view",
        "cad_open_popup",
        "cad_close_popup",
        "cad_settings",
        "cad_set_setting",
        "cad_layers",
        "cad_set_layer_visible",
        "cad_restore_layers",
        "cad_set_active_layer",
        "cad_create_layer",
        "cad_rename_layer",
        "cad_delete_layer",
        "cad_set_layer_property",
    ]
}

/// `cad_status` is deliberately **not** in this transport: it is a status
/// projection the carrier builds from its own session (`shell_status`), not a
/// command that carries a request. The exclusion is asserted so it can never sneak
/// back in as a silently different answer.
#[test]
fn cad_status_is_not_a_wire_command() {
    assert!(!is_wire_command("cad_status"));
    let refused: Value = serde_json::from_str(&invoke("cad_status", "{}")).expect("valid JSON");
    assert_eq!(refused["ok"], Value::Bool(false));
}

/// Every argument struct decodes to exactly the values the Tauri signature would
/// bind, so the transport cannot change a request on its way in.
#[test]
fn every_argument_struct_maps_to_the_tauri_signature_values() {
    let command: CommandArgs = serde_json::from_str(r#"{"command":"LINE"}"#).expect("decodes");
    assert_eq!(command.command, "LINE");

    let parameter: ToolParameterArgs =
        serde_json::from_str(r#"{"parameter":"polygonSides","number":4}"#).expect("decodes");
    assert_eq!(parameter.parameter, "polygonSides");
    assert_eq!(parameter.number, Some(4.0));
    assert_eq!(parameter.flag, None);
    let flag: ToolParameterArgs =
        serde_json::from_str(r#"{"parameter":"polygonSides","flag":true}"#).expect("decodes");
    assert_eq!(flag.number, None);
    assert_eq!(flag.flag, Some(true));

    let text: TextArgs = serde_json::from_str(r#"{"text":"hi"}"#).expect("decodes");
    assert_eq!(text.text, "hi");

    let property: PropertyArgs =
        serde_json::from_str(r#"{"key":"layer","value":"0"}"#).expect("decodes");
    assert_eq!(
        (property.key.as_str(), property.value.as_str()),
        ("layer", "0")
    );

    let view: StandardViewArgs = serde_json::from_str(r#"{"view":"top"}"#).expect("decodes");
    assert_eq!(view.view, "top");

    let setting: SettingArgs =
        serde_json::from_str(r#"{"key":"snapSpacing","value":2.5}"#).expect("decodes");
    assert_eq!(setting.key, "snapSpacing");
    assert_eq!(setting.value, serde_json::json!(2.5));

    let visible: LayerVisibleArgs =
        serde_json::from_str(r#"{"name":"0","visible":false}"#).expect("decodes");
    assert_eq!((visible.name.as_str(), visible.visible), ("0", false));

    let name: LayerNameArgs = serde_json::from_str(r#"{"name":"DOORS"}"#).expect("decodes");
    assert_eq!(name.name, "DOORS");

    let rename: LayerRenameArgs =
        serde_json::from_str(r#"{"layer":"DOORS","name":"PARTITIONS"}"#).expect("decodes");
    assert_eq!(
        (rename.layer.as_str(), rename.name.as_str()),
        ("DOORS", "PARTITIONS")
    );

    let layer: LayerArgs = serde_json::from_str(r#"{"layer":"0"}"#).expect("decodes");
    assert_eq!(layer.layer, "0");

    let attribute: LayerPropertyArgs =
        serde_json::from_str(r#"{"layer":"0","key":"color","value":"aci:1"}"#).expect("decodes");
    assert_eq!(attribute.layer, "0");
    assert_eq!(attribute.key, "color");
    assert_eq!(attribute.value, "aci:1");

    let popup: PopupArgs = serde_json::from_str(r#"{"kind":"about"}"#).expect("decodes");
    assert_eq!(popup.kind, "about");
}

/// The ack the wire layer produces is the frozen `{accepted:true}` object.
#[test]
fn the_acknowledgement_is_the_frozen_wire_object() {
    assert_eq!(
        serde_json::to_value(CadCommandAck::accepted()).expect("serializes"),
        serde_json::json!({"accepted": true})
    );
    assert!(CadCommandAck::accepted().accepted);
}
