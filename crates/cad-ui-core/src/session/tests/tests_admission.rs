//! Typed-admission contracts, moved here with the code from the Linux host.
//!
//! These pin the shared discipline of the dedicated CAD IPC channels: the wire
//! forms parse into typed values with named refusals, the static content rules
//! and gates run before the queue, and the queue bounds are real.

use std::collections::VecDeque;

use super::super::*;
use crate::shell::POPUP_MODAL_OPEN;

#[test]
fn tool_parameter_admission_is_typed_bounded_and_refuses_probe_mode() {
    // The `cad_set_tool_parameter` ack path: the value is only *queued* for the
    // owner thread (never applied off the main thread), refused while the CAD
    // session is disabled (probe mode), and bounded. Range/kind/Viewer validation
    // is the session's authority, applied on drain, not here.
    use cad_app::{ToolParameter, ToolValue};
    let mut queue: Vec<(ToolParameter, ToolValue)> = Vec::new();
    assert_eq!(
        admit_tool_parameter(
            &mut queue,
            false,
            ToolParameter::PolygonSides,
            ToolValue::Number(4.0)
        )
        .unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");

    // Accepted values are queued verbatim — no clamp, no range/kind check here.
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Number(3.0),
    )
    .expect("accepted");
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Number(4.0),
    )
    .expect("accepted");
    // A kind-mismatched value is *also* admitted verbatim here: the session
    // refuses it on drain, exactly like an out-of-range number.
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Flag(true),
    )
    .expect("accepted verbatim; the session refuses the mismatch on drain");
    assert_eq!(
        queue,
        vec![
            (ToolParameter::PolygonSides, ToolValue::Number(3.0)),
            (ToolParameter::PolygonSides, ToolValue::Number(4.0)),
            (ToolParameter::PolygonSides, ToolValue::Flag(true)),
        ]
    );

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_TOOL_PARAMETER_CAP {
        admit_tool_parameter(
            &mut queue,
            true,
            ToolParameter::PolygonSides,
            ToolValue::Number(6.0),
        )
        .expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_TOOL_PARAMETER_CAP);
    assert_eq!(
        admit_tool_parameter(
            &mut queue,
            true,
            ToolParameter::PolygonSides,
            ToolValue::Number(6.0)
        )
        .unwrap_err(),
        "CAD tool-parameter queue is full"
    );
}

#[test]
fn tool_text_admission_is_bounded_and_refuses_probe_mode_and_static_content() {
    // The `cad_set_tool_text` ack path (ADR 0019 decision A.1): the text is only
    // *queued* for the owner thread, the static content rules (non-empty, at most
    // 1024 characters) are refused before the queue, and the live state
    // (is a tool awaiting text? are its points complete?) is the session's
    // authority on drain — this admission deliberately knows no session state.
    let mut queue: Vec<String> = Vec::new();
    assert_eq!(
        admit_tool_text(&mut queue, false, "Hello".into()).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");
    assert_eq!(
        admit_tool_text(&mut queue, true, String::new()).unwrap_err(),
        "tool.text_empty: cad_set_tool_text needs a non-empty text"
    );
    assert_eq!(
        admit_tool_text(&mut queue, true, "  \t".into()).unwrap_err(),
        "tool.text_empty: cad_set_tool_text needs a non-empty text"
    );
    let over_long = "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1);
    let refusal = admit_tool_text(&mut queue, true, over_long).unwrap_err();
    assert!(refusal.contains("tool.text_too_long"), "{refusal}");
    assert!(
        queue.is_empty(),
        "content refusals queue nothing: {queue:?}"
    );

    // Exactly at the bound is accepted, verbatim (no trimming, no truncation).
    let at_bound = "b".repeat(cad_app::MAX_TOOL_TEXT_CHARS);
    admit_tool_text(&mut queue, true, at_bound.clone()).expect("the bound is accepted");
    admit_tool_text(&mut queue, true, "  padded  ".into()).expect("accepted verbatim");
    assert_eq!(queue, vec![at_bound, "  padded  ".to_string()]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_TOOL_TEXT_CAP {
        admit_tool_text(&mut queue, true, "x".into()).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_TOOL_TEXT_CAP);
    assert_eq!(
        admit_tool_text(&mut queue, true, "x".into()).unwrap_err(),
        "CAD tool-text queue is full"
    );
}

#[test]
fn selected_text_admission_is_bounded_and_refuses_probe_mode_and_static_content() {
    // The `cad_set_selected_text` ack path (ADR 0020 amendment B): the text is
    // only *queued* for the owner thread, the static content rules are refused
    // before the queue, and whether the current selection is one editable `Text`
    // is the session's authority on drain — this admission knows no selection.
    let mut queue: Vec<String> = Vec::new();
    assert_eq!(
        admit_selected_text(&mut queue, false, "Hello".into()).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");
    assert_eq!(
        admit_selected_text(&mut queue, true, String::new()).unwrap_err(),
        "tool.text_empty: cad_set_selected_text needs a non-empty text"
    );
    assert_eq!(
        admit_selected_text(&mut queue, true, "  \t".into()).unwrap_err(),
        "tool.text_empty: cad_set_selected_text needs a non-empty text"
    );
    let over_long = "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1);
    let refusal = admit_selected_text(&mut queue, true, over_long).unwrap_err();
    assert!(refusal.contains("tool.text_too_long"), "{refusal}");
    assert!(refusal.contains("cad_set_selected_text"), "{refusal}");
    assert!(
        queue.is_empty(),
        "content refusals queue nothing: {queue:?}"
    );

    // Exactly at the bound is accepted verbatim; the queue holds the text only
    // (no entity id — the host resolves the current selection).
    let at_bound = "b".repeat(cad_app::MAX_TOOL_TEXT_CHARS);
    admit_selected_text(&mut queue, true, at_bound.clone()).expect("the bound is accepted");
    admit_selected_text(&mut queue, true, "  padded  ".into()).expect("accepted verbatim");
    assert_eq!(queue, vec![at_bound, "  padded  ".to_string()]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_SELECTED_TEXT_CAP {
        admit_selected_text(&mut queue, true, "x".into()).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_SELECTED_TEXT_CAP);
    assert_eq!(
        admit_selected_text(&mut queue, true, "x".into()).unwrap_err(),
        "CAD selected-text queue is full"
    );
}

#[test]
fn standard_view_wire_names_parse_case_insensitively_and_refuse_unknown_or_empty() {
    // The `cad_standard_view` wire form: the stable machine key of a standard
    // view, matched ASCII case-insensitively. Every one of the seven views is
    // reachable by name; an empty request and an unknown name are distinct,
    // named refusals (never a default view, never a clamp).
    use cad_app::StandardView;

    for (wire, expected) in [
        ("top", StandardView::Top),
        ("BOTTOM", StandardView::Bottom),
        ("Front", StandardView::Front),
        ("bAcK", StandardView::Back),
        ("left", StandardView::Left),
        ("RIGHT", StandardView::Right),
        ("isometric", StandardView::Isometric),
        ("  Isometric  ", StandardView::Isometric),
    ] {
        assert_eq!(parse_standard_view(wire), Ok(expected), "{wire:?}");
    }
    // Distinct wire names map to distinct variants: no key aliases another.
    let mut keys: Vec<&str> = StandardView::ALL.iter().map(|view| view.key()).collect();
    let count = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count, "standard-view keys are unique");

    let empty = parse_standard_view("   ").unwrap_err();
    let unknown = parse_standard_view("diagonal").unwrap_err();
    assert!(empty.starts_with("view.standard_empty"), "{empty}");
    assert!(unknown.starts_with("view.standard_unknown"), "{unknown}");
    assert_ne!(empty, unknown);
    assert!(unknown.contains("diagonal"), "{unknown}");
    // The refusal lists the accepted vocabulary instead of guessing a view.
    assert!(unknown.contains("isometric"), "{unknown}");
}

#[test]
fn standard_view_admission_is_bounded_and_refuses_probe_mode_and_the_modal_gate() {
    // The `cad_standard_view` ack path: the typed view is only *queued* for the
    // owner thread. The gates are `admit_command`'s: the modal popup gate first,
    // then a disabled CAD session, then the bound.
    use cad_app::StandardView;

    let mut queue: VecDeque<StandardView> = VecDeque::new();
    assert_eq!(
        admit_standard_view(&mut queue, false, false, StandardView::Top).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert_eq!(
        admit_standard_view(&mut queue, true, true, StandardView::Top).unwrap_err(),
        POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty(), "refusals queue nothing");

    admit_standard_view(&mut queue, true, false, StandardView::Isometric).expect("accepted");
    assert_eq!(queue.pop_front(), Some(StandardView::Isometric));

    for _ in 0..CAD_STANDARD_VIEW_CAP {
        admit_standard_view(&mut queue, true, false, StandardView::Front).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_STANDARD_VIEW_CAP);
    assert_eq!(
        admit_standard_view(&mut queue, true, false, StandardView::Top).unwrap_err(),
        "CAD standard-view queue is full"
    );
    // The modal gate is checked before the bound, exactly like `admit_command`:
    // a full queue behind an open modal reports the modal reason.
    assert_eq!(
        admit_standard_view(&mut queue, true, true, StandardView::Top).unwrap_err(),
        POPUP_MODAL_OPEN
    );
}

#[test]
fn drained_standard_views_apply_in_order_and_empty_the_queue() {
    // Two queued views apply in admission order (the last one wins on the
    // camera) and the queue is emptied. A real refusal is returned, never
    // swallowed, and does not stop the remaining views.
    use cad_app::StandardView;

    let mut queue = VecDeque::from([StandardView::Front, StandardView::Isometric]);
    let mut applied = Vec::new();
    let refusals = drain_standard_views(&mut queue, |view| {
        applied.push(view);
        Ok(())
    });
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        applied,
        vec![StandardView::Front, StandardView::Isometric],
        "admission order is preserved"
    );
    assert!(queue.is_empty(), "everything queued was drained");

    let mut queue = VecDeque::from([StandardView::Back, StandardView::Top]);
    let mut applied = Vec::new();
    let refusals = drain_standard_views(&mut queue, |view| {
        applied.push(view);
        if view == StandardView::Back {
            Err("view.refused: test refusal".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(applied, vec![StandardView::Back, StandardView::Top]);
    assert_eq!(refusals, vec!["view.refused: test refusal".to_string()]);
    assert!(queue.is_empty());
}

#[test]
fn tool_parameter_wire_shape_parses_the_key_and_exactly_one_value() {
    // The exact `cad_set_tool_parameter` wire shape: a stable machine-key string
    // plus exactly one of `number`/`flag`. An unknown key or a malformed value
    // pairing is refused before anything is queued.
    use cad_app::{ToolParameter, ToolValue};

    assert_eq!(
        parse_tool_parameter("polygonSides", Some(6.0), None).unwrap(),
        (ToolParameter::PolygonSides, ToolValue::Number(6.0))
    );
    assert_eq!(
        parse_tool_parameter("alignScale", None, Some(true)).unwrap(),
        (ToolParameter::AlignScale, ToolValue::Flag(true))
    );
    assert_eq!(
        parse_tool_parameter("arrayFillAngle", Some(90.0), None).unwrap(),
        (ToolParameter::ArrayFillAngle, ToolValue::Number(90.0))
    );

    // An unknown key is refused with the key named.
    assert!(parse_tool_parameter("nope", Some(1.0), None)
        .unwrap_err()
        .contains("nope"));
    // Neither value, or both, is refused.
    assert!(parse_tool_parameter("polygonSides", None, None).is_err());
    assert!(parse_tool_parameter("polygonSides", Some(1.0), Some(true)).is_err());
}

#[test]
fn the_property_wire_parses_the_five_supported_keys_and_refuses_everything_else() {
    for key in ["layer", "color", "linetype", "lineweight", "transparency"] {
        assert_eq!(
            parse_property_write(key, "ByLayer").expect("a supported key"),
            (key.to_string(), "ByLayer".to_string())
        );
    }
    // `text` has its own command, every other key has no write path: the refusal
    // is the stable code only (no human-readable message on this wire).
    for key in ["text", "id", "type", "space", "", "COLOR", " layer"] {
        assert_eq!(
            parse_property_write(key, "ByLayer").unwrap_err(),
            "property.unsupported_key",
            "'{key}'"
        );
    }
    // The value is carried verbatim: its grammar is validated against the
    // drawing's tables on the owner thread, never guessed here.
    assert_eq!(
        parse_property_write("layer", "  WALLS  ").expect("the key is supported"),
        ("layer".to_string(), "  WALLS  ".to_string())
    );
}

#[test]
fn property_admission_is_gated_bounded_and_carries_the_pair_verbatim() {
    let mut queue: Vec<(String, String)> = Vec::new();

    // The modal popup gate comes first, like every other CAD mutation: while a
    // popup owns input the write is refused by name.
    assert_eq!(
        admit_selected_property(&mut queue, true, true, "layer".to_string(), "0".to_string())
            .unwrap_err(),
        POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty(), "a gated refusal queues nothing");

    // Probe mode (no CAD session) is refused explicitly.
    assert_eq!(
        admit_selected_property(
            &mut queue,
            false,
            false,
            "layer".to_string(),
            "0".to_string()
        )
        .unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");

    // Admitted verbatim: no entity id, no trimming, no clamping.
    admit_selected_property(
        &mut queue,
        true,
        false,
        "transparency".to_string(),
        "60".to_string(),
    )
    .expect("admitted");
    assert_eq!(queue, vec![("transparency".to_string(), "60".to_string())]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_SELECTED_PROPERTY_CAP {
        admit_selected_property(
            &mut queue,
            true,
            false,
            "layer".to_string(),
            "0".to_string(),
        )
        .expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_SELECTED_PROPERTY_CAP);
    assert_eq!(
        admit_selected_property(
            &mut queue,
            true,
            false,
            "layer".to_string(),
            "0".to_string()
        )
        .unwrap_err(),
        "CAD selected-property queue is full"
    );
}

#[test]
fn settings_admission_is_typed_bounded_and_has_no_modal_gate() {
    // The admission signature intentionally has no `modal_open` parameter: an
    // Options write from the popup window is not CAD input and must stay
    // allowed while the modal gate refuses commands.
    let mut queue = VecDeque::new();
    admit_setting(
        &mut queue,
        true,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .unwrap();
    assert_eq!(queue.len(), 1);

    // Validation happens before the queue.
    assert_eq!(
        admit_setting(
            &mut queue,
            true,
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("png".to_string()),
        )
        .unwrap_err(),
        SETTINGS_INVALID_PLOT_FORMAT
    );
    assert_eq!(queue.len(), 1);

    // Probe mode is explicit.
    assert!(admit_setting(
        &mut VecDeque::new(),
        false,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .is_err());

    // The queue is bounded.
    let mut full = VecDeque::new();
    for _ in 0..CAD_SETTING_CAP {
        admit_setting(
            &mut full,
            true,
            CadSettingKey::Grid,
            CadSettingValue::Bool(true),
        )
        .unwrap();
    }
    assert!(admit_setting(
        &mut full,
        true,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .is_err());
}
