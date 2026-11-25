//! `ToolParameter::validate`: the pure range/kind checks.
//!
//! Split out of `parameter.rs` to keep that module inside the code-size gate;
//! the method is an inherent impl, so every caller is unchanged.
use super::*;

impl ToolParameter {
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
            // DIVIDE: an integer in `2..=1024`. `1` would place no mark and
            // `1025` is a runaway count; both are refused by name, never clamped.
            (Self::DivideSegments, ToolValue::Number(number)) => {
                integer_in_range(self, number, 2, 1024)
            }
            // LENGTHEN mode: the four documented codes, read by name in the
            // refusal so a raw `7` cannot be mistaken for an amount.
            (Self::LengthenMode, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[
                    LENGTHEN_MODE_DELTA,
                    LENGTHEN_MODE_PERCENT,
                    LENGTHEN_MODE_TOTAL,
                    LENGTHEN_MODE_DYNAMIC,
                ],
                "0 (Delta), 1 (Percent), 2 (Total) or 3 (Dynamic)",
            ),
            // LENGTHEN amount: finite and **non-zero**. A zero change is refused
            // rather than clamped into a no-op; Delta may be negative (shorten),
            // while the command additionally requires `> 0` for Percent/Total.
            (Self::LengthenValue, ToolValue::Number(number)) => finite_non_zero(self, number),
            // SCALETEXT factor: finite and **strictly positive**. `1.0` is a valid
            // value at the channel (the command refuses it as `scaletext.unchanged`
            // without a transaction); `0`, a negative value and NaN are refused
            // here, never clamped.
            (Self::ScaleTextFactor, ToolValue::Number(number)) => {
                finite_greater_than(self, number, 0.0)
            }
            // PLINE width: finite and non-negative. Zero is the hairline default
            // and a positive value is a real stroke; a negative or non-finite
            // width is refused here, never clamped.
            (Self::PolylineWidth, ToolValue::Number(number)) => finite_at_least(self, number, 0.0),
            // LINE's typed length is a real distance: `0` is the unset sentinel
            // and a negative value is refused by name.
            (Self::LineLength, ToolValue::Number(number)) => finite_at_least(self, number, 0.0),
            // LINE's typed direction is any finite degrees value (no implicit
            // wrap); `0` is stored as the unset sentinel.
            (Self::LineAngle, ToolValue::Number(number)) => finite(self, number),
            // RECTANG's typed extents are real distances: `0` is the unset
            // sentinel and a negative value is refused by name.
            (Self::RectWidth, ToolValue::Number(number))
            | (Self::RectHeight, ToolValue::Number(number))
            | (Self::CircleRadius, ToolValue::Number(number)) => finite_at_least(self, number, 0.0),
            // ROTATE's absolute angle: any finite degrees value, no implicit
            // wrap; `0` is the unset sentinel.
            (Self::RotateAngle, ToolValue::Number(number)) => finite(self, number),
            // MOVE/OFFSET distances and SCALE's factor: `0` is the unset
            // sentinel, a negative value is refused by name.
            (Self::MoveDistance, ToolValue::Number(number))
            | (Self::ScaleFactor, ToolValue::Number(number))
            | (Self::OffsetDistance, ToolValue::Number(number)) => {
                finite_at_least(self, number, 0.0)
            }
            // PRESSPULL's signed distance: either sign is meaningful, `0` is the
            // unset sentinel (refused at execute, not here, so the control can be
            // cleared), and the seam's magnitude bound is the channel's bound.
            (Self::PressPullDistance, ToolValue::Number(number))
                if number.is_finite()
                    && number.abs() <= cad_kernel_adapter::MAX_OFFSET_FACE_DISTANCE =>
            {
                Ok(())
            }
            (Self::PressPullDistance, ToolValue::Number(number)) => {
                Err(CadError::InvalidInput(format!(
                    "tool parameter '{}' must be finite and at most {} in magnitude, got {number}",
                    self.key(),
                    cad_kernel_adapter::MAX_OFFSET_FACE_DISTANCE
                )))
            }
            // EXTRUDE's signed distance: same shape, with the seam's extrusion bound.
            (Self::ExtrudeDistance, ToolValue::Number(number))
                if number.is_finite()
                    && number.abs() <= cad_kernel_adapter::MAX_EXTRUDE_DISTANCE =>
            {
                Ok(())
            }
            (Self::ExtrudeDistance, ToolValue::Number(number)) => {
                Err(CadError::InvalidInput(format!(
                    "tool parameter '{}' must be finite and at most {} in magnitude, got {number}",
                    self.key(),
                    cad_kernel_adapter::MAX_EXTRUDE_DISTANCE
                )))
            }
            // STRETCH window mode: the three documented shapes, read by name in
            // the refusal. `3`, a fraction and a negative code are refused, never
            // clamped onto a neighbouring shape.
            (Self::StretchWindow, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[
                    STRETCH_WINDOW_RECT,
                    STRETCH_WINDOW_POLYGON,
                    STRETCH_WINDOW_LASSO,
                ],
                "0 (rectangle), 1 (polygon) or 2 (lasso)",
            ),
            // DIMROTATED's angle: any **finite** degrees value, no implicit wrap
            // and no normalisation (a negative angle is a legal rotation the other
            // way). NaN/±∞ are refused by name.
            (Self::DimRotation, ToolValue::Number(number)) => finite(self, number),
            // ALIGN pair count: the three documented shapes, read by name in the
            // refusal. `0`, `4` and a fraction are refused, never clamped.
            (Self::AlignPairs, ToolValue::Number(number)) => closed_choice(
                self,
                number,
                &[1.0, 2.0, 3.0],
                "1 (one pair), 2 (two pairs) or 3 (three pairs)",
            ),
            // BREAK `F` / DIVIDE block: booleans only.
            (Self::BreakFirstPoint, ToolValue::Flag(_))
            | (Self::DivideUseBlock, ToolValue::Flag(_))
            | (Self::DivideAlignBlock, ToolValue::Flag(_)) => Ok(()),
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
