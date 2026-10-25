//! Predefined HATCH line patterns (pattern type code `2` = predefined).
//!
//! This is a **small hand-transcribed table**, not a pattern library: only
//! `ANSI31`, `ANSI32` and `ANSI37` exist, and they are selected by a **1-based
//! index** ([`predefined_pattern_by_index`]) or by name
//! ([`predefined_pattern`], trimmed and case-insensitive).
//!
//! **These are our transcribed values (ADR U-1 discipline), not vendor-verified
//! values.** The rows are written the way a `.pat` file states them — `angle`,
//! `base`, `offset` in the row's own rotated frame, plus optional dashes — but
//! their numbers were transcribed by hand and no reference CAD file, no vendor
//! renderer and no golden image was compared. `.pat` file loading is **NOT
//! IMPLEMENTED**: nothing here reads a pattern file, there is no pattern-name
//! resolution against `acad.pat`, and an unknown name/index is refused by the
//! caller rather than approximated.
//!
//! The rendered geometry comes from the existing
//! [`HatchPattern::families`](crate::HatchPattern::families) path (this module
//! only produces the definition rows), so predefined patterns need no renderer
//! work and get exactly the composition every imported pattern gets.

use crate::{HatchPattern, HatchPatternLine};

/// One predefined pattern-definition row, in the frame a `.pat` file states it.
///
/// `offset` is given in the row's **rotated** frame: its `y` component is the
/// perpendicular distance between neighbouring lines and its `x` component is
/// the shift along the line direction. [`PredefinedPattern::to_hatch_pattern`]
/// converts it into the plane frame that
/// [`HatchPattern::families`](crate::HatchPattern::families) consumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredefinedPatternLine {
    /// Line direction, in degrees.
    pub angle_deg: f64,
    /// A point on line `k = 0`, in pattern space.
    pub base: [f64; 2],
    /// Step to the next line, in the row's rotated frame (see the type docs).
    pub offset: [f64; 2],
    /// Dash pattern in pattern space: positive = dash, negative = gap, empty =
    /// solid (the same convention as [`HatchPatternLine::dashes`]).
    pub dashes: &'static [f64],
}

/// One predefined pattern: its stable uppercase name, a short description and
/// its definition rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredefinedPattern {
    /// Stable uppercase name (`ANSI31`, …); the value written as the HATCH
    /// pattern name (DXF group 2).
    pub name: &'static str,
    /// Short human-readable description of the family (advisory only; never
    /// used as a pattern identity).
    pub description: &'static str,
    /// The definition rows, in pattern space.
    pub lines: &'static [PredefinedPatternLine],
}

impl PredefinedPattern {
    /// Build the domain [`HatchPattern`] for this predefined pattern.
    ///
    /// The hatch-level `scale`/`angle` are stored as given (the angle in
    /// **radians**, the command channel's degrees are converted by the caller)
    /// and `double` is the `HPDOUBLE` flag. Every row is converted with
    /// [`HatchPatternLine`] so the existing composition
    /// ([`HatchPattern::families`]) does all the geometry — this module never
    /// re-implements it.
    ///
    /// Row offsets are **rotated by the row's own angle** here: a `.pat` row
    /// states its offset in the row's rotated frame, while
    /// [`HatchPatternLine::offset_x`]/[`HatchPatternLine::offset_y`] live in the
    /// hatch-plane frame (composition applies only the hatch-level angle and
    /// scale). Rotating at this boundary makes the composed result the `.pat`
    /// formula: the line angle is `row angle + hatch angle` and the
    /// perpendicular spacing is `offset_y * scale`. Every transcribed base point
    /// is the origin, so the frame is moot for it; a future row with a non-zero
    /// base would need the same rotation.
    pub fn to_hatch_pattern(&self, scale: f64, angle_deg: f64, double: bool) -> HatchPattern {
        HatchPattern {
            name: self.name.to_string(),
            angle: angle_deg.to_radians(),
            scale,
            double,
            lines: self
                .lines
                .iter()
                .map(|line| line.to_hatch_pattern_line())
                .collect(),
        }
    }
}

impl PredefinedPatternLine {
    /// Rotate the row into the hatch-plane frame (see
    /// [`PredefinedPattern::to_hatch_pattern`]).
    fn to_hatch_pattern_line(self) -> HatchPatternLine {
        let angle = self.angle_deg.to_radians();
        let (sin, cos) = angle.sin_cos();
        HatchPatternLine {
            angle,
            base_x: self.base[0],
            base_y: self.base[1],
            offset_x: cos * self.offset[0] - sin * self.offset[1],
            offset_y: sin * self.offset[0] + cos * self.offset[1],
            dashes: self.dashes.to_vec(),
        }
    }
}

/// `ANSI31` — one 45° family, perpendicular spacing `0.125` pattern units.
const ANSI31_LINES: &[PredefinedPatternLine] = &[PredefinedPatternLine {
    angle_deg: 45.0,
    base: [0.0, 0.0],
    offset: [0.0, 0.125],
    dashes: &[],
}];

/// `ANSI32` — one dashed 45° family (`0.125` dash, `0.0625` gap).
///
/// A real `ANSI32` pattern has a second, solid row; this build transcribes the
/// dashed row only (declared, not silently merged).
const ANSI32_LINES: &[PredefinedPatternLine] = &[PredefinedPatternLine {
    angle_deg: 45.0,
    base: [0.0, 0.0],
    offset: [0.0, 0.375],
    dashes: &[0.125, -0.0625],
}];

/// `ANSI37` — crossed 45° and 135° families, perpendicular spacing `0.125`.
const ANSI37_LINES: &[PredefinedPatternLine] = &[
    PredefinedPatternLine {
        angle_deg: 45.0,
        base: [0.0, 0.0],
        offset: [0.0, 0.125],
        dashes: &[],
    },
    PredefinedPatternLine {
        angle_deg: 135.0,
        base: [0.0, 0.0],
        offset: [0.0, 0.125],
        dashes: &[],
    },
];

/// The implemented predefined patterns, in **index order** (index `1` first).
const PREDEFINED: [PredefinedPattern; 3] = [
    PredefinedPattern {
        name: "ANSI31",
        description: "Iron, brick and stone masonry: 45 degree lines",
        lines: ANSI31_LINES,
    },
    PredefinedPattern {
        name: "ANSI32",
        description: "Steel: dashed 45 degree lines",
        lines: ANSI32_LINES,
    },
    PredefinedPattern {
        name: "ANSI37",
        description: "Lead, zinc and sound insulation: crossed 45 and 135 degree lines",
        lines: ANSI37_LINES,
    },
];

/// The implemented predefined patterns, in **index order**.
///
/// The slice length is the closed set the typed parameter channel validates
/// against, so the index can never be accepted for a pattern this table does not
/// hold.
pub fn predefined_patterns() -> &'static [PredefinedPattern] {
    &PREDEFINED
}

/// Resolve a name (trimmed, case-insensitive) to an implemented pattern.
///
/// `None` means "not in the implemented set"; callers refuse by name and never
/// fall back to another pattern.
pub fn predefined_pattern(name: &str) -> Option<&'static PredefinedPattern> {
    let wanted = name.trim();
    PREDEFINED
        .iter()
        .find(|pattern| pattern.name.eq_ignore_ascii_case(wanted))
}

/// Resolve a **1-based** index to an implemented pattern.
///
/// Index `0` and anything past the table length return `None`; there is no
/// clamping and no modulo.
pub fn predefined_pattern_by_index(index: usize) -> Option<&'static PredefinedPattern> {
    index.checked_sub(1).and_then(|index| PREDEFINED.get(index))
}
