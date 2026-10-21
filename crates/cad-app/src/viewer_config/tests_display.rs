//! Display options: the model/paper backgrounds and the crosshair size.
//!
//! Split out of `viewer_config/tests.rs` so both files stay under the code-file
//! line limit.

use super::*;
use serde_json::json;

fn store_allowing_display() -> ViewerConfigStore {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "view.background.model",
                "view.background.paper",
                "view.crosshair.sizePercent"
            ] } }
        }))
        .unwrap();
    store
}

#[test]
fn hex_colour_grammar_accepts_only_six_digits() {
    // Lower and upper case are equivalent and canonicalise to lower case.
    for (text, canonical) in [
        ("#000000", "#000000"),
        ("#abcdef", "#abcdef"),
        ("#ABCDEF", "#abcdef"),
        ("#aBcD09", "#abcd09"),
        ("#FFFFFF", "#ffffff"),
    ] {
        let colour = HexColor::parse(text).unwrap_or_else(|| panic!("{text} must parse"));
        assert_eq!(colour.to_hex(), canonical);
        assert_eq!(colour.to_string(), canonical);
        // The persisted spelling is the canonical one.
        assert_eq!(
            serde_json::to_string(&colour).unwrap(),
            format!("\"{canonical}\"")
        );
    }
    // The three-digit shorthand, CSS names, a missing/extra digit, a wrong
    // prefix and an empty string are refusals, never expansions.
    for text in [
        "#abc",
        "red",
        "red#ffffff",
        "",
        "#",
        "#fffff",
        "#ffffff0",
        "ffffff",
        "#gggggg",
        " #ffffff",
        "##ffffff",
    ] {
        assert!(
            HexColor::parse(text).is_none(),
            "{text:?} must be refused, not guessed"
        );
    }
    assert_eq!(HexColor::default().to_hex(), "#000000");
}

#[test]
fn hex_colour_decodes_to_the_renderers_linear_channels() {
    assert_eq!(HexColor::BLACK.to_linear_rgba(), [0.0, 0.0, 0.0, 1.0]);
    let white = HexColor::parse("#ffffff").unwrap().to_linear_rgba();
    for channel in &white[..3] {
        assert!((channel - 1.0).abs() < 1e-6, "{white:?}");
    }
    assert_eq!(white[3], 1.0);
    // sRGB mid grey is ~0.216 in linear light (not the raw 0.502).
    let grey = HexColor::parse("#808080").unwrap().to_linear_rgba();
    assert!((grey[0] - 0.2158).abs() < 1e-3, "{grey:?}");
    assert_eq!(
        HexColor::parse("#808080").unwrap().channels(),
        [128, 128, 128]
    );
}

#[test]
fn display_defaults_are_black_backgrounds_and_a_full_crosshair() {
    let config = ViewerConfig::default();
    assert_eq!(config.view.background.model.to_hex(), "#000000");
    assert_eq!(config.view.background.paper.to_hex(), "#000000");
    assert_eq!(config.view.crosshair.size_percent, 100);
    // A JSON document that omits the display block inherits exactly those.
    let parsed: ViewerConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed.view.background, config.view.background);
    assert_eq!(parsed.view.crosshair, config.view.crosshair);
}

#[test]
fn config_json_sets_both_backgrounds_and_the_crosshair() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "view": {
                "background": { "model": "#102030", "paper": "#F0E0D0" },
                "crosshair": { "sizePercent": 25 }
            }
        }))
        .unwrap();
    let effective = store.effective();
    assert_eq!(effective.view.background.model.to_hex(), "#102030");
    assert_eq!(effective.view.background.paper.to_hex(), "#f0e0d0");
    assert_eq!(effective.view.crosshair.size_percent, 25);
}

#[test]
fn a_malformed_colour_is_refused_by_the_schema() {
    for text in ["red", "#abc", "", "#12345g"] {
        let mut store = ViewerConfigStore::default();
        let error = store
            .update_config(json!({ "view": { "background": { "model": text } } }))
            .unwrap_err();
        assert!(
            error.to_string().contains("#rrggbb"),
            "{text:?} must name the accepted grammar: {error}"
        );
        // Atomic: the previous effective config is untouched.
        assert_eq!(store.effective().view.background.model.to_hex(), "#000000");
    }
}

#[test]
fn the_crosshair_percentage_is_validated_not_clamped() {
    for percent in [0, 101, 400] {
        let mut store = ViewerConfigStore::default();
        let error = store
            .update_config(json!({ "view": { "crosshair": { "sizePercent": percent } } }))
            .unwrap_err();
        assert_eq!(error.path, "view.crosshair.sizePercent");
        assert_eq!(store.effective().view.crosshair.size_percent, 100);
    }
    for percent in [1, 50, 100] {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({ "view": { "crosshair": { "sizePercent": percent } } }))
            .unwrap();
        assert_eq!(store.effective().view.crosshair.size_percent, percent);
    }
}

#[test]
fn a_non_integer_crosshair_percentage_is_refused() {
    // A fractional, negative or string percentage is a schema type error, not a
    // value to round or coerce. The effective config is never mutated.
    for patch in [
        json!({ "view": { "crosshair": { "sizePercent": 40.5 } } }),
        json!({ "view": { "crosshair": { "sizePercent": "50" } } }),
        json!({ "view": { "crosshair": { "sizePercent": -1 } } }),
    ] {
        let mut store = ViewerConfigStore::default();
        assert!(
            store.update_config(patch).is_err(),
            "a non-integer percentage must be refused"
        );
        assert_eq!(store.effective().view.crosshair.size_percent, 100);
    }
}

#[test]
fn an_allowed_preference_fills_the_display_leaves() {
    let mut store = store_allowing_display();
    store
        .apply_user_preference(json!({
            "view": {
                "background": { "model": "#112233", "paper": "#445566" },
                "crosshair": { "sizePercent": 40 }
            }
        }))
        .unwrap();
    assert_eq!(store.effective().view.background.model.to_hex(), "#112233");
    assert_eq!(store.effective().view.background.paper.to_hex(), "#445566");
    assert_eq!(store.effective().view.crosshair.size_percent, 40);
    // The durable projection carries exactly those leaves.
    let projected = store.projected_user_preference_json();
    assert!(projected.contains("\"model\":\"#112233\""), "{projected}");
    assert!(projected.contains("\"paper\":\"#445566\""), "{projected}");
    assert!(projected.contains("\"sizePercent\":40"), "{projected}");

    // A preference outside the allowed leaves is refused, not projected.
    let error = store
        .apply_user_preference(json!({ "view": { "overlays": { "grid": true } } }))
        .unwrap_err();
    assert!(error.to_string().contains("not allowed"), "{error}");
}

#[test]
fn background_resolution_picks_the_space_colour() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "view": { "background": { "model": "#010203", "paper": "#0a0b0c" } }
        }))
        .unwrap();
    let background = store.effective().view.background.clone();
    assert_eq!(background.resolve(false).to_hex(), "#010203");
    assert_eq!(background.resolve(true).to_hex(), "#0a0b0c");
}

#[test]
fn the_failed_preference_path_is_unknown_until_the_schema_defines_it() {
    // Guard the new leaves against `allowed_path_is_known`: a typo in the policy
    // must be rejected by `validate`, not silently accepted.
    let mut store = ViewerConfigStore::default();
    let error = store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": ["view.background.colour"] } }
        }))
        .unwrap_err();
    assert_eq!(error.path, "ui.userCustomization.allowedPaths[0]");
}
