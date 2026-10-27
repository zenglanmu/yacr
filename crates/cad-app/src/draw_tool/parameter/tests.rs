//! Contract tests for the typed tool-parameter channel (ADR 0016 decision A).
//!
//! Split out of `parameter.rs` to keep that module inside the code-size gate;
//! the module under test is the parent (`super`), and nothing here writes state:
//! every case exercises the pure value model (`ToolParameter`).

use super::*;

/// A refusal must name the parameter and the accepted range (or kind).
fn assert_refusal_names_parameter(error: CadError, key: &str) {
    match error {
        CadError::InvalidInput(message) => {
            assert!(
                message.contains(key),
                "the refusal must name the parameter '{key}': {message}"
            );
        }
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn every_member_has_a_unique_stable_key_and_round_trips() {
    assert_eq!(ToolParameter::ALL.len(), ToolParameter::COUNT);
    let mut keys = std::collections::BTreeSet::new();
    for parameter in ToolParameter::ALL {
        assert!(
            keys.insert(parameter.key()),
            "duplicate key {}",
            parameter.key()
        );
        assert_eq!(
            ToolParameter::from_key(parameter.key()),
            Some(parameter),
            "the key must round-trip"
        );
    }
    assert_eq!(ToolParameter::from_key("nope"), None);
}

#[test]
fn defaults_are_the_documented_values() {
    assert_eq!(
        ToolParameter::PolygonSides.default_value(),
        ToolValue::Number(4.0)
    );
    assert_eq!(
        ToolParameter::FilletRadius.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::ChamferDistanceA.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::ChamferDistanceB.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::AlignScale.default_value(),
        ToolValue::Flag(false)
    );
    assert_eq!(
        ToolParameter::ArrayRows.default_value(),
        ToolValue::Number(1.0)
    );
    assert_eq!(
        ToolParameter::ArrayColumns.default_value(),
        ToolValue::Number(1.0)
    );
    assert_eq!(
        ToolParameter::ArrayRowSpacing.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::ArrayColumnSpacing.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::ArrayItems.default_value(),
        ToolValue::Number(1.0)
    );
    assert_eq!(
        ToolParameter::ArrayFillAngle.default_value(),
        ToolValue::Number(360.0)
    );
    assert_eq!(
        ToolParameter::ArrayRotateItems.default_value(),
        ToolValue::Flag(false)
    );

    // The store starts at exactly those defaults.
    let store = ToolParameterStore::defaults();
    for parameter in ToolParameter::ALL {
        assert_eq!(store.get(parameter), parameter.default_value());
    }
}

#[test]
fn polygon_sides_accepts_the_integer_range_and_refuses_fractional_or_out_of_range() {
    for accepted in [3.0, 4.0, 1024.0] {
        ToolParameter::PolygonSides
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
    }
    // Fractional 4.5 is refused, not rounded.
    assert_refusal_names_parameter(
        ToolParameter::PolygonSides
            .validate(ToolValue::Number(4.5))
            .unwrap_err(),
        "polygonSides",
    );
    for refused in [2.0, 0.0, 1025.0, f64::NAN, f64::INFINITY] {
        assert_refusal_names_parameter(
            ToolParameter::PolygonSides
                .validate(ToolValue::Number(refused))
                .unwrap_err(),
            "polygonSides",
        );
    }
    // A Flag for a numeric parameter is a kind mismatch.
    assert_refusal_names_parameter(
        ToolParameter::PolygonSides
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "polygonSides",
    );
}

#[test]
fn fillet_radius_and_chamfer_distances_accept_zero_and_positive_finite() {
    for parameter in [
        ToolParameter::FilletRadius,
        ToolParameter::ChamferDistanceA,
        ToolParameter::ChamferDistanceB,
    ] {
        for accepted in [0.0, 0.5, 1e9] {
            parameter
                .validate(ToolValue::Number(accepted))
                .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
        }
        for refused in [-1e-9, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_refusal_names_parameter(
                parameter.validate(ToolValue::Number(refused)).unwrap_err(),
                parameter.key(),
            );
        }
        assert_refusal_names_parameter(
            parameter.validate(ToolValue::Flag(true)).unwrap_err(),
            parameter.key(),
        );
    }
}

#[test]
fn array_counts_accept_one_and_above_and_refuse_zero_fractional_and_negative() {
    for parameter in [
        ToolParameter::ArrayRows,
        ToolParameter::ArrayColumns,
        ToolParameter::ArrayItems,
    ] {
        for accepted in [1.0, 2.0, 100.0] {
            parameter
                .validate(ToolValue::Number(accepted))
                .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
        }
        for refused in [0.0, 0.5, -1.0, f64::NAN, f64::INFINITY] {
            assert_refusal_names_parameter(
                parameter.validate(ToolValue::Number(refused)).unwrap_err(),
                parameter.key(),
            );
        }
        assert_refusal_names_parameter(
            parameter.validate(ToolValue::Flag(true)).unwrap_err(),
            parameter.key(),
        );
    }
}

#[test]
fn array_spacing_accepts_any_finite_value_and_refuses_non_finite() {
    for parameter in [
        ToolParameter::ArrayRowSpacing,
        ToolParameter::ArrayColumnSpacing,
    ] {
        // The channel accepts any finite spacing (including negative); the
        // ARRAYRECT command (P4c) is what refuses a negative spacing.
        for accepted in [0.0, 1.5, -2.0] {
            parameter
                .validate(ToolValue::Number(accepted))
                .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
        }
        for refused in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_refusal_names_parameter(
                parameter.validate(ToolValue::Number(refused)).unwrap_err(),
                parameter.key(),
            );
        }
        assert_refusal_names_parameter(
            parameter.validate(ToolValue::Flag(false)).unwrap_err(),
            parameter.key(),
        );
    }
}

#[test]
fn array_fill_angle_accepts_within_a_turn_including_the_full_circle_and_refuses_zero_and_beyond() {
    // The legal range is `0 < |angle| <= 360`: the full-circle default 360 is
    // re-settable (and -360 too), so a user who steps away can return to it.
    for accepted in [90.0, 180.0, 359.0, -90.0, 0.5, 360.0, -360.0] {
        ToolParameter::ArrayFillAngle
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
    }
    // 0, anything beyond a single turn, and non-finite values are refused
    // (no implicit wrap).
    for refused in [0.0, 360.5, 720.0, -720.0, -360.5, f64::NAN, f64::INFINITY] {
        assert_refusal_names_parameter(
            ToolParameter::ArrayFillAngle
                .validate(ToolValue::Number(refused))
                .unwrap_err(),
            "arrayFillAngle",
        );
    }
    assert_refusal_names_parameter(
        ToolParameter::ArrayFillAngle
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "arrayFillAngle",
    );
}

#[test]
fn flags_accept_booleans_and_refuse_numbers() {
    for parameter in [ToolParameter::AlignScale, ToolParameter::ArrayRotateItems] {
        assert!(parameter.validate(ToolValue::Flag(true)).is_ok());
        assert!(parameter.validate(ToolValue::Flag(false)).is_ok());
        assert_refusal_names_parameter(
            parameter.validate(ToolValue::Number(1.0)).unwrap_err(),
            parameter.key(),
        );
    }
}

#[test]
fn the_store_round_trips_a_validated_value_per_slot() {
    let mut store = ToolParameterStore::defaults();
    store.set(ToolParameter::PolygonSides, ToolValue::Number(7.0));
    store.set(ToolParameter::AlignScale, ToolValue::Flag(true));
    assert_eq!(
        store.get(ToolParameter::PolygonSides),
        ToolValue::Number(7.0)
    );
    assert_eq!(store.get(ToolParameter::AlignScale), ToolValue::Flag(true));
    // Untouched slots keep their defaults.
    assert_eq!(
        store.get(ToolParameter::ArrayFillAngle),
        ToolValue::Number(360.0)
    );
}

#[test]
fn hatch_defaults_are_valid_and_documented() {
    assert_eq!(
        ToolParameter::HatchPatternType.default_value(),
        ToolValue::Number(HATCH_PATTERN_TYPE_SOLID)
    );
    assert_eq!(
        ToolParameter::HatchPattern.default_value(),
        ToolValue::Number(1.0),
        "the default predefined pattern is the first table entry (ANSI31)"
    );
    assert_eq!(
        ToolParameter::HatchSpacing.default_value(),
        ToolValue::Number(1.0)
    );
    assert_eq!(
        ToolParameter::HatchAngle.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::HatchDouble.default_value(),
        ToolValue::Flag(false)
    );
    assert_eq!(
        ToolParameter::HatchStyle.default_value(),
        ToolValue::Number(HATCH_STYLE_NORMAL)
    );
    // A default the channel would refuse at commit would make the tool limp
    // into a forced error, so every HATCH default must itself be valid.
    for parameter in [
        ToolParameter::HatchPatternType,
        ToolParameter::HatchPattern,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
    ] {
        parameter
            .validate(parameter.default_value())
            .unwrap_or_else(|error| panic!("{parameter:?} default refused: {error}"));
    }
    assert_eq!(ToolParameter::HatchDouble.kind(), ToolValueKind::Flag);
    for parameter in [
        ToolParameter::HatchPatternType,
        ToolParameter::HatchPattern,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchStyle,
    ] {
        assert_eq!(parameter.kind(), ToolValueKind::Number, "{parameter:?}");
    }
}

#[test]
fn hatch_spacing_is_finite_and_strictly_positive() {
    for accepted in [1e-9, 0.5, 1.0, 1e9] {
        ToolParameter::HatchSpacing
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
    }
    for refused in [0.0, -1e-9, -2.0, f64::NAN, f64::INFINITY] {
        assert_refusal_names_parameter(
            ToolParameter::HatchSpacing
                .validate(ToolValue::Number(refused))
                .unwrap_err(),
            "hatchSpacing",
        );
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchSpacing
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "hatchSpacing",
    );
}

#[test]
fn hatch_angle_accepts_any_finite_value_and_refuses_non_finite() {
    for accepted in [0.0, 90.0, -270.0, 1e6] {
        ToolParameter::HatchAngle
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
    }
    for refused in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_refusal_names_parameter(
            ToolParameter::HatchAngle
                .validate(ToolValue::Number(refused))
                .unwrap_err(),
            "hatchAngle",
        );
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchAngle
            .validate(ToolValue::Flag(false))
            .unwrap_err(),
        "hatchAngle",
    );
}

#[test]
fn hatch_pattern_type_and_style_are_closed_sets() {
    // Pattern type: 0 solid, 1 user-defined, 2 predefined (the hand-transcribed
    // ANSI31/ANSI32/ANSI37 table), 3 gradient. 4 and anything non-integral are
    // outside the implemented set (custom `76=2` patterns and `.pat` files are
    // NOT IMPLEMENTED).
    for accepted in [
        HATCH_PATTERN_TYPE_SOLID,
        HATCH_PATTERN_TYPE_USER_DEFINED,
        HATCH_PATTERN_TYPE_PREDEFINED,
        HATCH_PATTERN_TYPE_GRADIENT,
    ] {
        ToolParameter::HatchPatternType
            .validate(ToolValue::Number(accepted))
            .unwrap();
    }
    for refused in [4.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
        let message = match ToolParameter::HatchPatternType
            .validate(ToolValue::Number(refused))
            .unwrap_err()
        {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("hatchPatternType"), "{message}");
        assert!(message.contains("user-defined"), "{message}");
        assert!(message.contains("predefined"), "{message}");
    }
    // Style: 0 Normal, 1 Outer, 2 Ignore.
    for accepted in [HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER, HATCH_STYLE_IGNORE] {
        ToolParameter::HatchStyle
            .validate(ToolValue::Number(accepted))
            .unwrap();
    }
    for refused in [3.0, -1.0, 1.5, f64::NAN] {
        let message = match ToolParameter::HatchStyle
            .validate(ToolValue::Number(refused))
            .unwrap_err()
        {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("hatchStyle"), "{message}");
        assert!(message.contains("Ignore"), "{message}");
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchStyle
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "hatchStyle",
    );
}

#[test]
fn the_hatch_pattern_index_is_a_one_based_closed_set_of_the_implemented_table() {
    let count = cad_domain::hatch::predefined_patterns().len();
    assert_eq!(count, 3, "ANSI31, ANSI32 and ANSI37 are implemented");
    for index in 1..=count {
        ToolParameter::HatchPattern
            .validate(ToolValue::Number(index as f64))
            .unwrap_or_else(|error| panic!("index {index} refused: {error}"));
        // The accepted index really resolves, and it is 1-based.
        assert_eq!(
            cad_domain::hatch::predefined_pattern_by_index(index).map(|pattern| pattern.name),
            Some(cad_domain::hatch::predefined_patterns()[index - 1].name),
            "index {index} is 1-based"
        );
    }
    // 0 (an off-by-one), anything past the table, a fraction and non-finite
    // values are refused by name: never clamped onto the nearest pattern.
    for refused in [0.0, -1.0, (count + 1) as f64, 1.5, f64::NAN, f64::INFINITY] {
        let message = match ToolParameter::HatchPattern
            .validate(ToolValue::Number(refused))
            .unwrap_err()
        {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("hatchPattern"), "{message}");
        assert!(message.contains("ANSI31"), "{message}");
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchPattern
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "hatchPattern",
    );
    // The member joined the frozen vocabulary with its own stable key.
    assert_eq!(ToolParameter::HatchPattern.key(), "hatchPattern");
    assert_eq!(
        ToolParameter::from_key("hatchPattern"),
        Some(ToolParameter::HatchPattern)
    );
}

#[test]
fn hatch_gradient_defaults_are_valid_and_visually_distinct() {
    // The four gradient parameters joined the frozen vocabulary with their own
    // stable keys, appended after the existing members so the slot order of the
    // pre-existing parameters does not move.
    for (parameter, key, index) in [
        (ToolParameter::HatchGradientKind, "hatchGradientKind", 19),
        (ToolParameter::HatchGradientAngle, "hatchGradientAngle", 20),
        (
            ToolParameter::HatchGradientColorA,
            "hatchGradientColorA",
            21,
        ),
        (
            ToolParameter::HatchGradientColorB,
            "hatchGradientColorB",
            22,
        ),
    ] {
        assert_eq!(parameter.key(), key);
        assert_eq!(ToolParameter::from_key(key), Some(parameter));
        assert_eq!(parameter.index(), index, "{key} slot");
        assert_eq!(parameter.kind(), ToolValueKind::Number, "{key}");
        // A default the channel would refuse at commit would make the tool limp
        // into a forced error, so every gradient default must itself be valid.
        parameter
            .validate(parameter.default_value())
            .unwrap_or_else(|error| panic!("{key} default refused: {error}"));
    }
    assert_eq!(
        ToolParameter::HatchGradientKind.default_value(),
        ToolValue::Number(HATCH_GRADIENT_LINEAR)
    );
    assert_eq!(
        ToolParameter::HatchGradientAngle.default_value(),
        ToolValue::Number(0.0)
    );
    assert_eq!(
        ToolParameter::HatchGradientColorA.default_value(),
        ToolValue::Number(1.0),
        "the start stop defaults to ACI 1 (red)"
    );
    assert_eq!(
        ToolParameter::HatchGradientColorB.default_value(),
        ToolValue::Number(5.0),
        "the end stop defaults to ACI 5 (blue)"
    );
    // The two defaults are real palette colours (ACI 0 is the ByBlock
    // placeholder with no sRGB triple at all) and they differ from each other.
    let rgb = |parameter: ToolParameter| {
        let ToolValue::Number(index) = parameter.default_value() else {
            panic!("{parameter:?} default is not a number");
        };
        crate::property_value::rgb_for_aci(index as u16)
            .unwrap_or_else(|| panic!("{parameter:?} default ACI {index} has no colour"))
    };
    assert_ne!(
        rgb(ToolParameter::HatchGradientColorA),
        rgb(ToolParameter::HatchGradientColorB),
        "the default stops must be visually distinct"
    );
    assert_eq!(rgb(ToolParameter::HatchGradientColorA), [255, 0, 0]);
    assert_eq!(rgb(ToolParameter::HatchGradientColorB), [0, 0, 255]);
}

#[test]
fn hatch_gradient_kind_is_a_closed_set_of_linear_and_spherical() {
    for accepted in [HATCH_GRADIENT_LINEAR, HATCH_GRADIENT_SPHERICAL] {
        ToolParameter::HatchGradientKind
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("kind {accepted} refused: {error}"));
    }
    // 2 is `CYLINDER`-adjacent territory and is not offered at all: the domain
    // bakes `CYLINDER` as spherical, so a second spelling would be a second name
    // for one behaviour.
    for refused in [2.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
        let message = match ToolParameter::HatchGradientKind
            .validate(ToolValue::Number(refused))
            .unwrap_err()
        {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("hatchGradientKind"), "{message}");
        assert!(message.contains("Linear"), "{message}");
        assert!(message.contains("Spherical"), "{message}");
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchGradientKind
            .validate(ToolValue::Flag(true))
            .unwrap_err(),
        "hatchGradientKind",
    );
}

#[test]
fn hatch_gradient_angle_accepts_any_finite_degrees_value() {
    // Degrees on the channel (like `hatchAngle`), no implicit wrap; the command
    // converts exactly once with `.to_radians()`.
    for accepted in [0.0, 30.0, 90.0, -270.0, 1e6] {
        ToolParameter::HatchGradientAngle
            .validate(ToolValue::Number(accepted))
            .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
    }
    for refused in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_refusal_names_parameter(
            ToolParameter::HatchGradientAngle
                .validate(ToolValue::Number(refused))
                .unwrap_err(),
            "hatchGradientAngle",
        );
    }
    assert_refusal_names_parameter(
        ToolParameter::HatchGradientAngle
            .validate(ToolValue::Flag(false))
            .unwrap_err(),
        "hatchGradientAngle",
    );
}

#[test]
fn hatch_gradient_colours_are_integer_aci_indices_in_range() {
    for parameter in [
        ToolParameter::HatchGradientColorA,
        ToolParameter::HatchGradientColorB,
    ] {
        for accepted in [0.0, 1.0, 7.0, 255.0] {
            parameter
                .validate(ToolValue::Number(accepted))
                .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
        }
        // Out of the ACI domain, fractional (never truncated to an index) and
        // non-finite values are refused by name.
        for refused in [256.0, -1.0, 1.5, f64::NAN, f64::INFINITY] {
            assert_refusal_names_parameter(
                parameter.validate(ToolValue::Number(refused)).unwrap_err(),
                parameter.key(),
            );
        }
        assert_refusal_names_parameter(
            parameter.validate(ToolValue::Flag(true)).unwrap_err(),
            parameter.key(),
        );
    }
}
