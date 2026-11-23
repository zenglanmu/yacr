//! MTEXT honesty contract (ADR 0019 decision H, P7a).
//!
//! `MTEXT → Text` used to report `Complete` while silently dropping columns,
//! background fill, line-spacing style and drawing direction. It now reports
//! `Partial` with three stable named reasons — line spacing is *modelled* now
//! (DXF groups 44 + 73 → `TextLineSpacing`) and is carried, not dropped.

use super::*;

#[test]
fn an_mtext_fold_is_partial_with_the_three_named_reasons() {
    let req = super::request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mtext = acadrust::entities::MText::new();
    mtext.value = "line1\\Pline2".into();
    mtext.insertion_point = acadrust::types::Vector3::new(1.0, 2.0, 0.0);
    mtext.height = 2.5;
    mtext.rotation = 0.25;
    let entity = EntityType::MText(Box::new(mtext));
    let common = entity.common();

    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Text {
        text,
        position,
        height,
        rotation,
        ..
    } = &geometry
    else {
        panic!("MTEXT folds into Text, got {geometry:?}");
    };
    assert_eq!(text, "line1\\Pline2", "the raw value is carried verbatim");
    assert!((position.x - 1.0).abs() < 1e-12);
    assert!((position.y - 2.0).abs() < 1e-12);
    assert_eq!(*height, 2.5);
    assert_eq!(*rotation, 0.25);

    // The old `Complete` claim was false: columns/background/drawing direction
    // are still not modelled by `Text`.
    let reasons = match completeness {
        Completeness::Partial(reasons) => reasons,
        other => panic!("MTEXT must be Partial, got {other:?}"),
    };
    assert_eq!(
        reasons,
        vec![
            "text.background_unsupported".to_string(),
            "text.columns_unsupported".to_string(),
            "text.drawing_direction_unsupported".to_string(),
        ]
    );

    // Each reason is visible in the report once, with a stable diagnostic code.
    let codes: Vec<(String, String)> = builder
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.code.clone(), diagnostic.message.clone()))
        .collect();
    for reason in &reasons {
        assert!(
            codes
                .iter()
                .any(|(code, message)| code == "import.text_partial" && message == reason),
            "{reason} must be reported: {codes:?}"
        );
    }
    // Line spacing is no longer refused: the reason key is gone entirely.
    assert!(
        !codes
            .iter()
            .any(|(_, message)| message.contains("line_spacing_style_unsupported")),
        "line spacing is modelled now: {codes:?}"
    );
}

#[test]
fn a_second_mtext_does_not_duplicate_the_reported_reasons() {
    let req = super::request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    for value in ["first", "second"] {
        let mut mtext = acadrust::entities::MText::new();
        mtext.value = value.into();
        mtext.height = 1.0;
        let entity = EntityType::MText(Box::new(mtext));
        let common = entity.common();
        let (_, completeness) = builder.convert(&entity, common);
        assert!(matches!(completeness, Completeness::Partial(_)));
    }
    assert_eq!(
        builder
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "import.text_partial")
            .count(),
        3,
        "the three reasons are reported once per import: {:?}",
        builder.diagnostics
    );
}

#[test]
fn mtext_line_spacing_groups_44_and_73_are_carried() {
    let req = super::request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mtext = acadrust::entities::MText::new();
    mtext.value = "a\\Pb".into();
    mtext.height = 1.0;
    mtext.line_spacing_factor = 1.75;
    mtext.line_spacing_style = acadrust::entities::LineSpacingStyle::Exactly;
    let entity = EntityType::MText(Box::new(mtext));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Text { line_spacing, .. } = &geometry else {
        panic!("MTEXT folds into Text, got {geometry:?}");
    };
    assert_eq!(line_spacing.factor, 1.75, "group 44 is the factor");
    assert_eq!(
        line_spacing.style,
        cad_domain::TextLineSpacingStyle::Exactly,
        "group 73 selects the style"
    );
    assert!(
        !matches!(&completeness, Completeness::Partial(reasons)
            if reasons.iter().any(|reason| reason.contains("line_spacing"))),
        "the carried spacing raises no line-spacing reason: {completeness:?}"
    );
}

#[test]
fn a_degenerate_mtext_line_spacing_factor_falls_back_with_a_named_reason() {
    let req = super::request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mtext = acadrust::entities::MText::new();
    mtext.value = "a".into();
    mtext.height = 1.0;
    mtext.line_spacing_factor = 0.0;
    let entity = EntityType::MText(Box::new(mtext));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Text { line_spacing, .. } = &geometry else {
        panic!("MTEXT folds into Text, got {geometry:?}");
    };
    assert!(
        line_spacing.is_default(),
        "an unusable factor falls back to the neutral one"
    );
    let reasons = match completeness {
        Completeness::Partial(reasons) => reasons,
        other => panic!("MTEXT must be Partial, got {other:?}"),
    };
    assert!(
        reasons.contains(&"text.line_spacing_factor_degenerate".to_string()),
        "the narrower reason names the exact value that cannot be stored: {reasons:?}"
    );
    assert!(
        builder
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.text_partial"
                && diagnostic
                    .message
                    .contains("line-spacing factor (group 44) is 0")),
        "the diagnostic says which value was refused: {:?}",
        builder.diagnostics
    );
}
