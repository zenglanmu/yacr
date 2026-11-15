//! The plain-text projection used by search/replace (`FIND`).
//!
//! [`mtext_projection`] must be the **drawn** text of the same tokenizer that
//! renders the value — not a second parser — with every character traced back to
//! the markup bytes that produced it, so a caller can edit the markup instead of
//! rewriting the whole stored string.

use super::*;

#[test]
fn projection_is_the_drawn_text_with_markup_origins() {
    let raw = "{\\fArial|b1;Bold} plain\\Psecond\\~x%%d\\S1/2;";
    let projection = mtext_projection(raw);
    // Cross-check against the rendering parse, not against a second parser.
    let drawn = parse_mtext(raw, 10.0)
        .lines
        .iter()
        .map(|line| {
            line.runs
                .iter()
                .map(TextRun::rendered_text)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(projection.text(), drawn);
    assert_eq!(projection.text(), "Bold plain\nsecond\u{00A0}x°1/2");
    for projected in &projection.chars {
        assert!(
            projected.start < projected.end && projected.end <= raw.len(),
            "{projected:?}"
        );
        assert!(
            raw.is_char_boundary(projected.start) && raw.is_char_boundary(projected.end),
            "{projected:?}"
        );
    }
    let source = |ch: char| {
        let projected = projection
            .chars
            .iter()
            .find(|projected| projected.ch == ch)
            .unwrap_or_else(|| panic!("no projected {ch:?}"));
        raw[projected.start..projected.end].to_string()
    };
    // A literal character maps to itself, a code to its own bytes.
    assert_eq!(source('B'), "B");
    assert_eq!(source('\n'), "\\P");
    assert_eq!(source('\u{00A0}'), "\\~");
    assert_eq!(source('°'), "%%d");
    // A `\S` fraction draws `num<sep>den`; all of it maps to the whole code.
    assert_eq!(source('/'), "\\S1/2;");
}

/// A code that draws several characters (`^I` draws four spaces) owns one byte
/// range shared by all of them: they can only be edited as a unit. The origin
/// says whether a partial rewrite would drop a structure the grammar encodes.
#[test]
fn projection_shares_one_range_for_a_multi_character_code() {
    let raw = "a^Ib";
    let projection = mtext_projection(raw);
    assert_eq!(projection.text(), "a    b");
    let tabs: Vec<_> = projection
        .chars
        .iter()
        .filter(|projected| projected.ch == ' ')
        .collect();
    assert_eq!(tabs.len(), super::mtext::TAB_STEP);
    for projected in tabs {
        assert_eq!(&raw[projected.start..projected.end], "^I");
        assert_eq!(projected.origin, ProjectedOrigin::Tab);
    }
    // Ordinary text and a literal code are `Literal`: rewriting them as text
    // loses nothing.
    assert_eq!(projection.chars[0].origin, ProjectedOrigin::Literal);
    assert_eq!(projection.chars[5].origin, ProjectedOrigin::Literal);
    let unknown = mtext_projection("a\\Zx");
    assert_eq!(unknown.text(), "a\\Zx");
    assert!(unknown
        .chars
        .iter()
        .all(|projected| projected.origin == ProjectedOrigin::Literal));
    // A `\S` fraction marks every drawn character it produces.
    let fraction = mtext_projection("x\\S1/2;y");
    assert_eq!(fraction.text(), "x1/2y");
    for projected in &fraction.chars[1..4] {
        assert_eq!(projected.origin, ProjectedOrigin::Fraction);
        assert_eq!(&"x\\S1/2;y"[projected.start..projected.end], "\\S1/2;");
    }
    assert_eq!(fraction.chars[0].origin, ProjectedOrigin::Literal);
    assert_eq!(fraction.chars[4].origin, ProjectedOrigin::Literal);
}
