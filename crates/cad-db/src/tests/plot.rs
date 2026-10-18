//! Plot settings queries and defaults.

use super::*;

#[test]
fn plot_settings_are_optional_and_default_to_an_explicit_page() {
    use crate::tables::{
        PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType,
    };

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    let db = b.finish().unwrap();
    // No stored record: the query returns an explicit documented default, and
    // the raw accessor stays honest about the absence.
    assert!(db.plot_settings(LayoutId(1)).is_none());
    let fallback = db.plot_settings_for(LayoutId(1));
    assert!(matches!(
        fallback.provenance,
        PlotProvenance::DefaultPage { .. }
    ));
    assert_eq!(fallback.paper_width, 210.0);
    assert_eq!(fallback.paper_height, 297.0);

    // A stored record round-trips exactly, including rotation and margins.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.set_plot_settings(PlotSettingsRecord {
        layout: LayoutId(1),
        paper_size_name: "ISO_A3".into(),
        paper_width: 297.0,
        paper_height: 420.0,
        margins: PlotMargins::uniform(12.0),
        rotation: PlotRotation::Degrees90,
        scale_numerator: 1.0,
        scale_denominator: 100.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    })
    .unwrap();
    let db = b.finish().unwrap();
    let stored = db.plot_settings(LayoutId(1)).unwrap();
    assert_eq!(stored.rotation.to_degrees(), 90.0);
    assert_eq!(stored.rotated_size(), (420.0, 297.0));
    assert_eq!(stored.margins.horizontal_total(), 24.0);
    assert_eq!(stored.margins.vertical_total(), 24.0);
}

#[test]
fn plot_settings_for_an_unknown_layout_are_rejected() {
    use crate::tables::{
        PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType,
    };
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // No layout inserted: attaching settings would be silently unreachable.
    let result = b.set_plot_settings(PlotSettingsRecord {
        layout: LayoutId(7),
        paper_size_name: String::new(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: PlotMargins::default(),
        rotation: PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    });
    assert!(result.is_err());
}
