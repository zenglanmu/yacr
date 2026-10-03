//! Text outlining: TrueType/OpenType and compiled SHX shape fonts (spec §3.2, §7.1).
//!
//! Hosts supply font bytes (raw sfnt, WOFF1, or SHX); this module turns a text
//! run into world-space polylines so the scene can batch text like any other
//! line geometry. A fallback chain is applied when a drawing's referenced font
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
//! honouring per-run font/height/width/oblique and returns the polylines, the
//! run list and a list of honest [`TextFormatIssue`]s for anything that could
//! only be approximated. [`FontEngine::outline`] stays a thin wrapper returning
//! only the polylines.
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

use cad_domain::{CadError, CadResult, Point3, TextAlignH, TextAlignV};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use ttf_parser::OutlineBuilder;

use crate::shx::ShxFont;

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

/// An honest report that some MTEXT formatting could not be applied exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFormatIssue {
    /// Stable reason code (see [`text_issue`]).
    pub code: &'static str,
    /// Human-readable explanation; may change without notice.
    pub message: String,
}

impl TextFormatIssue {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
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
    fn effective_height(&self, fallback: f64) -> f64 {
        if self.height.is_finite() && self.height > 0.0 {
            self.height
        } else {
            fallback
        }
    }

    /// Whether two runs share the same style (ignoring their text/fraction).
    fn same_style(&self, other: &TextRun) -> bool {
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
    /// World-space polylines, one per glyph contour.
    pub polylines: Vec<Vec<Point3>>,
    /// The flattened styled runs in reading order, for hosts that apply color.
    pub runs: Vec<TextRun>,
    /// Approximations/unsupported formatting encountered while parsing/shaping.
    pub issues: Vec<TextFormatIssue>,
}

/// A parsed font face.
enum FaceData {
    /// Raw sfnt bytes (TTF/OTF, or WOFF1 already decoded to sfnt).
    Sfnt(Arc<[u8]>),
    /// Compiled AutoCAD shape font.
    Shx(ShxFont),
    /// A face we recognised but cannot decode yet; reported explicitly.
    Unsupported(String),
}

/// A face from the resolved fallback chain that can actually supply glyphs.
///
/// `Sfnt` faces are parsed once per [`FontEngine::outline`] call and borrow the
/// caller's kept-alive `Arc<[u8]>`; `Shx` faces are borrowed from the chain.
enum ResolvedFace<'a> {
    Sfnt {
        // Boxed: `ttf_parser::Face` is several KiB and would dominate this enum.
        face: Box<ttf_parser::Face<'a>>,
        kern: Option<ttf_parser::kern::Table<'a>>,
        /// World units per font unit: `height / units_per_em`.
        scale: f64,
    },
    Shx(&'a ShxFont),
}

/// A bounded, in-memory set of font faces with an optional fallback chain.
///
/// Faces are keyed by the name a drawing references (`arial.ttf`) and by the
/// file stem (`arial`), so a drawing that names a `.ttf` still resolves to the
/// matching `.woff` catalog entry. No I/O happens here.
#[derive(Default)]
pub struct FontEngine {
    faces: HashMap<String, Arc<FaceData>>,
    fallback: Vec<String>,
}

impl FontEngine {
    pub fn new() -> Self {
        FontEngine::default()
    }

    /// Register font bytes under `key` plus its file stem.
    ///
    /// SHX shape fonts, raw sfnt and WOFF1 are recognised; WOFF2 is rejected
    /// explicitly. The result must parse or the registration fails.
    pub fn register(&mut self, key: &str, bytes: Arc<[u8]>) -> CadResult<()> {
        self.register_with_encoding(key, bytes, None)
    }

    /// Like [`register`](Self::register) but declares the SHX code page used to
    /// map Unicode back to the font's character codes (for example `gbk`).
    pub fn register_with_encoding(
        &mut self,
        key: &str,
        bytes: Arc<[u8]>,
        encoding: Option<&str>,
    ) -> CadResult<()> {
        let normalized = normalize_key(key);
        if normalized.is_empty() {
            return Err(CadError::InvalidInput("font key is empty".into()));
        }
        let face = Arc::new(parse_face(&normalized, &bytes, encoding)?);
        self.faces.insert(normalized.clone(), face.clone());
        if let Some(stem) = stem_of(&normalized) {
            self.faces.entry(stem).or_insert(face);
        }
        Ok(())
    }

    /// Set the fallback chain used when a referenced font is missing.
    ///
    /// Keys are matched as in [`register`](Self::register); the first that
    /// resolves to a face is used for the whole run.
    pub fn set_fallback(&mut self, keys: Vec<String>) {
        self.fallback = keys;
    }

    pub fn fallback_keys(&self) -> &[String] {
        &self.fallback
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Whether a face for `key` (or its stem) is available.
    pub fn contains(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    fn lookup(&self, key: &str) -> Option<Arc<FaceData>> {
        let normalized = normalize_key(key);
        if let Some(data) = self.faces.get(&normalized) {
            return Some(data.clone());
        }
        stem_of(&normalized).and_then(|stem| self.faces.get(&stem).cloned())
    }

    /// The face used for `font_key`: the primary face, else the first available
    /// fallback. Public so hosts can report which font actually rendered.
    pub fn resolve_face<'a>(&'a self, font_key: &'a str) -> Option<&'a str> {
        if self.lookup(font_key).is_some() {
            return Some(font_key);
        }
        self.fallback
            .iter()
            .find(|key| self.lookup(key).is_some())
            .map(String::as_str)
    }

    /// Outline one SHX shape glyph (by shape code) into world-space polylines.
    ///
    /// Returns `Unsupported` when `font_key` does not resolve to an SHX shape
    /// font and `InvalidInput` when the code is absent, so a SHAPE entity is
    /// never silently dropped or guessed.
    pub fn shape_glyph(
        &self,
        font_key: &str,
        code: u32,
        origin: Point3,
        size: f64,
        rotation: f64,
    ) -> CadResult<Vec<Vec<Point3>>> {
        if !size.is_finite() || size <= 0.0 {
            return Err(CadError::InvalidInput(
                "shape size must be positive and finite".into(),
            ));
        }
        let Some(face) = self.lookup(font_key) else {
            return Err(self.chain_error(font_key));
        };
        let FaceData::Shx(font) = &*face else {
            return Err(CadError::Unsupported(format!(
                "font '{font_key}' is not an SHX shape font"
            )));
        };
        let glyph = font.glyph_by_code(code, size).ok_or_else(|| {
            CadError::InvalidInput(format!("shape code {code} is not in font '{font_key}'"))
        })?;
        let (sin, cos) = rotation.sin_cos();
        Ok(glyph
            .polylines
            .iter()
            .filter(|polyline| polyline.len() >= 2)
            .map(|polyline| {
                polyline
                    .iter()
                    .map(|p| Point3 {
                        x: origin.x + p[0] * cos - p[1] * sin,
                        y: origin.y + p[0] * sin + p[1] * cos,
                        z: origin.z,
                    })
                    .collect()
            })
            .collect())
    }

    /// Parse MTEXT/TEXT formatting in `raw` and shape it into world polylines.
    ///
    /// This is the rich counterpart to [`outline`](Self::outline): it honours
    /// the per-run font/height/width/oblique overrides carried by the MTEXT
    /// control content (see [`parse_mtext`]) and returns the flattened styled
    /// runs plus an honest list of [`TextFormatIssue`]s for formatting that
    /// could only be approximated (inline color, decorations, stacked
    /// fractions, ...).
    ///
    /// `origin` is the alignment/anchor point; `height` is the base em height in
    /// world units; `rotation` is radians about `origin`. When the requested
    /// font is missing the fallback chain is used, resolved per glyph.
    #[allow(clippy::too_many_arguments)]
    pub fn shape(
        &self,
        font_key: &str,
        raw: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<ShapedText> {
        let size = height.abs();
        if !size.is_finite() || size <= 0.0 {
            return Err(CadError::InvalidInput(
                "text height must be a positive finite value".into(),
            ));
        }
        if !self.has_usable_face(font_key) {
            return Err(self.chain_error(font_key));
        }

        let parsed = parse_mtext(raw, size);
        let mut issues = parsed.issues.clone();
        if !self.contains(font_key) {
            issues.push(TextFormatIssue::new(
                "text.font_fallback",
                format!(
                    "font '{font_key}' unavailable; rendering with the registered fallback chain"
                ),
            ));
        }

        // Shape each paragraph. A run that names an unregistered font is
        // reported, not silently dropped; the rest of the text still renders.
        let mut missing_font: Option<String> = None;
        let lines = layout_parsed(&parsed, size, |run, text| {
            let key = run.font.as_deref().unwrap_or(font_key);
            match self.measure_run(key, text, run.effective_height(size)) {
                Some(shaped) => Some(shaped),
                None => {
                    missing_font.get_or_insert_with(|| key.to_string());
                    None
                }
            }
        });

        if let Some(key) = missing_font {
            issues.push(TextFormatIssue::new(
                text_issue::FONT_UNAVAILABLE,
                format!("run font '{key}' is not registered and no fallback is available"),
            ));
        }

        let runs: Vec<TextRun> = parsed
            .lines
            .iter()
            .flat_map(|line| line.runs.iter().cloned())
            .collect();
        if runs.iter().any(|run| run.color.is_some()) {
            issues.push(TextFormatIssue::new(
                text_issue::COLOR_NOT_APPLIED,
                "inline MTEXT color is parsed but line geometry carries no color",
            ));
        }
        if runs
            .iter()
            .any(|run| run.underline || run.overline || run.strike)
        {
            issues.push(TextFormatIssue::new(
                text_issue::DECORATION_NOT_APPLIED,
                "MTEXT underline/overline/strikethrough are parsed but not drawn",
            ));
        }

        Ok(ShapedText {
            polylines: finalize_lines(lines, size, origin, rotation, h_align, v_align),
            runs,
            issues: dedupe_issues(issues),
        })
    }

    /// Outline `text` into world-space polylines at `origin`.
    ///
    /// A thin wrapper over [`shape`](Self::shape) that drops the run list and
    /// diagnostics. One polyline per glyph contour; paragraphs in `text` (via
    /// `\P`, a literal `\n` or `^J`) become consecutive lines. `height` is the
    /// base em height in world units and `rotation` is radians about `origin`.
    /// When the requested font is missing, the fallback chain is used, resolved
    /// **per glyph**: each character is drawn with the first face in the chain
    /// (primary, then fallbacks in order) that actually contains it, and the pen
    /// advances by that face's advance. When the primary face has every glyph,
    /// the result is byte-for-byte the same as single-face rendering.
    #[allow(clippy::too_many_arguments)]
    pub fn outline(
        &self,
        font_key: &str,
        text: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<Vec<Vec<Point3>>> {
        Ok(self
            .shape(font_key, text, origin, height, rotation, h_align, v_align)?
            .polylines)
    }

    /// Whether `font_key` or the fallback chain holds any decodable face.
    fn has_usable_face(&self, font_key: &str) -> bool {
        self.chain(font_key)
            .iter()
            .any(|face| matches!(&**face, FaceData::Sfnt(_) | FaceData::Shx(_)))
    }

    fn chain(&self, font_key: &str) -> Vec<Arc<FaceData>> {
        let mut chain: Vec<Arc<FaceData>> = Vec::new();
        if let Some(face) = self.lookup(font_key) {
            chain.push(face);
        }
        for key in &self.fallback {
            if let Some(face) = self.lookup(key) {
                chain.push(face);
            }
        }
        chain
    }

    /// The error to report when [`has_usable_face`](Self::has_usable_face) is
    /// `false`: an explicit `Unsupported` reason if one was recognised, else
    /// `ResourceMissing`. Preserves the pre-MTEXT behaviour.
    fn chain_error(&self, font_key: &str) -> CadError {
        for face in self.chain(font_key) {
            if let FaceData::Unsupported(reason) = &*face {
                return CadError::Unsupported(reason.clone());
            }
        }
        CadError::ResourceMissing(format!(
            "font '{font_key}' is not registered and no fallback is available"
        ))
    }

    /// Shape one run's `text` at `size`, returning run-local glyph polylines
    /// (baseline at y=0, pen starting at x=0) and the unscaled advance.
    ///
    /// Width factor and oblique are applied by [`layout_parsed`], not here, so
    /// the per-glyph fallback/kerning logic stays independent of the MTEXT
    /// style overrides. `None` means no usable face was available.
    fn measure_run(
        &self,
        font_key: &str,
        text: &str,
        size: f64,
    ) -> Option<(Vec<Vec<[f64; 2]>>, f64)> {
        let chain = self.chain(font_key);
        if chain.is_empty() {
            return None;
        }

        // Keep every sfnt `Arc<[u8]>` alive for the whole call so the parsed
        // faces (which borrow the bytes) stay valid.
        let keepalive: Vec<Arc<[u8]>> = chain
            .iter()
            .filter_map(|face| match &**face {
                FaceData::Sfnt(data) => Some(data.clone()),
                _ => None,
            })
            .collect();

        let mut resolved: Vec<ResolvedFace<'_>> = Vec::new();
        let mut sfnt_index = 0usize;
        for face in &chain {
            match &**face {
                FaceData::Unsupported(_) => {}
                FaceData::Sfnt(_) => {
                    let data: &[u8] = &keepalive[sfnt_index];
                    sfnt_index += 1;
                    // Registration already proved parseability; skip defensively
                    // so a bad fallback does not abort a usable chain.
                    if let Ok(parsed) = ttf_parser::Face::parse(data, 0) {
                        let upem = parsed.units_per_em().max(1) as f64;
                        let scale = size / upem;
                        if scale.is_finite() && scale > 0.0 {
                            let kern = parsed.tables().kern;
                            resolved.push(ResolvedFace::Sfnt {
                                face: Box::new(parsed),
                                kern,
                                scale,
                            });
                        }
                    }
                }
                FaceData::Shx(font) => resolved.push(ResolvedFace::Shx(font)),
            }
        }
        if resolved.is_empty() {
            return None;
        }

        // `previous` remembers which face supplied the preceding glyph so that
        // kerning is only applied within a single face (cross-face pairs are
        // meaningless).
        let mut previous: Option<(usize, ttf_parser::GlyphId)> = None;
        let lines = layout_glyphs(text, |ch, pen, first| {
            if first {
                previous = None;
            }
            for (index, candidate) in resolved.iter().enumerate() {
                match candidate {
                    ResolvedFace::Sfnt { face, kern, scale } => {
                        let Some(glyph) = face.glyph_index(ch) else {
                            continue;
                        };
                        let mut kerning = 0.0;
                        if let (Some(table), Some((prev_index, prev))) = (kern.as_ref(), previous) {
                            if prev_index == index {
                                for subtable in table.subtables {
                                    if subtable.horizontal {
                                        if let Some(value) = subtable.glyphs_kerning(prev, glyph) {
                                            kerning = value as f64;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        previous = Some((index, glyph));
                        let advance = face.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale;
                        let shift = kerning * scale;
                        let mut builder = OutlineToPolylines::new(*scale, [pen[0] + shift, pen[1]]);
                        if face.outline_glyph(glyph, &mut builder).is_some() {
                            builder.flush();
                        }
                        return Some((std::mem::take(&mut builder.polys), advance + shift));
                    }
                    ResolvedFace::Shx(font) => {
                        if let Some(glyph) = font.glyph(ch, size) {
                            previous = None;
                            let polys: Vec<Vec<[f64; 2]>> = glyph
                                .polylines
                                .iter()
                                .map(|poly| {
                                    poly.iter()
                                        .map(|p| [p[0] + pen[0], p[1] + pen[1]])
                                        .collect()
                                })
                                .collect();
                            return Some((polys, glyph.advance));
                        }
                    }
                }
            }
            None
        });

        let mut polys = Vec::new();
        let mut advance = 0.0;
        for (line_polys, width) in lines {
            polys.extend(line_polys);
            advance += width;
        }
        Some((polys, advance))
    }
}

/// Parse bytes into a face, detecting SHX, sfnt and WOFF.
fn parse_face(key: &str, bytes: &[u8], encoding: Option<&str>) -> CadResult<FaceData> {
    if bytes.starts_with(b"AutoCAD-") {
        return match ShxFont::parse(bytes, encoding) {
            Ok(font) => Ok(FaceData::Shx(font)),
            // Recognised but undecodable: keep the face so the fallback chain
            // can substitute it, instead of failing registration outright.
            Err(CadError::Unsupported(reason)) => Ok(FaceData::Unsupported(reason)),
            Err(other) => Err(other),
        };
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"wOF2" {
        return Ok(FaceData::Unsupported(
            "WOFF2 fonts are not supported yet".into(),
        ));
    }
    let data = prepare_font(bytes)?;
    ttf_parser::Face::parse(&data, 0)
        .map_err(|e| CadError::CorruptData(format!("font '{key}' cannot be parsed: {e}")))?;
    Ok(FaceData::Sfnt(data))
}

fn normalize_key(raw: &str) -> String {
    let bare = raw.trim().trim_matches('"');
    let bare = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
    bare.to_ascii_lowercase()
}

fn stem_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .filter(|stem| !stem.is_empty())
}

/// Decode WOFF1 to sfnt; pass raw sfnt through.
fn prepare_font(bytes: &[u8]) -> CadResult<Arc<[u8]>> {
    if bytes.len() >= 4 && &bytes[0..4] == b"wOFF" {
        woff_to_sfnt(bytes).map(Arc::from)
    } else {
        Ok(Arc::from(bytes.to_vec()))
    }
}

/// Reconstruct an sfnt (TTF) container from a WOFF1 file.
fn woff_to_sfnt(data: &[u8]) -> CadResult<Vec<u8>> {
    if data.len() < 44 {
        return Err(CadError::CorruptData("WOFF file is truncated".into()));
    }
    let u32be = |o: usize| u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let u16be = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);

    let flavor = u32be(4);
    let num_tables = u16be(12) as usize;
    let dir_start = 44;
    if num_tables == 0 || dir_start + num_tables * 20 > data.len() {
        return Err(CadError::CorruptData(
            "WOFF table directory is invalid".into(),
        ));
    }

    struct Entry {
        tag: [u8; 4],
        offset: usize,
        comp: usize,
        orig: usize,
        checksum: u32,
    }
    let mut entries = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let o = dir_start + index * 20;
        entries.push(Entry {
            tag: [data[o], data[o + 1], data[o + 2], data[o + 3]],
            offset: u32be(o + 4) as usize,
            comp: u32be(o + 8) as usize,
            orig: u32be(o + 12) as usize,
            checksum: u32be(o + 16),
        });
    }
    // sfnt requires the directory sorted by tag.
    entries.sort_by_key(|entry| entry.tag);

    let mut max_power = 0usize;
    while (1usize << (max_power + 1)) <= num_tables {
        max_power += 1;
    }
    let search_range = (1usize << max_power) * 16;
    let entry_selector = max_power;
    let range_shift = num_tables * 16 - search_range;

    let mut header = Vec::with_capacity(12);
    header.extend_from_slice(&flavor.to_be_bytes());
    header.extend_from_slice(&(num_tables as u16).to_be_bytes());
    header.extend_from_slice(&(search_range as u16).to_be_bytes());
    header.extend_from_slice(&(entry_selector as u16).to_be_bytes());
    header.extend_from_slice(&(range_shift as u16).to_be_bytes());

    let mut records: Vec<([u8; 4], u32, u32, u32)> = Vec::with_capacity(num_tables);
    let mut body: Vec<u8> = Vec::new();
    let mut next_offset = 12 + num_tables * 16;
    for entry in &entries {
        if entry.offset + entry.comp > data.len() {
            return Err(CadError::CorruptData("WOFF table extends past EOF".into()));
        }
        let raw = &data[entry.offset..entry.offset + entry.comp];
        let table: Vec<u8> = if entry.comp == entry.orig {
            raw.to_vec()
        } else {
            let mut decoder = flate2::read::ZlibDecoder::new(raw);
            let mut decoded = Vec::with_capacity(entry.orig);
            decoder
                .read_to_end(&mut decoded)
                .map_err(|e| CadError::CorruptData(format!("WOFF table decompress failed: {e}")))?;
            if decoded.len() != entry.orig {
                return Err(CadError::CorruptData(
                    "WOFF table decompressed to the wrong size".into(),
                ));
            }
            decoded
        };
        while next_offset % 4 != 0 {
            next_offset += 1;
            body.push(0);
        }
        records.push((
            entry.tag,
            entry.checksum,
            next_offset as u32,
            entry.orig as u32,
        ));
        body.extend_from_slice(&table);
        next_offset += table.len();
    }

    let mut out = header;
    for (tag, checksum, offset, length) in records {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

/// Flatten a glyph outline into polylines in glyph-local, scaled coordinates.
struct OutlineToPolylines {
    scale: f64,
    pen: [f64; 2],
    polys: Vec<Vec<[f64; 2]>>,
    current: Vec<[f64; 2]>,
    last: [f64; 2],
}

impl OutlineToPolylines {
    fn new(scale: f64, pen: [f64; 2]) -> Self {
        OutlineToPolylines {
            scale,
            pen,
            polys: Vec::new(),
            current: Vec::new(),
            last: [0.0, 0.0],
        }
    }

    fn emit(&mut self, x: f64, y: f64) {
        self.current
            .push([x * self.scale + self.pen[0], y * self.scale + self.pen[1]]);
        self.last = [x, y];
    }

    fn flush(&mut self) {
        if self.current.len() >= 2 {
            self.polys.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }

    fn quad(&mut self, c: (f64, f64), end: (f64, f64)) {
        let start = self.last;
        const STEPS: usize = 6;
        for step in 1..=STEPS {
            let t = step as f64 / STEPS as f64;
            let mt = 1.0 - t;
            let x = mt * mt * start[0] + 2.0 * mt * t * c.0 + t * t * end.0;
            let y = mt * mt * start[1] + 2.0 * mt * t * c.1 + t * t * end.1;
            self.emit(x, y);
        }
    }

    fn cubic(&mut self, c1: (f64, f64), c2: (f64, f64), end: (f64, f64)) {
        let start = self.last;
        const STEPS: usize = 10;
        for step in 1..=STEPS {
            let t = step as f64 / STEPS as f64;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            let x = a * start[0] + b * c1.0 + c * c2.0 + d * end.0;
            let y = a * start[1] + b * c1.1 + c * c2.1 + d * end.1;
            self.emit(x, y);
        }
    }
}

impl OutlineBuilder for OutlineToPolylines {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.emit(x as f64, y as f64);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.emit(x as f64, y as f64);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.quad((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.cubic(
            (x1 as f64, y1 as f64),
            (x2 as f64, y2 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        if let Some(first) = self.current.first().copied() {
            if self.current.last().copied() != Some(first) {
                self.current.push(first);
            }
        }
        self.flush();
    }
}

/// Lay out each `\n`-separated line: place glyphs at the pen and record the
/// line's advance width. The glyph callback receives the pen position and
/// returns glyph-local polylines plus its advance.
fn layout_glyphs<F>(text: &str, mut glyph: F) -> Vec<(Vec<Vec<[f64; 2]>>, f64)>
where
    F: FnMut(char, [f64; 2], bool) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut pen = [0.0f64, 0.0];
        let mut polys = Vec::new();
        let mut first = true;
        for ch in line.chars() {
            if let Some((glyph_polys, advance)) = glyph(ch, pen, first) {
                polys.extend(glyph_polys);
                pen[0] += advance;
            }
            first = false;
        }
        lines.push((polys, pen[0]));
    }
    lines
}

/// Default baseline-to-baseline spacing as a multiple of the line's height.
pub const LINE_SPACING: f64 = 1.2;

/// One laid-out paragraph before alignment/rotation.
#[derive(Clone)]
struct ShapedLine {
    /// Glyph-local polylines with the pen already applied, width factor and
    /// oblique shear included, baseline at y=0.
    polys: Vec<Vec<[f64; 2]>>,
    /// Horizontal advance of the line in world units.
    width: f64,
    /// Height of the tallest run on the line (drives line spacing).
    height: f64,
}

/// Lay out parsed paragraphs run by run, applying per-run width factor and
/// oblique shear.
///
/// `measure(run, text)` returns the run's glyph polylines positioned with a
/// run-local pen (baseline at y=0) and the *unscaled* advance; the width factor
/// and oblique shear are applied here so the math is testable without a font.
fn layout_parsed<F>(parsed: &ParsedText, fallback_height: f64, mut measure: F) -> Vec<ShapedLine>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let fallback = if fallback_height.is_finite() && fallback_height > 0.0 {
        fallback_height
    } else {
        1.0
    };
    let mut out = Vec::new();
    for line in &parsed.lines {
        let mut pen = 0.0f64;
        let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut line_height = fallback;
        for run in &line.runs {
            let text = run.rendered_text();
            if text.is_empty() {
                continue;
            }
            let height = run.effective_height(fallback);
            line_height = line_height.max(height);
            // Guard against non-finite/negative style values rather than
            // emitting NaN geometry.
            let width_factor = if run.width_factor.is_finite() && run.width_factor > 0.0 {
                run.width_factor
            } else {
                1.0
            };
            let oblique = if run.oblique_deg.is_finite() {
                run.oblique_deg.to_radians().tan()
            } else {
                0.0
            };
            let Some((glyph_polys, advance)) = measure(run, &text) else {
                continue;
            };
            for poly in glyph_polys {
                let mapped: Vec<[f64; 2]> = poly
                    .iter()
                    .map(|[gx, gy]| [pen + gx * width_factor + gy * oblique, *gy])
                    .collect();
                polys.push(mapped);
            }
            if advance.is_finite() {
                pen += advance * width_factor;
            }
        }
        out.push(ShapedLine {
            polys,
            width: pen,
            height: line_height,
        });
    }
    out
}

/// Apply per-line horizontal alignment, vertical block placement, rotation and
/// translation.
///
/// Horizontal alignment is applied per line (so a multi-line block stays
/// flush against its anchor); vertical placement uses the accumulated line
/// heights with [`LINE_SPACING`]. Vertical alignment of a multi-line block is a
/// documented approximation (see the module docs).
fn finalize_lines(
    lines: Vec<ShapedLine>,
    size: f64,
    origin: Point3,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
) -> Vec<Vec<Point3>> {
    let vertical_shift = match v_align {
        TextAlignV::Baseline => 0.0,
        TextAlignV::Bottom => 0.2 * size,
        TextAlignV::Middle => 0.5 * size,
        TextAlignV::Top => size,
    };
    let (sin, cos) = rotation.sin_cos();
    let mut out = Vec::new();
    let mut baseline = 0.0f64;
    for line in lines {
        let dx = match h_align {
            TextAlignH::Left => 0.0,
            TextAlignH::Center => -line.width / 2.0,
            TextAlignH::Right => -line.width,
        };
        let line_y = baseline + vertical_shift;
        for poly in line.polys {
            if poly.len() < 2 {
                continue;
            }
            let points: Vec<Point3> = poly
                .iter()
                .map(|p| {
                    let lx = p[0] + dx;
                    let ly = p[1] + line_y;
                    Point3 {
                        x: origin.x + cos * lx - sin * ly,
                        y: origin.y + sin * lx + cos * ly,
                        z: origin.z,
                    }
                })
                .collect();
            out.push(points);
        }
        baseline -= LINE_SPACING * line.height;
    }
    out
}

/// Drop repeated issue codes, keeping the first message for each.
fn dedupe_issues(issues: Vec<TextFormatIssue>) -> Vec<TextFormatIssue> {
    let mut out: Vec<TextFormatIssue> = Vec::new();
    for issue in issues {
        if !out.iter().any(|existing| existing.code == issue.code) {
            out.push(issue);
        }
    }
    out
}

/// Spaces a tab expands to.
///
/// Tabs are rendered as a fixed run of spaces rather than snapping the pen to
/// proportional tab stops, so `\t`/`^I` spacing is an explicit approximation
/// (see the module-level limitations).
const TAB_STEP: usize = 4;

/// Consume a `\X...;` parameter code whose value begins at `start` (the index
/// just after the code letter), returning the index just past the terminating
/// `;` — or the end of input when no `;` is present.
fn skip_param_code(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && chars[j] != ';' {
        j += 1;
    }
    if j < chars.len() {
        j + 1
    } else {
        j
    }
}

/// Parse MTEXT/TEXT control content into a structured run list.
///
/// `base_height` is the entity's nominal em height in world units; `\H` values
/// are resolved against it (absolute heights are used as-is, relative factors
/// multiply the current height). The result always has at least one paragraph.
///
/// Malformed or unrecognised escapes never panic: an unrecognised `\X` is
/// rendered literally and reported through [`TextFormatIssue`], while a
/// recognised code with a bad value is consumed and reported.
pub fn parse_mtext(raw: &str, base_height: f64) -> ParsedText {
    MTextParser::new(raw, base_height).run()
}

/// Active span style while parsing; `{`/`}` push and pop a clone of this.
#[derive(Clone)]
struct ParseState {
    font: Option<String>,
    /// Absolute height override from a non-relative `\H`.
    height_abs: Option<f64>,
    /// Relative height multiplier (on the base height, or on `height_abs`).
    height_mul: f64,
    width_factor: f64,
    oblique_deg: f64,
    color: Option<TextColor>,
    line_align: Option<u8>,
    underline: bool,
    overline: bool,
    strike: bool,
}

impl ParseState {
    fn new() -> Self {
        ParseState {
            font: None,
            height_abs: None,
            height_mul: 1.0,
            width_factor: 1.0,
            oblique_deg: 0.0,
            color: None,
            line_align: None,
            underline: false,
            overline: false,
            strike: false,
        }
    }
}

struct MTextParser {
    chars: Vec<char>,
    pos: usize,
    base_height: f64,
    stack: Vec<ParseState>,
    current: ParseState,
    lines: Vec<TextLine>,
    runs: Vec<TextRun>,
    buf: String,
    issues: Vec<TextFormatIssue>,
}

impl MTextParser {
    fn new(raw: &str, base_height: f64) -> Self {
        let base = if base_height.is_finite() && base_height > 0.0 {
            base_height
        } else {
            1.0
        };
        MTextParser {
            chars: raw.chars().collect(),
            pos: 0,
            base_height: base,
            stack: Vec::new(),
            current: ParseState::new(),
            lines: Vec::new(),
            runs: Vec::new(),
            buf: String::new(),
            issues: Vec::new(),
        }
    }

    fn run(mut self) -> ParsedText {
        self.parse_all();
        ParsedText {
            lines: self.lines,
            issues: dedupe_issues(self.issues),
        }
    }

    fn issue(&mut self, code: &'static str, message: impl Into<String>) {
        self.issues.push(TextFormatIssue::new(code, message));
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn parse_all(&mut self) {
        while self.pos < self.chars.len() {
            let ch = self.chars[self.pos];
            match ch {
                '\\' => {
                    self.flush_run();
                    self.parse_code();
                }
                '{' => {
                    self.stack.push(self.current.clone());
                    self.pos += 1;
                }
                '}' => {
                    self.flush_run();
                    if let Some(previous) = self.stack.pop() {
                        self.current = previous;
                    }
                    self.pos += 1;
                }
                '\n' => {
                    self.pos += 1;
                    self.paragraph();
                }
                '%' => self.special_char(),
                '^' => self.caret(),
                c if (c as u32) < 0x20 => {
                    // Any other control character renders as a space.
                    self.buf.push(' ');
                    self.pos += 1;
                }
                _ => {
                    self.buf.push(ch);
                    self.pos += 1;
                }
            }
        }
        self.flush_run();
        // Always terminate the current paragraph, even when empty.
        self.lines.push(TextLine {
            runs: std::mem::take(&mut self.runs),
        });
    }

    fn paragraph(&mut self) {
        self.flush_run();
        self.lines.push(TextLine {
            runs: std::mem::take(&mut self.runs),
        });
    }

    fn flush_run(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.buf);
        let run = self.make_run(text, None);
        self.push_run(run);
    }

    fn push_run(&mut self, run: TextRun) {
        if run.fraction.is_none() {
            if let Some(last) = self.runs.last_mut() {
                if last.fraction.is_none() && last.same_style(&run) {
                    last.text.push_str(&run.text);
                    return;
                }
            }
        }
        self.runs.push(run);
    }

    fn make_run(&self, text: String, fraction: Option<StackedFraction>) -> TextRun {
        let height = self
            .current
            .height_abs
            .unwrap_or(self.base_height * self.current.height_mul);
        TextRun {
            text,
            fraction,
            font: self.current.font.clone(),
            height: if height.is_finite() && height > 0.0 {
                height
            } else {
                self.base_height
            },
            width_factor: self.current.width_factor,
            oblique_deg: self.current.oblique_deg,
            color: self.current.color,
            line_align: self.current.line_align,
            underline: self.current.underline,
            overline: self.current.overline,
            strike: self.current.strike,
        }
    }

    /// Read a `;`-terminated value from the current position. Returns the raw
    /// value and whether the terminating `;` was present.
    fn read_value(&mut self) -> (String, bool) {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && self.chars[end] != ';' {
            end += 1;
        }
        let value: String = self.chars[start..end].iter().collect();
        let terminated = end < self.chars.len();
        self.pos = if terminated { end + 1 } else { end };
        (value, terminated)
    }

    fn parse_code(&mut self) {
        let next = self.pos + 1;
        if next >= self.chars.len() {
            self.buf.push('\\');
            self.issue(text_issue::MALFORMED_CODE, "trailing '\\' with no code");
            self.pos += 1;
            return;
        }
        let code = self.chars[next];
        self.pos = next + 1;
        match code {
            '\\' => self.buf.push('\\'),
            '{' => self.buf.push('{'),
            '}' => self.buf.push('}'),
            ';' => self.buf.push(';'),
            '~' => self.buf.push('\u{00A0}'),
            'P' => self.paragraph(),
            'p' => self.paragraph_code(),
            'N' => {
                self.issue(
                    text_issue::COLUMN_BREAK,
                    "column break '\\N' treated as a plain line break",
                );
                self.paragraph();
            }
            'X' => {}
            'L' => self.current.underline = true,
            'l' => self.current.underline = false,
            'O' => self.current.overline = true,
            'o' => self.current.overline = false,
            'K' => self.current.strike = true,
            'k' => self.current.strike = false,
            'C' => {
                let (value, terminated) = self.read_value();
                self.apply_aci(&value, terminated);
                self.consume_optional_aci_end();
            }
            'c' => {
                let (value, terminated) = self.read_value();
                self.apply_rgb(&value, terminated);
            }
            'f' => {
                let (value, terminated) = self.read_value();
                self.apply_font(&value, false, terminated);
            }
            'F' => {
                let (value, terminated) = self.read_value();
                self.apply_font(&value, true, terminated);
            }
            'H' => {
                let (value, terminated) = self.read_value();
                self.apply_height(&value, terminated);
            }
            'W' | 'w' => {
                let (value, terminated) = self.read_value();
                self.apply_width(&value, terminated);
            }
            'Q' => {
                let (value, terminated) = self.read_value();
                self.apply_oblique(&value, terminated);
            }
            'A' => {
                let (value, terminated) = self.read_value();
                self.apply_align(&value, terminated);
            }
            'S' | 's' => {
                let (value, terminated) = self.read_value();
                self.apply_fraction(&value, terminated);
            }
            'T' => {
                let (_, terminated) = self.read_value();
                self.issue(
                    text_issue::TRACKING_NOT_APPLIED,
                    "character tracking '\\T' parsed but not applied",
                );
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\T code is missing its terminating ';'",
                    );
                }
            }
            'B' => {
                let _ = self.read_value();
                self.issue(
                    text_issue::BACKGROUND_MASK,
                    "background mask '\\B' parsed but not drawn",
                );
            }
            'b' => {
                let (value, terminated) = self.read_value();
                match value.trim() {
                    "1" => self.current.strike = true,
                    "0" => self.current.strike = false,
                    _ => self.issue(
                        text_issue::MALFORMED_CODE,
                        format!("\\b value '{value}' is not 0 or 1"),
                    ),
                }
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\b code is missing its terminating ';'",
                    );
                }
            }
            'U' if self.peek() == Some('+') => self.unicode_escape(),
            _ => {
                self.buf.push('\\');
                self.buf.push(code);
                self.issue(
                    text_issue::UNKNOWN_CODE,
                    format!("unrecognised MTEXT code '\\{code}' rendered literally"),
                );
            }
        }
    }

    /// `\p`: a bare `\p`/`\p;` is a paragraph break; `\p...;` holds paragraph
    /// properties this build does not lay out.
    fn paragraph_code(&mut self) {
        match self.peek() {
            None | Some(';') => {
                if self.peek() == Some(';') {
                    self.pos += 1;
                }
                self.paragraph();
            }
            Some(_) => {
                let (_, terminated) = self.read_value();
                self.issue(
                    text_issue::PARAGRAPH_PROPERTIES,
                    "\\p paragraph properties (indent/alignment/tabs) ignored",
                );
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\p code is missing its terminating ';'",
                    );
                }
            }
        }
    }

    fn special_char(&mut self) {
        let first = self.pos;
        if self.chars.get(first + 1) != Some(&'%') {
            self.buf.push('%');
            self.pos = first + 1;
            return;
        }
        let code_index = first + 2;
        if self.chars.get(code_index) == Some(&'%') && self.chars.get(code_index + 1) == Some(&'%')
        {
            self.buf.push('%');
            self.pos = code_index + 2;
            return;
        }
        if let Some(&code) = self.chars.get(code_index) {
            match code.to_ascii_lowercase() {
                'd' => {
                    self.buf.push('°');
                    self.pos = code_index + 1;
                    return;
                }
                'p' => {
                    self.buf.push('±');
                    self.pos = code_index + 1;
                    return;
                }
                'c' => {
                    self.buf.push('⌀');
                    self.pos = code_index + 1;
                    return;
                }
                _ => {}
            }
            if code.is_ascii_digit() {
                let mut end = code_index;
                while end < self.chars.len() && self.chars[end].is_ascii_digit() {
                    end += 1;
                }
                let digits: String = self.chars[code_index..end].iter().collect();
                if let Ok(value) = digits.parse::<u32>() {
                    if let Some(ch) = char::from_u32(value) {
                        self.buf.push(ch);
                        self.pos = end;
                        return;
                    }
                }
            }
        }
        // Unknown `%%x` — emit both percent signs literally and let the code
        // character fall through as normal text.
        self.buf.push('%');
        self.buf.push('%');
        self.pos = first + 2;
    }

    fn caret(&mut self) {
        match self.chars.get(self.pos + 1).copied() {
            Some('I') => {
                for _ in 0..TAB_STEP {
                    self.buf.push(' ');
                }
                self.pos += 2;
            }
            Some('J') => {
                self.pos += 2;
                self.paragraph();
            }
            Some('M') => self.pos += 2,
            _ => {
                self.buf.push('^');
                self.pos += 1;
            }
        }
    }

    fn unicode_escape(&mut self) {
        // `self.pos` is at the '+' following `\U`.
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_hexdigit() {
            self.pos += 1;
        }
        let hex: String = self.chars[start..self.pos].iter().collect();
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(ch) => self.buf.push(ch),
            None => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\U+{hex} is not a valid code point"),
            ),
        }
    }

    fn apply_height(&mut self, value: &str, terminated: bool) {
        let (number, relative) = split_relative(value);
        match number.trim().parse::<f64>() {
            Ok(factor) if factor.is_finite() => {
                let factor = factor.abs();
                if relative {
                    if let Some(absolute) = self.current.height_abs {
                        self.current.height_abs = Some(absolute * factor);
                    } else {
                        self.current.height_mul *= factor;
                    }
                } else {
                    self.current.height_abs = Some(factor);
                }
            }
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\H value '{value}' is not a number"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\H code is missing its terminating ';'",
            );
        }
    }

    fn apply_width(&mut self, value: &str, terminated: bool) {
        let (number, relative) = split_relative(value);
        match number.trim().parse::<f64>() {
            Ok(factor) if factor.is_finite() => {
                let factor = factor.abs();
                if relative {
                    self.current.width_factor *= factor;
                } else {
                    self.current.width_factor = factor;
                }
            }
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\W value '{value}' is not a number"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\W code is missing its terminating ';'",
            );
        }
    }

    fn apply_oblique(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<f64>() {
            Ok(angle) if angle.is_finite() => self.current.oblique_deg = angle,
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\Q value '{value}' is not an angle"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\Q code is missing its terminating ';'",
            );
        }
    }

    fn apply_align(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<u8>() {
            Ok(code) if code <= 2 => self.current.line_align = Some(code),
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\A value '{value}' is not 0, 1 or 2"),
            ),
        }
        self.issue(
            text_issue::LINE_ALIGNMENT,
            "\\A run alignment parsed but not applied",
        );
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\A code is missing its terminating ';'",
            );
        }
    }

    fn apply_font(&mut self, value: &str, is_f: bool, terminated: bool) {
        let mut spec = value.trim();
        if is_f {
            // `\FN{name}.shx;` selects a compiled shape font.
            if let Some(rest) = spec.strip_prefix('N') {
                spec = rest;
            }
        }
        let name = spec.split(['|', ',']).next().unwrap_or("").trim();
        if !name.is_empty() {
            self.current.font = Some(name.to_string());
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "font code is missing its terminating ';'",
            );
        }
    }

    fn apply_aci(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<i64>() {
            Ok(index) if (0..=256).contains(&index) => {
                self.current.color = Some(TextColor::Aci(index as u16));
            }
            Ok(index) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("ACI color {index} is out of range 0..=256"),
            ),
            Err(_) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\C value '{value}' is not an ACI index"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\C code is missing its terminating ';'",
            );
        }
    }

    /// Consume an optional second (`;`-terminated) ACI value from `\C1;2;`,
    /// which this build does not use (no gradient rendering).
    fn consume_optional_aci_end(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && (self.chars[end].is_ascii_digit() || self.chars[end] == '-')
        {
            end += 1;
        }
        if end > start && self.chars.get(end) == Some(&';') {
            self.pos = end + 1;
        }
    }

    fn apply_rgb(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<u32>() {
            Ok(packed) => {
                // MTEXT true color is byte-reversed: low byte is red.
                let red = (packed & 0xFF) as u8;
                let green = ((packed >> 8) & 0xFF) as u8;
                let blue = ((packed >> 16) & 0xFF) as u8;
                self.current.color = Some(TextColor::Rgb([red, green, blue]));
            }
            Err(_) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\c value '{value}' is not packed RGB"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\c code is missing its terminating ';'",
            );
        }
    }

    fn apply_fraction(&mut self, value: &str, terminated: bool) {
        match split_fraction(value) {
            Some((numerator, denominator, separator)) => {
                let run = self.make_run(
                    String::new(),
                    Some(StackedFraction {
                        numerator,
                        denominator,
                        separator,
                    }),
                );
                self.runs.push(run);
                self.issue(
                    text_issue::STACKED_FRACTION_FLAT,
                    "stacked fraction drawn inline as 'num<sep>den' (no bar/stacking)",
                );
            }
            None => {
                self.issue(
                    text_issue::MALFORMED_CODE,
                    format!("\\S value '{value}' has no '/', '#' or '^' separator"),
                );
                self.buf.push_str(value);
            }
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\S code is missing its terminating ';'",
            );
        }
    }
}

/// Split a `\H`/`\W` value into its number and whether it was relative (`x`).
fn split_relative(value: &str) -> (&str, bool) {
    if value.ends_with('x') || value.ends_with('X') {
        (&value[..value.len() - 1], true)
    } else {
        (value, false)
    }
}

/// Split a `\S` payload at its first `/`, `#` or `^` separator.
fn split_fraction(value: &str) -> Option<(String, String, char)> {
    for (index, ch) in value.char_indices() {
        if matches!(ch, '/' | '#' | '^') {
            let numerator = value[..index].to_string();
            let mut denominator = &value[index + ch.len_utf8()..];
            if ch == '^' {
                denominator = denominator.strip_prefix(' ').unwrap_or(denominator);
            }
            return Some((numerator, denominator.to_string(), ch));
        }
    }
    None
}

/// Strip MTEXT/TEXT formatting so the raw glyph text can be shaped.
///
/// Handles:
///
/// - `\P`/`\p` line breaks, `\~` non-breaking space and `\\` escapes;
/// - `%%d`/`%%p`/`%%c` symbols and brace grouping;
/// - `\S<num><sep><den>;` stacked fractions, rendered as two lines (see below);
/// - `\t` and `^I` tabs, expanded to `TAB_STEP` spaces;
/// - no-argument toggles `\L`/`\l` (underline), `\O`/`\o` (overline) and
///   `\K`/`\k` (strikethrough), which affect styling we do not render;
/// - parameter codes `\H`, `\W`, `\A`, `\Q`, `\C`, `\f`, `\F`, … terminated by
///   `;`, consumed without emitting text.
///
/// Unknown escapes are dropped. This is an approximation, not a full MTEXT
/// layout engine: stacked fractions use the normal line spacing with no
/// fraction bar or vertical scaling, tabs do not align to true tab stops, and
/// MTEXT columns are ignored.
pub fn sanitize_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // DXF caret notation: `^I` is a TAB control character.
        if c == '^' && i + 1 < chars.len() && matches!(chars[i + 1], 'I' | 'i') {
            for _ in 0..TAB_STEP {
                out.push(' ');
            }
            i += 2;
            continue;
        }
        if c == '\\' {
            if i + 1 >= chars.len() {
                i += 1;
                continue;
            }
            let n = chars[i + 1];
            match n {
                'P' | 'p' => {
                    out.push('\n');
                    i += 2;
                }
                '~' => {
                    out.push(' ');
                    i += 2;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                't' => {
                    // `\t` is a tab; expand to a fixed run of spaces.
                    for _ in 0..TAB_STEP {
                        out.push(' ');
                    }
                    i += 2;
                }
                'S' => {
                    i = push_stacked_fraction(&chars, i, &mut out);
                }
                // No-argument style toggles. These must not scan for `;`, or
                // they would swallow the rest of the run.
                'L' | 'l' | 'O' | 'o' | 'K' | 'k' => {
                    i += 2;
                }
                // Parameter codes: `\H<val>;`, `\W<val>;`, `\A<val>;`,
                // `\Q<val>;`, `\C<val>;`, `\f<name|…>;`, etc. Consume the value
                // and its terminating `;` without emitting text.
                'H' | 'W' | 'A' | 'Q' | 'C' | 'f' | 'F' => {
                    i = skip_param_code(&chars, i + 2);
                }
                _ if n.is_ascii_alphabetic() => {
                    i = skip_param_code(&chars, i + 2);
                }
                _ => i += 2,
            }
            continue;
        }
        if c == '{' || c == '}' {
            i += 1;
            continue;
        }
        if c == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            match chars[i + 2].to_ascii_lowercase() {
                'd' => {
                    out.push('°');
                    i += 3;
                    continue;
                }
                'p' => {
                    out.push('±');
                    i += 3;
                    continue;
                }
                'c' => {
                    out.push('⌀');
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Render an MTEXT stacked fraction starting at the `\` of `\S`.
///
/// The separator is the first of `^` (centre), `/` (horizontal bar) or `#`
/// (diagonal). All three are rendered the same way here: numerator and
/// denominator on consecutive lines, in the order written, using the normal
/// line spacing. There is no fraction bar, no vertical scaling and no reduced
/// font size — a deliberate, documented approximation. Returns the index just
/// past the consumed `\S…;` code.
fn push_stacked_fraction(chars: &[char], backslash: usize, out: &mut String) -> usize {
    let end = skip_param_code(chars, backslash + 2);
    let content_end = if end > backslash + 2 && chars.get(end - 1) == Some(&';') {
        end - 1
    } else {
        end
    };
    let content: String = chars[backslash + 2..content_end].iter().collect();
    match content
        .char_indices()
        .find(|(_, ch)| matches!(ch, '^' | '/' | '#'))
    {
        Some((idx, sep)) => {
            out.push_str(&sanitize_text(&content[..idx]));
            out.push('\n');
            out.push_str(&sanitize_text(&content[idx + sep.len_utf8()..]));
        }
        None => out.push_str(&sanitize_text(&content)),
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic SHAPES font (no third-party bytes embedded).
    fn synthetic_shapes_font() -> Arc<[u8]> {
        let info = b"Synthetic\0\x78\x14\x02\0";
        let glyph = b"H\0\x01\x08\0\x78\0";
        let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        for value in [0u16, 72, 2, 0, info.len() as u16, 72, glyph.len() as u16] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(info);
        bytes.extend(glyph);
        Arc::from(bytes.into_boxed_slice())
    }

    #[test]
    fn shape_glyph_outlines_an_shx_shape_at_its_code() {
        let mut engine = FontEngine::new();
        engine
            .register("ltypeshp.shx", synthetic_shapes_font())
            .unwrap();
        let polylines = engine
            .shape_glyph(
                "ltypeshp.shx",
                72,
                Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                },
                14.0,
                0.0,
            )
            .unwrap();
        assert!(!polylines.is_empty());
        assert!(polylines.iter().all(|p| p.len() >= 2));
        // The glyph is translated to the entity position.
        assert!(polylines
            .iter()
            .flatten()
            .any(|p| (p.x - 1.0).abs() > 1e-9 || (p.y - 2.0).abs() > 1e-9));
        assert!(engine
            .shape_glyph("ltypeshp.shx", 999, Point3::default(), 14.0, 0.0)
            .is_err());
        assert!(engine
            .shape_glyph("missing", 72, Point3::default(), 14.0, 0.0)
            .is_err());
    }

    #[test]
    fn sanitize_handles_mtext_and_text_codes() {
        assert_eq!(sanitize_text("a\\Pb"), "a\nb");
        assert_eq!(sanitize_text("{\\fArial|b0;Hi}"), "Hi");
        assert_eq!(sanitize_text("45%%d"), "45°");
        assert_eq!(sanitize_text("plain"), "plain");
        assert_eq!(sanitize_text("a\\~b"), "a b");
    }

    #[test]
    fn sanitize_stacks_fractions_on_two_lines() {
        // `^`, `/` and `#` all render as numerator over denominator.
        assert_eq!(sanitize_text("\\S1/2;"), "1\n2");
        assert_eq!(sanitize_text("x\\Sa^b;y"), "xa\nby");
        assert_eq!(sanitize_text("\\S10#3;"), "10\n3");
        // No separator: the payload is emitted verbatim (sans the code).
        assert_eq!(sanitize_text("\\Sabc;"), "abc");
        // Missing terminator is tolerated.
        assert_eq!(sanitize_text("\\S1/2"), "1\n2");
    }

    #[test]
    fn sanitize_expands_tabs() {
        let four = " ".repeat(TAB_STEP);
        assert_eq!(sanitize_text("a\\tb"), format!("a{four}b"));
        assert_eq!(sanitize_text("a^Ib"), format!("a{four}b"));
        // A caret that is not `^I` is left alone.
        assert_eq!(sanitize_text("a^b"), "a^b");
    }

    #[test]
    fn sanitize_consumes_style_toggles_without_swallowing_text() {
        // No-argument toggles must consume exactly two characters.
        assert_eq!(sanitize_text("\\Lunder\\l"), "under");
        assert_eq!(sanitize_text("\\Oover\\o"), "over");
        assert_eq!(sanitize_text("\\Kstrike\\k"), "strike");
        // Regression: `\L` must not scan the rest of the run for a `;`.
        assert_eq!(sanitize_text("a\\Lb"), "ab");
    }

    #[test]
    fn sanitize_consumes_parameter_codes() {
        assert_eq!(sanitize_text("\\H2.5;Hi"), "Hi");
        assert_eq!(sanitize_text("\\W0.8;Hi"), "Hi");
        assert_eq!(sanitize_text("\\A1;Hi"), "Hi");
        assert_eq!(sanitize_text("\\Q45;Hi"), "Hi");
        assert_eq!(sanitize_text("\\C1;Hi"), "Hi");
        assert_eq!(sanitize_text("{\\fArial|b0;Hi}"), "Hi");
        // Codes may follow one another without corrupting the text.
        assert_eq!(sanitize_text("\\H1;\\W2;\\C3;ok"), "ok");
    }

    #[test]
    fn unregistered_font_is_reported_not_faked() {
        let engine = FontEngine::new();
        let err = engine
            .outline(
                "arial.ttf",
                "hi",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap_err();
        assert!(matches!(err, CadError::ResourceMissing(_)));
        assert!(engine.is_empty());
    }

    #[test]
    fn garbage_font_bytes_are_rejected() {
        let mut engine = FontEngine::new();
        assert!(engine
            .register("bad.ttf", Arc::from(vec![0u8; 128]))
            .is_err());
        assert!(engine.is_empty());
    }

    #[test]
    fn woff2_is_explicitly_unsupported() {
        let mut engine = FontEngine::new();
        // Registration keeps the face so it can be reported; outlining it fails.
        engine
            .register("x.woff2", Arc::from(b"wOF2....".to_vec()))
            .unwrap();
        let err = engine
            .outline(
                "x.woff2",
                "hi",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
    }

    #[test]
    fn missing_font_uses_the_fallback_chain() {
        // No real face registered: outline must not invent glyphs.
        let engine = FontEngine::new();
        assert!(matches!(
            engine
                .outline(
                    "arial.ttf",
                    "hi",
                    Point3::default(),
                    2.0,
                    0.0,
                    TextAlignH::Left,
                    TextAlignV::Baseline
                )
                .unwrap_err(),
            CadError::ResourceMissing(_)
        ));
    }

    #[test]
    fn unsupported_primary_falls_through_to_fallback() {
        let mut engine = FontEngine::new();
        engine
            .register("bad.woff2", Arc::from(b"wOF2....".to_vec()))
            .unwrap();
        // A usable autumn font is not present here, so the chain exposes the
        // unsupported reason rather than a missing-font error.
        engine.set_fallback(vec!["also-missing".into()]);
        let err = engine
            .outline(
                "bad.woff2",
                "x",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
    }

    /// Opt-in glyph test: run with `YACR_TEST_FONT=/path/to/font.woff`.
    ///
    /// Proves WOFF decoding, key stemming and outline generation against a real
    /// face without committing font binaries (see `docs/fonts.md`).
    #[test]
    fn outlines_a_real_font_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_FONT") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let mut engine = FontEngine::new();
        // Registered as `arial.woff`; the run asks for `arial.ttf`, which must
        // resolve through the shared file stem.
        engine
            .register("arial.woff", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline(
                "arial.ttf",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        assert!(!polys.is_empty(), "no glyph outlines produced");
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        for poly in &polys {
            assert!(poly.len() >= 2);
            for p in poly {
                assert!(p.x.is_finite() && p.y.is_finite());
                min_x = min_x.min(p.x);
                max_x = max_x.max(p.x);
            }
        }
        // Two glyphs at height 10 stay within a modest horizontal span.
        assert!(min_x >= -1.0 && max_x <= 30.0, "span {min_x}..{max_x}");
    }

    /// Opt-in SHX test: run with `YACR_TEST_SHX=/path/to/txt.shx`.
    #[test]
    fn outlines_a_real_shx_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_SHX") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let mut engine = FontEngine::new();
        engine
            .register("txt.shx", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline(
                "txt.shx",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        assert!(!polys.is_empty(), "no SHX glyph outlines");
        let max_x = polys
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x > 0.0, "SHX advance produced no horizontal extent");
    }

    /// Opt-in fallback test: a missing primary font uses a registered fallback.
    #[test]
    fn missing_primary_uses_registered_fallback_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_SHX") else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap();
        let mut engine = FontEngine::new();
        engine.set_fallback(vec!["fallback.shx".into()]);
        engine
            .register("fallback.shx", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline(
                "not-installed.shx",
                "A",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        assert!(!polys.is_empty(), "fallback did not render");
    }

    /// Outline a run with the standard test parameters.
    fn outline_run(engine: &FontEngine, key: &str, text: &str) -> Vec<Vec<Point3>> {
        engine
            .outline(
                key,
                text,
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap()
    }

    fn max_x(polys: &[Vec<Point3>]) -> f64 {
        polys
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Opt-in per-glyph fallback test (needs both `YACR_TEST_FONT` and
    /// `YACR_TEST_SHX`).
    ///
    /// The SHX face is the primary and the sfnt face the fallback, so this also
    /// exercises the branch where the sfnt face is only consulted for glyphs the
    /// primary lacks.
    #[test]
    fn per_glyph_fallback_uses_first_face_with_the_glyph() {
        let (Ok(sfnt_path), Ok(shx_path)) = (
            std::env::var("YACR_TEST_FONT"),
            std::env::var("YACR_TEST_SHX"),
        ) else {
            return;
        };
        let sfnt_bytes = std::fs::read(sfnt_path).unwrap();
        let shx_bytes = std::fs::read(shx_path).unwrap();

        let mut sfnt_only = FontEngine::new();
        sfnt_only
            .register(
                "arial.woff",
                Arc::from(sfnt_bytes.clone().into_boxed_slice()),
            )
            .unwrap();
        let mut shx_only = FontEngine::new();
        shx_only
            .register(
                "simplex.shx",
                Arc::from(shx_bytes.clone().into_boxed_slice()),
            )
            .unwrap();

        // Find a probe glyph the SHX primary lacks but the sfnt fallback has.
        let probe = ['Ω', 'Ж', 'é', '→', '∑']
            .into_iter()
            .find(|p| {
                let text = p.to_string();
                !outline_run(&sfnt_only, "arial.woff", &text).is_empty()
                    && outline_run(&shx_only, "simplex.shx", &text).is_empty()
            })
            .expect("no probe glyph shared by the supplied test fonts");

        let mut mixed = FontEngine::new();
        mixed
            .register("simplex.shx", Arc::from(shx_bytes.into_boxed_slice()))
            .unwrap();
        mixed
            .register("arial.woff", Arc::from(sfnt_bytes.into_boxed_slice()))
            .unwrap();
        mixed.set_fallback(vec!["arial.woff".into()]);

        // The glyph missing from the primary is supplied by the fallback.
        let probe_text = probe.to_string();
        let fallback_polys = outline_run(&mixed, "simplex.shx", &probe_text);
        assert!(
            !fallback_polys.is_empty(),
            "fallback did not supply {probe:?}"
        );

        // A glyph the primary has still renders exactly as the primary alone.
        let primary_only = outline_run(&shx_only, "simplex.shx", "A");
        let primary_mixed = outline_run(&mixed, "simplex.shx", "A");
        assert_eq!(primary_only, primary_mixed);

        // A run mixing both sources advances past either source alone.
        let mixed_run = outline_run(&mixed, "simplex.shx", &format!("A{probe}"));
        assert!(max_x(&mixed_run) > max_x(&primary_mixed));
        assert!(max_x(&mixed_run) > max_x(&fallback_polys));
    }

    #[test]
    fn alignment_shifts_the_run_relative_to_its_anchor() {
        let lines = vec![ShapedLine {
            polys: vec![vec![[0.0, 0.0], [10.0, 0.0]]],
            width: 10.0,
            height: 2.0,
        }];
        let left = finalize_lines(
            lines.clone(),
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
        );
        let center = finalize_lines(
            lines.clone(),
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Center,
            TextAlignV::Baseline,
        );
        let right = finalize_lines(
            lines,
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Right,
            TextAlignV::Baseline,
        );
        assert_eq!(left[0][0].x, 0.0);
        assert_eq!(center[0][0].x, -5.0);
        assert_eq!(right[0][0].x, -10.0);
    }

    // ---- MTEXT formatting ----

    /// Deterministic font-independent glyph metrics: each non-space character
    /// is a rectangle `0.5 * height` wide and `height` tall, advancing by its
    /// width. Lets the layout tests assert advances/shear without a font.
    fn mock_measure(run: &TextRun, text: &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)> {
        let height = run.effective_height(1.0);
        let mut polys = Vec::new();
        let mut x = 0.0f64;
        for ch in text.chars() {
            let width = if ch == ' ' {
                0.4 * height
            } else {
                0.5 * height
            };
            polys.push(vec![
                [x, 0.0],
                [x + width, 0.0],
                [x + width, height],
                [x, height],
            ]);
            x += width;
        }
        Some((polys, x))
    }

    fn line_text(line: &TextLine) -> String {
        line.runs.iter().map(TextRun::rendered_text).collect()
    }

    #[test]
    fn parse_mtext_keeps_grouping_height_and_color() {
        let parsed = parse_mtext("{\\fArial|b0;Hello}\\P{\\H2x;Big} \\C1;red", 10.0);
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(line_text(&parsed.lines[0]), "Hello");
        assert!(parsed.lines[0]
            .runs
            .iter()
            .any(|run| run.font.as_deref() == Some("Arial")));
        assert_eq!(line_text(&parsed.lines[1]), "Big red");
        // The grouped `\H2x;` sets 2x the base height for that run only.
        let big = parsed.lines[1]
            .runs
            .iter()
            .find(|run| run.text == "Big")
            .expect("Big run");
        assert!((big.height - 20.0).abs() < 1e-9, "height {}", big.height);
        let red = parsed.lines[1]
            .runs
            .iter()
            .find(|run| run.text == "red")
            .expect("red run");
        assert_eq!(red.color, Some(TextColor::Aci(1)));
        assert!((red.height - 10.0).abs() < 1e-9);
    }

    #[test]
    fn parse_mtext_combines_absolute_and_relative_height() {
        let parsed = parse_mtext("\\H5;A\\H2x;B", 10.0);
        let runs = &parsed.lines[0].runs;
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "A");
        assert!((runs[0].height - 5.0).abs() < 1e-9);
        // A relative factor compounds the active absolute height: 5 * 2.
        assert_eq!(runs[1].text, "B");
        assert!((runs[1].height - 10.0).abs() < 1e-9);
    }

    #[test]
    fn parse_mtext_keeps_true_color_and_aci_range() {
        let parsed = parse_mtext("\\c255;R\\C256;S", 10.0);
        assert_eq!(
            parsed.lines[0].runs[0].color,
            Some(TextColor::Rgb([255, 0, 0]))
        );
        assert_eq!(parsed.lines[0].runs[1].color, Some(TextColor::Aci(256)));
    }

    #[test]
    fn parse_mtext_handles_escapes_specials_and_cjk() {
        // `%%%%` is a literal percent; `%%%` leaves one percent and the code
        // char as literal text.
        let parsed = parse_mtext("a\\~b\\{c\\}d%%d%%p%%c%%%%", 10.0);
        assert_eq!(line_text(&parsed.lines[0]), "a\u{00A0}b{c}d°±⌀%");
        // CJK code points are kept whole; run splitting is by style, not bytes.
        let cjk = parse_mtext("中\\H20;文", 10.0);
        let runs = &cjk.lines[0].runs;
        assert_eq!(runs[0].text, "中");
        assert_eq!(runs[1].text, "文");
        assert!((runs[1].height - 20.0).abs() < 1e-9);
    }

    #[test]
    fn parse_mtext_reports_malformed_and_unknown_codes() {
        // Missing terminator/value: the code is consumed, not misframed.
        let malformed = parse_mtext("a\\Hb", 10.0);
        assert_eq!(line_text(&malformed.lines[0]), "a");
        assert!(malformed
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::MALFORMED_CODE));
        // Unknown codes degrade to literal text with a diagnostic.
        let unknown = parse_mtext("a\\Zx", 10.0);
        assert_eq!(line_text(&unknown.lines[0]), "a\\Zx");
        assert!(unknown
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::UNKNOWN_CODE));
        // A trailing backslash is literal, never a panic.
        let dangling = parse_mtext("a\\", 10.0);
        assert_eq!(line_text(&dangling.lines[0]), "a\\");
        assert!(dangling
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::MALFORMED_CODE));
    }

    #[test]
    fn parse_mtext_keeps_stacked_fractions_structured_but_flat() {
        let parsed = parse_mtext("x\\S1/2;y", 10.0);
        assert_eq!(line_text(&parsed.lines[0]), "x1/2y");
        let fraction = parsed.lines[0]
            .runs
            .iter()
            .find_map(|run| run.fraction.as_ref())
            .expect("fraction run");
        assert_eq!(fraction.numerator, "1");
        assert_eq!(fraction.denominator, "2");
        assert_eq!(fraction.separator, '/');
        assert!(parsed
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::STACKED_FRACTION_FLAT));
    }

    #[test]
    fn layout_applies_width_factor_and_oblique_to_advances_and_glyphs() {
        // Width factor scales both the glyph x and the line advance.
        let narrow = parse_mtext("\\W0.5;AB", 10.0);
        let lines = layout_parsed(&narrow, 10.0, mock_measure);
        assert!(
            (lines[0].width - 5.0).abs() < 1e-9,
            "width {}",
            lines[0].width
        );
        let max_x = lines[0]
            .polys
            .iter()
            .flatten()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((max_x - 5.0).abs() < 1e-9, "max_x {max_x}");

        // Oblique shears glyph x by y*tan(angle); the baseline stays put.
        let oblique = parse_mtext("\\Q45;A", 10.0);
        let lines = layout_parsed(&oblique, 10.0, mock_measure);
        let max_x = lines[0]
            .polys
            .iter()
            .flatten()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        // Glyph is 0.5*10 = 5 wide, 10 tall; top shears by 10 * tan(45) = 10.
        assert!((max_x - 15.0).abs() < 1e-9, "max_x {max_x}");
        let baseline_x = lines[0]
            .polys
            .iter()
            .flatten()
            .filter(|p| p[1].abs() < 1e-9)
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((baseline_x - 5.0).abs() < 1e-9, "baseline_x {baseline_x}");
    }

    #[test]
    fn layout_counts_lines_and_advances_them_by_line_spacing() {
        let parsed = parse_mtext("AB\\PCDE", 10.0);
        let lines = layout_parsed(&parsed, 10.0, mock_measure);
        assert_eq!(lines.len(), 2);
        assert!((lines[0].width - 10.0).abs() < 1e-9, "{}", lines[0].width);
        assert!((lines[1].width - 15.0).abs() < 1e-9, "{}", lines[1].width);

        let out = finalize_lines(
            lines,
            10.0,
            Point3::default(),
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
        );
        let (min_y, max_y) = out
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.y), hi.max(p.y))
            });
        // Line 0 sits in [0, 10]; line 1 is shifted down by 1.2 * 10.
        assert!((max_y - 10.0).abs() < 1e-9, "max_y {max_y}");
        assert!((min_y + 12.0).abs() < 1e-9, "min_y {min_y}");
    }

    #[test]
    fn layout_applies_per_run_height_to_line_spacing() {
        // A taller run on line 0 pushes line 1 down by the taller height.
        let parsed = parse_mtext("{\\H2x;Big}\\Psmall", 10.0);
        let lines = layout_parsed(&parsed, 10.0, mock_measure);
        assert!((lines[0].height - 20.0).abs() < 1e-9);
        assert!((lines[1].height - 10.0).abs() < 1e-9);
        let out = finalize_lines(
            lines,
            10.0,
            Point3::default(),
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
        );
        let min_y = out
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        assert!((min_y + 24.0).abs() < 1e-9, "min_y {min_y}");
    }

    /// Opt-in MTEXT integration: run with `YACR_TEST_FONT=/path/to/font.woff`.
    #[test]
    fn shapes_mtext_runs_and_reports_approximations_when_font_provided() {
        let Ok(path) = std::env::var("YACR_TEST_FONT") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let mut engine = FontEngine::new();
        engine
            .register("arial.woff", Arc::from(bytes.into_boxed_slice()))
            .unwrap();

        let shaped = engine
            .shape(
                "arial.ttf",
                "AB\\P{\\H2x;CD}\\C1;E\\S1/2;",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        // Two paragraphs plus a stacked-fraction run.
        assert!(shaped.runs.iter().any(|run| run.text == "CD"));
        assert!(shaped.runs.iter().any(|run| run.fraction.is_some()));
        assert_eq!(
            shaped
                .runs
                .iter()
                .find(|run| run.text == "CD")
                .unwrap()
                .height,
            20.0
        );
        assert!(shaped
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::COLOR_NOT_APPLIED));
        assert!(shaped
            .issues
            .iter()
            .any(|issue| issue.code == text_issue::STACKED_FRACTION_FLAT));
        assert!(!shaped.polylines.is_empty());
        // The 2x run must be drawn taller than a 1x run.
        let tall = shaped
            .polylines
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let plain = engine
            .outline(
                "arial.ttf",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        let plain_top = plain
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(tall > plain_top, "tall {tall} plain {plain_top}");
    }
}
