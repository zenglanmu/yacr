use super::*;
use cad_domain::{CadError, Point3, TextAlignH, TextAlignV};
use std::sync::Arc;

use super::finalize::{finalize_lines, finalize_text, validate_nonzero_fill_equivalence};
use super::layout::{layout_parsed, ShapedLine};
use super::mtext::TAB_STEP;

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

#[test]
fn bundled_osifont_sfnt_glyphs_fill_faces_and_preserve_even_odd_holes() {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    engine.set_fallback(vec!["osifont.ttf".into()]);
    let shaped = engine
        .shape(
            "osifont.ttf",
            "O",
            Point3::default(),
            20.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    assert!(
        !shaped.meshes.is_empty(),
        "SFNT O must produce a filled mesh"
    );
    assert!(
        shaped.polylines.is_empty(),
        "successful SFNT faces must not be emitted as wireframe"
    );
    assert!(
        !shaped.outlines.is_empty(),
        "outline data remains available to callers"
    );
    assert!(!engine
        .outline(
            "osifont.ttf",
            "O",
            Point3::default(),
            20.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline
        )
        .unwrap()
        .is_empty());
    assert!(shaped
        .issues
        .iter()
        .all(|issue| issue.code != "text.sfnt_fill_partial"));
    let xs: Vec<f64> = shaped.meshes[0].vertices.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = shaped.meshes[0].vertices.iter().map(|p| p.y).collect();
    let hole = [
        (xs.iter().copied().fold(f64::INFINITY, f64::min)
            + xs.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            / 2.0,
        (ys.iter().copied().fold(f64::INFINITY, f64::min)
            + ys.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            / 2.0,
    ];
    let contains = |point: [f64; 2], triangle: [u32; 3]| {
        let [a, b, c] = triangle.map(|i| shaped.meshes[0].vertices[i as usize]);
        let cross = |p: Point3, q: Point3, r: [f64; 2]| {
            (q.x - p.x) * (r[1] - p.y) - (q.y - p.y) * (r[0] - p.x)
        };
        let values = [cross(a, b, point), cross(b, c, point), cross(c, a, point)];
        values.iter().all(|v| *v >= -1e-9) || values.iter().all(|v| *v <= 1e-9)
    };
    // The glyph's central cavity is not covered by any triangle.
    assert!(!shaped.meshes[0]
        .triangles
        .iter()
        .copied()
        .any(|t| contains(hole, t)));
    let mixed = engine
        .shape(
            "osifont.ttf",
            "{\\fmissing|b0|i0;A}O",
            Point3::default(),
            20.0,
            0.0,
            TextAlignH::Center,
            TextAlignV::Middle,
            None,
        )
        .unwrap();
    assert!(
        mixed.meshes.len() >= 2,
        "MTEXT run fallback must retain both glyphs"
    );
    assert!(mixed.polylines.is_empty());
}

#[test]
fn sfnt_fill_failure_is_retained_as_partial_diagnostic() {
    let line = ShapedLine {
        polys: vec![Vec::new(), vec![[0.0, 0.0], [1.0, 0.0]], Vec::new()],
        width: 1.0,
        height: 1.0,
    };
    let finalized = finalize_text(
        vec![line],
        1.0,
        Point3::default(),
        0.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
    );
    assert!(finalized.meshes.is_empty());
    assert_eq!(finalized.fill_failures.len(), 1);
    assert_eq!(
        finalized.polylines.len(),
        1,
        "failed glyph keeps its available contour fallback"
    );
}

#[test]
fn even_odd_fill_is_refused_for_same_winding_nested_sfnt_contours() {
    let outer = vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 10.0],
        [0.0, 10.0],
        [0.0, 0.0],
    ];
    let inner = vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0], [3.0, 3.0]];
    assert_eq!(
        validate_nonzero_fill_equivalence(&[outer.clone(), inner.clone()]),
        Err("nested contour winding does not alternate")
    );
    let line = ShapedLine {
        polys: vec![Vec::new(), outer, inner, Vec::new()],
        width: 10.0,
        height: 10.0,
    };
    let finalized = finalize_text(
        vec![line],
        10.0,
        Point3::default(),
        0.0,
        TextAlignH::Left,
        TextAlignV::Baseline,
    );
    assert!(finalized.meshes.is_empty());
    assert_eq!(
        finalized.polylines.len(),
        2,
        "failed fill keeps boundary line fallback"
    );
    assert!(finalized.fill_failures[0].contains("winding does not alternate"));
}

#[test]
fn sfnt_contour_components_with_intersections_are_refused() {
    let a = vec![[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0], [0.0, 0.0]];
    let b = vec![[2.0, 2.0], [5.0, 2.0], [5.0, 5.0], [2.0, 5.0], [2.0, 2.0]];
    assert_eq!(
        validate_nonzero_fill_equivalence(&[a, b]),
        Err("intersecting or overlapping contours")
    );
}

#[test]
fn opposite_winding_nested_sfnt_contours_are_fill_equivalent() {
    let outer = vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 10.0],
        [0.0, 10.0],
        [0.0, 0.0],
    ];
    let hole = vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0], [3.0, 3.0]];
    assert_eq!(validate_nonzero_fill_equivalence(&[outer, hole]), Ok(()));
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
    let shaped = engine
        .shape(
            "txt.shx",
            "AB",
            Point3::default(),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    let polys = shaped.polylines;
    assert!(shaped.meshes.is_empty(), "SHX remains linework");
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
pub(super) fn mock_measure(run: &TextRun, text: &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)> {
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
    let lines = layout_parsed(&narrow, 10.0, None, mock_measure);
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
    let lines = layout_parsed(&oblique, 10.0, None, mock_measure);
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
    let lines = layout_parsed(&parsed, 10.0, None, mock_measure);
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
    let lines = layout_parsed(&parsed, 10.0, None, mock_measure);
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
            None,
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
    assert!(!shaped.outlines.is_empty());
    // Compare just the height-overridden run so paragraph baseline spacing
    // cannot mask its larger glyph extent.
    let tall_run = engine
        .shape(
            "arial.ttf",
            "{\\H2x;CD}",
            Point3::default(),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    let plain = engine
        .outline(
            "arial.ttf",
            "CD",
            Point3::default(),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
        )
        .unwrap();
    let height = |polys: &[Vec<Point3>]| {
        let min = polys
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let max = polys
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        max - min
    };
    let tall_height = height(&tall_run.outlines);
    let plain_height = height(&plain);
    assert!(
        tall_height > plain_height * 1.5,
        "2x MTEXT run height {tall_height} should exceed plain glyph height {plain_height}"
    );
}
