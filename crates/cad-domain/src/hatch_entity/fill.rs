//! The HATCH pattern / gradient fill definition (ADR 0018 A.1).
//!
//! Split out of `hatch_entity.rs` to stay inside the code-size gate: the entity
//! ([`super::HatchGeometry`]) owns the loops, plane and derived display, while
//! these types own the raw definition rows and the gradient block.

use crate::hatch::{GradientDef, GradientKind, GradientStop, PatternLine};

/// One pattern-definition line, in **pattern space** (DXF groups 53/43/44/45/46
/// and 49). The hatch-level angle (group 52) and scale (group 41) are applied by
/// [`HatchPattern::families`], so these values round-trip unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base_x: f64,
    pub base_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub dashes: Vec<f64>,
}

/// The HATCH pattern definition: name, hatch-level angle/scale, the double flag
/// and the definition rows (ADR A.1).
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPattern {
    pub name: String,
    /// DXF group 52: pattern angle in radians.
    pub angle: f64,
    /// DXF group 41: pattern scale. For a user-defined pattern this is the
    /// spacing between the synthesized line families.
    pub scale: f64,
    /// DXF group 77: draw a second family perpendicular to the first.
    pub double: bool,
    pub lines: Vec<HatchPatternLine>,
}

impl HatchPattern {
    /// Compose the definition rows into hatch-plane line families.
    ///
    /// The composite is **our implementation** (ADR U-1): the hatch-level angle
    /// rotates both the base point and the offset, and the hatch-level scale
    /// scales the base point, the offset and the dash lengths. Before P6a the
    /// importer rotated only the base point, did not scale it and did not rotate
    /// the offset (ADR U-7); both are fixed here and pinned by a numeric test.
    /// The `double` flag adds a family at +90 degrees whose offset is the first
    /// family's offset rotated by +90 degrees.
    pub fn families(&self) -> Vec<PatternLine> {
        let (sin, cos) = self.angle.sin_cos();
        let rotate = |x: f64, y: f64| [cos * x - sin * y, sin * x + cos * y];
        let mut out = Vec::new();
        for line in &self.lines {
            let base = rotate(self.scale * line.base_x, self.scale * line.base_y);
            let offset = rotate(self.scale * line.offset_x, self.scale * line.offset_y);
            let dashes: Vec<f64> = line.dashes.iter().map(|d| d * self.scale).collect();
            let angle = line.angle + self.angle;
            out.push(PatternLine {
                angle,
                base,
                offset,
                dashes: dashes.clone(),
            });
            if self.double {
                out.push(PatternLine {
                    angle: angle + std::f64::consts::FRAC_PI_2,
                    base,
                    offset: [-offset[1], offset[0]],
                    dashes,
                });
            }
        }
        out
    }

    /// Add a family rotation (in-plane radians) and scale, keeping the raw
    /// definition rows untouched.
    pub(super) fn transformed(&self, delta: f64, scale: f64) -> Self {
        HatchPattern {
            name: self.name.clone(),
            angle: self.angle + delta,
            scale: self.scale * scale,
            double: self.double,
            lines: self.lines.clone(),
        }
    }
}

/// A gradient fill (ADR A.1/B3).
///
/// The variant is the DXF colour mode; `name` carries the gradient kind
/// (`LINEAR`, `SPHERICAL`, ...). Group 460 (`angle_raw`) is the gradient angle
/// **in radians** per the DXF reference and acadrust's own field documentation;
/// it is stored verbatim and never converted, and the bake
/// ([`GradientDef::angle`]) interprets it as radians — pinned by
/// `gradient_angle_is_stored_in_radians_not_degrees`. Do not convert it to or
/// from degrees. Neither the writer nor the reader rewrites the value, so this
/// is a documented-unit claim, not a vendor-file round-trip.
#[derive(Debug, Clone, PartialEq)]
pub enum HatchGradient {
    TwoColor {
        name: String,
        reserved: i32,
        angle_raw: f64,
        shift: f64,
        stops: Vec<GradientStop>,
    },
    SingleColor {
        name: String,
        reserved: i32,
        angle_raw: f64,
        shift: f64,
        tint: f64,
        stops: Vec<GradientStop>,
    },
}

impl HatchGradient {
    pub fn name(&self) -> &str {
        match self {
            HatchGradient::TwoColor { name, .. } | HatchGradient::SingleColor { name, .. } => name,
        }
    }

    /// Rotate the ramp direction by `delta` radians about the plane normal.
    ///
    /// Sign convention: **`angle_raw += delta`**, the same `+ delta` that
    /// [`HatchPattern::transformed`] applies to the pattern angle. `angle_raw`
    /// is consumed as the direction `(cos angle, sin angle)` in the hatch-plane
    /// frame (`crate::hatch::gradient`), and a similarity whose induced in-plane
    /// rotation is `delta` maps every direction `phi` in that frame to
    /// `phi + delta` — the loops turn one way, so the ramp must turn the same
    /// way. Without this the geometry rotates but the gradient stays behind.
    pub(super) fn rotated(&self, delta: f64) -> Self {
        match self {
            HatchGradient::TwoColor {
                name,
                reserved,
                angle_raw,
                shift,
                stops,
            } => HatchGradient::TwoColor {
                name: name.clone(),
                reserved: *reserved,
                angle_raw: angle_raw + delta,
                shift: *shift,
                stops: stops.clone(),
            },
            HatchGradient::SingleColor {
                name,
                reserved,
                angle_raw,
                shift,
                tint,
                stops,
            } => HatchGradient::SingleColor {
                name: name.clone(),
                reserved: *reserved,
                angle_raw: angle_raw + delta,
                shift: *shift,
                tint: *tint,
                stops: stops.clone(),
            },
        }
    }

    fn stops(&self) -> &[GradientStop] {
        match self {
            HatchGradient::TwoColor { stops, .. } | HatchGradient::SingleColor { stops, .. } => {
                stops
            }
        }
    }

    /// The gradient kind this build can bake, or `None` for a curved/inversion
    /// kind (declared unsupported, never approximated as a straight ramp).
    pub(super) fn kind(&self) -> Option<GradientKind> {
        match self.name().trim().to_ascii_uppercase().as_str() {
            "LINEAR" => Some(GradientKind::Linear),
            "SPHERICAL" | "CYLINDER" => Some(GradientKind::Spherical),
            // A missing name is ambiguous; fall back to LINEAR only when the
            // file actually carried stops (the pre-P6a rule, preserved).
            "" if !self.stops().is_empty() => Some(GradientKind::Linear),
            _ => None,
        }
    }

    /// The resolved bake definition, or `None` when the gradient is outside the
    /// implemented subset (unsupported kind or unusable stops).
    pub(super) fn definition(&self) -> Option<GradientDef> {
        let kind = self.kind()?;
        let (shift, single_color, tint) = match self {
            HatchGradient::TwoColor { shift, .. } => (*shift, false, 0.0),
            HatchGradient::SingleColor { shift, tint, .. } => (*shift, true, *tint),
        };
        let def = GradientDef {
            kind,
            angle: match self {
                HatchGradient::TwoColor { angle_raw, .. }
                | HatchGradient::SingleColor { angle_raw, .. } => *angle_raw,
            },
            shift,
            single_color,
            tint,
            stops: self.stops().to_vec(),
        };
        def.is_usable().then_some(def)
    }
}

/// The pattern / gradient definition of a hatch.
#[derive(Debug, Clone, PartialEq)]
pub enum HatchFill {
    Solid,
    Pattern(HatchPattern),
    Gradient(HatchGradient),
}
