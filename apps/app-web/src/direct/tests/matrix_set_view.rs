//! The `cad_set_view` equivalence cases.
//!
//! Split out of `matrix.rs` for the code-size gate: one case per wire shape
//! (preset, absolute camera, malformed forms) driven through the same
//! desktop/direct comparison.
use super::matrix::LIVE;
use super::matrix::MODAL;

pub(super) const SET_VIEW_CASES: &[(&str, &str, &str, (bool, bool))] = &[
    (
        "set-view/preset",
        "cad_set_view",
        r#"{"preset":"overview"}"#,
        LIVE,
    ),
    (
        "set-view/plan",
        "cad_set_view",
        r#"{"preset":"plan"}"#,
        LIVE,
    ),
    (
        "set-view/camera",
        "cad_set_view",
        r#"{"camera":{"eye":[0,-10,10],"target":[0,0,0],"up":[0,0,1],"projection":{"kind":"perspective","verticalFovRadians":0.785398},"mode":"3d"}}"#,
        LIVE,
    ),
    (
        "set-view/unknown-preset",
        "cad_set_view",
        r#"{"preset":"nope"}"#,
        LIVE,
    ),
    (
        "set-view/both-forms",
        "cad_set_view",
        r#"{"preset":"plan","camera":{"eye":[0,0,10],"target":[0,0,0],"up":[0,1,0],"projection":{"kind":"orthographic","scale":1.0},"mode":"2d"}}"#,
        LIVE,
    ),
    (
        "set-view/mode-mismatch",
        "cad_set_view",
        r#"{"camera":{"eye":[0,-10,10],"target":[0,0,0],"up":[0,0,1],"projection":{"kind":"perspective","verticalFovRadians":0.785398},"mode":"2d"}}"#,
        LIVE,
    ),
    (
        "set-view/modal",
        "cad_set_view",
        r#"{"preset":"overview"}"#,
        MODAL,
    ),
];
