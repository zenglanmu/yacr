//! Layer attribute import contract tests.
//!
//! These prove **storage**, not behaviour: the importer records the source
//! layer's flags, colour, linetype, lineweight, plottability and opacity beside
//! the `Layer` row, and keeps `off` and `frozen` distinct even though both make
//! the row invisible. Nothing reads them to change what is drawn.

use super::*;

/// sRGB of an ACI index through acadrust's own canonical table, so the test
/// never hard-codes a colour triple the reader might map differently.
fn aci_rgb(index: u8) -> [u8; 3] {
    let (r, g, b) = acadrust::Color::Index(index)
        .rgb()
        .expect("a non-zero ACI index resolves");
    [r, g, b]
}

/// A synthetic writer-produced DWG carrying layers that exercise every stored
/// attribute.
///
/// Uses acadrust's own writer, exactly like `synthetic_dwg`, so the bytes are a
/// real AC10xx DWG. Layer `A-ALL` sets every field the storage models; `A-OFF`
/// and `A-FROZEN` exist so the two flags can be proven **distinct**.
fn layer_dwg() -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();

    // A dash linetype with a real definition, so `A-ALL`'s linetype name
    // resolves through its handle instead of falling back to `Continuous`.
    let mut dashed = acadrust::tables::LineType::new("DASHED");
    dashed.elements = vec![acadrust::tables::LineTypeElement {
        length: 0.5,
        complex: None,
    }];
    dashed.pattern_length = 0.75;
    doc.line_types.add_or_replace(dashed);

    // One model-space line keeps the drawing a realistic non-empty file.
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    line.end = acadrust::types::Vector3::new(1.0, 1.0, 0.0);
    doc.add_entity(EntityType::Line(line)).expect("add line");

    let mut all = acadrust::tables::Layer::new("A-ALL");
    all.flags.off = true;
    all.flags.frozen = true;
    all.flags.frozen_in_new_viewport = true;
    all.flags.locked = true;
    all.flags.xref_dependent = true;
    all.is_plottable = false;
    all.color = acadrust::Color::Index(1);
    all.line_type = "DASHED".to_string();
    // 0.35 mm, in the DWG table's 1/100 mm scale and one of its indexed values.
    all.line_weight = acadrust::LineWeight::Value(35);
    // 20 % transparent, in the source's own byte scale (0 = opaque).
    all.transparency = acadrust::Transparency::Explicit(51);
    doc.layers.add_or_replace(all);

    let mut off_only = acadrust::tables::Layer::new("A-OFF");
    off_only.flags.off = true;
    doc.layers.add_or_replace(off_only);

    let mut frozen_only = acadrust::tables::Layer::new("A-FROZEN");
    frozen_only.flags.frozen = true;
    doc.layers.add_or_replace(frozen_only);

    acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG")
}

fn imported_layers() -> DrawingDatabase {
    AcadrustImporter::new()
        .import(&request(layer_dwg()), &|| false)
        .expect("synthetic layer fixture imports")
        .database
}

/// The id of the layer with this exact name.
fn layer_id(db: &DrawingDatabase, name: &str) -> LayerId {
    db.layers()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("no layer named {name}"))
        .id
}

#[test]
fn importer_records_every_layer_attribute_field() {
    let db = imported_layers();
    let id = layer_id(&db, "A-ALL");
    let attributes = db.layer_attributes(id);

    assert!(attributes.off);
    assert!(attributes.frozen);
    assert!(attributes.frozen_in_new_viewport);
    assert!(attributes.locked);
    assert!(attributes.xref_dependent);
    assert!(!attributes.plottable, "`is_plottable = false` must survive");

    // The ACI identity and the resolved sRGB value are both carried, and the
    // index is not the default 7.
    assert_eq!(attributes.color.aci, Some(1));
    assert_eq!(attributes.color.rgb, aci_rgb(1));
    assert_ne!(attributes.color.rgb, default_white());

    assert_eq!(attributes.linetype, "DASHED");
    assert_eq!(attributes.lineweight, LayerLineWeight::Hundredths(35));
    // 51/255 transparency → opacity 0.8 (the stored value is the complement).
    assert!(
        (attributes.opacity - (1.0 - 51.0 / 255.0)).abs() < 1e-6,
        "unexpected opacity {}",
        attributes.opacity
    );
}

/// The default white, spelled out so the colour assertion cannot pass by
/// accidentally comparing two identical defaults.
fn default_white() -> [u8; 3] {
    [255, 255, 255]
}

#[test]
fn importer_keeps_off_and_frozen_distinct_while_visible_is_their_negation() {
    // This is the whole point of the phase: the pre-attribute importer dropped
    // both bits into one `visible` boolean, so a frozen layer became
    // indistinguishable from an off one.
    let db = imported_layers();

    let off_only = layer_id(&db, "A-OFF");
    let off_attributes = db.layer_attributes(off_only);
    assert!(off_attributes.off);
    assert!(!off_attributes.frozen, "an off layer is not a frozen layer");
    assert!(!db.layer(off_only).expect("row exists").visible);

    let frozen_only = layer_id(&db, "A-FROZEN");
    let frozen_attributes = db.layer_attributes(frozen_only);
    assert!(!frozen_attributes.off, "a frozen layer is not an off layer");
    assert!(frozen_attributes.frozen);
    assert!(!db.layer(frozen_only).expect("row exists").visible);

    // The documented invariant, asserted on every imported row.
    for layer in db.layers() {
        let attributes = db.layer_attributes(layer.id);
        assert_eq!(
            layer.visible,
            !(attributes.off || attributes.frozen),
            "visible != !(off || frozen) for layer '{}'",
            layer.name
        );
        assert_eq!(layer.visible, attributes.is_visible());
    }
}

#[test]
fn importer_gives_an_unflagged_layer_the_documented_defaults() {
    let db = imported_layers();
    let id = layer_id(&db, "0");
    let attributes = db.layer_attributes(id);
    assert!(!attributes.off);
    assert!(!attributes.frozen);
    assert!(!attributes.frozen_in_new_viewport);
    assert!(!attributes.locked);
    assert!(!attributes.xref_dependent);
    assert!(attributes.plottable);
    assert_eq!(attributes.color.aci, Some(7));
    assert_eq!(attributes.linetype, "Continuous");
    // A source layer that declares no weight says "Default" (DXF 370 = -3), and
    // that is preserved verbatim. `LayerLineWeight::ByLayer` is the *cad-db*
    // default for a row with no recorded attributes, not a rewrite of a source
    // that explicitly said `Default`.
    assert_eq!(attributes.lineweight, LayerLineWeight::Default);
    assert_eq!(attributes.opacity, 1.0);
    // A plot-style *name* never survives a file round trip: neither reader
    // populates it (the writers emit a handle, group 390). `Some(name)` is
    // covered by the direct-mapping test below.
    assert_eq!(attributes.plot_style, None);
    assert!(db.layer(id).expect("row exists").visible);
}

#[test]
fn layer_attributes_mapping_keeps_a_plot_style_name_and_defaults_an_empty_linetype() {
    // `plot_style` cannot arrive through a written file (it is a name, and both
    // writers serialise only the plot-style handle), so the mapping itself is
    // exercised directly. An empty linetype name is the DXF default and is
    // stored as `Continuous` rather than an empty string.
    let mut source = acadrust::tables::Layer::new("X");
    source.line_type = "  ".to_string();
    source.plot_style = "  Normal  ".to_string();
    source.color = acadrust::Color::Rgb {
        r: 10,
        g: 20,
        b: 30,
    };
    source.line_weight = acadrust::LineWeight::Default;
    source.transparency = acadrust::Transparency::ByLayer;

    let attributes = crate::style::layer_attributes(&source);
    assert_eq!(attributes.linetype, "Continuous");
    assert_eq!(attributes.plot_style.as_deref(), Some("Normal"));
    // A true-colour source has no ACI identity to preserve.
    assert_eq!(attributes.color.aci, None);
    assert_eq!(attributes.color.rgb, [10, 20, 30]);
    assert_eq!(attributes.lineweight, LayerLineWeight::Default);
    // `ByLayer`/`ByBlock` are degenerate on a layer and resolve to opaque
    // rather than a fabricated transparency.
    assert_eq!(attributes.opacity, 1.0);
}
