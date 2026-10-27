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

use cad_domain::hatch::{
    predefined_pattern_by_index, predefined_patterns, GradientDef, GradientKind, GradientStop,
    PredefinedPattern,
};

use crate::draw_tool::{
    ToolParameter, HATCH_GRADIENT_LINEAR, HATCH_GRADIENT_SPHERICAL, HATCH_PATTERN_TYPE_GRADIENT,
    HATCH_PATTERN_TYPE_PREDEFINED, HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED,
    HATCH_STYLE_IGNORE, HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER,
};

/// The gradient-kind name written for the `LINEAR` kind.
const GRADIENT_NAME_LINEAR: &str = "LINEAR";
/// The gradient-kind name written for the `SPHERICAL` kind.
///
/// `CYLINDER` is never written: the domain bakes it as spherical, so emitting it
/// would make one behaviour answer to two names.
const GRADIENT_NAME_SPHERICAL: &str = "SPHERICAL";

/// The fixed DXF `reserved` value (group 451) of a created gradient.
const GRADIENT_RESERVED: i32 = 0;
/// The fixed normalised ramp start (group 461) of a created gradient: no shift.
const GRADIENT_SHIFT: f64 = 0.0;

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
    /// Gradient kind closed set (`0` linear, `1` spherical); only read for
    /// `HATCH_PATTERN_TYPE_GRADIENT`.
    gradient_kind: f64,
    /// Gradient angle in **degrees** (the prompt unit). The entity stores
    /// radians, converted exactly once in [`HatchParameters::gradient`].
    gradient_angle_degrees: f64,
    /// The gradient start-stop colour as an ACI index (`hatchGradientColorA`).
    gradient_color_a: f64,
    /// The gradient end-stop colour as an ACI index (`hatchGradientColorB`).
    gradient_color_b: f64,
}

/// Read and defensively re-validate the HATCH typed parameters.
///
/// The parameter set is **fill-dependent**: a gradient never consults the
/// pattern spacing/angle (`scale_and_angle`), so a stale invalid spacing must
/// not refuse a gradient and likewise a stale invalid gradient colour must not
/// refuse a pattern. The channel already refuses those values at set time; this
/// re-validation is the defence in depth for a value injected past the channel.
///
/// The predefined pattern index is deliberately **not** re-validated here: it is
/// resolved in [`HatchParameters::fill`], which refuses an index outside the
/// implemented table with a message that names the index and the table (the
/// generic range refusal would hide which pattern set the command actually
/// implements).
pub(super) fn hatch_parameters(session: &SessionState) -> CadResult<HatchParameters> {
    let validate = |parameter: ToolParameter| -> CadResult<()> {
        parameter
            .validate(session.tool_parameter(parameter))
            .map_err(|error| match error {
                CadError::InvalidInput(message) => {
                    CadError::InvalidInput(format!("HATCH: {message}"))
                }
                other => other,
            })
    };
    // Always validated: the fill selector itself, the style and the flag.
    for parameter in [
        ToolParameter::HatchPatternType,
        ToolParameter::HatchDouble,
        ToolParameter::HatchStyle,
    ] {
        validate(parameter)?;
    }
    let number = |parameter: ToolParameter| {
        session
            .tool_parameter(parameter)
            .number()
            .unwrap_or(f64::NAN)
    };
    let parameters = HatchParameters {
        pattern_type: number(ToolParameter::HatchPatternType),
        pattern_index: number(ToolParameter::HatchPattern),
        spacing: number(ToolParameter::HatchSpacing),
        angle_degrees: number(ToolParameter::HatchAngle),
        double: session
            .tool_parameter(ToolParameter::HatchDouble)
            .flag()
            .unwrap_or(false),
        style: number(ToolParameter::HatchStyle),
        gradient_kind: number(ToolParameter::HatchGradientKind),
        gradient_angle_degrees: number(ToolParameter::HatchGradientAngle),
        gradient_color_a: number(ToolParameter::HatchGradientColorA),
        gradient_color_b: number(ToolParameter::HatchGradientColorB),
    };
    let fill_parameters: &[ToolParameter] = if parameters.is_gradient() {
        &[
            ToolParameter::HatchGradientKind,
            ToolParameter::HatchGradientAngle,
            ToolParameter::HatchGradientColorA,
            ToolParameter::HatchGradientColorB,
        ]
    } else {
        &[ToolParameter::HatchSpacing, ToolParameter::HatchAngle]
    };
    for parameter in fill_parameters {
        validate(*parameter)?;
    }
    Ok(parameters)
}

impl HatchParameters {
    /// Whether the fill selector names a gradient.
    fn is_gradient(&self) -> bool {
        self.pattern_type == HATCH_PATTERN_TYPE_GRADIENT
    }

    /// The gradient kind name actually written, or `None` when the stored code is
    /// outside the closed set.
    fn gradient_name(&self) -> Option<&'static str> {
        if self.gradient_kind == HATCH_GRADIENT_LINEAR {
            Some(GRADIENT_NAME_LINEAR)
        } else if self.gradient_kind == HATCH_GRADIENT_SPHERICAL {
            Some(GRADIENT_NAME_SPHERICAL)
        } else {
            None
        }
    }

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

    /// The two-colour gradient definition and its validation.
    ///
    /// Every field is re-validated here (the channel already refused an
    /// out-of-set value, so this is defence in depth) and the assembled
    /// definition is checked against `GradientDef::is_usable`, the same predicate
    /// the bake consults: a gradient this build could not render must be refused
    /// **by the command**, never deferred to "cannot build the hatch display".
    ///
    /// Spacing, double and style are deliberately not read: they belong to the
    /// pattern kinds and must not influence (or refuse) a gradient.
    fn gradient(&self) -> CadResult<HatchGradient> {
        let Some(name) = self.gradient_name() else {
            return Err(CadError::InvalidInput(format!(
                "HATCH gradient kind must be {HATCH_GRADIENT_LINEAR} (LINEAR) or \
                 {HATCH_GRADIENT_SPHERICAL} (SPHERICAL), got {}",
                self.gradient_kind
            )));
        };
        if !self.gradient_angle_degrees.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "HATCH gradient angle must be finite, got {}",
                self.gradient_angle_degrees
            )));
        }
        // The single degrees -> radians conversion of this path. The import path
        // stores group 460 verbatim and never converts, so this is the only place
        // the two conventions meet (pinned by the create/export/reimport test).
        let angle_raw = self.gradient_angle_degrees.to_radians();
        let stops = vec![
            gradient_stop(0.0, self.gradient_color_a, "A")?,
            gradient_stop(1.0, self.gradient_color_b, "B")?,
        ];
        let kind = if name == GRADIENT_NAME_LINEAR {
            GradientKind::Linear
        } else {
            GradientKind::Spherical
        };
        let definition = GradientDef {
            kind,
            angle: angle_raw,
            shift: GRADIENT_SHIFT,
            single_color: false,
            tint: 0.0,
            stops: stops.clone(),
        };
        if !definition.is_usable() {
            return Err(CadError::InvalidInput(format!(
                "HATCH gradient definition is not usable ({} stop(s), angle {angle_raw}, shift \
                 {GRADIENT_SHIFT})",
                stops.len()
            )));
        }
        Ok(HatchGradient::TwoColor {
            name: name.to_string(),
            reserved: GRADIENT_RESERVED,
            angle_raw,
            shift: GRADIENT_SHIFT,
            stops,
        })
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
    /// table entry the index names. A gradient fill (pattern type code `3`) also
    /// records `Predefined` (`76=1`): a gradient HATCH is written as a solid fill
    /// plus the 450-470 block, so it has no definition rows (an **unverified**
    /// vendor convention, declared).
    ///
    /// The gradient branch comes **before** [`Self::scale_and_angle`]: a gradient
    /// has no line family, so a stale non-positive spacing must not refuse it and
    /// spacing/double/style must not affect it.
    pub(super) fn fill(&self) -> CadResult<(HatchFill, HatchPatternType)> {
        if self.pattern_type == HATCH_PATTERN_TYPE_SOLID {
            return Ok((HatchFill::Solid, HatchPatternType::Predefined));
        }
        if self.is_gradient() {
            return Ok((
                HatchFill::Gradient(self.gradient()?),
                HatchPatternType::Predefined,
            ));
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
                "HATCH pattern type must be {} (solid), {} (user-defined), {} (predefined) or {} \
                 (gradient), got {}",
                HATCH_PATTERN_TYPE_SOLID,
                HATCH_PATTERN_TYPE_USER_DEFINED,
                HATCH_PATTERN_TYPE_PREDEFINED,
                HATCH_PATTERN_TYPE_GRADIENT,
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

/// One gradient colour stop resolved from an ACI index.
///
/// The `ToolValue` channel carries no colour type, so the stop colour arrives as
/// an integer ACI index and is resolved through the canonical palette here — one
/// conversion, never a caller-supplied triple. Index `0` is the ByBlock
/// placeholder with **no** sRGB triple ([`crate::property_value::rgb_for_aci`]
/// returns `None`), so it is refused by name instead of being written as black.
fn gradient_stop(value: f64, aci: f64, label: &str) -> CadResult<GradientStop> {
    if !aci.is_finite() || aci.fract() != 0.0 || !(0.0..=255.0).contains(&aci) {
        return Err(CadError::InvalidInput(format!(
            "HATCH gradient colour {label} must be an integer ACI index between 0 and 255, got \
             {aci}"
        )));
    }
    let index = aci as u16;
    let Some(rgb) = crate::property_value::rgb_for_aci(index) else {
        return Err(CadError::InvalidInput(format!(
            "HATCH gradient colour {label} ACI {index} is the ByBlock placeholder, not a colour \
             (use 1..=255)"
        )));
    };
    Ok(GradientStop { value, rgb })
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
    } else if parameters.is_gradient() {
        // The command already validated the gradient in `fill()`; the name and
        // colours are echoed so the summary says which ramp was written instead
        // of naming a pattern that does not exist.
        format!(
            "gradient {} (angle {} deg, ACI {} -> {}, stops 0/1)",
            parameters.gradient_name().unwrap_or("<unresolved>"),
            parameters.gradient_angle_degrees,
            parameters.gradient_color_a,
            parameters.gradient_color_b
        )
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
