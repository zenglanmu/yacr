//! Contract tests for the hand-transcribed predefined HATCH pattern table.
//!
//! Pins the table's shape (uppercase unique names, finite non-degenerate rows),
//! the 1-based index / trimmed-name resolvers (no clamping, no fallback), the
//! family count and directions of `ANSI31`/`ANSI37`, and that
//! `to_hatch_pattern` composes through [`HatchPattern::families`] exactly like
//! the `.pat` row formula (rotated row offset, scaled base/offset/dashes).

use super::*;

fn unit(x: f64, y: f64) -> [f64; 2] {
    let length = x.hypot(y);
    [x / length, y / length]
}

#[test]
fn every_predefined_pattern_has_an_uppercase_unique_name_and_a_description() {
    let patterns = predefined_patterns();
    assert_eq!(
        patterns.len(),
        3,
        "the implemented set is exactly the three"
    );
    assert_eq!(
        patterns
            .iter()
            .map(|pattern| pattern.name)
            .collect::<Vec<_>>(),
        vec!["ANSI31", "ANSI32", "ANSI37"]
    );
    let mut seen = std::collections::BTreeSet::new();
    for pattern in patterns {
        assert!(
            seen.insert(pattern.name),
            "duplicate pattern name {}",
            pattern.name
        );
        assert_eq!(
            pattern.name,
            pattern.name.to_ascii_uppercase(),
            "the pattern name is a stable uppercase identity: {}",
            pattern.name
        );
        assert!(!pattern.name.trim().is_empty());
        assert!(
            !pattern.description.trim().is_empty(),
            "{} must describe its family",
            pattern.name
        );
        assert!(
            !pattern.lines.is_empty(),
            "{} must carry at least one definition row",
            pattern.name
        );
    }
}

#[test]
fn every_definition_row_is_finite_with_a_non_degenerate_offset() {
    for pattern in predefined_patterns() {
        for (index, line) in pattern.lines.iter().enumerate() {
            let label = format!("{} row {index}", pattern.name);
            assert!(line.angle_deg.is_finite(), "{label}: non-finite angle");
            assert!(line.base[0].is_finite(), "{label}: non-finite base x");
            assert!(line.base[1].is_finite(), "{label}: non-finite base y");
            assert!(line.offset[0].is_finite(), "{label}: non-finite offset x");
            assert!(line.offset[1].is_finite(), "{label}: non-finite offset y");
            // A zero offset would stack every family line on one line (and the
            // clip sampler skips a zero-length step), i.e. the pattern would
            // render nothing: refused as data, not silently accepted.
            let offset_length = line.offset[0].hypot(line.offset[1]);
            assert!(
                offset_length > 1e-9,
                "{label}: degenerate offset (length {offset_length})"
            );
            for dash in line.dashes {
                assert!(dash.is_finite(), "{label}: non-finite dash {dash}");
                assert!(*dash != 0.0, "{label}: a zero dash length has no meaning");
            }
        }
    }
}

#[test]
fn the_name_resolver_is_trimmed_case_insensitive_and_never_falls_back() {
    for (index, pattern) in predefined_patterns().iter().enumerate() {
        assert_eq!(
            predefined_pattern_by_index(index + 1).map(|found| found.name),
            Some(pattern.name),
            "the index is 1-based"
        );
        assert_eq!(
            predefined_pattern(pattern.name).map(|found| found.name),
            Some(pattern.name)
        );
        assert_eq!(
            predefined_pattern(&format!("  {}  ", pattern.name.to_ascii_lowercase()))
                .map(|found| found.name),
            Some(pattern.name),
            "a trimmed, case-insensitive name resolves"
        );
    }
    // Index 0 is not a pattern and neither is one past the end: no clamping.
    assert_eq!(predefined_pattern_by_index(0), None);
    assert_eq!(predefined_pattern_by_index(usize::MAX), None);
    assert_eq!(
        predefined_pattern_by_index(predefined_patterns().len()),
        Some(&predefined_patterns()[predefined_patterns().len() - 1]),
        "the last 1-based index is the last table entry"
    );
    assert_eq!(
        predefined_pattern_by_index(predefined_patterns().len() + 1),
        None
    );
    // Unknown / empty names are refused, never mapped to the first pattern.
    assert_eq!(predefined_pattern("ANSI33"), None);
    assert_eq!(predefined_pattern(""), None);
    assert_eq!(predefined_pattern("   "), None);
}

#[test]
fn ansi31_is_one_family_at_45_degrees_and_ansi37_is_two_ninety_degrees_apart() {
    let ansi31 = predefined_pattern("ANSI31").expect("ANSI31 is implemented");
    let pattern = ansi31.to_hatch_pattern(1.0, 0.0, false);
    assert_eq!(pattern.name, "ANSI31");
    let families = pattern.families();
    assert_eq!(families.len(), 1, "ANSI31 is a single line family");
    assert!(
        (families[0].angle - std::f64::consts::FRAC_PI_4).abs() < 1e-12,
        "ANSI31 runs at 45 degrees: {}",
        families[0].angle
    );
    // The offset is perpendicular to the line and one pattern unit (0.125)
    // long, i.e. the family's perpendicular spacing is exactly the transcribed
    // `.pat` spacing at scale 1.
    let direction = unit(families[0].angle.cos(), families[0].angle.sin());
    let perpendicular = unit(families[0].offset[0], families[0].offset[1]);
    assert!(
        (direction[0] * perpendicular[0] + direction[1] * perpendicular[1]).abs() < 1e-12,
        "the offset must be perpendicular to the family direction"
    );
    let spacing = families[0].offset[0].hypot(families[0].offset[1]);
    assert!((spacing - 0.125).abs() < 1e-12, "spacing {spacing}");
    assert!(families[0].dashes.is_empty(), "ANSI31 rows are solid");

    let ansi37 = predefined_pattern("ANSI37").expect("ANSI37 is implemented");
    let families = ansi37.to_hatch_pattern(1.0, 0.0, false).families();
    assert_eq!(families.len(), 2, "ANSI37 is two crossed families");
    let difference = (families[1].angle - families[0].angle).to_degrees() % 360.0;
    assert!(
        (difference - 90.0).abs() < 1e-9 || (difference - 270.0).abs() < 1e-9,
        "the two ANSI37 families are 90 degrees apart: {difference}"
    );
    for family in &families {
        let spacing = family.offset[0].hypot(family.offset[1]);
        assert!((spacing - 0.125).abs() < 1e-12, "spacing {spacing}");
    }
}

#[test]
fn ansi32_is_a_dashed_family_and_keeps_its_dash_lengths() {
    let ansi32 = predefined_pattern("ANSI32").expect("ANSI32 is implemented");
    let pattern = ansi32.to_hatch_pattern(1.0, 0.0, false);
    assert_eq!(pattern.lines.len(), 1);
    assert_eq!(pattern.lines[0].dashes, vec![0.125, -0.0625]);
    let families = pattern.families();
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].dashes, vec![0.125, -0.0625]);
    // At scale 2 the dash pattern and the perpendicular spacing scale together.
    let scaled = ansi32.to_hatch_pattern(2.0, 0.0, false).families();
    assert_eq!(scaled[0].dashes, vec![0.25, -0.125]);
    let spacing = scaled[0].offset[0].hypot(scaled[0].offset[1]);
    assert!((spacing - 0.75).abs() < 1e-12, "spacing {spacing}");
}

#[test]
fn the_definition_composes_with_scale_angle_and_double_like_the_pat_formula() {
    // The `.pat` formula this table is transcribed from: line angle =
    // row angle + hatch angle; base/offset/dashes scaled by the hatch scale;
    // offset rotated by the hatch-level angle on top of the row's own rotation;
    // `double` adds the same family turned by +90 degrees.
    let (scale, angle_deg, double): (f64, f64, bool) = (2.0, 30.0, true);
    let hatch_angle = angle_deg.to_radians();
    for pattern in predefined_patterns() {
        let built = pattern.to_hatch_pattern(scale, angle_deg, double);
        assert_eq!(built.name, pattern.name);
        assert!((built.angle - hatch_angle).abs() < 1e-12);
        assert!((built.scale - scale).abs() < 1e-12);
        assert!(built.double);

        let expected: Vec<PatternLine> = pattern
            .lines
            .iter()
            .flat_map(|line| {
                let row_angle = line.angle_deg.to_radians();
                let (row_sin, row_cos) = row_angle.sin_cos();
                // Row offset in the row's rotated frame -> plane frame.
                let plane = [
                    row_cos * line.offset[0] - row_sin * line.offset[1],
                    row_sin * line.offset[0] + row_cos * line.offset[1],
                ];
                // Then the hatch-level rotation and scale.
                let (sin, cos) = hatch_angle.sin_cos();
                let rotate = |x: f64, y: f64| [cos * x - sin * y, sin * x + cos * y];
                let base = rotate(scale * line.base[0], scale * line.base[1]);
                let offset = rotate(scale * plane[0], scale * plane[1]);
                let dashes: Vec<f64> = line.dashes.iter().map(|dash| dash * scale).collect();
                let angle = row_angle + hatch_angle;
                let mut families = vec![PatternLine {
                    angle,
                    base,
                    offset,
                    dashes: dashes.clone(),
                }];
                if double {
                    families.push(PatternLine {
                        angle: angle + std::f64::consts::FRAC_PI_2,
                        base,
                        offset: [-offset[1], offset[0]],
                        dashes,
                    });
                }
                families
            })
            .collect();

        let families = built.families();
        assert_eq!(
            families.len(),
            expected.len(),
            "{}: family count",
            pattern.name
        );
        for (found, wanted) in families.iter().zip(&expected) {
            assert!(
                (found.angle - wanted.angle).abs() < 1e-12,
                "{}: angle {} vs {}",
                pattern.name,
                found.angle,
                wanted.angle
            );
            assert!((found.base[0] - wanted.base[0]).abs() < 1e-12);
            assert!((found.base[1] - wanted.base[1]).abs() < 1e-12);
            assert!((found.offset[0] - wanted.offset[0]).abs() < 1e-12);
            assert!((found.offset[1] - wanted.offset[1]).abs() < 1e-12);
            assert_eq!(found.dashes, wanted.dashes);
        }
        // The double flag really doubles the row families.
        assert_eq!(families.len(), pattern.lines.len() * 2);
    }
}

#[test]
fn the_pattern_name_survives_the_conversion() {
    for pattern in predefined_patterns() {
        assert_eq!(pattern.to_hatch_pattern(1.0, 0.0, false).name, pattern.name);
        assert_eq!(
            pattern.to_hatch_pattern(1.0, 0.0, false).lines.len(),
            pattern.lines.len()
        );
    }
}
