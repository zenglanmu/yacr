//! The `ui.units` / `ui.recovery` preference leaves (ADR-0037 D9/D12).
//!
//! Both groups are ordinary validated schema leaves: the defaults reproduce
//! today's behaviour exactly, a preference round-trips under the camelCase wire
//! spelling, and every out-of-range or incoherent value is refused **by name**,
//! never clamped.

use super::*;
use serde_json::json;

fn store_with_paths() -> ViewerConfigStore {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "ui.units.drawingUnit",
                "ui.units.displayUnit",
                "ui.units.decimalPlaces",
                "ui.recovery.intervalSeconds"
            ] } }
        }))
        .unwrap();
    store
}

/// The defaults are [`cad_domain::UnitContext::default`] and the ADR's 60 s.
#[test]
fn units_and_recovery_defaults_preserve_todays_behaviour() {
    let config = ViewerConfig::default();
    let units = config.ui.units;
    assert_eq!(units.drawing_unit, UnitToken::Drawing);
    assert_eq!(units.display_unit, UnitToken::Drawing);
    assert_eq!(units.decimal_places, 3);
    assert_eq!(
        units.unit_context().unwrap(),
        cad_domain::UnitContext::default(),
        "the schema default is the domain default, bit for bit"
    );
    assert_eq!(config.ui.recovery.interval_seconds, 60);
    assert_eq!(
        config.ui.recovery.interval_seconds,
        DEFAULT_RECOVERY_INTERVAL_SECONDS
    );

    // The wire spelling of every token, and the camelCase leaves.
    assert_eq!(
        UnitToken::ALL
            .into_iter()
            .map(UnitToken::as_token)
            .collect::<Vec<_>>(),
        vec!["drawing", "mm", "cm", "m", "in", "ft"]
    );
    for token in UnitToken::ALL {
        assert_eq!(UnitToken::parse(token.as_token()), Some(token));
        assert_eq!(UnitToken::from_unit(&token.to_unit()), token);
    }
    assert_eq!(UnitToken::parse("MM"), None, "tokens are lowercase");
    assert_eq!(UnitToken::parse("millimetre"), None);
    let wire = serde_json::to_value(config).unwrap();
    assert_eq!(wire["ui"]["units"]["drawingUnit"], json!("drawing"));
    assert_eq!(wire["ui"]["units"]["displayUnit"], json!("drawing"));
    assert_eq!(wire["ui"]["units"]["decimalPlaces"], json!(3));
    assert_eq!(wire["ui"]["recovery"]["intervalSeconds"], json!(60));
}

/// A preference round-trips through the effective config and the projection.
#[test]
fn units_and_recovery_round_trip_through_the_preference() {
    let mut store = store_with_paths();
    store
        .apply_user_preference(json!({
            "ui": {
                "units": { "drawingUnit": "mm", "displayUnit": "cm", "decimalPlaces": 2 },
                "recovery": { "intervalSeconds": 15 }
            }
        }))
        .unwrap();
    let effective = store.effective();
    assert_eq!(effective.ui.units.drawing_unit, UnitToken::Mm);
    assert_eq!(effective.ui.units.display_unit, UnitToken::Cm);
    assert_eq!(effective.ui.units.decimal_places, 2);
    assert_eq!(effective.ui.recovery.interval_seconds, 15);

    let projected: serde_json::Value =
        serde_json::from_str(&store.projected_user_preference_json()).unwrap();
    assert_eq!(projected["ui"]["units"]["drawingUnit"], json!("mm"));
    assert_eq!(projected["ui"]["units"]["displayUnit"], json!("cm"));
    assert_eq!(projected["ui"]["units"]["decimalPlaces"], json!(2));
    assert_eq!(projected["ui"]["recovery"]["intervalSeconds"], json!(15));

    let reparsed: ViewerConfig =
        serde_json::from_str(&serde_json::to_string(effective).unwrap()).unwrap();
    reparsed.validate().unwrap();
    assert_eq!(&reparsed, effective);
}

/// `UnitContext` derivation: the ratios are exact metre-per-unit quotients.
#[test]
fn unit_context_derives_exact_display_ratios() {
    let context = |drawing: UnitToken, display: UnitToken| {
        UnitsConfig {
            drawing_unit: drawing,
            display_unit: display,
            decimal_places: 3,
        }
        .unit_context()
        .unwrap()
    };

    // cm → m: one centimetre is 0.01 m.
    let cm_m = context(UnitToken::Cm, UnitToken::M);
    assert_eq!(cm_m.source, cad_domain::Unit::Centimeter);
    assert_eq!(cm_m.display, cad_domain::Unit::Meter);
    assert_eq!(cm_m.display_per_source, Some(0.01));
    assert_eq!(cm_m.to_display(250.0), Some(2.5));

    // in → mm: one inch is 25.4 mm.
    let in_mm = context(UnitToken::In, UnitToken::Mm);
    assert_eq!(in_mm.display_per_source, Some(25.4));
    assert_eq!(in_mm.to_display(1.0), Some(25.4));

    // Same unit on both sides is exactly 1.0, not a fitted factor.
    assert_eq!(
        context(UnitToken::M, UnitToken::M).display_per_source,
        Some(1.0)
    );
    assert_eq!(
        context(UnitToken::Ft, UnitToken::Ft).display_per_source,
        Some(1.0)
    );

    // The exact factor table, in metres per unit.
    assert_eq!(UnitToken::Mm.metres_per_unit(), Some(0.001));
    assert_eq!(UnitToken::Cm.metres_per_unit(), Some(0.01));
    assert_eq!(UnitToken::M.metres_per_unit(), Some(1.0));
    assert_eq!(UnitToken::In.metres_per_unit(), Some(0.0254));
    assert_eq!(UnitToken::Ft.metres_per_unit(), Some(0.3048));
    assert_eq!(UnitToken::Drawing.metres_per_unit(), None);

    // drawing → drawing is the untyped default: no ratio at all.
    let untyped = context(UnitToken::Drawing, UnitToken::Drawing);
    assert_eq!(untyped, cad_domain::UnitContext::default());
    assert_eq!(untyped.display_per_source, None);

    // A typed source displayed as raw numbers is the identity ratio: the values
    // are shown as stored, never as a invented conversion.
    assert_eq!(
        context(UnitToken::Mm, UnitToken::Drawing).display_per_source,
        Some(1.0)
    );
}

/// A typed display unit over the untyped `drawing` source is refused by name.
#[test]
fn a_typed_display_over_an_untyped_source_is_refused_by_name() {
    let config = UnitsConfig {
        drawing_unit: UnitToken::Drawing,
        display_unit: UnitToken::M,
        decimal_places: 3,
    };
    let error = config.unit_context().unwrap_err();
    assert_eq!(error.path, "ui.units.displayUnit");
    assert!(
        error.reason.contains("requires a drawing unit"),
        "the refusal names the rule: {error}"
    );

    // The store refuses the same pair, so a preference cannot smuggle it in.
    let mut store = store_with_paths();
    let error = store
        .apply_user_preference(json!({ "ui": { "units": { "displayUnit": "m" } } }))
        .unwrap_err();
    assert_eq!(error.path, "ui.units.displayUnit");
    assert_eq!(store.effective().ui.units.display_unit, UnitToken::Drawing);

    // A typed source with the untyped display stays legal (identity display).
    let mut store = store_with_paths();
    store
        .apply_user_preference(json!({ "ui": { "units": { "drawingUnit": "mm" } } }))
        .unwrap();
    assert_eq!(store.effective().ui.units.drawing_unit, UnitToken::Mm);
    assert_eq!(store.effective().ui.units.display_unit, UnitToken::Drawing);
}

/// Precision and interval bounds are refusals that name the field.
#[test]
fn precision_and_interval_refuse_out_of_range_values_by_name() {
    for places in [9_u32, 200] {
        let mut store = store_with_paths();
        let error = store
            .apply_user_preference(json!({ "ui": { "units": { "decimalPlaces": places } } }))
            .unwrap_err();
        assert_eq!(error.path, "ui.units.decimalPlaces", "places = {places}");
        assert_eq!(store.effective().ui.units.decimal_places, 3);
    }
    // 8 is the ceiling and is accepted.
    let mut store = store_with_paths();
    store
        .apply_user_preference(json!({ "ui": { "units": { "decimalPlaces": 8 } } }))
        .unwrap();
    assert_eq!(store.effective().ui.units.decimal_places, 8);

    for seconds in [3601_u32, 86_400] {
        let mut store = store_with_paths();
        let error = store
            .apply_user_preference(json!({ "ui": { "recovery": { "intervalSeconds": seconds } } }))
            .unwrap_err();
        assert_eq!(
            error.path, "ui.recovery.intervalSeconds",
            "seconds = {seconds}"
        );
        assert_eq!(store.effective().ui.recovery.interval_seconds, 60);
    }
    // 0 (off) and 3600 are both accepted.
    let mut store = store_with_paths();
    store
        .apply_user_preference(json!({ "ui": { "recovery": { "intervalSeconds": 0 } } }))
        .unwrap();
    assert_eq!(store.effective().ui.recovery.interval_seconds, 0);
    store
        .apply_user_preference(json!({ "ui": { "recovery": { "intervalSeconds": 3600 } } }))
        .unwrap();
    assert_eq!(store.effective().ui.recovery.interval_seconds, 3600);
}
