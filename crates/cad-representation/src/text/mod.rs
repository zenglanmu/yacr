//! Text outlining: TrueType/OpenType and compiled SHX shape fonts (spec §3.2, §7.1).
//!
//! Hosts supply font bytes (raw sfnt, WOFF1, or SHX); this module turns a text
//! run into world-space contours and filled SFNT glyph meshes. SHX remains
//! linework. A fallback chain is applied when a drawing's referenced font
//! is not registered or cannot be parsed, so text is not silently dropped.
//!
//! ## MTEXT formatting
//!
//! [`parse_mtext`] parses MTEXT/TEXT control content into a structured run list
//! ([`ParsedText`]) instead of stripping codes: paragraphs (`\P`, literal `\n`,
//! `^J`), grouping (`{...}`), fonts (`\f`/`\F`), absolute and relative height
//! (`\H`), width factor (`\W`), oblique (`\Q`), line alignment (`\A`), color
//! (`\C` ACI / `\c` true color), stacked fractions (`\S`), non-breaking space
//! (`\~`), special characters (`%%d`/`%%p`/`%%c`/`%%%%`), Unicode escapes
//! (`\U+XXXX`) and literal escapes. [`FontEngine::shape`] lays the runs out
//! honouring per-run font/height/width/oblique and returns filled faces, outline
//! contours, the run list and honest [`TextFormatIssue`]s for anything that could
//! only be approximated. [`FontEngine::outline`] stays a thin wrapper returning
//! all transformed glyph contours, including the contours used for SFNT fills.
//!
//! ## Known limitations (explicit, not silently approximated)
//!
//! This is a glyph-per-glyph outline renderer, not a full text shaping engine:
//!
//! - **Bidirectional text and complex-script reordering** are not performed; the
//!   logical order is rendered as-is.
//! - **Contextual shaping** (Arabic joining, Indic reordering, ligature
//!   substitution) is not applied; the OpenType `kern` table is the only layout
//!   feature consulted.
//! - **MTEXT columns, paragraph indents and tab stops** are ignored. `\t`/`^I`
//!   expand to a fixed run of spaces (see `TAB_STEP`).
//! - **Exact vertical metrics** are not read from the font; line spacing is
//!   [`LINE_SPACING`] times the tallest run on the line. Vertical alignment of a
//!   multi-line block against its anchor remains an approximation.
//! - **Stacked fractions** (`\S`) keep their structured numerator/denominator in
//!   the run list, but the built-in layout draws them inline as `num<sep>den`
//!   and reports [`text_issue::STACKED_FRACTION_FLAT`].
//! - **Inline color and underline/overline/strikethrough** are parsed and kept
//!   in the run list, but the polyline output is uncolored and undecorated;
//!   [`FontEngine::shape`] reports that as a [`TextFormatIssue`].
//! - **Per-glyph fallback only**: a missing glyph is substituted from the next
//!   face in the chain. There is no per-style (bold/italic) or per-script fallback.
//!
//! See `docs/mtext.md` and `docs/fonts.md` for the tracked gaps.

mod engine;
mod faces;
mod finalize;
mod layout;
mod model;
mod mtext;
mod sanitize;

pub use engine::FontEngine;
pub use layout::LINE_SPACING;
pub use model::{
    ParsedText, ShapedText, StackedFraction, TextColor, TextFormatIssue, TextLine, TextRun,
};
pub use mtext::parse_mtext;
pub use sanitize::sanitize_text;

/// Stable, machine-readable codes for MTEXT parsing/layout [`TextFormatIssue`]s.
///
/// They are part of the contract: callers may branch on the code and the
/// human-readable message may change without breaking them.
pub mod text_issue {
    /// An unrecognised `\X` control code was rendered literally.
    pub const UNKNOWN_CODE: &str = "mtext.unknown_code";
    /// A recognised code carried a missing or non-numeric value.
    pub const MALFORMED_CODE: &str = "mtext.malformed_code";
    /// `\S` stacked fractions are drawn inline (`num<sep>den`), not stacked.
    pub const STACKED_FRACTION_FLAT: &str = "mtext.stacked_fraction_flat";
    /// Inline color was parsed but the line output carries no color.
    pub const COLOR_NOT_APPLIED: &str = "mtext.color_not_applied";
    /// Underline/overline/strikethrough toggles were parsed but not drawn.
    pub const DECORATION_NOT_APPLIED: &str = "mtext.decoration_not_applied";
    /// `\T` character tracking was parsed but not applied.
    pub const TRACKING_NOT_APPLIED: &str = "mtext.tracking_not_applied";
    /// `\p...;` paragraph properties (indents/alignment/tabs) are ignored.
    pub const PARAGRAPH_PROPERTIES: &str = "mtext.paragraph_properties";
    /// `\N` column break was treated as a plain line break.
    pub const COLUMN_BREAK: &str = "mtext.column_break";
    /// `\A` run/line vertical alignment was parsed but not applied.
    pub const LINE_ALIGNMENT: &str = "mtext.line_alignment_not_applied";
    /// `\B` background mask was parsed but not drawn.
    pub const BACKGROUND_MASK: &str = "mtext.background_mask_not_applied";
    /// A run named a font that is neither registered nor covered by the fallback.
    pub const FONT_UNAVAILABLE: &str = "mtext.font_unavailable";
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wrap_tests;
