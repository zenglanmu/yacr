//! DIMSTYLE table import and the DIMLUNIT degradation honesty test (ADR 0017
//! B.1/B.2, F.4).
//!
//! The synthetic document is produced by acadrust's own writer, so the bytes are
//! a real DXF rather than a vendored sample (no authorized fixture required).

use super::*;

/// A synthetic DXF with a linear dimension whose style sets `DIMLUNIT`.
fn dxf_with_dimlunit(format: i16) -> Vec<u8> {
    use acadrust::entities::{Dimension, DimensionLinear, EntityType};
    use acadrust::tables::DimStyle;
    use acadrust::types::Vector3;

    let mut doc = acadrust::CadDocument::new();
    let mut style = DimStyle::standard();
    style.name = "Arch".to_string();
    style.dimlunit = format;
    style.dimdec = 3;
    style.dimtxt = 4.0;
    doc.dim_styles.add_or_replace(style);

    let mut dim = DimensionLinear::new(Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0));
    dim.base.style_name = "Arch".to_string();
    dim.base.normal = Vector3::UNIT_Z;
    dim.definition_point = Vector3::new(0.0, -5.0, 0.0);
    doc.add_entity(EntityType::Dimension(Dimension::Linear(dim)))
        .expect("add dimension");

    acadrust::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("write synthetic DXF")
}

#[test]
fn imported_dimstyle_table_carries_the_full_variable_set() {
    let drawing = AcadrustImporter::new()
        .import(&request(dxf_with_dimlunit(4)), &|| false)
        .expect("import");
    let style = drawing
        .database
        .dim_style_by_name("Arch")
        .expect("the source style is imported");
    assert_eq!(style.linear_unit_format, 4);
    assert_eq!(style.decimals, 3);
    assert!((style.text_height - 4.0).abs() < 1e-9);
    // The pinned Standard is present alongside the source style.
    assert!(drawing.database.dim_style_by_name("Standard").is_some());
    // The creation-time resolution API yields the same stored values.
    let resolved = drawing
        .database
        .resolve_dim_style("Arch")
        .expect("the style resolves");
    assert_eq!(resolved.linear_unit_format, 4);
    assert!((resolved.text_height - 4.0).abs() < 1e-9);
}

#[test]
fn non_decimal_dimlunit_is_declared_partial_not_a_silent_decimal() {
    // ADR 0017 F.4 / B.2(b): DIMLUNIT != 2 must be declared unimplemented, not
    // silently rendered as decimal. The dimension's semantic capability is
    // `Partial` (the reason text is pinned by the cad-domain unit test).
    let drawing = AcadrustImporter::new()
        .import(&request(dxf_with_dimlunit(4)), &|| false)
        .expect("import");
    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|c| c.type_key == "AcDbDimension")
        .expect("a dimension capability");
    assert_eq!(
        capability.semantic,
        SupportStatus::Partial,
        "DIMLUNIT 4 must be declared, not reported Complete"
    );

    // The decimal format stays Complete, so the declaration is specific.
    let drawing = AcadrustImporter::new()
        .import(&request(dxf_with_dimlunit(2)), &|| false)
        .expect("import");
    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|c| c.type_key == "AcDbDimension")
        .expect("a dimension capability");
    assert_eq!(
        capability.semantic,
        SupportStatus::Verified,
        "decimal DIMLUNIT stays Complete"
    );
}
