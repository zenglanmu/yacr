use std::ops::Range;

use super::model::{ParsedText, TextLine, TextRun};

/// Default baseline-to-baseline spacing as a multiple of the line's height.
pub const LINE_SPACING: f64 = 1.2;

/// Absolute slack, in world units, when deciding whether a wrap unit still fits
/// inside a width boundary.
///
/// It only absorbs floating-point summation error in the measured advances; it
/// is not a layout allowance, so a line may exceed the boundary by at most this
/// much.
pub const WRAP_EPSILON: f64 = 1e-9;

/// One laid-out paragraph before alignment/rotation.
#[derive(Clone)]
pub(super) struct ShapedLine {
    /// Glyph-local polylines with the pen already applied, width factor and
    /// oblique shear included, baseline at y=0.
    pub(super) polys: Vec<Vec<[f64; 2]>>,
    /// Horizontal advance of the line in world units.
    pub(super) width: f64,
    /// Height of the tallest run on the line (drives line spacing).
    pub(super) height: f64,
}

pub(super) struct MeasuredRun {
    pub(super) geometry: (Vec<Vec<[f64; 2]>>, f64),
    pub(super) missing_glyphs: Vec<char>,
}

/// Lay out parsed paragraphs run by run, applying per-run width factor and
/// oblique shear.
///
/// `measure(run, text)` returns the run's glyph polylines positioned with a
/// run-local pen (baseline at y=0) and the *unscaled* advance; the width factor
/// and oblique shear are applied here so the math is testable without a font.
///
/// `wrap_width` is an optional width boundary in world units (DXF MLEADER group
/// 43 / MTEXT reference-rectangle width). `None` lays every `\P` paragraph out
/// as one line, exactly as written; `Some(w)` greedily wraps each paragraph
/// independently at word boundaries (and between ideographs, which have no
/// spaces to break at), so a line never exceeds `w` except for a single
/// unbreakable token or the [`WRAP_EPSILON`] slack. Each wrapped line is its own
/// [`ShapedLine`], so horizontal alignment is applied per wrapped line.
pub(super) fn layout_parsed<F>(
    parsed: &ParsedText,
    fallback_height: f64,
    wrap_width: Option<f64>,
    mut measure: F,
) -> Vec<ShapedLine>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let fallback = if fallback_height.is_finite() && fallback_height > 0.0 {
        fallback_height
    } else {
        1.0
    };
    let wrap = wrap_width.filter(|width| width.is_finite() && *width > 0.0);
    let mut out = Vec::new();
    for line in &parsed.lines {
        match wrap {
            None => out.push(layout_line(line, fallback, &mut measure)),
            Some(width) => out.extend(layout_wrapped_line(line, fallback, width, &mut measure)),
        }
    }
    out
}

/// Lay one paragraph out as a single line (the unwrapped path).
fn layout_line<F>(line: &TextLine, fallback: f64, measure: &mut F) -> ShapedLine
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
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
        let Some((glyph_polys, advance)) = measure(run, &text) else {
            continue;
        };
        for poly in glyph_polys {
            polys.push(map_poly(poly, pen, run));
        }
        if advance.is_finite() {
            pen += advance * width_factor(run);
        }
    }
    ShapedLine {
        polys,
        width: pen,
        height: line_height,
    }
}

/// A run's horizontal width factor, guarded against non-finite/negative values
/// so a bad style value never emits NaN geometry.
fn width_factor(run: &TextRun) -> f64 {
    if run.width_factor.is_finite() && run.width_factor > 0.0 {
        run.width_factor
    } else {
        1.0
    }
}

/// Map one glyph-local polyline into the line's pen space.
fn map_poly(poly: Vec<[f64; 2]>, pen: f64, run: &TextRun) -> Vec<[f64; 2]> {
    let width_factor = width_factor(run);
    let oblique = if run.oblique_deg.is_finite() {
        run.oblique_deg.to_radians().tan()
    } else {
        0.0
    };
    poly.iter()
        .map(|[gx, gy]| [pen + gx * width_factor + gy * oblique, *gy])
        .collect()
}

/// One break opportunity of a paragraph: a word, a whitespace run attached to
/// the preceding word, or a single ideograph.
struct WrapToken {
    /// Index into the paragraph's `runs` (finds the measurement style).
    run_index: usize,
    text: String,
    /// Whether a line break may start at this token. Style continuations of one
    /// word (a mid-word run change) are glued to the previous token.
    break_before: bool,
    /// A single ideograph: never extended by the following characters.
    isolated: bool,
}

/// A token measured through the caller's `measure` closure.
struct MeasuredToken {
    /// Glyph-local polylines, baseline at y=0, pen starting at x=0.
    polys: Vec<Vec<[f64; 2]>>,
    /// Advance including trailing whitespace.
    advance: f64,
    /// Advance excluding trailing whitespace: the visible (ink) width a line
    /// keeps for alignment and for the fit decision.
    ink_advance: f64,
    height: f64,
    /// Whether the token holds any non-whitespace character.
    has_content: bool,
}

/// Split one paragraph into wrap tokens.
///
/// Latin words stay whole (they break only where a space precedes them);
/// ideographs are single-character tokens so a run without spaces can wrap at
/// character boundaries. Whitespace attaches to the preceding token, so a line
/// break never drops a character.
fn wrap_tokens(line: &TextLine) -> Vec<WrapToken> {
    let mut tokens: Vec<WrapToken> = Vec::new();
    let mut last_was_space = false;
    for (run_index, run) in line.runs.iter().enumerate() {
        let rendered = run.rendered_text();
        for ch in rendered.chars() {
            if ch.is_whitespace() {
                match tokens.last_mut() {
                    Some(last) if last.run_index == run_index && !last.isolated => {
                        last.text.push(ch);
                    }
                    _ => tokens.push(WrapToken {
                        run_index,
                        text: ch.to_string(),
                        break_before: true,
                        isolated: false,
                    }),
                }
                last_was_space = true;
                continue;
            }
            if is_breakable_ideograph(ch) {
                tokens.push(WrapToken {
                    run_index,
                    text: ch.to_string(),
                    break_before: true,
                    isolated: true,
                });
                last_was_space = false;
                continue;
            }
            let extend = matches!(
                tokens.last(),
                Some(last)
                    if last.run_index == run_index && !last.isolated && !last_was_space
            );
            if extend {
                if let Some(last) = tokens.last_mut() {
                    last.text.push(ch);
                }
            } else {
                // A break is possible after whitespace, after an ideograph, and
                // at the paragraph start — but not in the middle of a word that
                // a style change split across runs.
                let break_before = tokens.is_empty()
                    || last_was_space
                    || tokens.last().is_some_and(|last| last.isolated);
                tokens.push(WrapToken {
                    run_index,
                    text: ch.to_string(),
                    break_before,
                    isolated: false,
                });
            }
            last_was_space = false;
        }
    }
    tokens
}

/// Whether `ch` is an ideographic/full-width character that may be wrapped
/// between characters (CJK, kana, Hangul).
///
/// This is a script-range heuristic, not a Unicode line-breaking implementation:
/// kinsoku rules (no line start after an opening bracket, no line end before a
/// closing bracket) are not applied. Declared as an approximation.
fn is_breakable_ideograph(ch: char) -> bool {
    let code = ch as u32;
    matches!(code,
        0x1100..=0x11FF     // Hangul Jamo
        | 0x2E80..=0x2FFF   // CJK radicals, Kangxi, ideographic description
        | 0x3000..=0x303F   // CJK symbols and punctuation
        | 0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF   // CJK unified ideographs extension A
        | 0x4E00..=0x9FFF   // CJK unified ideographs
        | 0xA000..=0xA4CF   // Yi
        | 0xAC00..=0xD7AF   // Hangul syllables
        | 0xF900..=0xFAFF   // CJK compatibility ideographs
        | 0xFE30..=0xFE4F   // CJK compatibility forms
        | 0xFF00..=0xFFEF   // Half-width and full-width forms
        | 0x20000..=0x3FFFF // CJK extensions B and beyond
    )
}

/// Lay one paragraph out as one or more wrapped lines.
fn layout_wrapped_line<F>(
    line: &TextLine,
    fallback: f64,
    wrap_width: f64,
    measure: &mut F,
) -> Vec<ShapedLine>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let tokens = wrap_tokens(line);
    if tokens.is_empty() {
        return vec![ShapedLine {
            polys: Vec::new(),
            width: 0.0,
            height: fallback,
        }];
    }
    let measured = measure_tokens(line, &tokens, fallback, measure);
    wrap_ranges(&tokens, &measured, wrap_width)
        .into_iter()
        .map(|range| build_line(line, &tokens[range.clone()], &measured[range], fallback))
        .collect()
}

/// Measure every token once through the caller's closure, applying the run's
/// width factor and recording the advance with and without trailing whitespace.
fn measure_tokens<F>(
    line: &TextLine,
    tokens: &[WrapToken],
    fallback: f64,
    measure: &mut F,
) -> Vec<MeasuredToken>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    tokens
        .iter()
        .map(|token| {
            let run = &line.runs[token.run_index];
            let height = run.effective_height(fallback);
            let factor = width_factor(run);
            let has_content = token.text.chars().any(|ch| !ch.is_whitespace());
            let (polys, advance) = match measure(run, &token.text) {
                Some((polys, advance)) => (polys, advance),
                // An unshaped token contributes no geometry and no advance, like
                // an unshaped run in the unwrapped path.
                None => (Vec::new(), 0.0),
            };
            let advance = if advance.is_finite() {
                advance * factor
            } else {
                0.0
            };
            let trimmed = token.text.trim_end();
            let ink_advance = if trimmed.len() == token.text.len() {
                advance
            } else {
                match measure(run, trimmed) {
                    Some((_, trimmed_advance)) if trimmed_advance.is_finite() => {
                        trimmed_advance * factor
                    }
                    _ => advance,
                }
            };
            MeasuredToken {
                polys,
                advance,
                ink_advance,
                height,
                has_content,
            }
        })
        .collect()
}

/// Greedily split measured tokens into line ranges.
///
/// Trailing whitespace does not have to fit (it is invisible at a line end).
/// A token that cannot fit and has no break opportunity is glued to the current
/// line rather than dropped; a token that is itself wider than the boundary ends
/// up alone on an over-long line. Every token belongs to exactly one range, so
/// no character is lost or duplicated and the loop always terminates.
fn wrap_ranges(
    tokens: &[WrapToken],
    measured: &[MeasuredToken],
    wrap_width: f64,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut pen = 0.0f64;
    let mut line_has_content = false;
    for (index, token) in tokens.iter().enumerate() {
        let item = &measured[index];
        if index == start {
            pen = item.advance;
            line_has_content = item.has_content;
            continue;
        }
        // Never break before a line's first visible character: leading spaces
        // belong to the first word.
        let fits = pen + item.ink_advance <= wrap_width + WRAP_EPSILON;
        if !line_has_content || fits || !token.break_before {
            pen += item.advance;
            line_has_content |= item.has_content;
            continue;
        }
        ranges.push(start..index);
        start = index;
        pen = item.advance;
        line_has_content = item.has_content;
    }
    ranges.push(start..tokens.len());
    ranges
}

/// Build one shaped line from a contiguous group of measured tokens.
fn build_line(
    line: &TextLine,
    tokens: &[WrapToken],
    measured: &[MeasuredToken],
    fallback: f64,
) -> ShapedLine {
    let mut pen = 0.0f64;
    let mut polys = Vec::new();
    let mut height = fallback;
    for (offset, item) in measured.iter().enumerate() {
        let run = &line.runs[tokens[offset].run_index];
        height = height.max(item.height);
        for poly in &item.polys {
            polys.push(map_poly(poly.clone(), pen, run));
        }
        pen += item.advance;
    }
    // A line's visible width ignores the trailing whitespace of its last token:
    // those glyphs are invisible and must not shift center/right alignment.
    let width = match measured.last() {
        Some(last) => (pen - last.advance + last.ink_advance).max(0.0),
        None => 0.0,
    };
    ShapedLine {
        polys,
        width,
        height,
    }
}

#[cfg(test)]
pub(super) fn wrapped_texts<F>(
    parsed: &ParsedText,
    fallback: f64,
    wrap_width: f64,
    mut measure: F,
) -> Vec<String>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let mut out = Vec::new();
    for line in &parsed.lines {
        let tokens = wrap_tokens(line);
        if tokens.is_empty() {
            out.push(String::new());
            continue;
        }
        let measured = measure_tokens(line, &tokens, fallback, &mut measure);
        out.extend(
            wrap_ranges(&tokens, &measured, wrap_width)
                .into_iter()
                .map(|range| {
                    tokens[range]
                        .iter()
                        .map(|token| token.text.as_str())
                        .collect()
                }),
        );
    }
    out
}
