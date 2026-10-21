//! Width-boundary (word wrap) layout contracts.
//!
//! `SemanticGeometry::Text::wrap_width` (DXF MLEADER group 43 equivalent) is
//! applied here, without a font: the mock measure gives every non-space
//! character an advance of `0.5 * height` and every space `0.4 * height`, so the
//! line widths are exactly predictable.

use cad_domain::{Point3, TextAlignH, TextAlignV};

use super::finalize::finalize_lines;
use super::layout::{layout_parsed, wrapped_texts, ShapedLine, WRAP_EPSILON};
use super::mtext::parse_mtext;
use super::tests::mock_measure;

const HEIGHT: f64 = 10.0;

fn shape(text: &str, wrap_width: Option<f64>) -> Vec<ShapedLine> {
    let parsed = parse_mtext(text, HEIGHT);
    layout_parsed(&parsed, HEIGHT, wrap_width, mock_measure)
}

#[test]
fn wrap_none_keeps_the_written_lines_and_the_full_advance() {
    // Pinned pre-wrap output: one `\P` paragraph is one line, and its width is
    // the full advance including the spaces.
    let parsed = parse_mtext("the quick brown fox", HEIGHT);
    let lines = layout_parsed(&parsed, HEIGHT, None, mock_measure);
    assert_eq!(lines.len(), 1, "no boundary => no reflow");
    assert!((lines[0].width - 92.0).abs() < 1e-9, "{}", lines[0].width);
    // 15 characters (12 letters + 3 spaces), one glyph rectangle each.
    assert_eq!(lines[0].polys.len(), 19);

    // A non-positive or non-finite boundary sanitizes to "no wrap", so it must
    // produce exactly the same line set.
    for boundary in [0.0, -4.0, f64::NAN, f64::INFINITY] {
        let same = layout_parsed(&parsed, HEIGHT, Some(boundary), mock_measure);
        assert_eq!(same.len(), 1, "boundary {boundary} must not wrap");
        assert!((same[0].width - 92.0).abs() < 1e-9, "{boundary}");
    }
}

#[test]
fn wrap_splits_a_long_paragraph_and_preserves_every_character() {
    let text = "the quick brown fox jumps over the lazy dog";
    let parsed = parse_mtext(text, HEIGHT);
    let lines = layout_parsed(&parsed, HEIGHT, Some(30.0), mock_measure);
    assert!(lines.len() > 1, "a 43-character line must wrap at 30");
    for line in &lines {
        assert!(
            line.width <= 30.0 + WRAP_EPSILON,
            "line {} exceeds the boundary",
            line.width
        );
    }
    let texts = wrapped_texts(&parsed, HEIGHT, 30.0, mock_measure);
    assert_eq!(texts.len(), lines.len());
    assert_eq!(
        texts.concat(),
        text,
        "wrapping must not lose or duplicate a character"
    );
    // Word boundaries: no line starts or ends in the middle of a word.
    for line in &texts {
        let trimmed = line.trim();
        assert!(
            text.split(' ').any(|word| word == trimmed),
            "line {line:?} is not a whole word"
        );
    }
}

#[test]
fn wrap_breaks_paragraphs_independently_and_keeps_their_order() {
    let parsed = parse_mtext("alpha beta\\Pgamma delta", HEIGHT);
    let lines = layout_parsed(&parsed, HEIGHT, Some(25.0), mock_measure);
    let texts = wrapped_texts(&parsed, HEIGHT, 25.0, mock_measure);
    assert_eq!(
        texts,
        vec![
            "alpha ".to_string(),
            "beta".to_string(),
            "gamma ".to_string(),
            "delta".to_string()
        ],
        "each paragraph wraps on its own and order is preserved"
    );
    assert_eq!(lines.len(), 4);
    // The paragraph boundary is never merged: `beta` does not continue `gamma`.
    assert_eq!(lines[1].polys.len(), 4);
    assert_eq!(lines[3].polys.len(), 5);
}

#[test]
fn wrap_emits_an_unbreakable_token_as_one_over_long_line() {
    let text = "aaaaaaaaaaaaaaaa";
    let lines = shape(text, Some(10.0));
    assert_eq!(lines.len(), 1, "an unbreakable token is never split");
    assert!(
        (lines[0].width - 80.0).abs() < 1e-9,
        "the whole token is kept: {}",
        lines[0].width
    );
    let texts = wrapped_texts(&parse_mtext(text, HEIGHT), HEIGHT, 10.0, mock_measure);
    assert_eq!(texts, vec![text.to_string()]);
}

#[test]
fn wrap_breaks_cjk_between_characters() {
    let text = "中文测试文本";
    let parsed = parse_mtext(text, HEIGHT);
    let lines = layout_parsed(&parsed, HEIGHT, Some(12.0), mock_measure);
    assert_eq!(lines.len(), 3, "6 ideographs, two per 12-unit line");
    for line in &lines {
        assert!(line.width <= 12.0 + WRAP_EPSILON, "{}", line.width);
    }
    let texts = wrapped_texts(&parsed, HEIGHT, 12.0, mock_measure);
    assert!(texts.iter().all(|line| line.chars().count() == 2));
    assert_eq!(texts.concat(), text);
}

#[test]
fn wrap_centres_each_wrapped_line() {
    // Widths 20, 14, 20: a short line and a long line must sit at different
    // x origins under center alignment, or alignment was applied to the whole
    // paragraph instead of per wrapped line.
    let parsed = parse_mtext("aaaa bb cccc", HEIGHT);
    let lines = layout_parsed(&parsed, HEIGHT, Some(25.0), mock_measure);
    let widths: Vec<f64> = lines.iter().map(|line| line.width).collect();
    assert_eq!(widths.len(), 3, "{widths:?}");
    let out = finalize_lines(
        lines,
        HEIGHT,
        Point3::default(),
        0.0,
        TextAlignH::Center,
        TextAlignV::Baseline,
    );
    // Each line's glyphs share one baseline; group the output by it.
    let mut groups: Vec<(f64, f64)> = Vec::new();
    for poly in &out {
        let baseline = poly[0].y;
        let min_x = poly.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        match groups.last_mut() {
            Some((y, min_x_so_far)) if (*y - baseline).abs() < 1e-9 => {
                *min_x_so_far = min_x_so_far.min(min_x);
            }
            _ => groups.push((baseline, min_x)),
        }
    }
    assert_eq!(groups.len(), widths.len());
    for (index, (_, min_x)) in groups.iter().enumerate() {
        let expected = -widths[index] / 2.0;
        assert!(
            (min_x - expected).abs() < 1e-9,
            "line {index}: x origin {min_x} should be {expected}"
        );
    }
    assert!(
        (groups[0].1 - groups[1].1).abs() > 1e-9,
        "a short and a long wrapped line must not share an x origin"
    );
}
