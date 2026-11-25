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
fn parse_property_write_admits_a_bim_pset_key() {
    // A canonical dynamic BIM key is admitted by its shape and carried verbatim:
    // `cad-app` dispatches it to the BIM write path, so the fixed
    // `property_value` table must stay untouched.
    for key in ["bim.pset.0.0", "bim.pset.1.4", "bim.pset.12.63"] {
        assert_eq!(
            parse_property_write(key, "3.5 m").expect("a canonical BIM key"),
            (key.to_string(), "3.5 m".to_string())
        );
    }
    // The fixed table is unchanged: a BIM key is still not one of its keys.
    assert!(!cad_app::property_value::is_supported_key("bim.pset.0.0"));
}

#[test]
fn parse_property_write_refuses_a_malformed_bim_key() {
    // Every non-canonical shape is the same stable refusal as any other unknown
    // key: only the panel's own written form is addressable.
    for key in [
        "bim.pset.x",
        "bim.pset.0",
        "bim.pset.00.0",
        "bim.pset.0.0.1",
        "bim.pset.",
        "bim.pset.0.",
        "BIM.pset.0.0",
        " bim.pset.0.0",
    ] {
        assert_eq!(
            parse_property_write(key, "1").unwrap_err(),
            "property.unsupported_key",
            "'{key}'"
        );
    }
}

#[test]
fn a_plain_unsupported_key_is_still_refused() {
    for key in ["text", "id", "status", "bim_category", ""] {
        assert_eq!(
            parse_property_write(key, "1").unwrap_err(),
            "property.unsupported_key",
            "'{key}'"
        );
    }
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

#[test]
fn orbit_wire_angles_must_be_finite() {
    // The `cad_orbit` wire form has no string shape: `{yaw, pitch}` arrive as
    // radians and are carried verbatim. The one content rule is finiteness — a
    // NaN/infinite delta would corrupt the camera instead of orbiting it, so it is
    // refused with its stable code before any host gate runs.
    assert_eq!(
        parse_orbit(0.25, -0.5),
        Ok(OrbitDelta {
            yaw: 0.25,
            pitch: -0.5
        })
    );
    // Zero is a legal delta: it orbits by nothing, it is not a malformed request.
    assert_eq!(
        parse_orbit(0.0, 0.0),
        Ok(OrbitDelta {
            yaw: 0.0,
            pitch: 0.0
        })
    );

    for (yaw, pitch) in [
        (f64::NAN, 0.1),
        (0.1, f64::NAN),
        (f64::INFINITY, 0.1),
        (0.1, f64::NEG_INFINITY),
        (f64::NAN, f64::NAN),
    ] {
        assert_eq!(
            parse_orbit(yaw, pitch).unwrap_err(),
            ORBIT_ANGLES_NOT_FINITE,
            "yaw={yaw} pitch={pitch}"
        );
    }
    assert_eq!(
        ORBIT_ANGLES_NOT_FINITE,
        "orbit.angles_not_finite: cad_orbit needs finite yaw and pitch"
    );
}

#[test]
fn orbit_admission_is_bounded_and_refuses_probe_mode_and_the_modal_gate() {
    // The `cad_orbit` ack path: the typed delta is only *queued* for the owner
    // thread. The gates are the standard-view ones, in the same order: the modal
    // popup gate first, then a disabled CAD session, then the bound.
    let delta = OrbitDelta {
        yaw: 0.3,
        pitch: 0.2,
    };

    let mut queue: VecDeque<OrbitDelta> = VecDeque::new();
    assert_eq!(
        admit_orbit(&mut queue, false, false, delta).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert_eq!(
        admit_orbit(&mut queue, true, true, delta).unwrap_err(),
        POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty(), "refusals queue nothing");

    admit_orbit(&mut queue, true, false, delta).expect("accepted");
    assert_eq!(queue.pop_front(), Some(delta));

    for _ in 0..CAD_ORBIT_CAP {
        admit_orbit(&mut queue, true, false, delta).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_ORBIT_CAP);
    assert_eq!(
        admit_orbit(&mut queue, true, false, delta).unwrap_err(),
        "CAD orbit queue is full"
    );
    // The modal gate is checked before the bound, exactly like `admit_command` and
    // `admit_standard_view`: a full queue behind an open modal reports the modal
    // reason.
    assert_eq!(
        admit_orbit(&mut queue, true, true, delta).unwrap_err(),
        POPUP_MODAL_OPEN
    );
    // The orbit bound is the standard-view bound, not a second invented number.
    assert_eq!(CAD_ORBIT_CAP, CAD_STANDARD_VIEW_CAP);
}

#[test]
fn drained_orbits_apply_in_order_and_collect_refusals() {
    // Two queued deltas apply in admission order (the last one leaves the camera)
    // and the queue is emptied. A real refusal is returned, never swallowed, and
    // does not stop the remaining deltas.
    let mut queue = VecDeque::from([
        OrbitDelta {
            yaw: 0.1,
            pitch: 0.0,
        },
        OrbitDelta {
            yaw: 0.2,
            pitch: 0.0,
        },
    ]);
    let mut applied = Vec::new();
    let refusals = drain_orbits(&mut queue, |delta| {
        applied.push(delta);
        Ok(())
    });
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        applied,
        vec![
            OrbitDelta {
                yaw: 0.1,
                pitch: 0.0
            },
            OrbitDelta {
                yaw: 0.2,
                pitch: 0.0
            }
        ],
        "admission order is preserved"
    );
    assert!(queue.is_empty(), "everything queued was drained");

    let mut queue = VecDeque::from([
        OrbitDelta {
            yaw: 1.0,
            pitch: 0.0,
        },
        OrbitDelta {
            yaw: 2.0,
            pitch: 0.0,
        },
    ]);
    let mut applied = Vec::new();
    let refusals = drain_orbits(&mut queue, |delta| {
        applied.push(delta);
        if delta.yaw == 1.0 {
            Err("orbit.refused: test refusal".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(applied.len(), 2, "a refusal does not skip the rest");
    assert_eq!(refusals, vec!["orbit.refused: test refusal".to_string()]);
    assert!(queue.is_empty());
}

#[test]
fn set_view_wire_parses_one_preset_or_one_validated_camera() {
    use crate::session::wire::{SetViewCameraArgs, SetViewProjectionArgs};
    use cad_app::{ViewModeRequest, ViewPlacement, ViewPreset};

    for preset in ViewPreset::ALL {
        assert_eq!(
            parse_set_view(Some(preset.key()), None),
            Ok(ViewPlacement::Preset(preset)),
            "{}",
            preset.key()
        );
    }
    assert_eq!(
        parse_set_view(Some(" OVERVIEW "), None),
        Ok(ViewPlacement::Preset(ViewPreset::Overview))
    );

    let error = parse_set_view(Some("diagonal"), None).unwrap_err();
    assert!(error.starts_with("view.set.unknown_preset"), "{error}");
    let error = parse_set_view(Some(""), None).unwrap_err();
    assert!(error.starts_with("view.set.unknown_preset"), "{error}");

    let perspective = SetViewCameraArgs {
        eye: [0.0, -10.0, 10.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        projection: SetViewProjectionArgs::Perspective {
            vertical_fov_radians: std::f64::consts::FRAC_PI_4,
        },
        mode: "3d".into(),
    };
    let Ok(ViewPlacement::Camera { camera, mode }) =
        parse_set_view(None, Some(perspective.clone()))
    else {
        panic!("a valid 3d camera must parse");
    };
    assert_eq!(mode, ViewModeRequest::ThreeD);
    assert_eq!(
        camera.eye,
        cad_domain::Point3 {
            x: 0.0,
            y: -10.0,
            z: 10.0
        }
    );
    assert!(!camera.projection.is_orthographic());

    let orthographic = SetViewCameraArgs {
        eye: [0.0, 0.0, 10.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: SetViewProjectionArgs::Orthographic { scale: 1.0 },
        mode: "2d".into(),
    };
    assert!(matches!(
        parse_set_view(None, Some(orthographic.clone())),
        Ok(ViewPlacement::Camera {
            mode: ViewModeRequest::Plan,
            ..
        })
    ));

    let mismatch = SetViewCameraArgs {
        mode: "2d".into(),
        ..perspective.clone()
    };
    let error = parse_set_view(None, Some(mismatch)).unwrap_err();
    assert!(
        error.starts_with("view.set.mode_projection_mismatch"),
        "{error}"
    );

    let mismatch = SetViewCameraArgs {
        mode: "3d".into(),
        ..orthographic.clone()
    };
    let error = parse_set_view(None, Some(mismatch)).unwrap_err();
    assert!(
        error.starts_with("view.set.mode_projection_mismatch"),
        "{error}"
    );

    let unknown_mode = SetViewCameraArgs {
        mode: "orbit".into(),
        ..perspective.clone()
    };
    let error = parse_set_view(None, Some(unknown_mode)).unwrap_err();
    assert!(error.starts_with("view.set.unknown_mode"), "{error}");

    let non_finite = SetViewCameraArgs {
        eye: [f64::NAN, 0.0, 1.0],
        ..perspective.clone()
    };
    let error = parse_set_view(None, Some(non_finite)).unwrap_err();
    assert!(error.starts_with("view.set.invalid_camera"), "{error}");

    let coincident = SetViewCameraArgs {
        eye: [0.0, 0.0, 0.0],
        ..perspective.clone()
    };
    let error = parse_set_view(None, Some(coincident)).unwrap_err();
    assert!(error.starts_with("view.set.invalid_camera"), "{error}");

    let invalid_projection = SetViewCameraArgs {
        projection: SetViewProjectionArgs::Perspective {
            vertical_fov_radians: f64::NAN,
        },
        ..perspective.clone()
    };
    let error = parse_set_view(None, Some(invalid_projection)).unwrap_err();
    assert!(error.starts_with("view.set.invalid_projection"), "{error}");

    assert!(parse_set_view(None, None)
        .unwrap_err()
        .starts_with("view.set.request_invalid"));
    assert!(parse_set_view(Some("plan"), Some(orthographic))
        .unwrap_err()
        .starts_with("view.set.request_invalid"));
}

#[test]
fn set_view_admission_is_bounded_and_refuses_probe_mode_and_the_modal_gate() {
    use cad_app::{ViewPlacement, ViewPreset};
    let placement = ViewPlacement::Preset(ViewPreset::Plan);
    let mut queue: VecDeque<ViewPlacement> = VecDeque::new();
    assert_eq!(
        admit_set_view(&mut queue, false, false, placement).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty());
    assert_eq!(
        admit_set_view(&mut queue, true, true, placement).unwrap_err(),
        POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty());

    admit_set_view(&mut queue, true, false, placement).unwrap();
    for _ in 1..CAD_SET_VIEW_CAP {
        admit_set_view(&mut queue, true, false, placement).unwrap();
    }
    assert_eq!(
        admit_set_view(&mut queue, true, false, placement).unwrap_err(),
        "CAD set-view queue is full"
    );
    // The modal gate is still checked first when the queue is also full.
    assert_eq!(
        admit_set_view(&mut queue, true, true, placement).unwrap_err(),
        POPUP_MODAL_OPEN
    );

    let mut applied = Vec::new();
    let refusals = drain_set_views(&mut queue, |placement| {
        applied.push(placement);
        Ok(())
    });
    assert!(refusals.is_empty());
    assert!(queue.is_empty());
    assert_eq!(applied.len(), CAD_SET_VIEW_CAP);
}

#[test]
fn setview_names_the_missing_payload_on_the_text_funnel() {
    use super::test_support::{session, FakeFiles};
    use std::cell::RefCell;
    use std::rc::Rc;

    // `SETVIEW` is a selector only so the vocabulary contract has a name for
    // `CommandId::SetView`; the typed `cad_set_view` channel is the real entry.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("SETVIEW").unwrap_err();
    assert!(
        error.contains("SetView needs a placement payload"),
        "{error}"
    );
}
