//! Annotation scales.

use super::*;

// ---------------------------------------------------------------------------
// Annotation scales
// ---------------------------------------------------------------------------

fn scale(id: u128, name: &str, paper: f64, drawing: f64) -> crate::tables::Scale {
    crate::tables::Scale {
        id: ScaleId(id),
        name: name.to_string(),
        paper_units: paper,
        drawing_units: drawing,
    }
}

#[test]
fn scale_factor_matches_the_paper_drawing_ratio() {
    let unit = scale(1, "1:1", 1.0, 1.0);
    assert_eq!(unit.factor(), 1.0);
    assert!(unit.is_unit_scale());
    assert!(!unit.is_reduction());
    assert!(!unit.is_enlargement());

    let reduction = scale(2, "1:100", 1.0, 100.0);
    assert!((reduction.factor() - 0.01).abs() < 1e-12);
    assert!((reduction.inverse_factor() - 100.0).abs() < 1e-9);
    assert!(reduction.is_reduction());

    let enlargement = scale(3, "2:1", 2.0, 1.0);
    assert_eq!(enlargement.factor(), 2.0);
    assert!(enlargement.is_enlargement());

    // A degenerate drawing-units value falls back to 1.0 rather than infinity.
    let degenerate = scale(4, "bad", 1.0, 0.0);
    assert_eq!(degenerate.factor(), 1.0);
    assert!(degenerate.is_well_formed());

    let non_finite = scale(5, "nan", f64::NAN, 1.0);
    assert!(!non_finite.is_well_formed());
}

#[test]
fn scales_and_active_scale_are_resolvable_by_name() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_scale(scale(1, "1:1", 1.0, 1.0)).unwrap();
    b.insert_scale(scale(2, "1:100", 1.0, 100.0)).unwrap();
    b.set_active_annotation_scale("1:100", 0.01).unwrap();
    let db = b.finish().unwrap();

    assert_eq!(db.scales().count(), 2);
    assert_eq!(db.scale_by_name("1:100").unwrap().factor(), 0.01);
    assert!(db.scale_by_name("1:999").is_none());

    let active = db.active_annotation_scale().expect("active scale set");
    assert_eq!(active.name, "1:100");
    assert!((active.value - 0.01).abs() < 1e-12);
    assert!(active.named, "1:100 is present in the table");
    assert_eq!(db.annotation_scale_name(), Some("1:100"));
    assert_eq!(db.annotation_scale(), (Some("1:100".into()), 0.01, true));
}

#[test]
fn active_scale_not_in_the_table_is_reported_named_false() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.set_active_annotation_scale("1:50", 0.02).unwrap();
    let db = b.finish().unwrap();
    let active = db.active_annotation_scale().expect("active scale set");
    assert!(!active.named, "1:50 was never inserted");
    assert!(!db.annotation_scale().2);
}

#[test]
fn builder_rejects_non_finite_scale_and_non_finite_active_value() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    let err = b
        .insert_scale(scale(1, "bad", f64::INFINITY, 1.0))
        .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    let err = b.set_active_annotation_scale("1:1", f64::NAN).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
}
