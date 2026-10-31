//! Tests for the cad-event wire: the revisioned bus, the pure status diff and
//! the command-log derivation.

use super::*;
use crate::session::unsupported_status;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingSink {
    events: Arc<Mutex<Vec<CadEvent>>>,
}

impl EventSink for RecordingSink {
    fn deliver(&self, event: &CadEvent) {
        self.events.lock().expect("sink lock").push(event.clone());
    }
}

fn status() -> CadStatus {
    unsupported_status(None)
}

fn entry(
    sequence: u64,
    kind: CadCommandLogKind,
    command: Option<&str>,
    code: Option<&str>,
) -> CadCommandLogEntry {
    CadCommandLogEntry {
        sequence,
        kind,
        command: command.map(str::to_string),
        code: code.map(str::to_string),
        measurement: None,
        point: None,
    }
}

fn kinds(events: &[CadEventKind]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            CadEventKind::StatusChanged { .. } => "status-changed",
            CadEventKind::CommandAdmitted { .. } => "command-admitted",
            CadEventKind::CommandRejected { .. } => "command-rejected",
            CadEventKind::SelectionChanged { .. } => "selection-changed",
            CadEventKind::ToolStep { .. } => "tool-step",
            CadEventKind::DocumentOpened { .. } => "document-opened",
            CadEventKind::DocumentClosed => "document-closed",
            CadEventKind::ErrorRaised { .. } => "error-raised",
            CadEventKind::PopupChanged { .. } => "popup-changed",
            CadEventKind::EventsDropped { .. } => "events-dropped",
        })
        .collect()
}

#[test]
fn revisions_increase_by_one_per_emitted_event() {
    let mut bus = EventBus::new();
    assert_eq!(bus.revision(), 0);
    let first = bus.push(CadEventKind::CommandAdmitted {
        command: "LINE".to_string(),
    });
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].revision, 1);
    let second = bus.push_status(StatusFrame::new(status(), HostProfile::native_below()));
    assert_eq!(second[0].revision, 2);
    assert_eq!(bus.revision(), 2);
}

#[test]
fn semantic_events_are_pushed_before_the_status_frame() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut bus = EventBus::new();
    bus.add_sink(Box::new(RecordingSink {
        events: Arc::clone(&recorded),
    }));
    assert_eq!(bus.subscribers(), 1);

    bus.push(CadEventKind::CommandAdmitted {
        command: "LINE".to_string(),
    });
    bus.push_status(StatusFrame::new(status(), HostProfile::native_below()));

    let events = recorded.lock().expect("sink lock");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].revision, 1);
    assert_eq!(events[1].revision, 2);
    assert!(matches!(
        events[0].kind,
        CadEventKind::CommandAdmitted { .. }
    ));
    assert!(matches!(events[1].kind, CadEventKind::StatusChanged { .. }));
}

#[test]
fn the_ring_is_bounded_and_reports_dropped_events() {
    let mut bus = EventBus::new();
    for index in 0..EVENT_RING_CAPACITY {
        bus.push(CadEventKind::CommandAdmitted {
            command: format!("C{index}"),
        });
    }
    assert_eq!(bus.len(), EVENT_RING_CAPACITY);
    assert!(!bus.is_empty());

    let emitted = bus.push(CadEventKind::CommandAdmitted {
        command: "OVERFLOW".to_string(),
    });
    assert_eq!(
        kinds(&emitted.iter().map(|e| e.kind.clone()).collect::<Vec<_>>()),
        ["command-admitted", "events-dropped"]
    );
    assert_eq!(bus.len(), EVENT_RING_CAPACITY);
    let newest: Vec<_> = bus.events().rev().take(1).collect();
    assert!(matches!(
        newest[0].kind,
        CadEventKind::CommandAdmitted { .. }
    ));
    assert!(
        !bus.events()
            .any(|event| matches!(event.kind, CadEventKind::EventsDropped { .. })),
        "the marker is delivered, not retained"
    );
}

#[test]
fn push_quiet_overflows_and_reports_dropped_events_like_push() {
    let mut bus = EventBus::new();
    for index in 0..EVENT_RING_CAPACITY {
        let emitted = bus.push_quiet(CadEventKind::CommandAdmitted {
            command: format!("C{index}"),
        });
        assert_eq!(emitted.len(), 1, "the ring is not full yet: no marker");
    }
    assert_eq!(bus.len(), EVENT_RING_CAPACITY);

    let emitted = bus.push_quiet(CadEventKind::CommandAdmitted {
        command: "OVERFLOW".to_string(),
    });
    assert_eq!(
        kinds(
            &emitted
                .iter()
                .map(|event| event.kind.clone())
                .collect::<Vec<_>>()
        ),
        ["command-admitted", "events-dropped"]
    );
    assert_eq!(emitted[0].revision, EVENT_RING_CAPACITY as u64 + 1);
    assert_eq!(emitted[1].revision, emitted[0].revision + 1);
    assert_eq!(bus.len(), EVENT_RING_CAPACITY);
    let newest: Vec<_> = bus.events().rev().take(1).collect();
    assert!(matches!(
        newest[0].kind,
        CadEventKind::CommandAdmitted { .. }
    ));
    assert!(
        !bus.events()
            .any(|event| matches!(event.kind, CadEventKind::EventsDropped { .. })),
        "push_quiet keeps the same ring contract as push"
    );
}

#[test]
fn no_event_is_derived_from_the_first_snapshot() {
    assert!(derive_batch(None, &status()).is_empty());
}

#[test]
fn command_entries_become_admitted_and_batch_local_errors_become_rejected() {
    let prev = status();
    let mut next = status();
    next.command_log = vec![
        entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
        entry(
            2,
            CadCommandLogKind::Error,
            None,
            Some("zoom.argument_required"),
        ),
        entry(3, CadCommandLogKind::Command, Some("LINE"), None),
    ];
    let events = derive_batch(Some(&prev), &next);
    assert_eq!(
        kinds(&events),
        ["command-admitted", "command-rejected", "command-admitted"]
    );
    assert!(matches!(
        &events[1],
        CadEventKind::CommandRejected { command, code }
            if command == "ZOOM" && code == "zoom.argument_required"
    ));
}

#[test]
fn a_refusal_without_a_selector_or_code_emits_no_rejection_event() {
    // No admitted selector at all: nothing to name, so no event is invented.
    let prev = status();
    let mut next = status();
    next.command_log = vec![entry(
        1,
        CadCommandLogKind::Error,
        None,
        Some("zoom.argument_required"),
    )];
    assert!(derive_batch(Some(&prev), &next).is_empty());

    // A selector with no stable code: the code is not invented either.
    let mut next = status();
    next.command_log = vec![
        entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
        entry(2, CadCommandLogKind::Error, None, None),
    ];
    assert_eq!(
        kinds(&derive_batch(Some(&prev), &next)),
        ["command-admitted"]
    );
}

#[test]
fn an_error_is_not_attributed_to_a_command_from_an_earlier_batch() {
    // OPEN was admitted in a previous flush. A later internal failure appends
    // only an `Error` entry, so the batch-local rule refuses to label it as
    // an OPEN rejection; the error still rides `status.error`.
    let mut prev = status();
    prev.command_log = vec![entry(1, CadCommandLogKind::Command, Some("OPEN"), None)];
    let mut next = status();
    next.command_log = vec![
        entry(1, CadCommandLogKind::Command, Some("OPEN"), None),
        entry(
            2,
            CadCommandLogKind::Error,
            None,
            Some("open.internal_failure"),
        ),
    ];
    next.error = Some("open failed".to_string());

    let events = derive_batch(Some(&prev), &next);
    assert_eq!(kinds(&events), ["error-raised"]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, CadEventKind::CommandRejected { .. })),
        "an error with no batch-local admission is never attributed: {events:?}"
    );
}

#[test]
fn a_second_error_is_not_attributed_to_the_previous_command() {
    // The first error is attributable to ZOOM; the second directly follows
    // an error, so it has no selector and is skipped (never re-labelled).
    let prev = status();
    let mut next = status();
    next.command_log = vec![
        entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
        entry(2, CadCommandLogKind::Error, None, Some("zoom.first")),
        entry(3, CadCommandLogKind::Error, None, Some("zoom.second")),
    ];
    let events = derive_batch(Some(&prev), &next);
    assert_eq!(kinds(&events), ["command-admitted", "command-rejected"]);
    assert!(matches!(
        &events[1],
        CadEventKind::CommandRejected { command, code }
            if command == "ZOOM" && code == "zoom.first"
    ));
}

#[test]
fn a_bounded_log_reports_only_the_appended_tail() {
    let capacity = cad_host::COMMAND_LOG_CAPACITY;
    let mut prev = status();
    prev.command_log = (0..capacity as u64)
        .map(|index| {
            entry(
                index + 1,
                CadCommandLogKind::Command,
                Some(&format!("C{index}")),
                None,
            )
        })
        .collect();
    let mut next = status();
    next.command_log = prev.command_log[1..].to_vec();
    next.command_log.push(entry(
        capacity as u64 + 1,
        CadCommandLogKind::Command,
        Some("NEWEST"),
        None,
    ));

    let events = derive_batch(Some(&prev), &next);
    assert_eq!(kinds(&events), ["command-admitted"]);
    assert!(matches!(
        &events[0],
        CadEventKind::CommandAdmitted { command } if command == "NEWEST"
    ));
}

#[test]
fn a_cleared_log_reports_its_current_entries() {
    // A document replacement clears the host log; no overlap proves what was
    // there before, so the current entries are reported (documented limitation).
    let mut prev = status();
    prev.command_log = vec![entry(1, CadCommandLogKind::Command, Some("OLD"), None)];
    let mut next = status();
    next.command_log = vec![entry(2, CadCommandLogKind::Command, Some("NEW"), None)];

    let events = derive_batch(Some(&prev), &next);
    assert_eq!(kinds(&events), ["command-admitted"]);
}

#[test]
#[allow(deprecated)] // the frozen `CadPopup::label` is required to build a popup at all
fn selection_tool_document_error_and_popup_are_derived() {
    let mut prev = status();
    prev.document_label = Some("plan.dwg".to_string());
    prev.selection.count = 1;
    prev.selection.empty = false;
    prev.tool = None;
    prev.error = None;
    prev.popup = None;

    let mut next = status();
    next.document_label = None;
    next.selection.count = 2;
    next.selection.empty = false;
    next.tool = Some(cad_tool("draw.line", 2));
    next.error = Some("resource: missing SHX".to_string());
    next.popup = Some(CadPopup {
        kind: "about".to_string(),
        label: crate::popup::POPUP_LABEL.to_string(),
        modal: true,
    });

    let events = derive_batch(Some(&prev), &next);
    assert_eq!(
        kinds(&events),
        [
            "selection-changed",
            "tool-step",
            "document-closed",
            "error-raised",
            "popup-changed",
        ]
    );
    assert!(matches!(
        &events[1],
        CadEventKind::ToolStep { tool, step } if tool == "draw.line" && *step == 2
    ));
}

#[test]
fn a_document_label_transition_emits_opened_then_closed() {
    let mut prev = status();
    prev.document_label = None;
    let mut next = status();
    next.document_label = Some("plan.dwg".to_string());
    let events = derive_batch(Some(&prev), &next);
    assert_eq!(kinds(&events), ["document-opened"]);
    assert!(matches!(
        &events[0],
        CadEventKind::DocumentOpened { label } if label == "plan.dwg"
    ));

    let closed = derive_batch(Some(&next), &prev);
    assert_eq!(kinds(&closed), ["document-closed"]);
}

#[test]
fn an_unchanged_status_derives_nothing() {
    let status = status();
    assert!(derive_batch(Some(&status), &status).is_empty());
}

#[test]
fn the_status_frame_serializes_exactly_like_cad_status() {
    let mut status = status();
    status.document_label = Some("plan.dwg".to_string());
    let frame = StatusFrame::new(status, HostProfile::dom_canvas()).with_canvas(Some(
        CanvasJson::new(r#"{"label":"plan.dwg","batches":2,"draws":0}"#.to_string()),
    ));
    let event = CadEvent {
        revision: 7,
        kind: CadEventKind::StatusChanged {
            status: Box::new(frame),
        },
    };
    let json = serde_json::to_string(&event).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(value["revision"], serde_json::json!(7));
    assert_eq!(value["type"], serde_json::json!("status-changed"));
    assert_eq!(
        value["status"]["documentLabel"],
        serde_json::json!("plan.dwg")
    );
    assert_eq!(
        value["status"]["hostProfile"],
        serde_json::to_value(HostProfile::dom_canvas()).expect("profile")
    );
    assert_eq!(
        value["status"]["canvasSession"],
        serde_json::json!({"label": "plan.dwg", "batches": 2, "draws": 0})
    );
    // The flattened status fields precede the profile, exactly like
    // `wire_status_json` (no `Value` key sorting).
    let label = json.find("\"documentLabel\"").expect("documentLabel");
    let profile = json.find("\"hostProfile\"").expect("hostProfile");
    let canvas = json.find("\"canvasSession\"").expect("canvasSession");
    assert!(label < profile && profile < canvas, "{json}");
}

#[test]
fn a_status_frame_without_a_canvas_omits_the_key() {
    let event = CadEvent {
        revision: 1,
        kind: CadEventKind::StatusChanged {
            status: Box::new(StatusFrame::new(status(), HostProfile::native_below())),
        },
    };
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(!json.contains("canvasSession"), "{json}");
}

#[test]
fn event_types_are_kebab_case_and_the_envelope_is_frozen() {
    let cases = [
        (
            CadEventKind::CommandAdmitted {
                command: "LINE".to_string(),
            },
            serde_json::json!({"revision": 1, "type": "command-admitted", "command": "LINE"}),
        ),
        (
            CadEventKind::CommandRejected {
                command: "ZOOM".to_string(),
                code: "zoom.argument_required".to_string(),
            },
            serde_json::json!({"revision": 1, "type": "command-rejected", "command": "ZOOM", "code": "zoom.argument_required"}),
        ),
        (
            CadEventKind::SelectionChanged {
                count: 2,
                empty: false,
            },
            serde_json::json!({"revision": 1, "type": "selection-changed", "count": 2, "empty": false}),
        ),
        (
            CadEventKind::ToolStep {
                tool: "line".to_string(),
                step: 2,
            },
            serde_json::json!({"revision": 1, "type": "tool-step", "tool": "line", "step": 2}),
        ),
        (
            CadEventKind::DocumentOpened {
                label: "plan.dwg".to_string(),
            },
            serde_json::json!({"revision": 1, "type": "document-opened", "label": "plan.dwg"}),
        ),
        (
            CadEventKind::DocumentClosed,
            serde_json::json!({"revision": 1, "type": "document-closed"}),
        ),
        (
            CadEventKind::ErrorRaised {
                error: "boom".to_string(),
            },
            serde_json::json!({"revision": 1, "type": "error-raised", "error": "boom"}),
        ),
        (
            CadEventKind::PopupChanged { popup: None },
            serde_json::json!({"revision": 1, "type": "popup-changed", "popup": null}),
        ),
        (
            CadEventKind::EventsDropped { count: 3 },
            serde_json::json!({"revision": 1, "type": "events-dropped", "count": 3}),
        ),
    ];
    for (kind, expected) in cases {
        let event = CadEvent { revision: 1, kind };
        assert_eq!(serde_json::to_value(&event).expect("serializes"), expected);
    }
}

fn cad_tool(key: &str, points: usize) -> super::super::status::CadTool {
    super::super::status::CadTool {
        key: key.to_string(),
        points,
        ready: false,
        open_ended: false,
        awaiting_text: false,
        text_prefill: None,
        params: super::super::status::CadToolParams {
            sides: None,
            fillet_radius: None,
            chamfer_distance_a: None,
            chamfer_distance_b: None,
            array_rows: None,
            array_columns: None,
            array_row_spacing: None,
            array_column_spacing: None,
            array_items: None,
            array_fill_angle: None,
            array_rotate_items: None,
            align_scale: None,
            hatch_pattern_type: None,
            hatch_pattern: None,
            hatch_spacing: None,
            hatch_angle: None,
            hatch_double: None,
            hatch_style: None,
            hatch_gradient_kind: None,
            hatch_gradient_angle: None,
            hatch_gradient_color_a: None,
            hatch_gradient_color_b: None,
            text_height: None,
            text_rotation: None,
            mtext_attachment: None,
        },
    }
}
