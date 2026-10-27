//! The session-side resolution of the two text-creation parameters.
//!
//! `textRotation` and `mtextAttachment` are session-scoped typed parameters
//! ([`ToolParameter`]), so the creation sites resolve them through this one module
//! instead of inventing a second channel: TEXT creation reads the rotation
//! directly, and the interactive MTEXT capture fills its payload from both before
//! the command consumes it verbatim.
//!
//! Both conversions happen **exactly once** and neither clamps:
//!
//! * **degrees → radians** (`.to_radians()`), because the domain stores the text
//!   rotation in radians (the HATCH angle parameters follow the same rule);
//! * **attachment code → the domain alignment pair**, `h = (code - 1) % 3` and
//!   `row = (code - 1) / 3` over AutoCAD's nine attachment codes. `Baseline` is
//!   never produced: no attachment code names a baseline.
//!
//! A value that the typed channel would never store (a forced NaN rotation or a
//! code outside `1..=9`) is refused **by name** rather than clamped or folded onto
//! another value; the session is still the single authority that validates writes.

use super::*;
use crate::draw_tool::{
    ToolParameter, MTEXT_ATTACHMENT_MAX, MTEXT_ATTACHMENT_MIN, MTEXT_ATTACHMENT_TOP_LEFT,
    TEXT_DEFAULT_ROTATION_DEGREES,
};

/// The rotation of newly created TEXT/MTEXT, **in radians**, from the session's
/// [`ToolParameter::TextRotation`] (degrees, default `0`).
///
/// Refuses a non-finite value by name. The channel already refuses NaN/∞, so this
/// guard only covers a store value that did not come through it (a contract test's
/// forced value); it never substitutes a default and never wraps the angle.
pub(crate) fn text_rotation_radians(session: &SessionState) -> CadResult<f64> {
    let degrees = session
        .tool_parameter(ToolParameter::TextRotation)
        .number()
        .unwrap_or(TEXT_DEFAULT_ROTATION_DEGREES);
    if !degrees.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be finite, got {degrees}",
            ToolParameter::TextRotation.key()
        )));
    }
    // The one degree → radian conversion: the domain stores radians and the
    // channel carries the degrees AutoCAD prompts with.
    Ok(degrees.to_radians())
}

/// The attachment of newly created MTEXT, from the session's
/// [`ToolParameter::MTextAttachment`] (default `1`, top-left).
///
/// See [`mtext_align_for_code`] for the closed-set mapping.
pub(crate) fn mtext_attachment_align(
    session: &SessionState,
) -> CadResult<(TextAlignH, TextAlignV)> {
    let code = session
        .tool_parameter(ToolParameter::MTextAttachment)
        .number()
        .unwrap_or(MTEXT_ATTACHMENT_TOP_LEFT);
    mtext_align_for_code(code)
}

/// Map one AutoCAD MTEXT attachment code (DXF group `71`) onto the domain's
/// alignment pair.
///
/// The nine codes are a 3×3 grid: with `offset = code - 1`, the horizontal cell is
/// `offset % 3` (`Left`, `Center`, `Right`) and the row is `offset / 3` (`Top`,
/// `Middle`, `Bottom`). So `1` is top-left, `5` is centre-middle and `9` is
/// bottom-right. `TextAlignV::Baseline` is never produced.
///
/// A code outside the closed set is refused **by name** instead of being clamped
/// or folded onto another cell.
fn mtext_align_for_code(code: f64) -> CadResult<(TextAlignH, TextAlignV)> {
    if !code.is_finite()
        || code.fract() != 0.0
        || code < MTEXT_ATTACHMENT_MIN
        || code > MTEXT_ATTACHMENT_MAX
    {
        return Err(CadError::InvalidInput(format!(
            "tool parameter '{}' must be an integer between {} and {}, got {code}",
            ToolParameter::MTextAttachment.key(),
            MTEXT_ATTACHMENT_MIN as i64,
            MTEXT_ATTACHMENT_MAX as i64
        )));
    }
    let offset = code as i64 - 1;
    let h_align = match offset % 3 {
        0 => TextAlignH::Left,
        1 => TextAlignH::Center,
        _ => TextAlignH::Right,
    };
    let v_align = match offset / 3 {
        0 => TextAlignV::Top,
        1 => TextAlignV::Middle,
        _ => TextAlignV::Bottom,
    };
    Ok((h_align, v_align))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw_tool::ToolValue;
    use crate::{AppMode, DocumentId};

    #[test]
    fn the_nine_attachment_codes_are_the_autocad_three_by_three_grid() {
        use TextAlignH::{Center, Left, Right};
        use TextAlignV::{Bottom, Middle, Top};
        for (code, h_align, v_align) in [
            (1.0, Left, Top),
            (2.0, Center, Top),
            (3.0, Right, Top),
            (4.0, Left, Middle),
            (5.0, Center, Middle),
            (6.0, Right, Middle),
            (7.0, Left, Bottom),
            (8.0, Center, Bottom),
            (9.0, Right, Bottom),
        ] {
            assert_eq!(
                mtext_align_for_code(code).expect("a code in the closed set maps"),
                (h_align, v_align),
                "attachment code {code}"
            );
            assert_ne!(
                mtext_align_for_code(code).expect("mapped").1,
                TextAlignV::Baseline,
                "no attachment code names a baseline"
            );
        }
    }

    #[test]
    fn an_attachment_code_outside_the_closed_set_is_refused_by_name() {
        for refused in [0.0, 10.0, -1.0, 1.5, 4.5, f64::NAN, f64::INFINITY] {
            let error = mtext_align_for_code(refused).expect_err("out of the set is refused");
            match error {
                CadError::InvalidInput(message) => {
                    assert!(message.contains("mtextAttachment"), "{message}");
                    assert!(message.contains("between 1 and 9"), "{message}");
                }
                other => panic!("expected an InvalidInput refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_default_rotation_is_zero_radians_and_the_default_attachment_is_top_left() {
        let session = SessionState::new(DocumentId(1), AppMode::Work);
        assert_eq!(text_rotation_radians(&session).expect("finite"), 0.0);
        assert_eq!(
            mtext_attachment_align(&session).expect("valid code"),
            (TextAlignH::Left, TextAlignV::Top)
        );
    }

    #[test]
    fn a_forced_non_finite_rotation_is_refused_instead_of_stored() {
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        session.force_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(f64::NAN));
        let error = text_rotation_radians(&session).expect_err("NaN is refused");
        assert!(
            error.to_string().contains("textRotation"),
            "the refusal names the parameter: {error}"
        );
    }
}
