//! DIMSTYLE table import and the DIMLUNIT degradation honesty test (ADR 0017
//! B.1/B.2, F.4).
//!
//! The synthetic document is produced by acadrust's own writer, so the bytes are
//! a real DXF rather than a vendored sample (no authorized fixture required).

use super::*;

/// ADR 0017 U-8, pinned with (a) a **real artifact**: `entities.dxf` `*D65` is a
/// QCAD-written two-line angular dimension whose groups 13->14 and 15->10 are
/// non-collinear and whose group 15 is **not** at their intersection. The reading
/// is line 1 = 13->14, line 2 = 15->10, vertex = their intersection, and group 16
/// selects the sector: group 42 (1.975688113 rad = 113.19859 degrees) and the
/// anonymous block's ARC centre (90, 40) both confirm it. The synthesis now agrees
/// with the block picture; the `-Z` reading (vertex = group 15, ignore group 14)
/// would measure 112.56 degrees and place the arc at (90, 39.375).
#[test]
fn angular_two_line_reading_is_pinned_by_the_committed_qcad_fixture() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("fixtures/dxf/qcad-examples/entities.dxf"))
        .expect("fixture readable");
    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("import");

    let mut inspected = 0;
    for entity in drawing.database.entities() {
        let SemanticGeometry::Dimension(dimension) = &entity.geometry else {
            continue;
        };
        let DimensionPoints::Angular2Ln {
            line1_start,
            line1_end: _,
            line2_start: _,
            line2_end: _,
            arc_point: _,
        } = &dimension.definition().points
        else {
            continue;
        };
        if (line1_start.x - 42.363_533_475_452_33).abs() > 1e-6 {
            continue;
        }
        inspected += 1;
        assert!(
            (dimension.measurement().to_radians() - 1.975_688_113_079_98).abs() < 1e-9,
            "group 42 is 1.975688113 rad; the imported measurement is {} deg = {} rad",
            dimension.measurement(),
            dimension.measurement().to_radians()
        );
        // The anonymous block picture is the authority for the vertex: its ARC is
        // centred on the intersection of the two lines, not on group 15.
        let arc_center = dimension.display().iter().find_map(|child| match child {
            SemanticGeometry::Arc { center, .. } => Some(*center),
            _ => None,
        });
        let arc_center = arc_center.expect("the picture carries an angular ARC");
        assert!(
            (arc_center.x - 90.0).abs() < 1e-6 && (arc_center.y - 40.0).abs() < 1e-6,
            "the block's ARC is centred on the lines' intersection (90, 40), got {arc_center:?}"
        );
    }
    assert_eq!(
        inspected, 1,
        "one *D65 two-line angular dimension is inspected"
    );
}

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
    doc.add_entity(EntityType::Dimension(Box::new(Dimension::Linear(dim))))
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
