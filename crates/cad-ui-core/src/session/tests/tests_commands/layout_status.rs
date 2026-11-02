//! The wire projection of paper-layout tabs and the active drawing space.
//!
//! The tabs are derived from the authoritative drawing (the same descriptors
//! `cad_app::render_scene` renders), the synthetic `LayoutId(0)` "Model"
//! descriptor is filtered because model space is its own tab, and an unsupported
//! layout carries the descriptor's stable technical reason verbatim.

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn layout_tabs_and_active_space_are_projected_from_the_authoritative_drawing() {
    let session = session_with_layouts(&[
        // The synthetic model layout must never become a paper tab.
        (0, "Model", None),
        (2, "Sheet One", None),
        (3, "Broken", Some("importer dropped height")),
    ]);
    session
        .command("SPACE #2")
        .expect("switch to the real paper layout");
    let status = session.snapshot(None, None, 0);

    assert_eq!(
        status.layouts.len(),
        2,
        "the synthetic model descriptor is filtered, paper order preserved"
    );
    assert_eq!(status.layouts[0].id, 2);
    assert_eq!(status.layouts[0].name, "Sheet One");
    assert!(status.layouts[0].supported);
    assert_eq!(status.layouts[0].reason, None);
    assert_eq!(status.layouts[0].viewport_count, 0);

    assert_eq!(status.layouts[1].id, 3);
    assert_eq!(status.layouts[1].name, "Broken");
    assert!(!status.layouts[1].supported);
    assert_eq!(status.layouts[1].viewport_count, 1);

    // The reason is exactly the descriptor's stable technical string, including
    // its code prefix — never a localized guess.
    let expected_reason = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("the fixture drawing");
        cad_app::render_scene::layout_descriptors(drawing.as_ref())
            .into_iter()
            .find(|descriptor| descriptor.id.0 == 3)
            .expect("the broken layout descriptor")
            .reason
    };
    assert_eq!(
        status.layouts[1].reason.as_deref(),
        Some(expected_reason.as_str())
    );
    assert!(
        expected_reason.starts_with("viewport."),
        "the reason keeps its stable code prefix: {expected_reason:?}"
    );

    assert_eq!(status.active_space, CadActiveSpace::Paper { id: 2 });

    // Exact wire shape: additive camelCase keys, and the tagged active space.
    let value = serde_json::to_value(&status).expect("the status serializes");
    let object = value.as_object().expect("a JSON object");
    assert!(object.contains_key("layouts"), "wire key `layouts`");
    assert!(object.contains_key("activeSpace"), "wire key `activeSpace`");
    let layouts = value["layouts"].as_array().expect("an array");
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[0]["id"], serde_json::json!(2));
    assert_eq!(layouts[0]["name"], "Sheet One");
    assert_eq!(layouts[0]["supported"], serde_json::json!(true));
    assert_eq!(layouts[0]["reason"], serde_json::Value::Null);
    assert_eq!(layouts[0]["viewportCount"], serde_json::json!(0));
    assert_eq!(
        layouts[1]["reason"].as_str(),
        Some(expected_reason.as_str()),
        "the technical reason passes through unchanged"
    );
    assert_eq!(
        value["activeSpace"],
        serde_json::json!({"kind": "paper", "id": 2})
    );
}

#[test]
fn no_document_reports_no_layout_tabs_and_model_space() {
    let status = unsupported_status(None);
    assert!(status.layouts.is_empty(), "no drawing means no paper tabs");
    assert_eq!(status.active_space, CadActiveSpace::Model);

    let value = serde_json::to_value(&status).expect("the status serializes");
    assert_eq!(value["layouts"], serde_json::json!([]));
    assert_eq!(value["activeSpace"], serde_json::json!({"kind": "model"}));
}

#[test]
fn the_synthetic_model_layout_is_not_selectable_as_a_paper_name() {
    // `MODEL` is handled by the grammar, and the synthetic layout is filtered
    // from the tab list, so its display name never resolves as a paper tab.
    let session = session_with_layouts(&[(0, "Model", None), (2, "Sheet One", None)]);
    session
        .command("SPACE model")
        .expect("MODEL switches to model space");
    assert_eq!(
        session.snapshot(None, None, 0).active_space,
        CadActiveSpace::Model
    );
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .layouts
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        vec![2]
    );
}
