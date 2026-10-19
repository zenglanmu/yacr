//! Typed session tool parameters (ADR 0016 decision A).
//!
//! This is the general parameter channel that replaces ADR 0015 decision C's
//! narrow, single-purpose `SetPolygonSides` path: a [`ToolValue`]
//! ([`ToolValue::Number`] / [`ToolValue::Flag`]) addressed by a [`ToolParameter`]
//! member that carries a **stable machine key**.
//!
//! Each parameter owns its legal range and its default. Validation refuses
//! **explicitly** (never a silent clamp) with a message naming the parameter and
//! the accepted range, and a value whose variant does not match the parameter's
//! expected kind is refused too. Nothing here writes state: this module is a pure
//! value model; the session is the single authority that authorizes and stores.
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit; re-exported from `cad_app` under the same names.

use cad_domain::{CadError, CadResult};

/// The kind of [`ToolValue`] a [`ToolParameter`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolValueKind {
    Number,
    Flag,
}

impl ToolValueKind {
    /// Stable machine key (`"number"` / `"flag"`), used in refusal messages.
    pub fn key(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Flag => "flag",
        }
    }
}

/// A typed value carried to a [`ToolParameter`] (ADR 0016 decision A).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolValue {
    Number(f64),
    Flag(bool),
}

impl ToolValue {
    /// The variant kind of this value.
    pub fn kind(self) -> ToolValueKind {
        match self {
            Self::Number(_) => ToolValueKind::Number,
            Self::Flag(_) => ToolValueKind::Flag,
        }
    }

    /// The number, or `None` for a flag (the caller has already checked the kind).
    pub fn number(self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(value),
            Self::Flag(_) => None,
        }
    }

    /// The flag, or `None` for a number.
    pub fn flag(self) -> Option<bool> {
        match self {
            Self::Flag(value) => Some(value),
            Self::Number(_) => None,
        }
    }
}

/// A session-scoped tool parameter with a stable machine key (ADR 0016 decision A).
///
/// The member set and the machine keys are the frozen channel vocabulary: the
/// array parameters **must** land here (P4a) so P4c does not invent a second
/// channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolParameter {
    PolygonSides,
    FilletRadius,
    ChamferDistanceA,
    ChamferDistanceB,
    AlignScale,
    ArrayRows,
    ArrayColumns,
    ArrayRowSpacing,
    ArrayColumnSpacing,
    ArrayItems,
    ArrayFillAngle,
    ArrayRotateItems,
}

impl ToolParameter {
    /// Every member, in a fixed order (the key round-trip uses it).
    pub const ALL: [ToolParameter; 12] = [
        ToolParameter::PolygonSides,
        ToolParameter::FilletRadius,
        ToolParameter::ChamferDistanceA,
        ToolParameter::ChamferDistanceB,
        ToolParameter::AlignScale,
        ToolParameter::ArrayRows,
        ToolParameter::ArrayColumns,
        ToolParameter::ArrayRowSpacing,
        ToolParameter::ArrayColumnSpacing,
        ToolParameter::ArrayItems,
        ToolParameter::ArrayFillAngle,
        ToolParameter::ArrayRotateItems,
    ];

    /// Number of members (the store's fixed width).
    pub const COUNT: usize = 12;

    /// The dense index of this member in [`Self::ALL`] (the store's slot).
    pub fn index(self) -> usize {
        match self {
            Self::PolygonSides => 0,
            Self::FilletRadius => 1,
            Self::ChamferDistanceA => 2,
            Self::ChamferDistanceB => 3,
            Self::AlignScale => 4,
            Self::ArrayRows => 5,
            Self::ArrayColumns => 6,
            Self::ArrayRowSpacing => 7,
            Self::ArrayColumnSpacing => 8,
            Self::ArrayItems => 9,
            Self::ArrayFillAngle => 10,
            Self::ArrayRotateItems => 11,
        }
    }

    /// The **stable machine key** of this parameter.
    ///
    /// This is the exact string the `cad_set_tool_parameter` wire request carries
    /// in its `parameter` field (camelCase, matching the frozen frontend wire
    /// style such as `openEnded`). It is never translated.
    pub fn key(self) -> &'static str {
        match self {
            Self::PolygonSides => "polygonSides",
            Self::FilletRadius => "filletRadius",
            Self::ChamferDistanceA => "chamferDistanceA",
            Self::ChamferDistanceB => "chamferDistanceB",
            Self::AlignScale => "alignScale",
            Self::ArrayRows => "arrayRows",
            Self::ArrayColumns => "arrayColumns",
            Self::ArrayRowSpacing => "arrayRowSpacing",
            Self::ArrayColumnSpacing => "arrayColumnSpacing",
            Self::ArrayItems => "arrayItems",
            Self::ArrayFillAngle => "arrayFillAngle",
            Self::ArrayRotateItems => "arrayRotateItems",
        }
    }

    /// Resolve a stable machine key back to its member, or `None` if unknown.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|parameter| parameter.key() == key)
    }

    /// The kind of value this parameter accepts.
    pub fn kind(self) -> ToolValueKind {
        match self {
            Self::AlignScale | Self::ArrayRotateItems => ToolValueKind::Flag,
            _ => ToolValueKind::Number,
        }
    }

    /// The documented session default of this parameter.
    ///
    /// * `PolygonSides` **4** — AutoCAD's `POLYSIDES` initial value.
    /// * `FilletRadius` **0** — AutoCAD's `FILLETRAD` initial value (`R = 0`
    ///   means "trim/extend to the corner only").
    /// * `ChamferDistanceA` / `ChamferDistanceB` **0 / 0** — AutoCAD's
    ///   `CHAMFERA`/`CHAMFERB` initial values.
    /// * `AlignScale` **false** — AutoCAD prompts "scale? " with the default No.
    /// * `ArrayRows` / `ArrayColumns` / `ArrayItems` **1 / 1 / 1**.
    /// * `ArrayRowSpacing` / `ArrayColumnSpacing` **0**.
    /// * `ArrayFillAngle` **360** — AutoCAD's full-circle ARRAYPOLAR default.
    /// * `ArrayRotateItems` **false**.
    ///
    /// Note the deliberate asymmetry the ADR specifies for `ArrayFillAngle`: the
    /// default is the full-circle **360**, while the setter **refuses** `0` and
    /// `±360` (no implicit wrap). The default is therefore the one value the
    /// channel cannot re-set — it is the honest "unset/full turn" starting point,
    /// and any explicit request must be a partial, non-wrapping angle.
    pub fn default_value(self) -> ToolValue {
        match self {
            Self::PolygonSides => ToolValue::Number(4.0),
            Self::FilletRadius => ToolValue::Number(0.0),
            Self::ChamferDistanceA => ToolValue::Number(0.0),
            Self::ChamferDistanceB => ToolValue::Number(0.0),
            Self::AlignScale => ToolValue::Flag(false),
            Self::ArrayRows => ToolValue::Number(1.0),
            Self::ArrayColumns => ToolValue::Number(1.0),
            Self::ArrayRowSpacing => ToolValue::Number(0.0),
            Self::ArrayColumnSpacing => ToolValue::Number(0.0),
            Self::ArrayItems => ToolValue::Number(1.0),
            Self::ArrayFillAngle => ToolValue::Number(360.0),
            Self::ArrayRotateItems => ToolValue::Flag(false),
        }
    }

    /// Validate `value` against this parameter's kind and legal range.
    ///
    /// Refuses **explicitly** (never a silent clamp) with a message naming the
    /// parameter and the accepted range, and refuses a `ToolValue` whose variant
    /// does not match the expected kind (for example a `Flag` for
    /// `PolygonSides`).
    pub fn validate(self, value: ToolValue) -> CadResult<()> {
        match (self, value) {
            (Self::PolygonSides, ToolValue::Number(number)) => {
                integer_in_range(self, number, 3, 1024)
            }
            (Self::FilletRadius, ToolValue::Number(number))
            | (Self::ChamferDistanceA, ToolValue::Number(number))
            | (Self::ChamferDistanceB, ToolValue::Number(number)) => {
                finite_at_least(self, number, 0.0)
            }
            (Self::ArrayRows, ToolValue::Number(number))
            | (Self::ArrayColumns, ToolValue::Number(number))
            | (Self::ArrayItems, ToolValue::Number(number)) => integer_at_least(self, number, 1),
            (Self::ArrayRowSpacing, ToolValue::Number(number))
            | (Self::ArrayColumnSpacing, ToolValue::Number(number)) => finite(self, number),
            (Self::ArrayFillAngle, ToolValue::Number(number)) => fill_angle(self, number),
            (Self::AlignScale, ToolValue::Flag(_))
            | (Self::ArrayRotateItems, ToolValue::Flag(_)) => Ok(()),
            // Any other pairing is a kind mismatch: a `Flag` for a numeric
            // parameter, or a `Number` for a flag parameter.
            _ => Err(CadError::InvalidInput(format!(
                "tool parameter '{}' takes a {}, got a {}",
                self.key(),
                self.kind().key(),
                value.kind().key()
            ))),
        }
    }
}

/// An integral value within an inclusive `[low, high]` bound.
fn integer_in_range(parameter: ToolParameter, number: f64, low: i64, high: i64) -> CadResult<()> {
    if number.is_finite() && number.fract() == 0.0 && number >= low as f64 && number <= high as f64
    {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be an integer between {low} and {high}, got {number}",
            parameter.key()
        )))
    }
}

/// An integral value with a lower bound and no upper bound.
fn integer_at_least(parameter: ToolParameter, number: f64, low: i64) -> CadResult<()> {
    if number.is_finite() && number.fract() == 0.0 && number >= low as f64 {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be an integer >= {low}, got {number}",
            parameter.key()
        )))
    }
}

/// A finite value with a lower bound.
fn finite_at_least(parameter: ToolParameter, number: f64, low: f64) -> CadResult<()> {
    if number.is_finite() && number >= low {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite and >= {low}, got {number}",
            parameter.key()
        )))
    }
}

/// Any finite value.
fn finite(parameter: ToolParameter, number: f64) -> CadResult<()> {
    if number.is_finite() {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite, got {number}",
            parameter.key()
        )))
    }
}

/// A finite, non-zero angle that is **not** a full turn (no implicit wrap).
///
/// `0` and every exact multiple of a full turn (`±360`, `±720`, …) are refused,
/// so the caller can never ask for a wrap-around fill; the fill angle is always a
/// partial, explicitly-bounded sweep.
fn fill_angle(parameter: ToolParameter, number: f64) -> CadResult<()> {
    if number.is_finite() && number != 0.0 && (number % 360.0) != 0.0 {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite and non-zero and must not be a full turn \
             (0, ±360, …), got {number}",
            parameter.key()
        )))
    }
}

/// The fixed-width session store of every [`ToolParameter`].
///
/// Initialised to each parameter's documented default ([`ToolParameter::default_value`]);
/// the session owns one of these and is the single authority that validates
/// before writing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolParameterStore {
    values: [ToolValue; ToolParameter::COUNT],
}

impl ToolParameterStore {
    /// A store at every parameter's documented default.
    pub fn defaults() -> Self {
        let mut values = [ToolValue::Flag(false); ToolParameter::COUNT];
        for parameter in ToolParameter::ALL {
            values[parameter.index()] = parameter.default_value();
        }
        Self { values }
    }

    /// The current value of `parameter`.
    pub fn get(&self, parameter: ToolParameter) -> ToolValue {
        self.values[parameter.index()]
    }

    /// Write `parameter`'s value (no validation — the caller validates first).
    pub fn set(&mut self, parameter: ToolParameter, value: ToolValue) {
        self.values[parameter.index()] = value;
    }
}

impl Default for ToolParameterStore {
    fn default() -> Self {
        Self::defaults()
    }
}

#[cfg(test)]
mod tests {
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
    fn array_fill_angle_accepts_partial_turns_and_refuses_zero_and_full_turns() {
        for accepted in [90.0, 180.0, 359.0, -90.0, 0.5] {
            ToolParameter::ArrayFillAngle
                .validate(ToolValue::Number(accepted))
                .unwrap_or_else(|error| panic!("{accepted} refused: {error}"));
        }
        // 0 and every full-turn multiple are refused (no implicit wrap).
        for refused in [0.0, 360.0, -360.0, 720.0, f64::NAN, f64::INFINITY] {
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
}
