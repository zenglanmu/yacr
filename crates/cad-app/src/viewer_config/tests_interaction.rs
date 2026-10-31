//! The persisted interaction leaves (`SNAP`, `ORTHO`, `SNAPUNIT`).
//!
//! These three follow the exact nullable-preference machinery the older
//! `interaction.snapEnabled`/`hoverPreview` leaves use: `None`/`null` means "the
//! user never chose", a real value round-trips through the effective config and
//! the durable projection, and an unusable value is refused by name.

use super::*;
use serde_json::json;

fn store_with_drafting_paths() -> ViewerConfigStore {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "interaction.gridSnapEnabled",
                "interaction.orthoEnabled",
                "interaction.snapSpacing"
            ] } }
        }))
        .unwrap();
    store
}

/// `interaction.leftDragPans` is a plain (non-nullable) switch, the same
/// allowed-preference machinery `interaction.snapEnabled` uses: absent keeps the
/// documented `false` default, an allowed preference fills the leaf, it
/// serializes under the camelCase wire spelling and the projection carries it.
#[test]
fn left_drag_pans_defaults_to_false_and_projects_the_camel_case_leaf() {
    let defaults = ViewerConfig::default().interaction;
    assert!(
        !defaults.left_drag_pans,
        "the default scheme is left drag = box select"
    );
    let default_json = serde_json::to_value(ViewerConfig::default()).unwrap();
    assert_eq!(default_json["interaction"]["leftDragPans"], json!(false));

    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "interaction.leftDragPans"
            ] } }
        }))
        .unwrap();
    assert!(!store.effective().interaction.left_drag_pans);
    store
        .apply_user_preference(json!({ "interaction": { "leftDragPans": true } }))
        .unwrap();
    assert!(store.effective().interaction.left_drag_pans);

    // Parsing and serializing keep the value under the same wire key.
    let parsed: ViewerConfig =
        serde_json::from_str(r#"{"interaction":{"leftDragPans":true}}"#).unwrap();
    parsed.validate().unwrap();
    assert!(parsed.interaction.left_drag_pans);
    assert_eq!(&parsed.interaction, &store.effective().interaction);
    let roundtrip: ViewerConfig =
        serde_json::from_str(&serde_json::to_string(store.effective()).unwrap()).unwrap();
    assert!(roundtrip.interaction.left_drag_pans);
    // A host config that names the leaf reproduces it.
    let mut host = ViewerConfigStore::default();
    host.set_config_json(r#"{"interaction":{"leftDragPans":true}}"#)
        .unwrap();
    assert!(host.effective().interaction.left_drag_pans);
    // The projection a host may persist carries exactly this leaf.
    let projected: serde_json::Value =
        serde_json::from_str(&store.projected_user_preference_json()).unwrap();
    assert_eq!(projected["interaction"]["leftDragPans"], json!(true));

    // A non-boolean value is a type error naming the field, and the store keeps
    // the previous value (atomic refusal).
    for bad in [json!(1), json!("true"), json!(null)] {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({
                "ui": { "userCustomization": { "allowedPaths": [
                    "interaction.leftDragPans"
                ] } }
            }))
            .unwrap();
        let error = store
            .apply_user_preference(json!({ "interaction": { "leftDragPans": bad } }))
            .unwrap_err();
        assert_eq!(
            error.path, "interaction.leftDragPans",
            "{bad} must be refused by name"
        );
        assert!(!store.effective().interaction.left_drag_pans);
    }
}

/// The three drafting leaves use the same nullable preference machinery as
/// `snapEnabled`: absent is `None`, a value round-trips through the effective
/// config and the projection, and a non-finite or non-positive spacing is refused
/// by name, never clamped to the default.
#[test]
fn drafting_interaction_leaves_round_trip_and_reject_invalid_spacing() {
    let defaults = ViewerConfig::default().interaction;
    assert_eq!(defaults.grid_snap_enabled, None);
    assert_eq!(defaults.ortho_enabled, None);
    assert_eq!(defaults.snap_spacing, None);
    let default_json = serde_json::to_value(ViewerConfig::default()).unwrap();
    assert_eq!(default_json["interaction"]["gridSnapEnabled"], json!(null));
    assert_eq!(default_json["interaction"]["orthoEnabled"], json!(null));
    assert_eq!(default_json["interaction"]["snapSpacing"], json!(null));

    let mut store = store_with_drafting_paths();
    store
        .apply_user_preference(json!({
            "interaction": { "gridSnapEnabled": true, "orthoEnabled": true, "snapSpacing": 2.5 }
        }))
        .unwrap();
    let effective = store.effective();
    assert_eq!(effective.interaction.grid_snap_enabled, Some(true));
    assert_eq!(effective.interaction.ortho_enabled, Some(true));
    assert_eq!(effective.interaction.snap_spacing, Some(2.5));
    // Parsing and serializing keep the real value (never a rounded integer).
    let parsed: ViewerConfig = serde_json::from_str(
        r#"{"interaction":{"gridSnapEnabled":true,"orthoEnabled":true,"snapSpacing":2.5}}"#,
    )
    .unwrap();
    parsed.validate().unwrap();
    assert_eq!(&parsed.interaction, &effective.interaction);
    let roundtrip: ViewerConfig =
        serde_json::from_str(&serde_json::to_string(effective).unwrap()).unwrap();
    assert_eq!(&roundtrip, effective);
    // A host config that names the leaves reproduces them in the effective
    // config (the same `set_config_json` path `config.json` loads through).
    let mut host = ViewerConfigStore::default();
    host.set_config_json(
        r#"{"interaction":{"gridSnapEnabled":false,"orthoEnabled":false,"snapSpacing":5}}"#,
    )
    .unwrap();
    assert_eq!(host.effective().interaction.grid_snap_enabled, Some(false));
    assert_eq!(host.effective().interaction.ortho_enabled, Some(false));
    assert_eq!(host.effective().interaction.snap_spacing, Some(5.0));
    // The projection a host may persist carries exactly these leaves.
    let projected: serde_json::Value =
        serde_json::from_str(&store.projected_user_preference_json()).unwrap();
    assert_eq!(projected["interaction"]["gridSnapEnabled"], json!(true));
    assert_eq!(projected["interaction"]["orthoEnabled"], json!(true));
    assert_eq!(projected["interaction"]["snapSpacing"], json!(2.5));

    // An invalid spacing is an explicit error naming the field, and the store
    // stays unchanged (atomic refusal).
    for bad in [json!(0), json!(0.0), json!(-1.5), json!(-2)] {
        let mut store = store_with_drafting_paths();
        let error = store
            .apply_user_preference(json!({ "interaction": { "snapSpacing": bad } }))
            .unwrap_err();
        assert_eq!(
            error.path, "interaction.snapSpacing",
            "{bad} must be refused"
        );
        assert_eq!(store.effective().interaction.snap_spacing, None);
    }
    // A non-finite value cannot appear in JSON, so the validator is the authority
    // a programmatic config hits.
    for spacing in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -3.0] {
        let mut config = ViewerConfig::default();
        config.interaction.snap_spacing = Some(spacing);
        let error = config.validate().unwrap_err();
        assert_eq!(
            error.path, "interaction.snapSpacing",
            "{spacing:?} must be refused by name"
        );
    }
    // A valid programmatic spacing validates like the parsed one.
    let mut config = ViewerConfig::default();
    config.interaction.snap_spacing = Some(0.5);
    config.validate().unwrap();
}
