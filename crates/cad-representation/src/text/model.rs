use cad_domain::Point3;

/// An honest report that some MTEXT formatting could not be applied exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFormatIssue {
    /// Stable reason code (see [`text_issue`]).
    pub code: &'static str,
    /// Human-readable explanation; may change without notice.
    pub message: String,
}

impl TextFormatIssue {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        TextFormatIssue {
            code,
            message: message.into(),
        }
    }
}

/// An MTEXT/TEXT color override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextColor {
    /// AutoCAD Color Index (`\C`); 0 = ByBlock, 256 = ByLayer.
    Aci(u16),
    /// 24-bit sRGB from `\c`.
    Rgb([u8; 3]),
}

/// A `\S` stacked fraction/limit.
///
/// The numerator and denominator are kept structured; the built-in layout
/// renders them inline (see [`text_issue::STACKED_FRACTION_FLAT`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackedFraction {
    pub numerator: String,
    pub denominator: String,
    /// The separator as written: `/` (bar), `#` (diagonal) or `^` (limit).
    pub separator: char,
}

/// One run of characters sharing an effective style, after parsing.
///
/// A run either carries literal [`text`](Self::text) or, for a `\S` code, a
/// structured [`fraction`](Self::fraction); [`rendered_text`](Self::rendered_text)
/// resolves both to the string the layout engine draws.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Literal characters in this run (empty for a fraction run).
    pub text: String,
    /// Structured `\S` fraction, when this run is a stack.
    pub fraction: Option<StackedFraction>,
    /// Font override (`\f`/`\F`), `None` means the entity/paragraph font.
    pub font: Option<String>,
    /// Effective absolute height for this run, in world units.
    pub height: f64,
    /// Horizontal width factor (`\W`); `1.0` is normal.
    pub width_factor: f64,
    /// Oblique angle in degrees, positive leans right (`\Q`); `0.0` is upright.
    pub oblique_deg: f64,
    /// Inline color override (`\C`/`\c`); `None` means inherit.
    pub color: Option<TextColor>,
    /// `\A` line-alignment code (0 = bottom/2 = top); `None` means baseline.
    pub line_align: Option<u8>,
    /// `\L` underline toggle active for this run.
    pub underline: bool,
    /// `\O` overline toggle active for this run.
    pub overline: bool,
    /// `\K` strikethrough toggle active for this run.
    pub strike: bool,
}

impl TextRun {
    /// The text this run draws: its literal text, or `num<sep>den` for a stack.
    pub fn rendered_text(&self) -> String {
        match &self.fraction {
            Some(fraction) => format!(
                "{}{}{}",
                fraction.numerator, fraction.separator, fraction.denominator
            ),
            None => self.text.clone(),
        }
    }

    /// The run's height if it is a positive finite value, else `fallback`.
    pub(super) fn effective_height(&self, fallback: f64) -> f64 {
        if self.height.is_finite() && self.height > 0.0 {
            self.height
        } else {
            fallback
        }
    }

    /// Whether two runs share the same style (ignoring their text/fraction).
    pub(super) fn same_style(&self, other: &TextRun) -> bool {
        self.font == other.font
            && self.height == other.height
            && self.width_factor == other.width_factor
            && self.oblique_deg == other.oblique_deg
            && self.color == other.color
            && self.line_align == other.line_align
            && self.underline == other.underline
            && self.overline == other.overline
            && self.strike == other.strike
    }
}

/// One paragraph (line) of parsed MTEXT, an ordered list of styled runs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLine {
    pub runs: Vec<TextRun>,
}

/// The result of parsing MTEXT/TEXT control content.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedText {
    /// Paragraphs in reading order; always at least one (possibly empty).
    pub lines: Vec<TextLine>,
    /// Formatting that could not be represented exactly.
    pub issues: Vec<TextFormatIssue>,
}

/// The result of shaping a text run, including the style overrides that shaped
/// it and an honest report of anything approximated.
#[derive(Debug, Clone)]
pub struct ShapedText {
    /// World-space line geometry: SHX outlines and SFNT fill-failure fallback.
    pub polylines: Vec<Vec<Point3>>,
    /// World-space SFNT glyph contours, retained for the public outline API.
    pub outlines: Vec<Vec<Point3>>,
    /// Filled SFNT glyph faces. SHX fonts remain outline-only.
    pub meshes: Vec<cad_domain::Mesh>,
    /// Whether parsed text has at least one non-whitespace drawable character.
    pub has_renderable_content: bool,
    /// The flattened styled runs in reading order, for hosts that apply color.
    pub runs: Vec<TextRun>,
    /// Approximations/unsupported formatting encountered while parsing/shaping.
    pub issues: Vec<TextFormatIssue>,
}
