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
/// (angle/spacing/double); `2` is a **predefined** pattern from the small
/// hand-transcribed table in `cad_domain::hatch::patterns` (`ANSI31`, `ANSI32`,
/// `ANSI37`), selected by [`ToolParameter::HatchPattern`]; `3` is a two-colour
/// **gradient** ([`ToolParameter::HatchGradientKind`] / `HatchGradientAngle` /
/// `HatchGradientColorA` / `HatchGradientColorB`). A custom (`76=2`) pattern
/// name / `.pat` file is NOT IMPLEMENTED.
pub const HATCH_PATTERN_TYPE_SOLID: f64 = 0.0;
/// The user-defined pattern type code (see [`HATCH_PATTERN_TYPE_SOLID`]).
pub const HATCH_PATTERN_TYPE_USER_DEFINED: f64 = 1.0;
/// The predefined pattern type code (see [`HATCH_PATTERN_TYPE_SOLID`]).
pub const HATCH_PATTERN_TYPE_PREDEFINED: f64 = 2.0;
/// The gradient fill type code (see [`HATCH_PATTERN_TYPE_SOLID`]).
///
/// The created entity still records `HatchPatternType::Predefined` (`76=1`):
/// a gradient HATCH is written as a solid fill plus the 450-470 gradient block,
/// so no definition rows exist. That mapping is an **unverified vendor
/// convention** (declared), not a round-tripped file claim.
pub const HATCH_PATTERN_TYPE_GRADIENT: f64 = 3.0;

/// HATCH gradient-kind closed set (`LINEAR`): a 1D ramp along the gradient
/// angle.
pub const HATCH_GRADIENT_LINEAR: f64 = 0.0;
/// The radial gradient kind code (`SPHERICAL`): the value is the normalised
/// distance from the centre of the boundary extent.
///
/// Never emit the DXF spelling `CYLINDER`: the domain silently bakes it as
/// spherical, so offering it would be a second name for one behaviour.
pub const HATCH_GRADIENT_SPHERICAL: f64 = 1.0;

/// The default ACI index of the gradient's start stop: **1 (red)**.
///
/// ACI `0` is the ByBlock placeholder (it has no sRGB triple, so it cannot be a
/// gradient stop) and ACI `7` is white, which merges with a light background;
/// `1..=9` are the standard saturated colours, so `1` (red) is a real colour and
/// the canonical first stop.
pub const HATCH_GRADIENT_DEFAULT_ACI_A: f64 = 1.0;
/// The default ACI index of the gradient's end stop: **5 (blue)**, the red/blue
/// pair being visually distinct on any background.
pub const HATCH_GRADIENT_DEFAULT_ACI_B: f64 = 5.0;

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

/// The documented default TEXT/MTEXT rotation, in **degrees**: `0` (unrotated).
///
/// The channel carries degrees — the unit AutoCAD prompts with, like
/// [`ToolParameter::HatchAngle`] and [`ToolParameter::HatchGradientAngle`] — and
/// the creation site converts exactly once with `.to_radians()`, because the
/// domain stores the rotation **in radians**. `0` is a valid value for
/// [`ToolParameter::TextRotation`], so the default can never limp into a forced
/// refusal at commit.
pub const TEXT_DEFAULT_ROTATION_DEGREES: f64 = 0.0;

/// The default MTEXT attachment code: **`1`, top-left**.
///
/// AutoCAD addresses an MTEXT's attachment with DXF group `71`, a 3×3 grid:
/// `1..=3` are the top row (Left/Center/Right), `4..=6` the middle row and
/// `7..=9` the bottom row. `1` is AutoCAD's `MTEXT` default and exactly what
/// MTEXT creation stored before the parameter existed, so the default keeps
/// today's output. It is a valid code, so the default can never limp into a
/// forced refusal at commit.
pub const MTEXT_ATTACHMENT_TOP_LEFT: f64 = 1.0;
/// The lowest MTEXT attachment code (see [`MTEXT_ATTACHMENT_TOP_LEFT`]).
pub const MTEXT_ATTACHMENT_MIN: f64 = 1.0;
/// The highest MTEXT attachment code: `9`, bottom-right.
pub const MTEXT_ATTACHMENT_MAX: f64 = 9.0;

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
    /// HATCH pattern type: a **closed set** — `0` solid, `1` user-defined,
    /// `2` predefined (ADR 0018 decision D, P6b). A predefined pattern is
    /// chosen by [`Self::HatchPattern`]; a custom (`76=2`) pattern name or a
    /// `.pat` file is NOT IMPLEMENTED.
    HatchPatternType,
    /// HATCH predefined pattern index: a **closed set** of the **1-based**
    /// indices into `cad_domain::hatch::predefined_patterns()` (`1` ANSI31,
    /// `2` ANSI32, `3` ANSI37). The set is the table length, so a validated
    /// index can never name a pattern this build does not implement.
    HatchPattern,
    /// HATCH pattern spacing: for a user-defined pattern the perpendicular
    /// distance between the synthesized pattern lines, and for a predefined
    /// pattern the pattern scale (`HPSCALE`). Finite and **strictly positive** (a
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
    /// HATCH gradient kind for a [`HATCH_PATTERN_TYPE_GRADIENT`] fill: a
    /// **closed set** — `0` Linear (`"LINEAR"`), `1` Spherical
    /// (`"SPHERICAL"`). The domain's `"CYLINDER"` spelling is deliberately not
    /// offered: it bakes as spherical.
    HatchGradientKind,
    /// HATCH gradient angle in **degrees** (the prompt unit, like
    /// [`Self::HatchAngle`]): any finite value, no implicit wrap. The command
    /// converts it exactly once with `.to_radians()` when it builds the entity,
    /// because the domain stores the gradient angle **in radians** verbatim (the
    /// import path never converts either — pinned by a round-trip test).
    HatchGradientAngle,
    /// HATCH gradient start-stop colour as an **integer ACI index** in
    /// `0..=255` (the `ToolValue` channel carries no colour type), resolved
    /// through the ACI palette into the stop's sRGB triple. ACI `0` is accepted
    /// by the channel but refused by the command: it is the ByBlock
    /// placeholder, not a colour.
    HatchGradientColorA,
    /// HATCH gradient end-stop colour (see [`Self::HatchGradientColorA`]).
    HatchGradientColorB,
    /// TEXT/MTEXT rotation in **degrees** (the unit AutoCAD prompts with, exactly
    /// like [`Self::HatchAngle`]): any **finite** value, with **no implicit
    /// wrap**, so `-30` stays `-30` and a full turn is not silently folded into
    /// `0`. The creation sites convert it exactly once with `.to_radians()`,
    /// because the domain stores the rotation in radians. TEXT and MTEXT are the
    /// only consumers — `TEXTEDIT` never re-rotates and MLEADER keeps its own
    /// orientation rule.
    TextRotation,
    /// MTEXT attachment: a **closed set** of AutoCAD's nine attachment codes
    /// (`1..=9`, DXF group `71`). `1..=3` are the top row (Left/Center/Right),
    /// `4..=6` the middle row (Left/Center/Right) and `7..=9` the bottom row
    /// (Left/Center/Right): the creation site maps `h = (code - 1) % 3` and
    /// `row = (code - 1) / 3` onto the domain's `TextAlignH`/`TextAlignV` pair,
    /// so `TextAlignV::Baseline` is never produced. MTEXT is the only consumer.
    MTextAttachment,
}

impl ToolParameter {
    /// Every member, in a fixed order (the key round-trip uses it).
    pub const ALL: [ToolParameter; 25] = [
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
        ToolParameter::HatchPattern,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
        ToolParameter::TextHeight,
        ToolParameter::HatchGradientKind,
        ToolParameter::HatchGradientAngle,
        ToolParameter::HatchGradientColorA,
        ToolParameter::HatchGradientColorB,
        ToolParameter::TextRotation,
        ToolParameter::MTextAttachment,
    ];

    /// Number of members (the store's fixed width).
    pub const COUNT: usize = 25;

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
            Self::HatchPattern => 13,
            Self::HatchSpacing => 14,
            Self::HatchAngle => 15,
            Self::HatchDouble => 16,
            Self::HatchStyle => 17,
            Self::TextHeight => 18,
            Self::HatchGradientKind => 19,
            Self::HatchGradientAngle => 20,
            Self::HatchGradientColorA => 21,
            Self::HatchGradientColorB => 22,
            Self::TextRotation => 23,
            Self::MTextAttachment => 24,
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
            Self::HatchPattern => "hatchPattern",
            Self::HatchSpacing => "hatchSpacing",
            Self::HatchAngle => "hatchAngle",
            Self::HatchDouble => "hatchDouble",
            Self::HatchStyle => "hatchStyle",
            Self::TextHeight => "textHeight",
            Self::HatchGradientKind => "hatchGradientKind",
            Self::HatchGradientAngle => "hatchGradientAngle",
            Self::HatchGradientColorA => "hatchGradientColorA",
            Self::HatchGradientColorB => "hatchGradientColorB",
            Self::TextRotation => "textRotation",
            Self::MTextAttachment => "mtextAttachment",
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
    /// * `HatchPatternType` **0 (solid)** — the solid fill needs no spacing and
    ///   no pattern name, so it stays the initial fill kind.
    /// * `HatchPattern` **1 (ANSI31)** — the first entry of the implemented
    ///   predefined table; a valid index, so the default never limps into a
    ///   forced refusal at commit.
    /// * `HatchSpacing` **1** — AutoCAD's `HPSCALE` initial value (and a *valid*
    ///   spacing, so the default never limps into a forced refusal at commit).
    /// * `HatchAngle` **0** degrees.
    /// * `HatchDouble` **false** — `HPDOUBLE` initial 0.
    /// * `HatchStyle` **0 (Normal)** — DXF `75=0`, the odd-parity area.
    /// * `TextHeight` **2.5** — [`TEXT_DEFAULT_HEIGHT`], the documented project
    ///   constant (the drawing's `Style` table carries no height, so there is no
    ///   style value to defer to and the default is stated rather than invented
    ///   per invocation). 2.5 matches the dimension text height default.
    /// * `HatchGradientKind` **0 (Linear)** — the straightforward ramp; the
    ///   radial kind is the deliberate alternative.
    /// * `HatchGradientAngle` **0** degrees — the ramp runs along the hatch
    ///   plane's +X axis (the same 0 as the pattern angle).
    /// * `HatchGradientColorA` / `HatchGradientColorB` **1 (red) / 5 (blue)** —
    ///   both are real palette colours ([`HATCH_GRADIENT_DEFAULT_ACI_A`] /
    ///   [`HATCH_GRADIENT_DEFAULT_ACI_B`]) and are visually distinct, which ACI
    ///   `0` (ByBlock, no colour at all) could never be.
    /// * `TextRotation` **0** degrees — [`TEXT_DEFAULT_ROTATION_DEGREES`], the
    ///   unrotated TEXT/MTEXT creation of today (and a *valid* value, so the
    ///   default can never limp into a forced refusal at commit).
    /// * `MTextAttachment` **1 (top-left)** — [`MTEXT_ATTACHMENT_TOP_LEFT`],
    ///   AutoCAD's MTEXT default attachment and the exact pair MTEXT creation
    ///   stored before the parameter existed (Left/Top).
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
            Self::HatchPattern => ToolValue::Number(1.0),
            Self::HatchSpacing => ToolValue::Number(1.0),
            Self::HatchAngle => ToolValue::Number(0.0),
            Self::HatchDouble => ToolValue::Flag(false),
            Self::HatchStyle => ToolValue::Number(HATCH_STYLE_NORMAL),
            Self::TextHeight => ToolValue::Number(TEXT_DEFAULT_HEIGHT),
            Self::HatchGradientKind => ToolValue::Number(HATCH_GRADIENT_LINEAR),
            Self::HatchGradientAngle => ToolValue::Number(0.0),
            Self::HatchGradientColorA => ToolValue::Number(HATCH_GRADIENT_DEFAULT_ACI_A),
            Self::HatchGradientColorB => ToolValue::Number(HATCH_GRADIENT_DEFAULT_ACI_B),
            Self::TextRotation => ToolValue::Number(TEXT_DEFAULT_ROTATION_DEGREES),
            Self::MTextAttachment => ToolValue::Number(MTEXT_ATTACHMENT_TOP_LEFT),
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
                &[
                    HATCH_PATTERN_TYPE_SOLID,
                    HATCH_PATTERN_TYPE_USER_DEFINED,
                    HATCH_PATTERN_TYPE_PREDEFINED,
                    HATCH_PATTERN_TYPE_GRADIENT,
                ],
                "0 (solid), 1 (user-defined), 2 (predefined) or 3 (gradient)",
            ),
            (Self::HatchPattern, ToolValue::Number(number)) => {
                predefined_pattern_choice(self, number)
            }
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
            (Self::HatchGradientKind, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[HATCH_GRADIENT_LINEAR, HATCH_GRADIENT_SPHERICAL],
                "0 (Linear) or 1 (Spherical)",
            ),
            (Self::HatchGradientAngle, ToolValue::Number(number)) => finite(self, number),
            // The ACI index is an integer in `0..=255`. Index `0` is the ByBlock
            // placeholder (no colour), so the command refuses it when it builds
            // the stop; the channel accepts the documented ACI domain.
            (Self::HatchGradientColorA, ToolValue::Number(number))
            | (Self::HatchGradientColorB, ToolValue::Number(number)) => {
                integer_in_range(self, number, 0, 255)
            }
            // Any finite degrees value: no implicit wrap, so a negative angle is
            // stored as the negative radian value. NaN/∞ are refused by name.
            (Self::TextRotation, ToolValue::Number(number)) => finite(self, number),
            // AutoCAD's nine attachment codes: an integer in `1..=9`. `0`, `10`,
            // a fraction and a non-finite value are refused by name — never
            // rounded and never folded onto another cell.
            (Self::MTextAttachment, ToolValue::Number(number)) => integer_in_range(
                self,
                number,
                MTEXT_ATTACHMENT_MIN as i64,
                MTEXT_ATTACHMENT_MAX as i64,
            ),
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

/// An integer index into the **implemented** predefined pattern table.
///
/// The closed set is `1..=predefined_patterns().len()`, derived from the table
/// itself, so an accepted index always names a pattern that exists. A
/// non-integer, `0`, a negative value and anything past the last entry are all
/// refused by name — never clamped, rounded or mapped onto another pattern.
fn predefined_pattern_choice(parameter: ToolParameter, number: f64) -> CadResult<()> {
    let patterns = cad_domain::hatch::predefined_patterns();
    let high = patterns.len();
    if number.is_finite() && number.fract() == 0.0 && number >= 1.0 && number <= high as f64 {
        return Ok(());
    }
    let names = patterns
        .iter()
        .map(|pattern| pattern.name)
        .collect::<Vec<_>>()
        .join(", ");
    Err(CadError::InvalidInput(format!(
        "tool parameter '{}' must be an integer between 1 and {high} (the implemented predefined \
         patterns: {names}), got {number}",
        parameter.key()
    )))
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
mod tests;
