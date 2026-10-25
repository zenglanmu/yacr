//! The HATCH typed channel, fill synthesis and the summary diagnostic.
//!
//! Split out of `hatch.rs` to keep every file under the 1000-line gate; the
//! command itself (`create_hatch`) lives in the parent module and owns the
//! boundary work. Nothing here touches geometry: the typed parameters are read
//! from the session ([`SessionState::tool_parameter`]), validated, and turned into
//! the domain [`HatchFill`] / [`HatchPatternType`] pair, and the diagnostic echoes
//! what was actually written.

use super::*;

use super::hatch_stitch::STITCH_TOLERANCE;

use cad_domain::hatch::{predefined_pattern_by_index, predefined_patterns, PredefinedPattern};

use crate::draw_tool::{
    ToolParameter, HATCH_PATTERN_TYPE_PREDEFINED, HATCH_PATTERN_TYPE_SOLID,
    HATCH_PATTERN_TYPE_USER_DEFINED, HATCH_STYLE_IGNORE, HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER,
};

/// The pattern name written for a created user-defined pattern.
///
/// There is no pattern table and no `.pat` parser, so this is an opaque,
/// locale-neutral identity (ADR U-11: what AutoCAD prefers when a `76=0` hatch
/// carries both a name and definition rows is unverified). The definition rows
/// are the authoritative data.
const USER_DEFINED_PATTERN_NAME: &str = "USER";

/// The HATCH parameters read from the session typed channel.
pub(super) struct HatchParameters {
    pub(super) pattern_type: f64,
    /// The **1-based** index into the implemented predefined pattern table
    /// (`hatchPattern`); only read for `HATCH_PATTERN_TYPE_PREDEFINED`.
    pattern_index: f64,
    spacing: f64,
    /// Degrees on the channel (the AutoCAD prompt unit); the domain stores
    /// radians.
    angle_degrees: f64,
    double: bool,
    pub(super) style: f64,
}

/// Read and defensively re-validate the HATCH typed parameters.
///
/// The predefined pattern index is deliberately **not** re-validated here: it is
/// resolved in [`HatchParameters::fill`], which refuses an index outside the
/// implemented table with a message that names the index and the table (the
/// generic range refusal would hide which pattern set the command actually
/// implements).
pub(super) fn hatch_parameters(session: &SessionState) -> CadResult<HatchParameters> {
    let parameters = [
        ToolParameter::HatchPatternType,
        ToolParameter::HatchSpacing,
        ToolParameter::HatchAngle,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
    ];
    for parameter in parameters {
        parameter
            .validate(session.tool_parameter(parameter))
            .map_err(|error| match error {
                CadError::InvalidInput(message) => {
                    CadError::InvalidInput(format!("HATCH: {message}"))
                }
                other => other,
            })?;
    }
    let number = |parameter: ToolParameter| {
        session
            .tool_parameter(parameter)
            .number()
            .unwrap_or(f64::NAN)
    };
    Ok(HatchParameters {
        pattern_type: number(ToolParameter::HatchPatternType),
        pattern_index: number(ToolParameter::HatchPattern),
        spacing: number(ToolParameter::HatchSpacing),
        angle_degrees: number(ToolParameter::HatchAngle),
        double: session
            .tool_parameter(ToolParameter::HatchDouble)
            .flag()
            .unwrap_or(false),
        style: number(ToolParameter::HatchStyle),
    })
}

impl HatchParameters {
    /// The predefined pattern the `hatchPattern` index names, or `None` when the
    /// index is not an implemented table entry.
    ///
    /// The channel already bounds the index to `1..=table.len()`, but the command
    /// re-resolves it (defence in depth) and refuses by name in [`Self::fill`];
    /// nothing is clamped onto a neighbouring pattern.
    fn predefined(&self) -> Option<&'static PredefinedPattern> {
        let index = self.pattern_index;
        if !index.is_finite() || index.fract() != 0.0 || index < 1.0 {
            return None;
        }
        predefined_pattern_by_index(index as usize)
    }

    /// The pattern scale (`HPSCALE`) and angle, validated once for the two
    /// pattern kinds that use them (user-defined and predefined).
    fn scale_and_angle(&self) -> CadResult<()> {
        if !self.spacing.is_finite() || self.spacing <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "HATCH spacing must be finite and > 0, got {}",
                self.spacing
            )));
        }
        if !self.angle_degrees.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "HATCH pattern angle must be finite, got {}",
                self.angle_degrees
            )));
        }
        Ok(())
    }

    /// The fill definition and the DXF pattern type it must record.
    ///
    /// A solid fill records `Predefined` (`76=1`), matching the committed SOLID
    /// fixtures (a solid pattern has no user-defined definition rows). A
    /// user-defined fill records `UserDefined` (`76=0`) and carries the
    /// synthesized definition rows. A predefined fill (pattern type code `2`)
    /// records `Predefined` (`76=1`) and carries the rows of the hand-transcribed
    /// table entry the index names.
    pub(super) fn fill(&self) -> CadResult<(HatchFill, HatchPatternType)> {
        if self.pattern_type == HATCH_PATTERN_TYPE_SOLID {
            return Ok((HatchFill::Solid, HatchPatternType::Predefined));
        }
        if self.pattern_type == HATCH_PATTERN_TYPE_PREDEFINED {
            self.scale_and_angle()?;
            let Some(predefined) = self.predefined() else {
                let names = predefined_patterns()
                    .iter()
                    .map(|pattern| pattern.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CadError::InvalidInput(format!(
                    "HATCH pattern index {} is not in the implemented predefined set (1..={}: \
                     {names})",
                    self.pattern_index,
                    predefined_patterns().len()
                )));
            };
            let pattern =
                predefined.to_hatch_pattern(self.spacing, self.angle_degrees, self.double);
            return Ok((HatchFill::Pattern(pattern), HatchPatternType::Predefined));
        }
        if self.pattern_type != HATCH_PATTERN_TYPE_USER_DEFINED {
            return Err(CadError::InvalidInput(format!(
                "HATCH pattern type must be {} (solid), {} (user-defined) or {} (predefined), got \
                 {}",
                HATCH_PATTERN_TYPE_SOLID,
                HATCH_PATTERN_TYPE_USER_DEFINED,
                HATCH_PATTERN_TYPE_PREDEFINED,
                self.pattern_type
            )));
        }
        self.scale_and_angle()?;
        let pattern = HatchPattern {
            name: USER_DEFINED_PATTERN_NAME.to_string(),
            angle: self.angle_degrees.to_radians(),
            scale: self.spacing,
            double: self.double,
            lines: vec![synthesized_user_defined_line()],
        };
        Ok((HatchFill::Pattern(pattern), HatchPatternType::UserDefined))
    }
}

/// One pattern-definition row at offset `(0, 1)` in pattern space.
///
/// This mirrors `cad_import_acadrust::hatch::import_pattern`'s `76=0`/`78=0`
/// synthesis exactly: the hatch-level scale is then the line spacing, the
/// hatch-level angle rotates the family and the double flag adds the
/// perpendicular family in [`HatchPattern::families`]. The generated geometry is
/// **our implementation** (ADR U-1), not a documented AutoCAD formula.
fn synthesized_user_defined_line() -> HatchPatternLine {
    HatchPatternLine {
        angle: 0.0,
        base_x: 0.0,
        base_y: 0.0,
        offset_x: 0.0,
        offset_y: 1.0,
        dashes: Vec::new(),
    }
}

/// Map the closed-set style code to the domain style.
pub(super) fn hatch_style(code: f64) -> CadResult<HatchStyle> {
    if code == HATCH_STYLE_NORMAL {
        Ok(HatchStyle::Normal)
    } else if code == HATCH_STYLE_OUTER {
        Ok(HatchStyle::Outer)
    } else if code == HATCH_STYLE_IGNORE {
        Ok(HatchStyle::Ignore)
    } else {
        Err(CadError::InvalidInput(format!(
            "HATCH style must be {HATCH_STYLE_NORMAL} (Normal), {HATCH_STYLE_OUTER} (Outer) or \
             {HATCH_STYLE_IGNORE} (Ignore), got {code}"
        )))
    }
}

/// Relabel a creation-path rejection with the command context (the reason itself
/// is the domain's, e.g. "self-intersecting").
pub(super) fn hatch_creation_refusal(error: CadError) -> CadError {
    match error {
        CadError::InvalidInput(reason) => {
            CadError::InvalidInput(format!("HATCH boundary rejected: {reason}"))
        }
        other => other,
    }
}

/// The explicit summary diagnostic (locale-neutral, like the ARRAY commands).
pub(super) fn hatch_diagnostic(
    object: ObjectId,
    loop_count: usize,
    stitched_edges: usize,
    style: HatchStyle,
    parameters: &HatchParameters,
) -> Diagnostic {
    let fill = if parameters.pattern_type == HATCH_PATTERN_TYPE_SOLID {
        "solid".to_string()
    } else if parameters.pattern_type == HATCH_PATTERN_TYPE_PREDEFINED {
        // The command already resolved the index successfully in `fill()`; the
        // name is repeated here so the echo names the pattern instead of
        // claiming a user-defined one.
        let name = parameters
            .predefined()
            .map(|pattern| pattern.name)
            .unwrap_or("<unresolved>");
        format!(
            "predefined pattern {name} (scale {}, angle {} deg, double {})",
            parameters.spacing, parameters.angle_degrees, parameters.double
        )
    } else {
        format!(
            "user-defined pattern (spacing {}, angle {} deg, double {})",
            parameters.spacing, parameters.angle_degrees, parameters.double
        )
    };
    Diagnostic {
        object: Some(object),
        code: "drawing.hatch".into(),
        message: format!(
            "HATCH: one entity with {loop_count} boundary loop(s) inserted (style {style:?}, \
             {fill}; {}associative hatch NOT IMPLEMENTED)",
            stitching_summary(stitched_edges)
        ),
    }
}

/// The stitching half of the summary: which edges were chained, that arcs stay
/// exact bulges and the tolerance that closed the chain.
fn stitching_summary(edges: usize) -> String {
    if edges == 0 {
        return String::new();
    }
    format!(
        "{edges} LINE/ARC edge(s) stitched into loops at the absolute tolerance \
         {STITCH_TOLERANCE:e} world units, arcs kept as exact bulge segments (never sampled); "
    )
}
