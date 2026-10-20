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

/// HATCH pattern-type closed set (ADR 0018 decision D, P6b): the fill kind the
/// command synthesizes. `0` is a solid fill; `1` is a user-defined pattern
/// (angle/spacing/double). A predefined/custom **pattern name** is NOT
/// IMPLEMENTED — there is no pattern table and no `.pat` parser.
pub const HATCH_PATTERN_TYPE_SOLID: f64 = 0.0;
/// The user-defined pattern type code (see [`HATCH_PATTERN_TYPE_SOLID`]).
pub const HATCH_PATTERN_TYPE_USER_DEFINED: f64 = 1.0;

/// HATCH style closed set (DXF group 75, ADR 0018 A.3): `0` Normal (odd-parity
/// area), `1` Outer (outermost area only), `2` Ignore (through the entire area).
pub const HATCH_STYLE_NORMAL: f64 = 0.0;
/// The `Outer` style code (see [`HATCH_STYLE_NORMAL`]).
pub const HATCH_STYLE_OUTER: f64 = 1.0;
/// The `Ignore` style code (see [`HATCH_STYLE_NORMAL`]).
pub const HATCH_STYLE_IGNORE: f64 = 2.0;

/// The documented default TEXT height, in world units (ADR 0019 decision F.1).
///
/// The drawing's `Style` table carries no height, so there is no style value to
/// defer to; `2.5` is stated as our default (it also matches the DIMSTYLE text
/// height default) instead of inventing one per invocation. It is a **valid**
/// value for [`ToolParameter::TextHeight`], so the default can never limp into a
/// forced refusal at commit.
pub const TEXT_DEFAULT_HEIGHT: f64 = 2.5;

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
    /// HATCH pattern type: a **closed set** — `0` solid, `1` user-defined
    /// (ADR 0018 decision D, P6b). Predefined/custom pattern names are NOT
    /// IMPLEMENTED (there is no pattern table and no `.pat` parser).
    HatchPatternType,
    /// HATCH user-defined pattern spacing: the perpendicular distance between
    /// the synthesized pattern lines. Finite and **strictly positive** (a
    /// non-positive spacing has no line family; it is refused, not clamped).
    HatchSpacing,
    /// HATCH pattern angle in **degrees** (AutoCAD's prompt unit); the command
    /// converts it to the radians the domain stores. Any finite value is
    /// accepted — no implicit wrap (declared).
    HatchAngle,
    /// HATCH `HPDOUBLE` flag: draw the perpendicular second line family.
    HatchDouble,
    /// HATCH style: a **closed set** — `0` Normal, `1` Outer, `2` Ignore
    /// (ADR 0018 A.3).
    HatchStyle,
    /// TEXT glyph height in world units (ADR 0019 decision F.1): finite and
    /// **strictly positive**. There is no style-table height, so the session
    /// parameter is the single source and its default is the documented
    /// [`TEXT_DEFAULT_HEIGHT`] constant.
    TextHeight,
}

impl ToolParameter {
    /// Every member, in a fixed order (the key round-trip uses it).
    pub const ALL: [ToolParameter; 18] = [
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
        ToolParameter::HatchPatternType,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
        ToolParameter::TextHeight,
    ];

    /// Number of members (the store's fixed width).
    pub const COUNT: usize = 18;

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
            Self::HatchPatternType => 12,
            Self::HatchSpacing => 13,
            Self::HatchAngle => 14,
            Self::HatchDouble => 15,
            Self::HatchStyle => 16,
            Self::TextHeight => 17,
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
            Self::HatchPatternType => "hatchPatternType",
            Self::HatchSpacing => "hatchSpacing",
            Self::HatchAngle => "hatchAngle",
            Self::HatchDouble => "hatchDouble",
            Self::HatchStyle => "hatchStyle",
            Self::TextHeight => "textHeight",
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
            Self::AlignScale | Self::ArrayRotateItems | Self::HatchDouble => ToolValueKind::Flag,
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
    /// * `HatchPatternType` **0 (solid)** — with no pattern table the solid fill is
    ///   the only definition that needs no spacing/pattern name.
    /// * `HatchSpacing` **1** — AutoCAD's `HPSCALE` initial value (and a *valid*
    ///   spacing, so the default never limps into a forced refusal at commit).
    /// * `HatchAngle` **0** degrees.
    /// * `HatchDouble` **false** — `HPDOUBLE` initial 0.
    /// * `HatchStyle` **0 (Normal)** — DXF `75=0`, the odd-parity area.
    /// * `TextHeight` **2.5** — [`TEXT_DEFAULT_HEIGHT`], the documented project
    ///   constant (the drawing's `Style` table carries no height, so there is no
    ///   style value to defer to and the default is stated rather than invented
    ///   per invocation). 2.5 matches the dimension text height default.
    ///
    /// Note the fill-angle range (ADR 0016 decision A): the legal values are
    /// `0 < |angle| <= 360`, so the full-circle **default 360 is re-settable** (a
    /// user who steps away from it can return to it), while `0` and any value
    /// beyond a single turn (`±720`, …) are refused with no implicit wrap.
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
            Self::HatchPatternType => ToolValue::Number(HATCH_PATTERN_TYPE_SOLID),
            Self::HatchSpacing => ToolValue::Number(1.0),
            Self::HatchAngle => ToolValue::Number(0.0),
            Self::HatchDouble => ToolValue::Flag(false),
            Self::HatchStyle => ToolValue::Number(HATCH_STYLE_NORMAL),
            Self::TextHeight => ToolValue::Number(TEXT_DEFAULT_HEIGHT),
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
            (Self::HatchPatternType, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED],
                "0 (solid) or 1 (user-defined)",
            ),
            (Self::HatchSpacing, ToolValue::Number(number)) => {
                finite_greater_than(self, number, 0.0)
            }
            (Self::HatchAngle, ToolValue::Number(number)) => finite(self, number),
            (Self::HatchStyle, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER, HATCH_STYLE_IGNORE],
                "0 (Normal), 1 (Outer) or 2 (Ignore)",
            ),
            (Self::TextHeight, ToolValue::Number(number)) => finite_greater_than(self, number, 0.0),
            (Self::AlignScale, ToolValue::Flag(_))
            | (Self::ArrayRotateItems, ToolValue::Flag(_))
            | (Self::HatchDouble, ToolValue::Flag(_)) => Ok(()),
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

/// A finite value **strictly above** `low` (the HATCH spacing: a zero or negative
/// spacing has no line family and is refused, never clamped to a minimum).
fn finite_greater_than(parameter: ToolParameter, number: f64, low: f64) -> CadResult<()> {
    if number.is_finite() && number > low {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite and > {low}, got {number}",
            parameter.key()
        )))
    }
}

/// A value from a closed integer set, named in the refusal with its accepted
/// codes (`description` is the human-readable choice list).
fn closed_choice(
    parameter: ToolParameter,
    number: f64,
    choices: &[f64],
    description: &str,
) -> CadResult<()> {
    if number.is_finite() && number.fract() == 0.0 && choices.contains(&number) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be one of {description}, got {number}",
            parameter.key()
        )))
    }
}

/// A finite, non-zero angle no larger than one full turn (no implicit wrap).
///
/// The legal range is `0 < |angle| <= 360`: `360`/`-360` (a full turn) **are**
/// accepted — the documented default is the full-circle `360`, so it must be
/// re-settable — while `0`, `|angle| > 360` (`±720`, …) and non-finite values are
/// refused. The fill angle is always an explicitly-bounded sweep, never a
/// silently wrapped one.
fn fill_angle(parameter: ToolParameter, number: f64) -> CadResult<()> {
    if number.is_finite() && number != 0.0 && number.abs() <= 360.0 {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite, non-zero and within a single turn \
             (0 < |angle| <= 360), got {number}",
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
    fn array_fill_angle_accepts_within_a_turn_including_the_full_circle_and_refuses_zero_and_beyond(
    ) {
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
        // Pattern type: 0 solid, 1 user-defined. 2 (custom) and 3 are outside the
        // implemented set (no pattern table / .pat parser).
        for accepted in [HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED] {
            ToolParameter::HatchPatternType
                .validate(ToolValue::Number(accepted))
                .unwrap();
        }
        for refused in [2.0, 3.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
            let message = match ToolParameter::HatchPatternType
                .validate(ToolValue::Number(refused))
                .unwrap_err()
            {
                CadError::InvalidInput(message) => message,
                other => panic!("expected an InvalidInput refusal, got {other:?}"),
            };
            assert!(message.contains("hatchPatternType"), "{message}");
            assert!(message.contains("user-defined"), "{message}");
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
}
