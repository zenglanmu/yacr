//! MTEXT honesty contract (ADR 0019 decision H, P7a).
//!
//! `MTEXT → Text` used to report `Complete` while silently dropping columns,
//! background fill, line-spacing style and drawing direction. It now reports
//! `Partial` with four stable named reasons. No domain field is added: the fold is
//! still lossy, it is just no longer dishonest.

use super::*;

#[test]
fn an_mtext_fold_is_partial_with_the_four_named_reasons() {
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

    // The old `Complete` claim was false: columns/background/line-spacing
    // style/drawing direction are not modelled by `Text`.
    let reasons = match completeness {
        Completeness::Partial(reasons) => reasons,
        other => panic!("MTEXT must be Partial, got {other:?}"),
    };
    assert_eq!(
        reasons,
        vec![
            "text.background_unsupported".to_string(),
            "text.columns_unsupported".to_string(),
            "text.line_spacing_style_unsupported".to_string(),
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
        4,
        "the four reasons are reported once per import: {:?}",
        builder.diagnostics
    );
}
